#!/usr/bin/env python3
"""Read-only BR-194 production review/replay authority verifier."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import stat
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DATABASE = ROOT / "data/durable_delivery.sqlite3"
EXPECTED_SCHEMA_VERSION = 5
MANIFEST = {
    "database": "data/durable_delivery.sqlite3",
    "durable_audit_dir": "data/durable_delivery_audit/",
    "push_log_dir": "data/push_log/",
    "delivery_audit_dir": "data/event_audit/",
    "attempt_table": "review_terminal_replay_attempts",
    "completion_table": "review_terminal_replay_completions",
    "start_audit_kind": "ReviewTerminalReplayStarted",
    "completion_audit_kind": "ReviewTerminalReplayCompleted",
    "attempt_identity_domain": "BR-194-terminal-replay-attempt-v1",
    "audit_identity_domain": "delivery-critical-audit-v1",
    "audit_attempt_binding": "NONE",
}

EXPECTED_REPLAY_COLUMNS = {
    "review_terminal_replay_attempts": [
        ("attempt_identity", "TEXT", 0, 1),
        ("business_date", "TEXT", 1, 0),
        ("review_task", "TEXT", 1, 0),
        ("task_identity", "TEXT", 1, 0),
        ("decision_identity", "TEXT", 1, 0),
        ("replay_ordinal", "INTEGER", 1, 0),
        ("started_at", "TEXT", 1, 0),
        ("pre_sink_count", "INTEGER", 1, 0),
        ("pre_sink_set_sha256", "TEXT", 1, 0),
        ("pre_delivery_audit_count", "INTEGER", 1, 0),
        ("pre_delivery_audit_set_sha256", "TEXT", 1, 0),
        ("provider_calls", "INTEGER", 1, 0),
        ("start_canonical", "BLOB", 1, 0),
        ("start_sha256", "TEXT", 1, 0),
        ("start_audit_identity", "TEXT", 1, 0),
    ],
    "review_terminal_replay_completions": [
        ("attempt_identity", "TEXT", 0, 1),
        ("decision_identity", "TEXT", 1, 0),
        ("state", "TEXT", 1, 0),
        ("completed_at", "TEXT", 1, 0),
        ("post_sink_count", "INTEGER", 1, 0),
        ("post_sink_set_sha256", "TEXT", 1, 0),
        ("post_delivery_audit_count", "INTEGER", 1, 0),
        ("post_delivery_audit_set_sha256", "TEXT", 1, 0),
        ("provider_calls", "INTEGER", 1, 0),
        ("resume_calls", "INTEGER", 1, 0),
        ("sink_calls", "INTEGER", 1, 0),
        ("delivery_audit_appends", "INTEGER", 1, 0),
        ("reason_code", "TEXT", 1, 0),
        ("completion_canonical", "BLOB", 1, 0),
        ("completion_sha256", "TEXT", 1, 0),
        ("completion_audit_identity", "TEXT", 1, 0),
    ],
}

EXPECTED_REPLAY_TRIGGER_SQL = {
    "validate_review_terminal_replay_attempt_audit_insert": """
        CREATE TRIGGER validate_review_terminal_replay_attempt_audit_insert
        BEFORE INSERT ON review_terminal_replay_attempts
        WHEN NOT EXISTS(
          SELECT 1 FROM immutable_audit_outbox audit
          WHERE audit.audit_identity=NEW.start_audit_identity
            AND audit.decision_identity=NEW.decision_identity
            AND audit.attempt_identity IS NULL
            AND audit.audit_kind='ReviewTerminalReplayStarted'
            AND audit.audit_canonical=NEW.start_canonical
            AND audit.audit_sha256=NEW.start_sha256
            AND sha256_hex(NEW.start_canonical)=NEW.start_sha256
            AND sha256_hex(audit.audit_canonical)=audit.audit_sha256
        )
        BEGIN
          SELECT RAISE(ABORT,'review terminal replay start audit mismatch');
        END
    """,
    "validate_review_terminal_replay_completion_audit_insert": """
        CREATE TRIGGER validate_review_terminal_replay_completion_audit_insert
        BEFORE INSERT ON review_terminal_replay_completions
        WHEN NOT EXISTS(
          SELECT 1 FROM immutable_audit_outbox audit
          WHERE audit.audit_identity=NEW.completion_audit_identity
            AND audit.decision_identity=NEW.decision_identity
            AND audit.attempt_identity IS NULL
            AND audit.audit_kind='ReviewTerminalReplayCompleted'
            AND audit.audit_canonical=NEW.completion_canonical
            AND audit.audit_sha256=NEW.completion_sha256
            AND sha256_hex(NEW.completion_canonical)=NEW.completion_sha256
            AND sha256_hex(audit.audit_canonical)=audit.audit_sha256
        )
        BEGIN
          SELECT RAISE(ABORT,'review terminal replay completion audit mismatch');
        END
    """,
    "immutable_review_terminal_replay_attempt_update": """
        CREATE TRIGGER immutable_review_terminal_replay_attempt_update
        BEFORE UPDATE ON review_terminal_replay_attempts
        BEGIN
          SELECT RAISE(ABORT,'review terminal replay attempts are immutable');
        END
    """,
    "immutable_review_terminal_replay_attempt_delete": """
        CREATE TRIGGER immutable_review_terminal_replay_attempt_delete
        BEFORE DELETE ON review_terminal_replay_attempts
        BEGIN
          SELECT RAISE(ABORT,'review terminal replay attempts are retained');
        END
    """,
    "immutable_review_terminal_replay_completion_update": """
        CREATE TRIGGER immutable_review_terminal_replay_completion_update
        BEFORE UPDATE ON review_terminal_replay_completions
        BEGIN
          SELECT RAISE(ABORT,'review terminal replay completions are immutable');
        END
    """,
    "immutable_review_terminal_replay_completion_delete": """
        CREATE TRIGGER immutable_review_terminal_replay_completion_delete
        BEFORE DELETE ON review_terminal_replay_completions
        BEGIN
          SELECT RAISE(ABORT,'review terminal replay completions are retained');
        END
    """,
}


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"BR-194 review join verification failed: {message}")


def sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def stable_identity(domain: str, parts: list[str]) -> str:
    digest = hashlib.sha256()
    encoded_domain = domain.encode()
    digest.update(len(encoded_domain).to_bytes(8, "big"))
    digest.update(encoded_domain)
    for part in parts:
        encoded = part.encode()
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
    return digest.hexdigest()


def canonical_bytes(value: object) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), allow_nan=False
    ).encode()


def sha256_domain(domain: str, payload: bytes) -> str:
    return sha256(domain.encode() + b"\0" + payload)


def review_task_identity(business_date: str, task: str) -> str:
    return sha256(
        b"stock_analysis/review/v1\0"
        + b"review-task\0"
        + f"{business_date}:{task}".encode()
    )


def read_regular_nofollow(path: Path) -> tuple[bytes, tuple[int, int, int, str]]:
    try:
        relative = path.relative_to(ROOT)
    except ValueError:
        fail(f"authority escaped repository root: {path}")
    current = ROOT
    for component in relative.parts:
        current = current / component
        metadata = current.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            fail(f"symlink authority is forbidden: {current}")
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags)
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            fail(f"authority is not a regular file: {path}")
        chunks: list[bytes] = []
        while True:
            chunk = os.read(descriptor, 1024 * 1024)
            if not chunk:
                break
            chunks.append(chunk)
        payload = b"".join(chunks)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    if (
        before.st_dev,
        before.st_ino,
        before.st_size,
    ) != (
        after.st_dev,
        after.st_ino,
        after.st_size,
    ) or len(payload) != before.st_size:
        fail(f"authority changed while being read: {path}")
    return payload, (before.st_dev, before.st_ino, before.st_size, sha256(payload))


def read_optional_regular_nofollow(
    path: Path,
) -> tuple[bytes, tuple[int, int, int, str]] | None:
    try:
        return read_regular_nofollow(path)
    except FileNotFoundError:
        return None


def assert_authority_unchanged(
    path: Path,
    expected: tuple[bytes, tuple[int, int, int, str]] | None,
) -> None:
    observed = read_optional_regular_nofollow(path)
    if observed != expected:
        fail(f"production authority changed during verification: {path}")


def copy_database_authority(
    temporary_root: Path,
) -> tuple[Path, dict[Path, tuple[bytes, tuple[int, int, int, str]] | None]]:
    snapshots: dict[Path, tuple[bytes, tuple[int, int, int, str]] | None] = {}
    database_snapshot = read_regular_nofollow(DATABASE)
    snapshots[DATABASE] = database_snapshot
    for suffix in ("-wal", "-shm"):
        path = Path(f"{DATABASE}{suffix}")
        snapshots[path] = read_optional_regular_nofollow(path)

    copied_database = temporary_root / DATABASE.name
    copied_database.write_bytes(database_snapshot[0])
    wal = snapshots[Path(f"{DATABASE}-wal")]
    if wal is not None:
        Path(f"{copied_database}-wal").write_bytes(wal[0])
    return copied_database, snapshots


def matching_regular_files(root: Path, names: set[str]) -> list[Path]:
    try:
        relative = root.relative_to(ROOT)
    except ValueError:
        fail(f"directory authority escaped repository root: {root}")
    current = ROOT
    for component in relative.parts:
        current = current / component
        metadata = current.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            fail(f"directory authority is not a real directory: {current}")

    matches: list[Path] = []
    pending = [root]
    while pending:
        directory = pending.pop()
        with os.scandir(directory) as entries:
            for entry in entries:
                if entry.is_symlink():
                    fail(f"symlink inside authority is forbidden: {entry.path}")
                if entry.is_dir(follow_symlinks=False):
                    pending.append(Path(entry.path))
                elif entry.is_file(follow_symlinks=False) and entry.name in names:
                    matches.append(Path(entry.path))
    return sorted(matches)


def verify_immutable_append(
    expected: dict[str, tuple[str, bytes, str, str]]
) -> dict[Path, tuple[bytes, tuple[int, int, int, str]]]:
    path = ROOT / MANIFEST["durable_audit_dir"] / "durable_delivery_v1.jsonl"
    payload, snapshot = read_regular_nofollow(path)
    previous: str | None = None
    observed: dict[str, dict] = {}
    for line_number, line in enumerate(payload.splitlines(), 1):
        if not line:
            fail(f"blank durable append line {line_number}")
        record = json.loads(line)
        if set(record) != {
            "hash_domain",
            "record_kind",
            "identity",
            "canonical_hex",
            "canonical_sha256",
            "previous_hash",
            "record_hash",
        }:
            fail(f"durable append field set changed at line {line_number}")
        if (
            record["hash_domain"]
            != "stock_analysis.durable_delivery_immutable_append.v1"
            or record["previous_hash"] != previous
        ):
            fail(f"durable append chain mismatch at line {line_number}")
        canonical = bytes.fromhex(record["canonical_hex"])
        material = {
            "hash_domain": record["hash_domain"],
            "record_kind": record["record_kind"],
            "identity": record["identity"],
            "canonical_hex": record["canonical_hex"],
            "canonical_sha256": record["canonical_sha256"],
            "previous_hash": record["previous_hash"],
        }
        if (
            sha256(canonical) != record["canonical_sha256"]
            or sha256(canonical_bytes(material)) != record["record_hash"]
        ):
            fail(f"durable append hash mismatch at line {line_number}")
        previous = record["record_hash"]
        if record["identity"] in observed:
            fail(f"duplicate durable append identity: {record['identity']}")
        observed[record["identity"]] = record

    for identity, (kind, canonical, canonical_hash, immutable_ref) in expected.items():
        record = observed.get(identity)
        if (
            record is None
            or record["record_kind"] != kind
            or bytes.fromhex(record["canonical_hex"]) != canonical
            or record["canonical_sha256"] != canonical_hash
            or immutable_ref != f"durable-delivery:{record['record_hash']}"
        ):
            fail(f"durable immutable append join mismatch: {identity}")
    return {path: (payload, snapshot)}


def counted_join_hash(
    kind: str,
    outcome: str,
    channel: str,
    decision_hash: str,
    attempt_hash: str,
    artifact_hash: str,
    result_hash: str,
    receipt_hash: str,
) -> str:
    digest = hashlib.sha256(b"stock_analysis.counted_delivery_join.v1")
    for value in (
        kind,
        outcome,
        channel,
        decision_hash,
        attempt_hash,
        artifact_hash,
        result_hash,
        receipt_hash,
    ):
        digest.update(b"\0")
        digest.update(value.encode())
    return digest.hexdigest()


def verify_counted_push_and_delivery_audit(
    evidence: dict,
) -> dict[Path, tuple[bytes, tuple[int, int, int, str]]]:
    envelope = evidence["envelope"]
    decision_identity = evidence["decision_identity"]
    attempt_identity = evidence["delivery_attempt_identity"]
    result = evidence["result"]
    receipt = evidence["receipt"]
    decision_hash = sha256_domain(
        "stock_analysis.counted_decision_identity.v1", decision_identity.encode()
    )
    attempt_hash = sha256_domain(
        "stock_analysis.counted_attempt_identity.v1", attempt_identity.encode()
    )
    prefix = f"{decision_hash}_{attempt_hash}"
    pending_name = f"{prefix}_audit_pending.json"
    commit_name = f"{prefix}_committed.json"
    files = matching_regular_files(
        ROOT / MANIFEST["push_log_dir"], {pending_name, commit_name}
    )
    if len(files) != 2 or {path.name for path in files} != {pending_name, commit_name}:
        fail("counted delivery does not have exactly one pending/commit artifact pair")
    snapshots = {path: read_regular_nofollow(path) for path in files}
    pending_bytes = next(
        snapshots[path][0] for path in files if path.name == pending_name
    )
    commit_bytes = next(snapshots[path][0] for path in files if path.name == commit_name)
    pending = json.loads(pending_bytes)
    commit = json.loads(commit_bytes)
    if canonical_bytes(pending) != pending_bytes or canonical_bytes(commit) != commit_bytes:
        fail("counted push artifacts are not byte-exact canonical JSON")
    result_domain_hash = sha256_domain(
        "stock_analysis.counted_sink_result.v1", canonical_bytes(result)
    )
    receipt_domain_hash = sha256_domain(
        "stock_analysis.counted_receipt.v1", canonical_bytes(receipt)
    )
    artifact_hash = sha256_domain(
        "stock_analysis.counted_push_log_artifact.v1", pending_bytes
    )
    rendered = bytes(envelope["rendered_content"]).decode("utf-8")
    expected_kind = envelope["push_kind"]
    expected_template = (
        "review_lhb_v1"
        if evidence["review_task"] == "R-04"
        else "review_provider_top_n_v1"
    )
    if (
        pending.get("schema") != "stock_analysis.counted_push_log.v1"
        or pending.get("state") != "AuditPending"
        or pending.get("durable_push_kind") != expected_kind
        or pending.get("stable_template_id") != expected_template
        or pending.get("decision_identity") != decision_identity
        or pending.get("attempt_identity") != attempt_identity
        or pending.get("decision_identity_hash") != decision_hash
        or pending.get("attempt_identity_hash") != attempt_hash
        or pending.get("rendered_content_sha256")
        != envelope["rendered_content_sha256"]
        or pending.get("rendered_content") != rendered
        or pending.get("sink_result") != result
        or pending.get("sink_result_sha256") != result_domain_hash
        or pending.get("receipt_sha256") != receipt_domain_hash
    ):
        fail("counted pending artifact join mismatch")
    join_hash = counted_join_hash(
        expected_kind,
        "Pushed",
        evidence["channel"],
        decision_hash,
        attempt_hash,
        artifact_hash,
        result_domain_hash,
        receipt_domain_hash,
    )
    if (
        commit.get("schema") != "stock_analysis.counted_push_log.v1"
        or commit.get("state") != "Committed"
        or commit.get("durable_push_kind") != expected_kind
        or commit.get("stable_template_id") != expected_template
        or commit.get("decision_identity_hash") != decision_hash
        or commit.get("attempt_identity_hash") != attempt_hash
        or commit.get("pending_artifact_sha256") != artifact_hash
        or commit.get("delivery_audit_event_id") != join_hash
        or commit.get("counted_join_hash") != join_hash
    ):
        fail("counted commit artifact join mismatch")

    audit_files = matching_regular_files(
        ROOT / MANIFEST["delivery_audit_dir"],
        {f"{str(receipt['accepted_at'])[:4]}.jsonl"},
    )
    if len(audit_files) != 1:
        fail("counted delivery audit year authority is not unique")
    audit_path = audit_files[0]
    audit_bytes, audit_snapshot = read_regular_nofollow(audit_path)
    previous = "GENESIS"
    matching: list[dict] = []
    for line_number, line in enumerate(audit_bytes.splitlines(), 1):
        if not line:
            fail(f"blank delivery-audit line {line_number}")
        record = json.loads(line)
        if record.get("previous_hash") != previous:
            fail(f"delivery-audit chain mismatch at line {line_number}")
        stored_hash = record.get("record_hash")
        material = dict(record)
        material.pop("record_hash", None)
        digest = hashlib.sha256()
        if material.get("hash_domain") == "stock_analysis.delivery_audit_record.v2":
            digest.update(b"stock_analysis.delivery_audit_record.v2\0")
        elif "hash_domain" in material:
            fail(f"unsupported delivery-audit hash domain at line {line_number}")
        digest.update(canonical_bytes(material))
        if digest.hexdigest() != stored_hash:
            fail(f"delivery-audit hash mismatch at line {line_number}")
        previous = stored_hash
        if record.get("envelope", {}).get("id") == join_hash:
            matching.append(record["envelope"])
    if len(matching) != 1:
        fail("counted delivery audit event is not unique")
    audit = matching[0]
    payload = audit.get("payload", {})
    if (
        audit.get("event_type") != "push.delivery.audit"
        or payload.get("audit_schema_version") != 3
        or payload.get("kind") != expected_kind
        or payload.get("outcome") != "Pushed"
        or payload.get("channel") != evidence["channel"]
        or payload.get("subject_hash") != decision_hash
        or payload.get("decision_identity_hash") != decision_hash
        or payload.get("attempt_identity_hash") != attempt_hash
        or payload.get("artifact_sha256") != artifact_hash
        or payload.get("sink_result_sha256") != result_domain_hash
        or payload.get("receipt_sha256") != receipt_domain_hash
        or payload.get("counted_join_hash") != join_hash
    ):
        fail("schema-v3 counted delivery audit join mismatch")
    return {
        **{path: snapshot for path, snapshot in snapshots.items()},
        audit_path: (audit_bytes, audit_snapshot),
    }


def watermark(connection: sqlite3.Connection, decision: str, delivery: bool) -> dict:
    if delivery:
        rows = connection.execute(
            """
            SELECT result_event_identity,delivery_audit_ref,
                   frozen_delivery_audit_sha256
            FROM sink_results
            WHERE decision_identity=?
              AND delivery_audit_ref IS NOT NULL
              AND frozen_delivery_audit_sha256 IS NOT NULL
            ORDER BY result_event_identity ASC
            """,
            (decision,),
        ).fetchall()
        payload = [
            {
                "result_event_identity": row[0],
                "delivery_audit_ref": row[1],
                "frozen_delivery_audit_sha256": row[2],
            }
            for row in rows
        ]
    else:
        rows = connection.execute(
            """
            SELECT result_event_identity,attempt_identity,result_sha256
            FROM sink_results
            WHERE decision_identity=?
            ORDER BY result_event_identity ASC
            """,
            (decision,),
        ).fetchall()
        payload = [
            {
                "result_event_identity": row[0],
                "attempt_identity": row[1],
                "result_sha256": row[2],
            }
            for row in rows
        ]
    return {
        "count": len(payload),
        "ordered_identity_set_sha256": sha256(canonical_bytes(payload)),
    }


def normalize_schema_sql(value: str) -> str:
    normalized = " ".join(value.replace("IF NOT EXISTS", "").split())
    return normalized.removesuffix(";").strip().lower()


def verify_schema(connection: sqlite3.Connection) -> None:
    schema_version = connection.execute("PRAGMA user_version").fetchone()[0]
    if schema_version != EXPECTED_SCHEMA_VERSION:
        fail(
            f"durable schema version mismatch: expected {EXPECTED_SCHEMA_VERSION}, "
            f"got {schema_version}"
        )
    triggers = {
        row[0]: row[1]
        for row in connection.execute(
            "SELECT name,sql FROM sqlite_master WHERE type='trigger'"
        )
    }
    missing = sorted(EXPECTED_REPLAY_TRIGGER_SQL.keys() - triggers.keys())
    if missing:
        fail(f"missing replay triggers: {missing}")
    for name, expected_sql in EXPECTED_REPLAY_TRIGGER_SQL.items():
        actual = normalize_schema_sql(triggers[name] or "")
        expected = normalize_schema_sql(expected_sql)
        if actual != expected:
            fail(f"replay trigger SQL mismatch: {name}")

    for table, expected in EXPECTED_REPLAY_COLUMNS.items():
        actual = [
            (row[1], row[2].upper(), row[3], row[5])
            for row in connection.execute(f"PRAGMA table_info({table})")
        ]
        if actual != expected:
            fail(f"replay table column contract mismatch: {table}")

    replay_table_sql = {
        row[0]: normalize_schema_sql(row[1] or "")
        for row in connection.execute(
            """
            SELECT name,sql FROM sqlite_master
            WHERE type='table' AND name IN (
              'review_terminal_replay_attempts',
              'review_terminal_replay_completions'
            )
            """
        )
    }
    table_constraints = {
        "review_terminal_replay_attempts": (
            "CHECK(review_task IN ('R-04','R-09'))",
            "CHECK(replay_ordinal > 0)",
            "CHECK(provider_calls = 0)",
            "UNIQUE(attempt_identity,decision_identity)",
            """
            UNIQUE(
              business_date,review_task,task_identity,decision_identity,replay_ordinal
            )
            """,
        ),
        "review_terminal_replay_completions": (
            "CHECK(state IN ('Passed','Failed'))",
            "CHECK(provider_calls = 0)",
            "CHECK(resume_calls >= 0)",
            "CHECK(sink_calls >= 0)",
            "CHECK(delivery_audit_appends >= 0)",
            """
            state != 'Passed'
            OR (
              resume_calls=0 AND sink_calls=0 AND delivery_audit_appends=0
            )
            """,
        ),
    }
    for table, required_fragments in table_constraints.items():
        sql = replay_table_sql.get(table, "")
        if not sql or any(
            normalize_schema_sql(fragment) not in sql
            for fragment in required_fragments
        ):
            fail(f"replay table CHECK/UNIQUE contract mismatch: {table}")

    foreign_keys = connection.execute(
        "PRAGMA foreign_key_list(review_terminal_replay_completions)"
    ).fetchall()
    composite = [
        row
        for row in foreign_keys
        if row[2] == "review_terminal_replay_attempts"
    ]
    if sorted((row[3], row[4]) for row in composite) != [
        ("attempt_identity", "attempt_identity"),
        ("decision_identity", "decision_identity"),
    ]:
        fail("completion composite attempt+decision FK is absent or weakened")
    for table, column in (
        ("review_terminal_replay_attempts", "start_audit_identity"),
        ("review_terminal_replay_completions", "completion_audit_identity"),
    ):
        keys = connection.execute(f"PRAGMA foreign_key_list({table})").fetchall()
        if not any(
            row[2] == "immutable_audit_outbox"
            and row[3] == column
            and row[4] == "audit_identity"
            for row in keys
        ):
            fail(f"missing exact immutable-audit FK {table}.{column}")


def verify_connection(
    connection: sqlite3.Connection, args: argparse.Namespace
) -> dict:
    verify_schema(connection)
    expected_task_identity = review_task_identity(args.business_date, args.task)

    attempt_row = connection.execute(
        """
        SELECT attempt_identity,business_date,review_task,task_identity,
               decision_identity,replay_ordinal,started_at,
               pre_sink_count,pre_sink_set_sha256,
               pre_delivery_audit_count,pre_delivery_audit_set_sha256,
               provider_calls,start_canonical,start_sha256,start_audit_identity
        FROM review_terminal_replay_attempts
        WHERE business_date=? AND review_task=? AND task_identity=?
        ORDER BY replay_ordinal DESC
        LIMIT 1
        """,
        (args.business_date, args.task, expected_task_identity),
    ).fetchone()
    if attempt_row is None:
        fail("latest replay attempt is missing")
    (
        attempt_identity,
        business_date,
        review_task,
        task_identity,
        decision_identity,
        ordinal,
        started_at,
        pre_sink_count,
        pre_sink_hash,
        pre_audit_count,
        pre_audit_hash,
        provider_calls,
        start_canonical,
        start_hash,
        start_audit_identity,
    ) = attempt_row
    if task_identity != expected_task_identity:
        fail("review task identity is not the deterministic BR-140 root")
    expected_attempt = stable_identity(
        MANIFEST["attempt_identity_domain"],
        [
            business_date,
            review_task,
            task_identity,
            decision_identity,
            str(ordinal),
        ],
    )
    if attempt_identity != expected_attempt:
        fail("attempt identity mismatch")
    if sha256(start_canonical) != start_hash:
        fail("start canonical hash mismatch")
    start = json.loads(start_canonical)
    expected_start = {
        "schema_version": 1,
        "attempt_identity": attempt_identity,
        "business_date": business_date,
        "review_task": review_task,
        "task_identity": task_identity,
        "decision_identity": decision_identity,
        "replay_ordinal": ordinal,
        "started_at": started_at,
        "pre_sink_watermark": {
            "count": pre_sink_count,
            "ordered_identity_set_sha256": pre_sink_hash,
        },
        "pre_delivery_audit_watermark": {
            "count": pre_audit_count,
            "ordered_identity_set_sha256": pre_audit_hash,
        },
        "provider_calls": provider_calls,
    }
    if start != expected_start or canonical_bytes(start) != start_canonical:
        fail("start canonical is not byte-exact typed evidence")

    completion_row = connection.execute(
        """
        SELECT state,completed_at,post_sink_count,post_sink_set_sha256,
               post_delivery_audit_count,post_delivery_audit_set_sha256,
               provider_calls,resume_calls,sink_calls,delivery_audit_appends,
               reason_code,completion_canonical,completion_sha256,
               completion_audit_identity,decision_identity
        FROM review_terminal_replay_completions
        WHERE attempt_identity=?
        """,
        (attempt_identity,),
    ).fetchone()
    if completion_row is None:
        fail("latest attempt has no completion")
    (
        state,
        completed_at,
        post_sink_count,
        post_sink_hash,
        post_audit_count,
        post_audit_hash,
        completion_provider_calls,
        resume_calls,
        sink_calls,
        delivery_audit_appends,
        reason_code,
        completion_canonical,
        completion_hash,
        completion_audit_identity,
        completion_decision,
    ) = completion_row
    if completion_decision != decision_identity or state != "Passed":
        fail("latest completion is not Passed for the same decision")
    if sha256(completion_canonical) != completion_hash:
        fail("completion canonical hash mismatch")
    completion = json.loads(completion_canonical)
    expected_completion = {
        "schema_version": 1,
        "attempt_identity": attempt_identity,
        "decision_identity": decision_identity,
        "state": "Passed",
        "completed_at": completed_at,
        "post_sink_watermark": {
            "count": post_sink_count,
            "ordered_identity_set_sha256": post_sink_hash,
        },
        "post_delivery_audit_watermark": {
            "count": post_audit_count,
            "ordered_identity_set_sha256": post_audit_hash,
        },
        "provider_calls": completion_provider_calls,
        "resume_calls": resume_calls,
        "sink_calls": sink_calls,
        "delivery_audit_appends": delivery_audit_appends,
        "reason_code": reason_code,
    }
    if completion != expected_completion or canonical_bytes(completion) != completion_canonical:
        fail("completion canonical is not byte-exact typed evidence")

    current_sink = watermark(connection, decision_identity, False)
    current_audit = watermark(connection, decision_identity, True)
    expected_sink = {
        "count": pre_sink_count,
        "ordered_identity_set_sha256": pre_sink_hash,
    }
    expected_audit = {
        "count": pre_audit_count,
        "ordered_identity_set_sha256": pre_audit_hash,
    }
    if (
        expected_sink != current_sink
        or expected_audit != current_audit
        or post_sink_count != pre_sink_count
        or post_sink_hash != pre_sink_hash
        or post_audit_count != pre_audit_count
        or post_audit_hash != pre_audit_hash
        or pre_sink_count != 1
        or pre_audit_count != 1
        or any(
            value != 0
            for value in (
                provider_calls,
                completion_provider_calls,
                resume_calls,
                sink_calls,
                delivery_audit_appends,
            )
        )
        or reason_code != "existing_terminal_hydrated"
    ):
        fail("Passed replay counters or authority watermarks are invalid")

    immutable_expected: dict[str, tuple[str, bytes, str, str]] = {}
    for audit_identity, kind, canonical, canonical_hash in (
        (
            start_audit_identity,
            MANIFEST["start_audit_kind"],
            start_canonical,
            start_hash,
        ),
        (
            completion_audit_identity,
            MANIFEST["completion_audit_kind"],
            completion_canonical,
            completion_hash,
        ),
    ):
        audit = connection.execute(
            """
            SELECT decision_identity,attempt_identity,audit_kind,audit_canonical,
                   audit_sha256,append_state,immutable_audit_ref
            FROM immutable_audit_outbox
            WHERE audit_identity=?
            """,
            (audit_identity,),
        ).fetchone()
        if audit is None:
            fail(f"replay audit missing: {kind}")
        expected_audit_identity = stable_identity(
            MANIFEST["audit_identity_domain"],
            [decision_identity, "NONE", kind, canonical_hash],
        )
        if (
            audit_identity != expected_audit_identity
            or audit[0] != decision_identity
            or audit[1] is not None
            or audit[2] != kind
            or audit[3] != canonical
            or audit[4] != canonical_hash
            or audit[5] != "Appended"
            or not audit[6]
        ):
            fail(f"replay audit join mismatch: {kind}")
        immutable_expected[audit_identity] = (kind, canonical, canonical_hash, audit[6])

    decision = connection.execute(
        """
        SELECT state,envelope_canonical,envelope_sha256
        FROM delivery_decisions WHERE decision_identity=?
        """,
        (decision_identity,),
    ).fetchone()
    if decision is None or decision[0] != "Delivered":
        fail("replayed decision is not Delivered")
    if sha256(decision[1]) != decision[2]:
        fail("stored envelope hash mismatch")
    envelope = json.loads(decision[1])
    binding = envelope.get("task_binding")
    source_canonical = bytes(envelope.get("source_binding_canonical", []))
    transition_basis = (
        bytes(binding.get("transition_basis_canonical", []))
        if isinstance(binding, dict)
        else b""
    )
    if (
        canonical_bytes(envelope) != decision[1]
        or envelope.get("business_date") != business_date
        or envelope.get("decision_identity") != decision_identity
        or envelope.get("schedule_occurrence_identity") != task_identity
        or sha256(source_canonical) != envelope.get("source_binding_sha256")
        or sha256(bytes(envelope.get("rendered_content", [])))
        != envelope.get("rendered_content_sha256")
        or not isinstance(binding, dict)
        or binding.get("task_identity") != task_identity
        or sha256(transition_basis) != binding.get("transition_basis_sha256")
    ):
        fail("stored envelope/task binding mismatch")
    decision_material = {
        "domain": "durable-delivery-decision-v1",
        "policy_version": envelope["policy_version"],
        "business_date": envelope["business_date"],
        "push_kind": envelope["push_kind"],
        "sub_kind": envelope["sub_kind"],
        "cooldown_scope": envelope["cooldown_scope"],
        "scope_key": envelope["scope_key"],
        "schedule_occurrence_identity": envelope["schedule_occurrence_identity"],
        "source_evidence_fingerprint": envelope["source_evidence_fingerprint"],
        "delivery_subject_hash": envelope["delivery_subject_hash"],
        "rendered_content_sha256": envelope["rendered_content_sha256"],
    }
    if sha256(canonical_bytes(decision_material)) != decision_identity:
        fail("durable decision identity material mismatch")

    source_binding = json.loads(source_canonical)
    expected_template = (
        "review_lhb_v1" if review_task == "R-04" else "review_provider_top_n_v1"
    )
    if (
        canonical_bytes(source_binding) != source_canonical
        or source_binding.get("business_date") != business_date
        or source_binding.get("template_id") != expected_template
        or source_binding.get("review_task_identity") != task_identity
        or source_binding.get("delivery_subject_identity")
        != envelope.get("delivery_subject_hash")
        or source_binding.get("rendered_content_sha256")
        != envelope.get("rendered_content_sha256")
    ):
        fail("producer source binding is not canonical or task-bound")
    basis = source_binding.get("task_transition_basis")
    if (
        not isinstance(basis, dict)
        or canonical_bytes(basis) != transition_basis
        or basis.get("task_identity") != task_identity
        or basis.get("business_date") != business_date
        or basis.get("task") != review_task
    ):
        fail("producer transition basis is not byte-exact")
    ordered_batches = basis.get("batch_ids")
    expected_batch_count = 1 if review_task == "R-04" else 2
    if (
        not isinstance(ordered_batches, list)
        or len(ordered_batches) != expected_batch_count
        or ordered_batches != envelope.get("original_batch_ids")
        or any(not isinstance(value, str) or not value for value in ordered_batches)
    ):
        fail("producer batch binding count/order mismatch")
    if review_task == "R-04":
        if source_binding.get("evidence", {}).get("batch_id") != ordered_batches[0]:
            fail("R-04 accepted batch does not match source evidence")
    else:
        r09_batches = [
            source_binding.get("volume_ratio_batch", {}).get("batch_id"),
            source_binding.get("main_net_inflow_batch", {}).get("batch_id"),
        ]
        if r09_batches != ordered_batches:
            fail("R-09 accepted batch order does not match source evidence")

    hydration = connection.execute(
        """
        SELECT transition_identity,disposition_identity,task_binding_sha256,
               transition_canonical,transition_sha256,append_state,
               immutable_audit_ref,hydration_state
        FROM task_transition_payloads WHERE decision_identity=?
        """,
        (decision_identity,),
    ).fetchall()
    if len(hydration) != 1:
        fail("terminal task hydration is not uniquely Applied")
    (
        transition_identity,
        disposition_identity,
        task_binding_hash,
        transition_canonical,
        transition_hash,
        transition_append_state,
        transition_audit_ref,
        hydration_state,
    ) = hydration[0]
    transition = json.loads(transition_canonical)
    expected_transition_identity = stable_identity(
        "BR-140-disposition-v1",
        [
            task_identity,
            decision_identity,
            transition["source_identity"],
            transition["task_disposition"],
        ],
    )
    if (
        hydration_state != "Applied"
        or transition_append_state != "Appended"
        or not transition_audit_ref
        or canonical_bytes(transition) != transition_canonical
        or sha256(transition_canonical) != transition_hash
        or transition_identity != expected_transition_identity
        or transition.get("transition_identity") != transition_identity
        or transition.get("task_identity") != task_identity
        or transition.get("decision_identity") != decision_identity
        or transition.get("source_identity") != basis.get("source")
        or transition.get("task_disposition") != "Accepted"
        or transition.get("task_binding_sha256") != binding["transition_basis_sha256"]
        or task_binding_hash != binding["transition_basis_sha256"]
        or transition.get("generic_disposition_identity") != disposition_identity
    ):
        fail("BR-140 task transition identity/hydration join mismatch")
    disposition = connection.execute(
        """
        SELECT disposition,disposition_canonical,disposition_sha256,
               append_state,immutable_audit_ref
        FROM delivery_disposition_payloads
        WHERE disposition_identity=? AND decision_identity=?
        """,
        (disposition_identity, decision_identity),
    ).fetchone()
    if (
        disposition is None
        or disposition[0] != "Accepted"
        or sha256(disposition[1]) != disposition[2]
        or disposition[2] != transition.get("generic_disposition_sha256")
        or disposition[3] != "Appended"
        or not disposition[4]
    ):
        fail("accepted generic disposition join mismatch")
    immutable_expected[disposition_identity] = (
        "DeliveryDisposition",
        disposition[1],
        disposition[2],
        disposition[4],
    )
    immutable_expected[transition_identity] = (
        "BR-140TaskTransition",
        transition_canonical,
        transition_hash,
        transition_audit_ref,
    )

    accepted = connection.execute(
        """
        SELECT result_event_identity,attempt_identity,result_kind,
               authoritative_for_state,late_after_fence,result_canonical,
               result_sha256,channel,provider,message_id,platform_message_id,
               accepted_at,latency_ms,frozen_delivery_audit_canonical,
               frozen_delivery_audit_sha256,delivery_audit_ref
        FROM sink_results WHERE decision_identity=?
        """,
        (decision_identity,),
    ).fetchall()
    if len(accepted) != 1:
        fail("durable decision does not have exactly one sink result")
    (
        result_event_identity,
        delivery_attempt_identity,
        result_kind,
        authoritative_for_state,
        late_after_fence,
        result_canonical,
        result_hash,
        channel,
        provider,
        message_id,
        platform_message_id,
        accepted_at,
        latency_ms,
        frozen_audit,
        frozen_audit_hash,
        delivery_audit_ref,
    ) = accepted[0]
    result = json.loads(result_canonical)
    receipt = result.get("receipt")
    if (
        result_kind != "Accepted"
        or authoritative_for_state != 1
        or late_after_fence != 0
        or canonical_bytes(result) != result_canonical
        or sha256(result_canonical) != result_hash
        or result.get("kind") != "Accepted"
        or not isinstance(receipt, dict)
        or receipt.get("channel") != channel
        or receipt.get("provider") != provider
        or receipt.get("message_id") != message_id
        or receipt.get("platform_message_id") != platform_message_id
        or receipt.get("accepted_at") != accepted_at
        or receipt.get("latency_ms") != latency_ms
        or sha256(frozen_audit) != frozen_audit_hash
        or not delivery_audit_ref
    ):
        fail("accepted sink receipt/audit join mismatch")
    immutable_expected[result_event_identity] = (
        "DeliveryAcceptedAudit",
        frozen_audit,
        frozen_audit_hash,
        delivery_audit_ref,
    )

    return {
        "review_task": review_task,
        "decision_identity": decision_identity,
        "ordinal": ordinal,
        "envelope": envelope,
        "delivery_attempt_identity": delivery_attempt_identity,
        "result": result,
        "receipt": receipt,
        "channel": channel,
        "immutable_expected": immutable_expected,
    }


def verify(args: argparse.Namespace) -> None:
    with tempfile.TemporaryDirectory(prefix="br194-review-join-") as temporary:
        copied_database, authority_snapshots = copy_database_authority(Path(temporary))
        connection = sqlite3.connect(copied_database)
        try:
            connection.execute("PRAGMA query_only=ON")
            evidence = verify_connection(connection, args)
        finally:
            connection.close()
        consumed = {
            **verify_immutable_append(evidence["immutable_expected"]),
            **verify_counted_push_and_delivery_audit(evidence),
        }
        for path, snapshot in authority_snapshots.items():
            assert_authority_unchanged(path, snapshot)
        for path, snapshot in consumed.items():
            assert_authority_unchanged(path, snapshot)

    review_task = evidence["review_task"]
    producer_batches = 1 if review_task == "R-04" else 2
    print(
        f"BR194_JOIN task={review_task} producer_batches={producer_batches} "
        "task_bindings=1 durable_terminal=1 sink_receipts=1 push_logs=1 "
        "delivery_audits=1 joined_identities=1 hydration_state=Applied "
        "replay_passed=1 replay_provider_calls=0 replay_resume_calls=0 "
        "replay_sink_delta=0 replay_delivery_audit_delta=0 replay_audits=2"
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--business-date", required=True)
    parser.add_argument("--task", choices=("R-04", "R-09"), required=True)
    parser.add_argument("--require-passed-replay", choices=("1",), required=True)
    parser.add_argument("--replay-ordinal", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.replay_ordinal:
        fail("replay ordinal override is forbidden")
    return args


if __name__ == "__main__":
    try:
        verify(parse_args())
    except (OSError, sqlite3.Error, ValueError, KeyError, TypeError) as error:
        fail(str(error))
