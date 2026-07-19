#!/usr/bin/env python3
"""Build reproducible current/history documentation and implementation manifests."""

from __future__ import annotations

import hashlib
import os
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SNAPSHOT = sys.argv[1] if len(sys.argv) > 1 else "HEAD"
DOC_EXTENSIONS = {".md", ".txt", ".rst", ".adoc", ".org", ".pdf", ".doc", ".docx"}
CODE_ROOTS = (
    "src/", "tests/", "tools/", "config/", "migrations/", ".github/", "benches/", "deploy/",
)
CODE_ROOT_FILES = {
    "Cargo.toml", "Cargo.lock", "build.rs", "diesel.toml", ".env.example", ".gitignore", ".claude/settings.json",
    "AGENTS.md", "CLAUDE.md",
}
TEXT_EXTENSIONS = {
    ".rs", ".sh", ".toml", ".sql", ".yml", ".yaml", ".md", ".json",
    ".txt", ".cpp", ".h", ".py",
}


def git(*args: str, check: bool = True) -> bytes:
    return subprocess.run(
        ["git", *args], cwd=ROOT, check=check, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    ).stdout


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def is_text(path: str, data: bytes) -> bool:
    if Path(path).suffix.lower() in TEXT_EXTENSIONS:
        return True
    return b"\0" not in data[:8192]


def line_count(data: bytes, text: bool) -> str:
    if not text:
        return ""
    return str(data.count(b"\n") + (1 if data and not data.endswith(b"\n") else 0))


def snapshot_paths() -> list[str]:
    raw = git("ls-tree", "-r", "--name-only", SNAPSHOT).decode("utf-8", "replace")
    return [line for line in raw.splitlines() if line]


def current_doc_paths() -> list[str]:
    docs = ROOT / "docs"
    return sorted({
        str(path.relative_to(ROOT))
        for path in docs.rglob("*")
        if path.is_file() and path.name != ".DS_Store"
    })


def historical_doc_paths() -> set[str]:
    raw = git("-c", "core.quotePath=false", "log", "--all", "--format=", "--name-only").decode(
        "utf-8", "replace"
    )
    return {
        path.strip()
        for path in raw.splitlines()
        if path.strip()
        and (
            path.strip().startswith("docs/")
            or Path(path.strip()).suffix.lower() in DOC_EXTENSIONS
        )
        and not path.strip().startswith((".agents/", ".codex/", ".planning/2026-07-16-historical-doc-code-audit/"))
    }


def current_code_paths() -> set[str]:
    paths: set[str] = set()
    for root_name in CODE_ROOTS:
        root = ROOT / root_name.rstrip("/")
        if not root.exists():
            continue
        paths.update(
            str(path.relative_to(ROOT))
            for path in root.rglob("*")
            if path.is_file() and path.name != ".DS_Store"
        )
    paths.update(name for name in CODE_ROOT_FILES if (ROOT / name).is_file())
    return paths


def blob_at_snapshot(path: str) -> bytes | None:
    proc = subprocess.run(
        ["git", "show", f"{SNAPSHOT}:{path}"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return proc.stdout if proc.returncode == 0 else None


def last_commit_for(path: str) -> str:
    raw = git("log", "--all", "-1", "--format=%H", "--", path, check=False)
    return raw.decode().strip()


def historical_blob(path: str) -> tuple[bytes | None, str]:
    commits = git("log", "--all", "--format=%H", "--", path, check=False).decode().splitlines()
    for commit in commits:
        proc = subprocess.run(
            ["git", "show", f"{commit}:{path}"],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if proc.returncode == 0:
            return proc.stdout, commit
    return None, ""


def emit_docs() -> None:
    current = set(current_doc_paths())
    at_snapshot = {
        p for p in snapshot_paths()
        if p.startswith("docs/") or Path(p).suffix.lower() in DOC_EXTENSIONS
    }
    historical = historical_doc_paths()
    all_paths = sorted(current | at_snapshot | historical)
    print("path\tpresent_workspace\tpresent_snapshot\thistorical_only\ttracked_snapshot\tbytes\tlines\tsha256\tcontent_kind\tcontent_commit\tlast_touch_commit")
    for path in all_paths:
        workspace_path = ROOT / path
        present_workspace = workspace_path.is_file()
        present_snapshot = path in at_snapshot
        content_commit = "workspace" if present_workspace else (SNAPSHOT if present_snapshot else "")
        data = workspace_path.read_bytes() if present_workspace else blob_at_snapshot(path)
        if data is None and path in historical:
            data, content_commit = historical_blob(path)
        if data is None:
            size = lines = digest = ""
            kind = "historical-path-only"
        else:
            text = is_text(path, data)
            size = str(len(data))
            lines = line_count(data, text)
            digest = sha256(data)
            kind = "text" if text else "binary"
        print(
            "\t".join(
                [
                    path,
                    str(present_workspace).lower(),
                    str(present_snapshot).lower(),
                    str(not present_workspace and not present_snapshot).lower(),
                    str(present_snapshot).lower(),
                    size,
                    lines,
                    digest,
                    kind,
                    content_commit,
                    last_commit_for(path),
                ]
            )
        )


def emit_code() -> None:
    snapshot = snapshot_paths()
    snapshot_set = set(snapshot)
    workspace_set = current_code_paths()
    paths = sorted(
        path
        for path in snapshot_set | workspace_set
        if path in CODE_ROOT_FILES or any(path.startswith(prefix) for prefix in CODE_ROOTS)
    )
    print("path\tpresent_workspace\tpresent_snapshot\tworkspace_differs\tbytes\tlines\tsha256\tcontent_kind\tscan_class")
    for path in paths:
        present_workspace = path in workspace_set
        present_snapshot = path in snapshot_set
        snapshot_data = blob_at_snapshot(path) if present_snapshot else None
        workspace_data = (ROOT / path).read_bytes() if present_workspace else None
        data = snapshot_data if snapshot_data is not None else workspace_data
        if data is None:
            continue
        workspace_differs = bool(
            present_workspace
            and present_snapshot
            and workspace_data is not None
            and snapshot_data is not None
            and workspace_data != snapshot_data
        )
        text = is_text(path, data)
        ext = Path(path).suffix.lower()
        if not text:
            scan_class = "binary-asset"
        elif ext == ".rs":
            scan_class = "rust"
        elif ext == ".sh":
            scan_class = "shell"
        elif ext in {".toml", ".yaml", ".yml", ".json"}:
            scan_class = "config"
        elif ext == ".sql":
            scan_class = "migration"
        elif path.startswith("tests/"):
            scan_class = "test-support"
        else:
            scan_class = "text-support"
        if not present_snapshot:
            scan_class = f"workspace-only-excluded:{scan_class}"
        print("\t".join([
            path,
            str(present_workspace).lower(),
            str(present_snapshot).lower(),
            str(workspace_differs).lower(),
            str(len(data)),
            line_count(data, text),
            sha256(data),
            "text" if text else "binary",
            scan_class,
        ]))


if __name__ == "__main__":
    if len(sys.argv) < 3 or sys.argv[2] not in {"docs", "code"}:
        raise SystemExit("usage: build_manifest.py <snapshot> <docs|code>")
    os.chdir(ROOT)
    emit_docs() if sys.argv[2] == "docs" else emit_code()
