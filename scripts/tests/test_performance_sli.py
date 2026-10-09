import importlib.util
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
