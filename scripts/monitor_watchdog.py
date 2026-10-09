#!/usr/bin/env python3
"""Independent health probe, durable local alerts, and opt-in mobile operations alerts."""
import argparse
import copy
import fcntl
import json
import os
from pathlib import Path
import re
import selectors
import subprocess
import time

from reliability_common import (Calendar, absolute, clock, digest, json_bytes,
                                open_dir, read_at, regular, write_at)
from watchdog_mobile import dispatch_mobile

PROBE_LIMIT = 64 * 1024
RUNTIME = '/Users/zhangzhen/.local/share/stock-analysis-runtime'


def probe(runtime, timeout=5):
    command = [str(absolute(runtime) / 'target/release/monitor'), '--health', '--json']
    child = None
    try:
        child = subprocess.Popen(command, cwd=str(runtime), stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, shell=False)
        selector = selectors.DefaultSelector()
        for stream in (child.stdout, child.stderr):
            os.set_blocking(stream.fileno(), False)
            selector.register(stream, selectors.EVENT_READ)
        deadline = time.monotonic() + timeout
        output, total = bytearray(), 0
        try:
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return None, 'probe_timeout'
                for key, _ in selector.select(min(remaining, 0.1)):
                    raw = os.read(key.fileobj.fileno(), 8192)
                    if not raw:
                        selector.unregister(key.fileobj)
                        continue
                    total += len(raw)
                    if total > PROBE_LIMIT:
                        return None, 'probe_output_oversized'
                    if key.fileobj is child.stdout:
                        output.extend(raw)
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return None, 'probe_timeout'
            code = child.wait(timeout=remaining)
            if code not in (0, 1):
                return None, 'probe_exit_unavailable'
            try:
                report = json.loads(output)
                validate_report(report)
            except (ValueError, KeyError, TypeError):
                return None, 'probe_schema_unavailable'
            if (code == 0) != (report['status'] == 'ok'):
                return None, 'probe_exit_status_mismatch'
            return report, None
        finally:
            selector.close()
    except subprocess.TimeoutExpired:
        return None, 'probe_timeout'
    except OSError:
        return None, 'probe_spawn_unavailable'
    finally:
        if child is not None:
            if child.poll() is None:
                child.kill()
            child.wait()
            child.stdout.close()
            child.stderr.close()


def validate_report(report):
    if report['status'] not in ('ok', 'unhealthy'):
        raise ValueError('unknown status')
    snapshot = report['runtime_snapshot']
    if snapshot['version'] != 2:
        raise ValueError('unknown version')
    clock(snapshot['checked_at'])
    for name in ('process', 'account', 'data'):
        item = snapshot[name]
        if item['status'] not in ('ok', 'unhealthy', 'unavailable'):
            raise ValueError('unknown component status')
        if item.get('reason_code') is not None and not isinstance(item['reason_code'], str):
            raise ValueError('invalid reason')
    p = snapshot['process']
    if type(p['monitor_running']) is not bool or type(p['heartbeat_fresh']) is not bool:
        raise ValueError('invalid process flags')
    boot = p['boot_identity_sha256']
    if boot is not None and not re.fullmatch('[0-9a-f]{64}', boot):
        raise ValueError('invalid boot hash')
    if p['heartbeat_observed_at'] is not None:
        clock(p['heartbeat_observed_at'])
    if p['status'] == 'ok' and (not p['monitor_running'] or not p['heartbeat_fresh'] or boot is None or p['heartbeat_observed_at'] is None):
        raise ValueError('process ok lacks required evidence')
    a, d = snapshot['account'], snapshot['data']
    if a['metrics_complete'] is not None and type(a['metrics_complete']) is not bool:
        raise ValueError('invalid account flag')
    for item in (a, d):
        if item['mode'] is not None and not isinstance(item['mode'], str):
            raise ValueError('invalid mode')
        if item['evaluated_at'] is not None:
            clock(item['evaluated_at'])
    if not isinstance(d['missing_capabilities'], list) or any(not isinstance(x, str) for x in d['missing_capabilities']):
        raise ValueError('invalid capabilities')
    if snapshot['raw_global_news']['recovery']['status'] not in ('ok', 'degraded', 'warming', 'idle', 'unavailable'):
        raise ValueError('invalid news status')
    if snapshot['durable_delivery']['status'] not in ('observed', 'not_initialized', 'unavailable', 'not_observed'):
        raise ValueError('invalid delivery status')


def age(at, now):
    return (now - clock(at)).total_seconds() if at else None


def fresh(at, now):
    seconds = age(at, now)
    return seconds is not None and -5 <= seconds <= 600


def classify(report, now, calendar, prior=None, probe_error=None):
    """Allowlisted observations only. Quote streak is persisted independently of events."""
    prior = prior or {}
    incidents = []
    observation = {'health_status': 'unavailable', 'session': 'Unavailable',
                   'business_date': now.date().isoformat(), 'boot_hash': 'unknown', 'resolved_rules': []}
    try:
        observation['session'] = calendar.session(now)
        observation['calendar_sha256'] = calendar.sha256
        observation['calendar_extra_sha256'] = calendar.extra_sha256
        observation['resolved_rules'].append('calendar')
    except (ValueError, AttributeError):
        incidents.append({'rule': 'calendar', 'reason': 'calendar_unavailable', 'severity': 'P1'})
    streak = 0
    if report is None:
        incidents.append({'rule': 'probe', 'reason': probe_error or 'probe_unavailable', 'severity': 'P0'})
    else:
        validate_report(report)
        snapshot = report['runtime_snapshot']
        p, a, d = (snapshot[k] for k in ('process', 'account', 'data'))
        observation['health_status'] = report['status']
        observation['boot_hash'] = p['boot_identity_sha256'] or 'unknown'
        snapshot_fresh = fresh(snapshot['checked_at'], now)
        observation['resolved_rules'].append('probe')
        if snapshot_fresh and p['monitor_running'] and p['status'] == 'ok' and p['heartbeat_fresh'] and fresh(p['heartbeat_observed_at'], now):
            observation['resolved_rules'].append('process')
        if not snapshot_fresh or not p['monitor_running'] or p['status'] != 'ok' or not p['heartbeat_fresh'] or not fresh(p['heartbeat_observed_at'], now):
            known = {'process_heartbeat_missing', 'process_heartbeat_invalid', 'process_heartbeat_stale',
                     'process_heartbeat_process_mismatch', 'process_heartbeat_monitor_not_running',
                     'process_lease_changed_during_observation', 'process_lease_invalid'}
            reason = p.get('reason_code')
            incidents.append({'rule': 'process', 'reason': reason if reason in known else 'process_unavailable', 'severity': 'P0'})
        data_available = snapshot_fresh and d['status'] != 'unavailable' and fresh(d['evaluated_at'], now)
        missing_quote = data_available and 'Quote' in d['missing_capabilities']
        data_reason = 'quote_missing' if missing_quote else 'data_observation_unavailable' if not data_available else None
        active = observation['session'] in ('Morning', 'Afternoon')
        token = [observation['business_date'], observation['session'], observation['boot_hash'], data_reason]
        if data_reason and active:
            previous_at = prior.get('poll_at')
            consecutive = previous_at and 0 < age(previous_at, now) <= 120 and prior.get('quote_token') == token
            streak = min(2, prior.get('quote_streak', 0) + 1) if consecutive else 1
        if (data_available and not missing_quote) or (missing_quote and observation['session'] in ('Closed', 'LunchBreak', 'AfterHours')):
            observation['resolved_rules'].append('data')
        if data_reason:
            if not data_available or active or observation['session'] == 'Auction':
                incidents.append({'rule': 'data', 'reason': data_reason, 'severity': 'P0' if active and streak >= 2 else 'P1'})
        if observation['session'] == 'Auction' and missing_quote:
            observation['auction_freshness_contract'] = 'unavailable'
        if not snapshot_fresh or a['status'] != 'ok' or a['mode'] != 'Normal' or not a['metrics_complete'] or not fresh(a['evaluated_at'], now):
            incidents.append({'rule': 'account', 'reason': 'account_unavailable_or_frozen', 'severity': 'P1'})
        else:
            observation['resolved_rules'].append('account')
        if data_available:
            observation['resolved_rules'].append('capability')
            # Never persist arbitrary capability strings supplied by a failed probe.
            for capability in ('MoneyFlow', 'Kline', 'Tick', 'OrderBook', 'News'):
                if capability in d['missing_capabilities']:
                    incidents.append({'rule': 'capability', 'reason': 'missing_' + capability, 'severity': 'P2'})
        if snapshot['raw_global_news']['recovery']['status'] != 'ok':
            incidents.append({'rule': 'news', 'reason': 'news_recovery_unavailable', 'severity': 'P2'})
        else:
            observation['resolved_rules'].append('news')
        delivery = snapshot['durable_delivery']
        counts = delivery.get('counts')
        if isinstance(counts, dict) and isinstance(counts.get('non_progressable_manual_reviews'), int) and counts['non_progressable_manual_reviews'] > 0:
            incidents.append({'rule': 'delivery', 'reason': 'durable_manual_review', 'severity': 'P2'})
        elif delivery['status'] == 'observed' and isinstance(counts, dict) and counts.get('non_progressable_manual_reviews') == 0:
            observation['resolved_rules'].append('delivery')
    for incident in incidents:
        incident['key'] = digest(json_bytes([incident['rule'], incident['reason'], observation['business_date'], observation['boot_hash']]))
    return incidents, observation, {'quote_streak': streak, 'quote_token': token if report is not None else None, 'poll_at': now.isoformat()}


def persist_incidents(fd, incidents, observation, poll, prior, now):
    def binding():
        named = open_dir(Path(observation['output_root']), private=True)
        try:
            if (os.fstat(named).st_dev, os.fstat(named).st_ino) != (os.fstat(fd).st_dev, os.fstat(fd).st_ino):
                raise ValueError('watchdog root changed')
        finally:
            os.close(named)
    binding()
    state = copy.deepcopy(prior)
    old = state.get('active', {})
    current = {}
    events = []
    # Only events referenced by a successfully committed local state are eligible
    # for mobile delivery. A crash after event write but before state publication
    # leaves an unsent audit artifact, rather than a second sendable first event.
    outbox = [entry for entry in state.get('mobile_outbox', [])
              if 0 <= age(entry['observed_at'], now) <= 86400]
    resolved_generations = {key: entry for key, entry in state.get('mobile_resolved_generations', {}).items()
                            if 0 <= age(entry['observed_at'], now) <= 86400}
    for incident in incidents:
        previous = old.get(incident['key'])
        same_rule = [v for v in old.values() if v['rule'] == incident['rule'] and incident['rule'] != 'capability']
        action = ('changed' if same_rule else 'first') if previous is None else 'changed' if previous['severity'] != incident['severity'] else None
        if action is None and incident['severity'] == 'P0' and age(previous['last_event_at'], now) >= 3600:
            action = 'reminder'
        entry = dict(incident)
        entry['generation_sha256'] = (previous.get('generation_sha256') if previous else None) or digest(
            json_bytes([incident['key'], previous['last_event_at'] if previous else now.isoformat()]))
        entry['last_event_at'] = now.isoformat() if action else previous['last_event_at']
        current[incident['key']] = entry
        if action:
            events.append(dict(incident, action=action, generation_sha256=entry['generation_sha256']))
    for key, previous in old.items():
        if key not in current:
            if any(i['rule'] == previous['rule'] for i in incidents) and previous['rule'] != 'capability':
                continue  # Changed reason/severity is represented by the new event.
            if previous['rule'] in observation['resolved_rules']:
                generation = previous.get('generation_sha256') or digest(
                    json_bytes([key, previous['last_event_at']]))
                events.append(dict(previous, action='recovery', generation_sha256=generation,
                                   recovery_basis='verified_scope_or_session_resolution'))
                resolved_generations[key] = {'generation_sha256': generation, 'observed_at': now.isoformat()}
            else:
                current[key] = previous  # Unknown probe/data does not establish recovery.
    eventfd = open_dir(Path(observation['output_root']) / 'events', create=True, private=True)
    try:
        for event in events:
            binding()
            event.update(schema_version=1, observed_at=now.isoformat(), delivery_scope='local_persistence_only')
            event_hash = digest(json_bytes(event))
            raw = json_bytes(event)
            try:
                write_at(eventfd, event_hash + '.json', raw)
            except FileExistsError:
                if read_at(eventfd, event_hash + '.json', private=True)[0] != raw:
                    raise ValueError('event retry content mismatch')
            outbox.append({'event_sha256': event_hash, 'observed_at': event['observed_at']})
    finally:
        os.close(eventfd)
    binding()
    state.update(schema_version=1, active=current, mobile_outbox=outbox,
                 mobile_resolved_generations=resolved_generations, **poll)
    # Event failures above never advance this durable dedup state.
    write_at(fd, 'state.json', json_bytes(state), replace=True)
    write_at(fd, 'latest.json', json_bytes(dict(schema_version=1, observed_at=now.isoformat(),
             observation=observation, incidents=incidents, delivery_scope='local_persistence_only')), replace=True)
    day = 'self-check-' + now.date().isoformat() + '.json'
    try:
        read_at(fd, day, private=True)
    except FileNotFoundError:
        raw = json_bytes({'schema_version': 1, 'observed_at': now.isoformat(),
                          'scope': 'local_write_read_only', 'mobile_delivery': 'not_attempted'})
        write_at(fd, day, raw)
        if read_at(fd, day, private=True)[0] != raw:
            raise ValueError('self-check reread failed')
    return state


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime-root', default=RUNTIME, type=absolute)
    parser.add_argument('--calendar', required=True, type=absolute)
    parser.add_argument('--extra-holiday', action='append', default=[], help='explicit YYYY-MM-DD closure; no .env reading')
    parser.add_argument('--output-root', required=True, type=absolute)
    parser.add_argument('--observed-at')
    parser.add_argument('--probe-timeout-seconds', type=float, default=5)
    parser.add_argument('--mobile-config', type=absolute,
                        help='explicit opt-in private JSON; absent means no mobile network calls')
    args = parser.parse_args()
    if not 0 < args.probe_timeout_seconds <= 30:
        parser.error('probe timeout must be >0 and <=30 seconds')
    os.umask(0o077)
    fd = lockfd = None
    try:
        now = clock(args.observed_at)
        fd = open_dir(args.output_root, create=True, private=True)
        lockfd = os.open('.watchdog.lock', os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600, dir_fd=fd)
        regular(os.fstat(lockfd), private=True)
        try:
            fcntl.flock(lockfd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print('watchdog_overlap_skipped')
            return 0
        try:
            prior = json.loads(read_at(fd, 'state.json', private=True)[0])
            if prior.get('schema_version') != 1:
                raise ValueError('state schema unavailable')
        except FileNotFoundError:
            prior = {}
        try:
            calendar = Calendar(args.calendar, args.extra_holiday)
        except (OSError, ValueError, UnicodeError):
            calendar = None
        report, error = probe(args.runtime_root, args.probe_timeout_seconds)
        incidents, observation, poll = classify(report, now, calendar, prior, error)
        observation['output_root'] = str(args.output_root)
        state = persist_incidents(fd, incidents, observation, poll, prior, now)
        print('local_observation_persisted')
        # This independent operations alert owner never enters business delivery
        # or modifies its database. Local persistence always precedes networking.
        try:
            mobile = dispatch_mobile(fd, args.output_root, state, now, args.mobile_config)
            print('mobile_configuration=' + mobile['configuration_status'])
        except (OSError, ValueError, KeyError, TypeError):
            print('watchdog_mobile_receipt_unavailable_local_observation_retained')
            return 2
        return 1 if any(i['severity'] == 'P0' for i in incidents) else 0
    except (OSError, ValueError, KeyError, TypeError):
        print('watchdog_local_persistence_or_input_unavailable')
        return 2
    finally:
        if lockfd is not None:
            os.close(lockfd)
        if fd is not None:
            os.close(fd)


if __name__ == '__main__':
    raise SystemExit(main())
