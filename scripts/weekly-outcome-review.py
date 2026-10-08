#!/usr/bin/env python3
"""Read a consistent SQLite backup, then run the local H16 review CLI.

The source is opened mode=ro/query_only. Only the private backup is normalized;
no production DatabaseManager, schema migration, provider or delivery is used.
"""
from __future__ import annotations
import argparse
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import sqlite3
import stat
import subprocess
import tempfile
import time
import uuid

SHANGHAI = timezone(timedelta(hours=8))


def weekly_scope(observed_at: str | None):
    now = datetime.fromisoformat(observed_at) if observed_at else datetime.now(SHANGHAI)
    if now.utcoffset() != timedelta(hours=8):
        raise ValueError("observed-at must use Shanghai +08:00")
    start = now.date() - timedelta(days=now.weekday())
    return start.isoformat(), (start + timedelta(days=6)).isoformat(), now.isoformat()


def private_json(path: Path, value) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def weekly_run(source: Path, binary: Path, output_root: Path, observed_at: str | None) -> int:
    start, end, clock = weekly_scope(observed_at)
    output_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    if output_root.is_symlink() or not output_root.is_dir():
        raise ValueError("weekly output root must be a real directory")
    os.chmod(output_root, 0o700)
    stamp = datetime.fromisoformat(clock).strftime("%Y%m%dT%H%M%S%f")
    run = output_root / f"{start}_{end}__{stamp}__{uuid.uuid4().hex}"
    run.mkdir(mode=0o700)
    status = {"requested_from": start, "requested_to": end, "observed_at": clock,
              "original_source_label": str(source.absolute()), "completed_artifacts": [],
              "stage": "backup", "exit_code": 2}
    result = 2
    try:
        # JSON and Markdown use exactly the same logical backup and fixed clock.
        with tempfile.TemporaryDirectory(prefix="stock-analysis-weekly-") as directory:
            os.chmod(directory, 0o700)
            backup = Path(directory) / "snapshot.db"
            read_only_backup(source, backup)
            base = [str(binary.resolve(strict=True)), "--database", str(backup),
                    "--from", start, "--to", end, "--observed-at", clock,
                    "--source-label", str(source.resolve(strict=True)), "--temporary-snapshot"]
            for format_name, file_name in (("json", "review.json"), ("markdown", "review.md")):
                status["stage"] = format_name
                output = run / file_name
                result = subprocess.run(base + ["--format", format_name, "--output", str(output)],
                                        check=False).returncode
                if result:
                    break
                # Keep even a completed first artifact if the later program fails.
                os.chmod(output, 0o600)
                status["completed_artifacts"].append(file_name)
            if not result:
                status["stage"] = "complete"
    except (OSError, ValueError, sqlite3.Error, TimeoutError, RuntimeError) as error:
        status["error"] = str(error)
        result = 2
    status["exit_code"] = result
    private_json(run / "run-status.json", status)
    print(str(run))
    return result


def read_only_backup(source: Path, destination: Path) -> None:
    before = source.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("source must be an existing regular file, not a symlink")
    source = source.resolve(strict=True)
    if destination.exists() or destination.is_symlink():
        raise ValueError("backup destination must be new")
    deadline = time.monotonic() + 30

    def progress(_status, _remaining, _total):
        if time.monotonic() > deadline:
            raise TimeoutError("read-only SQLite backup exceeded 30 seconds")

    reader = sqlite3.connect(source.as_uri() + "?mode=ro", uri=True, timeout=5)
    try:
        reader.execute("PRAGMA query_only=ON")
        reader.execute("BEGIN")
        reader.execute("SELECT count(*) FROM sqlite_master").fetchone()
        writer = sqlite3.connect(destination)
        try:
            reader.backup(writer, pages=256, progress=progress, sleep=0.05)
            mode = writer.execute("PRAGMA journal_mode=DELETE").fetchone()[0]
            if mode.lower() != "delete":
                raise RuntimeError("private backup journal mode did not normalize")
        finally:
            writer.close()
    finally:
        reader.close()
    after = source.stat()
    if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
        raise RuntimeError("source database identity changed during backup")
    wal = Path(str(destination) + "-wal")
    if wal.exists() and wal.stat().st_size:
        raise RuntimeError("private backup retained a nonempty SQLite WAL")
    # Some bundled SQLite versions leave an orphan SHM after switching the
    # private backup to DELETE. Reopen read-only to verify the persisted mode;
    # with all backup connections closed and no WAL frames, SHM is not data.
    checker = sqlite3.connect(destination.resolve().as_uri() + "?mode=ro", uri=True)
    try:
        checker.execute("PRAGMA query_only=ON")
        if checker.execute("PRAGMA journal_mode").fetchone()[0].lower() != "delete":
            raise RuntimeError("private backup persisted journal mode is not DELETE")
    finally:
        checker.close()
    shm = Path(str(destination) + "-shm")
    if shm.exists():
        shm.unlink()
    os.chmod(destination, 0o400)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--from", dest="start")
    parser.add_argument("--to")
    parser.add_argument("--observed-at")
    parser.add_argument("--format", choices=("markdown", "json"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--weekly-output-root", type=Path,
                        help="Generate JSON and Markdown for the current Shanghai calendar week in a new private version directory")
    args = parser.parse_args()
    if args.weekly_output_root:
        if args.start or args.to or args.format or args.output:
            parser.error("weekly-output-root computes its own week and writes both formats; from/to/format/output are not accepted")
        return weekly_run(args.database, args.binary, args.weekly_output_root, args.observed_at)
    if not args.start or not args.to:
        parser.error("explicit from/to are required without weekly-output-root")
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="stock-analysis-weekly-") as directory:
        os.chmod(directory, 0o700)
        backup = Path(directory) / "snapshot.db"
        read_only_backup(args.database, backup)
        command = [str(binary), "--database", str(backup), "--from", args.start,
                   "--to", args.to, "--format", args.format or "markdown",
                   "--source-label", str(args.database.resolve(strict=True)),
                   "--temporary-snapshot"]
        if args.observed_at:
            command.extend(["--observed-at", args.observed_at])
        if args.output:
            command.extend(["--output", str(args.output)])
        return subprocess.run(command, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
