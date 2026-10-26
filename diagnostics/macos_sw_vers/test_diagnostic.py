"""Synthetic only: no provider executable, FFI, selected input or network request."""

from contextlib import contextmanager
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("swvers_diagnostic", HERE / "diagnostic.py")
diag = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(diag)
BOUNDS = json.loads((HERE / "bounds.json").read_bytes())
SHA = "1" * 40
JOB = BOUNDS["jobs"][0]


def context_environment():
    return {"GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/heads/" + BOUNDS["branch"],
            "GITHUB_REPOSITORY": "lossyrob/telex", "GITHUB_RUN_ATTEMPT": "1", "GITHUB_SHA": SHA,
            "GITHUB_WORKFLOW_REF": "lossyrob/telex/" + BOUNDS["workflow"] + "@refs/heads/" + BOUNDS["branch"],
            "GH_TOKEN": "synthetic-not-a-credential"}


def expected_context():
    return diag.gate_context(context_environment(), BOUNDS, "a" * 64, "b" * 64, JOB)


def frozen_comment(record=None):
    record = dict(record or expected_context())
    record.update(source_review="https://github.com/lossyrob/telex/issues/172#issuecomment-123",
                  source_review_body_sha256=diag.digest(b"synthetic approval"))
    return {"user": {"login": "lossyrob"}, "html_url": "https://github.com/lossyrob/telex/issues/172#issuecomment-456",
            "body": "<!-- macos-sw-vers-diagnostic-v1 " + json.dumps(record) + " -->"}


class GateTests(unittest.TestCase):
    def test_unique_exact_diagnostic_freeze(self):
        comment = frozen_comment()
        self.assertEqual(diag.validate_freeze([comment], expected_context(), BOUNDS)[0], comment)

    def test_full_component_or_other_source_freeze_cannot_authorize(self):
        full = frozen_comment()
        full["body"] = full["body"].replace("macos-sw-vers-diagnostic-v1", "arm24-intel25-static-v1")
        other = expected_context()
        other["sha"] = "2" * 40
        for comments in ([], [full], [frozen_comment(other)]):
            with self.subTest(comments=comments), self.assertRaises(diag.DiagnosticError):
                diag.validate_freeze(comments, expected_context(), BOUNDS)

    def test_duplicate_same_sha_records_fail_even_if_one_is_disabled(self):
        disabled = expected_context()
        disabled["allow_launch"] = False
        for comments in ([frozen_comment(), frozen_comment()], [frozen_comment(), frozen_comment(disabled)]):
            with self.subTest(comments=comments), self.assertRaises(diag.DiagnosticError):
                diag.validate_freeze(comments, expected_context(), BOUNDS)

    def test_wrong_bounds_closure_jobs_attempt_and_false_marker(self):
        for key, value in (("bounds_sha256", "0" * 64), ("source_manifest_sha256", "0" * 64),
                           ("jobs", list(reversed(BOUNDS["jobs"]))), ("attempt", 2), ("attempt", True),
                           ("allow_launch", False), ("allow_launch", 1)):
            record = expected_context()
            record[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(diag.DiagnosticError):
                diag.validate_freeze([frozen_comment(record)], expected_context(), BOUNDS)

    def test_foreign_author_and_missing_review_fail(self):
        wrong = frozen_comment()
        wrong["user"]["login"] = "someone-else"
        with self.assertRaises(diag.DiagnosticError):
            diag.validate_freeze([wrong], expected_context(), BOUNDS)
        missing = expected_context()
        comment = frozen_comment()
        comment["body"] = "<!-- macos-sw-vers-diagnostic-v1 " + json.dumps(missing) + " -->"
        with self.assertRaises(diag.DiagnosticError):
            diag.validate_freeze([comment], expected_context(), BOUNDS)

    def test_context_requires_exact_push_branch_repo_workflow_and_attempt1(self):
        for key, value in (("GITHUB_EVENT_NAME", "workflow_dispatch"), ("GITHUB_REF", "refs/heads/main"),
                           ("GITHUB_REPOSITORY", "other/repo"), ("GITHUB_RUN_ATTEMPT", "2"),
                           ("GITHUB_SHA", "wrong"), ("GITHUB_WORKFLOW_REF", "wrong")):
            env = context_environment()
            env[key] = value
            with self.subTest(key=key), self.assertRaises(diag.DiagnosticError):
                diag.gate_context(env, BOUNDS, "a" * 64, "b" * 64, JOB)

    def test_gate_pins_review_body_and_preserves_receipt_before_provider(self):
        review = {"user": {"login": "lossyrob"},
            "html_url": "https://github.com/lossyrob/telex/issues/172#issuecomment-123",
            "issue_url": "https://api.github.com/repos/lossyrob/telex/issues/172", "body": "synthetic approval"}
        with tempfile.TemporaryDirectory() as temp:
            output = diag.Output(Path(temp), BOUNDS["limits"])
            with patch.object(diag, "fetch_json", side_effect=[[frozen_comment()], review]), \
                    patch.object(diag.subprocess, "Popen") as spawn:
                diag.check_gate(expected_context(), BOUNDS, context_environment(), output, 100)
            spawn.assert_not_called()
            receipt = json.loads((Path(temp) / "gate-evidence.json").read_bytes())
            self.assertFalse(receipt["provider_query_performed"])
        review["body"] += "changed"
        with tempfile.TemporaryDirectory() as temp, \
                patch.object(diag, "fetch_json", side_effect=[[frozen_comment()], review]), \
                self.assertRaises(diag.DiagnosticError):
            diag.check_gate(expected_context(), BOUNDS, context_environment(),
                            diag.Output(Path(temp), BOUNDS["limits"]), 100)

    def test_gate_page_bound_does_not_become_absence_or_retry(self):
        bounds = copy.deepcopy(BOUNDS)
        bounds["limits"]["github_comment_pages"] = 2
        with patch.object(diag, "fetch_json", return_value=[{}] * 100) as fetch, self.assertRaises(diag.DiagnosticError):
            diag.check_gate(expected_context(), bounds, context_environment(), None, 100)
        self.assertEqual(fetch.call_count, 2)

    def test_bounded_GitHub_read_rejects_overflow_redirect_and_deadline_without_retry(self):
        limits = dict(BOUNDS["limits"], github_response_bytes=8)
        url = "https://api.github.com/repos/lossyrob/telex/issues/172"
        for chunks, redirected, times in (([b"123456789"], url, [0, 0, 0]),
                                         ([b"{}"], "https://example.invalid/", [0, 0, 0]),
                                         ([b"{}"], url, [0, 20, 20])):
            with self.subTest(chunks=chunks, redirected=redirected, times=times):
                response = SimpleNamespace(geturl=lambda: redirected)
                response.read1 = lambda size: chunks.pop(0) if chunks else b""
                @contextmanager
                def opened(*args, **kwargs):
                    yield response
                with patch.object(diag.urllib.request, "urlopen", side_effect=opened) as fetch, \
                        patch.object(diag.time, "monotonic", side_effect=times), \
                        self.assertRaises(diag.DiagnosticError):
                    diag.fetch_json("issues/172", "synthetic", limits, 10)
                self.assertEqual(fetch.call_count, 1)


class EnvironmentTests(unittest.TestCase):
    def test_decoder_environment_exact_additions_and_inheritance(self):
        inherited = {"PATH": "/synthetic/path", "UNCHANGED": "value", "NO_COLOR": "old"}
        work = Path("/synthetic/private")
        checked = diag.checked_environment(inherited, BOUNDS, work)
        tracing = diag.decoder_environment(checked, BOUNDS)
        self.assertNotIn("DYLD_PRINT_LIBRARIES", checked)
        self.assertEqual(tracing, {**checked, "DYLD_PRINT_LIBRARIES": "1", "NO_COLOR": "1"})
        self.assertEqual(tracing["UNCHANGED"], "value")
        self.assertEqual({key: tracing[key] for key in BOUNDS["environment"]}, BOUNDS["environment"])
        self.assertEqual(tracing["TELEX_DB"], "/synthetic/private/store.sqlite")
        self.assertEqual(inherited["NO_COLOR"], "old")

    def test_loader_search_insertion_root_cache_and_fixed_env_mismatches_fail(self):
        for key in ("DYLD_PRINT_LIBRARIES", "DYLD_SHARED_CACHE_DIR", "DYLD_ROOT_PATH", "DYLD_INSERT_LIBRARIES",
                    "LD_PRELOAD", "LD_LIBRARY_PATH", "SIMULATOR_ROOT", "IPHONE_SIMULATOR_ROOT",
                    "DEVELOPER_DIR", "TOOLCHAINS", "SDKROOT"):
            with self.subTest(key=key), self.assertRaises(diag.DiagnosticError):
                diag.checked_environment({key: "wrong"}, BOUNDS, Path("/private"))

    def test_decoder_environment_rechecks_fixed_values_and_does_not_mutate_input(self):
        checked = diag.checked_environment({}, BOUNDS, Path("/private"))
        for key in (*BOUNDS["environment"], "DYLD_PRINT_LIBRARIES"):
            wrong = checked.copy()
            wrong[key] = "wrong"
            with self.subTest(key=key), self.assertRaises(diag.DiagnosticError):
                diag.decoder_environment(wrong, BOUNDS)


class Pipe:
    def __init__(self, fd):
        self.fd, self.closed = fd, False

    def fileno(self):
        return self.fd

    def close(self):
        self.closed = True


class FakeProcess:
    def __init__(self, args, rc=0, wait_error=None):
        self.args, self.pid, self.returncode = args, 1234, None
        self.stdout, self.stderr = Pipe(100), Pipe(101)
        self.rc, self.wait_error, self.kills, self.waits = rc, wait_error, 0, 0

    def wait(self, timeout):
        self.waits += 1
        if self.wait_error and not self.kills:
            raise self.wait_error
        self.returncode = self.rc
        return self.rc

    def kill(self):
        self.kills += 1


class FakePoller:
    def __init__(self, clock, tick):
        self.keys, self.clock, self.tick, self.closed = {}, clock, tick, False

    def register(self, fileobj, flags, data):
        self.keys[fileobj.fd] = SimpleNamespace(fileobj=fileobj, data=data)

    def unregister(self, fileobj):
        self.keys.pop(fileobj.fd)

    def get_map(self):
        return self.keys

    def select(self, timeout):
        self.clock[0] += self.tick
        return [(key, 1) for key in list(self.keys.values())]

    def close(self):
        self.closed = True


class CaptureTests(unittest.TestCase):
    @contextmanager
    def capture_case(self, stdout=b"26.6.2\n", stderr=b"", rc=0, wait_error=None, read_error=None,
                     tick=0, no_eof=False, wrong_argv=False, stream_limit=None):
        with tempfile.TemporaryDirectory() as temp:
            limits = copy.deepcopy(BOUNDS["limits"])
            if stream_limit is not None:
                limits["per_stream_bytes"] = stream_limit
            root = Path(temp)
            output = diag.Output(root, limits)
            argv = BOUNDS["diagnostic_command"]
            process = FakeProcess(["/wrong"] if wrong_argv else argv.copy(), rc, wait_error)
            clock = [0]
            poller = FakePoller(clock, tick)
            streams = {100: bytearray(stdout), 101: bytearray(stderr)}
            def read(fd, size):
                if read_error is not None:
                    raise read_error
                if no_eof and not streams[fd]:
                    return b"x"
                result = bytes(streams[fd][:size])
                del streams[fd][:size]
                return result
            with patch.object(diag.time, "monotonic", side_effect=lambda: clock[0]), \
                    patch.object(diag.subprocess, "Popen", return_value=process) as spawn, \
                    patch.object(diag.selectors, "DefaultSelector", return_value=poller), \
                    patch.object(diag.os, "read", side_effect=read):
                capture = diag.Capture(root, output, limits, 100)
                yield capture, process, spawn, output, root, poller

    def test_raw_noisy_and_nonUTF8_streams_argv_hashes_rc_and_EOF_are_preserved(self):
        for stdout, stderr in ((b"26.6.2\n", b""), (b"26.6.2\n", b"dyld[1234]: fixture\n"),
                               (b"\xff\x0026.6.2\nextra", b"\xfe")):
            with self.subTest(stdout=stdout), self.capture_case(stdout, stderr) as (
                    capture, process, spawn, output, root, poller):
                env = {"SYNTHETIC": "value", "DYLD_PRINT_LIBRARIES": "1", "NO_COLOR": "1"}
                self.assertEqual(capture.command("version", BOUNDS["diagnostic_command"], env), (stdout, stderr))
                self.assertEqual((root / "version-stdout.bin").read_bytes(), stdout)
                self.assertEqual((root / "version-stderr.bin").read_bytes(), stderr)
                receipt = json.loads((root / "version.json").read_bytes())
                self.assertEqual(receipt["requested_argv"], BOUNDS["diagnostic_command"])
                self.assertEqual(receipt["recorded_argv"], BOUNDS["diagnostic_command"])
                self.assertEqual(receipt["pid"], 1234)
                self.assertEqual(receipt["rc"], 0)
                self.assertTrue(receipt["streams_complete"])
                self.assertTrue(receipt["exact_child_reaped"])
                self.assertEqual(receipt["streams"]["stdout"]["sha256"], diag.digest(stdout))
                self.assertEqual(receipt["streams"]["stderr"]["bytes"], len(stderr))
                self.assertEqual(spawn.call_args.kwargs["env"], env)
                self.assertNotIn("shell", spawn.call_args.kwargs)
                self.assertTrue(process.stdout.closed and process.stderr.closed and poller.closed)
                self.assertEqual(process.kills, 0)

    def test_nonzero_exit_retains_complete_failure_bytes_before_rejecting(self):
        with self.capture_case(b"version", b"error", rc=1) as (capture, process, _, _, root, _):
            with self.assertRaises(diag.DiagnosticError):
                capture.command("nonzero", BOUNDS["diagnostic_command"], {})
            receipt = json.loads((root / "nonzero.json").read_bytes())
            self.assertTrue(receipt["streams_complete"])
            self.assertEqual(receipt["rc"], 1)
            self.assertEqual((root / "nonzero-stderr.bin").read_bytes(), b"error")
            self.assertEqual(process.kills, 0)

    def test_output_cap_retains_cap_plus_one_prefix_and_kills_only_owned_child(self):
        with self.capture_case(b"abcdefghijkl", stream_limit=8) as (capture, process, _, _, root, _):
            with self.assertRaises(diag.DiagnosticError):
                capture.command("overflow", BOUNDS["diagnostic_command"], {})
            self.assertEqual((root / "overflow-stdout.bin").read_bytes(), b"abcdefghi")
            receipt = json.loads((root / "overflow.json").read_bytes())
            self.assertFalse(receipt["streams_complete"])
            self.assertTrue(receipt["streams"]["stdout"]["retained_prefix_only"])
            self.assertIn("stream bound", receipt["errors"][0]["message"])
            self.assertEqual(process.kills, 1)

    def test_missing_EOF_deadline_retains_partial_bytes_and_reaps(self):
        with self.capture_case(tick=8, no_eof=True) as (capture, process, _, _, root, _):
            with self.assertRaises(diag.DiagnosticError):
                capture.command("timeout", BOUNDS["diagnostic_command"], {})
            receipt = json.loads((root / "timeout.json").read_bytes())
            self.assertFalse(receipt["streams_complete"])
            self.assertIn("deadline", receipt["errors"][0]["message"])
            self.assertTrue(receipt["exact_child_reaped"])
            self.assertEqual(process.kills, 1)
            self.assertTrue((root / "timeout-stdout.bin").read_bytes().startswith(b"26.6.2"))

    def test_wait_timeout_and_pipe_errors_are_explicit_not_success(self):
        for kwargs in ({"wait_error": subprocess.TimeoutExpired("fixture", 3)},
                       {"read_error": OSError("synthetic pipe error")}):
            with self.subTest(kwargs=kwargs), self.capture_case(**kwargs) as (capture, process, _, _, root, _):
                with self.assertRaises(diag.DiagnosticError):
                    capture.command("failure", BOUNDS["diagnostic_command"], {})
                receipt = json.loads((root / "failure.json").read_bytes())
                self.assertFalse(receipt["streams_complete"])
                self.assertTrue(receipt["errors"])
                self.assertEqual(process.kills, 1)
                self.assertTrue(receipt["exact_child_reaped"])

    def test_spawn_error_records_no_PID_or_fabricated_result(self):
        with self.capture_case() as (capture, _, _, _, root, _), \
                patch.object(diag.subprocess, "Popen", side_effect=OSError("synthetic spawn error")):
            with self.assertRaises(diag.DiagnosticError):
                capture.command("spawn", BOUNDS["diagnostic_command"], {})
            receipt = json.loads((root / "spawn.json").read_bytes())
            self.assertIsNone(receipt["pid"])
            self.assertIsNone(receipt["rc"])
            self.assertFalse(receipt["exact_child_reaped"])
            self.assertEqual(receipt["errors"][0]["phase"], "spawn")

    def test_recorded_argv_mismatch_stops_and_reaps_owned_child(self):
        with self.capture_case(wrong_argv=True) as (capture, process, _, _, root, _):
            with self.assertRaises(diag.DiagnosticError):
                capture.command("argv", BOUNDS["diagnostic_command"], {})
            receipt = json.loads((root / "argv.json").read_bytes())
            self.assertEqual(receipt["recorded_argv"], ["/wrong"])
            self.assertEqual(receipt["requested_argv"], BOUNDS["diagnostic_command"])
            self.assertIn("argv mismatch", receipt["errors"][0]["message"])
            self.assertEqual(process.kills, 1)

    def test_request_sink_failure_prevents_spawn(self):
        with self.capture_case() as (capture, _, spawn, output, _, _), \
                patch.object(output, "json", side_effect=OSError("synthetic sink error")):
            with self.assertRaises(OSError):
                capture.command("sink", BOUNDS["diagnostic_command"], {})
            spawn.assert_not_called()

    def test_postcapture_sink_failure_cannot_return_success(self):
        with self.capture_case() as (capture, process, _, output, _, _):
            original = output.write
            def failed(name, raw):
                if name.endswith("-stdout.bin"):
                    raise OSError("synthetic disk full")
                return original(name, raw)
            with patch.object(output, "write", side_effect=failed), self.assertRaises(OSError):
                capture.command("sink", BOUNDS["diagnostic_command"], {})
            self.assertEqual(process.waits, 1)
            self.assertEqual(process.kills, 0)

    def test_exhausted_command_bound_prevents_spawn(self):
        with self.capture_case() as (capture, _, spawn, _, _, _):
            capture.commands = BOUNDS["limits"]["maximum_commands"]
            with self.assertRaises(diag.DiagnosticError):
                capture.command("extra", BOUNDS["diagnostic_command"], {})
            spawn.assert_not_called()

    def test_exact_stream_cap_completes_without_truncating(self):
        with self.capture_case(b"abcdefgh", stream_limit=8) as (capture, _, _, _, root, _):
            self.assertEqual(capture.command("exact", BOUNDS["diagnostic_command"], {}), (b"abcdefgh", b""))
            record = json.loads((root / "exact.json").read_bytes())
            self.assertTrue(record["streams_complete"])
            self.assertFalse(record["streams"]["stdout"]["retained_prefix_only"])

    def test_kill_and_reap_failure_is_explicit_and_blocks_success(self):
        with self.capture_case(read_error=OSError("read")) as (capture, process, _, _, root, _):
            with patch.object(process, "kill", side_effect=PermissionError("kill denied")), \
                    patch.object(process, "wait", side_effect=subprocess.TimeoutExpired("owned", 3)), \
                    self.assertRaises(diag.DiagnosticError):
                capture.command("cleanup", BOUNDS["diagnostic_command"], {})
            record = json.loads((root / "cleanup.json").read_bytes())
            self.assertFalse(record["exact_child_reaped"])
            self.assertFalse(record["streams_complete"])
            self.assertEqual([e["phase"] for e in record["errors"]],
                             ["pipe-capture", "owned-child-kill", "owned-child-reap"])


class IntegrationTests(unittest.TestCase):
    def baseline_result(self, name, args, env):
        values = {"baseline-uname": b"Darwin synthetic XNU kernel\n", "baseline-architecture": b"arm64\n",
                  "baseline-product": b"26.6.2\n", "baseline-build": b"25G83\n",
                  "baseline-source": SHA.encode() + b"\n"}
        return values[name], b""

    def test_baseline_and_source_match_before_single_traced_diagnostic(self):
        for diagnostic in ((b"26.6.2\n", b""), (b"26.6.2\n", b"dyld[1234]: noise\n"),
                           (b"noise\n26.6.2\n", b"\xff\x00")):
            with tempfile.TemporaryDirectory() as temp:
                output = diag.Output(Path(temp), BOUNDS["limits"])
                events = []
                def command(capture, name, args, env):
                    events.append((name, args, env.copy()))
                    return diagnostic if name == "decoder-env-product-version" else self.baseline_result(name, args, env)
                with patch.object(diag, "check_gate", side_effect=lambda *args: events.append(("gate",))), \
                        patch.object(diag.Capture, "command", command), patch.object(diag.sys, "platform", "darwin"), \
                        patch.object(diag.os, "getuid", return_value=501):
                    result = diag.execute(BOUNDS, "a" * 64, HERE.parents[1], Path(temp), output, JOB, context_environment())
                self.assertEqual(events[0], ("gate",))
                self.assertEqual(len(events), 7)
                self.assertEqual(events[-1][1], ["/usr/bin/sw_vers", "-productVersion"])
                self.assertNotIn("DYLD_PRINT_LIBRARIES", events[1][2])
                self.assertEqual(events[-1][2], {**events[1][2], "DYLD_PRINT_LIBRARIES": "1", "NO_COLOR": "1"})
                observed = json.loads((Path(temp) / "observation.json").read_bytes())
                self.assertEqual(observed["stdout_exact_expected_product_line"], diagnostic[0] == b"26.6.2\n")
                self.assertEqual(observed["stderr_empty"], not diagnostic[1])
                self.assertFalse(observed["version_acceptance_performed"])
                self.assertTrue(observed["not_ipsw_internal_buffer_or_Go_CombinedOutput"])
                self.assertEqual(result["disposition"], "diagnostic-collected-not-version-acceptance")
                self.assertNotIn("GH_TOKEN", (Path(temp) / "environment.json").read_text())

    def test_gate_failure_prevents_any_provider_query(self):
        with tempfile.TemporaryDirectory() as temp, \
                patch.object(diag, "check_gate", side_effect=diag.DiagnosticError("not frozen")), \
                patch.object(diag.Capture, "command") as command, patch.object(diag.os, "getuid") as uid, \
                self.assertRaises(diag.DiagnosticError):
            diag.execute(BOUNDS, "a" * 64, HERE.parents[1], Path(temp),
                         diag.Output(Path(temp), BOUNDS["limits"]), JOB, context_environment())
        command.assert_not_called()
        uid.assert_not_called()

    def test_each_baseline_mismatch_stops_before_traced_query_without_trimming(self):
        for failure in BOUNDS["baseline_commands"]:
            for suffix in (b"wrong", b"26.6.2\nextra"):
                with tempfile.TemporaryDirectory() as temp:
                    calls = []
                    def command(name, args, env):
                        calls.append(name)
                        return (suffix, b"") if name == "baseline-" + failure else self.baseline_result(name, args, env)
                    with patch.object(diag.sys, "platform", "darwin"), patch.object(diag.os, "getuid", return_value=501), \
                            self.assertRaises(diag.DiagnosticError):
                        diag.baseline(SimpleNamespace(command=command), BOUNDS, {}, Path(temp), SHA,
                                      diag.Output(Path(temp), BOUNDS["limits"]))
                    self.assertEqual(calls[-1], "baseline-" + failure)

    def test_baseline_stderr_or_privilege_or_platform_mismatch_fails(self):
        with tempfile.TemporaryDirectory() as temp:
            output = diag.Output(Path(temp), BOUNDS["limits"])
            capture = SimpleNamespace(command=lambda *args: (b"Darwin synthetic\n", b"noise"))
            with patch.object(diag.sys, "platform", "darwin"), patch.object(diag.os, "getuid", return_value=501), \
                    self.assertRaises(diag.DiagnosticError):
                diag.baseline(capture, BOUNDS, {}, Path(temp), SHA, output)
            for platform, uid in (("linux", 501), ("darwin", 0)):
                with patch.object(diag.sys, "platform", platform), patch.object(diag.os, "getuid", return_value=uid), \
                        patch.object(capture, "command") as command, self.assertRaises(diag.DiagnosticError):
                    diag.baseline(capture, BOUNDS, {}, Path(temp), SHA, output)
                command.assert_not_called()

    def test_environment_failure_prevents_baseline_and_diagnostic(self):
        env = context_environment()
        env["DYLD_INSERT_LIBRARIES"] = "/unexpected"
        with tempfile.TemporaryDirectory() as temp, patch.object(diag, "check_gate"), \
                patch.object(diag.Capture, "command") as command, self.assertRaises(diag.DiagnosticError):
            diag.execute(BOUNDS, "a" * 64, HERE.parents[1], Path(temp),
                         diag.Output(Path(temp), BOUNDS["limits"]), JOB, env)
        command.assert_not_called()

    def test_output_manifest_requires_exact_flat_files_and_hashes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            output = diag.Output(root, BOUNDS["limits"])
            output.write("raw.bin", b"\xff")
            output.finish()
            diag.verify_output(root, BOUNDS)
            (root / "raw.bin").write_bytes(b"changed")
            with self.assertRaises(diag.DiagnosticError):
                diag.verify_output(root, BOUNDS)

    def test_output_caps_and_existing_receipt_refuse_without_overwriting(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            output = diag.Output(root, dict(BOUNDS["limits"], per_file_bytes=4, artifact_bytes=6, artifact_files=2))
            output.write("first.bin", b"1234")
            for name, raw in (("first.bin", b"x"), ("large.bin", b"12345"), ("total.bin", b"123")):
                with self.subTest(name=name), self.assertRaises(diag.DiagnosticError):
                    output.write(name, raw)
            self.assertEqual((root / "first.bin").read_bytes(), b"1234")
            self.assertEqual(len(list(root.iterdir())), 1)

    def test_main_gate_failure_persists_bounded_failure_and_manifest_without_provider(self):
        with tempfile.TemporaryDirectory() as temp:
            work, evidence = Path(temp) / "work", Path(temp) / "evidence"
            with patch.object(diag.sys, "argv", ["diagnostic.py", JOB, str(work), str(evidence)]), \
                    patch.dict(diag.os.environ, context_environment(), clear=True), \
                    patch.object(diag, "check_gate", side_effect=diag.DiagnosticError("missing freeze")), \
                    patch.object(diag.subprocess, "Popen") as spawn:
                self.assertEqual(diag.main(), 1)
            spawn.assert_not_called()
            result = json.loads((evidence / "result.json").read_bytes())
            self.assertEqual(result["disposition"], "diagnostic-failed-no-version-acceptance")
            self.assertIn("missing freeze", result["error"]["message"])
            diag.verify_output(evidence, BOUNDS)

    def test_source_closure_contains_only_new_diagnostic_paths(self):
        records, sha = diag.source_manifest(HERE.parents[1], BOUNDS)
        self.assertEqual(len(records), 4)
        self.assertEqual(sha, diag.digest(diag.encoded(records)))
        self.assertTrue(all(r["file"].startswith("diagnostics/macos_sw_vers/") or
                            r["file"] == ".github/workflows/macos-sw-vers-diagnostic.yml" for r in records))
        self.assertFalse(any("macos_cross_static" in r["file"] for r in records))


if __name__ == "__main__":
    unittest.main()
