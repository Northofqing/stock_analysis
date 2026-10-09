#!/usr/bin/env python3
"""Lossless Markdown rotation under the existing writer lock. Default: dry-run."""
import argparse
import datetime as dt
import fcntl
import io
import json
import os
from pathlib import Path
import stat
import tarfile
import time

from reliability_common import (MAX_FILE, absolute, clock, date, digest, identity,
                                json_bytes, open_dir, read_at, regular, write_at)

ARCHIVE_LIMIT = 64 * 1024 * 1024


def same_directory(path, fd):
    probe = open_dir(path)
    try:
        if identity(os.fstat(probe))[:2] != identity(os.fstat(fd))[:2]:
            raise ValueError('directory identity changed')
    finally:
        os.close(probe)


def validate_archive(archivefd, day):
    manifest_raw, _ = read_at(archivefd, day + '.manifest.json', private=True)
    manifest = json.loads(manifest_raw)
    if manifest['schema_version'] != 1 or manifest['date'] != day or manifest['archive'] != day + '.tar':
        raise ValueError('archive manifest schema invalid')
    raw, _ = read_at(archivefd, manifest['archive'], ARCHIVE_LIMIT, private=True)
    if len(raw) != manifest['archive_bytes'] or digest(raw) != manifest['archive_sha256']:
        raise ValueError('archive checksum mismatch')
    expected = {e['path']: e for e in manifest['entries']}
    if len(expected) != len(manifest['entries']):
        raise ValueError('duplicate manifest path')
    for path in expected:
        parts = Path(path).parts
        if len(parts) != 2 or parts[0] != day or parts[1] in ('.', '..') or not parts[1].endswith('.md'):
            raise ValueError('invalid archive path')
    bodies = {}
    with tarfile.open(fileobj=io.BytesIO(raw), mode='r:') as archive:
        for member in archive:
            if not member.isfile() or member.name not in expected or member.name in bodies or member.size > MAX_FILE:
                raise ValueError('unexpected archive member')
            body = archive.extractfile(member).read(MAX_FILE + 1)
            entry = expected[member.name]
            if len(body) != entry['bytes'] or digest(body) != entry['sha256']:
                raise ValueError('archive member checksum mismatch')
            bodies[member.name] = body
    if set(bodies) != set(expected):
        raise ValueError('archive member set mismatch')
    return manifest, bodies


def rotate(runtime, archive_root, before=None, archive=False, prune=False,
           observed_at=None, retention=30, max_dates=1, max_files=2000,
           lock_wait=2, time_limit=30, after_unlink=None):
    runtime, archive_root = absolute(runtime), absolute(archive_root)
    root = runtime / 'data/push_log'
    if archive_root == root or root in archive_root.parents:
        raise ValueError('archive root must be outside push_log')
    if prune and not archive:
        raise ValueError('prune requires --archive and immutable verified archive')
    now = clock(observed_at)
    cutoff = date(before) if before else now.date() - dt.timedelta(days=retention)
    cutoff = min(cutoff, now.date())
    deadline = time.monotonic() + time_limit
    rootfd = open_dir(root)
    lockfd = archivefd = None
    results = []
    try:
        lockfd = os.open('.push_log.lock', os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=rootfd)
        lock_info = os.fstat(lockfd)
        regular(lock_info)
        lock_deadline = min(deadline, time.monotonic() + lock_wait)
        while True:
            try:
                fcntl.flock(lockfd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= lock_deadline:
                    raise TimeoutError('push_log_lock_unavailable')
                time.sleep(0.02)
        def binding():
            if time.monotonic() > deadline:
                raise TimeoutError('rotation_time_limit')
            same_directory(root, rootfd)
            named = os.stat('.push_log.lock', dir_fd=rootfd, follow_symlinks=False)
            if identity(named) != identity(lock_info):
                raise ValueError('writer lock identity changed')
            if archivefd is not None:
                same_directory(archive_root, archivefd)
        binding()
        days = []
        for name in sorted(os.listdir(rootfd)):
            try:
                day = date(name)
            except ValueError:
                continue
            if day < cutoff:
                days.append(name)
        selected = days[:max_dates]
        if archive:
            archivefd = open_dir(archive_root, create=True, private=True)
        for day in selected:
            binding()
            daypath = root / day
            dayfd = os.open(day, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=rootfd)
            try:
                if os.fstat(dayfd).st_mode & 0o022:
                    raise ValueError('writable source date directory')
                names = sorted(n for n in os.listdir(dayfd) if n.endswith('.md'))
                if len(names) > max_files:
                    raise ValueError('date exceeds max-files; no partial date archive')
                manifest = bodies = None
                if archive:
                    try:
                        manifest, bodies = validate_archive(archivefd, day)
                    except FileNotFoundError:
                        # A published orphan tar from an interrupted publication is never overwritten.
                        if day + '.tar' in os.listdir(archivefd):
                            raise ValueError('orphan archive requires human review')
                if manifest is None:
                    entries, bodies, total = [], {}, 0
                    for name in names:
                        binding()
                        same_directory(daypath, dayfd)
                        raw, witness = read_at(dayfd, name)
                        total += len(raw)
                        if total > ARCHIVE_LIMIT - 1024 * 1024:
                            raise ValueError('date exceeds archive byte limit')
                        path = day + '/' + name
                        bodies[path] = raw
                        entries.append({'path': path, 'bytes': len(raw), 'sha256': digest(raw), 'source_identity': witness})
                    manifest = {'schema_version': 1, 'date': day, 'source_root': str(root),
                                'archive': day + '.tar', 'entries': entries}
                    if archive:
                        buffer = io.BytesIO()
                        with tarfile.open(fileobj=buffer, mode='w', format=tarfile.USTAR_FORMAT) as tar:
                            for entry in entries:
                                member = tarfile.TarInfo(entry['path'])
                                member.size, member.mode = entry['bytes'], 0o600
                                tar.addfile(member, io.BytesIO(bodies[entry['path']]))
                        raw = buffer.getvalue()
                        manifest.update(archive_bytes=len(raw), archive_sha256=digest(raw))
                        binding()
                        same_directory(daypath, dayfd)
                        # Recheck entire source inventory before publication.
                        if names != sorted(n for n in os.listdir(dayfd) if n.endswith('.md')):
                            raise ValueError('source inventory changed')
                        for entry in entries:
                            raw_source, witness = read_at(dayfd, Path(entry['path']).name)
                            if witness != entry['source_identity'] or digest(raw_source) != entry['sha256']:
                                raise ValueError('source changed before publication')
                        write_at(archivefd, day + '.tar', raw)
                        write_at(archivefd, day + '.manifest.json', json_bytes(manifest))
                        manifest, bodies = validate_archive(archivefd, day)
                if manifest['source_root'] != str(root):
                    raise ValueError('archive belongs to another source root')
                removed, already_absent = [], []
                if prune:
                    # Validate ALL remaining sources before deleting any; crash-resume accepts absent members.
                    for entry in manifest['entries']:
                        try:
                            raw, witness = read_at(dayfd, Path(entry['path']).name)
                        except FileNotFoundError:
                            already_absent.append(entry['path'])
                            continue
                        if witness != entry['source_identity'] or len(raw) != entry['bytes'] or digest(raw) != entry['sha256']:
                            raise ValueError('source changed; pruning refused')
                    for entry in manifest['entries']:
                        if entry['path'] in already_absent:
                            continue
                        binding()
                        same_directory(daypath, dayfd)
                        raw, witness = read_at(dayfd, Path(entry['path']).name)
                        if witness != entry['source_identity'] or digest(raw) != entry['sha256']:
                            raise ValueError('source changed before unlink')
                        os.unlink(Path(entry['path']).name, dir_fd=dayfd)
                        os.fsync(dayfd)
                        removed.append(entry['path'])
                        if after_unlink:
                            after_unlink(entry['path'])
                same_directory(daypath, dayfd)
                remaining = sorted(n for n in os.listdir(dayfd) if n.endswith('.md'))
                result = {'date': day, 'members': len(manifest['entries']), 'removed': removed,
                          'already_absent': already_absent, 'remaining_md': remaining,
                          'manifest_sha256': digest(json_bytes(manifest)), 'mode': 'pruned' if prune else 'archived' if archive else 'dry_run'}
                results.append(result)
            finally:
                os.close(dayfd)
        binding()
        return {'schema_version': 1, 'observed_at': now.isoformat(), 'before_exclusive': cutoff.isoformat(),
                'eligible_dates': len(days), 'selected_dates': len(selected), 'truncated': len(days) > len(selected),
                'max_dates': max_dates, 'max_files_per_date': max_files, 'time_limit_seconds': time_limit,
                'results': results, 'json_and_directories_preserved': True}
    finally:
        for fd in (archivefd, lockfd, rootfd):
            if fd is not None:
                os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime-root', required=True, type=absolute)
    parser.add_argument('--archive-root', required=True, type=absolute)
    parser.add_argument('--before')
    parser.add_argument('--retention-days', type=int, default=30)
    parser.add_argument('--archive', action='store_true')
    parser.add_argument('--prune-verified-md', action='store_true')
    parser.add_argument('--max-dates', type=int, default=1)
    parser.add_argument('--max-files', type=int, default=2000)
    parser.add_argument('--lock-wait-seconds', type=float, default=2)
    parser.add_argument('--time-limit-seconds', type=float, default=30)
    parser.add_argument('--observed-at')
    parser.add_argument('--output', type=absolute, help='optional private exclusive JSON evidence')
    args = parser.parse_args()
    if not (1 <= args.retention_days <= 3650 and 1 <= args.max_dates <= 10 and 1 <= args.max_files <= 10000
            and 0 <= args.lock_wait_seconds <= 10 and 0 < args.time_limit_seconds <= 120):
        parser.error('bounds invalid')
    os.umask(0o077)
    try:
        if args.output and (args.output.exists() or args.output.is_symlink()):
            raise ValueError('output must be new')
        report = rotate(args.runtime_root, args.archive_root, args.before, args.archive, args.prune_verified_md,
                        args.observed_at, args.retention_days, args.max_dates, args.max_files,
                        args.lock_wait_seconds, args.time_limit_seconds)
        if args.output:
            from reliability_common import exclusive_json
            exclusive_json(args.output, report)
        else:
            print(json.dumps(report, ensure_ascii=False, sort_keys=True))
        return 0
    except (OSError, ValueError, TimeoutError, KeyError, tarfile.TarError):
        print('rotation_unavailable; originals not eligible for unverified pruning')
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
