"""Frozen two-job static component. Importing this module performs no network or image work."""

import collections
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import stat
import sys
import time
import urllib.request
import zipfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from macos_static_cache import EvidenceError, require, save
from macos_static_ridiff import Run, ridiff_output_bound
from cache import Output, choose_family, collect
import gate
import derived
from native import NativeBinding, bind_hash

BOUNDS = json.loads((HERE / "bounds.json").read_text())
NEW_SOURCES = ["bounds.json", "run.py", "cache.py", "gate.py", "native.py", "derived.py", "test_component.py", "README.md"]
DEPENDENCIES = ["macos_static_cache.py", "macos_static_ridiff.py", "macos_provenance_capture.py"]


def verified_metadata(raw, spec):
    require(len(raw) == spec["bytes"] and hashlib.sha256(raw).hexdigest() == spec["sha256"],
            "selected metadata identity mismatch: " + spec["member"])
    # plistlib's Expat parser rejects external entities. Hash validation precedes parsing.
    return plistlib.loads(raw)


def metadata_checks(top, inner, catalog, manifest, config):
    properties = top["MobileAssetProperties"]
    tag = Path(config["patch"]["member"]).name
    for data, version in ((properties, "OSVersion"), (inner, "ProductVersion")):
        require(data["Build"] == config["build"] and data[version] == config["version"],
                "inner build/version discrepancy")
        require(any(r["CryptexTag"] == tag for r in data["CryptexSizeInfo"]),
                "selected cryptex tag absent from metadata")
    require(inner["TargetUpdate"] == config["build"], "TargetUpdate discrepancy")
    selected = [r for r in catalog["Assets"] if r.get("__RelativePath") == config["zip"]["member"]]
    require(len(selected) == config["catalog_entries"], "asset catalog selection count discrepancy")
    for item in selected:
        require(item["Build"] == config["build"] and item["OSVersion"] == config["version"] and
                item["_DownloadSize"] == config["zip"]["bytes"] and item["_MeasurementAlgorithm"] == "SHA-1" and
                item["_Measurement"].hex() == config["zip"]["sha1"] and
                item["_Measurement-SHA256"].hex() == config["zip"]["sha256"] and
                any(r["CryptexTag"] == tag for r in item["CryptexSizeInfo"]),
                "catalog selected ZIP/build/tag measurement discrepancy")
    require(manifest["ProductBuildVersion"] == config["build"] and
            manifest["ProductVersion"] == config["version"], "BuildManifest discrepancy")
    rows = []
    for identity in manifest["BuildIdentities"]:
        info = identity["Info"]
        require(info["BuildNumber"] == config["build"], "manifest identity build discrepancy")
        components = []
        for name in ("KernelCache", "RestoreKernelCache", "x86,MacKernelCache", "Cryptex1,SystemOS"):
            if name in identity["Manifest"]:
                component = identity["Manifest"][name]
                components.append({"component": name, "path": component["Info"]["Path"],
                                   "digest_hex": component.get("Digest", b"").hex()})
        rows.append({"target": identity["Ap,TargetType"], "chip_id": identity["ApChipID"],
                     "variant": info["Variant"], "components": components})
    require(dict(collections.Counter(row["variant"] for row in rows)) == config["manifest_variants"],
            "manifest variant count discrepancy")
    return {"build": config["build"], "version": config["version"], "selected_tag": tag,
            "catalog_entries": len(selected), "manifest_identities": rows,
            "packaging_product_type": inner.get("ProductType"),
            "packaging_hardware_model": inner.get("HardwareModel"),
            "declared_cryptex_sizes": [r for r in inner["CryptexSizeInfo"] if r["CryptexTag"] == tag],
            "cryptex_size_units_inferred": False, "native_observation": False}


def selected_zip_member(archive, spec):
    matches = [info for info in archive.infolist() if info.filename == spec["member"]]
    require(len(matches) == 1, "missing/duplicate selected ZIP member")
    info = matches[0]
    mode = (info.external_attr >> 16) & 0xF000
    require(not info.is_dir() and mode in (0, stat.S_IFREG) and info.file_size == spec["bytes"],
            "selected ZIP member type/length discrepancy")
    return info


class ComponentRun(Run):
    def __init__(self, work, evidence, job):
        super().__init__(work, evidence)
        self.bounds, self.config, self.job = BOUNDS, BOUNDS["jobs"][job], job
        self.deadline = time.monotonic() + BOUNDS["limits"]["processing_seconds"]
        self.output = Output(evidence, BOUNDS["limits"])
        self.otool_path = None
        self.hdiutil_path = None
        self.native_binding = None
        self.derived_artifact = None
        self.receipt.update(component=BOUNDS["component"], job=job,
                            authority_comments=BOUNDS["authority_comments"],
                            authority="https://github.com/lossyrob/telex/issues/172#issuecomment-6400716200",
                            catalog_product=self.config["product"], input_url=self.config["url"],
                            limits=BOUNDS["limits"], new_offline_candidates_only=True,
                            freeze_required_before_launch=True, native_binding_verified=False)

    def command(self, name, args, **kwargs):
        if args and Path(str(args[0])).name == "ipsw":
            require(str(args[0]) == str(self.work / "ipsw"), "alternate candidate path prohibited")
            derived.before_use(self.work, self.bounds["tools"]["ipsw"], self.derived_artifact, self.deadline)
        require(args and str(args[0]) not in ("xcrun", "otool", "llvm-otool", "hdiutil"),
                "native components must use frozen absolute resolved paths")
        policy = self.bounds["native_binding"]
        is_otool = name.endswith("-disassembly") or str(args[0]) in (
            policy["otool_canonical_path"], policy["otool_nominal_path"])
        if is_otool:
            require(str(args[0]) == self.otool_path == policy["otool_canonical_path"],
                    "otool invocation must use fixed canonical C")
            require(self.native_binding is not None and self.native_binding.before is not None and
                    self.native_binding.before["native_tools"]["otool"]["path"] == policy["otool_canonical_path"],
                    "otool invocation lacks canonical C binding")
        if name != "RawImagePatch":
            result = super().command(name, args, **kwargs)
            if is_otool:
                require(result.get("argv") == [str(arg) for arg in args] and
                        result["argv"][0] == policy["otool_canonical_path"],
                        "recorded otool invocation differs from fixed canonical C")
            return result
        require(self.native_binding is not None and self.native_binding.before is not None,
                "decoder binding absent before RawImagePatch")
        require(set(kwargs) == {"timeout", "file_limit"}, "unexpected decoder invocation options")
        return self.native_binding.decode(args, kwargs["timeout"], kwargs["file_limit"])

    def attach(self, image, label):
        require(self.hdiutil_path is not None, "unbound hdiutil")
        mount = self.work / label
        mount.mkdir()
        entry = {"image": str(image), "mount": str(mount), "detached": False,
                 "hdiutil": self.hdiutil_path}
        self.receipt["mounts"].append(entry)
        result = self.command("attach-" + label, [
            self.hdiutil_path, "attach", "-readonly", "-nobrowse", "-noautoopen", "-noverify",
            "-mountpoint", mount, "-plist", image], timeout=180)
        details = plistlib.loads(result["stdout"].encode())
        mounted = [e for e in details.get("system-entities", []) if e.get("mount-point") == str(mount)]
        require(len(mounted) == 1, "attachment lacks exact private mountpoint")
        entry["device"] = mounted[0]["dev-entry"]
        return mount, entry

    def detach(self, entry):
        require(self.hdiutil_path is not None and entry["hdiutil"] == self.hdiutil_path,
                "cleanup hdiutil binding differs")
        self.command("detach-" + Path(entry["mount"]).name,
                     [self.hdiutil_path, "detach", entry.get("device", entry["mount"])],
                     timeout=90, cleanup=True)
        entry["detached"] = True

    def phase(self, name, cleanup=False):
        super().phase(name, cleanup)
        files = list(self.evidence.iterdir())
        require(len(files) <= BOUNDS["limits"]["artifact_files_per_job"] - 2 and
                sum(p.stat().st_size for p in files) <= BOUNDS["limits"]["artifact_bytes_per_job"] - 1024 * 1024,
                "artifact budget exhausted")

    def download(self, url, path, size, digest, name, range_header=None, sha1=None):
        self.phase("download-" + name)
        started = time.monotonic()
        request = urllib.request.Request(url, headers={"Accept-Encoding": "identity",
                                         **({"Range": range_header} if range_header else {})})
        with urllib.request.urlopen(request, timeout=30) as response, path.open("xb") as target:
            self.output.json(name + "-http.json", {"requested": url, "final": response.url,
                "status": response.status, "headers": dict(response.headers)})
            if range_header:
                c = self.config
                require(url == c["url"] and response.url == url and response.status == 206 and
                        response.headers.get("Content-Range") ==
                        f"bytes {c['range_offset']}-{c['range_offset'] + size - 1}/{c['package_bytes']}",
                        "authenticated Apple range mismatch")
            else:
                require(response.status == 200 and response.url.startswith("https://"),
                        "tool download response mismatch")

            def copy(consume):
                while chunk := response.read(8 * 1024 * 1024):
                    require(time.monotonic() - started <= BOUNDS["limits"]["download_seconds"],
                            "download deadline; no automatic retry")
                    consume(chunk)
            self.stream_hash(copy, size, digest, name, sha1, target)

    def pinned_tool(self, name):
        pin = BOUNDS["tools"][name]
        if name == "ipsw":
            record = {}
            self.receipt["derived_artifact_attempt"] = record
            binary, record = derived.acquire(self.work, pin, os.environ.get("GH_TOKEN"),
                                              min(self.deadline, time.monotonic() + BOUNDS["limits"]["download_seconds"]),
                                              record)
            self.output.json("derived-artifact-verification.json", record)
            self.derived_artifact = record
            self.receipt.setdefault("tools", {})["ipsw"] = {
                "artifact_id": pin["artifact_id"], "archive_sha256": pin["archive_sha256"],
                "binary_sha256": pin["binary_sha256"], "binary_bytes": pin["binary_bytes"],
                "version": pin["version"], "build": pin["build"], "expires_at": pin["expires_at"]}
            return binary
        binary = self.tool(name, pin["url"], pin["archive_bytes"], pin["archive_sha256"])
        observed = self.receipt["tools"][name]
        require(observed["binary_sha256"] == pin["binary_sha256"] and
                observed["binary_bytes"] == pin["binary_bytes"], "actual runner tool binary pin mismatch")
        return binary

    def metadata(self, raw, spec):
        require(spec["bytes"] <= BOUNDS["limits"]["metadata_input_bytes"], "metadata input cap")
        parsed = verified_metadata(raw, spec)
        self.receipt["ancestry"].append({"label": spec["member"], "bytes": len(raw),
            "sha256": spec["sha256"], "verified": True})
        # Large manifests are hashed and projected, never uploaded whole.
        if len(raw) <= 2 * 1024 * 1024:
            self.output.write("metadata-" + spec["member"].replace("/", "-"), raw)
        return parsed


def execute(run):
    c = run.config
    run.native_binding = NativeBinding(run)
    host = {name: run.command("host-" + name, args) for name, args in (
        ("uname", ["/usr/bin/uname", "-a"]), ("build", ["/usr/bin/sw_vers", "-buildVersion"]),
        ("product", ["/usr/bin/sw_vers", "-productVersion"]),
        ("source", ["/usr/bin/git", "-C", HERE.parents[1], "rev-parse", "HEAD"]))}
    require(host["source"]["stdout"].strip() == os.environ["GITHUB_SHA"], "checked-out source SHA mismatch")
    require(platform.machine() == BOUNDS["runner"]["architecture"] and
            host["build"]["stdout"].strip() == BOUNDS["runner"]["build"] and
            host["product"]["stdout"].strip() == BOUNDS["runner"]["product_version"],
            "decoder-host binding mismatch; no alternate host")
    host.update(image_os=os.environ.get("ImageOS"), image_version=os.environ.get("ImageVersion"),
                architecture=platform.machine(), role=BOUNDS["runner"]["role"])
    run.output.json("decoder-host.json", host)
    run.native_binding.resolve()
    run.capacity(c["dmg"]["bytes"] + c["patch"]["bytes"] + BOUNDS["limits"]["tool_stage_bytes"],
                 "before-any-transfer")
    ipsw, seven = run.pinned_tool("ipsw"), run.pinned_tool("7zz")
    version = run.command("ipsw-version", [ipsw, "version"])
    require(version["stdout"] == "Version: " + BOUNDS["tools"]["ipsw"]["version"] +
            ", BuildCommit: " + BOUNDS["tools"]["ipsw"]["build"] + "\n",
            "ipsw exact version/build discrepancy")
    run.command("7zip-version", [seven, "i"])
    dmg, patch = run.work / "SharedSupport.dmg", run.work / Path(c["patch"]["member"]).name
    end = c["range_offset"] + c["dmg"]["bytes"] - 1
    run.download(c["url"], dmg, c["dmg"]["bytes"], c["dmg"]["sha256"], "SharedSupport",
                 f"bytes={c['range_offset']}-{end}", c["dmg"]["sha1"])
    hfs = c["hfs"]
    run.stream_hash(lambda sink: run.command("stream-HFS", [seven, "x", "-tdmg", "-so", "-bsp0",
        dmg, hfs["member"]], timeout=900, stdout_sink=sink), hfs["bytes"], hfs["sha256"], "5.hfs")
    mount, attached = run.attach(dmg, "shared-support")
    selected = mount / c["zip"]["member"]
    require(selected.is_file() and not selected.is_symlink(), "selected asset ZIP not regular")
    with selected.open("rb") as source:
        run.stream_hash(source, c["zip"]["bytes"], c["zip"]["sha256"], "asset-ZIP", c["zip"]["sha1"])
    metadata = {}
    for spec in c["mounted_metadata"]:
        file = mount / spec["member"]
        require(file.is_file() and not file.is_symlink() and file.stat().st_size == spec["bytes"],
                "mounted metadata type/size discrepancy")
        metadata[spec["member"]] = run.metadata(file.read_bytes(), spec)
    run.capacity(c["patch"]["bytes"], "before-selected-patch")
    with zipfile.ZipFile(selected) as archive:
        for spec in c["zip_metadata"]:
            member = selected_zip_member(archive, spec)
            metadata[spec["member"]] = run.metadata(archive.read(member), spec)
        projection = metadata_checks(metadata["Info.plist"], metadata["AssetData/Info.plist"],
            metadata["com_apple_MobileAsset_MacSoftwareUpdate/com_apple_MobileAsset_MacSoftwareUpdate.xml"],
            metadata["AssetData/boot/BuildManifest.plist"], c)
        run.output.json("authenticated-build-projection.json", projection)
        member = selected_zip_member(archive, c["patch"])
        with archive.open(member) as source, patch.open("xb") as target:
            run.stream_hash(source, c["patch"]["bytes"], c["patch"]["sha256"],
                            c["patch"]["member"], destination=target)
    run.detach(attached)
    run.delete(dmg)
    bound = ridiff_output_bound(patch)
    run.output.json("RIDIFF-bound.json", bound)
    require(bound["conservative_output_file_bound"] <= BOUNDS["limits"]["image_bytes"], "image cap")
    run.capacity(bound["conservative_output_file_bound"], "before-RawImagePatch")
    run.command("RawImagePatch", [ipsw, "ota", "patch", "rsr", "--cryptex", patch],
                timeout=BOUNDS["limits"]["reconstruction_seconds"], file_limit=bound["conservative_output_file_bound"])
    image = Path(str(patch) + ".dmg")
    require(image.is_file() and image.stat().st_size <= bound["conservative_output_file_bound"],
            "reconstructed image missing or oversized")
    digest = hashlib.sha256()
    with image.open("rb") as source:
        while chunk := source.read(8 * 1024 * 1024):
            require(time.monotonic() < run.deadline, "image hash deadline")
            digest.update(chunk)
    run.receipt["reconstructed_image"] = {"bytes": image.stat().st_size, "sha256": digest.hexdigest()}
    run.delete(patch)
    mount, attached = run.attach(image, "selected-image")
    primary = choose_family(mount, c)
    run.output.json("family-selection.json", {"rule": c["family_rule"], "selected": str(primary),
        "checked_paths": c["cache_primary_candidates"] + c.get("ambiguity_checks", []),
        "candidate_only": True})
    report = collect(primary, c["cache_family"], run.output, run)
    run.receipt["wrapper_missing_count"] = len(report["missing"])
    run.receipt["helper_incomplete_count"] = report["helper_incomplete_count"]
    run.detach(attached)
    run.delete(image)
    run.native_binding.recheck_tools_after_semantics()
    pinned_after = {}
    for name in ("ipsw", "7zz"):
        pin = BOUNDS["tools"][name]
        observed = bind_hash(run.work / name, [pin["binary_bytes"]], run.deadline)
        require(observed["sha256"] == pin["binary_sha256"] and observed["bytes"] == pin["binary_bytes"],
                "pinned decoder changed before acceptance")
        pinned_after[name] = observed
    run.output.json("pinned-decoder-acceptance.json", pinned_after)
    derived.unexpired(BOUNDS["tools"]["ipsw"])
    run.receipt["native_binding_verified"] = True
    run.receipt["disposition"] = ("decision-needed-incomplete-wrapper-evidence" if report["helper_incomplete_count"]
                                  else "offline-candidate-evidence-collected-pending-adjudication")
    return 1 if report["helper_incomplete_count"] else 0


def main():
    require(len(sys.argv) == 4 and sys.argv[1] in BOUNDS["jobs"], "usage: run.py JOB ABS_WORK ABS_EVIDENCE")
    require(sys.platform == "darwin" and os.getuid() != 0, "ordinary-user macOS decoder required")
    job, work, evidence = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
    require(work.is_absolute() and evidence.is_absolute() and work != evidence, "private paths required")
    work.mkdir(parents=True, exist_ok=False)
    evidence.mkdir(parents=True, exist_ok=False)
    run = ComponentRun(work, evidence, job)
    result = 1
    try:
        run.output.json("freeze-check.json", gate.check(job))
        for name in NEW_SOURCES:
            run.output.write(name, (HERE / name).read_bytes())
        for name in DEPENDENCIES:
            run.output.write(name, (HERE.parent / name).read_bytes())
        run.output.write("macos-cross-static.yml",
                         (HERE.parents[1] / ".github" / "workflows" / "macos-cross-static.yml").read_bytes())
        os.environ.update(TELEX_HOME=str(work / "home"), TELEX_DB=str(work / "store.sqlite"),
                          TELEX_INSTALL_ROOT=str(work / "install"), TMPDIR=str(work))
        result = execute(run)
    except Exception as error:
        run.receipt.update(disposition="decision-needed", failure=repr(error),
                           failed_phase=run.receipt["phase"], semantic_retry_permitted=False)
    finally:
        for attachment in reversed(run.receipt["mounts"]):
            if not attachment["detached"]:
                try:
                    run.detach(attachment)
                except Exception as error:
                    run.receipt.setdefault("cleanup_errors", []).append(repr(error))
                    result = 1
        if all(a["detached"] for a in run.receipt["mounts"]):
            patch_name = Path(run.config["patch"]["member"]).name
            for name in ("SharedSupport.dmg", patch_name, patch_name + ".dmg"):
                file = work / name
                if file.is_file():
                    run.delete(file)
        save(evidence / "receipt.json", run.receipt)
        files = list(evidence.iterdir())
        require(len(files) < BOUNDS["limits"]["artifact_files_per_job"] and
                sum(p.stat().st_size for p in files) < BOUNDS["limits"]["artifact_bytes_per_job"] - 128 * 1024,
                "output gate failure; no artifact upload")
        save(evidence / "sha256.json", {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                                        for p in sorted(files)})
    return result


if __name__ == "__main__":
    sys.exit(main())
