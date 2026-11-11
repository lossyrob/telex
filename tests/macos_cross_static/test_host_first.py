import copy
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import host_first as h


HERE = Path(__file__).resolve().parent


class HostFirstTests(unittest.TestCase):
    def setUp(self):
        self.sha = "a" * 40
        self.now = h.instant("2026-11-11T18:00:00Z")
        self.expect = {"label": "synthetic-arm", "architecture": "arm64", "product": "99.1",
                       "build": "99A1", "kern_osversion": "99A1",
                       "kern_uuid": "11111111-2222-3333-4444-555555555555"}
        self.readme = b"# fixture\n- OS Version: macOS 99.1 (99A1)\n- Image Version: 20261111.1\n"
        self.index = (b"| fixture | arm64 | `synthetic-arm` | [fixture] |\n"
                      b"[fixture]: https://github.com/actions/runner-images/blob/main/images/macos/synthetic-Readme.md\n")
        self.spec = {"revision": "b" * 40, "readme_path": "images/macos/synthetic-Readme.md",
                     "readme_sha256": h.digest(self.readme), "index_sha256": h.digest(self.index),
                     "observed_at": "2026-11-11T17:59:00Z", "maximum_age_seconds": 120}
        self.acquisition = {k: v for k, v in self.spec.items() if k != "maximum_age_seconds"}
        self.policy = {"schema": "host-first-preparation-v1", "binding": self.expect,
                       "documentation": self.spec, "status": "synthetic fixture only",
                       "documents": {"readme_utf8": self.readme.decode(), "index_utf8": self.index.decode(),
                                     "acquisition": self.acquisition}}
        self.env = {"GITHUB_SHA": self.sha, "GITHUB_RUN_ID": "901", "GITHUB_RUN_ATTEMPT": "1",
                    "GITHUB_JOB": "Intel25", "GITHUB_EVENT_NAME": "push", "GITHUB_REPOSITORY": "lossyrob/telex",
                    "ImageOS": "context-only", "ImageVersion": "not-an-attestation"}
        self.workflow = (HERE.parents[1] / h.WORKFLOW).read_text().replace(
            "runs-on: macos-26", "runs-on: synthetic-arm").encode()
        self.docs = {"readme": self.readme, "index": self.index, "acquisition": self.acquisition}
        self.commands = []
        self.values = {**{k: self.expect[k] for k in h.HOST_FIELDS}, "source_sha": self.sha}
        self.terms = {"schema": "future-finite-hfr-v1", "finite_comment_id": 9900000000,
                      "finite_body_sha256": "c" * 64, "source_sha": self.sha, "hfr_cap": 2,
                      "one_consuming_attempt": 1, "retries": 0, "every_dispatch_owner_directed": True,
                      "effective": True}
        self.steps = [{"step": "checkout", "conclusion": "success"},
                      {"step": "offline-tests", "conclusion": "success"},
                      {"step": "host-first", "conclusion": "failure"},
                      {"step": "component", "conclusion": "skipped"}]

    def command(self, argv):
        names = {tuple(args): name for name, args in h.HOST_COMMANDS}
        names[("/usr/bin/git", "-C", str(HERE.parents[1]), "rev-parse", "HEAD")] = "source_sha"
        self.commands.append(argv)
        return {"rc": 0, "stdout": self.values[names[tuple(argv)]] + "\n", "stderr": ""}

    def record(self):
        return h.host_record(self.policy, self.docs, self.env, self.command, HERE.parents[1], self.workflow, self.now)

    def bundle(self, refusal=True):
        if refusal:
            self.values["build"] = self.values["kern_osversion"] = "99A2"
        record = self.record()
        files = {"host-first.json": h.encoded(record), "workflow.yml": self.workflow,
                 "policy.json": h.encoded(self.policy), "runner-readme.md": self.readme,
                 "runner-index.md": self.index, "documentation-source.json": h.encoded(self.acquisition)}
        return files, h.evidence_manifest(files)

    def classify(self, files, manifest, count=0):
        return h.classify_refusal(files, manifest, self.sha, 901, self.terms, count, self.steps)

    def test_docs_consistency_does_not_authorize_or_observe_host(self):
        result = h.documentation_gate(self.readme, self.index, self.spec, self.expect, self.now, self.acquisition)
        self.assertFalse(result["authorizes_launch"])
        self.assertIn("live allocation unverified", result["status"])
        self.assertEqual(self.commands, [])

    def test_docs_reject_mismatch_missing_wrong_label_arch_revision_and_stale(self):
        for field, value in (("label", "other"), ("architecture", "x86_64"), ("build", "99A2"), ("product", "99.2")):
            expected = {**self.expect, field: value}
            with self.subTest(field=field), self.assertRaises(RuntimeError):
                h.documentation_gate(self.readme, self.index, self.spec, expected, self.now, self.acquisition)
        for spec in ({}, {**self.spec, "revision": "d" * 40}, {**self.spec, "maximum_age_seconds": 1},
                     {**self.spec, "maximum_age_seconds": True}):
            with self.subTest(spec=spec), self.assertRaises(RuntimeError):
                h.documentation_gate(self.readme, self.index, spec, self.expect, self.now, self.acquisition)
        self.policy["binding"]["build"] = "99A2"
        with self.assertRaises(RuntimeError):
            self.record()
        self.assertEqual(self.commands, [])

    def test_missing_metadata_and_wrong_source_are_not_refusals(self):
        self.spec["readme_sha256"] = "0" * 64
        with self.assertRaises(RuntimeError):
            self.record()
        self.assertEqual(self.commands, [])
        self.spec["readme_sha256"] = h.digest(self.readme)
        self.values["source_sha"] = "d" * 40
        with self.assertRaisesRegex(RuntimeError, "source"):
            self.record()

    def test_unbound_policy_stops_before_observation(self):
        self.policy["binding"] = None
        with self.assertRaisesRegex(RuntimeError, "unselected"):
            self.record()
        self.assertEqual(self.commands, [])
        actual = h.strict_json((HERE / "host_first_policy.json").read_bytes())
        self.assertIsNone(actual["binding"])
        self.assertIsNone(actual["documentation"])

    def test_unbound_entrypoint_records_stop_without_command_or_HFR_credit(self):
        with tempfile.TemporaryDirectory() as temp, \
                patch.object(h.subprocess, "run", side_effect=AssertionError("no observation")) as command:
            output = Path(temp) / "host"
            with self.assertRaisesRegex(RuntimeError, "unselected"):
                h.main([str(output)])
            stop = h.strict_json((output / "preparation-stop.json").read_bytes())
            self.assertFalse(stop["non_consuming_HFR_claimed"])
            self.assertFalse(stop["authorizes_launch"])
            self.assertFalse((output / "host-first.json").exists())
            command.assert_not_called()

    def test_host_records_every_required_field_context_and_same_job(self):
        record = self.record()
        self.assertEqual(record["status"], "host-accepted")
        self.assertEqual(set(record["observed"]), {*h.HOST_FIELDS, "source_sha"})
        self.assertEqual(record["image_environment_context"],
                         {"ImageOS": "context-only", "ImageVersion": "not-an-attestation"})
        self.assertEqual(len(record["commands"]), 6)
        self.assertFalse(record["authorizes_launch"])
        files, manifest = self.bundle(False)
        self.assertEqual(h.verify_evidence(files, manifest, self.sha, 901)["status"], "host-accepted")
        for source, run_id in (("d" * 40, 901), (self.sha, 902)):
            with self.assertRaises(RuntimeError):
                h.verify_evidence(files, manifest, source, run_id)

    def assert_invalid_retained_observation(self, files, record):
        altered = {**files, "host-first.json": h.encoded(record)}
        manifest = h.evidence_manifest(altered)
        with self.assertRaises((RuntimeError, ValueError)):
            h.verify_evidence(altered, manifest, self.sha, 901)
        with self.assertRaises((RuntimeError, ValueError)):
            self.classify(altered, manifest)

    def test_control_and_nonascii_observations_never_earn_refusal_credit(self):
        files, _ = self.bundle()
        separators = ("\n", "\r", "\r\n", "\0", "\t", "\x1f", "\x7f",
                      "\x85", "\u2028", "\u2029", "\u00e9")
        for field in h.HOST_FIELDS:
            for separator in separators:
                # Same architecture/LF pattern as the reviewer's inert witness, extended to every host field.
                value = self.values[field] + separator + "UNEXPECTED-SECOND-LINE"
                with self.subTest(field=field, separator=repr(separator)):
                    record = h.strict_json(files["host-first.json"])
                    record["observed"][field] = value
                    next(row for row in record["commands"] if row["field"] == field)["stdout"] = value + "\n"
                    self.assert_invalid_retained_observation(files, record)
                    with patch.dict(self.values, {field: value}), self.assertRaisesRegex(
                            RuntimeError, "malformed host observation"):
                        self.record()

    def test_valid_producer_normalization_verifies_and_preserves_uuid_canonical_form(self):
        canonical = "ABCDEFAB-CDEF-ABCD-EFAB-CDEFABCDEFAB"
        self.expect["kern_uuid"] = canonical
        self.values["kern_uuid"] = canonical.lower()
        original_command = self.command

        def padded_command(argv):
            result = original_command(argv)
            return {**result, "stdout": " \t" + result["stdout"].strip() + "\r\n"}

        with patch.object(self, "command", side_effect=padded_command):
            for refusal in (False, True):
                with self.subTest(refusal=refusal):
                    files, manifest = self.bundle(refusal)
                    verified = h.verify_evidence(files, manifest, self.sha, 901)
                    self.assertEqual(verified["observed"]["kern_uuid"], canonical)
                    self.assertEqual(verified["status"], "host-first-refusal" if refusal else "host-accepted")
                    for row in verified["commands"]:
                        self.assertEqual(h.normalize_host_output(row["field"], row),
                                         verified["observed"][row["field"]])
                    record = copy.deepcopy(verified)
                    record["observed"]["kern_uuid"] = canonical.lower()
                    self.assert_invalid_retained_observation(files, record)

    def test_unstripped_observed_values_do_not_match_normalizer(self):
        files, _ = self.bundle()
        for field in (*h.HOST_FIELDS, "source_sha"):
            record = h.strict_json(files["host-first.json"])
            record["observed"][field] = " " + record["observed"][field] + "\n"
            with self.subTest(field=field):
                self.assert_invalid_retained_observation(files, record)

    def test_command_failures_empty_and_overbyte_outputs_reject_before_counting(self):
        files, _ = self.bundle()
        mutations = ({"rc": 1}, {"rc": True}, {"stderr": "warning"},
                     {"stdout": ""}, {"stdout": " \r\n\t"},
                     {"stdout": "x" * 4097},
                     {"stdout": "\u0085" * 2048 + "x"})
        for field in (*h.HOST_FIELDS, "source_sha"):
            for mutation in mutations:
                with self.subTest(field=field, mutation=repr(mutation)[:80]):
                    record = h.strict_json(files["host-first.json"])
                    row = next(row for row in record["commands"] if row["field"] == field)
                    row.update(mutation)
                    self.assert_invalid_retained_observation(files, record)
                    original_command = self.command

                    def malformed_command(argv):
                        result = original_command(argv)
                        return {**result, **mutation} if argv == row["argv"] else result

                    with patch.object(self, "command", side_effect=malformed_command), \
                            self.assertRaises(RuntimeError):
                        self.record()

    def test_host_output_byte_limit_is_inclusive_before_stripping(self):
        files, _ = self.bundle()
        record = h.strict_json(files["host-first.json"])
        row = next(row for row in record["commands"] if row["field"] == "architecture")
        row["stdout"] = "x" * 4095 + "\n"
        record["observed"]["architecture"] = "x" * 4095
        altered = {**files, "host-first.json": h.encoded(record)}
        manifest = h.evidence_manifest(altered)
        self.assertEqual(h.verify_evidence(altered, manifest, self.sha, 901)["status"], "host-first-refusal")
        self.assertEqual(self.classify(altered, manifest)["refusals"], 1)
        row["stdout"] += "\n"
        self.assert_invalid_retained_observation(files, record)

    def test_attempt_two_ARM_and_nonpush_rejected_before_commands(self):
        for key, value in (("GITHUB_RUN_ATTEMPT", "2"), ("GITHUB_JOB", "ARM24"),
                           ("GITHUB_EVENT_NAME", "workflow_dispatch")):
            with patch.dict(self.env, {key: value}), self.subTest(key=key), self.assertRaises(RuntimeError):
                self.record()
        self.assertEqual(self.commands, [])

    def test_refusal_proof_requires_manifest_commands_step_list_and_no_effects(self):
        files, manifest = self.bundle()
        self.assertEqual(self.classify(files, manifest)["refusals"], 1)
        self.assertFalse(self.classify(files, manifest)["finite_ended"])
        for mutation in ("hash", "extra-file", "effect", "command", "workflow"):
            altered = dict(files)
            if mutation == "hash":
                altered["runner-readme.md"] += b"\n"
                sealed = manifest
            elif mutation == "extra-file":
                altered["candidate.bin"] = b"forbidden"
                sealed = h.evidence_manifest(altered)
            elif mutation in ("effect", "command"):
                record = h.strict_json(altered["host-first.json"])
                if mutation == "effect":
                    record["effects"]["artifact_acquisition"] = 1
                else:
                    record["commands"][0]["argv"] = ["/bin/other"]
                altered["host-first.json"] = h.encoded(record)
                sealed = h.evidence_manifest(altered)
            else:
                altered["workflow.yml"] = self.workflow.replace(
                    b"      - name: Host-first observation", b"      - run: curl bad\n      - name: Host-first observation")
                sealed = h.evidence_manifest(altered)
            with self.subTest(mutation=mutation), self.assertRaises(RuntimeError):
                self.classify(altered, sealed)
        self.steps[-1]["conclusion"] = "success"
        with self.assertRaisesRegex(RuntimeError, "step proof"):
            self.classify(files, manifest)

    def test_refusal_schema_rejects_unknown_fields_bool_attempt_and_missing_observations(self):
        files, _ = self.bundle()
        for mutation in ("extra", "bool-attempt", "missing-uuid", "false-status"):
            altered = dict(files)
            record = h.strict_json(altered["host-first.json"])
            if mutation == "extra":
                record["skip_gate"] = True
            elif mutation == "bool-attempt":
                record["run_attempt"] = True
            elif mutation == "missing-uuid":
                record["observed"].pop("kern_uuid")
            else:
                record["status"] = "host-accepted"
            altered["host-first.json"] = h.encoded(record)
            with self.subTest(mutation=mutation), self.assertRaises(RuntimeError):
                self.classify(altered, h.evidence_manifest(altered))

    def test_accepted_bundle_must_match_current_source_policy_and_workflow(self):
        files, manifest = self.bundle(False)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            here = root / "tests" / "macos_cross_static"
            here.mkdir(parents=True)
            workflow = root / h.WORKFLOW
            workflow.parent.mkdir(parents=True)
            workflow.write_bytes(self.workflow)
            (here / "host_first_policy.json").write_bytes(files["policy.json"])
            evidence = root / "host-evidence"
            evidence.mkdir()
            for name, raw in files.items():
                (evidence / name).write_bytes(raw)
            (evidence / "sha256.json").write_bytes(h.encoded(manifest))
            bounds = {"runner": {"label": self.expect["label"], "architecture": "arm64",
                                 "product_version": "99.1", "build": "99A1"}}
            with patch.object(h, "HERE", here):
                self.assertEqual(h.accepted_host(evidence, self.sha, 901, bounds)["status"], "host-accepted")
                (here / "host_first_policy.json").write_bytes(b"{}")
                with self.assertRaisesRegex(RuntimeError, "current reviewed source"):
                    h.accepted_host(evidence, self.sha, 901, bounds)

    def test_cap_ends_finite_and_never_redispatches(self):
        files, manifest = self.bundle()
        last = self.classify(files, manifest, 1)
        self.assertTrue(last["finite_ended"])
        self.assertFalse(last["automatic_dispatch"])
        with self.assertRaisesRegex(RuntimeError, "cap"):
            self.classify(files, manifest, 2)
        for cap in (0, 3, True):
            with patch.dict(self.terms, {"hfr_cap": cap}), self.subTest(cap=cap), self.assertRaises(RuntimeError):
                self.classify(files, manifest)
        with patch.dict(self.terms, {"hfr_cap": 1}):
            self.assertTrue(self.classify(files, manifest)["finite_ended"])

    def test_every_consuming_effect_blocks_nonconsuming_classification(self):
        files, _ = self.bundle()
        for effect in h.EFFECTS:
            altered = dict(files)
            record = h.strict_json(altered["host-first.json"])
            record["effects"][effect] = 1
            altered["host-first.json"] = h.encoded(record)
            with self.subTest(effect=effect), self.assertRaisesRegex(RuntimeError, "consuming effect"):
                self.classify(altered, h.evidence_manifest(altered))

    def test_old_finite_template_and_retry_cannot_get_HFR_credit(self):
        files, manifest = self.bundle()
        for key, value in (("finite_comment_id", 6664239510), ("effective", False),
                           ("retries", 1), ("every_dispatch_owner_directed", False),
                           ("source_sha", "3a0d88d0320507660096a1e0662b2d770fe9d37d")):
            with patch.dict(self.terms, {key: value}), self.subTest(key=key), self.assertRaises(RuntimeError):
                self.classify(files, manifest)
        with self.assertRaises(RuntimeError):
            h.classify_refusal(files, manifest, self.sha, 901, None, 0, self.steps)

    def test_docs_match_without_injob_record_never_admits_component(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            with self.assertRaises(RuntimeError):
                h.accepted_host(directory, self.sha, 901, {})

    def test_refusal_cannot_construct_native_or_fetch_gate(self):
        from test_component import run
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            argv = ["run.py", "Intel25", str(root / "work"), str(root / "evidence"), str(root / "host")]
            with patch.object(sys, "argv", argv), patch.object(sys, "platform", "darwin"), \
                    patch.object(os, "getuid", return_value=501), \
                    patch.dict(os.environ, self.env), \
                    patch.object(run.host_first, "accepted_host", side_effect=RuntimeError("refused")), \
                    patch.object(run, "NativeBinding", side_effect=AssertionError("native")) as native, \
                    patch.object(run, "ComponentRun", side_effect=AssertionError("controller")) as controller, \
                    patch.object(run.gate, "check", side_effect=AssertionError("fetch")) as gate:
                with self.assertRaisesRegex(RuntimeError, "refused"):
                    run.main()
                native.assert_not_called()
                controller.assert_not_called()
                gate.assert_not_called()
            self.assertFalse((root / "work").exists())

    def test_effect_record_never_claims_transport_or_principal(self):
        effect = {"head_sha": self.sha, "event": "push", "workflow": h.WORKFLOW,
                  "jobs": ["Intel25"], "run_count": 1, "run_attempt": 1,
                  "actor": {"login": "fixture", "id": 123}}
        record = h.push_effect_record("https://example.invalid/repo.git", "b" * 40, self.sha, effect)
        self.assertNotIn("actual_target_used", record)
        self.assertEqual(record["effective_transport"], "not attested")
        self.assertEqual(record["git_credential_principal"], "not attested")
        self.assertEqual(record["command_target_argument"], "https://example.invalid/repo.git")

    def test_workflow_prefix_rejects_hidden_action_condition_and_matrix(self):
        for old, new in ((b"      - name: Host-first observation", b"      - run: download\n      - name: Host-first observation"),
                         (b"        id: host", b"        id: host\n        if: always()"),
                         (b"jobs:\n", b"jobs:\n  Other:\n    runs-on: x\n"),
                         (b"    timeout-minutes: 55", b"    strategy:\n      matrix: {}\n    timeout-minutes: 55")):
            with self.subTest(new=new), self.assertRaises(RuntimeError):
                h.workflow_prefix(self.workflow.replace(old, new))

    def test_import_of_preparation_is_inert(self):
        spec = importlib.util.spec_from_file_location("host_first_import_probe", HERE / "host_first.py")
        module = importlib.util.module_from_spec(spec)
        with patch.object(h.subprocess, "run", side_effect=AssertionError("process")), \
                patch.object(h.subprocess, "Popen", side_effect=AssertionError("process")):
            spec.loader.exec_module(module)
        self.assertTrue(callable(module.host_record))


if __name__ == "__main__":
    unittest.main()
