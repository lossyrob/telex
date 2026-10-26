"""New offline candidate reader; completed ARM25 parser and evidence stay immutable."""

import bisect
import collections
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import sys
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from macos_static_cache import Cache, EvidenceError, adrp_page, decode_pointer, require, uleb
from macos_provenance_capture import instructions, macho_function


class Output:
    def __init__(self, root, limits):
        self.root, self.limits = root, limits

    def write(self, name, data):
        require(Path(name).name == name, "flat artifact path required")
        if isinstance(data, str):
            data = data.encode()
        require(len(data) <= self.limits["per_file_bytes"], "per-file evidence cap")
        destination = self.root / name
        files = list(self.root.iterdir())
        require(all(p.is_file() and not p.is_symlink() for p in files), "non-file artifact")
        count = len(files) + (not destination.exists())
        total = sum(p.stat().st_size for p in files if p != destination) + len(data)
        require(count <= self.limits["artifact_files_per_job"] - 2 and
                total <= self.limits["artifact_bytes_per_job"] - 1024 * 1024,
                "artifact budget exhausted")
        destination.write_bytes(data)
        return name

    def json(self, name, data):
        return self.write(name, json.dumps(data, sort_keys=True, indent=2) + "\n")


def image_location(root, relative):
    """Only ENOENT is absence; never traverse a symlink or escape an image root."""
    require(root.is_absolute() and root.resolve(strict=True) == root, "image root is not canonical")
    parts = Path(relative).parts
    require(parts and not Path(relative).is_absolute() and all(p not in (".", "..") for p in parts),
            "invalid image-relative location")
    current = root
    root_device = root.lstat().st_dev
    for index, part in enumerate(parts):
        current = current / part
        try:
            info = current.lstat()
        except FileNotFoundError:
            return None
        except OSError as error:
            raise EvidenceError("inaccessible/unclassifiable image location: " + str(current)) from error
        require(not stat.S_ISLNK(info.st_mode), "symlinked image location/ancestor")
        require(info.st_dev == root_device, "image location crosses into another mounted filesystem")
        require(current.resolve(strict=True).is_relative_to(root), "image alias escapes selected root")
        if index < len(parts) - 1:
            require(stat.S_ISDIR(info.st_mode), "image location ancestor is not a directory")
        else:
            require(stat.S_ISREG(info.st_mode), "image location is not a regular file")
            try:
                with current.open("rb"):
                    pass
            except OSError as error:
                raise EvidenceError("unreadable image candidate: " + str(current)) from error
    return current


def choose_family(mount, config):
    candidates = [image_location(mount, p) for p in config["cache_primary_candidates"]]
    ambiguous = [image_location(mount, p) for p in config.get("ambiguity_checks", [])]
    require(not any(p is not None for p in ambiguous),
            "different Intel family present; decision-needed, not fallback")
    existing = [p for p in candidates if p is not None]
    require(len(existing) == 1, "missing/ambiguous selected cache family")
    return existing[0]


def validate_cpu(family, cpu, subtype):
    expected_cpu = 0x100000C if family == "arm64e" else 0x1000007
    allowed = (2, -2147483646) if family == "arm64e" else (3,)
    require(cpu == expected_cpu and subtype in allowed, "raw CPU/subtype/capabilities not in frozen policy")


def export_entries(raw, base, node_limit=100000):
    stack, visited, entries = [(0, "")], set(), []
    while stack:
        node, name = stack.pop()
        require(node not in visited and len(visited) < node_limit and len(name) <= 4096,
                "cyclic or excessive export trie")
        visited.add(node)
        size, cursor = uleb(raw, node)
        end = cursor + size
        require(end < len(raw), "truncated export trie")
        if size:
            flags, pos = uleb(raw, cursor)
            if flags & 8:
                ordinal, pos = uleb(raw, pos)
                close = raw.find(b"\0", pos, end)
                require(close >= pos, "bad reexport string")
                entries.append({"name": name, "flags": flags, "node": node,
                                "reexport_ordinal": ordinal,
                                "reexport_name": raw[pos:close].decode()})
            else:
                value, _ = uleb(raw, pos)
                entries.append({"name": name, "flags": flags, "node": node,
                                "address": value if flags & 3 == 2 else base + value})
        children, cursor = raw[end], end + 1
        for _ in range(children):
            close = raw.find(b"\0", cursor)
            require(close >= cursor, "bad export edge")
            edge = raw[cursor:close].decode()
            child, cursor = uleb(raw, close + 1)
            stack.append((child, name + edge))
    return entries


class CandidateCache(Cache):
    def __init__(self, primary, output, family):
        self.output, self.family = output, family
        self.parsed, self.pages = {}, {}
        self.metadata_bytes = 0
        super().__init__(primary, output.root)
        self.output.json("cache-layout.json", {
            "candidate_only": True, "family": family, "primary": str(primary),
            "files": [{k: v for k, v in h.items() if k != "raw"} for h in self.files.values()],
            "subcaches": self.subcaches, "mapping_count": len(self.mappings),
            "image_count": len(self.images)})

    def add_file(self, path):
        require(path.is_file() and not path.is_symlink(), "missing declared subcache")
        raw = self.read(path, 0, 0x240)
        magic = raw[:16].rstrip(b"\0 ").decode("ascii")
        require(magic.split() == ["dyld_v1", self.family], "cache signature/full family token mismatch")
        offset, count = struct.unpack_from("<II", raw, 16)
        require(offset >= 0x200 and 0 < count <= 128, "unsupported cache header")
        cache_type = struct.unpack_from("<Q", raw, 104)[0]
        subtype = struct.unpack_from("<I", raw, 0x1C8)[0]
        identity = uuid.UUID(bytes=raw[88:104]).hex
        header = {"raw": raw, "uuid": identity, "mapping_offset": offset,
                  "size": path.stat().st_size, "path": path.name, "mapping_count": count,
                  "cache_type": cache_type, "cache_subtype": subtype}
        self.files[path] = header
        self.output.write(path.name + ".header.bin", raw)
        offset, count = struct.unpack_from("<II", raw, 0x138)
        require(0 < count <= 128, "mapping count bound")
        for i in range(count):
            record = self.read(path, offset + i * 56, 56)
            address, size, fileoff, slideoff, slidesize, flags, maxprot, initprot = struct.unpack("<6Q2I", record)
            require(fileoff + size <= path.stat().st_size, "mapping outside cache")
            self.mappings.append(dict(path=path, address=address, size=size, file_offset=fileoff,
                                      slide_offset=slideoff, slide_size=slidesize, flags=flags,
                                      maxprot=maxprot, initprot=initprot, record_offset=offset + i * 56,
                                      record_hex=record.hex(), cache_uuid=identity))
        return header

    def macho(self, image):
        if image["address"] in self.parsed:
            return self.parsed[image["address"]]
        require(len(self.parsed) < self.output.limits["image_metadata_count"], "image metadata cap")
        head = self.vmread(image["address"], 32)
        magic, cpu, subtype, _, count, size, _, _ = struct.unpack("<IiiIIIII", head)
        require(magic == 0xFEEDFACF, "image is not Mach-O64")
        validate_cpu(self.family, cpu, subtype)
        require(count <= 1024 and size <= 1024 * 1024, "load command cap")
        raw = head + self.vmread(image["address"] + 32, size)
        result = {**image, "uuid": None, "cpu": cpu, "subtype": subtype,
                  "subtype_unsigned": subtype & 0xFFFFFFFF, "capability_bits": (subtype & 0xFFFFFFFF) >> 24,
                  "segments": [], "sections": [], "starts": None, "exports": None}
        cursor = 32
        for _ in range(count):
            cmd, length = struct.unpack_from("<II", raw, cursor)
            require(length >= 8 and cursor + length <= len(raw), "invalid load command")
            body = raw[cursor:cursor + length]
            if cmd == 0x1B:
                result["uuid"] = uuid.UUID(bytes=body[8:24]).hex
            elif cmd == 0x19:
                name, addr, vmsize, fileoff, filesize, maxprot, initprot, nsects, _ = struct.unpack_from(
                    "<16sQQQQiiII", body, 8)
                require(72 + nsects * 80 <= len(body), "truncated sections")
                segment = {"name": name.rstrip(b"\0").decode(), "address": addr, "size": vmsize,
                           "file_offset": fileoff, "file_size": filesize, "initprot": initprot, "maxprot": maxprot}
                result["segments"].append(segment)
                for i in range(nsects):
                    values = struct.unpack_from("<16s16sQQ8I", body, 72 + i * 80)
                    result["sections"].append({"name": values[0].rstrip(b"\0").decode(),
                        "segment": values[1].rstrip(b"\0").decode(), "address": values[2],
                        "size": values[3], "flags": values[8], "stub_size": values[10]})
            elif cmd in (0x26, 0x80000033):
                result["starts" if cmd == 0x26 else "exports"] = struct.unpack_from("<II", body, 8)
            elif cmd in (0x22, 0x80000022) and not result["exports"]:
                result["exports"] = struct.unpack_from("<II", body, 40)
            cursor += length
        require(result["uuid"], "missing offline image LC_UUID")
        stem = "image-" + result["uuid"]
        self.output.write(stem + ".header.bin", raw)
        result["header_file"] = stem + ".header.bin"
        entries = self.filedata(result, result["exports"])
        starts = self.filedata(result, result["starts"])
        self.metadata_bytes += len(raw) + len(entries) + len(starts)
        require(len(entries) <= self.output.limits["export_trie_bytes"] and
                self.metadata_bytes <= self.output.limits["image_metadata_bytes"], "image linkedit cap")
        self.output.write(stem + ".exports.bin", entries)
        self.output.write(stem + ".starts.bin", starts)
        result["export_file"], result["starts_file"] = stem + ".exports.bin", stem + ".starts.bin"
        result["symbols"] = export_entries(entries, image["address"])
        result["function_starts"] = []
        current, offset = image["address"], 0
        while offset < len(starts):
            delta, offset = uleb(starts, offset)
            if not delta:
                break
            current += delta
            result["function_starts"].append(current)
        self.output.json(stem + ".json", {k: v for k, v in result.items()
                                         if k not in ("symbols", "function_starts")})
        self.parsed[image["address"]] = result
        return result

    def body(self, address):
        images = [i for i in self.images if i["address"] <= address]
        require(images, "target outside image list")
        image = self.macho(images[-1])
        require(any(s["initprot"] & 4 and s["address"] <= address < s["address"] + s["size"]
                    for s in image["segments"]), "target not executable image code")
        stubs = [s for s in image["sections"] if s["flags"] & 255 == 8 and
                 s["address"] <= address < s["address"] + s["size"]]
        if stubs:
            require(len(stubs) == 1, "ambiguous stub section")
            section = stubs[0]
            size = section["stub_size"]
            require(0 < size <= 64 and (address - section["address"]) % size == 0 and
                    address + size <= section["address"] + section["size"], "bad exact stub entry")
            start, end, derivation = address, address + size, "S_SYMBOL_STUBS_reserved2_exact_entry"
        else:
            starts = image["function_starts"]
            index = bisect.bisect_right(starts, address) - 1
            require(0 <= index < len(starts) - 1, "no exact adjacent function-start extent")
            start, end, derivation = starts[index], starts[index + 1], "LC_FUNCTION_STARTS_adjacent"
        require(0 < end - start <= self.output.limits["function_bytes"], "function byte cap")
        return image, start, end, derivation, self.vmread(start, end - start)

    def pointer(self, address):
        mapping = self.mapping(address, 8)
        require(mapping["slide_size"] >= 24, "pointer has no cache slide metadata")
        version = struct.unpack("<I", self.read(mapping["path"], mapping["slide_offset"], 4))[0]
        require(version in (2, 3, 5), "unsupported pointer slide version")
        header_size = 40 if version == 2 else 24
        header = self.read(mapping["path"], mapping["slide_offset"], header_size)
        page_size = struct.unpack_from("<I", header, 4)[0]
        require(page_size in (4096, 16384), "unsupported fixup page")
        index = (address - mapping["address"]) // page_size
        key = (mapping["cache_uuid"], mapping["record_offset"], index)
        if key not in self.pages:
            require(len(self.pages) < self.output.limits["fixup_pages"], "fixup page budget")
            if version == 2:
                starts_offset, count, extras_offset, extras_count, mask, add = struct.unpack_from("<4I2Q", header, 8)
                require(mask and ((mask >> ((mask & -mask).bit_length() - 1)) + 1) &
                        (mask >> ((mask & -mask).bit_length() - 1)) == 0, "noncontiguous v2 delta mask")
            else:
                starts_offset, count = 24, struct.unpack_from("<I", header, 8)[0]
                add = struct.unpack_from("<Q", header, 16)[0]
            require(index < count and starts_offset + index * 2 + 2 <= mapping["slide_size"],
                    "page-start metadata outside mapping")
            start_record = self.read(mapping["path"], mapping["slide_offset"] + starts_offset + index * 2, 2)
            start = struct.unpack("<H", start_record)[0]
            extras = []
            if version == 2:
                require(start != 0x4000, "pointer page has no rebases")
                if start & 0x8000:
                    j = start & 0x3FFF
                    while True:
                        require(j < extras_count and len(extras) < page_size // 4 and
                                extras_offset + j * 2 + 2 <= mapping["slide_size"], "v2 extras bound")
                        value = self.read(mapping["path"], mapping["slide_offset"] + extras_offset + j * 2, 2)
                        entry = struct.unpack("<H", value)[0]
                        extras.append({"index": j, "raw_hex": value.hex(), "offset": (entry & 0x3FFF) * 4})
                        if entry & 0x8000:
                            break
                        j += 1
                    beginnings = [e["offset"] for e in extras]
                else:
                    require(start & 0xC000 == 0, "reserved v2 page flags")
                    beginnings = [start * 4]
            else:
                require(start != 0xFFFF, "pointer page has no rebases")
                beginnings = [start]
            raw_page = self.read(mapping["path"], mapping["file_offset"] + index * page_size, page_size)
            page_vm = mapping["address"] + index * page_size
            decoded = {}
            for begin in beginnings:
                cursor = begin
                while True:
                    require(cursor >= 0 and cursor + 8 <= page_size and cursor not in decoded,
                            "fixup chain invalid or overlapping")
                    raw = struct.unpack_from("<Q", raw_page, cursor)[0]
                    if version == 2:
                        shift = (mask & -mask).bit_length() - 1 - 2
                        require(shift >= 0, "invalid v2 delta shift")
                        delta = (raw & mask) >> shift
                        value = raw & (~mask & ((1 << 64) - 1))
                        target = value + add if value else 0
                    else:
                        item = decode_pointer(raw, version, add)
                        delta = item["next_units_8_bytes"] * 8
                        target = int(item["target_unslid"], 16)
                    decoded[cursor] = {"raw": hex(raw), "target": hex(target), "delta": delta}
                    if not delta:
                        break
                    cursor += delta
            stem = f"fixup-{len(self.pages):02d}"
            self.output.write(stem + ".page.bin", raw_page)
            self.output.write(stem + ".header.bin", header)
            self.output.write(stem + ".start.bin", start_record)
            details = {"mapping": {k: str(v) if isinstance(v, Path) else v for k, v in mapping.items()},
                       "page_index": index, "page_size": page_size, "page_vm": hex(page_vm),
                       "version": version, "value_add": hex(add), "extras": extras,
                       "chain": decoded, "kind": "offline-rebase-not-live-pointer"}
            self.output.json(stem + ".json", details)
            self.pages[key] = (stem, page_vm, decoded)
        stem, base, decoded = self.pages[key]
        require(address - base in decoded, "requested pointer not in recorded fixup chain")
        item = decoded[address - base]
        return int(item["target"], 16), {"slot": hex(address), "page_evidence": stem, **item}


def arm_transfer(op, arg, regs, pc, start, machine_bytes):
    """Only proven forms preserve unrelated provenance; unknown writes discard all state."""
    previous = dict(regs)
    if op in ("nop", "cmp", "cmn", "tst"):
        return
    # This explicit write handling applies only to the fully matched forms below.
    for register in re.findall(r"\b[wx](\d+)\b", arg):
        regs.pop("x" + register, None)
    if op == "adrp":
        match = re.fullmatch(r"(x\d+),\s*(-?\d+)\s*;\s*(0x[0-9a-fA-F]+)", arg)
        if match:
            page = (pc & ~4095) + int(match[2]) * 4096
            require(page == int(match[3], 16), "decimal ADRP mismatch")
            if machine_bytes is not None:
                opcode = struct.unpack_from("<I", machine_bytes, pc - start)[0]
                require(adrp_page(opcode, pc) == page, "raw/text ADRP disagreement")
            regs[match[1]] = page
            return
    elif op == "add":
        match = re.fullmatch(r"(x\d+),\s*(x\d+),\s*#(0x[0-9a-fA-F]+|\d+)", arg)
        if match:
            if isinstance(previous.get(match[2]), int):
                regs[match[1]] = previous[match[2]] + int(match[3], 0)
            return
    elif op == "ldr":
        match = re.fullmatch(r"(x\d+),\s*\[(x\d+)(?:,\s*#(0x[0-9a-fA-F]+|\d+))?\]", arg)
        if match:
            if isinstance(previous.get(match[2]), int):
                regs[match[1]] = ("slot", previous[match[2]] + int(match[3] or "0", 0))
                # A non-writeback base remains available.
                if match[1] != match[2]:
                    regs[match[2]] = previous[match[2]]
            return
    # Operand spelling cannot enumerate implicit writes (for example LD64B's register range).
    regs.clear()


def unresolved_classification(branch):
    if branch.get("identified_slot") is not None or branch.get("ambiguous_states"):
        return "helper-incomplete"
    return "unclassified-indirect-owner-scope-review"


def edges(decoded, arch, start, end, pointer, machine_bytes=None, *, entry=None):
    """Explore bounded (PC, register-state) pairs; never select a first-path indirect target."""
    rows = {r["address"]: r for r in decoded}
    entry = start if entry is None else entry
    require(start <= entry < end and entry in rows, "entry is not a validated instruction boundary")
    ordered = sorted(rows)
    next_pc = {a: ordered[i + 1] if i + 1 < len(ordered) else end for i, a in enumerate(ordered)}
    queue, visited, states, result = [(entry, {})], set(), collections.defaultdict(set), []
    while queue:
        pc, regs = queue.pop()
        while pc in rows:
            state = tuple(sorted(regs.items()))
            if (pc, state) in visited:
                break
            require(len(visited) < 4096 and len(states[pc]) < 16,
                    "CFG state budget exhausted; target remains incomplete")
            visited.add((pc, state))
            states[pc].add(state)
            row, following = rows[pc], next_pc[pc]
            op, arg = row["mnemonic"], row["operands"]
            target, kind, conditional, slot = None, None, False, None
            if arch == "arm64":
                if op.startswith("ret"):
                    break
                if op in ("b", "bl") or op.startswith("b.") or op in ("cbz", "cbnz", "tbz", "tbnz"):
                    match = re.findall(r"0x[0-9a-fA-F]+", arg)
                    require(match, "unparsed direct ARM branch")
                    target, kind, conditional = int(match[-1], 16), "direct", op not in ("b", "bl")
                elif op in ("br", "blr", "braa", "brab", "blraa", "blrab", "braaz", "brabz", "blraaz", "blrabz"):
                    kind = "indirect"
                    value = regs.get(arg.split(",")[0].strip())
                    if isinstance(value, tuple):
                        slot = value[1]
                else:
                    arm_transfer(op, arg, regs, pc, start, machine_bytes)
            else:
                if op.startswith("ret"):
                    break
                if op.startswith("j") or op in ("call", "callq"):
                    conditional = op.startswith("j") and op not in ("jmp", "jmpq")
                    m = re.match(r"(0x[0-9a-fA-F]+)\b", arg)
                    if m:
                        target, kind = int(m[1], 16), "direct"
                    else:
                        kind = "indirect"
                        m = re.search(r"\*([+-]?(?:0x[0-9a-fA-F]+|\d+))\(%rip\)", arg)
                        if m:
                            after = following
                            if machine_bytes is not None:
                                offset = pc - start
                                prefix = 1 if machine_bytes[offset] in range(0x40, 0x50) else 0
                                require(machine_bytes[offset + prefix:offset + prefix + 2] in
                                        (b"\xff\x15", b"\xff\x25"), "unsupported raw RIP branch encoding")
                                displacement = struct.unpack_from("<i", machine_bytes, offset + prefix + 2)[0]
                                require(displacement == int(m[1], 0), "raw/text RIP displacement disagreement")
                                after = pc + prefix + 6
                            slot = after + int(m[1], 0)
            if kind:
                record = {**row, "kind": kind, "target": hex(target) if target is not None else None,
                          "identified_slot": hex(slot) if slot is not None else None}
                if slot is not None:
                    try:
                        target, evidence = pointer(slot)
                        record.update(target=hex(target), fixup=evidence)
                        if target == 0:
                            record["unresolved"] = "identified import slot has null target, not callable helper evidence"
                    except EvidenceError as error:
                        record["unresolved"] = str(error)
                elif kind == "indirect":
                    record["unresolved"] = "unclassified register-indirect semantics; owner scope review required"
                record["internal"] = target is not None and start <= target < end
                result.append(record)
                is_call = op in ("bl", "blr", "blraa", "blrab", "blraaz", "blrabz", "call", "callq")
                if record["internal"]:
                    require(target in rows, "internal branch is not a validated instruction boundary")
                    queue.append((target, dict(regs)))
                if not is_call and not conditional:
                    break
                if is_call:
                    regs.clear()
            if arch == "arm64":
                require(following == pc + 4, "undecoded reachable ARM instruction gap; state is incomplete")
            pc = following
    merged = []
    for address in sorted({record["address"] for record in result}):
        variants = [r for r in result if r["address"] == address]
        signatures = {(r["target"], r["identified_slot"], r.get("unresolved")) for r in variants}
        record = dict(variants[0])
        if len(signatures) > 1:
            record.update(target=None, internal=False, identified_slot=None,
                          unresolved="ambiguous incoming register/slot states; no unique target",
                          ambiguous_states=[{"target": t, "identified_slot": s, "unresolved": u}
                                            for t, s, u in sorted(signatures, key=repr)])
            record.pop("fixup", None)
        merged.append(record)
    return merged


def otool_instructions(text, container, start, end):
    lines = text.splitlines()
    require(len(lines) > 2 and lines[:2] == [str(container) + ":", "(__TEXT,__text) section"],
            "unexpected otool disassembly header")
    # These real ARM/x86 mnemonics must not be mistaken for hexadecimal encodings.
    hex_mnemonics = {"aaa", "aad", "adc", "adcb", "add", "addb", "b", "bfc", "bfadd",
                     "cadd", "daa", "dc", "dec", "decb", "fabd", "fadd", "fadda", "fcadd"}
    number = r"[+-]?(?:0x[0-9a-fA-F]+|\d+(?:\.\d+)?)"
    arm_register = r"(?:[bhsdqvwxzp]\d+(?:\.\w+)?(?:\[\d+\])?(?:/[mz])?|[wx]?sp|[wx]zr)"
    x86_register = r"%[a-zA-Z][a-zA-Z0-9]*(?:\(\d+\))?"
    index = rf"(?:{x86_register}(?:, *{x86_register}(?:, *[1248])?)?|, *{x86_register}(?:, *[1248])?)"
    memory = rf"(?:%[a-zA-Z]+:)?(?:(?:{number})?\({index}\)|{number})"
    shift = rf"(?:lsl|lsr|asr|ror|[us]xt[bwhx])(?: +#{number})?"
    operand = rf"(?:{memory}|{x86_register}|{arm_register}|[#$]{number}|{number})"
    comment = r"(?: *(?:;|##).*)?"
    operand_list = rf"{operand}(?:, *(?:{operand}|{shift}))*{comment}"
    for line in lines[2:]:
        row = re.fullmatch(r"\s*[0-9a-fA-F]{8,16}\t([a-zA-Z][a-zA-Z0-9_.]*)(?:(?: +|\t)([^\t]*))?",
                           line)
        require(row is not None, "unexpected otool disassembly row")
        mnemonic = row[1].lower()
        if re.fullmatch(r"[0-9a-f]+", mnemonic):
            operands = (row[2] or "").strip()
            require(mnemonic in hex_mnemonics, "unexpected otool encoding column")
            # Validate the whole field, not a numeric/register prefix of an encoding column.
            if mnemonic == "dc":
                valid_operands = re.fullmatch(rf"[a-zA-Z]\w*, *x(?:\d+|zr){comment}", operands)
            else:
                valid_operands = not operands or re.fullmatch(operand_list, operands)
            require(valid_operands, "unexpected otool encoding operands")
    decoded = instructions(text, start, end)
    require(len(decoded) == len(lines) - 2 and decoded[0]["address"] == start and
            all(left["address"] < right["address"] for left, right in zip(decoded, decoded[1:])),
            "unusable otool disassembly addresses")
    return decoded


def collect(primary, family, output, run):
    cache = CandidateCache(primary, output, family)
    root_images = {}
    for name in ("/usr/lib/system/libsystem_c.dylib", "/usr/lib/system/libsystem_kernel.dylib"):
        matches = [i for i in cache.images if i["name"] == name]
        require(len(matches) == 1, "required offline library missing/duplicate")
        root_images[name] = cache.macho(matches[0])
    output.json("candidate-identities.json", {name: {"uuid": m["uuid"], "cpu": m["cpu"],
        "subtype": m["subtype"], "subtype_unsigned": m.get("subtype_unsigned", m["subtype"] & 0xFFFFFFFF),
        "capability_bits": m.get("capability_bits", (m["subtype"] & 0xFFFFFFFF) >> 24),
        "header": m["header_file"], "native_observation": False}
        for name, m in root_images.items()})
    queue, root_records = collections.deque(), {}
    for symbol in run.bounds["roots"]:
        image = root_images["/usr/lib/system/libsystem_c.dylib" if symbol in ("fork", "vfork")
                            else "/usr/lib/system/libsystem_kernel.dylib"]
        matches = [s for s in image["symbols"] if s["name"] == "_" + symbol]
        require(len(matches) == 1 and "address" in matches[0] and matches[0]["flags"] & 3 == 0,
                "missing/ambiguous/nonregular root export " + symbol)
        queue.append((matches[0]["address"], 0, symbol, None))
        root_records[symbol] = matches[0]
    captures, missing, seen, bodies, total, analysis_count = [], [], set(), {}, 0, 0
    while queue:
        address, depth, root, parent = queue.popleft()
        if (address, root) in seen:
            continue
        seen.add((address, root))
        if depth > output.limits["helper_depth"] or analysis_count >= 128:
            missing.append({"address": hex(address), "reason": "entry/depth cap", "root": root})
            continue
        try:
            image, start, end, derivation, raw = cache.body(address)
            body_key = (image["uuid"], start, end)
            if body_key not in bodies:
                require(len(captures) < output.limits["functions"], "function extent cap")
                total += len(raw)
                require(total <= output.limits["code_bytes"], "total code byte cap")
                stem = f"body-{len(captures):02d}-{start:x}"
                output.write(stem + ".bin", raw)
                output.write(stem + ".macho", macho_function(start, raw, "arm64" if family == "arm64e" else "x86_64"))
                dis = run.command(stem + "-disassembly", [run.otool_path, "-tvV", output.root / (stem + ".macho")])
                output.write(stem + ".txt", dis["stdout"])
                decoded = otool_instructions(dis["stdout"], output.root / (stem + ".macho"), start, end)
                record = {"root": root, "parent": parent, "entry": hex(address), "start": hex(start),
                          "end": hex(end), "extent": derivation, "bytes": len(raw),
                          "sha256": hashlib.sha256(raw).hexdigest(), "image": image["name"],
                          "uuid": image["uuid"], "stem": stem, "depth": depth,
                          "symbols": [s for s in image["symbols"] if s.get("address") == start],
                          "analyses": []}
                bodies[body_key] = (record, decoded)
                captures.append(record)
            record, decoded = bodies[body_key]
            stem = record["stem"]
            require(hashlib.sha256(raw).hexdigest() == record["sha256"], "extent bytes changed between entries")
            analysis_count += 1
            branches = edges(decoded, "arm64" if family == "arm64e" else "x86_64",
                             start, end, cache.pointer, raw, entry=address)
            record["analyses"].append({"root": root, "entry": hex(address), "parent": parent,
                                      "depth": depth, "branches": branches})
            if len(record["analyses"]) == 1:
                record["branches"] = branches
            output.json(stem + ".json", record)
            for branch in branches:
                if branch.get("unresolved"):
                    missing.append({"function": stem, "root": root,
                        "classification": unresolved_classification(branch),
                        **branch})
                if not branch["internal"] and branch["target"] not in (None, "0x0"):
                    queue.append((int(branch["target"], 16), depth + 1, root, stem))
        except EvidenceError as error:
            missing.append({"root": root, "entry": hex(address), "reason": str(error)})
    report = {"candidate_only": True, "roots": root_records, "captures": captures, "missing": missing,
              "callbacks_exhaustively_closed": False, "semantic_adjudication_required": True,
              "helper_incomplete_count": sum(m.get("classification") != "callback-owner-scope-review"
                                            for m in missing),
              "entry_analyses": analysis_count, "code_bytes": total, "no_fallback_windows": True,
              "unknown_indirects_not_automatically_callback_exempt": True,
              "owner_may_classify_callback_scope_without_exhaustive_callback_audit": True}
    output.json("wrapper-evidence.json", report)
    require(all(any(a["entry"] == hex(r["address"]) and a["root"] == name
                    for c in captures for a in c["analyses"])
                for name, r in root_records.items()), "root entry evidence incomplete")
    return report
