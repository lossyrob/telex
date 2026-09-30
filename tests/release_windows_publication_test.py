"""Native owned-file controls for the proof observer, not historical-cause attribution."""

import argparse
import ctypes
from ctypes import wintypes
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    "proof", Path(__file__).with_name("release_candidate.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)
EVENTS = []

if os.name == "nt":
    kernel = proof._kernel32
    kernel.MoveFileExW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD]
    kernel.MoveFileExW.restype = wintypes.BOOL
    kernel.DeleteFileW.argtypes = [wintypes.LPCWSTR]
    kernel.DeleteFileW.restype = wintypes.BOOL
    kernel.GetFileInformationByHandleEx.argtypes = [
        wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD,
    ]
    kernel.GetFileInformationByHandleEx.restype = wintypes.BOOL
    kernel.LocalFree.argtypes = [wintypes.HLOCAL]
    kernel.LocalFree.restype = wintypes.HLOCAL
    advapi = ctypes.WinDLL("advapi32", use_last_error=True)
    advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [
        wintypes.LPCWSTR, wintypes.DWORD, ctypes.POINTER(wintypes.LPVOID),
        ctypes.POINTER(wintypes.DWORD),
    ]
    advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.restype = wintypes.BOOL
    advapi.SetFileSecurityW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.LPVOID]
    advapi.SetFileSecurityW.restype = wintypes.BOOL


class StandardInfo(ctypes.Structure):
    _fields_ = [("allocation", ctypes.c_longlong), ("size", ctypes.c_longlong),
                ("links", wintypes.DWORD), ("delete_pending", wintypes.BOOLEAN),
                ("directory", wintypes.BOOLEAN)]


def event(operation, **data):
    EVENTS.append({"monotonic_ns": time.perf_counter_ns(), "owner_pid": os.getpid(),
                   "operation": operation, **data})


def opened(path, share):
    handle = kernel.CreateFileW(str(path), 0x80000000, share, None, 3, 0x80, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    event("owned-handle-open", path=str(path), share=share)
    return handle


def close(handle):
    if not kernel.CloseHandle(handle):
        raise ctypes.WinError(ctypes.get_last_error())
    event("owned-handle-close")


@unittest.skipUnless(os.name == "nt", "native Windows-only observation controls")
class WindowsPublicationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="telex-native-cap-")
        self.root = Path(self.tmp.name)
        self.path = self.root / "daemon-owned.cap"
        self.path.write_text('{"instance_id":"old"}', encoding="utf-8")

    def tearDown(self):
        self.tmp.cleanup()
        self.assertFalse(self.root.exists())
        event("owned-root-removed", path=str(self.root))

    def test_native_sharing_disambiguates_crt_permission_error(self):
        handle = opened(self.path, 0)
        try:
            with self.assertRaises(PermissionError) as old:
                self.path.read_text()
            self.assertEqual(old.exception.errno, 13)
            with self.assertRaises(OSError) as native:
                proof.read_identity_text(self.path)
            self.assertEqual(native.exception.winerror, 32)
            self.assertEqual(native.exception.operation, "CreateFileW")
            event("sharing-control", path=str(self.path), crt_errno=old.exception.errno,
                  native_winerror=native.exception.winerror)
        finally:
            close(handle)
        self.assertEqual(json.loads(proof.read_identity_text(self.path))["instance_id"], "old")

    def test_native_acl_denial_is_fatal_and_distinct(self):
        sid = subprocess.run(
            ["pwsh", "-NoProfile", "-NonInteractive", "-Command",
             "[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value"],
            check=True, capture_output=True, text=True).stdout.strip()
        self.assertRegex(sid, r"^S-1-\d+(?:-\d+)+$")

        def set_dacl(sddl):
            descriptor, size = wintypes.LPVOID(), wintypes.DWORD()
            if not advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl, 1, ctypes.byref(descriptor), ctypes.byref(size)):
                raise ctypes.WinError(ctypes.get_last_error())
            try:
                if not advapi.SetFileSecurityW(str(self.path), 4 | 0x80000000, descriptor):
                    raise ctypes.WinError(ctypes.get_last_error())
            finally:
                if kernel.LocalFree(descriptor):
                    raise RuntimeError("fixture descriptor release failed")

        set_dacl(f"D:P(D;;0x1;;;{sid})(A;;FA;;;{sid})")
        try:
            with self.assertRaises(PermissionError) as old:
                self.path.read_text()
            with self.assertRaises(OSError) as native:
                proof.read_identity_text(self.path)
            self.assertEqual(old.exception.errno, 13)
            self.assertEqual(native.exception.winerror, 5)
            event("permanent-acl-control", path=str(self.path), crt_errno=13, native_winerror=5)
        finally:
            set_dacl(f"D:P(A;;FA;;;{sid})")
        self.assertEqual(json.loads(proof.read_identity_text(self.path))["instance_id"], "old")

    def test_known_delete_pending_is_observed_not_reclassified_as_sharing(self):
        handle = opened(self.path, 7)
        try:
            if not kernel.DeleteFileW(str(self.path)):
                raise ctypes.WinError(ctypes.get_last_error())
            info = StandardInfo()
            if not kernel.GetFileInformationByHandleEx(
                    handle, 1, ctypes.byref(info), ctypes.sizeof(info)):
                raise ctypes.WinError(ctypes.get_last_error())
            self.assertTrue(info.delete_pending)
            with self.assertRaises(OSError) as failed:
                proof.read_identity_text(self.path)
            self.assertIn(failed.exception.winerror, (2, 5, 303))
            event("known-owned-delete-pending", path=str(self.path),
                  delete_pending=True, native_winerror=failed.exception.winerror)
        finally:
            close(handle)
        self.assertFalse(self.path.exists())

    def test_actual_sharing_window_then_replacement_preserves_identity_gate(self):
        old = {"server_pid": 42, "server_start_time": 100, "instance_id": "old"}
        new = {"server_pid": 43, "server_start_time": 200, "instance_id": "new"}
        self.path.write_text(json.dumps(old))
        pending = self.root / "new.cap.tmp"
        pending.write_text(json.dumps(new))
        report = {"daemons": [{"pid": 42, "start_time": 100, "instance_id": "old"}]}
        fixture = proof.Proof(SimpleNamespace(postgres_url=None), self.root, report)
        env = proof.clean_environment(self.root / "env")
        env["TELEX_RUN_DIR"] = str(self.root)
        fixture.daemons.append((SimpleNamespace(pid=42, poll=lambda: 0),
                                self.root / "old", env, None))
        handle = None
        status_calls = []

        def spawn(*_args, **_kwargs):
            nonlocal handle
            # Acquired only after the old publication was verified, inside the owned spawn window.
            handle = opened(self.path, 0)
            event("publication-window-held", path=str(self.path), expected_successor=43)
            return SimpleNamespace(pid=43, poll=lambda: None)

        def release_and_publish(_delay):
            nonlocal handle
            self.assertIsNotNone(handle)
            error = next(r for r in report["readiness_observations"] if r["event"] == "read-error")
            self.assertEqual(error["winerror"], 32)
            self.assertTrue(error["retryable"])
            self.assertEqual(status_calls, [])
            close(handle)
            handle = None
            if not kernel.MoveFileExW(str(pending), str(self.path), 1):
                raise ctypes.WinError(ctypes.get_last_error())
            event("successor-published", path=str(self.path), expected_successor=43)

        def status(*_args, **_kwargs):
            status_calls.append(True)
            return SimpleNamespace(stdout=json.dumps({
                "instance_id": "new", "protocol_version": {"major": 1, "minor": 5},
            }))

        with patch.object(proof.subprocess, "Popen", side_effect=spawn), \
                patch.object(proof.time, "sleep", side_effect=release_and_publish), \
                patch.object(fixture, "telex", side_effect=status):
            try:
                fixture.start_daemon(self.root / "candidate", env, 5)
                self.assertEqual(len(status_calls), 1)
                self.assertEqual(report["daemons"][-1]["instance_id"], "new")
                event("bounded-native-observer-accepted", observations=report["readiness_observations"])
            finally:
                if handle is not None:
                    close(handle)
                fixture.daemons[-1][3].close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--report", type=Path)
    args, remaining = parser.parse_known_args()
    program = unittest.main(argv=[sys.argv[0], *remaining], exit=False)
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps({
            "status": "passed" if program.result.wasSuccessful() else "failed",
            "tests_run": program.result.testsRun, "skipped": len(program.result.skipped),
            "events": EVENTS,
            "scope": "Real native file/handle/ACL/publication controls; daemon/Status are synthetic in ordering test. Does not identify the historical hosted Errno13 cause.",
        }, indent=2) + "\n", encoding="utf-8")
    sys.exit(not program.result.wasSuccessful())
