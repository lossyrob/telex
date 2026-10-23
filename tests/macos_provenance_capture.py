"""Bounded #172 provenance capture. No detector, group-signal or churn workloads."""

import collections
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import uuid


EXPECTED = {
    "arm64": {
        "product": "26.6.2", "build": "25G83",
        "kernel": "xnu-12377.161.14~5/RELEASE_ARM64_VMAPPLE",
        "kernel_uuid": "B2455C4B-781C-3F38-8CA0-DD11CF317C15",
        "kdk_sha256": "3705c0e82528a955857428735b8c7c86a9518f3a7849a65332c26d714a6b1eea",
        "libsystem_c": "d77ceb62aff63cecba9b4f057fbe2eb5",
        "libsystem_kernel": "c6a4a4cb92e63bafaae0e8306259209a",
    },
    "x86_64": {
        "product": "15.7.9", "build": "24G830",
        "kernel": "xnu-11417.140.69.711.44~1/RELEASE_X86_64",
        "kernel_uuid": "F6A1B3D4-1EBB-3628-A1C5-89506C51DDEC",
        "kdk_sha256": "04a501246caf768356f4090a8fd662daf5bbc949993353526c66771f5ba1d8e4",
        "libsystem_c": "065490dcbbdb3311898d77232fb7409f",
        "libsystem_kernel": "7e438d0aac233adabae399fb51f2d898",
    },
}
ROOT_SYMBOLS = ("proc_listpids", "kill", "waitid", "posix_spawn", "execve", "fork", "vfork")
SOURCES = ("tests/macos_provenance_capture.c", "tests/macos_provenance_capture.py",
           ".github/workflows/macos-provenance-capture.yml")
MAX_FUNCTIONS = 64
MAX_DEPTH = 4


def dump(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def command(*args, timeout=40):
    result = subprocess.run(args, capture_output=True, text=True, timeout=timeout, check=False)
    return {"argv": args, "rc": result.returncode, "stdout": result.stdout, "stderr": result.stderr}


class Reader:
    def __init__(self, process, file):
        self.process = process
        self.file = file
        self.buffer = b""

    def next(self):
        deadline = time.monotonic() + 30
        while b"\n" not in self.buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.process.stdout], [], [], remaining)[0]:
                raise TimeoutError("capture protocol deadline")
            data = os.read(self.process.stdout.fileno(), 131072)
            if not data:
                raise RuntimeError("capture process ended before its response")
            self.buffer += data
            if len(self.buffer) > 150000:
                raise RuntimeError("capture response exceeded bound")
        line, self.buffer = self.buffer.split(b"\n", 1)
        self.file.write(line + b"\n")
        self.file.flush()
        return json.loads(line)

    def request(self, operation, address):
        self.process.stdin.write(f"{operation} {address:x}\n".encode())
        return self.next()


def macho_function(address, data, arch):
    """Relocatable instruction container at original runtime VA; not a system image."""
    cpu, subtype = (0x0100000C, 0) if arch == "arm64" else (0x01000007, 3)
    header = struct.pack("<IiiIIIII", 0xFEEDFACF, cpu, subtype, 1, 1, 152, 0, 0)
    segment = struct.pack("<II16sQQQQiiII", 0x19, 152, b"__TEXT", address,
                          len(data), 256, len(data), 5, 5, 1, 0)
    section = struct.pack("<16s16sQQIIIIIIII", b"__text", b"__TEXT", address, len(data),
                          256, 2 if arch == "arm64" else 0, 0, 0, 0x80000400, 0, 0, 0)
    return (header + segment + section).ljust(256, b"\0") + data


def instructions(text, start, end):
    result = []
    for line in text.splitlines():
        match = re.match(r"^\s*([0-9a-fA-F]{8,16})\s+(.+)$", line)
        if not match:
            continue
        address = int(match[1], 16)
        if not start <= address < end:
            continue
        operation = re.match(r"([a-zA-Z][a-zA-Z0-9_.]*)\s*(.*)", match[2].strip())
        if not operation:
            continue
        result.append({"address": address, "mnemonic": operation[1].lower(),
                       "operands": operation[2], "raw": line})
    return result


def branch_edges(items, arch, start, end, reader, pointer_cache):
    """Resolve explicit branches and simple import stubs; flag other indirections."""
    edges = []
    registers = {}

    def pointer(address):
        if address not in pointer_cache:
            if len(pointer_cache) >= 128:
                return None
            pointer_cache[address] = reader.request("pointer", address)
        result = pointer_cache[address]
        return int(result["code_value"], 16) if result["event"] == "pointer_read" else None

    for index, item in enumerate(items):
        mnemonic, operands, address = item["mnemonic"], item["operands"], item["address"]
        target = None
        resolution = None
        is_branch = False
        if arch == "arm64":
            if mnemonic == "adrp":
                match = re.match(r"(x\d+),\s*#?(0x[0-9a-fA-F]+)", operands)
                if match:
                    registers[match[1]] = int(match[2], 16) & ~4095
            elif mnemonic == "add":
                match = re.match(r"(x\d+),\s*(x\d+),\s*#(0x[0-9a-fA-F]+|\d+)", operands)
                if match:
                    registers[match[1]] = (registers[match[2]] + int(match[3], 0)
                                           if registers.get(match[2]) is not None else None)
            elif mnemonic == "ldr":
                match = re.match(r"(x\d+),\s*\[(x\d+)(?:,\s*#(0x[0-9a-fA-F]+|\d+))?\]", operands)
                if match:
                    base = registers.get(match[2])
                    registers[match[1]] = pointer(base + int(match[3] or "0", 0)) if base else None
            if mnemonic in ("b", "bl") or mnemonic.startswith("b.") or mnemonic in (
                    "cbz", "cbnz", "tbz", "tbnz"):
                is_branch = True
                numbers = re.findall(r"0x[0-9a-fA-F]+", operands)
                if numbers:
                    target, resolution = int(numbers[-1], 16), "explicit-PC-relative-branch"
            elif mnemonic in ("br", "blr", "braa", "brab", "blraa", "blrab",
                              "braaz", "brabz", "blraaz", "blrabz"):
                is_branch = True
                register = operands.split(",")[0].strip()
                target = registers.get(register)
                resolution = "simple-import-pointer-read" if target else None
        else:
            is_branch = mnemonic.startswith("j") or mnemonic in ("call", "callq")
            if is_branch:
                direct = re.match(r"(0x[0-9a-fA-F]+)\b", operands)
                indirect = re.search(r"\*([+-]?(?:0x[0-9a-fA-F]+|\d+))\(%rip\)", operands)
                if direct:
                    target, resolution = int(direct[1], 16), "explicit-PC-relative-branch"
                elif indirect:
                    following = items[index + 1]["address"] if index + 1 < len(items) else end
                    slot = following + int(indirect[1], 0)
                    target = pointer(slot)
                    resolution = "RIP-relative-import-pointer-read" if target else None
        if is_branch:
            edges.append({**item, "target": hex(target) if target is not None else None,
                          "resolution": resolution or "unresolved-indirect-or-unparsed-target",
                          "internal": target is not None and start <= target < end})
            # Do not carry straight-line assumptions through a branch or call.
            registers.clear()
        elif arch == "arm64" and mnemonic not in ("adrp", "add", "ldr", "nop"):
            destination = re.match(r"(x\d+)\b", operands)
            if destination:
                registers.pop(destination[1], None)
    return edges


def main():
    if sys.platform != "darwin" or os.getuid() == 0 or len(sys.argv) != 2:
        raise RuntimeError("ordinary-user native macOS and absolute evidence directory required")
    output = Path(sys.argv[1])
    if not output.is_absolute():
        raise RuntimeError("absolute evidence directory required")
    output.mkdir(parents=True, exist_ok=False)
    arch = os.environ["EXPECTED_ARCH"]
    expected = EXPECTED[arch]
    summary = {
        "purpose": "new read-only provenance; not retroactive evidence for run 39909079250",
        "expected": expected, "target_architecture": arch,
        "target_operations_invoked_by_capture": False,
        "previous_evidence_modified": False, "product_proof": False,
        "complete_wrapper_semantics_proven": False,
        "limits": {"functions": MAX_FUNCTIONS, "bytes_per_function": 32768,
                   "helper_depth": MAX_DEPTH, "pointer_reads": 128, "job_minutes": 10},
        "captures": [], "missing_evidence": [],
    }
    process = None
    raw = errors = None
    try:
        repo = Path(__file__).resolve().parent.parent
        for relative in SOURCES:
            shutil.copy2(repo / relative, output / Path(relative).name)
        host = {key: command(*args) for key, args in {
            "uname": ("uname", "-a"), "architecture": ("uname", "-m"),
            "product": ("sw_vers", "-productVersion"), "build": ("sw_vers", "-buildVersion"),
            "compiler": ("xcrun", "clang", "--version"), "sdk": ("xcrun", "--show-sdk-version"),
            "sdk_path": ("xcrun", "--show-sdk-path"), "source": ("git", "rev-parse", "HEAD"),
        }.items()}
        host.update(run_id=os.environ.get("GITHUB_RUN_ID"), run_attempt=os.environ.get("GITHUB_RUN_ATTEMPT"),
                    source_sha=os.environ.get("GITHUB_SHA"), image_os=os.environ.get("ImageOS"),
                    image_version=os.environ.get("ImageVersion"))
        dump(output / "host.json", host)
        mismatches = []
        for key, value in (("architecture", arch), ("product", expected["product"]),
                           ("build", expected["build"])):
            if host[key]["rc"] != 0 or host[key]["stdout"].strip() != value:
                mismatches.append(key)
        if expected["kernel"] not in host["uname"]["stdout"]:
            mismatches.append("kernel_build_variant")
        if host["source"]["stdout"].strip() != os.environ["GITHUB_SHA"]:
            raise RuntimeError("source commit mismatch")
        if os.environ.get("GITHUB_RUN_ATTEMPT") != "1":
            raise RuntimeError("rerun refused: one workflow attempt only")
        summary["host_mismatches"] = mismatches
        if mismatches:
            summary["disposition"] = "nonmatching-host-retained-no-implementation-capture"
            return 0
        binary = output / "provenance-reader"
        compiled = command("xcrun", "clang", "-std=c11", "-Wall", "-Wextra", "-Werror", "-O2",
                           str(repo / "tests" / "macos_provenance_capture.c"), "-o", str(binary),
                           timeout=60)
        dump(output / "compile.json", compiled)
        if compiled["rc"]:
            summary["disposition"] = "compile-failure-no-retry"
            return 1
        with tempfile.TemporaryDirectory(prefix="telex-provenance-readonly-") as directory:
            root = Path(directory).resolve()
            isolation = {"TELEX_HOME": str(root / "home"), "TELEX_DB": str(root / "db.sqlite"),
                         "TELEX_INSTALL_ROOT": str(root / "install")}
            os.environ.update(isolation)
            dump(output / "isolation.json", isolation)
            raw = (output / "capture-protocol.jsonl").open("wb")
            errors = (output / "capture.stderr").open("wb")
            process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                       stderr=errors, bufsize=0)
            reader = Reader(process, raw)
            metadata = []
            while True:
                item = reader.next()
                metadata.append(item)
                if item["event"] == "metadata_ready":
                    break
            dump(output / "live-metadata.json", metadata)
            sysctls = {item["name"]: item for item in metadata if item["event"] == "live_sysctl"}
            kernel = sysctls["kern.uuid"]
            kernel_uuid = None
            if kernel["rc"] == 0:
                try:
                    kernel_uuid = str(uuid.UUID(kernel["text"])).upper()
                except (ValueError, TypeError, AttributeError):
                    if kernel["returned_size"] == 16:
                        kernel_uuid = str(uuid.UUID(bytes=bytes.fromhex(kernel["raw_hex"]))).upper()
            summary["live_kernel_uuid"] = kernel_uuid
            summary["kernel_matches_owner_KDK_uuid"] = kernel_uuid == expected["kernel_uuid"]
            if kernel_uuid is None:
                summary["missing_evidence"].append("live kern.uuid unavailable or undecodable; no substitute claimed")
            symbols = {item["symbol"]: item for item in metadata if item["event"] == "symbol_metadata"}
            library_mismatches = []
            for name in ROOT_SYMBOLS:
                item = symbols.get(name)
                library = "libsystem_c" if name in ("fork", "vfork") else "libsystem_kernel"
                if item is None or item["image_uuid"] != expected[library] or \
                        not item["image"].endswith("/" + library + ".dylib"):
                    library_mismatches.append(name)
            summary["library_mismatches"] = library_mismatches
            if library_mismatches or kernel_uuid and kernel_uuid != expected["kernel_uuid"]:
                summary["disposition"] = "nonmatching-live-image-retained-no-implementation-capture"
                return 0
            queue = collections.deque((int(symbols[name]["address"], 16), 0, name, None)
                                      for name in ROOT_SYMBOLS)
            requested, ranges, pointer_cache = {}, [], {}
            deadline = time.monotonic() + 300
            while queue and len(requested) < MAX_FUNCTIONS and time.monotonic() < deadline:
                address, depth, root_name, parent = queue.popleft()
                if address in requested or any(start <= address < end for start, end in ranges):
                    continue
                capture = reader.request("capture", address)
                requested[address] = capture
                if capture["event"] != "function_capture":
                    summary["missing_evidence"].append({
                        "address": hex(address), "root": root_name, "parent": parent,
                        "failure": capture})
                    continue
                start, end = int(capture["start"], 16), int(capture["end"], 16)
                ranges.append((start, end))
                index = len(summary["captures"])
                stem = f"function-{index:02d}-{start:x}"
                data = bytes.fromhex(capture.pop("bytes_hex"))
                if len(data) != capture["byte_count"] or len(data) != end - start:
                    raise RuntimeError("captured byte count mismatch")
                (output / (stem + ".bin")).write_bytes(data)
                container = output / (stem + ".macho")
                container.write_bytes(macho_function(start, data, arch))
                disassembly = command("xcrun", "otool", "-tvV", str(container))
                dump(output / (stem + ".disassembly.json"), disassembly)
                (output / (stem + ".disassembly.txt")).write_text(
                    disassembly["stdout"], encoding="utf-8")
                decoded = instructions(disassembly["stdout"], start, end)
                edges = branch_edges(decoded, arch, start, end, reader, pointer_cache)
                capture.update(root=root_name, parent=parent, depth=depth,
                               stem=stem, sha256=hashlib.sha256(data).hexdigest(),
                               disassembler_rc=disassembly["rc"],
                               decoded_instructions=len(decoded), edges=edges)
                dump(output / (stem + ".json"), capture)
                summary["captures"].append(capture)
                if disassembly["rc"] != 0 or not decoded:
                    summary["missing_evidence"].append({"function": stem, "reason": "disassembly unavailable"})
                if not capture["full_derived_extent_captured"]:
                    summary["missing_evidence"].append({"function": stem, "reason": "complete extent unavailable"})
                for edge in edges:
                    if edge["internal"]:
                        continue
                    if edge["target"] and depth < MAX_DEPTH:
                        queue.append((int(edge["target"], 16), depth + 1, root_name, stem))
            summary["pointer_reads"] = pointer_cache
            for capture in summary["captures"]:
                for edge in capture["edges"]:
                    target = int(edge["target"], 16) if edge["target"] else None
                    if not edge["internal"] and (target is None or not any(
                            start <= target < end for start, end in ranges)):
                        summary["missing_evidence"].append({
                            "function": capture["stem"], "branch": edge,
                            "reason": "unresolved indirect branch or target outside bounded captured graph"})
            summary["pending_targets_at_bound"] = [
                {"address": hex(address), "depth": depth, "root": root_name, "parent": parent}
                for address, depth, root_name, parent in queue
                if not any(start <= address < end for start, end in ranges)]
            summary["roots"] = {}
            for name, item in symbols.items():
                address = int(item["address"], 16)
                covered = [c["stem"] for c in summary["captures"]
                           if int(c["start"], 16) <= address < int(c["end"], 16)]
                summary["roots"][name] = {"metadata": item, "captured_function": covered}
                if not covered:
                    summary["missing_evidence"].append({"root": name, "reason": "root body not captured"})
            summary["disposition"] = "matching-build-and-images-bounded-capture" if kernel_uuid else \
                "matching-build-and-libraries-kernel-identity-incomplete"
            summary["limits_explanation"] = [
                "Adjacent LC_FUNCTION_STARTS entries bound bodies, possibly including alignment padding.",
                "Synthetic Mach-O files contain captured bytes at original runtime VAs for disassembly only.",
                "Resolved direct branches/simple import pointers are followed to a bounded depth/count.",
                "Other indirect calls, data-dependent targets, runtime patching and kernel semantics remain unproven.",
                "Helper images have their own recorded UUIDs; only seven root image identities were pre-gated.",
                "Capture never calls the seven target operations; compiler/disassembler/controller launches are tooling, not workloads.",
            ]
            return 0
    except Exception as error:
        summary["error"] = repr(error)
        summary["disposition"] = "capture-failure-retained-no-retry"
        raise
    finally:
        if process is not None:
            process.stdin.close()
            # EOF is cooperative. The reader's finite alarm is the last bound; no signals are sent.
            summary["reader_exit"] = process.wait(timeout=430)
            process.stdout.close()
            if summary["reader_exit"] != 0:
                summary["disposition"] = "capture-reader-failure-retained-no-retry"
                summary["error"] = f"capture reader exited {summary['reader_exit']}"
        if raw is not None:
            raw.close()
        if errors is not None:
            errors.close()
        dump(output / "summary.json", summary)
        dump(output / "sha256.json", {
            item.name: hashlib.sha256(item.read_bytes()).hexdigest()
            for item in sorted(output.iterdir()) if item.is_file() and item.name != "sha256.json"})
        if process is not None and summary["reader_exit"] != 0:
            raise RuntimeError(summary["error"])


if __name__ == "__main__":
    sys.exit(main())
