"""Offline regression checks for the release proof's isolation and evidence."""

import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch
import urllib.error
import urllib.request
import zipfile


spec = importlib.util.spec_from_file_location(
    "release_candidate", Path(__file__).with_name("release_candidate.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


class ReleaseProofTests(unittest.TestCase):
    def test_upstream_token_is_neither_redirected_nor_used_for_assets(self):
        url = f"https://api.github.com/repos/{proof.REPO}/releases/tags/{proof.BASELINE_TAG}"
        with patch.dict(os.environ, {"TELEX_PROOF_GITHUB_TOKEN": "sentinel"}, clear=True):
            with patch.object(proof.urllib.request, "urlopen", return_value=io.BytesIO(b"{}")) as open:
                proof.download(url, github_metadata=True)
                request = open.call_args.args[0]
                self.assertEqual(request.get_header("Authorization"), "Bearer sentinel")
                redirected = urllib.request.HTTPRedirectHandler().redirect_request(
                    request, None, 302, "redirect", {}, "https://example.invalid/asset")
                self.assertIsNone(redirected.get_header("Authorization"))
            with patch.object(proof.urllib.request, "urlopen", return_value=io.BytesIO(b"asset")) as open:
                proof.download("http://127.0.0.1:12345/candidate")
                self.assertIsNone(open.call_args.args[0].get_header("Authorization"))
            with patch.object(proof.urllib.request, "urlopen") as open:
                with self.assertRaisesRegex(RuntimeError, "unexpected upstream metadata"):
                    proof.download("https://example.invalid/metadata", github_metadata=True)
                open.assert_not_called()

    def test_readiness_waits_for_new_publication_including_reused_pid(self):
        old = {"server_pid": 42, "server_start_time": 100, "instance_id": "old"}
        new = {"server_pid": 42, "server_start_time": 200, "instance_id": "new"}
        self.assertIsNone(proof.successor_identity(None, 42, old))
        self.assertIsNone(proof.successor_identity(dict(old), 42, old))
        self.assertEqual(proof.successor_identity(new, 42, old), new)
        different_pid = {**new, "server_pid": 43}
        self.assertEqual(proof.successor_identity(different_pid, 43, old), different_pid)

    def test_readiness_rejects_foreign_or_incomplete_identity(self):
        old = {"server_pid": 42, "server_start_time": 100, "instance_id": "old"}
        with self.assertRaisesRegex(RuntimeError, "unexpected fixture daemon"):
            proof.successor_identity({**old, "server_pid": 99}, 43, old)
        with self.assertRaisesRegex(RuntimeError, "missing daemon start"):
            proof.successor_identity({"server_pid": 43, "instance_id": "new"}, 43, old)
        with self.assertRaisesRegex(RuntimeError, "missing daemon instance"):
            proof.successor_identity({"server_pid": 43, "server_start_time": 200}, 43, old)

    def test_status_is_not_requested_until_successor_publication(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            old = {"server_pid": 42, "server_start_time": 100, "instance_id": "old"}
            new = {"server_pid": 43, "server_start_time": 200, "instance_id": "new"}
            report = {"daemons": [{"pid": 42, "start_time": 100, "instance_id": "old"}]}
            fixture = proof.Proof(SimpleNamespace(postgres_url=None), root, report)
            env = proof.clean_environment(root / "env")
            binary = root / "telex"
            fixture.daemons.append((SimpleNamespace(pid=42, poll=lambda: 0), binary, env, None))
            child = Mock(pid=43)
            child.poll.return_value = None
            with patch.object(proof, "read_fixture_identity", side_effect=[old, old, new]) as read:
                def authenticated_status(*_args, **_kwargs):
                    self.assertEqual(read.call_count, 3, "status raced the known stale publication")
                    return SimpleNamespace(stdout=json.dumps({
                        "instance_id": "new", "protocol_version": {"major": 1, "minor": 5},
                    }))
                with patch.object(proof.subprocess, "Popen", return_value=child), \
                        patch.object(proof.time, "sleep"), \
                        patch.object(fixture, "telex", side_effect=authenticated_status) as status:
                    try:
                        self.assertIs(fixture.start_daemon(binary, env, 5), child)
                        status.assert_called_once()
                    finally:
                        fixture.daemons[-1][3].close()

    def test_readiness_status_uses_remaining_budget_and_rejects_late_success(self):
        for published, finished, accepted in ((14, 14.5, True), (14, 15, False),
                                               (14, 16, False), (15, 16, False)):
            with self.subTest(published=published, finished=finished), \
                    tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                report = {"daemons": []}
                fixture = proof.Proof(SimpleNamespace(postgres_url=None), root, report)
                env = proof.clean_environment(root / "env")
                child = Mock(pid=43)
                child.poll.return_value = None
                clock = SimpleNamespace(now=0)
                identity = {"server_pid": 43, "server_start_time": 200, "instance_id": "new"}
                reads = []
                timeouts = []

                def publication(_run_dir):
                    reads.append(True)
                    if len(reads) == 1:
                        return None
                    clock.now = published
                    return identity

                def status(*_args, timeout=45):
                    timeouts.append(timeout)
                    clock.now = finished
                    return SimpleNamespace(stdout=json.dumps({
                        "instance_id": "new", "protocol_version": {"major": 1, "minor": 5},
                    }))

                with patch.object(proof, "read_fixture_identity", side_effect=publication), \
                        patch.object(proof.subprocess, "Popen", return_value=child), \
                        patch.object(proof.time, "monotonic", side_effect=lambda: clock.now), \
                        patch.object(fixture, "telex", side_effect=status):
                    try:
                        if accepted:
                            self.assertIs(fixture.start_daemon(root / "telex", env, 5), child)
                        else:
                            with self.assertRaisesRegex(RuntimeError, "readiness deadline expired"):
                                fixture.start_daemon(root / "telex", env, 5)
                        self.assertEqual(timeouts, [1] if published < 15 else [])
                        self.assertEqual(len(report["daemons"]), int(accepted))
                    finally:
                        fixture.daemons[-1][3].close()

    def test_telex_keeps_default_timeout_and_forwards_explicit_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = proof.Proof(SimpleNamespace(postgres_url=None), Path(directory), {})
            with patch.object(fixture, "run") as run:
                fixture.telex(Path(directory) / "telex", {}, "version")
                self.assertEqual(run.call_args.kwargs["timeout"], 45)
                fixture.telex(Path(directory) / "telex", {}, "daemon", "status", timeout=0.25)
                self.assertEqual(run.call_args.kwargs["timeout"], 0.25)

    def test_acknowledge_requires_exact_first_marked_consumption(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "telex"
            fixture = proof.Proof(SimpleNamespace(postgres_url=None), Path(directory), {})
            receipt = {"message_id": 7, "recipient": "proof:inbox", "delivery_outcome": "marked"}
            with patch.object(fixture, "telex", return_value=SimpleNamespace(
                    stdout=json.dumps(receipt))) as command:
                self.assertEqual(fixture.acknowledge(binary, {}, "7", "proof:inbox"), receipt)
                command.assert_called_once_with(
                    binary, {}, "--address", "proof:inbox", "ack", "--id", "7")
            invalid = [{**receipt, "delivery_outcome": outcome} for outcome in (
                "no-delivery", "not-owner", "already-consumed", "ack-no-op", "delivery-mismatch",
            )] + [{**receipt, "message_id": 8}, {**receipt, "message_id": "7"},
                  {**receipt, "recipient": "proof:other"}, {}]
            for result in invalid:
                with self.subTest(result=result), patch.object(
                        fixture, "telex", return_value=SimpleNamespace(stdout=json.dumps(result))):
                    with self.assertRaisesRegex(RuntimeError, "Ack"):
                        fixture.acknowledge(binary, {}, "7", "proof:inbox")

    def test_child_environment_is_isolated_without_modifying_parent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ambient = {"GITHUB_TOKEN": "sentinel", "TELEX_PROOF_GITHUB_TOKEN": "sentinel",
                       "TELEX_BACKEND": "real",
                       "HTTPS_PROXY": "http://example.invalid", "PATH": os.environ["PATH"]}
            with patch.dict(os.environ, ambient, clear=True):
                before = dict(os.environ)
                env = proof.clean_environment(root)
                self.assertEqual(dict(os.environ), before)
                for key in ("GITHUB_TOKEN", "TELEX_PROOF_GITHUB_TOKEN", "TELEX_BACKEND", "HTTPS_PROXY"):
                    self.assertNotIn(key, env)
                for key in ("TELEX_HOME", "TELEX_CONFIG", "TELEX_DB", "TELEX_INSTALL_ROOT",
                            "TELEX_RUN_DIR", "LOCALAPPDATA", "HOME", "TEMP", "TMPDIR"):
                    self.assertTrue(Path(env[key]).is_relative_to(root), key)
                self.assertEqual(env["TELEX_NO_MODIFY_PATH"], "1")

    def test_loopback_serves_exact_bytes_and_records_requests(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "candidate.zip"
            archive.write_bytes(b"candidate archive")
            with proof.CandidateServer(archive, "v0.2.0", "x86_64-pc-windows-msvc") as server:
                route = "/lossyrob/telex/releases/download/v0.2.0/" + server.asset
                data = proof.download(server.url + route)
                self.assertEqual(data, archive.read_bytes())
                checksum = proof.download(server.url + route + ".sha256").decode().split()[0]
                self.assertEqual(checksum, hashlib.sha256(data).hexdigest())
                with self.assertRaises(urllib.error.HTTPError):
                    proof.download(server.url + "/unknown")
                self.assertEqual(server.requests, [route, route + ".sha256", "/unknown"])
            self.assertFalse(server.thread.is_alive())

    def test_extraction_reads_only_named_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "candidate.zip"
            with zipfile.ZipFile(archive, "w") as package:
                package.writestr("telex.exe", b"binary")
                package.writestr("../outside", b"must not extract")
            binary = proof.unpack_binary(archive, root / "payload", windows=True)
            self.assertEqual(binary.read_bytes(), b"binary")
            self.assertFalse((root / "outside").exists())

    def test_bad_candidate_writes_failed_report_and_cleanup_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "candidate.zip"
            archive.write_bytes(b"invalid")
            Path(str(archive) + ".sha256").write_text("0" * 64)
            report_path = root / "report.json"
            argv = ["proof", "--archive", str(archive), "--target", "x86_64-pc-windows-msvc",
                    "--source-sha", "fixture", "--report", str(report_path)]
            with patch("sys.argv", argv):
                with self.assertRaisesRegex(RuntimeError, "candidate archive checksum mismatch"):
                    proof.main()
            report = json.loads(report_path.read_text())
            self.assertEqual(report["status"], "failed")
            self.assertIn("candidate archive checksum mismatch", report["error"])
            self.assertEqual(report["commands"], [])
            self.assertTrue(report["cleanup"][-1]["removed"])

    def test_cleanup_failure_preserves_primary_error_logs_and_safe_root_disposition(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture_root = root / "fixture"
            fixture_root.mkdir()
            archive = root / "candidate.zip"
            archive.write_bytes(b"invalid")
            Path(str(archive) + ".sha256").write_text("0" * 64)
            report_path = root / "report.json"
            argv = ["proof", "--archive", str(archive), "--target", "x86_64-pc-windows-msvc",
                    "--source-sha", "fixture", "--report", str(report_path)]
            original = proof.Proof
            child = Mock(pid=42)
            child.returncode = None
            child.poll.side_effect = lambda: child.returncode
            child.kill.side_effect = lambda: setattr(child, "returncode", -9)
            child.wait.return_value = -9

            def owned_fixture(args, path, report):
                fixture = original(args, path, report)
                log = (path / "daemon-0.log").open("wb")
                log.write(b"owned cleanup diagnostic")
                log.flush()
                fixture.daemons.append((child, path / "telex", {}, log))
                fixture.stop_daemon = Mock(side_effect=RuntimeError("drain sentinel"))
                return fixture

            with patch("sys.argv", argv), \
                    patch.object(proof.tempfile, "mkdtemp", return_value=str(fixture_root)), \
                    patch.object(proof, "Proof", side_effect=owned_fixture):
                with self.assertRaisesRegex(RuntimeError, "candidate archive checksum mismatch"):
                    proof.main()
            report = json.loads(report_path.read_text())
            self.assertEqual(report["status"], "failed")
            self.assertIn("candidate archive checksum mismatch", report["error"])
            self.assertIn("drain sentinel", report["cleanup_error"])
            self.assertEqual(report["daemon_logs"]["daemon-0.log"], "owned cleanup diagnostic")
            self.assertTrue(report["cleanup"][-1]["removed"])
            self.assertFalse(fixture_root.exists())
            child.kill.assert_called_once()
            child.wait.assert_called_once_with(timeout=10)

    def test_cleanup_failure_remains_nonzero_after_logs_and_safe_removal(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture_root = root / "fixture"
            fixture_root.mkdir()
            (fixture_root / "daemon-0.log").write_text("cleanup evidence")
            child = SimpleNamespace(pid=42, poll=lambda: -9)
            fixture = SimpleNamespace(
                daemons=[(child, None, None, None)],
                cleanup=Mock(side_effect=RuntimeError("cleanup sentinel")),
            )
            report = {"status": "passed", "cleanup": [], "daemons": []}
            output = root / "report.json"
            with self.assertRaisesRegex(RuntimeError, "cleanup sentinel"):
                proof.finish_report(fixture, fixture_root, report, output)
            saved = json.loads(output.read_text())
            self.assertEqual(saved["status"], "failed")
            self.assertEqual(saved["daemon_logs"]["daemon-0.log"], "cleanup evidence")
            self.assertTrue(saved["cleanup"][-1]["removed"])
            self.assertFalse(fixture_root.exists())

    def test_unready_owned_process_retains_root_even_without_readiness_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture_root = root / "fixture"
            fixture_root.mkdir()
            (fixture_root / "daemon-0.log").write_text("unready process evidence")
            child = SimpleNamespace(pid=42, poll=lambda: None)
            fixture = SimpleNamespace(
                daemons=[(child, None, None, None)],
                cleanup=Mock(side_effect=RuntimeError("owned process stop failed")),
            )
            report = {"status": "passed", "cleanup": [], "daemons": []}
            output = root / "report.json"
            with self.assertRaisesRegex(RuntimeError, "owned process stop failed"):
                proof.finish_report(fixture, fixture_root, report, output)
            saved = json.loads(output.read_text())
            self.assertEqual(saved["status"], "failed")
            self.assertEqual(saved["daemon_logs"]["daemon-0.log"], "unready process evidence")
            self.assertFalse(saved["cleanup"][-1]["removed"])
            self.assertEqual(saved["cleanup"][-1]["owned_pids"], [42])
            self.assertTrue(fixture_root.exists())

    def test_nonlocal_postgres_is_rejected_before_access_and_root_is_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            report_path = Path(directory) / "report.json"
            argv = ["proof", "--archive", str(Path(directory) / "unused.zip"),
                    "--target", "x86_64-pc-windows-msvc", "--source-sha", "fixture",
                    "--postgres-url", "postgresql://example.invalid/shared",
                    "--disposable-postgres", "--report", str(report_path)]
            with patch("sys.argv", argv):
                with self.assertRaisesRegex(RuntimeError, "disposable loopback"):
                    proof.main()
            report = json.loads(report_path.read_text())
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["commands"], [])
            self.assertTrue(report["cleanup"][-1]["removed"])


if __name__ == "__main__":
    unittest.main()
