"""Host-first preparation. Import is inert; documentation and HFR accounting are data-only."""

import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import uuid


HERE = Path(__file__).resolve().parent
WORKFLOW = ".github/workflows/macos-cross-static.yml"
HOST_FIELDS = ("architecture", "product", "build", "kern_osversion", "kern_uuid")
EFFECTS = ("candidate_acquisition", "artifact_acquisition", "apple_transfer",
           "decoder", "tool_staging", "native_resolution", "product")
PREFIX = ("checkout", "offline-tests", "host-first")
RECORD_KEYS = {
    "schema", "status", "expected", "observed", "commands", "documentation",
    "workflow_sha256", "steps_before_component", "source_sha", "run_id", "run_attempt", "job",
    "image_environment_context", "identity_precision", "effects", "effect_phase",
    "consumption", "authorizes_launch",
}
HOST_COMMANDS = (
    ("architecture", ("/usr/bin/uname", "-m")),
    ("product", ("/usr/bin/sw_vers", "-productVersion")),
    ("build", ("/usr/bin/sw_vers", "-buildVersion")),
    ("kern_osversion", ("/usr/sbin/sysctl", "-n", "kern.osversion")),
    ("kern_uuid", ("/usr/sbin/sysctl", "-n", "kern.uuid")),
)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=pairs,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError("nonfinite JSON")))


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def encoded(value):
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()


def instant(value):
    require(isinstance(value, str) and value.endswith("Z"), "UTC timestamp required")
    return datetime.datetime.fromisoformat(value[:-1] + "+00:00").timestamp()


def binding(policy):
    require(set(policy) == {"schema", "binding", "documentation", "documents", "status"} and
            policy["schema"] == "host-first-preparation-v1", "unknown host-first policy")
    value = policy["binding"]
    require(isinstance(value, dict), "host binding is unselected; preparation is not launch authority")
    require(set(value) == {"label", *HOST_FIELDS}, "incomplete coherent host observation binding")
    require(all(isinstance(v, str) and v and v.strip() == v for v in value.values()),
            "invalid host binding field")
    require(value["architecture"] in ("arm64", "x86_64") and
            value["build"] == value["kern_osversion"], "inconsistent host binding")
    require(str(uuid.UUID(value["kern_uuid"])).upper() == value["kern_uuid"], "canonical kernel UUID required")
    return value


def documents_from_policy(policy):
    value = policy["documents"]
    require(isinstance(value, dict) and set(value) == {"readme_utf8", "index_utf8", "acquisition"},
            "pinned documentation bytes are unselected")
    require(all(isinstance(value[k], str) and len(value[k].encode()) <= 65536
                for k in ("readme_utf8", "index_utf8")), "documentation byte bound")
    return {"readme": value["readme_utf8"].encode(), "index": value["index_utf8"].encode(),
            "acquisition": value["acquisition"]}


def documentation_gate(readme, index, specification, expected, now, acquisition):
    """Read pinned local runner-images bytes; never fetch or select a label."""
    keys = {"revision", "readme_path", "readme_sha256", "index_sha256",
            "observed_at", "maximum_age_seconds"}
    require(isinstance(specification, dict) and set(specification) == keys, "missing documentation metadata")
    require(re.fullmatch(r"[0-9a-f]{40}", specification["revision"]) is not None, "invalid docs revision")
    require(isinstance(acquisition, dict) and set(acquisition) ==
            {"revision", "readme_path", "readme_sha256", "index_sha256", "observed_at"} and
            all(acquisition[key] == specification[key] for key in acquisition),
            "documentation acquisition revision/path/time differs from frozen pins")
    require(specification["readme_path"].startswith("images/macos/") and
            re.fullmatch(r"images/macos/[A-Za-z0-9-]+-Readme\.md", specification["readme_path"]),
            "invalid runner documentation path")
    require(digest(readme) == specification["readme_sha256"] and
            digest(index) == specification["index_sha256"], "documentation pin mismatch")
    age = now - instant(specification["observed_at"])
    maximum = specification["maximum_age_seconds"]
    require(type(maximum) is int and maximum > 0 and 0 <= age <= maximum, "stale/future documentation")
    text, table = readme.decode("utf-8"), index.decode("utf-8")
    values = {}
    for key, pattern in (
        ("product_build", r"^- OS Version: macOS ([0-9.]+) \(([A-Za-z0-9]+)\)\r?$"),
        ("image", r"^- Image Version: ([A-Za-z0-9.]+)\r?$"),
    ):
        matches = re.findall(pattern, text, re.M)
        require(len(matches) == 1, "missing/ambiguous documented " + key)
        values[key] = matches[0]
    matches = []
    for row in table.splitlines():
        cells = row.split("|")
        if len(cells) == 6 and expected["label"] in re.findall(r"`([^`]+)`", cells[3]):
            matches.append(cells)
    require(len(matches) == 1, "wrong/ambiguous documented label")
    row = matches[0]
    architecture = {"arm64": "arm64", "x64": "x86_64"}.get(row[2].strip())
    require(architecture == expected["architecture"], "documented label architecture mismatch")
    links = re.findall(r"\[([^\]]+)\]", row[4])
    require(len(links) == 1, "ambiguous label documentation link")
    definitions = re.findall(r"^\[" + re.escape(links[0]) + r"\]: (\S+)\r?$", table, re.M)
    require(len(definitions) == 1 and definitions[0] ==
            "https://github.com/actions/runner-images/blob/main/" + specification["readme_path"],
            "label does not resolve to pinned documentation")
    require(values["product_build"] == (expected["product"], expected["build"]),
            "documented host contradicts frozen decoder host")
    return {"schema": "documentation-consistency-v1", "label": expected["label"],
            "architecture": architecture, "product": expected["product"], "build": expected["build"],
            "image_version_context": values["image"], **specification,
            "checked_at": datetime.datetime.fromtimestamp(now, datetime.timezone.utc).isoformat().replace("+00:00", "Z"),
            "status": "documentation-consistent; live allocation unverified", "authorizes_launch": False}


def workflow_prefix(raw):
    """Fail closed on changes to the deliberately tiny pre-host workflow grammar."""
    text = raw.decode("utf-8").replace("\r\n", "\n")
    before = text.split("      - name: Frozen Intel25 static processing", 1)
    require(len(before) == 2, "component step boundary missing")
    prefix = before[0].split("    steps:\n", 1)
    require(len(prefix) == 2, "workflow step boundary missing")
    # Exact blocks prevent an extra env/run/uses/condition from masquerading as an allowed step.
    expected = """      - uses: actions/checkout@v4
        with:
          persist-credentials: false
      - name: Offline source tests
        env:
          PYTHONDONTWRITEBYTECODE: "1"
        run: python3 tests/macos_cross_static/offline_tests.py
      - name: Host-first observation
        id: host
        env:
          PYTHONDONTWRITEBYTECODE: "1"
        run: python3 tests/macos_cross_static/host_first.py "$RUNNER_TEMP/cross-intel25-host"
"""
    require(prefix[1] == expected, "unreviewed step before host-first boundary")
    require(text.count("    steps:\n") == 1 and
            re.findall(r"^  ([A-Za-z0-9_-]+):$", text.split("jobs:\n", 1)[1], re.M) == ["Intel25"] and
            "matrix:" not in text and "workflow_dispatch" not in text,
            "only one Intel25 push job is permitted")
    return list(PREFIX)


def observe(expected, environment, command, source_root):
    """Only these bounded identity observations are permitted before acceptance."""
    records, observed = [], {}
    commands = HOST_COMMANDS + (("source_sha", ("/usr/bin/git", "-C", str(source_root), "rev-parse", "HEAD")),)
    for name, argv in commands:
        result = command(list(argv))
        require(set(result) == {"rc", "stdout", "stderr"} and type(result["rc"]) is int and
                isinstance(result["stdout"], str) and isinstance(result["stderr"], str),
                "invalid host command observation")
        records.append({"field": name, "argv": list(argv), **result})
        require(result["rc"] == 0 and not result["stderr"] and len(result["stdout"].encode()) <= 4096,
                "host observation unavailable")
        value = result["stdout"].strip()
        require(value and "\n" not in value and "\r" not in value, "malformed host observation")
        observed[name] = value
    require(re.fullmatch(r"[0-9a-f]{40}", observed["source_sha"]) is not None and
            observed["source_sha"] == environment["GITHUB_SHA"], "host source mismatch")
    observed["kern_uuid"] = str(uuid.UUID(observed["kern_uuid"])).upper()
    return observed, records


def host_record(policy, documents, environment, command, source_root, workflow, now):
    expected = binding(policy)
    require(documents == documents_from_policy(policy), "documentation bytes differ from source-bound policy")
    require(environment.get("GITHUB_RUN_ATTEMPT") == "1" and
            environment.get("GITHUB_JOB") == "Intel25" and
            environment.get("GITHUB_EVENT_NAME") == "push" and
            environment.get("GITHUB_REPOSITORY") == "lossyrob/telex",
            "first-attempt single Intel25 push context required")
    require(re.fullmatch(r"[0-9a-f]{40}", environment.get("GITHUB_SHA", "")) and
            re.fullmatch(r"[1-9][0-9]*", environment.get("GITHUB_RUN_ID", "")), "invalid run identity")
    steps = workflow_prefix(workflow)
    require(re.findall(r"^    runs-on: ([^\r\n]+)\r?$", workflow.decode(), re.M) == [expected["label"]],
            "workflow label differs from reviewed host binding")
    docs = documentation_gate(documents["readme"], documents["index"], policy["documentation"], expected, now,
                              documents["acquisition"])
    observed, commands = observe(expected, environment, command, source_root)
    matches = all(observed[key] == expected[key] for key in HOST_FIELDS)
    return {
        "schema": "host-first-observation-v1",
        "status": "host-accepted" if matches else "host-first-refusal",
        "expected": expected, "observed": observed, "commands": commands,
        "documentation": docs, "workflow_sha256": digest(workflow), "steps_before_component": steps,
        "source_sha": environment["GITHUB_SHA"], "run_id": int(environment["GITHUB_RUN_ID"]),
        "run_attempt": 1, "job": "Intel25",
        "image_environment_context": {k: environment.get(k) for k in ("ImageOS", "ImageVersion")},
        "identity_precision": "Same-job non-adversarial provider identity observation; image environment is context, not security attestation.",
        "effects": dict.fromkeys(EFFECTS, 0), "effect_phase": "before-component",
        "consumption": "UNADJUDICATED: only a separately authenticated future finite with HFR terms can grant non-consuming refusal",
        "authorizes_launch": False,
    }


def evidence_manifest(files):
    return {name: {"bytes": len(raw), "sha256": digest(raw)} for name, raw in sorted(files.items())}


def verify_evidence(files, manifest, expected_source, expected_run):
    require(set(files) == {"host-first.json", "workflow.yml", "policy.json",
                           "runner-readme.md", "runner-index.md", "documentation-source.json"},
            "unexpected/missing pre-host evidence")
    require(manifest == evidence_manifest(files), "pre-host evidence manifest mismatch")
    record = strict_json(files["host-first.json"])
    policy = strict_json(files["policy.json"])
    require(isinstance(record, dict) and set(record) == RECORD_KEYS and
            record["schema"] == "host-first-observation-v1", "unknown host observation schema")
    require(set(record["observed"]) == {*HOST_FIELDS, "source_sha"} and
            all(isinstance(value, str) and value for value in record["observed"].values()),
            "incomplete host identity observation")
    require(record["source_sha"] == expected_source and record["observed"]["source_sha"] == expected_source and
            type(record["run_id"]) is int and record["run_id"] == expected_run and
            type(record["run_attempt"]) is int and record["run_attempt"] == 1 and record["job"] == "Intel25",
            "cross-job/source or retry host evidence")
    require(record["expected"] == binding(policy), "host expectation not bound to policy")
    require(record["steps_before_component"] == workflow_prefix(files["workflow.yml"]) and
            record["workflow_sha256"] == digest(files["workflow.yml"]), "workflow proof mismatch")
    require(re.findall(r"^    runs-on: ([^\r\n]+)\r?$", files["workflow.yml"].decode(), re.M) ==
            [record["expected"]["label"]], "workflow/binding label mismatch")
    docs = record["documentation"]
    checked = documentation_gate(files["runner-readme.md"], files["runner-index.md"], policy["documentation"],
                                 record["expected"], instant(docs["checked_at"]),
                                 strict_json(files["documentation-source.json"]))
    require(checked == docs, "documentation observation mismatch")
    require(documents_from_policy(policy) == {
        "readme": files["runner-readme.md"], "index": files["runner-index.md"],
        "acquisition": strict_json(files["documentation-source.json"])},
        "manifest documentation differs from source-bound policy")
    require(set(record["effects"]) == set(EFFECTS) and
            all(type(n) is int and n == 0 for n in record["effects"].values()) and
            record["effect_phase"] == "before-component", "pre-host consuming effect; refusal consumed")
    require(record["authorizes_launch"] is False, "host observation is not authority")
    expected_commands = list(HOST_COMMANDS)
    require(len(record["commands"]) == 6, "incomplete host commands")
    for index, row in enumerate(record["commands"]):
        field = HOST_FIELDS[index] if index < 5 else "source_sha"
        require(set(row) == {"field", "argv", "rc", "stdout", "stderr"} and
                row["field"] == field and row["rc"] == 0 and type(row["rc"]) is int and
                row["stderr"] == "" and isinstance(row["stdout"], str) and len(row["stdout"].encode()) <= 4096,
                "failed/incomplete host evidence")
        if index < 5:
            require(row["argv"] == list(expected_commands[index][1]), "unexpected host command")
        else:
            argv = row["argv"]
            require(len(argv) == 5 and argv[:2] == ["/usr/bin/git", "-C"] and
                    argv[3:] == ["rev-parse", "HEAD"] and Path(argv[2]).is_absolute(),
                    "unexpected source observation command")
        value = row["stdout"].strip()
        if field == "kern_uuid":
            value = str(uuid.UUID(value)).upper()
        require(value == record["observed"][field], "observation does not match command result")
    matched = all(record["observed"][k] == record["expected"][k] for k in HOST_FIELDS)
    require(record["status"] == ("host-accepted" if matched else "host-first-refusal"), "false host status")
    return record


def classify_refusal(files, manifest, source_sha, run_id, terms, previous_refusals, job_steps):
    """Offline proposed accounting only. Caller must independently authenticate the future finite."""
    record = verify_evidence(files, manifest, source_sha, run_id)
    require(job_steps == [
        {"step": "checkout", "conclusion": "success"},
        {"step": "offline-tests", "conclusion": "success"},
        {"step": "host-first", "conclusion": "failure"},
        {"step": "component", "conclusion": "skipped"},
    ], "missing/nonzero-effect job step proof; refusal is not proven non-consuming")
    require(record["status"] == "host-first-refusal", "not a host refusal")
    require(isinstance(terms, dict) and set(terms) ==
            {"schema", "finite_comment_id", "finite_body_sha256", "source_sha", "hfr_cap",
             "one_consuming_attempt", "retries", "every_dispatch_owner_directed", "effective"},
            "missing explicit future-finite HFR terms")
    require(terms["schema"] == "future-finite-hfr-v1" and terms["effective"] is True and
            type(terms["finite_comment_id"]) is int and terms["finite_comment_id"] > 6804129659 and
            re.fullmatch(r"[0-9a-f]{64}", terms["finite_body_sha256"]), "historical/template finite not effective")
    require(terms["source_sha"] == record["source_sha"] and
            terms["source_sha"] not in ("3a0d88d0320507660096a1e0662b2d770fe9d37d",
                                       "3d3d0a3cfe29af14dd2105e37d6043857ba4d646",
                                       "a06a0a389a45d1bfd075f32f72387512dfce2341"), "HFR is not retroactive")
    require(type(terms["hfr_cap"]) is int and 1 <= terms["hfr_cap"] <= 2 and
            type(previous_refusals) is int and 0 <= previous_refusals < terms["hfr_cap"],
            "HFR finite cap reached/invalid; return to steward")
    require(type(terms["one_consuming_attempt"]) is int and terms["one_consuming_attempt"] == 1 and
            type(terms["retries"]) is int and terms["retries"] == 0 and
            terms["every_dispatch_owner_directed"] is True, "HFR cannot authorize retries/automatic dispatch")
    require(set(record["effects"]) == set(EFFECTS) and
            all(type(v) is int and v == 0 for v in record["effects"].values()),
            "effect before refusal consumes attempt")
    count = previous_refusals + 1
    return {"proposed_classification": "non-consuming-host-refusal", "refusals": count,
            "finite_ended": count == terms["hfr_cap"], "automatic_dispatch": False,
            "requires_authenticated_finite_and_manifest": True}


def push_effect_record(command_target_argument, before, after, run):
    require(isinstance(command_target_argument, str) and command_target_argument, "missing command target argument")
    require(all(re.fullmatch(r"[0-9a-f]{40}", sha) for sha in (before, after)), "invalid remote ref evidence")
    require(set(run) == {"head_sha", "event", "workflow", "jobs", "run_count", "run_attempt", "actor"} and
            run["head_sha"] == after and run["event"] == "push" and run["workflow"] == WORKFLOW and
            run["jobs"] == ["Intel25"] and type(run["run_count"]) is int and run["run_count"] == 1 and
            type(run["run_attempt"]) is int and run["run_attempt"] == 1,
            "unexpected push effect")
    require(isinstance(run["actor"], dict) and set(run["actor"]) == {"login", "id"} and
            isinstance(run["actor"]["login"], str) and run["actor"]["login"] and
            type(run["actor"]["id"]) is int and run["actor"]["id"] > 0, "missing observable run actor")
    return {"command_target_argument": command_target_argument, "remote_ref_before": before,
            "remote_ref_after": after, "run_effect": run,
            "effective_transport": "not attested", "git_credential_principal": "not attested"}


def accepted_host(directory, source, run_id, bounds):
    paths = list(directory.iterdir())
    require(len(paths) == 7 and all(p.is_file() and not p.is_symlink() and p.stat().st_size <= 262144
                                   for p in paths), "invalid bounded host evidence directory")
    files = {p.name: p.read_bytes() for p in paths if p.name != "sha256.json"}
    record = verify_evidence(files, strict_json((directory / "sha256.json").read_bytes()), source, run_id)
    require(files["policy.json"] == (HERE / "host_first_policy.json").read_bytes() and
            files["workflow.yml"] == (HERE.parents[1] / WORKFLOW).read_bytes(),
            "same-job host evidence differs from current reviewed source")
    require(record["status"] == "host-accepted", "component requires same-job accepted host evidence")
    expected = record["expected"]
    require(all(expected[k] == bounds["runner"][v] for k, v in
                (("label", "label"), ("architecture", "architecture"), ("product", "product_version"), ("build", "build"))),
            "host-first policy and coherent decoder binding disagree")
    return record


def docs_main(argv):
    require(len(argv) == 5, "usage: --docs ABS_POLICY ABS_README ABS_INDEX ABS_ACQUISITION ABS_OUTPUT")
    paths = [Path(p) for p in argv]
    require(all(p.is_absolute() for p in paths), "absolute documentation reader paths required")
    policy_path, readme, index, acquisition, output = paths
    policy = strict_json(policy_path.read_bytes())
    value = documentation_gate(readme.read_bytes(), index.read_bytes(), policy["documentation"], binding(policy),
                               datetime.datetime.now(datetime.timezone.utc).timestamp(),
                               strict_json(acquisition.read_bytes()))
    with output.open("xb") as target:
        target.write(encoded(value))
    return 0


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv and argv[0] == "--docs":
        return docs_main(argv[1:])
    require(len(argv) == 1, "usage: host_first.py ABS_EVIDENCE_DIRECTORY")
    output = Path(argv[0])
    require(output.is_absolute() and not output.exists(), "fresh absolute host evidence directory required")
    output.mkdir()
    try:
        policy_raw = (HERE / "host_first_policy.json").read_bytes()
        policy = strict_json(policy_raw)
        binding(policy)  # R2 is deliberately unbound: do not observe a host or imply HFR authority.
        require(sys.platform == "darwin", "host observation is native macOS only")
        documents = documents_from_policy(policy)
        workflow = (HERE.parents[1] / WORKFLOW).read_bytes()
        def command(args):
            result = subprocess.run(args, capture_output=True, text=True, timeout=15, check=False)
            return {"rc": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
        record = host_record(policy, documents, os.environ, command, HERE.parents[1], workflow,
                             datetime.datetime.now(datetime.timezone.utc).timestamp())
        files = {"host-first.json": encoded(record), "workflow.yml": workflow, "policy.json": policy_raw,
                 "runner-readme.md": documents["readme"], "runner-index.md": documents["index"],
                 "documentation-source.json": encoded(documents["acquisition"])}
        for name, raw in files.items():
            (output / name).write_bytes(raw)
        (output / "sha256.json").write_bytes(encoded(evidence_manifest(files)))
        return 0 if record["status"] == "host-accepted" else 1
    except (RuntimeError, ValueError, OSError, subprocess.SubprocessError) as error:
        (output / "preparation-stop.json").write_bytes(encoded({
            "status": "preparation-or-observation-failed", "error": str(error),
            "non_consuming_HFR_claimed": False, "authorizes_launch": False}))
        raise


if __name__ == "__main__":
    sys.exit(main())
