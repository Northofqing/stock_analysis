"""Build an M0 matrix from a caller-supplied, isolated stable SQLite snapshot.

The queries are logically read-only. SQLite may still create WAL sidecar files
with mode=ro/query_only; a live production WAL database requires a separately
controlled snapshot before it is passed here.
"""

import datetime as dt
import hashlib
import json
from contextlib import closing
from pathlib import Path
import re
import sqlite3

import push_catalog_drift


CATALOG = Path("docs/push-system/push-capability-catalog.v1.json")
DELTA = Path("docs/push-system/push-current-kind-delta.v2.json")
CATALOG_SHA256 = "0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3"
DELTA_SHA256 = "8a1c436ea24650057b7b0da88daceddd51bd95306df354447d2f136f75615448"
DATE = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}\Z")

REQUIRED_COLUMNS = {
    "delivery_decisions": {"decision_identity", "business_date", "push_kind", "state"},
    "sink_results": {
        "result_event_identity",
        "decision_identity",
        "result_kind",
        "authoritative_for_state",
        "late_after_fence",
    },
}


def _date(value: str) -> dt.date:
    if not isinstance(value, str) or not DATE.fullmatch(value):
        raise ValueError("date must be YYYY-MM-DD")
    return dt.date.fromisoformat(value)


def _catalog_rows(root: Path) -> tuple[list[dict], str, str]:
    catalog_bytes = (root / CATALOG).read_bytes()
    delta_bytes = (root / DELTA).read_bytes()
    catalog_hash = hashlib.sha256(catalog_bytes).hexdigest()
    delta_hash = hashlib.sha256(delta_bytes).hexdigest()
    if catalog_hash != CATALOG_SHA256 or delta_hash != DELTA_SHA256:
        raise ValueError("catalog or current-kind delta digest mismatch")
    drift = push_catalog_drift.check(root)
    if drift["errors"]:
        raise ValueError("push catalog drift: " + "; ".join(drift["errors"]))

    catalog = json.loads(catalog_bytes)
    delta = json.loads(delta_bytes)
    units = catalog["migration_units"]
    producers = catalog["producers"]
    unit_ids = [unit["id"] for unit in units]
    producer_ids = [producer["id"] for producer in producers]
    if len(units) != 52 or len(set(unit_ids)) != 52:
        raise ValueError("expected 52 distinct migration units")
    if len(producers) != 102 or len(set(producer_ids)) != 102:
        raise ValueError("expected 102 distinct producers")
    by_producer = {producer["id"]: producer for producer in producers}
    if sum(not producer["kinds"] for producer in producers) != 10:
        raise ValueError("expected 10 enum-external producers")

    current_kinds = {producer["id"]: producer["kinds"] for producer in producers}
    for mapping in delta["mappings"]:
        producer = by_producer[mapping["producer_id"]]
        if (
            producer["migration_unit_id"] != mapping["migration_unit_id"]
            or producer["kinds"] != [mapping["replaces_producer_kind"]]
        ):
            raise ValueError("current-kind delta producer/unit mismatch")
        current_kinds[producer["id"]] = [mapping["kind"]]

    rows = []
    completion_owners = set()
    for unit in units:
        registered = unit["producer_ids"]
        unit_producers = [
            producer for producer in producers if producer["migration_unit_id"] == unit["id"]
        ]
        if not registered or len(registered) != len(set(registered)) or set(registered) != {
            producer["id"] for producer in unit_producers
        }:
            raise ValueError("migration unit producer closure mismatch")
        owner = unit["completion_owner"]
        if owner in completion_owners or any(
            producer["completion_owner"] != owner for producer in unit_producers
        ):
            raise ValueError("migration unit completion owner mismatch")
        completion_owners.add(owner)
        if set(unit["occurrence_families"]) != {
            producer["occurrence_family"] for producer in unit_producers
        } or set(unit["phase_epics"]) != {
            phase for producer in unit_producers for phase in producer["phase_epics"]
        }:
            raise ValueError("migration unit occurrence/phase closure mismatch")
        rows.append(
            {
                "id": unit["id"],
                "completion_owner": owner,
                "occurrence_families": unit["occurrence_families"],
                "phase_epics": unit["phase_epics"],
                "producers": [
                    {"id": producer_id, "current_kinds": current_kinds[producer_id]}
                    for producer_id in registered
                ],
                "source_status": "CatalogOnly",
                "correlation": "NotRecorded",
                "delivery": "Unknown",
                "terminal": "Unknown",
                "finalizer": "Unknown",
            }
        )
    emitted_producers = [
        producer["id"] for row in rows for producer in row["producers"]
    ]
    if len(emitted_producers) != 102 or set(emitted_producers) != set(producer_ids):
        raise ValueError("migration unit rows do not cover all 102 producers exactly once")
    return rows, catalog_hash, delta_hash


def _kind_counts(durable_db: Path, from_date: str, to_date: str) -> list[dict]:
    uri = durable_db.resolve().as_uri() + "?mode=ro"
    with closing(sqlite3.connect(uri, uri=True)) as connection:
        connection.execute("PRAGMA query_only=ON")
        connection.execute("BEGIN")
        try:
            version = connection.execute("PRAGMA user_version").fetchone()[0]
            if version != 9:
                raise ValueError(f"unsupported durable-delivery schema version {version}")
            for table, required in REQUIRED_COLUMNS.items():
                table_type = connection.execute(
                    "SELECT type FROM sqlite_master WHERE name=?", (table,)
                ).fetchone()
                columns = {
                    row[1] for row in connection.execute(f"PRAGMA table_info({table})")
                }
                if table_type != ("table",) or not required <= columns:
                    raise ValueError(f"missing durable-delivery columns in {table}")
            rows = connection.execute(
                """
                SELECT d.business_date, d.push_kind, d.state,
                       COUNT(DISTINCT d.decision_identity),
                       COUNT(DISTINCT CASE
                           WHEN s.result_kind='Accepted'
                            AND s.authoritative_for_state=1
                            AND s.late_after_fence=0
                           THEN d.decision_identity END),
                       COUNT(DISTINCT CASE
                           WHEN s.result_kind='Accepted'
                            AND s.authoritative_for_state=1
                            AND s.late_after_fence=0
                           THEN s.result_event_identity END)
                FROM delivery_decisions AS d
                LEFT JOIN sink_results AS s
                  ON s.decision_identity=d.decision_identity
                WHERE d.business_date>=? AND d.business_date<=?
                GROUP BY d.business_date, d.push_kind, d.state
                ORDER BY d.business_date, d.push_kind, d.state
                """,
                (from_date, to_date),
            ).fetchall()
        finally:
            connection.rollback()
    return [
        {
            "business_date": day,
            "push_kind": kind,
            "state": state,
            "decisions": decisions,
            "accepted_decision_candidates": accepted_decisions,
            "accepted_result_events": accepted_events,
        }
        for day, kind, state, decisions, accepted_decisions, accepted_events in rows
    ]


def build_report(root: Path, durable_db: Path, from_date: str, to_date: str) -> dict:
    """Return logically read-only counts from an isolated stable v9 snapshot.

    The caller must supply a fixture or controlled snapshot, not a live
    production WAL path. WAL sidecar creation is possible despite mode=ro and
    query_only. Kind counts never attribute delivery to Units.
    """
    if _date(from_date) > _date(to_date):
        raise ValueError("from_date must not exceed to_date")
    units, catalog_hash, delta_hash = _catalog_rows(root.resolve())
    candidates = _kind_counts(durable_db, from_date, to_date)
    return {
        "catalog_sha256": catalog_hash,
        "delta_sha256": delta_hash,
        "schema_version": 9,
        "from_date": from_date,
        "to_date": to_date,
        "units": units,
        "unattributed_kind_candidates": candidates,
        "scope": (
            "Kind counts are durable sink result candidates, not manual acceptance. "
            "They do not identify a producer or Unit, prove external receipt, "
            "or prove business finalization."
        ),
    }
