#!/usr/bin/env python3
"""Explicit opt-in three-way build; exports committed source into a fresh owned directory."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import time


def output(*command):
    return subprocess.check_output(command, text=True).strip()


def disk(root):
    return sum(p.stat().st_size for p in root.rglob("*") if p.is_file())


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--execute", action="store_true", required=True)
    p.add_argument("--root", type=Path, required=True, help="new directory, must not exist")
    p.add_argument("--commit", required=True)
    p.add_argument("--incremental", choices=("0", "1"), required=True)
    p.add_argument("--profile", choices=("dev", "release"), default="dev")
    args = p.parse_args()
    commit = output("git", "rev-parse", "--verify", args.commit + "^{commit}")
    root = args.root.resolve()
    root.mkdir(parents=False, exist_ok=False)
    checkout = root / "source"
    checkout.mkdir()
    # git archive emits trusted committed tree; no user working files are edited.
    archive = subprocess.Popen(["git", "archive", commit], stdout=subprocess.PIPE)
    extraction = subprocess.run(["tar", "-x", "-C", str(checkout)], stdin=archive.stdout)
    archive.stdout.close()
    if extraction.returncode or archive.wait():
        raise RuntimeError("source export failed")
    target = root / "target"
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL=args.incremental,
               STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT=str(checkout))
    command = ["cargo", "build", "--locked", "--offline", "--bin", "monitor", "--profile", args.profile]
    report = {"version": 1, "commit": commit, "root": str(root), "command": command,
              "profile": args.profile, "incremental": args.incremental,
              "build_production_root": str(checkout),
              "rustc": output("rustc", "-Vv"), "cargo": output("cargo", "-V"),
              "hardware": platform.platform() + " " + platform.machine() + " " + platform.processor(),
              "cpu_count": os.cpu_count(), "features": [],
              "inherited_build_environment": {k: os.environ[k] for k in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "CARGO_BUILD_JOBS", "RUSTC_WRAPPER") if k in os.environ},
              "cache_policy": "fresh target; OS and Cargo dependency caches retained", "runs": []}
    for phase in ("cold", "unchanged", "single_file_edit"):
        if phase == "single_file_edit":
            edit_path = checkout / "src/bin/monitor/main.rs"
            report["edit_file_sha256_before"] = hashlib.sha256(edit_path.read_bytes()).hexdigest()
            with edit_path.open("a") as edited:
                edited.write("\n// TEST_CODE performance baseline controlled comment edit.\n")
            report["edit_file_sha256_after"] = hashlib.sha256(edit_path.read_bytes()).hexdigest()
            report["edit"] = "append one comment to disposable src/bin/monitor/main.rs; crate invalidation only"
        started = time.monotonic()
        with (root / (phase + ".log")).open("w") as log:
            result = subprocess.run(command, cwd=checkout, env=env, stdout=log, stderr=subprocess.STDOUT)
        report["runs"].append({"phase": phase, "elapsed_seconds": time.monotonic()-started,
                               "exit_code": result.returncode, "target_bytes": disk(target) if target.exists() else 0})
        (root / "report.json").write_text(json.dumps(report, indent=2))
        if result.returncode:
            raise SystemExit(result.returncode)
    print(root / "report.json")
    # No cleanup; deletion is a separate explicit caller action scoped to this root.


if __name__ == "__main__":
    main()
