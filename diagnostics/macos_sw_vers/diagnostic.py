"""Separately gated diagnostic. Importing performs no provider query or subprocess."""

import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import stat
import subprocess
import sys
import time
import urllib.request


COMPONENT = "macos-sw-vers-diagnostic-v1"
MARKER = re.compile(r"<!-- macos-sw-vers-diagnostic-v1 (\{[^\r\n]*\}) -->")


class DiagnosticError(RuntimeError):
    pass


def require(condition, reason):
    if not condition:
        raise DiagnosticError(reason)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def encoded(value):
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode("utf-8")


def error_record(error, phase):
    return {"phase": phase, "type": type(error).__name__, "message": str(error)}


class Output:
    def __init__(self, root, limits):
        self.root, self.limits = root, limits

    def write(self, name, raw):
        require(Path(name).name == name, "flat output name required")
        require(len(raw) <= self.limits["per_file_bytes"], "diagnostic per-file bound")
        files = list(self.root.iterdir())
        require(all(stat.S_ISREG(p.lstat().st_mode) for p in files), "nonregular diagnostic output")
        require(not (self.root / name).exists(), "diagnostic receipt must not be overwritten")
        require(len(files) + 1 <= self.limits["artifact_files"] and
                sum(p.stat().st_size for p in files) + len(raw) <= self.limits["artifact_bytes"],
                "diagnostic aggregate output bound")
        with (self.root / name).open("xb") as target:
            target.write(raw)
        return {"file": name, "bytes": len(raw), "sha256": digest(raw)}

    def json(self, name, value):
        return self.write(name, encoded(value))

    def finish(self):
        files = sorted(self.root.iterdir())
        require(all(stat.S_ISREG(p.lstat().st_mode) for p in files), "nonregular diagnostic output")
        self.json("sha256.json", {p.name: digest(p.read_bytes()) for p in files})


def source_manifest(checkout, bounds):
    records = []
    for relative in bounds["source_files"]:
        file = checkout / relative
        require(file.is_file() and not file.is_symlink(), "missing diagnostic source closure")
        raw = file.read_bytes()
        records.append({"file": relative, "bytes": len(raw), "sha256": digest(raw)})
    return records, digest(encoded(records))


def gate_context(environment, bounds, bounds_hash, manifest_hash, job):
    require(environment.get("GITHUB_EVENT_NAME") == "push", "diagnostic requires push event")
    require(environment.get("GITHUB_REF") == "refs/heads/" + bounds["branch"], "diagnostic branch mismatch")
    require(environment.get("GITHUB_REPOSITORY") == bounds["repository"], "diagnostic repository mismatch")
    require(environment.get("GITHUB_RUN_ATTEMPT") == "1", "diagnostic attempt1 only; no retries")
    require(environment.get("GITHUB_WORKFLOW_REF") ==
            bounds["repository"] + "/" + bounds["workflow"] + "@refs/heads/" + bounds["branch"],
            "diagnostic workflow identity mismatch")
    sha = environment.get("GITHUB_SHA", "")
    require(re.fullmatch(r"[0-9a-f]{40}", sha) is not None, "invalid diagnostic source SHA")
    require(job in bounds["jobs"], "unknown diagnostic context job")
    return {"component": COMPONENT, "sha": sha, "bounds_sha256": bounds_hash,
            "source_manifest_sha256": manifest_hash, "jobs": bounds["jobs"], "attempt": 1,
            "allow_launch": True}


def validate_freeze(comments, expected, bounds):
    candidates = []
    for comment in comments:
        if comment.get("user", {}).get("login") != bounds["freeze_author"]:
            continue
        for match in MARKER.finditer(comment.get("body", "")):
            record = json.loads(match[1])
            require(isinstance(record, dict), "malformed diagnostic marker")
            if record.get("component") == COMPONENT and record.get("sha") == expected["sha"]:
                candidates.append((comment, record))
    require(len(candidates) == 1, "missing/duplicate exact diagnostic freeze")
    comment, record = candidates[0]
    require(all(type(record.get(k)) is type(v) and record[k] == v for k, v in expected.items()) and
            record.get("allow_launch") is True,
            "diagnostic freeze source/bounds/closure/jobs/attempt mismatch")
    require(re.fullmatch(r"https://github\.com/lossyrob/telex/issues/172#issuecomment-\d+",
                         comment.get("html_url", "")) is not None, "diagnostic freeze comment identity")
    require(re.fullmatch(r"https://github\.com/lossyrob/telex/issues/172#issuecomment-\d+",
                         record.get("source_review", "")) is not None, "missing diagnostic source review")
    require(re.fullmatch(r"[0-9a-f]{64}", record.get("source_review_body_sha256", "")) is not None,
            "missing exact diagnostic source-review digest")
    return comment, record


class RefuseRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        fp.close()
        raise DiagnosticError("diagnostic HTTP redirect refused before forwarding: " + str(code))


def fetch_json(endpoint, token, limits, deadline):
    remaining = deadline - time.monotonic()
    require(remaining > 0, "diagnostic gate deadline")
    request = urllib.request.Request("https://api.github.com/repos/lossyrob/telex/" + endpoint,
        headers={"Authorization": "Bearer " + token, "Accept": "application/vnd.github+json"})
    opener = urllib.request.build_opener(RefuseRedirects())
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


def check_gate(expected, bounds, environment, output, deadline):
    comments = []
    for page in range(1, bounds["limits"]["github_comment_pages"] + 1):
        batch = fetch_json(f"issues/172/comments?per_page=100&page={page}",
                           environment["GH_TOKEN"], bounds["limits"], deadline)
        require(isinstance(batch, list), "invalid diagnostic comment page")
        comments.extend(batch)
        if len(batch) < 100:
            break
    else:
        raise DiagnosticError("diagnostic comment-page bound")
    comment, frozen = validate_freeze(comments, expected, bounds)
    identifier = frozen["source_review"].rsplit("-", 1)[1]
    review = fetch_json("issues/comments/" + identifier, environment["GH_TOKEN"], bounds["limits"], deadline)
    require(review.get("user", {}).get("login") == bounds["freeze_author"] and
            review.get("html_url") == frozen["source_review"] and
            review.get("issue_url") == "https://api.github.com/repos/lossyrob/telex/issues/172" and
            digest(review.get("body", "").encode()) == frozen["source_review_body_sha256"],
            "diagnostic source-review identity mismatch")
    output.json("gate-evidence.json", {"freeze_url": comment["html_url"],
        "freeze_body": comment["body"], "freeze_body_sha256": digest(comment["body"].encode()),
        "source_review_url": review["html_url"], "source_review_body": review["body"],
        "source_review_body_sha256": digest(review["body"].encode()),
        "expected": expected, "provider_query_performed": False,
        "scope": "Exact record verification; owner/reviewer supply semantic approval. No full-component freeze/retry reuse."})


def checked_environment(inherited, bounds, work):
    require(not any(bounds["reject_environment_key_substring"] in key or
                    key in bounds["reject_environment_keys"] for key in inherited),
            "unapproved loader/search/insertion override")
    environment = dict(inherited)
    for key, value in bounds["environment"].items():
        require(not environment.get(key) or environment[key] == value, "developer/SDK environment override")
        environment[key] = value
    environment.update(TELEX_HOME=str(work / "home"), TELEX_DB=str(work / "store.sqlite"),
                       TELEX_INSTALL_ROOT=str(work / "install"), TMPDIR=str(work))
    return environment


def decoder_environment(checked, bounds):
    require(not any("DYLD" in key or key in bounds["reject_environment_keys"] for key in checked),
            "bound decoder environment override")
    require(all(checked.get(k) == v for k, v in bounds["environment"].items()),
            "bound developer/SDK environment changed")
    result = checked.copy()
    result.update(bounds["decoder_environment_additions"])
    return result


class Capture:
    def __init__(self, work, output, limits, deadline):
        self.work, self.output, self.limits, self.deadline = work, output, limits, deadline
        self.commands = 0

    def command(self, name, argv, environment):
        require(self.commands < self.limits["maximum_commands"], "diagnostic command count bound")
        require(time.monotonic() < self.deadline, "diagnostic total deadline before launch")
        self.commands += 1
        requested = [str(a) for a in argv]
        require(requested and Path(requested[0]).is_absolute(), "absolute diagnostic executable required")
        self.output.json(name + "-request.json", {"requested_argv": requested,
            "stream_mode": "separate owned pipes; not Go CombinedOutput", "no_executed_image_attestation": True})
        started = time.monotonic()
        stop = min(self.deadline, started + self.limits["command_seconds"])
        streams = {"stdout": bytearray(), "stderr": bytearray()}
        observed = {"stdout": 0, "stderr": 0}
        eof, errors = set(), []
        process, poller, code, recorded = None, None, None, None
        reaped, kill_sent, phase = False, False, "spawn"
        try:
            process = subprocess.Popen(requested.copy(), cwd=self.work, env=environment,
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            recorded = process.args
            require(recorded == requested, "recorded diagnostic argv mismatch")
            phase = "pipe-capture"
            poller = selectors.DefaultSelector()
            for stream, label in ((process.stdout, "stdout"), (process.stderr, "stderr")):
                poller.register(stream, selectors.EVENT_READ, label)
            while poller.get_map():
                require(time.monotonic() < stop, "diagnostic stream/EOF deadline")
                for key, _ in poller.select(min(self.limits["select_seconds"], max(0, stop - time.monotonic()))):
                    label = key.data
                    remaining = self.limits["per_stream_bytes"] + 1 - len(streams[label])
                    raw = os.read(key.fileobj.fileno(), min(self.limits["read_chunk_bytes"], remaining))
                    if not raw:
                        eof.add(label)
                        poller.unregister(key.fileobj)
                        continue
                    observed[label] += len(raw)
                    streams[label].extend(raw)
                    require(len(streams[label]) <= self.limits["per_stream_bytes"],
                            "diagnostic stream bound exceeded; prefix retained")
            phase = "exact-wait"
            require(time.monotonic() < stop, "diagnostic wait deadline")
            code = process.wait(timeout=min(self.limits["wait_seconds"], stop - time.monotonic()))
            reaped = True
        except Exception as error:
            errors.append(error_record(error, phase))
        finally:
            if process is not None and not reaped:
                try:
                    if process.returncode is None:
                        process.kill()
                        kill_sent = True
                except OSError as error:
                    errors.append(error_record(error, "owned-child-kill"))
                try:
                    code = process.wait(timeout=self.limits["kill_wait_seconds"])
                    reaped = True
                except (OSError, subprocess.TimeoutExpired) as error:
                    errors.append(error_record(error, "owned-child-reap"))
            if poller is not None:
                try:
                    poller.close()
                except OSError as error:
                    errors.append(error_record(error, "selector-close"))
            if process is not None:
                for stream in (process.stdout, process.stderr):
                    try:
                        stream.close()
                    except OSError as error:
                        errors.append(error_record(error, "pipe-close"))
        complete = eof == {"stdout", "stderr"} and not errors and reaped
        stream_records = {}
        for label, raw in streams.items():
            stream_records[label] = {**self.output.write(name + "-" + label + ".bin", bytes(raw)),
                "observed_bytes": observed[label], "eof": label in eof,
                "complete": complete, "retained_prefix_only": not complete}
        record = {"requested_argv": requested, "recorded_argv": recorded,
            "pid": process.pid if process is not None else None, "rc": code,
            "elapsed_seconds": time.monotonic() - started, "streams": stream_records,
            "streams_complete": complete, "errors": errors, "exact_child_reaped": reaped,
            "owned_child_kill_sent": kill_sent, "no_discovered_pid_or_group_signals": True,
            "no_executed_image_attestation": True}
        self.output.json(name + ".json", record)
        require(complete and code == 0 and recorded == requested, "failed diagnostic command; raw evidence retained")
        return bytes(streams["stdout"]), bytes(streams["stderr"])


def baseline(capture, bounds, environment, checkout, source_sha, output):
    require(sys.platform == bounds["host"]["system"] and os.getuid() != 0,
            "ordinary-user macOS diagnostic required")
    reports = {}
    for name, arguments in bounds["baseline_commands"].items():
        argv = [str(checkout) if arg == "{checkout}" else arg for arg in arguments]
        reports[name] = capture.command("baseline-" + name, argv, environment)
        if name == "uname":
            require(reports[name][0].startswith(b"Darwin ") and not reports[name][1],
                    "unexpected uname baseline")
        else:
            expected = {"architecture": bounds["host"]["architecture"], "product": bounds["host"]["product_version"],
                        "build": bounds["host"]["build"], "source": source_sha}[name]
            require(reports[name] == (expected.encode() + b"\n", b""), "diagnostic baseline mismatch: " + name)
    output.json("baseline.json", {"source_sha": source_sha, "host": bounds["host"],
        "checks": "exact unmodified output lines; source from fixed git argv",
        "uname_raw": "baseline-uname-stdout.bin", "python_executable": sys.executable, "python_version": sys.version,
        "image_os": environment.get("ImageOS"), "image_version": environment.get("ImageVersion"),
        "not_executed_image_or_identical_runner_content_attestation": True})


def execute(bounds, bounds_hash, checkout, work, output, job, inherited):
    start = time.monotonic()
    manifest, manifest_hash = source_manifest(checkout, bounds)
    for item in manifest:
        output.write("source-" + Path(item["file"]).name, (checkout / item["file"]).read_bytes())
    output.json("source-manifest.json", {"files": manifest, "sha256": manifest_hash,
        "import_closure": bounds["import_boundary"]})
    expected = gate_context(inherited, bounds, bounds_hash, manifest_hash, job)
    check_gate(expected, bounds, inherited, output, start + bounds["limits"]["gate_seconds"])
    environment = checked_environment(inherited, bounds, work)
    tracing = decoder_environment(environment, bounds)
    output.json("environment.json", {"fixed": bounds["environment"],
        "diagnostic_additions": bounds["decoder_environment_additions"],
        "other_loader_overrides": [], "remaining_environment": "inherited, values not exported",
        "isolated_paths": {key: environment[key] for key in ("TELEX_HOME", "TELEX_DB", "TELEX_INSTALL_ROOT", "TMPDIR")},
        "relation_to_parent": bounds["environment_relation"]})
    capture = Capture(work, output, bounds["limits"], start + bounds["limits"]["total_seconds"])
    baseline(capture, bounds, environment, checkout, expected["sha"], output)
    stdout, stderr = capture.command("decoder-env-product-version", bounds["diagnostic_command"], tracing)
    observation = {"standalone_command": bounds["diagnostic_command"],
        "stdout_exact_expected_product_line": stdout == bounds["host"]["product_version"].encode() + b"\n",
        "stderr_empty": not stderr, "version_acceptance_performed": False,
        "raw_stdout": "decoder-env-product-version-stdout.bin", "raw_stderr": "decoder-env-product-version-stderr.bin",
        "no_normalization_concatenation_or_semver_conversion": True,
        "not_ipsw_internal_buffer_or_Go_CombinedOutput": True,
        "cause": "not established; these are separate standalone child streams",
        "decoder_trace_policy_and_pin_changed": False, "new_native_binding": False}
    output.json("observation.json", observation)
    return {"disposition": "diagnostic-collected-not-version-acceptance", "job": job, "source_sha": expected["sha"],
            "version_acceptance_performed": False, "HOLD_unchanged": True}


def verify_output(root, bounds):
    files = sorted(root.iterdir())
    require(0 < len(files) <= bounds["limits"]["artifact_files"], "diagnostic artifact count")
    require(all(stat.S_ISREG(p.lstat().st_mode) and p.stat().st_size <= bounds["limits"]["per_file_bytes"]
                for p in files), "diagnostic artifact type/size")
    require(sum(p.stat().st_size for p in files) <= bounds["limits"]["artifact_bytes"], "diagnostic artifact bytes")
    manifest = json.loads((root / "sha256.json").read_bytes())
    require(set(manifest) == {p.name for p in files if p.name != "sha256.json"}, "diagnostic manifest set mismatch")
    require(all(digest((root / name).read_bytes()) == value for name, value in manifest.items()),
            "diagnostic manifest hash mismatch")


def main():
    here = Path(__file__).resolve().parent
    raw_bounds = (here / "bounds.json").read_bytes()
    bounds = json.loads(raw_bounds)
    if len(sys.argv) == 3 and sys.argv[1] == "--verify-output":
        verify_output(Path(sys.argv[2]), bounds)
        return 0
    require(len(sys.argv) == 4, "usage: diagnostic.py JOB ABS_WORK ABS_EVIDENCE")
    job, work, evidence = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
    require(work.is_absolute() and evidence.is_absolute() and work != evidence and
            not work.is_relative_to(evidence) and not evidence.is_relative_to(work), "isolated absolute sibling paths")
    work.mkdir(parents=True, exist_ok=False)
    evidence.mkdir(parents=True, exist_ok=False)
    output = Output(evidence, bounds["limits"])
    code = 1
    try:
        result = execute(bounds, digest(raw_bounds), here.parents[1], work, output, job, os.environ.copy())
        code = 0
    except Exception as error:
        result = {"disposition": "diagnostic-failed-no-version-acceptance",
                  "error": error_record(error, "diagnostic"), "HOLD_unchanged": True}
    output.json("result.json", result)
    output.finish()
    verify_output(evidence, bounds)
    return code


if __name__ == "__main__":
    sys.exit(main())
