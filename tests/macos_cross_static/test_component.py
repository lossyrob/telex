import copy
import hashlib
import importlib.util
import json
import plistlib
from pathlib import Path
import struct
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(1, str(HERE.parent))
import cache
import gate
import native

spec = importlib.util.spec_from_file_location("component_run", HERE / "run.py")
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)


class FreezeTests(unittest.TestCase):
    def setUp(self):
        self.context = {"sha": "a" * 40, "bounds_sha256": "b" * 64, "job": "ARM24",
                        "attempt": 1, "run_id": 12}
        self.record = {"component": "arm24-intel25-static-v1", "sha": "a" * 40,
                       "bounds_sha256": "b" * 64, "jobs": ["ARM24", "Intel25"],
                       "allow_launch": True, "comment": "https://example.invalid/freeze",
                       "decoder_pin_disposition": "https://github.com/lossyrob/telex/issues/172#issuecomment-123",
                       "decoder_consumer_disposition": "https://github.com/lossyrob/telex/issues/172#issuecomment-125",
                       "intel_family_disposition": "https://github.com/lossyrob/telex/issues/172#issuecomment-124"}

    def test_missing_freeze_never_launches(self):
        with self.assertRaises(RuntimeError):
            gate.validate([], self.context)

    def test_exact_freeze(self):
        self.assertFalse(gate.validate([self.record], self.context)["retry"])
        for key, wrong in (("sha", "c" * 40), ("bounds_sha256", "d" * 64),
                           ("jobs", ["ARM24"]), ("allow_launch", False)):
            altered = {**self.record, key: wrong}
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                gate.validate([altered], self.context)

    def test_missing_pin_or_family_disposition_blocks(self):
        for field in ("decoder_pin_disposition", "decoder_consumer_disposition", "intel_family_disposition"):
            record = dict(self.record)
            record.pop(field)
            with self.subTest(field=field), self.assertRaises(RuntimeError):
                gate.validate([record], self.context)

    def test_untrusted_comment_cannot_freeze(self):
        text = "<!-- arm24-intel25-static-v1 " + json.dumps(self.record) + " -->"
        self.assertEqual(gate.approved_records([{"user": {"login": "someone"},
                                                "body": text, "html_url": "url"}]), [])

    def test_retry_only_sole_failed_job_same_sha(self):
        context = {**self.context, "attempt": 2}
        prior = {"head_sha": "a" * 40, "run_attempt": 1,
                 "jobs": [{"name": "ARM24", "id": 10, "conclusion": "failure"},
                          {"name": "Intel25", "id": 11, "conclusion": "success"}]}
        self.record["infrastructure_retry"] = {
            "run_id": 12, "job": "ARM24", "failed_job_id": 10, "same_sha": "a" * 40,
            "classification": "infrastructure", "reason": "runner outage, original attempt evidence preserved"}
        self.assertTrue(gate.validate([self.record], context, prior)["retry"])
        for field, value in (("same_sha", "different"), ("job", "Intel25"),
                             ("classification", "capacity"), ("run_id", 13)):
            record = copy.deepcopy(self.record)
            record["infrastructure_retry"][field] = value
            with self.subTest(field=field), self.assertRaises(RuntimeError):
                gate.validate([record], context, prior)
        for status in ("failure", "cancelled"):
            bad = copy.deepcopy(prior)
            bad["jobs"][1]["conclusion"] = status
            self.assertTrue(gate.validate([self.record], context, bad)["retry"])
        bad = copy.deepcopy(prior)
        bad["jobs"][1]["conclusion"] = "in_progress"
        with self.assertRaises(RuntimeError):
            gate.validate([self.record], context, bad)
        for job in ("Intel25",):
            with self.assertRaises(RuntimeError):
                gate.validate([self.record], {**context, "job": job}, prior)
        with self.assertRaises(RuntimeError):
            gate.validate([self.record], {**context, "attempt": 3}, prior)


class StaticReaderTests(unittest.TestCase):
    def test_family_selection_has_no_alternate(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            c = run.BOUNDS["jobs"]["Intel25"]
            selected = root / c["cache_primary_candidates"][0]
            with self.assertRaises(cache.EvidenceError):
                cache.choose_family(root, c)
            selected.parent.mkdir(parents=True)
            selected.write_bytes(b"selected")
            self.assertEqual(cache.choose_family(root, c), selected)
            alternative = root / c["ambiguity_checks"][0]
            alternative.write_bytes(b"not authorized fallback")
            with self.assertRaises(cache.EvidenceError):
                cache.choose_family(root, c)
            alternative.unlink()
            duplicate = root / c["cache_primary_candidates"][1]
            duplicate.parent.mkdir(parents=True)
            duplicate.write_bytes(b"duplicate")
            with self.assertRaises(cache.EvidenceError):
                cache.choose_family(root, c)

    def test_branch_cfg_does_not_follow_neighbor_stub(self):
        decoded = [
            {"address": 0x1000, "mnemonic": "jmpq", "operands": "*0x8(%rip)", "raw": ""},
            {"address": 0x1006, "mnemonic": "jmpq", "operands": "*0x10(%rip)", "raw": ""},
        ]
        calls = []

        def pointer(address):
            calls.append(address)
            return 0x2000, {"evidence": "synthetic"}
        branches = cache.edges(decoded, "x86_64", 0x1000, 0x100C, pointer)
        self.assertEqual(calls, [0x100E])
        self.assertEqual(len(branches), 1)
        self.assertEqual(branches[0]["target"], "0x2000")

    def test_decimal_arm_stub_fixup(self):
        base = 0x18BADE310
        ops = [("adrp", "x17, 450937 ; 0x1f9c57000"), ("add", "x17, x17, #0x250"),
               ("ldr", "x16, [x17]"), ("braa", "x16, x17")]
        decoded = [{"address": base + 4*i, "mnemonic": op, "operands": arg, "raw": ""}
                   for i, (op, arg) in enumerate(ops)]
        slots = []

        def pointer(address):
            slots.append(address)
            return 0x1804C38EC, {"raw": "synthetic"}
        branches = cache.edges(decoded, "arm64", base, base + 16, pointer)
        self.assertEqual(slots, [0x1F9C57250])
        self.assertEqual(branches[0]["target"], "0x1804c38ec")

    def test_callbacks_are_explicit_not_invented(self):
        row = {"address": 0x1000, "mnemonic": "blr", "operands": "x8", "raw": ""}
        branches = cache.edges([row], "arm64", 0x1000, 0x1004, lambda _: self.fail())
        self.assertIsNone(branches[0]["target"])
        self.assertIn("unclassified", branches[0]["unresolved"])
        self.assertEqual(cache.unresolved_classification(branches[0]), "unclassified-indirect-owner-scope-review")

    def test_rip_branch_uses_machine_length_not_padding(self):
        row = {"address": 0x1000, "mnemonic": "jmpq", "operands": "*0x8(%rip)", "raw": ""}
        addresses = []
        def pointer(address):
            addresses.append(address)
            return 0x2000, {}
        raw = b"\xff\x25\x08\x00\x00\x00" + b"\x00" * 10
        cache.edges([row], "x86_64", 0x1000, 0x1010, pointer, raw)
        self.assertEqual(addresses, [0x100E])
        with self.assertRaises(cache.EvidenceError):
            cache.edges([row], "x86_64", 0x1000, 0x1010, pointer,
                        b"\xff\x25\x09\x00\x00\x00" + b"\0" * 10)

    def test_output_count_and_byte_bounds(self):
        limits = dict(run.BOUNDS["limits"])
        limits.update(per_file_bytes=8, artifact_files_per_job=4)
        with tempfile.TemporaryDirectory() as d:
            output = cache.Output(Path(d), limits)
            output.write("a.bin", b"1234")
            output.write("b.bin", b"1234")
            with self.assertRaises(cache.EvidenceError):
                output.write("c.bin", b"1")
            with self.assertRaises(cache.EvidenceError):
                output.write("a.bin", b"012345678")

    def test_v2_and_v5_pages(self):
        for version in (2, 5):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as d:
                root = Path(d)
                file = root / "synthetic-cache"
                data = bytearray(8192)
                if version == 2:
                    struct.pack_into("<6I2Q", data, 0, 2, 4096, 40, 1, 42, 0,
                                     0x00FFFF0000000000, 0x180000000)
                    struct.pack_into("<H", data, 40, 4)
                    struct.pack_into("<Q", data, 4096 + 16, 0x1234)
                else:
                    struct.pack_into("<IIIIQ", data, 0, 5, 4096, 1, 0, 0x180000000)
                    struct.pack_into("<H", data, 24, 16)
                    struct.pack_into("<Q", data, 4096 + 16, (1 << 63) | 0x1234)
                file.write_bytes(data)
                c = cache.CandidateCache.__new__(cache.CandidateCache)
                c.pages = {}
                c.output = cache.Output(root, run.BOUNDS["limits"])
                c.mappings = [{"path": file, "address": 0x200000000, "size": 4096,
                               "file_offset": 4096, "slide_offset": 0, "slide_size": 64,
                               "cache_uuid": "synthetic", "record_offset": 0, "record_hex": "00"*56}]
                target, evidence = c.pointer(0x200000010)
                self.assertEqual(target, 0x180001234)
                self.assertEqual(evidence["slot"], "0x200000010")
                with self.assertRaises(cache.EvidenceError):
                    c.pointer(0x200000018)

    def test_export_address_not_name_expectation(self):
        # Root edge "_x" points to a regular-export node with image offset16.
        raw = bytes([0, 1]) + b"_x\0" + bytes([6, 2, 0, 16, 0])
        self.assertEqual(cache.export_entries(raw, 0x1000),
                         [{"name": "_x", "flags": 0, "node": 6, "address": 0x1010}])


class ConfigTests(unittest.TestCase):
    def test_frozen_byte_ranges(self):
        expected = {"ARM24": (15292005, 15655957295), "Intel25": (17953771, 18384623377)}
        for name, c in run.BOUNDS["jobs"].items():
            self.assertEqual((c["range_offset"], c["range_offset"] + c["dmg"]["bytes"] - 1), expected[name])
            self.assertLess(expected[name][1], c["package_bytes"])
            self.assertEqual(sum(c["manifest_variants"].values()), 147 if name == "ARM24" else 136)
            self.assertNotIn("expected_library_uuid", c)

    def test_metadata_hash_precedes_parse(self):
        with self.assertRaises(cache.EvidenceError):
            run.verified_metadata(b"not plist", {"member": "test", "bytes": 9, "sha256": "0"*64})

    def test_query_roots_are_not_silently_included(self):
        self.assertEqual(run.BOUNDS["roots"],
                         ["proc_listpids", "kill", "waitid", "posix_spawn", "execve", "fork", "vfork"])
        self.assertEqual(run.BOUNDS["query_wrapper_scope"]["additional_query_roots_proposed"], [])

    def test_workflow_has_two_disjoint_jobs_and_no_dispatch(self):
        text = (HERE.parents[1] / ".github" / "workflows" / "macos-cross-static.yml").read_text()
        self.assertNotIn("workflow_dispatch", text)
        self.assertEqual(text.count("runs-on:"), 2)
        self.assertIn("  ARM24:\n", text)
        self.assertIn("  Intel25:\n", text)
        self.assertNotIn("matrix:", text)
        for old in ("macos-group-proof.yml", "macos-provenance-capture.yml", "macos-static-ridiff.yml"):
            content = (HERE.parents[1] / ".github" / "workflows" / old).read_text()
            self.assertNotIn("macos_cross_static", content)
            self.assertNotIn("macos-cross-static", content)


def row(address, mnemonic, operands):
    return {"address": address, "mnemonic": mnemonic, "operands": operands, "raw": ""}


class OwnerRegressionTests(unittest.TestCase):
    def stale_sequence(self, overwrite):
        decoded = [row(0x1000, "adrp", "x17, 1 ; 0x2000"),
                   row(0x1004, "add", "x17, x17, #0x20"),
                   row(0x1008, "ldr", "x16, [x17]"), row(0x100C, *overwrite),
                   row(0x1010, "br", "x16")]
        slots = []
        branches = cache.edges(decoded, "arm64", 0x1000, 0x1014,
                               lambda address: (slots.append(address), {}) )
        self.assertEqual(slots, [])
        self.assertIsNone(branches[-1]["target"])
        self.assertIn("unclassified", branches[-1]["unresolved"])

    def test_w_alias_overwrite_cannot_reuse_x_slot(self):
        self.stale_sequence(("mov", "w16, #0"))

    def test_unsupported_sp_ldr_cannot_reuse_destination(self):
        self.stale_sequence(("ldr", "x16, [sp, #0x20]"))

    def test_unsupported_add_register_form_cannot_reuse_destination(self):
        self.stale_sequence(("add", "x16, x0, x1"))

    def test_unsupported_ldr_writeback_cannot_reuse_destination(self):
        self.stale_sequence(("ldr", "x16, [x17, #8]!"))

    def test_register_pair_alias_write_cannot_reuse_destination(self):
        self.stale_sequence(("ldp", "w15, w16, [sp]"))

    def test_reconvergence_retains_ambiguity_not_first_state(self):
        decoded = [
            row(0x1000, "adrp", "x17, 1 ; 0x2000"), row(0x1004, "cbz", "x0, 0x1014"),
            row(0x1008, "add", "x17, x17, #0x20"), row(0x100C, "ldr", "x16, [x17]"),
            row(0x1010, "b", "0x1020"), row(0x1014, "add", "x17, x17, #0x28"),
            row(0x1018, "ldr", "x16, [x17]"), row(0x101C, "b", "0x1020"),
            row(0x1020, "br", "x16")]
        slots = []
        def pointer(address):
            slots.append(address)
            return address + 0x10000, {}
        records = cache.edges(decoded, "arm64", 0x1000, 0x1024, pointer)
        join = next(r for r in records if r["address"] == 0x1020)
        self.assertEqual(set(slots), {0x2020, 0x2028})
        self.assertIsNone(join["target"])
        self.assertEqual({r["identified_slot"] for r in join["ambiguous_states"]}, {"0x2020", "0x2028"})
        self.assertEqual(cache.unresolved_classification(join), "helper-incomplete")

    def test_actual_interior_entry_does_not_reach_prefix(self):
        decoded = [row(0x1000, "jmpq", "0x2000"), row(0x1005, "jmpq", "0x3000")]
        branches = cache.edges(decoded, "x86_64", 0x1000, 0x100A, lambda _: self.fail(), entry=0x1005)
        self.assertEqual([r["target"] for r in branches], ["0x3000"])
        with self.assertRaises(cache.EvidenceError):
            cache.edges(decoded, "x86_64", 0x1000, 0x100A, lambda _: self.fail(), entry=0x1003)

    def fixture_collect(self, roots, entries, raw, decoded, fail_pointer=False):
        class MemoryOutput:
            root = Path("synthetic-not-created")
            limits = run.BOUNDS["limits"]
            def write(self, name, data):
                return name
            def json(self, name, data):
                return name

        class Fixture:
            images = [{"name": "/usr/lib/system/libsystem_c.dylib", "address": 0x1000},
                      {"name": "/usr/lib/system/libsystem_kernel.dylib", "address": 0x4000}]
            def macho(self, image):
                selected = [r for r in roots if ("libsystem_c" in image["name"]) == (r in ("fork", "vfork"))]
                return {**image, "uuid": image["name"], "cpu": 0x1000007, "subtype": 3,
                        "header_file": "fixture", "symbols": [
                            {"name": "_" + r, "address": entries[r], "flags": 0} for r in selected]}
            def body(self, address):
                if address not in entries.values():
                    raise cache.EvidenceError("bounded synthetic external helper unavailable")
                image = self.images[0 if roots[0] in ("fork", "vfork") else 1]
                return self.macho(image), 0x1000, 0x1000 + len(raw), "LC_FUNCTION_STARTS_adjacent", raw
            def pointer(self, address):
                assert fail_pointer and address == 0x100E
                raise cache.EvidenceError("identified import-slot decode failure")
        commands = []
        controller = SimpleNamespace(bounds={"roots": roots}, otool_path="/frozen/otool",
                                     command=lambda name, args: (commands.append(args) or {"stdout": "fixture"}))
        with patch.object(cache, "CandidateCache", return_value=Fixture()), \
                patch.object(cache, "instructions", return_value=decoded):
            result = cache.collect(Path("not-opened"), "x86_64", MemoryOutput(), controller)
        return result, commands

    def test_collect_uses_actual_entry(self):
        decoded = [row(0x1000, "jmpq", "0x2000"), row(0x1005, "jmpq", "0x3000")]
        report, _ = self.fixture_collect(["kill"], {"kill": 0x1005},
                                        bytes.fromhex("e9fb0f0000e9f61f0000"), decoded)
        self.assertEqual(report["captures"][0]["branches"][0]["target"], "0x3000")

    def test_extent_reuse_keeps_later_entry_obligations(self):
        decoded = [row(0x1000, "jmpq", "0x2000"), row(0x1005, "jmpq", "0x3000")]
        report, commands = self.fixture_collect(["kill", "waitid"], {"kill": 0x1000, "waitid": 0x1005},
                                               bytes.fromhex("e9fb0f0000e9f61f0000"), decoded)
        self.assertEqual(len(report["captures"]), 1)
        self.assertEqual(len(commands), 1)
        self.assertEqual(commands[0][0], "/frozen/otool")
        analyses = report["captures"][0]["analyses"]
        self.assertEqual([(a["entry"], a["branches"][0]["target"]) for a in analyses],
                         [("0x1000", "0x2000"), ("0x1005", "0x3000")])

    def test_identified_import_failure_not_callback_exempt(self):
        report, _ = self.fixture_collect(["fork"], {"fork": 0x1000}, bytes.fromhex("ff2508000000"),
            [row(0x1000, "jmpq", "*0x8(%rip)")], fail_pointer=True)
        self.assertEqual(report["helper_incomplete_count"], 1)
        self.assertEqual(report["missing"][0]["classification"], "helper-incomplete")
        self.assertEqual(report["missing"][0]["identified_slot"], "0x100e")

    def test_null_identified_slot_is_incomplete(self):
        records = cache.edges([row(0x1000, "jmpq", "*0x8(%rip)")], "x86_64", 0x1000, 0x1006,
                              lambda address: (0, {"slot": hex(address)}),
                              bytes.fromhex("ff2508000000"))
        self.assertIn("null", records[0]["unresolved"])
        self.assertEqual(cache.unresolved_classification(records[0]), "helper-incomplete")


class ImageContainmentTests(unittest.TestCase):
    def test_ancestor_symlink_escape_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp, tempfile.TemporaryDirectory() as other:
            root = Path(temp).resolve()
            (root / "System").symlink_to(Path(other), target_is_directory=True)
            with self.assertRaises(cache.EvidenceError):
                cache.choose_family(root, run.BOUNDS["jobs"]["Intel25"])

    def test_inaccessible_competitor_is_not_absence(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            c = run.BOUNDS["jobs"]["Intel25"]
            primary = root / c["cache_primary_candidates"][0]
            primary.parent.mkdir(parents=True)
            primary.write_bytes(b"candidate")
            original = Path.lstat
            def denied(path):
                if str(path).endswith("dyld_shared_cache_x86_64h"):
                    raise PermissionError("fixture denied")
                return original(path)
            with patch.object(Path, "lstat", denied), self.assertRaises(cache.EvidenceError):
                cache.choose_family(root, c)

    def test_competitor_dangling_symlink_is_not_absence(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            c = run.BOUNDS["jobs"]["Intel25"]
            primary = root / c["cache_primary_candidates"][0]
            primary.parent.mkdir(parents=True)
            primary.write_bytes(b"candidate")
            (root / c["ambiguity_checks"][0]).symlink_to(root / "missing")
            with self.assertRaises(cache.EvidenceError):
                cache.choose_family(root, c)

    def test_raw_cpu_capability_policy_no_implicit_masking(self):
        cache.validate_cpu("x86_64", 0x1000007, 3)
        for subtype in (8, 0x01000003, -2147483645):
            with self.assertRaises(cache.EvidenceError):
                cache.validate_cpu("x86_64", 0x1000007, subtype)
        cache.validate_cpu("arm64e", 0x100000C, -2147483646)
        with self.assertRaises(cache.EvidenceError):
            cache.validate_cpu("arm64e", 0x100000C, 0x01000002)


class NativeBindingTests(unittest.TestCase):
    def test_actual_loader_trace_required_and_uuid_matched(self):
        identity = "123456781234123412341234567890ab"
        path = "/usr/lib/libParallelCompression.dylib"
        line = "dyld[12]: <12345678-1234-1234-1234-1234567890ab> " + path
        self.assertEqual(native.decode_trace(line, path, identity, 12)["exact_child"][0]["pid"], 12)
        for text, expected in (("", identity), (path, identity), (line, "0" * 32)):
            with self.assertRaises(cache.EvidenceError):
                native.decode_trace(text, path, expected, 12)

    def test_unrelated_child_cannot_supply_decoder_identity(self):
        identity = "123456781234123412341234567890ab"
        path = "/usr/lib/libParallelCompression.dylib"
        unrelated = "dyld[99]: <12345678-1234-1234-1234-1234567890ab> " + path
        exact = unrelated.replace("[99]", "[12]")
        with self.assertRaises(cache.EvidenceError):
            native.decode_trace(unrelated, path, identity, 12)
        result = native.decode_trace(unrelated + "\n" + exact, path, identity, 12)
        self.assertEqual(result["exact_child"][0]["pid"], 12)
        self.assertEqual(result["unrelated_children"][0]["pid"], 99)

    def test_duplicated_or_truncated_child_trace_fails(self):
        identity = "123456781234123412341234567890ab"
        path = "/usr/lib/libParallelCompression.dylib"
        line = "dyld[12]: <12345678-1234-1234-1234-1234567890ab> " + path
        for text in (line + "\n" + line, "dyld[12]: <truncated> " + path):
            with self.assertRaises(cache.EvidenceError):
                native.decode_trace(text, path, identity, 12)

    def test_post_decoder_checks_gate_semantic_advance(self):
        binding = native.NativeBinding.__new__(native.NativeBinding)
        binding.run = SimpleNamespace(receipt={})
        order = []
        with patch.object(binding, "verify_use", side_effect=lambda _: order.append("exact-child")), \
                patch.object(binding, "recheck", side_effect=lambda: order.append("rehash")):
            binding.confirm_decoder_result({"fixture": True})
        self.assertEqual(order, ["exact-child", "rehash"])
        self.assertTrue(binding.run.receipt["decoder_confirmed_before_semantic_processing"])
        binding.run.receipt = {}
        with patch.object(binding, "verify_use"), \
                patch.object(binding, "recheck", side_effect=cache.EvidenceError("changed")):
            with self.assertRaises(cache.EvidenceError):
                binding.confirm_decoder_result({})
        self.assertNotIn("decoder_confirmed_before_semantic_processing", binding.run.receipt)

    def test_incomplete_owned_streams_are_not_consumer_confirmation(self):
        binding = native.NativeBinding.__new__(native.NativeBinding)
        binding.run = SimpleNamespace(work=Path("/private-work"))
        binding.before = {"loaded_parallel_compression": {"image": {"uuid": "0" * 32}}}
        binding.use_record = None
        result = {"rc": 0, "argv": ["/private-work/ipsw", "ota", "patch", "rsr", "--cryptex"],
                  "streams_complete": False}
        with self.assertRaises(cache.EvidenceError):
            binding.verify_use(result)

    def test_decoder_relation_rejects_alternate_architecture(self):
        with tempfile.TemporaryDirectory() as temp, patch.dict("os.environ", {}, clear=True):
            work = Path(temp)
            binary = work / "ipsw"
            binary.write_bytes(b"synthetic-tool")
            bounds = copy.deepcopy(run.BOUNDS)
            bounds["tools"]["ipsw"]["binary_bytes"] = len(b"synthetic-tool")
            bounds["tools"]["ipsw"]["binary_sha256"] = hashlib.sha256(b"synthetic-tool").hexdigest()
            config = bounds["jobs"]["ARM24"]
            controller = SimpleNamespace(work=work, bounds=bounds, config=config,
                deadline=time.monotonic() + 10,
                receipt={"ancestry": [{"label": config["patch"]["member"], "verified": True,
                                      "sha256": config["patch"]["sha256"]}]})
            binding = native.NativeBinding(controller)
            binding.before = {"bound": True}
            arguments = [str(binary), "ota", "patch", "rsr", "--cryptex",
                         str(work / Path(config["patch"]["member"]).name)]
            with patch.object(native, "file_machos", return_value=[{"cpu_raw": 0x1000007}]):
                with self.assertRaises(cache.EvidenceError):
                    binding.decoder_launch(arguments)

    def test_native_change_is_not_accepted(self):
        native.equal_binding({"hash": "same"}, {"hash": "same"})
        with self.assertRaises(cache.EvidenceError):
            native.equal_binding({"hash": "before"}, {"hash": "after"})

    def test_acceptance_requires_actual_decoder_use(self):
        binding = native.NativeBinding.__new__(native.NativeBinding)
        binding.before, binding.use_record = {"bound": True}, None
        with self.assertRaises(cache.EvidenceError):
            binding.recheck()

    def test_native_file_hash_budget(self):
        with tempfile.TemporaryDirectory() as temp:
            file = Path(temp) / "tool"
            file.write_bytes(b"12345678")
            with self.assertRaises(cache.EvidenceError):
                native.bind_hash(file, [4], time.monotonic() + 10)
            actual = native.bind_hash(file, [8], time.monotonic() + 10)
            self.assertEqual(actual["sha256"], hashlib.sha256(b"12345678").hexdigest())

    def test_loaded_header_must_match_actual_backing(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            primary = root / "dyld_shared_cache_arm64e"
            image_uuid = bytes.fromhex("123456781234123412341234567890ab")
            image = struct.pack("<IiiIIIII", 0xFEEDFACF, 0x100000C, 2, 6, 2, 96, 0, 0)
            image += struct.pack("<II16s4Q4I", 0x19, 72, b"__TEXT", 0x1000, 4096, 0x400, 128, 5, 5, 0, 0)
            image += struct.pack("<II", 0x1B, 24) + image_uuid
            data = bytearray(8192)
            data[:16] = b"dyld_v1  arm64e".ljust(16, b"\0")
            data[88:104] = image_uuid
            struct.pack_into("<II", data, 0x138, 0x300, 1)
            struct.pack_into("<3Q", data, 0x300, 0x1000, 4096, 0x400)
            data[0x400:0x400 + len(image)] = image
            primary.write_bytes(data)
            loaded = {"shared_cache_uuid": image_uuid.hex(), "unslid_header_address": "0x1000",
                      "image": native.macho_header(lambda offset, size: image[offset:offset+size])}
            policy = {"cache_primary_paths": [str(primary)], "cache_roots": [str(root)]}
            result = native.backing_binding(loaded, policy, [16384], time.monotonic() + 10)
            self.assertTrue(result["loaded_header_matches_backing_bytes"])
            self.assertEqual(len(result["file_hashes"]), 1)
            bad = copy.deepcopy(loaded)
            bad["shared_cache_uuid"] = "0" * 32
            with self.assertRaises(cache.EvidenceError):
                native.backing_binding(bad, policy, [16384], time.monotonic() + 10)
            bad = copy.deepcopy(loaded)
            altered = bytearray(bytes.fromhex(bad["image"]["header_hex"]))
            altered[24] ^= 1
            bad["image"]["header_hex"] = altered.hex()
            with self.assertRaises(cache.EvidenceError):
                native.backing_binding(bad, policy, [16384], time.monotonic() + 10)

    def test_resolution_overrides_rejected_without_loading(self):
        controller = SimpleNamespace(bounds=run.BOUNDS)
        with patch.dict("os.environ", {"DEVELOPER_DIR": "/different"}, clear=True), self.assertRaises(cache.EvidenceError):
            native.NativeBinding(controller)
        with patch.dict("os.environ", {"DYLD_LIBRARY_PATH": "/different"}, clear=True), self.assertRaises(cache.EvidenceError):
            native.NativeBinding(controller)

    def test_native_path_policy_is_frozen(self):
        policy = run.BOUNDS["native_binding"]
        self.assertEqual(policy["hdiutil"], "/usr/bin/hdiutil")
        self.assertEqual(policy["xcrun"], "/usr/bin/xcrun")
        self.assertEqual(policy["library"], "/usr/lib/libParallelCompression.dylib")
        self.assertIn("/Applications/Xcode_26.6.app/", policy["otool"])
        self.assertEqual(policy["maximum_native_snapshots"], 2)
        self.assertEqual(run.BOUNDS["limits"]["artifact_bytes_per_job"], 33554432)

    def test_native_search_path_invocation_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            controller = run.ComponentRun(Path(temp), Path(temp), "ARM24")
            for name in ("xcrun", "otool", "hdiutil"):
                with self.subTest(name=name), self.assertRaises(cache.EvidenceError):
                    controller.command("bad-path", [name, "--version"])

    def test_attach_and_detach_use_same_bound_component(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            controller = run.ComponentRun(root, root, "ARM24")
            controller.hdiutil_path = "/frozen/hdiutil"
            attachment = {"system-entities": [{"mount-point": str(root / "mount"), "dev-entry": "fixture-device"}]}
            result = {"stdout": plistlib.dumps(attachment).decode()}
            with patch.object(run.Run, "command", return_value=result) as command:
                _, entry = controller.attach(root / "fixture.dmg", "mount")
                controller.detach(entry)
            self.assertTrue(entry["detached"])
            self.assertEqual([call.args[1][0] for call in command.call_args_list],
                             ["/frozen/hdiutil", "/frozen/hdiutil"])


if __name__ == "__main__":
    unittest.main()
