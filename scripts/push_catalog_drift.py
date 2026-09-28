#!/usr/bin/env python3
"""Check current PushKind identities against the frozen push catalog and v2 delta."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys


CATALOG = Path("docs/push-system/push-capability-catalog.v1.json")
DELTA = Path("docs/push-system/push-current-kind-delta.v2.json")
SOURCE = Path("src/bin/monitor/notify.rs")
VARIANT = re.compile(r"\s*([A-Za-z][A-Za-z0-9_]*),\s*$")


def enum_kinds(source):
    body = source.split("pub enum PushKind {", 1)
    if len(body) != 2:
        raise ValueError("PushKind enum not found")
    body = body[1].split("}", 1)[0]
    variants = []
    for line in body.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("//"):
            continue
        match = VARIANT.fullmatch(line)
        if not match:
            raise ValueError(f"unrecognized PushKind enum line: {stripped}")
        variants.append(match.group(1))
    if len(variants) != len(set(variants)):
        raise ValueError("duplicate PushKind variant")
    return variants


def check(root):
    catalog_bytes = (root / CATALOG).read_bytes()
    catalog = json.loads(catalog_bytes)
    delta = json.loads((root / DELTA).read_bytes())
    kinds = enum_kinds((root / SOURCE).read_text(encoding="utf-8"))
    historical = {entry["kind"] for entry in catalog["kinds"]}
    producers = {entry["id"]: entry for entry in catalog["producers"]}
    units = {entry["id"]: entry for entry in catalog["migration_units"]}
    mappings = delta["mappings"]
    errors = []

    if catalog["schema_version"] != 1 or delta["schema_version"] != 2:
        errors.append("unsupported catalog/delta version")
    if delta["role"] != "current-source-kind-delta" or delta["status"] != "PROVISIONAL":
        errors.append("unexpected delta role/status")
    if hashlib.sha256(catalog_bytes).hexdigest() != delta["historical_catalog_sha256"]:
        errors.append("frozen catalog digest changed")
    if len(catalog["kinds"]) != len(historical):
        errors.append("duplicate historical kind")
    if len(producers) != len(catalog["producers"]) or len(units) != len(catalog["migration_units"]):
        errors.append("duplicate historical producer/unit")

    source = set(kinds)
    added = sorted(source - historical)
    removed = sorted(historical - source)
    mapped = [entry["kind"] for entry in mappings]
    if len(mapped) != len(set(mapped)) or sorted(mapped) != added:
        errors.append("delta mappings do not exactly cover new source kinds")
    if removed:
        errors.append("historical kinds missing from source: " + ", ".join(removed))
    for entry in mappings:
        kind, producer_id, unit_id = (entry["kind"], entry["producer_id"], entry["migration_unit_id"])
        producer = producers.get(producer_id)
        unit = units.get(unit_id)
        if producer is None or unit is None:
            errors.append(f"{kind}: unknown historical producer/unit")
            continue
        if producer["migration_unit_id"] != unit_id or producer_id not in unit["producer_ids"]:
            errors.append(f"{kind}: producer/unit relationship mismatch")
        if producer["kinds"] != [entry["replaces_producer_kind"]]:
            errors.append(f"{kind}: historical producer kind mismatch")
        if entry["replaces_producer_kind"] not in historical:
            errors.append(f"{kind}: replaced kind is not historical")
        if entry["source_status"] != "ACTIVE" or entry["production_status"] != "UNVERIFIED":
            errors.append(f"{kind}: source/production status must be explicit")
        for evidence in entry["evidence"]:
            path, fragment = evidence["path"], evidence["fragment"]
            if not path.startswith("src/") or not (root / path).is_file():
                errors.append(f"{kind}: missing source evidence {path}")
            elif not fragment or fragment not in (root / path).read_text(encoding="utf-8"):
                errors.append(f"{kind}: source evidence changed at {path}")

    return {
        "historical_kinds": len(historical),
        "source_kinds": len(kinds),
        "historical_producers": len(producers),
        "historical_units": len(units),
        "added_kinds": added,
        "removed_kinds": removed,
        "mappings": mappings,
        "errors": errors,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    try:
        result = check(args.root.resolve())
    except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError) as exc:
        result = {"errors": [str(exc)]}
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 1 if result["errors"] else 0


if __name__ == "__main__":
    sys.exit(main())
