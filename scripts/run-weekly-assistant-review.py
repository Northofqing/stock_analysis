#!/usr/bin/env python3
"""Run one strictly offline Phase A comparison after a frozen weekly report.

Clocks come from the matching report/manifest period, never from wall-clock now.
The child loads one frozen byte set and emits JSON + Markdown from one compare.
No model, provider, pricing, database or delivery configuration is forwarded.
"""
from __future__ import annotations

import argparse
from datetime import date, datetime, timedelta
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess

MAX_REPORT = 64 * 1024 * 1024
MAX_MANIFEST = 2 * 1024 * 1024
MAX_REGISTRY = 128 * 1024
MAX_OUTPUT = 8 * 1024 * 1024
CHILD_TIMEOUT_SECONDS = 30
OUTPUT_NAMES = ("comparison.json", "comparison.md", "status.json")


def file_version(value):
    return (value.st_dev, value.st_ino, value.st_size, value.st_uid,
            value.st_mode, value.st_nlink, value.st_mtime_ns, value.st_ctime_ns)


def read_private(path: Path, maximum: int) -> bytes:
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        before = os.fstat(fd)
        if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.geteuid()
                or before.st_mode & 0o077 or before.st_nlink != 1
                or before.st_size > maximum):
            raise ValueError("private_regular_bounded_file_required")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            value = stream.read(maximum + 1)
        if len(value) > maximum:
            raise ValueError("input_limit")
        after = os.fstat(fd)
        if file_version(before) != file_version(after) or file_version(after) != file_version(path.lstat()):
            raise ValueError("input_changed_during_read")
        return value
    finally:
        os.close(fd)


def write_private_new(path: Path, value: bytes, maximum: int = MAX_REPORT) -> None:
    if len(value) > maximum:
        raise ValueError("output_limit")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(value)
        stream.flush()
        os.fsync(stream.fileno())


def private_directory(path: Path) -> None:
    value = path.lstat()
    if not stat.S_ISDIR(value.st_mode) or value.st_uid != os.geteuid() or value.st_mode & 0o077:
        raise ValueError("private_owned_directory_required")


def create_output_directory(path: Path) -> None:
    private_directory(path.parent)
    try:
        path.mkdir(mode=0o700)
    except FileExistsError:
        private_directory(path)
    private_directory(path)
    if any((path / name).exists() or (path / name).is_symlink() for name in OUTPUT_NAMES) or (path / "inputs").exists() or (path / "inputs").is_symlink():
        raise ValueError("output_already_exists")


def report_clocks(report: dict, manifest: dict, as_of: str | None, completed_session: str | None) -> tuple[str, str]:
    if (not isinstance(report, dict) or not isinstance(manifest, dict)
            or report.get("report_version") != "H16-descriptive-weekly-v1"
            or manifest.get("schema_version") != "weekly-outcome-evidence-manifest-v1"
            or report.get("evidence_manifest") != manifest
            or report.get("period") != manifest.get("period")):
        raise ValueError("weekly_artifact_identity_mismatch")
    period = report.get("period")
    if not isinstance(period, dict):
        raise ValueError("weekly_period_required")
    frozen_as_of = period.get("observed_at")
    frozen_completed = period.get("latest_completed_session")
    if not isinstance(frozen_as_of, str) or not isinstance(frozen_completed, str):
        raise ValueError("weekly_period_clocks_required")
    observation = datetime.fromisoformat(frozen_as_of)
    completed = date.fromisoformat(frozen_completed)
    if observation.utcoffset() != timedelta(hours=8) or completed > observation.date():
        raise ValueError("weekly_period_clock_invalid")
    if as_of is not None and datetime.fromisoformat(as_of) != observation:
        raise ValueError("as_of_mismatch")
    if as_of is not None and datetime.fromisoformat(as_of).utcoffset() != timedelta(hours=8):
        raise ValueError("Shanghai_as_of_required")
    if completed_session is not None and date.fromisoformat(completed_session) != completed:
        raise ValueError("completed_session_mismatch")
    # Rust FrozenPack verifies actual trading-day/calendar and PIT scope. This
    # wrapper only carries the already-recorded weekly clock without inventing it.
    return frozen_as_of, frozen_completed


def child_environment() -> dict[str, str]:
    return {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C.UTF-8", "TZ": "Asia/Shanghai"}


def checked_binary(path: Path) -> Path:
    metadata = path.lstat()
    if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid()
            or metadata.st_mode & 0o022 or not os.access(path, os.X_OK)):
        raise ValueError("owned_regular_executable_required")
    return path.resolve(strict=True)


def collect_artifacts(directory: Path, status: dict) -> dict[str, bytes]:
    artifacts = {}
    for name in OUTPUT_NAMES[:2]:
        path = directory / name
        if path.exists() or path.is_symlink():
            value = read_private(path, MAX_OUTPUT)
            artifacts[name] = value
            status["completed_artifacts"].append(name)
            status["artifact_sha256"][name] = hashlib.sha256(value).hexdigest()
    return artifacts


def verify_comparison(value: bytes, hashes: dict, as_of: str, completed: str) -> dict:
    comparison = json.loads(value)
    if not isinstance(comparison, dict) or not isinstance(comparison.get("period"), dict) or not isinstance(comparison.get("run_reservations"), dict):
        raise ValueError("comparison_schema_invalid")
    if (not isinstance(comparison, dict)
            or comparison.get("schema_version") != "assistant-phase-a-comparison-v1"
            or comparison.get("mode") != "read_only_weekly_review"
            or comparison.get("report_sha256") != hashes["report"]
            or comparison.get("manifest_sha256") != hashes["manifest"]
            or not isinstance(comparison.get("as_of"), str)
            or datetime.fromisoformat(comparison["as_of"]) != datetime.fromisoformat(as_of)
            or datetime.fromisoformat(comparison["as_of"]).utcoffset() != timedelta(hours=8)
            or comparison.get("period", {}).get("latest_completed_session") != completed):
        raise ValueError("comparison_artifact_identity_mismatch")
    arms = comparison.get("arms")
    if (not isinstance(arms, list) or len(arms) != 3
            or [arm.get("arm") if isinstance(arm, dict) else None for arm in arms] != ["template", "model_without_outcomes", "model_with_outcomes"]
            or any(not isinstance(arm, dict) or arm.get("mode") not in ("deterministic_template", "degraded_template")
                   or arm.get("model_receipt") is not None for arm in arms)
            or type(comparison["run_reservations"].get("attempt_slots_issued")) is not int
            or type(comparison["run_reservations"].get("retained_maximum_micro_cny")) is not int
            or comparison.get("run_reservations", {}).get("attempt_slots_issued") != 0
            or comparison.get("run_reservations", {}).get("retained_maximum_micro_cny") != 0):
        raise ValueError("offline_zero_model_attempts_required")
    return comparison


def run_review(report: Path, manifest: Path, output_dir: Path, assistant_binary: Path,
               registry: Path | None = None, as_of: str | None = None,
               completed_session: str | None = None) -> int:
    create_output_directory(output_dir)
    # This exclusive directory is also the per-output-run admission guard.
    # A concurrent caller must fail before it can launch a child or publish a
    # competing status over the in-flight run.
    inputs = output_dir / "inputs"
    inputs.mkdir(mode=0o700)
    status = {"schema_version": "weekly-assistant-review-run-v1", "status": "failed",
              "stage": "input", "mode": "strict_offline", "child_calls": 0,
              "model_calls_verified": None, "completed_artifacts": [],
              "artifact_sha256": {}, "exit_code": 2}
    result = 2
    try:
        report_bytes = read_private(report, MAX_REPORT)
        manifest_bytes = read_private(manifest, MAX_MANIFEST)
        registry_bytes = read_private(registry, MAX_REGISTRY) if registry else None
        report_value, manifest_value = json.loads(report_bytes), json.loads(manifest_bytes)
        clock, completed = report_clocks(report_value, manifest_value, as_of, completed_session)
        hashes = {"report": hashlib.sha256(report_bytes).hexdigest(),
                  "manifest": hashlib.sha256(manifest_bytes).hexdigest(),
                  "registry": hashlib.sha256(registry_bytes).hexdigest() if registry_bytes is not None else None}
        status.update({"input_sha256": hashes, "as_of": clock, "completed_session": completed})
        binary = checked_binary(assistant_binary)
        write_private_new(inputs / "report.json", report_bytes)
        write_private_new(inputs / "manifest.json", manifest_bytes, MAX_MANIFEST)
        (inputs / "report.json").chmod(0o400)
        (inputs / "manifest.json").chmod(0o400)
        command = [str(binary), "--report", str((inputs / "report.json").resolve()),
                   "--manifest", str((inputs / "manifest.json").resolve()),
                   "--as-of", clock, "--completed-session", completed,
                   "--format", "json", "--output", str((output_dir / "comparison.json").resolve()),
                   "--markdown-output", str((output_dir / "comparison.md").resolve())]
        if registry_bytes is not None:
            write_private_new(inputs / "signal_registry.toml", registry_bytes, MAX_REGISTRY)
            (inputs / "signal_registry.toml").chmod(0o400)
            command.extend(["--registry", str((inputs / "signal_registry.toml").resolve())])
        status["stage"] = "compare_once"
        status["child_calls"] = 1
        child = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, env=child_environment(),
                               check=False, timeout=CHILD_TIMEOUT_SECONDS)
        status["child_exit_code"] = child.returncode
        status["stage"] = "verify_outputs"
        artifacts = collect_artifacts(output_dir, status)
        result = child.returncode if child.returncode > 0 else 2 if child.returncode < 0 else 0
        if child.returncode:
            status["error_code"] = "assistant_child_failed"
        else:
            if set(artifacts) != set(OUTPUT_NAMES[:2]):
                raise ValueError("assistant_child_missing_outputs")
            comparison = verify_comparison(artifacts["comparison.json"], hashes, clock, completed)
            markdown = artifacts["comparison.md"].decode("utf-8")
            if hashes["report"] not in markdown or hashes["manifest"] not in markdown:
                raise ValueError("markdown_input_identity_missing")
            status["model_calls_verified"] = 0
            status["comparison_status"] = comparison.get("status")
            status["status"] = "complete"
            status["stage"] = "complete"
    except subprocess.TimeoutExpired:
        status["error_code"] = "assistant_child_timeout"
        result = 2
    except (OSError, ValueError, TypeError, KeyError):
        status["error_code"] = "assistant_input_or_output_invalid"
        result = 2
    # A failed second publication keeps the completed first artifact. It is
    # evidence to inspect, never an excuse to rerun the model/comparison.
    if status["status"] != "complete":
        try:
            if not status["completed_artifacts"]:
                collect_artifacts(output_dir, status)
        except (OSError, ValueError):
            status["error_code"] = "assistant_output_integrity_failed"
        if status["completed_artifacts"]:
            status["status"] = "partial"
    status["exit_code"] = result
    write_private_new(output_dir / "status.json", (json.dumps(status, ensure_ascii=False, indent=2) + "\n").encode(), MAX_OUTPUT)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--registry", type=Path)
    parser.add_argument("--as-of")
    parser.add_argument("--completed-session")
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--assistant-binary", type=Path, required=True)
    args = parser.parse_args()
    try:
        return run_review(**vars(args))
    except (OSError, ValueError):
        print("weekly_assistant_review: output_preflight_or_status_failed", file=__import__("sys").stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
