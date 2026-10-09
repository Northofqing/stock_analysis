#!/usr/bin/env python3
"""Read a consistent SQLite backup, then run the local H16 review CLI.

The source is opened mode=ro/query_only. Only the private backup is normalized;
no production DatabaseManager, schema migration, provider or delivery is used.
"""
from __future__ import annotations
import argparse
from datetime import datetime, timedelta, timezone
import json
import hashlib
import shlex
import os
from pathlib import Path
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
import uuid

SHANGHAI = timezone(timedelta(hours=8))
BINDING_ENV = "PAPER_LEDGER_ACCOUNT_BINDING"


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


def read_binding_env(path: Path | None) -> str | None:
    """Extract one literal JSON assignment; never source or forward the .env."""
    if path is None:
        return None
    source = path.parent.resolve(strict=True) / path.name
    descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        before = os.fstat(descriptor)
        if (not stat.S_ISREG(before.st_mode) or before.st_mode & 0o077
                or before.st_uid != os.getuid() or before.st_size > 128 * 1024):
            raise ValueError("binding env must be an owner-private regular file <=128 KiB")
        data = bytearray()
        while len(data) <= 128 * 1024:
            chunk = os.read(descriptor, 128 * 1024 + 1 - len(data))
            if not chunk:
                break
            data.extend(chunk)
        after = os.fstat(descriptor)
        named = source.lstat()
        signature = lambda info: (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        if (len(data) > 128 * 1024 or signature(before) != signature(after)
                or signature(before) != signature(named) or len(data) != before.st_size):
            raise ValueError("binding env changed during read")
    finally:
        os.close(descriptor)
    assignments = []
    for line in data.decode("utf-8").splitlines():
        line = line.strip()
        if line.startswith("export "):
            line = line[7:].lstrip()
        key, separator, value = line.partition("=")
        if separator and key.strip() == BINDING_ENV:
            assignments.append(value.strip())
    if len(assignments) != 1:
        raise ValueError("binding env must contain exactly one paper account binding")
    literal = assignments[0]
    if literal.startswith(("'", '"')):
        tokens = shlex.split(literal, comments=False, posix=True)
        if len(tokens) != 1:
            raise ValueError("paper binding must be one literal JSON assignment")
        literal = tokens[0]
    def unique_fields(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate paper binding JSON field")
            result[key] = value
        return result
    binding = json.loads(literal, object_pairs_hook=unique_fields)
    if (not isinstance(binding, dict) or set(binding) != {"account_id", "epoch_id", "manifest_hash"}
            or not all(isinstance(binding[key], str) and binding[key].strip() for key in binding)
            or len(binding["manifest_hash"]) != 64
            or any(ch not in "0123456789abcdef" for ch in binding["manifest_hash"])):
        raise ValueError("invalid paper account binding")
    return json.dumps(binding, ensure_ascii=False, separators=(",", ":"))


def child_environment(binding: str | None) -> dict[str, str]:
    environment = dict(os.environ)
    environment.pop(BINDING_ENV, None)  # No inherited binding or LegacyRaw fallback.
    if binding is not None:
        environment[BINDING_ENV] = binding
    return environment


def snapshot_source_args(backup: Path, origin: dict, directory: Path, binding: str | None) -> list[str]:
    metadata = backup.stat()
    digest = hashlib.sha256()
    with backup.open("rb") as stream:
        for chunk in iter(lambda: stream.read(64 * 1024), b""):
            digest.update(chunk)
    manifest = directory / "snapshot-source.json"
    private_json(manifest, {"schema_version": 1, "original_database": origin,
        "snapshot_database": {"path": str(backup.resolve(strict=True)), "device": metadata.st_dev,
                              "inode": metadata.st_ino},
        "snapshot_sha256": digest.hexdigest(),
        "paper_binding_sha256": hashlib.sha256(binding.encode()).hexdigest() if binding is not None else None})
    manifest.chmod(0o400)
    return ["--snapshot-source-manifest", str(manifest)]


def weekly_run(source: Path, binary: Path, output_root: Path, observed_at: str | None, registry: Path | None = None, binding_env_file: Path | None = None,
               assistant_binary: Path | None = None, assistant_script: Path | None = None) -> int:
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
              "stage": "backup", "exit_code": 2,
              "assistant": {"status": "not_configured", "invocations": 0}}
    result = 2
    try:
        # JSON and Markdown use exactly the same logical backup and fixed clock.
        with tempfile.TemporaryDirectory(prefix="stock-analysis-weekly-") as directory:
            os.chmod(directory, 0o700)
            binding = read_binding_env(binding_env_file)
            backup = Path(directory) / "snapshot.db"
            origin = read_only_backup(source, backup)
            registry_args = registry_snapshot_args(registry, Path(directory))
            base = [str(binary.resolve(strict=True)), "--database", str(backup),
                    "--from", start, "--to", end, "--observed-at", clock,
                    "--source-label", str(source.resolve(strict=True)), "--temporary-snapshot",
                    "--evidence-manifest", str(run / "evidence-manifest.json")] + registry_args + snapshot_source_args(backup, origin, Path(directory), binding)
            for format_name, file_name in (("json", "review.json"), ("markdown", "review.md")):
                status["stage"] = format_name
                output = run / file_name
                result = subprocess.run(base + ["--format", format_name, "--output", str(output)],
                                        check=False, env=child_environment(binding), timeout=180).returncode
                if result:
                    break
                # Keep even a completed first artifact if the later program fails.
                os.chmod(output, 0o600)
                status["completed_artifacts"].append(file_name)
                manifest = run / "evidence-manifest.json"
                if not manifest.is_file():
                    raise RuntimeError("review child succeeded without required evidence manifest")
                if "evidence-manifest.json" not in status["completed_artifacts"]:
                    os.chmod(manifest, 0o600)
                    status["completed_artifacts"].append("evidence-manifest.json")
            if not result:
                status["stage"] = "complete"
                if assistant_binary is not None:
                    status["stage"] = "assistant"
                    status["assistant"] = {"status": "preparing", "invocations": 0}
                    result = assistant_once(run, registry_args, assistant_binary, assistant_script,
                                            status["assistant"], child_environment(binding))
                    status["stage"] = "complete" if not result else "complete_weekly_assistant_failed"
    except (OSError, ValueError, sqlite3.Error, TimeoutError, RuntimeError, subprocess.TimeoutExpired) as error:
        status["error"] = str(error)
        if status["stage"] == "assistant":
            status["assistant"]["status"] = "failed"
            status["assistant"]["error"] = str(error)
            status["stage"] = "complete_weekly_assistant_failed"
        result = 2
    for name in ("comparison.json", "comparison.md", "status.json"):
        artifact = run / "assistant" / name
        if artifact.is_file() and not artifact.is_symlink():
            os.chmod(artifact, 0o600)
            status["completed_artifacts"].append("assistant/" + name)
    status["exit_code"] = result
    private_json(run / "run-status.json", status)
    print(str(run))
    return result


def assistant_once(run: Path, registry_args: list[str], binary: Path, script: Path | None,
                   status: dict, environment: dict[str, str]) -> int:
    """One offline comparison after both weekly artifacts; never restart it."""
    report = run / "review.json"
    manifest = run / "evidence-manifest.json"
    report_value = json.loads(report.read_text())
    manifest_value = json.loads(manifest.read_text())
    period = report_value.get("period")
    if not isinstance(period, dict) or manifest_value.get("period") != period:
        raise ValueError("assistant needs exact matching weekly report/manifest period")
    as_of = period.get("observed_at")
    completed = period.get("latest_completed_session")
    if not isinstance(as_of, str) or not isinstance(completed, str):
        raise ValueError("assistant needs original observed_at/latest_completed_session fields")
    script = script or Path(__file__).with_name("run-weekly-assistant-review.py")
    for path, name in ((script, "assistant script"), (binary, "assistant binary")):
        if not stat.S_ISREG(path.lstat().st_mode):
            raise ValueError(name + " must be a regular file, not a symlink")
    command = [sys.executable, str(script.resolve(strict=True)), "--report", str(report),
               "--manifest", str(manifest), "--as-of", as_of, "--completed-session", completed,
               "--output-dir", str(run / "assistant"), "--assistant-binary", str(binary.resolve(strict=True))] + registry_args
    status["invocations"] = 1
    result = subprocess.run(command, check=False, env=environment, timeout=180).returncode
    status["exit_code"] = result
    status["status"] = "complete" if result == 0 else "failed"
    if result == 0 and any(not (run / "assistant" / name).is_file()
                           or (run / "assistant" / name).is_symlink()
                           for name in ("comparison.json", "comparison.md", "status.json")):
        raise RuntimeError("assistant child succeeded without all required comparison artifacts")
    return result


def registry_snapshot_args(registry: Path | None, directory: Path) -> list[str]:
    if registry is None:
        return []  # CLI's embedded registry is independent of launchd cwd.
    # Resolve only the parent: a substituted leaf symlink must never be followed.
    source = registry.parent.resolve(strict=True) / registry.name
    descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise ValueError("registry must be a bounded regular TOML file (<=128 KiB)")
        data = bytearray()
        while len(data) <= 128 * 1024:
            chunk = os.read(descriptor, 128 * 1024 + 1 - len(data))
            if not chunk:
                break
            data.extend(chunk)
        if len(data) > 128 * 1024:
            raise ValueError("registry exceeds 128 KiB bound")
        def signature(info):
            return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        after = os.fstat(descriptor)
        named = registry.lstat()
        pinned = source.lstat()
        if (not stat.S_ISREG(named.st_mode) or not stat.S_ISREG(pinned.st_mode)
                or signature(before) != signature(after) or signature(before) != signature(named)
                or signature(before) != signature(pinned) or len(data) != before.st_size):
            raise ValueError("registry changed during read")
    finally:
        os.close(descriptor)
    frozen = directory / "signal_registry.toml"
    descriptor = os.open(frozen, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o400)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(data)
    return ["--registry", str(frozen)]


def read_only_backup(source: Path, destination: Path) -> dict:
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
    return {"path": str(source), "device": before.st_dev, "inode": before.st_ino}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--binding-env-file", type=Path, help="Owner-private runtime .env; extract only the literal paper binding")
    parser.add_argument("--assistant-binary", type=Path, help="Run one offline comparison after both weekly formats succeed")
    parser.add_argument("--assistant-script", type=Path, help="Regular helper script; defaults to sibling run-weekly-assistant-review.py")
    parser.add_argument("--registry", type=Path, help="Explicit descriptive registry; frozen once per run; defaults to CLI embedded config")
    parser.add_argument("--evidence-manifest", type=Path, help="Explicit sidecar path for single-format mode")
    parser.add_argument("--from", dest="start")
    parser.add_argument("--to")
    parser.add_argument("--observed-at")
    parser.add_argument("--format", choices=("markdown", "json"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--weekly-output-root", type=Path,
                        help="Generate JSON and Markdown for the current Shanghai calendar week in a new private version directory")
    args = parser.parse_args()
    if args.assistant_script and not args.assistant_binary:
        parser.error("assistant-script requires assistant-binary")
    if args.assistant_binary and not args.weekly_output_root:
        parser.error("assistant hook requires weekly-output-root with both formats")
    if args.weekly_output_root:
        if args.start or args.to or args.format or args.output or args.evidence_manifest:
            parser.error("weekly-output-root computes its own week and writes both formats; from/to/format/output are not accepted")
        return weekly_run(args.database, args.binary, args.weekly_output_root, args.observed_at, args.registry, args.binding_env_file, args.assistant_binary, args.assistant_script)
    if not args.start or not args.to:
        parser.error("explicit from/to are required without weekly-output-root")
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="stock-analysis-weekly-") as directory:
        os.chmod(directory, 0o700)
        binding = read_binding_env(args.binding_env_file)
        backup = Path(directory) / "snapshot.db"
        origin = read_only_backup(args.database, backup)
        registry_args = registry_snapshot_args(args.registry, Path(directory))
        command = [str(binary), "--database", str(backup), "--from", args.start,
                   "--to", args.to, "--format", args.format or "markdown",
                   "--source-label", str(args.database.resolve(strict=True)),
                   "--temporary-snapshot"] + registry_args + snapshot_source_args(backup, origin, Path(directory), binding)
        if args.evidence_manifest:
            command.extend(["--evidence-manifest", str(args.evidence_manifest)])
        if args.observed_at:
            command.extend(["--observed-at", args.observed_at])
        if args.output:
            command.extend(["--output", str(args.output)])
        return subprocess.run(command, check=False, env=child_environment(binding), timeout=180).returncode


if __name__ == "__main__":
    raise SystemExit(main())
