import importlib.util
import json
import hashlib
import os
from pathlib import Path
import plistlib
import sqlite3
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "weekly_outcome_review", Path(__file__).resolve().parents[1] / "weekly-outcome-review.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReadOnlyWeeklySnapshotTests(unittest.TestCase):
    def fake_cli(self, root, fail_markdown=False, mutate_registry=None, mutate_binding=None):
        binary = root / "fake_cli.py"
        binary.write_text("""#!/usr/bin/env python3
import hashlib, json, os, pathlib, sys
args=sys.argv
def value(flag): return args[args.index(flag)+1]
if value('--format')=='markdown' and FAIL_MARKDOWN: sys.exit(23)
report={'observed_at':value('--observed-at'),'snapshot':value('--database'),
        'sha256':hashlib.sha256(pathlib.Path(value('--database')).read_bytes()).hexdigest(),
        'source_label':value('--source-label'),
        'binding':os.environ.get('PAPER_LEDGER_ACCOUNT_BINDING'),
        'private_env_secret':os.environ.get('TEST_CODE_ENV_SECRET'),
        'period':{'observed_at':value('--observed-at'),'latest_completed_session':'2026-10-09','period_completed_through':'2026-10-09'}}
if '--snapshot-source-manifest' in args:
    report['snapshot_source']=json.loads(pathlib.Path(value('--snapshot-source-manifest')).read_text())
if '--registry' in args:
    registry=pathlib.Path(value('--registry'))
    report['registry_content']=registry.read_text()
    report['registry_sha256']=hashlib.sha256(registry.read_bytes()).hexdigest()
    report['registry_snapshot']=str(registry)
manifest=pathlib.Path(value('--evidence-manifest'))
if manifest.exists():
    assert json.loads(manifest.read_text()) == report
else:
    fd=os.open(manifest,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
    with os.fdopen(fd,'w') as stream: json.dump(report,stream)
if MUTATE_REGISTRY and value('--format')=='json':
    pathlib.Path(MUTATE_REGISTRY).write_text('changed after first child')
if MUTATE_BINDING and value('--format')=='json':
    pathlib.Path(MUTATE_BINDING).write_text('PAPER_LEDGER_ACCOUNT_BINDING=changed after first child')
fd=os.open(value('--output'),os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(fd,'w') as stream: json.dump(report,stream)
""".replace("FAIL_MARKDOWN", repr(fail_markdown)).replace("MUTATE_REGISTRY", repr(str(mutate_registry) if mutate_registry else None)).replace("MUTATE_BINDING", repr(str(mutate_binding) if mutate_binding else None)))
        binary.chmod(0o700)
        return binary

    def fake_assistant(self, root, exit_code=0, runtime=False):
        directory = root / "bin" if runtime else root
        binary = directory / "assistant_review"
        binary.write_bytes(b"TEST_CODE_binary_marker")
        script = directory / "run-weekly-assistant-review.py"
        script.write_text("""import argparse, json, os, pathlib, sys
p=argparse.ArgumentParser()
for name in ('report','manifest','as-of','completed-session','output-dir','assistant-binary','registry'):
    p.add_argument('--'+name)
a=p.parse_args()
report=json.loads(pathlib.Path(a.report).read_text())
assert a.as_of==report['period']['observed_at']
assert a.completed_session==report['period']['latest_completed_session']
assert json.loads(pathlib.Path(a.manifest).read_text())==report
output=pathlib.Path(a.output_dir); output.mkdir(mode=0o700)
result={'as_of':a.as_of,'completed_session':a.completed_session,'calls':1,'registry':pathlib.Path(a.registry).read_text() if a.registry else None}
for name in ('comparison.json','status.json'):
    fd=os.open(output/name,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
    with os.fdopen(fd,'w') as stream: json.dump(result,stream)
if EXIT_CODE==0: (output/'comparison.md').write_text('TEST_CODE_comparison')
sys.exit(EXIT_CODE)
""".replace("EXIT_CODE",str(exit_code)))
        return binary, script

    def test_assistant_hook_uses_original_period_once_and_preserves_completed_weekly_on_failure(self):
        for code in (0,23):
            with self.subTest(code=code), tempfile.TemporaryDirectory() as directory:
                root=Path(directory)
                source=root/'source.db'; sqlite3.connect(source).close()
                cli=self.fake_cli(root)
                cli.write_text(cli.read_text().replace("'period_completed_through':'2026-10-09'", "'period_completed_through':'2026-09-30'"))
                binary,script=self.fake_assistant(root,exit_code=code)
                registry=root/'registry.toml'; registry.write_text('TEST_CODE_original_registry')
                output=root/'reports'
                self.assertEqual(MODULE.weekly_run(source,cli,output,'2026-10-09T22:00:00+08:00',registry=registry,assistant_binary=binary,assistant_script=script),code)
                version=next(output.iterdir())
                status=json.loads((version/'run-status.json').read_text())
                self.assertEqual(status['assistant']['invocations'],1)
                self.assertEqual(status['assistant']['status'],'complete' if code==0 else 'failed')
                self.assertEqual(status['stage'],'complete' if code==0 else 'complete_weekly_assistant_failed')
                for name in ('review.json','review.md','evidence-manifest.json'):
                    self.assertTrue((version/name).is_file()); self.assertIn(name,status['completed_artifacts'])
                comparison=json.loads((version/'assistant'/'comparison.json').read_text())
                self.assertEqual(comparison['calls'],1)
                self.assertEqual(comparison['as_of'],'2026-10-09T22:00:00+08:00')
                self.assertEqual(comparison['completed_session'],'2026-10-09')
                self.assertEqual(comparison['registry'],'TEST_CODE_original_registry')

    def test_weekly_failure_skips_assistant_and_missing_helper_keeps_weekly_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); source=root/'source.db'; sqlite3.connect(source).close()
            binary,script=self.fake_assistant(root)
            output=root/'reports'
            self.assertEqual(MODULE.weekly_run(source,self.fake_cli(root,fail_markdown=True),output,'2026-10-09T22:00:00+08:00',assistant_binary=binary,assistant_script=script),23)
            version=next(output.iterdir()); self.assertFalse((version/'assistant').exists())
            self.assertEqual(json.loads((version/'run-status.json').read_text())['assistant']['invocations'],0)
            output=root/'missing_helper'
            self.assertEqual(MODULE.weekly_run(source,self.fake_cli(root),output,'2026-10-09T22:00:00+08:00',assistant_binary=binary,assistant_script=root/'missing.py'),2)
            version=next(output.iterdir()); status=json.loads((version/'run-status.json').read_text())
            self.assertEqual(status['stage'],'complete_weekly_assistant_failed'); self.assertEqual(status['assistant']['invocations'],0)
            self.assertTrue((version/'review.json').exists()); self.assertTrue((version/'review.md').exists())

    def binding_env(self, root):
        binding = {"account_id": "TEST_CODE_account", "epoch_id": "TEST_CODE_epoch", "manifest_hash": "a" * 64}
        path = root / ".env"
        path.write_text("TEST_CODE_ENV_SECRET=TEST_CODE_do_not_forward\nPAPER_LEDGER_ACCOUNT_BINDING='" + json.dumps(binding, separators=(",", ":")) + "'\n")
        path.chmod(0o600)
        return path, json.dumps(binding, separators=(",", ":"))

    def test_binding_is_literal_private_frozen_and_only_binding_reaches_children(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "TEST_CODE_source.db"
            sqlite3.connect(source).close()
            env_file, binding = self.binding_env(root)
            binary = self.fake_cli(root, mutate_binding=env_file)
            with patch.dict(os.environ, {MODULE.BINDING_ENV: "TEST_CODE_wrong_inherited"}):
                self.assertEqual(MODULE.weekly_run(source, binary, root / "reports", "2026-10-09T22:00:00+08:00", binding_env_file=env_file), 0)
            version = next((root / "reports").iterdir())
            first = json.loads((version / "review.json").read_text())
            self.assertEqual(first, json.loads((version / "review.md").read_text()))
            self.assertEqual(first["binding"], binding)
            self.assertIsNone(first["private_env_secret"])
            context = first["snapshot_source"]
            identity = context["original_database"]
            self.assertEqual((identity["device"], identity["inode"]), (source.stat().st_dev, source.stat().st_ino))
            self.assertEqual(context["snapshot_sha256"], first["sha256"])
            self.assertEqual(context["paper_binding_sha256"], hashlib.sha256(binding.encode()).hexdigest())
            self.assertNotEqual(identity, context["snapshot_database"])

    def test_binding_rejects_shared_symlink_missing_duplicate_invalid_and_clears_inherited(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env_file, binding = self.binding_env(root)
            self.assertEqual(MODULE.read_binding_env(env_file), binding)
            env_file.chmod(0o644)
            with self.assertRaises(ValueError): MODULE.read_binding_env(env_file)
            env_file.chmod(0o600)
            link = root / "link"
            link.symlink_to(env_file)
            with self.assertRaises(OSError): MODULE.read_binding_env(link)
            for value in ("OTHER=unused", "PAPER_LEDGER_ACCOUNT_BINDING={}\nPAPER_LEDGER_ACCOUNT_BINDING={}", "PAPER_LEDGER_ACCOUNT_BINDING='$(touch should-never-exist)'", "PAPER_LEDGER_ACCOUNT_BINDING='{\"account_id\":\"a\",\"epoch_id\":\"b\",\"manifest_hash\":\"invalid\"}'"):
                env_file.write_text(value)
                with self.assertRaises(ValueError): MODULE.read_binding_env(env_file)
            self.assertFalse((root / "should-never-exist").exists())
            with patch.dict(os.environ, {MODULE.BINDING_ENV: "wrong"}):
                self.assertNotIn(MODULE.BINDING_ENV, MODULE.child_environment(None))

    def test_week_scope_is_shanghai_monday_sunday_including_holidays_and_close_boundary(self):
        for clock, expected in (
            ("2026-09-28T00:00:00+08:00", ("2026-09-28", "2026-10-04")),
            ("2026-10-04T23:59:59+08:00", ("2026-09-28", "2026-10-04")),
            ("2026-10-05T09:00:00+08:00", ("2026-10-05", "2026-10-11")),
            ("2026-10-09T14:59:59+08:00", ("2026-10-05", "2026-10-11")),
            ("2026-10-09T15:00:00+08:00", ("2026-10-05", "2026-10-11")),
        ):
            self.assertEqual(MODULE.weekly_scope(clock)[:2], expected)
        with self.assertRaises(ValueError):
            MODULE.weekly_scope("2026-10-09T12:00:00Z")

    def test_weekly_formats_share_snapshot_clock_private_modes_and_keep_versions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "TEST_CODE_source.db"
            sqlite3.connect(source).close()
            before = source.read_bytes()
            binary = self.fake_cli(root)
            reports = root / "reports"
            clock = "2026-10-09T20:30:00+08:00"
            for _ in range(2):
                self.assertEqual(MODULE.weekly_run(source, binary, reports, clock), 0)
            versions = list(reports.iterdir())
            self.assertEqual(len(versions), 2)
            self.assertEqual(reports.stat().st_mode & 0o777, 0o700)
            for version in versions:
                json_report = json.loads((version / "review.json").read_text())
                markdown_report = json.loads((version / "review.md").read_text())
                self.assertEqual(json_report, markdown_report)
                self.assertEqual(json_report, json.loads((version / "evidence-manifest.json").read_text()))
                self.assertFalse(Path(json_report["snapshot"]).exists())
                self.assertEqual(json_report["observed_at"], clock)
                self.assertEqual(version.stat().st_mode & 0o777, 0o700)
                for artifact in version.iterdir():
                    self.assertEqual(artifact.stat().st_mode & 0o777, 0o600)
                status = json.loads((version / "run-status.json").read_text())
                self.assertEqual(status["completed_artifacts"], ["review.json", "evidence-manifest.json", "review.md"])
                self.assertEqual(status["exit_code"], 0)
            self.assertEqual(source.read_bytes(), before)

    def test_second_child_failure_preserves_json_and_original_nonzero(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "TEST_CODE_source.db"
            sqlite3.connect(source).close()
            binary = self.fake_cli(root, fail_markdown=True)
            reports = root / "reports"
            self.assertEqual(MODULE.weekly_run(source, binary, reports, "2026-10-09T20:30:00+08:00"), 23)
            version = next(reports.iterdir())
            self.assertTrue((version / "review.json").exists())
            self.assertFalse((version / "review.md").exists())
            status = json.loads((version / "run-status.json").read_text())
            self.assertEqual(status["completed_artifacts"], ["review.json", "evidence-manifest.json"])
            self.assertEqual(status["stage"], "markdown")
            self.assertEqual(status["exit_code"], 23)

    def test_missing_source_fails_before_child_and_preserves_failure_status(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "missing.db"
            reports = root / "reports"
            self.assertEqual(MODULE.weekly_run(source, root / "absent_cli", reports, "2026-10-09T20:30:00+08:00"), 2)
            status = json.loads((next(reports.iterdir()) / "run-status.json").read_text())
            self.assertEqual(status["stage"], "backup")
            self.assertEqual(status["completed_artifacts"], [])
            self.assertFalse(source.exists())

    def test_runtime_launcher_uses_only_injected_temporary_runtime_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            (root / "data").mkdir()
            (root / "bin" / "weekly-outcome-review.py").write_bytes(Path(SPEC.origin).read_bytes())
            binary = self.fake_cli(root)
            binary.rename(root / "bin" / "weekly_outcome_review")
            sqlite3.connect(root / "data" / "stock_analysis.db").close()
            self.binding_env(root)
            self.fake_assistant(root, runtime=True)
            launcher = Path(__file__).resolve().parents[1] / "run-weekly-outcome-review.sh"
            result = subprocess.run(["/bin/bash", str(launcher)],
                                    env={**os.environ, "STOCK_ANALYSIS_RUNTIME_ROOT": str(root)},
                                    check=False, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            versions = list((root / "reports" / "weekly-outcome-review").iterdir())
            self.assertEqual(len(versions), 1)
            source = json.loads((versions[0] / "review.json").read_text())["source_label"]
            self.assertEqual(source, str((root / "data" / "stock_analysis.db").resolve()))
            template = Path(__file__).resolve().parents[1] / "launchd" / "com.stockanalysis.weekly-outcome-review.plist"
            plist = plistlib.loads(template.read_bytes())
            self.assertEqual(plist["StartCalendarInterval"], {"Weekday": 5, "Hour": 20, "Minute": 30})
            self.assertFalse(plist["RunAtLoad"])
            self.assertFalse(plist["KeepAlive"])

    def test_registry_is_frozen_for_both_children_despite_original_change(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "TEST_CODE_registry.db"
            sqlite3.connect(source).close()
            registry = root / "signal_registry.toml"
            original = 'registry_version = "fixture-v0"\n'
            registry.write_text(original)
            binary = self.fake_cli(root, mutate_registry=registry)
            reports = root / "reports"
            self.assertEqual(MODULE.weekly_run(source, binary, reports,
                             "2026-10-09T20:30:00+08:00", registry), 0)
            version = next(reports.iterdir())
            json_report = json.loads((version / "review.json").read_text())
            markdown_report = json.loads((version / "review.md").read_text())
            self.assertEqual(json_report, markdown_report)
            self.assertEqual(json_report["registry_content"], original)
            self.assertEqual(registry.read_text(), "changed after first child")
            self.assertFalse(Path(json_report["registry_snapshot"]).exists())
            self.assertEqual(json_report, json.loads((version / "evidence-manifest.json").read_text()))

    def test_missing_explicit_registry_preserves_source_and_fails_before_child(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "TEST_CODE_missing_registry.db"
            sqlite3.connect(source).close()
            before = source.read_bytes()
            reports = root / "reports"
            self.assertEqual(MODULE.weekly_run(source, root / "absent_cli", reports,
                             "2026-10-09T20:30:00+08:00", root / "missing.toml"), 2)
            status = json.loads((next(reports.iterdir()) / "run-status.json").read_text())
            self.assertEqual(status["completed_artifacts"], [])
            self.assertEqual(source.read_bytes(), before)

    def test_registry_descriptor_cap_permissions_and_raw_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "registry.toml"
            source.write_bytes(b"# raw original bytes\n" + b" " * (128 * 1024 - 21))
            source.chmod(0o644)
            frozen_root = root / "frozen"
            frozen_root.mkdir()
            args = MODULE.registry_snapshot_args(source, frozen_root)
            frozen = Path(args[1])
            self.assertEqual(frozen.read_bytes(), source.read_bytes())
            self.assertEqual(hashlib.sha256(frozen.read_bytes()).hexdigest(), hashlib.sha256(source.read_bytes()).hexdigest())
            self.assertEqual(frozen.stat().st_mode & 0o777, 0o400)
            frozen.unlink()
            source.write_bytes(b" " * (128 * 1024 + 1))
            with self.assertRaisesRegex(ValueError, "128 KiB"):
                MODULE.registry_snapshot_args(source, frozen_root)
            self.assertFalse(frozen.exists())
            source.unlink()
            source.symlink_to(root / "missing")
            with self.assertRaises(OSError): MODULE.registry_snapshot_args(source, frozen_root)
            with self.assertRaises(ValueError): MODULE.registry_snapshot_args(root, frozen_root)
            self.assertEqual(MODULE.registry_snapshot_args(None, frozen_root), [])

    def test_registry_growth_and_path_replacement_are_rejected_before_freeze(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "registry.toml"
            frozen_root = root / "frozen"
            frozen_root.mkdir()
            real_read = os.read
            for change in ("growth", "replacement"):
                source.write_bytes(b"original")
                def changed_read(fd, count):
                    if change == "growth":
                        with source.open("ab") as stream: stream.write(b" " * (128 * 1024))
                    else:
                        source.rename(root / "old.toml")
                        source.write_bytes(b"original")
                    return real_read(fd, count)
                with patch.object(MODULE.os, "read", side_effect=changed_read):
                    with self.assertRaises(ValueError): MODULE.registry_snapshot_args(source, frozen_root)
                self.assertFalse((frozen_root / "signal_registry.toml").exists())

    def test_registry_regular_to_fifo_replacement_has_finite_child_deadline(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "registry.toml"
            source.write_bytes(b"original")
            frozen_root = root / "frozen"
            frozen_root.mkdir()
            code = """
import importlib.util, os, pathlib, sys
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('weekly',sys.argv[1])
m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
source=pathlib.Path(sys.argv[2]); frozen=pathlib.Path(sys.argv[3]); real_open=os.open
def replaced(path,flags,*args):
    if pathlib.Path(path)==source:
        source.unlink(); os.mkfifo(source,0o600)
    return real_open(path,flags,*args)
with patch.object(m.os,'open',side_effect=replaced):
    try: m.registry_snapshot_args(source,frozen)
    except ValueError: sys.exit(0)
sys.exit(1)
"""
            result = subprocess.run([os.sys.executable, "-B", "-c", code, SPEC.origin, str(source), str(frozen_root)], capture_output=True, text=True, timeout=3)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse((frozen_root / "signal_registry.toml").exists())

    def test_wal_backup_includes_committed_rows_and_preserves_source_inode_catalog_and_data(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "TEST_CODE_live.db"
            writer = sqlite3.connect(source)
            self.addCleanup(writer.close)
            writer.execute("PRAGMA journal_mode=WAL")
            writer.execute("PRAGMA wal_autocheckpoint=0")
            writer.execute("CREATE TABLE original(value TEXT)")
            writer.execute("INSERT INTO original VALUES ('committed original')")
            writer.commit()
            identity = (source.stat().st_dev, source.stat().st_ino)
            catalog = writer.execute("SELECT type,name,tbl_name,sql FROM sqlite_master").fetchall()
            rows = writer.execute("SELECT * FROM original").fetchall()
            main_before = source.read_bytes()
            wal_before = Path(str(source) + "-wal").read_bytes()
            backup = Path(directory) / "snapshot.db"
            MODULE.read_only_backup(source, backup)
            self.assertEqual((source.stat().st_dev, source.stat().st_ino), identity)
            self.assertEqual(source.read_bytes(), main_before)
            self.assertEqual(Path(str(source) + "-wal").read_bytes(), wal_before)
            self.assertEqual(writer.execute("SELECT type,name,tbl_name,sql FROM sqlite_master").fetchall(), catalog)
            self.assertEqual(writer.execute("SELECT * FROM original").fetchall(), rows)
            reader = sqlite3.connect(backup.as_uri() + "?mode=ro", uri=True)
            self.addCleanup(reader.close)
            self.assertEqual(reader.execute("SELECT * FROM original").fetchall(), rows)
            self.assertEqual(reader.execute("PRAGMA journal_mode").fetchone()[0], "delete")
            self.assertEqual(backup.stat().st_mode & 0o777, 0o400)
            self.assertFalse(Path(str(backup) + "-shm").exists())

    def test_missing_and_symlink_source_are_rejected_without_creating_a_database(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            missing = root / "missing.db"
            with self.assertRaises(FileNotFoundError):
                MODULE.read_only_backup(missing, root / "out.db")
            self.assertFalse(missing.exists())
            self.assertFalse((root / "out.db").exists())
            source = root / "source.db"
            sqlite3.connect(source).close()
            link = root / "link.db"
            link.symlink_to(source)
            with self.assertRaises(ValueError):
                MODULE.read_only_backup(link, root / "out.db")
            self.assertFalse((root / "out.db").exists())

    def test_existing_backup_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.db"
            sqlite3.connect(source).close()
            destination = root / "existing.db"
            destination.write_bytes(b"existing original")
            with self.assertRaises(ValueError):
                MODULE.read_only_backup(source, destination)
            self.assertEqual(destination.read_bytes(), b"existing original")


if __name__ == "__main__":
    unittest.main()
