#!/usr/bin/env python3
"""Restore one evidenced legacy prefix; preview is read-only and the default.

This incident restores original bytes, not price qualification or a ruling.
Apply additionally requires the deployed lifecycle guard's verified readback.
No initialization, UPDATE, DELETE, provider, sink, or ledger seed is used.
"""
import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import struct
import sys

COLS = ("id", "plan_id", "code", "name", "direction", "price", "quantity",
        "status", "fill_price", "not_fill_reason", "virtual_reason",
        "account_mode", "data_mode", "ts", "updated_at")
PROTECTED = ("order_audit", "order_audit_chain", "paper_inventory_failure_audit",
             "paper_inventory_failure_audit_chain", "attribution_sample_epoch_receipt",
             "attribution_sample_epoch_receipt_chain", "attribution_legacy_carry_item",
             "attribution_epoch_attempt_audit", "attribution_epoch_attempt_chain",
             "user_account_summary", "user_position_snapshot",
             "user_position_snapshot_item", "real_account_snapshot")


@dataclass(frozen=True)
class RecoveryPins:
    backup_sha256: str
    candidate_sha256: str
    ids: tuple
    original_count: int
    sequence: int
    audit_count: int
    audit_id: int
    audit_snapshot: str
    audit_tip: str
    epoch_id: str
    epoch_receipt: str
    legacy_manifest: str
    protected: tuple = PROTECTED


PINS = RecoveryPins(
    "a6f36c96047d0ca885d8e1af2764bdf76327f23533fe379bff893b1a5aba144e",
    "83e7216d6692aa35449718e46c69ad97903e0a342a708ae734e76c1089650662",
    (375, 376, 377, 382, 385, 386, 389, 394, 397, 400, 403, 406, 409,
     412, 415, 418, 421, 442, 454), 2589, 2592, 29, 29,
    "beebdb8316841a43d38667757113f6755607eddb5fec05187bd7ed0e4b49d215",
    "cba9d83613b0a491338de05a3d26e19f528ba4f020a7401ace890a2c3a66db6b",
    "a23b38cc1f414cb86f0cefeeb4f9d2cb868f2a2d30152e7f2ebfb73af902e4c2",
    "c4133d4125724b87cfd0c4eaf94b4b893d80c36867e0826773e3e170461c7901",
    "53a6051bc7aba1cb1ffdf677afa5ae9a9a1a1c137dabb1b96af46c7ee4be17af")


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def digest_file(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def sync_receipt_directory(path):
    # The intent's new directory entry must survive before the DB can commit.
    fd = os.open(Path(path).parent, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def packed(value):
    if isinstance(value, float):
        return ["float64", struct.pack(">d", value).hex()]
    if isinstance(value, bytes):
        return ["bytes", value.hex()]
    return [type(value).__name__, value]


def row_key(row):
    return tuple(json.dumps(packed(v), ensure_ascii=False) for v in row)


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"),
                      allow_nan=False).encode("utf-8")


def hash_fields(domain, *fields):
    h = hashlib.sha256(domain)
    for field in fields:
        b = field.encode("utf-8") if isinstance(field, str) else field
        h.update(len(b).to_bytes(8, "big"))
        h.update(b)
    return h.hexdigest()


def connect(path, writable=False):
    path = Path(path).resolve(strict=True)
    require(path.is_file(), "database must be an existing regular file")
    c = sqlite3.connect(path.as_uri() + ("?mode=rw" if writable else "?mode=ro"),
                        uri=True, timeout=5, isolation_level=None)
    c.row_factory = sqlite3.Row
    if not writable:
        c.execute("PRAGMA query_only=ON")
    return c


def paper_rows(c):
    require(tuple(r[1] for r in c.execute("PRAGMA table_info(paper_trades)")) == COLS,
            "unexpected original paper schema")
    return {r[0]: tuple(r) for r in c.execute("SELECT * FROM paper_trades ORDER BY id")}


def snapshot(c, tables):
    catalog = [tuple(r) for r in c.execute(
        "SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name")]
    out = {"catalog": hashlib.sha256(compact(catalog)).hexdigest()}
    for table in tables:
        rows = sorted(row_key(r) for r in c.execute('SELECT * FROM "' + table + '"'))
        out[table] = hashlib.sha256(compact(rows)).hexdigest()
    out["sequence"] = tuple(c.execute(
        "SELECT seq FROM sqlite_sequence WHERE name='paper_trades'").fetchone())
    return out


def validate_audit(c, pins):
    rows = [dict(r) for r in c.execute("SELECT * FROM paper_inventory_failure_audit ORDER BY id")]
    chains = list(c.execute("SELECT * FROM paper_inventory_failure_audit_chain ORDER BY failure_audit_id"))
    require(len(rows) == len(chains) == pins.audit_count, "unexpected inventory audit chain length")
    previous = "BR249_PAPER_INVENTORY_FAILURE_AUDIT_GENESIS_V1"
    for row, chain in zip(rows, chains):
        require(row["id"] == chain["failure_audit_id"] and chain["previous_hash"] == previous,
                "inventory chain linkage changed")
        source = hash_fields(b"BR249_PAPER_INVENTORY_SOURCE_SNAPSHOT_V1\0", row["source_facts_json"])
        diagnostic = hash_fields(b"BR249_PAPER_INVENTORY_DIAGNOSTIC_V1\0", row["diagnostic"])
        identity = hash_fields(b"BR249_PAPER_INVENTORY_FAILURE_IDENTITY_V1\0",
                              row["as_of_date"], row["stage"], source, diagnostic)
        require((source, diagnostic, identity) == (row["source_snapshot_hash"],
                row["diagnostic_hash"], row["failure_identity"]), "inventory source identity changed")
        facts = json.loads(row["source_facts_json"])
        require(compact(facts).decode() == row["source_facts_json"] and
                len(facts) == row["source_row_count"] and
                [f["id"] for f in facts] == json.loads(row["source_fill_ids_json"]),
                "inventory source facts changed")
        previous = hash_fields(b"BR249_PAPER_INVENTORY_FAILURE_RECORD_V1\0",
                               previous, compact(row))
        require(previous == chain["record_hash"], "inventory record hash changed")
    require(previous == pins.audit_tip, "inventory audit tip is not the approved original")
    selected = next(r for r in rows if r["id"] == pins.audit_id)
    require(selected["source_snapshot_hash"] == pins.audit_snapshot, "original audit snapshot changed")
    return {f["id"]: f for f in json.loads(selected["source_facts_json"])}


def validate_epoch(c, pins, original):
    row = c.execute("SELECT * FROM attribution_sample_epoch_receipt WHERE id=1").fetchone()
    require(row is not None and row["epoch_id"] == pins.epoch_id and
            row["receipt_hash"] == pins.epoch_receipt and
            row["legacy_filled_manifest_hash"] == pins.legacy_manifest,
            "frozen epoch receipt is not the approved original")
    # Stored hash strings alone do not authenticate the other receipt fields.
    # The SHA-pinned backup supplies the complete immutable receipt/chain/carry.
    for table in ("attribution_sample_epoch_receipt",
                  "attribution_sample_epoch_receipt_chain", "attribution_legacy_carry_item"):
        expected = sorted(row_key(r) for r in original.execute('SELECT * FROM "' + table + '"'))
        actual = sorted(row_key(r) for r in c.execute('SELECT * FROM "' + table + '"'))
        require(actual == expected, "frozen epoch original rows changed: " + table)


def verify_guard(path, pins):
    require(path is not None, "apply requires a verified deployed lifecycle-guard proof")
    p = json.loads(Path(path).read_text())
    require(p.get("schema") == "paper-source-recovery-guard-v1" and
            p.get("disputed_fill_ids") == list(pins.ids) and
            p.get("backup_sha256") == pins.backup_sha256 and
            p.get("net_summary") == "Unavailable" and
            p.get("account_anchor_rejected") is True,
            "guard proof does not cover the complete restored disputed lifecycles")
    for stem in ("monitor", "economic_readback"):
        require(digest_file(p[stem + "_path"]) == p[stem + "_sha256"],
                "guard artifact changed: " + stem)
    readback = json.loads(Path(p["economic_readback_path"]).read_text())
    require(readback.get("mode") == "readonly_guarded_recovery_validation" and
            readback.get("guarded_price_dispute_ids") == list(pins.ids) and
            readback.get("net_summary") == "Unavailable" and
            readback.get("account_anchor_available") is False and
            readback.get("original_count") == pins.original_count,
            "guard readback does not reject the restored complete source lifecycles")
    return digest_file(path)


def recover(database, backup, candidate, *, apply=False, guard_proof=None,
            receipt=None, pins=PINS):
    database, backup, candidate = map(Path, (database, backup, candidate))
    require(database.resolve() != backup.resolve(), "source backup cannot be a write target")
    require(digest_file(backup) == pins.backup_sha256, "original backup SHA256 mismatch")
    require(digest_file(candidate) == pins.candidate_sha256, "original candidate SHA256 mismatch")
    review = json.loads(candidate.read_text())
    require(review.get("applied") is False, "candidate was not an unexecuted original review")
    candidates = review["candidates"]["rows"]
    require(sorted(r["id"] for r in candidates) == list(pins.ids) and
            all(r.get("executed") is False for r in candidates), "candidate set or disposition changed")
    guard_hash = verify_guard(guard_proof, pins) if apply else None
    require(not apply or receipt is not None, "apply requires an exclusive private JSONL receipt")
    original = connect(backup)
    target = None
    output = None
    try:
        original.execute("BEGIN")
        source = paper_rows(original)
        require(len(source) == pins.original_count, "unexpected original row count")
        for item in candidates:
            require(row_key(source[item["id"]]) == row_key(tuple(item[k] for k in COLS)),
                    "candidate no longer matches original full row")
        # The retained file must remain exactly the pinned original while open.
        require(digest_file(backup) == pins.backup_sha256, "original backup changed during read")
        target = connect(database, writable=apply)
        target.execute("BEGIN IMMEDIATE" if apply else "BEGIN")
        current = paper_rows(target)
        missing = sorted(set(source) - set(current))
        require(not (set(current) - set(source)) and
                all(row_key(row) == row_key(source[i]) for i, row in current.items()),
                "unexpected new or changed surviving paper facts; no writes authorized")
        require(missing in ([], list(pins.ids)), "partial or unexpected missing originals")
        facts = validate_audit(target, pins)
        validate_epoch(target, pins, original)
        for i in pins.ids:
            row = dict(zip(COLS, source[i]))
            f = facts[i]
            require((row["code"], row["name"], row["direction"],
                     struct.pack(">d", row["fill_price"]).hex(), row["quantity"], row["ts"]) ==
                    (f["code"], f["name"], f["direction"], f["fill_price_bits"],
                     f["quantity"], f["occurred_at"]), "backup disagrees with original inventory audit")
        before = snapshot(target, pins.protected)
        require(before["sequence"] == (pins.sequence,), "original sequence changed")
        inode = (database.stat().st_dev, database.stat().st_ino)
        result = {"schema": "paper-source-recovery-v1", "mode": "apply" if apply else "preview",
                  "original_count": len(source), "surviving_count": len(current),
                  "restore_ids": missing, "already_restored": not missing,
                  "backup_sha256": pins.backup_sha256, "candidate_sha256": pins.candidate_sha256,
                  "epoch_receipt": pins.epoch_receipt, "guard_proof_sha256": guard_hash,
                  "price_qualification_issued": False, "committed": False}
        if apply:
            fd = os.open(receipt, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            output = os.fdopen(fd, "wb")
            output.write(compact({"event": "intent", **result}) + b"\n")
            output.flush()
            os.fsync(output.fileno())
            sync_receipt_directory(receipt)
            start_changes = target.total_changes
            for i in missing:
                target.execute("INSERT INTO paper_trades VALUES (" + ",".join("?" for _ in COLS) + ")",
                               source[i])
            require(target.total_changes - start_changes == len(missing), "unexpected trigger or extra mutation")
            require({i: row_key(r) for i, r in paper_rows(target).items()} ==
                    {i: row_key(r) for i, r in source.items()}, "restored full rows do not match original")
            require(snapshot(target, pins.protected) == before, "protected catalog/account/audit/epoch changed")
            require((database.stat().st_dev, database.stat().st_ino) == inode, "target database was replaced")
            require(digest_file(candidate) == pins.candidate_sha256 and
                    verify_guard(guard_proof, pins) == guard_hash, "evidence changed before commit")
            target.commit()
            result["committed"] = True
            output.write(compact({"event": "committed", **result}) + b"\n")
            output.flush()
            os.fsync(output.fileno())
        else:
            target.rollback()
        return result
    finally:
        if target is not None:
            if target.in_transaction:
                target.rollback()
            target.close()
        original.rollback()
        original.close()
        if output is not None:
            output.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True, type=Path)
    parser.add_argument("--backup", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--guard-proof", type=Path)
    parser.add_argument("--receipt", type=Path)
    args = parser.parse_args()
    try:
        result = recover(args.db, args.backup, args.candidate, apply=args.apply,
                         guard_proof=args.guard_proof, receipt=args.receipt)
        print(json.dumps(result, ensure_ascii=False, sort_keys=True))
        return 0
    except (ValueError, KeyError, StopIteration, OSError, sqlite3.Error) as error:
        print("paper source recovery stopped: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
