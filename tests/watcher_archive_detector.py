"""Deterministic, state-aware detector used only by the archive upgrade proof."""

import json
from pathlib import Path
import sys


def detect(request, mode):
    if mode == "failure":
        raise RuntimeError("archive proof historical detector failure")
    state = request["state"]
    if state == {"cursor": 1}:
        return {"schemaVersion": 2, "outcome": "idle", "nextState": state}
    if state is not None:
        raise ValueError(f"unexpected archive proof state: {state!r}")
    return {
        "schemaVersion": 2,
        "outcome": "event",
        "nextState": {"cursor": 1},
        "event": {
            "kind": "fixture.archive-upgrade",
            "subject": "Archive upgrade proof",
            "body": "exactly one receipt-gated event",
        },
    }


if __name__ == "__main__":
    request = json.load(sys.stdin)
    directory = Path(sys.argv[1])
    if not directory.is_absolute():
        raise ValueError("fixture evidence directory must be absolute")
    (directory / f'{request["attempt"]["id"]}.json').write_text(
        json.dumps(request), encoding="utf-8")
    json.dump(detect(request, sys.argv[2]), sys.stdout)
