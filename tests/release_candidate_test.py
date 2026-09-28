"""Offline regression checks for the release proof's isolation and evidence."""

import hashlib
import importlib.util
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
                def authenticated_status(*_args):
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

    def test_child_environment_is_isolated_without_modifying_parent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ambient = {"GITHUB_TOKEN": "sentinel", "TELEX_BACKEND": "real",
                       "HTTPS_PROXY": "http://example.invalid", "PATH": os.environ["PATH"]}
            with patch.dict(os.environ, ambient, clear=True):
                before = dict(os.environ)
                env = proof.clean_environment(root)
                self.assertEqual(dict(os.environ), before)
                for key in ("GITHUB_TOKEN", "TELEX_BACKEND", "HTTPS_PROXY"):
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
