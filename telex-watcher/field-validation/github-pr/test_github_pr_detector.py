import copy
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import time
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("github_pr_detector", HERE / "github_pr_detector.py")
detector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(detector)


def fixture(name="pr-open"):
    return detector.decode((HERE / "fixtures" / (name + ".json")).read_bytes())


def request(state=None):
    value = detector.decode((HERE / "request.json").read_bytes())
    value["state"] = state
    return value


class GitHubPrDetectorTests(unittest.TestCase):
    def test_initial_baseline_then_unchanged_idle(self):
        first = detector.evaluate(request(), fixture())
        self.assertEqual(first["outcome"], "idle")
        self.assertNotIn("event", first)
        second = detector.evaluate(request(first["nextState"]), fixture())
        self.assertEqual(second, {"schemaVersion": 2, "outcome": "idle"})

    def test_initial_snapshot_is_opt_in(self):
        value = request()
        value["watch"]["parameters"]["emitInitialSnapshot"] = True
        result = detector.evaluate(value, fixture())
        self.assertEqual(result["outcome"], "event")
        self.assertEqual(result["event"]["kind"], "github.pull-request.snapshot")
        self.assertEqual(result["event"]["metadata"]["changed"], [])

    def test_committed_changed_fixture_covers_four_signal_types(self):
        state = detector.evaluate(request(), fixture())["nextState"]
        result = detector.evaluate(request(state), fixture("pr-changed"))
        self.assertEqual(result["outcome"], "event")
        self.assertEqual(result["event"]["metadata"]["changed"], ["checks", "comments", "merge", "reviews"])
        self.assertEqual(
            detector.evaluate(request(result["nextState"]), fixture("pr-changed"))["outcome"],
            "idle",
        )

    def test_individual_changes_are_observed(self):
        state = detector.evaluate(request(), fixture())["nextState"]
        changed = fixture("pr-changed")
        for provider_field, category in [
            ("reviews", "reviews"), ("reviewDecision", "reviews"),
            ("comments", "comments"), ("statusCheckRollup", "checks"),
            ("mergeStateStatus", "merge"), ("mergeable", "merge"),
            ("headRefOid", "head"), ("isDraft", "draft"), ("title", "title"),
        ]:
            with self.subTest(field=provider_field):
                observation = fixture()
                if provider_field == "mergeable":
                    observation[provider_field] = "CONFLICTING"
                elif provider_field == "headRefOid":
                    observation[provider_field] = "2" * 40
                elif provider_field == "isDraft":
                    observation[provider_field] = True
                elif provider_field == "title":
                    observation[provider_field] = "Edited title"
                else:
                    observation[provider_field] = changed[provider_field]
                result = detector.evaluate(request(state), observation)
                self.assertEqual(result["outcome"], "event")
                self.assertEqual(result["event"]["metadata"]["changed"], [category])

    def test_edited_deleted_and_reordered_comments(self):
        observation = fixture("pr-changed")
        observation["comments"].append({"id": "comment-2", "body": "Another comment"})
        state = detector.evaluate(request(), observation)["nextState"]
        reordered = copy.deepcopy(observation)
        reordered["comments"].reverse()
        self.assertEqual(detector.evaluate(request(state), reordered)["outcome"], "idle")
        edited = copy.deepcopy(observation)
        edited["comments"][0]["body"] = "Edited"
        self.assertEqual(detector.evaluate(request(state), edited)["outcome"], "event")
        deleted = copy.deepcopy(observation)
        deleted["comments"].pop()
        self.assertEqual(detector.evaluate(request(state), deleted)["outcome"], "event")

    def test_closed_and_merged_are_terminal_even_on_first_observation(self):
        state = detector.evaluate(request(), fixture())["nextState"]
        for name in ("pr-closed", "pr-merged"):
            for previous in (None, state):
                with self.subTest(fixture=name, initial=previous is None):
                    result = detector.evaluate(request(previous), fixture(name))
                    self.assertEqual(result["outcome"], "terminal")
                    self.assertEqual(result["event"]["kind"], "github.pull-request.terminal")
                    self.assertEqual(result["nextState"]["snapshot"]["state"], name[3:].upper())

    def test_foreign_invalid_cursor_cannot_silently_reset(self):
        state = detector.evaluate(request(), fixture())["nextState"]
        for key, invalid in [
            ("repo", "different/project"), ("pullRequest", 163), ("version", 2),
            ("snapshot", {}), ("extra", True),
        ]:
            value = copy.deepcopy(state)
            value[key] = invalid
            with self.subTest(key=key), self.assertRaises(detector.ObservationError):
                detector.evaluate(request(value), fixture())

    def test_partial_or_potentially_truncated_observation_fails(self):
        for field in ("reviews", "comments", "statusCheckRollup"):
            for count in (100, 101):
                observation = fixture()
                observation[field] = [{"id": str(index)} for index in range(count)]
                with self.assertRaises(detector.ObservationError):
                    detector.evaluate(request(), observation)
            observation = fixture()
            del observation[field]
            with self.assertRaises(detector.ObservationError):
                detector.evaluate(request(), observation)
        observation = fixture()
        observation["number"] = 99
        with self.assertRaises(detector.ObservationError):
            detector.evaluate(request(), observation)

    def test_fixtures_never_launch_gh(self):
        with patch.object(detector, "bounded_command", side_effect=AssertionError("no live query")):
            self.assertEqual(
                detector.load_observation(request()["watch"]["parameters"]), fixture()
            )

    def test_live_query_is_direct_read_only_argv(self):
        parameters = request()["watch"]["parameters"]
        del parameters["fixture"]
        parameters["ghTimeoutSeconds"] = 7
        with patch.object(detector, "bounded_command", return_value=detector.encode(fixture())) as command:
            detector.load_observation(parameters)
        argv, timeout = command.call_args.args
        self.assertEqual(argv, [
            "gh", "pr", "view", "162", "--repo", "example/project", "--json", detector.FIELDS
        ])
        self.assertEqual(timeout, 7)
        for invalid in ("--help", "a/b;echo bad", "a/b/extra"):
            value = request()
            value["watch"]["parameters"]["repo"] = invalid
            with self.assertRaises(detector.ObservationError):
                detector.request_parameters(value)

    def test_strict_request_and_json(self):
        for raw in [
            b'{"a":1,"a":2}', b'{"a":{"x":1,"\\u0078":2}}', b'NaN', b'1e400',
            b'1e-400', b'9007199254740992', b'"\\ud800"', b'"\\uffff"', b"\xff",
        ]:
            with self.subTest(raw=raw), self.assertRaises(detector.ObservationError):
                detector.decode(raw)
        for path, invalid in [
            (("attempt", "now"), "2025-02-29T00:00:00Z"),
            (("attempt", "now"), "2026-01-01T00:00:60Z"),
            (("attempt", "now"), "2026-01-01T00:00:00+00:00"),
            (("attempt", "now"), "2026-01-01T00:00:00.1234Z"),
            (("watch", "parameters", "ghTimeoutSeconds"), 21),
            (("watch", "parameters", "pullRequest"), True),
            (("watch", "parameters", "emitInitialSnapshot"), "yes"),
            (("watch", "parameters", "command"), "mutation"),
        ]:
            value = request()
            target = value
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = invalid
            with self.subTest(path=path, invalid=invalid), self.assertRaises(detector.ObservationError):
                detector.request_parameters(value)

    def test_result_contains_no_detector_owned_routing_identity_or_action(self):
        state = detector.evaluate(request(), fixture())["nextState"]
        result = detector.evaluate(request(state), fixture("pr-changed"))
        self.assertEqual(set(result), {"schemaVersion", "outcome", "nextState", "event"})
        self.assertEqual(set(result["event"]), {"kind", "subject", "body", "metadata"})

    def test_bounded_command_success_failure_timeout_and_both_output_caps(self):
        python = sys.executable
        self.assertEqual(
            detector.bounded_command([python, "-c", "print('ok', end='')"], 3),
            b"ok",
        )
        for code, timeout, stdout_cap, stderr_cap in [
            ("import sys; sys.exit(3)", 3, 1024, 1024),
            ("import time; time.sleep(10)", 0.1, 1024, 1024),
            ("import sys; sys.stdout.write('x' * 20000)", 3, 100, 1024),
            ("import sys; sys.stderr.write('x' * 20000)", 3, 1024, 100),
        ]:
            started = time.monotonic()
            with self.subTest(code=code), self.assertRaises(detector.ObservationError):
                detector.bounded_command([python, "-c", code], timeout, stdout_cap, stderr_cap)
            self.assertLess(time.monotonic() - started, 5)

    def test_cli_fixture_protocol_and_degradation_do_not_require_gh(self):
        for payload, outcome in [
            (detector.encode(request()), "idle"),
            (b'{"not":"a-request"}', "degraded"),
        ]:
            process = subprocess.run(
                [sys.executable, "-B", str(HERE / "github_pr_detector.py")],
                input=payload, capture_output=True, timeout=5, check=True,
            )
            result = detector.decode(process.stdout)
            self.assertEqual(result["outcome"], outcome)
            self.assertLessEqual(len(process.stderr), 128)
            if outcome == "degraded":
                self.assertEqual(result, {"schemaVersion": 2, "outcome": "degraded"})

    def test_failed_observation_emits_only_degraded_without_sensitive_diagnostics(self):
        class BinaryText:
            def __init__(self, contents=b""):
                self.buffer = io.BytesIO(contents)
        stdin = BinaryText(detector.encode(request()))
        stdout = BinaryText()
        stderr = io.StringIO()
        with (
            patch.object(detector.sys, "argv", ["github_pr_detector.py"]),
            patch.object(detector.sys, "stdin", stdin),
            patch.object(detector.sys, "stdout", stdout),
            patch.object(detector.sys, "stderr", stderr),
            patch.object(detector, "load_observation", side_effect=detector.ObservationError("SECRET")),
        ):
            self.assertEqual(detector.main(), 0)
        self.assertEqual(detector.decode(stdout.buffer.getvalue()), {"schemaVersion": 2, "outcome": "degraded"})
        self.assertNotIn("SECRET", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
