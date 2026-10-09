"""Opt-in Bark operations alerts, independent of monitor/business delivery.

The HTTP API acknowledges service acceptance, never phone delivery or reading.
Contract: https://github.com/Finb/bark-server/blob/master/docs/API_V2.md
Failure semantics: https://github.com/Finb/bark-server/blob/master/route_push.go
"""
import datetime as dt
from dataclasses import dataclass, field
import http.client
import json
import os
from pathlib import Path
import re
import signal
import ssl
import subprocess
import sys
from urllib.parse import urlsplit

from reliability_common import absolute, clock, digest, json_bytes, open_dir, read_at, write_at

MAX_CONFIG = 4096
MAX_RESPONSE = 2048
MAX_RECEIPT = 4096
MAX_ATTEMPTS = 3
MAX_SENDS_PER_RUN = 3
EVENT_TTL_SECONDS = 900
TIMEOUT_SECONDS = 5
SOCKET_TIMEOUT_SECONDS = 2
FINAL_STATES = {'Accepted', 'Unknown', 'FailedFinal', 'Expired', 'Suppressed'}
REASONS = {'bark_service_accepted', 'bark_request_rejected', 'connect_failed_before_request',
           'tls_verification_failed_before_request', 'send_or_response_unknown',
           'response_oversized', 'response_unqualified', 'worker_deadline_or_result_unknown',
           'worker_spawn_failed_before_request', 'prior_inflight_result_unavailable',
           'event_expired', 'event_no_longer_current', 'outside_mobile_severity_policy',
           'incident_unknown_manual_review', 'retry_limit_reached'}
RULES = {'calendar', 'probe', 'process', 'data', 'account', 'capability', 'news', 'delivery'}
INCIDENT_REASONS = {'calendar_unavailable', 'probe_unavailable', 'probe_timeout',
    'probe_output_oversized', 'probe_exit_unavailable', 'probe_schema_unavailable',
    'probe_exit_status_mismatch', 'probe_spawn_unavailable', 'process_heartbeat_missing',
    'process_heartbeat_invalid', 'process_heartbeat_stale', 'process_heartbeat_process_mismatch',
    'process_heartbeat_monitor_not_running', 'process_lease_changed_during_observation',
    'process_lease_invalid', 'process_unavailable', 'quote_missing', 'data_observation_unavailable',
    'account_unavailable_or_frozen', 'missing_MoneyFlow', 'missing_Kline', 'missing_Tick',
    'missing_OrderBook', 'missing_News', 'news_recovery_unavailable', 'durable_manual_review'}


@dataclass(frozen=True)
class BarkConfig:
    endpoint: str = field(repr=False)
    device_key: str = field(repr=False)
    daily_self_check: bool = False
    allow_http_loopback: bool = False
    severities: tuple = ('P0', 'P1')


def parse_config(value):
    expected = {'schema_version', 'enabled', 'adapter', 'endpoint', 'device_key',
                'daily_self_check', 'allow_http_loopback', 'severities'}
    if not isinstance(value, dict) or set(value) - expected or value.get('schema_version') != 1:
        raise ValueError('mobile config schema unavailable')
    if type(value.get('enabled')) is not bool:
        raise ValueError('mobile enabled flag required')
    if not value['enabled']:
        return None
    if value.get('adapter') != 'bark':
        raise ValueError('unsupported mobile adapter')
    endpoint, key = value.get('endpoint'), value.get('device_key')
    if not isinstance(endpoint, str) or len(endpoint) > 512 or not isinstance(key, str) or not re.fullmatch(r'[A-Za-z0-9_-]{8,256}', key):
        raise ValueError('mobile endpoint or credential unavailable')
    parsed = urlsplit(endpoint)
    loopback = value.get('allow_http_loopback', False)
    daily = value.get('daily_self_check', False)
    severities = value.get('severities', ['P0', 'P1'])
    if type(loopback) is not bool or type(daily) is not bool:
        raise ValueError('invalid mobile options')
    if not isinstance(severities, list) or len(severities) != len(set(severities)) or not set(severities) <= {'P0', 'P1', 'P2'} or 'P0' not in severities:
        raise ValueError('mobile severity policy must include P0')
    if parsed.username is not None or parsed.password is not None or parsed.query or parsed.fragment or parsed.path != '/push' or not parsed.hostname:
        raise ValueError('only an explicit Bark /push endpoint is supported')
    if any(ord(ch) <= 32 or ord(ch) >= 127 for ch in endpoint):
        raise ValueError('invalid mobile endpoint')
    if parsed.scheme != 'https' and not (loopback and parsed.scheme == 'http' and parsed.hostname == '127.0.0.1'):
        raise ValueError('HTTPS required; HTTP only for explicit loopback fixture')
    if parsed.port is not None and not 0 < parsed.port <= 65535:
        raise ValueError('invalid mobile port')
    return BarkConfig(endpoint, key, daily, loopback, tuple(severities))


def load_config(path):
    if path is None:
        return None, 'unconfigured'
    try:
        path = absolute(path)
        fd = open_dir(path.parent, private=True)
        try:
            raw, _ = read_at(fd, path.name, MAX_CONFIG, private=True)
        finally:
            os.close(fd)
        config = parse_config(json.loads(raw))
        return config, 'enabled' if config else 'disabled'
    except FileNotFoundError:
        return None, 'unconfigured'
    except (OSError, ValueError, TypeError, UnicodeError):
        # Never serialize paths, endpoint, exceptions, response text, or secrets.
        return None, 'invalid'


def outcome(status, reason, http_status=None):
    value = {'status': status, 'reason': reason}
    if http_status is not None:
        value['http_status'] = http_status
    return value


def validate_outcome(value):
    if not isinstance(value, dict) or set(value) - {'status', 'reason', 'http_status'}:
        raise ValueError('transport outcome schema unavailable')
    if value.get('status') not in {'Accepted', 'FailedRetryable', 'FailedFinal', 'Unknown'} or value.get('reason') not in REASONS:
        raise ValueError('transport outcome unavailable')
    status = value.get('http_status')
    if status is not None and (type(status) is not int or not 100 <= status <= 599):
        raise ValueError('invalid HTTP outcome')
    allowed = {
        'Accepted': {'bark_service_accepted'},
        'FailedRetryable': {'connect_failed_before_request', 'worker_spawn_failed_before_request'},
        'FailedFinal': {'tls_verification_failed_before_request', 'bark_request_rejected'},
        'Unknown': {'send_or_response_unknown', 'response_oversized', 'response_unqualified',
                    'worker_deadline_or_result_unknown'},
    }
    if value['reason'] not in allowed[value['status']]:
        raise ValueError('transport proof/status mismatch')
    return value


def bark_request(config, message):
    """One direct request, no proxy, redirects, cookies, or implicit retries.

    Called in a bounded worker so DNS/connect/read can never stall the watchdog.
    Once connect succeeded, every transport failure is conservatively Unknown.
    """
    parsed = urlsplit(config.endpoint)
    connection = None
    try:
        if parsed.scheme == 'https':
            connection = http.client.HTTPSConnection(parsed.hostname, parsed.port,
                timeout=SOCKET_TIMEOUT_SECONDS, context=ssl.create_default_context())
        else:
            connection = http.client.HTTPConnection(parsed.hostname, parsed.port, timeout=SOCKET_TIMEOUT_SECONDS)
        try:
            connection.connect()
        except ssl.SSLCertVerificationError:
            return outcome('FailedFinal', 'tls_verification_failed_before_request')
        except (OSError, http.client.HTTPException):
            return outcome('FailedRetryable', 'connect_failed_before_request')
        raw = json_bytes(dict(message, device_key=config.device_key,
                              group='stock-analysis-watchdog', isArchive='1'))
        try:
            connection.request('POST', '/push', body=raw,
                               headers={'Content-Type': 'application/json; charset=utf-8'})
            response = connection.getresponse()
            body = response.read(MAX_RESPONSE + 1)
            if len(body) > MAX_RESPONSE:
                return outcome('Unknown', 'response_oversized', response.status)
            decoded = json.loads(body)
            code = decoded.get('code') if isinstance(decoded, dict) else None
            if type(code) is int and response.status == 200 and code == 200:
                return outcome('Accepted', 'bark_service_accepted', response.status)
            # Bark's single-device 400 occurs before APNs submission. Server
            # 500 may follow an ambiguous APNs send: never retry that response.
            if type(code) is int and response.status == 400 and code == 400:
                return outcome('FailedFinal', 'bark_request_rejected', response.status)
            return outcome('Unknown', 'response_unqualified', response.status)
        except (OSError, http.client.HTTPException, ValueError, UnicodeError):
            return outcome('Unknown', 'send_or_response_unknown')
    finally:
        if connection is not None:
            connection.close()


def send_bark(config, message):
    """Credentials live in private config/worker stdin; argv and receipts are clean."""
    raw = json_bytes({'config': {'schema_version': 1, 'enabled': True, 'adapter': 'bark',
        'endpoint': config.endpoint, 'device_key': config.device_key,
        'allow_http_loopback': config.allow_http_loopback}, 'message': message})
    try:
        result = subprocess.run([sys.executable, str(Path(__file__).absolute()), '--transport-worker'],
            input=raw, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            timeout=TIMEOUT_SECONDS, check=False)
    except OSError:
        return outcome('FailedRetryable', 'worker_spawn_failed_before_request')
    except subprocess.TimeoutExpired:
        return outcome('Unknown', 'worker_deadline_or_result_unknown')
    try:
        if result.returncode != 0 or len(result.stdout) > 512:
            raise ValueError('worker result unavailable')
        return validate_outcome(json.loads(result.stdout))
    except (ValueError, TypeError, UnicodeError):
        return outcome('Unknown', 'worker_deadline_or_result_unknown')


def validate_event(event):
    if not isinstance(event, dict) or event.get('schema_version') != 1 or event.get('delivery_scope') != 'local_persistence_only':
        raise ValueError('local event proof unavailable')
    if event.get('rule') not in RULES or event.get('reason') not in INCIDENT_REASONS or event.get('severity') not in {'P0', 'P1', 'P2'}:
        raise ValueError('local incident proof unavailable')
    if event.get('action') not in {'first', 'changed', 'reminder', 'recovery'} or not re.fullmatch(r'[0-9a-f]{64}', event.get('key', '')):
        raise ValueError('local event identity unavailable')
    if event.get('generation_sha256') is not None and not re.fullmatch(r'[0-9a-f]{64}', event['generation_sha256']):
        raise ValueError('local incident generation unavailable')
    clock(event['observed_at'])


def event_message(event, event_hash):
    # Only allowlisted incident fields reach the third party, never health raw
    # output, holdings, filesystem roots, endpoint, boot identity, or exceptions.
    return {'title': '交易监控 ' + event['severity'] + (' 恢复' if event['action'] == 'recovery' else ' 告警'),
            'body': event['rule'] + ': ' + event['reason'] + '\n时间: ' + event['observed_at'] + '\n事件: ' + event_hash[:16]}


def read_receipt(fd, event_hash):
    try:
        value = json.loads(read_at(fd, event_hash + '.json', MAX_RECEIPT, private=True)[0])
    except FileNotFoundError:
        return None
    if value.get('schema_version') != 1 or value.get('event_sha256') != event_hash or value.get('status') not in FINAL_STATES | {'InFlight', 'FailedRetryable'}:
        raise ValueError('mobile receipt unavailable')
    if type(value.get('attempts')) is not int or not 0 <= value['attempts'] <= MAX_ATTEMPTS:
        raise ValueError('mobile attempt proof unavailable')
    clock(value['observed_at'])
    if value.get('next_attempt_at') is not None:
        clock(value['next_attempt_at'])
    reasons = {
        'InFlight': {'attempt_started'},
        'Accepted': {'bark_service_accepted'},
        'FailedRetryable': {'connect_failed_before_request', 'worker_spawn_failed_before_request'},
        'FailedFinal': {'tls_verification_failed_before_request', 'bark_request_rejected', 'retry_limit_reached'},
        'Unknown': {'send_or_response_unknown', 'response_oversized', 'response_unqualified',
                    'worker_deadline_or_result_unknown', 'prior_inflight_result_unavailable'},
        'Expired': {'event_expired'},
        'Suppressed': {'event_no_longer_current', 'outside_mobile_severity_policy', 'incident_unknown_manual_review'},
    }
    if value.get('reason') not in reasons[value['status']]:
        raise ValueError('mobile receipt proof/status mismatch')
    if value['status'] in {'InFlight', 'Accepted', 'FailedRetryable', 'Unknown'} and value['attempts'] == 0:
        raise ValueError('mobile sent-state without attempt proof')
    if value['status'] == 'FailedRetryable' and value.get('next_attempt_at') is None:
        raise ValueError('mobile retry deadline unavailable')
    if value['status'] == 'Accepted' and value.get('http_status') != 200:
        raise ValueError('mobile acceptance lacks HTTP evidence')
    if value.get('reason') == 'bark_request_rejected' and value.get('http_status') != 400:
        raise ValueError('mobile rejection lacks HTTP evidence')
    return value


def unknown_subject(fd, subject_hash):
    """Keep Unknown from being bypassed by the next hourly reminder's new hash."""
    try:
        value = json.loads(read_at(fd, 'subject-' + subject_hash + '.json', MAX_RECEIPT, private=True)[0])
    except FileNotFoundError:
        return None
    if value.get('schema_version') != 1 or value.get('subject_sha256') != subject_hash or not re.fullmatch(r'[0-9a-f]{64}', value.get('event_sha256', '')):
        raise ValueError('mobile subject proof unavailable')
    receipt = read_receipt(fd, value['event_sha256'])
    if receipt is None or receipt['status'] in {'InFlight', 'Unknown'}:
        return value['event_sha256']
    return None


def dispatch_mobile(rootfd, output_root, state, now, config_path=None, sender=send_bark):
    """Caller holds the watchdog lease through local commit and this dispatch.

    Receipts are durable before/after each attempt. An interrupted InFlight is
    Unknown on the next run; neither configuration changes nor new runs resend it.
    """
    def binding(mobilefd=None):
        named = open_dir(output_root, private=True)
        try:
            if (os.fstat(named).st_dev, os.fstat(named).st_ino) != (os.fstat(rootfd).st_dev, os.fstat(rootfd).st_ino):
                raise ValueError('watchdog root changed')
        finally:
            os.close(named)
        if mobilefd is not None:
            named = open_dir(output_root / 'mobile-receipts', private=True)
            try:
                if (os.fstat(named).st_dev, os.fstat(named).st_ino) != (os.fstat(mobilefd).st_dev, os.fstat(mobilefd).st_ino):
                    raise ValueError('mobile receipt root changed')
            finally:
                os.close(named)

    binding()
    config, configuration = load_config(config_path)
    summary = {'schema_version': 1, 'observed_at': now.isoformat(), 'local_persistence': 'committed',
               'configuration_status': configuration, 'adapter': 'bark' if config else None,
               'attempted': 0, 'outcomes': {}, 'manual_review_required': False,
               'scope': 'service_acceptance_only_phone_delivery_unobserved'}
    if config is None:
        binding()
        write_at(rootfd, 'latest-mobile.json', json_bytes(summary), replace=True)
        return summary
    outbox = state.get('mobile_outbox', [])
    if not isinstance(outbox, list) or len(outbox) > 4096:
        raise ValueError('mobile committed outbox unavailable')
    eventfd = open_dir(output_root / 'events', create=True, private=True)
    mobilefd = open_dir(output_root / 'mobile-receipts', create=True, private=True)
    try:
        candidates = []
        for item in outbox:
            event_hash = item.get('event_sha256')
            if not isinstance(event_hash, str) or not re.fullmatch(r'[0-9a-f]{64}', event_hash):
                raise ValueError('invalid committed event reference')
            raw = read_at(eventfd, event_hash + '.json', MAX_RECEIPT, private=True)[0]
            if digest(raw) != event_hash:
                raise ValueError('committed event hash mismatch')
            event = json.loads(raw)
            validate_event(event)
            if item.get('observed_at') != event['observed_at']:
                raise ValueError('committed event time mismatch')
            active = state.get('active', {})
            if event['action'] == 'recovery':
                current = (state.get('mobile_resolved_generations', {}).get(event['key'], {}).get('generation_sha256')
                           == event.get('generation_sha256') and event['key'] not in active and not any(
                    entry['rule'] == event['rule'] and event['rule'] != 'capability' for entry in active.values()))
            else:
                entry = active.get(event['key'], {})
                current = (entry.get('severity') == event['severity']
                           and entry.get('generation_sha256') == event.get('generation_sha256'))
            subject = digest(json_bytes([event['key'], event.get('generation_sha256', event['observed_at']), event['severity'],
                                         'recovery' if event['action'] == 'recovery' else 'incident']))
            candidates.append((event['severity'], event_hash, event['observed_at'], event_message(event, event_hash),
                               current, event['severity'] in config.severities, subject))
        candidates.sort(key=lambda entry: (entry[0], entry[2], entry[1]))
        if config.daily_self_check:
            raw = read_at(rootfd, 'self-check-' + now.date().isoformat() + '.json', MAX_RECEIPT, private=True)[0]
            check = json.loads(raw)
            if check.get('schema_version') != 1 or check.get('scope') != 'local_write_read_only':
                raise ValueError('local self-check proof unavailable')
            clock(check['observed_at'])
            candidates.append(('P2', digest(raw), check['observed_at'], {'title': '交易监控 每日自检',
                'body': '独立看门狗本地落档与回读通过。\n时间: ' + check['observed_at']}, True, True, digest(raw)))
        # Current operational events precede a self-check. A fixed send budget
        # bounds one poll; unsent committed events remain eligible next poll.
        for severity, event_hash, observed_at, message, current, permitted, subject in candidates:
            binding(mobilefd)
            receipt = read_receipt(mobilefd, event_hash)
            if receipt is None:
                receipt = {'schema_version': 1, 'event_sha256': event_hash,
                           'attempts': 0, 'observed_at': now.isoformat()}
            status = receipt.get('status')
            if status == 'InFlight':
                receipt.update(status='Unknown', reason='prior_inflight_result_unavailable', observed_at=now.isoformat(),
                               manual_review_required=True)
                write_at(mobilefd, event_hash + '.json', json_bytes(receipt), replace=True)
                status = 'Unknown'
            if status not in FINAL_STATES:
                seconds = (now - clock(observed_at)).total_seconds()
                if not 0 <= seconds < EVENT_TTL_SECONDS:
                    receipt.update(status='Expired', reason='event_expired', observed_at=now.isoformat())
                elif not current:
                    receipt.update(status='Suppressed', reason='event_no_longer_current', observed_at=now.isoformat())
                elif not permitted:
                    receipt.update(status='Suppressed', reason='outside_mobile_severity_policy', observed_at=now.isoformat())
                elif unknown_subject(mobilefd, subject) is not None:
                    receipt.update(status='Suppressed', reason='incident_unknown_manual_review', observed_at=now.isoformat(),
                                   manual_review_required=True, blocked_by_event_sha256=unknown_subject(mobilefd, subject))
                elif receipt['attempts'] >= MAX_ATTEMPTS:
                    receipt.update(status='FailedFinal', reason='retry_limit_reached', observed_at=now.isoformat())
                elif summary['attempted'] >= MAX_SENDS_PER_RUN or (receipt.get('next_attempt_at') and now < clock(receipt['next_attempt_at'])):
                    continue
                else:
                    receipt.update(status='InFlight', attempts=receipt['attempts'] + 1,
                                   observed_at=now.isoformat(), reason='attempt_started')
                    receipt.pop('next_attempt_at', None)
                    write_at(mobilefd, event_hash + '.json', json_bytes(receipt), replace=True)
                    write_at(mobilefd, 'subject-' + subject + '.json', json_bytes({
                        'schema_version': 1, 'subject_sha256': subject, 'event_sha256': event_hash}), replace=True)
                    binding(mobilefd)
                    # If send/outcome/publication raises, the durable InFlight
                    # remains: the next owner records Unknown, never a retry.
                    result = validate_outcome(sender(config, message))
                    summary['attempted'] += 1
                    receipt.update(result)
                    receipt['manual_review_required'] = result['status'] == 'Unknown'
                    if result['status'] == 'FailedRetryable':
                        if receipt['attempts'] == MAX_ATTEMPTS:
                            receipt.update(status='FailedFinal', reason='retry_limit_reached')
                        else:
                            receipt['next_attempt_at'] = (now + dt.timedelta(seconds=60 * 4 ** (receipt['attempts'] - 1))).isoformat()
                binding(mobilefd)
                write_at(mobilefd, event_hash + '.json', json_bytes(receipt), replace=True)
            summary['outcomes'][receipt['status']] = summary['outcomes'].get(receipt['status'], 0) + 1
            summary['manual_review_required'] |= receipt['status'] == 'Unknown' or receipt.get('manual_review_required', False)
    finally:
        os.close(eventfd)
        os.close(mobilefd)
    binding()
    write_at(rootfd, 'latest-mobile.json', json_bytes(summary), replace=True)
    return summary


def transport_worker():
    # Fixed, sanitized output keeps the parent's capture bounded in practice;
    # remote response data is read only to MAX_RESPONSE + 1 and never printed.
    def deadline(_number, _frame):
        raise TimeoutError('worker deadline')
    signal.signal(signal.SIGALRM, deadline)
    # Also bound the child itself if its parent is killed mid-attempt.
    signal.setitimer(signal.ITIMER_REAL, max(.1, TIMEOUT_SECONDS - .5))
    try:
        value = json.loads(sys.stdin.buffer.read(MAX_CONFIG + 1))
        config = parse_config(value['config'])
        message = value['message']
        if config is None or set(message) != {'title', 'body'} or any(not isinstance(v, str) or len(v) > 1024 for v in message.values()):
            raise ValueError('worker input unavailable')
        result = bark_request(config, message)
    except Exception:
        result = outcome('Unknown', 'worker_deadline_or_result_unknown')
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    if sys.argv[1:] == ['--transport-worker']:
        transport_worker()
    else:
        raise SystemExit('watchdog_mobile is used by monitor_watchdog; no standalone send command')
