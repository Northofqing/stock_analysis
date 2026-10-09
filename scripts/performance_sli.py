#!/usr/bin/env python3
"""Comparable observational SLI records. Warnings are soft; unavailable is explicit."""
import argparse
import json
import math
import re
from pathlib import Path

IDENTITY = ("version", "metric", "stage", "unit", "statistic", "scenario", "snapshot", "rows", "target", "profile", "features", "toolchain", "hardware")


def compare(baseline, current):
    for label, record in (("baseline", baseline), ("current", current)):
        if not isinstance(record, dict) or any(key not in record or record[key] is None for key in IDENTITY):
            return {"status": "unavailable", "reason": f"{label} missing identity fields"}
        if type(record["version"]) is not int or record["version"] != 1:
            return {"status": "unavailable", "reason": "unsupported version"}
        text_keys = set(IDENTITY) - {"version", "rows", "features"}
        if any(not isinstance(record[k], str) or not record[k].strip() for k in text_keys):
            return {"status": "unavailable", "reason": "invalid textual identity"}
        if type(record["rows"]) is not int or record["rows"] < 0 or not isinstance(record["features"], list) or any(not isinstance(f, str) or not f.strip() for f in record["features"]):
            return {"status": "unavailable", "reason": "invalid row/features identity"}
        if record.get("log_status", "ok") not in ("ok", "observed"):
            return {"status": "unavailable", "reason": "stage did not complete successfully"}
        value = record.get("value")
        try:
            valid_value = not isinstance(value, bool) and isinstance(value, (int, float)) and math.isfinite(value) and value > 0
        except OverflowError:
            valid_value = False
        if not valid_value:
            return {"status": "unavailable", "reason": f"{label} value must be finite and positive"}
    mismatch = [key for key in IDENTITY if baseline[key] != current[key]]
    if mismatch:
        return {"status": "unavailable", "reason": "incompatible: " + ", ".join(mismatch)}
    ratio = current["value"] / baseline["value"]
    if not math.isfinite(ratio):
        return {"status": "unavailable", "reason": "ratio is not finite"}
    return {"status": "warning" if ratio > 1.2 else "within_threshold", "ratio": ratio, "threshold": 1.2}


def bounded_json(path):
    with Path(path).open("rb") as stream:
        data = stream.read(1_048_577)
    if len(data) > 1_048_576:
        raise ValueError("SLI/context exceeds 1 MiB")
    return json.loads(data)


def collect(log, context):
    patterns = [
        ("startup", re.compile(r"\[startup-profile\].*?stage=(\S+) status=(\S+) elapsed_ms=(\d+(?:\.\d+)?)")),
        ("database_init", re.compile(r"\[DB init\]\[timing\] phase=(\S+) elapsed_ms=(\d+(?:\.\d+)?)")),
    ]
    records = []
    with Path(log).open("rb") as stream:
        size = 0
        while True:
            raw = stream.readline(1_048_577)
            if not raw:
                break
            size += len(raw)
            if len(raw) > 1_048_576 or size > 67_108_864:
                raise ValueError("log exceeds bounded input budget")
            line = raw.decode("utf-8")
            for metric, pattern in patterns:
                match = pattern.search(line)
                if match:
                    records.append({**context, "version": 1, "metric": metric, "stage": match[1], "unit": "ms", "statistic": "sample", "value": float(match.groups()[-1]), "log_status": match[2] if metric == "startup" else "observed"})
    return {"records": records, "review_duration": "unavailable_no_existing_stage_log", "validation_only": "unavailable_use_audit_fixture_harness", "note": "database acquisition phase includes schema work; startup includes its named stage only"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    comparison = sub.add_parser("compare")
    comparison.add_argument("baseline")
    comparison.add_argument("current")
    logs = sub.add_parser("collect")
    logs.add_argument("log")
    logs.add_argument("context")
    args = parser.parse_args()
    try:
        result = compare(bounded_json(args.baseline), bounded_json(args.current)) if args.command == "compare" else collect(args.log, bounded_json(args.context))
    except (OSError, ValueError, TypeError) as error:
        result = {"status": "unavailable", "reason": str(error)}
    print(json.dumps(result, allow_nan=False))
    # Soft comparator deliberately succeeds even for unavailable/warning, never calls it pass.


if __name__ == "__main__":
    main()
