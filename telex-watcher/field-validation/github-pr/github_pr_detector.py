"""Bounded, read-only GitHub PR observation; fixtures require no credentials."""

import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import threading
import time

MAX_REQUEST_BYTES = 768 * 1024
MAX_RESULT_BYTES = 256 * 1024
MAX_PROVIDER_BYTES = 1024 * 1024
MAX_PROVIDER_STDERR_BYTES = 64 * 1024
MAX_SAFE_INTEGER = 9007199254740991
MAX_CONNECTION_ITEMS = 100
DEFAULT_GH_TIMEOUT_SECONDS = 15
MAX_GH_TIMEOUT_SECONDS = 20
FIELDS = (
    "number,url,title,state,headRefOid,isDraft,reviewDecision,mergeStateStatus,"
    "mergeable,reviews,comments,statusCheckRollup"
)
TIMESTAMP = re.compile(
    r"[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])"
    r"T([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\.[0-9]{1,3})?Z"
)


class ObservationError(Exception):
    """Failure that must not advance the detector's cursor."""


def _object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ObservationError("duplicate JSON member")
        result[key] = value
    return result


def _integer(text):
    number = int(text)
    if abs(number) > MAX_SAFE_INTEGER:
        raise ObservationError("integer outside interoperable range")
    return number


def _float(text):
    number = float(text)
    mantissa = re.split("[eE]", text)[0]
    if not math.isfinite(number) or (
        number == 0 and any(char in "123456789" for char in mantissa)
    ):
        raise ObservationError("number outside binary64 range")
    return number


def _constant(_):
    raise ObservationError("nonfinite JSON number")


def _unicode(value):
    if isinstance(value, str):
        for char in value:
            code = ord(char)
            if (
                0xD800 <= code <= 0xDFFF
                or 0xFDD0 <= code <= 0xFDEF
                or code & 0xFFFF in (0xFFFE, 0xFFFF)
            ):
                raise ObservationError("non-I-JSON Unicode")
    elif isinstance(value, dict):
        for key, item in value.items():
            _unicode(key)
            _unicode(item)
    elif isinstance(value, list):
        for item in value:
            _unicode(item)


def decode(raw):
    try:
        value = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=_object,
            parse_int=_integer,
            parse_float=_float,
            parse_constant=_constant,
        )
        _unicode(value)
        return value
    except (UnicodeError, ValueError, RecursionError) as error:
        raise ObservationError("invalid JSON") from error


def encode(value):
    return json.dumps(
        value, ensure_ascii=False, allow_nan=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")


def exact_object(value, required, optional=()):
    if not isinstance(value, dict):
        raise ObservationError("expected object")
    if not set(required) <= value.keys() or value.keys() - set(required) - set(optional):
        raise ObservationError("missing or unsupported fields")


def timestamp(value):
    if not isinstance(value, str) or not TIMESTAMP.fullmatch(value):
        raise ObservationError("invalid UTC timestamp")
    try:
        datetime.datetime.fromisoformat(value[:-1] + "+00:00")
    except ValueError as error:
        raise ObservationError("invalid calendar timestamp") from error


def non_control(value):
    return isinstance(value, str) and bool(value) and not any(
        ord(char) < 32 or ord(char) == 127 for char in value
    )


def request_parameters(request):
    exact_object(request, ("schemaVersion", "attempt", "watch", "state"))
    if type(request["schemaVersion"]) is not int or request["schemaVersion"] != 2:
        raise ObservationError("unsupported schema version")
    exact_object(request["attempt"], ("id", "now"))
    if not non_control(request["attempt"]["id"]):
        raise ObservationError("invalid attempt ID")
    timestamp(request["attempt"]["now"])
    watch = request["watch"]
    exact_object(watch, ("id", "activatedAt", "lastSuccessAt", "parameters"))
    if not isinstance(watch["id"], str) or not re.fullmatch(
        "[a-z0-9][a-z0-9._-]{0,127}", watch["id"]
    ):
        raise ObservationError("invalid watch ID")
    timestamp(watch["activatedAt"])
    if watch["lastSuccessAt"] is not None:
        timestamp(watch["lastSuccessAt"])
    parameters = watch["parameters"]
    exact_object(
        parameters,
        ("repo", "pullRequest"),
        ("fixture", "emitInitialSnapshot", "ghTimeoutSeconds"),
    )
    repo = parameters["repo"]
    if not isinstance(repo, str) or len(repo) > 256 or not re.fullmatch(
        "[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*", repo
    ):
        raise ObservationError("repo must be owner/name")
    number = parameters["pullRequest"]
    if type(number) is not int or not 1 <= number <= 2147483647:
        raise ObservationError("invalid PR number")
    timeout = parameters.get("ghTimeoutSeconds", DEFAULT_GH_TIMEOUT_SECONDS)
    if type(timeout) is not int or not 1 <= timeout <= MAX_GH_TIMEOUT_SECONDS:
        raise ObservationError("invalid gh timeout")
    if type(parameters.get("emitInitialSnapshot", False)) is not bool:
        raise ObservationError("emitInitialSnapshot must be boolean")
    if "fixture" in parameters and not non_control(parameters["fixture"]):
        raise ObservationError("fixture must name a local file")
    if len(encode(parameters)) > 256 * 1024 or len(encode(request["state"])) > 256 * 1024:
        raise ObservationError("opaque request value exceeds its bound")
    return parameters


def bounded_command(argv, timeout, stdout_cap=MAX_PROVIDER_BYTES, stderr_cap=MAX_PROVIDER_STDERR_BYTES):
    """Drain both pipes concurrently without unbounded communicate() buffering."""
    environment = os.environ.copy()
    environment.update(
        GH_PROMPT_DISABLED="1", GH_PAGER="cat", PAGER="cat", GH_NO_UPDATE_NOTIFIER="1"
    )
    process = subprocess.Popen(
        argv,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        shell=False,
        env=environment,
    )
    outputs = [bytearray(), bytearray()]
    failed = threading.Event()

    def drain(pipe, output, cap):
        try:
            while True:
                chunk = pipe.read(4096)
                if not chunk:
                    break
                if len(output) + len(chunk) > cap:
                    failed.set()
                    return
                output.extend(chunk)
        except OSError:
            failed.set()

    threads = [
        threading.Thread(target=drain, args=(process.stdout, outputs[0], stdout_cap), daemon=True),
        threading.Thread(target=drain, args=(process.stderr, outputs[1], stderr_cap), daemon=True),
    ]
    for thread in threads:
        thread.start()
    deadline = time.monotonic() + timeout
    problem = None
    while any(thread.is_alive() for thread in threads) or process.poll() is None:
        if failed.is_set():
            problem = "provider output limit"
            break
        if time.monotonic() >= deadline:
            problem = "provider timeout"
            break
        failed.wait(min(0.01, max(0, deadline - time.monotonic())))
    if failed.is_set():
        problem = "provider output limit"
    if problem:
        if process.poll() is None:
            process.kill()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            pass
    for thread in threads:
        thread.join(timeout=0.1)
    # Do not block closing a pipe while another thread owns its reader lock.
    for thread, pipe in zip(threads, (process.stdout, process.stderr)):
        if not thread.is_alive():
            pipe.close()
    if problem:
        raise ObservationError(problem)
    if process.returncode != 0:
        # Provider stderr may contain credentials or private content.
        raise ObservationError("provider command failed")
    return bytes(outputs[0])


def load_observation(parameters):
    if "fixture" in parameters:
        path = Path(parameters["fixture"])
        if not path.is_absolute():
            path = Path(__file__).resolve().parent / path
        with path.open("rb") as stream:
            raw = stream.read(MAX_PROVIDER_BYTES + 1)
        if len(raw) > MAX_PROVIDER_BYTES:
            raise ObservationError("fixture exceeds provider output limit")
    else:
        raw = bounded_command(
            [
                "gh", "pr", "view", str(parameters["pullRequest"]),
                "--repo", parameters["repo"], "--json", FIELDS,
            ],
            parameters.get("ghTimeoutSeconds", DEFAULT_GH_TIMEOUT_SECONDS),
        )
    return decode(raw)


def digest(value):
    return hashlib.sha256(encode(value)).hexdigest()


def connection_digest(observation, field):
    entries = observation[field]
    if not isinstance(entries, list) or any(not isinstance(item, dict) for item in entries):
        raise ObservationError("invalid provider collection")
    # gh pr view's connections are bounded. At the boundary we cannot prove
    # completeness: fail closed instead of silently advancing past observations.
    if len(entries) >= MAX_CONNECTION_ITEMS:
        raise ObservationError("provider collection may be truncated")
    return digest(sorted(entries, key=encode))


def snapshot(observation, parameters):
    required = set(FIELDS.split(","))
    if not isinstance(observation, dict) or not required <= observation.keys():
        raise ObservationError("incomplete provider observation")
    if type(observation["number"]) is not int or observation["number"] != parameters["pullRequest"]:
        raise ObservationError("provider PR number mismatch")
    if observation["state"] not in ("OPEN", "CLOSED", "MERGED"):
        raise ObservationError("unknown provider PR state")
    if type(observation["isDraft"]) is not bool:
        raise ObservationError("invalid draft state")
    for field in ("url", "title", "headRefOid", "reviewDecision", "mergeStateStatus", "mergeable"):
        if not isinstance(observation[field], str):
            raise ObservationError("invalid provider field")
    # Keep cursors small: raw provider bodies do not become detector state.
    return {
        "state": observation["state"],
        "head": observation["headRefOid"],
        "draft": observation["isDraft"],
        "title": digest(observation["title"]),
        "reviews": digest({
            "decision": observation["reviewDecision"],
            "items": connection_digest(observation, "reviews"),
        }),
        "comments": connection_digest(observation, "comments"),
        "checks": connection_digest(observation, "statusCheckRollup"),
        "merge": digest({
            "status": observation["mergeStateStatus"],
            "mergeable": observation["mergeable"],
        }),
    }


def evaluate(request, observation):
    parameters = request_parameters(request)
    current = snapshot(observation, parameters)
    previous_state = request["state"]
    previous = None
    if previous_state is not None:
        exact_object(previous_state, ("version", "repo", "pullRequest", "snapshot"))
        if (
            type(previous_state["version"]) is not int
            or previous_state["version"] != 1
            or previous_state["repo"] != parameters["repo"]
            or previous_state["pullRequest"] != parameters["pullRequest"]
        ):
            raise ObservationError("cursor does not match this detector and PR")
        previous = previous_state["snapshot"]
        exact_object(previous, current.keys())
        if previous["state"] not in ("OPEN", "CLOSED", "MERGED"):
            raise ObservationError("invalid cursor state")
        if type(previous["draft"]) is not bool or any(
            not isinstance(previous[key], str) for key in current if key != "draft"
        ):
            raise ObservationError("invalid cursor fields")
    next_state = {
        "version": 1,
        "repo": parameters["repo"],
        "pullRequest": parameters["pullRequest"],
        "snapshot": current,
    }
    terminal = current["state"] in ("CLOSED", "MERGED")
    changed = sorted(key for key in current if previous is not None and previous[key] != current[key])
    emit = terminal or bool(changed) or (
        previous is None and parameters.get("emitInitialSnapshot", False)
    )
    if not emit:
        result = {"schemaVersion": 2, "outcome": "idle"}
        if previous is None:
            result["nextState"] = next_state
        return result
    label = "terminal" if terminal else ("snapshot" if previous is None else "changed")
    number = parameters["pullRequest"]
    return {
        "schemaVersion": 2,
        "outcome": "terminal" if terminal else "event",
        "nextState": next_state,
        "event": {
            "kind": "github.pull-request." + label,
            "subject": f"PR #{number} {current['state'].lower()}: {label}",
            "body": (
                f"{parameters['repo']} PR #{number}\n"
                f"State: {current['state']}\n"
                f"Changed: {', '.join(changed) if changed else 'initial observation'}"
            ),
            "metadata": {
                "repository": parameters["repo"],
                "pullRequest": number,
                "state": current["state"],
                "changed": changed,
            },
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    try:
        raw = sys.stdin.buffer.read(MAX_REQUEST_BYTES + 1)
        if len(raw) > MAX_REQUEST_BYTES:
            raise ObservationError("request exceeds input limit")
        request = decode(raw)
        parameters = request_parameters(request)
        result = evaluate(request, load_observation(parameters))
        encoded = encode(result)
        if len(encoded) > MAX_RESULT_BYTES:
            raise ObservationError("result exceeds output limit")
    except (ObservationError, OSError, ValueError, TypeError, KeyError, RecursionError):
        # Bounded, content-free diagnostics. No cursor or partial event on failure.
        sys.stderr.write("github-pr-detector: observation failed; state unchanged\n")
        encoded = encode({"schemaVersion": 2, "outcome": "degraded"})
    sys.stdout.buffer.write(encoded + b"\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
