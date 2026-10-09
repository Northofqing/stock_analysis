#!/usr/bin/env python3
"""Read-only bounded reliability candidates. Never adjudicates, repairs or sends."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import tarfile

from reliability_common import (MAX_FILE, Calendar, absolute, clock, date, digest, exclusive_json,
                                json_bytes, open_dir, read_at, read_path)
from rotate_push_log import ARCHIVE_LIMIT, ArchiveMemberBudgetExceeded, validate_archive


def canonical(raw, sha, sorted_keys=False):
    if not isinstance(raw, bytes) or digest(raw) != sha:
        raise ValueError('canonical hash invalid')
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError('duplicate JSON key')
            result[key] = value
        return result
    value = json.loads(raw, object_pairs_hook=pairs)
    encoded = json.dumps(value, ensure_ascii=False, sort_keys=sorted_keys,
                         separators=(',', ':'), allow_nan=False).encode()
    if encoded != raw:
        raise ValueError('canonical bytes invalid')
    return value


def domain_hash(domain, raw):
    return digest(domain.encode() + b'\0' + raw)


def counted_artifact(value):
    pending_fields = {'schema', 'state', 'durable_push_kind', 'stable_template_id', 'decision_identity',
        'attempt_identity', 'decision_identity_hash', 'attempt_identity_hash', 'fence_token',
        'rendered_content_sha256', 'rendered_content', 'sink_result', 'sink_result_sha256',
        'receipt_sha256', 'observed_at'}
    committed_fields = {'schema', 'state', 'durable_push_kind', 'stable_template_id', 'decision_identity_hash',
        'attempt_identity_hash', 'pending_artifact_sha256', 'delivery_audit_event_id', 'counted_join_hash', 'committed_at'}
    required = pending_fields if value.get('state') == 'AuditPending' else committed_fields if value.get('state') == 'Committed' else None
    if required is None or set(value) != required:
        raise ValueError('unknown counted field set')
    if value['state'] == 'AuditPending':
        result = value['sink_result']
        raw = json.dumps(result, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
        if domain_hash('stock_analysis.counted_sink_result.v1', raw) != value['sink_result_sha256']:
            raise ValueError('counted sink hash invalid')
        if result['kind'] == 'Accepted':
            receipt = result['receipt']
            fields = ('channel', 'provider', 'message_id', 'platform_message_id', 'accepted_at', 'latency_ms')
            if set(result) != {'kind', 'receipt'} or set(receipt) != set(fields):
                raise ValueError('counted receipt field set invalid')
            # TypedReceipt serde field order, distinct from sorted JSON Value sink-result bytes.
            typed = {field: receipt[field] for field in fields}
            encoded = json.dumps(typed, ensure_ascii=False, separators=(',', ':')).encode()
            receipt_hash = domain_hash('stock_analysis.counted_receipt.v1', encoded)
        elif result['kind'] in ('Rejected', 'Uncertain'):
            field = 'rejection' if result['kind'] == 'Rejected' else 'uncertainty'
            if set(result) != {'kind', field}:
                raise ValueError('unknown counted result schema')
            receipt_hash = domain_hash('stock_analysis.counted_receipt.none.v1', b'NO_VALIDATED_RECEIPT')
        else:
            raise ValueError('unknown counted result kind')
        if receipt_hash != value['receipt_sha256']:
            raise ValueError('counted receipt hash invalid')
    return value


def bounded(connection, sql, limit, parameters=()):
    rows = connection.execute(sql + ' LIMIT ?', tuple(parameters) + (limit + 1,)).fetchall()
    return rows[:limit], len(rows) > limit


def tables(connection):
    return {row[0] for row in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")}


def snapshot(source, destination):
    # Reuse the already-reviewed mode=ro/query_only/BEGIN SQLite backup seam.
    spec = importlib.util.spec_from_file_location('weekly_backup', Path(__file__).with_name('weekly-outcome-review.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    parentfd = open_dir(source.parent)
    try:
        # Refuse symlinks/hardlinks/nonregular or unsafe source files before SQLite opens.
        from reliability_common import regular
        regular(os.stat(source.name, dir_fd=parentfd, follow_symlinks=False))
        module.read_only_backup(source, destination)
    finally:
        os.close(parentfd)
    os.chmod(destination, 0o600)
    connection = sqlite3.connect(destination.as_uri() + '?mode=ro', uri=True)
    connection.row_factory = sqlite3.Row
    connection.execute('PRAGMA query_only=ON')
    connection.execute('BEGIN')
    schema = [list(row) for row in connection.execute('SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name')]
    meta = {'path': str(source), 'snapshot_sha256': digest(destination.read_bytes()),
            'schema_sha256': digest(json_bytes(schema)), 'schema_version': connection.execute('PRAGMA user_version').fetchone()[0],
            'access': 'mode=ro/query_only/BEGIN/logical_backup', 'live_wal_included': True}
    return connection, meta


def unavailable(reason, **extra):
    return dict(status='unavailable', reason=reason, candidates=[], **extra)


def screen_r1(connection, limit, evidence=None):
    result = unavailable('unavailable_independent_same_day_price_authority',
                         authority_contract='QualifiedTradingFactsGateway::ContractNotDelivered',
                         supplied_json_admitted=False, raw_diagnostics=[], truncated=False)
    if evidence:
        result['supplied_evidence'] = {'path': str(evidence), 'sha256': digest(read_path(evidence)),
                                       'admission': 'unavailable_existing_qualified_reader'}
    if 'order_audit' not in tables(connection):
        result['raw_source_status'] = 'unavailable_order_audit'
        return result
    rows, truncated = bounded(connection, "SELECT id,business_order_id,code,execution_price,quote_observed_at,created_at FROM order_audit WHERE outcome='Filled' ORDER BY id", limit)
    result['raw_diagnostics'] = [dict(row) for row in rows]
    result['raw_source_status'], result['truncated'] = 'observed_unqualified_filled_audits', truncated
    return result


def screen_r3(connection, calendar, limit):
    if 'prediction_tracker' not in tables(connection):
        return unavailable('prediction_tracker_missing')
    rows, truncated = bounded(connection, 'SELECT id,pred_date,target_date FROM prediction_tracker ORDER BY id', limit)
    result = {'status': 'candidates_only', 'candidates': [], 'unavailable_rows': [],
              'rows_read': len(rows), 'truncated': truncated, 'calendar_sha256': calendar.sha256 if calendar else None}
    names = tables(connection)
    freeze_available = {'candidate_board_prediction_freeze_v2', 'candidate_board_prediction_member_v2'} <= names
    for row in rows:
        evidence = dict(row)
        evidence['calendar_sha256'] = result['calendar_sha256']
        try:
            date(row['pred_date'])
            target = date(row['target_date'])
        except (ValueError, TypeError):
            result['unavailable_rows'].append(dict(evidence, reason='invalid_original_date'))
            continue
        try:
            trading = calendar.trading(target) if calendar else None
        except ValueError:
            trading = None
        if trading is None:
            result['unavailable_rows'].append(dict(evidence, reason='calendar_year_or_source_unavailable'))
            continue
        if not trading:
            evidence['kind'] = 'nontrading_original_target_date'
            evidence['freeze_provenance'] = 'unavailable'
            if freeze_available:
                frozen = connection.execute('SELECT f.occurrence_identity,f.business_date,f.target_date,f.calendar_authority_hash,f.source_canonical,f.source_sha256,f.rendered_bytes,f.rendered_sha256 FROM candidate_board_prediction_member_v2 m JOIN candidate_board_prediction_freeze_v2 f USING(occurrence_identity) WHERE m.prediction_row_id=?', (row['id'],)).fetchone()
                if frozen:
                    valid = (digest(frozen['source_canonical']) == frozen['source_sha256'] and
                             digest(frozen['rendered_bytes']) == frozen['rendered_sha256'] and frozen['target_date'] == row['target_date'])
                    evidence['freeze_provenance'] = 'hash_verified' if valid else 'unavailable_hash_or_target_mismatch'
                    evidence['freeze'] = {k: frozen[k] for k in ('occurrence_identity', 'business_date', 'target_date', 'calendar_authority_hash', 'source_sha256', 'rendered_sha256')}
            result['candidates'].append(evidence)
    if result['unavailable_rows']:
        result['status'] = 'partial_unavailable'
    return result


def screen_r5(connection, evidence=None, limit=10000):
    names = tables(connection)
    required = {'paper_ledger_account', 'paper_ledger_event', 'paper_ledger_head'}
    result = unavailable('unavailable_seeded_account_lot_evidence', tables_present=sorted(required & names),
                         qualified_reader='unavailable_existing_seeded_account_lot_reader', supplied_json_admitted=False,
                         raw_diagnostics=[], truncated=False)
    if 'paper_trades' in names:
        rows, truncated = bounded(connection, 'SELECT id,plan_id,code,direction,quantity,status,fill_price,ts FROM paper_trades ORDER BY id', limit)
        result.update(raw_diagnostics=[dict(row) for row in rows], truncated=truncated,
                      raw_scope='legacy_paper_rows_no_account_lot_or_oversell_verdict')
    if evidence:
        result['supplied_evidence'] = {'path': str(evidence), 'sha256': digest(read_path(evidence)),
                                       'admission': 'unavailable_existing_qualified_reader'}
    # No pooled inventory arithmetic or Python ledger hash/replay implementation.
    return result


def screen_physical(connection, limit):
    version = connection.execute('PRAGMA user_version').fetchone()[0]
    if version != 9:
        return unavailable('delivery_schema_9_required', observed_schema=version)
    if not {'delivery_decisions', 'delivery_attempts', 'sink_results'} <= tables(connection):
        return unavailable('delivery_join_tables_missing')
    rows, truncated = bounded(connection, '''SELECT s.*,a.decision_identity AS attempt_decision,a.fence_token AS attempt_fence,
        d.envelope_canonical,d.envelope_sha256,d.envelope_version FROM sink_results s
        LEFT JOIN delivery_attempts a ON a.attempt_identity=s.attempt_identity
        LEFT JOIN delivery_decisions d ON d.decision_identity=s.decision_identity ORDER BY s.result_event_identity''', limit)
    grouped, invalid, unknown, late = {}, [], [], []
    invalid_decisions = set()
    for row in rows:
        evidence = {k: row[k] for k in ('result_event_identity', 'attempt_identity', 'decision_identity', 'result_kind', 'authoritative_for_state', 'late_after_fence', 'result_sha256', 'envelope_sha256')}
        if row['result_kind'] == 'Uncertain':
            unknown.append(evidence)
            continue
        if row['result_kind'] != 'Accepted':
            continue
        try:
            if row['attempt_decision'] != row['decision_identity'] or row['attempt_fence'] != row['fence_token'] or row['envelope_version'] != 1:
                raise ValueError('missing or mismatched join')
            envelope = canonical(row['envelope_canonical'], row['envelope_sha256'])
            if envelope['decision_identity'] != row['decision_identity'] or envelope['envelope_version'] != 1:
                raise ValueError('envelope binding mismatch')
            required_fields = {'envelope_version', 'decision_identity', 'business_date', 'push_kind', 'sub_kind', 'cooldown_scope', 'scope_key', 'schedule_occurrence_identity', 'source_evidence_fingerprint', 'source_binding_canonical', 'source_binding_sha256', 'delivery_subject_hash', 'rendered_content', 'rendered_content_sha256', 'policy_version', 'retry_authorized', 'provider_observed_at', 'provider_as_of', 'original_batch_ids', 'task_binding'}
            if set(envelope) not in (required_fields, required_fields | {'foundation_binding'}):
                raise ValueError('unknown envelope field set')
            for field in ('rendered_content', 'source_binding_canonical'):
                expected = 'rendered_content_sha256' if field == 'rendered_content' else 'source_binding_sha256'
                raw = bytes(envelope[field])
                if not raw or digest(raw) != envelope[expected]:
                    raise ValueError('nested envelope hash invalid')
            value = canonical(row['result_canonical'], row['result_sha256'], sorted_keys=True)
            receipt = value['receipt']
            if set(value) != {'kind', 'receipt'} or value['kind'] != 'Accepted' or set(receipt) != {'provider', 'channel', 'message_id', 'platform_message_id', 'accepted_at', 'latency_ms'}:
                raise ValueError('unknown accepted receipt schema')
            for field in receipt:
                if receipt[field] != row[field]:
                    raise ValueError('receipt column mismatch')
            for field in ('provider', 'channel', 'platform_message_id', 'message_id'):
                if not isinstance(receipt[field], str) or not receipt[field].strip():
                    raise ValueError('strong physical receipt unavailable')
            clock(receipt['accepted_at'])
            if not receipt['accepted_at'].endswith('Z') or (receipt['latency_ms'] is not None and (type(receipt['latency_ms']) is not int or receipt['latency_ms'] < 0)):
                raise ValueError('typed UTC receipt or latency invalid')
            physical_key = tuple(receipt[k] for k in ('provider', 'channel', 'platform_message_id'))
            evidence.update(physical_receipt=list(physical_key), rendered_content_sha256=envelope['rendered_content_sha256'])
            grouped.setdefault(row['decision_identity'], {}).setdefault(physical_key, []).append(evidence)
            if row['late_after_fence']:
                late.append(evidence)
        except (ValueError, KeyError, TypeError, OverflowError):
            invalid.append(dict(evidence, reason='unavailable_schema_hash_or_receipt_join'))
            invalid_decisions.add(row['decision_identity'])
    candidates = []
    for decision, physical in sorted(grouped.items()):
        if len(physical) > 1 and decision not in invalid_decisions:
            candidates.append({'kind': 'distinct_accepted_physical_ids_same_decision', 'decision_identity': decision,
                               'physical_observations': [observations for _, observations in sorted(physical.items())]})
    return {'status': 'partial_unavailable' if invalid or truncated else 'candidates_only', 'rows_read': len(rows),
            'truncated': truncated, 'candidates': candidates, 'unavailable_rows': invalid,
            'unknown_uncertain': unknown, 'late_after_fence_evidence': late,
            'distinct_verified_physical_observations': sum(len(v) for v in grouped.values()),
            'cross_decision_content_status': 'unavailable_exact_immutable_content_binding_join',
            'scope': 'transport_receipts_only_user_reading_unknown',
            'verification_scope': 'schema9_hashes_receipt_join_no_business_state_adjudication'}


def screen_archives(push_log, archive_root, limit):
    hashes, attempts, commits, unavailable_files = {}, {}, [], []
    read_count, source_successes, source_bytes, processed, truncated = 0, 0, 0, 0, False
    archive_dates_attempted = 0
    archive_io = {'file_read_attempts': 0, 'file_read_successes': 0, 'file_bytes_returned': 0,
                  'member_read_attempts': 0, 'member_read_successes': 0, 'member_bytes_returned': 0,
                  'declared_members': 0}
    revisions = {}
    def record_body(path, body, source, location):
        sha = digest(body)
        revisions.setdefault(path, []).append({'sha256': sha, 'bytes': len(body), 'source': source, 'location': location})
        group = hashes.setdefault(sha, {})
        group[path] = 'live_and_verified_rotated_markdown' if path in group else source
    rootfd = open_dir(push_log)
    try:
        for day in sorted(os.listdir(rootfd)):
            try:
                date(day)
            except ValueError:
                continue
            dayfd = os.open(day, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=rootfd)
            try:
                for name in sorted(os.listdir(dayfd)):
                    if not name.endswith(('.md', '.json')):
                        continue
                    if read_count >= limit:
                        truncated = True
                        break
                    read_count += 1
                    label = day + '/' + name
                    try:
                        raw, _ = read_at(dayfd, name)
                        source_successes += 1
                        source_bytes += len(raw)
                        processed += 1
                        if name.endswith('.md'):
                            record_body(label, raw, 'live_markdown', str(push_log / label))
                        else:
                            value = json.loads(raw)
                            if value.get('schema') != 'stock_analysis.counted_push_log.v1':
                                unavailable_files.append({'path': label, 'reason': 'unrecognized_json_schema'})
                                continue
                            counted_artifact(value)
                            key = (value['decision_identity_hash'], value['attempt_identity_hash'])
                            if value['state'] == 'AuditPending':
                                if (domain_hash('stock_analysis.counted_decision_identity.v1', value['decision_identity'].encode()) != key[0]
                                        or domain_hash('stock_analysis.counted_attempt_identity.v1', value['attempt_identity'].encode()) != key[1]
                                        or digest(value['rendered_content'].encode()) != value['rendered_content_sha256']):
                                    raise ValueError('pending binding hash invalid')
                                attempt = attempts.setdefault(key, {'pending_paths': [], 'committed_paths': [], 'pending_hashes': []})
                                attempt['pending_paths'].append(label)
                                attempt['pending_hashes'].append(domain_hash('stock_analysis.counted_push_log_artifact.v1', raw))
                            elif value['state'] == 'Committed':
                                commits.append((key, label, value['pending_artifact_sha256']))
                            else:
                                raise ValueError('unknown counted state')
                    except (ValueError, KeyError, TypeError, OSError):
                        unavailable_files.append({'path': label, 'reason': 'unavailable_file_or_artifact_hash'})
            finally:
                os.close(dayfd)
            if truncated:
                break
    finally:
        os.close(rootfd)
    manifests = []
    if archive_root:
        archivefd = open_dir(archive_root, private=True)
        try:
            for name in sorted(os.listdir(archivefd)):
                if not name.endswith('.manifest.json'):
                    continue
                remaining = limit - read_count - archive_io['member_read_attempts']
                if remaining <= 0 or archive_dates_attempted >= limit:
                    truncated = True
                    break
                archive_dates_attempted += 1
                day = name[:-len('.manifest.json')]
                try:
                    date(day)
                    manifest, bodies = validate_archive(archivefd, day, max_members=remaining, accounting=archive_io)
                    if manifest['source_root'] != str(push_log):
                        raise ValueError('archive source mismatch')
                    manifests.append({'path': str(archive_root / name), 'sha256': digest(json_bytes(manifest))})
                    # Entire date fits the remaining budget and was fully verified before any grouping.
                    for path, body in sorted(bodies.items()):
                        processed += 1
                        record_body(path, body, 'verified_rotated_markdown',
                                    str(archive_root / manifest['archive']) + '#' + path)
                except ArchiveMemberBudgetExceeded as error:
                    truncated = True
                    unavailable_files.append({'path': name, 'reason': 'archive_member_budget_exceeded_before_tar_read',
                                              'declared_members': error.declared, 'available_member_budget': error.available})
                except (OSError, ValueError, KeyError, TypeError, tarfile.TarError):
                    unavailable_files.append({'path': name, 'reason': 'archive_verification_unavailable'})
        finally:
            os.close(archivefd)
    conflicts = []
    for path, loaded in sorted(revisions.items()):
        if len({revision['sha256'] for revision in loaded}) > 1:
            conflicts.append({'path': path, 'revisions': loaded})
            unavailable_files.append({'path': path, 'reason': 'live_archive_path_hash_mismatch'})
            for paths in hashes.values():
                paths.pop(path, None)  # Unavailable original identity cannot support any candidate group.
    for key, path, pending_hash in commits:
        attempt = attempts.get(key)
        if not attempt or pending_hash not in attempt['pending_hashes']:
            unavailable_files.append({'path': path, 'reason': 'unavailable_pending_commit_join'})
        else:
            attempt['committed_paths'].append(path)
    return {'status': 'partial_unavailable' if unavailable_files or truncated else 'candidates_only',
            'source_root': str(push_log), 'archive_root': str(archive_root) if archive_root else None,
            'files_read': source_successes + archive_io['file_read_successes'],
            'processed_occurrences': processed, 'truncated': truncated, 'max_files': limit,
            'read_limit_semantics': 'source_leaf_attempts_plus_archive_member_attempts; separate_archive_date_attempt_budget',
            'read_budgets': {'source_leaf_plus_archive_members': limit, 'archive_date_attempts': limit,
                             'manifest_max_bytes': MAX_FILE, 'tar_max_bytes': ARCHIVE_LIMIT},
            'read_accounting': {'source_leaf_read_attempts': read_count, 'source_leaf_read_successes': source_successes,
                                'source_leaf_bytes_returned': source_bytes, 'archive_dates_attempted': archive_dates_attempted,
                                'archive_verification': archive_io},
            'content_conflicts': conflicts,
            'candidates': [{'kind': 'archive_body_duplicate', 'sha256': sha,
                            'occurrences': [{'path': p, 'source': source} for p, source in sorted(paths.items())]}
                           for sha, paths in sorted(hashes.items()) if len(paths) > 1],
            'counted_attempts': [dict(decision_identity_hash=key[0], attempt_identity_hash=key[1], **value)
                                 for key, value in sorted(attempts.items())],
            'counted_artifact_scope': 'pending_and_committed_one_attempt_delivery_audit_join_unavailable',
            'archive_manifests': manifests, 'unavailable_files': unavailable_files,
            'physical_delivery_inference': 'unavailable_from_markdown'}


def run(args):
    now = clock(args.observed_at)
    try:
        calendar = Calendar(args.calendar)
        calendar_meta = {'path': str(args.calendar), 'sha256': calendar.sha256, 'years': sorted(calendar.years), 'status': 'verified_csv_coverage'}
    except (OSError, ValueError, UnicodeError):
        calendar = None
        calendar_meta = {'path': str(args.calendar), 'status': 'unavailable'}
    report = {'schema_version': 1, 'observed_at': now.isoformat(), 'scope': 'read_only_candidates_no_adjudication',
              'calendar': calendar_meta, 'sources': [], 'read_limits': {'max_rows_per_rule': args.max_rows, 'max_archive_files': args.max_files}, 'rules': {}}
    with tempfile.TemporaryDirectory(prefix='reliability-screen-') as temporary:
        directory = Path(temporary).resolve()
        for label, source in (('main', args.db), ('delivery', args.delivery_db)):
            connection = None
            try:
                connection, meta = snapshot(source, directory / (label + '.db'))
                meta['role'] = label
                report['sources'].append(meta)
                if label == 'main':
                    for rule, function in (('R1', lambda: screen_r1(connection, args.max_rows, args.price_authority_evidence)),
                                           ('R3', lambda: screen_r3(connection, calendar, args.max_rows)),
                                           ('R5', lambda: screen_r5(connection, args.paper_evidence, args.max_rows))):
                        try:
                            report['rules'][rule] = function()
                        except (sqlite3.Error, ValueError, TypeError, OSError):
                            report['rules'][rule] = unavailable('source_schema_or_evidence_unavailable')
                else:
                    report['rules']['R2_physical'] = screen_physical(connection, args.max_rows)
            except (sqlite3.Error, ValueError, OSError, TimeoutError, RuntimeError):
                report['sources'].append({'role': label, 'path': str(source), 'status': 'unavailable'})
                for rule in (('R1', 'R3', 'R5') if label == 'main' else ('R2_physical',)):
                    report['rules'][rule] = unavailable('source_snapshot_or_schema_unavailable')
            finally:
                if connection:
                    connection.close()
        try:
            report['rules']['R2_archive'] = screen_archives(args.push_log, args.archive_root, args.max_files)
        except (ValueError, OSError, TypeError):
            report['rules']['R2_archive'] = unavailable('archive_source_unavailable')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('db', 'delivery-db', 'push-log', 'calendar', 'output'):
        parser.add_argument('--' + name, required=True, type=absolute)
    for name in ('archive-root', 'price-authority-evidence', 'paper-evidence'):
        parser.add_argument('--' + name, type=absolute)
    parser.add_argument('--max-rows', type=int, default=10000)
    parser.add_argument('--max-files', type=int, default=20000)
    parser.add_argument('--observed-at')
    args = parser.parse_args()
    if not 1 <= args.max_rows <= 100000 or not 1 <= args.max_files <= 100000:
        parser.error('read limits must be between 1 and 100000')
    os.umask(0o077)
    try:
        if args.output.exists() or args.output.is_symlink():
            raise ValueError('output must be new')
        exclusive_json(args.output, run(args))
        print('read_only_candidates_persisted')
        return 0
    except (ValueError, OSError):
        print('quality_screen_output_or_input_unavailable')
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
