"""Bounded native evidence for #165; never a product containment acceptance test."""

import hashlib
import json
import os
from pathlib import Path
import platform
import select
import shutil
import subprocess
import sys
import tempfile
import time
import uuid


def dump(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def capture(*command):
    result = subprocess.run(command, text=True, capture_output=True, timeout=20, check=False)
    return {"argv": command, "rc": result.returncode,
            "stdout": result.stdout, "stderr": result.stderr}


class Lines:
    def __init__(self, process, output):
        self.process = process
        self.output = output
        self.buffer = b""
        self.events = []
        self.eof = False

    def next(self, timeout):
        end = time.monotonic() + timeout
        while b"\n" not in self.buffer:
            remaining = end - time.monotonic()
            if remaining <= 0 or not select.select([self.process.stdout], [], [], remaining)[0]:
                raise TimeoutError("private event channel deadline")
            data = os.read(self.process.stdout.fileno(), 8192)
            if not data:
                self.eof = True
                if self.buffer:
                    raise RuntimeError("truncated JSON event")
                return None
            self.buffer += data
        line, self.buffer = self.buffer.split(b"\n", 1)
        event = json.loads(line)
        self.events.append(event)
        self.output.write(line + b"\n")
        self.output.flush()
        return event


def audit(events, boot):
    for event in events:
        if event["event"] == "boot_session" and event["uuid"].lower() != boot.lower():
            raise RuntimeError("boot UUID differs across private guard processes")
        if event["event"] == "nonzero_group_signal" and event["phase"] != 0:
            raise RuntimeError("candidate issued a nonzero signal after seal")
        if event["event"] == "libc_group_observation" and event["value"] != 0:
            raise RuntimeError("read-only observation was destructive")
    sealed = [e for e in events if e["event"] == "signal_authority_consumed"]
    reaped = [e for e in events if e["event"] == "leader_exactly_reaped"]
    if sealed:
        if len(sealed) != 1 or len(reaped) != 1 or sealed[0]["time_ns"] >= reaped[0]["time_ns"]:
            raise RuntimeError("seal/exact reap ordering failed")
        if any(e["nonzero_calls"] != sealed[0]["nonzero_calls"]
               for e in events if "nonzero_calls" in e and e["phase"] >= 1):
            raise RuntimeError("post-seal destructive syscall count changed")


def bounded_reap(process):
    if process.poll() is None:
        # This is our direct unreaped helper, never a discovered orphan PID/group.
        process.kill()
    process.wait(timeout=30)


def run_probe(binary, output, name, arguments, boot, loss=None, timeout=65, successor=False):
    cooperative = os.pipe() if loss else None
    process = None
    result = {"name": name, "argv": [str(binary), *arguments], "events_file": name + ".jsonl"}
    with (output / (name + ".jsonl")).open("wb") as events_file, \
            (output / (name + ".stderr")).open("wb") as errors:
        try:
            if loss:
                result["argv"].append(str(cooperative[0]))
            process = subprocess.Popen(
                result["argv"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors,
                pass_fds=(cooperative[0],) if loss else (), bufsize=0,
            )
            if loss:
                os.close(cooperative[0])
                cooperative = (None, cooperative[1])
            stream = Lines(process, events_file)
            if not loss and "runtime-eof" not in arguments:
                process.stdin.close()
            end = time.monotonic() + timeout
            while not stream.eof:
                event = stream.next(max(0.01, end - time.monotonic()))
                if event is None:
                    break
                if event["event"] == "awaiting_runtime_liveness_EOF":
                    process.stdin.close()
                if event["event"] == "guard_loss_barrier":
                    process.stdin.write(b"x")
                    process.stdin.close()
                    if process.wait(timeout=5) != 99:
                        raise RuntimeError("guard did not die at the requested barrier")
                    if loss == "after":
                        os.write(cooperative[1], b"e")
                    else:
                        os.close(cooperative[1])
                        cooperative = (None, None)
                if event["event"] == "loss_detector_exec_entered":
                    os.write(cooperative[1], b"Z")
                if event["event"] == "loss_live_response":
                    if event["value"] != ord("Z"):
                        raise RuntimeError("wrong post-guard-loss challenge")
                    os.close(cooperative[1])
                    cooperative = (None, None)
            result["rc"] = process.wait(timeout=5)
            audit(stream.events, boot)
            names = [e["event"] for e in stream.events]
            result["counterexample"] = any(name in names for name in (
                "LIVE_IN_GROUP_AFTER_SEAL", "ABSENCE_PRECEDED_OWNED_LIVE_RESPONSE"))
            if successor:
                result["counterexample"] |= any(name in names for name in (
                    "SUCCESSOR_TOKEN_OLD_ATTEMPT_RESPONSE", "UNSAFE_TOKEN_WITH_ACTIVE_FIXTURE",
                    "UNSAFE_TOKEN_WITH_RUNNING_SUCCESSOR", "FAULT_INJECTION_PERMITTED_SEAL",
                    "PRIOR_TOKEN_SURVIVED_REJECTED_RETRY", "PRIOR_TOKEN_SURVIVED_NEW_SIGNAL",
                    "SEAL_WITHOUT_FRESH_COMPLETION_TOKEN", "NONZERO_AFTER_SEAL",
                ))
            result["inconclusive"] = successor and result["rc"] == 43
            result["outcomes"] = [e for e in stream.events if e["event"] in {
                "candidate_outcome", "LIVE_IN_GROUP_AFTER_SEAL", "reuse_observed",
                "reuse_INCONCLUSIVE", "ACTUAL_PID_PGID_REUSE", "anchored_no_reuse_sample",
                "EPERM_not_success_source_gate_unresolved", "private_teardown_event_eof",
                "ABSENCE_PRECEDED_OWNED_LIVE_RESPONSE",
                "SUCCESSOR_TOKEN_OLD_ATTEMPT_RESPONSE", "completion_token_timeout_no_success",
                "completion_token_positive", "decision_seams_rejected_not_native_fault_injection",
            }]
            if loss:
                required = ("guard_loss_before_release_no_detector" if loss == "before"
                            else "loss_cooperative_exit")
                if result["rc"] != 99 or required not in names:
                    raise RuntimeError("guard-loss causal barrier or cooperative cleanup failed")
                if loss == "before" and "loss_detector_exec_entered" in names:
                    raise RuntimeError("detector executed without release")
                result["disposition"] = "no receipt; post-release remains unproven" if loss == "after" \
                    else "gate EOF prevented detector execution; no receipt"
            elif result["rc"] not in ((0, 42, 43) if successor else (0, 42)) and not (
                    successor and result["counterexample"]):
                raise RuntimeError(f"native helper failed with {result['rc']}")
            if not successor and (result["rc"] == 42) != result["counterexample"]:
                raise RuntimeError("counterexample exit code/evidence mismatch")
            return result
        finally:
            if cooperative:
                for fd in cooperative:
                    if fd is not None:
                        os.close(fd)
            if process is not None:
                bounded_reap(process)
                process.stdout.close()
                if not process.stdin.closed:
                    process.stdin.close()


def main():
    if sys.platform != "darwin" or os.getuid() == 0:
        raise RuntimeError("ordinary-user native macOS required")
    if len(sys.argv) != 2:
        raise RuntimeError("usage: macos_group_proof.py ABSOLUTE_EVIDENCE_DIRECTORY")
    output = Path(sys.argv[1])
    if not output.is_absolute():
        raise RuntimeError("evidence directory must be absolute")
    output.mkdir(parents=True, exist_ok=False)
    repo = Path(__file__).resolve().parent.parent
    summary = {
        "production_containment_proven": False,
        "source_native_conjunction_complete": False,
        "future_Rust_type_safety_proven": False,
        "receipt_written": False,
        "cases": [],
        "limits": {"churn_creations_including_target": 120000,
                   "simultaneous_churn_children": 8, "churn_seconds": 580,
                   "race_iterations": 120, "fixture_lifetime_seconds": 20},
        "limitations": [
            "Userspace barriers do not stop XNU inside pgrp_iterate or P_REF_NEW.",
            "C syscall audit rejects reconstructed authority; it is not Rust ownership enforcement.",
            "Native zombie-only EPERM alone does not authorize a source-gated continuation.",
            "No durable receipt/restart/product integration is implemented or proven.",
        ],
    }
    sentinel = None
    sentinel_log = None
    try:
        host = {key: capture(*command) for key, command in {
            "uname": ("uname", "-a"), "sw_vers": ("sw_vers",),
            "compiler": ("xcrun", "clang", "--version"),
            "sdk_version": ("xcrun", "--show-sdk-version"),
            "sdk_path": ("xcrun", "--show-sdk-path"),
            "boot": ("sysctl", "-n", "kern.bootsessionuuid"),
            "source": ("git", "rev-parse", "HEAD"),
        }.items()}
        host.update(architecture=platform.machine(), python=sys.version, uid=os.getuid(),
                    github_sha=os.environ.get("GITHUB_SHA"),
                    run_id=os.environ.get("GITHUB_RUN_ID"),
                    run_attempt=os.environ.get("GITHUB_RUN_ATTEMPT"),
                    image_os=os.environ.get("ImageOS"), image_version=os.environ.get("ImageVersion"))
        dump(output / "host.json", host)
        if any(host[key]["rc"] != 0 for key in (
                "uname", "sw_vers", "compiler", "sdk_version", "sdk_path", "boot", "source")):
            raise RuntimeError("required native host identity unavailable")
        boot = host["boot"]["stdout"].strip()
        uuid.UUID(boot)
        expected = os.environ.get("EXPECTED_ARCH")
        if expected and host["architecture"] != expected:
            raise RuntimeError(f"native architecture mismatch: {host['architecture']} != {expected}")
        if host["github_sha"] and host["source"]["stdout"].strip() != host["github_sha"]:
            raise RuntimeError("checkout source does not match the workflow SHA")
        binary = output / "macos-group-proof"
        compile_result = capture(
            "xcrun", "clang", "-std=c11", "-Wall", "-Wextra", "-Werror", "-O2",
            str(repo / "tests" / "macos_group_proof.c"), "-o", str(binary),
        )
        dump(output / "compile.json", compile_result)
        if compile_result["rc"] != 0:
            raise RuntimeError("native proof compilation failed")
        for relative in ("tests/macos_group_proof.c", "tests/macos_group_proof.py",
                         ".github/workflows/macos-group-proof.yml"):
            shutil.copy2(repo / relative, output / Path(relative).name)
        with tempfile.TemporaryDirectory(prefix="telex-native-proof-") as directory:
            root = Path(directory).resolve()
            isolation = {"TELEX_HOME": str(root / "home"), "TELEX_DB": str(root / "db.sqlite"),
                         "TELEX_INSTALL_ROOT": str(root / "install")}
            os.environ.update(isolation)
            (root / "home").mkdir()
            (root / "install").mkdir()
            dump(output / "isolation.json", isolation)
            sentinel_log = (output / "sentinel.jsonl").open("wb")
            sentinel = subprocess.Popen([str(binary), "sentinel"], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, bufsize=0)
            sentinel_stream = Lines(sentinel, sentinel_log)

            def challenge():
                sentinel.stdin.write(b"S")
                event = sentinel_stream.next(5)
                if event is None or event["event"] != "sentinel_response" or \
                        event["value"] != ord("S") or event["subject"] != sentinel.pid:
                    raise RuntimeError("unrelated owned sentinel failed fresh challenge")
                summary["sentinel_challenges"] = summary.get("sentinel_challenges", 0) + 1

            def run(name, args, **kwargs):
                challenge()
                item = run_probe(binary, output, name, args, boot, **kwargs)
                summary["cases"].append(item)
                challenge()
                dump(output / "summary.json", summary)
                return item["counterexample"]

            for case in ("ordinary", "multilevel", "ignored-signals", "leader-exit",
                         "runtime-eof", "zombie"):
                if run(case, ["case", case]):
                    summary["decisive_counterexample"] = case
                    return 42
            for when in ("before", "after"):
                run("guard-loss-" + when, ["loss-guard", when], loss=when)
            run("reuse", ["reuse"], timeout=630)
            for case in ("race-kill-first", "race-born-first"):
                if run(case, ["case", case]):
                    summary["decisive_counterexample"] = case
                    return 42
            for iteration in range(120):
                skew = (0, 10, 25, 50, 100, 250)[iteration % 6]
                case = f"race-{iteration:03d}-skew-{skew}"
                if run(case, ["case", "race-overlap", str(skew)]):
                    summary["decisive_counterexample"] = case
                    return 42
            summary["native_cases_completed"] = True
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
                    any(e["event"] == "sentinel_cooperative_exit" for e in sentinel_stream.events)
                )
            finally:
                bounded_reap(sentinel)
                sentinel.stdout.close()
                sentinel_log.close()
        summary["boot_after"] = capture("sysctl", "-n", "kern.bootsessionuuid")
        summary["boot_stable"] = (
            "boot" in locals() and summary["boot_after"]["rc"] == 0 and
            summary["boot_after"]["stdout"].strip().lower() == boot.lower()
        )
        dump(output / "summary.json", summary)
        hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                  for p in sorted(output.iterdir()) if p.is_file() and p.name != "sha256.json"}
        dump(output / "sha256.json", hashes)


if __name__ == "__main__":
    sys.exit(main())
