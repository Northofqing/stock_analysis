-- Business-Rule: BR-180 exact frozen pre-amendment schema fixture.
CREATE TABLE IF NOT EXISTS selection_v2_recovery_envelopes (
    stage_run_id TEXT PRIMARY KEY NOT NULL,
    subject_kind TEXT NOT NULL CHECK (
        subject_kind IN ('config_activation','ingress_run','generation_run','outcome_run')
    ),
    logical_subject_key TEXT NOT NULL,
    payload_schema TEXT NOT NULL CHECK (
        (subject_kind='config_activation' AND payload_schema='config-activation-stage-v1')
        OR (subject_kind='ingress_run' AND payload_schema='source-ingress-stage-v2')
        OR (subject_kind='generation_run' AND payload_schema='generation-stage-v2')
        OR (subject_kind='outcome_run' AND payload_schema='outcome-stage-v2')
    ),
    payload_json TEXT NOT NULL CHECK (
        json_valid(payload_json)
        AND COALESCE(json_type(payload_json, '$.domain')='text', 0)
        AND (
            (subject_kind='config_activation'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.config_activation_stage.v1')
            OR
            (subject_kind='ingress_run'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.source_ingress_stage.v2')
            OR
            (subject_kind='generation_run'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.generation_stage.v2')
            OR
            (subject_kind='outcome_run'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.outcome_stage.v2')
        )
    ),
    payload_json_hash TEXT NOT NULL,
    in_memory_payload_hash TEXT NOT NULL,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    enveloped_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        subject_kind<>'config_activation'
        OR config_activation_run_id=stage_run_id
    ),
    UNIQUE(stage_run_id, payload_json_hash, in_memory_payload_hash)
);

CREATE TABLE IF NOT EXISTS selection_source_batch_attempts (
    source_batch_attempt_id TEXT PRIMARY KEY NOT NULL,
    ingress_run_id TEXT NOT NULL,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    generation_market_date TEXT NOT NULL,
    registered_feed_identity TEXT NOT NULL,
    registered_feed_snapshot_hash TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    request_evidence_json TEXT NOT NULL CHECK (json_valid(request_evidence_json)),
    request_evidence_hash TEXT NOT NULL,
    feed_attempt_content_hash TEXT NOT NULL,
    status_kind TEXT NOT NULL CHECK (
        status_kind IN ('available','verified_empty','unavailable')
    ),
    record_count INTEGER CHECK (record_count >= 0),
    provider TEXT,
    source TEXT,
    source_at TEXT,
    observed_at TEXT,
    batch_id TEXT,
    batch_content_hash TEXT,
    failed_stage TEXT,
    reason_code TEXT,
    retryable INTEGER CHECK (retryable IN (0,1)),
    available_evidence_json TEXT,
    available_evidence_hash TEXT,
    error_detail_json TEXT,
    error_detail_hash TEXT,
    error_fingerprint TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (available_evidence_json IS NOT NULL AND available_evidence_hash IS NOT NULL
            AND json_valid(available_evidence_json))
    ),
    CHECK (
        (error_detail_json IS NULL AND error_detail_hash IS NULL)
        OR
        (error_detail_json IS NOT NULL AND error_detail_hash IS NOT NULL
            AND json_valid(error_detail_json))
    ),
    CHECK (
        (status_kind='available' AND record_count IS NOT NULL AND record_count>0
            AND provider IS NOT NULL AND source IS NOT NULL
            AND source_at IS NOT NULL AND observed_at IS NOT NULL
            AND batch_id IS NOT NULL AND batch_content_hash IS NOT NULL
            AND available_evidence_json IS NOT NULL
            AND failed_stage IS NULL AND reason_code IS NULL AND retryable IS NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL)
        OR
        (status_kind='verified_empty' AND record_count IS NOT NULL AND record_count=0
            AND provider IS NOT NULL AND source IS NOT NULL
            AND source_at IS NOT NULL AND observed_at IS NOT NULL
            AND batch_id IS NOT NULL AND batch_content_hash IS NOT NULL
            AND available_evidence_json IS NOT NULL
            AND failed_stage IS NULL AND reason_code IS NULL AND retryable IS NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL)
        OR
        (status_kind='unavailable' AND record_count IS NULL
            AND batch_content_hash IS NULL
            AND failed_stage IS NOT NULL AND length(failed_stage)>0
            AND reason_code IS NOT NULL AND length(reason_code)>0
            AND retryable IS NOT NULL
            AND error_detail_json IS NOT NULL AND error_fingerprint IS NOT NULL
            AND (
                (available_evidence_json IS NULL
                    AND provider IS NULL AND source IS NULL AND source_at IS NULL
                    AND observed_at IS NULL AND batch_id IS NULL)
                OR
                (available_evidence_json IS NOT NULL
                    AND (provider IS NOT NULL OR source IS NOT NULL
                         OR source_at IS NOT NULL OR observed_at IS NOT NULL
                         OR batch_id IS NOT NULL))
            ))
    ),
    FOREIGN KEY(ingress_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(ingress_run_id, registered_feed_identity)
);

CREATE TABLE IF NOT EXISTS selection_source_facts_v2 (
    source_fact_key TEXT PRIMARY KEY NOT NULL,
    event_id TEXT NOT NULL,
    payload_schema TEXT NOT NULL CHECK (payload_schema='global-news-source-fact-v2'),
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    generation_market_date TEXT NOT NULL,
    provider_source TEXT NOT NULL,
    item_id TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT,
    content TEXT,
    publisher TEXT,
    canonical_url TEXT,
    published_at TEXT,
    instruments_json TEXT NOT NULL CHECK (json_valid(instruments_json)),
    topics_json TEXT NOT NULL CHECK (json_valid(topics_json)),
    language TEXT,
    record_provider TEXT NOT NULL,
    record_source TEXT NOT NULL,
    record_source_at TEXT,
    record_observed_at TEXT NOT NULL,
    record_batch_id TEXT NOT NULL,
    record_batch_content_hash TEXT NOT NULL,
    provider_content_hash TEXT NOT NULL,
    first_ingress_run_id TEXT NOT NULL,
    ingress_gate_version TEXT NOT NULL,
    ingress_gate_input_json TEXT NOT NULL CHECK (json_valid(ingress_gate_input_json)),
    ingress_gate_input_hash TEXT NOT NULL,
    ingress_decision TEXT NOT NULL CHECK (ingress_decision IN ('admitted','rejected')),
    ingress_reason_code TEXT,
    ingress_retryable INTEGER CHECK (ingress_retryable IN (0,1)),
    ingress_gate_receipt_json TEXT NOT NULL CHECK (json_valid(ingress_gate_receipt_json)),
    ingress_gate_receipt_hash TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (ingress_decision='admitted' AND ingress_reason_code IS NULL
            AND ingress_retryable IS NULL)
        OR
        (ingress_decision='rejected' AND length(ingress_reason_code)>0
            AND ingress_retryable=0)
    ),
    FOREIGN KEY(first_ingress_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS selection_source_fact_attempts (
    source_fact_attempt_id TEXT PRIMARY KEY NOT NULL,
    ingress_run_id TEXT NOT NULL,
    source_batch_attempt_id TEXT NOT NULL,
    provider_ordinal INTEGER NOT NULL CHECK (provider_ordinal >= 0),
    source_fact_key TEXT NOT NULL,
    acquired_record_json TEXT NOT NULL CHECK (json_valid(acquired_record_json)),
    acquired_record_hash TEXT NOT NULL,
    batch_evidence_json TEXT NOT NULL CHECK (json_valid(batch_evidence_json)),
    batch_evidence_hash TEXT NOT NULL,
    event_projection_id TEXT NOT NULL,
    attempt_result TEXT NOT NULL CHECK (
        attempt_result IN ('inserted','exact_replay','conflict')
    ),
    conflict_hash TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (attempt_result IN ('inserted','exact_replay') AND conflict_hash IS NULL)
        OR (attempt_result='conflict' AND length(conflict_hash)>0)
    ),
    FOREIGN KEY(ingress_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_batch_attempt_id)
        REFERENCES selection_source_batch_attempts(source_batch_attempt_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_fact_key)
        REFERENCES selection_source_facts_v2(source_fact_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(source_batch_attempt_id, provider_ordinal)
);

CREATE TABLE IF NOT EXISTS selection_relation_attempts (
    relation_attempt_id TEXT PRIMARY KEY NOT NULL,
    relation_key TEXT NOT NULL,
    generation_run_id TEXT NOT NULL,
    source_fact_key TEXT NOT NULL,
    event_id TEXT NOT NULL,
    chain_id TEXT NOT NULL,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    relation_schema_version TEXT NOT NULL CHECK (relation_schema_version='event-relation-v2'),
    relation_kind TEXT NOT NULL CHECK (
        relation_kind IN ('direct_mention','provider_board_constituent')
    ),
    relation_source_identity_json TEXT NOT NULL CHECK (json_valid(relation_source_identity_json)),
    relation_source_identity_hash TEXT NOT NULL,
    typed_binding_state_json TEXT NOT NULL CHECK (json_valid(typed_binding_state_json)),
    typed_binding_state_hash TEXT NOT NULL,
    request_hash TEXT,
    request_evidence_json TEXT,
    request_evidence_hash TEXT,
    result_code TEXT NOT NULL CHECK (result_code IN ('resolved','rejected','unsupported')),
    failed_stage TEXT,
    retryable INTEGER CHECK (retryable IN (0,1)),
    raw_identity_json TEXT,
    raw_identity_hash TEXT,
    canonical_stock_code TEXT,
    canonical_stock_name TEXT,
    canonical_market TEXT,
    artifact_content_hash TEXT,
    binding_audit_hash TEXT,
    provider_board_kind TEXT,
    provider_board_code TEXT,
    provider_board_name TEXT,
    provider_source TEXT,
    provider_source_at TEXT,
    provider_observed_at TEXT,
    provider_batch_id TEXT,
    provider_batch_content_hash TEXT,
    actual_constituent_count INTEGER CHECK (actual_constituent_count >= 0),
    available_evidence_json TEXT,
    available_evidence_hash TEXT,
    error_detail_json TEXT,
    error_detail_hash TEXT,
    error_fingerprint TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (request_hash IS NULL
            AND request_evidence_json IS NULL AND request_evidence_hash IS NULL)
        OR
        (request_hash IS NOT NULL
            AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND json_valid(request_evidence_json))
    ),
    CHECK (
        (raw_identity_json IS NULL AND raw_identity_hash IS NULL)
        OR
        (raw_identity_json IS NOT NULL AND raw_identity_hash IS NOT NULL
            AND json_valid(raw_identity_json))
    ),
    CHECK (
        (available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (available_evidence_json IS NOT NULL AND available_evidence_hash IS NOT NULL
            AND json_valid(available_evidence_json))
    ),
    CHECK (
        (error_detail_json IS NULL AND error_detail_hash IS NULL)
        OR
        (error_detail_json IS NOT NULL AND error_detail_hash IS NOT NULL
            AND json_valid(error_detail_json))
    ),
    CHECK (
        (result_code='resolved' AND failed_stage IS NULL AND retryable IS NULL
            AND canonical_stock_code IS NOT NULL
            AND canonical_stock_name IS NOT NULL AND canonical_market IS NOT NULL
            AND raw_identity_json IS NOT NULL
            AND (
                (relation_kind='direct_mention'
                    AND available_evidence_json IS NULL
                    AND available_evidence_hash IS NULL)
                OR
                (relation_kind='provider_board_constituent'
                    AND available_evidence_json IS NOT NULL
                    AND available_evidence_hash IS NOT NULL)
            )
            AND error_detail_json IS NULL AND error_fingerprint IS NULL)
        OR
        (result_code IN ('rejected','unsupported')
            AND failed_stage IS NOT NULL AND length(failed_stage)>0
            AND retryable IS NOT NULL
            AND error_detail_json IS NOT NULL AND error_fingerprint IS NOT NULL)
    ),
    CHECK (
        (relation_kind='direct_mention'
            AND json_extract(typed_binding_state_json, '$.state')='direct_not_applicable'
            AND request_hash IS NULL AND request_evidence_json IS NULL
            AND request_evidence_hash IS NULL
            AND artifact_content_hash IS NULL AND binding_audit_hash IS NULL
            AND provider_board_kind IS NULL AND provider_board_code IS NULL
            AND provider_board_name IS NULL AND actual_constituent_count IS NULL
            AND provider_source IS NULL AND provider_source_at IS NULL
            AND provider_observed_at IS NULL AND provider_batch_id IS NULL
            AND provider_batch_content_hash IS NULL
            AND available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (relation_kind='provider_board_constituent'
            AND (
                (artifact_content_hash IS NULL AND binding_audit_hash IS NULL
                    AND provider_board_kind IS NULL AND provider_board_code IS NULL
                    AND provider_board_name IS NULL
                    AND request_hash IS NULL AND request_evidence_json IS NULL
                    AND request_evidence_hash IS NULL
                    AND json_extract(typed_binding_state_json, '$.state')='not_configured'
                    AND result_code IN ('rejected','unsupported'))
                OR
                (artifact_content_hash IS NOT NULL AND binding_audit_hash IS NOT NULL
                    AND provider_board_kind IS NOT NULL AND provider_board_code IS NOT NULL
                    AND provider_board_name IS NOT NULL
                    AND request_hash IS NOT NULL AND request_evidence_json IS NOT NULL
                    AND request_evidence_hash IS NOT NULL
                    AND json_extract(typed_binding_state_json, '$.state')='verified')
            ))
    ),
    CHECK (
        relation_kind<>'provider_board_constituent' OR result_code<>'resolved'
        OR (
            provider_source IS NOT NULL AND provider_observed_at IS NOT NULL
            AND provider_batch_id IS NOT NULL AND provider_batch_content_hash IS NOT NULL
            AND actual_constituent_count>0
        )
    ),
    FOREIGN KEY(generation_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_fact_key)
        REFERENCES selection_source_facts_v2(source_fact_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(generation_run_id, relation_key)
);

CREATE TABLE IF NOT EXISTS selection_evaluation_attempts (
    evaluation_attempt_id TEXT PRIMARY KEY NOT NULL,
    sample_key TEXT NOT NULL,
    generation_run_id TEXT NOT NULL,
    source_fact_key TEXT NOT NULL,
    event_id TEXT NOT NULL,
    chain_id TEXT NOT NULL,
    canonical_stock_code TEXT NOT NULL,
    canonical_stock_name TEXT NOT NULL,
    canonical_market TEXT NOT NULL,
    relation_evidence_set_hash TEXT NOT NULL,
    market_request_hash TEXT NOT NULL,
    request_evidence_json TEXT NOT NULL CHECK (json_valid(request_evidence_json)),
    request_evidence_hash TEXT NOT NULL,
    result_code TEXT NOT NULL CHECK (result_code IN ('completed','error')),
    failed_stage TEXT,
    retryable INTEGER CHECK (retryable IN (0,1)),
    provider TEXT,
    source TEXT,
    source_at TEXT,
    observed_at TEXT,
    batch_id TEXT,
    batch_content_hash TEXT,
    available_evidence_json TEXT,
    available_evidence_hash TEXT,
    terminal_decision_hash TEXT,
    error_detail_json TEXT,
    error_detail_hash TEXT,
    error_fingerprint TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (available_evidence_json IS NOT NULL AND available_evidence_hash IS NOT NULL
            AND json_valid(available_evidence_json))
    ),
    CHECK (
        (error_detail_json IS NULL AND error_detail_hash IS NULL)
        OR
        (error_detail_json IS NOT NULL AND error_detail_hash IS NOT NULL
            AND json_valid(error_detail_json))
    ),
    CHECK (
        (result_code='completed' AND failed_stage IS NULL AND retryable IS NULL
            AND provider IS NOT NULL AND source IS NOT NULL
            AND observed_at IS NOT NULL AND batch_id IS NOT NULL
            AND batch_content_hash IS NOT NULL AND available_evidence_json IS NOT NULL
            AND terminal_decision_hash IS NOT NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL)
        OR
        (result_code='error' AND length(failed_stage)>0 AND retryable IS NOT NULL
            AND terminal_decision_hash IS NULL
            AND error_detail_json IS NOT NULL AND error_fingerprint IS NOT NULL)
    ),
    FOREIGN KEY(generation_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_fact_key)
        REFERENCES selection_source_facts_v2(source_fact_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(generation_run_id, sample_key)
);

CREATE TABLE IF NOT EXISTS selection_samples (
    sample_key TEXT PRIMARY KEY NOT NULL,
    generation_run_id TEXT NOT NULL,
    source_fact_key TEXT NOT NULL,
    source_fact_content_hash TEXT NOT NULL,
    source_fact_attempt_id TEXT NOT NULL,
    source_batch_attempt_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    chain_id TEXT NOT NULL,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    matched_keyword TEXT NOT NULL,
    canonical_stock_code TEXT NOT NULL,
    canonical_stock_name TEXT NOT NULL,
    canonical_market TEXT NOT NULL,
    relation_schema_version TEXT NOT NULL,
    relation_evidence_json TEXT NOT NULL CHECK (json_valid(relation_evidence_json)),
    relation_evidence_set_hash TEXT NOT NULL,
    feature_version TEXT NOT NULL,
    t0_feature_json TEXT NOT NULL CHECK (json_valid(t0_feature_json)),
    t0_feature_hash TEXT NOT NULL,
    market_provider TEXT NOT NULL,
    market_source TEXT NOT NULL,
    market_source_at TEXT,
    market_observed_at TEXT NOT NULL,
    market_batch_id TEXT NOT NULL,
    market_batch_content_hash TEXT NOT NULL,
    admission_version TEXT NOT NULL,
    decision_kind TEXT NOT NULL CHECK (decision_kind IN ('admitted','hard_rejected')),
    rejection_count INTEGER NOT NULL CHECK (
        (decision_kind='admitted' AND rejection_count=0)
        OR (decision_kind='hard_rejected' AND rejection_count>0)
    ),
    rejection_row_hashes_in_ordinal_order TEXT NOT NULL CHECK (
        json_valid(rejection_row_hashes_in_ordinal_order)
        AND json_type(rejection_row_hashes_in_ordinal_order)='array'
    ),
    evaluation_market_date TEXT NOT NULL,
    t0_due_date TEXT NOT NULL,
    d1_due_date TEXT NOT NULL,
    d3_due_date TEXT NOT NULL,
    d5_due_date TEXT NOT NULL,
    calendar_version TEXT NOT NULL,
    calendar_hash TEXT NOT NULL,
    staged_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        evaluation_market_date=t0_due_date
        AND t0_due_date<d1_due_date
        AND d1_due_date<d3_due_date
        AND d3_due_date<d5_due_date
    ),
    CHECK (
        (decision_kind='admitted'
            AND json_array_length(rejection_row_hashes_in_ordinal_order)=0)
        OR
        (decision_kind='hard_rejected'
            AND json_array_length(rejection_row_hashes_in_ordinal_order)=rejection_count)
    ),
    FOREIGN KEY(generation_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_fact_key)
        REFERENCES selection_source_facts_v2(source_fact_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_fact_attempt_id)
        REFERENCES selection_source_fact_attempts(source_fact_attempt_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_batch_attempt_id)
        REFERENCES selection_source_batch_attempts(source_batch_attempt_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(source_fact_key, chain_id, canonical_stock_code, config_hash)
);

CREATE TABLE IF NOT EXISTS selection_rejections (
    sample_key TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    generation_run_id TEXT NOT NULL,
    reason_code TEXT NOT NULL CHECK (
        reason_code IN (
            'moving_average_nonpositive','trend_alignment_failed','price_below_ma5',
            'price_ma20_distance_out_of_range','five_day_return_out_of_range',
            'settled_volume_confirmation_failed','intraday_volume_confirmation_failed'
        )
    ),
    rule_id TEXT NOT NULL,
    retryable INTEGER NOT NULL CHECK (retryable=0),
    structured_detail_json TEXT NOT NULL CHECK (json_valid(structured_detail_json)),
    structured_detail_hash TEXT NOT NULL,
    provider TEXT,
    source TEXT,
    source_at TEXT,
    observed_at TEXT,
    batch_id TEXT,
    batch_content_hash TEXT,
    created_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (json_extract(structured_detail_json, '$.kind')=reason_code),
    PRIMARY KEY(sample_key, ordinal),
    FOREIGN KEY(sample_key) REFERENCES selection_samples(sample_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(generation_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS selection_sample_outcomes (
    sample_key TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (
        phase IN ('t0_close','d1_settled','d3_settled','d5_settled')
    ),
    outcome_run_id TEXT NOT NULL,
    due_trading_date TEXT NOT NULL,
    open TEXT NOT NULL CHECK (CAST(open AS REAL)>0),
    high TEXT NOT NULL CHECK (CAST(high AS REAL)>0),
    low TEXT NOT NULL CHECK (CAST(low AS REAL)>0),
    close TEXT NOT NULL CHECK (CAST(close AS REAL)>0),
    volume TEXT NOT NULL CHECK (CAST(volume AS REAL)>0),
    amount TEXT NOT NULL CHECK (CAST(amount AS REAL)>=0),
    return_from_t0_close TEXT NOT NULL,
    cumulative_mfe TEXT NOT NULL,
    cumulative_mae TEXT NOT NULL,
    volume_ratio TEXT NOT NULL CHECK (CAST(volume_ratio AS REAL)>0),
    provider TEXT NOT NULL,
    source TEXT NOT NULL,
    source_at TEXT,
    observed_at TEXT NOT NULL,
    batch_id TEXT NOT NULL,
    batch_content_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (CAST(high AS REAL)>=CAST(open AS REAL)),
    CHECK (CAST(high AS REAL)>=CAST(close AS REAL)),
    CHECK (CAST(low AS REAL)<=CAST(open AS REAL)),
    CHECK (CAST(low AS REAL)<=CAST(close AS REAL)),
    CHECK (
        phase<>'t0_close'
        OR (
            CAST(return_from_t0_close AS REAL)=0
            AND CAST(cumulative_mfe AS REAL)=0
            AND CAST(cumulative_mae AS REAL)=0
            AND CAST(volume_ratio AS REAL)=1
        )
    ),
    PRIMARY KEY(sample_key, phase),
    FOREIGN KEY(sample_key) REFERENCES selection_samples(sample_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(outcome_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS selection_outcome_attempts (
    outcome_attempt_id TEXT PRIMARY KEY NOT NULL,
    sample_key TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (
        phase IN ('t0_close','d1_settled','d3_settled','d5_settled')
    ),
    stored_due_date TEXT NOT NULL,
    outcome_run_id TEXT NOT NULL,
    request_hash TEXT,
    request_evidence_json TEXT,
    request_evidence_hash TEXT,
    result_code TEXT NOT NULL CHECK (result_code IN ('settled','expected_wait','error')),
    reason_code TEXT,
    retryable INTEGER CHECK (retryable IN (0,1)),
    provider TEXT,
    source TEXT,
    source_at TEXT,
    observed_at TEXT,
    batch_id TEXT,
    batch_content_hash TEXT,
    available_evidence_json TEXT,
    available_evidence_hash TEXT,
    error_detail_json TEXT,
    error_detail_hash TEXT,
    error_fingerprint TEXT,
    settled_outcome_content_hash TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (request_hash IS NULL
            AND request_evidence_json IS NULL AND request_evidence_hash IS NULL)
        OR
        (request_hash IS NOT NULL
            AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND json_valid(request_evidence_json))
    ),
    CHECK (
        (available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (available_evidence_json IS NOT NULL AND available_evidence_hash IS NOT NULL
            AND json_valid(available_evidence_json))
    ),
    CHECK (
        (error_detail_json IS NULL AND error_detail_hash IS NULL)
        OR
        (error_detail_json IS NOT NULL AND error_detail_hash IS NOT NULL
            AND json_valid(error_detail_json))
    ),
    CHECK (
        (result_code='settled' AND reason_code IS NULL AND retryable IS NULL
            AND request_hash IS NOT NULL AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND provider IS NOT NULL AND source IS NOT NULL
            AND observed_at IS NOT NULL AND batch_id IS NOT NULL
            AND batch_content_hash IS NOT NULL
            AND available_evidence_json IS NOT NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL
            AND settled_outcome_content_hash IS NOT NULL)
        OR
        (result_code='expected_wait' AND reason_code='market_session_unsettled'
            AND request_hash IS NULL AND request_evidence_json IS NULL
            AND request_evidence_hash IS NULL
            AND retryable IS NULL AND provider IS NULL
            AND source IS NULL AND source_at IS NULL AND observed_at IS NULL
            AND batch_id IS NULL AND batch_content_hash IS NULL
            AND available_evidence_json IS NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL
            AND settled_outcome_content_hash IS NULL)
        OR
        (result_code='error' AND reason_code IS NOT NULL
            AND length(reason_code)>0 AND retryable IS NOT NULL
            AND request_hash IS NOT NULL AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND error_detail_json IS NOT NULL AND error_fingerprint IS NOT NULL
            AND settled_outcome_content_hash IS NULL)
    ),
    FOREIGN KEY(sample_key) REFERENCES selection_samples(sample_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(outcome_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(outcome_run_id, sample_key, phase)
);

CREATE TABLE IF NOT EXISTS selection_v2_run_stages (
    subject_kind TEXT NOT NULL CHECK (
        subject_kind IN ('config_activation','ingress_run','generation_run','outcome_run')
    ),
    subject_id TEXT PRIMARY KEY NOT NULL,
    in_memory_payload_hash TEXT NOT NULL,
    prepared_record_hash TEXT NOT NULL,
    expected_staged_row_count INTEGER NOT NULL CHECK (expected_staged_row_count >= 1),
    staged_db_content_hash TEXT NOT NULL,
    recovery_envelope_content_hash TEXT NOT NULL,
    logical_subject_key TEXT NOT NULL,
    run_status TEXT NOT NULL,
    source_fact_key TEXT,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    config_snapshot_json_hash TEXT,
    config_activation_content_hash TEXT,
    config_activation_file_content_hash TEXT,
    config_effective_from TEXT,
    artifact_valid_from TEXT,
    artifact_expires_at TEXT,
    executable_revision TEXT,
    legacy_cutover_snapshot_hash TEXT,
    generation_market_date TEXT,
    aggregator_observed_at TEXT,
    ingress_source_batch_content_hash TEXT,
    outcome_phase TEXT,
    stored_due_date TEXT,
    staged_at TEXT NOT NULL,
    manifest_content_hash TEXT NOT NULL,
    CHECK (
        (subject_kind='config_activation' AND run_status='activated')
        OR
        (subject_kind='ingress_run'
            AND run_status IN ('completed','failed_non_retryable'))
        OR
        (subject_kind='generation_run'
            AND run_status IN (
                'completed','verified_no_relation','pending_dependency',
                'failed_non_retryable'
            ))
        OR
        (subject_kind='outcome_run'
            AND run_status IN (
                'settled','expected_wait','failed_retryable','failed_non_retryable'
            ))
    ),
    CHECK (
        (subject_kind='config_activation'
            AND config_activation_run_id=subject_id
            AND source_fact_key IS NULL
            AND config_snapshot_json_hash IS NOT NULL
            AND config_activation_content_hash IS NOT NULL
            AND config_activation_file_content_hash IS NOT NULL
            AND config_effective_from IS NOT NULL
            AND artifact_valid_from IS NOT NULL AND artifact_expires_at IS NOT NULL
            AND executable_revision IS NOT NULL
            AND legacy_cutover_snapshot_hash IS NOT NULL
            AND generation_market_date IS NULL
            AND aggregator_observed_at IS NULL
            AND ingress_source_batch_content_hash IS NULL
            AND outcome_phase IS NULL AND stored_due_date IS NULL)
        OR
        (subject_kind='ingress_run'
            AND source_fact_key IS NULL
            AND config_snapshot_json_hash IS NULL
            AND config_activation_content_hash IS NULL
            AND config_activation_file_content_hash IS NULL
            AND config_effective_from IS NULL
            AND artifact_valid_from IS NULL AND artifact_expires_at IS NULL
            AND executable_revision IS NULL AND legacy_cutover_snapshot_hash IS NULL
            AND generation_market_date IS NOT NULL
            AND aggregator_observed_at IS NOT NULL
            AND ingress_source_batch_content_hash IS NOT NULL
            AND outcome_phase IS NULL AND stored_due_date IS NULL)
        OR
        (subject_kind='generation_run'
            AND source_fact_key IS NOT NULL
            AND config_snapshot_json_hash IS NULL
            AND config_activation_content_hash IS NULL
            AND config_activation_file_content_hash IS NULL
            AND config_effective_from IS NULL
            AND artifact_valid_from IS NULL AND artifact_expires_at IS NULL
            AND executable_revision IS NULL AND legacy_cutover_snapshot_hash IS NULL
            AND generation_market_date IS NOT NULL
            AND aggregator_observed_at IS NULL
            AND ingress_source_batch_content_hash IS NULL
            AND outcome_phase IS NULL AND stored_due_date IS NULL)
        OR
        (subject_kind='outcome_run'
            AND source_fact_key IS NULL
            AND config_snapshot_json_hash IS NULL
            AND config_activation_content_hash IS NULL
            AND config_activation_file_content_hash IS NULL
            AND config_effective_from IS NULL
            AND artifact_valid_from IS NULL AND artifact_expires_at IS NULL
            AND executable_revision IS NULL AND legacy_cutover_snapshot_hash IS NULL
            AND generation_market_date IS NULL
            AND aggregator_observed_at IS NULL
            AND ingress_source_batch_content_hash IS NULL
            AND outcome_phase IN ('t0_close','d1_settled','d3_settled','d5_settled')
            AND stored_due_date IS NOT NULL)
    ),
    FOREIGN KEY(subject_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(subject_kind, subject_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS selection_v2_one_activation_per_config
ON selection_v2_run_stages(config_hash)
WHERE subject_kind='config_activation' AND run_status='activated';

CREATE TABLE IF NOT EXISTS selection_v2_commit_receipts (
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    logical_subject_key TEXT NOT NULL,
    in_memory_payload_hash TEXT NOT NULL,
    recovery_envelope_content_hash TEXT NOT NULL,
    prepared_audit_hash TEXT NOT NULL,
    run_manifest_content_hash TEXT NOT NULL,
    staged_db_content_hash TEXT NOT NULL,
    committed_audit_hash TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    PRIMARY KEY(subject_kind, subject_id),
    FOREIGN KEY(subject_kind, subject_id)
        REFERENCES selection_v2_run_stages(subject_kind, subject_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(subject_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX IF NOT EXISTS selection_v2_source_facts_pending
ON selection_source_facts_v2(ingress_decision, first_ingress_run_id, source_fact_key);
CREATE INDEX IF NOT EXISTS selection_v2_samples_generation
ON selection_samples(generation_run_id, sample_key);
CREATE INDEX IF NOT EXISTS selection_v2_outcome_attempt_run
ON selection_outcome_attempts(outcome_run_id, sample_key, phase);
CREATE INDEX IF NOT EXISTS selection_v2_receipt_subject
ON selection_v2_commit_receipts(subject_id, subject_kind);

CREATE TRIGGER IF NOT EXISTS selection_v2_batch_lineage
BEFORE INSERT ON selection_source_batch_attempts
BEGIN
    SELECT RAISE(ABORT, 'BR-174 batch/envelope config lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_recovery_envelopes e
        WHERE e.stage_run_id=NEW.ingress_run_id
          AND e.subject_kind='ingress_run'
          AND e.config_activation_run_id=NEW.config_activation_run_id
          AND e.config_hash=NEW.config_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_fact_lineage
BEFORE INSERT ON selection_source_facts_v2
BEGIN
    SELECT RAISE(ABORT, 'BR-174 fact/envelope config lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_recovery_envelopes e
        WHERE e.stage_run_id=NEW.first_ingress_run_id
          AND e.subject_kind='ingress_run'
          AND e.config_activation_run_id=NEW.config_activation_run_id
          AND e.config_hash=NEW.config_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_fact_attempt_lineage
BEFORE INSERT ON selection_source_fact_attempts
BEGIN
    SELECT RAISE(ABORT, 'BR-174 fact attempt lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_source_batch_attempts b
        JOIN selection_source_facts_v2 f
          ON f.source_fact_key=NEW.source_fact_key
        WHERE b.source_batch_attempt_id=NEW.source_batch_attempt_id
          AND b.ingress_run_id=NEW.ingress_run_id
          AND b.batch_content_hash=f.record_batch_content_hash
          AND b.provider=f.record_provider
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_relation_requires_admitted_source
BEFORE INSERT ON selection_relation_attempts
BEGIN
    SELECT RAISE(ABORT, 'BR-174 generation requires matching ingress-admitted source fact')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_source_facts_v2 f
        JOIN selection_v2_recovery_envelopes e
          ON e.stage_run_id=NEW.generation_run_id
        WHERE f.source_fact_key=NEW.source_fact_key
          AND f.ingress_decision='admitted'
          AND f.event_id=NEW.event_id
          AND f.config_activation_run_id=NEW.config_activation_run_id
          AND f.config_hash=NEW.config_hash
          AND e.subject_kind='generation_run'
          AND e.config_activation_run_id=f.config_activation_run_id
          AND e.config_hash=f.config_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_evaluation_requires_admitted_source
BEFORE INSERT ON selection_evaluation_attempts
BEGIN
    SELECT RAISE(ABORT, 'BR-174 evaluation/source lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_source_facts_v2 f
        JOIN selection_v2_recovery_envelopes e
          ON e.stage_run_id=NEW.generation_run_id
        WHERE f.source_fact_key=NEW.source_fact_key
          AND f.ingress_decision='admitted'
          AND f.event_id=NEW.event_id
          AND e.subject_kind='generation_run'
          AND e.config_activation_run_id=f.config_activation_run_id
          AND e.config_hash=f.config_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_sample_requires_admitted_source
BEFORE INSERT ON selection_samples
BEGIN
    SELECT RAISE(ABORT, 'BR-174 terminal sample lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_source_facts_v2 f
        JOIN selection_source_fact_attempts fa
          ON fa.source_fact_attempt_id=NEW.source_fact_attempt_id
        JOIN selection_source_batch_attempts b
          ON b.source_batch_attempt_id=NEW.source_batch_attempt_id
        JOIN selection_evaluation_attempts ev
          ON ev.generation_run_id=NEW.generation_run_id
         AND ev.sample_key=NEW.sample_key
        JOIN selection_v2_recovery_envelopes e
          ON e.stage_run_id=NEW.generation_run_id
        WHERE f.source_fact_key=NEW.source_fact_key
          AND f.ingress_decision='admitted'
          AND f.content_hash=NEW.source_fact_content_hash
          AND fa.source_fact_key=f.source_fact_key
          AND fa.ingress_run_id=f.first_ingress_run_id
          AND fa.source_batch_attempt_id=b.source_batch_attempt_id
          AND b.ingress_run_id=f.first_ingress_run_id
          AND f.event_id=NEW.event_id
          AND f.config_activation_run_id=NEW.config_activation_run_id
          AND f.config_hash=NEW.config_hash
          AND ev.source_fact_key=f.source_fact_key
          AND ev.event_id=NEW.event_id
          AND ev.chain_id=NEW.chain_id
          AND ev.canonical_stock_code=NEW.canonical_stock_code
          AND ev.relation_evidence_set_hash=NEW.relation_evidence_set_hash
          AND ev.result_code='completed'
          AND ev.terminal_decision_hash=NEW.content_hash
          AND e.config_activation_run_id=NEW.config_activation_run_id
          AND e.config_hash=NEW.config_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_rejection_requires_admitted_source
BEFORE INSERT ON selection_rejections
BEGIN
    SELECT RAISE(ABORT, 'BR-174 rejection parent/matrix mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_samples s
        JOIN selection_source_facts_v2 f ON f.source_fact_key=s.source_fact_key
        WHERE s.sample_key=NEW.sample_key
          AND s.generation_run_id=NEW.generation_run_id
          AND s.decision_kind='hard_rejected'
          AND NEW.ordinal < s.rejection_count
          AND NEW.ordinal=(
              SELECT COUNT(*) FROM selection_rejections r
              WHERE r.sample_key=NEW.sample_key
          )
          AND json_extract(
              s.rejection_row_hashes_in_ordinal_order,
              '$[' || NEW.ordinal || ']'
          )=NEW.content_hash
          AND f.ingress_decision='admitted'
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_manifest_envelope_binding
BEFORE INSERT ON selection_v2_run_stages
BEGIN
    SELECT RAISE(ABORT, 'BR-174 manifest/envelope binding mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_recovery_envelopes e
        WHERE e.stage_run_id=NEW.subject_id
          AND e.subject_kind=NEW.subject_kind
          AND e.logical_subject_key=NEW.logical_subject_key
          AND e.in_memory_payload_hash=NEW.in_memory_payload_hash
          AND e.config_activation_run_id=NEW.config_activation_run_id
          AND e.config_hash=NEW.config_hash
          AND e.content_hash=NEW.recovery_envelope_content_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_config_manifest_closure
BEFORE INSERT ON selection_v2_run_stages
WHEN NEW.subject_kind='config_activation'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 config activation manifest closure mismatch')
    WHERE NEW.expected_staged_row_count<>1
       OR NEW.config_activation_run_id<>NEW.subject_id;
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_ingress_manifest_closure
BEFORE INSERT ON selection_v2_run_stages
WHEN NEW.subject_kind='ingress_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 ingress requires receipted config activation')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_commit_receipts r
        WHERE r.subject_kind='config_activation'
          AND r.subject_id=NEW.config_activation_run_id
    );
    SELECT RAISE(ABORT, 'BR-174 ingress domain config mismatch')
    WHERE EXISTS (
        SELECT 1 FROM selection_source_batch_attempts b
        WHERE b.ingress_run_id=NEW.subject_id
          AND (
              b.config_activation_run_id<>NEW.config_activation_run_id
              OR b.config_hash<>NEW.config_hash
              OR b.generation_market_date<>NEW.generation_market_date
          )
    ) OR EXISTS (
        SELECT 1 FROM selection_source_facts_v2 f
        WHERE f.first_ingress_run_id=NEW.subject_id
          AND (
              f.config_activation_run_id<>NEW.config_activation_run_id
              OR f.config_hash<>NEW.config_hash
              OR f.generation_market_date<>NEW.generation_market_date
          )
    );
    SELECT RAISE(ABORT, 'BR-174 ingress feed/fact no-loss matrix mismatch')
    WHERE (SELECT COUNT(*) FROM selection_source_batch_attempts b
           WHERE b.ingress_run_id=NEW.subject_id)<>4
       OR (SELECT COUNT(DISTINCT registered_feed_snapshot_hash)
           FROM selection_source_batch_attempts b
           WHERE b.ingress_run_id=NEW.subject_id)<>1
       OR EXISTS (
        SELECT 1
        FROM selection_source_batch_attempts b
        WHERE b.ingress_run_id=NEW.subject_id
          AND (
            (b.status_kind='available' AND b.record_count<>(
                SELECT COUNT(*) FROM selection_source_fact_attempts fa
                WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
            ))
            OR (b.status_kind='available' AND (
                (SELECT MIN(provider_ordinal) FROM selection_source_fact_attempts fa
                 WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id)<>0
                OR (SELECT MAX(provider_ordinal) FROM selection_source_fact_attempts fa
                    WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id)
                   <>b.record_count-1
                OR EXISTS (
                    SELECT 1 FROM selection_source_fact_attempts fa
                    WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
                      AND fa.batch_evidence_hash<>b.available_evidence_hash
                )
            ))
            OR (b.status_kind<>'available' AND EXISTS (
                SELECT 1 FROM selection_source_fact_attempts fa
                WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
            ))
          )
    ) OR EXISTS (
        SELECT 1 FROM selection_source_fact_attempts fa
        LEFT JOIN selection_source_batch_attempts b
          ON b.source_batch_attempt_id=fa.source_batch_attempt_id
         AND b.ingress_run_id=fa.ingress_run_id
        LEFT JOIN selection_source_facts_v2 f
          ON f.source_fact_key=fa.source_fact_key
        WHERE fa.ingress_run_id=NEW.subject_id
          AND (b.source_batch_attempt_id IS NULL OR f.source_fact_key IS NULL)
    ) OR EXISTS (
        SELECT 1 FROM selection_source_facts_v2 f
        WHERE f.first_ingress_run_id=NEW.subject_id
          AND NOT EXISTS (
              SELECT 1 FROM selection_source_fact_attempts fa
              WHERE fa.ingress_run_id=NEW.subject_id
                AND fa.source_fact_key=f.source_fact_key
          )
    );
    SELECT RAISE(ABORT, 'BR-174 ingress staged row count mismatch')
    WHERE NEW.expected_staged_row_count <> (
        1
        + (SELECT COUNT(*) FROM selection_source_batch_attempts b
           WHERE b.ingress_run_id=NEW.subject_id)
        + (SELECT COUNT(*) FROM selection_source_facts_v2 f
           WHERE f.first_ingress_run_id=NEW.subject_id)
        + (SELECT COUNT(*) FROM selection_source_fact_attempts fa
           WHERE fa.ingress_run_id=NEW.subject_id)
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_generation_manifest_closure
BEFORE INSERT ON selection_v2_run_stages
WHEN NEW.subject_kind='generation_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 generation requires receipted activation and ingress')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_source_facts_v2 f
        JOIN selection_v2_commit_receipts ar
          ON ar.subject_kind='config_activation'
         AND ar.subject_id=f.config_activation_run_id
        JOIN selection_v2_commit_receipts ir
          ON ir.subject_kind='ingress_run'
         AND ir.subject_id=f.first_ingress_run_id
        WHERE f.source_fact_key=NEW.source_fact_key
          AND f.ingress_decision='admitted'
          AND f.config_activation_run_id=NEW.config_activation_run_id
          AND f.config_hash=NEW.config_hash
          AND f.generation_market_date=NEW.generation_market_date
    );
    SELECT RAISE(ABORT, 'BR-174 generation domain lineage mismatch')
    WHERE EXISTS (
        SELECT source_fact_key FROM selection_relation_attempts
        WHERE generation_run_id=NEW.subject_id AND source_fact_key<>NEW.source_fact_key
        UNION ALL
        SELECT source_fact_key FROM selection_evaluation_attempts
        WHERE generation_run_id=NEW.subject_id AND source_fact_key<>NEW.source_fact_key
        UNION ALL
        SELECT source_fact_key FROM selection_samples
        WHERE generation_run_id=NEW.subject_id AND source_fact_key<>NEW.source_fact_key
    );
    SELECT RAISE(ABORT, 'BR-174 generation terminal/rejection matrix mismatch')
    WHERE EXISTS (
        SELECT 1
        FROM selection_samples s
        WHERE s.generation_run_id=NEW.subject_id
          AND (
            (s.decision_kind='admitted' AND EXISTS (
                SELECT 1 FROM selection_rejections r WHERE r.sample_key=s.sample_key
            ))
            OR
            (s.decision_kind='hard_rejected' AND (
                (SELECT COUNT(*) FROM selection_rejections r
                 WHERE r.sample_key=s.sample_key)<>s.rejection_count
                OR (SELECT MIN(ordinal) FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key)<>0
                OR (SELECT MAX(ordinal) FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key)<>s.rejection_count-1
                OR EXISTS (
                    SELECT 1 FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key
                      AND json_extract(
                          s.rejection_row_hashes_in_ordinal_order,
                          '$[' || r.ordinal || ']'
                      )<>r.content_hash
                )
            ))
          )
    ) OR EXISTS (
        SELECT 1
        FROM selection_evaluation_attempts e
        WHERE e.generation_run_id=NEW.subject_id
          AND (
            (e.result_code='completed' AND NOT EXISTS (
                SELECT 1 FROM selection_samples s
                WHERE s.generation_run_id=e.generation_run_id
                  AND s.sample_key=e.sample_key
                  AND s.source_fact_key=e.source_fact_key
                  AND s.content_hash=e.terminal_decision_hash
                  AND s.relation_evidence_set_hash=e.relation_evidence_set_hash
            ))
            OR
            (e.result_code='error' AND EXISTS (
                SELECT 1 FROM selection_samples s
                WHERE s.generation_run_id=e.generation_run_id
                  AND s.sample_key=e.sample_key
            ))
          )
    );
    SELECT RAISE(ABORT, 'BR-174 generation status matrix mismatch')
    WHERE (NEW.run_status='verified_no_relation' AND (
              EXISTS (SELECT 1 FROM selection_relation_attempts
                      WHERE generation_run_id=NEW.subject_id)
              OR EXISTS (SELECT 1 FROM selection_evaluation_attempts
                         WHERE generation_run_id=NEW.subject_id)
              OR EXISTS (SELECT 1 FROM selection_samples
                         WHERE generation_run_id=NEW.subject_id)
          ))
       OR (NEW.run_status='completed' AND (
              NOT EXISTS (
                  SELECT 1 FROM selection_relation_attempts r
                  WHERE r.generation_run_id=NEW.subject_id
              )
              OR EXISTS (
                  SELECT 1 FROM selection_relation_attempts r
                  WHERE r.generation_run_id=NEW.subject_id
                    AND r.result_code<>'resolved'
                    AND r.retryable<>0
              )
              OR EXISTS (
                  SELECT 1 FROM selection_evaluation_attempts e
                  WHERE e.generation_run_id=NEW.subject_id
                    AND e.result_code<>'completed'
              )
          ));
    SELECT RAISE(ABORT, 'BR-174 generation dependency status mismatch')
    WHERE (NEW.run_status='pending_dependency' AND NOT EXISTS (
              SELECT 1 FROM selection_relation_attempts r
              WHERE r.generation_run_id=NEW.subject_id AND r.retryable=1
              UNION ALL
              SELECT 1 FROM selection_evaluation_attempts e
              WHERE e.generation_run_id=NEW.subject_id AND e.retryable=1
          ))
       OR (NEW.run_status='failed_non_retryable' AND (
              EXISTS (
                  SELECT 1 FROM selection_relation_attempts r
                  WHERE r.generation_run_id=NEW.subject_id AND r.retryable=1
                  UNION ALL
                  SELECT 1 FROM selection_evaluation_attempts e
                  WHERE e.generation_run_id=NEW.subject_id AND e.retryable=1
              )
              OR NOT EXISTS (
                  SELECT 1 FROM selection_relation_attempts r
                  WHERE r.generation_run_id=NEW.subject_id
                    AND r.result_code<>'resolved' AND r.retryable=0
                  UNION ALL
                  SELECT 1 FROM selection_evaluation_attempts e
                  WHERE e.generation_run_id=NEW.subject_id
                    AND e.result_code='error' AND e.retryable=0
              )
          ));
    SELECT RAISE(ABORT, 'BR-174 generation staged row count mismatch')
    WHERE NEW.expected_staged_row_count <> (
        1
        + (SELECT COUNT(*) FROM selection_relation_attempts r
           WHERE r.generation_run_id=NEW.subject_id)
        + (SELECT COUNT(*) FROM selection_evaluation_attempts e
           WHERE e.generation_run_id=NEW.subject_id)
        + (SELECT COUNT(*) FROM selection_samples s
           WHERE s.generation_run_id=NEW.subject_id)
        + (SELECT COUNT(*) FROM selection_rejections x
           WHERE x.generation_run_id=NEW.subject_id)
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_outcome_manifest_closure
BEFORE INSERT ON selection_v2_run_stages
WHEN NEW.subject_kind='outcome_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-178 outcome manifest must be inserted last')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_recovery_envelopes e
        WHERE e.stage_run_id=NEW.subject_id AND e.subject_kind='outcome_run'
    );
    SELECT RAISE(ABORT, 'BR-178 outcome upstream receipt lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_outcome_attempts a
        JOIN selection_samples s ON s.sample_key=a.sample_key
        JOIN selection_source_facts_v2 f ON f.source_fact_key=s.source_fact_key
        JOIN selection_v2_commit_receipts ar
          ON ar.subject_kind='config_activation'
         AND ar.subject_id=s.config_activation_run_id
        JOIN selection_v2_commit_receipts ir
          ON ir.subject_kind='ingress_run'
         AND ir.subject_id=f.first_ingress_run_id
        JOIN selection_v2_commit_receipts gr
          ON gr.subject_kind='generation_run'
         AND gr.subject_id=s.generation_run_id
        WHERE a.outcome_run_id=NEW.subject_id
          AND s.decision_kind IN ('admitted','hard_rejected')
          AND s.config_activation_run_id=NEW.config_activation_run_id
          AND s.config_hash=NEW.config_hash
          AND a.stored_due_date=CASE a.phase
              WHEN 't0_close' THEN s.t0_due_date
              WHEN 'd1_settled' THEN s.d1_due_date
              WHEN 'd3_settled' THEN s.d3_due_date
              WHEN 'd5_settled' THEN s.d5_due_date
          END
    );
    SELECT RAISE(ABORT, 'BR-178 required preceding settled phase receipt missing')
    WHERE (
        SELECT COUNT(DISTINCT pa.phase)
        FROM selection_outcome_attempts a
        JOIN selection_outcome_attempts pa ON pa.sample_key=a.sample_key
        JOIN selection_v2_run_stages pm
          ON pm.subject_kind='outcome_run'
         AND pm.subject_id=pa.outcome_run_id
         AND pm.run_status='settled'
        JOIN selection_v2_commit_receipts pr
          ON pr.subject_kind='outcome_run'
         AND pr.subject_id=pa.outcome_run_id
        WHERE a.outcome_run_id=NEW.subject_id
          AND pa.result_code='settled'
          AND (
              (a.phase='d1_settled' AND pa.phase='t0_close')
              OR (a.phase='d3_settled' AND pa.phase IN ('t0_close','d1_settled'))
              OR (a.phase='d5_settled'
                  AND pa.phase IN ('t0_close','d1_settled','d3_settled'))
          )
    ) <> CASE NEW.outcome_phase
        WHEN 't0_close' THEN 0
        WHEN 'd1_settled' THEN 1
        WHEN 'd3_settled' THEN 2
        WHEN 'd5_settled' THEN 3
    END;
    SELECT RAISE(ABORT, 'BR-178 outcome requires exactly one attempt')
    WHERE (SELECT COUNT(*) FROM selection_outcome_attempts a
           WHERE a.outcome_run_id=NEW.subject_id) <> 1;
    SELECT RAISE(ABORT, 'BR-178 outcome attempt identity mismatch')
    WHERE EXISTS (
        SELECT 1 FROM selection_outcome_attempts a
        WHERE a.outcome_run_id=NEW.subject_id
          AND (a.phase<>NEW.outcome_phase OR a.stored_due_date<>NEW.stored_due_date)
    );
    SELECT RAISE(ABORT, 'BR-178 outcome status/result mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_outcome_attempts a
        WHERE a.outcome_run_id=NEW.subject_id
          AND (
            (NEW.run_status='settled' AND a.result_code='settled')
            OR (NEW.run_status='expected_wait' AND a.result_code='expected_wait')
            OR (NEW.run_status='failed_retryable'
                AND a.result_code='error' AND a.retryable=1)
            OR (NEW.run_status='failed_non_retryable'
                AND a.result_code='error' AND a.retryable=0)
          )
    );
    SELECT RAISE(ABORT, 'BR-178 outcome cardinality mismatch')
    WHERE (
        (NEW.run_status='settled'
         AND (SELECT COUNT(*) FROM selection_sample_outcomes o
              WHERE o.outcome_run_id=NEW.subject_id) <> 1)
        OR
        (NEW.run_status<>'settled'
         AND (SELECT COUNT(*) FROM selection_sample_outcomes o
              WHERE o.outcome_run_id=NEW.subject_id) <> 0)
    );
    SELECT RAISE(ABORT, 'BR-178 outcome identity mismatch')
    WHERE EXISTS (
        SELECT 1
        FROM selection_sample_outcomes o
        JOIN selection_outcome_attempts a ON a.outcome_run_id=o.outcome_run_id
        WHERE o.outcome_run_id=NEW.subject_id
          AND (o.sample_key<>a.sample_key OR o.phase<>a.phase
               OR o.phase<>NEW.outcome_phase
               OR o.due_trading_date<>NEW.stored_due_date)
    );
    SELECT RAISE(ABORT, 'BR-178 settled outcome hash mismatch')
    WHERE NEW.run_status='settled' AND NOT EXISTS (
        SELECT 1
        FROM selection_outcome_attempts a
        JOIN selection_sample_outcomes o
          ON o.outcome_run_id=a.outcome_run_id
         AND o.sample_key=a.sample_key AND o.phase=a.phase
        WHERE a.outcome_run_id=NEW.subject_id
          AND a.settled_outcome_content_hash=o.content_hash
    );
    SELECT RAISE(ABORT, 'BR-178 staged row count mismatch')
    WHERE NEW.expected_staged_row_count
          <> CASE WHEN NEW.run_status='settled' THEN 3 ELSE 2 END
       OR NEW.expected_staged_row_count <> (
            1
            + (SELECT COUNT(*) FROM selection_outcome_attempts a
               WHERE a.outcome_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_sample_outcomes o
               WHERE o.outcome_run_id=NEW.subject_id)
       );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_receipt_manifest_binding
BEFORE INSERT ON selection_v2_commit_receipts
BEGIN
    SELECT RAISE(ABORT, 'BR-174 receipt/manifest/envelope binding mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_v2_recovery_envelopes e ON e.stage_run_id=m.subject_id
        WHERE m.subject_kind=NEW.subject_kind
          AND m.subject_id=NEW.subject_id
          AND m.logical_subject_key=NEW.logical_subject_key
          AND m.in_memory_payload_hash=NEW.in_memory_payload_hash
          AND m.prepared_record_hash=NEW.prepared_audit_hash
          AND m.staged_db_content_hash=NEW.staged_db_content_hash
          AND m.manifest_content_hash=NEW.run_manifest_content_hash
          AND m.recovery_envelope_content_hash=NEW.recovery_envelope_content_hash
          AND e.content_hash=NEW.recovery_envelope_content_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_config_receipt_closure
BEFORE INSERT ON selection_v2_commit_receipts
WHEN NEW.subject_kind='config_activation'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 config activation receipt closure mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_run_stages m
        WHERE m.subject_kind='config_activation'
          AND m.subject_id=NEW.subject_id
          AND m.run_status='activated'
          AND m.expected_staged_row_count=1
          AND m.config_activation_run_id=m.subject_id
          AND m.config_snapshot_json_hash IS NOT NULL
          AND m.config_activation_content_hash IS NOT NULL
          AND m.config_activation_file_content_hash IS NOT NULL
          AND m.legacy_cutover_snapshot_hash IS NOT NULL
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_ingress_receipt_closure
BEFORE INSERT ON selection_v2_commit_receipts
WHEN NEW.subject_kind='ingress_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 ingress receipt missing activation')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_v2_commit_receipts ar
          ON ar.subject_kind='config_activation'
         AND ar.subject_id=m.config_activation_run_id
        WHERE m.subject_kind='ingress_run' AND m.subject_id=NEW.subject_id
    );
    SELECT RAISE(ABORT, 'BR-174 ingress receipt no-loss/count mismatch')
    WHERE (SELECT COUNT(*) FROM selection_source_batch_attempts b
           WHERE b.ingress_run_id=NEW.subject_id)<>4
       OR (SELECT COUNT(DISTINCT registered_feed_snapshot_hash)
           FROM selection_source_batch_attempts b
           WHERE b.ingress_run_id=NEW.subject_id)<>1
       OR EXISTS (
        SELECT 1
        FROM selection_source_batch_attempts b
        JOIN selection_v2_run_stages m
          ON m.subject_kind='ingress_run' AND m.subject_id=b.ingress_run_id
        WHERE b.ingress_run_id=NEW.subject_id
          AND (
            b.config_activation_run_id<>m.config_activation_run_id
            OR b.config_hash<>m.config_hash
            OR b.generation_market_date<>m.generation_market_date
            OR (b.status_kind='available' AND b.record_count<>(
                SELECT COUNT(*) FROM selection_source_fact_attempts fa
                WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
            ))
            OR (b.status_kind='available' AND (
                (SELECT MIN(provider_ordinal) FROM selection_source_fact_attempts fa
                 WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id)<>0
                OR (SELECT MAX(provider_ordinal) FROM selection_source_fact_attempts fa
                    WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id)
                   <>b.record_count-1
                OR EXISTS (
                    SELECT 1 FROM selection_source_fact_attempts fa
                    WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
                      AND fa.batch_evidence_hash<>b.available_evidence_hash
                )
            ))
            OR (b.status_kind<>'available' AND EXISTS (
                SELECT 1 FROM selection_source_fact_attempts fa
                WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
            ))
          )
    ) OR EXISTS (
        SELECT 1
        FROM selection_source_facts_v2 f
        JOIN selection_v2_run_stages m
          ON m.subject_kind='ingress_run' AND m.subject_id=f.first_ingress_run_id
        WHERE f.first_ingress_run_id=NEW.subject_id
          AND (
            f.config_activation_run_id<>m.config_activation_run_id
            OR f.config_hash<>m.config_hash
            OR f.generation_market_date<>m.generation_market_date
            OR NOT EXISTS (
                SELECT 1 FROM selection_source_fact_attempts fa
                WHERE fa.ingress_run_id=f.first_ingress_run_id
                  AND fa.source_fact_key=f.source_fact_key
            )
          )
    ) OR NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        WHERE m.subject_kind='ingress_run' AND m.subject_id=NEW.subject_id
          AND m.expected_staged_row_count=(
            1
            + (SELECT COUNT(*) FROM selection_source_batch_attempts b
               WHERE b.ingress_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_source_facts_v2 f
               WHERE f.first_ingress_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_source_fact_attempts fa
               WHERE fa.ingress_run_id=NEW.subject_id)
          )
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_generation_receipt_closure
BEFORE INSERT ON selection_v2_commit_receipts
WHEN NEW.subject_kind='generation_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 generation receipt missing activation/source ingress')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_source_facts_v2 f
          ON f.source_fact_key=m.source_fact_key
        JOIN selection_v2_commit_receipts ar
          ON ar.subject_kind='config_activation'
         AND ar.subject_id=m.config_activation_run_id
        JOIN selection_v2_commit_receipts ir
          ON ir.subject_kind='ingress_run'
         AND ir.subject_id=f.first_ingress_run_id
        WHERE m.subject_kind='generation_run'
          AND m.subject_id=NEW.subject_id
          AND f.ingress_decision='admitted'
          AND f.config_activation_run_id=m.config_activation_run_id
          AND f.config_hash=m.config_hash
          AND f.generation_market_date=m.generation_market_date
    );
    SELECT RAISE(ABORT, 'BR-174 generation receipt source fact is not admitted')
    WHERE EXISTS (
        SELECT 1
        FROM (
            SELECT source_fact_key FROM selection_relation_attempts
            WHERE generation_run_id=NEW.subject_id
            UNION ALL
            SELECT source_fact_key FROM selection_evaluation_attempts
            WHERE generation_run_id=NEW.subject_id
            UNION ALL
            SELECT source_fact_key FROM selection_samples
            WHERE generation_run_id=NEW.subject_id
        ) x
        LEFT JOIN selection_source_facts_v2 f ON f.source_fact_key=x.source_fact_key
        WHERE f.source_fact_key IS NULL OR f.ingress_decision<>'admitted'
    );
    SELECT RAISE(ABORT, 'BR-174 generation receipt missing ingress receipt')
    WHERE EXISTS (
        SELECT 1
        FROM (
            SELECT source_fact_key FROM selection_relation_attempts
            WHERE generation_run_id=NEW.subject_id
            UNION ALL
            SELECT source_fact_key FROM selection_evaluation_attempts
            WHERE generation_run_id=NEW.subject_id
            UNION ALL
            SELECT source_fact_key FROM selection_samples
            WHERE generation_run_id=NEW.subject_id
        ) x
        JOIN selection_source_facts_v2 f ON f.source_fact_key=x.source_fact_key
        LEFT JOIN selection_v2_commit_receipts ir
          ON ir.subject_kind='ingress_run' AND ir.subject_id=f.first_ingress_run_id
        WHERE ir.subject_id IS NULL
    );
    SELECT RAISE(ABORT, 'BR-174 generation receipt terminal closure mismatch')
    WHERE EXISTS (
        SELECT 1
        FROM selection_samples s
        LEFT JOIN selection_evaluation_attempts e
          ON e.generation_run_id=s.generation_run_id
         AND e.sample_key=s.sample_key
        WHERE s.generation_run_id=NEW.subject_id
          AND (
            e.evaluation_attempt_id IS NULL
            OR e.result_code<>'completed'
            OR e.terminal_decision_hash<>s.content_hash
            OR e.source_fact_key<>s.source_fact_key
            OR e.relation_evidence_set_hash<>s.relation_evidence_set_hash
            OR (s.decision_kind='admitted' AND EXISTS (
                SELECT 1 FROM selection_rejections r WHERE r.sample_key=s.sample_key
            ))
            OR (s.decision_kind='hard_rejected' AND (
                (SELECT COUNT(*) FROM selection_rejections r
                 WHERE r.sample_key=s.sample_key)<>s.rejection_count
                OR (SELECT MIN(ordinal) FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key)<>0
                OR (SELECT MAX(ordinal) FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key)<>s.rejection_count-1
                OR EXISTS (
                    SELECT 1 FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key
                      AND json_extract(
                          s.rejection_row_hashes_in_ordinal_order,
                          '$[' || r.ordinal || ']'
                      )<>r.content_hash
                )
            ))
          )
    ) OR EXISTS (
        SELECT 1
        FROM selection_evaluation_attempts e
        WHERE e.generation_run_id=NEW.subject_id
          AND (
            (e.result_code='completed' AND NOT EXISTS (
                SELECT 1 FROM selection_samples s
                WHERE s.generation_run_id=e.generation_run_id
                  AND s.sample_key=e.sample_key
                  AND s.content_hash=e.terminal_decision_hash
                  AND s.relation_evidence_set_hash=e.relation_evidence_set_hash
            ))
            OR (e.result_code='error' AND EXISTS (
                SELECT 1 FROM selection_samples s
                WHERE s.generation_run_id=e.generation_run_id
                  AND s.sample_key=e.sample_key
            ))
          )
    ) OR NOT EXISTS (
        SELECT 1 FROM selection_v2_run_stages m
        WHERE m.subject_kind='generation_run' AND m.subject_id=NEW.subject_id
          AND m.expected_staged_row_count=(
            1
            + (SELECT COUNT(*) FROM selection_relation_attempts r
               WHERE r.generation_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_evaluation_attempts e
               WHERE e.generation_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_samples s
               WHERE s.generation_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_rejections x
               WHERE x.generation_run_id=NEW.subject_id)
          )
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_outcome_receipt_closure
BEFORE INSERT ON selection_v2_commit_receipts
WHEN NEW.subject_kind='outcome_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-178 outcome receipt upstream lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_outcome_attempts a ON a.outcome_run_id=m.subject_id
        JOIN selection_samples s ON s.sample_key=a.sample_key
        JOIN selection_source_facts_v2 f ON f.source_fact_key=s.source_fact_key
        JOIN selection_v2_commit_receipts ar
          ON ar.subject_kind='config_activation'
         AND ar.subject_id=s.config_activation_run_id
        JOIN selection_v2_commit_receipts ir
          ON ir.subject_kind='ingress_run'
         AND ir.subject_id=f.first_ingress_run_id
        JOIN selection_v2_commit_receipts gr
          ON gr.subject_kind='generation_run'
         AND gr.subject_id=s.generation_run_id
        WHERE m.subject_kind='outcome_run' AND m.subject_id=NEW.subject_id
          AND s.decision_kind IN ('admitted','hard_rejected')
          AND s.config_activation_run_id=m.config_activation_run_id
          AND s.config_hash=m.config_hash
          AND a.phase=m.outcome_phase
          AND a.stored_due_date=m.stored_due_date
          AND a.stored_due_date=CASE a.phase
              WHEN 't0_close' THEN s.t0_due_date
              WHEN 'd1_settled' THEN s.d1_due_date
              WHEN 'd3_settled' THEN s.d3_due_date
              WHEN 'd5_settled' THEN s.d5_due_date
          END
    );
    SELECT RAISE(ABORT, 'BR-178 outcome receipt preceding phase missing')
    WHERE (
        SELECT COUNT(DISTINCT pa.phase)
        FROM selection_outcome_attempts a
        JOIN selection_outcome_attempts pa ON pa.sample_key=a.sample_key
        JOIN selection_v2_run_stages pm
          ON pm.subject_kind='outcome_run'
         AND pm.subject_id=pa.outcome_run_id
         AND pm.run_status='settled'
        JOIN selection_v2_commit_receipts pr
          ON pr.subject_kind='outcome_run'
         AND pr.subject_id=pa.outcome_run_id
        WHERE a.outcome_run_id=NEW.subject_id
          AND pa.result_code='settled'
          AND (
              (a.phase='d1_settled' AND pa.phase='t0_close')
              OR (a.phase='d3_settled' AND pa.phase IN ('t0_close','d1_settled'))
              OR (a.phase='d5_settled'
                  AND pa.phase IN ('t0_close','d1_settled','d3_settled'))
          )
    ) <> (
        SELECT CASE m.outcome_phase
            WHEN 't0_close' THEN 0
            WHEN 'd1_settled' THEN 1
            WHEN 'd3_settled' THEN 2
            WHEN 'd5_settled' THEN 3
        END
        FROM selection_v2_run_stages m
        WHERE m.subject_kind='outcome_run' AND m.subject_id=NEW.subject_id
    );
    SELECT RAISE(ABORT, 'BR-178 outcome receipt requires exactly one attempt')
    WHERE (SELECT COUNT(*) FROM selection_outcome_attempts a
           WHERE a.outcome_run_id=NEW.subject_id) <> 1;
    SELECT RAISE(ABORT, 'BR-178 outcome receipt status/cardinality mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_outcome_attempts a ON a.outcome_run_id=m.subject_id
        WHERE m.subject_id=NEW.subject_id
          AND (
            (m.run_status='settled' AND a.result_code='settled'
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=1)
            OR
            (m.run_status='expected_wait' AND a.result_code='expected_wait'
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=0)
            OR
            (m.run_status='failed_retryable' AND a.result_code='error'
             AND a.retryable=1
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=0)
            OR
            (m.run_status='failed_non_retryable' AND a.result_code='error'
             AND a.retryable=0
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=0)
          )
    );
    SELECT RAISE(ABORT, 'BR-178 settled receipt hash mismatch')
    WHERE EXISTS (
        SELECT 1 FROM selection_v2_run_stages m
        JOIN selection_outcome_attempts a ON a.outcome_run_id=m.subject_id
        WHERE m.subject_id=NEW.subject_id AND m.run_status='settled'
          AND NOT EXISTS (
            SELECT 1 FROM selection_sample_outcomes o
            WHERE o.outcome_run_id=m.subject_id
              AND o.sample_key=a.sample_key AND o.phase=a.phase
              AND o.content_hash=a.settled_outcome_content_hash
          )
    );
END;
