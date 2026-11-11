"""Windows x64 archive install and genuine source-built #222 registry upgrade.

The baseline is PR #228's merge, before #243's record extension and #245's
forward-only registry-v3 migration, marked backups and offline restore fences.
Only public CLIs create or mutate the fixtures. SQLite reads below verify that
the OLD executable really wrote historical records, rather than fabricating a
baseline using SQL. Nothing here promises binary-only rollback after migration.
All binaries, profiles, owners, daemons and installation roots are private.
"""

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import sqlite3
import subprocess
import sys
import time
from types import SimpleNamespace
import uuid
import zipfile

from release_candidate import (
    Proof, clean_environment, create_private_directory, read_fixture_identity, require,
)

if os.name == "nt":
    import ctypes
    from ctypes import wintypes


ROOT = Path(__file__).resolve().parent.parent
BASELINE_SHA = "7036e7103aae4416cf0a0e2030175dc901ece893"
TARGET = "x86_64-pc-windows-msvc"
EVENT_WATCH = "archive-event"
FAILURE_WATCH = "archive-failure"
RECIPIENT = "project:archive-proof/inbox"


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class OwnedDaemon:
    """Retain a handle to the exact daemon bootstrapped by this fixture's Watcher."""

    def __init__(self, identity, binary):
        self.pid = identity["server_pid"]
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        signatures = {
            "OpenProcess": ([wintypes.DWORD, wintypes.BOOL, wintypes.DWORD], wintypes.HANDLE),
            "GetProcessTimes": ([wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4,
                                wintypes.BOOL),
            "QueryFullProcessImageNameW": ([wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR,
                                           ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL),
            "WaitForSingleObject": ([wintypes.HANDLE, wintypes.DWORD], wintypes.DWORD),
            "GetExitCodeProcess": ([wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL),
            "TerminateProcess": ([wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
            "CloseHandle": ([wintypes.HANDLE], wintypes.BOOL),
        }
        for name, (arguments, result) in signatures.items():
            function = getattr(self.kernel, name)
            function.argtypes, function.restype = arguments, result
        self.handle = self.kernel.OpenProcess(0x00100000 | 0x1000 | 0x1, False, self.pid)
        require(self.handle, f"open owned daemon handle: {ctypes.WinError(ctypes.get_last_error())}")
        try:
            times = [wintypes.FILETIME() for _ in range(4)]
            require(self.kernel.GetProcessTimes(self.handle, *(ctypes.byref(t) for t in times)),
                    "read owned daemon creation time")
            creation = (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime
            require(creation == identity["server_start_time"], "owned daemon creation time mismatch")
            image, length = ctypes.create_unicode_buffer(32768), wintypes.DWORD(32768)
            require(self.kernel.QueryFullProcessImageNameW(self.handle, 0, image,
                                                           ctypes.byref(length)),
                    "read owned daemon image")
            require(Path(image.value).resolve() == binary.resolve(), "owned daemon image mismatch")
        except Exception:
            self.close()
            raise

    def poll(self):
        status = self.kernel.WaitForSingleObject(self.handle, 0)
        if status == 258:  # WAIT_TIMEOUT
            return None
        require(status == 0, "query owned daemon exit")
        code = wintypes.DWORD()
        require(self.kernel.GetExitCodeProcess(self.handle, ctypes.byref(code)),
                "read owned daemon exit code")
        return code.value

    def wait(self, timeout):
        require(self.kernel.WaitForSingleObject(self.handle, int(timeout * 1000)) == 0,
                "owned daemon exit deadline")
        return self.poll()

    def kill(self):
        require(self.kernel.TerminateProcess(self.handle, 1), "terminate exact owned daemon handle")

    def close(self):
        if self.handle:
            require(self.kernel.CloseHandle(self.handle), "close owned daemon handle")
            self.handle = None


def unpack_archive(archive, destination, target):
    require(target == TARGET, "Watcher archive proof requires native Windows x86_64")
    sidecar = archive.with_name(archive.name + ".sha256")
    fields = sidecar.read_text(encoding="ascii").strip().split()
    require(len(fields) == 2 and fields[1] == archive.name,
            "archive checksum sidecar must name the exact archive")
    require(fields[0].lower() == sha256(archive), "archive checksum mismatch")
    expected = {"telex.exe", "telex-watcher.exe", "LICENSE"}
    with zipfile.ZipFile(archive) as package:
        names = package.namelist()
        require(len(names) == len(set(names)) and set(names) == expected,
                "Windows x64 archive must contain only telex.exe, telex-watcher.exe and LICENSE")
        # Read fixed member names, never extract arbitrary archive paths or links.
        contents = {name: package.read(name) for name in expected}
    destination.mkdir()
    for name, content in contents.items():
        require(content, f"empty archive member: {name}")
        (destination / name).write_bytes(content)
    return destination / "telex.exe", destination / "telex-watcher.exe"


def registry_snapshot(path):
    with closing(sqlite3.connect(path.as_uri() + "?mode=ro", uri=True)) as db:
        return {
            "version": db.execute("SELECT version FROM watcher_v2_meta").fetchone()[0],
            "watches": {
                row[0]: json.loads(row[1])
                for row in db.execute("SELECT id,data FROM watcher_v2_watches")
            },
            "attempts": {
                row[0]: json.loads(row[1])
                for row in db.execute("SELECT id,data FROM watcher_v2_attempts")
            },
        }


def check_baseline(snapshot):
    require(snapshot["version"] == 2, "source baseline did not create registry v2")
    require(snapshot["watches"][EVENT_WATCH]["state"] == {"cursor": 1},
            "source baseline did not commit receipt-gated detector state")
    require(snapshot["attempts"], "source baseline did not retain attempts")
    require(all("stderrExcerpt" not in row for row in snapshot["attempts"].values()),
            "baseline already contains #243 excerpt fields")
    require(all("latestFailure" not in row for row in snapshot["watches"].values()),
            "baseline already contains #243 failure fields")
    require(any(row["watchId"] == FAILURE_WATCH and row["outcome"] == "failed"
                for row in snapshot["attempts"].values()),
            "source baseline did not record the historical failure")


def check_migration(before, after):
    require(after["version"] == 3, "archive runtime must migrate registry v2 to v3")
    for key, old in before["watches"].items():
        new = after["watches"][key]
        for field in ("state", "lastAllocatedEventSequence", "lastCommittedEventSequence",
                      "logicalStoreId", "pendingOperationId", "registrationRevision"):
            require(new[field] == old[field], f"migration changed {key}.{field}")
        require("latestFailure" in new and new["latestFailure"] is None,
                "historical failure association must be present and null")
    for key, old in before["attempts"].items():
        new = after["attempts"][key]
        require("stderrExcerpt" in new and new["stderrExcerpt"] is None,
                "historical stderr excerpt must be present and null")
        require(all(new[field] == value for field, value in old.items()),
                "migration changed historical attempt evidence")


class WatcherArchiveProof:
    def __init__(self, root, report):
        self.root, self.report = root, report
        self.cli = Proof(SimpleNamespace(postgres_url=None), root, report)
        self.hosts = []
        self.detector = (ROOT / "tests" / "watcher_archive_detector.py").resolve()
        self.pwsh = Path(shutil.which("pwsh") or "").resolve()
        require(self.pwsh.is_file(), "PowerShell 7.4+ is required")

    def environment(self, name):
        env, _ = self.cli.environment(name)
        # Both binaries use the same explicit backend and never discover an
        # operator's config, ambient credential or live installation.
        Path(env["TELEX_CONFIG"]).write_text(
            'default = "archive"\n[backends.archive]\nkind = "sqlite"\n'
            f'path = {json.dumps(env["TELEX_DB"])}\n', encoding="utf-8")
        return env

    def telex(self, binary, env, *args):
        return self.cli.telex(binary, env, *args)

    def install(self, source, env, tag, expected_sha):
        self.telex(source, env, "upgrade", "--from", source, "--version", tag,
                   "--root", env["TELEX_INSTALL_ROOT"], "--skip-drain")
        installed = Path(env["TELEX_INSTALL_ROOT"]) / "versions" / tag / "telex.exe"
        require(sha256(source) == sha256(installed), "installed Telex differs from supplied binary")
        metadata = json.loads(self.telex(installed, env, "version").stdout)
        require(metadata["version"]["build_id"] == expected_sha, "binary/source SHA mismatch")
        skill = self.cli.run([installed, "copilot", "skill"], env).stdout
        require("backends [sqlite, postgres]" in skill and "entra auth available" in skill,
                "Telex must be built with sqlite, postgres and entra")
        self.telex(installed, env, "init")
        return installed, metadata["daemon_metadata"]["protocol_version"]["minor"]

    def admitted_daemon(self, binary, env, minor):
        previous = {
            (row["pid"], row["start_time"]) for row in self.report["daemons"]
        }

        def published():
            try:
                identity = read_fixture_identity(Path(env["TELEX_RUN_DIR"]))
            except OSError as error:
                if (getattr(error, "operation", None) == "CreateFileW"
                        and getattr(error, "winerror", None) == 32):
                    return None
                raise
            if identity is None or (
                    identity.get("server_pid"), identity.get("server_start_time")) in previous:
                return None
            require(identity.get("bootstrap_admission_version") == 1,
                    "Watcher must bootstrap the daemon through InstalledCurrent admission")
            require(identity.get("instance_id"), "owned daemon lacks instance identity")
            return identity

        identity = self.until(published, "Watcher-bootstrapped daemon publication")
        process = OwnedDaemon(identity, binary)
        # Reuse the existing handle-based stop/kill/wait cleanup, not PID searches.
        self.cli.daemons.append((process, binary, env, process))
        status = json.loads(self.telex(binary, env, "daemon", "status").stdout)
        require(status["protocol_version"] == {"major": 1, "minor": minor},
                "live admitted daemon protocol mismatch")
        require(status["instance_id"] == identity["instance_id"],
                "authenticated status differs from admitted daemon publication")
        self.report["daemons"].append({
            "pid": process.pid, "binary": str(binary), "protocol_minor": minor,
            "start_time": identity["server_start_time"], "instance_id": identity["instance_id"],
            "bootstrap_admission_version": identity["bootstrap_admission_version"],
        })
        return process

    def bootstrap(self, helper, telex, watcher, env):
        parameters = {
            "Action": "create", "TelexExecutable": str(telex),
            "WatcherExecutable": str(watcher),
            "WatcherRegistry": str(Path(env["TELEX_HOME"]).parent / "registry.sqlite"),
            "InstallRoot": env["TELEX_INSTALL_ROOT"], "BackendProfile": "archive",
            "Sender": "service:watcher/archive-proof", "PollIntervalSeconds": 1,
            "EnvironmentNames": ["TELEX_HOME", "TELEX_CONFIG", "LOCALAPPDATA"],
        }
        bootstrap_env = {**env, "TELEX_ARCHIVE_HELPER": str(helper),
                         "TELEX_ARCHIVE_PARAMETERS": json.dumps(parameters)}
        self.cli.run([
            self.pwsh, "-NoProfile", "-NonInteractive", "-Command",
            "$ErrorActionPreference='Stop'; "
            "$p=$env:TELEX_ARCHIVE_PARAMETERS | ConvertFrom-Json -AsHashtable; "
            "try { & $env:TELEX_ARCHIVE_HELPER @p } "
            "catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }",
        ], bootstrap_env)
        companion = Path(env["TELEX_HOME"]) / "watcher" / "installation.ps1"
        for action in ("verify", "preflight"):
            self.cli.run([self.pwsh, "-NoProfile", "-NonInteractive", "-File",
                          companion, "-Action", action], env)

    def watcher(self, binary, env, *args, allow_unready=False):
        require(binary.is_absolute(), "Watcher binary must be absolute")
        argv = [binary, "--json", "--registry",
                Path(env["TELEX_HOME"]).parent / "registry.sqlite",
                "--install-root", env["TELEX_INSTALL_ROOT"], *args]
        if allow_unready:
            # Startup has only this owned child's private endpoint to query.
            result = subprocess.run([str(x) for x in argv], env=env, cwd=self.root,
                                    capture_output=True, text=True, encoding="utf-8", timeout=10)
        else:
            result = self.cli.run(argv, env)
        value = json.loads(result.stdout)
        if allow_unready and not value["ok"]:
            require(value["error"]["code"] == "runtime-not-running",
                    f"unexpected startup management failure: {value}")
            return None
        require(result.returncode == 0 and value["ok"], f"Watcher command failed: {value}")
        return value["result"]["value"]

    def offline(self, binary, env, registry, *args, expected=0):
        require(binary.is_absolute() and registry.is_absolute(), "offline paths must be absolute")
        result = self.cli.run([
            binary, "--json", "--registry", registry, "--install-root",
            env["TELEX_INSTALL_ROOT"], *args,
        ], env, expected=expected)
        envelope = json.loads(result.stdout)
        require(envelope["ok"] == (expected == 0), "offline command exit/envelope mismatch")
        return envelope, result

    def refuse_v3(self, old_watcher, env, registry):
        before = registry_snapshot(registry)
        require(before["version"] == 3, "refusal fixture must really be registry v3")
        digest = sha256(registry)
        envelope, result = self.offline(old_watcher, env, registry, "run", expected=1)
        # #222's old public surface exposes only this generic startup error.
        # It has no #225 stderr cause chain; do not invent one in the report.
        require(envelope["error"]["code"] == "runtime-failed",
                "old binary did not refuse registry v3 at runtime startup")
        require(registry_snapshot(registry) == before and sha256(registry) == digest,
                "refused old binary changed the v3 registry")
        return {"registry_version": 3, "registry_sha256": digest,
                "envelope": envelope, "stderr": result.stderr,
                "qualification": "#222 reports generic runtime-failed, not a detailed schema cause"}

    def until(self, probe, label, timeout=60):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = probe()
            if value:
                return value
            time.sleep(0.1)
        raise RuntimeError(f"deadline waiting for {label}")

    def start(self, binary, env):
        registry = Path(env["TELEX_HOME"]).parent / "registry.sqlite"
        log = (self.root / f"watcher-{len(self.hosts)}.log").open("wb")
        child = subprocess.Popen(
            [str(binary), "--json", "--registry", str(registry), "--install-root",
             env["TELEX_INSTALL_ROOT"], "run"], env=env, cwd=self.root,
            stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        self.hosts.append((child, binary, env, log))

        def ready():
            require(child.poll() is None, f"owned Watcher exited early; see {log.name}")
            return self.watcher(binary, env, "status", allow_unready=True)

        self.until(ready, "owned Watcher management endpoint")
        return child

    def stop(self, child, binary, env):
        self.watcher(binary, env, "shutdown")
        require(child.wait(timeout=20) == 0, "owned Watcher did not shut down cleanly")

    def register(self, binary, env, watch_id, mode):
        directory = Path(env["TELEX_HOME"]).parent
        registration = directory / f"{watch_id}.json"
        registration.write_text(json.dumps({
            "schemaVersion": 2, "id": watch_id,
            "command": [str(Path(sys.executable).resolve()), str(self.detector),
                        str(directory), mode],
            "workingDirectory": str(directory),
            "intervalSeconds": 1 if mode == "event" else 86400,
            "timeoutSeconds": 15, "backendProfile": "archive",
            "sender": "service:watcher/archive-proof", "target": RECIPIENT,
            "environmentAllowlist": [],
        }), encoding="utf-8")
        self.watcher(binary, env, "register", registration)

    def committed_event(self, binary, env):
        watch = self.watcher(binary, env, "show", EVENT_WATCH)
        return watch if (watch["lastCommittedEventSequence"] == 1
                         and watch["pendingOperationId"] is None) else None

    def messages(self, telex, env):
        result = self.telex(telex, env, "export", "--address", RECIPIENT)
        return [json.loads(line) for line in result.stdout.splitlines() if line.strip()]

    def evidence(self, binary, env):
        events = self.watcher(binary, env, "events", EVENT_WATCH)
        require(len(events) == 1 and events[0]["status"] == "accepted",
                "expected exactly one accepted event")
        event = events[0]
        require(event["receipt"] is not None, "event lacks durable send receipt")
        require(event["receipt"]["operationId"] == event["operationId"],
                "receipt does not bind the exact operation")
        receipts = self.watcher(binary, env, "receipt-evidence", event["operationId"])
        require(receipts, "missing receipt observation history")
        return {"event": event, "receipts": receipts}

    def fresh(self, telex, watcher, source_sha):
        env = self.environment("fresh")
        installed, minor = self.install(telex, env, "archive-candidate", source_sha)
        self.bootstrap(ROOT / "telex-watcher" / "installation.ps1", installed, watcher, env)
        host = self.start(watcher, env)
        self.register(watcher, env, EVENT_WATCH, "event")
        daemon = self.admitted_daemon(installed, env, minor)
        self.until(lambda: self.committed_event(watcher, env), "fresh archive receipt-gated send")
        evidence = self.evidence(watcher, env)
        require(len(self.messages(installed, env)) == 1, "fresh archive send was duplicated")
        self.stop(host, watcher, env)
        self.cli.stop_daemon(daemon, installed, env)
        self.report["fresh_install"] = evidence

    def upgrade(self, baseline_root, baseline_telex, baseline_watcher,
                candidate_telex, candidate_watcher, source_sha):
        env = self.environment("upgrade")
        old, old_minor = self.install(baseline_telex, env, "source-222", BASELINE_SHA)
        self.bootstrap(baseline_root / "telex-watcher" / "installation.ps1",
                       old, baseline_watcher, env)
        host = self.start(baseline_watcher, env)
        self.register(baseline_watcher, env, EVENT_WATCH, "event")
        daemon = self.admitted_daemon(old, env, old_minor)
        self.until(lambda: self.committed_event(baseline_watcher, env), "source baseline send")
        self.watcher(baseline_watcher, env, "pause", EVENT_WATCH)
        self.register(baseline_watcher, env, FAILURE_WATCH, "failure")
        self.until(lambda: any(
            item["outcome"] == "failed"
            for item in self.watcher(baseline_watcher, env, "attempts", FAILURE_WATCH)),
            "source baseline historical failure")
        self.watcher(baseline_watcher, env, "pause", FAILURE_WATCH)
        original_evidence = self.evidence(baseline_watcher, env)
        original_messages = self.messages(old, env)
        require(len(original_messages) == 1, "baseline did not create exactly one message")
        self.stop(host, baseline_watcher, env)
        self.cli.stop_daemon(daemon, old, env)
        registry = Path(env["TELEX_HOME"]).parent / "registry.sqlite"
        before = registry_snapshot(registry)
        check_baseline(before)
        self.report["source_baseline"] = {
            "source_sha": BASELINE_SHA, "registry": before,
            "evidence": original_evidence, "messages": original_messages,
            "registry_sha256_before_migration": sha256(registry),
        }
        new, new_minor = self.install(candidate_telex, env, "archive-candidate", source_sha)
        host = self.start(candidate_watcher, env)
        after = registry_snapshot(registry)
        check_migration(before, after)
        for watch_id in (EVENT_WATCH, FAILURE_WATCH):
            show = self.watcher(candidate_watcher, env, "show", watch_id)
            require("latestFailure" in show and show["latestFailure"] is None,
                    "public show must not reconstruct a historical failure")
            historical = self.watcher(candidate_watcher, env, "attempts", watch_id)
            require(historical and all("stderrExcerpt" in item
                                      and item["stderrExcerpt"] is None for item in historical),
                    "public historical stderr excerpts must be null")
        self.watcher(candidate_watcher, env, "resume", EVENT_WATCH)
        daemon = self.admitted_daemon(new, env, new_minor)

        def resumed_idle():
            return next((item for item in self.watcher(
                candidate_watcher, env, "attempts", EVENT_WATCH)
                if item["id"] not in before["attempts"] and item["outcome"] == "idle"), None)

        resumed = self.until(resumed_idle, "archive runtime idle attempt with preserved state")
        request = json.loads((registry.parent / f'{resumed["id"]}.json').read_text(encoding="utf-8"))
        require(request["state"] == {"cursor": 1}, "archive detector did not receive committed state")
        self.watcher(candidate_watcher, env, "pause", EVENT_WATCH)
        # The old projection predates this additive flag; its attempt is retained.
        original_evidence = {
            **original_evidence,
            "event": {**original_evidence["event"], "attemptCompacted": False},
        }
        require(self.evidence(candidate_watcher, env) == original_evidence,
                "upgrade changed exact operation or receipt evidence")
        require(self.messages(new, env) == original_messages,
                "archive resume lost, modified or duplicated the delivered message")
        require(self.committed_event(candidate_watcher, env), "archive changed sequence high-water")
        host, backup_report = self.backup_restore(
            baseline_watcher, candidate_watcher, new, env, host, original_evidence,
            original_messages)
        self.stop(host, candidate_watcher, env)
        self.cli.stop_daemon(daemon, new, env)
        self.report["upgrade"] = {
            "migration": "source #222 registry v2 -> forward-only registry v3",
            "historical_diagnostics": "null; not reconstructed",
            "rollback": "not tested or promised; binary replacement is not rollback",
            "registry_after_open": after, "resumed_attempt": resumed,
            "preserved_operation_and_receipts": original_evidence,
            "preserved_messages": original_messages,
            "backup_restore": backup_report,
        }

    def backup_restore(self, old_watcher, watcher, telex, env, host, original_evidence,
                       original_messages):
        registry = Path(env["TELEX_HOME"]).parent / "registry.sqlite"
        backup = registry.with_name("live-backup.sqlite")
        for watch_id in (EVENT_WATCH, FAILURE_WATCH):
            self.until(lambda: self.watcher(watcher, env, "show", watch_id)["runningAttemptId"] is None,
                       f"paused {watch_id} detector completion before backup")
        backup_result = self.watcher(watcher, env, "backup", backup)
        marked = registry_snapshot(backup)
        require(marked["version"] == 3, "live snapshot did not retain the v3 downgrade guard")
        old_backup_refusal = self.refuse_v3(old_watcher, env, backup)
        digest = sha256(backup)
        refused, detail = self.offline(watcher, env, backup, "run", expected=1)
        require("backup" in (detail.stdout + detail.stderr).lower(),
                "new runtime refusal did not identify a marked backup")
        require(sha256(backup) == digest, "ordinary runtime open changed the marked backup")
        self.stop(host, watcher, env)
        before_restore = registry_snapshot(registry)
        evidence_before = set(registry.parent.glob("*pre-restore-*"))
        restored, _ = self.offline(watcher, env, registry, "restore", backup)
        retained = sorted(set(registry.parent.glob("*pre-restore-*")) - evidence_before)
        require(retained, "offline restore did not retain pre-restore evidence")
        retained_registries = []
        for path in retained:
            if path.is_file():
                with path.open("rb") as source:
                    sqlite_header = source.read(16) == b"SQLite format 3\x00"
                if sqlite_header and registry_snapshot(path) == before_restore:
                    retained_registries.append({"path": str(path), "sha256": sha256(path)})
        require(retained_registries, "retained pre-restore registry lost original state or attempts")
        restored_snapshot = registry_snapshot(registry)
        for watch_id in (EVENT_WATCH, FAILURE_WATCH):
            restored_watch = restored_snapshot["watches"][watch_id]
            require(restored_watch["lifecycle"] == "paused"
                    and restored_watch.get("restoreReviewRequired") is True,
                    "offline restore must persist individual fences before any binary opens it")
        old_restore_refusal = self.refuse_v3(old_watcher, env, registry)
        host = self.start(watcher, env)
        fenced = {}
        for watch_id in (EVENT_WATCH, FAILURE_WATCH):
            show = self.watcher(watcher, env, "show", watch_id)
            require(show["lifecycle"] == "paused" and show["restoreReviewRequired"] is True,
                    "restored nonterminal watch lacks its individual review fence")
            fenced[watch_id] = show
        # Both intervals have elapsed for the event watch. A restored owner must
        # not invoke even an idle detector before its individual explicit resume.
        time.sleep(2.5)
        fenced_snapshot = registry_snapshot(registry)
        require(set(fenced_snapshot["attempts"]) == set(marked["attempts"]),
                "restored fenced owner launched a detector before explicit resume")
        require(self.messages(telex, env) == original_messages,
                "fenced restore lost, changed or duplicated a delivered message")
        self.watcher(watcher, env, "resume", EVENT_WATCH)
        require(self.watcher(watcher, env, "show", EVENT_WATCH)["restoreReviewRequired"] is False,
                "individual resume did not clear the selected watch fence")
        require(self.watcher(watcher, env, "show", FAILURE_WATCH)["restoreReviewRequired"] is True,
                "resuming one watch incorrectly cleared another watch's fence")

        def restored_idle():
            return next((item for item in self.watcher(watcher, env, "attempts", EVENT_WATCH)
                         if item["id"] not in marked["attempts"] and item["outcome"] == "idle"), None)

        resumed = self.until(restored_idle, "restored archive runtime idle attempt")
        request = json.loads((registry.parent / f'{resumed["id"]}.json').read_text(encoding="utf-8"))
        require(request["state"] == {"cursor": 1}, "restored detector lost committed state")
        self.watcher(watcher, env, "pause", EVENT_WATCH)
        require(self.evidence(watcher, env) == original_evidence,
                "restore changed exact operation or receipt evidence")
        require(self.messages(telex, env) == original_messages,
                "restored resume lost, modified or duplicated the delivered message")
        return host, {
            "backup_result": backup_result,
            "source_before_restore": before_restore, "marked_backup": marked,
            "new_runtime_marked_backup_refusal": refused,
            "genuine_old_binary_marked_backup_refusal": old_backup_refusal,
            "genuine_old_binary_restored_registry_refusal": old_restore_refusal,
            "fenced_registry_before_old_binary_startup": restored_snapshot,
            "offline_restore": restored, "pre_restore_evidence": [str(path) for path in retained],
            "retained_original_registries": retained_registries,
            "individual_review_fences": fenced, "resumed_attempt": resumed,
            "preserved_operation_and_receipts": original_evidence,
            "preserved_messages": original_messages,
        }

    def cleanup(self):
        errors = []
        for child, binary, env, log in reversed(self.hosts):
            try:
                if child.poll() is None:
                    self.stop(child, binary, env)
            except Exception as error:
                errors.append(str(error))
                try:
                    if child.poll() is None:
                        child.kill()
                        child.wait(timeout=15)
                except Exception as kill_error:
                    errors.append(str(kill_error))
            finally:
                log.close()
        for child, binary, env, log in reversed(self.cli.daemons):
            try:
                if child.poll() is None:
                    self.cli.stop_daemon(child, binary, env)
            except Exception as error:
                errors.append(str(error))
                try:
                    if child.poll() is None:
                        child.kill()
                        child.wait(timeout=15)
                except Exception as kill_error:
                    errors.append(str(kill_error))
            finally:
                log.close()
        require(not errors, f"owned fixture cleanup failed: {errors}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--target", default=TARGET)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--baseline-root", required=True, type=Path)
    parser.add_argument("--baseline-bin-dir", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args(argv)
    report = {"ok": False, "commands": [], "daemons": [], "target": args.target,
              "candidate_source_sha": args.source_sha, "baseline_source_sha": BASELINE_SHA}
    root = ROOT / (".watcher-archive-proof-" + uuid.uuid4().hex)
    fixture = None
    try:
        require(os.name == "nt" and platform.machine().lower() in ("amd64", "x86_64"),
                "archive runtime proof must execute on native Windows x86_64")
        require(args.target == TARGET, "unsupported Watcher archive target")
        require(len(args.source_sha) == 40
                and all(char in "0123456789abcdef" for char in args.source_sha),
                "candidate source SHA must be a full Git object ID")
        # Match the native Rust isolation harness: hosted workspace/profile
        # ancestors can grant foreign mutation rights. Never repair their ACLs.
        root = Path(Path(os.environ["LOCALAPPDATA"]).anchor) / root.name
        create_private_directory(root)
        fixture = WatcherArchiveProof(root, report)
        archive = args.archive.resolve()
        telex, watcher = unpack_archive(archive, root / "archive", args.target)
        baseline_root = args.baseline_root.resolve()
        require(baseline_root != ROOT and baseline_root.is_relative_to(ROOT),
                "baseline must use a separate source checkout inside this workspace")
        baseline_env = clean_environment(root / "baseline-check")
        git = Path(shutil.which("git") or "").resolve()
        require(git.is_file(), "Git is required to verify baseline source provenance")
        actual = fixture.cli.run(
            [git, "-C", baseline_root, "rev-parse", "HEAD"], baseline_env).stdout.strip()
        require(actual == BASELINE_SHA, "baseline checkout is not the pinned #222 source")
        dirty = fixture.cli.run(
            [git, "-C", baseline_root, "status", "--porcelain", "--untracked-files=no"],
            baseline_env).stdout.strip()
        require(not dirty, "baseline source checkout has tracked modifications")
        report["baseline_source_tree"] = fixture.cli.run(
            [git, "-C", baseline_root, "rev-parse", "HEAD^{tree}"], baseline_env).stdout.strip()
        baseline_bin = args.baseline_bin_dir.resolve()
        old_telex, old_watcher = baseline_bin / "telex.exe", baseline_bin / "telex-watcher.exe"
        for binary in (old_telex, old_watcher, telex, watcher):
            require(binary.is_file(), f"missing proof binary: {binary}")
        for binary in (old_watcher, watcher):
            version = fixture.cli.run([binary, "--version"], baseline_env).stdout
            require("backends [sqlite, postgres]" in version and "entra auth available" in version,
                    "Watcher must be built with sqlite, postgres and entra")
        report["artifacts"] = {
            "archive": str(archive), "archive_sha256": sha256(archive),
            "archive_telex_sha256": sha256(telex), "archive_watcher_sha256": sha256(watcher),
            "baseline_telex_sha256": sha256(old_telex),
            "baseline_watcher_sha256": sha256(old_watcher),
        }
        fixture.fresh(telex, watcher, args.source_sha)
        fixture.upgrade(baseline_root, old_telex, old_watcher, telex, watcher, args.source_sha)
        report["ok"] = True
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        if fixture is not None:
            try:
                fixture.cleanup()
            except Exception as error:
                report["ok"] = False
                report["cleanup_error"] = f"{type(error).__name__}: {error}"
        if report["ok"]:
            try:
                shutil.rmtree(root)
            except Exception as error:
                report["ok"] = False
                report["cleanup_error"] = f"{type(error).__name__}: {error}"
        report["fixture_root"] = str(root)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if not report["ok"]:
        print(json.dumps({key: report[key] for key in ("error", "cleanup_error") if key in report}),
              file=sys.stderr)
        return 1
    print(f"Watcher archive install and genuine source upgrade passed: {args.report}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
