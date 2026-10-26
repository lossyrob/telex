"""Synthetic source/controller checks; no real tools, provider or network."""

from contextlib import contextmanager
from email.message import Message
import copy
import hashlib
import http.client
import importlib.util
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
import urllib.response
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("derived_build", HERE / "build.py")
b = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(b)
BOUNDS = json.loads((HERE / "bounds.json").read_bytes())


class Response(io.BytesIO):
    def __init__(self, url, raw=b"", status=200, headers=None):
        super().__init__(raw)
        self.url, self.status = url, status
        self.headers = Message()
        for key, value in (headers or {}).items():
            self.headers[key] = str(value)

    def geturl(self):
        return self.url


class FetchTests(unittest.TestCase):
    @contextmanager
    def scenario(self, replies, cap=16, clock=None):
        with tempfile.TemporaryDirectory() as temp:
            limits = dict(BOUNDS["limits"], module_transfer_bytes=cap, read_bytes=4)
            calls, ledger = [], []
            def opened(request, timeout):
                calls.append({"url": request.full_url, "headers": dict(request.header_items()), "timeout": timeout})
                return replies.pop(0)
            with patch.object(b.urllib.request, "build_opener",
                              return_value=SimpleNamespace(open=opened)), \
                    patch("socket.socket", side_effect=AssertionError("no sockets")), \
                    patch.object(b.time, "monotonic", side_effect=clock or (lambda: 0)):
                fetcher = b.Fetcher(limits, 100, ledger)
                yield fetcher, Path(temp), calls, ledger

    def test_exact_payload_bound_and_no_authorization_or_decoding(self):
        url = "https://proxy.golang.org/example/@v/v1.0.0.mod"
        with self.scenario([Response(url, b"abcdefgh", headers={"Content-Length": 8})], cap=8) as (f, root, calls, ledger):
            record = f.fetch(url, root / "data", "module", ["proxy.golang.org"])
            self.assertEqual(record["bytes"], 8)
            self.assertEqual((root / "data").read_bytes(), b"abcdefgh")
            self.assertEqual(f.counts["module"], 8)
            self.assertEqual(calls[0]["headers"], {"Accept-encoding": "identity"})
            self.assertIsNone(ledger[0]["error"])

    def test_redirect_bodies_each_count_and_sequential_exact_hosts(self):
        first, second = "https://proxy.golang.org/x", "https://storage.googleapis.com/x"
        for status in (301, 302, 303, 307, 308):
            with self.subTest(status=status), self.scenario([
                    Response(first, b"hop", status, {"Location": second}),
                    Response(second, b"abcde")], cap=8) as (f, root, calls, ledger):
                f.fetch(first, root / "data", "module", ["proxy.golang.org", "storage.googleapis.com"])
                self.assertEqual([c["url"] for c in calls], [first, second])
                self.assertEqual(f.counts["module"], 8)
                self.assertEqual([r["payload_bytes"] for r in ledger], [3, 5])

    def test_cross_request_budget_and_remaining_plus_one_fail(self):
        url = "https://proxy.golang.org/x"
        with self.scenario([Response(url, b"1234"), Response(url, b"abcdefghi")], cap=8) as (f, root, calls, ledger):
            f.fetch(url, root / "one", "module", ["proxy.golang.org"])
            with self.assertRaisesRegex(b.Error, "overflow"):
                f.fetch(url, root / "two", "module", ["proxy.golang.org"])
            self.assertEqual(f.counts["module"], 9)
            self.assertEqual((root / "two.hop-0.partial").read_bytes(), b"abcde")
            self.assertEqual(ledger[-1]["payload_bytes"], 5)
            self.assertEqual(len(calls), 2)
            self.assertFalse((root / "two").exists())

    def test_declared_length_exceeding_remaining_aborts_before_body_read(self):
        url = "https://proxy.golang.org/x"
        response = Response(url, b"too long", headers={"Content-Length": 9})
        with self.scenario([response], cap=8) as (f, root, _, ledger), \
                patch.object(response, "read1", side_effect=AssertionError("must not read")):
            with self.assertRaisesRegex(b.Error, "Content-Length"):
                f.fetch(url, root / "data", "module", ["proxy.golang.org"])
            self.assertEqual(f.counts["module"], 0)
            self.assertEqual(ledger[0]["payload_bytes"], 0)

    def test_errors_encoded_and_truncated_bodies_count_before_failure(self):
        url = "https://proxy.golang.org/x"
        for status, headers in ((404, {}), (200, {"Content-Encoding": "gzip"}),
                                (200, {"Content-Length": 8})):
            with self.subTest(status=status, headers=headers), self.scenario([
                    Response(url, b"\x00\xffbody", status, headers)]) as (f, root, calls, ledger):
                with self.assertRaises(b.Error):
                    f.fetch(url, root / "data", "module", ["proxy.golang.org"])
                self.assertEqual(f.counts["module"], 6)
                self.assertEqual((root / "data.hop-0.partial").read_bytes(), b"\x00\xffbody")
                self.assertEqual(len(calls), 1)
                self.assertIsNotNone(ledger[0]["error"])

    def test_incomplete_read_partial_is_counted_and_retained(self):
        url = "https://proxy.golang.org/x"
        response = Response(url)
        with self.scenario([response]) as (f, root, _, ledger), \
                patch.object(response, "read1", side_effect=http.client.IncompleteRead(b"abc", 2)):
            with self.assertRaises(http.client.IncompleteRead):
                f.fetch(url, root / "data", "module", ["proxy.golang.org"])
            self.assertEqual(f.counts["module"], 3)
            self.assertEqual((root / "data.hop-0.partial").read_bytes(), b"abc")
            self.assertEqual(ledger[0]["payload_bytes"], 3)

    def test_empty_incomplete_read_is_not_accepted_as_EOF(self):
        url = "https://proxy.golang.org/x"
        response = Response(url)
        with self.scenario([response]) as (f, root, _, ledger), \
                patch.object(response, "read1", side_effect=http.client.IncompleteRead(b"", 2)):
            with self.assertRaises(http.client.IncompleteRead):
                f.fetch(url, root / "data", "module", ["proxy.golang.org"])
            self.assertEqual(ledger[0]["payload_bytes"], 0)
            self.assertEqual(ledger[0]["error"]["type"], "IncompleteRead")

    def test_undeclared_redirect_is_never_requested(self):
        url = "https://proxy.golang.org/x"
        for destination in ("https://example.invalid/x", "http://proxy.golang.org/x",
                            "https://user:pass@proxy.golang.org/x", "https://proxy.golang.org:444/x"):
            with self.subTest(destination=destination), self.scenario([
                    Response(url, b"redirect", 302, {"Location": destination})]) as (f, root, calls, _):
                with self.assertRaises(b.Error):
                    f.fetch(url, root / "data", "module", ["proxy.golang.org"])
                self.assertEqual(len(calls), 1)
                self.assertEqual(f.counts["module"], 8)

    def test_redirect_hop_bound_no_retry(self):
        url = "https://proxy.golang.org/x"
        with self.scenario([Response(url, b"x", 302, {"Location": url}) for _ in range(4)]) as (f, root, calls, ledger):
            with self.assertRaisesRegex(b.Error, "hop bound"):
                f.fetch(url, root / "data", "module", ["proxy.golang.org"])
            self.assertEqual(len(calls), 4)
            self.assertEqual(f.counts["module"], 4)

    def test_go_archive_file_cap_is_separate_from_module_counter(self):
        url = "https://go.dev/dl/archive"
        with self.scenario([Response(url, b"abcdef")]) as (f, root, _, ledger):
            with self.assertRaisesRegex(b.Error, "overflow"):
                f.fetch(url, root / "go.tar.gz", "compiler", ["go.dev"], file_limit=4)
            self.assertEqual(f.counts["module"], 0)
            self.assertEqual(f.counts["compiler"], 5)

    def test_deadline_no_fetch_and_stream_write_failure_is_not_retried(self):
        url = "https://proxy.golang.org/x"
        with self.scenario([], clock=lambda: 101) as (f, root, calls, _):
            with self.assertRaisesRegex(b.Error, "deadline"):
                f.fetch(url, root / "x", "module", ["proxy.golang.org"])
            self.assertFalse(calls)
        with self.scenario([Response(url, b"data")]) as (f, root, calls, ledger), \
                patch.object(Path, "open", side_effect=OSError("ENOSPC")):
            with self.assertRaisesRegex(OSError, "ENOSPC"):
                f.fetch(url, root / "x", "module", ["proxy.golang.org"])
            self.assertEqual(len(calls), 1)
            self.assertEqual(ledger[0]["error"]["type"], "OSError")

    def test_standard_opener_returns_redirect_body_for_explicit_metering(self):
        original = b.urllib.request.build_opener
        calls = []
        class Transport(b.urllib.request.BaseHandler):
            handler_order = 0
            def https_open(self, request):
                calls.append(request.full_url)
                headers = Message()
                headers["Content-Length"] = "3"
                headers["Location"] = "https://storage.googleapis.com/next"
                status, raw = (302, b"hop") if len(calls) == 1 else (200, b"end")
                response = urllib.response.addinfourl(io.BytesIO(raw), headers, request.full_url, status)
                response.msg = "synthetic"
                return response
        with tempfile.TemporaryDirectory() as temp, patch("socket.socket", side_effect=AssertionError("no sockets")), \
                patch.object(b.urllib.request, "build_opener", side_effect=lambda *handlers: original(*handlers, Transport())):
            ledger = []
            f = b.Fetcher(BOUNDS["limits"], float("inf"), ledger)
            f.fetch("https://proxy.golang.org/first", Path(temp) / "file", "module",
                    ["proxy.golang.org", "storage.googleapis.com"])
            self.assertEqual(calls, ["https://proxy.golang.org/first", "https://storage.googleapis.com/next"])
            self.assertEqual([r["payload_bytes"] for r in ledger], [3, 3])
            self.assertEqual(f.counts["module"], 6)


class InputTests(unittest.TestCase):
    def test_frozen_plan_is_exact_deterministic_go_sum_derivation(self):
        sums = (HERE / "inputs" / "go.sum").read_bytes()
        frozen = json.loads((HERE / "inputs" / "module-requests.json").read_bytes())
        requests = b.request_plan(sums)
        self.assertEqual(frozen["requests"], requests)
        self.assertEqual(frozen["go_sum_sha256"], b.digest(sums))
        self.assertEqual(len(requests), 1856)
        self.assertEqual({kind: sum(r["kind"] == kind for r in requests) for kind in ("mod", "zip", "info")},
                         {"mod": 1200, "zip": 328, "info": 328})
        zip_versions = {(r["module"], r["version"]) for r in requests if r["kind"] == "zip"}
        self.assertEqual(zip_versions, {(r["module"], r["version"]) for r in requests if r["kind"] == "info"})
        self.assertTrue(all("@latest" not in r["url"] and "@v/list" not in r["url"] and
                            r["url"].startswith("https://proxy.golang.org/") for r in requests))

    def test_frozen_tree_reconstructs_exact_upstream_Git_root(self):
        tree = json.loads((HERE / "inputs" / "source-tree.json").read_bytes())
        self.assertEqual(b.tree_identity(tree["files"]), BOUNDS["upstream"]["tree"])
        self.assertEqual(tree["commit"], BOUNDS["upstream"]["commit"])
        self.assertEqual(sum(f["bytes"] for f in tree["files"]), 26011281)
        with self.assertRaises(b.Error):
            b.tree_identity(tree["files"] + tree["files"][:1])

    def test_uppercase_module_escape_and_duplicate_or_traversal_refused(self):
        raw = b"example.com/Upper v1.2.3 h1:YWJj=\nexample.com/Upper v1.2.3/go.mod h1:YWJj=\n"
        self.assertTrue(all("example.com/!upper/@v/v1.2.3" in r["url"] for r in b.request_plan(raw)))
        for bad in (raw + raw, b"../x v1.0.0 h1:YWJj=\n", b"x latest h1:YWJj=\n"):
            with self.subTest(bad=bad), self.assertRaises(b.Error):
                b.request_plan(bad)

    def test_exact_approved_patch_modules_and_licenses_are_pinned(self):
        u = BOUNDS["upstream"]
        self.assertEqual(len((HERE / "get-build-info.patch").read_bytes()), 1747)
        self.assertEqual(b.digest((HERE / "get-build-info.patch").read_bytes()), u["patch_sha256"])
        self.assertEqual(b.digest((HERE / "inputs" / "go.mod").read_bytes()), u["go_mod_sha256"])
        self.assertEqual(b.digest((HERE / "inputs" / "go.sum").read_bytes()), u["go_sum_sha256"])
        self.assertIn(b"Permission is hereby granted", (HERE / "inputs" / "LICENSE-ipsw").read_bytes())
        self.assertIn(b"Redistribution and use", (HERE / "inputs" / "LICENSE-Go").read_bytes())

    def make_archive(self, file, names):
        with tarfile.open(file, "w:gz") as archive:
            for name, kind, raw in names:
                info = tarfile.TarInfo(name)
                info.size, info.mode = len(raw), 0o644
                if kind == "symlink":
                    info.type, info.linkname = tarfile.SYMTYPE, "/outside"
                archive.addfile(info, io.BytesIO(raw))

    def test_safe_exact_archive_and_expanded_cap(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.make_archive(root / "x.tar.gz", [("root/file", "file", b"abcd")])
            self.assertEqual(b.unpack(root / "x.tar.gz", root / "out", "root", 4, float("inf")), 4)
            self.assertEqual((root / "out" / "file").read_bytes(), b"abcd")
            with self.assertRaisesRegex(b.Error, "expanded"):
                b.unpack(root / "x.tar.gz", root / "small", "root", 3, float("inf"))

    def test_archive_links_duplicates_and_escape_fail(self):
        for rows in ([("root/../../escape", "file", b"x")], [("/root/x", "file", b"x")],
                     [("other/x", "file", b"x")], [("root/x", "symlink", b"")],
                     [("root/x", "file", b"x"), ("root/x", "file", b"x")]):
            with self.subTest(rows=rows), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                self.make_archive(root / "x.tar.gz", rows)
                with self.assertRaises(b.Error):
                    b.unpack(root / "x.tar.gz", root / "out", "root", 100, float("inf"))

    def test_complete_git_blob_tree_and_only_exact_patch_candidate(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            file = root / "internal" / "utils" / "macos.go"
            file.parent.mkdir(parents=True)
            file.write_bytes(b"parent")
            file.chmod(0o644)
            blob = hashlib.sha1(b"blob 6\0parent").hexdigest()
            tree = {"files": [{"path": "internal/utils/macos.go", "mode": "100644", "sha": blob, "bytes": 6}]}
            b.verify_tree(root, tree, False, b.digest(b"candidate"), float("inf"))
            file.write_bytes(b"candidate")
            b.verify_tree(root, tree, True, b.digest(b"candidate"), float("inf"))
            with self.assertRaises(b.Error):
                b.verify_tree(root, tree, False, b.digest(b"candidate"), float("inf"))
            (root / "extra").write_bytes(b"x")
            with self.assertRaises(b.Error):
                b.verify_tree(root, tree, True, b.digest(b"candidate"), float("inf"))


class CapacityEnvironmentTests(unittest.TestCase):
    def test_reserve_boundary_records_logical_and_allocated_not_live_floor(self):
        limits = {"module_and_build_disk_bytes": 12000, "disk_reserve_bytes": 4000}
        info = SimpleNamespace(st_size=12000, st_blocks=10)
        volume = SimpleNamespace(f_blocks=11, f_frsize=1000)
        for available in (10879, 10880):
            fs = SimpleNamespace(f_bavail=available, f_frsize=1)
            record = b.volume_accounting(info, volume, fs, limits)
            self.assertEqual(record["required_initial_available"], 10880)
            self.assertEqual(record["initial_reserve_satisfied"], available == 10880)
            self.assertTrue(record["not_live_floor_or_all_host_writes_bound"])

    def test_volume_or_backing_over_limit_and_zero_fail(self):
        limits = {"module_and_build_disk_bytes": 12, "disk_reserve_bytes": 4}
        for size, capacity in ((13, 12), (12, 13), (0, 12), (12, 0)):
            with self.subTest(size=size, capacity=capacity), self.assertRaises(b.Error):
                b.volume_accounting(SimpleNamespace(st_size=size, st_blocks=0),
                    SimpleNamespace(f_blocks=capacity, f_frsize=1),
                    SimpleNamespace(f_bavail=100, f_frsize=1), limits)

    def test_file_only_offline_environment_and_all_build_paths_inside_volume(self):
        env = b.build_environment({}, BOUNDS, Path("/private/volume"))
        self.assertEqual(env["GOPROXY"], "file:///private/volume/proxy")
        for key in ("HOME", "TMPDIR", "GOTMPDIR", "GOMODCACHE", "GOCACHE", "GOPATH"):
            self.assertTrue(Path(env[key]).is_relative_to("/private/volume"))
        self.assertEqual(env["GOVCS"], "*:off")
        self.assertEqual(env["GOSUMDB"], "off")
        self.assertEqual(env["GOFLAGS"], "-mod=readonly")
        self.assertEqual(env["CXX"], BOUNDS["tools"]["clangxx_nominal"])
        self.assertFalse(any("direct" in env[key] for key in ("GOPROXY", "GONOPROXY")))
        self.assertNotIn("GH_TOKEN", env)

    def test_conflicting_build_search_injection_or_network_overrides_fail(self):
        for key in ("GOFLAGS", "GOEXPERIMENT", "GOTOOLCHAIN", "CGO_CFLAGS", "CC", "CXX",
                    "DYLD_INSERT_LIBRARIES", "LD_LIBRARY_PATH", "CPATH", "SDKROOT", "HTTPS_PROXY"):
            with self.subTest(key=key), self.assertRaises(b.Error):
                b.build_environment({key: "wrong"}, BOUNDS, Path("/private/volume"))

    def test_fixed_nonsparse_image_and_no_privileged_fallback(self):
        args = BOUNDS["volume"]["create"]
        self.assertIn("UDRW", args)
        self.assertNotIn("SPARSE", " ".join(args))
        self.assertEqual(BOUNDS["volume"]["attach"][1:4], ["-readwrite", "-nobrowse", "-noautoopen"])
        self.assertEqual(BOUNDS["runner"]["label"], "macos-26")
        self.assertEqual(BOUNDS["retries"], 0)

    def test_compiler_alias_and_first_hash_rule_rechecked_without_runtime_adoption(self):
        provenance, paths = {}, []
        tools = BOUNDS["tools"]
        runner = SimpleNamespace(deadline=float("inf"), phase_deadline=float("inf"))
        def command(label, argv, env):
            return (b"Apple clang version synthetic\n", b"") if label == "clang-version" else (b"26.5\n", b"")
        runner.command = command
        def resolved(file, strict=False):
            paths.append(str(file))
            self.assertTrue(strict)
            return Path(tools["clang"]) if str(file) == tools["clangxx_nominal"] else file
        def hashed(file, maximum, deadline):
            return {"path": str(file), "bytes": 1, "sha256": "first-observed"}
        with patch.object(Path, "resolve", resolved), patch.object(b, "file_hash", side_effect=hashed):
            b.bind_tools(BOUNDS, runner, {}, provenance, "before")
            b.bind_tools(BOUNDS, runner, {}, provenance, "after")
        self.assertEqual(provenance["tool_binding_before"], provenance["tool_binding_after"])
        self.assertEqual(provenance["tool_binding_before"]["clangxx_relation"]["resolved"], tools["clang"])
        self.assertEqual(paths.count(tools["clangxx_nominal"]), 2)
        self.assertNotIn("upstream_release_hash", provenance)
        with patch.object(Path, "resolve", resolved), patch.object(b, "file_hash",
                side_effect=lambda file, *args: {"path": str(file), "bytes": 1, "sha256": "changed"}), \
                self.assertRaisesRegex(b.Error, "changed"):
            b.bind_tools(BOUNDS, runner, {}, provenance, "after")

    def test_other_tool_or_CXX_or_SDK_alias_fails_and_retains_operand(self):
        for key in ("clang", "clangxx_nominal", "linker", "hdiutil", "SDKROOT"):
            with self.subTest(key=key):
                provenance = {}
                wrong = BOUNDS["compiler_identity"]["SDKROOT"] if key == "SDKROOT" else BOUNDS["tools"][key]
                def resolved(file, strict=False):
                    if str(file) == wrong:
                        return Path("/synthetic/unapproved")
                    return Path(BOUNDS["tools"]["clang"]) if str(file) == BOUNDS["tools"]["clangxx_nominal"] else file
                runner = SimpleNamespace(deadline=float("inf"), phase_deadline=float("inf"))
                with patch.object(Path, "resolve", resolved), patch.object(b, "file_hash",
                        side_effect=lambda file, *args: {"path": str(file), "bytes": 1, "sha256": "first"}), \
                        self.assertRaises(b.Error):
                    b.bind_tools(BOUNDS, runner, {}, provenance, "before")
                self.assertIn("/synthetic/unapproved", json.dumps(provenance))


class GateTests(unittest.TestCase):
    def environment(self):
        return {"GITHUB_EVENT_NAME": "push", "GITHUB_RUN_ATTEMPT": "1", "GITHUB_REPOSITORY": "lossyrob/telex",
                "GITHUB_REF": "refs/heads/" + BOUNDS["branch"], "GITHUB_SHA": "a" * 40,
                "GITHUB_WORKFLOW_REF": "lossyrob/telex/" + BOUNDS["workflow"] + "@refs/heads/" + BOUNDS["branch"],
                "GH_TOKEN": "synthetic"}

    def comments(self, **updates):
        record = {"component": BOUNDS["component"], "sha": "a" * 40, "bounds_sha256": "b" * 64,
                  "source_manifest_sha256": "c" * 64, "jobs": BOUNDS["jobs"], "attempt": 1,
                  "allow_launch": True, "source_review": "https://github.com/lossyrob/telex/issues/172#issuecomment-123",
                  "source_review_body_sha256": b.digest(b"synthetic approved body")}
        record.update(updates)
        return {"user": {"login": "lossyrob"}, "body": "<!-- macos-ipsw-derived-build-v1 " + json.dumps(record) + " -->",
                "html_url": "https://github.com/lossyrob/telex/issues/172#issuecomment-456"}

    def test_exact_separate_BUILD_freeze_and_review_before_provider(self):
        review = {"user": {"login": "lossyrob"}, "html_url": "https://github.com/lossyrob/telex/issues/172#issuecomment-123",
                  "issue_url": "https://api.github.com/repos/lossyrob/telex/issues/172", "body": "synthetic approved body"}
        with tempfile.TemporaryDirectory() as temp, patch.dict(os.environ, self.environment(), clear=True), \
                patch.object(b.github, "fetch_json", side_effect=[[self.comments()], review]), \
                patch.object(b.subprocess, "Popen") as provider:
            out = b.Evidence(Path(temp), BOUNDS["limits"])
            self.assertEqual(b.check_gate(BOUNDS, "b" * 64, "c" * 64, out, 100), "a" * 40)
            provider.assert_not_called()
            self.assertFalse(json.loads((Path(temp) / "gate.json").read_bytes())["provider_queries_yet"])

    def test_duplicates_false_wrong_closure_or_other_component_fail(self):
        wrong = self.comments()
        wrong["body"] = wrong["body"].replace("macos-ipsw-derived-build-v1", "macos-sw-vers-diagnostic-v1")
        for rows in ([], [wrong], [self.comments(), self.comments(allow_launch=False)],
                     [self.comments(allow_launch=False)], [self.comments(source_manifest_sha256="wrong")]):
            with self.subTest(rows=rows), patch.dict(os.environ, self.environment(), clear=True), \
                    patch.object(b.github, "fetch_json", return_value=rows), self.assertRaises(b.Error):
                b.check_gate(BOUNDS, "b" * 64, "c" * 64, None, 100)

    def test_attempt2_dispatch_wrong_branch_fail_without_API_or_provider(self):
        for key, value in (("GITHUB_RUN_ATTEMPT", "2"), ("GITHUB_EVENT_NAME", "workflow_dispatch"),
                           ("GITHUB_REF", "refs/heads/main"), ("GITHUB_WORKFLOW_REF", "other")):
            env = self.environment()
            env[key] = value
            with self.subTest(key=key), patch.dict(os.environ, env, clear=True), \
                    patch.object(b.github, "fetch_json") as fetch, self.assertRaises(b.Error):
                b.check_gate(BOUNDS, "b" * 64, "c" * 64, None, 100)
            fetch.assert_not_called()

    def test_execute_gate_failure_precedes_uid_host_compiler_fetch_or_volume(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(b, "check_gate", side_effect=b.Error("not frozen")), \
                patch.object(b.os, "getuid") as uid, patch.object(b.Runner, "command") as cmd, \
                patch.object(b.Fetcher, "fetch") as fetch, patch.object(b, "bind_tools") as tools:
            root = Path(temp)
            out = b.Evidence(root, BOUNDS["limits"])
            runner = b.Runner(BOUNDS, root, out, float("inf"))
            with self.assertRaisesRegex(b.Error, "not frozen"):
                b.execute(BOUNDS, out, root, {}, runner, [])
            for mock in (uid, cmd, fetch, tools):
                mock.assert_not_called()

    def test_failed_initial_reserve_precedes_every_payload_fetch(self):
        with tempfile.TemporaryDirectory() as temp:
            work, evidence = Path(temp) / "work", Path(temp) / "evidence"
            work.mkdir(); evidence.mkdir()
            out = b.Evidence(evidence, BOUNDS["limits"])
            runner = b.Runner(BOUNDS, work, out, float("inf"))
            mount = work / BOUNDS["volume"]["mount_name"]
            records = {}
            def command(label, argv, env, *args):
                if label == "host-uname": return b"Darwin synthetic\n", b""
                if label == "host-product": return b"26.6.2\n", b""
                if label == "host-build": return b"25G83\n", b""
                if label == "source": return b"synthetic\n", b""
                if label == "image-create":
                    (work / "build.dmg").write_bytes(b"synthetic-image")
                    return b"", b""
                if label == "image-attach":
                    return b.plistlib.dumps({"system-entities": [{"mount-point": str(mount)}]}), b""
                self.fail(label)
            original_stat = Path.stat
            def stat_path(file, *args, **kwargs):
                if file == mount: return SimpleNamespace(st_dev=123)
                if file == work: return SimpleNamespace(st_dev=456)
                return original_stat(file, *args, **kwargs)
            env = {"ImageVersion": "synthetic-image"}
            with patch.dict(os.environ, env, clear=True), patch.object(b, "check_gate", return_value="synthetic"), \
                    patch.object(b.sys, "platform", "darwin"), patch.object(b.os, "getuid", return_value=501), \
                    patch.object(b.os, "uname", return_value=SimpleNamespace(machine="arm64")), \
                    patch.object(b, "bind_tools"), patch.object(runner, "command", side_effect=command), \
                    patch.object(Path, "stat", stat_path), patch.object(b.os, "statvfs", return_value=object()), \
                    patch.object(b, "volume_accounting", return_value={"initial_reserve_satisfied": False}), \
                    patch.object(b.Fetcher, "fetch") as fetch, self.assertRaisesRegex(b.Error, "reserve"):
                b.execute(BOUNDS, out, work, records, runner, [])
            fetch.assert_not_called()
            self.assertFalse(records["volume_initial"]["initial_reserve_satisfied"])


class Pipe:
    def __init__(self, number):
        self.number, self.closed = number, False
    def fileno(self):
        return self.number
    def close(self):
        self.closed = True


class Process:
    def __init__(self, args):
        self.args, self.pid, self.stdout, self.stderr = args, 1234, Pipe(1), Pipe(2)
        self.waited = False
    def wait(self, timeout):
        self.waited = True
        return 0


class Poller:
    def __init__(self):
        self.keys = {}
    def register(self, file, events, name):
        self.keys[file.number] = SimpleNamespace(fileobj=file, data=name)
    def get_map(self):
        return self.keys
    def select(self, timeout):
        return [(key, 1) for key in list(self.keys.values())]
    def unregister(self, file):
        self.keys.pop(file.number)
    def close(self):
        pass


class RunnerTests(unittest.TestCase):
    @contextmanager
    def fake(self, raw=b"static", cap=100):
        with tempfile.TemporaryDirectory() as temp:
            bounds = copy.deepcopy(BOUNDS)
            bounds["limits"]["per_command_stream_bytes"] = cap
            root = Path(temp)
            runner = b.Runner(bounds, root, b.Evidence(root, bounds["limits"]), 1000)
            process = Process(["/fixed/tool"])
            streams = {1: bytearray(raw), 2: bytearray()}
            def read(fd, maximum):
                chunk = bytes(streams[fd][:maximum])
                del streams[fd][:maximum]
                return chunk
            with patch.object(b.subprocess, "Popen", return_value=process) as spawn, \
                    patch.object(b.selectors, "DefaultSelector", return_value=Poller()), \
                    patch.object(b.os, "read", side_effect=read), \
                    patch.object(b.os, "killpg") as kill, patch.object(b.time, "monotonic", return_value=0):
                yield runner, process, spawn, kill, root

    def test_owned_static_command_argv_rawlogs_and_no_decoder_execution(self):
        with self.fake() as (runner, process, spawn, kill, root):
            self.assertEqual(runner.command("static", ["/fixed/tool"], {}), (b"static", b""))
            self.assertTrue(spawn.call_args.kwargs["start_new_session"])
            self.assertEqual((root / "cmd-00-stdout.bin").read_bytes(), b"static")
            kill.assert_not_called()
            self.assertTrue(runner.records[0]["reaped"])
        with self.fake() as (runner, _, spawn, _, _), self.assertRaises(b.Error):
            runner.command("forbidden", ["/private/ipsw", "--version"], {})
        spawn.assert_not_called()

    def test_overflow_group_signal_is_before_exact_reap_and_never_after(self):
        with self.fake(b"abcdefgh", cap=4) as (runner, process, _, kill, root):
            kill.side_effect = lambda *args: self.assertFalse(process.waited)
            with self.assertRaises(b.Error):
                runner.command("overflow", ["/fixed/tool"], {})
            self.assertEqual((root / "cmd-00-stdout.bin").read_bytes(), b"abcde")
            kill.assert_called_once_with(1234, b.signal.SIGKILL)
            self.assertTrue(process.waited)
            self.assertTrue(runner.records[0]["group_kill_before_reap"])

    def test_negative_phase_budget_never_spawns(self):
        with self.fake() as (runner, _, spawn, _, _), self.assertRaises(b.Error):
            runner.command("late", ["/fixed/tool"], {}, "compile", -1)
        spawn.assert_not_called()

    def test_compile_and_signing_request_per_file_limit_without_running_it(self):
        for label in ("compile", "ad-hoc-sign"):
            with self.subTest(label=label), self.fake() as (runner, _, spawn, _, _), \
                    patch.object(b.resource, "setrlimit") as setting:
                runner.command(label, ["/fixed/tool"], {})
                bound = spawn.call_args.kwargs["preexec_fn"]
                self.assertIsNotNone(bound)
                setting.assert_not_called()
                bound()
                setting.assert_called_once_with(b.resource.RLIMIT_FSIZE,
                    (BOUNDS["limits"]["binary_bytes"], BOUNDS["limits"]["binary_bytes"]))

    def test_metadata_phase_charges_noncommand_time(self):
        with self.fake() as (runner, _, _, _, _):
            with patch.object(b.time, "monotonic", side_effect=[0, 181, 181]):
                with self.assertRaises(b.Error):
                    with b.metadata_phase(runner):
                        pass
            self.assertEqual(runner.metadata_seconds, 181)

    def test_spawn_failure_and_nonzero_never_signal_reaped_or_unknown_group(self):
        with self.fake() as (runner, _, _, kill, _), \
                patch.object(b.subprocess, "Popen", side_effect=OSError("spawn failed")):
            with self.assertRaises(b.Error):
                runner.command("failed", ["/fixed/tool"], {})
            kill.assert_not_called()
            self.assertIsNone(runner.records[0]["pid"])
        with self.fake() as (runner, process, _, kill, _), patch.object(process, "wait", return_value=1):
            with self.assertRaises(b.Error):
                runner.command("nonzero", ["/fixed/tool"], {})
            kill.assert_not_called()
            self.assertTrue(runner.records[0]["reaped"])

    def test_failed_owned_group_cleanup_is_explicit(self):
        with self.fake(b"overflow", cap=1) as (runner, process, _, kill, _):
            kill.side_effect = PermissionError("synthetic kill denied")
            with patch.object(process, "wait", side_effect=subprocess.TimeoutExpired("fixture", 10)), \
                    self.assertRaises(b.Error):
                runner.command("overflow", ["/fixed/tool"], {})
            self.assertFalse(runner.records[0]["reaped"])
            self.assertTrue(any("reap" in e for e in runner.records[0]["errors"]))


def synthetic_macho(identifier, flags=2, entitlements=False, cpu=0x100000C):
    ident = identifier.encode() + b"\0"
    cd = struct.pack(">6I", 0xFADE0C02, 24 + len(ident), 0x20400, flags, 24, 24) + ident
    count = 2 if entitlements else 1
    offset = 12 + 8 * count
    extra = struct.pack(">2I", 0xFADE7171, 8) if entitlements else b""
    indexes = struct.pack(">2I", 0, offset)
    if entitlements:
        indexes += struct.pack(">2I", 5, offset + len(cd))
    sig = struct.pack(">3I", 0xFADE0CC0, offset + len(cd) + len(extra), count) + indexes + cd + extra
    return struct.pack("<8I", 0xFEEDFACF, cpu, 0, 2, 1, 16, 0, 0) + struct.pack("<4I", 0x1D, 16, 48, len(sig)) + sig


class SignatureOutputTests(unittest.TestCase):
    def test_static_signed_ARM64_identity_and_no_binary_invocation(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(b.subprocess, "Popen") as process:
            file = Path(temp) / "ipsw"
            file.write_bytes(synthetic_macho(BOUNDS["signing"]["identifier"]))
            result = b.macho_signature(file, 10000, float("inf"), True, BOUNDS["signing"]["identifier"])
            self.assertEqual(result["code_directory_flags"], [2])
            self.assertEqual(result["sha256"], b.digest(file.read_bytes()))
            process.assert_not_called()

    def test_wrong_arch_identity_entitlement_runtime_or_missing_adhoc_fails(self):
        for kwargs in ({"identifier": "wrong"}, {"flags": 0}, {"flags": 0x10002},
                       {"entitlements": True}, {"cpu": 0x1000007}):
            with self.subTest(kwargs=kwargs), tempfile.TemporaryDirectory() as temp:
                file = Path(temp) / "ipsw"
                file.write_bytes(synthetic_macho(**{"identifier": BOUNDS["signing"]["identifier"], **kwargs}))
                with self.assertRaises(b.Error):
                    b.macho_signature(file, 10000, float("inf"), True, BOUNDS["signing"]["identifier"])

    def test_derived_go_metadata_not_upstream_version_and_not_execution(self):
        flags = next(f.split("=", 1)[1] for f in BOUNDS["build"]["flags"] if f.startswith("-ldflags="))
        raw = ("file: go1.27.3\npath\tgithub.com/blacktop/ipsw/cmd/ipsw\n" +
               "build\t-ldflags=" + json.dumps(flags) +
               "\nbuild\tGOOS=darwin\nbuild\tGOARCH=arm64\nbuild\tCGO_ENABLED=1\n"
               "build\t-trimpath=true\nbuild\t-tags=sandbox\nbuild\t-buildmode=exe\n").encode()
        self.assertEqual(b.go_metadata(raw, BOUNDS)["settings"]["-ldflags"], flags)
        for wrong in (raw.replace(b"GOARCH=arm64", b"GOARCH=amd64"),
                      raw.replace(b"GOARCH=arm64", b"GOARCH=arm64-extra"),
                      raw + b"build\tGOARCH=arm64\n",
                      raw.replace(BOUNDS["build"]["version"].encode(), b"3.1.739")):
            with self.assertRaises(b.Error):
                b.go_metadata(wrong, BOUNDS)

    def test_artifact_limits_overwrite_and_source_closure(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            out = b.Evidence(root, dict(BOUNDS["limits"], other_artifact_file_bytes=4, artifact_total_bytes=5))
            out.write("raw.bin", b"1234")
            for name, raw in (("large", b"12345"), ("raw.bin", b"x"), ("other", b"xx")):
                with self.subTest(name=name), self.assertRaises(b.Error):
                    out.write(name, raw)
            self.assertEqual((root / "raw.bin").read_bytes(), b"1234")
        rows, identity = b.closure(BOUNDS)
        self.assertEqual(len(rows), 13)
        self.assertEqual(identity, b.digest(b.encoded(rows)))
        self.assertIn("diagnostics/macos_sw_vers/diagnostic.py", [r["file"] for r in rows])

    def test_archive_hard_output_cap_is_checked_during_compression(self):
        with tempfile.TemporaryDirectory() as temp:
            file = Path(temp) / "fixture"
            file.write_bytes(bytes(range(256)) * 20)
            with self.assertRaises(b.Error):
                b.archive_bytes([("fixture", file)], 32, float("inf"))
            raw = b.archive_bytes([("fixture", file)], 4096, float("inf"))
            with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as archive:
                self.assertEqual(archive.extractfile("fixture").read(), file.read_bytes())

    def test_failure_retains_detached_backing_and_uploadable_failure_records(self):
        with tempfile.TemporaryDirectory() as temp:
            work, evidence = Path(temp) / "work", Path(temp) / "evidence"
            def fail(bounds, output, root, provenance, runner, ledger):
                image = root / "build.dmg"
                image.write_bytes(b"synthetic-partial-image")
                provenance.update(image=str(image), mount=str(root / "volume"), attach_attempted=True)
                ledger.append({"partial_payload": 9, "error": "overflow"})
                raise b.Error("fetched payload overflow")
            with patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]), \
                    patch.object(b, "execute", side_effect=fail), patch.object(b.Runner, "command") as command, \
                    patch.object(b.os, "statvfs", return_value=SimpleNamespace(f_bavail=5, f_frsize=1024)):
                self.assertEqual(b.main(), 1)
            self.assertTrue((work / "build.dmg").exists())
            self.assertEqual(command.call_args.args[0], "image-detach")
            report = json.loads((evidence / "provenance.json").read_bytes())
            self.assertFalse(report["output_binary_accepted"])
            self.assertTrue(report["detached"])
            self.assertIn("failed_backing_image_retained_on_runner", report)
            manifest = json.loads((evidence / "sha256.json").read_bytes())
            self.assertTrue(all(b.digest((evidence / name).read_bytes()) == value for name, value in manifest.items()))

    def test_detach_failure_is_not_success_or_image_deletion(self):
        with tempfile.TemporaryDirectory() as temp:
            work, evidence = Path(temp) / "work", Path(temp) / "evidence"
            def prepared(bounds, output, root, provenance, runner, ledger):
                image = root / "build.dmg"
                image.write_bytes(b"synthetic")
                provenance.update(image=str(image), mount=str(root / "volume"), attach_attempted=True)
            with patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]), \
                    patch.object(b, "execute", side_effect=prepared), \
                    patch.object(b.Runner, "command", side_effect=b.Error("busy")):
                self.assertEqual(b.main(), 1)
            self.assertTrue((work / "build.dmg").exists())
            report = json.loads((evidence / "provenance.json").read_bytes())
            self.assertFalse(report["output_binary_accepted"])
            self.assertIn("detach_error", report)


if __name__ == "__main__":
    unittest.main()
