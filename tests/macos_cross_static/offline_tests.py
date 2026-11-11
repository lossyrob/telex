"""Run synthetic tests with real process, network and native-loader effects denied."""

import ctypes
import os
from pathlib import Path
import resource
import socket
import subprocess
import sys
import unittest
from unittest.mock import patch
from contextlib import ExitStack


def main():
    here = Path(__file__).resolve().parent
    sys.path.insert(0, str(here))
    sys.path.insert(1, str(here.parent))
    effects = []

    def deny(name):
        def blocked(*args, **kwargs):
            effects.append(name)
            raise RuntimeError("offline test forbids real effect: " + name)
        return blocked

    def audit(event, args):
        if event.startswith(("subprocess.", "socket.", "ctypes.dlopen", "os.exec", "os.spawn")) or event in (
                "os.system", "os.fork", "os.kill", "os.killpg", "resource.setrlimit"):
            deny(event)()

    sys.addaudithook(audit)
    with ExitStack() as stack:
        for module, names in (
            (subprocess, ("Popen", "run", "call", "check_call", "check_output")),
            (socket, ("socket", "create_connection", "getaddrinfo")),
            (ctypes, ("CDLL", "PyDLL")),
            (resource, ("setrlimit",)),
            (os, ("system", "fork", "posix_spawn", "posix_spawnp", "kill", "killpg")),
        ):
            for name in names:
                if hasattr(module, name):
                    stack.enter_context(patch.object(module, name, side_effect=deny(module.__name__ + "." + name)))
        # Guards are installed before discovery imports runtime/helper modules.
        suite = unittest.defaultTestLoader.discover(str(here), pattern="test_*.py")
        result = unittest.TextTestRunner(verbosity=1).run(suite)
    if effects:
        raise RuntimeError("forbidden offline test effects attempted: " + repr(effects))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
