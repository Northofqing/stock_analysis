import datetime as dt
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import stat
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import monitor_watchdog as w
import watchdog_mobile as mobile
from reliability_common import Calendar, clock, digest, json_bytes, open_dir, read_at, write_at
from test_monitor_watchdog import health

ROOT = Path(__file__).resolve().parents[1]
SECRET = 'test_private_device_key_1234'


class MobileTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self.fd = open_dir(self.root, private=True)
        self.now = clock('2026-10-09T10:00:00+08:00')
        self.calendar = Calendar(ROOT / 'config/a_share_market_holidays.csv')
        self.config = self.root / 'bark.private.json'
        self.write_config()
        self.state = self.persist()

    def tearDown(self):
        os.close(self.fd)
        self.tmp.cleanup()

    def write_config(self, **overrides):
        value = dict(schema_version=1, enabled=True, adapter='bark',
                     endpoint='https://api.day.app/push', device_key=SECRET)
        value.update(overrides)
        write_at(self.fd, self.config.name, json_bytes(value), replace=True)

    def persist(self, prior=None, at=None, report=None):
        at = at or self.now
        if report is None:
            report = health(at.isoformat())
            report['runtime_snapshot']['process'].update(status='unhealthy',
                reason_code='process_heartbeat_missing', heartbeat_observed_at=None)
        incidents, observation, poll = w.classify(report, at, self.calendar, prior)
        observation['output_root'] = str(self.root)
        return w.persist_incidents(self.fd, incidents, observation, poll, prior or {}, at)

    def dispatch(self, sender=None, at=None, state=None, config='default'):
        return mobile.dispatch_mobile(self.fd, self.root, state or self.state,
            at or self.now, self.config if config == 'default' else config,
            sender or (lambda *_: mobile.outcome('Accepted', 'bark_service_accepted', 200)))

    def receipt(self, event_hash=None):
        event_hash = event_hash or self.state['mobile_outbox'][0]['event_sha256']
        return json.loads((self.root / 'mobile-receipts' / (event_hash + '.json')).read_bytes())

    def test_absent_disabled_invalid_configuration_never_sends_and_keeps_local(self):
        def forbidden(*_):
            self.fail('network call without valid opt-in')
        self.assertEqual(self.dispatch(forbidden, config=None)['configuration_status'], 'unconfigured')
        self.assertEqual(self.dispatch(forbidden, config=self.root / 'missing.json')['configuration_status'], 'unconfigured')
        self.write_config(enabled=False)
        self.assertEqual(self.dispatch(forbidden)['configuration_status'], 'disabled')
        self.config.chmod(0o644)
        self.assertEqual(self.dispatch(forbidden)['configuration_status'], 'invalid')
        self.assertTrue((self.root / 'state.json').exists())
        self.assertTrue((self.root / 'latest.json').exists())
        self.assertFalse((self.root / 'mobile-receipts').exists())

    def test_private_config_rejects_symlink_parent_credentials_in_url_and_http(self):
        for endpoint in ['https://example.com/push?key=secret', 'https://user:secret@example.com/push',
                         'https://example.com/private-key', 'http://example.com/push',
                         'https://example.com/push#fragment', 'https://example.com/push\n']:
            self.write_config(endpoint=endpoint)
            self.assertEqual(mobile.load_config(self.config)[1], 'invalid')
        self.write_config()
        linked = self.root / 'linked.json'
        linked.symlink_to(self.config)
        self.assertEqual(mobile.load_config(linked)[1], 'invalid')
        other = self.root / 'other'
        other.mkdir(mode=0o755)
        path = other / 'config.json'
        path.write_bytes(self.config.read_bytes())
        path.chmod(0o600)
        self.assertEqual(mobile.load_config(path)[1], 'invalid')
        self.assertNotIn(SECRET, repr(mobile.load_config(self.config)[0]))

    def test_accepted_is_service_only_and_exact_event_retry_after_restart_is_deduped(self):
        sent = []
        def send(config, message):
            sent.append(message)
            self.assertEqual(config.device_key, SECRET)
            return mobile.outcome('Accepted', 'bark_service_accepted', 200)
        self.assertEqual(self.dispatch(send)['attempted'], 1)
        reloaded = json.loads(read_at(self.fd, 'state.json', private=True)[0])
        self.assertEqual(self.dispatch(send, at=self.now + dt.timedelta(seconds=60), state=reloaded)['attempted'], 0)
        self.assertEqual(len(sent), 1)
        self.assertEqual(self.receipt()['status'], 'Accepted')
        self.assertEqual(self.receipt()['attempts'], 1)
        for path in self.root.rglob('*.json'):
            if path != self.config:
                self.assertNotIn(SECRET, path.read_text())
        summary = json.loads((self.root / 'latest-mobile.json').read_bytes())
        self.assertIn('phone_delivery_unobserved', summary['scope'])

    def test_unknown_is_never_retried_even_after_config_change(self):
        calls = []
        def send(*_):
            calls.append(1)
            return mobile.outcome('Unknown', 'send_or_response_unknown')
        self.dispatch(send)
        self.write_config(device_key='different_private_device_9999')
        self.dispatch(send, at=self.now + dt.timedelta(seconds=60))
        self.assertEqual(len(calls), 1)
        self.assertEqual(self.receipt()['status'], 'Unknown')

    def test_crash_after_inflight_or_after_accepted_before_receipt_never_resends(self):
        for crash in ('sender', 'receipt'):
            with self.subTest(crash=crash):
                # New private output for each independent crash scenario.
                event_hash = self.state['mobile_outbox'][0]['event_sha256']
                receipt_path = self.root / 'mobile-receipts' / (event_hash + '.json')
                if receipt_path.exists():
                    receipt_path.unlink()
                for guard in (self.root / 'mobile-receipts').glob('subject-*.json'):
                    guard.unlink()
                count = []
                def sender(*_):
                    count.append(1)
                    if crash == 'sender':
                        raise OSError('private remote exception')
                    return mobile.outcome('Accepted', 'bark_service_accepted', 200)
                original = mobile.write_at
                def publish(fd, name, raw, replace=False):
                    if crash == 'receipt' and json.loads(raw).get('status') == 'Accepted':
                        raise OSError('publication crash')
                    return original(fd, name, raw, replace)
                with patch.object(mobile, 'write_at', side_effect=publish):
                    with self.assertRaises(OSError):
                        self.dispatch(sender)
                self.assertEqual(self.receipt()['status'], 'InFlight')
                self.dispatch(sender, at=self.now + dt.timedelta(seconds=60))
                self.assertEqual(self.receipt()['status'], 'Unknown')
                self.assertEqual(len(count), 1)

    def test_durable_before_attempt_failure_prevents_any_network(self):
        calls = []
        with patch.object(mobile, 'write_at', side_effect=OSError('disk unavailable')):
            with self.assertRaises(OSError):
                self.dispatch(lambda *_: calls.append(1))
        self.assertEqual(calls, [])

    def test_verified_recovery_then_new_outage_is_a_new_incident_not_a_retry(self):
        calls = []
        def send(*_):
            calls.append(1)
            return mobile.outcome('Unknown', 'send_or_response_unknown') if len(calls) == 1 else mobile.outcome('Accepted', 'bark_service_accepted', 200)
        self.dispatch(send)
        recovered_at = self.now + dt.timedelta(seconds=60)
        recovered = self.persist(self.state, recovered_at, health(recovered_at.isoformat()))
        self.dispatch(send, at=recovered_at, state=recovered)
        failed_at = recovered_at + dt.timedelta(seconds=60)
        failed = self.persist(recovered, failed_at)
        self.dispatch(send, at=failed_at, state=failed)
        self.assertEqual(len(calls), 3)
        latest = failed['mobile_outbox'][-1]['event_sha256']
        self.assertEqual(self.receipt(latest)['status'], 'Accepted')

    def test_recovery_committed_without_dispatch_then_new_generation_sends_only_new_outage(self):
        calls = []
        def retryable(*_):
            calls.append(1)
            return mobile.outcome('FailedRetryable', 'connect_failed_before_request')
        self.dispatch(retryable)
        original_hash = self.state['mobile_outbox'][0]['event_sha256']
        recovered_at = self.now + dt.timedelta(seconds=60)
        recovered = self.persist(self.state, recovered_at, health(recovered_at.isoformat()))
        # Crash after local recovery commit: mobile never observed that run.
        failed_at = recovered_at + dt.timedelta(seconds=60)
        failed = self.persist(recovered, failed_at)
        sent = []
        self.dispatch(lambda config, message: sent.append(message) or mobile.outcome('Accepted', 'bark_service_accepted', 200),
                      at=failed_at, state=failed)
        self.assertEqual(len(sent), 1)
        self.assertEqual(self.receipt(original_hash)['status'], 'Suppressed')
        self.assertEqual(self.receipt(failed['mobile_outbox'][-1]['event_sha256'])['status'], 'Accepted')

    def test_pending_recovery_from_older_generation_cannot_send_after_new_generation_recovers(self):
        recovered_at = self.now + dt.timedelta(seconds=60)
        recovered = self.persist(self.state, recovered_at, health(recovered_at.isoformat()))
        old_recovery = recovered['mobile_outbox'][-1]['event_sha256']
        failed_at = recovered_at + dt.timedelta(seconds=60)
        failed = self.persist(recovered, failed_at)
        newest_at = failed_at + dt.timedelta(seconds=60)
        newest = self.persist(failed, newest_at, health(newest_at.isoformat()))
        sent = []
        self.dispatch(lambda config, message: sent.append(message) or mobile.outcome('Accepted', 'bark_service_accepted', 200),
                      at=newest_at, state=newest)
        self.assertEqual(len(sent), 1)
        self.assertEqual(self.receipt(old_recovery)['status'], 'Suppressed')
        self.assertEqual(self.receipt(newest['mobile_outbox'][-1]['event_sha256'])['status'], 'Accepted')

    def test_unknown_cannot_be_bypassed_by_hourly_reminder_even_after_outbox_prune(self):
        calls = []
        def send(*_):
            calls.append(1)
            return mobile.outcome('Unknown', 'send_or_response_unknown')
        self.dispatch(send)
        later = self.now + dt.timedelta(hours=1)
        state = self.persist(self.state, later)
        summary = self.dispatch(send, at=later, state=state)
        self.assertEqual(len(calls), 1)
        newest = state['mobile_outbox'][-1]['event_sha256']
        self.assertEqual(self.receipt(newest)['reason'], 'incident_unknown_manual_review')
        self.assertTrue(summary['manual_review_required'])
        later += dt.timedelta(days=1)
        # The same boot/date incident scope is retained here to exercise durable
        # subject proof even when its original reference is pruned from outbox.
        state = dict(state, mobile_outbox=[state['mobile_outbox'][-1]])
        original = json.loads((self.root / 'events' / (newest + '.json')).read_bytes())
        original['observed_at'] = later.isoformat()
        newer = digest(json_bytes(original))
        fd = open_dir(self.root / 'events', private=True)
        try:
            write_at(fd, newer + '.json', json_bytes(original))
        finally:
            os.close(fd)
        state['mobile_outbox'] = [{'event_sha256': newer, 'observed_at': later.isoformat()}]
        self.dispatch(send, at=later, state=state)
        self.assertEqual(len(calls), 1)
        self.assertEqual(self.receipt(newer)['reason'], 'incident_unknown_manual_review')

    def test_only_proven_pre_request_failure_retries_backoff_and_three_attempt_cap(self):
        calls = []
        def send(*_):
            calls.append(1)
            return mobile.outcome('FailedRetryable', 'connect_failed_before_request')
        self.dispatch(send)
        self.dispatch(send, at=self.now + dt.timedelta(seconds=59))
        self.assertEqual(len(calls), 1)
        self.dispatch(send, at=self.now + dt.timedelta(seconds=60))
        self.dispatch(send, at=self.now + dt.timedelta(seconds=299))
        self.assertEqual(len(calls), 2)
        self.dispatch(send, at=self.now + dt.timedelta(seconds=300))
        self.dispatch(send, at=self.now + dt.timedelta(seconds=360))
        self.assertEqual(len(calls), 3)
        self.assertEqual(self.receipt()['status'], 'FailedFinal')
        self.assertEqual(self.receipt()['reason'], 'retry_limit_reached')

    def test_expired_future_and_resolved_or_escalated_events_do_not_send(self):
        calls = []
        self.dispatch(lambda *_: calls.append(1), at=self.now + dt.timedelta(seconds=900))
        self.assertEqual(calls, [])
        self.assertEqual(self.receipt()['status'], 'Expired')
        event_hash = self.state['mobile_outbox'][0]['event_sha256']
        (self.root / 'mobile-receipts' / (event_hash + '.json')).unlink()
        self.dispatch(lambda *_: calls.append(1), at=self.now - dt.timedelta(seconds=1))
        self.assertEqual(self.receipt()['status'], 'Expired')
        (self.root / 'mobile-receipts' / (event_hash + '.json')).unlink()
        changed = json.loads(json.dumps(self.state))
        changed['active'] = {}
        self.dispatch(lambda *_: calls.append(1), state=changed)
        self.assertEqual(self.receipt()['status'], 'Suppressed')
        self.assertEqual(calls, [])

    def test_corrupt_retry_proof_and_tampered_event_are_fail_closed(self):
        self.dispatch(lambda *_: mobile.outcome('FailedRetryable', 'connect_failed_before_request'))
        value = self.receipt()
        value['reason'] = 'send_or_response_unknown'
        path = self.root / 'mobile-receipts' / (value['event_sha256'] + '.json')
        path.write_bytes(json_bytes(value))
        with self.assertRaises(ValueError):
            self.dispatch(at=self.now + dt.timedelta(seconds=60))
        path.unlink()
        event = self.root / 'events' / (value['event_sha256'] + '.json')
        event.write_bytes(b'{"secret":"private"}')
        with self.assertRaises(ValueError):
            self.dispatch()

    def test_uncommitted_local_event_orphan_is_not_a_mobile_candidate(self):
        before = self.state
        at = self.now + dt.timedelta(seconds=60)
        r = health(at.isoformat())
        r['runtime_snapshot']['data']['missing_capabilities'] = ['Quote']
        original = w.write_at
        def publish(fd, name, raw, replace=False):
            if name == 'state.json':
                raise OSError('state publish failed')
            return original(fd, name, raw, replace)
        with patch.object(w, 'write_at', side_effect=publish):
            with self.assertRaises(OSError):
                self.persist(prior=before, at=at, report=r)
        self.assertGreater(len(list((self.root / 'events').glob('*.json'))), len(before['mobile_outbox']))
        committed = json.loads(read_at(self.fd, 'state.json', private=True)[0])
        sent = []
        self.dispatch(lambda config, message: sent.append(message) or mobile.outcome('Accepted', 'bark_service_accepted', 200), state=committed)
        self.assertEqual(len(sent), 1)
        self.assertIn('process:', sent[0]['body'])
        # Same exact local event retry may reuse its immutable artifact safely.
        completed = self.persist(prior=before, at=at, report=r)
        self.assertEqual(len(completed['mobile_outbox']), 3)

    def test_self_check_requires_opt_in_and_is_service_acknowledged_once_per_day(self):
        self.write_config(daily_self_check=True)
        sent = []
        sender = lambda config, message: sent.append(message) or mobile.outcome('Accepted', 'bark_service_accepted', 200)
        self.assertEqual(self.dispatch(sender)['attempted'], 2)
        self.assertEqual(self.dispatch(sender)['attempted'], 0)
        self.assertEqual(sum('每日自检' in message['title'] for message in sent), 1)

    def test_priority_budget_p2_quiet_by_default_and_files_remain_private(self):
        r = health()
        r['runtime_snapshot']['process'].update(status='unhealthy', heartbeat_observed_at=None)
        r['runtime_snapshot']['account'].update(status='unhealthy', mode='Frozen')
        r['runtime_snapshot']['data']['missing_capabilities'] = ['MoneyFlow', 'Kline', 'Tick', 'OrderBook', 'News']
        state = self.persist(report=r, prior=self.state, at=self.now + dt.timedelta(seconds=60))
        sent = []
        sender = lambda config, message: sent.append(message) or mobile.outcome('Accepted', 'bark_service_accepted', 200)
        self.dispatch(sender, state=state, at=self.now + dt.timedelta(seconds=60))
        self.assertEqual(len(sent), 2)
        self.assertIn('P0', sent[0]['title'])
        self.assertTrue(all('P2' not in message['title'] for message in sent))
        for path in self.root.rglob('*'):
            if not path.is_symlink():
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o700 if path.is_dir() else 0o600)

    def fixture(self, status=200, body=b'{"code":200,"message":"success"}', delay=0):
        records = []
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                records.append((self.path, json.loads(self.rfile.read(int(self.headers['Content-Length'])))))
                time.sleep(delay)
                self.send_response(status)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass
            def log_message(self, *_):
                pass
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        config = mobile.BarkConfig('http://127.0.0.1:' + str(server.server_port) + '/push', SECRET, allow_http_loopback=True)
        return config, records

    def test_bounded_fake_service_accepts_json_body_key_no_secret_in_path_or_receipt(self):
        config, records = self.fixture(body=json_bytes({'code': 200, 'message': SECRET}))
        result = mobile.send_bark(config, {'title': 'P0', 'body': 'process unavailable'})
        self.assertEqual(result['status'], 'Accepted')
        self.assertEqual(records[0][0], '/push')
        self.assertEqual(records[0][1]['device_key'], SECRET)
        self.assertNotIn(SECRET, json.dumps(result))

    def test_http_success_alone_500_redirect_or_oversized_body_are_unknown(self):
        for status, body in [(200, b'{}'), (500, b'{"code":500}'), (302, b'{"code":200}'),
                             (200, b'x' * (mobile.MAX_RESPONSE + 1)), (200, b'{"code":true}')]:
            with self.subTest(status=status, body_size=len(body)):
                config, records = self.fixture(status, body)
                self.assertEqual(mobile.send_bark(config, {'title': 'P0', 'body': 'failure'})['status'], 'Unknown')
                self.assertEqual(len(records), 1)

    def test_documented_400_is_final_and_connection_refusal_is_pre_send_retryable(self):
        config, _ = self.fixture(400, b'{"code":400,"message":"device key invalid"}')
        self.assertEqual(mobile.send_bark(config, {'title': 'P0', 'body': 'failure'})['status'], 'FailedFinal')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
            config = mobile.BarkConfig('http://127.0.0.1:' + str(port) + '/push', SECRET, allow_http_loopback=True)
            result = mobile.send_bark(config, {'title': 'P0', 'body': 'failure'})
            self.assertEqual(result['status'], 'FailedRetryable')
            self.assertEqual(result['reason'], 'connect_failed_before_request')

    def test_worker_total_deadline_is_unknown_no_retry_and_credentials_not_in_argv(self):
        import subprocess
        config, records = self.fixture(delay=.4)
        with patch.object(mobile, 'TIMEOUT_SECONDS', .1):
            begin = time.monotonic()
            result = mobile.send_bark(config, {'title': 'P0', 'body': 'failure'})
            self.assertLess(time.monotonic() - begin, .3)
        self.assertEqual(result['status'], 'Unknown')
        captured = []
        def interrupted(command, **kwargs):
            captured.append((command, kwargs))
            raise subprocess.TimeoutExpired(command, 5)
        with patch.object(mobile.subprocess, 'run', side_effect=interrupted):
            mobile.send_bark(config, {'title': 'P0', 'body': 'failure'})
        self.assertNotIn(SECRET, str(captured[0][0]))
        self.assertNotIn(config.endpoint, str(captured[0][0]))
        self.assertIn(SECRET.encode(), captured[0][1]['input'])


if __name__ == '__main__':
    unittest.main()
