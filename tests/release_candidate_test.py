"""Offline regression checks for the release proof's isolation and evidence."""

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.error
import urllib.request
import zipfile


spec = importlib.util.spec_from_file_location(
    "release_candidate", Path(__file__).with_name("release_candidate.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


class ReleaseProofTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
