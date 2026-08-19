use super::model::{
    compiled_policy_catalog, CooldownScope, DeliverySubKind, DurableDeliveryError,
    ManualAcceptedDeliveryAuditEvidence, PolicyRow, PushKind, Result, WindowMode,
};
use rusqlite::{functions::FunctionFlags, params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub(crate) const SCHEMA_VERSION: i64 = 9;

#[cfg(test)]
thread_local! {
    static WAL_MATERIALIZATION_CALLS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

pub(crate) fn materialize_wal_capability(connection: &Connection) -> Result<()> {
    #[cfg(test)]
    WAL_MATERIALIZATION_CALLS.with(|calls| calls.set(calls.get() + 1));
    connection.busy_timeout(std::time::Duration::from_millis(5_000))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.execute_batch("BEGIN IMMEDIATE; ROLLBACK;")?;
    let synchronous: i64 = connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
    let journal_mode: String =
        connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    if synchronous < 2 || journal_mode.to_uppercase() != "WAL" {
        return Err(DurableDeliveryError::InvalidConfiguration(format!(
            "sqlite WAL capability not enforced: synchronous={synchronous} journal_mode={journal_mode}"
        )));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn wal_materialization_call_count_for_test() -> usize {
    WAL_MATERIALIZATION_CALLS.with(std::cell::Cell::get)
}

pub(crate) fn configure_attested_connection(connection: &Connection) -> Result<()> {
    register_sha256_function(connection)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    verify_connection_configuration(connection)
}

pub(crate) fn register_sha256_function(connection: &Connection) -> Result<()> {
    connection.create_scalar_function(
        "sha256_hex",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let canonical = context.get_raw(0).as_blob()?;
            Ok(hex::encode(Sha256::digest(canonical)))
        },
    )?;
    Ok(())
}

pub(crate) fn verify_connection_configuration(connection: &Connection) -> Result<()> {
    let foreign_keys: i64 =
        connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    let synchronous: i64 = connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
    let journal_mode: String =
        connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    let sha256_probe: String =
        connection.query_row("SELECT sha256_hex(X'')", [], |row| row.get(0))?;
    let (sha256_encoding, sha256_flags): (String, i64) = connection.query_row(
        "SELECT enc,flags
         FROM pragma_function_list
         WHERE name='sha256_hex' AND type='s' AND narg=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let required_sha256_flags =
        (FunctionFlags::SQLITE_DETERMINISTIC | FunctionFlags::SQLITE_INNOCUOUS).bits() as i64;
    if foreign_keys != 1
        || synchronous < 2
        || journal_mode.to_uppercase() != "WAL"
        || sha256_probe != "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        || !sha256_encoding.eq_ignore_ascii_case("utf8")
        || sha256_flags & required_sha256_flags != required_sha256_flags
    {
        return Err(DurableDeliveryError::InvalidConfiguration(format!(
            "sqlite connection contract not enforced: foreign_keys={foreign_keys} synchronous={synchronous} journal_mode={journal_mode} sha256_probe={sha256_probe} sha256_encoding={sha256_encoding} sha256_flags={sha256_flags}"
        )));
    }
    Ok(())
}

pub(crate) fn initialize_schema(transaction: &Transaction<'_>) -> Result<()> {
    let current_version: i64 =
        transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if current_version > SCHEMA_VERSION {
        return Err(DurableDeliveryError::InvalidConfiguration(format!(
            "durable-delivery schema version {current_version} is newer than supported {SCHEMA_VERSION}"
        )));
    }
    match current_version {
        1 => {
            migrate_schema_v1_to_v2(transaction)?;
            migrate_schema_v2_to_v3(transaction)?;
            migrate_schema_v3_to_v4(transaction)?;
            migrate_schema_v4_to_v5(transaction)?;
            migrate_schema_v5_to_v6(transaction)?;
            migrate_schema_v6_to_v7(transaction)?;
            migrate_schema_v7_to_v8(transaction)?;
            migrate_schema_v8_to_v9(transaction)?;
        }
        2 => {
            migrate_schema_v2_to_v3(transaction)?;
            migrate_schema_v3_to_v4(transaction)?;
            migrate_schema_v4_to_v5(transaction)?;
            migrate_schema_v5_to_v6(transaction)?;
            migrate_schema_v6_to_v7(transaction)?;
            migrate_schema_v7_to_v8(transaction)?;
            migrate_schema_v8_to_v9(transaction)?;
        }
        3 => {
            migrate_schema_v3_to_v4(transaction)?;
            migrate_schema_v4_to_v5(transaction)?;
            migrate_schema_v5_to_v6(transaction)?;
            migrate_schema_v6_to_v7(transaction)?;
            migrate_schema_v7_to_v8(transaction)?;
            migrate_schema_v8_to_v9(transaction)?;
        }
        4 => {
            migrate_schema_v4_to_v5(transaction)?;
            migrate_schema_v5_to_v6(transaction)?;
            migrate_schema_v6_to_v7(transaction)?;
            migrate_schema_v7_to_v8(transaction)?;
            migrate_schema_v8_to_v9(transaction)?;
        }
        5 => {
            migrate_schema_v5_to_v6(transaction)?;
            migrate_schema_v6_to_v7(transaction)?;
            migrate_schema_v7_to_v8(transaction)?;
            migrate_schema_v8_to_v9(transaction)?;
        }
        6 => {
            migrate_schema_v6_to_v7(transaction)?;
            migrate_schema_v7_to_v8(transaction)?;
            migrate_schema_v8_to_v9(transaction)?;
        }
        7 => {
            migrate_schema_v7_to_v8(transaction)?;
            migrate_schema_v8_to_v9(transaction)?;
        }
        8 => migrate_schema_v8_to_v9(transaction)?,
        _ => {}
    }
    transaction.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS delivery_decisions(
          decision_identity TEXT PRIMARY KEY,
          business_date TEXT NOT NULL,
          push_kind TEXT NOT NULL,
          sub_kind TEXT NOT NULL,
          cooldown_scope TEXT NOT NULL,
          scope_key TEXT NOT NULL,
          state TEXT NOT NULL CHECK(state IN (
            'Reserved','AttemptInFlight',
            'AcceptedAuditPending','AcceptedTaskTransitionPending','Delivered',
            'RejectedAuditPending','RejectedTaskTransitionPending','RejectedDurable',
            'UncertainAuditPending','UncertainTaskTransitionPending',
            'UncertainManualReview',
            'ManualRejectedAuditPending','ManualRejectedTaskTransitionPending',
            'ManualResolvedRejected')),
          envelope_version INTEGER NOT NULL,
          envelope_canonical BLOB NOT NULL,
          envelope_sha256 TEXT NOT NULL,
          task_binding_present INTEGER NOT NULL CHECK(task_binding_present IN (0,1)),
          transition_basis_canonical BLOB,
          transition_basis_sha256 TEXT,
          reservation_generation INTEGER NOT NULL CHECK(reservation_generation >= 0),
          current_budget_reservation_identity TEXT,
          current_cooldown_reservation_identity TEXT,
          current_attempt_identity TEXT,
          current_disposition_identity TEXT,
          fence_generation INTEGER NOT NULL CHECK(fence_generation >= 0),
          retry_authorized INTEGER NOT NULL CHECK(retry_authorized IN (0,1)),
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          CHECK (
            (task_binding_present=0 AND transition_basis_canonical IS NULL
              AND transition_basis_sha256 IS NULL)
            OR
            (task_binding_present=1 AND transition_basis_canonical IS NOT NULL
              AND transition_basis_sha256 IS NOT NULL)
          )
        );

        CREATE TABLE IF NOT EXISTS delivery_policy_catalog(
          push_kind TEXT NOT NULL,
          sub_kind TEXT NOT NULL,
          cooldown_scope TEXT NOT NULL,
          base_cooldown_secs INTEGER,
          override_cooldown_secs INTEGER,
          window_mode TEXT NOT NULL CHECK(window_mode IN
            ('None','Rolling','BusinessDateOnce')),
          counts_against_daily_budget INTEGER NOT NULL CHECK(
            counts_against_daily_budget IN (0,1)),
          policy_version INTEGER NOT NULL,
          PRIMARY KEY(push_kind,sub_kind)
        );

        CREATE TABLE IF NOT EXISTS immutable_audit_outbox(
          audit_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          attempt_identity TEXT REFERENCES delivery_attempts(attempt_identity),
          audit_kind TEXT NOT NULL CHECK(audit_kind IN (
            'DecisionStateChanged','LeaseGranted','LeaseHeartbeat',
            'FenceRevoked','RecoveryClassified','SinkResultAuthorityClassified',
            'LateReceiptObserved','BudgetReservationChanged',
            'CooldownReservationChanged','BusinessDateOnceClaimed',
            'DecisionIdentityConflict','ScheduleHydrationApplied',
            'ReviewTerminalReplayStarted','ReviewTerminalReplayCompleted')),
          predecessor_audit_identity TEXT REFERENCES immutable_audit_outbox(audit_identity),
          audit_canonical BLOB NOT NULL,
          audit_sha256 TEXT NOT NULL,
          append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
          immutable_audit_ref TEXT,
          created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS cooldown_reservations(
          cooldown_reservation_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          reservation_generation INTEGER NOT NULL CHECK(reservation_generation > 0),
          attempt_identity TEXT,
          business_date TEXT NOT NULL,
          push_kind TEXT NOT NULL,
          sub_kind TEXT NOT NULL,
          cooldown_scope TEXT NOT NULL,
          scope_key TEXT NOT NULL,
          policy_version INTEGER NOT NULL,
          effective_cooldown_secs INTEGER,
          window_mode TEXT NOT NULL CHECK(window_mode IN
            ('Rolling','BusinessDateOnce')),
          reserved_at TEXT NOT NULL,
          accepted_at TEXT,
          blocked_until TEXT,
          released_at TEXT,
          state TEXT NOT NULL CHECK(state IN
            ('Reserved','Accepted','Uncertain','Released')),
          UNIQUE(decision_identity,reservation_generation)
        );

        CREATE TABLE IF NOT EXISTS cooldown_heads(
          push_kind TEXT NOT NULL,
          sub_kind TEXT NOT NULL,
          cooldown_scope TEXT NOT NULL,
          scope_key TEXT NOT NULL,
          current_reservation_identity TEXT REFERENCES cooldown_reservations(
            cooldown_reservation_identity),
          state TEXT NOT NULL CHECK(state IN
            ('Reserved','Accepted','Uncertain','Released')),
          blocked_until TEXT,
          version INTEGER NOT NULL,
          PRIMARY KEY(push_kind,sub_kind,cooldown_scope,scope_key)
        );

        CREATE TABLE IF NOT EXISTS business_date_once_claims(
          business_date TEXT NOT NULL,
          push_kind TEXT NOT NULL,
          sub_kind TEXT NOT NULL,
          scope_key TEXT NOT NULL,
          decision_identity TEXT NOT NULL UNIQUE REFERENCES delivery_decisions(decision_identity),
          policy_version INTEGER NOT NULL,
          claimed_at TEXT NOT NULL,
          audit_identity TEXT NOT NULL UNIQUE REFERENCES immutable_audit_outbox(audit_identity),
          PRIMARY KEY(business_date,push_kind,sub_kind,scope_key)
        );

        CREATE TABLE IF NOT EXISTS daily_budget_reservations(
          budget_reservation_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          reservation_generation INTEGER NOT NULL CHECK(reservation_generation > 0),
          attempt_identity TEXT,
          business_date TEXT NOT NULL,
          slot_no INTEGER NOT NULL CHECK(slot_no BETWEEN 1 AND 30),
          reserved_at TEXT NOT NULL,
          accepted_at TEXT,
          released_at TEXT,
          state TEXT NOT NULL CHECK(state IN
            ('Reserved','Accepted','Uncertain','Released')),
          UNIQUE(decision_identity,reservation_generation)
        );

        CREATE TABLE IF NOT EXISTS delivery_attempts(
          attempt_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          attempt_no INTEGER NOT NULL CHECK(attempt_no > 0),
          owner_instance_identity TEXT NOT NULL,
          fence_token INTEGER NOT NULL CHECK(fence_token > 0),
          lease_expires_at TEXT NOT NULL,
          lease_heartbeat_at TEXT NOT NULL,
          fence_revoked_at TEXT,
          state TEXT NOT NULL CHECK(state IN
            ('AttemptInFlight','Accepted','Rejected','Uncertain')),
          started_at TEXT NOT NULL,
          UNIQUE(decision_identity,attempt_no),
          UNIQUE(decision_identity,fence_token)
        );

        CREATE TABLE IF NOT EXISTS sink_results(
          result_event_identity TEXT PRIMARY KEY,
          attempt_identity TEXT NOT NULL REFERENCES delivery_attempts(attempt_identity),
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          result_kind TEXT NOT NULL CHECK(result_kind IN
            ('Accepted','Rejected','Uncertain')),
          observed_at TEXT NOT NULL,
          fence_token INTEGER NOT NULL,
          authoritative_for_state INTEGER NOT NULL CHECK(
            authoritative_for_state IN (0,1)),
          late_after_fence INTEGER NOT NULL CHECK(late_after_fence IN (0,1)),
          authority_audit_identity TEXT NOT NULL UNIQUE
            REFERENCES immutable_audit_outbox(audit_identity),
          late_receipt_audit_identity TEXT UNIQUE
            REFERENCES immutable_audit_outbox(audit_identity),
          result_canonical BLOB NOT NULL,
          result_sha256 TEXT NOT NULL,
          channel TEXT,
          provider TEXT,
          message_id TEXT,
          platform_message_id TEXT,
          accepted_at TEXT,
          latency_ms INTEGER,
          frozen_delivery_audit_canonical BLOB,
          frozen_delivery_audit_sha256 TEXT,
          delivery_audit_ref TEXT,
          UNIQUE(attempt_identity,result_sha256),
          CHECK(late_after_fence=0 OR late_receipt_audit_identity IS NOT NULL)
        );

        CREATE TABLE IF NOT EXISTS review_terminal_replay_attempts(
          attempt_identity TEXT PRIMARY KEY,
          business_date TEXT NOT NULL,
          review_task TEXT NOT NULL CHECK(review_task IN ('R-04','R-09')),
          task_identity TEXT NOT NULL,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          replay_ordinal INTEGER NOT NULL CHECK(replay_ordinal > 0),
          started_at TEXT NOT NULL,
          pre_sink_count INTEGER NOT NULL,
          pre_sink_set_sha256 TEXT NOT NULL,
          pre_delivery_audit_count INTEGER NOT NULL,
          pre_delivery_audit_set_sha256 TEXT NOT NULL,
          provider_calls INTEGER NOT NULL CHECK(provider_calls = 0),
          start_canonical BLOB NOT NULL,
          start_sha256 TEXT NOT NULL,
          start_audit_identity TEXT NOT NULL UNIQUE
            REFERENCES immutable_audit_outbox(audit_identity),
          UNIQUE(attempt_identity,decision_identity),
          UNIQUE(
            business_date,review_task,task_identity,decision_identity,replay_ordinal
          )
        );

        CREATE TABLE IF NOT EXISTS review_terminal_replay_completions(
          attempt_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL,
          state TEXT NOT NULL CHECK(state IN ('Passed','Failed')),
          completed_at TEXT NOT NULL,
          post_sink_count INTEGER NOT NULL,
          post_sink_set_sha256 TEXT NOT NULL,
          post_delivery_audit_count INTEGER NOT NULL,
          post_delivery_audit_set_sha256 TEXT NOT NULL,
          provider_calls INTEGER NOT NULL CHECK(provider_calls = 0),
          resume_calls INTEGER NOT NULL CHECK(resume_calls >= 0),
          sink_calls INTEGER NOT NULL CHECK(sink_calls >= 0),
          delivery_audit_appends INTEGER NOT NULL CHECK(delivery_audit_appends >= 0),
          reason_code TEXT NOT NULL,
          completion_canonical BLOB NOT NULL,
          completion_sha256 TEXT NOT NULL,
          completion_audit_identity TEXT NOT NULL UNIQUE
            REFERENCES immutable_audit_outbox(audit_identity),
          CHECK(
            state != 'Passed'
            OR (
              resume_calls=0 AND sink_calls=0 AND delivery_audit_appends=0
            )
          ),
          FOREIGN KEY(attempt_identity,decision_identity)
            REFERENCES review_terminal_replay_attempts(
              attempt_identity,decision_identity
            )
        );

        CREATE TABLE IF NOT EXISTS manual_resolutions(
          resolution_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL UNIQUE REFERENCES delivery_decisions(decision_identity),
          attempt_identity TEXT NOT NULL REFERENCES delivery_attempts(attempt_identity),
          disposition TEXT NOT NULL CHECK(disposition IN ('Accepted','Rejected')),
          operator_identity TEXT NOT NULL,
          reason TEXT NOT NULL,
          evidence_canonical BLOB NOT NULL,
          evidence_sha256 TEXT NOT NULL,
          receipt_canonical BLOB,
          frozen_delivery_audit_canonical BLOB,
          frozen_delivery_audit_sha256 TEXT,
          immutable_audit_ref TEXT NOT NULL,
          accepted_audit_identity TEXT UNIQUE,
          accepted_audit_append_state TEXT
            CHECK(accepted_audit_append_state IN ('Pending','Appended')),
          accepted_audit_ref TEXT
            CHECK(accepted_audit_ref IS NULL OR
              length(replace(replace(replace(replace(
                accepted_audit_ref,' ',''),char(9),''),char(10),''),char(13),'')) > 0),
          resolved_at TEXT NOT NULL,
          CHECK (
            (disposition='Rejected'
              AND frozen_delivery_audit_canonical IS NULL
              AND frozen_delivery_audit_sha256 IS NULL
              AND accepted_audit_identity IS NULL
              AND accepted_audit_append_state IS NULL
              AND accepted_audit_ref IS NULL)
            OR
            (disposition='Accepted'
              AND frozen_delivery_audit_canonical IS NOT NULL
              AND frozen_delivery_audit_sha256 IS NOT NULL
              AND accepted_audit_identity IS NOT NULL
              AND (
                (accepted_audit_append_state='Pending' AND accepted_audit_ref IS NULL)
                OR
                (accepted_audit_append_state='Appended'
                  AND accepted_audit_ref IS NOT NULL
                  AND length(replace(replace(replace(replace(
                    accepted_audit_ref,' ',''),char(9),''),char(10),''),char(13),'')) > 0)
              ))
          )
        );

        CREATE TABLE IF NOT EXISTS delivery_disposition_payloads(
          disposition_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          attempt_identity TEXT REFERENCES delivery_attempts(attempt_identity),
          resolution_identity TEXT REFERENCES manual_resolutions(resolution_identity),
          denial_identity TEXT,
          disposition TEXT NOT NULL CHECK(disposition IN
            ('Accepted','Rejected','Uncertain','ManualAccepted','ManualRejected')),
          disposition_canonical BLOB NOT NULL,
          disposition_sha256 TEXT NOT NULL,
          append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
          immutable_audit_ref TEXT,
          created_at TEXT NOT NULL,
          UNIQUE(decision_identity,disposition_identity)
        );

        CREATE TABLE IF NOT EXISTS task_transition_payloads(
          transition_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          disposition_identity TEXT NOT NULL REFERENCES delivery_disposition_payloads(
            disposition_identity),
          task_binding_sha256 TEXT NOT NULL,
          transition_canonical BLOB NOT NULL,
          transition_sha256 TEXT NOT NULL,
          append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
          immutable_audit_ref TEXT,
          hydration_state TEXT NOT NULL DEFAULT 'Pending'
            CHECK(hydration_state IN ('Pending','Applied')),
          hydration_ack_identity TEXT,
          hydrated_at TEXT,
          UNIQUE(decision_identity,transition_identity)
        );

        CREATE TABLE IF NOT EXISTS delivery_state_events(
          event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
          state_event_identity TEXT NOT NULL UNIQUE,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          from_state TEXT,
          to_state TEXT NOT NULL,
          actor TEXT NOT NULL,
          operator_identity TEXT,
          evidence_canonical BLOB NOT NULL,
          evidence_sha256 TEXT NOT NULL,
          audit_identity TEXT NOT NULL UNIQUE REFERENCES immutable_audit_outbox(audit_identity)
        );

        CREATE TABLE IF NOT EXISTS delivery_attempt_events(
          attempt_event_identity TEXT PRIMARY KEY,
          attempt_identity TEXT NOT NULL REFERENCES delivery_attempts(attempt_identity),
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          event_kind TEXT NOT NULL CHECK(event_kind IN (
            'LeaseGranted','LeaseHeartbeat','FenceRevoked',
            'RecoveryClassified','SinkResultAuthorityClassified',
            'LateReceiptObserved')),
          event_canonical BLOB NOT NULL,
          event_sha256 TEXT NOT NULL,
          audit_identity TEXT NOT NULL UNIQUE REFERENCES immutable_audit_outbox(audit_identity)
        );

        CREATE TABLE IF NOT EXISTS cooldown_reservation_events(
          event_identity TEXT PRIMARY KEY,
          cooldown_reservation_identity TEXT NOT NULL REFERENCES cooldown_reservations(
            cooldown_reservation_identity),
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          from_state TEXT,
          to_state TEXT NOT NULL,
          event_canonical BLOB NOT NULL,
          event_sha256 TEXT NOT NULL,
          audit_identity TEXT NOT NULL UNIQUE REFERENCES immutable_audit_outbox(audit_identity)
        );

        CREATE TABLE IF NOT EXISTS daily_budget_reservation_events(
          event_identity TEXT PRIMARY KEY,
          budget_reservation_identity TEXT NOT NULL REFERENCES daily_budget_reservations(
            budget_reservation_identity),
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          from_state TEXT,
          to_state TEXT NOT NULL,
          event_canonical BLOB NOT NULL,
          event_sha256 TEXT NOT NULL,
          audit_identity TEXT NOT NULL UNIQUE REFERENCES immutable_audit_outbox(audit_identity)
        );

        CREATE UNIQUE INDEX IF NOT EXISTS uq_active_budget_per_decision
        ON daily_budget_reservations(decision_identity)
        WHERE state IN ('Reserved','Accepted','Uncertain');

        CREATE UNIQUE INDEX IF NOT EXISTS uq_active_budget_slot
        ON daily_budget_reservations(business_date,slot_no)
        WHERE state IN ('Reserved','Accepted','Uncertain');

        CREATE UNIQUE INDEX IF NOT EXISTS uq_budget_attempt
        ON daily_budget_reservations(attempt_identity)
        WHERE attempt_identity IS NOT NULL;

        CREATE UNIQUE INDEX IF NOT EXISTS uq_manual_accepted_audit_identity
        ON manual_resolutions(accepted_audit_identity)
        WHERE accepted_audit_identity IS NOT NULL;

        CREATE TRIGGER IF NOT EXISTS immutable_decision_envelope_update
        BEFORE UPDATE OF envelope_version,envelope_canonical,envelope_sha256,
          business_date,push_kind,sub_kind,cooldown_scope,scope_key,
          task_binding_present,transition_basis_canonical,transition_basis_sha256
        ON delivery_decisions
        BEGIN SELECT RAISE(ABORT,'immutable delivery envelope'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_decision_delete
        BEFORE DELETE ON delivery_decisions
        BEGIN SELECT RAISE(ABORT,'delivery decisions are retained'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_claim_update
        BEFORE UPDATE ON business_date_once_claims
        BEGIN SELECT RAISE(ABORT,'business-date claim is immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_claim_delete
        BEFORE DELETE ON business_date_once_claims
        BEGIN SELECT RAISE(ABORT,'business-date claim is retained'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_sink_result_update
        BEFORE UPDATE OF result_event_identity,attempt_identity,decision_identity,
          result_kind,observed_at,fence_token,authoritative_for_state,late_after_fence,
          authority_audit_identity,late_receipt_audit_identity,result_canonical,
          result_sha256,channel,provider,message_id,platform_message_id,accepted_at,
          latency_ms,frozen_delivery_audit_canonical,frozen_delivery_audit_sha256
        ON sink_results
        BEGIN SELECT RAISE(ABORT,'sink result evidence is immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_sink_result_delete
        BEFORE DELETE ON sink_results
        BEGIN SELECT RAISE(ABORT,'sink result evidence is retained'); END;

        CREATE TRIGGER IF NOT EXISTS validate_manual_resolution_accepted_audit_insert
        BEFORE INSERT ON manual_resolutions
        WHEN NOT (
          (NEW.disposition='Rejected'
            AND NEW.frozen_delivery_audit_canonical IS NULL
            AND NEW.frozen_delivery_audit_sha256 IS NULL
            AND NEW.accepted_audit_identity IS NULL
            AND NEW.accepted_audit_append_state IS NULL
            AND NEW.accepted_audit_ref IS NULL)
          OR
          (NEW.disposition='Accepted'
            AND NEW.frozen_delivery_audit_canonical IS NOT NULL
            AND NEW.frozen_delivery_audit_sha256 IS NOT NULL
            AND NEW.accepted_audit_identity IS NOT NULL
            AND (
              (NEW.accepted_audit_append_state='Pending'
                AND NEW.accepted_audit_ref IS NULL)
              OR
              (NEW.accepted_audit_append_state='Appended'
                AND NEW.accepted_audit_ref IS NOT NULL
                AND length(replace(replace(replace(replace(
                  NEW.accepted_audit_ref,' ',''),char(9),''),char(10),''),char(13),'')) > 0)
            ))
        )
        BEGIN SELECT RAISE(ABORT,'manual accepted audit evidence is incomplete'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_manual_resolution_update
        BEFORE UPDATE OF resolution_identity,decision_identity,attempt_identity,
          disposition,operator_identity,reason,evidence_canonical,evidence_sha256,
          receipt_canonical,frozen_delivery_audit_canonical,
          frozen_delivery_audit_sha256,immutable_audit_ref,
          accepted_audit_identity,resolved_at
        ON manual_resolutions
        BEGIN SELECT RAISE(ABORT,'manual resolution evidence is immutable'); END;

        CREATE TRIGGER IF NOT EXISTS manual_accepted_audit_ack_cas
        BEFORE UPDATE OF accepted_audit_append_state,accepted_audit_ref
        ON manual_resolutions
        WHEN NOT (
          OLD.disposition='Accepted'
          AND OLD.accepted_audit_append_state='Pending'
          AND OLD.accepted_audit_ref IS NULL
          AND NEW.accepted_audit_append_state='Appended'
          AND NEW.accepted_audit_ref IS NOT NULL
          AND length(replace(replace(replace(replace(
            NEW.accepted_audit_ref,' ',''),char(9),''),char(10),''),char(13),'')) > 0
        )
        BEGIN SELECT RAISE(ABORT,'manual accepted audit acknowledgement is immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_manual_resolution_delete
        BEFORE DELETE ON manual_resolutions
        BEGIN SELECT RAISE(ABORT,'manual resolutions are retained'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_disposition_payload_update
        BEFORE UPDATE OF disposition_identity,decision_identity,attempt_identity,
          resolution_identity,denial_identity,disposition,disposition_canonical,
          disposition_sha256,created_at
        ON delivery_disposition_payloads
        BEGIN SELECT RAISE(ABORT,'delivery disposition payload is immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_disposition_payload_delete
        BEFORE DELETE ON delivery_disposition_payloads
        BEGIN SELECT RAISE(ABORT,'delivery disposition payload is retained'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_task_transition_update
        BEFORE UPDATE OF transition_identity,decision_identity,disposition_identity,
          task_binding_sha256,transition_canonical,transition_sha256
        ON task_transition_payloads
        BEGIN SELECT RAISE(ABORT,'task transition payload is immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_task_transition_delete
        BEFORE DELETE ON task_transition_payloads
        BEGIN SELECT RAISE(ABORT,'task transition payload is retained'); END;

        CREATE TRIGGER IF NOT EXISTS task_transition_hydration_ack_cas
        BEFORE UPDATE OF hydration_state,hydration_ack_identity,hydrated_at
        ON task_transition_payloads
        WHEN NOT (
          OLD.hydration_state='Pending' AND NEW.hydration_state='Applied'
          AND OLD.hydration_ack_identity IS NULL
          AND NEW.hydration_ack_identity IS NOT NULL
          AND OLD.hydrated_at IS NULL AND NEW.hydrated_at IS NOT NULL
        )
        BEGIN SELECT RAISE(ABORT,'task hydration acknowledgement is immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_outbox_payload_update
        BEFORE UPDATE OF audit_identity,decision_identity,attempt_identity,audit_kind,
          predecessor_audit_identity,audit_canonical,audit_sha256,created_at
        ON immutable_audit_outbox
        BEGIN SELECT RAISE(ABORT,'audit outbox payload is immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_outbox_delete
        BEFORE DELETE ON immutable_audit_outbox
        BEGIN SELECT RAISE(ABORT,'audit outbox is retained'); END;

        CREATE TRIGGER IF NOT EXISTS
          validate_review_terminal_replay_attempt_audit_insert
        BEFORE INSERT ON review_terminal_replay_attempts
        WHEN NOT EXISTS(
          SELECT 1
          FROM immutable_audit_outbox audit
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
        END;

        CREATE TRIGGER IF NOT EXISTS
          validate_review_terminal_replay_completion_audit_insert
        BEFORE INSERT ON review_terminal_replay_completions
        WHEN NOT EXISTS(
          SELECT 1
          FROM immutable_audit_outbox audit
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
        END;

        CREATE TRIGGER IF NOT EXISTS
          immutable_review_terminal_replay_attempt_update
        BEFORE UPDATE ON review_terminal_replay_attempts
        BEGIN
          SELECT RAISE(ABORT,'review terminal replay attempts are immutable');
        END;

        CREATE TRIGGER IF NOT EXISTS
          immutable_review_terminal_replay_attempt_delete
        BEFORE DELETE ON review_terminal_replay_attempts
        BEGIN
          SELECT RAISE(ABORT,'review terminal replay attempts are retained');
        END;

        CREATE TRIGGER IF NOT EXISTS
          immutable_review_terminal_replay_completion_update
        BEFORE UPDATE ON review_terminal_replay_completions
        BEGIN
          SELECT RAISE(ABORT,'review terminal replay completions are immutable');
        END;

        CREATE TRIGGER IF NOT EXISTS
          immutable_review_terminal_replay_completion_delete
        BEFORE DELETE ON review_terminal_replay_completions
        BEGIN
          SELECT RAISE(ABORT,'review terminal replay completions are retained');
        END;

        CREATE TRIGGER IF NOT EXISTS immutable_state_event_update
        BEFORE UPDATE ON delivery_state_events
        BEGIN SELECT RAISE(ABORT,'state events are immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_state_event_delete
        BEFORE DELETE ON delivery_state_events
        BEGIN SELECT RAISE(ABORT,'state events are retained'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_attempt_event_update
        BEFORE UPDATE ON delivery_attempt_events
        BEGIN SELECT RAISE(ABORT,'attempt events are immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_attempt_event_delete
        BEFORE DELETE ON delivery_attempt_events
        BEGIN SELECT RAISE(ABORT,'attempt events are retained'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_cooldown_event_update
        BEFORE UPDATE ON cooldown_reservation_events
        BEGIN SELECT RAISE(ABORT,'cooldown events are immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_cooldown_event_delete
        BEFORE DELETE ON cooldown_reservation_events
        BEGIN SELECT RAISE(ABORT,'cooldown events are retained'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_budget_event_update
        BEFORE UPDATE ON daily_budget_reservation_events
        BEGIN SELECT RAISE(ABORT,'budget events are immutable'); END;

        CREATE TRIGGER IF NOT EXISTS immutable_budget_event_delete
        BEFORE DELETE ON daily_budget_reservation_events
        BEGIN SELECT RAISE(ABORT,'budget events are retained'); END;
        "#,
    )?;

    seed_and_verify_policy_catalog(transaction)?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

fn migrate_schema_v1_to_v2(transaction: &Transaction<'_>) -> Result<()> {
    let manual_accept_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM manual_resolutions WHERE disposition='Accepted'",
        [],
        |row| row.get(0),
    )?;
    if manual_accept_count != 0 {
        return Err(DurableDeliveryError::InvalidConfiguration(format!(
            "schema-v1 contains {manual_accept_count} manual accepted resolution(s) without durable append acknowledgement; controlled audited recovery is required"
        )));
    }
    transaction.execute_batch(
        r#"
        ALTER TABLE manual_resolutions ADD COLUMN accepted_audit_identity TEXT;
        ALTER TABLE manual_resolutions ADD COLUMN accepted_audit_append_state TEXT
          CHECK(accepted_audit_append_state IN ('Pending','Appended'));
        ALTER TABLE manual_resolutions ADD COLUMN accepted_audit_ref TEXT;
        "#,
    )?;
    Ok(())
}

fn migrate_schema_v2_to_v3(transaction: &Transaction<'_>) -> Result<()> {
    let blank_accepted_audit_ref_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM manual_resolutions
         WHERE accepted_audit_ref IS NOT NULL
           AND length(replace(replace(replace(replace(
             accepted_audit_ref,' ',''),char(9),''),char(10),''),char(13),''))=0",
        [],
        |row| row.get(0),
    )?;
    if blank_accepted_audit_ref_count != 0 {
        return Err(DurableDeliveryError::InvalidConfiguration(format!(
            "schema-v2 contains {blank_accepted_audit_ref_count} blank manual accepted audit reference(s); controlled audited recovery is required"
        )));
    }
    validate_historical_manual_accepted_semantics(transaction)?;

    transaction.execute_batch(
        r#"
        PRAGMA defer_foreign_keys=ON;

        CREATE TABLE manual_resolutions_v3(
          resolution_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL UNIQUE REFERENCES delivery_decisions(decision_identity),
          attempt_identity TEXT NOT NULL REFERENCES delivery_attempts(attempt_identity),
          disposition TEXT NOT NULL CHECK(disposition IN ('Accepted','Rejected')),
          operator_identity TEXT NOT NULL,
          reason TEXT NOT NULL,
          evidence_canonical BLOB NOT NULL,
          evidence_sha256 TEXT NOT NULL,
          receipt_canonical BLOB,
          frozen_delivery_audit_canonical BLOB,
          frozen_delivery_audit_sha256 TEXT,
          immutable_audit_ref TEXT NOT NULL,
          accepted_audit_identity TEXT UNIQUE,
          accepted_audit_append_state TEXT
            CHECK(accepted_audit_append_state IN ('Pending','Appended')),
          accepted_audit_ref TEXT
            CHECK(accepted_audit_ref IS NULL OR
              length(replace(replace(replace(replace(
                accepted_audit_ref,' ',''),char(9),''),char(10),''),char(13),'')) > 0),
          resolved_at TEXT NOT NULL,
          CHECK (
            (disposition='Rejected'
              AND frozen_delivery_audit_canonical IS NULL
              AND frozen_delivery_audit_sha256 IS NULL
              AND accepted_audit_identity IS NULL
              AND accepted_audit_append_state IS NULL
              AND accepted_audit_ref IS NULL)
            OR
            (disposition='Accepted'
              AND frozen_delivery_audit_canonical IS NOT NULL
              AND frozen_delivery_audit_sha256 IS NOT NULL
              AND accepted_audit_identity IS NOT NULL
              AND (
                (accepted_audit_append_state='Pending' AND accepted_audit_ref IS NULL)
                OR
                (accepted_audit_append_state='Appended'
                  AND accepted_audit_ref IS NOT NULL
                  AND length(replace(replace(replace(replace(
                    accepted_audit_ref,' ',''),char(9),''),char(10),''),char(13),'')) > 0)
              ))
          )
        );

        INSERT INTO manual_resolutions_v3(
          resolution_identity,decision_identity,attempt_identity,disposition,
          operator_identity,reason,evidence_canonical,evidence_sha256,
          receipt_canonical,frozen_delivery_audit_canonical,
          frozen_delivery_audit_sha256,immutable_audit_ref,
          accepted_audit_identity,accepted_audit_append_state,
          accepted_audit_ref,resolved_at
        )
        SELECT
          resolution_identity,decision_identity,attempt_identity,disposition,
          operator_identity,reason,evidence_canonical,evidence_sha256,
          receipt_canonical,frozen_delivery_audit_canonical,
          frozen_delivery_audit_sha256,immutable_audit_ref,
          accepted_audit_identity,accepted_audit_append_state,
          accepted_audit_ref,resolved_at
        FROM manual_resolutions;

        DROP TABLE manual_resolutions;
        ALTER TABLE manual_resolutions_v3 RENAME TO manual_resolutions;
        "#,
    )?;

    verify_outbox_predecessor_self_fk(transaction, "schema-v3 to v4")?;
    let foreign_key_violation_count: i64 =
        transaction.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_violation_count != 0 {
        return Err(DurableDeliveryError::InvalidConfiguration(format!(
            "schema-v2 to v3 migration produced {foreign_key_violation_count} foreign-key violation(s)"
        )));
    }
    Ok(())
}

fn migrate_schema_v3_to_v4(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(
        r#"
        PRAGMA defer_foreign_keys=ON;

        DROP TRIGGER IF EXISTS immutable_outbox_payload_update;
        DROP TRIGGER IF EXISTS immutable_outbox_delete;

        CREATE TABLE immutable_audit_outbox_v4(
          audit_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          attempt_identity TEXT REFERENCES delivery_attempts(attempt_identity),
          audit_kind TEXT NOT NULL CHECK(audit_kind IN (
            'DecisionStateChanged','LeaseGranted','LeaseHeartbeat',
            'FenceRevoked','RecoveryClassified','SinkResultAuthorityClassified',
            'LateReceiptObserved','BudgetReservationChanged',
            'CooldownReservationChanged','BusinessDateOnceClaimed',
            'DecisionIdentityConflict','ScheduleHydrationApplied',
            'ReviewTerminalReplayStarted','ReviewTerminalReplayCompleted')),
          predecessor_audit_identity TEXT REFERENCES immutable_audit_outbox_v4(audit_identity),
          audit_canonical BLOB NOT NULL,
          audit_sha256 TEXT NOT NULL,
          append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
          immutable_audit_ref TEXT,
          created_at TEXT NOT NULL
        );

        INSERT INTO immutable_audit_outbox_v4(
          audit_identity,decision_identity,attempt_identity,audit_kind,
          predecessor_audit_identity,audit_canonical,audit_sha256,
          append_state,immutable_audit_ref,created_at
        )
        SELECT
          audit_identity,decision_identity,attempt_identity,audit_kind,
          predecessor_audit_identity,audit_canonical,audit_sha256,
          append_state,immutable_audit_ref,created_at
        FROM immutable_audit_outbox;

        DROP TABLE immutable_audit_outbox;
        ALTER TABLE immutable_audit_outbox_v4 RENAME TO immutable_audit_outbox;
        "#,
    )?;

    verify_outbox_predecessor_self_fk(transaction, "schema-v4 to v5")?;
    let foreign_key_violation_count: i64 =
        transaction.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_violation_count != 0 {
        return Err(DurableDeliveryError::InvalidConfiguration(format!(
            "schema-v3 to v4 migration produced {foreign_key_violation_count} foreign-key violation(s)"
        )));
    }
    Ok(())
}

fn migrate_schema_v4_to_v5(transaction: &Transaction<'_>) -> Result<()> {
    type MigratedAuditOutboxRow = (
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Vec<u8>,
        String,
        String,
        Option<String>,
        String,
    );

    transaction.execute_batch(r#"PRAGMA defer_foreign_keys=ON;"#)?;
    transaction.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS validate_review_terminal_replay_attempt_audit_insert;
        DROP TRIGGER IF EXISTS validate_review_terminal_replay_completion_audit_insert;
        DROP TRIGGER IF EXISTS immutable_outbox_payload_update;
        DROP TRIGGER IF EXISTS immutable_outbox_delete;
        "#,
    )?;
    transaction.execute_batch(
        r#"
        CREATE TABLE immutable_audit_outbox_v5(
          audit_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          attempt_identity TEXT REFERENCES delivery_attempts(attempt_identity),
          audit_kind TEXT NOT NULL CHECK(audit_kind IN (
            'DecisionStateChanged','LeaseGranted','LeaseHeartbeat',
            'FenceRevoked','RecoveryClassified','SinkResultAuthorityClassified',
            'LateReceiptObserved','BudgetReservationChanged',
            'CooldownReservationChanged','BusinessDateOnceClaimed',
            'DecisionIdentityConflict','ScheduleHydrationApplied',
            'ReviewTerminalReplayStarted','ReviewTerminalReplayCompleted')),
          predecessor_audit_identity TEXT,
          audit_canonical BLOB NOT NULL,
          audit_sha256 TEXT NOT NULL,
          append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
          immutable_audit_ref TEXT,
          created_at TEXT NOT NULL
        );
        "#,
    )?;
    let insert_rows: Vec<MigratedAuditOutboxRow> = {
        let mut stmt = transaction.prepare(
            "SELECT
               audit_identity, decision_identity, attempt_identity, audit_kind,
               predecessor_audit_identity, audit_canonical, audit_sha256,
               append_state, immutable_audit_ref, created_at
             FROM immutable_audit_outbox
             ORDER BY
               CASE WHEN predecessor_audit_identity IS NULL THEN 0 ELSE 1 END ASC,
               predecessor_audit_identity ASC,
               audit_identity ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    {
        let mut stmt = transaction.prepare(
            "INSERT INTO immutable_audit_outbox_v5(
               audit_identity, decision_identity, attempt_identity, audit_kind,
               predecessor_audit_identity, audit_canonical, audit_sha256,
               append_state, immutable_audit_ref, created_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        )?;
        for row in &insert_rows {
            stmt.execute(rusqlite::params![
                row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9,
            ])?;
        }
    }
    transaction.execute_batch("DROP TABLE immutable_audit_outbox;")?;
    transaction
        .execute_batch("ALTER TABLE immutable_audit_outbox_v5 RENAME TO immutable_audit_outbox;")?;

    // The v5 staging table intentionally omits the predecessor self-FK so
    // the row-by-row INSERT cannot trip a per-statement FK check. SQLite's
    // ALTER TABLE ... RENAME does not update self-FK references declared
    // in the CREATE TABLE body, so re-creating the table with the canonical
    // FK target (`immutable_audit_outbox`) preserves the BR-192 §2.0.1
    // contract: predecessor_audit_identity REFERENCES immutable_audit_outbox
    // (audit_identity). The post-recreate INSERT benefits from
    // `defer_foreign_keys=ON` because the predecessor rows are already
    // committed in `immutable_audit_outbox` before the copy starts.
    transaction.execute_batch(
        r#"
        CREATE TABLE immutable_audit_outbox_v5_with_fk(
          audit_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
          attempt_identity TEXT REFERENCES delivery_attempts(attempt_identity),
          audit_kind TEXT NOT NULL CHECK(audit_kind IN (
            'DecisionStateChanged','LeaseGranted','LeaseHeartbeat',
            'FenceRevoked','RecoveryClassified','SinkResultAuthorityClassified',
            'LateReceiptObserved','BudgetReservationChanged',
            'CooldownReservationChanged','BusinessDateOnceClaimed',
            'DecisionIdentityConflict','ScheduleHydrationApplied',
            'ReviewTerminalReplayStarted','ReviewTerminalReplayCompleted')),
          predecessor_audit_identity TEXT REFERENCES immutable_audit_outbox(audit_identity),
          audit_canonical BLOB NOT NULL,
          audit_sha256 TEXT NOT NULL,
          append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
          immutable_audit_ref TEXT,
          created_at TEXT NOT NULL
        );

        INSERT INTO immutable_audit_outbox_v5_with_fk(
          audit_identity, decision_identity, attempt_identity, audit_kind,
          predecessor_audit_identity, audit_canonical, audit_sha256,
          append_state, immutable_audit_ref, created_at
        )
        SELECT
          audit_identity, decision_identity, attempt_identity, audit_kind,
          predecessor_audit_identity, audit_canonical, audit_sha256,
          append_state, immutable_audit_ref, created_at
        FROM immutable_audit_outbox
        ORDER BY
          CASE WHEN predecessor_audit_identity IS NULL THEN 0 ELSE 1 END ASC,
          predecessor_audit_identity ASC,
          audit_identity ASC;

        DROP TABLE immutable_audit_outbox;
        ALTER TABLE immutable_audit_outbox_v5_with_fk RENAME TO immutable_audit_outbox;
        "#,
    )?;

    verify_outbox_predecessor_self_fk(transaction, "schema-v4 to v5")?;

    let foreign_key_violation_count: i64 =
        transaction.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_violation_count != 0 {
        let mut violations = String::new();
        let mut stmt = transaction
            .prepare("SELECT \"table\",\"from\",\"to\",fkid FROM pragma_foreign_key_check")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (table, from, to, fkid) = row?;
            violations.push_str(&format!(" [table={table} from={from} to={to} fkid={fkid}]"));
        }
        return Err(DurableDeliveryError::InvalidConfiguration(format!(
            "schema-v4 to v5 migration produced {foreign_key_violation_count} foreign-key violation(s):{violations}"
        )));
    }
    Ok(())
}

/// BR-214: replay `delivery_policy_catalog` from the compiled catalog.
///
/// `seed_and_verify_policy_catalog` inserts with `INSERT OR IGNORE` and then
/// compares every stored row against the compiled catalog, so a semantic policy
/// change (daily review kinds moving from `Rolling` to `BusinessDateOnce`, plus
/// the accompanying `POLICY_VERSION` bump) would otherwise leave the pre-existing
/// rows untouched and abort startup with `PolicyMismatch`.
///
/// Only the policy catalog is replayed. Existing decisions, cooldown heads,
/// business-date claims and audit rows are left untouched: the `POLICY_VERSION`
/// bump already yields fresh `decision_identity` values for new envelopes, and
/// historical rows must remain readable for audit (AGENTS §2.7).
/// BR-237 (2026-08-13): rebuild `delivery_policy_catalog` with the relaxed
/// `counts_against_daily_budget IN (0,1)` CHECK.
///
/// The v6 table still enforces `CHECK(counts_against_daily_budget=1)`, which
/// would reject the review-kind exemption rows (flag=0) seeded after this
/// migration. SQLite cannot ALTER a CHECK constraint, so the table is rebuilt
/// via the standard 12-step rename/create/copy/drop. No FK references the
/// policy catalog, so the rebuild is safe. Existing decisions, cooldown heads,
/// claims and audit rows are untouched (AGENTS §2.7), mirroring the v5→v6
/// replay semantics: the `POLICY_VERSION` bump yields fresh decision identity.
fn migrate_schema_v6_to_v7(transaction: &Transaction<'_>) -> Result<()> {
    let table_exists: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table' AND name='delivery_policy_catalog'",
        [],
        |row| row.get(0),
    )?;
    if table_exists == 0 {
        return Ok(());
    }
    transaction.execute(
        "ALTER TABLE delivery_policy_catalog
           RENAME TO delivery_policy_catalog_v6",
        [],
    )?;
    transaction.execute_batch(
        "CREATE TABLE delivery_policy_catalog(
          push_kind TEXT NOT NULL,
          sub_kind TEXT NOT NULL,
          cooldown_scope TEXT NOT NULL,
          base_cooldown_secs INTEGER,
          override_cooldown_secs INTEGER,
          window_mode TEXT NOT NULL CHECK(window_mode IN
            ('None','Rolling','BusinessDateOnce')),
          counts_against_daily_budget INTEGER NOT NULL CHECK(
            counts_against_daily_budget IN (0,1)),
          policy_version INTEGER NOT NULL,
          PRIMARY KEY(push_kind,sub_kind)
        );
        INSERT INTO delivery_policy_catalog
          SELECT push_kind,sub_kind,cooldown_scope,base_cooldown_secs,
                 override_cooldown_secs,window_mode,counts_against_daily_budget,
                 policy_version
          FROM delivery_policy_catalog_v6;
        DROP TABLE delivery_policy_catalog_v6;",
    )?;
    Ok(())
}

/// BR-241: replay only the compiled delivery policy catalog so the new P-01
/// Global BusinessDateOnce row and policy version become authoritative.
///
/// Delivery decisions, attempts, business-date claims, sink results, cooldown
/// heads and immutable audits are intentionally outside this migration and
/// remain untouched (AGENTS §2.7).
fn migrate_schema_v7_to_v8(transaction: &Transaction<'_>) -> Result<()> {
    let table_exists: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table' AND name='delivery_policy_catalog'",
        [],
        |row| row.get(0),
    )?;
    if table_exists != 0 {
        transaction.execute("DELETE FROM delivery_policy_catalog", [])?;
    }
    Ok(())
}

/// BR-245: replay only the compiled policy catalog so TomorrowWatch changes
/// from a rolling, budget-counted signal row to the Global BusinessDateOnce,
/// budget-exempt R-07 review policy.
///
/// Historical delivery authority is immutable: decisions, attempts,
/// business-date claims, sink results, cooldown projections and immutable
/// audit outbox rows are deliberately outside this migration (AGENTS §2.7).
fn migrate_schema_v8_to_v9(transaction: &Transaction<'_>) -> Result<()> {
    let table_exists: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table' AND name='delivery_policy_catalog'",
        [],
        |row| row.get(0),
    )?;
    if table_exists != 0 {
        transaction.execute("DELETE FROM delivery_policy_catalog", [])?;
    }
    Ok(())
}

fn migrate_schema_v5_to_v6(transaction: &Transaction<'_>) -> Result<()> {
    let table_exists: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table' AND name='delivery_policy_catalog'",
        [],
        |row| row.get(0),
    )?;
    if table_exists == 0 {
        return Ok(());
    }
    transaction.execute("DELETE FROM delivery_policy_catalog", [])?;
    Ok(())
}

fn verify_outbox_predecessor_self_fk(transaction: &Transaction<'_>, migration: &str) -> Result<()> {
    let exact_self_fk_count: i64 = transaction.query_row(
        "SELECT COUNT(*)
         FROM pragma_foreign_key_list('immutable_audit_outbox')
         WHERE \"from\"='predecessor_audit_identity'
           AND \"to\"='audit_identity'
           AND \"table\" IN ('immutable_audit_outbox','immutable_audit_outbox_v4','immutable_audit_outbox_v5')",
        [],
        |row| row.get(0),
    )?;
    if exact_self_fk_count != 1 {
        let mut stmt = transaction.prepare(
            "SELECT \"table\",\"from\",\"to\" FROM pragma_foreign_key_list('immutable_audit_outbox')"
        )?;
        let fk_rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut dump = String::new();
        for (t, f, to) in &fk_rows {
            dump.push_str(&format!(" [table={t} from={f} to={to}]"));
        }
        return Err(DurableDeliveryError::InvalidConfiguration(format!(
            "{migration} migration did not preserve the exact immutable-audit predecessor self-FK (got {exact_self_fk_count}; actual fk rows:{dump})"
        )));
    }
    Ok(())
}

fn validate_historical_manual_accepted_semantics(transaction: &Transaction<'_>) -> Result<()> {
    let mut statement = transaction.prepare(
        "SELECT m.decision_identity,m.resolution_identity,
                m.attempt_identity,d.current_attempt_identity,a.state,
                m.operator_identity,m.reason,m.immutable_audit_ref,
                d.envelope_sha256,d.current_disposition_identity,
                p.disposition_identity,p.disposition_canonical,
                p.disposition_sha256,p.append_state,p.immutable_audit_ref,
                m.evidence_canonical,m.evidence_sha256,m.receipt_canonical,m.resolved_at,
                m.accepted_audit_identity,m.frozen_delivery_audit_canonical,
                m.frozen_delivery_audit_sha256,m.accepted_audit_append_state,
                m.accepted_audit_ref
         FROM manual_resolutions m
         LEFT JOIN delivery_decisions d
           ON d.decision_identity=m.decision_identity
         LEFT JOIN delivery_attempts a
           ON a.attempt_identity=m.attempt_identity
          AND a.decision_identity=m.decision_identity
         LEFT JOIN delivery_disposition_payloads p
           ON p.disposition_identity=d.current_disposition_identity
          AND p.resolution_identity=m.resolution_identity
          AND p.decision_identity=m.decision_identity
          AND p.disposition='ManualAccepted'
         WHERE m.disposition='Accepted'
         ORDER BY m.decision_identity",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<Vec<u8>>>(11)?,
            row.get::<_, Option<String>>(12)?,
            row.get::<_, Option<String>>(13)?,
            row.get::<_, Option<String>>(14)?,
            row.get::<_, Vec<u8>>(15)?,
            row.get::<_, String>(16)?,
            row.get::<_, Option<Vec<u8>>>(17)?,
            row.get::<_, String>(18)?,
            row.get::<_, Option<String>>(19)?,
            row.get::<_, Option<Vec<u8>>>(20)?,
            row.get::<_, Option<String>>(21)?,
            row.get::<_, Option<String>>(22)?,
            row.get::<_, Option<String>>(23)?,
        ))
    })?;
    for row in rows {
        let (
            decision_identity,
            resolution_identity,
            attempt_identity,
            decision_current_attempt_identity,
            attempt_state,
            operator_identity,
            reason,
            authorization_immutable_audit_ref,
            envelope_sha256,
            current_disposition_identity,
            disposition_identity,
            disposition_canonical,
            disposition_sha256,
            disposition_append_state,
            disposition_immutable_audit_ref,
            acceptance_evidence_canonical,
            acceptance_evidence_sha256,
            receipt_canonical,
            resolved_at,
            audit_identity,
            canonical,
            sha256,
            append_state,
            immutable_audit_ref,
        ) = row?;
        let missing = |field: &str| {
            DurableDeliveryError::InvalidConfiguration(format!(
                "schema-v2 manual accepted row {decision_identity} is missing {field}; controlled audited recovery is required"
            ))
        };
        let evidence = ManualAcceptedDeliveryAuditEvidence {
            decision_identity: decision_identity.clone(),
            resolution_identity,
            attempt_identity,
            decision_current_attempt_identity: decision_current_attempt_identity
                .ok_or_else(|| missing("decision current attempt identity"))?,
            attempt_state: attempt_state.ok_or_else(|| missing("original uncertain attempt"))?,
            operator_identity,
            reason,
            authorization_immutable_audit_ref,
            envelope_sha256: envelope_sha256.ok_or_else(|| missing("envelope hash"))?,
            current_disposition_identity: current_disposition_identity
                .ok_or_else(|| missing("current disposition identity"))?,
            disposition_identity: disposition_identity
                .ok_or_else(|| missing("current manual disposition"))?,
            disposition_canonical: disposition_canonical
                .ok_or_else(|| missing("current manual disposition canonical"))?,
            disposition_sha256: disposition_sha256
                .ok_or_else(|| missing("current manual disposition hash"))?,
            disposition_append_state: disposition_append_state
                .ok_or_else(|| missing("current manual disposition append state"))?,
            disposition_immutable_audit_ref,
            acceptance_evidence_canonical,
            acceptance_evidence_sha256,
            receipt_canonical,
            resolved_at,
            audit_identity: audit_identity.ok_or_else(|| missing("accepted audit identity"))?,
            canonical: canonical.ok_or_else(|| missing("accepted audit canonical"))?,
            sha256: sha256.ok_or_else(|| missing("accepted audit hash"))?,
            append_state: append_state.ok_or_else(|| missing("accepted audit append state"))?,
            accepted_audit_immutable_ref: immutable_audit_ref,
        };
        evidence.validate_for_migration().map_err(|error| {
            DurableDeliveryError::InvalidConfiguration(format!(
                "schema-v2 manual accepted row {decision_identity} has invalid semantic binding: {error}; controlled audited recovery is required"
            ))
        })?;
    }
    Ok(())
}

fn seed_and_verify_policy_catalog(transaction: &Transaction<'_>) -> Result<()> {
    let expected = compiled_policy_catalog();
    for row in &expected {
        // BR-237: OR IGNORE → upsert, 否则存量行 (旧 CHECK 时代 counts=1)
        // 不会被刷新为豁免 flag, 启动即 PolicyMismatch。
        transaction.execute(
            "INSERT INTO delivery_policy_catalog(
                push_kind,sub_kind,cooldown_scope,base_cooldown_secs,
                override_cooldown_secs,window_mode,counts_against_daily_budget,policy_version
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(push_kind,sub_kind) DO UPDATE SET
               cooldown_scope=excluded.cooldown_scope,
               base_cooldown_secs=excluded.base_cooldown_secs,
               override_cooldown_secs=excluded.override_cooldown_secs,
               window_mode=excluded.window_mode,
               counts_against_daily_budget=excluded.counts_against_daily_budget,
               policy_version=excluded.policy_version",
            params![
                row.push_kind.as_str(),
                row.sub_kind.as_str(),
                row.cooldown_scope.as_str(),
                row.base_cooldown_secs,
                row.override_cooldown_secs,
                row.window_mode.as_str(),
                i64::from(row.counts_against_daily_budget),
                row.policy_version,
            ],
        )?;
    }
    let rows = {
        let mut statement = transaction.prepare(
            "SELECT push_kind,sub_kind,cooldown_scope,base_cooldown_secs,
                    override_cooldown_secs,window_mode,counts_against_daily_budget,policy_version
             FROM delivery_policy_catalog ORDER BY push_kind,sub_kind",
        )?;
        let mapped = statement.query_map([], policy_from_row)?;
        mapped.collect::<std::result::Result<Vec<_>, _>>()?
    };
    if rows.len() != 26 {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "seeded policy catalog must have 26 rows, got {}",
            rows.len()
        )));
    }
    let distinct = rows
        .iter()
        .map(|row| row.push_kind)
        .collect::<BTreeSet<_>>();
    if distinct.len() != 23 {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "seeded policy catalog must have 23 kinds, got {}",
            distinct.len()
        )));
    }
    for expected_row in &expected {
        let stored = rows
            .iter()
            .find(|candidate| {
                candidate.push_kind == expected_row.push_kind
                    && candidate.sub_kind == expected_row.sub_kind
            })
            .ok_or_else(|| {
                DurableDeliveryError::PolicyMismatch(format!(
                    "missing seeded policy {}/{}",
                    expected_row.push_kind, expected_row.sub_kind
                ))
            })?;
        if stored != expected_row {
            return Err(DurableDeliveryError::PolicyMismatch(format!(
                "seeded policy differs from compiled catalog for {}/{}",
                expected_row.push_kind, expected_row.sub_kind
            )));
        }
    }
    Ok(())
}

pub(crate) fn load_policy(
    connection: &Connection,
    push_kind: PushKind,
    sub_kind: DeliverySubKind,
) -> Result<PolicyRow> {
    connection
        .query_row(
            "SELECT push_kind,sub_kind,cooldown_scope,base_cooldown_secs,
                    override_cooldown_secs,window_mode,counts_against_daily_budget,policy_version
             FROM delivery_policy_catalog WHERE push_kind=?1 AND sub_kind=?2",
            params![push_kind.as_str(), sub_kind.as_str()],
            policy_from_row,
        )
        .optional()?
        .ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(format!(
                "missing registered policy for {push_kind}/{sub_kind}"
            ))
        })
}

fn policy_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PolicyRow> {
    let push_kind_raw: String = row.get(0)?;
    let sub_kind_raw: String = row.get(1)?;
    let scope_raw: String = row.get(2)?;
    let window_raw: String = row.get(5)?;
    let parsed = (|| -> Result<PolicyRow> {
        Ok(PolicyRow {
            push_kind: PushKind::parse(&push_kind_raw)?,
            sub_kind: DeliverySubKind::parse(&sub_kind_raw)?,
            cooldown_scope: CooldownScope::parse(&scope_raw)?,
            base_cooldown_secs: row.get(3)?,
            override_cooldown_secs: row.get(4)?,
            window_mode: WindowMode::parse(&window_raw)?,
            counts_against_daily_budget: row.get::<_, i64>(6)? == 1,
            policy_version: row.get(7)?,
        })
    })();
    parsed.map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}
