#!/usr/bin/env python3
"""Run the daily wrapper against temporary SQLite and stand-in backfill tools."""

import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest


SCRIPT = Path(os.environ.get(
    "DAILY_VERIFY_SCRIPT", Path(__file__).with_name("daily_prediction_verify.sh")
)).resolve()


class DailyPredictionVerifyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="TEST_CODE_daily_prediction_")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in ("bin", "data", "logs", "commands"):
            (self.root / name).mkdir()
        self.db = self.root / "data/stock_analysis.db"
        with sqlite3.connect(self.db) as connection:
            connection.executescript("""
                CREATE TABLE prediction_tracker (
                    stock_code TEXT, actual_change_t1 REAL,
                    actual_change_t3 REAL, actual_change_t5 REAL
                );
                CREATE TABLE fixture_writes (stage TEXT);
            """)
        self.calls = self.root / "calls.txt"
        self.env = dict(os.environ,
            STOCK_ANALYSIS_RUNTIME_ROOT=str(self.root),
            TEST_CODE_DAILY_CALLS=str(self.calls),
            TEST_CODE_DAILY_RUNTIME=str(self.root / "runtime.txt"),
            STOCK_DB=str(self.root / "unselected.db"),
            PATH=str(self.root / "commands") + os.pathsep + os.environ["PATH"],
            DAILY_EXIT_CODE="0", PREDICTION_EXIT_CODE="0",
        )
        # The wrapper must not read runtime credentials to perform this workflow.
        (self.root / ".env").write_text("exit 91\n")
        self.write_tool("backfill_daily", """#!/bin/bash
printf 'daily:%s\\n' "$1" >> "$TEST_CODE_DAILY_CALLS"
printf 'cwd=%s\\ndb=%s\\n' "$PWD" "$STOCK_DB" >> "$TEST_CODE_DAILY_RUNTIME"
sqlite3 "$STOCK_DB" "INSERT INTO fixture_writes VALUES ('daily');"
echo daily-stdout
echo daily-stderr >&2
exit "$DAILY_EXIT_CODE"
""")
        self.write_tool("backfill_predictions", """#!/bin/bash
printf 'prediction\\n' >> "$TEST_CODE_DAILY_CALLS"
printf 'cwd=%s\\ndb=%s\\n' "$PWD" "$STOCK_DB" >> "$TEST_CODE_DAILY_RUNTIME"
sqlite3 "$STOCK_DB" "INSERT INTO fixture_writes VALUES ('prediction');"
echo 'TEST_CODE prediction diagnostic: deferred rows remain pending'
exit "$PREDICTION_EXIT_CODE"
""")

    def write_tool(self, name, content):
        path = self.root / "bin" / name
        path.write_text(content)
        path.chmod(0o755)

    def pending(self):
        with sqlite3.connect(self.db) as connection:
            connection.executemany("INSERT INTO prediction_tracker VALUES (?,?,?,?)", [
                ("TEST_CODE_B", None, None, None),
                ("TEST_CODE_A", 1, None, 3),
                ("TEST_CODE_A", None, None, None),
                ("TEST_CODE_COMPLETE", 1, 2, 3),
                ("", None, None, None),
                (None, None, None, None),
            ])

    def run_job(self):
        return subprocess.run(["/bin/bash", str(SCRIPT)], env=self.env,
            cwd=self.root / "commands", capture_output=True, text=True, timeout=15)

    def report(self, result):
        lines = [line.removeprefix("[daily] report ") for line in result.stdout.splitlines()
            if line.startswith("[daily] report ")]
        self.assertEqual(len(lines), 1, result.stdout + result.stderr)
        return json.loads(lines[0])

    def stages(self):
        with sqlite3.connect(self.db) as connection:
            return [row[0] for row in connection.execute("SELECT stage FROM fixture_writes")]

    def test_success_selects_distinct_pending_codes_and_runs_both_stages(self):
        self.pending()
        result = self.run_job()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), [
            "daily:TEST_CODE_A,TEST_CODE_B", "prediction"])
        self.assertEqual(self.stages(), ["daily", "prediction"])
        self.assertEqual((self.root / "runtime.txt").read_text().splitlines(), [
            f"cwd={self.root}", f"db={self.db}",
            f"cwd={self.root}", f"db={self.db}",
        ])
        self.assertFalse((self.root / "unselected.db").exists())
        self.assertEqual(self.report(result), {
            "schema_version": 1, "selection_exit_code": 0,
            "daily_exit_code": 0, "prediction_exit_code": 0, "exit_code": 0,
            "outcome_result_status": "not_evaluated_by_wrapper",
        })

    def test_daily_failure_is_returned_and_partial_writes_are_preserved(self):
        self.pending()
        self.env["DAILY_EXIT_CODE"] = "7"
        result = self.run_job()
        self.assertEqual(result.returncode, 7, result.stdout + result.stderr)
        self.assertEqual(self.stages(), ["daily", "prediction"])
        report = self.report(result)
        self.assertEqual((report["daily_exit_code"], report["prediction_exit_code"]), (7, 0))
        log = (self.root / "logs/backfill-daily.log").read_text()
        self.assertIn("daily-stdout", log)
        self.assertIn("daily-stderr", log)

    def test_prediction_failure_is_returned_after_successful_daily_writes(self):
        self.pending()
        self.env["PREDICTION_EXIT_CODE"] = "19"
        result = self.run_job()
        self.assertEqual(result.returncode, 19, result.stdout + result.stderr)
        self.assertEqual(self.stages(), ["daily", "prediction"])
        report = self.report(result)
        self.assertEqual((report["daily_exit_code"], report["prediction_exit_code"]), (0, 19))
        self.assertIn("deferred rows remain pending", result.stdout)

    def test_two_failures_keep_both_statuses_and_return_first_failure(self):
        self.pending()
        self.env.update(DAILY_EXIT_CODE="7", PREDICTION_EXIT_CODE="19")
        result = self.run_job()
        self.assertEqual(result.returncode, 7, result.stdout + result.stderr)
        report = self.report(result)
        self.assertEqual((report["daily_exit_code"], report["prediction_exit_code"]), (7, 19))
        self.assertEqual(self.stages(), ["daily", "prediction"])

    def test_no_pending_codes_skips_daily_and_still_verifies_predictions(self):
        result = self.run_job()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.calls.read_text(), "prediction\n")
        report = self.report(result)
        self.assertIsNone(report["daily_exit_code"])
        self.assertEqual(report["prediction_exit_code"], 0)
        self.assertEqual(report["outcome_result_status"], "not_evaluated_by_wrapper")

    def test_sqlite_failure_is_returned_before_any_backfill(self):
        path = self.root / "commands/sqlite3"
        path.write_text("#!/bin/bash\necho TEST_CODE_selection_failed >&2\nexit 23\n")
        path.chmod(0o755)
        result = self.run_job()
        self.assertEqual(result.returncode, 23, result.stdout + result.stderr)
        self.assertFalse(self.calls.exists())
        report = self.report(result)
        self.assertEqual(report["selection_exit_code"], 23)
        self.assertIsNone(report["daily_exit_code"])
        self.assertIsNone(report["prediction_exit_code"])
        self.assertEqual(self.stages(), [])

    def test_invalid_database_schema_fails_without_writes(self):
        with sqlite3.connect(self.db) as connection:
            connection.execute("DROP TABLE prediction_tracker")
        original = self.db.read_bytes()
        result = self.run_job()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.calls.exists())
        self.assertEqual(self.db.read_bytes(), original)
        self.assertIsNone(self.report(result)["prediction_exit_code"])

    def test_missing_database_is_not_created(self):
        self.db.unlink()
        result = self.run_job()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertFalse(self.db.exists())
        self.assertFalse(self.calls.exists())
        self.assertIsNone(self.report(result)["prediction_exit_code"])

    def test_relative_runtime_root_is_rejected_before_any_backfill(self):
        self.env["STOCK_ANALYSIS_RUNTIME_ROOT"] = ".."
        result = self.run_job()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertFalse(self.calls.exists())
        self.assertIsNone(self.report(result)["prediction_exit_code"])

    def test_missing_daily_tool_is_failure_but_prediction_still_runs(self):
        self.pending()
        (self.root / "bin/backfill_daily").unlink()
        result = self.run_job()
        self.assertEqual(result.returncode, 127, result.stdout + result.stderr)
        self.assertEqual(self.calls.read_text(), "prediction\n")
        report = self.report(result)
        self.assertEqual((report["daily_exit_code"], report["prediction_exit_code"]), (127, 0))


if __name__ == "__main__":
    unittest.main()
