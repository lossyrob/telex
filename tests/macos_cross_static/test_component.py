import copy
from contextlib import contextmanager
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
            root = Path(temp).resolve(strict=True)
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

    def test_family_selection_with_aliased_temporary_root(self):
        temporary_directory = tempfile.TemporaryDirectory
        with temporary_directory() as sandbox:
            base = Path(sandbox).resolve(strict=True)
            backing = base / "backing"
            backing.mkdir()
            alias = base / "temporary-alias"
            alias.symlink_to(backing, target_is_directory=True)
            with temporary_directory(dir=alias) as temp:
                raw_root = Path(temp)
                self.assertNotEqual(raw_root, raw_root.resolve(strict=True))
                with self.assertRaisesRegex(cache.EvidenceError, "image root is not canonical"):
                    cache.choose_family(raw_root, run.BOUNDS["jobs"]["Intel25"])
            # Exercise the same positive fixture through a deliberately aliased temp directory.
            with patch.object(tempfile, "TemporaryDirectory",
                              side_effect=lambda: temporary_directory(dir=alias)):
                self.test_family_selection_has_no_alternate()

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
    def test_ld64b_implicit_register_range_discards_old_slot(self):
        decoded = [row(0x1000, "adrp", "x9, 1 ; 0x2000"),
                   row(0x1004, "ldr", "x9, [x9, #0x20]"),
                   row(0x1008, "ld64b", "x8, [x0]"),
                   row(0x100C, "br", "x9")]
        raw = bytes.fromhex("090000b0291140f908d03ff820011fd6")
        slots = []
        def pointer(address):
            slots.append(address)
            return 0x9000, {"synthetic_slot": True}
        branches = cache.edges(decoded, "arm64", 0x1000, 0x1010, pointer, raw)
        self.assertEqual(slots, [])
        self.assertIsNone(branches[-1]["target"])
        self.assertIsNone(branches[-1]["identified_slot"])
        self.assertIn("unclassified", branches[-1]["unresolved"])

    def test_unsupported_instruction_with_unspelled_writes_loses_all_state(self):
        for op, operands in (("ld64b", "x8, [x0]"), ("future_instruction", ""),
                             ("add", "x0, x1, x2"), ("ldr", "x0, [sp, #0x20]"),
                             ("adrp", "x0, unsupported-form")):
            state = {"x9": ("slot", 0x2020), "x15": ("slot", 0x2028), "x28": 0x4000}
            with self.subTest(op=op, operands=operands):
                cache.arm_transfer(op, operands, state, 0x1000, 0x1000, None)
                self.assertEqual(state, {})

    def test_fully_supported_and_readonly_forms_preserve_unaffected_state(self):
        for op, operands in (("nop", ""), ("cmp", "w0, #0"), ("cmn", "x0, #1"),
                             ("tst", "x0, #1"), ("adrp", "x0, 1 ; 0x2000"),
                             ("add", "x0, x1, #0x20"), ("ldr", "x0, [x1]")):
            state = {"x9": ("slot", 0x2020), "x1": 0x2000}
            with self.subTest(op=op):
                cache.arm_transfer(op, operands, state, 0x1000, 0x1000, None)
                self.assertEqual(state["x9"], ("slot", 0x2020))

    def test_fresh_provenance_can_be_established_after_unknown_write(self):
        decoded = [row(0x1000, "adrp", "x9, 1 ; 0x2000"),
                   row(0x1004, "ldr", "x9, [x9, #0x20]"),
                   row(0x1008, "ld64b", "x8, [x0]"),
                   row(0x100C, "adrp", "x9, 1 ; 0x2000"),
                   row(0x1010, "ldr", "x9, [x9, #0x28]"),
                   row(0x1014, "br", "x9")]
        slots = []
        def pointer(address):
            slots.append(address)
            return 0x9000, {}
        branches = cache.edges(decoded, "arm64", 0x1000, 0x1018, pointer)
        self.assertEqual(slots, [0x2028])
        self.assertEqual(branches[-1]["identified_slot"], "0x2028")

    def test_undecoded_reachable_arm_gap_cannot_preserve_slot(self):
        decoded = [row(0x1000, "adrp", "x9, 1 ; 0x2000"),
                   row(0x1004, "ldr", "x9, [x9, #0x20]"),
                   row(0x100C, "br", "x9")]
        with self.assertRaises(cache.EvidenceError):
            cache.edges(decoded, "arm64", 0x1000, 0x1010, lambda _: self.fail())

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
        controller = SimpleNamespace(bounds={"roots": roots},
                                     otool_path=run.BOUNDS["native_binding"]["otool_canonical_path"],
                                     command=lambda name, args: (commands.append(args) or {"stdout": "fixture"}))
        with patch.object(cache, "CandidateCache", return_value=Fixture()), \
                patch.object(cache, "otool_instructions", return_value=decoded):
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
        self.assertEqual(commands[0][0], run.BOUNDS["native_binding"]["otool_canonical_path"])
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
    def test_reviewer_valid_plus_truncated_selected_path_fails(self):
        identity = "123456781234123412341234567890ab"
        path = "/usr/lib/libParallelCompression.dylib"
        prefix = "dyld[12]: <12345678-1234-1234-1234-1234567890ab> "
        text = prefix + path + "\n" + prefix + "/usr/lib/libParallelCompression"
        self.assertEqual(hashlib.sha256(text.encode("utf-8")).hexdigest(),
                         "eff0a3ccffc48c8a761fb0da296c5cd277bed0d37e1034b8a31ed7217e719607")
        with self.assertRaises(cache.EvidenceError):
            native.decode_trace(text, path, identity, 12)

    def test_partial_relevant_paths_fail_with_same_or_different_uuid(self):
        identity = "123456781234123412341234567890ab"
        path = "/usr/lib/libParallelCompression.dylib"
        valid = "dyld[12]: <12345678-1234-1234-1234-1234567890ab> " + path
        for observed_uuid in ("12345678-1234-1234-1234-1234567890ab",
                              "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"):
            for observed_path in ("/usr/lib/libParallelCompression", "/usr/lib/libP",
                                  "/usr/lib/libParallelCompression.dyl", "/usr/lib/",
                                  path + "/unexpected-suffix"):
                with self.subTest(uuid=observed_uuid, pathname=observed_path), \
                        self.assertRaises(cache.EvidenceError):
                    native.decode_trace(valid + "\ndyld[12]: <" + observed_uuid + "> " + observed_path,
                                        path, identity, 12)

    def test_selected_uuid_at_another_path_is_not_ignored(self):
        identity = "123456781234123412341234567890ab"
        path = "/usr/lib/libParallelCompression.dylib"
        prefix = "dyld[12]: <12345678-1234-1234-1234-1234567890ab> "
        for other in ("/alternate/libParallelCompression.dylib", "/elsewhere/unrelated.dylib"):
            with self.subTest(path=other), self.assertRaises(cache.EvidenceError):
                native.decode_trace(prefix + path + "\n" + prefix + other, path, identity, 12)

    def test_exact_child_incomplete_record_without_full_path_is_not_dropped(self):
        identity = "123456781234123412341234567890ab"
        path = "/usr/lib/libParallelCompression.dylib"
        valid = "dyld[12]: <12345678-1234-1234-1234-1234567890ab> " + path
        for partial in ("dyld[12]: <12345678-1234", "dyld[12]: <12345678-1234-1234-1234-1234567890ab>",
                        "dyld[12] <12345678-1234-1234-1234-1234567890ab> /usr/lib/libParallelCompression"):
            with self.subTest(partial=partial), self.assertRaises(cache.EvidenceError):
                native.decode_trace(valid + "\n" + partial, path, identity, 12)

    def test_unrelated_child_partial_path_cannot_supply_exact_match(self):
        identity = "123456781234123412341234567890ab"
        path = "/usr/lib/libParallelCompression.dylib"
        prefix = "dyld[99]: <12345678-1234-1234-1234-1234567890ab> "
        unrelated = prefix + "/usr/lib/libParallelCompression"
        exact = prefix.replace("[99]", "[12]") + path
        with self.assertRaises(cache.EvidenceError):
            native.decode_trace(unrelated, path, identity, 12)
        result = native.decode_trace(unrelated + "\n" + exact, path, identity, 12)
        self.assertEqual([r["pid"] for r in result["exact_child"]], [12])
        self.assertEqual(result["unrelated_children"][0]["pid"], 99)
        self.assertFalse(result["unrelated_children"][0]["supplies_exact_child_evidence"])

    def test_complete_other_exact_child_image_can_be_ignored_after_classification(self):
        identity = "123456781234123412341234567890ab"
        path = "/usr/lib/libParallelCompression.dylib"
        selected = "dyld[12]: <12345678-1234-1234-1234-1234567890ab> " + path
        other = "dyld[12]: <aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa> /usr/lib/libSystem.B.dylib"
        result = native.decode_trace(other + "\n" + selected, path, identity, 12)
        self.assertEqual(len(result["exact_child"]), 1)

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
        self.assertIn("/Applications/Xcode_26.6.app/", policy["otool_canonical_path"])
        self.assertNotIn("otool", policy)
        self.assertEqual(policy["maximum_native_snapshots"], 2)
        self.assertEqual(run.BOUNDS["limits"]["artifact_bytes_per_job"], 33554432)

    def test_native_search_path_invocation_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            controller = run.ComponentRun(Path(temp), Path(temp), "ARM24")
            for name in ("xcrun", "otool", "llvm-otool", "hdiutil"):
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


class SDKResolutionTests(unittest.TestCase):
    @contextmanager
    def sdk_case(self, stdout=None, versions=None, targets=None, errors=None, symlink_error=None):
        policy = run.BOUNDS["native_binding"]
        events = []
        targets = targets if targets is not None else {}
        errors = errors if errors is not None else {}
        versions = versions if versions is not None else {"prebinding": "26.5", "postdecoder": "26.5"}
        a, b = policy["sdk_canonical_path"], policy["sdk_versioned_path"]

        def command(name, arguments):
            events.append(("command", name, arguments))
            responses = {"resolve-otool": policy["otool_nominal_path"] + "\n",
                         "resolve-sdk": b + "\n" if stdout is None else stdout,
                         "resolve-sdk-version": versions["prebinding"] + "\n",
                         "recheck-sdk-version": versions["postdecoder"] + "\n",
                         "native-xcrun-version": "synthetic-version\n"}
            return {"rc": 0, "stdout": responses[name], "stderr": "", "argv": arguments}

        def resolve(path, strict=False):
            events.append(("resolve", str(path), strict))
            self.assertTrue(strict)
            if str(path) in (a, b):
                if str(path) in errors:
                    raise errors[str(path)]
                return Path(targets.get(str(path), a))
            self.assertIn(str(path), [policy[name] for name in (
                "hdiutil", "xcrun", "otool_canonical_path", "otool_nominal_path")])
            if str(path) in (policy["otool_canonical_path"], policy["otool_nominal_path"]):
                return Path(policy["otool_canonical_path"])
            return path

        original_lstat = Path.lstat
        def lstat(path):
            if str(path) not in (a, b):
                return original_lstat(path)
            events.append(("lstat", str(path)))
            if symlink_error is not None:
                raise symlink_error
            return SimpleNamespace(st_mode=native.stat.S_IFLNK if str(path) == b
                                   else native.stat.S_IFDIR)

        controller = SimpleNamespace(bounds=run.BOUNDS, command=command, job="ARM24", receipt={},
            output=SimpleNamespace(json=lambda name, value: events.append(("output", name, copy.deepcopy(value)))))
        with patch.dict("os.environ", {"GITHUB_SHA": "synthetic-sha", "GITHUB_RUN_ATTEMPT": "1"}, clear=True):
            binding = native.NativeBinding(controller)
            with patch.object(Path, "resolve", resolve), patch.object(Path, "lstat", lstat), \
                    patch.object(binding, "snapshot", return_value={"synthetic_first_metadata_hash": "fixture"}) as snapshot:
                yield binding, events, snapshot

    def test_policy_uses_only_the_two_declared_literals_with_new_roles(self):
        policy = run.BOUNDS["native_binding"]
        prefix = "/Applications/Xcode_26.6.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/"
        self.assertEqual(policy["sdk_canonical_path"], prefix + "MacOSX.sdk")
        self.assertEqual(policy["sdk_versioned_path"], prefix + "MacOSX26.5.sdk")
        self.assertNotIn("sdk_path", policy)
        self.assertEqual(policy["sdk_version"], "26.5")
        self.assertIn(prefix + "MacOSX.sdk/SDKSettings.plist", policy["developer_sdk_identity_files"])
        self.assertNotIn(prefix + "MacOSX26.5.sdk/SDKSettings.plist", policy["developer_sdk_identity_files"])

    def test_both_declared_stdout_spellings_require_both_relations_pre_and_post(self):
        policy = run.BOUNDS["native_binding"]
        a, b = policy["sdk_canonical_path"], policy["sdk_versioned_path"]
        for spelling in (a, b):
            with self.subTest(spelling=spelling), self.sdk_case(spelling + "\n") as (binding, events, snapshot):
                binding.resolve()
                self.assertEqual(binding.resolution["sdk"], spelling)
                self.assertEqual(binding.resolution["sdk_version"], "26.5")
                self.assertEqual(native.os.environ["SDKROOT"], a)
                self.assertEqual([e for e in events if e[0] == "resolve" and e[1] in (a, b)],
                                 [("resolve", a, True), ("resolve", b, True)])
                snapshot.assert_called_once_with()
                binding.use_record = {"synthetic_exact_child": True}
                binding.recheck()
                self.assertEqual(snapshot.call_count, 2)
                self.assertTrue(binding.decoder_confirmed)
                self.assertEqual([e for e in events if e[0] == "resolve" and e[1] in (a, b)],
                                 [("resolve", a, True), ("resolve", b, True)] * 2)
                self.assertEqual([e[1] for e in events if e[0] == "command" and "sdk-version" in e[1]],
                                 ["resolve-sdk-version", "recheck-sdk-version"])
                for checkpoint in ("prebinding", "postdecoder"):
                    receipt = [e[2] for e in events if e[:2] == ("output", f"sdk-identity-{checkpoint}.json")][-1]
                    self.assertEqual(receipt["observed_version"], "26.5")
                    self.assertEqual([r["expected_path"] for r in receipt["relations"]], [a, a])
                    self.assertEqual([r["resolved_path"] for r in receipt["relations"]], [a, a])
                    self.assertTrue(receipt["observations_are_sequential"])

    def test_either_relation_mismatch_is_recorded_before_its_guard_even_with_stdout_A(self):
        policy = run.BOUNDS["native_binding"]
        a, b = policy["sdk_canonical_path"], policy["sdk_versioned_path"]
        original_require = native.require
        for role, path in (("A", a), ("B", b)):
            with self.subTest(role=role), self.sdk_case(a + "\n", targets={path: "/synthetic/wrong.sdk"}) as (
                    binding, events, snapshot):
                def guarded(condition, reason):
                    if reason.startswith("SDK ") and "strict resolution differs" in reason:
                        events.append(("guard", reason, condition))
                    return original_require(condition, reason)
                with patch.object(native, "require", side_effect=guarded), \
                        self.assertRaisesRegex(cache.EvidenceError, f"SDK {role} strict resolution differs"):
                    binding.resolve()
                snapshot.assert_not_called()
                receipt_index = max(i for i, e in enumerate(events) if e[:2] == ("output", "sdk-identity-prebinding.json"))
                guard_index = next(i for i, e in enumerate(events) if e[0] == "guard")
                self.assertLess(receipt_index, guard_index)
                relations = events[receipt_index][2]["relations"]
                self.assertEqual(next(r for r in relations if r["role"] == role)["resolved_path"], "/synthetic/wrong.sdk")
                self.assertEqual([e for e in events if e[0] == "resolve"], [("resolve", a, True), ("resolve", b, True)])

    def test_common_third_target_is_not_dynamically_accepted(self):
        policy = run.BOUNDS["native_binding"]
        a, b = policy["sdk_canonical_path"], policy["sdk_versioned_path"]
        for stdout in (a, b):
            with self.subTest(stdout=stdout), self.sdk_case(stdout + "\n",
                    targets={a: "/synthetic/common.sdk", b: "/synthetic/common.sdk"}) as (binding, events, snapshot):
                with self.assertRaisesRegex(cache.EvidenceError, "SDK A strict resolution differs"):
                    binding.resolve()
                snapshot.assert_not_called()
                self.assertFalse(any(e[0] == "resolve" and e[1] == "/synthetic/common.sdk" for e in events))

    def test_old_alias_direction_and_self_resolving_B_are_rejected(self):
        policy = run.BOUNDS["native_binding"]
        a, b = policy["sdk_canonical_path"], policy["sdk_versioned_path"]
        for targets in ({a: b, b: b}, {a: a, b: b}, {a: b, b: a}):
            with self.subTest(targets=targets), self.sdk_case(b + "\n", targets=targets) as (binding, _, snapshot):
                with self.assertRaisesRegex(cache.EvidenceError, "strict resolution differs from canonical A"):
                    binding.resolve()
                snapshot.assert_not_called()

    def test_missing_inaccessible_or_looping_either_literal_fails_with_explicit_receipt(self):
        policy = run.BOUNDS["native_binding"]
        a, b = policy["sdk_canonical_path"], policy["sdk_versioned_path"]
        for role, path in (("A", a), ("B", b)):
            for error in (FileNotFoundError("synthetic missing"), PermissionError("synthetic denied"),
                          RuntimeError("synthetic alias loop")):
                with self.subTest(role=role, error=type(error).__name__), self.sdk_case(
                        a + "\n", errors={path: error}) as (binding, events, snapshot):
                    with self.assertRaises(type(error)):
                        binding.resolve()
                    snapshot.assert_not_called()
                    receipt = [e[2] for e in events if e[:2] == ("output", "sdk-identity-prebinding.json")][-1]
                    relation = next(r for r in receipt["relations"] if r["role"] == role)
                    self.assertEqual(relation["error_type"], type(error).__name__)
                    self.assertNotIn("resolved_path", relation)
                    self.assertEqual([e for e in events if e[0] == "resolve"], [("resolve", a, True), ("resolve", b, True)])

    def test_version_query_not_B_filename_controls_pre_and_post(self):
        policy = run.BOUNDS["native_binding"]
        b = policy["sdk_versioned_path"]
        for checkpoint in ("prebinding", "postdecoder"):
            for bad in ("", "26.4", "26.5\n26.4"):
                versions = {"prebinding": "26.5", "postdecoder": "26.5"}
                with self.subTest(checkpoint=checkpoint, version=bad), self.sdk_case(
                        b + "\n", versions=versions) as (binding, events, snapshot):
                    if checkpoint == "postdecoder":
                        binding.resolve()
                        binding.use_record = {"synthetic_child": True}
                        snapshot.reset_mock()
                        events.clear()
                    versions[checkpoint] = bad
                    with self.assertRaisesRegex(cache.EvidenceError, "SDK version mismatch"):
                        binding.resolve() if checkpoint == "prebinding" else binding.recheck()
                    snapshot.assert_not_called()
                    self.assertFalse(binding.decoder_confirmed)
                    self.assertFalse(any(e[0] == "resolve" for e in events))
                    version_call = next(e for e in events if e[0] == "command" and "sdk-version" in e[1])
                    self.assertEqual(version_call[2], [policy["xcrun"], "--sdk", "macosx", "--show-sdk-version"])

    def test_version_command_failure_is_not_a_filename_fallback(self):
        policy = run.BOUNDS["native_binding"]
        with self.sdk_case(policy["sdk_versioned_path"] + "\n") as (binding, events, snapshot):
            command = binding.run.command
            def fail_version(name, args):
                if name == "resolve-sdk-version":
                    raise cache.EvidenceError("synthetic command failure")
                return command(name, args)
            with patch.object(binding.run, "command", side_effect=fail_version), \
                    self.assertRaisesRegex(cache.EvidenceError, "synthetic command failure"):
                binding.resolve()
            snapshot.assert_not_called()
            self.assertFalse(any(e[0] == "resolve" for e in events))

    def test_undeclared_alias_is_rejected_even_if_it_would_resolve_to_same_sdk(self):
        policy = run.BOUNDS["native_binding"]
        with self.sdk_case("/some/other/alias.sdk\n") as (binding, events, snapshot), \
                patch.object(Path, "resolve", return_value=Path(policy["sdk_canonical_path"])) as resolver:
            with self.assertRaisesRegex(cache.EvidenceError, "xcrun selected an unfrozen SDK"):
                binding.resolve()
            resolver.assert_not_called()
            snapshot.assert_not_called()
            self.assertFalse(any(e[0] == "output" for e in events))

    def test_wrong_missing_or_nonliteral_sdk_output_fails_before_snapshot(self):
        policy = run.BOUNDS["native_binding"]
        a, b = policy["sdk_canonical_path"], policy["sdk_versioned_path"]
        for stdout in ("", "\n", b.replace("26.5.sdk", "26.4.sdk") + "\n", a + "/\n", a + "\n" + b):
            with self.subTest(stdout=stdout), self.sdk_case(stdout) as (binding, _, snapshot):
                with self.assertRaisesRegex(cache.EvidenceError, "xcrun selected an unfrozen SDK"):
                    binding.resolve()
                snapshot.assert_not_called()

    def test_either_relation_retargeted_postdecoder_blocks_snapshot_and_semantics(self):
        policy = run.BOUNDS["native_binding"]
        a, b = policy["sdk_canonical_path"], policy["sdk_versioned_path"]
        for role, path in (("A", a), ("B", b)):
            targets = {}
            with self.subTest(role=role), self.sdk_case(a + "\n", targets=targets) as (binding, events, snapshot):
                binding.resolve()
                binding.use_record = {"synthetic_exact_child": True}
                snapshot.reset_mock()
                events.clear()
                targets[path] = "/synthetic/retargeted.sdk"
                with self.assertRaisesRegex(cache.EvidenceError, f"SDK {role} strict resolution differs"):
                    binding.recheck()
                snapshot.assert_not_called()
                self.assertFalse(binding.decoder_confirmed)
                self.assertEqual([e for e in events if e[0] == "resolve"], [("resolve", a, True), ("resolve", b, True)])

    def test_either_relation_missing_or_inaccessible_postdecoder_blocks(self):
        policy = run.BOUNDS["native_binding"]
        for role, path in (("A", policy["sdk_canonical_path"]), ("B", policy["sdk_versioned_path"])):
            for error in (FileNotFoundError("missing post"), PermissionError("denied post")):
                errors = {}
                with self.subTest(role=role, error=type(error).__name__), self.sdk_case(
                        errors=errors) as (binding, events, snapshot):
                    binding.resolve()
                    binding.use_record = {"synthetic_exact_child": True}
                    snapshot.reset_mock()
                    errors[path] = error
                    with self.assertRaises(type(error)):
                        binding.recheck()
                    snapshot.assert_not_called()
                    self.assertFalse(binding.decoder_confirmed)

    def test_common_third_target_postdecoder_is_rejected(self):
        policy = run.BOUNDS["native_binding"]
        targets = {}
        with self.sdk_case(targets=targets) as (binding, events, snapshot):
            binding.resolve()
            binding.use_record = {"synthetic_exact_child": True}
            targets.update({policy["sdk_canonical_path"]: "/synthetic/common.sdk",
                            policy["sdk_versioned_path"]: "/synthetic/common.sdk"})
            snapshot.reset_mock()
            with self.assertRaisesRegex(cache.EvidenceError, "SDK A strict resolution differs"):
                binding.recheck()
            snapshot.assert_not_called()
            self.assertFalse(binding.decoder_confirmed)

    def test_optional_status_errors_do_not_supply_or_remove_relation_acceptance(self):
        policy = run.BOUNDS["native_binding"]
        for target in (policy["sdk_canonical_path"], "/synthetic/wrong.sdk"):
            with self.subTest(target=target), self.sdk_case(
                    targets={policy["sdk_versioned_path"]: target},
                    symlink_error=PermissionError("synthetic status denied")) as (binding, events, snapshot):
                if target == policy["sdk_canonical_path"]:
                    binding.resolve()
                    snapshot.assert_called_once_with()
                else:
                    with self.assertRaises(cache.EvidenceError):
                        binding.resolve()
                    snapshot.assert_not_called()
                receipt = [e[2] for e in events if e[:2] == ("output", "sdk-identity-prebinding.json")][-1]
                self.assertTrue(all(r["error_type"] == "PermissionError"
                                    for r in receipt["diagnostics_only"].values()))

    def test_relation_receipt_fits_existing_sink_and_error_still_blocks(self):
        policy = run.BOUNDS["native_binding"]
        with tempfile.TemporaryDirectory() as temp, self.sdk_case(
                targets={policy["sdk_versioned_path"]: "/synthetic/mismatch.sdk"}) as (binding, _, snapshot):
            evidence = Path(temp)
            binding.run.output = cache.Output(evidence, run.BOUNDS["limits"])
            with self.assertRaises(cache.EvidenceError):
                binding.resolve()
            snapshot.assert_not_called()
            receipt = evidence / "sdk-identity-prebinding.json"
            self.assertLess(receipt.stat().st_size, 4096)
            self.assertEqual(json.loads(receipt.read_text())["relations"][1]["resolved_path"], "/synthetic/mismatch.sdk")

    def test_receipt_sink_failure_cannot_become_sdk_acceptance(self):
        with self.sdk_case() as (binding, events, snapshot):
            with patch.object(binding.run.output, "json", side_effect=cache.EvidenceError("bounded output failure")), \
                    self.assertRaisesRegex(cache.EvidenceError, "bounded output failure"):
                binding.resolve()
            snapshot.assert_not_called()
            self.assertFalse(any(e[0] == "resolve" for e in events))

    def test_relation_receipt_failure_after_evaluation_stops_before_binding(self):
        policy = run.BOUNDS["native_binding"]
        with self.sdk_case() as (binding, events, snapshot):
            original = binding.run.output.json
            def reject_relations(name, value):
                if name == "sdk-identity-prebinding.json" and value["relations"]:
                    raise cache.EvidenceError("relation receipt limit")
                return original(name, value)
            with patch.object(binding.run.output, "json", side_effect=reject_relations), \
                    self.assertRaisesRegex(cache.EvidenceError, "relation receipt limit"):
                binding.resolve()
            snapshot.assert_not_called()
            self.assertEqual([e for e in events if e[0] == "resolve"], [
                ("resolve", policy["sdk_canonical_path"], True), ("resolve", policy["sdk_versioned_path"], True)])

    def test_sdkroot_accepts_A_only_not_B(self):
        policy = run.BOUNDS["native_binding"]
        a, b = policy["sdk_canonical_path"], policy["sdk_versioned_path"]
        with patch.dict("os.environ", {"SDKROOT": a}, clear=True):
            binding = native.NativeBinding(SimpleNamespace(bounds=run.BOUNDS))
            self.assertEqual(native.os.environ["SDKROOT"], a)
            native.os.environ["SDKROOT"] = b
            with self.assertRaisesRegex(cache.EvidenceError, "developer/SDK environment override"):
                binding.check_environment()
        with patch.dict("os.environ", {"SDKROOT": b}, clear=True):
            with self.assertRaisesRegex(cache.EvidenceError, "developer/SDK environment override"):
                native.NativeBinding(SimpleNamespace(bounds=run.BOUNDS))

    def test_metadata_snapshot_uses_A_and_binds_first_actual_hash_without_prior_value(self):
        policy = run.BOUNDS["native_binding"]
        a_metadata = str(Path(policy["sdk_canonical_path"]) / "SDKSettings.plist")
        self.assertEqual(policy["developer_sdk_identity_files"][1], a_metadata)
        with self.sdk_case() as (binding, _, snapshot), patch.dict("os.environ",
                {"GITHUB_SHA": "synthetic-sha", "GITHUB_RUN_ATTEMPT": "1"}):
            binding.run.deadline = time.monotonic() + 10
            paths = []
            def hash_file(path, budget, deadline):
                paths.append(str(path))
                return {"path": str(path), "sha256": "first-actual-fixture-hash"}
            with patch.object(native, "bind_hash", side_effect=hash_file), \
                    patch.object(native, "file_machos", return_value=[{"uuid": "fixture"}]), \
                    patch.object(native, "loaded_parallel", return_value=(object(), {"uuid": "fixture"})), \
                    patch.object(native, "backing_binding", return_value={"fixture": True}):
                receipt = native.NativeBinding.snapshot(binding)
            self.assertIn(a_metadata, paths)
            self.assertNotIn(str(Path(policy["sdk_versioned_path"]) / "SDKSettings.plist"), paths)
            self.assertEqual(receipt["developer_sdk_files"][a_metadata]["sha256"], "first-actual-fixture-hash")
            self.assertFalse(any(key.endswith("sha256") for key in policy["sdk_identity_policy"]))

    def test_changed_postdecoder_metadata_hash_blocks_even_when_both_relations_match(self):
        with self.sdk_case() as (binding, _, snapshot):
            binding.resolve()
            binding.use_record = {"synthetic_exact_child": True}
            snapshot.reset_mock()
            snapshot.return_value = {"synthetic_first_metadata_hash": "changed"}
            with self.assertRaisesRegex(cache.EvidenceError, "bound native component"):
                binding.recheck()
            snapshot.assert_called_once_with()
            self.assertFalse(binding.decoder_confirmed)

    def test_postdecoder_checks_precede_snapshot_and_confirmation(self):
        with self.sdk_case() as (binding, events, snapshot):
            binding.resolve()
            binding.use_record = {"synthetic_exact_child": True}
            events.clear()
            def observed_snapshot():
                events.append(("snapshot", "postdecoder"))
                return binding.before
            snapshot.side_effect = observed_snapshot
            binding.recheck()
            version = next(i for i, e in enumerate(events) if e[:2] == ("command", "recheck-sdk-version"))
            relation_receipt = max(i for i, e in enumerate(events) if e[:2] == ("output", "sdk-identity-postdecoder.json"))
            hashed = events.index(("snapshot", "postdecoder"))
            confirmation = next(i for i, e in enumerate(events) if e[:2] == ("output", "native-binding-acceptance.json"))
            self.assertLess(version, relation_receipt)
            self.assertLess(relation_receipt, hashed)
            self.assertLess(hashed, confirmation)

    def test_final_metadata_hash_check_is_not_an_alias_or_version_observation(self):
        policy = run.BOUNDS["native_binding"]
        metadata = str(Path(policy["sdk_canonical_path"]) / "SDKSettings.plist")
        for observed_hash in ("first", "changed"):
            with self.subTest(hash=observed_hash), self.sdk_case() as (binding, events, _):
                binding.before = {"native_tools": {}, "developer_sdk_files": {
                    metadata: {"path": metadata, "sha256": "first"}}}
                binding.decoder_confirmed = True
                binding.run.deadline = time.monotonic() + 10
                with patch.object(native, "bind_hash", return_value={"path": metadata, "sha256": observed_hash}) as hasher, \
                        patch.object(binding, "check_sdk_identity") as identities, \
                        patch.object(binding.run, "command") as command, patch.object(Path, "resolve") as resolver:
                    if observed_hash == "first":
                        binding.recheck_tools_after_semantics()
                    else:
                        with self.assertRaisesRegex(cache.EvidenceError, "bound native component"):
                            binding.recheck_tools_after_semantics()
                    self.assertEqual(hasher.call_args.args[0], Path(metadata))
                    identities.assert_not_called()
                    command.assert_not_called()
                    resolver.assert_not_called()
        self.assertIn("NOT a new alias or SDK-version observation", policy["final_sdk_precision"])


class NativeToolResolutionTests(unittest.TestCase):
    sdk_case = SDKResolutionTests.sdk_case
    names = ("hdiutil", "xcrun")
    guard_message = "native executable resolves outside its allowed absolute path"

    @contextmanager
    def tool_case(self, targets=None, errors=None):
        targets, errors = targets or {}, errors or {}
        policy = run.BOUNDS["native_binding"]
        with self.sdk_case() as (binding, events, snapshot):
            original_resolve, original_require = Path.resolve, native.require
            def resolve(path, strict=False):
                for name in self.names:
                    if str(path) == policy[name]:
                        events.append(("resolve", str(path), strict))
                        self.assertTrue(strict)
                        if name in errors:
                            raise errors[name]
                        return Path(targets.get(name, policy[name]))
                return original_resolve(path, strict=strict)
            def guarded(condition, reason):
                if reason == self.guard_message:
                    events.append(("tool-guard", condition, reason))
                return original_require(condition, reason)
            with patch.object(Path, "resolve", resolve), patch.object(native, "require", side_effect=guarded):
                yield binding, events, snapshot

    def tool_events(self, events):
        paths = [run.BOUNDS["native_binding"][name] for name in self.names]
        return [e for e in events if (e[0] == "resolve" and e[1] in paths) or
                (e[0] == "output" and e[1].startswith("resolve-native-")) or e[0] == "tool-guard"]

    def assert_stopped_before_binding(self, binding, events, snapshot):
        snapshot.assert_not_called()
        self.assertIsNone(binding.before)
        self.assertIsNone(binding.resolution)
        self.assertFalse(binding.decoder_confirmed)
        self.assertFalse(any(e[:2] == ("command", "native-xcrun-version") for e in events))

    def test_success_resolves_each_tool_once_in_order_before_same_guard_and_binding(self):
        policy = run.BOUNDS["native_binding"]
        with self.tool_case() as (binding, events, snapshot):
            result = binding.resolve()
            self.assertIs(result, snapshot.return_value)
            snapshot.assert_called_once_with()
            expected = []
            for name in self.names:
                expected.extend([
                    ("resolve", policy[name], True),
                    ("output", f"resolve-native-{name}.json", {
                        "tool": name, "declared_path": policy[name], "expected_path": policy[name],
                        "strict": True, "resolved_path": policy[name]}),
                    ("tool-guard", True, self.guard_message)])
            self.assertEqual(self.tool_events(events), expected)
            last_guard = max(i for i, e in enumerate(events) if e[0] == "tool-guard")
            version = next(i for i, e in enumerate(events) if e[:2] == ("command", "native-xcrun-version"))
            self.assertLess(last_guard, version)
            self.assertEqual(binding.run.otool_path, policy["otool_canonical_path"])
            self.assertEqual(binding.run.hdiutil_path, policy["hdiutil"])
            self.assertEqual(binding.resolution["xcrun_version"], "synthetic-version")
            self.assertEqual([e[1] for e in events if e[0] == "lstat"],
                             [policy["sdk_canonical_path"], policy["sdk_versioned_path"]])

    def test_each_tool_mismatch_records_exact_operand_then_same_failure_and_stops(self):
        policy = run.BOUNDS["native_binding"]
        for index, name in enumerate(self.names):
            target = "/synthetic/unaccepted-" + name
            with self.subTest(tool=name), self.tool_case(targets={name: target}) as (binding, events, snapshot):
                with self.assertRaises(cache.EvidenceError) as raised:
                    binding.resolve()
                self.assertEqual(str(raised.exception), self.guard_message)
                self.assert_stopped_before_binding(binding, events, snapshot)
                observed = self.tool_events(events)
                self.assertEqual([e for e in observed if e[0] == "resolve"],
                                 [("resolve", policy[n], True) for n in self.names[:index + 1]])
                self.assertEqual(observed[-2:], [
                    ("output", f"resolve-native-{name}.json", {
                        "tool": name, "declared_path": policy[name], "expected_path": policy[name],
                        "strict": True, "resolved_path": target}),
                    ("tool-guard", False, self.guard_message)])
                self.assertFalse(any(e[:2] == ("resolve", target) for e in events))

    def test_each_tool_resolution_error_is_recorded_without_value_and_reraised(self):
        policy = run.BOUNDS["native_binding"]
        for index, name in enumerate(self.names):
            for error in (FileNotFoundError("synthetic missing"), PermissionError("synthetic denied"),
                          OSError("synthetic I/O failure"), RuntimeError("synthetic alias loop")):
                with self.subTest(tool=name, error=type(error).__name__), self.tool_case(
                        errors={name: error}) as (binding, events, snapshot):
                    with self.assertRaises(type(error)) as raised:
                        binding.resolve()
                    self.assertIs(raised.exception, error)
                    self.assert_stopped_before_binding(binding, events, snapshot)
                    observed = self.tool_events(events)
                    self.assertEqual([e for e in observed if e[0] == "resolve"],
                                     [("resolve", policy[n], True) for n in self.names[:index + 1]])
                    self.assertEqual(observed[-1], ("output", f"resolve-native-{name}.json", {
                        "tool": name, "declared_path": policy[name], "expected_path": policy[name],
                        "strict": True, "error_type": type(error).__name__, "error": str(error)}))
                    self.assertEqual(len([e for e in observed if e[0] == "tool-guard"]), index)

    def test_sink_failure_on_each_tool_stops_before_guard_or_later_queries(self):
        policy = run.BOUNDS["native_binding"]
        for index, name in enumerate(self.names):
            for outcome in ("success", "mismatch", "resolve-error"):
                targets = {name: "/synthetic/mismatch"} if outcome == "mismatch" else {}
                errors = {name: PermissionError("synthetic denied")} if outcome == "resolve-error" else {}
                with self.subTest(tool=name, outcome=outcome), self.tool_case(
                        targets=targets, errors=errors) as (binding, events, snapshot):
                    output = binding.run.output.json
                    failure = cache.EvidenceError("synthetic sink failure")
                    def reject(filename, receipt):
                        if filename == f"resolve-native-{name}.json":
                            events.append(("sink-failure", filename))
                            raise failure
                        return output(filename, receipt)
                    with patch.object(binding.run.output, "json", side_effect=reject), \
                            self.assertRaises(cache.EvidenceError) as raised:
                        binding.resolve()
                    self.assertIs(raised.exception, failure)
                    self.assert_stopped_before_binding(binding, events, snapshot)
                    observed = self.tool_events(events)
                    self.assertEqual([e for e in observed if e[0] == "resolve"],
                                     [("resolve", policy[n], True) for n in self.names[:index + 1]])
                    self.assertEqual(len([e for e in observed if e[0] == "tool-guard"]), index)
                    self.assertEqual(events[-1], ("sink-failure", f"resolve-native-{name}.json"))

    def test_receipt_mutation_or_sink_return_value_cannot_supply_guard_operand(self):
        policy = run.BOUNDS["native_binding"]
        for accepted in (True, False):
            targets = {} if accepted else {"hdiutil": "/synthetic/mismatch"}
            with self.subTest(accepted=accepted), self.tool_case(targets=targets) as (binding, events, snapshot):
                output = binding.run.output.json
                def mutate(filename, receipt):
                    if filename.startswith("resolve-native-"):
                        receipt["resolved_path"] = "/synthetic/mismatch" if accepted else policy["hdiutil"]
                    output(filename, receipt)
                    return policy["hdiutil"]
                with patch.object(binding.run.output, "json", side_effect=mutate):
                    if accepted:
                        binding.resolve()
                        snapshot.assert_called_once_with()
                    else:
                        with self.assertRaisesRegex(cache.EvidenceError, self.guard_message):
                            binding.resolve()
                        self.assert_stopped_before_binding(binding, events, snapshot)

    def test_success_receipts_fit_unchanged_flat_output_limits(self):
        with tempfile.TemporaryDirectory() as temp, self.tool_case() as (binding, _, snapshot):
            output = Path(temp)
            binding.run.output = cache.Output(output, run.BOUNDS["limits"])
            binding.resolve()
            snapshot.assert_called_once_with()
            files = sorted(output.glob("resolve-native-*.json"))
            self.assertEqual([f.name for f in files], sorted(f"resolve-native-{name}.json" for name in self.names))
            for file in files:
                self.assertLess(file.stat().st_size, 1024)
                receipt = json.loads(file.read_text())
                self.assertEqual(receipt["resolved_path"], receipt["expected_path"])

    def test_oversized_error_receipt_fails_existing_cap_without_truncation_or_advance(self):
        error = OSError("x" * run.BOUNDS["limits"]["per_file_bytes"])
        with tempfile.TemporaryDirectory() as temp, self.tool_case(errors={"hdiutil": error}) as (
                binding, events, snapshot):
            output = Path(temp)
            binding.run.output = cache.Output(output, run.BOUNDS["limits"])
            with self.assertRaisesRegex(cache.EvidenceError, "per-file evidence cap"):
                binding.resolve()
            self.assert_stopped_before_binding(binding, events, snapshot)
            self.assertFalse(list(output.glob("resolve-native-*.json")))
            self.assertFalse(any(e[0] == "tool-guard" for e in events))


class OtoolIdentityTests(unittest.TestCase):
    sdk_case = SDKResolutionTests.sdk_case

    @contextmanager
    def otool_case(self, stdout=None, targets=None, errors=None):
        policy = run.BOUNDS["native_binding"]
        c, d = policy["otool_canonical_path"], policy["otool_nominal_path"]
        targets = targets if targets is not None else {}
        errors = errors if errors is not None else {}
        with self.sdk_case() as (binding, events, snapshot):
            original_resolve, command = Path.resolve, binding.run.command
            def resolve(path, strict=False):
                if str(path) not in (c, d):
                    return original_resolve(path, strict=strict)
                events.append(("resolve", str(path), strict))
                self.assertTrue(strict)
                if str(path) in errors:
                    raise errors[str(path)]
                return Path(targets.get(str(path), c))
            def selected(name, args):
                result = command(name, args)
                if name == "resolve-otool" and stdout is not None:
                    result["stdout"] = stdout
                return result
            with patch.object(Path, "resolve", resolve), patch.object(binding.run, "command", side_effect=selected):
                yield binding, events, snapshot

    def test_policy_has_distinct_fixed_C_and_D_without_old_ambiguous_key(self):
        policy = run.BOUNDS["native_binding"]
        prefix = "/Applications/Xcode_26.6.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/"
        self.assertEqual(policy["otool_canonical_path"], prefix + "llvm-otool")
        self.assertEqual(policy["otool_nominal_path"], prefix + "otool")
        self.assertNotIn("otool", policy)
        self.assertIn("6406629529", policy["otool_identity_policy"]["corrected_condition_1"])
        self.assertIn("6406665201", policy["otool_identity_policy"]["record_precision"])
        self.assertEqual(policy["library"], "/usr/lib/libParallelCompression.dylib")

    def test_D_stdout_requires_both_relations_once_per_checkpoint_before_snapshot(self):
        policy = run.BOUNDS["native_binding"]
        c, d = policy["otool_canonical_path"], policy["otool_nominal_path"]
        with self.otool_case(d + "\n") as (binding, events, snapshot):
            binding.resolve()
            self.assertEqual(binding.resolution["otool_nominal_selection"], d)
            self.assertEqual(binding.resolution["otool_canonical_path"], c)
            self.assertEqual(binding.run.otool_path, c)
            snapshot.assert_called_once_with()
            self.assertEqual([e for e in events if e[0] == "resolve" and e[1] in (c, d)],
                             [("resolve", c, True), ("resolve", d, True)])
            binding.use_record = {"synthetic_child": True}
            events.clear()
            snapshot.side_effect = lambda: (events.append(("snapshot",)) or binding.before)
            binding.recheck()
            self.assertTrue(binding.decoder_confirmed)
            self.assertEqual([e for e in events if e[0] == "resolve" and e[1] in (c, d)],
                             [("resolve", c, True), ("resolve", d, True)])
            sdk = max(i for i, e in enumerate(events) if e[:2] == ("output", "sdk-identity-postdecoder.json"))
            otool = next(i for i, e in enumerate(events) if e[:2] == ("output", "otool-identity-postdecoder.json"))
            self.assertLess(sdk, otool)
            self.assertLess(otool, events.index(("snapshot",)))
            receipt = events[otool][2]
            self.assertEqual([r["resolved_path"] for r in receipt["relations"]], [c, c])
            self.assertEqual([r["input_path"] for r in receipt["relations"]], [c, d])

    def test_C_stdout_and_every_nonD_spelling_fail_before_any_relation(self):
        policy = run.BOUNDS["native_binding"]
        c, d = policy["otool_canonical_path"], policy["otool_nominal_path"]
        for stdout in (c, "", "/synthetic/otool", d + "/", d + "\n" + c):
            with self.subTest(stdout=stdout), self.otool_case(stdout) as (binding, events, snapshot):
                with self.assertRaisesRegex(cache.EvidenceError, "xcrun selected an unfrozen otool path"):
                    binding.resolve()
                snapshot.assert_not_called()
                self.assertFalse(any(e[0] == "resolve" for e in events))

    def test_either_relation_and_common_third_targets_fail_pre_and_post(self):
        policy = run.BOUNDS["native_binding"]
        c, d = policy["otool_canonical_path"], policy["otool_nominal_path"]
        for checkpoint in ("prebinding", "postdecoder"):
            for changes in ({c: d}, {d: d}, {c: "/synthetic/third", d: "/synthetic/third"},
                            {d: "/synthetic/third"}):
                targets = {}
                with self.subTest(checkpoint=checkpoint, changes=changes), self.otool_case(
                        targets=targets) as (binding, events, snapshot):
                    if checkpoint == "postdecoder":
                        binding.resolve()
                        binding.use_record = {"synthetic_child": True}
                        events.clear()
                        snapshot.reset_mock()
                    targets.update(changes)
                    with self.assertRaisesRegex(cache.EvidenceError, "strict resolution differs from canonical C"):
                        binding.resolve() if checkpoint == "prebinding" else binding.recheck()
                    snapshot.assert_not_called()
                    self.assertFalse(binding.decoder_confirmed)
                    self.assertEqual([e for e in events if e[0] == "resolve" and e[1] in (c, d)],
                                     [("resolve", c, True), ("resolve", d, True)])
                    self.assertFalse(any(e[:2] == ("resolve", "/synthetic/third") for e in events))

    def test_missing_inaccessible_or_looping_C_or_D_records_error_before_rethrow_pre_and_post(self):
        policy = run.BOUNDS["native_binding"]
        c, d = policy["otool_canonical_path"], policy["otool_nominal_path"]
        for checkpoint in ("prebinding", "postdecoder"):
            for role, pathname in (("C", c), ("D", d)):
                for error in (FileNotFoundError("missing"), PermissionError("denied"),
                              OSError("I/O"), RuntimeError("loop")):
                    errors = {}
                    with self.subTest(checkpoint=checkpoint, role=role, error=type(error).__name__), \
                            self.otool_case(errors=errors) as (binding, events, snapshot):
                        if checkpoint == "postdecoder":
                            binding.resolve()
                            binding.use_record = {"synthetic_child": True}
                            snapshot.reset_mock()
                            events.clear()
                        errors[pathname] = error
                        with self.assertRaises(type(error)) as raised:
                            binding.resolve() if checkpoint == "prebinding" else binding.recheck()
                        self.assertIs(raised.exception, error)
                        snapshot.assert_not_called()
                        self.assertFalse(binding.decoder_confirmed)
                        receipt = next(e[2] for e in events if e[:2] == (
                            "output", f"otool-identity-{checkpoint}.json"))
                        relation = next(r for r in receipt["relations"] if r["role"] == role)
                        self.assertEqual(relation["error_type"], type(error).__name__)
                        self.assertNotIn("resolved_path", relation)
                        self.assertEqual([e for e in events if e[0] == "resolve" and e[1] in (c, d)],
                                         [("resolve", c, True), ("resolve", d, True)])

    def test_completed_receipt_precedes_guard_and_sink_failure_stops_both_checkpoints(self):
        c = run.BOUNDS["native_binding"]["otool_canonical_path"]
        for checkpoint in ("prebinding", "postdecoder"):
            with self.subTest(checkpoint=checkpoint), self.otool_case() as (binding, events, snapshot):
                if checkpoint == "postdecoder":
                    binding.resolve()
                    binding.use_record = {"synthetic_child": True}
                    snapshot.reset_mock()
                    events.clear()
                original = binding.run.output.json
                failure = cache.EvidenceError("synthetic otool sink cap")
                def fail(name, value):
                    if name == f"otool-identity-{checkpoint}.json":
                        self.assertEqual([r["resolved_path"] for r in value["relations"]], [c, c])
                        raise failure
                    return original(name, value)
                with patch.object(binding.run.output, "json", side_effect=fail), \
                        self.assertRaises(cache.EvidenceError) as raised:
                    binding.resolve() if checkpoint == "prebinding" else binding.recheck()
                self.assertIs(raised.exception, failure)
                snapshot.assert_not_called()
                self.assertFalse(binding.decoder_confirmed)

    def test_both_relations_recorded_before_rejection_without_sink_operand_feedback(self):
        policy = run.BOUNDS["native_binding"]
        c, d = policy["otool_canonical_path"], policy["otool_nominal_path"]
        with self.otool_case(targets={d: "/synthetic/third"}) as (binding, events, snapshot):
            output, original_require = binding.run.output.json, native.require
            def mutate(name, value):
                output(name, value)
                if name.startswith("otool-identity"):
                    value["relations"][1]["resolved_path"] = c
            def guarded(condition, reason):
                if reason.startswith("otool ") and "strict resolution" in reason:
                    events.append(("guard", condition))
                return original_require(condition, reason)
            with patch.object(binding.run.output, "json", side_effect=mutate), \
                    patch.object(native, "require", side_effect=guarded), \
                    self.assertRaisesRegex(cache.EvidenceError, "otool D strict resolution"):
                binding.resolve()
            snapshot.assert_not_called()
            receipt = next(i for i, e in enumerate(events) if e[:2] == ("output", "otool-identity-prebinding.json"))
            guard = next(i for i, e in enumerate(events) if e[0] == "guard")
            self.assertLess(receipt, guard)
            self.assertEqual(events[receipt][2]["relations"][1]["resolved_path"], "/synthetic/third")

    def test_first_snapshot_hashes_and_inspects_C_only_without_old_digest(self):
        policy = run.BOUNDS["native_binding"]
        c, d = policy["otool_canonical_path"], policy["otool_nominal_path"]
        with self.otool_case() as (binding, _, snapshot):
            binding.resolve()
            binding.run.deadline = time.monotonic() + 10
            hashed, inspected = [], []
            def hash_file(path, budget, deadline):
                hashed.append(str(path))
                return {"path": str(path), "sha256": "first-actual"}
            def inspect(path):
                inspected.append(str(path))
                return [{"uuid": "synthetic-controller-macho"}]
            with patch.object(native, "bind_hash", side_effect=hash_file), \
                    patch.object(native, "file_machos", side_effect=inspect), \
                    patch.object(native, "loaded_parallel", return_value=(object(), {"synthetic": True})), \
                    patch.object(native, "backing_binding", return_value={"synthetic": True}):
                manifest = native.NativeBinding.snapshot(binding)
            self.assertEqual(manifest["native_tools"]["otool"]["path"], c)
            self.assertEqual(manifest["native_tools"]["otool"]["sha256"], "first-actual")
            self.assertIn(c, hashed)
            self.assertIn(c, inspected)
            self.assertNotIn(d, hashed + inspected)

    def test_changed_C_hash_blocks_post_snapshot_even_with_correct_relations(self):
        with self.otool_case() as (binding, _, snapshot):
            binding.resolve()
            binding.use_record = {"synthetic_child": True}
            snapshot.return_value = {"synthetic_first_metadata_hash": "changed-C"}
            with self.assertRaisesRegex(cache.EvidenceError, "bound native component"):
                binding.recheck()
            self.assertFalse(binding.decoder_confirmed)

    def test_final_hash_uses_original_fixed_C_with_no_new_alias_or_image_trace(self):
        policy = run.BOUNDS["native_binding"]
        c = policy["otool_canonical_path"]
        for digest in ("first", "changed"):
            with self.subTest(digest=digest), self.otool_case() as (binding, _, _):
                binding.before = {"native_tools": {"otool": {"path": c, "sha256": "first"}},
                                  "developer_sdk_files": {}}
                binding.decoder_confirmed = True
                binding.run.deadline = time.monotonic() + 10
                with patch.object(native, "bind_hash", return_value={"path": c, "sha256": digest}) as hasher, \
                        patch.object(Path, "resolve") as resolver, patch.object(binding.run, "command") as command, \
                        patch.object(native, "decode_trace") as trace:
                    if digest == "first":
                        binding.recheck_tools_after_semantics()
                    else:
                        with self.assertRaisesRegex(cache.EvidenceError, "bound native component"):
                            binding.recheck_tools_after_semantics()
                    self.assertEqual(hasher.call_args.args[0], Path(c))
                    resolver.assert_not_called()
                    command.assert_not_called()
                    trace.assert_not_called()

    def test_otool_relation_receipt_fits_unchanged_sink(self):
        with tempfile.TemporaryDirectory() as temp, self.otool_case() as (binding, _, _):
            output = Path(temp)
            binding.run.output = cache.Output(output, run.BOUNDS["limits"])
            binding.resolve()
            receipt = output / "otool-identity-prebinding.json"
            self.assertLess(receipt.stat().st_size, 2048)
            self.assertEqual(len(json.loads(receipt.read_text())["relations"]), 2)

    def test_resolution_error_receipt_sink_failure_does_not_advance(self):
        c = run.BOUNDS["native_binding"]["otool_canonical_path"]
        with self.otool_case(errors={c: PermissionError("synthetic denied")}) as (binding, events, snapshot):
            output = binding.run.output.json
            def reject(name, value):
                if name == "otool-identity-prebinding.json":
                    self.assertNotIn("resolved_path", value["relations"][0])
                    raise cache.EvidenceError("synthetic error receipt failure")
                return output(name, value)
            with patch.object(binding.run.output, "json", side_effect=reject), \
                    self.assertRaisesRegex(cache.EvidenceError, "synthetic error receipt failure"):
                binding.resolve()
            snapshot.assert_not_called()
            self.assertFalse(any(e[:2] == ("command", "native-xcrun-version") for e in events))


class OtoolInvocationTests(unittest.TestCase):
    @contextmanager
    def invocation_case(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            controller = run.ComponentRun(root, root, "ARM24")
            c = run.BOUNDS["native_binding"]["otool_canonical_path"]
            controller.otool_path = c
            controller.native_binding = SimpleNamespace(before={"native_tools": {"otool": {"path": c}}})
            yield controller, [c, "-tvV", str(root / "fixture.macho")]

    def test_actual_inherited_launcher_receives_C_not_D_without_executing(self):
        with self.invocation_case() as (controller, args), patch(
                "macos_static_ridiff.subprocess.Popen", side_effect=OSError("synthetic launcher stop")) as launch:
            with self.assertRaisesRegex(OSError, "synthetic launcher stop"):
                controller.command("body-00-disassembly", args)
            self.assertEqual(launch.call_args.args[0], args)
            self.assertNotIn("executable", launch.call_args.kwargs)

    def test_D_other_or_unbound_invocation_is_rejected_before_launcher(self):
        policy = run.BOUNDS["native_binding"]
        c, d = policy["otool_canonical_path"], policy["otool_nominal_path"]
        for invoked, configured, bound in ((d, c, c), ("/other/llvm-otool", c, c), (c, d, c),
                                            (c, c, d), (c, c, "/other/otool")):
            with self.subTest(invoked=invoked, configured=configured, bound=bound), \
                    self.invocation_case() as (controller, args):
                controller.otool_path = configured
                controller.native_binding.before["native_tools"]["otool"]["path"] = bound
                args[0] = invoked
                with patch.object(run.Run, "command") as launch, self.assertRaises(cache.EvidenceError):
                    controller.command("body-00-disassembly", args)
                launch.assert_not_called()

    def test_D_is_rejected_even_outside_disassembly_name(self):
        with self.invocation_case() as (controller, args):
            args[0] = run.BOUNDS["native_binding"]["otool_nominal_path"]
            with patch.object(run.Run, "command") as launch, self.assertRaises(cache.EvidenceError):
                controller.command("synthetic-name", args)
            launch.assert_not_called()

    def test_recorded_D_other_missing_or_changed_argv_blocks_use_and_preserves_report(self):
        d = run.BOUNDS["native_binding"]["otool_nominal_path"]
        for replacement in (d, "/other/llvm-otool", None, []):
            with self.subTest(replacement=replacement), self.invocation_case() as (controller, args):
                reported = [replacement, *args[1:]] if isinstance(replacement, str) else replacement
                result = {"argv": reported, "rc": 0, "elapsed_seconds": 0.01,
                          "stdout": "synthetic", "stderr": "", "failure": None}
                def recorded(name, argv, **kwargs):
                    controller.output.json(name + ".json", result)
                    return result
                with patch.object(run.Run, "command", side_effect=recorded), \
                        self.assertRaisesRegex(cache.EvidenceError, "recorded otool invocation differs"):
                    controller.command("body-00-disassembly", args)
                self.assertEqual(json.loads((controller.evidence / "body-00-disassembly.json").read_text()), result)
        with self.invocation_case() as (controller, args), patch.object(
                run.Run, "command", return_value={"argv": [args[0], "--other"]}), self.assertRaises(cache.EvidenceError):
            controller.command("body-00-disassembly", args)

    def test_success_keeps_existing_report_shape_without_PID_or_main_image_trace(self):
        with self.invocation_case() as (controller, args):
            report = {"argv": args, "rc": 0, "elapsed_seconds": 0.01,
                      "stdout": "synthetic", "stderr": "", "failure": None}
            with patch.object(run.Run, "command", return_value=report) as launch, \
                    patch.object(native, "decode_trace") as trace:
                self.assertIs(controller.command("body-00-disassembly", args), report)
                self.assertEqual(launch.call_args.args[1], args)
                self.assertEqual(set(report), {"argv", "rc", "elapsed_seconds", "stdout", "stderr", "failure"})
                trace.assert_not_called()


class OtoolOutputTests(unittest.TestCase):
    def test_direct_C_format_uses_container_not_program_basename_for_both_architectures(self):
        for container in (Path("/synthetic/otool.macho"), Path("/synthetic/llvm-otool.macho")):
            for mnemonic in ("ret", "retq"):
                text = f"{container}:\n(__TEXT,__text) section\n0000000000001000\t{mnemonic}\n"
                decoded = cache.otool_instructions(text, container, 0x1000, 0x1004)
                self.assertEqual(decoded[0]["mnemonic"], mnemonic)

    def test_unexpected_headers_rows_and_objdump_formats_are_not_reinterpreted(self):
        container = Path("/synthetic/body.macho")
        prefix = f"{container}:\n(__TEXT,__text) section\n"
        valid = "0000000000001000\tret\n"
        for text in ("", valid, prefix.replace("__text", "__data") + valid,
                     "llvm-otool warning\n" + prefix + valid, prefix + "1000: c3 ret\n",
                     prefix + "0000000000001000\tc3\tret\nunexpected footer\n",
                     prefix + "0000000000001000\t.byte 0xc3\n", prefix + valid + "\n",
                     prefix + valid + "unparsed line\n", prefix.replace(str(container), "/other/body") + valid):
            with self.subTest(text=text), patch.object(cache, "instructions") as decode, \
                    self.assertRaises(cache.EvidenceError):
                cache.otool_instructions(text, container, 0x1000, 0x1004)
            decode.assert_not_called()

    def test_duplicate_out_of_range_missing_start_or_descending_rows_fail_closed(self):
        container = Path("/synthetic/body.macho")
        for addresses in ([0x1000, 0x1000], [0x1000, 0x1004], [0x1001], [0x1000, 0x1002, 0x1001]):
            text = f"{container}:\n(__TEXT,__text) section\n" + "".join(
                f"{address:016x}\tret\n" for address in addresses)
            with self.subTest(addresses=addresses), self.assertRaises(cache.EvidenceError):
                cache.otool_instructions(text, container, 0x1000, 0x1004)


class OtoolEncodingColumnTests(unittest.TestCase):
    container = Path("/synthetic/body.macho")
    prefix = "/synthetic/body.macho:\n(__TEXT,__text) section\n"

    def assert_rejected_before_decoding(self, body, end=0x1006):
        with patch.object(cache, "instructions") as decode, patch.object(cache, "edges") as cfg:
            with self.assertRaises(cache.EvidenceError):
                cache.otool_instructions(self.prefix + body, self.container, 0x1000, end)
            decode.assert_not_called()
            cfg.assert_not_called()

    def test_exact_owner_ff25_indirect_branch_columns_fail_before_instructions(self):
        self.assert_rejected_before_decoding(
            "0000000000001000\tff\t25 00 00 00 00\tjmpq *(%rip)\n")

    def test_exact_reviewer_ARM_word_br_columns_fail_before_instructions(self):
        self.assert_rejected_before_decoding("0000000000001000\td61f0000\tbr\tx0\n", 0x1004)

    def test_standalone_c3_ret_is_rejected_without_footer(self):
        self.assert_rejected_before_decoding("0000000000001000\tc3\tret\n", 0x1001)

    def test_byte_and_word_columns_cannot_hide_direct_or_indirect_control_flow(self):
        for instruction in (
                "ff 25 00 00 00 00\tjmpq *(%rip)",
                "ff\t25\t00\t00\t00\t00\tjmpq\t*(%rip)",
                "ff2500000000\tjmpq *(%rip)",
                "ff\tjmpq *(%rip)",
                "e9\t00 00 00 00\tjmp 0x1005",
                "e900000000\tjmp 0x1005",
                "eb\tjmp 0x1002",
                "c3 ret",
                "d61f0000\tbr x0",
                "d61f0000 br x0",
                "14000000\tb 0x1000",
                "00000014\tb\t0x1000",
                "deadbeef\tb 0x2000"):
            with self.subTest(instruction=instruction):
                self.assert_rejected_before_decoding("0000000000001000\t" + instruction + "\n")

    def test_extra_operand_tab_is_not_swallowed_after_a_valid_mnemonic(self):
        self.assert_rejected_before_decoding("0000000000001000\tadd\tx0, x1,\tx2\n")

    def test_encoding_words_spelled_like_real_mnemonics_cannot_hide_a_branch_operand(self):
        for instruction in ("dc\tbr x0", "fadd\tjmpq *(%rip)", "fabd\tb 0x2000",
                            "addb\tcallq 0x2000", "adcb\tret", "decb\tjmpq\t0x2000"):
            with self.subTest(instruction=instruction):
                self.assert_rejected_before_decoding("0000000000001000\t" + instruction + "\n")

    def test_a_later_encoding_row_invalidates_body_before_any_instruction_is_used(self):
        self.assert_rejected_before_decoding(
            "0000000000001000\tjmpq\t0x2000\n0000000000001005\tc3\tret\n")

    def test_legitimate_hex_looking_mnemonics_keep_canonical_operands(self):
        examples = {
            "b": "0x2000", "dc": "cvau, x0", "add": "x0, x1, x2", "adc": "%eax, %ebx",
            "adcb": "$0x1, %al", "addb": "$0x1, %al", "dec": "%eax", "decb": "%al",
            "fadd": "d0, d1, d2", "fabd": "s0, s1, s2", "fadda": "d0, p0, d1, z2.d",
            "fcadd": "v0.4s, v1.4s, v2.4s, #90", "cadd": "z0.s, z0.s, z1.s, #90",
            "bfc": "x0, #0, #8", "bfadd": "z0.h, z1.h, z2.h",
            "aaa": "", "aad": "$0xa", "daa": "",
        }
        for mnemonic, operands in examples.items():
            for separator in ("\t", " "):
                with self.subTest(mnemonic=mnemonic, separator=separator):
                    body = "0000000000001000\t" + mnemonic + (separator + operands if operands else "") + "\n"
                    decoded = cache.otool_instructions(self.prefix + body, self.container, 0x1000, 0x1004)
                    self.assertEqual((decoded[0]["mnemonic"], decoded[0]["operands"]), (mnemonic, operands))

        for mnemonic, operands in (("add", "sp, sp, #0x10"), ("adc", "xzr, x0, x1"),
                                   ("addb", "%al, 0x10(%rax)"), ("decb", "(%rax)"),
                                   ("fadd", "%st(1), %st"), ("dc", "zva, xzr")):
            with self.subTest(mnemonic=mnemonic, operands=operands):
                text = self.prefix + f"0000000000001000\t{mnemonic}\t{operands}\n"
                decoded = cache.otool_instructions(text, self.container, 0x1000, 0x1004)
                self.assertEqual(decoded[0]["operands"], operands)

    def test_canonical_control_flow_still_reaches_CFG_and_retains_edges(self):
        cases = [
            ("jmpq\t*(%rip)", "x86_64", bytes.fromhex("ff2500000000"), None),
            ("jmpq\t*0x0(%rip)", "x86_64", bytes.fromhex("ff2500000000"), "0x2000"),
            ("jmpq\t0x2000", "x86_64", bytes.fromhex("e9fb0f0000"), "0x2000"),
            ("br\tx0", "arm64", bytes.fromhex("00001fd6"), None),
            ("b\t0x2000", "arm64", bytes.fromhex("00040014"), "0x2000"),
        ]
        for instruction, arch, raw, target in cases:
            with self.subTest(instruction=instruction):
                end = 0x1000 + len(raw)
                decoded = cache.otool_instructions(self.prefix + "0000000000001000\t" + instruction + "\n",
                                                   self.container, 0x1000, end)
                branches = cache.edges(decoded, arch, 0x1000, end,
                                       lambda slot: (0x2000, {"synthetic": True}), machine_bytes=raw)
                self.assertEqual(len(branches), 1)
                self.assertEqual(branches[0]["target"], target)
                if target is None:
                    self.assertIn("unresolved", branches[0])


class OtoolCompleteOperandTests(unittest.TestCase):
    container = OtoolEncodingColumnTests.container
    prefix = OtoolEncodingColumnTests.prefix
    assert_rejected_before_decoding = OtoolEncodingColumnTests.assert_rejected_before_decoding

    def test_exact_dc00_compact_encoding_fails_without_footer(self):
        self.assert_rejected_before_decoding("0000000000001000\tdc\t00 faddl (%rax)\n", 0x1002)

    def test_every_hex_mnemonic_exception_rejects_compact_numeric_encoding_tails(self):
        mnemonics = ("aaa", "aad", "adc", "adcb", "add", "addb", "b", "bfc", "bfadd",
                     "cadd", "daa", "dc", "dec", "decb", "fabd", "fadd", "fadda", "fcadd")
        for mnemonic in mnemonics:
            for operand in ("00 faddl (%rax)", "00 ret", "12 34 br x0", "0x00 callq 0x2000"):
                with self.subTest(mnemonic=mnemonic, operand=operand):
                    self.assert_rejected_before_decoding(f"0000000000001000\t{mnemonic}\t{operand}\n")

    def test_register_immediate_memory_or_later_operand_prefix_cannot_hide_instruction(self):
        for mnemonic in ("add", "adc", "fadd", "dec"):
            for operand in ("%eax jmpq 0x2000", "x0 br x1", "$0x00 faddl (%rax)",
                            "#0 ret", "(%rax) callq 0x2000", "0x10(%rax) ret",
                            "x0, 00 faddl (%rax)", "%eax, %ebx jmpq 0x2000"):
                with self.subTest(mnemonic=mnemonic, operand=operand):
                    self.assert_rejected_before_decoding(f"0000000000001000\t{mnemonic}\t{operand}\n")

    def test_dc_requires_its_complete_named_operation_register_form_exclusively(self):
        for operand in ("", "00", "00, x0", "%eax", "x0", "#0", "(%rax)", "cvau, x0 ret",
                        "cvau, x0, 00 faddl (%rax)", "cvau, x0 ;ok\tbr x0"):
            with self.subTest(operand=operand):
                self.assert_rejected_before_decoding(f"0000000000001000\tdc\t{operand}\n")

    def test_complete_canonical_operand_forms_are_retained_without_rewriting(self):
        examples = [
            ("b", "0x2000"), ("dc", "cvau, x0"), ("dc", "zva, xzr"),
            ("dc", "cvau, x0 ; canonical comment"),
            ("add", "x0, x1, x2, lsl #0x2"), ("add", "x0, x1, w2, uxtw #2"),
            ("add", "sp, sp, #0x10"), ("adc", "xzr, x0, x1"), ("adc", "%eax, %ebx"),
            ("addb", "$0x1, %al"), ("decb", "(%rax)"), ("decb", "0x10(%rax)"),
            ("decb", "%fs:0x10(%rax,%rcx,4)"), ("decb", "0x20(,%rax,8)"),
            ("decb", "%gs:0x10"), ("dec", "0x1000"), ("dec", "-0x10(%rax)"),
            ("fadd", "%st(1), %st"), ("fadd", "d0, d1, d2"),
            ("fadd", "z0.s, p0/m, z0.s, #0.5"), ("fcadd", "v0.4s, v1.4s, v2.4s, #90"),
            ("addb", "%al, 0x10(%rax) ## canonical comment"), ("aad", "$0xa"), ("aaa", ""),
        ]
        for mnemonic, operands in examples:
            with self.subTest(mnemonic=mnemonic, operands=operands):
                text = self.prefix + f"0000000000001000\t{mnemonic}" + (
                    "\t" + operands if operands else "") + "\n"
                decoded = cache.otool_instructions(text, self.container, 0x1000, 0x1004)
                self.assertEqual((decoded[0]["mnemonic"], decoded[0]["operands"]), (mnemonic, operands))

    def test_incomplete_operand_lists_or_unbalanced_memory_are_not_valid_prefixes(self):
        for operand in ("x0,", ",x0", "x0,,x1", "0x10(%rax", "(%rax))", "%st(1",
                        "(%rax,%rcx,4) junk", "$0x1foo", "#0x1 ret", "x0, lsl #2 junk", "lsl #1"):
            with self.subTest(operand=operand):
                self.assert_rejected_before_decoding(f"0000000000001000\tadd\t{operand}\n")

    def test_bad_compact_encoding_after_valid_branch_blocks_the_whole_body(self):
        self.assert_rejected_before_decoding(
            "0000000000001000\tjmpq\t0x2000\n0000000000001005\tdc\t00 faddl (%rax)\n", 0x1007)


if __name__ == "__main__":
    unittest.main()
