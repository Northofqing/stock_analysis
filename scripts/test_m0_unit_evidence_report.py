"""Focused query-shape tests; this fixture is not the full production v9 schema."""

import json
from contextlib import closing
from pathlib import Path
import sqlite3
import tempfile
import unittest

from m0_unit_evidence_report import build_report


ROOT = Path(__file__).resolve().parents[1]
DAY = "2026-09-28"


class M0UnitEvidenceReportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.db = Path(self.temp.name) / "query-shape-v9.sqlite3"
        with closing(sqlite3.connect(self.db)) as connection, connection:
            connection.executescript(
                """
                PRAGMA user_version=9;
                CREATE TABLE delivery_decisions (
                    decision_identity TEXT PRIMARY KEY,
                    business_date TEXT NOT NULL,
                    push_kind TEXT NOT NULL,
                    state TEXT NOT NULL,
                    envelope_canonical BLOB
                );
                CREATE TABLE sink_results (
                    result_event_identity TEXT PRIMARY KEY,
                    decision_identity TEXT NOT NULL,
                    result_kind TEXT NOT NULL,
                    authoritative_for_state INTEGER NOT NULL,
                    late_after_fence INTEGER NOT NULL,
                    result_canonical BLOB,
                    message_id TEXT
                );
                """
            )

    def decision(self, identity, kind, state="Delivered", day=DAY):
        with closing(sqlite3.connect(self.db)) as connection, connection:
            connection.execute(
                "INSERT INTO delivery_decisions VALUES (?,?,?,?,?)",
                (identity, day, kind, state, b"SECRET_SOURCE_CONTENT"),
            )

    def result(self, identity, decision, kind="Accepted", authority=1, late=0):
        with closing(sqlite3.connect(self.db)) as connection, connection:
            connection.execute(
                "INSERT INTO sink_results VALUES (?,?,?,?,?,?,?)",
                (identity, decision, kind, authority, late, b"SECRET_SINK_CONTENT", "SECRET_MESSAGE"),
            )

    def report(self, from_date=DAY, to_date=DAY):
        return build_report(ROOT, self.db, from_date, to_date)

    def test_catalog_has_52_unattributed_units_and_only_news_ai_delta(self):
        report = self.report()
        units = report["units"]
        self.assertEqual(len(units), 52)
        self.assertEqual(len({unit["id"] for unit in units}), 52)
        self.assertEqual(
            len({producer["id"] for unit in units for producer in unit["producers"]}), 102
        )
        catalog = json.loads(
            (ROOT / "docs/push-system/push-capability-catalog.v1.json").read_text()
        )
        self.assertEqual(
            {unit["id"]: [producer["id"] for producer in unit["producers"]] for unit in units},
            {unit["id"]: unit["producer_ids"] for unit in catalog["migration_units"]},
        )
        by_unit = {unit["id"]: unit for unit in units}
        self.assertEqual(
            by_unit["MU-news-ai"]["producers"][0]["current_kinds"], ["NewsAiAnalysis"]
        )
        self.assertTrue(
            all(
                producer["current_kinds"] == ["NewsToIdea"]
                for producer in by_unit["MU-d01"]["producers"]
            )
        )
        self.assertEqual(
            sum(not producer["current_kinds"] for unit in units for producer in unit["producers"]),
            10,
        )
        self.assertEqual(
            by_unit["MU-cli-replay-force"]["producers"][0]["current_kinds"], []
        )
        for unit in units:
            self.assertEqual(unit["source_status"], "CatalogOnly")
            self.assertEqual(unit["correlation"], "NotRecorded")
            self.assertEqual(unit["delivery"], "Unknown")
            self.assertEqual(unit["terminal"], "Unknown")
            self.assertEqual(unit["finalizer"], "Unknown")

    def test_shared_kind_and_multiple_results_do_not_attribute_a_unit(self):
        self.decision("SECRET_DECISION", "MarketActionAlert")
        self.result("SECRET_RESULT_1", "SECRET_DECISION")
        self.result("SECRET_RESULT_2", "SECRET_DECISION")
        self.result("SECRET_RESULT_LATE", "SECRET_DECISION", late=1)
        self.result("SECRET_RESULT_WEAK", "SECRET_DECISION", authority=0)

        report = self.report()
        self.assertEqual(
            report["unattributed_kind_candidates"],
            [
                {
                    "business_date": DAY,
                    "push_kind": "MarketActionAlert",
                    "state": "Delivered",
                    "decisions": 1,
                    "accepted_decision_candidates": 1,
                    "accepted_result_events": 2,
                }
            ],
        )
        by_unit = {unit["id"]: unit for unit in report["units"]}
        for unit_id in ("MU-frozen-side", "MU-order-alert"):
            self.assertEqual(by_unit[unit_id]["delivery"], "Unknown")
            self.assertEqual(by_unit[unit_id]["correlation"], "NotRecorded")
        self.assertNotIn("SECRET_DECISION", json.dumps(report))

    def test_uncertain_result_is_not_accepted_and_dates_are_bounded(self):
        self.decision("SECRET_UNCERTAIN", "DataMode", "UncertainManualReview")
        self.result("SECRET_RESULT_U", "SECRET_UNCERTAIN", "Uncertain")
        self.decision("OUTSIDE_WINDOW", "DataMode", day="2026-09-27")
        self.result("OUTSIDE_RESULT", "OUTSIDE_WINDOW")
        rows = self.report()["unattributed_kind_candidates"]
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["state"], "UncertainManualReview")
        self.assertEqual(rows[0]["decisions"], 1)
        self.assertEqual(rows[0]["accepted_decision_candidates"], 0)
        self.assertEqual(rows[0]["accepted_result_events"], 0)

    def test_rejects_invalid_date_schema_version_and_missing_column(self):
        for start, end in (("20260928", DAY), ("2026-02-30", DAY), (DAY, "2026-09-27")):
            with self.subTest(start=start, end=end), self.assertRaises(ValueError):
                self.report(start, end)
        with closing(sqlite3.connect(self.db)) as connection, connection:
            connection.execute("PRAGMA user_version=8")
        with self.assertRaisesRegex(ValueError, "schema version"):
            self.report()
        with closing(sqlite3.connect(self.db)) as connection, connection:
            connection.executescript(
                """
                PRAGMA user_version=9;
                DROP TABLE sink_results;
                CREATE TABLE sink_results (
                    result_event_identity TEXT,
                    decision_identity TEXT,
                    result_kind TEXT,
                    authoritative_for_state INTEGER
                );
                """
            )
        with self.assertRaisesRegex(ValueError, "missing durable-delivery columns"):
            self.report()

    def test_read_only_query_does_not_expose_ids_or_change_database(self):
        self.decision("SECRET_ACCOUNT_DECISION", "NewsAiAnalysis")
        self.result("SECRET_RESULT_ID", "SECRET_ACCOUNT_DECISION")
        before = self.db.read_bytes()
        files_before = {path.name for path in self.db.parent.iterdir()}
        report = self.report()
        self.assertEqual(self.db.read_bytes(), before)
        self.assertEqual({path.name for path in self.db.parent.iterdir()}, files_before)
        output = json.dumps(report, ensure_ascii=False)
        for secret in (
            "SECRET_ACCOUNT_DECISION",
            "SECRET_RESULT_ID",
            "SECRET_SOURCE_CONTENT",
            "SECRET_SINK_CONTENT",
            "SECRET_MESSAGE",
            str(self.db),
        ):
            self.assertNotIn(secret, output)


if __name__ == "__main__":
    unittest.main()
