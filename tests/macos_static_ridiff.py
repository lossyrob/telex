"""One bounded static #172 run; never execute anything from the Apple images."""

import hashlib
import io
import json
import lzma
import os
from pathlib import Path
import platform
import plistlib
import resource
import selectors
import shutil
import signal
import struct
import subprocess
import sys
import tarfile
import time
import urllib.request
import zipfile

from macos_provenance_capture import macho_function
from macos_static_cache import EvidenceError, require, resolve, save, stub_receipt


APPLE = ("https://swcdn.apple.com/content/downloads/37/33/140-93587-A_GRFFH93NOL/"
         "f944yaqo1cjhh2m0kxrl0zhcpg9yb9qphv/InstallAssistant.pkg")
IPSW_COMMIT = "1f8b657858d7510219f59bf9ba183bf4c0a59104"
DMG_OFFSET, DMG_SIZE = 17953771, 18366669607
DMG_SHA1 = "65adfb4192ad9e8b6282f3ecc480cd3cafdabfef"
DMG_SHA256 = "9327c292fb8f804f17b7bcde542aa523d27014606bc3d74e8f3dc3b892399354"
HFS_SIZE, HFS_SHA256 = 19705679872, "e81000d15749a312825ad4e4cbca959ef44bfd89ca0adf3e9415a2dc61d1a0b8"
ZIP_SIZE, ZIP_SHA256 = 18356020461, "0b44a71bd6a721f5ddfb46f8dba39b09ee46bf9678a1c3100b978f512acfc07c"
ZIP_NAME = "60533a8ae8e47c23593a6965d64c1f00a6396cfe.zip"
PATCH_NAME = "AssetData/payloadv2/image_patches/cryptex-system-arm64e"
PATCH_SIZE, PATCH_SHA256 = 4456630130, "da9e630a2171ba404708ad92e87689e650c9027995637f76c482542cbeba0a85"
RESERVE = 2 * 1024**3
OUTPUT_LIMIT = 16 * 1024**2
LOG_LIMIT = 2 * 1024**2
SOURCES = ("tests/macos_static_ridiff.py", "tests/macos_static_cache.py",
           "tests/macos_static_cache_test.py", "tests/macos_provenance_capture.py",
           ".github/workflows/macos-static-ridiff.yml")


class Run:
    def __init__(self, work, evidence):
        self.work, self.evidence = work, evidence
        self.deadline = time.monotonic() + 50 * 60
        self.receipt = {"phase": "preflight", "disposition": "in-progress", "ancestry": [],
                        "mounts": [], "deleted_known_intermediates": [],
                        "source_sha": os.environ.get("GITHUB_SHA"),
                        "run_id": os.environ.get("GITHUB_RUN_ID"),
                        "attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
                        "authority": "https://github.com/lossyrob/telex/issues/172#issuecomment-6370104933",
                        "catalog_product": "140-93587", "input_url": APPLE,
                        "catalog_digest_scope": "compressed-XAR-TOC SHA1, not whole-file SHA1",
                        "prior_evidence_modified": False,
                        "product_authorization": False, "offline_not_live_pointer_evidence": True,
                        "limits": {"workflow_minutes": 55, "processing_minutes": 50,
                                   "artifact_bytes": OUTPUT_LIMIT, "per_command_log_bytes": LOG_LIMIT,
                                   "image_bytes": 32 * 1024**3, "reserve_bytes": RESERVE}}

    def phase(self, name, cleanup=False):
        self.receipt["phase"] = name
        save(self.evidence / "receipt.json", self.receipt)
        print(name, flush=True)
        require(cleanup or time.monotonic() < self.deadline, "finite processing deadline exhausted")

    def capacity(self, additional, label):
        usage = shutil.disk_usage(self.work)
        record = {"label": label, "free_bytes": usage.free, "total_bytes": usage.total,
                  "additional_bytes": additional, "reserve_bytes": RESERVE,
                  "required_free_bytes": additional + RESERVE}
        self.receipt.setdefault("capacity", []).append(record)
        self.phase("capacity-" + label)
        require(usage.free >= additional + RESERVE, "capacity failure at " + label)

    def command(self, name, args, timeout=120, stdout_sink=None, file_limit=None, cleanup=False):
        """Bound subprocess output and time. Never invoke a selected-image executable."""
        self.phase(name, cleanup=cleanup)
        started = time.monotonic()
        child_env = os.environ.copy()
        child_env["NO_COLOR"] = "1"

        def limits():
            resource.setrlimit(resource.RLIMIT_FSIZE, (file_limit or 32 * 1024**3,) * 2)

        process = subprocess.Popen(
            [str(arg) for arg in args], cwd=self.work, env=child_env,
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            preexec_fn=limits)
        poller = selectors.DefaultSelector()
        poller.register(process.stdout, selectors.EVENT_READ, "stdout")
        poller.register(process.stderr, selectors.EVENT_READ, "stderr")
        data = {"stdout": bytearray(), "stderr": bytearray()}
        failure = None
        try:
            while poller.get_map():
                if time.monotonic() - started > timeout or (not cleanup and time.monotonic() > self.deadline):
                    raise EvidenceError(name + ": finite command deadline")
                for key, _ in poller.select(0.5):
                    chunk = os.read(key.fileobj.fileno(), 1024 * 1024)
                    if not chunk:
                        poller.unregister(key.fileobj)
                        continue
                    stream = key.data
                    if stream == "stdout" and stdout_sink:
                        stdout_sink(chunk)
                    else:
                        data[stream].extend(chunk)
                        require(len(data[stream]) <= LOG_LIMIT, name + ": output bound exceeded")
            rc = process.wait(timeout=5)
        except BaseException as error:
            failure = str(error)
            # Only this still-owned, unreaped parser child is terminated on a safety bound.
            process.kill()
            rc = process.wait(timeout=10)
        finally:
            poller.close()
            process.stdout.close()
            process.stderr.close()
        report = {"argv": [str(a) for a in args], "rc": rc, "elapsed_seconds": time.monotonic() - started,
                  "stdout": data["stdout"].decode("utf-8", errors="replace"),
                  "stderr": data["stderr"].decode("utf-8", errors="replace"), "failure": failure}
        save(self.evidence / (name + ".json"), report)
        require(not failure and rc == 0, name + ": " + (failure or "nonzero exit"))
        return report

    def stream_hash(self, source, expected_size, sha256, label, sha1=None, destination=None):
        h256, h1, count = hashlib.sha256(), hashlib.sha1(), 0

        def consume(chunk):
            nonlocal count
            count += len(chunk)
            require(count <= expected_size, label + ": byte limit exceeded")
            require(time.monotonic() < self.deadline, "processing deadline during hashing")
            h256.update(chunk)
            h1.update(chunk)
            if destination is not None:
                destination.write(chunk)

        if callable(source):
            source(consume)
        else:
            while chunk := source.read(8 * 1024 * 1024):
                consume(chunk)
        receipt = {"label": label, "bytes": count, "sha256": h256.hexdigest(), "sha1": h1.hexdigest(),
                   "expected_size": expected_size, "expected_sha256": sha256, "expected_sha1": sha1}
        self.receipt["ancestry"].append(receipt)
        self.phase("hashed-" + label)
        require(count == expected_size and receipt["sha256"] == sha256 and
                (sha1 is None or receipt["sha1"] == sha1), label + ": identity mismatch")
        receipt["verified"] = True
        return receipt

    def download(self, url, path, size, digest, name, range_header=None, sha1=None):
        self.phase("download-" + name)
        request = urllib.request.Request(url, headers={"Accept-Encoding": "identity",
                                         **({"Range": range_header} if range_header else {})})
        with urllib.request.urlopen(request, timeout=60) as response, path.open("xb") as output:
            headers = dict(response.headers.items())
            save(self.evidence / (name + "-http.json"), {"requested": url, "final": response.url,
                                                       "status": response.status, "headers": headers})
            if range_header:
                require(response.url == APPLE and response.status == 206 and
                        response.headers.get("Content-Range") ==
                        f"bytes {DMG_OFFSET}-{DMG_OFFSET + DMG_SIZE - 1}/18384624402",
                        "Apple byte-range response mismatch")
            else:
                require(response.status == 200 and response.url.startswith("https://"),
                        "tool response mismatch")
            self.stream_hash(response, size, digest, name, sha1, output)

    def tool(self, name, url, size, digest):
        archive = self.work / (name + ".archive")
        self.download(url, archive, size, digest, name + "-release")
        with tarfile.open(archive) as tar:
            candidates = [m for m in tar.getmembers() if Path(m.name).name == name and m.isfile()]
            require(len(candidates) == 1 and candidates[0].size < 200 * 1024**2,
                    "ambiguous or oversized tool binary")
            member = candidates[0]
            require(member.name == name, "tool archive binary has unexpected path")
            tar.extract(member, self.work, filter="data")
        binary = self.work / name
        require(os.access(binary, os.X_OK), "portable tool executable mode missing")
        binary_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
        self.receipt.setdefault("tools", {})[name] = {
            "url": url, "archive_sha256": digest, "binary_sha256": binary_hash,
            "binary_bytes": binary.stat().st_size}
        archive.unlink()
        return binary

    def attach(self, image, label):
        mount = self.work / label
        mount.mkdir()
        # Record the exact recovery target before hdiutil so a partial attach is not lost.
        entry = {"image": str(image), "mount": str(mount), "detached": False}
        self.receipt["mounts"].append(entry)
        result = self.command("attach-" + label, [
            "hdiutil", "attach", "-readonly", "-nobrowse", "-noautoopen", "-noverify",
            "-mountpoint", mount, "-plist", image], timeout=180)
        attachment = plistlib.loads(result["stdout"].encode())
        mounted = [e for e in attachment.get("system-entities", []) if e.get("mount-point") == str(mount)]
        require(len(mounted) == 1, "read-only attachment lacks exact requested mountpoint")
        entry["device"] = mounted[0]["dev-entry"]
        return mount, entry

    def detach(self, entry):
        self.command("detach-" + Path(entry["mount"]).name,
                     ["hdiutil", "detach", entry.get("device", entry["mount"])],
                     timeout=90, cleanup=True)
        entry["detached"] = True

    def delete(self, path):
        require(path.parent == self.work and path.is_file() and not path.is_symlink(),
                "cleanup path is not a known private intermediate")
        self.receipt["deleted_known_intermediates"].append({"path": str(path), "bytes": path.stat().st_size})
        path.unlink()


def ridiff_output_bound(patch):
    with patch.open("rb") as stream:
        header = stream.read(62)
        require(header[:8] == b"RIDIFF10", "not exact RIDIFF10 input")
        variants = struct.unpack_from("<H", header, 8)[0]
        excess = struct.unpack_from("<I", header, 26)[0]
        metadata, controls = struct.unpack_from("<QQ", header, 30)
        require(variants == 1 and 0 <= metadata < controls and
                controls - metadata <= 8 * 1024**2, "RIDIFF metadata bound")
        stream.seek(metadata)
        encoded = stream.read(controls - metadata)
    require(encoded[:4] == b"pbzx", "RIDIFF metadata is not PBZX")
    cursor, plain = 12, bytearray()
    while cursor < len(encoded):
        require(cursor + 16 <= len(encoded), "short PBZX header")
        inflated, compressed = struct.unpack_from(">QQ", encoded, cursor)
        cursor += 16
        require(0 < compressed <= inflated <= 8 * 1024**2 and
                cursor + compressed <= len(encoded) and len(plain) + inflated <= 64 * 1024**2,
                "PBZX metadata bounds")
        block = encoded[cursor:cursor + compressed]
        cursor += compressed
        if compressed < inflated:
            decoder = lzma.LZMADecompressor(memlimit=128 * 1024**2)
            block = decoder.decompress(block, max_length=inflated + 1)
            require(decoder.eof and not decoder.unused_data, "unexpected PBZX XZ framing")
        require(len(block) == inflated, "PBZX length mismatch")
        plain.extend(block)
    require(len(plain) >= 64, "short RIDIFF decoded metadata")
    total, unknown, extents, forks = struct.unpack_from("<4Q", plain, 32)
    require(64 + extents * 16 <= len(plain), "RIDIFF extent array outside metadata")
    high = max((offset + size for offset, size in struct.iter_unpack(
        "<QQ", plain[64:64 + extents * 16])), default=0)
    bound = max(total, high) + excess + 1024**3
    require(0 < bound <= 32 * 1024**3, "reconstructed image exceeds declared bound")
    return {"magic": "RIDIFF10", "variants": variants, "metadata_offset": metadata,
            "controls_offset": controls, "metadata_sha256": hashlib.sha256(plain).hexdigest(),
            "declared_total_bytes": total, "extent_high_water": high,
            "excess_space": excess, "extent_count": extents, "fork_count": forks,
            "unknown_count": unknown, "conservative_output_file_bound": bound}


def execute(run):
    run.phase("record-host")
    host = {}
    for name, args in (("uname", ["uname", "-a"]), ("sw_vers", ["sw_vers"]),
                       ("df", ["df", "-k", run.work]), ("source", ["git", "-C", Path(__file__).parent.parent,
                                                                  "rev-parse", "HEAD"])):
        host[name] = run.command("host-" + name, args)
    host.update(image_os=os.environ.get("ImageOS"), image_version=os.environ.get("ImageVersion"),
                architecture=platform.machine())
    save(run.evidence / "host.json", host)
    require(platform.machine() == "arm64" and sys.platform == "darwin" and os.getuid() != 0,
            "ordinary-user native arm64 macOS runner required")
    require(host["source"]["stdout"].strip() == os.environ["GITHUB_SHA"], "source SHA mismatch")
    attempt = int(os.environ["GITHUB_RUN_ATTEMPT"])
    require(attempt in (1, 2), "more than one potential retry is forbidden")
    save(run.evidence / "stub-decimal-adrp-regression.json", stub_receipt())
    # Only a DMG and the selected 4.46 GB patch coexist. HFS and ZIP are streamed.
    run.capacity(DMG_SIZE + PATCH_SIZE + 256 * 1024**2, "before-tools-and-Apple-range")
    ipsw = run.tool(
        "ipsw", "https://github.com/blacktop/ipsw/releases/download/v3.1.739/ipsw_3.1.739_macOS_arm64.tar.gz",
        28498196, "c09e8eba02c332f5b50d15f94383e59dd35646d70f6ea9e78d185e9afda6abc2")
    seven = run.tool(
        "7zz", "https://github.com/ip7z/7zip/releases/download/26.05/7z2605-mac.tar.xz",
        1868068, "12ef6830317743cfd3f0ab1724bde43fcc270d8b6d8564f8c12c2a2b7f993570")
    version = run.command("ipsw-version", [ipsw, "version"])
    require("3.1.739" in version["stdout"] + version["stderr"] and
            IPSW_COMMIT[:8] in version["stdout"] + version["stderr"], "ipsw release/build mismatch")
    run.command("7zip-version", [seven, "i"])
    run.command("ipsw-ridiff-help", [ipsw, "ota", "patch", "rsr", "--help"])
    run.receipt["ipsw_source"] = IPSW_COMMIT
    run.receipt["raw_image_path"] = "ipsw ota patch rsr --cryptex -> pinned ridiff.RawImagePatch -> Apple's /usr/lib/libParallelCompression.dylib"
    dmg = run.work / "SharedSupport.dmg"
    run.capacity(DMG_SIZE + PATCH_SIZE, "before-Apple-range")
    run.download(APPLE, dmg, DMG_SIZE, DMG_SHA256, "SharedSupport",
                 f"bytes={DMG_OFFSET}-{DMG_OFFSET + DMG_SIZE - 1}", DMG_SHA1)
    run.command("selected-DMG-partitions", [seven, "l", "-slt", "-tdmg", dmg])
    run.stream_hash(
        lambda sink: run.command("stream-HFS-partition-5",
                                 [seven, "x", "-tdmg", "-so", "-bsp0", dmg, "5.hfs"],
                                 timeout=900, stdout_sink=sink),
        HFS_SIZE, HFS_SHA256, "5.hfs")
    mount, attachment = run.attach(dmg, "shared-support-mount")
    zip_path = mount / "com_apple_MobileAsset_MacSoftwareUpdate" / ZIP_NAME
    require(zip_path.is_file(), "selected authenticated asset ZIP missing")
    with zip_path.open("rb") as data:
        run.stream_hash(data, ZIP_SIZE, ZIP_SHA256, "asset-ZIP")
    patch = run.work / "cryptex-system-arm64e"
    run.capacity(PATCH_SIZE, "before-selected-arm64e-member")
    with zipfile.ZipFile(zip_path) as archive:
        for member, expected in (
            ("AssetData/Info.plist", "2c413816ba05814f4b5a04d0117eb2f7f57623dbd05c3e71ade78dd5bd2b3fb2"),
            ("Info.plist", "8659fdc115aa7e26a3bce2b7637eccf748c94cfb89712f0d28b76e78eded14a1")):
            info = archive.getinfo(member)
            require(info.file_size < 1024 * 1024, "plist byte bound")
            raw = archive.read(member)
            require(hashlib.sha256(raw).hexdigest() == expected, "inner build plist hash mismatch")
            parsed = plistlib.loads(raw)
            props = parsed if member.startswith("AssetData") else parsed["MobileAssetProperties"]
            require(props["Build"] == "25G83" and
                    props["ProductVersion" if member.startswith("AssetData") else "OSVersion"] == "26.6.2",
                    "inner build/version mismatch")
            if member.startswith("AssetData"):
                require(props["TargetUpdate"] == "25G83", "TargetUpdate mismatch")
            require(any(item["CryptexTag"] == "cryptex-system-arm64e"
                        for item in props["CryptexSizeInfo"]), "arm64e selection absent")
            (run.evidence / member.replace("/", "-")).write_bytes(raw)
            run.receipt["ancestry"].append({"label": member, "bytes": len(raw),
                                            "sha256": expected, "verified": True})
        require(archive.getinfo(PATCH_NAME).file_size == PATCH_SIZE, "patch member size mismatch")
        with archive.open(PATCH_NAME) as source, patch.open("xb") as output:
            run.stream_hash(source, PATCH_SIZE, PATCH_SHA256, PATCH_NAME, destination=output)
    run.detach(attachment)
    run.delete(dmg)
    bound = ridiff_output_bound(patch)
    save(run.evidence / "RIDIFF-header-and-capacity.json", bound)
    run.capacity(bound["conservative_output_file_bound"], "before-RawImagePatch")
    run.command("RawImagePatch-arm64e", [ipsw, "ota", "patch", "rsr", "--cryptex", patch],
                timeout=900, file_limit=bound["conservative_output_file_bound"])
    image = Path(str(patch) + ".dmg")
    require(image.is_file() and image.stat().st_size <= bound["conservative_output_file_bound"],
            "reconstructed image missing or oversized")
    image_hash = hashlib.sha256()
    with image.open("rb") as data:
        while chunk := data.read(8 * 1024**2):
            require(time.monotonic() < run.deadline, "image hash deadline")
            image_hash.update(chunk)
    run.receipt["reconstructed_image"] = {"bytes": image.stat().st_size,
                                         "sha256": image_hash.hexdigest(), "path": image.name}
    run.delete(patch)
    image_mount, image_attachment = run.attach(image, "arm64e-image-mount")
    selected = []
    for parent in ("System/Library/dyld", "System/Library/Caches/com.apple.dyld"):
        candidate = image_mount / parent / "dyld_shared_cache_arm64e"
        if candidate.is_file() and not candidate.is_symlink():
            selected.append(candidate)
    require(len(selected) == 1, "exact arm64e primary cache missing or ambiguous; no alternate family")
    run.phase("identity-gated-cache-fixup-resolution")
    target = resolve(selected[0], run.evidence)
    target_bytes = (run.evidence / "target.bin").read_bytes()
    (run.evidence / "target-disassembly-container.macho").write_bytes(
        macho_function(int(target["start"], 16), target_bytes, "arm64"))
    disassembly = run.command("target-disassembly", [
        "xcrun", "otool", "-tvV", run.evidence / "target-disassembly-container.macho"])
    (run.evidence / "target.disassembly.txt").write_text(disassembly["stdout"], encoding="utf-8")
    run.receipt["target"] = {"image": target["image"]["name"], "image_uuid": target["image"]["uuid"],
                              "symbols": target["symbols"], "address": target["start"],
                              "sha256": target["sha256"]}
    run.detach(image_attachment)
    run.delete(image)
    run.receipt["disposition"] = "static-fixup-resolved-pending-owner-adjudication"


def main():
    require(len(sys.argv) == 3, "usage: macos_static_ridiff.py ABSOLUTE_WORK ABSOLUTE_EVIDENCE")
    work, evidence = map(Path, sys.argv[1:])
    require(work.is_absolute() and evidence.is_absolute() and work != evidence, "private absolute paths required")
    work.mkdir(parents=True, exist_ok=False)
    evidence.mkdir(parents=True, exist_ok=False)
    run = Run(work, evidence)
    os.environ.update(TELEX_HOME=str(work / "telex-home"), TELEX_DB=str(work / "telex.sqlite"),
                      TELEX_INSTALL_ROOT=str(work / "telex-install"), TMPDIR=str(work))
    for relative in SOURCES:
        shutil.copy2(Path(__file__).resolve().parent.parent / relative, evidence / Path(relative).name)
    result = 0
    try:
        execute(run)
    except Exception as error:
        run.receipt["disposition"] = "decision-needed"
        run.receipt["failure"] = repr(error)
        run.receipt["failed_phase"] = run.receipt["phase"]
        result = 1
    finally:
        for entry in reversed(run.receipt["mounts"]):
            if not entry["detached"]:
                try:
                    run.detach(entry)
                except Exception as error:
                    run.receipt.setdefault("cleanup_errors", []).append(repr(error))
                    result = 1
        # Delete only explicitly named data files under this job-created private work directory.
        if all(m["detached"] for m in run.receipt["mounts"]):
            for name in ("SharedSupport.dmg", "cryptex-system-arm64e", "cryptex-system-arm64e.dmg"):
                candidate = work / name
                if candidate.is_file():
                    run.delete(candidate)
        save(evidence / "receipt.json", run.receipt)
        files = [p for p in evidence.iterdir() if p.is_file()]
        total = sum(p.stat().st_size for p in files)
        require(total <= OUTPUT_LIMIT, "bounded artifact limit exceeded; do not upload")
        save(evidence / "sha256.json", {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                                        for p in sorted(files)})
        print(json.dumps({"disposition": run.receipt["disposition"],
                          "phase": run.receipt["phase"], "artifact_bytes": total}), flush=True)
    return result


if __name__ == "__main__":
    sys.exit(main())
