"""Bounded data-only dyld cache fixup reader for the selected #172 arm64e image."""

import hashlib
import json
from pathlib import Path
import re
import struct
import uuid


SLIDE = 0xB6D8000
RUNTIME_STUB = 0x18BADE310
RUNTIME_SLOT = 0x1F9C57250
STUB_PREFIX = bytes.fromhex("d10b37b031420991300240f9110a1fd7")
EXPECTED_IMAGES = {
    "/usr/lib/system/libsystem_c.dylib": "d77ceb62aff63cecba9b4f057fbe2eb5",
    "/usr/lib/system/libsystem_kernel.dylib": "c6a4a4cb92e63bafaae0e8306259209a",
}


class EvidenceError(RuntimeError):
    pass


def require(condition, reason):
    if not condition:
        raise EvidenceError(reason)


def save(path, value):
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n", encoding="utf-8")


def adrp_page(instruction, pc):
    require(instruction & 0x9F000000 == 0x90000000, "not ADRP")
    immediate = ((instruction >> 29) & 3) | (((instruction >> 5) & 0x7FFFF) << 2)
    if immediate & (1 << 20):
        immediate -= 1 << 21
    return (pc & ~0xFFF) + (immediate << 12)


def parse_decimal_adrp(operands, pc):
    match = re.fullmatch(r"x(\d+),\s*(-?\d+)\s*;\s*(0x[0-9a-fA-F]+)", operands)
    require(match is not None, "unsupported decimal ADRP text")
    page = (pc & ~0xFFF) + int(match[2]) * 4096
    require(page == int(match[3], 16), "ADRP rendered comment disagrees with decimal operand")
    return page


def stub_receipt():
    first, add, load, branch = struct.unpack("<4I", STUB_PREFIX)
    page = adrp_page(first, RUNTIME_STUB)
    require((first & 31) == 17 and add & 0xFFC003FF == 0x91000231,
            "unexpected ADRP/ADD registers or immediate form")
    slot = page + ((add >> 10) & 0xFFF)
    require(load == 0xF9400230 and branch == 0xD71F0A11, "unexpected LDR/BRAA stub")
    require(slot == RUNTIME_SLOT, "decoded native stub slot mismatch")
    require(parse_decimal_adrp("x17, 450937 ; 0x1f9c57000", RUNTIME_STUB) == page,
            "decimal ADRP regression")
    return {
        "native_run": 39979771410,
        "native_file": "function-14-18bade310.bin",
        "native_full_window_sha256": "35f2438f3563735332e2da53512191f1b8285da8a163940baa98e4bb6f1a7f7c",
        "prefix_hex": STUB_PREFIX.hex(), "prefix_bytes_used": 16,
        "runtime_stub": hex(RUNTIME_STUB), "native_slide": hex(SLIDE),
        "runtime_page": hex(page), "runtime_slot": hex(slot),
        "unslid_stub": hex(RUNTIME_STUB - SLIDE), "unslid_slot": hex(slot - SLIDE),
        "text_operand": "x17, 450937 ; 0x1f9c57000",
        "decimal_and_raw_ADRP_agree": True,
        "neighboring_window_stubs_used_as_edges": False,
    }


def uleb(data, offset):
    value = 0
    for shift in range(0, 70, 7):
        require(offset < len(data), "truncated ULEB")
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << shift
        if not byte & 128:
            require(value <= 0xFFFFFFFFFFFFFFFF, "ULEB overflow")
            return value, offset
    raise EvidenceError("ULEB overflow")


def decode_pointer(raw, version, value_add):
    auth = bool(raw >> 63)
    if version == 5:
        offset = raw & ((1 << 34) - 1)
        target = value_add + (offset if auth else offset | (((raw >> 34) & 255) << 56))
        return {"raw": hex(raw), "version": 5, "authenticated": auth,
                "runtime_offset": hex(offset), "next_units_8_bytes": (raw >> 52) & 2047,
                "target_unslid": hex(target), "diversity": (raw >> 34) & 65535 if auth else None,
                "address_diversity": bool((raw >> 50) & 1) if auth else None,
                "key": ("DA" if (raw >> 51) & 1 else "IA") if auth else None,
                "kind": "cache-rebase-not-symbol-bind"}
    require(version == 3, "unsupported slide version; do not guess pointer format")
    if auth:
        target = value_add + (raw & 0xFFFFFFFF)
    else:
        # dyld pointer3 carries top byte plus a signed bottom 43-bit address.
        lower = raw & ((1 << 43) - 1)
        if lower & (1 << 42):
            lower |= ((1 << 56) - 1) ^ ((1 << 43) - 1)
        target = (((raw >> 43) & 255) << 56) | lower
    return {"raw": hex(raw), "version": 3, "authenticated": auth,
            "next_units_8_bytes": (raw >> 51) & 2047, "target_unslid": hex(target),
            "diversity": (raw >> 32) & 65535 if auth else None,
            "address_diversity": bool((raw >> 48) & 1) if auth else None,
            "key": ("IA", "IB", "DA", "DB")[(raw >> 49) & 3] if auth else None,
            "kind": "cache-rebase-not-symbol-bind"}


class Cache:
    """Modern cache headers; offsets follow pinned ipsw pkg/dyld/types.go."""

    def __init__(self, primary, evidence):
        self.primary = primary
        self.evidence = evidence
        self.files = {}
        self.mappings = []
        header = self.add_file(primary)
        require(header["mapping_offset"] >= 0x200, "unsupported old cache header")
        suboffset, subcount = struct.unpack_from("<II", header["raw"], 0x188)
        require(subcount <= 128, "subcache count bound")
        table = self.read(primary, suboffset, subcount * 56)
        (evidence / "subcache-table.bin").write_bytes(table)
        self.subcaches = []
        for index in range(subcount):
            entry = table[index * 56:(index + 1) * 56]
            expected = uuid.UUID(bytes=entry[:16]).hex
            suffix = entry[24:56].split(b"\0")[0].decode("ascii")
            require(re.fullmatch(r"\.[A-Za-z0-9_.]+", suffix) is not None, "invalid subcache suffix")
            subpath = primary.with_name(primary.name + suffix)
            subheader = self.add_file(subpath)
            require(subheader["uuid"] == expected, "subcache UUID mismatch")
            self.subcaches.append({"path": subpath.name, "uuid": expected,
                                  "vm_offset": hex(struct.unpack_from("<Q", entry, 16)[0])})
        self.images = []
        image_offset, image_count = struct.unpack_from("<II", header["raw"], 0x1C0)
        require(0 < image_count <= 65536, "image count outside bound")
        for index in range(image_count):
            entry = self.read(primary, image_offset + index * 32, 32)
            address, _, _, name_offset, _ = struct.unpack("<QQQII", entry)
            name = self.read(primary, name_offset, min(4096, primary.stat().st_size - name_offset))
            require(b"\0" in name, "unterminated cache image path")
            self.images.append({"address": address, "name": name.split(b"\0")[0].decode("utf-8")})
        self.images.sort(key=lambda item: item["address"])

    def read(self, path, offset, size):
        require(0 <= size <= 8 * 1024 * 1024 and offset >= 0 and
                offset + size <= path.stat().st_size, "bounded cache read outside file")
        with path.open("rb") as file:
            file.seek(offset)
            data = file.read(size)
        require(len(data) == size, "short cache read")
        return data

    def add_file(self, path):
        require(path.is_file() and not path.is_symlink(), "declared cache file missing or symlink")
        raw = self.read(path, 0, 0x240)
        require(raw[:16].startswith(b"dyld_") and b"arm64e" in raw[:16],
                "cache architecture/family mismatch")
        mapping_offset, mapping_count = struct.unpack_from("<II", raw, 16)
        require(mapping_offset >= 0x200, "unsupported cache header size")
        identity = uuid.UUID(bytes=raw[88:104]).hex
        header = {"raw": raw, "uuid": identity, "mapping_offset": mapping_offset,
                  "size": path.stat().st_size, "path": path.name, "mapping_count": mapping_count}
        self.files[path] = header
        (self.evidence / (path.name + ".header.bin")).write_bytes(raw)
        offset, count = struct.unpack_from("<II", raw, 0x138)
        require(0 < count <= 128, "mapping-with-slide count outside bound")
        for index in range(count):
            record = self.read(path, offset + index * 56, 56)
            address, size, fileoff, slideoff, slidesize, flags, maxprot, initprot = struct.unpack(
                "<6Q2I", record)
            require(fileoff + size <= path.stat().st_size, "cache mapping outside file")
            self.mappings.append({
                "path": path, "address": address, "size": size, "file_offset": fileoff,
                "slide_offset": slideoff, "slide_size": slidesize, "flags": flags,
                "maxprot": maxprot, "initprot": initprot, "record_offset": offset + index * 56,
                "record_hex": record.hex(), "cache_uuid": identity})
        return header

    def mapping(self, address, size=1):
        matches = [m for m in self.mappings if m["address"] <= address and
                   address + size <= m["address"] + m["size"]]
        require(len(matches) == 1, "ambiguous or absent cache mapping")
        return matches[0]

    def vmread(self, address, size):
        mapping = self.mapping(address, size)
        return self.read(mapping["path"], mapping["file_offset"] + address - mapping["address"], size)

    def macho(self, image):
        head = self.vmread(image["address"], 32)
        magic, cpu, subtype, _, ncmds, cmdsize, _, _ = struct.unpack("<IiiIIIII", head)
        require(magic == 0xFEEDFACF and cpu == 0x100000C and subtype & 255 == 2,
                "Mach-O not expected arm64e")
        require(ncmds <= 1024 and cmdsize <= 1024 * 1024, "load-command bound")
        data = self.vmread(image["address"] + 32, cmdsize)
        commands = []
        cursor = 0
        result = {**image, "uuid": None, "segments": [], "starts": None, "exports": None}
        for _ in range(ncmds):
            require(cursor + 8 <= len(data), "truncated load command")
            cmd, size = struct.unpack_from("<II", data, cursor)
            require(size >= 8 and cursor + size <= len(data), "invalid load command")
            body = data[cursor:cursor + size]
            if cmd == 0x1B:
                result["uuid"] = uuid.UUID(bytes=body[8:24]).hex
            elif cmd == 0x19:
                name, addr, vmsize, fileoff, filesize, maxprot, initprot, _, _ = struct.unpack_from(
                    "<16sQQQQiiII", body, 8)
                result["segments"].append({"name": name.rstrip(b"\0").decode("ascii"),
                                           "address": addr, "size": vmsize,
                                           "file_offset": fileoff, "file_size": filesize,
                                           "maxprot": maxprot, "initprot": initprot})
            elif cmd in (0x26, 0x80000033):
                result["starts" if cmd == 0x26 else "exports"] = struct.unpack_from("<II", body, 8)
            elif cmd in (0x22, 0x80000022) and not result["exports"]:
                result["exports"] = struct.unpack_from("<II", body, 40)
            commands.append({"cmd": hex(cmd), "size": size, "offset": cursor + 32})
            cursor += size
        require(result["uuid"] is not None, "missing LC_UUID")
        result["load_commands"] = commands
        result["load_commands_hex"] = (head + data).hex()
        return result

    def filedata(self, image, pair):
        require(pair is not None, "required linkedit data absent")
        offset, size = pair
        require(0 < size <= 8 * 1024 * 1024, "linkedit data bound")
        matches = [s for s in image["segments"] if s["file_offset"] <= offset and
                   offset + size <= s["file_offset"] + s["file_size"]]
        require(len(matches) == 1, "linkedit mapping ambiguous")
        segment = matches[0]
        return self.vmread(segment["address"] + offset - segment["file_offset"], size)

    def resolve_slot(self, address):
        mapping = self.mapping(address, 8)
        require(mapping["slide_size"] >= 24, "slot mapping has no slide-info header")
        header = self.read(mapping["path"], mapping["slide_offset"], 24)
        version, page_size, count, _, add = struct.unpack("<IIIIQ", header)
        require(version in (3, 5) and page_size in (4096, 16384), "unsupported cache slide layout")
        index = (address - mapping["address"]) // page_size
        require(index < count and 24 + (index + 1) * 2 <= mapping["slide_size"],
                "page-start outside slide metadata")
        start_offset = mapping["slide_offset"] + 24 + index * 2
        start_bytes = self.read(mapping["path"], start_offset, 2)
        start = struct.unpack("<H", start_bytes)[0]
        require(start != 0xFFFF, "slot page has no fixups")
        page_vm = mapping["address"] + index * page_size
        page_file = mapping["file_offset"] + index * page_size
        page = self.read(mapping["path"], page_file, page_size)
        (self.evidence / "slot-page.bin").write_bytes(page)
        (self.evidence / "slot-slide-header.bin").write_bytes(header)
        (self.evidence / "slot-page-start.bin").write_bytes(start_bytes)
        (self.evidence / "slot-mapping-record.bin").write_bytes(bytes.fromhex(mapping["record_hex"]))
        offset, chain, selected = start, [], None
        for _ in range(page_size // 8):
            require(offset % 8 == 0 and offset + 8 <= page_size, "fixup chain left page")
            raw = struct.unpack_from("<Q", page, offset)[0]
            decoded = decode_pointer(raw, version, add)
            record = {**decoded, "page_offset": offset, "unslid_location": hex(page_vm + offset)}
            chain.append(record)
            if page_vm + offset == address:
                require(selected is None, "repeated fixup slot")
                selected = record
            delta = decoded["next_units_8_bytes"] * 8
            if not delta:
                break
            offset += delta
        else:
            raise EvidenceError("fixup traversal exceeded page bound")
        require(selected is not None, "requested slot not in authenticated page chain")
        return {
            "mapping": {k: str(v) if isinstance(v, Path) else v for k, v in mapping.items()},
            "slide_version": version, "value_add": hex(add), "page_index": index,
            "page_size": page_size, "page_unslid_address": hex(page_vm),
            "page_file_offset": page_file, "page_start_file_offset": start_offset,
            "page_start": start, "chain": chain, "selected_fixup": selected,
            "evidence_kind": "offline cache slide rebase, not retroactive live pointer",
        }

    def target(self, address):
        candidates = [i for i in self.images if i["address"] <= address]
        require(candidates, "target before all cache images")
        image = self.macho(candidates[-1])
        require(any(s["address"] <= address < s["address"] + s["size"] and
                    s["initprot"] & 4 for s in image["segments"]), "target not in image executable segment")
        starts = self.filedata(image, image["starts"])
        cursor, current, previous, extent = 0, image["address"], None, None
        while cursor < len(starts):
            delta, cursor = uleb(starts, cursor)
            if delta == 0:
                break
            current += delta
            if current > address:
                extent = (previous, current)
                break
            previous = current
        require(extent and extent[0] == address and 0 < extent[1] - address <= 32768,
                "target lacks exact bounded function-start extent")
        (self.evidence / "target-function-starts.bin").write_bytes(starts)
        exports = self.filedata(image, image["exports"])
        require(len(exports) <= 2 * 1024 * 1024, "target export-trie evidence bound")
        (self.evidence / "target-export-trie.bin").write_bytes(exports)
        stack, visited, names = [(0, "")], set(), []
        while stack:
            offset, name = stack.pop()
            require(offset not in visited and len(visited) < 100000 and len(name) <= 4096,
                    "invalid or excessive export trie")
            visited.add(offset)
            terminal_size, cursor = uleb(exports, offset)
            terminal_end = cursor + terminal_size
            require(terminal_end < len(exports), "export terminal outside trie")
            if terminal_size:
                flags, value_cursor = uleb(exports, cursor)
                if not flags & 8:  # Reexports do not name a local address.
                    value, _ = uleb(exports, value_cursor)
                    if (image["address"] + value) == address:
                        names.append({"name": name, "flags": flags, "trie_node_offset": offset,
                                      "address": hex(address)})
            children = exports[terminal_end]
            cursor = terminal_end + 1
            for _ in range(children):
                end = exports.find(b"\0", cursor)
                require(end >= cursor, "unterminated export edge")
                part = exports[cursor:end].decode("utf-8")
                child, cursor = uleb(exports, end + 1)
                stack.append((child, name + part))
        require(names, "fixup-derived target has no matching export symbol")
        data = self.vmread(address, extent[1] - address)
        (self.evidence / "target.bin").write_bytes(data)
        return {"image": image, "symbols": names, "start": hex(address), "end": hex(extent[1]),
                "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                "symbol_lookup_performed_after_fixup_decode": True}


def resolve(primary, evidence):
    cache = Cache(primary, evidence)
    layout = {"primary": str(primary), "subcaches": cache.subcaches,
              "files": [{k: v for k, v in h.items() if k != "raw"} for h in cache.files.values()],
              "mapping_count": len(cache.mappings), "image_count": len(cache.images)}
    save(evidence / "cache-layout.json", layout)
    checked = {}
    for name, expected in EXPECTED_IMAGES.items():
        found = [image for image in cache.images if image["name"] == name]
        require(len(found) == 1, "required library missing or duplicated")
        image = cache.macho(found[0])
        checked[name] = image
        save(evidence / (Path(name).name + ".identity.json"), image)
        require(image["uuid"] == expected, "exact native library LC_UUID mismatch: " + name)
    receipt = stub_receipt()
    actual = cache.vmread(RUNTIME_STUB - SLIDE, len(STUB_PREFIX))
    (evidence / "static-stub-prefix.bin").write_bytes(actual)
    require(actual == STUB_PREFIX, "exact static/native stub prefix mismatch")
    save(evidence / "stub-mapping.json", receipt)
    fixup = cache.resolve_slot(RUNTIME_SLOT - SLIDE)
    save(evidence / "slot-fixup.json", fixup)
    target = cache.target(int(fixup["selected_fixup"]["target_unslid"], 16))
    save(evidence / "target.json", target)
    return target
