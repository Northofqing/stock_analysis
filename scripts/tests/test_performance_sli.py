import importlib.util
import json
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest
spec = importlib.util.spec_from_file_location("performance_sli", Path(__file__).parents[1]/"performance_sli.py")
sli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sli)

class PerformanceSliTest(unittest.TestCase):
    def test_comparison(self):
        base = {k:"fixture" for k in sli.IDENTITY}
        base.update(version=1, rows=100, features=[], value=100)
        self.assertEqual(sli.compare(base,{**base,"value":120})["status"],"within_threshold")
        self.assertEqual(sli.compare(base,{**base,"value":120.1})["status"],"warning")
        for current in ({}, {**base,"snapshot":"other"},{**base,"value":float("nan")},{**base,"value":True},{**base,"version":2},{**base,"rows":True},{**base,"log_status":"failed"},{**base,"value":10**500}):
            self.assertEqual(sli.compare(base,current)["status"],"unavailable")
    def test_stage_attribution(self):
        with tempfile.TemporaryDirectory() as root:
            log=Path(root)/"observed.log"
            log.write_text("[startup-profile] pid=1 stage=database status=ok elapsed_ms=123 since_process_start_ms=234\n[DB init][timing] phase=data_acquisition_audit elapsed_ms=111\n")
            result=sli.collect(log,{"snapshot":"TEST_CODE"})
            self.assertEqual([r["value"] for r in result["records"]],[123,111])
            self.assertEqual(result["records"][1]["metric"],"database_init")
            self.assertIn("unavailable",result["review_duration"])

    def test_attribution_pool_availability_required_from_actual_producer_lines(self):
        context = {key: "TEST_CODE" for key in sli.IDENTITY}
        context.update(version=1, rows=100, features=[])
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "stage.log"
            log.write_text("[DB init][timing] phase=attribution_pool elapsed_ms=100 available=true\n")
            baseline = sli.collect(log, context)["records"][0]
            self.assertIs(baseline["available"], True)
            self.assertEqual(sli.compare(baseline, baseline)["status"], "within_threshold")
            for suffix, expected in ((" available=false", False), ("", None), (" available=unknown", None), ("available=true", None), (" available=true available=false", None)):
                log.write_text("[DB init][timing] phase=attribution_pool elapsed_ms=1" + suffix + "\n")
                current = sli.collect(log, context)["records"][0]
                self.assertIs(current["available"], expected)
                self.assertEqual(current["log_status"], "unavailable")
                self.assertEqual(sli.compare(baseline, current)["status"], "unavailable")
                self.assertEqual(sli.compare(current, baseline)["status"], "unavailable")
            missing_availability = dict(baseline)
            del missing_availability["available"]
            self.assertEqual(sli.compare(baseline, missing_availability)["status"], "unavailable")
            log.write_text("[DB init][timing] phase=data_acquisition_audit elapsed_ms=1\n")
            ordinary = sli.collect(log, context)["records"][0]
            self.assertNotIn("available", ordinary)
            self.assertEqual(ordinary["log_status"], "observed")
            self.assertEqual(sli.compare(ordinary, ordinary)["status"], "within_threshold")

    def test_failed_attribution_collector_to_comparator_cli_exits_successfully(self):
        script = Path(__file__).parents[1] / "performance_sli.py"
        context = {key: "TEST_CODE" for key in sli.IDENTITY}
        context.update(version=1, rows=100, features=[])
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            context_path = root / "context.json"
            context_path.write_text(json.dumps(context))
            records = []
            for availability, elapsed in (("true", 100), ("false", 1)):
                log = root / (availability + ".log")
                log.write_text(f"[DB init][timing] phase=attribution_pool elapsed_ms={elapsed} available={availability}\n")
                collected = subprocess.run([sys.executable, str(script), "collect", str(log), str(context_path)], text=True, capture_output=True, check=True)
                record_path = root / (availability + ".json")
                record_path.write_text(json.dumps(json.loads(collected.stdout)["records"][0]))
                records.append(str(record_path))
            compared = subprocess.run([sys.executable, str(script), "compare", *records], text=True, capture_output=True, check=True)
            self.assertEqual(json.loads(compared.stdout)["status"], "unavailable")
