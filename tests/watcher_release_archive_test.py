"""Offline archive-proof contracts. Safe on both Windows and Linux CI hosts."""

from contextlib import contextmanager, redirect_stderr
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import textwrap
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch
import uuid
import warnings
import zipfile

import watcher_archive_detector as detector
import watcher_release_archive as proof


@contextmanager
def workspace():
    root = proof.ROOT / (".watcher-archive-unit-" + uuid.uuid4().hex)
    root.mkdir()
    try:
        yield root
    finally:
        shutil.rmtree(root)


def baseline_snapshot():
    return {
        "version": 2,
        "watches": {
            watch_id: {
                "state": {"cursor": 1} if watch_id == proof.EVENT_WATCH else None,
                "lastAllocatedEventSequence": 1, "lastCommittedEventSequence": 1,
                "logicalStoreId": "store", "pendingOperationId": None,
                "registrationRevision": 1,
            }
            for watch_id in (proof.EVENT_WATCH, proof.FAILURE_WATCH)
        },
        "attempts": {
            "accepted": {"watchId": proof.EVENT_WATCH, "outcome": "accepted"},
            "failed": {"watchId": proof.FAILURE_WATCH, "outcome": "failed"},
        },
    }


def migrated_snapshot():
    snapshot = baseline_snapshot()
    snapshot["version"] = 3
    for row in snapshot["watches"].values():
        row["latestFailure"] = None
    for row in snapshot["attempts"].values():
        row["stderrExcerpt"] = None
    return snapshot


class WatcherArchiveTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "nt" and shutil.which("pwsh"), "native Windows packaging")
    def test_release_packaging_block_includes_x64_watcher_but_excludes_arm_even_if_stale(self):
        workflow = (proof.ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        step = workflow.split("      - name: Package Windows binary\n", 1)[1]
        body = textwrap.dedent(step.split("        run: |\n", 1)[1].split(
            "      - name: Package Unix binary\n", 1)[0])
        for target in (proof.TARGET, "aarch64-pc-windows-msvc"):
            with self.subTest(target=target), workspace() as root:
                release = root / "target" / target / "release"
                release.mkdir(parents=True)
                (release / "telex.exe").write_bytes(b"fixture Telex; not executable")
                (release / "telex-watcher.exe").write_bytes(b"fixture Watcher; not executable")
                (root / "LICENSE").write_text("fixture license", encoding="ascii")
                package = root / "package"
                package.mkdir()
                (package / "telex-watcher.exe").write_bytes(b"stale unsupported Watcher")
                script = body.replace("${{ steps.ver.outputs.v }}", "fixture").replace(
                    "${{ matrix.target }}", target)
                result = subprocess.run(
                    [str(Path(shutil.which("pwsh")).resolve()), "-NoProfile", "-NonInteractive",
                     "-Command", "$ErrorActionPreference='Stop'; " + script],
                    cwd=root, env=proof.clean_environment(root),
                    capture_output=True, text=True, timeout=30)
                self.assertEqual(result.returncode, 0, result.stderr)
                archive = root / "dist" / f"telex-fixture-{target}.zip"
                with zipfile.ZipFile(archive) as zipped:
                    expected = {"telex.exe", "LICENSE"}
                    if target == proof.TARGET:
                        expected.add("telex-watcher.exe")
                    self.assertEqual(set(zipped.namelist()), expected)
                if target == proof.TARGET:
                    _, watcher = proof.unpack_archive(archive, root / "extracted", target)
                    self.assertEqual(watcher.read_bytes(), (release / "telex-watcher.exe").read_bytes())

    def test_archive_members_and_checksum_are_fail_closed(self):
        cases = ("valid", "missing-watcher", "duplicate", "traversal", "bad-checksum",
                 "wrong-sidecar-name", "arm", "empty")
        for case in cases:
            with self.subTest(case=case), workspace() as root:
                archive = root / "candidate.zip"
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", UserWarning)
                    with zipfile.ZipFile(archive, "w") as package:
                        package.writestr("telex.exe", b"telex")
                        package.writestr("LICENSE", b"license")
                        if case != "missing-watcher":
                            package.writestr("telex-watcher.exe", b"" if case == "empty" else b"watcher")
                        if case == "duplicate":
                            package.writestr("telex-watcher.exe", b"other")
                        if case == "traversal":
                            package.writestr("../escaped.exe", b"bad")
                digest = "0" * 64 if case == "bad-checksum" else proof.sha256(archive)
                name = "other.zip" if case == "wrong-sidecar-name" else archive.name
                archive.with_suffix(".zip.sha256").write_text(f"{digest}  {name}", encoding="ascii")
                target = "aarch64-pc-windows-msvc" if case == "arm" else proof.TARGET
                if case == "valid":
                    telex, watcher = proof.unpack_archive(archive, root / "unpacked", target)
                    self.assertTrue(telex.is_absolute())
                    self.assertEqual(telex.read_bytes(), b"telex")
                    self.assertEqual(watcher.read_bytes(), b"watcher")
                else:
                    with self.assertRaises(RuntimeError):
                        proof.unpack_archive(archive, root / "unpacked", target)
                self.assertFalse((root / "escaped.exe").exists())

    def test_baseline_requires_real_pre_extension_state_and_failure(self):
        proof.check_baseline(baseline_snapshot())
        cases = (
            lambda value: value.update(version=1),
            lambda value: value["watches"][proof.EVENT_WATCH].update(state=None),
            lambda value: value["watches"][proof.EVENT_WATCH].update(latestFailure=None),
            lambda value: value["attempts"]["failed"].update(stderrExcerpt=None),
            lambda value: value["attempts"].pop("failed"),
        )
        for mutate in cases:
            value = baseline_snapshot()
            mutate(value)
            with self.assertRaises(RuntimeError):
                proof.check_baseline(value)

    def test_migration_preserves_spine_and_never_reconstructs_historical_diagnostics(self):
        proof.check_migration(baseline_snapshot(), migrated_snapshot())
        cases = (
            lambda value: value.update(version=2),
            lambda value: value["watches"][proof.EVENT_WATCH].update(state=None),
            lambda value: value["watches"][proof.EVENT_WATCH].update(lastAllocatedEventSequence=0),
            lambda value: value["watches"][proof.EVENT_WATCH].update(logicalStoreId="other"),
            lambda value: value["watches"][proof.EVENT_WATCH].pop("latestFailure"),
            lambda value: value["watches"][proof.FAILURE_WATCH].update(latestFailure={}),
            lambda value: value["attempts"]["failed"].update(stderrExcerpt="invented"),
            lambda value: value["attempts"]["failed"].pop("stderrExcerpt"),
            lambda value: value["attempts"]["accepted"].update(outcome="failed"),
        )
        for mutate in cases:
            value = migrated_snapshot()
            mutate(value)
            with self.assertRaises(RuntimeError):
                proof.check_migration(baseline_snapshot(), value)

    def test_detector_emits_once_then_observably_resumes_with_committed_state(self):
        event = detector.detect({"state": None}, "event")
        self.assertEqual(event["outcome"], "event")
        idle = detector.detect({"state": event["nextState"]}, "event")
        self.assertEqual(idle, {
            "schemaVersion": 2, "outcome": "idle", "nextState": {"cursor": 1},
        })
        with self.assertRaises(RuntimeError):
            detector.detect({"state": None}, "failure")
        with self.assertRaises(ValueError):
            detector.detect({"state": {"cursor": 2}}, "event")
        with workspace() as root:
            request = {"schemaVersion": 2, "attempt": {"id": "test-attempt"}, "state": {"cursor": 1}}
            result = subprocess.run(
                [sys.executable, str(proof.ROOT / "tests" / "watcher_archive_detector.py"),
                 str(root), "event"], input=json.dumps(request), capture_output=True,
                text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout), idle)
            self.assertEqual(json.loads((root / "test-attempt.json").read_text()), request)

    def test_clean_environment_overrides_all_operator_roots_and_drops_credentials(self):
        with workspace() as root, patch.dict(os.environ, {
            "TELEX_HOME": "operator", "TELEX_DB": "operator",
            "TELEX_INSTALL_ROOT": "operator", "LOCALAPPDATA": "operator",
            "TELEX_WATCHER_REGISTRY": "operator", "PGPASSWORD": "secret",
            "AZURE_CLIENT_SECRET": "secret",
        }):
            env = proof.clean_environment(root)
            for name in ("TELEX_HOME", "TELEX_CONFIG", "TELEX_DB", "TELEX_INSTALL_ROOT",
                         "LOCALAPPDATA", "HOME", "USERPROFILE", "APPDATA", "TELEX_RUN_DIR"):
                self.assertTrue(Path(env[name]).is_absolute(), name)
                self.assertTrue(Path(env[name]).is_relative_to(root), name)
            for name in ("TELEX_WATCHER_REGISTRY", "PGPASSWORD", "AZURE_CLIENT_SECRET"):
                self.assertNotIn(name, env)

    def test_duplicate_operation_and_missing_receipt_are_rejected(self):
        fixture = proof.WatcherArchiveProof.__new__(proof.WatcherArchiveProof)
        event = {"status": "accepted", "operationId": "op", "receipt": {"operationId": "op"}}
        for events, receipts in (([], []), ([event, event], [{}]),
                                 ([{**event, "receipt": None}], [{}]),
                                 ([{**event, "receipt": {"operationId": "other"}}], [{}]),
                                 ([event], [])):
            fixture.watcher = Mock(side_effect=[events, receipts])
            with self.assertRaises(RuntimeError):
                fixture.evidence(Path("unused"), {})
        fixture.watcher = Mock(side_effect=[[event], [{"operationId": "op"}]])
        self.assertEqual(fixture.evidence(Path("unused"), {})["event"], event)

    def test_old_binary_refusal_is_v3_only_read_only_and_honest_about_generic_error(self):
        fixture = proof.WatcherArchiveProof.__new__(proof.WatcherArchiveProof)
        refused = {"ok": False, "error": {"code": "runtime-failed"}}
        fixture.offline = Mock(return_value=(refused, SimpleNamespace(stderr="")))
        with workspace() as root:
            with patch.object(proof, "registry_snapshot", return_value=migrated_snapshot()), \
                    patch.object(proof, "sha256", return_value="digest"):
                result = fixture.refuse_v3(root / "old.exe", {}, root / "marked.sqlite")
            self.assertIn("generic runtime-failed", result["qualification"])
            self.assertEqual(result["registry_version"], 3)
            self.assertEqual(fixture.offline.call_args.kwargs, {"expected": 1})
            fixture.offline.reset_mock()
            with patch.object(proof, "registry_snapshot", return_value=baseline_snapshot()):
                with self.assertRaisesRegex(RuntimeError, "must really be registry v3"):
                    fixture.refuse_v3(root / "old.exe", {}, root / "not-v3.sqlite")
            fixture.offline.assert_not_called()
            changed = migrated_snapshot()
            changed["watches"][proof.EVENT_WATCH]["state"] = None
            with patch.object(proof, "registry_snapshot",
                              side_effect=[migrated_snapshot(), changed]), \
                    patch.object(proof, "sha256", return_value="digest"):
                with self.assertRaisesRegex(RuntimeError, "changed the v3 registry"):
                    fixture.refuse_v3(root / "old.exe", {}, root / "marked.sqlite")

    def test_backup_restore_orders_offline_restore_and_checks_individual_fences(self):
        fixture = proof.WatcherArchiveProof.__new__(proof.WatcherArchiveProof)
        with workspace() as root:
            env = {"TELEX_HOME": str(root / "home")}
            registry = root / "registry.sqlite"
            backup = root / "live-backup.sqlite"
            retained = root / "registry.sqlite.pre-restore-evidence"
            old, candidate, telex = (root / name for name in ("old.exe", "candidate.exe", "telex.exe"))
            original_host, restored_host = Mock(), Mock()
            resumed = False
            messages, evidence = [{"message": {"id": 1}}], {"operationId": "op"}
            (root / "resumed.json").write_text(json.dumps({"state": {"cursor": 1}}))

            def watcher(_binary, _env, action, *args):
                nonlocal resumed
                if action == "show":
                    return {"runningAttemptId": None, "lifecycle": "paused",
                            "restoreReviewRequired": not (resumed and args[0] == proof.EVENT_WATCH)}
                if action == "resume":
                    resumed = True
                if action == "attempts":
                    return [{"id": "resumed", "outcome": "idle"}]
                return {"version": 1}

            def offline(_binary, _env, path, action, *args, **kwargs):
                if action == "restore":
                    self.assertTrue(fixture.stop.called, "original owner must be stopped first")
                    self.assertEqual(path, registry)
                    self.assertEqual(args, (backup,))
                    retained.write_bytes(b"SQLite format 3\x00")
                    return {"ok": True}, SimpleNamespace(stdout="", stderr="")
                self.assertEqual((path, action, kwargs), (backup, "run", {"expected": 1}))
                return {"ok": False}, SimpleNamespace(stdout="backup-restore-required", stderr="")

            fixture.watcher = Mock(side_effect=watcher)
            fixture.offline = Mock(side_effect=offline)
            fixture.refuse_v3 = Mock(return_value={"registry_version": 3})
            fixture.start = Mock(return_value=restored_host)
            fixture.stop = Mock()
            fixture.messages = Mock(return_value=messages)
            fixture.evidence = Mock(return_value=evidence)
            snapshot = migrated_snapshot()
            for row in snapshot["watches"].values():
                row.update(lifecycle="paused", restoreReviewRequired=True)
            with patch.object(proof, "registry_snapshot", side_effect=lambda _: snapshot), \
                    patch.object(proof, "sha256", return_value="digest"), \
                    patch.object(proof.time, "sleep"):
                host, report = fixture.backup_restore(
                    old, candidate, telex, env, original_host, evidence, messages)
            self.assertIs(host, restored_host)
            self.assertEqual([call.args[-1] for call in fixture.refuse_v3.call_args_list],
                             [backup, registry])
            fixture.stop.assert_called_once_with(original_host, candidate, env)
            self.assertEqual(report["retained_original_registries"][0]["path"], str(retained))
            self.assertTrue(report["individual_review_fences"][proof.FAILURE_WATCH]["restoreReviewRequired"])
            self.assertEqual(report["preserved_messages"], messages)

    def test_only_owned_child_handles_are_cleaned_up_even_when_shutdown_fails(self):
        fixture = proof.WatcherArchiveProof.__new__(proof.WatcherArchiveProof)
        child = Mock()
        child.poll.return_value = None
        log = io.BytesIO()
        fixture.hosts = [(child, Path("owned-watcher"), {}, log)]
        fixture.cli = SimpleNamespace(daemons=[])
        fixture.stop = Mock(side_effect=RuntimeError("shutdown failed"))
        with self.assertRaisesRegex(RuntimeError, "cleanup failed"):
            fixture.cleanup()
        child.kill.assert_called_once()
        child.wait.assert_called_once_with(timeout=15)
        self.assertTrue(log.closed)

    def test_native_platform_refusal_still_writes_failure_report(self):
        with workspace() as root, patch.object(proof.platform, "machine", return_value="arm64"), \
                redirect_stderr(io.StringIO()):
            report = root / "evidence" / "proof.json"
            self.assertEqual(proof.main([
                "--archive", str(root / "absent.zip"),
                "--source-sha", "1" * 40, "--baseline-root", str(root / "baseline"),
                "--baseline-bin-dir", str(root / "bins"), "--report", str(report),
            ]), 1)
            result = json.loads(report.read_text())
            self.assertFalse(result["ok"])
            self.assertIn("native Windows x86_64", result["error"])
            self.assertEqual(result["commands"], [])


if __name__ == "__main__":
    unittest.main()
