"""Disposable Windows proof of inherited-profile bridge access; no live App or profile changes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from types import SimpleNamespace
import uuid

from release_candidate import (
    Proof, clean_environment, create_fixture_root, finish_report, read_identity_text, require,
    unpack_binary,
)

ROOT = Path(__file__).resolve().parent.parent
RC1_MODULES = {
    "extension.mjs": "1c9b839bbfb26c2e239e6fbeabfb82998f4e5b391d78fbfa244a6ae746980100",
    "busy-state.mjs": "7187c58f3669439edcb3cdab4fc20f1c9f05dc6a3e0f70c8b4da7085f6127352",
    "probe-protocol.mjs": "d7a4ef43a33e8ebd2b61c4fdb5a8c5efc9bd700ced9e6189357af5a443aba7b9",
}

def finish_smoke_report(proof, root, report, output, processes, logs):
    """Attempt each owned cleanup independently and preserve failure evidence outermost."""
    errors = []
    primary = report.get("error")
    for child in processes:
        try:
            if child.poll() is None:
                child.terminate()
            child.communicate(timeout=10)
            report["cleanup"].append({"owned_pid": child.pid, "joined": True})
        except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
            errors.append(f"owned process {child.pid}: {error}")
    for process, binary, env, _ in proof.daemons:
        try:
            if process.poll() is None:
                proof.stop_daemon(process, binary, env)
        except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
            errors.append(f"daemon {process.pid}: {error}")
            try:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
            except (OSError, RuntimeError, subprocess.TimeoutExpired) as fallback:
                errors.append(f"daemon {process.pid} fallback: {fallback}")
    for log in logs:
        try:
            log.close()
            report.setdefault("producer_logs", {})[Path(log.name).name] = Path(log.name).read_text(errors="replace")
        except (OSError, RuntimeError) as error:
            errors.append(f"producer log: {error}")
    try:
        finish_report(proof, root, report, output, extra_processes=processes)
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        errors.append(str(error))
    finally:
        for process, _, _, _ in proof.daemons:
            try:
                if hasattr(process, "close"):
                    process.close()
            except (OSError, RuntimeError) as error:
                errors.append(f"daemon handle {process.pid}: {error}")
        if errors:
            report["status"] = "failed"
            report["owned_cleanup_errors"] = errors
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if errors and primary is None:
        raise RuntimeError("bridge fixture cleanup failed: " + "; ".join(errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--rc1-archive", type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    require(os.name == "nt", "native Windows required")
    binary = args.binary.resolve()
    root = create_fixture_root()
    report = {"status": "failed", "source_sha": args.source_sha, "commands": [],
              "daemons": [], "proofs": [], "cleanup": [],
              "qualification": "Actual RC1-identical Node bundle with fixture SDK, not a real App/sandbox experiment.",
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    report["source_files_sha256"] = {
        str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in [ROOT / "src" / "commands" / "copilot.rs", ROOT / "src" / "platform_fs.rs",
                     ROOT / "src" / "daemon_reconcile.rs", Path(__file__).resolve(),
                     *(ROOT / "copilot" / "bridge" / name for name in RC1_MODULES)]
    }
    proof = Proof(SimpleNamespace(postgres_url=None), root, report)
    node = None
    log = None
    unbound_node = None
    unbound_log = None
    lock_processes = []

    def wait_for(predicate, label, timeout=20):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(0.05)
        raise RuntimeError(label)

    def snapshot(directory):
        return {str(path.relative_to(directory)): (hashlib.sha256(path.read_bytes()).hexdigest(),
                                                   path.stat().st_mtime_ns)
                for path in directory.rglob("*") if path.is_file()}

    def finalized(address):
        result = proof.telex(binary, env, "--address", address, "copilot", "attach",
                             "--session", session, "--copilot-bridge")
        require("not finalized" not in result.stderr, "authenticated attach did not finalize")
        rows = [json.loads(path.read_text(encoding="utf-8"))
                for path in Path(env["TELEX_RUN_DIR"]).rglob("*.json")]
        require(any(row.get("address") == address and row.get("state") == "live" and row.get("armed")
                    for row in rows), "no live armed station intent")

    try:
        env = clean_environment(root / "case")
        for key in ("USERPROFILE", "HOME", "LOCALAPPDATA", "APPDATA", "TELEX_HOME"):
            Path(env[key]).mkdir(parents=True, exist_ok=True)
        Path(env["TELEX_CONFIG"]).write_text("", encoding="utf-8")
        session = "simple-" + uuid.uuid4().hex
        env["ACL_SESSION"] = session
        copilot = Path(env["USERPROFILE"]) / ".copilot"
        registry = copilot / "telex-bridge" / (session + ".json")
        registry.parent.mkdir(parents=True)
        extension = copilot / "session-state" / session / "extensions" / "telex-bridge"
        extension.mkdir(parents=True)
        for name, digest in RC1_MODULES.items():
            contents = (ROOT / "copilot" / "bridge" / name).read_bytes()
            require(hashlib.sha256(contents).hexdigest() == digest, "bridge module differs from RC1: " + name)
            (extension / name).write_bytes(contents)
            os.utime(extension / name, (1_600_000_000, 1_600_000_000))
        (extension / "installer-marker").write_text("retain", encoding="ascii")
        sdk = root / "case" / "node_modules" / "@github" / "copilot-sdk"
        sdk.mkdir(parents=True)
        (sdk / "package.json").write_text(
            '{"name":"@github/copilot-sdk","type":"module","exports":{"./extension":"./extension.mjs"}}',
            encoding="ascii")
        (sdk / "extension.mjs").write_text(
            "export async function joinSession(){return {sessionId:process.env.ACL_SESSION,"
            "on(){},async log(x){console.log(x)},async send(x){console.log('FIXTURE_SEND '+x.prompt);"
            "return 'fixture-message'}}}", encoding="ascii")
        log_path = root / "producer.log"
        log = log_path.open("wb")
        node = subprocess.Popen([shutil.which("node"), str(extension / "extension.mjs")],
                                env=env, cwd=root, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        wait_for(lambda: "telex-bridge ready for " + session in log_path.read_text(errors="replace"),
                 "RC1-identical producer did not become ready")
        metadata = json.loads(proof.telex(binary, env, "version").stdout)
        require(metadata["version"]["build_id"] == args.source_sha, "binary/source mismatch")
        old = None
        if args.rc1_archive:
            machine = binary.read_bytes()
            offset = int.from_bytes(machine[0x3c:0x40], "little")
            architecture = int.from_bytes(machine[offset + 4:offset + 6], "little")
            expected = {0x8664: "5b29977ca41cc4600ea53799f4f3493f5cb2531c7f07cb8bef75138a8a9d985e",
                        0xaa64: "9e7545ec0913e68622c040e9a8a01c31c3b784c63461da3677dcaa8f3b20f1f4"}
            require(hashlib.sha256(args.rc1_archive.read_bytes()).hexdigest() == expected[architecture],
                    "official RC1 archive mismatch")
            old = unpack_binary(args.rc1_archive.resolve(), root / "rc1", True)
            proof.metadata(old, env, "0.3.0-rc.1", "264e650fbba45ab029f5e796d85039ad50d0a224", 5, 3)
            old_daemon = proof.start_daemon(old, env, 5)
            proof.telex(old, env, "--address", "fixture:existing", "copilot", "attach",
                        "--session", session, "--copilot-bridge")
        # Ordinary Node publication inherits the containing profile's access policy.
        proof.run(["icacls", registry.parent, "/grant", "*S-1-1-0:(OI)(CI)(RX)"], env)
        report["credential_acl"] = proof.run(["icacls", registry], env).stdout
        if old:
            rejected = proof.telex(old, env, "--address", "fixture:existing", "copilot", "resume",
                                   "--session", session, expected=1)
            require("non-owner SID" in rejected.stderr, "RC1 did not reproduce its ACE refusal")
            proof.stop_daemon(old_daemon, old, env)
        modules_before = snapshot(extension)
        current_daemon = proof.start_daemon(binary, env, 5)
        finalized("fixture:existing")
        require(snapshot(extension) == modules_before, "matching RC1 modules/markers were rewritten")
        require(json.loads(read_identity_text(registry))["pid"] == node.pid, "producer was replaced")
        for label, address in (("same", "fixture:existing"), ("other", "fixture:new")):
            bindings = registry.with_name(session + ".bindings.json")
            before = bindings.read_bytes()
            rejected = proof.telex(binary, env, "--backend", "fixture-missing-backend", "--address", address,
                                   "copilot", "attach", "--session", session, "--copilot-bridge", expected=1)
            require("fixture-missing-backend" in rejected.stderr and bindings.read_bytes() == before,
                    label + " failed attach changed bindings or hid the refusal")
            require(snapshot(extension) == modules_before and node.poll() is None and registry.exists(),
                    "failed attach damaged live producer")
        nonce = "bridge-proof-" + uuid.uuid4().hex
        descriptor = {"message_id": 1, "address": "fixture:existing", "attention": "interrupt",
                      "body": nonce, "kind": "note"}
        push = subprocess.run([str(binary), "--json", "copilot", "push", "--session", session],
                              env=env, cwd=root, input=json.dumps(descriptor), capture_output=True,
                              text=True, encoding="utf-8", timeout=30)
        report["commands"].append({"operation": "authenticated-push", "exit_code": push.returncode,
                                   "stdout": push.stdout, "stderr": push.stderr})
        require(push.returncode == 0 and nonce in log_path.read_text(errors="replace"),
                "authenticated push did not reach actual source bridge")
        proof.stop_daemon(current_daemon, binary, env)
        proof.start_daemon(binary, env, 5)
        reconciled = json.loads(proof.telex(binary, env, "daemon", "reconcile").stdout)
        report["reconcile"] = reconciled
        status = json.loads(proof.telex(binary, env, "--address", "fixture:existing", "status").stdout)
        report["status_after_replacement"] = status
        require("credential_insecure" not in json.dumps(status), "daemon recovery retained strict producer reader")
        require('"attended_push"' in json.dumps(status), "daemon replacement did not restore attended push")
        report["proofs"].append({"scenario": "inherited-profile-live-rc1-bundle",
                                 "official_rc1_binary_reproduced": bool(old),
                                 "same_node_pid_no_reload": node.pid, "module_snapshot_unchanged": True,
                                 "failed_attach_preserved": ["same-binding", "other-binding"],
                                 "authenticated_push": True, "daemon_replacement": True})
        # A killed OS-lock holder leaves a file, but cannot leave a five-minute stale lock.
        lock_env = dict(env, FIXTURE_LOCK=str(registry.with_name(session + ".bindings.lock")),
                        FIXTURE_READY=str(root / "lock-ready"))
        powershell = Path(os.environ["SystemRoot"]) / "System32" / "WindowsPowerShell" / "v1.0" / "powershell.exe"
        holder = subprocess.Popen([str(powershell), "-NoProfile", "-NonInteractive", "-Command",
            "$ErrorActionPreference='Stop';"
            "$file=[IO.File]::Open($env:FIXTURE_LOCK,'OpenOrCreate','ReadWrite','None');"
            "try{[IO.File]::WriteAllText($env:FIXTURE_READY,'held');Start-Sleep -Seconds 15}"
            "finally{$file.Dispose()}"], env=lock_env, cwd=root,
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        lock_processes.append(holder)
        wait_for(lambda: Path(lock_env["FIXTURE_READY"]).exists() or holder.poll() is not None,
                 "owned lock holder did not start")
        require(holder.poll() is None, "owned lock holder failed before exclusion")
        contender = subprocess.Popen([str(binary), "--json", "--address", "fixture:concurrent",
            "copilot", "attach", "--session", session, "--copilot-bridge"],
            env=env, cwd=root, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        lock_processes.append(contender)
        time.sleep(0.3)
        require(contender.poll() is None, "contending attach did not wait for the OS lock")
        holder.terminate()
        holder.communicate(timeout=10)
        stdout, stderr = contender.communicate(timeout=20)
        require(contender.returncode == 0, "attach did not recover after lock-owner death: " + stderr.decode(errors="replace"))
        bindings = json.loads(registry.with_name(session + ".bindings.json").read_bytes())
        require(set(bindings) == {"fixture:existing", "fixture:concurrent"}, "concurrent binding lost an address")
        report["proofs"].append({"scenario": "binding-lock-owner-death",
                                 "contender_waited": True, "stale_lock_file_did_not_block": True})
        # Installer-loaded but unbound: even an early attach failure cannot remove its files.
        unbound = "unbound-" + uuid.uuid4().hex
        unbound_extension = copilot / "session-state" / unbound / "extensions" / "telex-bridge"
        shutil.copytree(extension, unbound_extension)
        unbound_registry = registry.with_name(unbound + ".json")
        unbound_log_path = root / "unbound-producer.log"
        unbound_log = unbound_log_path.open("wb")
        unbound_node = subprocess.Popen([shutil.which("node"), str(unbound_extension / "extension.mjs")],
            env=dict(env, ACL_SESSION=unbound), cwd=root, stdin=subprocess.DEVNULL,
            stdout=unbound_log, stderr=unbound_log)
        wait_for(lambda: "telex-bridge ready for " + unbound in unbound_log_path.read_text(errors="replace"),
                 "unbound installer producer did not become ready")
        before = snapshot(unbound_extension)
        proof.telex(binary, env, "--backend", "fixture-missing-backend", "--address", "fixture:unbound",
                    "copilot", "attach", "--session", unbound, "--copilot-bridge", expected=1)
        require(snapshot(unbound_extension) == before and unbound_registry.exists() and unbound_node.poll() is None,
                "unbound installer producer was removed on refusal")
        report["proofs"].append({"scenario": "installer-unbound-refusal-preserved"})
        # Oversized/non-regular registries refuse before matching module/binding mutation.
        node.terminate()
        node.wait(timeout=10)
        original = registry.read_bytes()
        binding_bytes = registry.with_name(session + ".bindings.json").read_bytes()
        for label in ("oversized", "directory"):
            if label == "oversized":
                registry.write_bytes(b"x" * (64 * 1024 + 1))
            else:
                registry.unlink()
                registry.mkdir()
            rejected = proof.telex(binary, env, "--address", "fixture:negative", "copilot", "attach",
                                   "--session", session, "--copilot-bridge", expected=1)
            require("invalid existing bridge registry" in rejected.stderr, "wrong registry shape refusal")
            require(snapshot(extension) == modules_before and
                    registry.with_name(session + ".bindings.json").read_bytes() == binding_bytes,
                    "invalid registry refusal mutated modules/bindings")
            if label == "directory":
                registry.rmdir()
            registry.write_bytes(original)
            report["proofs"].append({"scenario": "registry-" + label + "-refused"})
        report["status"] = "passed"
    except Exception as error:
        report["error"] = repr(error)
        raise
    finally:
        owners = [*lock_processes, *(child for child in (node, unbound_node) if child is not None)]
        finish_smoke_report(proof, root, report, args.report.resolve(), owners,
                            [output for output in (log, unbound_log) if output is not None])


if __name__ == "__main__":
    main()
