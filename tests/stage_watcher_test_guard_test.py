import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch
from types import SimpleNamespace

from stage_watcher_test_guard import stage_guard


class GuardianHandoffTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.manifest = self.root / "telex-watcher" / "Cargo.toml"
        self.messages = [{"reason": "build-finished", "success": True}]

    def artifact(self, relative, name, test, manifest=None):
        executable = self.root / relative
        executable.parent.mkdir(parents=True, exist_ok=True)
        executable.write_bytes(b"test host" if test else b"exact guardian")
        self.messages.append(
            {
                "reason": "compiler-artifact",
                "manifest_path": str(manifest or self.manifest),
                "executable": str(executable),
                "target": {"name": name, "kind": ["bin"]},
                "profile": {"test": test},
            }
        )
        return executable

    def test_exact_manifest_guard_is_staged_for_ordinary_and_relocated_hosts(self):
        guard = self.artifact("debug/watcher_process_guard", "watcher_process_guard", False)
        hosts = [
            self.artifact("debug/deps/process-test", "process_v2", True),
            self.artifact(
                "debug/build/telex-watcher/hash/out/unit-test", "telex_watcher", True
            ),
        ]
        self.artifact(
            "stale/watcher_process_guard",
            "watcher_process_guard",
            False,
            self.root / "other" / "Cargo.toml",
        )
        self.artifact("debug/deps/guard-test", "watcher_process_guard", True)
        stage_guard(self.messages, self.manifest)
        guard.write_bytes(b"rebuilt exact guardian")
        stage_guard(self.messages, self.manifest)
        self.assertEqual(guard.read_bytes(), b"rebuilt exact guardian")
        self.assertEqual(
            (hosts[1].parent / "watcher_process_guard").read_bytes(),
            b"rebuilt exact guardian",
        )
        self.assertFalse((hosts[0].parent / "watcher_process_guard").exists())

    def test_missing_and_ambiguous_guardians_fail_before_staging(self):
        host = self.artifact("debug/deps/host", "telex_watcher", True)
        with self.assertRaisesRegex(ValueError, "expected one exact"):
            stage_guard(self.messages, self.manifest)
        self.artifact("one/guard", "watcher_process_guard", False)
        self.artifact("two/guard", "watcher_process_guard", False)
        with self.assertRaisesRegex(ValueError, "expected one exact"):
            stage_guard(self.messages, self.manifest)
        self.assertFalse((host.parent / "watcher_process_guard").exists())

    def test_missing_emitted_file_and_unsuccessful_build_fail_closed(self):
        guard = self.artifact("debug/guard", "watcher_process_guard", False)
        self.artifact("debug/deps/host", "telex_watcher", True)
        guard.unlink()
        with self.assertRaises(OSError):
            stage_guard(self.messages, self.manifest)
        self.messages.append({"reason": "build-finished", "success": False})
        with self.assertRaisesRegex(ValueError, "successful fixture builds"):
            stage_guard(self.messages, self.manifest)

    def test_missing_build_completion_or_test_hosts_fails(self):
        self.artifact("debug/guard", "watcher_process_guard", False)
        with self.assertRaisesRegex(ValueError, "usable Watcher test executables"):
            stage_guard(self.messages, self.manifest)
        with self.assertRaisesRegex(ValueError, "successful fixture builds"):
            stage_guard(self.messages[1:], self.manifest)

    def test_probe_confirms_missing_guard_but_rejects_other_process_failures(self):
        self.artifact("debug/watcher_process_guard", "watcher_process_guard", False)
        host = self.artifact(
            "debug/build/telex-watcher/hash/out/process-test", "process_v2", True
        )
        result = SimpleNamespace(
            returncode=101,
            stdout="",
            stderr="start process guard: No such file or directory (os error 2)",
        )
        with patch("stage_watcher_test_guard.subprocess.run", return_value=result) as run:
            stage_guard(self.messages, self.manifest, diagnose=True)
        self.assertEqual(run.call_args.args[0][0], str(host.resolve()))
        (host.parent / "watcher_process_guard").unlink()
        result.stderr = "a detector assertion failed"
        with patch("stage_watcher_test_guard.subprocess.run", return_value=result):
            with self.assertRaisesRegex(ValueError, "unexpected pre-staging"):
                stage_guard(self.messages, self.manifest, diagnose=True)
        self.assertFalse((host.parent / "watcher_process_guard").exists())


if __name__ == "__main__":
    unittest.main()
