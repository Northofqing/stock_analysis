import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import unittest

import reliability_quality_screen as q
import rotate_push_log as r
from reliability_common import Calendar, digest, exclusive_json

ROOT = Path(__file__).resolve().parents[1]


@contextmanager
def writer(path):
    connection = sqlite3.connect(path)
    try:
        yield connection
        connection.commit()
    finally:
        connection.close()


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()


def envelope(decision):
    return dict(envelope_version=1, decision_identity=decision, business_date='2026-07-01',
                push_kind='PaperTrade', sub_kind='Default', cooldown_scope='PerKind', scope_key='global',
                schedule_occurrence_identity='occurrence', source_evidence_fingerprint='source',
                source_binding_canonical=list(b'source'), source_binding_sha256=digest(b'source'),
                delivery_subject_hash='a'*64, rendered_content=list(b'body'), rendered_content_sha256=digest(b'body'),
                policy_version=7, retry_authorized=False, provider_observed_at=None, provider_as_of=None,
                original_batch_ids=[], task_binding=None)


class QualityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self.main = self.root / 'main.db'
        self.delivery = self.root / 'delivery.db'
        self.push = self.root / 'data/push_log'
        self.day = self.push / '2026-07-01'
        self.day.mkdir(parents=True)
        (self.push / '.push_log.lock').touch(mode=0o600)
        self.calendar = Calendar(ROOT / 'config/a_share_market_holidays.csv')
        with writer(self.main) as c:
            c.executescript('''CREATE TABLE prediction_tracker(id INTEGER PRIMARY KEY,pred_date TEXT,target_date TEXT);
                CREATE TABLE order_audit(id INTEGER PRIMARY KEY,business_order_id TEXT,code TEXT,execution_price REAL,
                    quote_observed_at TEXT,created_at TEXT,outcome TEXT);''')
            c.execute("INSERT INTO order_audit VALUES(1,'order','600000',10.0,'2026-07-01T10:00:00+08:00','2026-07-01T10:00:00+08:00','Filled')")
        with writer(self.delivery) as c:
            c.executescript('''PRAGMA user_version=9;
                CREATE TABLE delivery_decisions(decision_identity TEXT PRIMARY KEY,envelope_version INTEGER,envelope_canonical BLOB,envelope_sha256 TEXT);
                CREATE TABLE delivery_attempts(attempt_identity TEXT PRIMARY KEY,decision_identity TEXT,fence_token INTEGER);
                CREATE TABLE sink_results(result_event_identity TEXT PRIMARY KEY,attempt_identity TEXT,decision_identity TEXT,
                    result_kind TEXT,observed_at TEXT,fence_token INTEGER,authoritative_for_state INTEGER,late_after_fence INTEGER,
                    result_canonical BLOB,result_sha256 TEXT,channel TEXT,provider TEXT,message_id TEXT,platform_message_id TEXT,
                    accepted_at TEXT,latency_ms INTEGER);''')
            raw = encode(envelope('decision'))
            c.execute('INSERT INTO delivery_decisions VALUES(?,?,?,?)', ('decision', 1, raw, digest(raw)))
            c.execute("INSERT INTO delivery_attempts VALUES('attempt','decision',1)")

    def tearDown(self):
        self.tmp.cleanup()

    def reader(self, path):
        source = sqlite3.connect(path)
        c = sqlite3.connect(':memory:')
        try:
            source.backup(c)
        finally:
            source.close()
        c.row_factory = sqlite3.Row
        c.execute('PRAGMA query_only=ON')
        c.execute('BEGIN')
        self.addCleanup(c.close)
        return c

    def accepted(self, event, physical, late=0, kind='Accepted'):
        receipt = {'channel': 'feishu', 'provider': 'Feishu', 'message_id': physical,
                   'platform_message_id': physical, 'accepted_at': '2026-07-01T02:00:00Z', 'latency_ms': 10}
        raw = json.dumps({'kind': kind, 'receipt': receipt}, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()
        with writer(self.delivery) as c:
            c.execute('INSERT INTO sink_results VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
                      (event, 'attempt', 'decision', kind, '2026-07-01T10:00:00+08:00', 1, 1-late, late,
                       raw, digest(raw), receipt['channel'], receipt['provider'], physical, physical, receipt['accepted_at'], 10))

    def test_exact_body_duplicate_without_physical_evidence_and_rotated_inclusion(self):
        (self.day / 'a.md').write_bytes(b'body\r\n')
        (self.day / 'b.md').write_bytes(b'body\r\n')
        archive = self.root / 'archive'
        r.rotate(self.root, archive, archive=True, observed_at='2026-10-09T10:00:00+08:00')
        report = q.screen_archives(self.push, archive, 100)
        self.assertEqual(len(report['candidates']), 1)
        self.assertEqual(len(report['candidates'][0]['occurrences']), 2)  # copies are not extra occurrences
        r.rotate(self.root, archive, archive=True, prune=True, observed_at='2026-10-09T10:00:00+08:00')
        rotated = q.screen_archives(self.push, archive, 100)
        self.assertEqual(len(rotated['candidates'][0]['occurrences']), 2)
        self.assertEqual(rotated['physical_delivery_inference'], 'unavailable_from_markdown')
        self.assertEqual(q.screen_physical(self.reader(self.delivery), 100)['candidates'], [])

    def test_pending_committed_pair_one_attempt_and_missing_join(self):
        pending = {'schema': 'stock_analysis.counted_push_log.v1', 'state': 'AuditPending',
                   'decision_identity': 'decision', 'attempt_identity': 'attempt',
                   'decision_identity_hash': q.domain_hash('stock_analysis.counted_decision_identity.v1', b'decision'),
                   'attempt_identity_hash': q.domain_hash('stock_analysis.counted_attempt_identity.v1', b'attempt'),
                   'rendered_content': 'body', 'rendered_content_sha256': digest(b'body'),
                   'durable_push_kind': 'PaperTrade', 'stable_template_id': 'paper_trade', 'fence_token': 1,
                   'observed_at': '2026-07-01T02:00:00Z',
                   'receipt_sha256': q.domain_hash('stock_analysis.counted_receipt.none.v1', b'NO_VALIDATED_RECEIPT'),
                   'sink_result': {'kind': 'Uncertain', 'uncertainty': {'reason_code': 'unknown', 'evidence': 'not observed'}}}
        result_bytes = json.dumps(pending['sink_result'], sort_keys=True, separators=(',', ':')).encode()
        pending['sink_result_sha256'] = q.domain_hash('stock_analysis.counted_sink_result.v1', result_bytes)
        raw = encode(pending)
        (self.day / 'pending.json').write_bytes(raw)
        commit = {'schema': pending['schema'], 'state': 'Committed',
                  'decision_identity_hash': pending['decision_identity_hash'], 'attempt_identity_hash': pending['attempt_identity_hash'],
                  'pending_artifact_sha256': q.domain_hash('stock_analysis.counted_push_log_artifact.v1', raw),
                  'durable_push_kind': 'PaperTrade', 'stable_template_id': 'paper_trade', 'delivery_audit_event_id': 'audit',
                  'counted_join_hash': 'a'*64, 'committed_at': '2026-07-01T02:00:00Z'}
        (self.day / 'committed.json').write_bytes(encode(commit))
        report = q.screen_archives(self.push, None, 100)
        self.assertEqual(len(report['counted_attempts']), 1)
        self.assertEqual(len(report['counted_attempts'][0]['committed_paths']), 1)
        wrong = dict(pending, sink_result_sha256=digest(result_bytes))  # plain DB hash is invalid in counted artifact domain
        (self.day / 'wrong-domain.json').write_bytes(encode(wrong))
        checked = q.screen_archives(self.push, None, 100)
        self.assertTrue(any(f['path'].endswith('wrong-domain.json') for f in checked['unavailable_files']))
        self.assertEqual(len(checked['counted_attempts'][0]['pending_paths']), 1)
        commit['pending_artifact_sha256'] = '0'*64
        (self.day / 'broken.json').write_bytes(encode(commit))
        report = q.screen_archives(self.push, None, 100)
        self.assertIn('unavailable_pending_commit_join', str(report))
        self.assertEqual(report['candidates'], [])

    def test_same_receipt_collapses_distinct_ids_candidate_including_late(self):
        self.accepted('one', 'physical1')
        self.accepted('repeat', 'physical1')
        first = q.screen_physical(self.reader(self.delivery), 100)
        self.assertEqual(first['distinct_verified_physical_observations'], 1)
        self.assertEqual(first['candidates'], [])
        self.accepted('late', 'physical2', late=1)
        self.accepted('unknown', 'unknown', kind='Uncertain')
        report = q.screen_physical(self.reader(self.delivery), 100)
        self.assertEqual(len(report['candidates']), 1)
        self.assertEqual(report['distinct_verified_physical_observations'], 2)
        self.assertEqual(len(report['late_after_fence_evidence']), 1)
        self.assertEqual(len(report['unknown_uncertain']), 1)

    def test_schema_hash_missing_join_fail_closed_and_limits(self):
        self.accepted('one', 'physical1')
        self.accepted('two', 'physical2')
        with writer(self.delivery) as c:
            c.execute("UPDATE sink_results SET result_sha256='invalid' WHERE result_event_identity='one'")
        report = q.screen_physical(self.reader(self.delivery), 100)
        self.assertEqual(report['candidates'], [])
        self.assertEqual(report['status'], 'partial_unavailable')
        report = q.screen_physical(self.reader(self.delivery), 1)
        self.assertTrue(report['truncated'])
        with writer(self.delivery) as c:
            c.execute('PRAGMA user_version=8')
        self.assertEqual(q.screen_physical(self.reader(self.delivery), 100)['status'], 'unavailable')

    def test_target_dates_weekend_holiday_invalid_coverage_and_weekend_creation(self):
        rows = [(1, '2026-10-09', '2026-10-10'), (2, '2026-09-30', '2026-10-01'),
                (3, '2026-10-10', '2026-10-12'), (4, 'bad', '2026-10-12'),
                (5, '2026-10-09', '2027-01-04')]
        with writer(self.main) as c:
            c.executemany('INSERT INTO prediction_tracker VALUES(?,?,?)', rows)
        report = q.screen_r3(self.reader(self.main), self.calendar, 100)
        self.assertEqual([r['id'] for r in report['candidates']], [1, 2])
        self.assertEqual([r['id'] for r in report['unavailable_rows']], [4, 5])
        self.assertTrue(all(r['calendar_sha256'] == self.calendar.sha256 for r in report['candidates']))
        self.assertTrue(q.screen_r3(self.reader(self.main), self.calendar, 1)['truncated'])

    def test_r1_r5_supplied_json_never_admitted_and_raw_diagnostics_preserved(self):
        evidence = self.root / 'self-asserted.json'
        # Includes flags, off-day band, tick boundaries, exception and apparent seed/lot facts.
        evidence.write_text(json.dumps({'qualified': True, 'same_day': True, 'date': '2026-06-30', 'lower': 9,
             'upper': 11, 'tick': '.01', 'exception': 'new_listing', 'seed': 1000, 'sells': 100, 'buys': 0,
             'account': 'other', 'lot': 'same_day_buy', 'sellable': True}))
        before = evidence.read_bytes()
        r1 = q.screen_r1(self.reader(self.main), 100, evidence)
        r5 = q.screen_r5(self.reader(self.main), evidence)
        self.assertEqual(r1['status'], 'unavailable')
        self.assertEqual(r5['status'], 'unavailable')
        self.assertEqual(len(r1['raw_diagnostics']), 1)
        self.assertFalse(r1['supplied_json_admitted'])
        self.assertFalse(r5['supplied_json_admitted'])
        self.assertEqual(evidence.read_bytes(), before)

    def test_freeze_hash_provenance_and_unqualified_legacy_paper_scope(self):
        with writer(self.main) as c:
            c.executescript('CREATE TABLE candidate_board_prediction_freeze_v2(occurrence_identity TEXT,business_date TEXT,target_date TEXT,calendar_authority_hash TEXT,source_canonical BLOB,source_sha256 TEXT,rendered_bytes BLOB,rendered_sha256 TEXT);\n                CREATE TABLE candidate_board_prediction_member_v2(prediction_row_id INTEGER,occurrence_identity TEXT);\n                CREATE TABLE paper_trades(id INTEGER,plan_id TEXT,code TEXT,direction TEXT,quantity INTEGER,status TEXT,fill_price REAL,ts TEXT);')
            c.execute("INSERT INTO prediction_tracker VALUES(1,'2026-10-09','2026-10-10')")
            c.execute('INSERT INTO candidate_board_prediction_freeze_v2 VALUES(?,?,?,?,?,?,?,?)',
                      ('occurrence', '2026-10-09', '2026-10-10', 'b'*64, b'source', digest(b'source'), b'body', digest(b'body')))
            c.execute("INSERT INTO candidate_board_prediction_member_v2 VALUES(1,'occurrence')")
            c.execute("INSERT INTO paper_trades VALUES(1,'plan','600000','sell',100,'Filled',10.0,'2026-07-01')")
        report = q.screen_r3(self.reader(self.main), self.calendar, 100)
        self.assertEqual(report['candidates'][0]['freeze_provenance'], 'hash_verified')
        self.assertEqual(report['candidates'][0]['freeze']['occurrence_identity'], 'occurrence')
        paper = q.screen_r5(self.reader(self.main))
        self.assertEqual(paper['status'], 'unavailable')
        self.assertEqual(paper['raw_diagnostics'][0]['direction'], 'sell')
        self.assertEqual(paper['candidates'], [])

    def test_logical_wal_snapshot_source_bytes_schemas_unchanged_exclusive_output(self):
        # Keep live WAL connection open so the logical backup must include uncheckpointed frames.
        live = sqlite3.connect(self.main)
        self.addCleanup(live.close)
        live.execute('PRAGMA journal_mode=WAL')
        live.execute("INSERT INTO prediction_tracker VALUES(1,'2026-10-09','2026-10-10')")
        live.commit()
        before = {str(p): p.read_bytes() for p in (self.main, Path(str(self.main)+'-wal'), self.delivery)}
        schemas = list(live.execute('SELECT name,sql FROM sqlite_master ORDER BY name'))
        args = argparse.Namespace(db=self.main, delivery_db=self.delivery, push_log=self.push,
                calendar=ROOT / 'config/a_share_market_holidays.csv', archive_root=None,
                price_authority_evidence=None, paper_evidence=None, observed_at='2026-10-09T10:00:00+08:00', max_rows=100, max_files=100)
        report = q.run(args)
        self.assertEqual(len(report['rules']['R3']['candidates']), 1)
        self.assertEqual(before, {path: Path(path).read_bytes() for path in before})
        self.assertEqual(schemas, list(live.execute('SELECT name,sql FROM sqlite_master ORDER BY name')))
        self.assertEqual(len(report['sources']), 2)
        self.assertTrue(all(source['live_wal_included'] for source in report['sources']))
        output = self.root / 'result.json'
        exclusive_json(output, report)
        with self.assertRaises(FileExistsError):
            exclusive_json(output, report)
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)


if __name__ == '__main__':
    unittest.main()
