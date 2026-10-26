"""Synthetic source/controller checks; no real tools, provider or network."""

from contextlib import contextmanager
from contextlib import ExitStack
import ast
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
        class Transport(b.urllib.request.HTTPSHandler):
            handler_order = 0
            def __init__(self):
                pass
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
        env = b.build_environment(BOUNDS, Path("/private/volume"))
        self.assertEqual(env["GOPROXY"], "file:///private/volume/proxy")
        for key in ("HOME", "TMPDIR", "GOTMPDIR", "GOMODCACHE", "GOCACHE", "GOPATH"):
            self.assertTrue(Path(env[key]).is_relative_to("/private/volume"))
        self.assertEqual(env["GOVCS"], "*:off")
        self.assertEqual(env["GOSUMDB"], "off")
        self.assertEqual(env["GOFLAGS"], "-mod=readonly")
        self.assertEqual(env["CXX"], BOUNDS["tools"]["clangxx_nominal"])
        self.assertFalse(any("direct" in env[key] for key in ("GOPROXY", "GONOPROXY")))
        self.assertNotIn("GH_TOKEN", env)

    def test_ambient_build_search_loader_and_proxy_names_are_not_child_inputs(self):
        expected = b.build_environment(BOUNDS, Path("/private/volume"))
        for key in ("GOFLAGS", "GOEXPERIMENT", "GOTOOLCHAIN", "CGO_CFLAGS", "CC", "CXX",
                    "DYLD_INSERT_LIBRARIES", "LD_LIBRARY_PATH", "CPATH", "SDKROOT", "HTTPS_PROXY",
                    "GOROOT_1_24_ARM64", "nO_PrOxY"):
            with self.subTest(key=key), patch.dict(os.environ, {key: "synthetic-ambient-value"}):
                self.assertEqual(b.build_environment(BOUNDS, Path("/private/volume")), expected)

    def test_fixed_nonsparse_image_and_no_privileged_fallback(self):
        args = BOUNDS["volume"]["create"]
        self.assertEqual(args, ["create", "-size", "12288m", "-fs", "APFS", "-volname",
                                "TelexDerivedBuild", "-type", "UDIF", "{image}"])
        for excluded in ("-format", "-srcfolder", "-srcdevice", "SPARSE", "SPARSEBUNDLE"):
            self.assertNotIn(excluded, args)
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
                patch.object(b, "fetch_github_json", side_effect=[[self.comments()], review]), \
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
                    patch.object(b, "fetch_github_json", return_value=rows), self.assertRaises(b.Error):
                b.check_gate(BOUNDS, "b" * 64, "c" * 64, None, 100)

    def test_attempt2_dispatch_wrong_branch_fail_without_API_or_provider(self):
        for key, value in (("GITHUB_RUN_ATTEMPT", "2"), ("GITHUB_EVENT_NAME", "workflow_dispatch"),
                           ("GITHUB_REF", "refs/heads/main"), ("GITHUB_WORKFLOW_REF", "other")):
            env = self.environment()
            env[key] = value
            with self.subTest(key=key), patch.dict(os.environ, env, clear=True), \
                    patch.object(b, "fetch_github_json") as fetch, self.assertRaises(b.Error):
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
            self.assertTrue(any(e["phase"] == "cleanup-wait" for e in runner.records[0]["errors"]))


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


def synthetic_go_metadata():
    return (b"file: go1.27.3\npath\tgithub.com/blacktop/ipsw/cmd/ipsw\n"
            b"build\tGOOS=darwin\nbuild\tGOARCH=arm64\nbuild\tCGO_ENABLED=1\n"
            b"build\t-trimpath=true\nbuild\t-tags=sandbox\nbuild\t-buildmode=exe\n")


def synthetic_release_binary(overrides=None):
    values = {"AppVersion": BOUNDS["build"]["version"], "AppBuildCommit": BOUNDS["build"]["commit_string"]}
    if overrides is not None:
        values.update(overrides)
    return synthetic_macho(BOUNDS["signing"]["identifier"]) + b"\0" + b"\0".join(
        value.encode("utf-8") for value in values.values() if value is not None)


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
        raw = synthetic_go_metadata()
        self.assertNotIn("-ldflags", b.go_metadata(raw, BOUNDS)["settings"])
        for wrong in (raw.replace(b"GOARCH=arm64", b"GOARCH=amd64"),
                      raw.replace(b"GOARCH=arm64", b"GOARCH=arm64-extra"),
                      raw + b"build\tGOARCH=arm64\n",
                      raw.replace(b"go1.27.3", b"go1.27.2")):
            with self.assertRaises(b.Error):
                b.go_metadata(wrong, BOUNDS)
        with tempfile.TemporaryDirectory() as temp:
            file = Path(temp) / "ipsw"
            file.write_bytes(synthetic_release_binary({"AppVersion": "3.1.739"}))
            with self.assertRaisesRegex(b.Error, "AppVersion"):
                b.macho_signature(file, 4096, float("inf"), release_values={
                    "AppVersion": BOUNDS["build"]["version"], "AppBuildCommit": BOUNDS["build"]["commit_string"]})

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


class FailureBoundaryTests(unittest.TestCase):
    def test_wait_reaps_then_interrupts_seals_signal_authority_and_preserves_uncertainty(self):
        with RunnerTests().fake(b"captured-output") as (runner, process, _, kill, root):
            events = []
            interruption = KeyboardInterrupt("synthetic wait reaped then interrupted")
            def wait(timeout):
                self.assertTrue(runner.records[0]["signal_authority_sealed"])
                self.assertTrue(runner.records[0]["wait_attempted"])
                process.waited, process.returncode = True, 0
                events.append("leader-reaped-before-KeyboardInterrupt")
                raise interruption
            with patch.object(process, "wait", side_effect=wait) as waiter, self.assertRaises(KeyboardInterrupt) as raised:
                runner.command("static", ["/fixed/tool"], {})
            self.assertIs(raised.exception, interruption)
            self.assertEqual(events, ["leader-reaped-before-KeyboardInterrupt"])
            self.assertEqual(waiter.call_count, 1)
            kill.assert_not_called()
            record = runner.records[0]
            self.assertFalse(record["group_kill_before_reap"])
            self.assertFalse(record["group_signal_attempted"])
            self.assertFalse(record["complete"])
            self.assertFalse(record["reaped"])
            self.assertEqual(record["process_returncode_observed"], 0)
            self.assertEqual(record["reap_state"], "wait-outcome-indeterminate")
            self.assertEqual(record["cleanup_wait_skipped"], "prior-wait-may-have-consumed-anchor")
            self.assertEqual(record["errors"][0]["type"], "KeyboardInterrupt")
            self.assertTrue(record["errors"][0]["interruption"])
            self.assertEqual((root / "cmd-00-stdout.bin").read_bytes(), b"captured-output")
            self.assertTrue(record["streams"]["stdout"]["persisted"])

    def test_any_potentially_consuming_wait_failure_cannot_rearm_signal_or_wait(self):
        for failure in (subprocess.TimeoutExpired("owned", 5), OSError("wait failed"),
                        SystemExit("wait exited")):
            with self.subTest(failure=type(failure).__name__), RunnerTests().fake() as (
                    runner, process, _, kill, _):
                expected = b.Error if isinstance(failure, Exception) else type(failure)
                with patch.object(process, "wait", side_effect=failure) as waiter, self.assertRaises(expected):
                    runner.command("static", ["/fixed/tool"], {})
                self.assertEqual(waiter.call_count, 1)
                kill.assert_not_called()
                self.assertTrue(runner.records[0]["signal_authority_sealed"])
                self.assertEqual(runner.records[0]["reap_state"], "wait-outcome-indeterminate")
                self.assertFalse(runner.records[0]["complete"])

    def test_prewait_interruption_disables_signals_but_may_finish_exact_owned_wait(self):
        with RunnerTests().fake() as (runner, process, _, kill, root), \
                patch.object(b.os, "read", side_effect=KeyboardInterrupt("capture interrupted")):
            with self.assertRaises(KeyboardInterrupt):
                runner.command("interrupted", ["/fixed/tool"], {})
            kill.assert_not_called()
            record = runner.records[0]
            self.assertTrue(record["reaped"])
            self.assertFalse(record["complete"])
            self.assertEqual(record["signal_seal_reason"], "interruption")
            self.assertEqual(record["waits"][0]["phase"], "cleanup-wait")
            self.assertEqual((root / "cmd-00-stdout.bin").read_bytes(), b"")

    def test_cleanup_wait_interruption_preserves_one_prewait_signal_without_second_signal(self):
        with RunnerTests().fake(b"overflow", cap=4) as (runner, process, _, kill, root):
            def wait(timeout):
                process.waited, process.returncode = True, -9
                raise KeyboardInterrupt("cleanup reaped then interrupted")
            with patch.object(process, "wait", side_effect=wait) as waiter, self.assertRaises(KeyboardInterrupt):
                runner.command("overflow", ["/fixed/tool"], {})
            self.assertEqual(waiter.call_count, 1)
            kill.assert_called_once_with(1234, b.signal.SIGKILL)
            record = runner.records[0]
            self.assertTrue(record["group_kill_before_reap"])
            self.assertFalse(record["reaped"])
            self.assertFalse(record["complete"])
            self.assertEqual([e["phase"] for e in record["errors"]], ["capture", "cleanup-wait"])
            self.assertEqual(record["process_returncode_observed"], -9)
            self.assertEqual((root / "cmd-00-stdout.bin").read_bytes(), b"overf")

    def test_each_selector_or_pipe_close_failure_retains_raw_evidence_for_normal_and_overflow(self):
        for endpoint in ("selector", "stdout", "stderr"):
            for overflow in (False, True):
                with self.subTest(endpoint=endpoint, overflow=overflow), RunnerTests().fake(
                        b"captured-output", cap=4 if overflow else 100) as (runner, process, _, kill, root):
                    closed = []
                    def close(name):
                        closed.append(name)
                        if name == endpoint:
                            raise OSError("synthetic " + name + " close")
                    with patch.object(Poller, "close", side_effect=lambda: close("selector")), \
                            patch.object(process.stdout, "close", side_effect=lambda: close("stdout")), \
                            patch.object(process.stderr, "close", side_effect=lambda: close("stderr")), \
                            self.assertRaises(b.Error):
                        runner.command("overflow" if overflow else "static", ["/fixed/tool"], {})
                    self.assertEqual(closed, ["selector", "stdout", "stderr"])
                    record = runner.records[0]
                    self.assertFalse(record["complete"])
                    self.assertTrue(record["reaped"])
                    self.assertTrue(any(e["phase"] == endpoint + "-close" for e in record["errors"]))
                    if overflow:
                        self.assertEqual(record["errors"][0]["message"], "command stream cap")
                    self.assertEqual(kill.call_count, int(overflow))
                    self.assertEqual(len(runner.logs), 2)
                    raw = b"captu" if overflow else b"captured-output"
                    self.assertEqual((root / "cmd-00-stdout.bin").read_bytes(), raw)
                    self.assertEqual((root / "cmd-00-stderr.bin").read_bytes(), b"")
                    self.assertEqual(record["streams"]["stdout"]["sha256"], b.digest(raw))
                    self.assertTrue(record["streams"]["stdout"]["persisted"])
                    self.assertIn("seconds", record)

    def test_multiple_closes_and_close_interruption_attempt_all_finalizers(self):
        with RunnerTests().fake() as (runner, process, _, kill, root):
            with patch.object(Poller, "close", side_effect=KeyboardInterrupt("selector interrupt")) as poller, \
                    patch.object(process.stdout, "close", side_effect=OSError("stdout close")) as stdout, \
                    patch.object(process.stderr, "close", side_effect=OSError("stderr close")) as stderr, \
                    self.assertRaises(KeyboardInterrupt):
                runner.command("static", ["/fixed/tool"], {})
            for close in (poller, stdout, stderr):
                close.assert_called_once()
            kill.assert_not_called()
            self.assertEqual([e["phase"] for e in runner.records[0]["errors"]],
                             ["selector-close", "stdout-close", "stderr-close"])
            self.assertFalse(runner.records[0]["complete"])
            self.assertEqual((root / "cmd-00-stdout.bin").read_bytes(), b"static")
            self.assertEqual(len(runner.logs), 2)

    def test_stdout_log_failure_still_attempts_stderr_and_keeps_captured_hash(self):
        with RunnerTests().fake() as (runner, _, _, kill, root):
            original_open = Path.open
            def opened(file, *args, **kwargs):
                if file.name == "cmd-00-stdout.bin":
                    raise OSError("synthetic stdout log write failure")
                return original_open(file, *args, **kwargs)
            with patch.object(Path, "open", opened), self.assertRaises(b.Error):
                runner.command("static", ["/fixed/tool"], {})
            kill.assert_not_called()
            record = runner.records[0]
            self.assertFalse(record["streams"]["stdout"]["persisted"])
            self.assertEqual(record["streams"]["stdout"]["sha256"], b.digest(b"static"))
            self.assertTrue(record["streams"]["stderr"]["persisted"])
            self.assertEqual((root / "cmd-00-stderr.bin").read_bytes(), b"")
            self.assertFalse(record["complete"])

    def test_raw_log_close_failure_does_not_mask_other_stream_or_primary_error(self):
        with RunnerTests().fake(b"overflow", cap=4) as (runner, _, _, _, root):
            original = Path.open
            class BrokenClose:
                def __init__(self, stream):
                    self.stream = stream
                def write(self, raw):
                    return self.stream.write(raw)
                def close(self):
                    self.stream.close()
                    raise OSError("synthetic raw stdout log close")
            def opened(file, *args, **kwargs):
                stream = original(file, *args, **kwargs)
                return BrokenClose(stream) if file.name == "cmd-00-stdout.bin" and args == ("xb",) else stream
            with patch.object(Path, "open", opened), self.assertRaises(b.Error):
                runner.command("overflow", ["/fixed/tool"], {})
            record = runner.records[0]
            self.assertEqual(record["errors"][0]["message"], "command stream cap")
            self.assertEqual(record["errors"][-1]["phase"], "stdout-log-close")
            self.assertEqual((root / "cmd-00-stdout.bin").read_bytes(), b"overf")
            self.assertEqual((root / "cmd-00-stderr.bin").read_bytes(), b"")
            self.assertFalse(record["streams"]["stdout"]["persisted"])
            self.assertTrue(record["streams"]["stderr"]["persisted"])
            self.assertFalse(record["complete"])

    def test_cleanup_at_absolute_deadline_authorizes_no_wait_and_records_unreaped_failure(self):
        with RunnerTests().fake() as (runner, process, _, kill, root):
            now = [990.0]
            original_read = b.os.read
            def read(fd, maximum):
                raw = original_read(fd, maximum)
                now[0] = 1000.0
                return raw
            with patch.object(b.time, "monotonic", side_effect=lambda: now[0]), \
                    patch.object(b.os, "read", side_effect=read), \
                    patch.object(process, "wait") as waiter, self.assertRaises(b.Error):
                runner.command("image-detach", ["/fixed/tool"], {}, "cleanup", 60)
            self.assertEqual(now[0], runner.cleanup_deadline)
            waiter.assert_not_called()
            kill.assert_called_once()
            record = runner.records[0]
            self.assertFalse(record["complete"])
            self.assertFalse(record["reaped"])
            self.assertFalse(record["wait_attempted"])
            self.assertEqual(record["cleanup_wait_skipped"], "controller-deadline-exhausted")
            self.assertTrue(record["signal_authority_sealed"])
            self.assertEqual((root / "cmd-00-stdout.bin").read_bytes(), b"static")

    def test_cleanup_wait_uses_only_remaining_controller_time(self):
        for remaining in (0.25, 2.0, 9.0, 20.0):
            with self.subTest(remaining=remaining), RunnerTests().fake(b"overflow", cap=1) as (
                    runner, process, _, kill, _):
                now = [1000.0 - remaining]
                def wait(timeout):
                    self.assertLessEqual(timeout, runner.cleanup_deadline - now[0])
                    now[0] += timeout
                    process.waited = True
                    return 0
                with patch.object(b.time, "monotonic", side_effect=lambda: now[0]), \
                        patch.object(process, "wait", side_effect=wait) as waiter, self.assertRaises(b.Error):
                    runner.command("image-detach", ["/fixed/tool"], {}, "cleanup", 60)
                self.assertEqual(waiter.call_args.kwargs["timeout"], min(10, remaining))
                self.assertLessEqual(now[0], runner.cleanup_deadline)
                self.assertTrue(runner.records[0]["reaped"])
                self.assertFalse(runner.records[0]["complete"])
                self.assertEqual(kill.call_count, 1)
                wait_record = runner.records[0]["waits"][0]
                self.assertLessEqual(wait_record["authorized_at"] + wait_record["timeout"], wait_record["deadline"])

    def test_metadata_finalizer_does_not_replace_primary_interruption(self):
        with RunnerTests().fake() as (runner, _, _, _, _):
            interruption = KeyboardInterrupt("primary")
            with patch.object(b.time, "monotonic", side_effect=[0, 181, 181]), \
                    self.assertRaises(KeyboardInterrupt) as raised:
                with b.metadata_phase(runner):
                    raise interruption
            self.assertIs(raised.exception, interruption)


class FetchFinalizationTests(unittest.TestCase):
    @staticmethod
    def close_error_response(url, raw=b"abc", status=200, headers=None):
        class CloseError(Response):
            def close(self):
                if not self.closed:
                    super().close()
                    raise OSError("synthetic response close error")
        return CloseError(url, raw, status, headers)

    def test_response_close_failure_keeps_all_four_request_ledgers_and_primary_errors(self):
        url = "https://proxy.golang.org/synthetic"
        for mode, status in (("success", 200), ("http-error", 404), ("redirect", 302), ("incomplete", 200)):
            response = self.close_error_response(url, headers={"Location": url} if status == 302 else {}, status=status)
            primary = http.client.IncompleteRead(b"ab", 3)
            if mode == "incomplete":
                response.read1 = lambda maximum: (_ for _ in ()).throw(primary)
            with self.subTest(mode=mode), FetchTests().scenario([response]) as (fetcher, root, calls, ledger):
                expected = http.client.IncompleteRead if mode == "incomplete" else b.Error if mode == "http-error" else OSError
                with self.assertRaises(expected) as raised:
                    fetcher.fetch(url, root / "data", "module", ["proxy.golang.org"])
                if mode == "incomplete":
                    self.assertIs(raised.exception, primary)
                self.assertEqual(len(calls), 1)
                self.assertEqual(len(ledger), 1)
                record = ledger[0]
                raw = b"ab" if mode == "incomplete" else b"abc"
                self.assertEqual(record["payload_bytes"], len(raw))
                self.assertEqual(record["payload_after"], len(raw))
                self.assertEqual(record["payload_counters_after"]["module"], len(raw))
                self.assertEqual(record["received_body_sha256"], b.digest(raw))
                self.assertEqual(record["partial_file_bytes"], len(raw))
                self.assertEqual((root / "data.hop-0.partial").read_bytes(), raw)
                self.assertFalse((root / "data").exists())
                self.assertFalse(record["complete"])
                self.assertEqual(record["retained_disposition"], "partial-not-accepted")
                self.assertEqual(record["finalization_errors"][0]["phase"], "response-close")
                self.assertEqual(record["error"]["type"], expected.__name__)
                if mode == "http-error":
                    self.assertEqual(record["error"]["message"], "non-success fetch response")
                self.assertTrue(response.closed)

    def test_body_file_close_and_response_close_errors_are_independent(self):
        url = "https://proxy.golang.org/synthetic"
        response = self.close_error_response(url)
        original = Path.open
        class BrokenClose:
            def __init__(self, stream):
                self.stream = stream
            def write(self, raw):
                return self.stream.write(raw)
            def close(self):
                self.stream.close()
                raise OSError("synthetic body file close error")
        def opened(file, *args, **kwargs):
            stream = original(file, *args, **kwargs)
            return BrokenClose(stream) if file.name.endswith(".partial") and args == ("xb",) else stream
        with FetchTests().scenario([response]) as (fetcher, root, calls, ledger), patch.object(Path, "open", opened):
            with self.assertRaisesRegex(OSError, "body file close error"):
                fetcher.fetch(url, root / "data", "module", ["proxy.golang.org"])
            self.assertEqual([e["phase"] for e in ledger[0]["finalization_errors"]],
                             ["body-file-close", "response-close"])
            self.assertEqual(ledger[0]["received_body_sha256"], b.digest(b"abc"))
            self.assertEqual(ledger[0]["partial_file_bytes"], 3)
            self.assertTrue(response.closed)
            self.assertEqual(len(calls), 1)

    def test_primary_interruption_is_not_masked_by_response_close_error(self):
        url = "https://proxy.golang.org/synthetic"
        response = self.close_error_response(url)
        interruption = KeyboardInterrupt("body interrupted")
        response.read1 = lambda maximum: (_ for _ in ()).throw(interruption)
        with FetchTests().scenario([response]) as (fetcher, root, _, ledger), self.assertRaises(KeyboardInterrupt) as raised:
            fetcher.fetch(url, root / "data", "module", ["proxy.golang.org"])
        self.assertIs(raised.exception, interruption)
        self.assertEqual(ledger[0]["error"]["type"], "KeyboardInterrupt")
        self.assertEqual(ledger[0]["finalization_errors"][0]["type"], "OSError")
        self.assertTrue(ledger[0]["error"]["interruption"])
        self.assertEqual(ledger[0]["payload_after"], 0)

    def test_partial_stat_failure_does_not_erase_counter_or_response_close_error(self):
        url = "https://proxy.golang.org/synthetic"
        response = self.close_error_response(url)
        with FetchTests().scenario([response]) as (fetcher, root, _, ledger):
            original = Path.stat
            def stat_file(file, *args, **kwargs):
                if file.name.endswith(".partial"):
                    raise PermissionError("partial stat denied")
                return original(file, *args, **kwargs)
            with patch.object(Path, "stat", stat_file), self.assertRaisesRegex(OSError, "response close"):
                fetcher.fetch(url, root / "data", "module", ["proxy.golang.org"])
            self.assertEqual(ledger[0]["payload_after"], 3)
            self.assertEqual(ledger[0]["received_body_sha256"], b.digest(b"abc"))
            self.assertIsNone(ledger[0]["partial_file_bytes"])
            self.assertEqual([e["phase"] for e in ledger[0]["finalization_errors"]],
                             ["response-close", "partial-file-stat"])

    def test_response_close_error_and_payload_totals_reach_main_failure_artifacts(self):
        url = "https://proxy.golang.org/synthetic"
        for status in (200, 404):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as directory:
                work, evidence = Path(directory) / "work", Path(directory) / "evidence"
                response = self.close_error_response(url, status=status)
                def fetch_only(bounds, output, root, provenance, runner, ledger):
                    fetcher = b.Fetcher(bounds["limits"], float("inf"), ledger)
                    fetcher.fetch(url, root / "data", "module", ["proxy.golang.org"])
                with patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]), \
                        patch.object(b, "execute", side_effect=fetch_only), \
                        patch.object(b.urllib.request, "build_opener",
                                     return_value=SimpleNamespace(open=lambda *args, **kwargs: response)):
                    self.assertEqual(b.main(), 1)
                provenance = json.loads((evidence / "provenance.json").read_bytes())
                ledger = json.loads((evidence / "fetches.json").read_bytes())
                self.assertEqual(provenance["fetch_payload_bytes"]["module"], 3)
                self.assertFalse(provenance["output_binary_accepted"])
                self.assertEqual(len(ledger), 1)
                self.assertEqual(ledger[0]["received_body_sha256"], b.digest(b"abc"))
                self.assertEqual(ledger[0]["finalization_errors"][0]["phase"], "response-close")
                self.assertEqual(provenance["failure"]["type"], "OSError" if status == 200 else "DiagnosticError")
                manifest = json.loads((evidence / "sha256.json").read_bytes())
                self.assertTrue(all(b.digest((evidence / f).read_bytes()) == h for f, h in manifest.items()))


class MainFinalizationTests(unittest.TestCase):
    def test_interrupted_wait_reaches_failed_provenance_without_success_claim(self):
        with tempfile.TemporaryDirectory() as directory:
            work, evidence = Path(directory) / "work", Path(directory) / "evidence"
            interruption = KeyboardInterrupt("retained interruption")
            def interrupted(bounds, output, root, provenance, runner, ledger):
                runner.records.append({"complete": False, "errors": [b.failure_record(interruption, "command-wait")]})
                raise interruption
            with patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]), \
                    patch.object(b, "execute", side_effect=interrupted):
                self.assertEqual(b.main(), 1)
            provenance = json.loads((evidence / "provenance.json").read_bytes())
            self.assertFalse(provenance["output_binary_accepted"])
            self.assertEqual(provenance["failure"]["type"], "KeyboardInterrupt")
            self.assertTrue(provenance["failure"]["interruption"])
            self.assertTrue((evidence / "commands.json").is_file())

    def test_failure_writing_one_required_artifact_still_attempts_the_others(self):
        with tempfile.TemporaryDirectory() as directory:
            work, evidence = Path(directory) / "work", Path(directory) / "evidence"
            original = b.Evidence.json
            def output(target, name, value):
                if name == "commands.json":
                    raise OSError("synthetic command receipt sink failure")
                return original(target, name, value)
            with patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]), \
                    patch.object(b, "execute", side_effect=b.Error("primary build failure")), \
                    patch.object(b.Evidence, "json", output), self.assertRaises(OSError):
                b.main()
            self.assertTrue((evidence / "fetches.json").is_file())
            provenance = json.loads((evidence / "provenance.json").read_bytes())
            self.assertFalse(provenance["output_binary_accepted"])
            self.assertEqual(provenance["failure"]["message"], "primary build failure")
            self.assertEqual(provenance["required_output_errors"][0]["file"], "commands.json")
            self.assertFalse((evidence / "sha256.json").exists())


class LateRetentionTests(unittest.TestCase):
    image_bytes = b"synthetic-completed-image-with-inputs"

    @contextmanager
    def prepared_case(self, mode=None, finish_after_write=False, interruption=False, failure_receipt_error=False):
        with tempfile.TemporaryDirectory() as directory:
            work, evidence = Path(directory) / "work", Path(directory) / "evidence"
            image = work / "build.dmg"
            clock, events, observed = [0.0], [], {}
            original_json, original_finish, original_unlink = b.Evidence.json, b.Evidence.finish, Path.unlink
            failure_type = KeyboardInterrupt if interruption else OSError
            failure = failure_type("synthetic late " + str(mode) + " failure")

            def prepared(bounds, output, root, provenance, runner, ledger):
                image.write_bytes(self.image_bytes)
                provenance.update(image=str(image), mount=str(root / "volume"), attach_attempted=True,
                    disposition="built-and-signed-not-executed-pending-independent-review", artifact_deadline=100.0)
                events.append("prepared-success")

            def detached(*args, **kwargs):
                self.assertEqual(args[0], "image-detach")
                self.assertEqual(image.read_bytes(), self.image_bytes)
                events.append("detach")
                return b"", b""

            def volume_stat(path):
                if mode == "acceptance":
                    clock[0] = 100.0
                return SimpleNamespace(f_bavail=20, f_frsize=1024)

            def json_sink(target, name, value):
                events.append(name)
                self.assertEqual(image.read_bytes(), self.image_bytes)
                if name == "provenance.json":
                    observed["attempted_provenance"] = copy.deepcopy(value)
                if name == mode:
                    raise failure
                if failure_receipt_error and name == "finalization-error.json":
                    raise OSError("synthetic secondary failure receipt error")
                return original_json(target, name, value)

            def finish(target):
                events.append("finish-enter")
                self.assertEqual(image.read_bytes(), self.image_bytes)
                persisted = json.loads((evidence / "provenance.json").read_bytes())
                self.assertFalse(persisted["output_binary_accepted"])
                if mode == "finish" and not finish_after_write:
                    raise failure
                result = original_finish(target)
                events.append("finish-written")
                self.assertEqual(image.read_bytes(), self.image_bytes)
                if mode == "finish":
                    raise failure
                events.append("finish-return")
                return result

            def unlink(file, *args, **kwargs):
                if file == image:
                    events.append("backing-unlink")
                    self.fail("Backing retention must not depend on a provisional success flag")
                return original_unlink(file, *args, **kwargs)

            with patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]), \
                    patch.object(b, "execute", side_effect=prepared), \
                    patch.object(b.Runner, "command", side_effect=detached) as detach, \
                    patch.object(b.os, "statvfs", side_effect=volume_stat), \
                    patch.object(b.time, "monotonic", side_effect=lambda: clock[0]), \
                    patch.object(Path, "unlink", unlink), patch.object(b.Evidence, "json", json_sink), \
                    patch.object(b.Evidence, "finish", finish):
                code, caught = None, None
                try:
                    code = b.main()
                    events.append("main-return")
                except BaseException as error:
                    caught = error
                    events.append("main-raise")
            self.assertEqual(image.read_bytes(), self.image_bytes)
            self.assertEqual(detach.call_count, 1)
            self.assertNotIn("backing-unlink", events)
            yield {"work": work, "evidence": evidence, "image": image, "events": events,
                   "code": code, "caught": caught, "injected": failure, **observed}

    def manifest_valid(self, root):
        file = root / "sha256.json"
        if not file.is_file():
            return False
        hashes = json.loads(file.read_bytes())
        names = {p.name for p in root.iterdir() if p.name != "sha256.json"}
        return set(hashes) == names and all(b.digest((root / name).read_bytes()) == sha for name, sha in hashes.items())

    def test_all_five_late_failures_keep_exact_backing_and_truthful_failed_outcomes(self):
        for mode in ("acceptance", "commands.json", "fetches.json", "provenance.json", "finish"):
            with self.subTest(mode=mode), self.prepared_case(mode) as case:
                root = case["evidence"]
                if mode == "acceptance":
                    self.assertEqual(case["code"], 1)
                    self.assertIsNone(case["caught"])
                    self.assertTrue(self.manifest_valid(root))
                else:
                    self.assertIs(case["caught"], case["injected"])
                    self.assertIsNone(case["code"])
                    self.assertFalse(self.manifest_valid(root))
                    failure = json.loads((root / "finalization-error.json").read_bytes())
                    self.assertFalse(failure["output_binary_accepted"])
                    self.assertFalse(failure["controller_unlinks_backing"])
                    self.assertTrue(failure["detached"])
                    self.assertEqual(failure["backing_image_retained_on_runner"], str(case["image"]))
                provenance = root / "provenance.json"
                if provenance.exists():
                    record = json.loads(provenance.read_bytes())
                    self.assertFalse(record["output_binary_accepted"])
                    self.assertEqual(record["backing_image_retained_on_runner"], str(case["image"]))
                    self.assertTrue(record["detached"])
                    self.assertNotIn("only_known_backing_image_deleted", record)
                    self.assertIn("never unlinks", record["retention_precision"])
                if mode in ("commands.json", "fetches.json"):
                    self.assertIn("commands.json", case["events"])
                    self.assertIn("fetches.json", case["events"])
                    self.assertIn("provenance.json", case["events"])

    def test_success_retains_backing_and_returns_zero_only_after_manifest_finish(self):
        with self.prepared_case() as case:
            self.assertEqual(case["code"], 0)
            self.assertIsNone(case["caught"])
            self.assertTrue(self.manifest_valid(case["evidence"]))
            self.assertLess(case["events"].index("finish-return"), case["events"].index("main-return"))
            self.assertFalse((case["evidence"] / "finalization-error.json").exists())
            record = json.loads((case["evidence"] / "provenance.json").read_bytes())
            self.assertTrue(record["finalization_candidate"])
            self.assertFalse(record["output_binary_accepted"])
            self.assertIn("main returns 0 after finish", record["finalization_contract"])
            self.assertEqual(record["backing_after"]["logical"], len(self.image_bytes))
            self.assertEqual(record["backing_image_retained_on_runner"], str(case["image"]))

    def test_failure_after_manifest_was_written_still_keeps_image_and_is_not_accepted(self):
        with self.prepared_case("finish", finish_after_write=True) as case:
            self.assertIs(case["caught"], case["injected"])
            self.assertTrue((case["evidence"] / "sha256.json").exists())
            self.assertFalse(self.manifest_valid(case["evidence"]))
            self.assertFalse(json.loads((case["evidence"] / "provenance.json").read_bytes())["output_binary_accepted"])
            failure = json.loads((case["evidence"] / "finalization-error.json").read_bytes())
            self.assertEqual(failure["error"]["phase"], "manifest-finish")
            self.assertTrue(failure["manifest_not_acceptance"])

    def test_each_late_interruption_keeps_image_and_original_exception(self):
        for mode in ("commands.json", "fetches.json", "provenance.json", "finish"):
            with self.subTest(mode=mode), self.prepared_case(mode, interruption=True) as case:
                self.assertIs(case["caught"], case["injected"])
                self.assertFalse(self.manifest_valid(case["evidence"]))
                failure = json.loads((case["evidence"] / "finalization-error.json").read_bytes())
                self.assertEqual(failure["error"]["type"], "KeyboardInterrupt")
                self.assertTrue(failure["error"]["interruption"])

    def test_finalization_error_sink_failure_never_discards_backing_or_masks_primary(self):
        for mode in ("provenance.json", "finish"):
            with self.subTest(mode=mode), self.prepared_case(mode, failure_receipt_error=True) as case:
                self.assertIs(case["caught"], case["injected"])
                self.assertFalse(self.manifest_valid(case["evidence"]))
                self.assertFalse((case["evidence"] / "finalization-error.json").exists())
                self.assertTrue(any("receipt also failed" in note for note in case["caught"].__notes__))
                self.assertNotEqual(case["code"], 0)


class PrimaryOutputErrorTests(unittest.TestCase):
    image_bytes = LateRetentionTests.image_bytes
    manifest_valid = LateRetentionTests.manifest_valid

    @contextmanager
    def compound_case(self, failures=None, finish_after_write=False, acceptance_failure=False,
                      partial_provenance=False):
        failures = {} if failures is None else failures
        with tempfile.TemporaryDirectory() as directory:
            work, evidence = Path(directory) / "work", Path(directory) / "evidence"
            image = work / "build.dmg"
            clock, events, observed = [0.0], [], {}
            original_json, original_finish, original_unlink = b.Evidence.json, b.Evidence.finish, Path.unlink

            def prepared(bounds, output, root, provenance, runner, ledger):
                image.write_bytes(self.image_bytes)
                provenance.update(image=str(image), mount=str(root / "volume"), attach_attempted=True,
                    disposition="built-and-signed-not-executed-pending-independent-review", artifact_deadline=100.0)
                events.append("prepared-success")

            def detach(*args, **kwargs):
                self.assertEqual(args[0], "image-detach")
                self.assertEqual(image.read_bytes(), self.image_bytes)
                events.append("fake-detach")
                return b"", b""

            def volume_stat(file):
                if acceptance_failure:
                    clock[0] = 100.0
                return SimpleNamespace(f_bavail=20, f_frsize=1024)

            def output(target, name, value):
                self.assertEqual(image.read_bytes(), self.image_bytes)
                events.append(name)
                if name in ("provenance.json", "finalization-error.json"):
                    observed["attempted_" + name] = copy.deepcopy(value)
                if name in failures:
                    if partial_provenance and name == "provenance.json":
                        (target.root / name).write_bytes(b'{"partial":')
                    raise failures[name]
                return original_json(target, name, value)

            def finish(target):
                events.append("finish-enter")
                self.assertFalse(json.loads((evidence / "provenance.json").read_bytes())["output_binary_accepted"])
                if "finish" in failures and not finish_after_write:
                    raise failures["finish"]
                result = original_finish(target)
                events.append("manifest-written")
                if "finish" in failures:
                    raise failures["finish"]
                events.append("finish-return")
                return result

            def unlink(file, *args, **kwargs):
                if file == image:
                    self.fail("No output-error path may unlink retained backing")
                return original_unlink(file, *args, **kwargs)

            with patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]), \
                    patch.object(b, "execute", side_effect=prepared), \
                    patch.object(b.Runner, "command", side_effect=detach) as detached, \
                    patch.object(b.os, "statvfs", side_effect=volume_stat), \
                    patch.object(b.time, "monotonic", side_effect=lambda: clock[0]), \
                    patch.object(Path, "unlink", unlink), patch.object(b.Evidence, "json", output), \
                    patch.object(b.Evidence, "finish", finish):
                code, caught = None, None
                try:
                    code = b.main()
                    events.append("main-return")
                except BaseException as error:
                    caught = error
                    events.append("main-raise")
            self.assertEqual(image.read_bytes(), self.image_bytes)
            self.assertEqual(detached.call_count, 1)
            yield {"image": image, "evidence": evidence, "events": events, "code": code,
                   "caught": caught, "failures": failures, **observed}

    def assert_error_record(self, record, error, filename, phase):
        self.assertEqual(record, {"file": filename, "phase": phase, "type": type(error).__name__,
                                  "message": str(error), "interruption": not isinstance(error, Exception)})

    def test_exact_command_interruption_and_provenance_error_preserves_first_in_both_outcomes(self):
        first = KeyboardInterrupt("owner synthetic primary command-write interruption")
        second = OSError("owner synthetic secondary provenance-write failure")
        with self.compound_case({"commands.json": first, "provenance.json": second}) as case:
            self.assertIs(case["caught"], first)
            self.assertIsNone(case["code"])
            root = case["evidence"]
            self.assertEqual({p.name for p in root.iterdir()}, {"fetches.json", "finalization-error.json"})
            receipt = json.loads((root / "finalization-error.json").read_bytes())
            self.assert_error_record(receipt["error"], first, "commands.json", "required-output")
            self.assert_error_record(receipt["secondary_errors"][0], second, "provenance.json", "provenance-write")
            self.assertEqual(len(receipt["secondary_errors"]), 1)
            self.assertEqual(receipt["required_output_states"], {
                "commands.json": "write-failed-persistence-unconfirmed", "fetches.json": "write-returned",
                "provenance.json": "write-failed-persistence-unconfirmed", "sha256.json": "not-attempted"})
            self.assertFalse(receipt["output_binary_accepted"])
            self.assertTrue(receipt["detached"])
            self.assertFalse(receipt["controller_unlinks_backing"])
            self.assertFalse(self.manifest_valid(root))
            self.assertNotIn("finish-enter", case["events"])
            notes = "\n".join(first.__notes__)
            self.assertIn("provenance.json", notes)
            self.assertIn("provenance-write", notes)
            self.assertIn(str(second), notes)
            self.assertTrue(case["attempted_provenance.json"]["required_output_errors"][0]["interruption"])

    def test_commands_or_fetches_first_and_provenance_later_keep_exception_precedence(self):
        for filename in ("commands.json", "fetches.json"):
            for first_type in (OSError, KeyboardInterrupt):
                for later_type in (OSError, KeyboardInterrupt):
                    first, later = first_type("first " + filename), later_type("later provenance")
                    with self.subTest(file=filename, first=first_type.__name__, later=later_type.__name__), \
                            self.compound_case({filename: first, "provenance.json": later}) as case:
                        self.assertIs(case["caught"], first)
                        self.assertIsNone(case["code"])
                        receipt = json.loads((case["evidence"] / "finalization-error.json").read_bytes())
                        self.assert_error_record(receipt["error"], first, filename, "required-output")
                        self.assertEqual(len(receipt["secondary_errors"]), 1)
                        self.assert_error_record(receipt["secondary_errors"][0], later,
                                                 "provenance.json", "provenance-write")
                        self.assertEqual(case["events"][2:5],
                                         ["commands.json", "fetches.json", "provenance.json"])
                        self.assertFalse((case["evidence"] / "provenance.json").exists())
                        self.assertFalse((case["evidence"] / "sha256.json").exists())
                        self.assertFalse(receipt["output_binary_accepted"])

    def test_multiple_required_output_failures_are_ordered_once_without_duplicate_primary(self):
        for provenance_failure in (False, True):
            errors = {"commands.json": OSError("first commands"),
                      "fetches.json": KeyboardInterrupt("second fetches")}
            if provenance_failure:
                errors["provenance.json"] = OSError("third provenance")
            with self.subTest(provenance_failure=provenance_failure), self.compound_case(errors) as case:
                self.assertIs(case["caught"], errors["commands.json"])
                receipt = json.loads((case["evidence"] / "finalization-error.json").read_bytes())
                records = [receipt["error"], *receipt["secondary_errors"]]
                self.assertEqual([r["file"] for r in records], list(errors))
                for record, (filename, error) in zip(records, errors.items()):
                    self.assert_error_record(record, error, filename,
                        "provenance-write" if filename == "provenance.json" else "required-output")
                if not provenance_failure:
                    provenance = json.loads((case["evidence"] / "provenance.json").read_bytes())
                    self.assertFalse(provenance["output_binary_accepted"])
                    self.assertFalse(provenance["finalization_candidate"])
                    self.assertEqual([r["file"] for r in provenance["required_output_errors"]],
                                     ["commands.json", "fetches.json"])
                self.assertNotIn("finish-enter", case["events"])

    def test_fallback_receipt_failure_keeps_first_and_structured_secondary_exception_notes(self):
        for first_name in ("commands.json", "fetches.json"):
            for first_type in (OSError, KeyboardInterrupt):
                first = first_type("first output")
                later = OSError("provenance write failed")
                fallback = KeyboardInterrupt("fallback interrupted")
                with self.subTest(file=first_name, kind=first_type.__name__), self.compound_case({
                        first_name: first, "provenance.json": later, "finalization-error.json": fallback}) as case:
                    self.assertIs(case["caught"], first)
                    self.assertFalse((case["evidence"] / "finalization-error.json").exists())
                    self.assertFalse((case["evidence"] / "provenance.json").exists())
                    self.assertFalse(self.manifest_valid(case["evidence"]))
                    attempted = case["attempted_finalization-error.json"]
                    self.assert_error_record(attempted["error"], first, first_name, "required-output")
                    self.assert_error_record(attempted["secondary_errors"][0], later,
                                             "provenance.json", "provenance-write")
                    notes = [json.loads(note.split(": ", 1)[1]) for note in first.__notes__]
                    self.assertEqual(len(notes), 2)
                    self.assert_error_record(next(r for r in notes if r["file"] == "provenance.json"),
                                             later, "provenance.json", "provenance-write")
                    self.assert_error_record(next(r for r in notes if r["file"] == "finalization-error.json"),
                                             fallback, "finalization-error.json", "failure-receipt-write")

    def test_single_errors_keep_file_phase_and_no_fabricated_provenance_completion(self):
        for filename, expected_file, phase in (("commands.json", "commands.json", "required-output"),
                ("fetches.json", "fetches.json", "required-output"),
                ("provenance.json", "provenance.json", "provenance-write"),
                ("finish", "sha256.json", "manifest-finish")):
            first = OSError("single " + filename)
            with self.subTest(filename=filename), self.compound_case({filename: first}) as case:
                self.assertIs(case["caught"], first)
                record = json.loads((case["evidence"] / "finalization-error.json").read_bytes())
                self.assert_error_record(record["error"], first, expected_file, phase)
                self.assertEqual(record["secondary_errors"], [])
                expected = "write-failed-persistence-unconfirmed" if filename == "provenance.json" else "write-returned"
                self.assertEqual(record["required_output_states"]["provenance.json"], expected)
                self.assertFalse(record["output_binary_accepted"])

    def test_valid_manifest_alone_is_not_collection_when_finish_and_fallback_fail(self):
        first, fallback = OSError("after manifest"), OSError("failure receipt unavailable")
        with self.compound_case({"finish": first, "finalization-error.json": fallback},
                                finish_after_write=True) as case:
            self.assertIs(case["caught"], first)
            self.assertIsNone(case["code"])
            self.assertTrue(self.manifest_valid(case["evidence"]))
            self.assertFalse((case["evidence"] / "finalization-error.json").exists())
            provenance = json.loads((case["evidence"] / "provenance.json").read_bytes())
            self.assertTrue(provenance["finalization_candidate"])
            self.assertFalse(provenance["output_binary_accepted"])
            self.assertNotIn("finish-return", case["events"])
            self.assertTrue(any("receipt also failed" in note for note in first.__notes__))

    def test_success_and_acceptance_failure_behavior_are_unchanged(self):
        for failure in (False, True):
            with self.subTest(acceptance_failure=failure), self.compound_case(acceptance_failure=failure) as case:
                self.assertEqual(case["code"], int(failure))
                self.assertIsNone(case["caught"])
                self.assertTrue(self.manifest_valid(case["evidence"]))
                self.assertFalse((case["evidence"] / "finalization-error.json").exists())
                record = json.loads((case["evidence"] / "provenance.json").read_bytes())
                self.assertFalse(record["output_binary_accepted"])
                self.assertEqual(record["finalization_candidate"], not failure)
                self.assertLess(case["events"].index("finish-return"), case["events"].index("main-return"))

    def test_partial_provenance_bytes_do_not_become_confirmed_primary_evidence(self):
        first, later = KeyboardInterrupt("first required interruption"), OSError("partial provenance write")
        with self.compound_case({"commands.json": first, "provenance.json": later},
                                partial_provenance=True) as case:
            self.assertIs(case["caught"], first)
            self.assertEqual((case["evidence"] / "provenance.json").read_bytes(), b'{"partial":')
            receipt = json.loads((case["evidence"] / "finalization-error.json").read_bytes())
            self.assert_error_record(receipt["error"], first, "commands.json", "required-output")
            self.assert_error_record(receipt["secondary_errors"][0], later, "provenance.json", "provenance-write")
            self.assertEqual(receipt["required_output_states"]["provenance.json"],
                             "write-failed-persistence-unconfirmed")
            self.assertFalse(self.manifest_valid(case["evidence"]))
            self.assertFalse(receipt["output_binary_accepted"])


class ControllerInputTests(unittest.TestCase):
    rejected = ("OPENSSL_CONF", "OPENSSL_CONF_INCLUDE", "OPENSSL_MODULES",
                "SSLKEYLOGFILE", "SSL_CERT_FILE", "SSL_CERT_DIR")

    def test_exact_six_presence_rejections_precede_every_main_and_execute_action(self):
        for name in self.rejected:
            for value in ("", "secret-not-to-be-read"):
                for entry in ("main", "execute"):
                    with self.subTest(name=name, empty=not value, entry=entry), tempfile.TemporaryDirectory() as temp:
                        work, evidence = Path(temp) / "work", Path(temp) / "evidence"
                        provenance = {}
                        with ExitStack() as patches:
                            patches.enter_context(patch.dict(os.environ, {name: value}, clear=True))
                            patches.enter_context(patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]))
                            blocked = []
                            for target, attribute in ((Path, "read_bytes"), (Path, "mkdir"), (b, "closure"),
                                    (b, "check_gate"), (b, "fetch_github_json"), (b, "encoded"),
                                    (b, "bind_tools"), (b.os, "getuid"), (b.os, "uname"), (b, "digest"),
                                    (b.Runner, "command"), (b.Fetcher, "fetch"),
                                    (b.urllib.request, "build_opener")):
                                blocked.append(patches.enter_context(patch.object(
                                    target, attribute, side_effect=AssertionError("controller action before check"))))
                            with self.assertRaises(b.Error) as raised:
                                b.main() if entry == "main" else b.execute(None, None, work, provenance, None, None)
                            self.assertIn(name, str(raised.exception))
                            self.assertNotIn("secret-not-to-be-read", str(raised.exception))
                            self.assertIn("rejected before any controller action", str(raised.exception))
                            for action in blocked:
                                action.assert_not_called()
                        self.assertFalse(work.exists())
                        self.assertFalse(evidence.exists())
                        self.assertEqual(provenance, {})

    def test_inventory_reads_only_names_sorted_with_categories_and_no_values(self):
        class NamesOnly(dict):
            def __getitem__(self, key):
                raise AssertionError("values forbidden")
            def items(self):
                raise AssertionError("values forbidden")
            def values(self):
                raise AssertionError("values forbidden")
        names = ("GOROOT_1_24_ARM64", "GOFLAGS", "CGO_CFLAGS", "CC", "SDKROOT", "DYLD_INSERT_LIBRARIES",
                 "LD_LIBRARY_PATH", "HtTpS_PrOxY", "no_proxy", "OPENSSL_ENGINES", "OPENSSL_armcap",
                 "QLOGDIR", "SSL_cert_file", "GH_TOKEN", "GITHUB_SHA", "irrelevant")
        inventory = b.controller_inputs(NamesOnly.fromkeys(reversed(names), "value-is-not-observable"))
        self.assertEqual(inventory["proxy"], ["HtTpS_PrOxY", "no_proxy"])
        self.assertEqual(inventory["go_compiler_sdk_search"],
                         ["CC", "CGO_CFLAGS", "GOFLAGS", "GOROOT_1_24_ARM64", "SDKROOT"])
        self.assertEqual(inventory["loader"], ["DYLD_INSERT_LIBRARIES", "LD_LIBRARY_PATH"])
        self.assertEqual(inventory["native_reference"],
                         ["OPENSSL_ENGINES", "OPENSSL_armcap", "QLOGDIR", "SSL_cert_file"])
        self.assertEqual(inventory["declared_control_plane"], ["GH_TOKEN", "GITHUB_SHA"])
        self.assertNotIn("value-is-not-observable", json.dumps(inventory))
        self.assertNotIn("irrelevant", json.dumps(inventory))

    def test_presence_is_case_sensitive_but_proxy_category_is_case_insensitive(self):
        names = [n.lower() for n in self.rejected]
        names += ["HTTP_PROXY", "https_proxy", "aLl_PrOxY", "NO_PROXY", "CuStOm_PrOxY"]
        result = b.controller_inputs(dict.fromkeys(names, "synthetic"))
        self.assertEqual(result["proxy"], sorted(names[6:]))
        self.assertEqual(result["native_reference"], sorted(names[:6]))

    def test_names_only_inventory_escapes_keys_and_fails_on_existing_cap(self):
        inventory = b.controller_inputs({'GO"\nFIELD': "never-retain"})
        self.assertIn('GO"\nFIELD', inventory["go_compiler_sdk_search"])
        self.assertNotIn("never-retain", b.encoded(inventory).decode())
        with self.assertRaisesRegex(b.Error, "existing per-file cap") as raised:
            b.controller_inputs({"GO" + "x" * 4194304: "secret"})
        self.assertLess(len(str(raised.exception)), 100)
        self.assertNotIn("secret", str(raised.exception))

    def test_pre_gate_provenance_inventory_and_failure_contain_names_only(self):
        with tempfile.TemporaryDirectory() as temp:
            work, evidence = Path(temp) / "work", Path(temp) / "evidence"
            environment = {"GOROOT_1_24_ARM64": "SENTINEL", "HTTPS_PROXY": "https://secret.invalid/",
                           "GH_TOKEN": "TOKEN_SENTINEL", "OPENSSL_TRACE": "TRACE_SENTINEL"}
            observed = {}
            original_execute = b.execute
            def execute(*args):
                observed["provenance"] = args[3]
                return original_execute(*args)
            def gate(*args):
                self.assertEqual(observed["provenance"]["inherited_input_names"],
                                 b.controller_inputs(environment))
                raise b.Error("synthetic missing freeze")
            with patch.dict(os.environ, environment, clear=True), \
                    patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]), \
                    patch.object(b, "execute", side_effect=execute), patch.object(b, "check_gate", side_effect=gate), \
                    patch.object(b.Runner, "command") as command:
                self.assertEqual(b.main(), 1)
            command.assert_not_called()
            receipt = (evidence / "provenance.json").read_text()
            for value in environment.values():
                self.assertNotIn(value, receipt)
            self.assertIn("HTTPS_PROXY", receipt)
            self.assertFalse(json.loads(receipt)["output_binary_accepted"])

    def test_none_environment_never_reaches_child_launcher(self):
        with RunnerTests().fake() as (runner, _, spawn, _, _), self.assertRaisesRegex(b.Error, "explicit child"):
            runner.command("static", ["/fixed/tool"], None)
        spawn.assert_not_called()

    def test_inventory_overflow_is_early_and_creates_no_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            work, evidence = Path(temp) / "work", Path(temp) / "evidence"
            # A synthetic mapping avoids putting an oversized name in the real process environment.
            with patch.object(b.os, "environ", {"GO" + "x" * 4194304: "not-read"}), \
                    patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]), \
                    patch.object(Path, "read_bytes") as bounds_read, patch.object(b, "check_gate") as gate, \
                    self.assertRaisesRegex(b.Error, "existing per-file cap"):
                b.main()
            bounds_read.assert_not_called()
            gate.assert_not_called()
            self.assertFalse(work.exists())
            self.assertFalse(evidence.exists())


class DirectTransportTests(unittest.TestCase):
    @contextmanager
    def transport(self, responses):
        calls, opener_handlers, discoveries = [], [], []
        original_build = b.urllib.request.build_opener
        original_global = b.urllib.request._opener
        class Transport(b.urllib.request.HTTPSHandler, b.urllib.request.HTTPHandler):
            def __init__(self):
                pass
            def https_open(self, request):
                calls.append({"url": request.full_url, "headers": dict(request.header_items()),
                              "timeout": request.timeout})
                status, headers, raw = responses.pop(0)
                msg = Message()
                for key, value in headers.items():
                    msg[key] = str(value)
                response = urllib.response.addinfourl(io.BytesIO(raw), msg, request.full_url, status)
                response.msg = "synthetic"
                return response
            http_open = https_open
        def build(*handlers):
            self.assertEqual([h.proxies for h in handlers if isinstance(h, b.urllib.request.ProxyHandler)], [{}])
            opener_handlers.append([type(h).__name__ for h in handlers])
            return original_build(*handlers, Transport())
        def forbidden_discovery(*args, **kwargs):
            discoveries.append("proxy-discovery-or-bypass")
            raise AssertionError("environment/Darwin proxy lookup forbidden")
        with ExitStack() as patches:
            patches.enter_context(patch.dict(os.environ, {"HTTP_PROXY": "SENTINEL", "no_proxy": "SENTINEL",
                "hTtPs_PrOxY": "SENTINEL"}, clear=True))
            patches.enter_context(patch.object(b.urllib.request, "build_opener", side_effect=build))
            install = patches.enter_context(patch.object(b.urllib.request, "install_opener"))
            patches.enter_context(patch("socket.socket", side_effect=AssertionError("no sockets")))
            for name in ("getproxies", "getproxies_environment", "getproxies_macosx_sysconf",
                         "_get_proxy_settings", "proxy_bypass", "proxy_bypass_environment",
                         "proxy_bypass_macosx_sysconf", "_proxy_bypass_macosx_sysconf"):
                patches.enter_context(patch.object(b.urllib.request, name, side_effect=forbidden_discovery, create=True))
            yield calls, opener_handlers, discoveries
        install.assert_not_called()
        self.assertIs(b.urllib.request._opener, original_global)
        self.assertEqual(discoveries, [])

    def test_both_actual_gate_GET_kinds_use_empty_proxies_with_auth_only_at_canonical_URLs(self):
        comment = GateTests().comments()
        review = {"user": {"login": "lossyrob"},
                  "html_url": "https://github.com/lossyrob/telex/issues/172#issuecomment-123",
                  "issue_url": "https://api.github.com/repos/lossyrob/telex/issues/172",
                  "body": "synthetic approved body"}
        responses = [(200, {}, json.dumps([comment]).encode()), (200, {}, json.dumps(review).encode())]
        with tempfile.TemporaryDirectory() as temp, self.transport(responses) as (calls, handlers, discoveries), \
                patch.dict(os.environ, GateTests().environment()), \
                patch.object(b.github, "fetch_json", side_effect=AssertionError("old proxy-aware getter unused")):
            b.check_gate(BOUNDS, "b" * 64, "c" * 64, b.Evidence(Path(temp), BOUNDS["limits"]), float("inf"))
            self.assertEqual([r["url"] for r in calls], [
                "https://api.github.com/repos/lossyrob/telex/issues/172/comments?per_page=100&page=1",
                "https://api.github.com/repos/lossyrob/telex/issues/comments/123"])
            self.assertTrue(all(r["headers"]["Authorization"] == "Bearer synthetic" for r in calls))
            self.assertEqual(handlers, [["RefuseRedirects", "ProxyHandler"]] * 2)
            self.assertFalse(discoveries)

    def test_all_gate_redirect_statuses_same_and_cross_origin_never_follow(self):
        for endpoint in ("issues/172/comments?per_page=100&page=1", "issues/comments/123"):
            for code in (301, 302, 303, 307, 308):
                for target in ("https://api.github.com/other", "https://example.invalid/other"):
                    with self.subTest(endpoint=endpoint, code=code, target=target), \
                            self.transport([(code, {"Location": target}, b"redirect")]) as (calls, _, _):
                        with self.assertRaisesRegex(b.Error, "redirect refused"):
                            b.fetch_github_json(endpoint, "synthetic", {
                                "github_request_seconds": 15, "github_response_bytes": 128}, float("inf"))
                        self.assertEqual(len(calls), 1)
                        self.assertEqual(calls[0]["url"], "https://api.github.com/repos/lossyrob/telex/" + endpoint)

    def test_payload_initial_and_redirect_transports_never_discover_proxies(self):
        with tempfile.TemporaryDirectory() as temp, self.transport([
                (302, {"Location": "https://storage.googleapis.com/module"}, b"hop"),
                (200, {}, b"body")]) as (calls, handlers, discoveries):
            ledger = []
            fetcher = b.Fetcher(BOUNDS["limits"], float("inf"), ledger)
            fetcher.fetch("https://proxy.golang.org/module", Path(temp) / "data", "module",
                          ["proxy.golang.org", "storage.googleapis.com"])
            self.assertEqual([r["url"] for r in calls],
                             ["https://proxy.golang.org/module", "https://storage.googleapis.com/module"])
            self.assertTrue(all("Authorization" not in r["headers"] for r in calls))
            self.assertEqual(handlers, [["ReturnRedirect", "ProxyHandler"]])
            self.assertEqual(fetcher.counts["module"], 7)
            self.assertFalse(discoveries)

    def test_gate_read_bounds_errors_deadline_and_response_close_preserved(self):
        limits = {"github_request_seconds": 15, "github_response_bytes": 8}
        for code, raw, expected in ((200, b"123456789", b.Error), (404, b"failed", b.urllib.error.HTTPError),
                                    (200, b'{"cut":', json.JSONDecodeError)):
            with self.subTest(code=code, raw=raw), self.transport([(code, {}, raw)]) as (calls, _, _), \
                    self.assertRaises(expected):
                b.fetch_github_json("issues/172", "synthetic", limits, float("inf"))
            self.assertEqual(len(calls), 1)
        with self.transport([]) as (calls, _, _), patch.object(b.time, "monotonic", return_value=10), \
                self.assertRaisesRegex(b.Error, "deadline"):
            b.fetch_github_json("issues/172", "synthetic", limits, 9)
        self.assertFalse(calls)
        url = "https://api.github.com/repos/lossyrob/telex/issues/172"
        response = Response(url, b"{}")
        with patch.object(b.urllib.request, "build_opener",
                          return_value=SimpleNamespace(open=lambda *args, **kwargs: response)), \
                patch.object(response, "read1", side_effect=OSError("truncated transport")):
            with self.assertRaisesRegex(OSError, "truncated transport"):
                b.fetch_github_json("issues/172", "synthetic", limits, float("inf"))
        self.assertTrue(response.closed)

    def test_gate_getter_is_exact_a303_read_logic_except_explicit_empty_proxy_opener(self):
        import inspect
        expected = inspect.getsource(b.github.fetch_json).replace(
            "def fetch_json(", "def fetch_github_json(").replace(
            "urllib.request.build_opener(RefuseRedirects())",
            "urllib.request.build_opener(github.RefuseRedirects(), urllib.request.ProxyHandler({}))")
        self.assertEqual(inspect.getsource(b.fetch_github_json), expected)

    def test_empty_environment_does_not_fall_back_to_Darwin_system_proxies(self):
        with self.transport([(200, {}, b"{}")]) as (calls, handlers, discoveries), \
                patch.dict(os.environ, {}, clear=True):
            self.assertEqual(b.fetch_github_json("issues/172", "synthetic",
                {"github_request_seconds": 15, "github_response_bytes": 8}, float("inf")), {})
            self.assertEqual(len(calls), 1)
            self.assertFalse(discoveries)
        with tempfile.TemporaryDirectory() as temp, self.transport([(200, {}, b"x")]) as (
                calls, handlers, discoveries), patch.dict(os.environ, {}, clear=True):
            b.Fetcher(BOUNDS["limits"], float("inf"), []).fetch("https://proxy.golang.org/x",
                Path(temp) / "data", "module", ["proxy.golang.org"])
            self.assertEqual(len(calls), 1)
            self.assertFalse(discoveries)

    def test_gate_response_URL_deadline_and_close_errors_never_fall_back(self):
        url = "https://api.github.com/repos/lossyrob/telex/issues/172"
        limits = {"github_request_seconds": 15, "github_response_bytes": 8}
        for kind in ("wrong-url", "deadline", "close-error", "success"):
            with self.subTest(kind=kind):
                response = Response(url if kind != "wrong-url" else "https://example.invalid/", b"{}")
                original_close = response.close
                def close():
                    original_close()
                    if kind == "close-error":
                        raise OSError("synthetic response close")
                with patch.object(response, "close", side_effect=close), \
                        patch.object(b.urllib.request, "build_opener", return_value=SimpleNamespace(
                            open=lambda *args, **kwargs: response)) as constructor, \
                        patch.object(b.time, "monotonic", side_effect=[0, 20] if kind == "deadline" else lambda: 0):
                    if kind == "success":
                        self.assertEqual(b.fetch_github_json("issues/172", "synthetic", limits, 10), {})
                    else:
                        with self.assertRaises(b.Error if kind != "close-error" else OSError):
                            b.fetch_github_json("issues/172", "synthetic", limits, 10)
                self.assertTrue(response.closed)
                self.assertEqual(constructor.call_count, 1)


class FullRecipeEnvironmentTests(unittest.TestCase):
    labels = [
        "host-uname", "host-product", "host-build", "source", "clang-version", "SDK-version",
        "image-create", "image-attach", "patch-check", "patch-apply", "go-version", "go-env",
        "modules", "modules-verify-before", "compile", "modules-verify-after",
        "go-metadata-pre", "ad-hoc-sign", "signature-verify", "signature-display",
        "signature-entitlements", "go-metadata-post", "image-detach",
    ]

    def expected_maps(self, work):
        volume = work / "volume"
        developer = "/Applications/Xcode_26.6.app/Contents/Developer"
        tools = developer + "/Toolchains/XcodeDefault.xctoolchain/usr/bin"
        build = {
            "DEVELOPER_DIR": developer, "TOOLCHAINS": "com.apple.dt.toolchain.XcodeDefault",
            "GOTOOLCHAIN": "local", "GOENV": "off", "GOWORK": "off", "GOOS": "darwin", "GOARCH": "arm64",
            "GOARM64": "v8.0", "CGO_ENABLED": "1", "GOFLAGS": "-mod=readonly", "GOSUMDB": "off",
            "GOVCS": "*:off", "GOPRIVATE": "", "GONOPROXY": "", "GONOSUMDB": "", "GOINSECURE": "",
            "GOTELEMETRY": "off", "GOMAXPROCS": "2", "LANG": "C", "LC_ALL": "C", "NO_COLOR": "1",
            "SDKROOT": developer + "/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk",
            "CC": tools + "/clang", "CXX": tools + "/clang++",
            "PATH": str(volume / "go" / "bin") + ":" + tools + ":/usr/bin:/bin",
            "HOME": str(volume / "home"), "TMPDIR": str(volume / "tmp"), "GOTMPDIR": str(volume / "tmp"),
            "GOCACHE": str(volume / "gocache"), "GOMODCACHE": str(volume / "gomodcache"),
            "GOPATH": str(volume / "gopath"), "GOPROXY": volume.as_uri() + "/proxy",
            "TELEX_HOME": str(volume / "telex-home"), "TELEX_DB": str(volume / "telex.sqlite"),
            "TELEX_INSTALL_ROOT": str(volume / "telex-install"),
        }
        initial = dict(build, HOME=str(work), TMPDIR=str(work))
        detach = {"PATH": "/usr/bin:/bin", "HOME": str(work), "TMPDIR": str(work)}
        return initial, build, detach

    def expected_argv(self, work):
        volume = work / "volume"
        source, unsigned, signed = volume / "source", volume / "output" / "ipsw", volume / "signing" / "ipsw"
        go = str(volume / "go" / "bin" / "go")
        clang = "/Applications/Xcode_26.6.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang"
        ldflags = ("-ldflags=-s -w -X github.com/blacktop/ipsw/cmd/ipsw/cmd.AppVersion=3.1.739-telex-swvers-b401e5777696 "
                   "-X github.com/blacktop/ipsw/cmd/ipsw/cmd.AppBuildCommit="
                   "1f8b657858d7510219f59bf9ba183bf4c0a59104+telex-b401e5777696")
        return [
            ["/usr/bin/uname", "-a"], ["/usr/bin/sw_vers", "-productVersion"], ["/usr/bin/sw_vers", "-buildVersion"],
            ["/usr/bin/git", "-C", str(b.ROOT), "rev-parse", "HEAD"], [clang, "--version"],
            ["/usr/bin/xcrun", "--sdk", "macosx", "--show-sdk-version"],
            ["/usr/bin/hdiutil", "create", "-size", "12288m", "-fs", "APFS", "-volname", "TelexDerivedBuild",
             "-type", "UDIF", str(work / "build.dmg")],
            ["/usr/bin/hdiutil", "attach", "-readwrite", "-nobrowse", "-noautoopen", "-mountpoint",
             str(volume), "-plist", str(work / "build.dmg")],
            ["/usr/bin/git", "-C", str(source), "apply", "--check", "--whitespace=error-all", str(volume / "get-build-info.patch")],
            ["/usr/bin/git", "-C", str(source), "apply", "--whitespace=error-all", str(volume / "get-build-info.patch")],
            [go, "version"],
            [go, "env", "-json", "GOTOOLCHAIN", "GOENV", "GOWORK", "GOOS", "GOARCH", "GOARM64", "CGO_ENABLED",
             "GOFLAGS", "GOPROXY", "GOSUMDB", "GOVCS", "GOPRIVATE", "GONOPROXY", "GONOSUMDB", "GOINSECURE",
             "GOMODCACHE", "GOCACHE", "GOPATH", "CC", "CXX"],
            [go, "mod", "download", "-json"], [go, "mod", "verify"],
            [go, "build", "-mod=readonly", "-trimpath", "-buildvcs=false", "-p=2", "-tags=sandbox",
             ldflags, "-o=" + str(unsigned), "./cmd/ipsw"],
            [go, "mod", "verify"], [go, "version", "-m", str(unsigned)],
            ["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none", "--identifier",
             "org.telex.harness.ipsw.stdout-only", str(signed)],
            ["/usr/bin/codesign", "--verify", "--strict", "--verbose=4", str(signed)],
            ["/usr/bin/codesign", "--display", "--verbose=4", str(signed)],
            ["/usr/bin/codesign", "--display", "--entitlements", ":-", str(signed)],
            [go, "version", "-m", str(signed)],
            ["/usr/bin/hdiutil", "detach", str(volume)],
        ]

    @contextmanager
    def recipe(self, fail_label=None, go_env_mismatch=False, fail_fetch=False,
               go_env_override=None, go_env_omit=None, pre_values=None, post_values=None, metadata_ldflags=None):
        with tempfile.TemporaryDirectory() as temporary:
            work, evidence = Path(temporary) / "work", Path(temporary) / "evidence"
            volume = work / "volume"
            initial, build, detach = self.expected_maps(work)
            argvs = self.expected_argv(work)
            sequence, streams, events, runners = [], {}, [], []
            environment = GateTests().environment()
            environment["ImageVersion"] = "synthetic-image"
            for name in ("GOROOT_1_24_ARM64", "GOFLAGS", "GOEXPERIMENT", "CGO_CFLAGS", "CC", "CXX",
                         "SDKROOT", "DEVELOPER_DIR", "TOOLCHAINS", "CFLAGS", "LIBRARY_PATH", "CPATH",
                         "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "DYLD_INSERT_LIBRARIES", "LD_PRELOAD",
                         "HTTP_PROXY", "https_proxy", "nO_PrOxY", "OPENSSL_TRACE", "OPENSSL_ENGINES",
                         "OPENSSL_armcap", "RANDFILE", "QLOGDIR", "AMBIENT_UNRELATED"):
                environment[name] = "AMBIENT_SENTINEL"
            environment["GH_TOKEN"] = "CONTROLLER_ONLY_TOKEN"
            original_init, original_stat, original_resolve = b.Runner.__init__, Path.stat, Path.resolve
            original_hash = b.file_hash

            def init(runner, *args, **kwargs):
                original_init(runner, *args, **kwargs)
                runners.append(runner)

            def gate(bounds, bounds_hash, closure_hash, sink, deadline):
                events.append("gate")
                sink.json("gate.json", {"synthetic": True})
                return "a" * 40

            def resolve(file, strict=False):
                if str(file) in BOUNDS["tools"].values() or str(file) == BOUNDS["compiler_identity"]["SDKROOT"]:
                    return Path(BOUNDS["tools"]["clang"]) if str(file) == BOUNDS["tools"]["clangxx_nominal"] else file
                return original_resolve(file, strict=strict)

            def stat_file(file, *args, **kwargs):
                result = original_stat(file, *args, **kwargs)
                if file == volume:
                    fields = list(result)
                    fields[2] += 1
                    return os.stat_result(fields)
                return result

            def unpack(archive, destination, root_name, maximum, deadline):
                destination.mkdir(parents=True)
                if root_name == "go":
                    (destination / "bin").mkdir()
                    (destination / "bin" / "go").write_bytes(b"synthetic-go")
                    (destination / "LICENSE").write_bytes(b"synthetic-license")
                else:
                    (destination / "go.mod").write_bytes((HERE / "inputs" / "go.mod").read_bytes())
                    (destination / "go.sum").write_bytes((HERE / "inputs" / "go.sum").read_bytes())
                    (destination / "LICENSE").write_bytes(b"synthetic-license")
                return 0

            def fetch(fetcher, url, destination, kind, hosts, file_limit=None):
                events.append("fetch-" + kind)
                if fail_fetch:
                    raise b.Error("synthetic execute payload failure")
                destination.parent.mkdir(parents=True, exist_ok=True)
                raw = b"synthetic"
                if destination.suffix == ".info":
                    version = destination.name[:-5]
                    raw = json.dumps({"Version": version}).encode()
                destination.write_bytes(raw)
                if kind == "compiler":
                    return {"path": str(destination), "bytes": 68196737,
                            "sha256": "dd3908cf20959d798c1e06ebe5b72213f0505ce01373b40ce3110975a6cd99c2"}
                return {"path": str(destination), "bytes": len(raw), "sha256": b.digest(raw)}

            def spawn(argv, **kwargs):
                self.assertTrue(runners)
                label = runners[-1].records[-1]["label"]
                index = self.labels.index(label)
                self.assertEqual(argv, argvs[index])
                expected_env = initial if index < 8 else detach if label == "image-detach" else build
                self.assertIsInstance(kwargs["env"], dict)
                self.assertEqual(kwargs["env"], expected_env)
                self.assertNotIn("AMBIENT_SENTINEL", kwargs["env"].values())
                self.assertNotIn("CONTROLLER_ONLY_TOKEN", kwargs["env"].values())
                self.assertNotIn("GH_TOKEN", kwargs["env"])
                self.assertEqual(os.environ, environment)
                self.assertTrue(kwargs["start_new_session"])
                sequence.append({"label": label, "argv": list(argv), "env": kwargs["env"].copy()})
                events.append("spawn-" + label)
                stdout, stderr = b"", b""
                if label == "host-uname": stdout = b"Darwin synthetic\n"
                elif label == "host-product": stdout = b"26.6.2\n"
                elif label == "host-build": stdout = b"25G83\n"
                elif label == "source": stdout = b"a" * 40 + b"\n"
                elif label == "clang-version": stdout = b"Apple clang version synthetic\n"
                elif label == "SDK-version": stdout = b"26.5\n"
                elif label == "image-create": (work / "build.dmg").write_bytes(b"tiny-fake-image")
                elif label == "image-attach":
                    stdout = b.plistlib.dumps({"system-entities": [{"mount-point": str(volume)}]})
                elif label == "go-version": stdout = b"go version go1.27.3 darwin/arm64\n"
                elif label == "go-env":
                    effective = {name: build[name] for name in argv[3:]}
                    effective["GOENV"] = ""
                    if go_env_mismatch: effective["CC"] = "/synthetic/unfrozen"
                    if go_env_override is not None: effective.update(go_env_override)
                    if go_env_omit is not None: del effective[go_env_omit]
                    stdout = json.dumps(effective).encode()
                elif label == "compile": (volume / "output" / "ipsw").write_bytes(synthetic_release_binary(pre_values))
                elif label in ("go-metadata-pre", "go-metadata-post"):
                    stdout = synthetic_go_metadata()
                    if metadata_ldflags is not None and label == metadata_ldflags[0]:
                        stdout += ("build\t-ldflags=" + json.dumps(metadata_ldflags[1]) + "\n").encode()
                elif label == "ad-hoc-sign" and post_values is not None:
                    (volume / "signing" / "ipsw").write_bytes(synthetic_release_binary(post_values))
                elif label == "signature-display": stderr = b"Signature=adhoc\nTimestamp=none\n"
                process = Process(argv.copy())
                fd = 100 + len(sequence) * 2
                process.stdout, process.stderr = Pipe(fd), Pipe(fd + 1)
                process.pid = 1000 + len(sequence)
                code = 1 if label == fail_label else 0
                def wait(timeout):
                    process.waited = True
                    return code
                process.wait = wait
                streams[fd], streams[fd + 1] = bytearray(stdout), bytearray(stderr)
                return process

            def pipe_read(fd, maximum):
                raw = bytes(streams[fd][:maximum])
                del streams[fd][:maximum]
                return raw

            with ExitStack() as patches:
                patches.enter_context(patch.dict(os.environ, environment, clear=True))
                patches.enter_context(patch.object(b.sys, "argv", ["build.py", str(work), str(evidence)]))
                patches.enter_context(patch.object(b.Runner, "__init__", init))
                patches.enter_context(patch.object(b, "check_gate", side_effect=gate))
                patches.enter_context(patch.object(b.sys, "platform", "darwin"))
                patches.enter_context(patch.object(b.os, "getuid", return_value=501))
                patches.enter_context(patch.object(b.os, "uname", return_value=SimpleNamespace(machine="arm64")))
                patches.enter_context(patch.object(Path, "resolve", resolve))
                patches.enter_context(patch.object(Path, "stat", stat_file))
                patches.enter_context(patch.object(b.os, "statvfs",
                    return_value=SimpleNamespace(f_blocks=1024, f_frsize=4096, f_bavail=16777216)))
                patches.enter_context(patch.object(b, "file_hash",
                    side_effect=lambda file, *args: original_hash(file, *args)
                    if file in (volume / "output" / "ipsw", volume / "signing" / "ipsw") else
                    {"path": str(file), "bytes": 1, "sha256": "synthetic-stable"}))
                patches.enter_context(patch.object(b, "unpack", side_effect=unpack))
                patches.enter_context(patch.object(b, "verify_tree"))
                patches.enter_context(patch.object(b, "archive_bytes", return_value=b"synthetic-archive"))
                patches.enter_context(patch.object(b.urllib.request, "build_opener", return_value=object()))
                patches.enter_context(patch.object(b.Fetcher, "fetch", fetch))
                patches.enter_context(patch.object(b.subprocess, "Popen", side_effect=spawn))
                patches.enter_context(patch.object(b.selectors, "DefaultSelector", side_effect=Poller))
                patches.enter_context(patch.object(b.os, "read", side_effect=pipe_read))
                patches.enter_context(patch.object(b.os, "killpg", side_effect=AssertionError("unexpected real signal")))
                patches.enter_context(patch.object(b.time, "monotonic", return_value=0))
                code = b.main()
                self.assertEqual(os.environ, environment)
            provenance = json.loads((evidence / "provenance.json").read_bytes())
            self.assertFalse(provenance["output_binary_accepted"])
            self.assertEqual(provenance["inherited_input_names"], b.controller_inputs(environment))
            for value in ("AMBIENT_SENTINEL", "CONTROLLER_ONLY_TOKEN"):
                self.assertNotIn(value, (evidence / "provenance.json").read_text())
            self.assertTrue((work / "build.dmg").is_file())
            self.assertEqual(events[0], "gate")
            records = json.loads((evidence / "commands.json").read_bytes())
            saved_streams = {}
            for record in records:
                self.assertTrue(record["complete"])
                saved_streams[record["label"]] = {}
                for name, stream in record["streams"].items():
                    raw = (evidence / stream["file"]).read_bytes()
                    self.assertTrue(stream["eof"] and stream["persisted"])
                    self.assertEqual(len(raw), stream["bytes"])
                    self.assertEqual(b.digest(raw), stream["sha256"])
                    saved_streams[record["label"]][name] = raw.decode("utf-8")
            yield {"code": code, "sequence": sequence, "expected_argv": argvs,
                   "expected_maps": {"initial-control": initial, "volume-build": build, "minimal-detach": detach},
                   "provenance": provenance, "events": events,
                   "command_records": records, "command_streams": saved_streams,
                   "manifest_valid": LateRetentionTests().manifest_valid(evidence),
                   "export_present": (evidence / "ipsw").exists(),
                   "backing_retained": (work / "build.dmg").is_file()}

    def test_all_23_actual_Runner_invocations_have_exact_independent_maps_and_order(self):
        with self.recipe() as result:
            self.assertEqual(result["code"], 0)
            self.assertEqual([r["label"] for r in result["sequence"]], self.labels)
            self.assertEqual(len(result["sequence"]), 23)
            self.assertTrue(result["manifest_valid"])
            self.assertTrue(result["provenance"]["finalization_candidate"])
            self.assertEqual(result["provenance"]["build_environment"], result["expected_maps"]["volume-build"])
            self.assertEqual(result["provenance"]["go_environment"]["GOENV"], "")
            self.assertEqual(result["expected_maps"]["volume-build"]["GOENV"], "off")
            self.assertEqual(result["provenance"]["go_environment_expected"], result["provenance"]["go_environment"])
            for phase in ("pre_explicit_sign", "post_sign"):
                matches = result["provenance"][phase]["release_identity"]["matches"]
                self.assertEqual(matches["AppVersion"]["value"], BOUNDS["build"]["version"])
                self.assertEqual(matches["AppBuildCommit"]["value"], BOUNDS["build"]["commit_string"])
            self.assertNotIn("-ldflags", result["provenance"]["pre_sign_build_metadata"]["settings"])
            self.assertNotIn("-ldflags", result["provenance"]["post_sign_build_metadata"]["settings"])
            self.assertEqual([r["label"] for r in result["sequence"]].count("clang-version"), 1)
            self.assertEqual([r["label"] for r in result["sequence"]].count("SDK-version"), 1)

    def test_detach_after_attach_failure_is_minimal_even_without_build_environment_provenance(self):
        with self.recipe(fail_label="image-attach") as result:
            self.assertEqual(result["code"], 1)
            self.assertEqual([r["label"] for r in result["sequence"]], self.labels[:8] + ["image-detach"])
            self.assertNotIn("build_environment", result["provenance"])
            self.assertEqual(result["sequence"][-1]["env"], result["expected_maps"]["minimal-detach"])
            self.assertFalse(any(e.startswith("fetch-") for e in result["events"]))

    def test_detach_after_execute_payload_failure_uses_minimal_map(self):
        with self.recipe(fail_fetch=True) as result:
            self.assertEqual(result["code"], 1)
            self.assertEqual([r["label"] for r in result["sequence"]], self.labels[:8] + ["image-detach"])
            self.assertIn("synthetic execute payload failure", result["provenance"]["failure"]["message"])
            self.assertEqual(result["sequence"][-1]["env"], result["expected_maps"]["minimal-detach"])

    def test_effective_go_env_mismatch_still_stops_before_modules_and_compile(self):
        with self.recipe(go_env_mismatch=True) as result:
            self.assertEqual(result["code"], 1)
            self.assertEqual([r["label"] for r in result["sequence"]], self.labels[:12] + ["image-detach"])
            self.assertEqual(result["provenance"]["failure"]["message"], "effective Go environment mismatch: CC")
            self.assertNotIn("spawn-compile", result["events"])


class PinnedGoOutputTests(unittest.TestCase):
    def expectations(self):
        work = Path("/synthetic-frozen-work")
        recipe = FullRecipeEnvironmentTests()
        environment = recipe.expected_maps(work)[1]
        keys = recipe.expected_argv(work)[11][3:]
        expected = {key: environment[key] for key in keys}
        expected["GOENV"] = ""
        return environment, expected

    def test_effective_map_has_all_20_keys_only_GOENV_differs_and_inputs_stay_unchanged(self):
        environment, expected = self.expectations()
        before, ambient = environment.copy(), os.environ.copy()
        self.assertEqual(len(expected), 20)
        self.assertEqual(b.validate_go_environment(expected, environment), expected)
        self.assertEqual([key for key in expected if expected[key] != environment[key]], ["GOENV"])
        self.assertEqual(environment["GOENV"], "off")
        self.assertEqual(environment, before)
        self.assertEqual(os.environ, ambient)

    def test_each_of_20_missing_or_wrong_effective_keys_fails(self):
        environment, expected = self.expectations()
        for key in expected:
            for mutation in ("missing", "wrong"):
                with self.subTest(key=key, mutation=mutation):
                    observed = expected.copy()
                    if mutation == "missing":
                        del observed[key]
                    else:
                        observed[key] = expected[key] + "-wrong"
                    with self.assertRaisesRegex(b.Error, "effective Go environment mismatch: " + key + "$"):
                        b.validate_go_environment(observed, environment)
        for observed in (None, [], "", 20):
            with self.subTest(shape=type(observed).__name__), self.assertRaises(b.Error):
                b.validate_go_environment(observed, environment)

    def test_metadata_requires_absent_ldflags_including_correct_or_empty_values(self):
        flags = next(f.split("=", 1)[1] for f in BOUNDS["build"]["flags"] if f.startswith("-ldflags="))
        for value in (flags, "", "wrong"):
            with self.subTest(value=value), self.assertRaisesRegex(b.Error, "must omit -ldflags"):
                b.go_metadata(synthetic_go_metadata() + ("build\t-ldflags=" + json.dumps(value) + "\n").encode(), BOUNDS)
        settings = b.go_metadata(synthetic_go_metadata(), BOUNDS)["settings"]
        for key, value in settings.items():
            row = ("build\t" + key + "=" + value + "\n").encode()
            for replacement in (b"", row.replace(value.encode(), (value + "-wrong").encode())):
                with self.subTest(key=key, replacement=replacement), self.assertRaises(b.Error):
                    b.go_metadata(synthetic_go_metadata().replace(row, replacement), BOUNDS)

    def test_both_complete_release_values_have_static_evidence_pre_and_post_sign(self):
        values = {"AppVersion": BOUNDS["build"]["version"], "AppBuildCommit": BOUNDS["build"]["commit_string"]}
        raw = synthetic_release_binary()
        with tempfile.TemporaryDirectory() as temp, patch.object(b.subprocess, "Popen") as process:
            file = Path(temp) / "ipsw"
            file.write_bytes(raw)
            for signed in (False, True):
                result = b.macho_signature(file, len(raw), float("inf"), signed,
                                           BOUNDS["signing"]["identifier"], release_values=values)
                self.assertEqual(result["sha256"], b.digest(raw))
                for name, value in values.items():
                    match = result["release_identity"]["matches"][name]
                    self.assertEqual(match["value"], value)
                    self.assertEqual(raw[match["first_file_offset"]:
                                         match["first_file_offset"] + match["utf8_bytes"]], value.encode("utf-8"))
            process.assert_not_called()

    def test_missing_shortened_or_wrong_one_or_both_values_fail_even_with_valid_macho(self):
        values = {"AppVersion": BOUNDS["build"]["version"], "AppBuildCommit": BOUNDS["build"]["commit_string"]}
        cases = [{name: wrong} for name, value in values.items() for wrong in (None, value[:-1], "wrong-" + value[1:])]
        cases += [dict.fromkeys(values, None), {name: value[:-1] for name, value in values.items()},
                  {"AppVersion": "AppVersion", "AppBuildCommit": "AppBuildCommit"}]
        with tempfile.TemporaryDirectory() as temp:
            file = Path(temp) / "ipsw"
            for overrides in cases:
                file.write_bytes(synthetic_release_binary(overrides))
                for signed in (False, True):
                    with self.subTest(overrides=overrides, signed=signed), self.assertRaisesRegex(b.Error, "identity bytes absent"):
                        b.macho_signature(file, 4096, float("inf"), signed,
                                          BOUNDS["signing"]["identifier"], release_values=values)

    def test_identity_read_is_bounded_hash_matched_and_deadline_checked(self):
        raw = synthetic_release_binary()
        values = {"AppVersion": BOUNDS["build"]["version"], "AppBuildCommit": BOUNDS["build"]["commit_string"]}
        sizes = []
        class Stream(io.BytesIO):
            def read(self, size=-1):
                sizes.append(size)
                return super().read(size)
        for data, elapsed, digest in ((raw, 0, b.digest(raw)), (raw + b"x", 0, b.digest(raw)),
                                      (raw[:-1] + b"x", 0, b.digest(raw)), (raw, 10, b.digest(raw))):
            with self.subTest(data=data, elapsed=elapsed), \
                    patch.object(b, "file_hash", return_value={"path": "synthetic", "bytes": len(raw), "sha256": digest}), \
                    patch.object(Path, "open", return_value=Stream(data)), patch.object(b.time, "monotonic", return_value=elapsed):
                if data == raw and elapsed == 0:
                    b.macho_signature(Path("synthetic"), len(raw), 10, release_values=values)
                else:
                    with self.assertRaisesRegex(b.Error, "bounded Mach-O bytes"):
                        b.macho_signature(Path("synthetic"), len(raw), 10, release_values=values)
        self.assertEqual(sizes, [len(raw) + 1] * 4)

    def test_old_or_missing_GOENV_stops_real_Runner_recipe_before_modules(self):
        for arguments in ({"go_env_override": {"GOENV": "off"}}, {"go_env_omit": "GOENV"}):
            with self.subTest(arguments=arguments), FullRecipeEnvironmentTests().recipe(**arguments) as result:
                self.assertEqual(result["code"], 1)
                self.assertIn("mismatch: GOENV", result["provenance"]["failure"]["message"])
                self.assertNotIn("spawn-modules", result["events"])
                self.assertTrue(result["backing_retained"])
                self.assertTrue(result["manifest_valid"])
                self.assertFalse(result["export_present"])

    def test_release_identity_failure_before_or_after_signing_prevents_export(self):
        for phase, arguments in (("pre", {"pre_values": {"AppVersion": None}}),
                                 ("post", {"post_values": {"AppBuildCommit": None}})):
            with self.subTest(phase=phase), FullRecipeEnvironmentTests().recipe(**arguments) as result:
                self.assertEqual(result["code"], 1)
                self.assertIn("identity bytes absent", result["provenance"]["failure"]["message"])
                self.assertNotIn("spawn-go-metadata-" + phase, result["events"])
                if phase == "pre":
                    self.assertNotIn("spawn-ad-hoc-sign", result["events"])
                self.assertFalse(result["export_present"])
                self.assertTrue(result["backing_retained"])
                self.assertTrue(result["manifest_valid"])

    def test_unexpected_ldflags_pre_and_post_sign_fail_with_complete_stream_evidence(self):
        flags = next(f.split("=", 1)[1] for f in BOUNDS["build"]["flags"] if f.startswith("-ldflags="))
        for label in ("go-metadata-pre", "go-metadata-post"):
            for value in ("", flags):
                with self.subTest(label=label, value=value), FullRecipeEnvironmentTests().recipe(
                        metadata_ldflags=(label, value)) as result:
                    self.assertEqual(result["code"], 1)
                    self.assertIn("must omit -ldflags", result["provenance"]["failure"]["message"])
                    self.assertIn("spawn-" + label, result["events"])
                    self.assertIn("build\t-ldflags=" + json.dumps(value), result["command_streams"][label]["stdout"])
                    self.assertEqual(result["command_streams"][label]["stderr"], "")
                    self.assertFalse(result["export_present"])
                    self.assertTrue(result["backing_retained"])
                    self.assertTrue(result["manifest_valid"])


class SourceSurfaceTests(unittest.TestCase):
    def test_first_entry_checks_and_complete_launch_network_inventory(self):
        source = (HERE / "build.py").read_text()
        tree = ast.parse(source)
        functions = {n.name: n for n in tree.body if isinstance(n, ast.FunctionDef)}
        for name in ("main", "execute"):
            first = functions[name].body[0]
            self.assertIsInstance(first, ast.Assign)
            self.assertEqual(ast.unparse(first.value), "controller_inputs(os.environ)")
        calls = [(ast.unparse(n.func), n.lineno, n) for n in ast.walk(tree) if isinstance(n, ast.Call)]
        popen = [(name, line, n) for name, line, n in calls if name == "subprocess.Popen"]
        self.assertEqual(len(popen), 1)
        self.assertEqual(next(k.value.id for k in popen[0][2].keywords if k.arg == "env"), "env")
        self.assertEqual(sum(name == "opener.open" for name, _, _ in calls), 1)
        self.assertEqual(sum(name == "self.opener.open" for name, _, _ in calls), 1)
        self.assertEqual(sum(name == "fetch_github_json" for name, _, _ in calls), 2)
        self.assertFalse(any(name in ("github.fetch_json", "urllib.request.urlopen", "urllib.request.install_opener")
                             for name, _, _ in calls))
        constructors = [n for name, _, n in calls if name == "urllib.request.build_opener"]
        self.assertEqual(len(constructors), 2)
        self.assertTrue(all(any(ast.unparse(arg) == "urllib.request.ProxyHandler({})" for arg in n.args)
                            for n in constructors))
        launches = [n for name, _, n in calls if name == "runner.command"]
        self.assertEqual(len(launches), 19)
        self.assertTrue(all(len(n.args) >= 3 and isinstance(n.args[2], ast.Name) and
                            n.args[2].id in ("env", "control_env") for n in launches))
        self.assertFalse(any(isinstance(n, ast.Call) and ast.unparse(n.func) in
                             ("os.environ.update", "os.environ.pop", "os.environ.clear", "os.putenv", "os.unsetenv")
                             for n in ast.walk(tree)))

    def test_module_scope_adds_no_controller_operation_or_import_deferral(self):
        source = (HERE / "build.py").read_text()
        tree = ast.parse(source)
        import_calls = []
        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.ClassDef)):
                decorators = node.decorator_list
                self.assertTrue(all(ast.unparse(d) == "contextmanager" for d in decorators))
                continue
            if isinstance(node, ast.If):
                self.assertEqual(ast.unparse(node.test), "__name__ == '__main__'")
                self.assertEqual(ast.unparse(node.body[0]), "sys.exit(main())")
                continue
            import_calls.extend(ast.unparse(n.func) for n in ast.walk(node) if isinstance(n, ast.Call))
        self.assertEqual(sorted(import_calls), sorted([
            "Path(__file__).resolve", "Path", "importlib.util.spec_from_file_location",
            "importlib.util.module_from_spec", "SPEC.loader.exec_module", "re.compile"]))
        self.assertIn("import hashlib", source)
        self.assertIn("import http.client", source)
        self.assertIn("import urllib.request", source)


class EmptyImageRecipeTests(unittest.TestCase):
    def test_explicit_UDIF_recipe_keeps_hard_capacity_and_initial_reserve_before_fetch(self):
        self.assertEqual(BOUNDS["limits"]["module_and_build_disk_bytes"], 12 * 1024**3)
        self.assertEqual(BOUNDS["limits"]["disk_reserve_bytes"], 4 * 1024**3)
        self.assertEqual(int(BOUNDS["volume"]["create"][2][:-1]) * 1024**2,
                         BOUNDS["limits"]["module_and_build_disk_bytes"])
        with FullRecipeEnvironmentTests().recipe() as result:
            self.assertEqual(result["code"], 0)
            create = result["sequence"][6]
            self.assertEqual(create["label"], "image-create")
            self.assertEqual(create["argv"][:-1], ["/usr/bin/hdiutil", "create", "-size", "12288m",
                "-fs", "APFS", "-volname", "TelexDerivedBuild", "-type", "UDIF"])
            self.assertTrue(create["argv"][-1].endswith("/work/build.dmg"))
            events = result["events"]
            self.assertLess(events.index("spawn-image-create"), events.index("spawn-image-attach"))
            self.assertLess(events.index("spawn-image-attach"), events.index("fetch-source"))
            reserve = result["provenance"]["volume_initial"]
            self.assertTrue(reserve["initial_reserve_satisfied"])
            self.assertEqual(reserve["required_initial_available"],
                             4 * 1024**3 + max(0, reserve["backing_logical_bytes"] - reserve["backing_allocated_bytes"]))
            self.assertTrue(reserve["not_live_floor_or_all_host_writes_bound"])
            self.assertLessEqual(reserve["volume_logical_capacity"], 12 * 1024**3)
            self.assertTrue(result["backing_retained"])

    def test_image_create_failure_keeps_existing_stop_frontier_without_postfailure_stat(self):
        with FullRecipeEnvironmentTests().recipe(fail_label="image-create") as result:
            self.assertEqual(result["code"], 1)
            self.assertEqual([item["label"] for item in result["sequence"]],
                             FullRecipeEnvironmentTests.labels[:7])
            self.assertFalse(result["provenance"]["attach_attempted"])
            self.assertFalse(result["provenance"]["attached"])
            self.assertFalse(result["provenance"]["detached"])
            self.assertNotIn("volume_initial", result["provenance"])
            self.assertNotIn("backing_after", result["provenance"])
            self.assertFalse(any(event.startswith("fetch-") for event in result["events"]))
            self.assertEqual(result["provenance"]["failure"]["message"], "command failed; no retry: image-create")
            self.assertFalse(result["provenance"]["output_binary_accepted"])


if __name__ == "__main__":
    unittest.main()
