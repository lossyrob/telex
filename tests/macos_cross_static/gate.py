"""Fresh component-source review -> finite disposition -> freeze. No retry or launch helper."""

import hashlib
import json
import os
from pathlib import Path
import re
import time

import derived


MARKER = re.compile(r"<!-- arm24-intel25-derived-authority-v1 (\{[^\r\n]*\}) -->")
ISSUE = "https://api.github.com/repos/lossyrob/telex/issues/172"
COMMENT = "https://github.com/lossyrob/telex/issues/172#issuecomment-"
WORKFLOW = ".github/workflows/macos-cross-static.yml"
BRANCH = "lossyrob-macos-containment-proof"
SOURCES = ["tests/macos_cross_static/" + name for name in
           ("bounds.json", "run.py", "cache.py", "gate.py", "native.py", "derived.py", "test_component.py", "README.md")] + [
               "tests/macos_static_cache.py", "tests/macos_static_ridiff.py",
               "tests/macos_provenance_capture.py", WORKFLOW]
require = derived.require


def closure(root):
    rows = []
    for name in SOURCES:
        file = root.joinpath(*name.split("/"))
        require(file.is_file() and not file.is_symlink(), "component closure member absent/nonregular")
        raw = file.read_bytes()
        rows.append({"file": name, "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()})
    raw = (json.dumps(rows, sort_keys=True, indent=2) + "\n").encode()
    return rows, hashlib.sha256(raw).hexdigest()


def approved_records(comments):
    records = []
    for comment in comments:
        if comment.get("user", {}).get("login") != "lossyrob":
            continue
        body = comment.get("body")
        require(isinstance(body, str), "comment body schema")
        matches = list(MARKER.finditer(body))
        if not matches:
            continue
        require(comment["user"].get("id") == 2320142 and len(matches) == 1 and type(comment.get("id")) is int and
                comment.get("html_url") == COMMENT + str(comment["id"]) and comment.get("issue_url") == ISSUE,
                "ambiguous/noncanonical authority comment")
        value = derived.strict_json(matches[0][1])
        require(isinstance(value, dict), "authority record must be object")
        records.append({"value": value, "comment": comment["html_url"], "id": comment["id"],
                        "created_at": comment["created_at"], "updated_at": comment["updated_at"],
                        "body_sha256": hashlib.sha256(body.encode()).hexdigest(), "body": body})
    return records


def validate(records, context, previous=None):
    require(previous is None and type(context["attempt"]) is int and context["attempt"] == 1,
            "zero retries, including infrastructure; return to design")
    require(context["job"] in ("ARM24", "Intel25"), "unknown component context")
    common = {"component": "arm24-intel25-derived-v1", "sha": context["sha"],
              "bounds_sha256": context["bounds_sha256"], "source_manifest_sha256": context["source_manifest_sha256"],
              "workflow": WORKFLOW, "jobs": ["ARM24", "Intel25"],
              "artifact_id": context["artifact_id"], "candidate_sha256": context["candidate_sha256"],
              "candidate_bytes": context["candidate_bytes"], "expires_at": context["expires_at"],
              "attempt": 1, "retries": 0}
    for name in ("sha", "bounds_sha256", "source_manifest_sha256", "candidate_sha256"):
        require(re.fullmatch(r"[0-9a-f]{40}" if name == "sha" else r"[0-9a-f]{64}", context[name]),
                "invalid exact source identity")
    selected = {}
    headers = {"review": "APPROVE - independent component-source review",
               "finite": "Design steward finite component disposition",
               "freeze": "NEW genuine owner component freeze"}
    extra = {"review": {"kind", "verdict", "scope"},
             "finite": {"kind", "source_review", "source_review_body_sha256"},
             "freeze": {"kind", "source_review", "source_review_body_sha256",
                        "finite_disposition", "finite_body_sha256", "allow_launch"}}
    for kind in headers:
        matches = [r for r in records if r["value"].get("kind") == kind and
                   all(type(r["value"].get(k)) is type(v) and r["value"][k] == v for k, v in common.items())]
        require(len(matches) == 1, "missing/ambiguous fresh component " + kind)
        row, value = matches[0], matches[0]["value"]
        require(set(value) == set(common) | extra[kind], "unknown authority/retry fields")
        require(row["created_at"] == row["updated_at"] and row["body"].startswith(headers[kind]),
                "authority must be unedited and have exact stage header")
        derived.instant(row["created_at"])
        selected[kind] = row
    review, finite, freeze = (selected[k] for k in ("review", "finite", "freeze"))
    require(review["value"]["verdict"] == "APPROVE" and review["value"]["scope"] == "whole-component-source",
            "BUILD/partial/actual approval does not authorize component source")
    for row in (finite, freeze):
        require(row["value"]["source_review"] == review["comment"] and
                row["value"]["source_review_body_sha256"] == review["body_sha256"], "source approval binding mismatch")
    require(freeze["value"]["finite_disposition"] == finite["comment"] and
            freeze["value"]["finite_body_sha256"] == finite["body_sha256"] and
            freeze["value"]["allow_launch"] is True, "finite disposition/freeze binding mismatch")
    times = [derived.instant(r["created_at"]) for r in (review, finite, freeze)]
    require(len({r["id"] for r in (review, finite, freeze)}) == 3 and
            times[0] < times[1] < times[2] < derived.instant(context["run_created_at"]) < derived.instant(context["expires_at"]),
            "review -> finite -> freeze -> run/retention order mismatch")
    require(time.time() < derived.instant(context["expires_at"]), "artifact retention elapsed")
    return {"retry": False, "attempt": 1, "retries": 0, "source_manifest_sha256": context["source_manifest_sha256"],
            "authorities": selected, "independence_precision": "Authenticated owner-routed independent review, steward and freeze records; semantic independence remains the owner/reviewer obligation."}


def check(job):
    here = Path(__file__).resolve().parent
    raw = (here / "bounds.json").read_bytes()
    bounds = derived.strict_json(raw)
    pin = bounds["tools"]["ipsw"]
    derived.unexpired(pin)
    require(os.environ.get("GITHUB_EVENT_NAME") == "push" and os.environ.get("GITHUB_RUN_ATTEMPT") == "1" and
            os.environ.get("GITHUB_REPOSITORY") == "lossyrob/telex" and
            os.environ.get("GITHUB_REF") == "refs/heads/" + BRANCH and
            os.environ.get("GITHUB_WORKFLOW_REF") == "lossyrob/telex/" + WORKFLOW + "@refs/heads/" + BRANCH,
            "exact dedicated component workflow/branch/push/attempt required")
    sha, run_id = os.environ.get("GITHUB_SHA"), os.environ.get("GITHUB_RUN_ID")
    require(isinstance(sha, str) and re.fullmatch(r"[0-9a-f]{40}", sha) and
            isinstance(run_id, str) and re.fullmatch(r"[1-9][0-9]*", run_id), "invalid execution identity")
    rows, source_hash = closure(here.parents[1])
    deadline, token = time.monotonic() + 120, os.environ.get("GH_TOKEN")
    current = derived.fetch_json("actions/runs/" + run_id, token, deadline)
    require(isinstance(current, dict) and current.get("id") == int(run_id) and current.get("head_sha") == sha and
            type(current.get("run_attempt")) is int and current["run_attempt"] == 1 and current.get("path") == WORKFLOW and
            current.get("event") == "push" and current.get("head_branch") == BRANCH,
            "live component run identity mismatch")
    # A new push of the same source cannot start another attempt under the same freeze.
    runs = derived.fetch_json("actions/runs?head_sha=" + sha + "&per_page=100", token, deadline)
    require(isinstance(runs, dict) and type(runs.get("total_count")) is int and
            isinstance(runs.get("workflow_runs"), list) and runs["total_count"] == len(runs["workflow_runs"]) <= 100,
            "incomplete same-source run inventory")
    own = [r for r in runs["workflow_runs"] if r.get("path") == WORKFLOW]
    require(len(own) == 1 and own[0].get("id") == int(run_id) and own[0].get("run_attempt") == 1,
            "component source already has another run; return to design")
    comments = []
    for page in range(1, 11):
        batch = derived.fetch_json(f"issues/172/comments?per_page=100&page={page}", token, deadline)
        require(isinstance(batch, list) and len(batch) <= 100 and all(isinstance(r, dict) for r in batch),
                "issue comments schema mismatch")
        comments.extend(batch)
        if len(batch) < 100:
            break
    else:
        raise RuntimeError("issue comment bound exhausted")
    context = {"sha": sha, "job": job, "attempt": 1, "bounds_sha256": hashlib.sha256(raw).hexdigest(),
               "source_manifest_sha256": source_hash, "artifact_id": pin["artifact_id"],
               "candidate_sha256": pin["binary_sha256"], "candidate_bytes": pin["binary_bytes"],
               "expires_at": pin["expires_at"], "run_created_at": current["created_at"]}
    result = validate(approved_records(comments), context)
    for row in result["authorities"].values():
        fresh = derived.fetch_json("issues/comments/" + str(row["id"]), token, deadline)
        require(approved_records([fresh]) == [row], "authority changed during gate read")
    result["source_manifest"] = rows
    result["artifact_id"] = pin["artifact_id"]
    result["run_id"] = int(run_id)
    return result
