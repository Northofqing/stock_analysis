#!/usr/bin/env python3
"""Behavior checks use only temporary TEST_CODE databases and local files."""
from dataclasses import replace
import json
from pathlib import Path
import shutil
import sqlite3
import struct
import tempfile
import unittest
from unittest.mock import patch

import paper_source_recovery as recovery


AUDIT_COLS = ("id", "schema_version", "failure_identity", "as_of_date", "stage",
              "reason_code", "diagnostic", "source_row_count", "source_fill_ids_json",
              "source_facts_json", "source_snapshot_hash", "diagnostic_hash",
              "observed_at", "minimum_retention_years", "created_at")


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.backup = self.root / "original.db"
        self.target = self.root / "target.db"
        self.candidate = self.root / "candidate.json"
        self.ids = (2, 3)
        c = sqlite3.connect(self.backup)
        types = ["INTEGER PRIMARY KEY AUTOINCREMENT", "TEXT", "TEXT", "TEXT", "TEXT",
                 "REAL", "INTEGER", "TEXT", "REAL", "TEXT", "TEXT", "TEXT", "TEXT",
                 "TIMESTAMP", "TIMESTAMP"]
        c.execute("CREATE TABLE paper_trades (" + ",".join(
            n + " " + t for n, t in zip(recovery.COLS, types)) + ")")
        self.rows = []
        for i in range(1, 6):
            self.rows.append((i, "TEST_CODE_plan_" + str(i), "TEST_CODE", "fixture", "Buy",
                              0.07333333333333333 + i, 100, "Filled",
                              0.07333333333333333 + i, None, "TEST_CODE", "Frozen", "Unsafe",
                              "2026-08-01 10:00:00", "2026-08-01 10:00:01"))
        c.executemany("INSERT INTO paper_trades VALUES (" + ",".join("?" for _ in recovery.COLS) + ")",
                      self.rows)
        c.execute("CREATE TABLE account(id INTEGER PRIMARY KEY, value TEXT)")
        c.execute("INSERT INTO account VALUES(1, 'TEST_CODE_confirmed')")
        c.execute("CREATE TABLE paper_inventory_failure_audit (" + ",".join(
            n + (" INTEGER" if n in ("id", "schema_version", "source_row_count", "minimum_retention_years")
                 else " TEXT") for n in AUDIT_COLS) + ")")
        c.execute("CREATE TABLE paper_inventory_failure_audit_chain(failure_audit_id INTEGER, previous_hash TEXT, record_hash TEXT, created_at TEXT)")
        facts = []
        for i in self.ids:
            r = dict(zip(recovery.COLS, self.rows[i - 1]))
            facts.append({"id": i, "code": r["code"], "name": r["name"], "direction": r["direction"],
                          "fill_price_bits": struct.pack(">d", r["fill_price"]).hex(),
                          "quantity": r["quantity"], "occurred_at": r["ts"]})
        text = recovery.compact(facts).decode()
        source_hash = recovery.hash_fields(b"BR249_PAPER_INVENTORY_SOURCE_SNAPSHOT_V1\0", text)
        diagnostic = "TEST_CODE_preserved_original_failure"
        diagnostic_hash = recovery.hash_fields(b"BR249_PAPER_INVENTORY_DIAGNOSTIC_V1\0", diagnostic)
        identity = recovery.hash_fields(b"BR249_PAPER_INVENTORY_FAILURE_IDENTITY_V1\0",
                                       "2026-09-01", "rebuild_fifo", source_hash, diagnostic_hash)
        a = dict(zip(AUDIT_COLS, (1, 1, identity, "2026-09-01", "rebuild_fifo",
                 "paper_inventory_rebuild_failed", diagnostic, len(facts),
                 recovery.compact(list(self.ids)).decode(), text, source_hash, diagnostic_hash,
                 "2026-09-01T01:00:00Z", 5, "2026-09-01T01:00:00Z")))
        tip = recovery.hash_fields(b"BR249_PAPER_INVENTORY_FAILURE_RECORD_V1\0",
                "BR249_PAPER_INVENTORY_FAILURE_AUDIT_GENESIS_V1", recovery.compact(a))
        c.execute("INSERT INTO paper_inventory_failure_audit VALUES (" + ",".join("?" for _ in a) + ")", tuple(a.values()))
        c.execute("INSERT INTO paper_inventory_failure_audit_chain VALUES (?,?,?,?)",
                  (1, "BR249_PAPER_INVENTORY_FAILURE_AUDIT_GENESIS_V1", tip, "TEST_CODE"))
        c.execute("CREATE TABLE attribution_sample_epoch_receipt(id INTEGER, epoch_id TEXT, receipt_hash TEXT, legacy_filled_manifest_hash TEXT, paper_trade_high_water INTEGER)")
        c.execute("INSERT INTO attribution_sample_epoch_receipt VALUES (1,?,?,?,5)", ("e" * 64, "f" * 64, "a" * 64))
        c.execute("CREATE TABLE attribution_sample_epoch_receipt_chain(id INTEGER, record_hash TEXT)")
        c.execute("INSERT INTO attribution_sample_epoch_receipt_chain VALUES(1,?)", ("f" * 64,))
        c.execute("CREATE TABLE attribution_legacy_carry_item(id INTEGER, quantity INTEGER)")
        c.execute("INSERT INTO attribution_legacy_carry_item VALUES(1,100)")
        c.commit()
        c.close()
        review = {"applied": False, "candidates": {"rows": [
            {**dict(zip(recovery.COLS, self.rows[i - 1])), "executed": False} for i in self.ids]}}
        self.candidate.write_bytes(recovery.compact(review))
        self.pins = recovery.RecoveryPins(recovery.digest_file(self.backup),
            recovery.digest_file(self.candidate), self.ids, 5, 5, 1, 1,
            source_hash, tip, "e" * 64, "f" * 64, "a" * 64,
            ("account", "paper_inventory_failure_audit", "paper_inventory_failure_audit_chain",
             "attribution_sample_epoch_receipt"))
        shutil.copyfile(self.backup, self.target)
        self.sql("DELETE FROM paper_trades WHERE id IN (2,3)")
        self.monitor = self.root / "TEST_CODE_monitor"
        self.monitor.write_bytes(b"TEST_CODE_guarded_artifact")
        self.readback = self.root / "guarded-readback.json"
        self.readback.write_bytes(recovery.compact({
            "mode": "readonly_guarded_recovery_validation", "guarded_price_dispute_ids": list(self.ids),
            "net_summary": "Unavailable", "account_anchor_available": False, "original_count": 5}))
        self.proof = self.root / "guard-proof.json"
        self.proof.write_bytes(recovery.compact({
            "schema": "paper-source-recovery-guard-v1", "disputed_fill_ids": list(self.ids),
            "backup_sha256": self.pins.backup_sha256, "net_summary": "Unavailable",
            "account_anchor_rejected": True, "monitor_path": str(self.monitor),
            "monitor_sha256": recovery.digest_file(self.monitor),
            "economic_readback_path": str(self.readback),
            "economic_readback_sha256": recovery.digest_file(self.readback)}))

    def sql(self, sql):
        with sqlite3.connect(self.target) as c:
            c.execute(sql)

    def run_case(self, **kw):
        return recovery.recover(self.target, self.backup, self.candidate, pins=self.pins, **kw)

    def counts(self):
        with sqlite3.connect(self.target) as c:
            return (c.execute("SELECT COUNT(*) FROM paper_trades").fetchone()[0],
                    c.execute("SELECT COUNT(*) FROM account").fetchone()[0])

    def test_preview_preserves_every_byte_and_inode(self):
        before = self.target.read_bytes(), self.target.stat().st_ino
        r = self.run_case()
        self.assertEqual(r["restore_ids"], list(self.ids))
        self.assertFalse(r["committed"])
        self.assertFalse(r["price_qualification_issued"])
        self.assertEqual(before, (self.target.read_bytes(), self.target.stat().st_ino))

    def test_apply_restores_exact_float_bits_and_preserves_other_facts(self):
        before_inode = self.target.stat().st_ino
        receipt = self.root / "receipt.jsonl"
        r = self.run_case(apply=True, guard_proof=self.proof, receipt=receipt)
        self.assertTrue(r["committed"])
        with sqlite3.connect(self.target) as c:
            got = list(c.execute("SELECT * FROM paper_trades ORDER BY id"))
            self.assertEqual([recovery.row_key(x) for x in got], [recovery.row_key(x) for x in self.rows])
            self.assertEqual(c.execute("SELECT seq FROM sqlite_sequence WHERE name='paper_trades'").fetchone(), (5,))
        self.assertEqual(self.target.stat().st_ino, before_inode)
        self.assertEqual(self.counts(), (5, 1))
        events = [json.loads(l) for l in receipt.read_text().splitlines()]
        self.assertEqual([e["event"] for e in events], ["intent", "committed"])
        self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)
        repeated = self.run_case(apply=True, guard_proof=self.proof, receipt=self.root / "repeat.jsonl")
        self.assertTrue(repeated["already_restored"])
        self.assertEqual(repeated["restore_ids"], [])
        self.assertEqual(self.counts(), (5, 1))

    def test_unexpected_surviving_change_or_partial_recovery_is_rejected(self):
        self.sql("UPDATE paper_trades SET quantity=101 WHERE id=1")
        with self.assertRaisesRegex(ValueError, "surviving"):
            self.run_case()
        self.sql("UPDATE paper_trades SET quantity=100 WHERE id=1")
        with sqlite3.connect(self.target) as c:
            c.execute("INSERT INTO paper_trades VALUES (" + ",".join("?" for _ in recovery.COLS) + ")", self.rows[1])
        with self.assertRaisesRegex(ValueError, "partial"):
            self.run_case()
        self.assertEqual(self.counts(), (4, 1))

    def test_bad_backup_or_audit_identity_cannot_authorize_recovery(self):
        with self.assertRaisesRegex(ValueError, "backup SHA256"):
            recovery.recover(self.target, self.backup, self.candidate,
                             pins=replace(self.pins, backup_sha256="0" * 64))
        self.sql("UPDATE paper_inventory_failure_audit SET diagnostic='TEST_CODE_changed'")
        with self.assertRaisesRegex(ValueError, "source identity"):
            self.run_case(apply=True, guard_proof=self.proof, receipt=self.root / "rejected.jsonl")
        self.assertEqual(self.counts(), (3, 1))
        self.assertFalse((self.root / "rejected.jsonl").exists())

    def test_missing_or_changed_guard_stops_before_any_insert(self):
        with self.assertRaisesRegex(ValueError, "requires.*guard"):
            self.run_case(apply=True, receipt=self.root / "absent.jsonl")
        self.monitor.write_bytes(b"TEST_CODE_unguarded_replaced_artifact")
        with self.assertRaisesRegex(ValueError, "guard artifact"):
            self.run_case(apply=True, guard_proof=self.proof, receipt=self.root / "changed.jsonl")
        self.assertEqual(self.counts(), (3, 1))

    def test_unchanged_hash_string_cannot_hide_epoch_receipt_chain_or_carry_tampering(self):
        for table, column, bad, original in (
                ("attribution_sample_epoch_receipt", "paper_trade_high_water", 6, 5),
                ("attribution_sample_epoch_receipt_chain", "record_hash", "0" * 64, "f" * 64),
                ("attribution_legacy_carry_item", "quantity", 101, 100)):
            with self.subTest(table=table):
                with sqlite3.connect(self.target) as c:
                    c.execute('UPDATE "' + table + '" SET "' + column + '"=?', (bad,))
                receipt = self.root / (table + ".jsonl")
                with self.assertRaisesRegex(ValueError, "frozen epoch original rows changed"):
                    self.run_case(apply=True, guard_proof=self.proof, receipt=receipt)
                self.assertFalse(receipt.exists())
                self.assertEqual(self.counts(), (3, 1))
                with sqlite3.connect(self.target) as c:
                    c.execute('UPDATE "' + table + '" SET "' + column + '"=?', (original,))

    def test_trigger_side_effect_rolls_back_all_inserts_and_keeps_intent(self):
        self.sql("CREATE TRIGGER TEST_CODE_unexpected AFTER INSERT ON paper_trades BEGIN INSERT INTO account VALUES(NEW.id, 'TEST_CODE_side_effect'); END")
        receipt = self.root / "rollback.jsonl"
        with self.assertRaisesRegex(ValueError, "trigger"):
            self.run_case(apply=True, guard_proof=self.proof, receipt=receipt)
        self.assertEqual(self.counts(), (3, 1))
        events = [json.loads(l) for l in receipt.read_text().splitlines()]
        self.assertEqual(len(events), 1)
        self.assertFalse(events[0]["committed"])

    def test_unsynced_receipt_directory_stops_before_any_insert(self):
        receipt = self.root / "unsynced.jsonl"
        with patch.object(recovery, "sync_receipt_directory", side_effect=OSError("TEST_CODE_fsync")):
            with self.assertRaisesRegex(OSError, "TEST_CODE_fsync"):
                self.run_case(apply=True, guard_proof=self.proof, receipt=receipt)
        self.assertEqual(self.counts(), (3, 1))
        self.assertEqual([json.loads(l)["event"] for l in receipt.read_text().splitlines()], ["intent"])

    def test_available_guard_readback_is_rejected_even_when_artifact_hash_matches(self):
        readback = json.loads(self.readback.read_text())
        readback["net_summary"] = "Available"
        self.readback.write_bytes(recovery.compact(readback))
        proof = json.loads(self.proof.read_text())
        proof["economic_readback_sha256"] = recovery.digest_file(self.readback)
        self.proof.write_bytes(recovery.compact(proof))
        receipt = self.root / "invalid-readback.jsonl"
        with self.assertRaisesRegex(ValueError, "guard readback"):
            self.run_case(apply=True, guard_proof=self.proof, receipt=receipt)
        self.assertEqual(self.counts(), (3, 1))
        self.assertFalse(receipt.exists())

    def test_existing_receipt_cannot_be_overwritten_or_cause_partial_writes(self):
        receipt = self.root / "existing.jsonl"
        receipt.write_bytes(b"TEST_CODE_existing_evidence")
        with self.assertRaises(FileExistsError):
            self.run_case(apply=True, guard_proof=self.proof, receipt=receipt)
        self.assertEqual(receipt.read_bytes(), b"TEST_CODE_existing_evidence")
        self.assertEqual(self.counts(), (3, 1))

    def test_missing_database_is_never_created(self):
        missing = self.root / "absent.db"
        with self.assertRaises(FileNotFoundError):
            recovery.recover(missing, self.backup, self.candidate, pins=self.pins)
        self.assertFalse(missing.exists())


if __name__ == "__main__":
    unittest.main()
