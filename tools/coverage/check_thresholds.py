#!/usr/bin/env python3
"""Enforce BR-250/BR-252 release and PR line-coverage policies.

The default remains the fixed Gate D release policy for backward compatibility.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass, replace
from typing import Any


CORE_PREFIXES = (
    "src/auth/",
    "src/bin/monitor/",
    "src/data_gateway/",
    "src/durable_delivery/",
    "src/risk/",
    "src/trading/",
    "src/database/",
    "src/data_provider/",
    "src/decision/",
    "src/market_analyzer/",
    "src/monitor/",
    "src/pipeline/",
    "src/portfolio/",
    "src/selection/",
    "src/event/",
)

CONTRACT_PATH = "config/design_contracts.toml"


class InputError(Exception):
    """Coverage evidence cannot be verified (CLI exit 2)."""


@dataclass(frozen=True)
class CoverageTotals:
    global_covered: int
    global_count: int
    core_covered: int
    core_count: int
    core_file_count: int
    source_files: frozenset[str]


@dataclass(frozen=True)
class CoverageContract:
    schema: int
    source_sha: str
    bootstrap_approved: bool
    bootstrap_rule: str
    global_covered: int
    global_count: int
    core_covered: int
    core_count: int
    core_file_count: int
    pr_core_patch_min: int
    pr_other_patch_min: int
    release_global_min: int
    release_core_min: int
    rustc_release: str
    rustc_commit: str
    llvm_version: str
    cargo_llvm_cov_version: str
    reviewed_no_region: tuple[tuple[str, str], ...]


def percentage(covered: int, count: int) -> float:
    return 100.0 if count == 0 else covered * 100.0 / count


def repository_relative_path(filename: str) -> str:
    """Normalize llvm-cov paths from local and repeated GitHub workspaces."""
    normalized = filename.replace("\\", "/")
    try:
        return (
            pathlib.Path(normalized)
            .resolve()
            .relative_to(pathlib.Path.cwd().resolve())
            .as_posix()
        )
    except (OSError, ValueError):
        marker = "/stock_analysis/"
        if marker in normalized:
            return normalized.rsplit(marker, 1)[-1]
        return normalized.lstrip("./")


def strict_repository_relative_path(filename: str, repository: pathlib.Path) -> str:
    normalized = filename.replace("\\", "/")
    candidate = pathlib.Path(normalized)
    try:
        if candidate.is_absolute():
            relative = candidate.resolve().relative_to(repository.resolve())
        else:
            if ".." in candidate.parts:
                raise ValueError("path contains parent traversal")
            relative = candidate
    except (OSError, ValueError) as exc:
        raise InputError(f"coverage source path escapes repository: {filename}: {exc}") from exc
    result = relative.as_posix().lstrip("./")
    if not result or result.startswith("../"):
        raise InputError(f"invalid coverage source path: {filename}")
    return result


def checked_counter(value: Any, field: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise InputError(f"{field} must be a non-negative integer")
    return value


def checked_ratio(covered: Any, count: Any, field: str) -> tuple[int, int]:
    checked_covered = checked_counter(covered, f"{field}.covered")
    checked_count = checked_counter(count, f"{field}.count")
    if checked_count == 0 or checked_covered > checked_count:
        raise InputError(f"{field} must satisfy 0 <= covered <= count and count > 0")
    return checked_covered, checked_count


def load_report(report: pathlib.Path, repository: pathlib.Path | None) -> CoverageTotals:
    try:
        payload = json.loads(report.read_text(encoding="utf-8"))
        run = payload["data"][0]
        raw_totals = run["totals"]["lines"]
        files = run["files"]
        if not isinstance(files, list):
            raise TypeError("files is not a list")
        global_covered, global_count = checked_ratio(
            raw_totals["covered"], raw_totals["count"], "global lines"
        )
    except (OSError, json.JSONDecodeError, KeyError, IndexError, TypeError) as exc:
        raise InputError(f"coverage report is missing or invalid: {exc}") from exc

    core_count = 0
    core_covered = 0
    matched: list[str] = []
    source_files: set[str] = set()
    seen_files: set[str] = set()
    for index, item in enumerate(files):
        if not isinstance(item, dict):
            raise InputError(f"coverage file entry {index} is invalid")
        filename = str(item.get("filename", ""))
        relative = (
            strict_repository_relative_path(filename, repository)
            if repository is not None
            else repository_relative_path(filename)
        )
        if relative in seen_files:
            raise InputError(f"duplicate coverage file entry: {relative}")
        seen_files.add(relative)
        if relative.startswith("src/") and relative.endswith(".rs"):
            source_files.add(relative)
        if not relative.startswith(CORE_PREFIXES):
            continue
        try:
            lines = item["summary"]["lines"]
            covered, count = checked_ratio(
                lines["covered"], lines["count"], f"file {relative} lines"
            )
        except (KeyError, TypeError) as exc:
            raise InputError(f"coverage file summary is invalid for {relative}: {exc}") from exc
        core_count += count
        core_covered += covered
        matched.append(relative)

    if not matched or core_count == 0:
        raise InputError("coverage report contains no registered core-module lines")
    return CoverageTotals(
        global_covered=global_covered,
        global_count=global_count,
        core_covered=core_covered,
        core_count=core_count,
        core_file_count=len(matched),
        source_files=frozenset(source_files),
    )


def print_totals(totals: CoverageTotals, global_min: float, core_min: float) -> None:
    print(
        f"global line coverage: {totals.global_covered}/{totals.global_count} = "
        f"{percentage(totals.global_covered, totals.global_count):.2f}% "
        f"(required {global_min:.2f}%)"
    )
    print(
        f"core line coverage: {totals.core_covered}/{totals.core_count} = "
        f"{percentage(totals.core_covered, totals.core_count):.2f}% "
        f"(required {core_min:.2f}%, {totals.core_file_count} files)"
    )


def verify_report_provenance(
    report: pathlib.Path,
    repository: pathlib.Path,
    contract: CoverageContract,
) -> None:
    try:
        payload = json.loads(report.read_text(encoding="utf-8"))
        report_type = payload["type"]
        report_version = payload["version"]
        cargo_metadata = payload["cargo_llvm_cov"]
        cargo_version = cargo_metadata["version"]
        manifest = pathlib.Path(cargo_metadata["manifest_path"]).resolve()
    except (OSError, json.JSONDecodeError, KeyError, TypeError) as exc:
        raise InputError(f"coverage report provenance is missing or invalid: {exc}") from exc
    if report_type != "llvm.coverage.json.export" or report_version != "3.1.0":
        raise InputError(
            "coverage report provenance has unsupported llvm coverage schema"
        )
    if cargo_version != contract.cargo_llvm_cov_version:
        raise InputError("coverage report provenance cargo-llvm-cov version mismatch")
    if manifest != (repository / "Cargo.toml").resolve():
        raise InputError("coverage report provenance manifest path mismatch")


def run_git(repository: pathlib.Path, args: list[str], *, allow_failure: bool = False) -> bytes:
    try:
        result = subprocess.run(
            ["git", *args],
            cwd=repository,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except OSError as exc:
        raise InputError(f"cannot execute git: {exc}") from exc
    if result.returncode != 0 and not allow_failure:
        diagnostic = result.stderr.decode("utf-8", errors="replace").strip()
        raise InputError(f"git {' '.join(args)} failed: {diagnostic}")
    return result.stdout if result.returncode == 0 else b""


def git_succeeds(repository: pathlib.Path, args: list[str]) -> bool:
    try:
        result = subprocess.run(
            ["git", *args],
            cwd=repository,
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
    except OSError as exc:
        raise InputError(f"cannot execute git: {exc}") from exc
    if result.returncode not in (0, 1):
        diagnostic = result.stderr.decode("utf-8", errors="replace").strip()
        raise InputError(f"git {' '.join(args)} failed: {diagnostic}")
    return result.returncode == 0


def discover_repository() -> pathlib.Path:
    output = run_git(pathlib.Path.cwd(), ["rev-parse", "--show-toplevel"])
    try:
        return pathlib.Path(output.decode("utf-8").strip()).resolve()
    except (OSError, UnicodeDecodeError) as exc:
        raise InputError(f"cannot resolve repository root: {exc}") from exc


def parse_contract(document: bytes, source: str) -> CoverageContract | None:
    try:
        payload = tomllib.loads(document.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
        raise InputError(f"invalid coverage contract in {source}: {exc}") from exc
    raw = payload.get("coverage")
    if raw is None:
        return None
    if not isinstance(raw, dict):
        raise InputError(f"coverage contract in {source} must be a table")

    def integer(name: str, *, positive: bool = False) -> int:
        value = checked_counter(raw.get(name), f"{source} coverage.{name}")
        if positive and value == 0:
            raise InputError(f"{source} coverage.{name} must be positive")
        return value

    def string(name: str) -> str:
        value = raw.get(name)
        if not isinstance(value, str) or not value.strip():
            raise InputError(f"{source} coverage.{name} must be a non-empty string")
        return value

    def boolean(name: str) -> bool:
        value = raw.get(name)
        if not isinstance(value, bool):
            raise InputError(f"{source} coverage.{name} must be a boolean")
        return value

    contract = CoverageContract(
        schema=integer("schema", positive=True),
        source_sha=string("source_sha"),
        bootstrap_approved=boolean("bootstrap_approved"),
        bootstrap_rule=string("bootstrap_rule"),
        global_covered=integer("global_covered"),
        global_count=integer("global_count", positive=True),
        core_covered=integer("core_covered"),
        core_count=integer("core_count", positive=True),
        core_file_count=integer("core_file_count", positive=True),
        pr_core_patch_min=integer("pr_core_patch_min", positive=True),
        pr_other_patch_min=integer("pr_other_patch_min", positive=True),
        release_global_min=integer("release_global_min", positive=True),
        release_core_min=integer("release_core_min", positive=True),
        rustc_release=string("rustc_release"),
        rustc_commit=string("rustc_commit"),
        llvm_version=string("llvm_version"),
        cargo_llvm_cov_version=string("cargo_llvm_cov_version"),
        reviewed_no_region=(),
    )
    if contract.schema != 1:
        raise InputError(f"unknown coverage contract schema in {source}: {contract.schema}")
    for name, covered, count in (
        ("global", contract.global_covered, contract.global_count),
        ("core", contract.core_covered, contract.core_count),
    ):
        if covered > count:
            raise InputError(f"{source} {name} baseline covered exceeds count")
    for name, threshold in (
        ("pr_core_patch_min", contract.pr_core_patch_min),
        ("pr_other_patch_min", contract.pr_other_patch_min),
        ("release_global_min", contract.release_global_min),
        ("release_core_min", contract.release_core_min),
    ):
        if threshold > 100:
            raise InputError(f"{source} coverage.{name} exceeds 100")
    for name, threshold, floor in (
        ("pr_core_patch_min", contract.pr_core_patch_min, 90),
        ("pr_other_patch_min", contract.pr_other_patch_min, 85),
        ("release_global_min", contract.release_global_min, 80),
        ("release_core_min", contract.release_core_min, 95),
    ):
        if threshold < floor:
            raise InputError(
                f"{source} coverage.{name} is below hard policy floor {floor}"
            )
    if re.fullmatch(r"[0-9a-f]{40}", contract.source_sha) is None:
        raise InputError(f"{source} coverage.source_sha is not a canonical Git SHA")
    reviewed_raw = raw.get("reviewed_no_region", {})
    if not isinstance(reviewed_raw, dict):
        raise InputError(f"{source} coverage.reviewed_no_region must be a table")
    reviewed: list[tuple[str, str]] = []
    for path, digest in sorted(reviewed_raw.items()):
        if (
            not isinstance(path, str)
            or not path.startswith("src/")
            or not path.endswith(".rs")
            or ".." in pathlib.PurePosixPath(path).parts
        ):
            raise InputError(f"{source} has invalid reviewed no-region path: {path!r}")
        if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
            raise InputError(f"{source} has invalid reviewed no-region SHA-256 for {path}")
        reviewed.append((path, digest))
    return replace(contract, reviewed_no_region=tuple(reviewed))


def verify_reviewed_no_region(
    contract: CoverageContract, repository: pathlib.Path
) -> frozenset[str]:
    verified: set[str] = set()
    for relative, expected in contract.reviewed_no_region:
        path = repository / relative
        try:
            actual = hashlib.sha256(path.read_bytes()).hexdigest()
        except OSError as exc:
            raise InputError(f"cannot verify reviewed no-region source {relative}: {exc}") from exc
        if actual != expected:
            raise InputError(f"reviewed no-region SHA-256 mismatch: {relative}")
        verified.add(relative)
    return frozenset(verified)


def load_candidate_contract(repository: pathlib.Path) -> CoverageContract:
    path = repository / CONTRACT_PATH
    try:
        document = path.read_bytes()
    except OSError as exc:
        raise InputError(f"coverage contract is missing: {exc}") from exc
    contract = parse_contract(document, CONTRACT_PATH)
    if contract is None:
        raise InputError(f"{CONTRACT_PATH} has no [coverage] table")
    run_git(repository, ["cat-file", "-e", f"{contract.source_sha}^{{commit}}"])
    return contract


def load_base_contract(repository: pathlib.Path, base_ref: str) -> CoverageContract | None:
    run_git(repository, ["cat-file", "-e", f"{base_ref}^{{commit}}"])
    document = run_git(
        repository,
        ["show", f"{base_ref}:{CONTRACT_PATH}"],
        allow_failure=True,
    )
    return None if not document else parse_contract(document, f"{base_ref}:{CONTRACT_PATH}")


def command_stdout(command: list[str], field: str) -> str:
    try:
        result = subprocess.run(
            command,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    except OSError as exc:
        raise InputError(f"cannot verify {field}: {exc}") from exc
    if result.returncode != 0:
        raise InputError(f"cannot verify {field}: {result.stderr.strip()}")
    return result.stdout


def verify_tool_identity(contract: CoverageContract) -> None:
    rustc = command_stdout(["rustc", "-Vv"], "rustc identity")
    llvm = re.search(r"^LLVM version: (.+)$", rustc, re.MULTILINE)
    release = re.search(r"^release: (.+)$", rustc, re.MULTILINE)
    commit = re.search(r"^commit-hash: (.+)$", rustc, re.MULTILINE)
    if not llvm or not release or not commit:
        raise InputError("rustc -Vv output is missing release/commit/LLVM identity")
    actual = (release.group(1), commit.group(1), llvm.group(1))
    expected = (contract.rustc_release, contract.rustc_commit, contract.llvm_version)
    if actual != expected:
        raise InputError(f"coverage tool identity mismatch: rustc/LLVM {actual!r} != {expected!r}")

    llvm_cov = command_stdout(["cargo", "llvm-cov", "--version"], "cargo-llvm-cov identity")
    match = re.search(r"cargo-llvm-cov\s+([^\s]+)", llvm_cov)
    if not match or match.group(1) != contract.cargo_llvm_cov_version:
        actual_version = match.group(1) if match else llvm_cov.strip()
        raise InputError(
            "coverage tool identity mismatch: cargo-llvm-cov "
            f"{actual_version!r} != {contract.cargo_llvm_cov_version!r}"
        )


def verify_source_binding(contract: CoverageContract, repository: pathlib.Path) -> None:
    inputs = ["src", "build_support", "Cargo.toml", "Cargo.lock", "build.rs"]
    if not git_succeeds(
        repository, ["merge-base", "--is-ancestor", contract.source_sha, "HEAD"]
    ):
        raise InputError("coverage.source_sha is not an ancestor of HEAD")
    if not git_succeeds(
        repository,
        ["diff", "--quiet", f"{contract.source_sha}..HEAD", "--", *inputs],
    ):
        raise InputError("coverage inputs changed after coverage.source_sha")
    dirty = run_git(
        repository,
        ["status", "--porcelain", "--untracked-files=no", "--", *inputs],
    )
    if dirty:
        raise InputError("coverage inputs have uncommitted changes after coverage.source_sha")


def parse_lcov(path: pathlib.Path, repository: pathlib.Path) -> dict[str, dict[int, int]]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        raise InputError(f"LCOV report is missing or invalid: {exc}") from exc
    records: dict[str, dict[int, int]] = {}
    current: str | None = None
    for raw in lines:
        if raw.startswith("SF:"):
            current = strict_repository_relative_path(raw[3:], repository)
            if current in records:
                raise InputError(f"duplicate LCOV source record: {current}")
            records[current] = {}
        elif raw.startswith("DA:"):
            if current is None:
                raise InputError("LCOV DA record appears before SF")
            parts = raw[3:].split(",", 2)
            try:
                line = int(parts[0])
                hits = int(parts[1])
            except (IndexError, ValueError) as exc:
                raise InputError(f"invalid LCOV DA record: {raw}") from exc
            if line <= 0 or hits < 0 or line in records[current]:
                raise InputError(f"invalid or duplicate LCOV line record: {raw}")
            records[current][line] = hits
    if not records:
        raise InputError("LCOV report contains no source records")
    return records


def changed_lines(repository: pathlib.Path, base_ref: str) -> tuple[dict[str, set[int]], set[str]]:
    try:
        patch = run_git(
            repository,
            ["diff", "--find-renames", "--unified=0", f"{base_ref}...HEAD", "--", "src"],
        ).decode("utf-8", errors="strict")
    except UnicodeDecodeError as exc:
        raise InputError(f"Git diff is not UTF-8: {exc}") from exc
    changed: dict[str, set[int]] = {}
    observed_files: set[str] = set()
    current: str | None = None
    for raw in patch.splitlines():
        if raw.startswith("diff --git a/"):
            match = re.match(r"^diff --git a/(.+) b/(.+)$", raw)
            if match is None or match.group(1).startswith('"') or match.group(2).startswith('"'):
                raise InputError(f"unsupported Git diff header: {raw}")
            observed_files.add(match.group(2))
        elif raw.startswith("+++ "):
            token = raw[4:]
            if token == "/dev/null":
                current = None
            elif token.startswith("b/") and not token.startswith('b/"'):
                current = token[2:]
                observed_files.add(current)
                changed.setdefault(current, set())
            else:
                raise InputError(f"unsupported Git patch path: {token}")
        elif raw.startswith("@@ "):
            if current is None:
                continue
            match = re.match(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@", raw)
            if match is None:
                raise InputError(f"cannot parse Git diff hunk: {raw}")
            start = int(match.group(1))
            count = int(match.group(2) or "1")
            changed[current].update(range(start, start + count))

    names = run_git(
        repository,
        [
            "diff",
            "--find-renames",
            "--name-only",
            "--diff-filter=ACMR",
            "-z",
            f"{base_ref}...HEAD",
            "--",
            "src",
        ],
    )
    try:
        changed_files = {
            item.decode("utf-8")
            for item in names.split(b"\0")
            if item and item.decode("utf-8").endswith(".rs")
        }
    except UnicodeDecodeError as exc:
        raise InputError(f"Git changed path is not UTF-8: {exc}") from exc
    missing_patch_files = sorted(changed_files - observed_files)
    if missing_patch_files:
        raise InputError(
            "Git diff omitted changed source paths: " + ", ".join(missing_patch_files)
        )
    return changed, changed_files


def ratio_at_least(covered: int, count: int, baseline_covered: int, baseline_count: int) -> bool:
    return covered * baseline_count >= baseline_covered * count


def percent_at_least(covered: int, count: int, minimum: int) -> bool:
    return count == 0 or covered * 100 >= minimum * count


def contract_regressed(candidate: CoverageContract, base: CoverageContract) -> bool:
    ratios = (
        (
            candidate.global_covered,
            candidate.global_count,
            base.global_covered,
            base.global_count,
        ),
        (
            candidate.core_covered,
            candidate.core_count,
            base.core_covered,
            base.core_count,
        ),
    )
    if any(not ratio_at_least(*ratio) for ratio in ratios):
        return True
    return any(
        candidate_value < base_value
        for candidate_value, base_value in (
            (candidate.pr_core_patch_min, base.pr_core_patch_min),
            (candidate.pr_other_patch_min, base.pr_other_patch_min),
            (candidate.release_global_min, base.release_global_min),
            (candidate.release_core_min, base.release_core_min),
        )
    )


def print_patch_bucket(name: str, covered: int, count: int, minimum: int) -> bool:
    if count == 0:
        print(f"{name} patch coverage: N/A (0 executable changed lines)")
        return True
    print(
        f"{name} patch coverage: {covered}/{count} = {percentage(covered, count):.2f}% "
        f"(required {minimum:.2f}%)"
    )
    return percent_at_least(covered, count, minimum)


def tracked_production_sources(repository: pathlib.Path) -> frozenset[str]:
    output = run_git(repository, ["ls-files", "-z", "--", "src"])
    try:
        return frozenset(
            item.decode("utf-8")
            for item in output.split(b"\0")
            if item and item.decode("utf-8").endswith(".rs")
        )
    except UnicodeDecodeError as exc:
        raise InputError(f"Git source path is not UTF-8: {exc}") from exc


def run_release_policy(
    totals: CoverageTotals,
    report_path: pathlib.Path,
    lcov_path: pathlib.Path | None,
    global_min: float,
    core_min: float,
) -> int:
    print_totals(totals, global_min, core_min)
    threshold_failed = (
        percentage(totals.global_covered, totals.global_count) + 1e-9 < global_min
        or percentage(totals.core_covered, totals.core_count) + 1e-9 < core_min
    )
    if threshold_failed:
        if percentage(totals.global_covered, totals.global_count) + 1e-9 < global_min:
            print("global coverage gate failed", file=sys.stderr)
        if percentage(totals.core_covered, totals.core_count) + 1e-9 < core_min:
            print("core coverage gate failed", file=sys.stderr)
        return 1
    if lcov_path is None:
        raise InputError("release PASS requires --lcov and complete provenance evidence")

    repository = discover_repository()
    strict_totals = load_report(report_path, repository)
    contract = load_candidate_contract(repository)
    verify_source_binding(contract, repository)
    verify_tool_identity(contract)
    verify_report_provenance(report_path, repository, contract)
    if strict_totals.core_file_count != contract.core_file_count:
        raise InputError(
            "coverage report core file count mismatch: "
            f"{strict_totals.core_file_count} != {contract.core_file_count}"
        )
    records = parse_lcov(lcov_path, repository)
    lcov_sources = frozenset(
        path for path in records if path.startswith("src/") and path.endswith(".rs")
    )
    if lcov_sources != strict_totals.source_files:
        raise InputError("JSON and LCOV production source file sets differ")
    reviewed = verify_reviewed_no_region(contract, repository)
    expected_sources = tracked_production_sources(repository)
    if strict_totals.source_files | reviewed != expected_sources:
        missing = sorted(expected_sources - strict_totals.source_files - reviewed)
        extra = sorted((strict_totals.source_files | reviewed) - expected_sources)
        raise InputError(
            "release coverage source inventory mismatch: "
            f"missing={missing!r} extra={extra!r}"
        )
    effective_global_min = max(global_min, float(contract.release_global_min))
    effective_core_min = max(core_min, float(contract.release_core_min))
    if not percent_at_least(
        strict_totals.global_covered, strict_totals.global_count, int(effective_global_min)
    ) or not percent_at_least(
        strict_totals.core_covered, strict_totals.core_count, int(effective_core_min)
    ):
        print("release coverage gate failed against contract", file=sys.stderr)
        return 1
    return 0


def run_pr_policy(
    totals: CoverageTotals,
    report_path: pathlib.Path,
    lcov_path: pathlib.Path | None,
    base_ref: str | None,
    bootstrap: bool,
) -> int:
    if lcov_path is None or not base_ref:
        raise InputError("PR policy requires --lcov and --base-ref")
    repository = discover_repository()
    contract = load_candidate_contract(repository)
    verify_source_binding(contract, repository)
    verify_tool_identity(contract)
    verify_report_provenance(report_path, repository, contract)
    if totals.core_file_count != contract.core_file_count:
        raise InputError(
            "coverage report core file count mismatch: "
            f"{totals.core_file_count} != {contract.core_file_count}"
        )
    reviewed_no_region = verify_reviewed_no_region(contract, repository)
    print_totals(
        totals,
        percentage(contract.global_covered, contract.global_count),
        percentage(contract.core_covered, contract.core_count),
    )
    base_contract = load_base_contract(repository, base_ref)
    if base_contract is None and not bootstrap:
        raise InputError("initial coverage contract requires --bootstrap-baseline")
    if base_contract is None and (
        not contract.bootstrap_approved or contract.bootstrap_rule != "BR-252"
    ):
        raise InputError("initial baseline requires tracked BR-252 bootstrap approval")
    if base_contract is not None:
        candidate_reviewed_map = dict(contract.reviewed_no_region)
        base_reviewed_map = dict(base_contract.reviewed_no_region)
        candidate_reviewed = set(candidate_reviewed_map)
        base_reviewed = set(base_reviewed_map)
        added_reviewed = sorted(candidate_reviewed - base_reviewed)
        if added_reviewed:
            raise InputError(
                "non-bootstrap PR cannot add reviewed no-region paths: "
                + ", ".join(added_reviewed)
            )
        changed_reviewed = sorted(
            path
            for path in candidate_reviewed & base_reviewed
            if candidate_reviewed_map[path] != base_reviewed_map[path]
        )
        if changed_reviewed:
            raise InputError(
                "non-bootstrap PR cannot change reviewed no-region hashes: "
                + ", ".join(changed_reviewed)
            )

    failed = False
    if base_contract is not None and contract_regressed(contract, base_contract):
        print("candidate baseline regression", file=sys.stderr)
        failed = True
    for name, covered, count, baseline_covered, baseline_count in (
        (
            "global",
            totals.global_covered,
            totals.global_count,
            contract.global_covered,
            contract.global_count,
        ),
        (
            "core",
            totals.core_covered,
            totals.core_count,
            contract.core_covered,
            contract.core_count,
        ),
    ):
        if not ratio_at_least(covered, count, baseline_covered, baseline_count):
            print(f"{name} coverage ratchet failed", file=sys.stderr)
            failed = True

    records = parse_lcov(lcov_path, repository)
    lcov_sources = frozenset(
        path for path in records if path.startswith("src/") and path.endswith(".rs")
    )
    if lcov_sources != totals.source_files:
        raise InputError("JSON and LCOV production source file sets differ")
    changed, changed_files = changed_lines(repository, base_ref)
    missing_changed_files = sorted(
        changed_files - totals.source_files - reviewed_no_region
    )
    if missing_changed_files:
        raise InputError(
            "changed source is absent from JSON/LCOV coverage evidence: "
            + ", ".join(missing_changed_files)
        )

    buckets = {"core": [0, 0], "other production": [0, 0]}
    for path, lines in changed.items():
        executable = records.get(path, {})
        bucket = "core" if path.startswith(CORE_PREFIXES) else "other production"
        for line in lines:
            if line not in executable:
                continue
            buckets[bucket][1] += 1
            buckets[bucket][0] += int(executable[line] > 0)

    core_ok = print_patch_bucket(
        "core", *buckets["core"], contract.pr_core_patch_min
    )
    other_ok = print_patch_bucket(
        "other production", *buckets["other production"], contract.pr_other_patch_min
    )
    if not core_ok or not other_ok:
        print("patch coverage gate failed", file=sys.stderr)
        failed = True
    return 1 if failed else 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("legacy_report", nargs="?", type=pathlib.Path)
    parser.add_argument("--report", dest="named_report", type=pathlib.Path)
    parser.add_argument("--policy", choices=("pr", "release"), default="release")
    parser.add_argument("--lcov", type=pathlib.Path)
    parser.add_argument("--base-ref")
    parser.add_argument("--bootstrap-baseline", action="store_true")
    parser.add_argument("--global-min", type=float, default=80.0)
    parser.add_argument("--core-min", type=float, default=95.0)
    args = parser.parse_args()

    if not (80.0 <= args.global_min <= 100.0) or not (95.0 <= args.core_min <= 100.0):
        print("release thresholds cannot be lower than the fixed release floors 80/95", file=sys.stderr)
        return 2

    if args.legacy_report is not None and args.named_report is not None:
        parser.error("provide coverage report once, positionally or with --report")
    report = args.named_report or args.legacy_report
    if report is None:
        parser.error("a coverage report is required")

    try:
        repository = discover_repository() if args.policy == "pr" else None
        totals = load_report(report, repository)
        if args.policy == "release":
            return run_release_policy(
                totals, report, args.lcov, args.global_min, args.core_min
            )

        return run_pr_policy(totals, report, args.lcov, args.base_ref, args.bootstrap_baseline)
    except InputError as exc:
        print(str(exc), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
