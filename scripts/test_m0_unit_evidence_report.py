"""Focused query-shape tests; fixtures are not the full production schema."""

import json
from contextlib import closing
from pathlib import Path
import sqlite3
import tempfile
import unittest

from m0_unit_evidence_report import build_report, _origin_observation_identity


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
                "INSERT INTO delivery_decisions "
                "(decision_identity,business_date,push_kind,state,envelope_canonical) "
                "VALUES (?,?,?,?,?)",
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

    def upgrade_v10(self):
        with closing(sqlite3.connect(self.db)) as connection, connection:
            connection.executescript(
                """
                PRAGMA user_version=10;
                ALTER TABLE delivery_decisions ADD COLUMN sub_kind TEXT NOT NULL DEFAULT 'None';
                ALTER TABLE delivery_decisions ADD COLUMN scope_key TEXT NOT NULL DEFAULT 'GLOBAL';
                CREATE TABLE delivery_correlation_observations (
                    observation_identity TEXT,
                    identity_version INTEGER,
                    decision_identity TEXT,
                    producer_id TEXT,
                    occurrence_identity TEXT,
                    role TEXT,
                    observed_at TEXT
                );
                """
            )

    def observe(self, decision, producer="p01-scheduled", occurrence=f"p01:{DAY}",
                observed_at="2026-09-28T03:00:00.000Z"):
        identity = _origin_observation_identity(decision, producer, occurrence)
        with closing(sqlite3.connect(self.db)) as connection, connection:
            connection.execute(
                "INSERT INTO delivery_correlation_observations VALUES (?,?,?,?,?,?,?)",
                (identity, 1, decision, producer, occurrence, "Origin", observed_at),
            )
        return identity

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

    def test_v10_empty_sidecar_is_unattributed_and_orphan_fails_closed(self):
        self.upgrade_v10()
        report = self.report()
        self.assertEqual(report["schema_version"], 10)
        self.assertTrue(all(unit["correlation"] == "NotRecorded" for unit in report["units"]))

        with closing(sqlite3.connect(self.db)) as connection, connection:
            connection.execute(
                "INSERT INTO delivery_correlation_observations VALUES (?,?,?,?,?,?,?)",
                (
                    "SECRET_OBSERVATION",
                    1,
                    "SECRET_DECISION",
                    "p01-scheduled",
                    "p01:2026-09-28",
                    "Origin",
                    "2026-09-28T03:00:00Z",
                ),
            )
        with self.assertRaisesRegex(ValueError, "orphan v10"):
            self.report()

        with closing(sqlite3.connect(self.db)) as connection, connection:
            connection.execute("DROP TABLE delivery_correlation_observations")
        with self.assertRaisesRegex(ValueError, "missing durable-delivery columns"):
            self.report()

    def test_v10_p01_origin_attribution_counts_durable_candidates_once(self):
        self.upgrade_v10()
        decision = "a" * 64
        self.decision(decision, "PreopenNewsHot")
        self.result("SECRET_RESULT_1", decision)
        self.result("SECRET_RESULT_2", decision)
        self.result("SECRET_RESULT_LATE", decision, late=1)
        self.decision("SECRET_UNOBSERVED", "PreopenNewsHot")
        self.result("SECRET_UNOBSERVED_RESULT", "SECRET_UNOBSERVED")
        self.observe(decision)
        self.observe(decision, producer="p01-compensation")

        report = self.report()
        p01 = next(unit for unit in report["units"] if unit["id"] == "MU-p01")
        self.assertEqual(p01["correlation"], "Observed")
        self.assertEqual(p01["delivery"], "Unknown")
        self.assertEqual(p01["terminal"], "Unknown")
        self.assertEqual(p01["finalizer"], "Unknown")
        self.assertEqual(p01["correlated_durable_candidates"], [{
            "business_date": DAY,
            "push_kind": "PreopenNewsHot",
            "state": "Delivered",
            "decisions": 1,
            "accepted_decision_candidates": 1,
            "accepted_result_events": 2,
            "observed_producer_ids": ["p01-compensation", "p01-scheduled"],
        }])
        self.assertEqual(report["unattributed_kind_candidates"], [{
            "business_date": DAY,
            "push_kind": "PreopenNewsHot",
            "state": "Delivered",
            "decisions": 1,
            "accepted_decision_candidates": 1,
            "accepted_result_events": 1,
        }])
        output = json.dumps(report)
        for secret in (decision, "SECRET_RESULT_1", "SECRET_UNOBSERVED"):
            self.assertNotIn(secret, output)

    def test_v10_origin_identity_is_compatible_with_rust_golden(self):
        self.assertEqual(
            _origin_observation_identity("0" * 64, "p01-scheduled", "p01:2026-08-18"),
            "e5c9454f53641a7d13fed11e7d1290085f266ffe48f7a4c3e99fe9614646eaf1",
        )

    def test_v10_rejects_forged_or_misbound_origin(self):
        self.upgrade_v10()
        decision = "b" * 64
        self.decision(decision, "PreopenNewsHot")
        identity = self.observe(decision)
        replacements = (
            ("observation_identity", "0" * 64),
            ("producer_id", "not-registered"),
            ("occurrence_identity", "p01:2026-09-27"),
            ("role", "Resume"),
            ("identity_version", 2),
            ("observed_at", "2026-09-28T03:00:00Z"),
        )
        for column, replacement in replacements:
            with self.subTest(column=column):
                with closing(sqlite3.connect(self.db)) as connection, connection:
                    connection.execute(
                        f"UPDATE delivery_correlation_observations SET {column}=?",
                        (replacement,),
                    )
                with self.assertRaisesRegex(ValueError, "invalid v10"):
                    self.report()
                with closing(sqlite3.connect(self.db)) as connection, connection:
                    connection.execute(
                        "DELETE FROM delivery_correlation_observations"
                    )
                    connection.execute(
                        "INSERT INTO delivery_correlation_observations VALUES (?,?,?,?,?,?,?)",
                        (identity, 1, decision, "p01-scheduled", f"p01:{DAY}",
                         "Origin", "2026-09-28T03:00:00.000Z"),
                    )
        with closing(sqlite3.connect(self.db)) as connection, connection:
            connection.execute(
                "UPDATE delivery_decisions SET push_kind='DataMode' WHERE decision_identity=?",
                (decision,),
            )
        with self.assertRaisesRegex(ValueError, "invalid v10"):
            self.report()

    def test_rollback_journal_fixture_does_not_expose_ids_or_change_database(self):
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

    def test_wal_read_only_connection_can_create_sidecars(self):
        self.decision("SECRET_WAL_DECISION", "DataMode")
        self.result("SECRET_WAL_RESULT", "SECRET_WAL_DECISION")
        with closing(sqlite3.connect(self.db)) as connection, connection:
            self.assertEqual(connection.execute("PRAGMA journal_mode=WAL").fetchone(), ("wal",))
        before = self.db.read_bytes()
        self.assertFalse(self.db.with_name(self.db.name + "-wal").exists())
        self.assertFalse(self.db.with_name(self.db.name + "-shm").exists())

        report = self.report()

        self.assertEqual(self.db.read_bytes(), before)
        self.assertTrue(self.db.with_name(self.db.name + "-wal").exists())
        self.assertTrue(self.db.with_name(self.db.name + "-shm").exists())
        self.assertEqual(report["unattributed_kind_candidates"][0]["decisions"], 1)


if __name__ == "__main__":
    unittest.main()
