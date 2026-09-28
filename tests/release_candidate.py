"""Exercise actual release archives, never the user's installation or database.

Run explicitly from the Release workflow or maintainer runbook, not ordinary
offline cargo tests. Only the genuine v0.1.2 baseline is fetched from GitHub.
"""

import argparse
from contextlib import closing
import hashlib
import http.server
import json
import os
from pathlib import Path
import platform
import shutil
import sqlite3
import subprocess
import tarfile
import tempfile
import threading
import time
import urllib.parse
import urllib.request
import uuid
import zipfile


BASELINE_TAG = "v0.1.2"
BASELINE_SHA = "636ecce360de80bbcbc0d18a61d6d6cdbbcf23f3"
BASELINE_RELEASE_ID = 359082164
REPO = "lossyrob/telex"
ROOT = Path(__file__).resolve().parent.parent


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def download(url, github_metadata=False):
    request = urllib.request.Request(url, headers={"User-Agent": "telex-release-proof"})
    if github_metadata:
        require(url in (
            f"https://api.github.com/repos/{REPO}/releases/tags/{BASELINE_TAG}",
            f"https://api.github.com/repos/{REPO}/git/ref/tags/{BASELINE_TAG}",
        ), "unexpected upstream metadata URL")
        token = os.environ.get("TELEX_PROOF_GITHUB_TOKEN")
        if token:
            # urllib does not copy unredirected headers to redirected requests.
            request.add_unredirected_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(request, timeout=90) as response:
        return response.read()


def unpack_binary(archive, destination, windows):
    name = "telex.exe" if windows else "telex"
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as package:
            data = package.read(name)
    else:
        with tarfile.open(archive, "r:gz") as package:
            member = package.getmember(name)
            require(member.isfile(), "binary archive member must be a regular file")
            with package.extractfile(member) as source:
                data = source.read()
    destination.mkdir(parents=True, exist_ok=True)
    executable = destination / name
    executable.write_bytes(data)
    executable.chmod(0o755)
    return executable.resolve()


def clean_environment(root):
    (root / "tmp").mkdir(parents=True, exist_ok=True)
    allowed = {
        "PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "SYSTEMDRIVE",
        "TEMP", "TMP", "USERNAME", "USER", "PROCESSOR_ARCHITECTURE",
        "NUMBER_OF_PROCESSORS", "PSMODULEPATH",
    }
    env = {k: v for k, v in os.environ.items() if k.upper() in allowed}
    env.update({
        "TELEX_HOME": str(root / "home"),
        "TELEX_CONFIG": str(root / "config.toml"),
        "TELEX_DB": str(root / "store.sqlite"),
        "TELEX_INSTALL_ROOT": str(root / "install"),
        "TELEX_RUN_DIR": str(root / "run"),
        "TELEX_SESSION_ID": "release-proof",
        "TELEX_LAUNCHER_ACTIVE": "1",
        "TELEX_NO_MODIFY_PATH": "1",
        "TEMP": str(root / "tmp"),
        "TMP": str(root / "tmp"),
        "TMPDIR": str(root / "tmp"),
        "LOCALAPPDATA": str(root / "state"),
        "APPDATA": str(root / "appdata"),
        "XDG_STATE_HOME": str(root / "state"),
        "HOME": str(root / "user"),
        "USERPROFILE": str(root / "user"),
    })
    return env


def read_fixture_identity(run_dir):
    caps = list(run_dir.glob("daemon-*.cap"))
    require(len(caps) <= 1, "multiple capability files in isolated fixture")
    if not caps:
        return None
    try:
        return json.loads(caps[0].read_text())
    except FileNotFoundError:
        # Windows replacement can briefly remove the previous publication.
        return None


def successor_identity(identity, pid, predecessor):
    if identity is None or identity == predecessor:
        return None
    require(identity.get("server_pid") == pid, "unexpected fixture daemon publication")
    require(identity.get("server_start_time") is not None, "missing daemon start identity")
    require(identity.get("instance_id"), "missing daemon instance identity")
    return identity


class CandidateServer:
    def __init__(self, archive, tag, target):
        extension = "zip" if target.endswith("windows-msvc") else "tar.gz"
        self.asset = f"telex-{tag}-{target}.{extension}"
        data = archive.read_bytes()
        prefix = f"/{REPO}/releases/download/{tag}/"
        release = json.dumps({
            "tag_name": tag, "draft": False, "prerelease": False,
            "assets": [{"name": self.asset}, {"name": self.asset + ".sha256"}],
        }).encode()
        self.routes = {
            f"/repos/{REPO}/releases/latest": release,
            f"/repos/{REPO}/releases/tags/{tag}": release,
            prefix + self.asset: data,
            prefix + self.asset + ".sha256": f"{digest(data)}  {self.asset}\n".encode(),
        }
        self.requests = []
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                owner.requests.append(self.path)
                body = owner.routes.get(self.path)
                self.send_response(200 if body is not None else 404)
                self.send_header("Content-Length", str(len(body or b"")))
                self.end_headers()
                self.wfile.write(body or b"")

            def log_message(self, *_args):
                pass

        self.http = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.http.serve_forever)
        self.url = f"http://127.0.0.1:{self.http.server_port}"

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_args):
        self.http.shutdown()
        self.http.server_close()
        self.thread.join(timeout=10)
        require(not self.thread.is_alive(), "candidate HTTP server did not join")

    def verify_requests(self, start, tag):
        observed = self.requests[start:]
        for path in (
            f"/repos/{REPO}/releases/latest",
            f"/{REPO}/releases/download/{tag}/{self.asset}",
            f"/{REPO}/releases/download/{tag}/{self.asset}.sha256",
        ):
            require(path in observed, f"release proof never requested {path}")


class Proof:
    def __init__(self, args, root, report):
        self.args, self.root, self.report = args, root, report
        self.windows = os.name == "nt"
        self.daemons = []
        self.schemas = []
        self.pg_env = None
        if args.postgres_url:
            url = urllib.parse.urlparse(args.postgres_url)
            require(url.scheme in ("postgres", "postgresql"), "use a PostgreSQL URI")
            require(url.hostname in ("127.0.0.1", "::1", "localhost"),
                    "PostgreSQL proof permits only an explicitly disposable loopback server")
            require(args.disposable_postgres, "--disposable-postgres acknowledgement required")
            require(shutil.which("psql"), "psql is required for PostgreSQL schema assertions")
            self.pg_env = clean_environment(root)
            self.pg_env.update({
                "PGHOST": url.hostname, "PGPORT": str(url.port or 5432),
                "PGUSER": urllib.parse.unquote(url.username or "postgres"),
                "PGPASSWORD": urllib.parse.unquote(url.password or ""),
                "PGDATABASE": url.path.lstrip("/") or "postgres",
                "PGSSLMODE": "disable",
            })

    def run(self, command, env, expected=0, timeout=45):
        command = [str(part) for part in command]
        started = time.monotonic()
        result = subprocess.run(command, env=env, cwd=self.root, stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, encoding="utf-8",
                                errors="replace", timeout=timeout)
        self.report["commands"].append({
            "argv": command, "exit_code": result.returncode,
            "seconds": round(time.monotonic() - started, 3),
            "stdout": result.stdout, "stderr": result.stderr,
        })
        require(result.returncode == expected,
                f"expected exit {expected}, got {result.returncode}: {command}\n"
                f"{result.stdout}\n{result.stderr}")
        return result

    def telex(self, binary, env, *args, expected=0, timeout=45):
        require(binary.is_absolute(), "proof must execute an absolute binary path")
        return self.run([binary, "--json", *args], env, expected, timeout=timeout)

    def metadata(self, binary, env, version, sha, minor, schema_max):
        value = json.loads(self.telex(binary, env, "version").stdout)
        require(value["version"]["package_version"] == version, "package version mismatch")
        require(value["version"]["build_id"] == sha, "executable build ID/source mismatch")
        require(value["version"]["supported_schema_min"] == 2, "schema minimum mismatch")
        require(value["version"]["supported_schema_max"] == schema_max, "schema maximum mismatch")
        require(value["daemon_metadata"]["protocol_version"] == {"major": 1, "minor": minor},
                "protocol mismatch")
        return value

    def acknowledge(self, binary, env, message_id, recipient):
        receipt = json.loads(self.telex(binary, env, "--address", recipient,
                                        "ack", "--id", str(message_id)).stdout)
        require(receipt.get("message_id") == int(message_id), "Ack message identity mismatch")
        require(receipt.get("recipient") == recipient, "Ack recipient identity mismatch")
        require(receipt.get("delivery_outcome") == "marked", "Ack did not mark first consumption")
        return receipt

    def environment(self, name, postgres=False):
        root = self.root / name
        root.mkdir()
        env = clean_environment(root)
        for name in ("home", "run", "state", "appdata", "user"):
            (root / name).mkdir()
        if postgres:
            schema = "telex_release_" + uuid.uuid4().hex
            self.schemas.append(schema)
            # Explicit fixture configuration; never load an operator profile or helper.
            Path(env["TELEX_CONFIG"]).write_text(
                'default = "proof"\n[backends.proof]\nkind = "postgres"\n'
                f'url = {json.dumps(self.args.postgres_url)}\n'
                f'schema = "{schema}"\nauth = "password"\n', encoding="utf-8")
        else:
            schema = None
        return env, schema

    def sql(self, env, schema, query):
        if schema:
            result = self.run(["psql", "-X", "-A", "-t", "-v", "ON_ERROR_STOP=1",
                               "-c", f'SET search_path TO "{schema}"; {query}'], self.pg_env)
            return result.stdout.strip().splitlines()[-1]
        with closing(sqlite3.connect(env["TELEX_DB"])) as db:
            return str(db.execute(query).fetchone()[0])

    def schema_version(self, env, schema, expected):
        actual = self.sql(env, schema, "SELECT MAX(version) FROM telex_schema_version")
        require(actual == str(expected), f"schema version {actual}, expected {expected}")

    def start_daemon(self, binary, env, minor):
        run_dir = Path(env["TELEX_RUN_DIR"])
        predecessor = read_fixture_identity(run_dir)
        if predecessor is not None:
            require(any(
                row["pid"] == predecessor.get("server_pid")
                and row["start_time"] == predecessor.get("server_start_time")
                and row["instance_id"] == predecessor.get("instance_id")
                for row in self.report["daemons"]
            ) and any(
                process.pid == predecessor.get("server_pid") and process.poll() is not None
                for process, _, _, _ in self.daemons
            ), "preexisting publication is not an exited owned fixture daemon")
        log = (self.root / f"daemon-{len(self.daemons)}.log").open("wb")
        process = subprocess.Popen([str(binary), "--json", "daemon", "serve"],
                                   env=env, cwd=self.root, stdin=subprocess.DEVNULL,
                                   stdout=log, stderr=log)
        self.daemons.append((process, binary, env, log))
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            require(process.poll() is None, f"fixture daemon exited: {binary}")
            identity = successor_identity(read_fixture_identity(run_dir), process.pid, predecessor)
            if identity is None:
                time.sleep(0.05)
                continue
            remaining = deadline - time.monotonic()
            require(remaining > 0, "fixture daemon readiness deadline expired")
            result = self.telex(binary, env, "daemon", "status", timeout=remaining)
            status = json.loads(result.stdout)
            if "instance_id" in status:
                require(status["protocol_version"] == {"major": 1, "minor": minor},
                        "live daemon protocol mismatch")
                require(status["instance_id"] == identity["instance_id"],
                        "authenticated status differs from owned daemon publication")
                require(time.monotonic() < deadline, "fixture daemon readiness deadline expired")
                self.report["daemons"].append({
                    "pid": process.pid, "binary": str(binary), "protocol_minor": minor,
                    "start_time": identity["server_start_time"],
                    "instance_id": status["instance_id"],
                })
                return process
            time.sleep(0.05)
        raise RuntimeError("fixture daemon readiness deadline expired")

    def stop_daemon(self, process, binary, env):
        self.telex(binary, env, "daemon", "stop", "--drain")
        require(process.wait(timeout=15) == 0, "fixture daemon did not exit cleanly")

    def negative_upgrades(self, old, env, server, predecessor):
        original = dict(server.routes)
        release_path = f"/repos/{REPO}/releases/latest"
        asset_path = f"/{REPO}/releases/download/{self.args.tag}/{server.asset}"
        release = json.loads(original[release_path])
        cases = {
            "wrong-checksum": {asset_path + ".sha256": b"0" * 64 + b"  candidate\n"},
            "missing-sidecar": {release_path: json.dumps({
                **release, "assets": [{"name": server.asset}]}).encode()},
            "missing-target": {release_path: json.dumps({**release, "assets": []}).encode()},
        }
        # Correctly checksummed old bytes under the new tag must still be refused.
        old_archive = next(self.root.glob(f"telex-{BASELINE_TAG}-*")).read_bytes()
        cases["tag-binary-mismatch"] = {
            asset_path: old_archive,
            asset_path + ".sha256": f"{digest(old_archive)}  {server.asset}\n".encode(),
        }
        for name, routes in cases.items():
            try:
                server.routes.update(routes)
                self.telex(old, env, "upgrade", expected=1)
                require(predecessor.poll() is None, f"{name} drained the predecessor")
                status = json.loads(self.telex(old, env, "daemon", "status").stdout)
                require(status["protocol_version"]["minor"] == 4, f"{name} changed daemon")
                install = Path(env["TELEX_INSTALL_ROOT"])
                require((install / "current").read_text().strip() == BASELINE_TAG,
                        f"{name} changed the selector")
                require(not (install / "previous").exists(), f"{name} changed previous")
                self.report["proofs"].append({
                    "scenario": name, "rejected_before_activation_and_drain": True,
                })
            finally:
                server.routes = dict(original)

    def upgrade(self, old, candidate, server, postgres=False):
        backend = "postgres" if postgres else "sqlite"
        env, schema = self.environment(backend + "-migration", postgres)
        install = Path(env["TELEX_INSTALL_ROOT"])
        self.telex(old, env, "upgrade", "--from", str(old), "--version", BASELINE_TAG,
                   "--root", str(install), "--skip-drain")
        old_installed = install / "versions" / BASELINE_TAG / old.name
        self.metadata(old_installed, env, "0.1.2", BASELINE_SHA, 4, 2)
        self.telex(old_installed, env, "init")
        self.schema_version(env, schema, 2)
        predecessor = self.start_daemon(old_installed, env, 4)
        self.telex(old_installed, env, "--address", "proof:sender", "attach",
                   "--session", "release-proof")
        sent = json.loads(self.telex(
            old_installed, env, "send", "--to", "proof:inbox",
            "--from", "proof:sender", "--body", "preserve-through-schema3",
            "--requires-disposition").stdout)
        message_id = str(sent["id"])
        original_message = json.loads(
            self.telex(old_installed, env, "read", "--id", message_id).stdout)["message"]
        env["TELEX_UPGRADE_API_BASE"] = server.url
        env["TELEX_UPGRADE_DOWNLOAD_BASE"] = server.url
        if not postgres:
            self.negative_upgrades(old_installed, env, server, predecessor)
        request_start = len(server.requests)
        result = json.loads(self.telex(old_installed, env, "upgrade").stdout)
        server.verify_requests(request_start, self.args.tag)
        require(result["release"]["verified"] is True, "old binary did not verify candidate")
        require(result["drain"]["drained"] is True, "old binary did not drain owning daemon")
        require(result["switch"]["switched_to"] == self.args.tag, "selector did not advance")
        require(result["switch"]["previous_tag"] == BASELINE_TAG, "previous selector missing")
        require(predecessor.wait(timeout=15) == 0, "old daemon not exited before replacement")
        current = install / "versions" / self.args.tag / candidate.name
        manifest = json.loads((current.parent / "manifest.json").read_text(encoding="utf-8"))
        require((manifest["schema_min"], manifest["schema_max"]) == (2, 3),
                "genuine old upgrader did not preserve candidate schema manifest")
        require(manifest["build_id"] == self.args.source_sha, "installed manifest source mismatch")
        self.metadata(current, env, self.args.tag[1:], self.args.source_sha, 5, 3)
        successor = self.start_daemon(current, env, 5)
        self.telex(current, env, "--address", "proof:inbox", "attach", "--session", "release-proof")
        self.schema_version(env, schema, 3)
        row = json.loads(self.telex(current, env, "read", "--id", message_id).stdout)
        require(row["message"] == original_message,
                "migration changed old message identity, routing, content, or timestamps")
        require(row["dispositions"] == [], "migration invented a disposition")
        delivered = self.telex(current, env, "--address", "proof:inbox",
                               "wait", "--session", "release-proof", "--timeout-ms", "3000")
        require("preserve-through-schema3" in delivered.stdout, "old delivery not recoverable")
        require(json.loads(self.telex(current, env, "read", "--id", message_id).stdout)
                ["dispositions"] == [], "delivery incorrectly created a disposition")
        ack = self.acknowledge(current, env, message_id, "proof:inbox")
        self.stop_daemon(successor, current, env)
        # An older executable must still reject the migrated store. No downgrade bypass.
        rejected = self.telex(old_installed, env, "init", expected=1)
        require("newer than supported" in rejected.stderr, "old schema guard missing")
        self.schema_version(env, schema, 3)
        rejected = self.telex(current, env, "rollback", expected=1)
        require("requires schema 3" in rejected.stderr, "rollback compatibility guard missing")
        require((install / "current").read_text().strip() == self.args.tag,
                "failed rollback changed current")
        self.report["proofs"].append({
            "backend": backend, "scenario": "released-schema2-upgrade",
            "old_daemon_exit_before_new_start": True, "message_id": message_id,
            "manifest": manifest, "requests": server.requests[request_start:],
            "schema": "2 -> 3", "old_store_and_rollback_guards": "rejected",
            "ack": ack,
        })

    def preexisting(self, old, candidate, server, postgres=False):
        backend = "postgres" if postgres else "sqlite"
        env, schema = self.environment(backend + "-preexisting", postgres)
        self.telex(candidate, env, "init")
        self.schema_version(env, schema, 3)
        seed_daemon = self.start_daemon(candidate, env, 5)
        self.telex(candidate, env, "--address", "proof:seed", "attach",
                   "--session", "release-proof")
        sent = json.loads(self.telex(candidate, env, "send", "--to", "proof:preexisting",
                                    "--from", "proof:seed",
                                    "--body", "existing-schema3-message").stdout)
        self.stop_daemon(seed_daemon, candidate, env)
        rejected = self.telex(old, env, "init", expected=1)
        require("newer than supported" in rejected.stderr, "baseline did not reject schema3")
        env.update(TELEX_UPGRADE_API_BASE=server.url, TELEX_UPGRADE_DOWNLOAD_BASE=server.url)
        result = json.loads(self.telex(old, env, "upgrade").stdout)
        require(result["release"]["verified"] is True, "baseline upgrade did not verify archive")
        current = Path(env["TELEX_INSTALL_ROOT"]) / "versions" / self.args.tag / candidate.name
        self.metadata(current, env, self.args.tag[1:], self.args.source_sha, 5, 3)
        self.telex(current, env, "init")
        self.schema_version(env, schema, 3)
        daemon = self.start_daemon(current, env, 5)
        self.telex(current, env, "--address", "proof:preexisting", "attach",
                   "--session", "release-proof")
        require("existing-schema3-message" in
                self.telex(current, env, "read", "--id", str(sent["id"])).stdout,
                "preexisting schema3 data changed")
        self.stop_daemon(daemon, current, env)
        self.report["proofs"].append({
            "backend": backend, "scenario": "preexisting-schema3",
            "baseline_rejected_store_but_release_upgrade_succeeded": True,
            "preserved_message_id": sent["id"],
        })

    def fresh_install(self, candidate, server):
        env, _ = self.environment("fresh-install")
        env.update(TELEX_UPGRADE_API_BASE=server.url, TELEX_UPGRADE_DOWNLOAD_BASE=server.url)
        before_requests = len(server.requests)
        if self.windows:
            # Observe the persistent PATH before/after; the installer opt-out must not touch it.
            script = (
                "$ErrorActionPreference='Stop'; "
                "$before=[Environment]::GetEnvironmentVariable('Path','User'); "
                f"& '{str(ROOT / 'install.ps1').replace(chr(39), chr(39) * 2)}'; "
                "if ([Environment]::GetEnvironmentVariable('Path','User') -cne $before) "
                "{ throw 'installer changed persistent user PATH' }"
            )
            result = self.run(["pwsh", "-NoProfile", "-NonInteractive", "-Command", script], env)
            require("User PATH unchanged" in result.stdout, "PATH opt-out did not execute")
        else:
            result = self.run(["sh", ROOT / "install.sh"], env)
        require("Checksum OK." in result.stdout, "fresh installer did not check the sidecar")
        server.verify_requests(before_requests, self.args.tag)
        installed = Path(env["TELEX_INSTALL_ROOT"]) / "versions" / self.args.tag / candidate.name
        self.metadata(installed, env, self.args.tag[1:], self.args.source_sha, 5, 3)
        launcher_env = dict(env)
        del launcher_env["TELEX_LAUNCHER_ACTIVE"]
        self.metadata(Path(env["TELEX_INSTALL_ROOT"]) / "bin" / candidate.name,
                      launcher_env, self.args.tag[1:], self.args.source_sha, 5, 3)
        self.report["proofs"].append({
            "scenario": "fresh-install.ps1" if self.windows else "fresh-install.sh",
            "checksum_verified": True, "persistent_user_path_modified": False,
            "requests": server.requests[before_requests:],
        })
        # Keep the script's existing best-effort missing-sidecar policy; only
        # checksum mismatch is asserted fail-closed here.
        bad_env, _ = self.environment("fresh-tampered")
        bad_env.update(TELEX_UPGRADE_API_BASE=server.url, TELEX_UPGRADE_DOWNLOAD_BASE=server.url)
        sidecar = f"/{REPO}/releases/download/{self.args.tag}/{server.asset}.sha256"
        original = server.routes[sidecar]
        try:
            server.routes[sidecar] = b"0" * 64 + b"  candidate\n"
            command = (["pwsh", "-NoProfile", "-NonInteractive", "-File", ROOT / "install.ps1"]
                       if self.windows else ["sh", ROOT / "install.sh"])
            rejected = self.run(command, bad_env, expected=1)
            require("checksum mismatch" in (rejected.stdout + rejected.stderr),
                    "tampered fresh install failed for an unrelated reason")
            require(not (Path(bad_env["TELEX_INSTALL_ROOT"]) / "current").exists(),
                    "tampered fresh install activated a binary")
            self.report["proofs"].append({"scenario": "fresh-install-tamper", "rejected": True})
        finally:
            server.routes[sidecar] = original

    def cleanup(self):
        failures = []
        for process, binary, env, log in reversed(self.daemons):
            try:
                if process.poll() is None:
                    self.stop_daemon(process, binary, env)
                self.report["cleanup"].append({"pid": process.pid, "exit_code": process.returncode})
            except (RuntimeError, subprocess.TimeoutExpired) as error:
                failures.append(str(error))
                # The Popen handle is the exact still-owned fixture, never a searched PID.
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
                self.report["cleanup"].append({"pid": process.pid, "fallback_kill": True})
            finally:
                log.close()
        for schema in self.schemas:
            try:
                self.run(["psql", "-X", "-v", "ON_ERROR_STOP=1", "-c",
                          f'DROP SCHEMA IF EXISTS "{schema}" CASCADE'], self.pg_env)
                self.report["cleanup"].append({"postgres_schema_dropped": schema})
            except (RuntimeError, subprocess.TimeoutExpired) as error:
                failures.append(str(error))
        require(not failures, "fixture cleanup failed: " + "; ".join(failures))


def finish_report(proof, root, report, output):
    errors = []
    try:
        if proof is not None:
            proof.cleanup()
    except (RuntimeError, OSError, subprocess.TimeoutExpired) as error:
        errors.append(error)

    try:
        for path in root.glob("daemon-*.log"):
            try:
                report.setdefault("daemon_logs", {})[path.name] = path.read_text(errors="replace")
            except OSError as error:
                errors.append(error)
    except OSError as error:
        errors.append(error)

    unproven = []
    if proof is not None:
        # Include processes that failed before publishing a readiness receipt.
        for process, _, _, _ in proof.daemons:
            try:
                if process.poll() is None:
                    unproven.append(process.pid)
            except OSError as error:
                unproven.append(process.pid)
                errors.append(error)
    disposition = {"fixture_root": str(root), "removed": False}
    if unproven:
        disposition.update(retained_reason="owned process exit unproven", owned_pids=unproven)
        errors.append(RuntimeError("fixture root retained: owned process exit unproven"))
    else:
        try:
            shutil.rmtree(root)
            require(not root.exists(), "fixture root remains after removal")
            disposition["removed"] = True
        except (RuntimeError, OSError) as error:
            disposition["retained_reason"] = str(error)
            errors.append(error)
    report["cleanup"].append(disposition)
    if errors:
        report["status"] = "failed"
        report["cleanup_error"] = "; ".join(str(error) for error in errors)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    # main is already re-raising a primary error when this field is present.
    if errors and "error" not in report:
        raise errors[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--tag", default="v0.2.0")
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--postgres-url")
    parser.add_argument("--disposable-postgres", action="store_true")
    args = parser.parse_args()
    args.archive = args.archive.resolve()
    args.report = args.report.resolve()
    report = {"source_sha": args.source_sha, "candidate_tag": args.tag,
              "target": args.target, "host": platform.platform(),
              "commands": [], "daemons": [], "proofs": [], "cleanup": [],
              "status": "failed"}
    # macOS's default temporary path can exceed the Unix socket path limit.
    root = Path(tempfile.mkdtemp(prefix="tr-", dir=None if os.name == "nt" else "/tmp")).resolve()
    proof = None
    try:
        proof = Proof(args, root, report)
        archive = args.archive.read_bytes()
        expected = Path(str(args.archive) + ".sha256").read_text().split()[0]
        require(digest(archive) == expected, "candidate archive checksum mismatch")
        report["candidate"] = {
            "archive": str(args.archive), "sha256": digest(archive),
            "sidecar_sha256": digest(Path(str(args.archive) + ".sha256").read_bytes()),
            "served_alias": f"telex-{args.tag}-{args.target}." +
                            ("zip" if proof.windows else "tar.gz"),
            "alias_bytes": "identical archive bytes; no tag or release published",
        }
        candidate = unpack_binary(args.archive, root / "candidate", proof.windows)
        env = clean_environment(root / "probe")
        proof.metadata(candidate, env, args.tag[1:], args.source_sha, 5, 3)
        report["candidate"]["binary_sha256"] = digest(candidate.read_bytes())
        release_url = f"https://api.github.com/repos/{REPO}/releases/tags/{BASELINE_TAG}"
        release = json.loads(download(release_url, github_metadata=True))
        ref = json.loads(download(
            f"https://api.github.com/repos/{REPO}/git/ref/tags/{BASELINE_TAG}",
            github_metadata=True))
        require(ref["object"]["type"] == "commit" and ref["object"]["sha"] == BASELINE_SHA,
                "released baseline tag identity moved")
        require(release["tag_name"] == BASELINE_TAG and not release["draft"]
                and not release["prerelease"], "baseline is not the genuine stable release")
        require(release["id"] == BASELINE_RELEASE_ID, "baseline release identity changed")
        extension = "zip" if proof.windows else "tar.gz"
        name = f"telex-{BASELINE_TAG}-{args.target}.{extension}"
        asset = next(a for a in release["assets"] if a["name"] == name)
        sidecar = next(a for a in release["assets"] if a["name"] == name + ".sha256")
        data = download(asset["browser_download_url"])
        checksum = download(sidecar["browser_download_url"])
        require(digest(data) == checksum.decode().split()[0], "released baseline checksum mismatch")
        require(asset["digest"] == "sha256:" + digest(data), "GitHub asset digest mismatch")
        require(sidecar["digest"] == "sha256:" + digest(checksum), "GitHub sidecar digest mismatch")
        baseline_archive = root / name
        baseline_archive.write_bytes(data)
        old = unpack_binary(baseline_archive, root / "baseline", proof.windows)
        proof.metadata(old, env, "0.1.2", BASELINE_SHA, 4, 2)
        report["baseline"] = {
            "release_url": release["html_url"], "release_id": release["id"],
            "asset_id": asset["id"], "asset_url": asset["browser_download_url"],
            "source_sha": BASELINE_SHA, "archive_sha256": digest(data),
            "binary_sha256": digest(old.read_bytes()), "sidecar_sha256": digest(checksum),
        }
        with CandidateServer(args.archive, args.tag, args.target) as server:
            proof.upgrade(old, candidate, server)
            proof.preexisting(old, candidate, server)
            proof.fresh_install(candidate, server)
            if args.postgres_url:
                proof.upgrade(old, candidate, server, postgres=True)
                proof.preexisting(old, candidate, server, postgres=True)
            report["requests"] = server.requests
        report["coverage"] = {
            "sqlite": "executed", "postgres": "executed" if args.postgres_url else "not executed",
            "credential_providers": "not exercised; separate required CI owns recovery/credential tests",
            "platform": args.target, "cross_build_live_ipc": "not claimed; ordered replacement only",
        }
        report["status"] = "passed"
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        finish_report(proof, root, report, args.report)
    print(f"Release proof passed: {args.report}")


if __name__ == "__main__":
    main()
