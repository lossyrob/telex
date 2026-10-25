import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(1, str(HERE.parent))
import cache
import gate

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
        for field in ("decoder_pin_disposition", "intel_family_disposition"):
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
        self.assertIn("callback", branches[0]["unresolved"])

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


if __name__ == "__main__":
    unittest.main()
