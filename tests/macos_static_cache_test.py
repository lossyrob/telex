import hashlib
import io
import lzma
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

from macos_static_cache import (
    Cache, EvidenceError, RUNTIME_SLOT, RUNTIME_STUB, SLIDE, STUB_PREFIX,
    adrp_page, decode_pointer, parse_decimal_adrp, stub_receipt, uleb,
)
from macos_static_ridiff import Run, ridiff_output_bound


class StaticEvidenceTests(unittest.TestCase):
    def test_native_stub_raw_and_decimal_adrp(self):
        self.assertEqual(STUB_PREFIX.hex(), "d10b37b031420991300240f9110a1fd7")
        raw = struct.unpack_from("<I", STUB_PREFIX)[0]
        self.assertEqual(adrp_page(raw, RUNTIME_STUB), 0x1F9C57000)
        self.assertEqual(parse_decimal_adrp("x17, 450937 ; 0x1f9c57000", RUNTIME_STUB),
                         0x1F9C57000)
        self.assertEqual(stub_receipt()["unslid_slot"], hex(RUNTIME_SLOT - SLIDE))
        self.assertEqual(stub_receipt()["prefix_bytes_used"], 16)
        with self.assertRaises(EvidenceError):
            parse_decimal_adrp("x17, 450938 ; 0x1f9c57000", RUNTIME_STUB)
        self.assertEqual(adrp_page(0xF0FFFFF1, 0x10000), 0xF000)

    def test_uleb(self):
        self.assertEqual(uleb(b"\xe5\x8e\x26", 0), (624485, 3))
        with self.assertRaises(EvidenceError):
            uleb(b"\x80", 0)

    def test_authenticated_pointer5(self):
        raw = (1 << 63) | (9 << 52) | (1 << 51) | (1 << 50) | (0x1234 << 34) | 0x443322
        decoded = decode_pointer(raw, 5, 0x180000000)
        self.assertEqual(decoded["target_unslid"], "0x180443322")
        self.assertEqual(decoded["next_units_8_bytes"], 9)
        self.assertEqual(decoded["diversity"], 0x1234)
        self.assertTrue(decoded["address_diversity"])
        self.assertEqual(decoded["key"], "DA")
        self.assertEqual(decode_pointer((0xAB << 34) | 0x100, 5, 0x180000000)["target_unslid"],
                         "0xab00000180000100")

    def test_authenticated_pointer3(self):
        raw = (1 << 63) | (4 << 51) | (2 << 49) | (1 << 48) | (0x5678 << 32) | 0x1234
        decoded = decode_pointer(raw, 3, 0x180000000)
        self.assertEqual(decoded["target_unslid"], "0x180001234")
        self.assertEqual(decoded["next_units_8_bytes"], 4)
        self.assertEqual(decoded["key"], "DA")
        self.assertEqual(decoded["diversity"], 0x5678)
        with self.assertRaises(EvidenceError):
            decode_pointer(raw, 7, 0)

    def test_exact_page_chain_membership_and_receipts(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory)
            cache_file = evidence / "cache"
            data = bytearray(8192)
            struct.pack_into("<IIIIQ", data, 0, 5, 4096, 1, 0, 0x180000000)
            struct.pack_into("<H", data, 24, 0x10)
            struct.pack_into("<Q", data, 4096 + 0x10, (1 << 63) | (2 << 52) | 0x100)
            struct.pack_into("<Q", data, 4096 + 0x20, (1 << 63) | 0x200)
            cache_file.write_bytes(data)
            cache = Cache.__new__(Cache)
            cache.evidence = evidence
            cache.mappings = [{"path": cache_file, "address": 0x200000000, "size": 4096,
                               "file_offset": 4096, "slide_offset": 0, "slide_size": 26,
                               "record_hex": "00" * 56, "cache_uuid": "synthetic"}]
            result = cache.resolve_slot(0x200000020)
            self.assertEqual(result["selected_fixup"]["target_unslid"], "0x180000200")
            self.assertEqual(len(result["chain"]), 2)
            self.assertEqual((evidence / "slot-page.bin").stat().st_size, 4096)
            with self.assertRaises(EvidenceError):
                cache.resolve_slot(0x200000018)

    def test_ridiff_capacity_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            patch_path = Path(directory) / "cryptex-system-arm64e"
            plain = b"\0" * 32 + struct.pack("<4Q", 4096, 0, 1, 0) + struct.pack("<QQ", 0, 8192)
            compressed = lzma.compress(plain)
            self.assertLess(len(compressed), len(plain))
            pbzx = b"pbzx" + struct.pack(">QQQ", 1048576, len(plain), len(compressed)) + compressed
            header = bytearray(62)
            header[:8] = b"RIDIFF10"
            struct.pack_into("<H", header, 8, 1)
            struct.pack_into("<I", header, 26, 100)
            struct.pack_into("<QQ", header, 30, 62, 62 + len(pbzx))
            patch_path.write_bytes(header + pbzx)
            result = ridiff_output_bound(patch_path)
            self.assertEqual(result["extent_high_water"], 8192)
            self.assertEqual(result["conservative_output_file_bound"], 8192 + 100 + 1024**3)
            self.assertEqual(result["metadata_sha256"], hashlib.sha256(plain).hexdigest())

    def test_capacity_refuses_without_mutating_shared_state(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = Run(root, root)
            with patch("macos_static_ridiff.shutil.disk_usage") as usage:
                usage.return_value = type("Usage", (), {"free": 1024, "total": 2048})()
                with self.assertRaises(EvidenceError):
                    run.capacity(2048, "test")
            self.assertEqual(run.receipt["capacity"][0]["free_bytes"], 1024)

    def test_hash_gate_rejects_mismatch_and_oversize(self):
        with tempfile.TemporaryDirectory() as directory:
            run = Run(Path(directory), Path(directory))
            with self.assertRaises(EvidenceError):
                run.stream_hash(io.BytesIO(b"wrong"), 5, "0" * 64, "wrong")
            with self.assertRaises(EvidenceError):
                run.stream_hash(io.BytesIO(b"oversized"), 1, "0" * 64, "oversized")


if __name__ == "__main__":
    unittest.main()
