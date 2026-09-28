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
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
UTC_MILLIS = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}Z\Z")
ORIGIN_DOMAIN = "durable-delivery-correlation-observation-v1"

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
CORRELATION_COLUMNS = {
    "observation_identity",
    "identity_version",
    "decision_identity",
    "producer_id",
    "occurrence_identity",
    "role",
    "observed_at",
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
                "correlated_durable_candidates": [],
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


def _origin_observation_identity(decision: str, producer: str, occurrence: str) -> str:
    parts = (ORIGIN_DOMAIN, decision, producer, occurrence, "Origin", "1")
    digest = hashlib.sha256()
    for part in parts:
        encoded = part.encode("utf-8")
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
    return digest.hexdigest()


def _correlated_candidates(connection, units, from_date, to_date):
    orphan = connection.execute(
        """
        SELECT 1 FROM delivery_correlation_observations AS c
        LEFT JOIN delivery_decisions AS d ON d.decision_identity=c.decision_identity
        WHERE d.decision_identity IS NULL LIMIT 1
        """
    ).fetchone()
    if orphan:
        raise ValueError("orphan v10 correlation observation")
    by_producer = {
        producer["id"]: (unit["id"], producer["current_kinds"])
        for unit in units for producer in unit["producers"]
    }
    rows = connection.execute(
        """
        SELECT c.observation_identity,c.identity_version,c.decision_identity,
               c.producer_id,c.occurrence_identity,c.role,c.observed_at,
               d.business_date,d.push_kind,d.sub_kind,d.scope_key,d.state,
               COALESCE(s.accepted_events,0)
        FROM delivery_correlation_observations AS c
        JOIN delivery_decisions AS d ON d.decision_identity=c.decision_identity
        LEFT JOIN (
          SELECT decision_identity,COUNT(DISTINCT result_event_identity) AS accepted_events
          FROM sink_results
          WHERE result_kind='Accepted' AND authoritative_for_state=1
            AND late_after_fence=0
          GROUP BY decision_identity
        ) AS s ON s.decision_identity=d.decision_identity
        WHERE d.business_date>=? AND d.business_date<=?
        ORDER BY d.business_date,c.decision_identity,c.observation_identity
        """,
        (from_date, to_date),
    )
    decisions = {}
    for (identity, version, decision, producer, occurrence, role, observed_at,
         day, kind, sub_kind, scope_key, state, accepted_events) in rows:
        if (not all(isinstance(value, str) for value in
                    (identity, decision, producer, occurrence, role, observed_at,
                     day, kind, sub_kind, scope_key, state))
                or not SHA256.fullmatch(identity) or not SHA256.fullmatch(decision)
                or version != 1 or role != "Origin"
                or producer not in ("p01-scheduled", "p01-compensation")
                or by_producer.get(producer) != ("MU-p01", ["PreopenNewsHot"])
                or kind != "PreopenNewsHot" or sub_kind != "None"
                or scope_key != "GLOBAL" or occurrence != f"p01:{day}"
                or _origin_observation_identity(decision, producer, occurrence) != identity
                or not UTC_MILLIS.fullmatch(observed_at)):
            raise ValueError("invalid v10 P01 Origin correlation observation")
        try:
            _date(day)
            parsed_at = dt.datetime.fromisoformat(observed_at.replace("Z", "+00:00"))
        except ValueError as exc:
            raise ValueError("invalid v10 P01 Origin correlation date") from exc
        if parsed_at.isoformat(timespec="milliseconds").replace("+00:00", "Z") != observed_at:
            raise ValueError("noncanonical v10 P01 Origin observation time")
        previous = decisions.setdefault(decision, {
            "unit": "MU-p01", "business_date": day, "push_kind": kind,
            "state": state, "accepted_events": accepted_events, "producers": set(),
        })
        if (previous["business_date"], previous["push_kind"], previous["state"],
            previous["accepted_events"]) != (day, kind, state, accepted_events):
            raise ValueError("inconsistent v10 P01 decision correlation")
        previous["producers"].add(producer)

    grouped = {}
    for item in decisions.values():
        key = (item["unit"], item["business_date"], item["push_kind"], item["state"])
        group = grouped.setdefault(key, {
            "business_date": item["business_date"], "push_kind": item["push_kind"],
            "state": item["state"], "decisions": 0,
            "accepted_decision_candidates": 0, "accepted_result_events": 0,
            "observed_producer_ids": set(),
        })
        group["decisions"] += 1
        group["accepted_decision_candidates"] += item["accepted_events"] > 0
        group["accepted_result_events"] += item["accepted_events"]
        group["observed_producer_ids"].update(item["producers"])
    by_unit = {unit["id"]: unit for unit in units}
    for (unit_id, _, _, _), group in sorted(grouped.items()):
        group["observed_producer_ids"] = sorted(group["observed_producer_ids"])
        unit = by_unit[unit_id]
        unit["correlation"] = "Observed"
        unit["correlated_durable_candidates"].append(group)


def _kind_counts(durable_db: Path, from_date: str, to_date: str, units: list[dict]) -> tuple[int, list[dict]]:
    uri = durable_db.resolve().as_uri() + "?mode=ro"
    with closing(sqlite3.connect(uri, uri=True)) as connection:
        connection.execute("PRAGMA query_only=ON")
        connection.execute("BEGIN")
        try:
            version = connection.execute("PRAGMA user_version").fetchone()[0]
            if version not in (9, 10):
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
            if version == 10:
                for column in ("sub_kind", "scope_key"):
                    if column not in {
                        row[1] for row in connection.execute("PRAGMA table_info(delivery_decisions)")
                    }:
                        raise ValueError("missing durable-delivery columns in delivery_decisions")
                table = "delivery_correlation_observations"
                table_type = connection.execute(
                    "SELECT type FROM sqlite_master WHERE name=?", (table,)
                ).fetchone()
                columns = {
                    row[1] for row in connection.execute(f"PRAGMA table_info({table})")
                }
                if table_type != ("table",) or not CORRELATION_COLUMNS <= columns:
                    raise ValueError("missing durable-delivery columns in correlation sidecar")
                _correlated_candidates(connection, units, from_date, to_date)
            unattributed = "" if version == 9 else (
                "AND NOT EXISTS (SELECT 1 FROM delivery_correlation_observations AS c "
                "WHERE c.decision_identity=d.decision_identity)"
            )
            rows = connection.execute(
                f"""
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
                  {unattributed}
                GROUP BY d.business_date, d.push_kind, d.state
                ORDER BY d.business_date, d.push_kind, d.state
                """,
                (from_date, to_date),
            ).fetchall()
        finally:
            connection.rollback()
    return version, [
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
    """Return logically read-only counts from an isolated stable v9/v10 snapshot.

    The caller must supply a fixture or controlled snapshot, not a live
    production WAL path. WAL sidecar creation is possible despite mode=ro and
    query_only. Only validated v10 P01 Origin edges attribute durable candidates.
    """
    if _date(from_date) > _date(to_date):
        raise ValueError("from_date must not exceed to_date")
    units, catalog_hash, delta_hash = _catalog_rows(root.resolve())
    schema_version, candidates = _kind_counts(durable_db, from_date, to_date, units)
    return {
        "catalog_sha256": catalog_hash,
        "delta_sha256": delta_hash,
        "schema_version": schema_version,
        "from_date": from_date,
        "to_date": to_date,
        "units": units,
        "unattributed_kind_candidates": candidates,
        "scope": (
            "Kind counts are durable sink result candidates, not manual acceptance. "
            "Only validated v10 P01 Origin edges identify an observed producer "
            "and Unit for durable candidates. Neither a correlation edge nor "
            "an Accepted sink result proves external receipt or business finalization."
        ),
    }
