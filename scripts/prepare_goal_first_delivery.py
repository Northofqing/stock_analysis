#!/usr/bin/env python3
"""Fixed goal-first tools package. Default check/plan; installation never runs jobs."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import stat
import subprocess
import tempfile

RUNTIME = Path('/Users/zhangzhen/.local/share/stock-analysis-runtime')
AGENTS = Path('/Users/zhangzhen/Library/LaunchAgents')
BINS = ('weekly_outcome_review', 'assistant_review', 'sell_reminder_preview',
        'streak_leader_research', 'day_capture_check')
SCRIPTS = ('weekly-outcome-review.py', 'monitor_watchdog.py', 'reliability_common.py',
           'rotate_push_log.py', 'reliability_quality_screen.py', 'performance_sli.py')
LABELS = ('com.stockanalysis.weekly-outcome-review', 'com.stockanalysis.watchdog')
RESOURCES = ('signal_registry.toml', 'a_share_market_holidays.csv')
CONTRACTS = ('contracts/local_bridge_v1/market.proto',
             'contracts/external_v1_current/market.proto', 'contracts/external_v1_current/bundle-metadata.json',
             'contracts/external_v1_history/market.proto', 'contracts/external_v1_history/bundle-20260917.1.json',
             'contracts/external_v1_history/20260928.2/market.proto', 'contracts/external_v1_history/20260928.2/bundle-metadata.json',
             'contracts/external_v1_history/20261001.3/market.proto', 'contracts/external_v1_history/20261001.3/bundle-metadata.json',
             'contracts/durable_monitor_v9/schema.sql')


def clean_path(path):
    path = Path(path)
    if not path.is_absolute() or any(p in ('.', '..') for p in str(path).split('/')):
        raise ValueError('absolute normalized path required')
    for p in (path, *path.parents):
        if p.exists() or p.is_symlink():
            if p.is_symlink():
                raise ValueError('symlink path refused')
    return path


def read(path, private=False, limit=128 * 1024 * 1024):
    path = clean_path(path)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or before.st_uid != os.getuid():
            raise ValueError('owned single-link regular file required')
        if private and before.st_mode & 0o077:
            raise ValueError('private file required')
        if limit is not None and before.st_size > limit:
            raise ValueError('size limit')
        with os.fdopen(fd, 'rb', closefd=False) as stream:
            data = stream.read(before.st_size + 1)
        after = os.fstat(fd)
        if (before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns) or len(data) != before.st_size:
            raise ValueError('file changed during read')
        return data
    finally:
        os.close(fd)


def identity(data):
    return {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}


def write(path, data, mode=0o600):
    clean_path(path.parent)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    try:
        with os.fdopen(fd, 'wb', closefd=False) as stream:
            stream.write(data)
            stream.flush()
            os.fsync(fd)
        os.fchmod(fd, mode)
    finally:
        os.close(fd)


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + '\n').encode()


def baseline(runtime=RUNTIME, agents=AGENTS):
    # Hash bytes only: never read .env, open databases, probe monitor or prepare activation.
    paths = []
    for prefix in ('src', 'config'):
        directory = clean_path(runtime / prefix)
        if not directory.is_dir():
            raise ValueError('missing monitor-bound input tree')
        for root, dirs, files in os.walk(directory, followlinks=False):
            for name in dirs:
                clean_path(Path(root) / name)
            paths.extend(Path(root) / name for name in files)
    paths.extend(runtime.glob('Cargo*.toml'))
    if len(paths) > 4096:
        raise ValueError('monitor-bound input inventory limit')
    paths.extend(runtime / name for name in ('Cargo.toml', 'Cargo.lock', 'build.rs', *CONTRACTS,
                 'target/release/monitor', 'target/release/grpc_market_server'))
    result = {}
    for path in sorted(set(paths)):
        result[str(path)] = identity(read(path))
    for path in (runtime / 'bin/run-weekly-outcome-review.sh', *(agents / (label + '.plist') for label in LABELS),
                 agents / 'com.stockanalysis.monitor.plist', agents / 'com.northofqing.grpc-market-server.plist',
                 agents / 'com.stockanalysis.prediction-verify.plist'):
        result[str(path)] = identity(read(path)) if path.exists() else None
    return result


def layout():
    return {**{'bin/' + n: 0o500 for n in BINS},
            **{'scripts/' + n: 0o600 for n in SCRIPTS},
            'scripts/run-weekly-outcome-review.sh': 0o500,
            'scripts/prepare_goal_first_delivery.py': 0o600,
            **{'resources/' + n: 0o400 for n in RESOURCES},
            **{'resources/' + n.replace('/', '__'): 0o400 for n in CONTRACTS},
            **{'launchd/' + label + '.plist': 0o600 for label in LABELS},
            'RUNBOOK.md': 0o600}


def validate(bundle):
    bundle = clean_path(bundle)
    info = bundle.stat()
    if not bundle.is_dir() or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError('private package root required')
    for root, dirs, files in os.walk(bundle, followlinks=False):
        for name in dirs:
            p = clean_path(Path(root) / name)
            info = p.stat()
            if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
                raise ValueError('private package directory required')
    m = json.loads(read(bundle / 'manifest.json', True, 2 * 1024 * 1024))
    version = m['version']
    if not version or len(version) > 64 or version in ('.', '..') or any(c not in 'abcdefghijklmnopqrstuvwxyz0123456789-.' for c in version):
        raise ValueError('version')
    destination = RUNTIME / 'tools/goal-first' / version
    if m['schema'] != 'goal-first-tools-package/v1' or m['runtime'] != str(RUNTIME) or m['destination'] != str(destination) or m['launchd_labels'] != list(LABELS):
        raise ValueError('fixed destination/labels mismatch')
    fixed = layout()
    if set(m['files']) != set(fixed):
        raise ValueError('file allowlist mismatch')
    actual = {str(p.relative_to(bundle)) for p in bundle.rglob('*') if not p.is_dir()}
    if actual != set(fixed) | {'manifest.json'}:
        raise ValueError('unlisted package file')
    for name, mode in fixed.items():
        p = bundle / name
        data = read(p, True)
        if identity(data) != {k: m['files'][name][k] for k in ('bytes', 'sha256')} or stat.S_IMODE(p.stat().st_mode) != mode:
            raise ValueError('package content/mode mismatch: ' + name)
    expected_launcher = launcher(destination).encode()
    if read(bundle / 'scripts/run-weekly-outcome-review.sh', True) != expected_launcher:
        raise ValueError('launcher version path mismatch')
    for label in LABELS:
        p = plistlib.loads(read(bundle / ('launchd/' + label + '.plist'), True))
        if p != job(label, destination):
            raise ValueError('plist dispatch/schedule mismatch')
    return m


def launcher(destination):
    return '#!/bin/bash\nset -euo pipefail\numask 077\nexec /usr/bin/python3 -B "' + str(destination / 'scripts/weekly-outcome-review.py') + '" --database "' + str(RUNTIME / 'data/stock_analysis.db') + '" --binary "' + str(destination / 'bin/weekly_outcome_review') + '" --registry "' + str(destination / 'resources/signal_registry.toml') + '" --weekly-output-root "' + str(RUNTIME / 'reports/weekly-outcome-review') + '"\n'


def job(label, destination):
    weekly = label == LABELS[0]
    args = ['/bin/bash', str(destination / 'scripts/run-weekly-outcome-review.sh')] if weekly else ['/usr/bin/python3', '-B', str(destination / 'scripts/monitor_watchdog.py'), '--runtime-root', str(RUNTIME), '--calendar', str(destination / 'resources/a_share_market_holidays.csv'), '--output-root', str(RUNTIME / 'data/watchdog'), '--probe-timeout-seconds', '5']
    p = {'Label': label, 'ProgramArguments': args, 'WorkingDirectory': str(RUNTIME),
         'RunAtLoad': False, 'KeepAlive': False, 'Umask': 63,
         'EnvironmentVariables': {'TZ': 'Asia/Shanghai', 'PYTHONDONTWRITEBYTECODE': '1'},
         'StandardOutPath': str(RUNTIME / ('logs/' + label + '.stdout.log')),
         'StandardErrorPath': str(RUNTIME / ('logs/' + label + '.stderr.log'))}
    p.update({'StartCalendarInterval': {'Weekday': 5, 'Hour': 20, 'Minute': 30}} if weekly else {'StartInterval': 60})
    return p


def check(bundle):
    m = validate(bundle)
    if baseline() != m['preconditions']:
        raise ValueError('monitor-bound inputs or prior launcher state drifted; regenerate reviewed package')
    return m


def install(bundle):
    m = check(bundle)
    destination = clean_path(Path(m['destination']))
    # Exact final directory is atomically published; do not overwrite any existing version.
    if destination.exists():
        old = validate(destination)
        if read(destination / 'manifest.json', True) != read(bundle / 'manifest.json', True):
            raise ValueError('existing version differs')
        return {'status': 'already_published', 'destination': str(destination), 'jobs': 'not_loaded'}
    parent = destination.parent
    for p in reversed((parent, *parent.parents)):
        if p == RUNTIME or RUNTIME in p.parents:
            if not p.exists():
                p.mkdir(mode=0o700)
            if p.stat().st_uid != os.getuid():
                raise ValueError('destination owner')
    staging = Path(tempfile.mkdtemp(prefix='.goal-first-', dir=parent))
    try:
        shutil.copytree(bundle, staging, dirs_exist_ok=True)
        validate(staging)
        check(bundle)  # refuse drift during preparation, before publication
        # macOS RENAME_EXCL gives atomic publication without replacing a raced version.
        native = ctypes.CDLL(None, use_errno=True)
        if native.renamex_np(os.fsencode(staging), os.fsencode(destination), 4) != 0:
            raise OSError(ctypes.get_errno(), 'atomic exclusive package publication failed')
        fd = os.open(parent, os.O_RDONLY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return {'status': 'published', 'destination': str(destination), 'jobs': 'not_loaded'}


def prepare(repo, candidate, version, source_commit, build_log, base_commit=None, assistant_log=None):
    if not version or len(version) > 64 or version in ('.', '..') or any(c not in 'abcdefghijklmnopqrstuvwxyz0123456789-.' for c in version):
        raise ValueError('version')
    repo = clean_path(repo)
    if subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip() != source_commit or subprocess.check_output(['git', 'diff', 'HEAD', '--name-only'], cwd=repo):
        raise ValueError('clean committed source required')
    if not base_commit or len(base_commit) != 40 or any(c not in '0123456789abcdef' for c in base_commit) or assistant_log is None:
        raise ValueError('explicit two-build provenance required')
    changed_binary_inputs = subprocess.check_output(['git', 'diff', '--name-only', base_commit, source_commit, '--', 'src', 'config', 'contracts', 'Cargo.toml', 'Cargo.lock', 'build.rs', 'build_support'], cwd=repo, text=True).splitlines()
    if set(changed_binary_inputs) - {'src/bin/assistant_review.rs'}:
        raise ValueError('other compiled inputs changed since first release build')
    bundle = clean_path(candidate) / 'goal-first' / version
    if 'Desktop' in bundle.parts or bundle.exists():
        raise ValueError('fresh Desktop-external candidate required')
    bundle.mkdir(parents=True, mode=0o700)
    for p in (bundle.parent, bundle, *(bundle / n for n in ('bin', 'scripts', 'resources', 'launchd'))):
        p.mkdir(exist_ok=True, mode=0o700)
        p.chmod(0o700)
    destination = RUNTIME / 'tools/goal-first' / version
    for name in BINS:
        write(bundle / ('bin/' + name), read(repo / ('target/release/' + name)), 0o500)
    for name in (*SCRIPTS, 'prepare_goal_first_delivery.py'):
        write(bundle / ('scripts/' + name), read(repo / ('scripts/' + name)))
    for name in RESOURCES:
        write(bundle / ('resources/' + name), read(repo / ('config/' + name)), 0o400)
    for name in CONTRACTS:
        write(bundle / ('resources/' + name.replace('/', '__')), read(repo / name), 0o400)
    write(bundle / 'scripts/run-weekly-outcome-review.sh', launcher(destination).encode(), 0o500)
    for label in LABELS:
        write(bundle / ('launchd/' + label + '.plist'), plistlib.dumps(job(label, destination)))
    write(bundle / 'RUNBOOK.md', read(repo / 'docs/ops/goal-first-delivery-runbook.md'))
    preconditions = baseline()
    rollback = bundle.parent.parent / ('rollback-' + version)
    rollback.mkdir(mode=0o700)
    rollback_files = {}
    for path in (RUNTIME / 'bin/run-weekly-outcome-review.sh', *(AGENTS / (label + '.plist') for label in LABELS)):
        data = read(path) if path.exists() else None
        if data is not None:
            target = rollback / path.name
            write(target, data, 0o400)
            rollback_files[str(path)] = {'preserved_path': str(target), **identity(data)}
        else:
            rollback_files[str(path)] = {'absent': True}
    activation = json.loads(read(RUNTIME / 'config/selection/selection_activation.v1.json'))
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--offline', '--format-version', '1'], cwd=repo))
    m = {'schema': 'goal-first-tools-package/v1', 'version': version, 'source_commit': source_commit, 'source_dirty': False,
         'runtime': str(RUNTIME), 'destination': str(destination), 'launchd_labels': list(LABELS),
         'build': {'selected_bins': list(BINS), 'source_commit': base_commit, 'per_bin_source_commit': {name: source_commit if name == 'assistant_review' else base_commit for name in BINS}, 'assistant_rebuild': {'source_commit': source_commit, 'selected_bins': ['assistant_review'], 'log': str(assistant_log), 'log_identity': identity(read(assistant_log))}, 'profile': 'release', 'production_root': str(destination), 'rustc': subprocess.check_output(['rustc', '-Vv'], text=True), 'cargo': subprocess.check_output(['cargo', '-V'], text=True), 'log': str(build_log), 'log_identity': identity(read(build_log)), 'features': [], 'python': subprocess.check_output(['/usr/bin/python3', '--version'], text=True).strip(), 'metadata_target_count': len(metadata['packages'][0]['targets'])},
         'schemas': ['H16-descriptive-weekly-v1', 'weekly-signal-scorecard-v1', 'weekly-outcome-evidence-manifest-v1', 'assistant-phase-a-comparison-v1', 'sell-preview/v1-legacy-rule-units', 'streak-study/v1', 'monitor-health-runtime_snapshot-v2'],
         'preconditions': preconditions, 'rollback': rollback_files,
         'observed_activation_expected_config_hash': activation.get('expected_config_hash'),
         'attestation': 'disk observations only; no live process, Gateway, PIT or physical delivery authority',
         'source_inputs': {name: identity(read(repo / name)) for name in subprocess.check_output(['git', 'ls-files', 'src', 'config', 'contracts', 'Cargo.toml', 'Cargo.lock', 'build.rs', 'build_support'], cwd=repo, text=True).splitlines()},
         'files': {name: {**identity(read(bundle / name, True)), 'mode': oct(mode)} for name, mode in layout().items()}}
    write(bundle / 'manifest.json', json_bytes(m))
    validate(bundle)
    return {'bundle': str(bundle), 'manifest': identity(read(bundle / 'manifest.json', True)), 'rollback': str(rollback)}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--bundle', type=Path)
    p.add_argument('--install', action='store_true', help='publish this immutable version only; never load jobs')
    p.add_argument('--prepare', action='store_true')
    p.add_argument('--repo', type=Path)
    p.add_argument('--candidate-root', type=Path)
    p.add_argument('--version')
    p.add_argument('--source-commit')
    p.add_argument('--build-log', type=Path)
    p.add_argument('--base-binary-source-commit')
    p.add_argument('--assistant-build-log', type=Path)
    a = p.parse_args()
    try:
        if a.prepare:
            if a.install or a.bundle or not all((a.repo, a.candidate_root, a.version, a.source_commit, a.build_log, a.base_binary_source_commit, a.assistant_build_log)):
                raise ValueError('explicit preparation arguments required')
            result = prepare(a.repo, a.candidate_root, a.version, a.source_commit, a.build_log, a.base_binary_source_commit, a.assistant_build_log)
        elif a.bundle:
            result = install(a.bundle) if a.install else {'status': 'checked_plan_only', 'destination': check(a.bundle)['destination'], 'jobs': 'not_loaded', 'changes': 'none'}
        else:
            raise ValueError('--bundle or --prepare required')
        print(json.dumps(result, ensure_ascii=False, indent=2))
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        p.exit(2, 'goal-first delivery refused: ' + str(error) + '\n')


if __name__ == '__main__':
    main()
