"""Declared provider-host native decoder binding; no selected-image code is loaded."""

import ctypes
import hashlib
import os
from pathlib import Path
import re
import resource
import selectors
import struct
import subprocess
import time
import uuid

from macos_static_cache import EvidenceError, require


def macho_header(read_at, offset=0):
    header = read_at(offset, 32)
    require(len(header) == 32 and struct.unpack_from("<I", header)[0] == 0xFEEDFACF,
            "native component is not a supported Mach-O64")
    _, cpu, subtype, _, count, size, _, _ = struct.unpack("<IiiIIIII", header)
    require(count <= 4096 and size <= 1024 * 1024, "native Mach-O metadata bound")
    raw = header + read_at(offset + 32, size)
    require(len(raw) == size + 32, "native Mach-O metadata truncated")
    position, identity, versions, segments = 32, None, {}, []
    for _ in range(count):
        command, length = struct.unpack_from("<II", raw, position)
        require(length >= 8 and position + length <= len(raw), "invalid native load command")
        body = raw[position:position + length]
        if command == 0x1B:
            require(length >= 24 and identity is None, "invalid/duplicate native LC_UUID")
            identity = uuid.UUID(bytes=body[8:24]).hex
        elif command == 0xD:
            require(length >= 24, "invalid LC_ID_DYLIB")
            versions["current_version_packed"] = struct.unpack_from("<I", body, 16)[0]
            versions["compatibility_version_packed"] = struct.unpack_from("<I", body, 20)[0]
        elif command == 0x19:
            require(length >= 72, "invalid native segment")
            name, address, vm_size, file_offset, file_size = struct.unpack_from("<16s4Q", body, 8)
            segments.append({"name": name.rstrip(b"\0").decode(), "address": address,
                             "vm_size": vm_size, "file_offset": file_offset, "file_size": file_size})
        elif command == 0x2A:
            require(length >= 16, "invalid LC_SOURCE_VERSION")
            versions["source_version_packed"] = struct.unpack_from("<Q", body, 8)[0]
        position += length
    require(identity is not None, "native component LC_UUID missing")
    return {"cpu_raw": cpu, "subtype_raw": subtype, "uuid": identity,
            "header_sha256": hashlib.sha256(raw).hexdigest(), "header_hex": raw.hex(),
            "segments": segments, **versions}


def file_machos(path):
    with path.open("rb") as stream:
        def read_at(offset, count):
            stream.seek(offset)
            return stream.read(count)
        magic = read_at(0, 8)
        if magic[:4] == b"\xcf\xfa\xed\xfe":
            return [macho_header(read_at)]
        require(magic[:4] in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"),
                "native tool has unsupported file format")
        count = struct.unpack_from(">I", magic, 4)[0]
        require(0 < count <= 8, "native universal architecture count")
        width = 32 if magic[:4] == b"\xca\xfe\xba\xbf" else 20
        raw = read_at(8, count * width)
        require(len(raw) == count * width, "native universal table truncated")
        results = []
        for index in range(count):
            if width == 32:
                cpu, subtype, offset, size, _, _ = struct.unpack_from(">iiQQII", raw, index * width)
            else:
                cpu, subtype, offset, size, _ = struct.unpack_from(">iiIII", raw, index * width)
            require(offset + size <= path.stat().st_size, "native slice outside tool")
            item = macho_header(read_at, offset)
            require(item["cpu_raw"] == cpu and item["subtype_raw"] == subtype, "native slice identity mismatch")
            results.append({**item, "slice_offset": offset, "slice_bytes": size})
        return results


def bind_hash(path, budget, deadline):
    path = Path(path)
    require(path.is_absolute() and path.is_file(), "native backing component missing")
    resolved = path.resolve(strict=True)
    with resolved.open("rb") as stream:
        before = os.fstat(stream.fileno())
        require(before.st_size <= budget[0], "native backing hash byte budget")
        digest = hashlib.sha256()
        while chunk := stream.read(8 * 1024 * 1024):
            require(time.monotonic() < deadline, "native manifest hash deadline")
            digest.update(chunk)
        after = os.fstat(stream.fileno())
    current = resolved.stat()
    require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) ==
            (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns) ==
            (current.st_dev, current.st_ino, current.st_size, current.st_mtime_ns),
            "native file changed while hashing")
    budget[0] -= before.st_size
    return {"path": str(path), "resolved_path": str(resolved), "bytes": before.st_size,
            "sha256": digest.hexdigest(), "device": before.st_dev, "inode": before.st_ino}


def loaded_parallel(policy):
    """Inspect the actual provider library loaded by this controller, not an SDK/disk stub."""
    library = ctypes.CDLL(policy["library"], mode=os.RTLD_LOCAL | os.RTLD_NOW)
    provider = ctypes.CDLL(policy["query_provider"], mode=os.RTLD_LOCAL | os.RTLD_NOW)

    class DlInfo(ctypes.Structure):
        _fields_ = [("name", ctypes.c_char_p), ("base", ctypes.c_void_p),
                    ("symbol", ctypes.c_char_p), ("symbol_address", ctypes.c_void_p)]

    provider.dladdr.argtypes, provider.dladdr.restype = [ctypes.c_void_p, ctypes.POINTER(DlInfo)], ctypes.c_int
    symbol = ctypes.cast(library.RawImagePatch, ctypes.c_void_p).value
    info = DlInfo()
    require(provider.dladdr(symbol, ctypes.byref(info)) != 0 and info.base,
            "actual RawImagePatch image identity unavailable")
    require(info.name and info.name.decode() == policy["library"], "loaded decoder pathname mismatch")
    header = macho_header(lambda offset, size: ctypes.string_at(info.base + offset, size))
    require(header["cpu_raw"] == 0x100000C, "loaded decoder ABI is not ARM64")
    count = provider._dyld_image_count
    count.argtypes, count.restype = [], ctypes.c_uint32
    get_header = provider._dyld_get_image_header
    get_header.argtypes, get_header.restype = [ctypes.c_uint32], ctypes.c_void_p
    get_slide = provider._dyld_get_image_vmaddr_slide
    get_slide.argtypes, get_slide.restype = [ctypes.c_uint32], ctypes.c_ssize_t
    image_count = count()
    require(0 < image_count <= 4096, "provider-loaded-image count bound")
    slides = [get_slide(i) for i in range(image_count) if get_header(i) == info.base]
    require(len(slides) == 1, "loaded decoder slide not uniquely resolved")
    get_uuid = provider._dyld_get_shared_cache_uuid
    get_uuid.argtypes, get_uuid.restype = [ctypes.POINTER(ctypes.c_ubyte)], ctypes.c_bool
    cache_uuid = (ctypes.c_ubyte * 16)()
    require(get_uuid(cache_uuid), "actual process shared-cache UUID unavailable")
    return library, {"path": info.name.decode(), "loaded_header_address": hex(info.base),
                     "raw_image_patch_address": hex(symbol), "slide": hex(slides[0]),
                     "unslid_header_address": hex(info.base - slides[0]),
                     "shared_cache_uuid": bytes(cache_uuid).hex(), "image": header}


def backing_binding(loaded, policy, budget, deadline):
    """Match the actual shared cache and image header to exact allowed backing files."""
    locations = []
    for allowed in policy["cache_primary_paths"]:
        candidate = Path(allowed)
        try:
            candidate.lstat()
        except FileNotFoundError:
            continue
        except OSError as error:
            raise EvidenceError("native cache location unclassifiable") from error
        resolved = candidate.resolve(strict=True)
        require(resolved.is_file() and str(resolved.parent) in policy["cache_roots"],
                "native cache resolution outside frozen roots")
        identity = (resolved.stat().st_dev, resolved.stat().st_ino)
        if not any(entry[0] == identity for entry in locations):
            locations.append((identity, resolved))
    require(len(locations) == 1, "native backing cache missing/ambiguous; no search fallback")
    primary = locations[0][1]
    files, mappings, headers = {}, [], {}

    def load(path, expected_uuid=None):
        require(path.parent == primary.parent and not path.is_symlink() and path.is_file(),
                "native subcache missing or substituted")
        with path.open("rb") as stream:
            raw = stream.read(0x240)
            require(len(raw) == 0x240 and raw[:16].rstrip(b"\0 ").split() == [b"dyld_v1", b"arm64e"],
                    "native backing cache family mismatch")
            identity = raw[88:104].hex()
            require(expected_uuid is None or identity == expected_uuid, "native subcache UUID mismatch")
            offset, count = struct.unpack_from("<II", raw, 0x138)
            require(0 < count <= 128, "native cache mapping bound")
            stream.seek(offset)
            table = stream.read(count * 56)
            require(len(table) == count * 56, "native mapping table truncated")
            for n in range(count):
                address, size, file_offset = struct.unpack_from("<3Q", table, n * 56)
                require(file_offset + size <= path.stat().st_size, "native mapping outside backing file")
                mappings.append({"path": str(path), "uuid": identity, "address": address,
                                 "size": size, "file_offset": file_offset})
        headers[str(path)] = {"uuid": identity, "header_hex": raw.hex(), "mapping_table_hex": table.hex()}
        files[str(path)] = path
        return raw

    main = load(primary, loaded["shared_cache_uuid"])
    offset, count = struct.unpack_from("<II", main, 0x188)
    require(count <= 32, "native subcache count cap")
    with primary.open("rb") as stream:
        stream.seek(offset)
        table = stream.read(count * 56)
    require(len(table) == count * 56, "native subcache table truncated")
    for index in range(count):
        entry = table[index * 56:(index + 1) * 56]
        suffix = entry[24:56].split(b"\0")[0].decode()
        require(re.fullmatch(r"\.[A-Za-z0-9_.]+", suffix), "native subcache suffix invalid")
        load(primary.with_name(primary.name + suffix), entry[:16].hex())

    def containing(address):
        found = [m for m in mappings if m["address"] <= address < m["address"] + m["size"]]
        require(len(found) == 1, "native image backing mapping ambiguous/missing")
        return found[0]

    header_address = int(loaded["unslid_header_address"], 16)
    image_bytes = bytes.fromhex(loaded["image"]["header_hex"])
    mapping = containing(header_address)
    require(header_address + len(image_bytes) <= mapping["address"] + mapping["size"],
            "native image header crosses mapping")
    with Path(mapping["path"]).open("rb") as stream:
        stream.seek(mapping["file_offset"] + header_address - mapping["address"])
        stored = stream.read(len(image_bytes))
    require(stored == image_bytes, "loaded decoder header differs from bound cache bytes")
    relevant = {str(primary), mapping["path"]}
    coverage = []
    for segment in loaded["image"]["segments"]:
        cursor, end = segment["address"], segment["address"] + segment["file_size"]
        while cursor < end:
            mapped = containing(cursor)
            stop = min(end, mapped["address"] + mapped["size"])
            coverage.append({"segment": segment["name"], "start": hex(cursor), "end": hex(stop),
                             "cache_file": mapped["path"], "cache_uuid": mapped["uuid"]})
            relevant.add(mapped["path"])
            cursor = stop
    require(len(relevant) <= 16, "native relevant backing file count cap")
    hashes = {name: bind_hash(files[name], budget, deadline) for name in sorted(relevant)}
    return {"primary": str(primary), "actual_cache_uuid": loaded["shared_cache_uuid"],
            "declared_subcache_table_hex": table.hex(), "headers": headers,
            "loaded_header_matches_backing_bytes": True, "segment_coverage": coverage, "file_hashes": hashes}


def decode_trace(stderr, expected_path, expected_uuid, child_pid):
    expected_uuid = uuid.UUID(expected_uuid).hex
    expected_name = expected_path.rsplit("/", 1)[-1]
    expected_stem = expected_name.removesuffix(".dylib")
    uuid_pattern = r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"

    def related_path(candidate):
        name = candidate.rsplit("/", 1)[-1]
        return (expected_path.casefold() in candidate.casefold() or
                expected_path.casefold().startswith(candidate.casefold()) or
                bool(name) and (expected_name.casefold().startswith(name.casefold()) or
                                name.casefold().startswith(expected_name.casefold())))

    def selected_hint(line):
        folded = line.casefold()
        return (expected_stem.casefold() in folded or expected_uuid in folded or
                str(uuid.UUID(expected_uuid)) in folded)

    records, unrelated = [], []
    for line in stderr.splitlines():
        prefix = re.fullmatch(r"dyld\[(\d+)\]:\s*(.*)", line)
        if prefix is None:
            require(not line.startswith("dyld[") and not selected_hint(line),
                    "unclassifiable potentially relevant loader record")
            continue
        pid = int(prefix[1])
        match = re.fullmatch(r"<(" + uuid_pattern + r")>\s+(/\S*)\s*", prefix[2])
        if pid != child_pid:
            if match and (uuid.UUID(match[1]).hex == expected_uuid or related_path(match[2])):
                unrelated.append({"pid": pid, "path": match[2], "uuid": uuid.UUID(match[1]).hex,
                                  "line": line, "supplies_exact_child_evidence": False})
            elif selected_hint(line):
                unrelated.append({"pid": pid, "line": line, "unclassified": True,
                                  "supplies_exact_child_evidence": False})
            continue
        # Classify every exact-child dyld record before filtering for the selected image.
        require(match is not None, "incomplete or ambiguous exact-child loader record")
        identity, pathname = uuid.UUID(match[1]).hex, match[2]
        if identity != expected_uuid and not related_path(pathname):
            continue
        require(pathname == expected_path, "incomplete or mismatching selected-library pathname")
        require(identity == expected_uuid, "exact ipsw child loaded a different decoder image")
        records.append({"pid": pid, "path": pathname, "uuid": identity, "line": line})
    require(len(records) == 1, "exact child decoder load missing/duplicated; unrelated children cannot supply it")
    return {"exact_child": records, "unrelated_children": unrelated}


def equal_binding(before, after):
    require(before == after, "bound native component/developer/SDK/cache changed before acceptance")


class NativeBinding:
    def __init__(self, run):
        self.run = run
        self.policy = run.bounds["native_binding"]
        self.handles = []
        self.before = None
        self.resolution = None
        self.use_record = None
        self.snapshots = 0
        self.decoder_confirmed = False
        self.check_environment(set_values=True)

    def check_environment(self, set_values=False):
        policy = self.policy
        require(not any("DYLD" in key or key in ("LD_LIBRARY_PATH", "LD_PRELOAD", "SIMULATOR_ROOT",
                        "IPHONE_SIMULATOR_ROOT") for key in os.environ), "unapproved loader/search/insertion override")
        for key, value in (("DEVELOPER_DIR", policy["developer_dir"]),
                           ("TOOLCHAINS", policy["toolchain"]), ("SDKROOT", policy["sdk_path"])):
            require(not os.environ.get(key) or os.environ[key] == value, "developer/SDK environment override")
            if set_values:
                os.environ[key] = value
            else:
                require(os.environ.get(key) == value, "bound developer/SDK environment changed")

    def resolve(self):
        policy = self.policy
        self.check_environment()
        result = self.run.command("resolve-otool", [policy["xcrun"], "--sdk", "macosx", "--find", "otool"])
        require(result["stdout"].strip() == policy["otool"], "xcrun selected an unfrozen otool path")
        sdk = self.run.command("resolve-sdk", [policy["xcrun"], "--sdk", "macosx", "--show-sdk-path"])
        require(sdk["stdout"].strip() in (policy["sdk_path"], policy["sdk_canonical_path"]),
                "xcrun selected an unfrozen SDK")
        version = self.run.command("resolve-sdk-version", [policy["xcrun"], "--sdk", "macosx", "--show-sdk-version"])
        require(version["stdout"].strip() == policy["sdk_version"], "SDK version mismatch")
        require(str(Path(policy["sdk_path"]).resolve(strict=True)) == policy["sdk_canonical_path"],
                "SDK symlink resolution differs from frozen policy")
        for name in ("hdiutil", "xcrun", "otool"):
            require(str(Path(policy[name]).resolve(strict=True)) == policy[name],
                    "native executable resolves outside its allowed absolute path")
        xcrun_version = self.run.command("native-xcrun-version", [policy["xcrun"], "--version"])
        self.resolution = {"otool": result["stdout"].strip(), "sdk": sdk["stdout"].strip(),
                           "sdk_version": version["stdout"].strip(),
                           "otool_version_source": "LC_SOURCE_VERSION when present; see native_tools.otool.macho_slices",
                           "xcrun_version": xcrun_version["stdout"].strip()}
        self.run.otool_path = policy["otool"]
        self.run.hdiutil_path = policy["hdiutil"]
        self.before = self.snapshot()
        self.run.output.json("native-binding-before.json", self.before)
        return self.before

    def snapshot(self):
        policy, run = self.policy, self.run
        require(self.snapshots < policy["maximum_native_snapshots"], "native snapshot count bound")
        self.snapshots += 1
        budget = [policy["hash_bytes_per_snapshot"]]
        files = {}
        for name in ("hdiutil", "xcrun", "otool"):
            path = Path(policy[name])
            files[name] = {**bind_hash(path, budget, run.deadline), "macho_slices": file_machos(path)}
        sdk_files = {path: bind_hash(Path(path), budget, run.deadline)
                     for path in policy["developer_sdk_identity_files"]}
        handle, loaded = loaded_parallel(policy)
        self.handles.append(handle)
        backing = backing_binding(loaded, policy, budget, run.deadline)
        return {"source_sha": os.environ["GITHUB_SHA"], "job": run.job,
                "attempt": os.environ["GITHUB_RUN_ATTEMPT"], "paths": policy,
                "actual_resolution_and_versions": self.resolution,
                "native_tools": files, "developer_sdk_files": sdk_files,
                "loaded_parallel_compression": loaded, "backing_cache": backing,
                "hash_bytes": policy["hash_bytes_per_snapshot"] - budget[0],
                "trust_boundary": "declared provider decoder host and Apple toolchain; observed hashes are execution identity, not independent pre-run authentication"}

    def verify_use(self, result):
        require(self.before is not None and self.use_record is None, "decoder use binding state invalid")
        require(result["rc"] == 0 and result["argv"][0] == str(self.run.work / "ipsw") and
                result["argv"][1:5] == ["ota", "patch", "rsr", "--cryptex"],
                "decoder use did not come from the fixed pinned ipsw command")
        loaded = self.before["loaded_parallel_compression"]
        require(result["streams_complete"] and not result.get("failure"),
                "truncated or failed exact-child decoder observation")
        trace = decode_trace(result["stderr"], self.policy["library"], loaded["image"]["uuid"], result["pid"])
        self.use_record = {
            "source_sha": os.environ["GITHUB_SHA"], "job": self.run.job,
            "attempt": os.environ["GITHUB_RUN_ATTEMPT"], "actual_ipsw_load_events": trace,
            "exact_child_pid": result["pid"], "invocation_id": result["invocation_id"],
            "pretransfer_binding": "native-binding-before.json",
            "actual_component_uuid_matches_bound_cache_image": True,
            "pinned_decoder_command": result["argv"],
            "provider_loader_trace_trusted": True,
            "trace_observes_only": ["exact child PID", "loaded pathname", "loaded image UUID"],
            "trace_does_not_observe": ["slide", "cache hash", "function call", "PAC validation"],
            "not_an_independent_loader_attestation": True}
        self.run.output.json("native-decoder-use.json", self.use_record)

    def recheck(self):
        require(self.before is not None and self.use_record is not None, "acceptance lacks actual decoder use evidence")
        self.check_environment()
        require(str(Path(self.policy["sdk_path"]).resolve(strict=True)) == self.policy["sdk_canonical_path"],
                "bound SDK resolution changed")
        after = self.snapshot()
        self.run.output.json("native-binding-after.json", after)
        equal_binding(self.before, after)
        self.decoder_confirmed = True
        self.run.output.json("native-binding-acceptance.json", {
            "unchanged": True, "source_sha": os.environ["GITHUB_SHA"], "job": self.run.job,
            "attempt": os.environ["GITHUB_RUN_ATTEMPT"],
            "phase": "after exact decoder child, before using its output for semantic processing",
            "manifest": "native-binding-before.json", "rechecked": "native-binding-after.json"})

    def decoder_launch(self, args):
        """Freeze the exact invocation and justified resolution relation before spawning."""
        self.check_environment()
        require(self.before is not None and self.use_record is None, "decoder launch binding state invalid")
        config = self.run.config
        patch = self.run.work / Path(config["patch"]["member"]).name
        expected = [str(self.run.work / "ipsw"), "ota", "patch", "rsr", "--cryptex", str(patch)]
        require([str(arg) for arg in args] == expected, "alternate decoder route/arguments refused")
        require(any(row.get("label") == config["patch"]["member"] and row.get("verified") and
                    row.get("sha256") == config["patch"]["sha256"]
                    for row in self.run.receipt["ancestry"]), "selected input not authenticated before decoder")
        pin = self.run.bounds["tools"]["ipsw"]
        binary = Path(expected[0])
        identity = bind_hash(binary, [pin["binary_bytes"]], self.run.deadline)
        require(identity["sha256"] == pin["binary_sha256"] and identity["bytes"] == pin["binary_bytes"],
                "unchanged pinned ipsw identity missing before launch")
        slices = file_machos(binary)
        require(len(slices) == 1 and slices[0]["cpu_raw"] == 0x100000C,
                "pinned decoder effective architecture must be a single native ARM64 slice")
        environment = os.environ.copy()
        environment["DYLD_PRINT_LIBRARIES"] = "1"
        environment["NO_COLOR"] = "1"
        invocation = uuid.uuid4().hex
        record = {
            "invocation_id": invocation, "source_sha": os.environ["GITHUB_SHA"], "job": self.run.job,
            "attempt": os.environ["GITHUB_RUN_ATTEMPT"], "argv": expected,
            "pinned_ipsw_file": identity, "ipsw_macho": slices,
            "effective_resolution_environment": {k: environment[k] for k in
                ("DEVELOPER_DIR", "TOOLCHAINS", "SDKROOT", "DYLD_PRINT_LIBRARIES")},
            "other_loader_search_or_insertion_overrides": [],
            "relation": {
                "route": "unchanged pinned ipsw ota patch rsr --cryptex; pinned Darwin ridiff source dlopen absolute /usr/lib/libParallelCompression.dylib",
                "ipsw_build_commit": pin["build"],
                "architecture": "direct native ARM64 executable on fixed ARM64 provider host; no arch/translation wrapper",
                "selection_assumptions": "trusted unchanged provider shared-cache resolver, native ABI, fixed absolute library route, no loader search/insertion/root/cache overrides",
                "controller_cache_uuid": self.before["loaded_parallel_compression"]["shared_cache_uuid"],
                "controller_library_uuid": self.before["loaded_parallel_compression"]["image"]["uuid"],
                "backing_proof": "native-binding-before.json: actual controller image/header/segments match UUID-bound backing cache files and hashes",
                "child_confirmation_required": "exact owned child loader UUID/path plus unchanged component recheck before output semantic use",
                "not_claimed": "controller slide/cache hashes are not fields observed by the child trace; no call or PAC observation",
            },
        }
        self.run.output.json("native-decoder-launch.json", record)
        return expected, environment, invocation

    def decode(self, args, timeout, file_limit):
        """One exact owned child with complete bounded logs; no result advances before confirmation."""
        self.run.phase("predecoder-resolution-relation")
        args, environment, invocation = self.decoder_launch(args)
        def limits():
            resource.setrlimit(resource.RLIMIT_FSIZE, (file_limit, file_limit))
        started = time.monotonic()
        self.run.phase("RawImagePatch")
        poller = selectors.DefaultSelector()
        try:
            process = subprocess.Popen(args, cwd=self.run.work, env=environment, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, preexec_fn=limits)
        except BaseException:
            poller.close()
            raise
        streams, complete, failure = {"stdout": bytearray(), "stderr": bytearray()}, set(), None
        try:
            self.run.output.json("native-decoder-child.json", {
                "pid": process.pid, "invocation_id": invocation, "source_sha": os.environ["GITHUB_SHA"],
                "job": self.run.job, "attempt": os.environ["GITHUB_RUN_ATTEMPT"],
                "stdio_owner": "these exact Popen pipes, not a PID-discovery or global log source"})
            for stream, name in ((process.stdout, "stdout"), (process.stderr, "stderr")):
                poller.register(stream, selectors.EVENT_READ, name)
            while poller.get_map():
                require(time.monotonic() - started <= timeout and time.monotonic() < self.run.deadline,
                        "decoder command observation deadline")
                for key, _ in poller.select(0.5):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        complete.add(key.data)
                        poller.unregister(key.fileobj)
                        continue
                    require(len(streams[key.data]) + len(chunk) <= self.run.bounds["limits"]["per_command_log_bytes"],
                            "decoder log would truncate at bound")
                    streams[key.data].extend(chunk)
            code = process.wait(timeout=5)
        except BaseException as error:
            failure = str(error)
            process.kill()
            code = process.wait(timeout=10)
        finally:
            poller.close()
            process.stdout.close()
            process.stderr.close()
        result = {"pid": process.pid, "invocation_id": invocation, "argv": args, "rc": code,
                  "streams_complete": complete == {"stdout", "stderr"}, "failure": failure,
                  "elapsed_seconds": time.monotonic() - started}
        for name, raw in streams.items():
            self.run.output.write("native-decoder-" + name + ".bin", raw)
            try:
                result[name] = raw.decode("utf-8")
            except UnicodeDecodeError:
                result[name] = ""
                result["failure"] = "undecodable decoder log; raw bytes retained, no success"
        self.run.output.json("RawImagePatch.json", result)
        require(not result["failure"] and result["streams_complete"] and code == 0,
                "failed/incomplete exact-child decoding receipt; output must not be used")
        self.confirm_decoder_result(result)
        return result

    def confirm_decoder_result(self, result):
        self.verify_use(result)
        self.recheck()
        self.run.receipt["decoder_confirmed_before_semantic_processing"] = True

    def recheck_tools_after_semantics(self):
        """The decoder result was already confirmed; final tool checks cover subsequent hdiutil/otool use."""
        require(self.decoder_confirmed, "semantic acceptance lacks post-decoder confirmation")
        self.check_environment()
        budget = [self.policy["final_tool_hash_bytes"]]
        observed = {}
        for name, before in self.before["native_tools"].items():
            after = bind_hash(Path(self.policy[name]), budget, self.run.deadline)
            equal_binding({key: before[key] for key in after}, after)
            observed[name] = after
        for path, before in self.before["developer_sdk_files"].items():
            after = bind_hash(Path(path), budget, self.run.deadline)
            equal_binding(before, after)
            observed[path] = after
        self.run.output.json("native-semantic-tool-acceptance.json", {
            "unchanged": observed, "decoder_cache_confirmation": "native-binding-after.json",
            "decoder_cache_confirmation_phase": "before semantic processing, not a second child mapping observation"})
