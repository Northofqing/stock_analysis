import copy
import datetime as dt
import fcntl
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import monitor_watchdog as w
from reliability_common import Calendar, clock, open_dir, read_at

ROOT = Path(__file__).resolve().parents[1]


def health(now='2026-10-09T10:00:00+08:00'):
    return {'status': 'ok', 'runtime_snapshot': {'version': 2, 'checked_at': now,
        'process': {'status': 'ok', 'reason_code': None, 'monitor_running': True,
                    'boot_identity_sha256': 'a'*64, 'heartbeat_observed_at': now, 'heartbeat_fresh': True},
        'account': {'status': 'ok', 'reason_code': None, 'mode': 'Normal', 'metrics_complete': True, 'evaluated_at': now},
        'data': {'status': 'ok', 'reason_code': None, 'mode': 'Full', 'evaluated_at': now, 'missing_capabilities': []},
        'raw_global_news': {'recovery': {'status': 'ok'}},
        'durable_delivery': {'status': 'observed', 'counts': {'non_progressable_manual_reviews': 0}}}}


class WatchdogTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self.calendar = Calendar(ROOT / 'config/a_share_market_holidays.csv')
        self.now = clock('2026-10-09T10:00:00+08:00')

    def tearDown(self):
        self.tmp.cleanup()

    def classify(self, report, at=None, prior=None):
        return w.classify(report, clock(at) if at else self.now, self.calendar, prior)

    def test_real_unhealthy_schema_secrets_and_scopes(self):
        report = health()
        report['status'] = 'unhealthy'
        report['runtime_snapshot']['account'].update(status='unhealthy', mode='Frozen', metrics_complete=False)
        report['runtime_snapshot']['data']['missing_capabilities'] = ['MoneyFlow', 'secret_token=hidden']
        report['secret'] = 'hidden'
        report['runtime_snapshot']['raw_global_news']['recovery']['status'] = 'warming'
        incidents, observation, _ = self.classify(report)
        self.assertEqual(observation['health_status'], 'unhealthy')
        self.assertFalse(any(i['severity'] == 'P0' for i in incidents))
        self.assertIn('missing_MoneyFlow', str(incidents))
        self.assertNotIn('hidden', json.dumps([incidents, observation]))

    def test_process_all_failure_conditions_immediate(self):
        for changes in ({'monitor_running': False}, {'reason_code': 'process_heartbeat_process_mismatch', 'status': 'unhealthy'},
                        {'heartbeat_observed_at': None}, {'heartbeat_observed_at': '2026-10-09T09:49:59+08:00'},
                        {'heartbeat_observed_at': '2026-10-09T10:00:06+08:00'}):
            report = health()
            report['runtime_snapshot']['process'].update(changes)
            report['runtime_snapshot']['process']['status'] = 'unhealthy'
            incidents, _, _ = self.classify(report)
            self.assertTrue(any(i['rule'] == 'process' and i['severity'] == 'P0' for i in incidents))

    def test_verified_sessions_two_polls_and_gaps(self):
        report = health()
        report['runtime_snapshot']['data']['missing_capabilities'] = ['Quote']
        incidents, _, poll = self.classify(report)
        self.assertEqual(next(i['severity'] for i in incidents if i['rule'] == 'data'), 'P1')
        report2 = health('2026-10-09T10:01:00+08:00')
        report2['runtime_snapshot']['data']['missing_capabilities'] = ['Quote']
        incidents, _, _ = self.classify(report2, '2026-10-09T10:01:00+08:00', poll)
        self.assertEqual(next(i['severity'] for i in incidents if i['rule'] == 'data'), 'P0')
        incidents, _, _ = self.classify(report2, '2026-10-09T10:03:00+08:00', poll)
        self.assertEqual(next(i['severity'] for i in incidents if i['rule'] == 'data'), 'P1')
        for at, session in [('2026-10-09T09:20:00+08:00', 'Auction'), ('2026-10-09T09:25:00+08:00', 'Closed'),
                            ('2026-10-09T11:30:00+08:00', 'LunchBreak'), ('2026-10-09T13:00:00+08:00', 'Afternoon'),
                            ('2026-10-09T15:00:00+08:00', 'AfterHours'), ('2026-10-10T10:00:00+08:00', 'Closed'),
                            ('2026-10-01T10:00:00+08:00', 'Closed')]:
            r = health(at)
            r['runtime_snapshot']['data']['missing_capabilities'] = ['Quote']
            incidents, observation, _ = self.classify(r, at)
            self.assertEqual(observation['session'], session)
            self.assertFalse(any(i['severity'] == 'P0' for i in incidents))
            if session in ('Closed', 'LunchBreak', 'AfterHours'):
                self.assertFalse(any(i['rule'] == 'data' for i in incidents))

    def test_stale_data_unavailable_and_year_coverage(self):
        report = health()
        report['runtime_snapshot']['data']['evaluated_at'] = None
        incidents, _, poll = self.classify(report)
        self.assertIn('data_observation_unavailable', str(incidents))
        report['runtime_snapshot']['checked_at'] = '2026-10-09T10:01:00+08:00'
        incidents, _, _ = self.classify(report, '2026-10-09T10:01:00+08:00', poll)
        self.assertTrue(any(i['rule'] == 'data' and i['severity'] == 'P0' for i in incidents))
        incidents, _, _ = self.classify(health('2027-01-04T10:00:00+08:00'), '2027-01-04T10:00:00+08:00')
        self.assertIn('calendar_unavailable', str(incidents))

    def test_bounded_probe_accepts_exit_one_and_sanitizes_errors(self):
        binary = self.root / 'target/release/monitor'
        binary.parent.mkdir(parents=True)
        def program(body):
            binary.write_text('#!/usr/bin/env python3\n' + body)
            binary.chmod(0o700)
        report = health()
        report['status'] = 'unhealthy'
        program('import json\nprint(' + repr(json.dumps(report)) + ')\nraise SystemExit(1)\n')
        self.assertIsNone(w.probe(self.root)[1])
        for body, reason in [('import time\ntime.sleep(1)', 'probe_timeout'),
                             ('raise SystemExit(2)', 'probe_exit_unavailable'),
                             ("print('secret-not-json')", 'probe_schema_unavailable'),
                             ("print('x'*70000)", 'probe_output_oversized'),
                             ("import sys\nsys.stderr.write('private-secret')\nraise SystemExit(2)", 'probe_exit_unavailable')]:
            program(body)
            result, error = w.probe(self.root, .1)
            self.assertIsNone(result)
            self.assertEqual(error, reason)
            self.assertNotIn('secret', error)

    def test_dedup_recovery_reminder_persistence_failure_and_private_files(self):
        fd = open_dir(self.root, private=True)
        self.addCleanup(os.close, fd)
        report = health()
        report['runtime_snapshot']['process'].update(status='unhealthy', reason_code='process_heartbeat_missing', heartbeat_observed_at=None)
        def persist(r, at, prior):
            incidents, observation, poll = self.classify(r, at, prior)
            observation['output_root'] = str(self.root)
            return w.persist_incidents(fd, incidents, observation, poll, prior, clock(at))
        state = persist(report, self.now.isoformat(), {})
        self.assertEqual(len(list((self.root / 'events').glob('*.json'))), 1)
        state = persist(report, '2026-10-09T10:01:00+08:00', state)
        self.assertEqual(len(list((self.root / 'events').glob('*.json'))), 1)
        # A valid report is required to recover: a failed probe retains the unresolved process incident.
        state = persist(None, '2026-10-09T10:02:00+08:00', state)
        self.assertTrue(any(v['rule'] == 'process' for v in state['active'].values()))
        state = persist(health('2026-10-09T11:02:00+08:00'), '2026-10-09T11:02:00+08:00', state)
        events = [json.loads(p.read_bytes()) for p in (self.root / 'events').glob('*.json')]
        self.assertEqual(sum(e['action'] == 'recovery' and e['rule'] == 'process' for e in events), 1)
        before = read_at(fd, 'state.json', private=True)[0]
        with patch.object(w, 'write_at', side_effect=OSError('injected')):
            with self.assertRaises(OSError):
                persist(report, '2026-10-09T11:03:00+08:00', state)
        self.assertEqual(read_at(fd, 'state.json', private=True)[0], before)
        for p in self.root.rglob('*'):
            self.assertEqual(stat.S_IMODE(p.stat().st_mode), 0o700 if p.is_dir() else 0o600)
        self.assertTrue(list(self.root.glob('self-check-*.json')))

    def test_p0_reminder_and_severity_change(self):
        fd = open_dir(self.root, private=True)
        self.addCleanup(os.close, fd)
        report = health()
        report['runtime_snapshot']['data']['missing_capabilities'] = ['Quote']
        state = {}
        for at in ('2026-10-09T10:00:00+08:00', '2026-10-09T10:01:00+08:00', '2026-10-09T11:01:00+08:00'):
            r = health(at)
            r['runtime_snapshot']['process'].update(status='unhealthy', heartbeat_observed_at=None, reason_code='process_heartbeat_missing')
            incidents, observation, poll = self.classify(r, at, state)
            observation['output_root'] = str(self.root)
            state = w.persist_incidents(fd, incidents, observation, poll, state, clock(at))
        self.assertEqual(sum(json.loads(p.read_bytes())['action'] == 'reminder' for p in (self.root / 'events').glob('*.json')), 1)

    def test_severity_change_survives_restart_and_missing_schema_is_unavailable(self):
        fd = open_dir(self.root, private=True)
        self.addCleanup(os.close, fd)
        state = {}
        for at in ('2026-10-09T10:00:00+08:00', '2026-10-09T10:01:00+08:00'):
            report = health(at)
            report['runtime_snapshot']['data']['missing_capabilities'] = ['Quote']
            incidents, observation, poll = self.classify(report, at, state)
            observation['output_root'] = str(self.root)
            w.persist_incidents(fd, incidents, observation, poll, state, clock(at))
            state = json.loads(read_at(fd, 'state.json', private=True)[0])
        events = [json.loads(p.read_bytes()) for p in (self.root / 'events').glob('*.json')]
        self.assertEqual({e['action'] for e in events}, {'first', 'changed'})
        self.assertEqual({e['severity'] for e in events}, {'P0', 'P1'})
        invalid = health()
        del invalid['runtime_snapshot']['account']['mode']
        with self.assertRaises(KeyError):
            w.validate_report(invalid)
        invalid = health()
        invalid['runtime_snapshot']['process']['boot_identity_sha256'] = None
        with self.assertRaises(ValueError):
            w.validate_report(invalid)

    def test_overlap_cli_and_symlink_outputs(self):
        lock = self.root / '.watchdog.lock'
        lock.touch(mode=0o600)
        with lock.open('r+') as held:
            fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = subprocess.run(['python3', str(ROOT / 'scripts/monitor_watchdog.py'), '--runtime-root', str(self.root),
                '--calendar', str(ROOT / 'config/a_share_market_holidays.csv'), '--output-root', str(self.root)], capture_output=True)
            self.assertEqual(result.returncode, 0)
            self.assertIn(b'overlap_skipped', result.stdout)
        link = self.root / 'link'
        link.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            open_dir(link, private=True)


if __name__ == '__main__':
    unittest.main()
