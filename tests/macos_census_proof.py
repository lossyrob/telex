"""One bounded successor experiment; source mapping and product proof remain separate."""

import ast
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

from macos_group_proof import Lines, bounded_reap, capture, dump, run_probe


SOURCES = ("tests/macos_group_proof.c", "tests/macos_group_proof.py",
           "tests/macos_census_proof.py", ".github/workflows/macos-group-proof.yml")


def load_events(file):
    return [json.loads(line) for line in file.read_text(encoding="utf-8").splitlines()]


def audit_census(events):
    """Verify log causality independently of the C runtime assertions."""
    generation = 0
    leader = None
    admission_closed = False
    wait_return = None
    wait_code = None
    census = None
    positive = None
    counts = {"passes": 0, "rejected": 0, "positive": 0, "signals": 0, "injected": 0}
    for item in events:
        event = item["event"]
        if event == "prepared_unreleased_owned_group":
            leader = item["subject"]
        elif event == "external_root_admission_quiesced":
            admission_closed = item["subject"] == leader and item["value"] == 1
        elif event == "nonzero_group_signal":
            if item["phase"] != 0 or item["subject"] != leader:
                raise RuntimeError("unanchored signal or discovered-PID signal")
            positive = None
            counts["signals"] += 1
        elif event == "fresh_WNOWAIT_enter":
            if item["value"] != generation + 1 or item["phase"] != 0:
                raise RuntimeError("fresh observation generation missing")
            generation = item["value"]
            positive = wait_return = wait_code = census = None
            counts["passes"] += 1
        elif event == "fresh_WNOWAIT_return":
            if item["value"] != generation:
                raise RuntimeError("stale WNOWAIT generation")
            wait_return = item
        elif event == "fresh_WNOWAIT_code":
            wait_code = item
        elif event == "group_only_census_enter":
            if wait_return is None or wait_code is None or item["value"] != generation:
                raise RuntimeError("census was not preceded by fresh WNOWAIT")
        elif event == "group_only_census":
            if item["generation"] != generation or item["leader"] != leader or \
                    item["pgid"] != leader or item["phase"] != 0:
                raise RuntimeError("census generation/assigned-group mismatch")
            if item["int_size"] != item["pid_size"] or item["capacity"] != 2 * item["int_size"]:
                raise RuntimeError("census ABI mismatch")
            if item["byte_count"] == item["int_size"] and (
                    item["slot1_state"] != "prefill-not-read-after-call" or
                    item["slot1"] != -222222):
                raise RuntimeError("exact-one census loaded or misclassified second slot")
            if item["source"] != "native":
                counts["injected"] += 1
            census = item
        elif event == "completion_token_positive":
            if not (admission_closed and wait_return and wait_code and census and
                    item["value"] == generation and
                    wait_return["rc"] == 0 and wait_return["subject"] == leader and
                    wait_code["subject"] == leader and wait_code["value"] in (1, 2, 3) and
                    census["source"] == "native" and
                    census["byte_count"] == census["int_size"] and census["slot0"] == leader and
                    wait_return["time_ns"] < census["time_ns"] < item["time_ns"]):
                raise RuntimeError("positive token lacks fresh native full conjunction")
            positive = item
            counts["positive"] += 1
        elif event == "completion_token_rejected":
            positive = None
            counts["rejected"] += 1
        elif event == "signal_authority_consumed":
            if not (positive and positive["value"] == generation and
                    positive["nonzero_calls"] == item["nonzero_calls"] and
                    positive["subject"] == item["subject"] and
                    positive["time_ns"] < item["time_ns"]):
                raise RuntimeError("seal lacks an unconsumed fresh completion token")
        elif event == "libc_group_observation":
            if item["phase"] != 2 or item["value"] != 0:
                raise RuntimeError("post-reap observation was not read-only")
    return counts


def compile_probe(repo, output):
    for relative in SOURCES:
        shutil.copy2(repo / relative, output / Path(relative).name)
        if relative.endswith(".py"):
            ast.parse((repo / relative).read_text(encoding="utf-8"))
    binary = output / "macos-census-proof"
    result = capture("xcrun", "clang", "-std=c11", "-Wall", "-Wextra", "-Werror", "-O2",
                     str(repo / "tests" / "macos_group_proof.c"), "-lproc", "-o", str(binary))
    dump(output / "compile.json", result)
    if result["rc"] != 0:
        raise RuntimeError("native compilation failed before any fixture ran")
    return binary


def main():
    if sys.platform != "darwin" or os.getuid() == 0:
        raise RuntimeError("ordinary-user native macOS required")
    if len(sys.argv) != 3 or sys.argv[1] not in ("--compile-only", "--execute"):
        raise RuntimeError("usage: macos_census_proof.py MODE ABSOLUTE_EVIDENCE_DIRECTORY")
    output = Path(sys.argv[2])
    if not output.is_absolute():
        raise RuntimeError("absolute isolated evidence directory required")
    output.mkdir(parents=True, exist_ok=False)
    repo = Path(__file__).resolve().parent.parent
    summary = {
        "mode": sys.argv[1], "algorithm": "fresh-terminal-anchor-then-group-only-census",
        "production_containment_proven": False, "future_Rust_type_safety_proven": False,
        "source_native_conjunction_complete": False, "receipt_written": False,
        "cases": [], "negative_controls": [],
        "previous_one_shot_rejection": {
            "source": "810bd24fe7ca3415a4b32c19b762ed32a68d952b",
            "run": "https://github.com/lossyrob/telex/actions/runs/39904383638",
        },
        "authority_sha256": [
            "79549fc4736e7f7a4039e83271d3f8e535d09236686f4e4683acddafae0e1768",
            "fa8cade27a1653e2108cafaf1c7d7779c0b9ebcecb4d464dd3a680167e63004d",
        ],
        "limits": {"reuse_churn_children": 116000, "all_owned_child_creations_cap": 120000,
                   "max_simultaneously_live_fixture_children_including_guard_and_sentinel": 11,
                   "reuse_seconds": 580, "fixture_lifetime_seconds": 20,
                   "execute_seconds": 900, "negative_control_cases": 6,
                   "successor_overlap_cases": 48},
        "limitations": [
            "C assertions and log audit are not future Rust type-safety proof.",
            "Userspace syscall brackets cannot observe or stop P_REF_NEW or exec-shadow internals.",
            "Zero/error/permission/malformed/mismatch/stale observations are decision seams, not induced kernel faults.",
            "Kernel release to pinned-source mapping is a separate owner obligation.",
            "No product durable receipt, restart/recovery or Watcher integration is tested.",
            "Replacement groups are distinguished from old-attempt private-channel descendants.",
        ],
    }
    sentinel = None
    boot = None
    try:
        host = {name: capture(*command) for name, command in {
            "uname": ("uname", "-a"), "sw_vers": ("sw_vers",),
            "compiler": ("xcrun", "clang", "--version"),
            "sdk_version": ("xcrun", "--show-sdk-version"),
            "sdk_path": ("xcrun", "--show-sdk-path"),
            "boot": ("sysctl", "-n", "kern.bootsessionuuid"),
            "source": ("git", "rev-parse", "HEAD"),
        }.items()}
        host.update(architecture=platform.machine(), uid=os.getuid(), python=sys.version,
                    github_sha=os.environ.get("GITHUB_SHA"), run_id=os.environ.get("GITHUB_RUN_ID"),
                    run_attempt=os.environ.get("GITHUB_RUN_ATTEMPT"),
                    image_os=os.environ.get("ImageOS"), image_version=os.environ.get("ImageVersion"))
        dump(output / "host.json", host)
        if any(host[name]["rc"] != 0 for name in (
                "uname", "sw_vers", "compiler", "sdk_version", "sdk_path", "boot", "source")):
            raise RuntimeError("required host information unavailable")
        boot = host["boot"]["stdout"].strip()
        uuid.UUID(boot)
        if host["architecture"] != os.environ["EXPECTED_ARCH"]:
            raise RuntimeError("unexpected native architecture")
        if host["source"]["stdout"].strip() != os.environ["GITHUB_SHA"]:
            raise RuntimeError("source SHA mismatch")
        binary = compile_probe(repo, output)
        if sys.argv[1] == "--compile-only":
            summary["fixtures_started"] = False
            return 0
        deadline = time.monotonic() + 900
        with tempfile.TemporaryDirectory(prefix="telex-census-native-") as directory:
            root = Path(directory).resolve()
            isolation = {"TELEX_HOME": str(root / "home"), "TELEX_DB": str(root / "db.sqlite"),
                         "TELEX_INSTALL_ROOT": str(root / "install")}
            os.environ.update(isolation)
            (root / "home").mkdir()
            (root / "install").mkdir()
            dump(output / "isolation.json", isolation)
            abi = capture(str(binary), "native-abi")
            dump(output / "native-abi.json", abi)
            dump(output / "linked-libraries.json", capture("otool", "-L", str(binary)))
            if abi["rc"] != 0:
                raise RuntimeError("native libc/ABI identity unavailable")
            summary["native_abi_events"] = [json.loads(line) for line in abi["stdout"].splitlines()]
            sentinel_log = (output / "sentinel.jsonl").open("wb")
            sentinel = subprocess.Popen([str(binary), "sentinel"], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, bufsize=0)
            sentinel_stream = Lines(sentinel, sentinel_log)
            summary["fixtures_started"] = True
            # Reserve introspection/compiler subprocesses as well as the ABI helper and sentinel.
            summary["owned_child_creation_upper_bound"] = 64

            def challenge():
                sentinel.stdin.write(b"S")
                event = sentinel_stream.next(5)
                if event is None or event["event"] != "sentinel_response" or \
                        event["subject"] != sentinel.pid or event["value"] != ord("S"):
                    raise RuntimeError("unrelated sentinel failed fresh challenge")
                summary["sentinel_challenges"] = summary.get("sentinel_challenges", 0) + 1

            def run(name, arguments, *, negative=False, loss=None, reuse=False):
                if time.monotonic() >= deadline:
                    raise TimeoutError("finite overall experiment deadline")
                summary["active_case"] = name
                summary["owned_child_creation_upper_bound"] += 116002 if reuse else 12
                if summary["owned_child_creation_upper_bound"] > 120000:
                    raise RuntimeError("whole-experiment creation budget exhausted")
                dump(output / "summary.json", summary)
                challenge()
                item = run_probe(binary, output, name, arguments, boot,
                                 successor=not negative and loss is None, loss=loss,
                                 timeout=630 if reuse else 65)
                if not negative and loss is None:
                    item["census_audit"] = audit_census(load_events(output / item["events_file"]))
                item["classification"] = (
                    "expected-one-shot-rejection" if negative and item["counterexample"] else
                    "one-shot-counterexample-not-reproduced" if negative else
                    "successor-counterexample" if item["counterexample"] else
                    "successor-inconclusive" if item["inconclusive"] else
                    "guard-loss-no-receipt" if loss else "bounded-native-case-completed")
                summary["negative_controls" if negative else "cases"].append(item)
                summary.pop("active_case", None)
                challenge()
                dump(output / "summary.json", summary)
                return item

            for iteration in range(6):
                item = run(f"negative-one-shot-{iteration:02d}",
                           ["case", "race-overlap", str((0, 10, 25)[iteration % 3])], negative=True)
                if item["counterexample"]:
                    break
            summary["negative_control_reproduced"] = any(
                item["counterexample"] for item in summary["negative_controls"])
            for scenario in (
                    "ordinary", "multilevel", "ignored-signals", "leader-exit", "runtime-eof",
                    "zombie", "decision-faults", "active-root", "creator-fork", "creator-spawn",
                    "creator-vfork", "exec-transfer", "exec-overlap"):
                item = run(scenario, ["census-case", scenario])
                if item["counterexample"] or item["inconclusive"]:
                    summary["decisive_successor_case"] = scenario
                    return 42 if item["counterexample"] else 43
            for when in ("before", "after"):
                run("guard-loss-" + when, ["loss-guard", when], loss=when)
            run("reuse", ["census-reuse"], reuse=True)
            for scenario in ("race-kill-first", "race-born-first"):
                item = run(scenario, ["census-case", scenario])
                if item["counterexample"] or item["inconclusive"]:
                    summary["decisive_successor_case"] = scenario
                    return 42 if item["counterexample"] else 43
            for route in ("race-overlap", "race-spawn", "race-vfork", "race-exec"):
                for iteration in range(12):
                    name = f"{route}-{iteration:02d}"
                    skew = (0, 10, 25, 50, 100, 250)[iteration % 6]
                    item = run(name, ["census-case", route, str(skew)])
                    if item["counterexample"] or item["inconclusive"]:
                        summary["decisive_successor_case"] = name
                        return 42 if item["counterexample"] else 43
            summary["bounded_native_matrix_completed"] = True
            summary["routes_exercised"] = ["fork", "execve", "posix_spawn", "public-libc-vfork"]
            return 0
    except Exception as error:
        summary["error"] = repr(error)
        raise
    finally:
        if sentinel is not None:
            if not sentinel.stdin.closed:
                sentinel.stdin.close()
            try:
                while not sentinel_stream.eof:
                    sentinel_stream.next(5)
                summary["sentinel_cooperative_reap_rc"] = sentinel.wait(timeout=5)
                summary["sentinel_survived_all_completed_cases"] = (
                    summary["sentinel_cooperative_reap_rc"] == 0 and
                    any(e["event"] == "sentinel_cooperative_exit" for e in sentinel_stream.events))
            finally:
                bounded_reap(sentinel)
                sentinel.stdout.close()
                sentinel_log.close()
        summary["boot_after"] = capture("sysctl", "-n", "kern.bootsessionuuid")
        summary["boot_stable"] = (
            boot is not None and summary["boot_after"]["rc"] == 0 and
            summary["boot_after"]["stdout"].strip().lower() == boot.lower())
        dump(output / "summary.json", summary)
        dump(output / "sha256.json", {
            p.name: hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(output.iterdir()) if p.is_file() and p.name != "sha256.json"})


if __name__ == "__main__":
    sys.exit(main())
