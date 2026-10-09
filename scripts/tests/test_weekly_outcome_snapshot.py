import importlib.util
import json
import os
from pathlib import Path
import plistlib
import sqlite3
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "weekly_outcome_review", Path(__file__).resolve().parents[1] / "weekly-outcome-review.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReadOnlyWeeklySnapshotTests(unittest.TestCase):
    def fake_cli(self, root, fail_markdown=False, mutate_registry=None):
        binary = root / "fake_cli.py"
        binary.write_text("""#!/usr/bin/env python3
import hashlib, json, os, pathlib, sys
args=sys.argv
def value(flag): return args[args.index(flag)+1]
if value('--format')=='markdown' and FAIL_MARKDOWN: sys.exit(23)
report={'observed_at':value('--observed-at'),'snapshot':value('--database'),
        'sha256':hashlib.sha256(pathlib.Path(value('--database')).read_bytes()).hexdigest(),
        'source_label':value('--source-label')}
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
fd=os.open(value('--output'),os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(fd,'w') as stream: json.dump(report,stream)
""".replace("FAIL_MARKDOWN", repr(fail_markdown)).replace("MUTATE_REGISTRY", repr(str(mutate_registry) if mutate_registry else None)))
        binary.chmod(0o700)
        return binary

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
