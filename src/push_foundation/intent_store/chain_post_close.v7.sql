-- Private v7 draft only. The migration runner must execute this DDL, legacy
-- qualification backfill, full validation, registry seal, and layout-7 receipt
-- in one caller-owned IMMEDIATE transaction.

CREATE TABLE chain_post_close_concept_rpc_occurrences (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    outer_ordinal INTEGER NOT NULL CHECK (outer_ordinal >= 0),
    code TEXT NOT NULL CHECK (length(code) BETWEEN 1 AND 512),
    operation TEXT NOT NULL CHECK (operation = 'BoardConstituents'),
    request_id TEXT NOT NULL CHECK (length(request_id) BETWEEN 1 AND 512),
    request_codec_version INTEGER NOT NULL CHECK (request_codec_version = 1),
    request_bytes BLOB NOT NULL CHECK (
        typeof(request_bytes) = 'blob' AND length(request_bytes) > 0
    ),
    request_length INTEGER NOT NULL CHECK (request_length = length(request_bytes)),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    acquisition_request_hash TEXT NOT NULL CHECK (
        length(acquisition_request_hash) = 64
        AND acquisition_request_hash NOT GLOB '*[^0-9a-f]*'
    ),
    profile TEXT NOT NULL CHECK (profile IN ('LocalBridgeV1', 'ExternalV1')),
    acquisition_authority TEXT CHECK (
        acquisition_authority IS NULL
        OR length(acquisition_authority) BETWEEN 1 AND 512
    ),
    retry_max_attempts INTEGER NOT NULL CHECK (retry_max_attempts >= 1),
    retry_base_delay_ms INTEGER NOT NULL CHECK (retry_base_delay_ms >= 0),
    retry_max_delay_ms INTEGER NOT NULL CHECK (
        retry_max_delay_ms >= retry_base_delay_ms
    ),
    retry_jitter_ms INTEGER NOT NULL CHECK (retry_jitter_ms >= 0),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (
        length(run_context_sha256) = 64
        AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    input_sha256 TEXT NOT NULL CHECK (
        length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version >= 0),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    planned_at INTEGER NOT NULL CHECK (planned_at >= 0),
    PRIMARY KEY (intent_id, outer_ordinal),
    UNIQUE (intent_id, code),
    UNIQUE (intent_id, run_version),
    UNIQUE (intent_id, outer_ordinal, run_version),
    UNIQUE (intent_id, outer_ordinal, request_sha256),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_concept_rpc_attempt_begins (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    outer_ordinal INTEGER NOT NULL CHECK (outer_ordinal >= 0),
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal >= 1),
    occurrence_run_version INTEGER NOT NULL CHECK (occurrence_run_version >= 1),
    request_id TEXT NOT NULL CHECK (length(request_id) BETWEEN 1 AND 512),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    previous_attempt_ordinal INTEGER,
    previous_result_run_version INTEGER,
    previous_result_sha256 TEXT,
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (
        length(run_context_sha256) = 64
        AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    input_sha256 TEXT NOT NULL CHECK (
        length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version >= 0),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    begun_at INTEGER NOT NULL CHECK (begun_at >= 0),
    CHECK (
        (
            attempt_ordinal = 1
            AND previous_attempt_ordinal IS NULL
            AND previous_result_run_version IS NULL
            AND previous_result_sha256 IS NULL
        )
        OR (
            attempt_ordinal > 1
            AND previous_attempt_ordinal IS NOT NULL
            AND previous_attempt_ordinal = attempt_ordinal - 1
            AND previous_result_run_version IS NOT NULL
            AND previous_result_run_version >= 1
            AND previous_result_sha256 IS NOT NULL
            AND length(previous_result_sha256) = 64
            AND previous_result_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    ),
    PRIMARY KEY (intent_id, outer_ordinal, attempt_ordinal),
    UNIQUE (intent_id, run_version),
    UNIQUE (
        intent_id, outer_ordinal, attempt_ordinal, run_version,
        request_sha256, lease_owner, lease_generation
    ),
    FOREIGN KEY (intent_id, outer_ordinal, occurrence_run_version)
        REFERENCES chain_post_close_concept_rpc_occurrences(
            intent_id, outer_ordinal, run_version
        ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id, outer_ordinal, previous_attempt_ordinal,
        previous_result_run_version, previous_result_sha256
    ) REFERENCES chain_post_close_concept_rpc_attempt_results(
        intent_id, outer_ordinal, attempt_ordinal, run_version, result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_concept_rpc_attempt_results (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    outer_ordinal INTEGER NOT NULL CHECK (outer_ordinal >= 0),
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal >= 1),
    begin_run_version INTEGER NOT NULL CHECK (begin_run_version >= 1),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    wire_outcome TEXT NOT NULL CHECK (wire_outcome IN ('Response', 'Status')),
    result_codec_version INTEGER NOT NULL CHECK (result_codec_version = 1),
    result_bytes BLOB NOT NULL CHECK (
        typeof(result_bytes) = 'blob' AND length(result_bytes) > 0
    ),
    result_length INTEGER NOT NULL CHECK (result_length = length(result_bytes)),
    result_sha256 TEXT NOT NULL CHECK (
        length(result_sha256) = 64 AND result_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    continuation TEXT NOT NULL CHECK (continuation IN ('Retry', 'Terminal')),
    retry_decision TEXT NOT NULL CHECK (
        retry_decision IN ('RetryBackoff', 'RetryBounded', 'NoRetry')
    ),
    backoff_ms INTEGER CHECK (
        (
            continuation = 'Retry'
            AND retry_decision IN ('RetryBackoff', 'RetryBounded')
            AND backoff_ms IS NOT NULL
            AND backoff_ms >= 0
        )
        OR (continuation = 'Terminal' AND backoff_ms IS NULL)
    ),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (
        length(run_context_sha256) = 64
        AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    input_sha256 TEXT NOT NULL CHECK (
        length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version >= 0),
    run_version INTEGER NOT NULL CHECK (
        run_version = prior_head_version + 1 AND begin_run_version < run_version
    ),
    returned_at INTEGER NOT NULL CHECK (returned_at >= 0),
    committed_at INTEGER NOT NULL CHECK (committed_at >= returned_at),
    PRIMARY KEY (intent_id, outer_ordinal, attempt_ordinal),
    UNIQUE (intent_id, run_version),
    UNIQUE (
        intent_id, outer_ordinal, attempt_ordinal, run_version, result_sha256
    ),
    FOREIGN KEY (
        intent_id, outer_ordinal, attempt_ordinal, begin_run_version,
        request_sha256, lease_owner, lease_generation
    ) REFERENCES chain_post_close_concept_rpc_attempt_begins(
        intent_id, outer_ordinal, attempt_ordinal, run_version,
        request_sha256, lease_owner, lease_generation
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_concept_rpc_status_materials (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    outer_ordinal INTEGER NOT NULL CHECK (outer_ordinal >= 0),
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal >= 1),
    result_run_version INTEGER NOT NULL CHECK (result_run_version >= 1),
    result_sha256 TEXT NOT NULL CHECK (
        length(result_sha256) = 64 AND result_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    provenance TEXT NOT NULL CHECK (provenance = 'Captured'),
    projection_version INTEGER NOT NULL CHECK (projection_version = 1),
    material_codec_version INTEGER NOT NULL CHECK (material_codec_version = 1),
    material_bytes BLOB NOT NULL CHECK (
        typeof(material_bytes) = 'blob' AND length(material_bytes) > 0
    ),
    material_length INTEGER NOT NULL CHECK (material_length = length(material_bytes)),
    material_sha256 TEXT NOT NULL CHECK (
        length(material_sha256) = 64
        AND material_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (
        length(run_context_sha256) = 64
        AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    input_sha256 TEXT NOT NULL CHECK (
        length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (
        prior_head_version = result_run_version
    ),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    captured_at INTEGER NOT NULL CHECK (captured_at >= 0),
    PRIMARY KEY (intent_id, outer_ordinal, attempt_ordinal),
    UNIQUE (intent_id, run_version),
    UNIQUE (
        intent_id, outer_ordinal, attempt_ordinal, run_version, material_sha256
    ),
    FOREIGN KEY (
        intent_id, outer_ordinal, attempt_ordinal, result_run_version, result_sha256
    ) REFERENCES chain_post_close_concept_rpc_attempt_results(
        intent_id, outer_ordinal, attempt_ordinal, run_version, result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_concept_rpc_error_materials (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    outer_ordinal INTEGER NOT NULL CHECK (outer_ordinal >= 0),
    terminal_attempt_ordinal INTEGER NOT NULL CHECK (terminal_attempt_ordinal >= 1),
    terminal_result_run_version INTEGER NOT NULL CHECK (terminal_result_run_version >= 1),
    terminal_result_sha256 TEXT NOT NULL CHECK (
        length(terminal_result_sha256) = 64
        AND terminal_result_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    status_material_attempt_ordinal INTEGER,
    status_material_run_version INTEGER,
    status_material_sha256 TEXT,
    material_codec_version INTEGER NOT NULL CHECK (material_codec_version = 1),
    material_bytes BLOB NOT NULL CHECK (
        typeof(material_bytes) = 'blob' AND length(material_bytes) > 0
    ),
    material_length INTEGER NOT NULL CHECK (material_length = length(material_bytes)),
    material_sha256 TEXT NOT NULL CHECK (
        length(material_sha256) = 64 AND material_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    observed_fallback TEXT NOT NULL CHECK (length(observed_fallback) BETWEEN 1 AND 64),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (
        length(run_context_sha256) = 64
        AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    input_sha256 TEXT NOT NULL CHECK (
        length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version >= 1),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    captured_at INTEGER NOT NULL CHECK (captured_at >= 0),
    CHECK (
        (
            status_material_attempt_ordinal IS NULL
            AND status_material_run_version IS NULL
            AND status_material_sha256 IS NULL
        )
        OR (
            status_material_attempt_ordinal IS NOT NULL
            AND status_material_attempt_ordinal = terminal_attempt_ordinal
            AND status_material_run_version IS NOT NULL
            AND status_material_run_version >= 1
            AND status_material_sha256 IS NOT NULL
            AND length(status_material_sha256) = 64
            AND status_material_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    ),
    PRIMARY KEY (intent_id, outer_ordinal),
    UNIQUE (intent_id, run_version),
    UNIQUE (intent_id, outer_ordinal, run_version, material_sha256),
    FOREIGN KEY (
        intent_id, outer_ordinal, terminal_attempt_ordinal,
        terminal_result_run_version, terminal_result_sha256
    ) REFERENCES chain_post_close_concept_rpc_attempt_results(
        intent_id, outer_ordinal, attempt_ordinal, run_version, result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id, outer_ordinal, status_material_attempt_ordinal,
        status_material_run_version, status_material_sha256
    ) REFERENCES chain_post_close_concept_rpc_status_materials(
        intent_id, outer_ordinal, attempt_ordinal, run_version, material_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_concept_rpc_finals (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    outer_ordinal INTEGER NOT NULL CHECK (outer_ordinal >= 0),
    code TEXT NOT NULL CHECK (length(code) BETWEEN 1 AND 512),
    provenance TEXT NOT NULL CHECK (provenance = 'CompatibilityProjection'),
    occurrence_run_version INTEGER NOT NULL CHECK (occurrence_run_version >= 1),
    occurrence_request_sha256 TEXT NOT NULL CHECK (
        length(occurrence_request_sha256) = 64
        AND occurrence_request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    terminal_attempt_ordinal INTEGER NOT NULL CHECK (terminal_attempt_ordinal >= 1),
    terminal_result_run_version INTEGER NOT NULL CHECK (terminal_result_run_version >= 1),
    terminal_result_sha256 TEXT NOT NULL CHECK (
        length(terminal_result_sha256) = 64
        AND terminal_result_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    error_material_run_version INTEGER,
    error_material_sha256 TEXT,
    final_outcome TEXT NOT NULL CHECK (
        final_outcome IN ('Available', 'VerifiedEmpty', 'Error')
    ),
    final_codec_version INTEGER NOT NULL CHECK (final_codec_version = 1),
    final_bytes BLOB NOT NULL CHECK (
        typeof(final_bytes) = 'blob' AND length(final_bytes) > 0
    ),
    final_length INTEGER NOT NULL CHECK (final_length = length(final_bytes)),
    final_sha256 TEXT NOT NULL CHECK (
        length(final_sha256) = 64 AND final_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    outer_outcome TEXT NOT NULL CHECK (outer_outcome IN ('Returned', 'BusinessError')),
    outer_begin_run_version INTEGER NOT NULL CHECK (outer_begin_run_version >= 1),
    outer_begin_sha256 TEXT NOT NULL CHECK (
        length(outer_begin_sha256) = 64
        AND outer_begin_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    outer_result_run_version INTEGER NOT NULL CHECK (outer_result_run_version >= 1),
    outer_result_sha256 TEXT NOT NULL CHECK (
        length(outer_result_sha256) = 64
        AND outer_result_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    audit_id INTEGER NOT NULL CHECK (audit_id >= 1),
    audit_record_hash TEXT NOT NULL CHECK (
        length(audit_record_hash) = 64
        AND audit_record_hash NOT GLOB '*[^0-9a-f]*'
    ),
    previous_outcome TEXT CHECK (
        previous_outcome IS NULL OR previous_outcome IN (
            'available', 'verified_empty', 'invalid_request', 'unavailable',
            'stale', 'partial', 'conflict', 'unsupported'
        )
    ),
    current_outcome TEXT NOT NULL CHECK (current_outcome IN (
        'available', 'verified_empty', 'invalid_request', 'unavailable',
        'stale', 'partial', 'conflict', 'unsupported'
    )),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (
        length(run_context_sha256) = 64
        AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    input_sha256 TEXT NOT NULL CHECK (
        length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (
        prior_head_version = outer_result_run_version
        AND prior_head_version >= 2
    ),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    applied_at INTEGER NOT NULL CHECK (applied_at >= 0),
    CHECK (outer_begin_run_version + 1 = outer_result_run_version),
    CHECK (
        (
            final_outcome = 'Available'
            AND outer_outcome = 'Returned'
            AND current_outcome = 'available'
            AND error_material_run_version IS NULL
            AND error_material_sha256 IS NULL
        )
        OR (
            final_outcome = 'VerifiedEmpty'
            AND outer_outcome = 'BusinessError'
            AND current_outcome = 'verified_empty'
            AND error_material_run_version IS NULL
            AND error_material_sha256 IS NULL
        )
        OR (
            final_outcome = 'Error'
            AND outer_outcome = 'BusinessError'
            AND current_outcome NOT IN ('available', 'verified_empty')
            AND error_material_run_version IS NOT NULL
            AND error_material_run_version >= 1
            AND error_material_sha256 IS NOT NULL
            AND length(error_material_sha256) = 64
            AND error_material_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    ),
    PRIMARY KEY (intent_id, outer_ordinal),
    UNIQUE (intent_id, run_version),
    UNIQUE (audit_id),
    FOREIGN KEY (intent_id, outer_ordinal, occurrence_run_version)
        REFERENCES chain_post_close_concept_rpc_occurrences(
            intent_id, outer_ordinal, run_version
        ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id, outer_ordinal, terminal_attempt_ordinal,
        terminal_result_run_version, terminal_result_sha256
    ) REFERENCES chain_post_close_concept_rpc_attempt_results(
        intent_id, outer_ordinal, attempt_ordinal, run_version, result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id, outer_ordinal, error_material_run_version, error_material_sha256
    ) REFERENCES chain_post_close_concept_rpc_error_materials(
        intent_id, outer_ordinal, run_version, material_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, outer_begin_run_version)
        REFERENCES chain_post_close_stage_begins(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, outer_result_run_version)
        REFERENCES chain_post_close_stage_results(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (audit_id) REFERENCES data_acquisition_audit(id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (audit_id) REFERENCES data_acquisition_audit_chain(acquisition_audit_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_concept_rpc_legacy_outer_qualifications (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    effect_kind TEXT NOT NULL CHECK (effect_kind = 'ConceptProvider'),
    outer_ordinal INTEGER NOT NULL CHECK (outer_ordinal >= 0),
    code TEXT NOT NULL CHECK (length(code) BETWEEN 1 AND 512),
    qualification_kind TEXT NOT NULL CHECK (qualification_kind IN (
        'LegacyReturnedApplied', 'LegacyReturnedPendingCache',
        'LegacyBusinessError', 'LegacyUnconfirmed'
    )),
    begin_run_version INTEGER NOT NULL CHECK (begin_run_version >= 1),
    begin_request_sha256 TEXT NOT NULL CHECK (
        length(begin_request_sha256) = 64
        AND begin_request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    begin_lease_owner TEXT NOT NULL CHECK (length(begin_lease_owner) BETWEEN 1 AND 512),
    begin_lease_generation INTEGER NOT NULL CHECK (begin_lease_generation >= 1),
    begun_at INTEGER NOT NULL CHECK (begun_at >= 0),
    result_outcome TEXT,
    result_run_version INTEGER,
    result_sha256 TEXT,
    result_lease_owner TEXT,
    result_lease_generation INTEGER,
    result_committed_at INTEGER,
    cache_run_version INTEGER,
    cache_sha256 TEXT,
    cache_lease_owner TEXT,
    cache_lease_generation INTEGER,
    cache_written_at INTEGER,
    qualified_from_layout_version INTEGER NOT NULL CHECK (
        qualified_from_layout_version = 6
    ),
    sealed_by_layout_version INTEGER NOT NULL CHECK (sealed_by_layout_version = 7),
    CHECK (
        (
            qualification_kind = 'LegacyUnconfirmed'
            AND result_outcome IS NULL
            AND result_run_version IS NULL
            AND result_sha256 IS NULL
            AND result_lease_owner IS NULL
            AND result_lease_generation IS NULL
            AND result_committed_at IS NULL
            AND cache_run_version IS NULL
            AND cache_sha256 IS NULL
            AND cache_lease_owner IS NULL
            AND cache_lease_generation IS NULL
            AND cache_written_at IS NULL
        )
        OR (
            qualification_kind IN ('LegacyReturnedPendingCache', 'LegacyBusinessError')
            AND result_outcome IS NOT NULL
            AND (
                (qualification_kind = 'LegacyReturnedPendingCache' AND result_outcome = 'Returned')
                OR (qualification_kind = 'LegacyBusinessError' AND result_outcome = 'BusinessError')
            )
            AND result_run_version IS NOT NULL
            AND result_run_version >= 1
            AND result_sha256 IS NOT NULL
            AND length(result_sha256) = 64
            AND result_sha256 NOT GLOB '*[^0-9a-f]*'
            AND result_lease_owner IS NOT NULL
            AND length(result_lease_owner) BETWEEN 1 AND 512
            AND result_lease_generation IS NOT NULL
            AND result_lease_generation >= 1
            AND result_committed_at IS NOT NULL
            AND result_committed_at >= begun_at
            AND cache_run_version IS NULL
            AND cache_sha256 IS NULL
            AND cache_lease_owner IS NULL
            AND cache_lease_generation IS NULL
            AND cache_written_at IS NULL
        )
        OR (
            qualification_kind = 'LegacyReturnedApplied'
            AND result_outcome IS NOT NULL
            AND result_outcome = 'Returned'
            AND result_run_version IS NOT NULL
            AND result_run_version >= 1
            AND result_sha256 IS NOT NULL
            AND length(result_sha256) = 64
            AND result_sha256 NOT GLOB '*[^0-9a-f]*'
            AND result_lease_owner IS NOT NULL
            AND length(result_lease_owner) BETWEEN 1 AND 512
            AND result_lease_generation IS NOT NULL
            AND result_lease_generation >= 1
            AND result_committed_at IS NOT NULL
            AND result_committed_at >= begun_at
            AND cache_run_version IS NOT NULL
            AND cache_run_version >= 1
            AND cache_sha256 IS NOT NULL
            AND length(cache_sha256) = 64
            AND cache_sha256 NOT GLOB '*[^0-9a-f]*'
            AND cache_lease_owner IS NOT NULL
            AND length(cache_lease_owner) BETWEEN 1 AND 512
            AND cache_lease_generation IS NOT NULL
            AND cache_lease_generation >= 1
            AND cache_written_at IS NOT NULL
            AND cache_written_at >= result_committed_at
        )
    ),
    PRIMARY KEY (intent_id, outer_ordinal),
    UNIQUE (intent_id, code),
    FOREIGN KEY (intent_id, effect_kind, outer_ordinal)
        REFERENCES chain_post_close_stage_begins(
            intent_id, effect_kind, effect_ordinal
        ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TRIGGER chain_post_close_concept_rpc_occurrences_guard
BEFORE INSERT ON chain_post_close_concept_rpc_occurrences
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 7
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.run_id = NEW.run_id
      AND run.run_context_sha256 = NEW.run_context_sha256
      AND run.input_sha256 = NEW.input_sha256
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.updated_at = NEW.planned_at
      AND run.lease_until > NEW.planned_at
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id, run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_status_materials
        WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_finals
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_stage_begins AS begun
    WHERE begun.intent_id = NEW.intent_id
      AND begun.effect_kind = 'ConceptProvider'
      AND (begun.effect_ordinal = NEW.outer_ordinal OR begun.effect_key = NEW.code)
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_legacy_outer_qualifications AS legacy
    WHERE legacy.intent_id = NEW.intent_id
      AND (legacy.outer_ordinal = NEW.outer_ordinal OR legacy.code = NEW.code)
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_occurrence_invalid');
END;

CREATE TRIGGER chain_post_close_concept_rpc_occurrences_update
BEFORE UPDATE ON chain_post_close_concept_rpc_occurrences
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_occurrence_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_occurrences_delete
BEFORE DELETE ON chain_post_close_concept_rpc_occurrences
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_occurrence_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_attempt_begins_guard
BEFORE INSERT ON chain_post_close_concept_rpc_attempt_begins
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 7
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.run_id = NEW.run_id
      AND run.run_context_sha256 = NEW.run_context_sha256
      AND run.input_sha256 = NEW.input_sha256
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.updated_at = NEW.begun_at
      AND run.lease_until > NEW.begun_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_occurrences AS occurrence
    WHERE occurrence.intent_id = NEW.intent_id
      AND occurrence.outer_ordinal = NEW.outer_ordinal
      AND occurrence.run_version = NEW.occurrence_run_version
      AND occurrence.request_id = NEW.request_id
      AND occurrence.request_sha256 = NEW.request_sha256
      AND occurrence.run_id = NEW.run_id
      AND occurrence.run_context_sha256 = NEW.run_context_sha256
      AND occurrence.input_sha256 = NEW.input_sha256
      AND occurrence.run_version < NEW.run_version
      AND occurrence.planned_at <= NEW.begun_at
      AND NEW.attempt_ordinal <= occurrence.retry_max_attempts
)
OR (
    NEW.attempt_ordinal = 1
    AND EXISTS (
        SELECT 1 FROM chain_post_close_concept_rpc_attempt_results AS result
        WHERE result.intent_id = NEW.intent_id
          AND result.outer_ordinal = NEW.outer_ordinal
    )
)
OR (
    NEW.attempt_ordinal > 1
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_concept_rpc_attempt_results AS result
        WHERE result.intent_id = NEW.intent_id
          AND result.outer_ordinal = NEW.outer_ordinal
          AND result.attempt_ordinal = NEW.previous_attempt_ordinal
          AND result.run_version = NEW.previous_result_run_version
          AND result.result_sha256 = NEW.previous_result_sha256
          AND result.request_sha256 = NEW.request_sha256
          AND result.continuation = 'Retry'
          AND result.run_version < NEW.run_version
          AND result.committed_at <= NEW.begun_at
    )
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_finals AS final
    WHERE final.intent_id = NEW.intent_id
      AND final.outer_ordinal = NEW.outer_ordinal
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id, run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_status_materials
        WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_finals
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_attempt_begin_invalid');
END;

CREATE TRIGGER chain_post_close_concept_rpc_attempt_begins_update
BEFORE UPDATE ON chain_post_close_concept_rpc_attempt_begins
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_attempt_begin_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_attempt_begins_delete
BEFORE DELETE ON chain_post_close_concept_rpc_attempt_begins
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_attempt_begin_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_attempt_results_guard
BEFORE INSERT ON chain_post_close_concept_rpc_attempt_results
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 7
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.run_id = NEW.run_id
      AND run.run_context_sha256 = NEW.run_context_sha256
      AND run.input_sha256 = NEW.input_sha256
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.updated_at = NEW.committed_at
      AND run.lease_until > NEW.committed_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_attempt_begins AS begun
    JOIN chain_post_close_concept_rpc_occurrences AS occurrence
      ON occurrence.intent_id = begun.intent_id
     AND occurrence.outer_ordinal = begun.outer_ordinal
    WHERE begun.intent_id = NEW.intent_id
      AND begun.outer_ordinal = NEW.outer_ordinal
      AND begun.attempt_ordinal = NEW.attempt_ordinal
      AND begun.run_version = NEW.begin_run_version
      AND begun.request_sha256 = NEW.request_sha256
      AND begun.run_id = NEW.run_id
      AND begun.run_context_sha256 = NEW.run_context_sha256
      AND begun.input_sha256 = NEW.input_sha256
      AND begun.lease_owner = NEW.lease_owner
      AND begun.lease_generation = NEW.lease_generation
      AND begun.begun_at <= NEW.returned_at
      AND (
          NEW.continuation = 'Terminal'
          OR NEW.attempt_ordinal < occurrence.retry_max_attempts
      )
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_attempt_begins AS later
    WHERE later.intent_id = NEW.intent_id
      AND later.outer_ordinal = NEW.outer_ordinal
      AND later.attempt_ordinal > NEW.attempt_ordinal
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id, run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_status_materials
        WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_finals
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_attempt_result_invalid');
END;

CREATE TRIGGER chain_post_close_concept_rpc_attempt_results_update
BEFORE UPDATE ON chain_post_close_concept_rpc_attempt_results
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_attempt_result_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_attempt_results_delete
BEFORE DELETE ON chain_post_close_concept_rpc_attempt_results
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_attempt_result_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_status_materials_guard
BEFORE INSERT ON chain_post_close_concept_rpc_status_materials
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 7
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.run_id = NEW.run_id
      AND run.run_context_sha256 = NEW.run_context_sha256
      AND run.input_sha256 = NEW.input_sha256
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.updated_at = NEW.captured_at
      AND run.lease_until > NEW.captured_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_attempt_results AS result
    WHERE result.intent_id = NEW.intent_id
      AND result.outer_ordinal = NEW.outer_ordinal
      AND result.attempt_ordinal = NEW.attempt_ordinal
      AND result.run_version = NEW.result_run_version
      AND result.result_sha256 = NEW.result_sha256
      AND result.request_sha256 = NEW.request_sha256
      AND result.wire_outcome = 'Status'
      AND result.run_id = NEW.run_id
      AND result.run_context_sha256 = NEW.run_context_sha256
      AND result.input_sha256 = NEW.input_sha256
      AND result.lease_owner = NEW.lease_owner
      AND result.lease_generation = NEW.lease_generation
      AND result.committed_at <= NEW.captured_at
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id, run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_status_materials
        WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_finals
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_status_material_invalid');
END;

CREATE TRIGGER chain_post_close_concept_rpc_status_materials_update
BEFORE UPDATE ON chain_post_close_concept_rpc_status_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_status_material_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_status_materials_delete
BEFORE DELETE ON chain_post_close_concept_rpc_status_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_status_material_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_error_materials_guard
BEFORE INSERT ON chain_post_close_concept_rpc_error_materials
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 7
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.run_id = NEW.run_id
      AND run.run_context_sha256 = NEW.run_context_sha256
      AND run.input_sha256 = NEW.input_sha256
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.updated_at = NEW.captured_at
      AND run.lease_until > NEW.captured_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_attempt_results AS result
    WHERE result.intent_id = NEW.intent_id
      AND result.outer_ordinal = NEW.outer_ordinal
      AND result.attempt_ordinal = NEW.terminal_attempt_ordinal
      AND result.run_version = NEW.terminal_result_run_version
      AND result.result_sha256 = NEW.terminal_result_sha256
      AND result.request_sha256 = NEW.request_sha256
      AND result.continuation = 'Terminal'
      AND result.run_id = NEW.run_id
      AND result.run_context_sha256 = NEW.run_context_sha256
      AND result.input_sha256 = NEW.input_sha256
      AND result.committed_at <= NEW.captured_at
      AND (
          (
              result.wire_outcome = 'Response'
              AND NEW.status_material_attempt_ordinal IS NULL
              AND NEW.status_material_run_version IS NULL
              AND NEW.status_material_sha256 IS NULL
              AND NEW.prior_head_version = result.run_version
          )
          OR (
              result.wire_outcome = 'Status'
              AND EXISTS (
                  SELECT 1
                  FROM chain_post_close_concept_rpc_status_materials AS status
                  WHERE status.intent_id = NEW.intent_id
                    AND status.outer_ordinal = NEW.outer_ordinal
                    AND status.attempt_ordinal = NEW.status_material_attempt_ordinal
                    AND status.run_version = NEW.status_material_run_version
                    AND status.material_sha256 = NEW.status_material_sha256
                    AND status.result_run_version = result.run_version
                    AND status.request_sha256 = NEW.request_sha256
                    AND status.run_version = NEW.prior_head_version
                    AND status.captured_at <= NEW.captured_at
              )
          )
      )
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_attempt_begins AS later
    WHERE later.intent_id = NEW.intent_id
      AND later.outer_ordinal = NEW.outer_ordinal
      AND later.attempt_ordinal > NEW.terminal_attempt_ordinal
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id, run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_status_materials
        WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_finals
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_error_material_invalid');
END;

CREATE TRIGGER chain_post_close_concept_rpc_error_materials_update
BEFORE UPDATE ON chain_post_close_concept_rpc_error_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_error_material_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_error_materials_delete
BEFORE DELETE ON chain_post_close_concept_rpc_error_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_error_material_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_finals_guard
BEFORE INSERT ON chain_post_close_concept_rpc_finals
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 7
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.run_id = NEW.run_id
      AND run.run_context_sha256 = NEW.run_context_sha256
      AND run.input_sha256 = NEW.input_sha256
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.updated_at = NEW.applied_at
      AND run.lease_until > NEW.applied_at
)
OR NOT EXISTS (
    SELECT 1
    FROM chain_post_close_concept_rpc_occurrences AS occurrence
    JOIN chain_post_close_concept_rpc_attempt_results AS terminal
      ON terminal.intent_id = occurrence.intent_id
     AND terminal.outer_ordinal = occurrence.outer_ordinal
    WHERE occurrence.intent_id = NEW.intent_id
      AND occurrence.outer_ordinal = NEW.outer_ordinal
      AND occurrence.code = NEW.code
      AND occurrence.run_version = NEW.occurrence_run_version
      AND occurrence.request_sha256 = NEW.occurrence_request_sha256
      AND terminal.attempt_ordinal = NEW.terminal_attempt_ordinal
      AND terminal.run_version = NEW.terminal_result_run_version
      AND terminal.result_sha256 = NEW.terminal_result_sha256
      AND terminal.request_sha256 = occurrence.request_sha256
      AND terminal.continuation = 'Terminal'
      AND terminal.committed_at <= NEW.applied_at
      AND (
          (
              NEW.final_outcome IN ('Available', 'VerifiedEmpty')
              AND terminal.wire_outcome = 'Response'
              AND NEW.error_material_run_version IS NULL
              AND NEW.error_material_sha256 IS NULL
          )
          OR (
              NEW.final_outcome = 'Error'
              AND EXISTS (
                  SELECT 1
                  FROM chain_post_close_concept_rpc_error_materials AS error
                  WHERE error.intent_id = NEW.intent_id
                    AND error.outer_ordinal = NEW.outer_ordinal
                    AND error.run_version = NEW.error_material_run_version
                    AND error.material_sha256 = NEW.error_material_sha256
                    AND error.terminal_attempt_ordinal = terminal.attempt_ordinal
                    AND error.terminal_result_run_version = terminal.run_version
                    AND error.terminal_result_sha256 = terminal.result_sha256
                    AND error.run_version < NEW.outer_begin_run_version
              )
          )
      )
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_attempt_begins AS later
    WHERE later.intent_id = NEW.intent_id
      AND later.outer_ordinal = NEW.outer_ordinal
      AND later.attempt_ordinal > NEW.terminal_attempt_ordinal
)
OR NOT EXISTS (
    SELECT 1
    FROM chain_post_close_stage_begins AS begun
    JOIN chain_post_close_stage_results AS result
      ON result.intent_id = begun.intent_id
     AND result.effect_kind = begun.effect_kind
     AND result.effect_ordinal = begun.effect_ordinal
    WHERE begun.intent_id = NEW.intent_id
      AND begun.effect_kind = 'ConceptProvider'
      AND begun.effect_ordinal = NEW.outer_ordinal
      AND begun.effect_key = NEW.code
      AND begun.run_version = NEW.outer_begin_run_version
      AND begun.request_sha256 = NEW.outer_begin_sha256
      AND begun.lease_owner = NEW.lease_owner
      AND begun.lease_generation = NEW.lease_generation
      AND begun.begun_at <= NEW.applied_at
      AND result.outcome = NEW.outer_outcome
      AND result.run_version = NEW.outer_result_run_version
      AND result.result_sha256 = NEW.outer_result_sha256
      AND result.lease_owner = NEW.lease_owner
      AND result.lease_generation = NEW.lease_generation
      AND result.committed_at <= NEW.applied_at
)
OR NOT EXISTS (
    SELECT 1
    FROM data_acquisition_audit AS audit
    JOIN data_acquisition_audit_chain AS chain
      ON chain.acquisition_audit_id = audit.id
    WHERE audit.id = NEW.audit_id
      AND audit.request_hash = (
          SELECT occurrence.acquisition_request_hash
          FROM chain_post_close_concept_rpc_occurrences AS occurrence
          WHERE occurrence.intent_id = NEW.intent_id
            AND occurrence.outer_ordinal = NEW.outer_ordinal
      )
      AND audit.outcome = NEW.current_outcome
      AND chain.record_hash = NEW.audit_record_hash
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_legacy_outer_qualifications AS legacy
    WHERE legacy.intent_id = NEW.intent_id
      AND legacy.outer_ordinal = NEW.outer_ordinal
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id, run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_status_materials
        WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_concept_rpc_finals
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_final_invalid');
END;

CREATE TRIGGER chain_post_close_concept_rpc_finals_update
BEFORE UPDATE ON chain_post_close_concept_rpc_finals
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_final_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_finals_delete
BEFORE DELETE ON chain_post_close_concept_rpc_finals
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_final_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_legacy_outer_qualifications_guard
BEFORE INSERT ON chain_post_close_concept_rpc_legacy_outer_qualifications
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 6
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version >= 7
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_stage_begins AS begun
    WHERE begun.intent_id = NEW.intent_id
      AND begun.effect_kind = NEW.effect_kind
      AND begun.effect_ordinal = NEW.outer_ordinal
      AND begun.effect_key = NEW.code
      AND begun.run_version = NEW.begin_run_version
      AND begun.request_sha256 = NEW.begin_request_sha256
      AND begun.lease_owner = NEW.begin_lease_owner
      AND begun.lease_generation = NEW.begin_lease_generation
      AND begun.begun_at = NEW.begun_at
)
OR (
    NEW.qualification_kind = 'LegacyUnconfirmed'
    AND EXISTS (
        SELECT 1 FROM chain_post_close_stage_results AS result
        WHERE result.intent_id = NEW.intent_id
          AND result.effect_kind = NEW.effect_kind
          AND result.effect_ordinal = NEW.outer_ordinal
    )
)
OR (
    NEW.qualification_kind IN (
        'LegacyReturnedApplied', 'LegacyReturnedPendingCache',
        'LegacyBusinessError'
    )
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_stage_results AS result
        WHERE result.intent_id = NEW.intent_id
          AND result.effect_kind = NEW.effect_kind
          AND result.effect_ordinal = NEW.outer_ordinal
          AND result.outcome = NEW.result_outcome
          AND result.run_version = NEW.result_run_version
          AND result.result_sha256 = NEW.result_sha256
          AND result.lease_owner = NEW.result_lease_owner
          AND result.lease_generation = NEW.result_lease_generation
          AND result.returned_at <= NEW.result_committed_at
          AND result.committed_at = NEW.result_committed_at
    )
)
OR (
    NEW.qualification_kind IN (
        'LegacyReturnedPendingCache', 'LegacyBusinessError'
    )
    AND EXISTS (
        SELECT 1 FROM chain_post_close_concept_cache_writes AS cache
        WHERE cache.intent_id = NEW.intent_id
          AND cache.effect_kind = NEW.effect_kind
          AND cache.effect_ordinal = NEW.outer_ordinal
    )
)
OR (
    NEW.qualification_kind = 'LegacyReturnedApplied'
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_concept_cache_writes AS cache
        WHERE cache.intent_id = NEW.intent_id
          AND cache.effect_kind = NEW.effect_kind
          AND cache.effect_ordinal = NEW.outer_ordinal
          AND cache.code = NEW.code
          AND cache.provider_result_run_version = NEW.result_run_version
          AND cache.run_version = NEW.cache_run_version
          AND cache.concepts_sha256 = NEW.cache_sha256
          AND cache.lease_owner = NEW.cache_lease_owner
          AND cache.lease_generation = NEW.cache_lease_generation
          AND cache.written_at = NEW.cache_written_at
    )
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_occurrences AS occurrence
    WHERE occurrence.intent_id = NEW.intent_id
      AND occurrence.outer_ordinal = NEW.outer_ordinal
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_finals AS final
    WHERE final.intent_id = NEW.intent_id
      AND final.outer_ordinal = NEW.outer_ordinal
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_legacy_qualification_invalid');
END;

CREATE TRIGGER chain_post_close_concept_rpc_legacy_outer_qualifications_update
BEFORE UPDATE ON chain_post_close_concept_rpc_legacy_outer_qualifications
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_legacy_qualification_immutable');
END;

CREATE TRIGGER chain_post_close_concept_rpc_legacy_outer_qualifications_delete
BEFORE DELETE ON chain_post_close_concept_rpc_legacy_outer_qualifications
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_rpc_legacy_qualification_immutable');
END;
