#!/usr/bin/env python3
"""Verify a sealed executable-input manifest against one checkout or runtime root.

The manifest excludes the activation file. Pass that file as an allowed extra
only after checking its approved SHA-256 separately.

For activation readiness, use --activation-ready: activation hashes every
regular src/config file except the activation file, plus selected root
Cargo*.toml/Cargo.lock/build.rs inputs, so no other executable extra is safe.
V2 also requires every checked-in public gRPC build input. It is selected
from the declared manifest, sealed build entry, or current contract directory.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re


SHA256 = re.compile(r"[0-9a-f]{64}\Z")
ACTIVATION_FILE = "config/selection/selection_activation.v1.json"
MANIFEST_V1 = "selection-executable-inputs-v1"
MANIFEST_V2 = "selection-executable-inputs-v2"
COMPILED_PUBLIC_INPUTS = {
    "contracts/local_bridge_v1/market.proto",
    "contracts/external_v1_current/market.proto",
    "contracts/external_v1_current/bundle-metadata.json",
    "contracts/external_v1_history/market.proto",
    "contracts/external_v1_history/bundle-20260917.1.json",
    "contracts/external_v1_history/20260928.2/market.proto",
    "contracts/external_v1_history/20260928.2/bundle-metadata.json",
}


def canonical_path(value: object) -> str | None:
    if not isinstance(value, str) or not value or "\\" in value or "\x00" in value:
        return None
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in (".", "..") for part in path.parts):
        return None
    return value if path.as_posix() == value else None


def verify(
    manifest: Path, root: Path, allowed_extra: set[str], activation_ready: bool = False,
    input_manifest_version: str | None = None,
) -> tuple[int, list[str]]:
    errors: list[str] = []
    try:
        entries = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        return 0, [f"manifest unreadable: {error}"]
    if not isinstance(entries, list) or not entries:
        return 0, ["manifest must contain a nonempty list"]
    if not root.is_dir() or root.is_symlink():
        return 0, ["root must be a real directory"]
    if activation_ready:
        for name in sorted(allowed_extra - {ACTIVATION_FILE}):
            errors.append(f"activation input cannot be allowed extra: {name}")

    expected: dict[str, tuple[int, str]] = {}
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            errors.append(f"manifest row {index}: invalid object")
            continue
        name = canonical_path(entry.get("path"))
        length = entry.get("length")
        digest = entry.get("sha256")
        if (
            name is None
            or name in expected
            or not isinstance(length, int)
            or isinstance(length, bool)
            or length < 0
            or not isinstance(digest, str)
            or SHA256.fullmatch(digest) is None
        ):
            errors.append(f"manifest row {index}: invalid or duplicate path/length/hash")
            continue
        expected[name] = (length, digest)

    current_directory = root / "contracts/external_v1_current"
    current_contract_required = os.path.lexists(current_directory) or any(
        name.startswith("contracts/external_v1_current/") for name in expected
    )
    build_path = root / "build.rs"
    if "build.rs" in expected and build_path.is_file() and not build_path.is_symlink():
        try:
            current_contract_required |= (
                b"contracts/external_v1_current/market.proto" in build_path.read_bytes()
            )
        except OSError as error:
            errors.append(f"build entry unreadable: {error}")
    version = input_manifest_version or (
        MANIFEST_V2 if current_contract_required else MANIFEST_V1
    )
    if version not in {MANIFEST_V1, MANIFEST_V2}:
        return 0, ["unsupported executable input manifest version"]
    if activation_ready and current_contract_required and version != MANIFEST_V2:
        errors.append("current public contracts require executable input manifest v2")

    if version == MANIFEST_V2:
        for name in sorted(COMPILED_PUBLIC_INPUTS - expected.keys()):
            errors.append(f"compiled public input missing from manifest: {name}")

    if activation_ready:
        selected_root_inputs: set[str] = set()
        try:
            for path in root.iterdir():
                name = path.name
                selected = (name.startswith("Cargo") and name.endswith(".toml")) or name in {
                    "Cargo.lock", "build.rs",
                }
                if not selected:
                    continue
                selected_root_inputs.add(name)
                if path.is_symlink() or not path.is_file():
                    errors.append(f"root input is not a regular file: {name}")
                elif name not in expected:
                    errors.append(f"unexpected root input file: {name}")
            if "Cargo.toml" not in selected_root_inputs:
                errors.append("required root input missing: Cargo.toml")
        except OSError as error:
            errors.append(f"root input enumeration failed: {error}")

    for name, (expected_length, expected_digest) in expected.items():
        path = root
        parts = PurePosixPath(name).parts
        try:
            for part in parts:
                path = path / part
                if path.is_symlink():
                    raise ValueError("symlink in input path")
            if not path.is_file():
                raise ValueError("missing or non-file input")
            measured = hashlib.sha256()
            measured_length = 0
            with path.open("rb") as source:
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    measured.update(chunk)
                    measured_length += len(chunk)
            if measured_length != expected_length or measured.hexdigest() != expected_digest:
                errors.append(f"input differs: {name}")
        except (OSError, ValueError) as error:
            errors.append(f"input invalid: {name}: {error}")

    for section in ("src", "config"):
        directory = root / section
        if not directory.is_dir() or directory.is_symlink():
            errors.append(f"input directory invalid: {section}")
            continue
        for current, directories, files in os.walk(directory, followlinks=False):
            for child in directories + files:
                path = Path(current) / child
                name = path.relative_to(root).as_posix()
                if path.is_symlink():
                    errors.append(f"unexpected symlink: {name}")
                elif path.is_file() and name not in expected and name not in allowed_extra:
                    errors.append(f"unexpected input file: {name}")

    return len(expected), errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("root", type=Path)
    parser.add_argument("--allow-extra", action="append", default=[], metavar="RELATIVE_PATH")
    parser.add_argument(
        "--activation-ready", action="store_true",
        help="reject allowed extras other than the separately verified activation file",
    )
    parser.add_argument(
        "--input-manifest-version", choices=(MANIFEST_V1, MANIFEST_V2),
        help="defaults to v2 when declared public inputs or the sealed build require it",
    )
    args = parser.parse_args()
    allowed_extra = set(args.allow_extra)
    if any(canonical_path(name) is None for name in allowed_extra):
        parser.error("--allow-extra requires canonical relative paths")
    count, errors = verify(
        args.manifest, args.root, allowed_extra, args.activation_ready,
        args.input_manifest_version,
    )
    if errors:
        for error in errors:
            print(error)
        print(f"FAIL: {count} declared inputs, {len(errors)} errors")
        return 1
    print(f"OK: {count} declared inputs match; input scope verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
