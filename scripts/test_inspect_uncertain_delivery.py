import hashlib
import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from inspect_uncertain_delivery import inspect_database


class InspectionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "original.sqlite3"
        with sqlite3.connect(self.path) as db:
            db.executescript("""
                PRAGMA user_version=9;
                CREATE TABLE delivery_decisions(
                    decision_identity TEXT, business_date TEXT, push_kind TEXT,
                    sub_kind TEXT, cooldown_scope TEXT, scope_key TEXT, state TEXT,
                    envelope_canonical BLOB, envelope_sha256 TEXT,
                    current_cooldown_reservation_identity TEXT, created_at TEXT, updated_at TEXT);
                CREATE TABLE delivery_policy_catalog(push_kind TEXT,sub_kind TEXT,cooldown_scope TEXT,window_mode TEXT);
                CREATE TABLE cooldown_heads(push_kind TEXT,sub_kind TEXT,cooldown_scope TEXT,scope_key TEXT,current_reservation_identity TEXT,state TEXT,blocked_until TEXT);
                CREATE TABLE cooldown_reservations(cooldown_reservation_identity TEXT,decision_identity TEXT);
                CREATE TABLE business_date_once_claims(business_date TEXT,push_kind TEXT,sub_kind TEXT,scope_key TEXT,decision_identity TEXT);
                CREATE TABLE sink_results(result_event_identity TEXT,decision_identity TEXT,result_kind TEXT,observed_at TEXT,authoritative_for_state INTEGER,late_after_fence INTEGER,platform_message_id TEXT,accepted_at TEXT,result_canonical BLOB,result_sha256 TEXT);
            """)

    def add_record(self, identity, kind="CloseCall", mode="Rolling", current=True):
        raw = json.dumps({"TEST_CODE_identity": identity}).encode()
        with sqlite3.connect(self.path) as db:
            db.execute("INSERT INTO delivery_decisions VALUES(?,?,?,?,?,?,?,?,?,?,?,?)", (
                identity, "2026-10-08", kind, "NONE", "Stock", identity, "UncertainManualReview",
                raw, hashlib.sha256(raw).hexdigest(), identity + "-reservation", "created", "updated",
            ))
            db.execute("INSERT INTO delivery_policy_catalog VALUES(?,?,?,?)", (kind, "NONE", "Stock", mode))
            db.execute("INSERT INTO cooldown_reservations VALUES(?,?)", (identity + "-reservation", identity))
            db.execute("INSERT INTO cooldown_heads VALUES(?,?,?,?,?,?,?)", (
                kind, "NONE", "Stock", identity, identity + "-reservation" if current else "later-reservation", "Uncertain", None,
            ))

    def test_current_rolling_block_differs_from_superseded_and_once_claim(self):
        self.add_record("current")
        self.add_record("old", kind="T0Advice", current=False)
        self.add_record("once", kind="DataMode", mode="BusinessDateOnce")
        with sqlite3.connect(self.path) as db:
            db.execute("INSERT INTO business_date_once_claims VALUES(?,?,?,?,?)", ("2026-10-08", "DataMode", "NONE", "once", "once"))
        before = self.path.read_bytes()
        report = inspect_database(self.path)
        rows = {row["decision_identity"]: row for row in report["records"]}
        self.assertEqual(report["manual_review_count"], 3)
        self.assertEqual(report["blocking_rolling_scope_count"], 1)
        self.assertFalse(rows["old"]["owns_current_cooldown_head"])
        self.assertTrue(rows["once"]["holds_original_business_date_claim"])
        self.assertFalse(rows["once"]["blocks_new_rolling_scope"])
        self.assertEqual(before, self.path.read_bytes())

    def test_accepted_and_late_receipts_do_not_resolve_uncertain(self):
        self.add_record("original")
        raw = b'{"TEST_CODE_receipt":"accepted"}'
        with sqlite3.connect(self.path) as db:
            db.execute("INSERT INTO sink_results VALUES(?,?,?,?,?,?,?,?,?,?)", (
                "late-accepted", "original", "Accepted", "observed", 0, 1, "TEST_CODE_platform_message", "accepted",
                raw, hashlib.sha256(raw).hexdigest(),
            ))
        before = self.path.read_bytes()
        row = inspect_database(self.path)["records"][0]
        self.assertEqual(row["state"], "UncertainManualReview")
        self.assertEqual(row["sink_results"][0]["authoritative_for_state"], 0)
        self.assertEqual(row["required_action"], "human_review_of_original_evidence")
        self.assertEqual(before, self.path.read_bytes())

    def test_tampered_originals_and_wrong_reservation_owner_are_reported(self):
        self.add_record("original")
        with sqlite3.connect(self.path) as db:
            db.execute("UPDATE delivery_decisions SET envelope_sha256='wrong'")
            db.execute("UPDATE cooldown_reservations SET decision_identity='another'")
        row = inspect_database(self.path)["records"][0]
        self.assertEqual(set(row["integrity_issues"]), {"original_envelope_hash_mismatch", "original_cooldown_reservation_owner_mismatch"})

    def test_missing_database_is_not_created_and_other_schema_is_not_migrated(self):
        missing = self.path.with_name("missing.sqlite3")
        with self.assertRaises(FileNotFoundError):
            inspect_database(missing)
        self.assertFalse(missing.exists())
        with sqlite3.connect(self.path) as db:
            db.execute("PRAGMA user_version=14")
        before = self.path.read_bytes()
        with self.assertRaisesRegex(ValueError, "Schema9"):
            inspect_database(self.path)
        self.assertEqual(before, self.path.read_bytes())

    def test_cli_private_output_is_exclusive_and_database_cannot_be_overwritten(self):
        self.add_record("original")
        script = Path(__file__).with_name("inspect_uncertain_delivery.py")
        output = self.path.with_suffix(".json")
        command = [sys.executable, str(script), "--db", str(self.path), "--output", str(output)]
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)
        self.assertEqual(json.loads(output.read_text())["manual_review_count"], 1)
        saved = output.read_bytes()
        self.assertEqual(subprocess.run(command, capture_output=True).returncode, 1)
        self.assertEqual(saved, output.read_bytes())
        before = self.path.read_bytes()
        command[-1] = str(self.path)
        self.assertEqual(subprocess.run(command, capture_output=True).returncode, 1)
        self.assertEqual(before, self.path.read_bytes())


if __name__ == "__main__":
    unittest.main()
