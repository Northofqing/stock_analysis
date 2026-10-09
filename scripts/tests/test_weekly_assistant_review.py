import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "weekly_assistant_review", Path(__file__).resolve().parents[1] / "run-weekly-assistant-review.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class WeeklyAssistantReviewTests(unittest.TestCase):
    def inputs(self, root):
        period = {"observed_at": "2026-10-08T16:00:00+08:00",
                  "latest_completed_session": "2026-10-08",
                  "period_completed_through": "2026-09-30"}
        manifest = {"schema_version": "weekly-outcome-evidence-manifest-v1", "period": period}
        report = {"report_version": "H16-descriptive-weekly-v1", "period": period,
                  "evidence_manifest": manifest}
        report_path, manifest_path = root / "report.json", root / "manifest.json"
        MODULE.write_private_new(report_path, json.dumps(report).encode())
        MODULE.write_private_new(manifest_path, json.dumps(manifest).encode())
        return report_path, manifest_path

    def fake_cli(self, root, *, fail_after_json=False, omit_markdown=False,
                 model_attempts=0, original_to_mutate=None):
        binary = root / "fake-assistant"
        calls = root / "calls.jsonl"
        binary.write_text("""#!PYTHON
import hashlib, json, os, pathlib, sys
args=sys.argv
def value(flag): return args[args.index(flag)+1]
with open(CALLS, 'a') as stream: stream.write(json.dumps(args)+'\\n')
assert '--model' not in args and '--reviewed-pricing' not in args
report=pathlib.Path(value('--report')).read_bytes()
manifest=pathlib.Path(value('--manifest')).read_bytes()
period=json.loads(report)['period']
if MUTATE: pathlib.Path(MUTATE).write_bytes(b'changed original after freeze')
comparison={'schema_version':'assistant-phase-a-comparison-v1','mode':'read_only_weekly_review',
  'status':'degraded','as_of':value('--as-of'),'period':period,
  'report_sha256':hashlib.sha256(report).hexdigest(),'manifest_sha256':hashlib.sha256(manifest).hexdigest(),
  'arms':[{'arm':'template','mode':'deterministic_template'},
          {'arm':'model_without_outcomes','mode':'degraded_template','model_receipt':None},
          {'arm':'model_with_outcomes','mode':'degraded_template','model_receipt':None}],
  'run_reservations':{'attempt_slots_issued':ATTEMPTS,'retained_maximum_micro_cny':0},
  'credential_inherited':os.environ.get('OPENAI_API_KEY'),
  'binding_inherited':os.environ.get('PAPER_LEDGER_ACCOUNT_BINDING'),
  'registry_path':value('--registry') if '--registry' in args else None}
fd=os.open(value('--output'),os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(fd,'w') as stream: json.dump(comparison,stream)
if FAIL: sys.exit(23)
if not OMIT:
    fd=os.open(value('--markdown-output'),os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
    with os.fdopen(fd,'w') as stream:
        stream.write(comparison['report_sha256']+' '+comparison['manifest_sha256'])
""".replace("#!PYTHON", "#!" + sys.executable)
            .replace("CALLS", repr(str(calls))).replace("MUTATE", repr(str(original_to_mutate) if original_to_mutate else None))
            .replace("ATTEMPTS", repr(model_attempts)).replace("FAIL", repr(fail_after_json)).replace("OMIT", repr(omit_markdown)))
        binary.chmod(0o700)
        return binary, calls

    def run_fixture(self, root, **fake_options):
        report, manifest = self.inputs(root)
        binary, calls = self.fake_cli(root, **fake_options)
        output = root / "assistant"
        result = MODULE.run_review(report, manifest, output, binary)
        status = json.loads((output / "status.json").read_text())
        return result, status, output, calls, report, manifest

    def test_one_offline_child_writes_two_artifacts_from_same_frozen_inputs_and_latest_clock(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report, manifest = self.inputs(root)
            registry = root / "registry.toml"
            MODULE.write_private_new(registry, b"registry_version='fixture-only'\n")
            binary, calls = self.fake_cli(root, original_to_mutate=report)
            output = root / "assistant"
            expected_report = hashlib.sha256(report.read_bytes()).hexdigest()
            expected_manifest = hashlib.sha256(manifest.read_bytes()).hexdigest()
            with patch.dict(os.environ, {"OPENAI_API_KEY": "TEST_CODE_never_forward", "PAPER_LEDGER_ACCOUNT_BINDING": "TEST_CODE_private"}):
                self.assertEqual(MODULE.run_review(report, manifest, output, binary, registry), 0)
            status = json.loads((output / "status.json").read_text())
            comparison = json.loads((output / "comparison.json").read_text())
            self.assertEqual(status["status"], "complete")
            self.assertEqual(status["child_calls"], 1)
            self.assertEqual(status["model_calls_verified"], 0)
            self.assertEqual(len(calls.read_text().splitlines()), 1)
            self.assertEqual(status["completed_session"], "2026-10-08")
            self.assertNotEqual(status["completed_session"], comparison["period"]["period_completed_through"])
            self.assertEqual(comparison["report_sha256"], expected_report)
            self.assertEqual(comparison["manifest_sha256"], expected_manifest)
            self.assertIsNone(comparison["credential_inherited"])
            self.assertIsNone(comparison["binding_inherited"])
            self.assertEqual(Path(comparison["registry_path"]).read_bytes(), registry.read_bytes())
            self.assertEqual(status["completed_artifacts"], ["comparison.json", "comparison.md"])
            for name in ("comparison.json", "comparison.md", "status.json"):
                self.assertEqual((output / name).stat().st_mode & 0o777, 0o600)
            for name in ("report.json", "manifest.json", "signal_registry.toml"):
                self.assertEqual((output / "inputs" / name).stat().st_mode & 0o777, 0o400)
            self.assertEqual(output.stat().st_mode & 0o777, 0o700)
            self.assertIn(expected_report, (output / "comparison.md").read_text())

    def test_missing_original_registry_is_not_reconstructed_or_assumed(self):
        with tempfile.TemporaryDirectory() as directory:
            result, status, output, calls, _, _ = self.run_fixture(Path(directory))
            self.assertEqual(result, 0)
            command = json.loads(calls.read_text())
            self.assertNotIn("--registry", command)
            self.assertIsNone(status["input_sha256"]["registry"])

    def test_mismatched_clock_or_completed_period_rejected_before_any_child(self):
        for overrides in ({"as_of": "2026-10-08T16:01:00+08:00"}, {"completed_session": "2026-09-30"}, {"as_of": "2026-10-08T08:00:00Z"}):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                report, manifest = self.inputs(root)
                binary, calls = self.fake_cli(root)
                output = root / "assistant"
                self.assertEqual(MODULE.run_review(report, manifest, output, binary, **overrides), 2)
                status = json.loads((output / "status.json").read_text())
                self.assertEqual(status["child_calls"], 0)
                self.assertFalse(calls.exists())
                self.assertEqual(status["status"], "failed")

    def test_failed_second_output_preserves_first_and_never_reruns_comparison(self):
        with tempfile.TemporaryDirectory() as directory:
            result, status, output, calls, _, _ = self.run_fixture(Path(directory), fail_after_json=True)
            self.assertEqual(result, 23)
            self.assertEqual(status["status"], "partial")
            self.assertEqual(status["completed_artifacts"], ["comparison.json"])
            self.assertIsNone(status["model_calls_verified"])
            self.assertFalse((output / "comparison.md").exists())
            self.assertEqual(len(calls.read_text().splitlines()), 1)

    def test_missing_output_or_model_attempt_receipt_cannot_report_success(self):
        for options in ({"omit_markdown": True}, {"model_attempts": 1}):
            with tempfile.TemporaryDirectory() as directory:
                result, status, _, calls, _, _ = self.run_fixture(Path(directory), **options)
                self.assertEqual(result, 2)
                self.assertEqual(status["status"], "partial")
                self.assertIsNone(status["model_calls_verified"])
                self.assertEqual(len(calls.read_text().splitlines()), 1)

    def test_timeout_does_not_retry_or_claim_completed_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report, manifest = self.inputs(root)
            binary, _ = self.fake_cli(root)
            output = root / "assistant"
            with patch.object(MODULE.subprocess, "run", side_effect=subprocess.TimeoutExpired("fixture", 30)) as run:
                self.assertEqual(MODULE.run_review(report, manifest, output, binary), 2)
                self.assertEqual(run.call_count, 1)
                self.assertEqual(run.call_args.kwargs["timeout"], 30)
            status = json.loads((output / "status.json").read_text())
            self.assertEqual(status["error_code"], "assistant_child_timeout")
            self.assertEqual(status["completed_artifacts"], [])

    def test_private_regular_bounded_inputs_and_output_no_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report, manifest = self.inputs(root)
            binary, calls = self.fake_cli(root)
            output = root / "assistant"
            self.assertEqual(MODULE.run_review(report, manifest, output, binary), 0)
            saved = (output / "comparison.json").read_bytes()
            with self.assertRaises(ValueError):
                MODULE.run_review(report, manifest, output, binary)
            self.assertEqual((output / "comparison.json").read_bytes(), saved)
            self.assertEqual(len(calls.read_text().splitlines()), 1)
            shared = root / "shared"
            shared.mkdir(mode=0o755)
            with self.assertRaises(ValueError):
                MODULE.run_review(report, manifest, shared, binary)
            link = root / "linked-report"
            link.symlink_to(report)
            with self.assertRaises(OSError): MODULE.read_private(link, MODULE.MAX_REPORT)
            report.chmod(0o644)
            with self.assertRaises(ValueError): MODULE.read_private(report, MODULE.MAX_REPORT)
            report.chmod(0o600)
            with open(report, "r+b") as stream: stream.truncate(MODULE.MAX_REPORT + 1)
            with self.assertRaises(ValueError): MODULE.read_private(report, MODULE.MAX_REPORT)


if __name__ == "__main__":
    unittest.main()
