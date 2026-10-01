#!/usr/bin/env python3
"""Verify a sealed executable-input manifest against one checkout or runtime root.

The manifest excludes the activation file. Pass that file as an allowed extra
only after checking its approved SHA-256 separately.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re


SHA256 = re.compile(r"[0-9a-f]{64}\Z")


def canonical_path(value: object) -> str | None:
    if not isinstance(value, str) or not value or "\\" in value or "\x00" in value:
        return None
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in (".", "..") for part in path.parts):
        return None
    return value if path.as_posix() == value else None


def verify(manifest: Path, root: Path, allowed_extra: set[str]) -> tuple[int, list[str]]:
    errors: list[str] = []
    try:
        entries = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        return 0, [f"manifest unreadable: {error}"]
    if not isinstance(entries, list) or not entries:
        return 0, ["manifest must contain a nonempty list"]
    if not root.is_dir() or root.is_symlink():
        return 0, ["root must be a real directory"]

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
    args = parser.parse_args()
    allowed_extra = set(args.allow_extra)
    if any(canonical_path(name) is None for name in allowed_extra):
        parser.error("--allow-extra requires canonical relative paths")
    count, errors = verify(args.manifest, args.root, allowed_extra)
    if errors:
        for error in errors:
            print(error)
        print(f"FAIL: {count} declared inputs, {len(errors)} errors")
        return 1
    print(f"OK: {count} declared inputs match; no unexpected src/config files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
