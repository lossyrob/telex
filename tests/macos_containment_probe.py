"""Owned-process counterexamples, not a production containment implementation."""

import json
import os
from pathlib import Path
import platform
import select
import signal
import subprocess
import sys
import tempfile
import time


def emit(fd, event):
    data = (json.dumps(event, sort_keys=True) + "\n").encode()
    if os.write(fd, data) != len(data):
        raise RuntimeError("short probe event write")


class Events:
    def __init__(self, fd):
        self.fd = fd
        self.buffer = b""
        self.items = []
        self.eof = False

    def until(self, predicate, timeout=5):
        deadline = time.monotonic() + timeout
        while not predicate(self.items) and not self.eof:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.fd], [], [], remaining)[0]:
                raise TimeoutError("probe event deadline expired")
            chunk = os.read(self.fd, 4096)
            if not chunk:
                self.eof = True
                if self.buffer:
                    raise RuntimeError("incomplete probe event at EOF")
                break
            self.buffer += chunk
            while b"\n" in self.buffer:
                line, self.buffer = self.buffer.split(b"\n", 1)
                item = json.loads(line)
                self.items.append(item)
                if item["event"] == "error":
                    raise RuntimeError(item["detail"])
        return predicate(self.items)

    def drain(self):
        self.until(lambda _: False)
        if not self.eof:
            raise RuntimeError("owned probe writers did not close")


def leaf(control, events, mode):
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    signal.alarm(20)
    if mode == "setpgid":
        os.setpgid(0, 0)
    identity = {
        "pid": os.getpid(),
        "pgid": os.getpgrp(),
        "sid": os.getsid(0),
    }
    emit(events, {"event": "leaf-ready", **identity})
    while True:
        request = os.read(control, 1)
        if not request:
            emit(events, {"event": "leaf-exiting", **identity})
            os._exit(0)
        if request != b"p":
            raise RuntimeError("invalid private probe request")
        emit(events, {"event": "after-signal", "ppid": os.getppid(), **identity})


def runtime(control, events, mode):
    os.setpgid(0, 0)
    signal.alarm(20)
    intermediate = os.fork()
    if intermediate == 0:
        if mode == "setsid-double-fork":
            os.setsid()
        descendant = os.fork()
        if descendant == 0:
            leaf(control, events, mode)
        os._exit(0)
    waited, status = os.waitpid(intermediate, 0)
    if waited != intermediate or not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
        raise RuntimeError("intermediate process failed")
    emit(events, {"event": "runtime-ready", "pid": os.getpid(), "pgid": os.getpgrp()})
    while True:
        signal.pause()


def probe(mode, kill_group, expect_survivor):
    control_r, control_w = os.pipe()
    events_r, events_w = os.pipe()
    root = os.fork()
    if root == 0:
        os.close(control_w)
        os.close(events_r)
        try:
            runtime(control_r, events_w, mode)
        except BaseException as error:
            emit(events_w, {"event": "error", "detail": repr(error)})
            os._exit(1)
    os.close(control_r)
    os.close(events_w)
    events = Events(events_r)
    reaped = False
    try:
        ready = events.until(
            lambda items: {"runtime-ready", "leaf-ready"} <= {i["event"] for i in items}
        )
        if not ready:
            raise RuntimeError("probe ended before readiness")
        root_info = next(i for i in events.items if i["event"] == "runtime-ready")
        leaf_info = next(i for i in events.items if i["event"] == "leaf-ready")
        if root_info["pid"] != root or root_info["pgid"] != root or root == os.getpgrp():
            raise RuntimeError("owned process-group anchor was not established")
        if mode != "ordinary" and leaf_info["pgid"] == root:
            raise RuntimeError("descendant did not escape the original process group")
        if mode == "setsid-double-fork" and leaf_info["sid"] == os.getsid(0):
            raise RuntimeError("descendant did not establish a new session")

        # This direct child is unreaped. Its PID pins the newly created group.
        if kill_group:
            os.killpg(root, signal.SIGKILL)
        else:
            os.kill(root, signal.SIGKILL)
        waited, status = os.waitpid(root, 0)
        reaped = True
        if waited != root or not os.WIFSIGNALED(status) or os.WTERMSIG(status) != signal.SIGKILL:
            raise RuntimeError("runtime did not exit from the requested SIGKILL")
        try:
            os.write(control_w, b"p")
        except BrokenPipeError:
            if expect_survivor:
                raise RuntimeError("expected survivor closed its private control pipe")
        survived = events.until(lambda items: any(i["event"] == "after-signal" for i in items))
        if survived != expect_survivor:
            raise RuntimeError(f"unexpected descendant survival: {survived}")
        response = next((i for i in events.items if i["event"] == "after-signal"), None)
        if response is not None and response["pid"] != leaf_info["pid"]:
            raise RuntimeError("private challenge response identity changed")
    finally:
        # Orphans self-exit on this private pipe's EOF; never signal a discovered PID.
        os.close(control_w)
        try:
            if not reaped:
                try:
                    os.kill(root, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                os.waitpid(root, 0)
            events.drain()
        finally:
            os.close(events_r)
    if expect_survivor and not any(i["event"] == "leaf-exiting" for i in events.items):
        raise RuntimeError("surviving leaf did not confirm cooperative cleanup")
    return {
        "probe": "owned-process-escape",
        "mode": mode,
        "signal_target": "anchored-group" if kill_group else "unreaped-runtime",
        "survived_post_kill_challenge": survived,
        "expected_survivor": expect_survivor,
        "events": events.items,
        "cleanup": "private-control-eof-and-event-eof",
    }


def main():
    if sys.platform != "darwin":
        raise RuntimeError("native macOS execution is required; no emulated success")
    if len(sys.argv) != 2:
        raise RuntimeError("usage: macos_containment_probe.py ABSOLUTE_KQUEUE_PROBE")
    binary = Path(sys.argv[1])
    if not binary.is_absolute() or not binary.is_file():
        raise RuntimeError("kqueue probe must be an absolute existing binary path")
    print(json.dumps({
        "probe": "host", "platform": platform.platform(), "machine": platform.machine(),
        "python": sys.version, "source_sha": os.environ.get("GITHUB_SHA"),
        "run_id": os.environ.get("GITHUB_RUN_ID"), "uid": os.getuid(),
    }), flush=True)
    if os.getuid() == 0:
        raise RuntimeError("run the probe as an ordinary user, not root")
    with tempfile.TemporaryDirectory(prefix="telex-macos-containment-") as directory:
        root = Path(directory).resolve()
        os.environ.update({
            "TELEX_HOME": str(root / "home"),
            "TELEX_DB": str(root / "store.sqlite"),
            "TELEX_INSTALL_ROOT": str(root / "install"),
        })
        subprocess.run([str(binary)], check=True, timeout=10)
        for mode, group, survivor in [
            ("ordinary", False, True),
            ("ordinary", True, False),
            ("setpgid", True, True),
            ("setsid-double-fork", True, True),
        ]:
            print(json.dumps(probe(mode, group, survivor), sort_keys=True), flush=True)
    print(json.dumps({"probe": "summary", "counterexamples_confirmed": True,
                      "production_containment_proven": False}), flush=True)


if __name__ == "__main__":
    main()
