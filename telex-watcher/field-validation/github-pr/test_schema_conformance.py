"""Execute Draft 2020-12 schemas and the corpus shared with Rust protocol tests."""

import copy
import datetime
import json
from pathlib import Path
import re
import unittest

from jsonschema import Draft202012Validator, FormatChecker

import github_pr_detector as detector

HERE = Path(__file__).resolve().parent
SCHEMA_DIRECTORY = HERE.parents[2] / "docs" / "design" / "schemas"
SCHEMA_NAMES = {
    "registration": "watcher-registration-v2.schema.json",
    "request": "watcher-detector-request-v2.schema.json",
    "result": "watcher-detector-result-v2.schema.json",
}
RFC3339 = re.compile(
    r"[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])"
    r"[Tt]([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\.[0-9]+)?"
    r"([Zz]|[+-]([01][0-9]|2[0-3]):[0-5][0-9])"
)


def calendar_format_checker():
    checker = FormatChecker()

    # jsonschema's optional format extras are not required. V2 forbids leap
    # seconds lexically; stdlib datetime supplies the independent calendar check.
    @checker.checks("date-time", raises=ValueError)
    def date_time(value):
        if not isinstance(value, str):
            return True
        if not RFC3339.fullmatch(value):
            return False
        datetime.datetime.fromisoformat(value.upper().replace("Z", "+00:00"))
        return True

    return checker


def load_validator(schema_name):
    """Reusable by health/metadata conformance tests; no remote schema fetches."""
    schema = json.loads((SCHEMA_DIRECTORY / schema_name).read_text(encoding="utf-8"))
    if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
        raise AssertionError(f"{schema_name} does not declare Draft 2020-12")
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema, format_checker=calendar_format_checker())


def instantiate_case(templates, case):
    """Apply object-only JSON Pointer changes to a committed concrete template."""
    instance = copy.deepcopy(templates[case["model"]])
    for pointer, value in case.get("set", {}).items():
        parts = [part.replace("~1", "/").replace("~0", "~") for part in pointer[1:].split("/")]
        target = instance
        for part in parts[:-1]:
            target = target[part]
        target[parts[-1]] = copy.deepcopy(value)
    for pointer in case.get("remove", []):
        parts = [part.replace("~1", "/").replace("~0", "~") for part in pointer[1:].split("/")]
        target = instance
        for part in parts[:-1]:
            target = target[part]
        del target[parts[-1]]
    return instance


class SchemaConformanceTests(unittest.TestCase):
    def test_concrete_metadata_and_health_corpus(self):
        corpus = json.loads(
            (HERE.parents[1] / "tests" / "fixtures" / "evidence-v2.json").read_text(encoding="utf-8")
        )
        metadata = load_validator("watcher-event-metadata-v2.schema.json")
        health = load_validator("watcher-health-v2.schema.json")
        metadata.validate(corpus["metadata"])
        for name in ("runtime", "watch"):
            health.validate(corpus[name])
            invalid = copy.deepcopy(corpus[name])
            invalid["retention"]["warningRows"] = 0
            self.assertFalse(health.is_valid(invalid))
            invalid = copy.deepcopy(corpus[name])
            invalid["unrecognizedField"] = True
            self.assertFalse(health.is_valid(invalid))
        invalid = copy.deepcopy(corpus["metadata"])
        invalid["watcher"]["eventSequence"] = 0
        self.assertFalse(metadata.is_valid(invalid))
        invalid = copy.deepcopy(corpus["metadata"])
        invalid["state"] = {"mustNotLeak": True}
        self.assertFalse(metadata.is_valid(invalid))
        invalid = copy.deepcopy(corpus["runtime"])
        invalid["reconciledRegistryRevision"] = None
        self.assertFalse(health.is_valid(invalid))
        invalid["productionReady"] = False
        self.assertTrue(health.is_valid(invalid))

    def test_all_watcher_v2_schemas_are_valid_draft202012(self):
        paths = sorted(SCHEMA_DIRECTORY.glob("watcher-*-v2.schema.json"))
        self.assertTrue(set(SCHEMA_NAMES.values()) <= {path.name for path in paths})
        for path in paths:
            with self.subTest(schema=path.name):
                load_validator(path.name)

    def test_shared_protocol_corpus(self):
        corpus = json.loads((HERE / "protocol-schema-cases.json").read_text(encoding="utf-8"))
        validators = {model: load_validator(name) for model, name in SCHEMA_NAMES.items()}
        names = [case["name"] for case in corpus["cases"]]
        self.assertEqual(len(names), len(set(names)), "case names must be unique")
        for case in corpus["cases"]:
            with self.subTest(case=case["name"]):
                instance = instantiate_case(corpus["templates"], case)
                errors = list(validators[case["model"]].iter_errors(instance))
                self.assertEqual(
                    not errors, case["schemaValid"],
                    "\n".join(error.message for error in errors),
                )

    def test_calendar_format_is_asserted_not_annotation_only(self):
        validator = load_validator(SCHEMA_NAMES["request"])
        instance = json.loads((HERE / "request.json").read_text(encoding="utf-8"))
        instance["attempt"]["now"] = "2025-02-29T00:00:00Z"
        errors = list(validator.iter_errors(instance))
        self.assertTrue(any(error.validator == "format" for error in errors))

    def test_committed_inputs_and_real_detector_results_match_schemas(self):
        registration = json.loads((HERE / "registration.json").read_text(encoding="utf-8"))
        request = json.loads((HERE / "request.json").read_text(encoding="utf-8"))
        load_validator(SCHEMA_NAMES["registration"]).validate(registration)
        load_validator(SCHEMA_NAMES["request"]).validate(request)
        validator = load_validator(SCHEMA_NAMES["result"])
        baseline = detector.evaluate(
            request, detector.decode((HERE / "fixtures" / "pr-open.json").read_bytes())
        )
        validator.validate(baseline)
        request["state"] = baseline["nextState"]
        for fixture in ("pr-open", "pr-changed", "pr-closed", "pr-merged"):
            with self.subTest(fixture=fixture):
                observation = detector.decode((HERE / "fixtures" / (fixture + ".json")).read_bytes())
                validator.validate(detector.evaluate(request, observation))
        request["state"] = None
        request["watch"]["parameters"]["emitInitialSnapshot"] = True
        validator.validate(detector.evaluate(
            request, detector.decode((HERE / "fixtures" / "pr-open.json").read_bytes())
        ))
        validator.validate({"schemaVersion": 2, "outcome": "degraded"})


if __name__ == "__main__":
    unittest.main()
