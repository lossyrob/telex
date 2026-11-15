"""Reproduce the installed sibling layout for Cargo-built Linux test hosts."""

import json
from pathlib import Path
import shutil
import subprocess
import sys


def guard_directory(host):
    return host.parent.parent if host.parent.name == "deps" else host.parent


def stage_guard(messages, manifest, diagnose=False):
    builds = [m for m in messages if m.get("reason") == "build-finished"]
    if not builds or any(m.get("success") is not True for m in builds):
        raise ValueError("Cargo did not report successful fixture builds")
    artifacts = [
        m
        for m in messages
        if m.get("reason") == "compiler-artifact"
        and Path(m["manifest_path"]).resolve() == manifest.resolve()
        and m.get("executable")
    ]
    guards = {
        Path(m["executable"]).resolve(strict=True)
        for m in artifacts
        if m["target"]["name"] == "watcher_process_guard"
        and m["target"]["kind"] == ["bin"]
        and m["profile"]["test"] is False
    }
    if len(guards) != 1:
        raise ValueError(
            f"expected one exact Cargo-built watcher_process_guard, found {sorted(map(str, guards))}"
        )
    guard = guards.pop()
    if not guard.is_file():
        raise ValueError(f"Cargo guardian is not a file: {guard}")
    hosts = {
        Path(m["executable"]).resolve(strict=True)
        for m in artifacts
        if m["profile"]["test"] is True
    }
    if not hosts or any(not host.is_file() for host in hosts):
        raise ValueError("Cargo did not report usable Watcher test executables")
    if diagnose:
        for artifact in artifacts:
            if (
                artifact["profile"]["test"] is True
                and artifact["target"]["name"] == "process_v2"
            ):
                host = Path(artifact["executable"]).resolve(strict=True)
                expected = guard_directory(host) / "watcher_process_guard"
                result = subprocess.run(
                    [
                        str(host),
                        "direct_argv_canonical_cwd_environment_and_redaction",
                        "--exact",
                        "--nocapture",
                    ],
                    capture_output=True,
                    text=True,
                    timeout=30,
                    check=False,
                )
                output = result.stdout + result.stderr
                if result.returncode:
                    missing = "start process guard: No such file or directory (os error 2)"
                    if expected.exists() or missing not in output:
                        raise ValueError(
                            f"unexpected pre-staging process fixture failure:\n{output}"
                        )
                    print(
                        f"Confirmed missing test-host guardian {expected}:\n{output}",
                        flush=True,
                    )
                else:
                    if not expected.is_file():
                        raise ValueError(
                            f"process fixture passed without expected guardian {expected}"
                        )
                    print(
                        f"Pre-staging process fixture passed with guardian {expected}",
                        flush=True,
                    )
    for directory in sorted({guard_directory(host) for host in hosts}):
        target = directory / "watcher_process_guard"
        if target.is_symlink():
            raise ValueError(f"refusing to overwrite guardian symlink: {target}")
        if target != guard:
            shutil.copy2(guard, target)
        print(f"Watcher test guardian: {guard} -> {target}", flush=True)


def main():
    messages = [json.loads(line) for line in sys.stdin if line.strip()]
    for message in messages:
        if message.get("reason") == "compiler-message":
            rendered = message.get("message", {}).get("rendered")
            if rendered:
                print(rendered, file=sys.stderr, end="")
    manifest = Path(__file__).resolve().parents[1] / "telex-watcher" / "Cargo.toml"
    stage_guard(messages, manifest, diagnose=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        sys.exit(f"Watcher test guardian setup failed: {error}")
