
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
        