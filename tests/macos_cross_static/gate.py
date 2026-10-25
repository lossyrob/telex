"""Read-only issue freeze and failed-job-only retry checks. Never launches a job."""

import hashlib
import json
import os
from pathlib import Path
import re
import urllib.request


MARKER = re.compile(r"<!-- arm24-intel25-static-v1 (\{[^\r\n]*\}) -->")


def require(condition, reason):
    if not condition:
        raise RuntimeError(reason)


def approved_records(comments):
    result = []
    for comment in comments:
        if comment.get("user", {}).get("login") != "lossyrob":
            continue
        for match in MARKER.finditer(comment.get("body", "")):
            result.append({"comment": comment["html_url"], **json.loads(match[1])})
    return result


def validate(records, context, previous=None):
    expected = {"component": "arm24-intel25-static-v1", "sha": context["sha"],
                "bounds_sha256": context["bounds_sha256"], "jobs": ["ARM24", "Intel25"],
                "allow_launch": True}
    matches = [r for r in records if all(r.get(k) == v for k, v in expected.items())]
    require(len(matches) == 1, "missing/ambiguous exact-SHA issue freeze; do not launch")
    frozen = matches[0]
    for field in ("decoder_pin_disposition", "decoder_consumer_disposition", "intel_family_disposition"):
        require(isinstance(frozen.get(field), str) and re.fullmatch(
            r"https://github\.com/lossyrob/telex/issues/172#issuecomment-[0-9]+", frozen[field]),
            "unresolved prelaunch disposition: " + field)
    require(context["job"] in expected["jobs"] and context["attempt"] in (1, 2),
            "job or retry budget mismatch")
    if context["attempt"] == 1:
        return {"freeze": frozen["comment"], "attempt": 1, "retry": False,
                "decoder_pin_disposition": frozen["decoder_pin_disposition"],
                "decoder_consumer_disposition": frozen["decoder_consumer_disposition"],
                "intel_family_disposition": frozen["intel_family_disposition"]}
    require(previous is not None and previous["head_sha"] == context["sha"] and
            previous["run_attempt"] == 1, "retry original attempt/SHA mismatch")
    jobs = previous["jobs"]
    own = [j for j in jobs if j["name"] == context["job"]]
    other = [j for j in jobs if j["name"] != context["job"]]
    require(len(jobs) == 2 and len(own) == len(other) == 1 and
            own[0]["conclusion"] in ("failure", "cancelled", "timed_out") and
            other[0]["conclusion"] in ("success", "failure", "cancelled", "timed_out"),
            "retry must select a failed job from a completed two-job attempt")
    retry = frozen.get("infrastructure_retry")
    require(isinstance(retry, dict) and retry.get("run_id") == context["run_id"] and
            retry.get("job") == context["job"] and retry.get("failed_job_id") == own[0]["id"] and
            retry.get("same_sha") == context["sha"] and retry.get("classification") == "infrastructure" and
            isinstance(retry.get("reason"), str) and 20 <= len(retry["reason"]) <= 2000,
            "retry requires exact failed-job infrastructure disposition in frozen issue record")
    return {"freeze": frozen["comment"], "attempt": 2, "retry": retry}


def fetch(endpoint):
    request = urllib.request.Request("https://api.github.com/repos/lossyrob/telex/" + endpoint,
                                    headers={"Authorization": "Bearer " + os.environ["GH_TOKEN"],
                                             "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        raw = response.read(4 * 1024 * 1024 + 1)
    require(len(raw) <= 4 * 1024 * 1024, "bounded GitHub read exceeded")
    return json.loads(raw)


def check(job):
    config = Path(__file__).with_name("bounds.json").read_bytes()
    context = {"sha": os.environ["GITHUB_SHA"], "job": job,
               "attempt": int(os.environ["GITHUB_RUN_ATTEMPT"]),
               "run_id": int(os.environ["GITHUB_RUN_ID"]),
               "bounds_sha256": hashlib.sha256(config).hexdigest()}
    require(os.environ.get("GITHUB_EVENT_NAME") == "push", "only frozen push run permitted")
    comments = []
    for page in range(1, 11):
        batch = fetch(f"issues/172/comments?per_page=100&page={page}")
        comments.extend(batch)
        if len(batch) < 100:
            break
    else:
        raise RuntimeError("issue comment read bound exhausted")
    prior = None
    if context["attempt"] == 2:
        prior = fetch(f"actions/runs/{context['run_id']}/attempts/1")
        prior["jobs"] = fetch(f"actions/runs/{context['run_id']}/attempts/1/jobs?per_page=100")["jobs"]
    records = approved_records(comments)
    result = validate(records, context, prior)
    selected = next(r for r in records if r.get("comment") == result["freeze"])
    disposition_receipts = {}
    for field in ("decoder_pin_disposition", "decoder_consumer_disposition", "intel_family_disposition"):
        identifier = selected[field].rsplit("-", 1)[1]
        comment = fetch("issues/comments/" + identifier)
        require(comment.get("user", {}).get("login") == "lossyrob" and
                comment.get("html_url") == selected[field] and
                comment.get("issue_url", "").rsplit("/", 1)[-1] == "172",
                "prelaunch disposition reference is not an owner-routed issue172 comment")
        disposition_receipts[field] = {"url": selected[field],
            "body_sha256": hashlib.sha256(comment.get("body", "").encode()).hexdigest()}
    # The owner is responsible for semantic adjudication of these linked dispositions.
    result["referenced_dispositions"] = disposition_receipts
    return result
