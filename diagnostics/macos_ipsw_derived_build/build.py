"""One separately frozen hosted build. Never invokes the derived executable."""

import hashlib
from contextlib import contextmanager
import http.client
import importlib.util
import io
import json
import os
from pathlib import Path
import plistlib
import re
import resource
import selectors
import shutil
import signal
import stat
import struct
import subprocess
import sys
import tarfile
import time
import urllib.error
import urllib.parse
import urllib.request


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SPEC = importlib.util.spec_from_file_location("bounded_github", HERE.parent / "macos_sw_vers" / "diagnostic.py")
github = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(github)
IMPORT_SPEC = importlib.util.spec_from_file_location("static_imports", HERE / "static_imports.py")
static_imports = importlib.util.module_from_spec(IMPORT_SPEC)
IMPORT_SPEC.loader.exec_module(static_imports)
require = github.require
Error = github.DiagnosticError
encoded = github.encoded
digest = github.digest
MARKER = re.compile(r"<!-- macos-ipsw-derived-build-v1 (\{[^\r\n]*\}) -->")


def controller_inputs(environment):
    rejected = [name for name in ("OPENSSL_CONF", "OPENSSL_CONF_INCLUDE", "OPENSSL_MODULES",
                                  "SSLKEYLOGFILE", "SSL_CERT_FILE", "SSL_CERT_DIR") if name in environment]
    require(not rejected, "controller inputs rejected before any controller action (names only): " +
            json.dumps(rejected))
    inventory = {"proxy": [], "go_compiler_sdk_search": [], "loader": [], "native_reference": [],
                 "declared_control_plane": []}
    size = len(encoded(inventory))
    for name in environment:
        categories = []
        if name.lower().endswith("_proxy"):
            categories.append("proxy")
        if name.startswith(("GO", "CGO_", "CC", "CXX", "SDKROOT", "DEVELOPER_DIR", "TOOLCHAINS",
                            "CFLAGS", "CPPFLAGS", "CXXFLAGS", "LDFLAGS", "LIBRARY_PATH", "CPATH",
                            "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH")):
            categories.append("go_compiler_sdk_search")
        if name.startswith(("DYLD", "LD_")):
            categories.append("loader")
        if name.upper().startswith(("OPENSSL", "SSL_", "OSSL_")) or name.upper() in (
                "SSLKEYLOGFILE", "TSGET", "RANDFILE", "CTLOG_FILE", "QLOGDIR"):
            categories.append("native_reference")
        if name in ("GH_TOKEN", "GITHUB_EVENT_NAME", "GITHUB_RUN_ATTEMPT", "GITHUB_REPOSITORY",
                    "GITHUB_REF", "GITHUB_WORKFLOW_REF", "GITHUB_SHA", "ImageVersion"):
            categories.append("declared_control_plane")
        for category in categories:
            size += len(json.dumps(name, ensure_ascii=True).encode()) + 8
            require(size <= 4194304, "inherited input-name inventory exceeds existing per-file cap")
            inventory[category].append(name)
    for names in inventory.values():
        names.sort()
    require(len(encoded(inventory)) <= 4194304, "inherited input-name inventory exceeds existing per-file cap")
    return inventory


def fetch_github_json(endpoint, token, limits, deadline):
    remaining = deadline - time.monotonic()
    require(remaining > 0, "diagnostic gate deadline")
    request = urllib.request.Request("https://api.github.com/repos/lossyrob/telex/" + endpoint,
        headers={"Authorization": "Bearer " + token, "Accept": "application/vnd.github+json"})
    opener = urllib.request.build_opener(github.RefuseRedirects(), urllib.request.ProxyHandler({}))
    with opener.open(request, timeout=min(remaining, limits["github_request_seconds"])) as response:
        require(response.geturl() == request.full_url, "unexpected diagnostic gate redirect")
        raw = bytearray()
        while True:
            require(time.monotonic() < deadline, "diagnostic gate deadline during read")
            chunk = response.read1(min(65536, limits["github_response_bytes"] + 1 - len(raw)))
            if not chunk:
                break
            raw.extend(chunk)
            require(len(raw) <= limits["github_response_bytes"], "diagnostic GitHub response bound")
    require(len(raw) <= limits["github_response_bytes"], "diagnostic GitHub response bound")
    require(time.monotonic() < deadline, "diagnostic gate deadline after read")
    return json.loads(raw)


def failure_record(error, phase):
    return {"phase": phase, "type": type(error).__name__, "message": str(error),
            "interruption": not isinstance(error, Exception)}


def file_hash(file, maximum, deadline):
    info = file.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_size <= maximum, "file type/size bound: " + str(file))
    h = hashlib.sha256()
    with file.open("rb") as stream:
        while chunk := stream.read(65536):
            require(time.monotonic() < deadline, "hash deadline")
            h.update(chunk)
    require(file.stat().st_size == info.st_size, "file changed during hashing")
    return {"path": str(file), "bytes": info.st_size, "sha256": h.hexdigest()}


def closure(bounds):
    rows = []
    for relative in bounds["source_files"]:
        file = ROOT / relative
        require(file.is_file() and not file.is_symlink(), "missing source closure")
        raw = file.read_bytes()
        rows.append({"file": relative, "bytes": len(raw), "sha256": digest(raw)})
    return rows, digest(encoded(rows))


class Evidence:
    def __init__(self, root, limits):
        self.root, self.limits = root, limits

    def write(self, name, raw):
        require(Path(name).name == name, "flat artifact name")
        maximum = self.limits["binary_bytes"] if name == "ipsw" else self.limits["other_artifact_file_bytes"]
        require(len(raw) <= maximum, "artifact file bound")
        files = list(self.root.iterdir())
        require(all(stat.S_ISREG(f.lstat().st_mode) for f in files), "nonregular artifact")
        target = self.root / name
        require(not target.exists(), "immutable artifact already exists")
        require(len(files) + 1 <= self.limits["artifact_files"] and
                sum(f.stat().st_size for f in files) + len(raw) <= self.limits["artifact_total_bytes"],
                "aggregate artifact bound")
        with target.open("xb") as stream:
            stream.write(raw)

    def json(self, name, value):
        self.write(name, encoded(value))

    def finish(self):
        self.json("sha256.json", {f.name: digest(f.read_bytes()) for f in sorted(self.root.iterdir())})


def check_gate(bounds, bounds_hash, source_hash, output, deadline):
    env = os.environ
    require(env.get("GITHUB_EVENT_NAME") == "push" and env.get("GITHUB_RUN_ATTEMPT") == "1",
            "BUILD push attempt1 only; zero retries")
    require(env.get("GITHUB_REPOSITORY") == "lossyrob/telex" and
            env.get("GITHUB_REF") == "refs/heads/" + bounds["branch"] and
            env.get("GITHUB_WORKFLOW_REF") ==
            "lossyrob/telex/" + bounds["workflow"] + "@refs/heads/" + bounds["branch"],
            "BUILD repository/ref/workflow mismatch")
    sha = env.get("GITHUB_SHA", "")
    require(re.fullmatch(r"[0-9a-f]{40}", sha), "invalid BUILD SHA")
    expected = {"component": bounds["component"], "sha": sha, "bounds_sha256": bounds_hash,
                "source_manifest_sha256": source_hash, "jobs": bounds["jobs"], "attempt": 1, "allow_launch": True}
    limits = {"github_request_seconds": 15, "github_response_bytes": 4194304}
    records = []
    for page in range(1, 11):
        batch = fetch_github_json(f"issues/172/comments?per_page=100&page={page}", env["GH_TOKEN"], limits, deadline)
        require(isinstance(batch, list), "invalid BUILD comment page")
        for comment in batch:
            if comment.get("user", {}).get("login") == "lossyrob":
                for match in MARKER.finditer(comment.get("body", "")):
                    record = json.loads(match[1])
                    require(isinstance(record, dict), "malformed BUILD marker")
                    if record.get("component") == bounds["component"] and record.get("sha") == sha:
                        records.append((comment, record))
        if len(batch) < 100:
            break
    else:
        raise Error("BUILD comment-page bound")
    require(len(records) == 1, "missing/duplicate exact BUILD freeze")
    comment, record = records[0]
    require(all(type(record.get(k)) is type(v) and record[k] == v for k, v in expected.items()),
            "BUILD freeze tuple mismatch")
    url = record.get("source_review", "")
    require(re.fullmatch(r"https://github\.com/lossyrob/telex/issues/172#issuecomment-\d+", url),
            "missing BUILD source review")
    review = fetch_github_json("issues/comments/" + url.rsplit("-", 1)[1], env["GH_TOKEN"], limits, deadline)
    require(review.get("user", {}).get("login") == "lossyrob" and review.get("html_url") == url and
            review.get("issue_url") == "https://api.github.com/repos/lossyrob/telex/issues/172" and
            digest(review.get("body", "").encode()) == record.get("source_review_body_sha256"),
            "BUILD source-review identity mismatch")
    output.json("gate.json", {"expected": expected, "freeze": comment, "review": review,
                             "semantic_review_is_owner_responsibility": True, "provider_queries_yet": False})
    return sha


class ReturnRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Fetcher:
    def __init__(self, limits, deadline, ledger):
        self.limits, self.deadline, self.ledger = limits, deadline, ledger
        self.counts = {"source": 0, "compiler": 0, "module": 0}
        self.caps = {"source": limits["source_transfer_bytes"], "compiler": limits["compiler_archive_bytes"],
                     "module": limits["module_transfer_bytes"]}
        self.opener = urllib.request.build_opener(ReturnRedirect(), urllib.request.ProxyHandler({}))

    def fetch(self, url, destination, kind, hosts, file_limit=None):
        require(not destination.exists(), "prefetch destination already exists")
        destination.parent.mkdir(parents=True, exist_ok=True)
        current = url
        for hop in range(self.limits["redirect_hops"] + 1):
            parsed = urllib.parse.urlsplit(current)
            require(parsed.scheme == "https" and parsed.hostname in hosts and parsed.port in (None, 443) and
                    not parsed.username and not parsed.password and not parsed.fragment,
                    "undeclared fetch origin/redirect")
            require(time.monotonic() < self.deadline, "combined download deadline")
            request = urllib.request.Request(current, headers={"Accept-Encoding": "identity"})
            record = {"kind": kind, "url": current, "hop": hop, "payload_before": self.counts[kind],
                      "payload_bytes": 0, "status": None, "error": None, "finalization_errors": [],
                      "complete": False}
            self.ledger.append(record)
            response, stream, primary, final_error = None, None, None, None
            redirect = None
            h = hashlib.sha256()
            partial = destination.with_name(destination.name + f".hop-{hop}.partial")
            record["partial_path"] = str(partial)
            phase = "request"
            try:
                try:
                    response = self.opener.open(request, timeout=min(
                        self.limits["request_seconds"], self.deadline - time.monotonic()))
                except urllib.error.HTTPError as error:
                    response = error
                require(response.geturl() == current, "fetcher followed redirect implicitly")
                code = response.status if hasattr(response, "status") else response.code
                record["status"] = code
                record["encoding"] = response.headers.get("Content-Encoding", "identity")
                length = response.headers.get("Content-Length")
                require(length is None or re.fullmatch(r"\d+", length), "invalid Content-Length")
                declared = int(length) if length is not None else None
                record["content_length"] = declared
                remaining = self.caps[kind] - self.counts[kind]
                body_cap = min(remaining, file_limit) if code == 200 and file_limit is not None else remaining
                require(body_cap >= 0 and (declared is None or declared <= body_cap),
                        "Content-Length exceeds remaining payload budget")
                phase = "body"
                stream = partial.open("xb")
                while True:
                    require(time.monotonic() < self.deadline, "combined download deadline during body")
                    maximum = min(self.limits["read_bytes"], body_cap + 1 - record["payload_bytes"])
                    failed_read = None
                    try:
                        raw = response.read1(maximum)
                    except http.client.IncompleteRead as error:
                        raw, failed_read = error.partial, error
                    if not raw:
                        if failed_read is not None:
                            raise failed_read
                        break
                    self.counts[kind] += len(raw)
                    record["payload_bytes"] += len(raw)
                    h.update(raw)
                    stream.write(raw)
                    require(record["payload_bytes"] <= body_cap, "fetched payload overflow; partial retained")
                    if failed_read is not None:
                        raise failed_read
                record["body_sha256"] = h.hexdigest()
                phase = "response-validation"
                require(declared is None or declared == record["payload_bytes"], "truncated HTTP body")
                require(record["encoding"] in ("", "identity"), "encoded response refused after counted raw capture")
                if code in (301, 302, 303, 307, 308):
                    location = response.headers.get("Location")
                    require(location and hop < self.limits["redirect_hops"], "redirect missing or hop bound")
                    redirect = urllib.parse.urljoin(current, location)
                    record["redirect"] = redirect
                else:
                    require(code == 200, "non-success fetch response")
            except BaseException as error:
                primary = error
                record["error"] = failure_record(error, phase)
            finally:
                record["received_body_sha256"] = h.hexdigest()
                record["payload_after"] = self.counts[kind]
                record["payload_counters_after"] = self.counts.copy()
                for name, resource_to_close in (("body-file-close", stream), ("response-close", response)):
                    if resource_to_close is not None:
                        try:
                            resource_to_close.close()
                        except BaseException as error:
                            record["finalization_errors"].append(failure_record(error, name))
                            if final_error is None:
                                final_error = error
                record["retained_path"] = str(partial)
                record["retained_disposition"] = "partial-not-accepted"
                try:
                    record["partial_file_bytes"] = partial.stat().st_size
                except FileNotFoundError:
                    record["partial_file_bytes"] = None
                except BaseException as error:
                    record["partial_file_bytes"] = None
                    record["finalization_errors"].append(failure_record(error, "partial-file-stat"))
                    if final_error is None:
                        final_error = error
                try:
                    require(len(encoded(self.ledger)) <= self.limits["other_artifact_file_bytes"],
                            "fetch ledger artifact bound")
                except BaseException as error:
                    record["finalization_errors"].append(failure_record(error, "ledger-bound"))
                    if final_error is None:
                        final_error = error
            if primary is not None:
                raise primary
            if final_error is not None:
                record["error"] = record["finalization_errors"][0]
                raise final_error
            if redirect is not None:
                record["complete"] = True
                record["retained_disposition"] = "counted-redirect-body"
                current = redirect
                continue
            try:
                partial.rename(destination)
            except BaseException as error:
                record["error"] = failure_record(error, "payload-rename")
                raise
            record.update(complete=True, retained_path=str(destination), retained_disposition="downloaded")
            return {"path": str(destination), "bytes": record["payload_bytes"],
                    "sha256": record["body_sha256"]}
        raise Error("redirect loop bound")


def request_plan(go_sum):
    requests, seen = [], set()
    escape = lambda value: re.sub(r"[A-Z]", lambda m: "!" + m[0].lower(), value)
    for line in go_sum.decode("utf-8").splitlines():
        fields = line.split()
        require(len(fields) == 3 and re.fullmatch(r"h1:[A-Za-z0-9+/]+=*", fields[2]), "invalid frozen go.sum")
        module, version, checksum = fields
        require(module and all(part not in ("", ".", "..") for part in module.split("/")) and
                re.fullmatch(r"[A-Za-z0-9._~+/-]+", module), "invalid module path")
        mod = version.endswith("/go.mod")
        version = version[:-7] if mod else version
        require(re.fullmatch(r"v[0-9A-Za-z.+-]+", version), "invalid exact module version")
        for extension in (("mod",) if mod else ("zip", "info")):
            relative = escape(module) + "/@v/" + escape(version) + "." + extension
            require(relative not in seen, "duplicate module request")
            seen.add(relative)
            requests.append({"module": module, "version": version, "kind": extension,
                "relative_path": relative, "url": "https://proxy.golang.org/" + relative,
                "go_sum": None if extension == "info" else checksum})
    return sorted(requests, key=lambda r: r["relative_path"])


def unpack(archive, destination, root_name, maximum, deadline):
    total, seen = 0, set()
    with tarfile.open(archive, "r:gz") as source:
        for member in source:
            require(time.monotonic() < deadline, "archive extraction deadline")
            parts = Path(member.name).parts
            require(parts and parts[0] == root_name and not Path(member.name).is_absolute() and
                    not any(p in (".", "..") for p in parts) and "\\" not in member.name,
                    "archive path escape")
            relative = Path(*parts[1:]) if len(parts) > 1 else Path(".")
            target = destination / relative
            require(member.name not in seen, "duplicate archive member")
            seen.add(member.name)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            require(member.isfile(), "archive links/special files prohibited")
            total += member.size
            require(total <= maximum, "expanded archive bound")
            target.parent.mkdir(parents=True, exist_ok=True)
            with source.extractfile(member) as src, target.open("xb") as dst:
                remaining = member.size
                while remaining:
                    require(time.monotonic() < deadline, "archive copy deadline")
                    raw = src.read(min(65536, remaining))
                    require(raw, "truncated archive member")
                    dst.write(raw)
                    remaining -= len(raw)
            target.chmod(0o755 if member.mode & 0o111 else 0o644)
    return total


def verify_tree(root, manifest, patched, candidate_hash, deadline):
    expected = {f["path"]: f for f in manifest["files"]}
    found = {str(f.relative_to(root)).replace(os.sep, "/"): f for f in root.rglob("*") if not f.is_dir()}
    require(set(found) == set(expected), "upstream tree file set changed")
    for name, file in found.items():
        require(time.monotonic() < deadline and stat.S_ISREG(file.lstat().st_mode), "source file type/deadline")
        raw = file.read_bytes()
        if patched and name == "internal/utils/macos.go":
            require(digest(raw) == candidate_hash, "patched GetBuildInfo hash mismatch")
        else:
            identity = hashlib.sha1(b"blob " + str(len(raw)).encode() + b"\0" + raw).hexdigest()
            require(identity == expected[name]["sha"] and len(raw) == expected[name]["bytes"],
                    "upstream Git blob mismatch: " + name)
        mode = "100755" if file.stat().st_mode & 0o111 else "100644"
        require(mode == expected[name]["mode"], "upstream mode mismatch")


def tree_identity(files):
    root = {}
    for item in files:
        parts = item["path"].split("/")
        require(all(p not in ("", ".", "..") for p in parts), "invalid frozen Git path")
        node = root
        for directory in parts[:-1]:
            require(directory not in node or isinstance(node[directory], dict), "Git file/directory collision")
            node = node.setdefault(directory, {})
        require(parts[-1] not in node, "duplicate frozen Git path")
        node[parts[-1]] = (item["mode"], item["sha"])
    def encode(node):
        entries = []
        for name, value in node.items():
            directory = isinstance(value, dict)
            mode, sha = ("40000", encode(value)) if directory else value
            entries.append(((name + ("/" if directory else "")).encode(),
                            mode.encode() + b" " + name.encode() + b"\0" + bytes.fromhex(sha)))
        raw = b"".join(value for _, value in sorted(entries))
        return hashlib.sha1(b"tree " + str(len(raw)).encode() + b"\0" + raw).hexdigest()
    return encode(root)


def volume_accounting(backing_info, mount_fs, backing_fs, limits):
    logical = backing_info.st_size
    allocated = backing_info.st_blocks * 512
    capacity = mount_fs.f_blocks * mount_fs.f_frsize
    available = backing_fs.f_bavail * backing_fs.f_frsize
    unprovisioned = max(0, logical - allocated)
    require(0 < logical <= limits["module_and_build_disk_bytes"] and
            0 < capacity <= limits["module_and_build_disk_bytes"], "fixed volume logical capacity bound")
    required = limits["disk_reserve_bytes"] + unprovisioned
    record = {"backing_logical_bytes": logical, "backing_allocated_blocks": backing_info.st_blocks,
              "backing_allocated_bytes": allocated, "volume_logical_capacity": capacity,
              "backing_user_available": available, "unprovisioned": unprovisioned,
              "required_initial_available": required, "initial_reserve_satisfied": available >= required,
              "not_live_floor_or_all_host_writes_bound": True}
    return record


def build_environment(bounds, volume):
    fixed = bounds["environment"]
    expected = {**fixed, "SDKROOT": bounds["compiler_identity"]["SDKROOT"], "CC": bounds["tools"]["clang"],
                "CXX": bounds["tools"]["clangxx_nominal"]}
    env = {**expected, "PATH": str(volume / "go" / "bin") + ":" +
           str(Path(bounds["tools"]["clang"]).parent) + ":/usr/bin:/bin"}
    for key, name in (("HOME", "home"), ("TMPDIR", "tmp"), ("GOTMPDIR", "tmp"), ("GOCACHE", "gocache"),
                      ("GOMODCACHE", "gomodcache"), ("GOPATH", "gopath")):
        env[key] = str(volume / name)
    env.update(GOPROXY=(volume / "proxy").as_uri(), TELEX_HOME=str(volume / "telex-home"),
               TELEX_DB=str(volume / "telex.sqlite"), TELEX_INSTALL_ROOT=str(volume / "telex-install"))
    return env


class Runner:
    def __init__(self, bounds, work, evidence, deadline):
        self.bounds, self.work, self.evidence = bounds, work, evidence
        self.deadline, self.cleanup_deadline = deadline - 60, deadline
        self.records, self.logs = [], []
        self.file_limit_configurations = []
        self.metadata_seconds = 0
        self.phase_deadline = self.deadline
        self.log_root = evidence.root

    def command(self, label, argv, env, budget="metadata", timeout=None):
        require(isinstance(env, dict), "explicit child environment dictionary required")
        limits = self.bounds["limits"]
        require(len(self.records) < limits["maximum_commands"], "command count bound")
        requested = [str(a) for a in argv]
        require(Path(requested[0]).is_absolute() and Path(requested[0]).name != "ipsw",
                "absolute non-ipsw tool required; never execute decoder")
        started = time.monotonic()
        absolute_stop = min(self.deadline, self.phase_deadline, self.cleanup_deadline)
        if budget == "cleanup":
            absolute_stop = min(started + 60, self.cleanup_deadline)
        if budget == "metadata":
            absolute_stop = min(absolute_stop, started + limits["metadata_signing_seconds_combined"] -
                                self.metadata_seconds)
        require(absolute_stop > started, "command aggregate time bound")
        require(timeout is None or timeout > 0, "phase deadline before command")
        stop = min(absolute_stop, started + (timeout if timeout is not None else 120))
        index = len(self.records)
        file_limit = (limits["compile_file_bytes"] if label == "compile" else
                      limits["binary_bytes"] if label == "ad-hoc-sign" else None)
        configured_limits = {
            "command_index": index, "label": label,
            "compile_file_bytes": limits["compile_file_bytes"], "binary_bytes": limits["binary_bytes"],
            "RLIMIT_FSIZE": None if file_limit is None else {"soft_bytes": file_limit, "hard_bytes": file_limit},
            "basis": "Configured child preexec request only; no kernel or nested-child limit readback.",
        }
        self.file_limit_configurations.append(configured_limits)
        record = {"label": label, "argv": requested, "pid": None, "rc": None, "errors": [],
                  "reaped": False, "reap_state": "not-started", "wait_attempted": False,
                  "group_kill_before_reap": False, "group_signal_attempted": False,
                  "signal_authority_sealed": False, "streams": {}, "complete": False,
                  "command_deadline": stop, "controller_deadline": self.cleanup_deadline,
                  "configured_file_limits": configured_limits}
        self.records.append(record)
        data, eof, process, poller = {"stdout": bytearray(), "stderr": bytearray()}, set(), None, None
        can_signal, interrupted, phase = False, None, "spawn"

        def seal(reason):
            nonlocal can_signal
            can_signal = False
            if not record["signal_authority_sealed"]:
                record["signal_authority_sealed"] = True
                record["signal_seal_reason"] = reason

        def note(error, where):
            nonlocal interrupted
            record["errors"].append(failure_record(error, where))
            if not isinstance(error, Exception):
                seal("interruption")
                if interrupted is None:
                    interrupted = error

        def wait_owned(wait_timeout, where):
            seal("before-" + where)
            authorized_at = time.monotonic()
            wait_deadline = self.cleanup_deadline if where == "cleanup-wait" else min(stop, self.cleanup_deadline)
            wait_timeout = min(wait_timeout, wait_deadline - authorized_at)
            require(wait_timeout > 0, "absolute deadline exhausted before wait")
            record["wait_attempted"] = True
            record["reap_state"] = "wait-outcome-indeterminate"
            record.setdefault("waits", []).append({"phase": where, "timeout": wait_timeout,
                                                  "authorized_at": authorized_at, "deadline": wait_deadline})
            record["rc"] = process.wait(timeout=wait_timeout)
            record["reaped"] = True
            record["reap_state"] = "confirmed-by-wait-return"

        try:
            def file_bound():
                resource.setrlimit(resource.RLIMIT_FSIZE, (file_limit, file_limit))
            process = subprocess.Popen(requested.copy(), cwd=self.work, env=env, stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
                preexec_fn=file_bound if file_limit is not None else None)
            can_signal = True
            record["pid"] = process.pid
            record["reap_state"] = "owned-unwaited"
            require(process.args == requested, "tool argv changed")
            phase = "capture"
            poller = selectors.DefaultSelector()
            for pipe, name in ((process.stdout, "stdout"), (process.stderr, "stderr")):
                poller.register(pipe, selectors.EVENT_READ, name)
            while poller.get_map():
                require(time.monotonic() < stop, "command/EOF deadline")
                for key, _ in poller.select(min(0.1, max(0, stop - time.monotonic()))):
                    name = key.data
                    raw = os.read(key.fileobj.fileno(), min(65536, limits["per_command_stream_bytes"] + 1 - len(data[name])))
                    if not raw:
                        eof.add(name)
                        poller.unregister(key.fileobj)
                    else:
                        data[name].extend(raw)
                        require(len(data[name]) <= limits["per_command_stream_bytes"], "command stream cap")
            phase = "command-wait"
            remaining = min(stop, self.cleanup_deadline) - time.monotonic()
            require(remaining > 0, "command wait deadline")
            wait_owned(min(limits["command_wait_seconds"], remaining), phase)
        except BaseException as error:
            note(error, phase)
        finally:
            if process is not None and not record["reaped"]:
                if can_signal:
                    seal("before-group-signal")
                    record["group_signal_attempted"] = True
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                        record["group_kill_before_reap"] = True
                    except BaseException as error:
                        note(error, "owned-group-signal")
                if record["wait_attempted"]:
                    record["cleanup_wait_skipped"] = "prior-wait-may-have-consumed-anchor"
                else:
                    remaining = self.cleanup_deadline - time.monotonic()
                    if remaining <= 0:
                        seal("controller-deadline")
                        record["cleanup_wait_skipped"] = "controller-deadline-exhausted"
                        note(Error("controller deadline exhausted; no cleanup wait authorized"), "cleanup-wait")
                    else:
                        try:
                            wait_owned(min(limits["cleanup_wait_seconds"], remaining), "cleanup-wait")
                        except BaseException as error:
                            note(error, "cleanup-wait")
            seal("command-finalization")
            if process is not None:
                record["process_returncode_observed"] = getattr(process, "returncode", None)
            endpoints = [("selector-close", poller)]
            if process is not None:
                endpoints.extend((("stdout-close", process.stdout), ("stderr-close", process.stderr)))
            for name, endpoint in endpoints:
                if endpoint is not None:
                    try:
                        endpoint.close()
                    except BaseException as error:
                        note(error, name)
            record["seconds"] = time.monotonic() - started
            if budget == "metadata":
                self.metadata_seconds += record["seconds"]
            for name, raw in data.items():
                file = self.log_root / f"cmd-{index:02d}-{name}.bin"
                stream_record = {"file": file.name, "bytes": len(raw), "sha256": digest(raw),
                                 "eof": name in eof, "persisted": False}
                record["streams"][name] = stream_record
                target, write_failed = None, False
                try:
                    target = file.open("xb")
                    self.logs.append(file)
                    target.write(raw)
                except BaseException as error:
                    write_failed = True
                    note(error, name + "-log-write")
                finally:
                    if target is not None:
                        try:
                            target.close()
                        except BaseException as error:
                            write_failed = True
                            note(error, name + "-log-close")
                stream_record["persisted"] = not write_failed
            record["complete"] = eof == {"stdout", "stderr"} and record["reaped"] and not record["errors"]
        if interrupted is not None:
            raise interrupted
        require(record["complete"] and record["rc"] == 0, "command failed; no retry: " + label)
        return bytes(data["stdout"]), bytes(data["stderr"])

@contextmanager
def metadata_phase(runner):
    start, prior = time.monotonic(), runner.metadata_seconds
    old_deadline = runner.phase_deadline
    runner.phase_deadline = min(runner.deadline, start +
        runner.bounds["limits"]["metadata_signing_seconds_combined"] - prior)
    primary = None
    try:
        require(start < runner.phase_deadline, "metadata/signing aggregate deadline")
        yield
    except BaseException as error:
        primary = error
        raise
    finally:
        runner.metadata_seconds = prior + time.monotonic() - start
        expired = time.monotonic() >= runner.phase_deadline
        runner.phase_deadline = old_deadline
        if primary is None:
            require(not expired, "metadata/signing aggregate deadline")


def bind_tools(bounds, runner, env, provenance, suffix):
    records = {}
    provenance["tool_binding_" + suffix] = records
    remaining = bounds["limits"]["tool_hash_bytes"]
    deadline = min(runner.deadline, runner.phase_deadline)
    for key in ("hdiutil", "git", "codesign", "xcrun", "clang", "linker"):
        file = Path(bounds["tools"][key])
        resolved = str(file.resolve(strict=True))
        records[key] = {"declared": str(file), "resolved": resolved}
        require(resolved == str(file), "unapproved tool alias: " + key)
        records[key]["identity"] = file_hash(file, remaining, deadline)
        remaining -= records[key]["identity"]["bytes"]
    nominal, canonical = bounds["tools"]["clangxx_nominal"], bounds["tools"]["clang"]
    resolved = str(Path(nominal).resolve(strict=True))
    records["clangxx_relation"] = {"nominal": nominal, "resolved": resolved, "expected": canonical}
    require(resolved == canonical, "unapproved CXX alias")
    sdk = bounds["compiler_identity"]["SDKROOT"]
    sdk_resolved = str(Path(sdk).resolve(strict=True))
    records["SDK_relation"] = {"path": sdk, "resolved": sdk_resolved}
    require(sdk_resolved == sdk, "unapproved SDK alias")
    for pathname in bounds["compiler_identity"]["metadata"]:
        records[pathname] = file_hash(Path(pathname), remaining, deadline)
        remaining -= records[pathname]["bytes"]
    if suffix == "before":
        stdout, stderr = runner.command("clang-version", [canonical, "--version"], env)
        require(b"Apple clang version " in stdout, "unexpected compiler version output")
        provenance["compiler_version"] = {"stdout": stdout.decode("utf-8"), "stderr": stderr.decode("utf-8")}
        version = runner.command("SDK-version", [bounds["tools"]["xcrun"], "--sdk", "macosx", "--show-sdk-version"], env)
        require(version == (bounds["compiler_identity"]["SDK_version"].encode() + b"\n", b""), "SDK version mismatch")
    else:
        require(records == provenance["tool_binding_before"], "build tools/SDK changed")


def validate_go_environment(observed, environment):
    # Go1.27.3 cfg.EnvFile/MkEnv report an empty file path for the unchanged GOENV=off input.
    expected = {
        "GOTOOLCHAIN": environment["GOTOOLCHAIN"], "GOENV": "", "GOWORK": environment["GOWORK"],
        "GOOS": environment["GOOS"], "GOARCH": environment["GOARCH"], "GOARM64": environment["GOARM64"],
        "CGO_ENABLED": environment["CGO_ENABLED"], "GOFLAGS": environment["GOFLAGS"],
        "GOPROXY": environment["GOPROXY"], "GOSUMDB": environment["GOSUMDB"], "GOVCS": environment["GOVCS"],
        "GOPRIVATE": environment["GOPRIVATE"], "GONOPROXY": environment["GONOPROXY"],
        "GONOSUMDB": environment["GONOSUMDB"], "GOINSECURE": environment["GOINSECURE"],
        "GOMODCACHE": environment["GOMODCACHE"], "GOCACHE": environment["GOCACHE"],
        "GOPATH": environment["GOPATH"], "CC": environment["CC"], "CXX": environment["CXX"],
    }
    require(isinstance(observed, dict), "effective Go environment must be an object")
    for key, value in expected.items():
        require(key in observed and observed[key] == value, "effective Go environment mismatch: " + key)
    return expected


def macho_signature(file, maximum, deadline, signed=False, identifier=None, release_values=None):
    identity = file_hash(file, maximum, deadline)
    with file.open("rb") as stream:
        raw = stream.read(maximum + 1)
    require(time.monotonic() < deadline and len(raw) <= maximum and len(raw) == identity["bytes"] and
            digest(raw) == identity["sha256"], "bounded Mach-O bytes changed after hashing")
    require(len(raw) >= 32, "truncated Mach-O")
    magic, cpu, subtype, filetype, ncmds, size, flags, reserved = struct.unpack_from("<8I", raw)
    require(magic == 0xFEEDFACF and cpu == 0x100000C and filetype == 2 and ncmds <= 1024 and 32 + size <= len(raw),
            "derived output must be thin native ARM64 Mach-O executable")
    offset, signatures = 32, []
    for _ in range(ncmds):
        cmd, length = struct.unpack_from("<2I", raw, offset)
        require(length >= 8 and offset + length <= 32 + size, "invalid Mach-O command")
        if cmd == 0x1D:
            require(length == 16, "invalid signature load command")
            start, count = struct.unpack_from("<2I", raw, offset + 8)
            require(start + count <= len(raw), "signature outside file")
            signatures.append(raw[start:start + count])
        offset += length
    require(offset == 32 + size and len(signatures) <= 1, "ambiguous Mach-O signature")
    record = {**identity, "cpu": cpu, "subtype": subtype, "signature_present": bool(signatures)}
    if release_values is not None:
        require(set(release_values) == {"AppVersion", "AppBuildCommit"}, "both release values required")
        matches = {}
        for name, value in release_values.items():
            require(isinstance(value, str) and value, "empty or invalid release identity value")
            needle = value.encode("utf-8")
            first = raw.find(needle)
            require(first >= 0, "complete release identity bytes absent: " + name)
            matches[name] = {"value": value, "utf8_bytes": len(needle), "first_file_offset": first}
        record["release_identity"] = {
            "matches": matches, "scope": "Both complete UTF-8 value byte sequences in the bounded hashed file; no decoder execution.",
            "not_claimed": "Runtime variable lookup, uniqueness or whole-binary equivalence to the upstream release."}
    if signed:
        require(len(signatures) == 1 and len(signatures[0]) >= 12, "missing explicit signature")
        sig = signatures[0]
        magic, length, count = struct.unpack_from(">3I", sig)
        require(magic == 0xFADE0CC0 and 12 + count * 8 <= length <= len(sig), "invalid signature superblob")
        directories = []
        for i in range(count):
            slot, start = struct.unpack_from(">2I", sig, 12 + 8 * i)
            require(slot not in (5, 7) and start + 8 <= length, "entitlements prohibited or bad signature slot")
            kind, child_length = struct.unpack_from(">2I", sig, start)
            require(child_length >= 8 and start + child_length <= length, "invalid signature child")
            if kind == 0xFADE0C02:
                require(child_length >= 24, "short CodeDirectory")
                code_flags, ident_offset = struct.unpack_from(">I", sig, start + 12)[0], struct.unpack_from(">I", sig, start + 20)[0]
                require(code_flags & 2 and not code_flags & 0x10000 and ident_offset < child_length,
                        "signature not ad-hoc or hardened runtime present")
                close = sig.find(b"\0", start + ident_offset, start + child_length)
                require(close >= 0 and sig[start + ident_offset:close].decode("utf-8") == identifier,
                        "signature identifier mismatch")
                directories.append(code_flags)
            if slot == 0x10000:
                require(child_length == 8, "identity/CMS signature prohibited")
        require(directories, "missing ad-hoc CodeDirectory")
        record["code_directory_flags"] = directories
    return record


def go_metadata(raw, bounds):
    text = raw.decode("utf-8")
    lines = text.splitlines()
    require(lines and lines[0].rsplit(": ", 1)[-1] == bounds["go"]["version"], "static Go toolchain mismatch")
    settings, paths, modules = {}, [], []
    for line in lines[1:]:
        fields = line.strip().split("\t")
        require(fields and fields[0] in ("path", "mod", "dep", "=>", "build"), "unexpected static Go metadata")
        if fields[0] == "path":
            require(len(fields) == 2, "invalid Go package path record")
            paths.append(fields[1])
        elif fields[0] == "build":
            require(len(fields) == 2 and "=" in fields[1], "invalid Go build setting")
            key, value = fields[1].split("=", 1)
            require(key not in settings, "duplicate Go build setting")
            settings[key] = json.loads(value) if value.startswith('"') else value
        else:
            modules.append(fields)
    require(paths == ["github.com/blacktop/ipsw/cmd/ipsw"], "static Go package path mismatch")
    # Pinned Go1.27.3 load/pkg.go omits this setting under -trimpath; presence is a mismatch.
    require("-ldflags" not in settings, "static Go metadata must omit -ldflags under -trimpath")
    # Pinned Go1.27.3 load/pkg.go:2492-2493 records tags only for nonempty BuildTags.
    require("-tags" not in settings, "static Go metadata must omit -tags for the no-tags build")
    expected = {"GOOS": "darwin", "GOARCH": "arm64", "CGO_ENABLED": "1",
                "-trimpath": "true", "-buildmode": "exe"}
    require(all(settings.get(k) == v for k, v in expected.items()), "static Go build setting mismatch")
    return {"toolchain": bounds["go"]["version"], "path": paths[0], "settings": settings, "modules": modules}


def archive_bytes(files, maximum, deadline):
    class BoundedBytes(io.BytesIO):
        def write(self, raw):
            require(time.monotonic() < deadline and self.tell() + len(raw) <= maximum,
                    "artifact archive time/byte cap")
            return super().write(raw)
    class TimedInput:
        def __init__(self, source):
            self.source = source
        def read(self, size):
            require(time.monotonic() < deadline, "artifact archive input deadline")
            return self.source.read(size)
    stream = BoundedBytes()
    with tarfile.open(fileobj=stream, mode="w:gz") as archive:
        for name, file in files:
            require(time.monotonic() < deadline and file.is_file() and not file.is_symlink(), "artifact archive input")
            info = tarfile.TarInfo(name)
            info.size, info.mode = file.stat().st_size, 0o644
            info.uid, info.gid, info.uname, info.gname, info.mtime = 0, 0, "", "", 0
            with file.open("rb") as source:
                archive.addfile(info, TimedInput(source))
            require(stream.tell() <= maximum, "artifact archive cap")
    raw = stream.getvalue()
    require(len(raw) <= maximum, "artifact archive cap")
    return raw


def execute(bounds, output, work, provenance, runner, ledger):
    provenance["inherited_input_names"] = controller_inputs(os.environ)
    limits = bounds["limits"]
    manifest, source_hash = closure(bounds)
    provenance.update(source_manifest=manifest, source_manifest_sha256=source_hash, bounds=bounds,
                      source_sha=os.environ.get("GITHUB_SHA"), outside_writers=bounds["volume"]["outside_writers"])
    output.write("source-closure.tar.gz", archive_bytes(
        [(r["file"], ROOT / r["file"]) for r in manifest], limits["other_artifact_file_bytes"], runner.deadline))
    sha = check_gate(bounds, digest((HERE / "bounds.json").read_bytes()), source_hash,
                    output, time.monotonic() + limits["gate_seconds"])
    require(sys.platform == "darwin" and os.getuid() != 0, "ordinary-user macOS build required")
    mount, image = work / bounds["volume"]["mount_name"], work / bounds["volume"]["image_name"]
    provenance.update(image=str(image), mount=str(mount), attached=False, attach_attempted=False, detached=False)
    env = build_environment(bounds, mount)
    control_env = {**env, "HOME": str(work), "TMPDIR": str(work)}
    for label, argv, expected in (
        ("host-uname", ["/usr/bin/uname", "-a"], None),
        ("host-product", ["/usr/bin/sw_vers", "-productVersion"], bounds["runner"]["product"]),
        ("host-build", ["/usr/bin/sw_vers", "-buildVersion"], bounds["runner"]["build"]),
        ("source", [bounds["tools"]["git"], "-C", str(ROOT), "rev-parse", "HEAD"], sha)):
        stdout, stderr = runner.command(label, argv, control_env)
        require(not stderr and (stdout.startswith(b"Darwin ") if expected is None else
                stdout == expected.encode() + b"\n"), "host/source baseline mismatch")
    require(os.uname().machine == bounds["runner"]["architecture"], "build host must be ARM64")
    require(os.environ.get("ImageVersion"), "missing runner ImageVersion")
    provenance["host"] = {**bounds["runner"], "ImageVersion": os.environ["ImageVersion"]}
    with metadata_phase(runner):
        bind_tools(bounds, runner, control_env, provenance, "before")
    mount.mkdir()
    runner.command("image-create", [bounds["tools"]["hdiutil"], *[
        str(image) if a == "{image}" else a for a in bounds["volume"]["create"]]], control_env, "setup", 120)
    provenance["attach_attempted"] = True
    attached, _ = runner.command("image-attach", [bounds["tools"]["hdiutil"], *[
        str(image) if a == "{image}" else str(mount) if a == "{mount}" else a
        for a in bounds["volume"]["attach"]]], control_env, "setup", 120)
    details = plistlib.loads(attached)
    require(any(row.get("mount-point") == str(mount) for row in details.get("system-entities", [])), "fixed mount not observed")
    provenance["attached"] = True
    require(mount.stat().st_dev != work.stat().st_dev, "image not mounted at fixed point")
    volume = volume_accounting(image.stat(), os.statvfs(mount), os.statvfs(image.parent), limits)
    provenance["volume_initial"] = volume
    require(volume["initial_reserve_satisfied"], "initial provisioned reserve failed; no fetch")
    for name in ("logs", "home", "tmp", "gocache", "gomodcache", "gopath", "proxy", "output", "signing"):
        (mount / name).mkdir()
    runner.log_root = mount / "logs"
    runner.work = mount
    provenance["build_environment"] = env
    deadline = min(runner.deadline, time.monotonic() + limits["all_downloads_seconds_combined"])
    fetcher = Fetcher(limits, deadline, ledger)
    source_archive = mount / "source.tar.gz"
    provenance["source_archive"] = fetcher.fetch(bounds["upstream"]["url"], source_archive, "source",
                                                  bounds["upstream"]["redirect_hosts"])
    source = mount / "source"
    unpack(source_archive, source, bounds["upstream"]["archive_root"], limits["source_expanded_bytes"], deadline)
    tree = json.loads((HERE / "inputs" / "source-tree.json").read_bytes())
    require(tree["commit"] == bounds["upstream"]["commit"] and
            tree_identity(tree["files"]) == tree["tree"] == bounds["upstream"]["tree"], "frozen Git tree mismatch")
    verify_tree(source, tree, False, bounds["upstream"]["candidate_macos_sha256"], runner.deadline)
    provenance["source_tree_before"] = {"commit": tree["commit"], "tree": tree["tree"], "all_blobs_match": True}
    plan = json.loads((HERE / bounds["module_fetch"]["plan"]).read_bytes())
    require(plan["requests"] == request_plan((source / "go.sum").read_bytes()), "frozen request plan is not exact go.sum derivation")
    import_report = static_imports.check_tree(source, tree, plan["requests"],
                                              limits["source_expanded_bytes"], runner.deadline)
    output.json("import-closure.json", import_report)
    provenance["static_import_closure"] = {"status": import_report["status"], "errors": import_report["errors"],
                                          "report": "import-closure.json", "compilation_proven": False}
    require(import_report["status"] == "static-closure-only" and not import_report["errors"],
            "static import closure rejected; no compiler fetch or build")
    patch = (HERE / "get-build-info.patch").read_bytes()
    require(len(patch) == bounds["upstream"]["patch_bytes"] and digest(patch) == bounds["upstream"]["patch_sha256"],
            "exact patch identity")
    patch_file = mount / "get-build-info.patch"
    patch_file.write_bytes(patch)
    runner.command("patch-check", [bounds["tools"]["git"], "-C", str(source), "apply", "--check",
                                  "--whitespace=error-all", str(patch_file)], env)
    runner.command("patch-apply", [bounds["tools"]["git"], "-C", str(source), "apply",
                                  "--whitespace=error-all", str(patch_file)], env)
    verify_tree(source, tree, True, bounds["upstream"]["candidate_macos_sha256"], runner.deadline)
    provenance["only_GetBuildInfo_changed"] = True
    go_archive = mount / "go.tar.gz"
    fetched = fetcher.fetch(bounds["go"]["url"], go_archive, "compiler", bounds["go"]["redirect_hosts"], bounds["go"]["bytes"])
    require(fetched["bytes"] == bounds["go"]["bytes"] and fetched["sha256"] == bounds["go"]["sha256"], "Go archive pin mismatch")
    provenance["go_archive"] = fetched
    unpack(go_archive, mount / "go", "go", bounds["go"]["extracted_bytes_max"], deadline)
    go = mount / "go" / "bin" / "go"
    provenance["go_executable_before"] = file_hash(go, limits["binary_bytes"], runner.deadline)
    sum_raw = (source / "go.sum").read_bytes()
    require(digest(sum_raw) == bounds["upstream"]["go_sum_sha256"] and
            digest((source / "go.mod").read_bytes()) == bounds["upstream"]["go_mod_sha256"], "module graph mismatch")
    for item in plan["requests"]:
        destination = mount / "proxy" / item["relative_path"]
        fetcher.fetch(item["url"], destination, "module", bounds["module_fetch"]["redirect_hosts"])
        if item["kind"] == "info":
            require(destination.stat().st_size <= limits["other_artifact_file_bytes"], "module info size bound")
            require(json.loads(destination.read_bytes()).get("Version") == item["version"], "module info version mismatch")
    provenance["fetch_payload_bytes"] = fetcher.counts.copy()
    compile_deadline = min(runner.deadline, time.monotonic() + limits["compilation_seconds"])
    require(runner.command("go-version", [str(go), "version"], env)[0] ==
            b"go version go1.27.3 darwin/arm64\n", "Go compiler version mismatch")
    go_keys = ("GOTOOLCHAIN", "GOENV", "GOWORK", "GOOS", "GOARCH", "GOARM64", "CGO_ENABLED",
                "GOFLAGS", "GOPROXY", "GOSUMDB", "GOVCS", "GOPRIVATE", "GONOPROXY", "GONOSUMDB", "GOINSECURE",
                "GOMODCACHE", "GOCACHE", "GOPATH", "CC", "CXX")
    stdout, _ = runner.command("go-env", [str(go), "env", "-json", *go_keys], env)
    observed = json.loads(stdout)
    provenance["go_environment_expected"] = validate_go_environment(observed, env)
    provenance["go_environment"] = observed
    runner.work = source
    def modules_unchanged():
        for name, key in (("go.mod", "go_mod_sha256"), ("go.sum", "go_sum_sha256")):
            require(digest((source / name).read_bytes()) == bounds["upstream"][key], "module files changed")
    for label, arguments in (("modules", ["mod", "download", "-json"]), ("modules-verify-before", ["mod", "verify"])):
        runner.command(label, [str(go), *arguments], env, "compile", compile_deadline - time.monotonic())
        modules_unchanged()
    binary = mount / "output" / "ipsw"
    arguments = [str(go), "build", *[a.replace("{output}", str(mount / "output")) for a in bounds["build"]["flags"]]]
    runner.command("compile", arguments, env, "compile", compile_deadline - time.monotonic())
    produced = binary.lstat()
    require(stat.S_ISREG(produced.st_mode) and produced.st_size <= limits["binary_bytes"],
            "produced ipsw type/size bound immediately after compile")
    modules_unchanged()
    runner.command("modules-verify-after", [str(go), "mod", "verify"], env, "compile", compile_deadline - time.monotonic())
    modules_unchanged()
    verify_tree(source, tree, True, bounds["upstream"]["candidate_macos_sha256"], runner.deadline)
    signed = mount / "signing" / "ipsw"
    release_values = {"AppVersion": bounds["build"]["version"], "AppBuildCommit": bounds["build"]["commit_string"]}
    with metadata_phase(runner):
        provenance["pre_explicit_sign"] = macho_signature(binary, limits["binary_bytes"], runner.phase_deadline,
                                                         release_values=release_values)
        provenance["pre_sign_build_metadata"] = go_metadata(runner.command("go-metadata-pre", [str(go), "version", "-m", str(binary)], env)[0], bounds)
        shutil.copyfile(binary, signed)
        runner.command("ad-hoc-sign", [bounds["tools"]["codesign"], *[
            str(signed) if a == "{binary}" else a for a in bounds["signing"]["arguments"]]], env)
        runner.command("signature-verify", [bounds["tools"]["codesign"], "--verify", "--strict", "--verbose=4", str(signed)], env)
        display = runner.command("signature-display", [bounds["tools"]["codesign"], "--display", "--verbose=4", str(signed)], env)
        require(b"Signature=adhoc" in display[1] and all(
            line == b"Timestamp=none" for line in display[1].splitlines() if line.startswith(b"Timestamp=")),
            "unexpected signing identity/timestamp")
        entitlements = runner.command("signature-entitlements", [bounds["tools"]["codesign"], "--display", "--entitlements", ":-", str(signed)], env)
        require(not entitlements[0].strip(), "signature entitlements must be absent")
        provenance["post_sign"] = macho_signature(signed, limits["binary_bytes"], runner.phase_deadline,
                                                 True, bounds["signing"]["identifier"], release_values=release_values)
        provenance["post_sign_build_metadata"] = go_metadata(runner.command("go-metadata-post", [str(go), "version", "-m", str(signed)], env)[0], bounds)
        require(provenance["pre_sign_build_metadata"] == provenance["post_sign_build_metadata"],
                "static Go metadata changed during signing")
        provenance["go_executable_after"] = file_hash(go, limits["binary_bytes"], runner.phase_deadline)
        require(provenance["go_executable_before"] == provenance["go_executable_after"], "Go executable changed")
        bind_tools(bounds, runner, env, provenance, "after")
    artifact_deadline = min(runner.deadline, time.monotonic() + limits["artifact_preparation_seconds"])
    licenses = [("ipsw/LICENSE", source / "LICENSE"), ("Go/LICENSE", mount / "go" / "LICENSE")]
    for file in (mount / "gomodcache").rglob("*"):
        require(time.monotonic() < artifact_deadline, "license enumeration deadline")
        if file.is_file() and file.name.lower().startswith(("license", "copying", "notice")):
            licenses.append(("modules/" + str(file.relative_to(mount / "gomodcache")), file))
    output.write("licenses.tar.gz", archive_bytes(licenses, limits["other_artifact_file_bytes"], artifact_deadline))
    require(time.monotonic() < artifact_deadline, "artifact export deadline")
    output.write("ipsw", signed.read_bytes())
    require(digest((output.root / "ipsw").read_bytes()) == provenance["post_sign"]["sha256"], "export changed signed binary")
    provenance["artifact_deadline"] = artifact_deadline
    provenance["disposition"] = "built-and-signed-not-executed-pending-independent-review"


def main():
    input_names = controller_inputs(os.environ)
    bounds = json.loads((HERE / "bounds.json").read_bytes())
    require(len(sys.argv) == 3, "usage: build.py ABS_WORK ABS_EVIDENCE")
    work, evidence = Path(sys.argv[1]), Path(sys.argv[2])
    require(work.is_absolute() and evidence.is_absolute() and work != evidence and
            not work.is_relative_to(evidence) and not evidence.is_relative_to(work), "private disjoint paths")
    work.mkdir(parents=True, exist_ok=False)
    evidence.mkdir(parents=True, exist_ok=False)
    output = Evidence(evidence, bounds["limits"])
    runner = Runner(bounds, work, output, time.monotonic() + bounds["limits"]["controller_seconds"])
    provenance, ledger, code = {"disposition": "failed-return-to-design", "ipsw_executed": False,
                               "inherited_input_names": input_names,
                               "child_file_limit_configurations": runner.file_limit_configurations}, [], 1
    try:
        execute(bounds, output, work, provenance, runner, ledger)
        code = 0
    except BaseException as error:
        provenance["failure"] = failure_record(error, "execute")
    finally:
        provenance["fetch_payload_bytes"] = {
            kind: sum(row["payload_bytes"] for row in ledger if row.get("kind") == kind)
            for kind in ("source", "compiler", "module")}
        artifact_stop = provenance.get("artifact_deadline", min(runner.deadline,
            time.monotonic() + bounds["limits"]["artifact_preparation_seconds"]))
        for log in runner.logs:
            try:
                require(time.monotonic() < artifact_stop, "artifact log export deadline")
                if log.parent != evidence:
                    output.write(log.name, log.read_bytes())
            except BaseException as error:
                provenance.setdefault("log_export_errors", []).append({
                    "file": str(log), **failure_record(error, "log-export")})
                code = 1
        if provenance.get("attach_attempted"):
            try:
                runner.work, runner.log_root = work, evidence
                control_env = {"PATH": "/usr/bin:/bin", "HOME": str(work), "TMPDIR": str(work)}
                runner.command("image-detach", [bounds["tools"]["hdiutil"], "detach", provenance["mount"]],
                               control_env, "cleanup", 60)
                provenance["detached"] = True
            except BaseException as error:
                provenance["detach_error"] = failure_record(error, "detach")
                code = 1
        if provenance.get("detached"):
            image = Path(provenance["image"])
            provenance["backing_image_retained_on_runner"] = str(image)
            provenance["retention_precision"] = (
                "The controller never unlinks the detached backing image, on success or failure. "
                "Inputs/logs remain on the runner until runner teardown; no bulk image is uploaded. "
                "This retention survives acceptance, required-record and manifest-finalization failures.")
            try:
                provenance["backing_after"] = {"logical": image.stat().st_size, "allocated_blocks": image.stat().st_blocks,
                    "user_available_sample_only": os.statvfs(work).f_bavail * os.statvfs(work).f_frsize}
            except BaseException as error:
                provenance["backing_finalization_error"] = failure_record(error, "backing-finalization")
                code = 1
            if code != 0:
                provenance["failed_backing_image_retained_on_runner"] = str(image)
        if code == 0:
            try:
                require(time.monotonic() < artifact_stop and time.monotonic() < runner.cleanup_deadline,
                        "artifact/controller deadline before acceptance")
            except BaseException as error:
                provenance["acceptance_error"] = failure_record(error, "acceptance")
                code = 1
        output_error, output_errors = None, []
        output_states = {name: "not-attempted" for name in
                         ("commands.json", "fetches.json", "provenance.json", "sha256.json")}

        def note_output_error(error, filename, phase):
            nonlocal output_error, code
            record = {"file": filename, **failure_record(error, phase)}
            output_errors.append(record)
            output_states[filename] = "write-failed-persistence-unconfirmed"
            code = 1
            if output_error is None:
                output_error = error
            return record

        for filename, value in (("commands.json", runner.records), ("fetches.json", ledger)):
            try:
                output.json(filename, value)
                output_states[filename] = "write-returned"
            except BaseException as error:
                provenance.setdefault("required_output_errors", []).append(
                    note_output_error(error, filename, "required-output"))
        if code != 0:
            provenance["disposition"] = "failed-return-to-design"
        provenance["output_binary_accepted"] = False
        provenance["finalization_candidate"] = code == 0
        provenance["finalization_contract"] = (
            "This immutable provenance precedes manifest finalization and does not certify success. "
            "Collection succeeds only if main returns 0 after finish and the exact artifact manifest verifies; "
            "the output binary still requires independent review.")
        try:
            output.json("provenance.json", provenance)
            output_states["provenance.json"] = "write-returned"
        except BaseException as error:
            note_output_error(error, "provenance.json", "provenance-write")
        if output_error is None:
            try:
                output.finish()
                output_states["sha256.json"] = "write-returned"
            except BaseException as error:
                note_output_error(error, "sha256.json", "manifest-finish")
        if output_error is not None:
            failure = {"disposition": "failed-return-to-design",
                       "output_binary_accepted": False,
                       "error": output_errors[0],
                       "secondary_errors": output_errors[1:],
                       "required_output_states": output_states,
                       "write_state_precision": (
                           "write-returned records a successful call, not fsync durability; "
                           "a failed write may have partial bytes and its persistence is unconfirmed. "
                           "An attempted provenance write is not persisted evidence."),
                       "backing_image_retained_on_runner": provenance.get("backing_image_retained_on_runner"),
                       "detached": provenance.get("detached", False),
                       "controller_unlinks_backing": False,
                       "manifest_not_acceptance": True}
            try:
                output.json("finalization-error.json", failure)
            except BaseException as receipt_error:
                output_error.add_note("Finalization failure receipt also failed: " + json.dumps({
                    "file": "finalization-error.json", **failure_record(receipt_error, "failure-receipt-write")},
                    sort_keys=True))
            for secondary in output_errors[1:]:
                output_error.add_note("Secondary required-output error: " + json.dumps(secondary, sort_keys=True))
            raise output_error
    return code


if __name__ == "__main__":
    sys.exit(main())
