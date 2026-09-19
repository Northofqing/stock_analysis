-- Private v9 DDL candidate only. The caller-owned migration transaction must
-- verify layout 8 and all old facts, install this DDL, validate layout 9 facts,
-- then install registry/layout rows, seal, re-verify, and COMMIT.

CREATE TABLE chain_post_close_position_concept_rpc_occurrences (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    cache_material_run_version INTEGER NOT NULL CHECK (cache_material_run_version >= 1),
    position_ordinal INTEGER NOT NULL CHECK (position_ordinal >= 0),
    code TEXT NOT NULL CHECK (length(code) BETWEEN 1 AND 512),
    cache_material_sha256 TEXT NOT NULL CHECK (
        length(cache_material_sha256) = 64
        AND cache_material_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    positions_run_version INTEGER NOT NULL CHECK (positions_run_version >= 1),
    positions_sha256 TEXT NOT NULL CHECK (
        length(positions_sha256) = 64
        AND positions_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
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
    retry_max_delay_ms INTEGER NOT NULL CHECK (retry_max_delay_ms >= 0),
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
    run_version INTEGER NOT NULL CHECK (
        run_version = prior_head_version + 1
        AND cache_material_run_version < run_version
        AND positions_run_version < cache_material_run_version
    ),
    planned_at INTEGER NOT NULL CHECK (planned_at >= 0),
    PRIMARY KEY (intent_id, cache_material_run_version, position_ordinal),
    UNIQUE (intent_id, run_version),
    UNIQUE (intent_id, cache_material_run_version, position_ordinal, run_version),
    UNIQUE (intent_id, cache_material_run_version, position_ordinal, request_sha256),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, cache_material_run_version)
        REFERENCES chain_post_close_position_concept_materials(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, positions_run_version, positions_sha256)
        REFERENCES chain_post_close_position_materials(
            intent_id, run_version, material_sha256
        ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_position_concept_rpc_attempt_begins (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    cache_material_run_version INTEGER NOT NULL CHECK (cache_material_run_version >= 1),
    position_ordinal INTEGER NOT NULL CHECK (position_ordinal >= 0),
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
    PRIMARY KEY (
        intent_id, cache_material_run_version, position_ordinal, attempt_ordinal
    ),
    UNIQUE (intent_id, run_version),
    UNIQUE (
        intent_id, cache_material_run_version, position_ordinal, attempt_ordinal,
        run_version, request_sha256, lease_owner, lease_generation
    ),
    FOREIGN KEY (
        intent_id, cache_material_run_version, position_ordinal,
        occurrence_run_version
    ) REFERENCES chain_post_close_position_concept_rpc_occurrences(
        intent_id, cache_material_run_version, position_ordinal, run_version
    ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id, cache_material_run_version, position_ordinal,
        previous_attempt_ordinal, previous_result_run_version,
        previous_result_sha256
    ) REFERENCES chain_post_close_position_concept_rpc_attempt_results(
        intent_id, cache_material_run_version, position_ordinal,
        attempt_ordinal, run_version, result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_position_concept_rpc_attempt_results (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    cache_material_run_version INTEGER NOT NULL CHECK (cache_material_run_version >= 1),
    position_ordinal INTEGER NOT NULL CHECK (position_ordinal >= 0),
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
            wire_outcome = 'Status'
            AND continuation = 'Retry'
            AND retry_decision IN ('RetryBackoff', 'RetryBounded')
            AND backoff_ms IS NOT NULL
            AND backoff_ms >= 0
        )
        OR (
            continuation = 'Terminal'
            AND (
                wire_outcome = 'Status'
                OR (wire_outcome = 'Response' AND retry_decision = 'NoRetry')
            )
            AND backoff_ms IS NULL
        )
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
    PRIMARY KEY (
        intent_id, cache_material_run_version, position_ordinal, attempt_ordinal
    ),
    UNIQUE (intent_id, run_version),
    UNIQUE (
        intent_id, cache_material_run_version, position_ordinal,
        attempt_ordinal, run_version, result_sha256
    ),
    FOREIGN KEY (
        intent_id, cache_material_run_version, position_ordinal,
        attempt_ordinal, begin_run_version, request_sha256,
        lease_owner, lease_generation
    ) REFERENCES chain_post_close_position_concept_rpc_attempt_begins(
        intent_id, cache_material_run_version, position_ordinal,
        attempt_ordinal, run_version, request_sha256,
        lease_owner, lease_generation
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_position_concept_rpc_status_materials (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    cache_material_run_version INTEGER NOT NULL CHECK (cache_material_run_version >= 1),
    position_ordinal INTEGER NOT NULL CHECK (position_ordinal >= 0),
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
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version = result_run_version),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    captured_at INTEGER NOT NULL CHECK (captured_at >= 0),
    PRIMARY KEY (
        intent_id, cache_material_run_version, position_ordinal, attempt_ordinal
    ),
    UNIQUE (intent_id, run_version),
    UNIQUE (
        intent_id, cache_material_run_version, position_ordinal,
        attempt_ordinal, run_version, material_sha256
    ),
    FOREIGN KEY (
        intent_id, cache_material_run_version, position_ordinal,
        attempt_ordinal, result_run_version, result_sha256
    ) REFERENCES chain_post_close_position_concept_rpc_attempt_results(
        intent_id, cache_material_run_version, position_ordinal,
        attempt_ordinal, run_version, result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_position_concept_rpc_error_materials (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    cache_material_run_version INTEGER NOT NULL CHECK (cache_material_run_version >= 1),
    position_ordinal INTEGER NOT NULL CHECK (position_ordinal >= 0),
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
        length(material_sha256) = 64
        AND material_sha256 NOT GLOB '*[^0-9a-f]*'
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
    PRIMARY KEY (intent_id, cache_material_run_version, position_ordinal),
    UNIQUE (intent_id, run_version),
    UNIQUE (
        intent_id, cache_material_run_version, position_ordinal,
        run_version, material_sha256
    ),
    FOREIGN KEY (
        intent_id, cache_material_run_version, position_ordinal,
        terminal_attempt_ordinal, terminal_result_run_version,
        terminal_result_sha256
    ) REFERENCES chain_post_close_position_concept_rpc_attempt_results(
        intent_id, cache_material_run_version, position_ordinal,
        attempt_ordinal, run_version, result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id, cache_material_run_version, position_ordinal,
        status_material_attempt_ordinal, status_material_run_version,
        status_material_sha256
    ) REFERENCES chain_post_close_position_concept_rpc_status_materials(
        intent_id, cache_material_run_version, position_ordinal,
        attempt_ordinal, run_version, material_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_position_concept_rpc_finals (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    cache_material_run_version INTEGER NOT NULL CHECK (cache_material_run_version >= 1),
    position_ordinal INTEGER NOT NULL CHECK (position_ordinal >= 0),
    code TEXT NOT NULL CHECK (length(code) BETWEEN 1 AND 512),
    provenance TEXT NOT NULL CHECK (provenance = 'PositionConceptRpc'),
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
        prior_head_version >= terminal_result_run_version
    ),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    applied_at INTEGER NOT NULL CHECK (applied_at >= 0),
    CHECK (
        (
            final_outcome IN ('Available', 'VerifiedEmpty')
            AND error_material_run_version IS NULL
            AND error_material_sha256 IS NULL
        )
        OR (
            final_outcome = 'Error'
            AND error_material_run_version IS NOT NULL
            AND error_material_run_version >= 1
            AND error_material_sha256 IS NOT NULL
            AND length(error_material_sha256) = 64
            AND error_material_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    ),
    PRIMARY KEY (intent_id, cache_material_run_version, position_ordinal),
    UNIQUE (intent_id, run_version),
    UNIQUE (audit_id),
    UNIQUE (
        intent_id, cache_material_run_version, position_ordinal, code,
        run_version, final_sha256
    ),
    FOREIGN KEY (
        intent_id, cache_material_run_version, position_ordinal,
        occurrence_run_version
    ) REFERENCES chain_post_close_position_concept_rpc_occurrences(
        intent_id, cache_material_run_version, position_ordinal, run_version
    ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id, cache_material_run_version, position_ordinal,
        terminal_attempt_ordinal, terminal_result_run_version,
        terminal_result_sha256
    ) REFERENCES chain_post_close_position_concept_rpc_attempt_results(
        intent_id, cache_material_run_version, position_ordinal,
        attempt_ordinal, run_version, result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id, cache_material_run_version, position_ordinal,
        error_material_run_version, error_material_sha256
    ) REFERENCES chain_post_close_position_concept_rpc_error_materials(
        intent_id, cache_material_run_version, position_ordinal,
        run_version, material_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (audit_id) REFERENCES data_acquisition_audit(id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (audit_id) REFERENCES data_acquisition_audit_chain(acquisition_audit_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_position_concept_cache_writes (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    cache_material_run_version INTEGER NOT NULL CHECK (cache_material_run_version >= 1),
    position_ordinal INTEGER NOT NULL CHECK (position_ordinal >= 0),
    code TEXT NOT NULL CHECK (length(code) BETWEEN 1 AND 512),
    final_run_version INTEGER NOT NULL CHECK (final_run_version >= 1),
    final_sha256 TEXT NOT NULL CHECK (
        length(final_sha256) = 64 AND final_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    terminal_result_run_version INTEGER NOT NULL CHECK (terminal_result_run_version >= 1),
    concepts_codec_version INTEGER NOT NULL CHECK (concepts_codec_version = 1),
    concepts_bytes BLOB NOT NULL CHECK (
        typeof(concepts_bytes) = 'blob' AND length(concepts_bytes) > 0
    ),
    concepts_length INTEGER NOT NULL CHECK (concepts_length = length(concepts_bytes)),
    concepts_sha256 TEXT NOT NULL CHECK (
        length(concepts_sha256) = 64
        AND concepts_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    cache_updated_at TEXT NOT NULL CHECK (length(cache_updated_at) = 19),
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
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version >= final_run_version),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    written_at INTEGER NOT NULL CHECK (written_at >= 0),
    PRIMARY KEY (intent_id, cache_material_run_version, position_ordinal),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (
        intent_id, cache_material_run_version, position_ordinal, code,
        final_run_version, final_sha256
    ) REFERENCES chain_post_close_position_concept_rpc_finals(
        intent_id, cache_material_run_version, position_ordinal, code,
        run_version, final_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TRIGGER chain_post_close_position_concept_rpc_occurrences_guard
BEFORE INSERT ON chain_post_close_position_concept_rpc_occurrences
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 9
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
OR NOT EXISTS (
    SELECT 1
    FROM chain_post_close_position_concept_materials AS cache_material
    JOIN chain_post_close_position_materials AS positions
      ON positions.intent_id = cache_material.intent_id
     AND positions.run_version = cache_material.positions_run_version
     AND positions.material_sha256 = cache_material.positions_sha256
    WHERE cache_material.intent_id = NEW.intent_id
      AND cache_material.run_id = NEW.run_id
      AND cache_material.run_context_sha256 = NEW.run_context_sha256
      AND cache_material.input_sha256 = NEW.input_sha256
      AND cache_material.run_version = NEW.cache_material_run_version
      AND cache_material.material_sha256 = NEW.cache_material_sha256
      AND cache_material.positions_run_version = NEW.positions_run_version
      AND cache_material.positions_sha256 = NEW.positions_sha256
      AND cache_material.requested_count > NEW.position_ordinal
      AND cache_material.committed_at <= NEW.planned_at
      AND cache_material.lease_generation <= NEW.lease_generation
      AND (
          cache_material.lease_generation < NEW.lease_generation
          OR cache_material.lease_owner = NEW.lease_owner
      )
      AND positions.run_id = NEW.run_id
      AND positions.run_context_sha256 = NEW.run_context_sha256
      AND positions.input_sha256 = NEW.input_sha256
      AND positions.run_version = NEW.positions_run_version
      AND positions.material_sha256 = NEW.positions_sha256
      AND positions.committed_at <= cache_material.observed_at
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
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_cache_writes
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_occurrence_invalid');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_occurrences_update
BEFORE UPDATE ON chain_post_close_position_concept_rpc_occurrences
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_occurrence_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_occurrences_delete
BEFORE DELETE ON chain_post_close_position_concept_rpc_occurrences
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_occurrence_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_attempt_begins_guard
BEFORE INSERT ON chain_post_close_position_concept_rpc_attempt_begins
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 9
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
    SELECT 1 FROM chain_post_close_position_concept_rpc_occurrences AS occurrence
    WHERE occurrence.intent_id = NEW.intent_id
      AND occurrence.cache_material_run_version = NEW.cache_material_run_version
      AND occurrence.position_ordinal = NEW.position_ordinal
      AND occurrence.run_version = NEW.occurrence_run_version
      AND occurrence.request_id = NEW.request_id
      AND occurrence.request_sha256 = NEW.request_sha256
      AND occurrence.run_id = NEW.run_id
      AND occurrence.run_context_sha256 = NEW.run_context_sha256
      AND occurrence.input_sha256 = NEW.input_sha256
      AND occurrence.run_version < NEW.run_version
      AND occurrence.planned_at <= NEW.begun_at
      AND NEW.attempt_ordinal <= occurrence.retry_max_attempts
      AND occurrence.lease_generation <= NEW.lease_generation
      AND (
          occurrence.lease_generation < NEW.lease_generation
          OR occurrence.lease_owner = NEW.lease_owner
      )
)
OR (
    NEW.attempt_ordinal = 1
    AND EXISTS (
        SELECT 1 FROM chain_post_close_position_concept_rpc_attempt_results AS result
        WHERE result.intent_id = NEW.intent_id
          AND result.cache_material_run_version = NEW.cache_material_run_version
          AND result.position_ordinal = NEW.position_ordinal
    )
)
OR (
    NEW.attempt_ordinal > 1
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_position_concept_rpc_attempt_results AS result
        WHERE result.intent_id = NEW.intent_id
          AND result.cache_material_run_version = NEW.cache_material_run_version
          AND result.position_ordinal = NEW.position_ordinal
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
    SELECT 1 FROM chain_post_close_position_concept_rpc_finals AS final
    WHERE final.intent_id = NEW.intent_id
      AND final.cache_material_run_version = NEW.cache_material_run_version
      AND final.position_ordinal = NEW.position_ordinal
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
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_cache_writes
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_attempt_begin_invalid');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_attempt_begins_update
BEFORE UPDATE ON chain_post_close_position_concept_rpc_attempt_begins
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_attempt_begin_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_attempt_begins_delete
BEFORE DELETE ON chain_post_close_position_concept_rpc_attempt_begins
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_attempt_begin_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_attempt_results_guard
BEFORE INSERT ON chain_post_close_position_concept_rpc_attempt_results
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 9
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
    SELECT 1
    FROM chain_post_close_position_concept_rpc_attempt_begins AS begun
    JOIN chain_post_close_position_concept_rpc_occurrences AS occurrence
      ON occurrence.intent_id = begun.intent_id
     AND occurrence.cache_material_run_version = begun.cache_material_run_version
     AND occurrence.position_ordinal = begun.position_ordinal
    WHERE begun.intent_id = NEW.intent_id
      AND begun.cache_material_run_version = NEW.cache_material_run_version
      AND begun.position_ordinal = NEW.position_ordinal
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
    SELECT 1 FROM chain_post_close_position_concept_rpc_attempt_begins AS later
    WHERE later.intent_id = NEW.intent_id
      AND later.cache_material_run_version = NEW.cache_material_run_version
      AND later.position_ordinal = NEW.position_ordinal
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
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_cache_writes
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_attempt_result_invalid');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_attempt_results_update
BEFORE UPDATE ON chain_post_close_position_concept_rpc_attempt_results
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_attempt_result_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_attempt_results_delete
BEFORE DELETE ON chain_post_close_position_concept_rpc_attempt_results
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_attempt_result_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_status_materials_guard
BEFORE INSERT ON chain_post_close_position_concept_rpc_status_materials
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 9
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
    SELECT 1 FROM chain_post_close_position_concept_rpc_attempt_results AS result
    WHERE result.intent_id = NEW.intent_id
      AND result.cache_material_run_version = NEW.cache_material_run_version
      AND result.position_ordinal = NEW.position_ordinal
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
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_cache_writes
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_status_material_invalid');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_status_materials_update
BEFORE UPDATE ON chain_post_close_position_concept_rpc_status_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_status_material_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_status_materials_delete
BEFORE DELETE ON chain_post_close_position_concept_rpc_status_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_status_material_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_error_materials_guard
BEFORE INSERT ON chain_post_close_position_concept_rpc_error_materials
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 9
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
    SELECT 1 FROM chain_post_close_position_concept_rpc_attempt_results AS result
    WHERE result.intent_id = NEW.intent_id
      AND result.cache_material_run_version = NEW.cache_material_run_version
      AND result.position_ordinal = NEW.position_ordinal
      AND result.attempt_ordinal = NEW.terminal_attempt_ordinal
      AND result.run_version = NEW.terminal_result_run_version
      AND result.result_sha256 = NEW.terminal_result_sha256
      AND result.request_sha256 = NEW.request_sha256
      AND result.continuation = 'Terminal'
      AND result.run_id = NEW.run_id
      AND result.run_context_sha256 = NEW.run_context_sha256
      AND result.input_sha256 = NEW.input_sha256
      AND result.lease_owner = NEW.lease_owner
      AND result.lease_generation = NEW.lease_generation
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
                  FROM chain_post_close_position_concept_rpc_status_materials AS status
                  WHERE status.intent_id = NEW.intent_id
                    AND status.cache_material_run_version = NEW.cache_material_run_version
                    AND status.position_ordinal = NEW.position_ordinal
                    AND status.attempt_ordinal = NEW.status_material_attempt_ordinal
                    AND status.run_version = NEW.status_material_run_version
                    AND status.material_sha256 = NEW.status_material_sha256
                    AND status.result_run_version = result.run_version
                    AND status.request_sha256 = NEW.request_sha256
                    AND status.run_id = NEW.run_id
                    AND status.run_context_sha256 = NEW.run_context_sha256
                    AND status.input_sha256 = NEW.input_sha256
                    AND status.lease_owner = NEW.lease_owner
                    AND status.lease_generation = NEW.lease_generation
                    AND status.run_version = NEW.prior_head_version
                    AND status.captured_at <= NEW.captured_at
              )
          )
      )
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_position_concept_rpc_attempt_begins AS later
    WHERE later.intent_id = NEW.intent_id
      AND later.cache_material_run_version = NEW.cache_material_run_version
      AND later.position_ordinal = NEW.position_ordinal
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
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_cache_writes
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_error_material_invalid');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_error_materials_update
BEFORE UPDATE ON chain_post_close_position_concept_rpc_error_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_error_material_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_error_materials_delete
BEFORE DELETE ON chain_post_close_position_concept_rpc_error_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_error_material_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_finals_guard
BEFORE INSERT ON chain_post_close_position_concept_rpc_finals
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 9
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
    FROM chain_post_close_position_concept_rpc_occurrences AS occurrence
    JOIN chain_post_close_position_concept_rpc_attempt_results AS terminal
      ON terminal.intent_id = occurrence.intent_id
     AND terminal.cache_material_run_version = occurrence.cache_material_run_version
     AND terminal.position_ordinal = occurrence.position_ordinal
    WHERE occurrence.intent_id = NEW.intent_id
      AND occurrence.cache_material_run_version = NEW.cache_material_run_version
      AND occurrence.position_ordinal = NEW.position_ordinal
      AND occurrence.code = NEW.code
      AND occurrence.run_version = NEW.occurrence_run_version
      AND occurrence.request_sha256 = NEW.occurrence_request_sha256
      AND terminal.attempt_ordinal = NEW.terminal_attempt_ordinal
      AND terminal.run_version = NEW.terminal_result_run_version
      AND terminal.result_sha256 = NEW.terminal_result_sha256
      AND terminal.request_sha256 = occurrence.request_sha256
      AND terminal.continuation = 'Terminal'
      AND terminal.run_id = NEW.run_id
      AND terminal.run_context_sha256 = NEW.run_context_sha256
      AND terminal.input_sha256 = NEW.input_sha256
      AND terminal.committed_at <= NEW.applied_at
      AND terminal.lease_generation <= NEW.lease_generation
      AND (
          terminal.lease_generation < NEW.lease_generation
          OR terminal.lease_owner = NEW.lease_owner
      )
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
                  FROM chain_post_close_position_concept_rpc_error_materials AS error
                  WHERE error.intent_id = NEW.intent_id
                    AND error.cache_material_run_version = NEW.cache_material_run_version
                    AND error.position_ordinal = NEW.position_ordinal
                    AND error.run_version = NEW.error_material_run_version
                    AND error.material_sha256 = NEW.error_material_sha256
                    AND error.terminal_attempt_ordinal = terminal.attempt_ordinal
                    AND error.terminal_result_run_version = terminal.run_version
                    AND error.terminal_result_sha256 = terminal.result_sha256
                    AND error.request_sha256 = occurrence.request_sha256
                    AND error.run_id = NEW.run_id
                    AND error.run_context_sha256 = NEW.run_context_sha256
                    AND error.input_sha256 = NEW.input_sha256
                    AND error.run_version <= NEW.prior_head_version
                    AND error.captured_at <= NEW.applied_at
                    AND error.lease_generation <= NEW.lease_generation
                    AND (
                        error.lease_generation < NEW.lease_generation
                        OR error.lease_owner = NEW.lease_owner
                    )
              )
          )
      )
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_position_concept_rpc_attempt_begins AS later
    WHERE later.intent_id = NEW.intent_id
      AND later.cache_material_run_version = NEW.cache_material_run_version
      AND later.position_ordinal = NEW.position_ordinal
      AND later.attempt_ordinal > NEW.terminal_attempt_ordinal
)
OR NOT EXISTS (
    SELECT 1
    FROM data_acquisition_audit AS audit
    JOIN data_acquisition_audit_chain AS chain
      ON chain.acquisition_audit_id = audit.id
    WHERE audit.id = NEW.audit_id
      AND audit.request_hash = (
          SELECT occurrence.acquisition_request_hash
          FROM chain_post_close_position_concept_rpc_occurrences AS occurrence
          WHERE occurrence.intent_id = NEW.intent_id
            AND occurrence.cache_material_run_version = NEW.cache_material_run_version
            AND occurrence.position_ordinal = NEW.position_ordinal
      )
      AND audit.outcome = NEW.current_outcome
      AND chain.record_hash = NEW.audit_record_hash
      AND NEW.previous_outcome IS (
          SELECT previous.outcome
          FROM data_acquisition_audit AS previous
          WHERE previous.id < audit.id
            AND previous.capability = audit.capability
            AND previous.provider = audit.provider
          ORDER BY previous.id DESC
          LIMIT 1
      )
)
OR (
    NEW.final_outcome = 'Available' AND NEW.current_outcome != 'available'
)
OR (
    NEW.final_outcome = 'VerifiedEmpty' AND NEW.current_outcome != 'verified_empty'
)
OR (
    NEW.final_outcome = 'Error'
    AND NEW.current_outcome IN ('available', 'verified_empty')
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_rpc_finals AS old_final
    WHERE old_final.audit_id = NEW.audit_id
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
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_cache_writes
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_final_invalid');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_finals_update
BEFORE UPDATE ON chain_post_close_position_concept_rpc_finals
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_final_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_rpc_finals_delete
BEFORE DELETE ON chain_post_close_position_concept_rpc_finals
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_rpc_final_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_cache_writes_guard
BEFORE INSERT ON chain_post_close_position_concept_cache_writes
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version = 9
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
      AND run.updated_at = NEW.written_at
      AND run.lease_until > NEW.written_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_position_concept_rpc_finals AS final
    WHERE final.intent_id = NEW.intent_id
      AND final.cache_material_run_version = NEW.cache_material_run_version
      AND final.position_ordinal = NEW.position_ordinal
      AND final.code = NEW.code
      AND final.run_version = NEW.final_run_version
      AND final.final_sha256 = NEW.final_sha256
      AND final.final_outcome = 'Available'
      AND final.terminal_result_run_version = NEW.terminal_result_run_version
      AND final.run_id = NEW.run_id
      AND final.run_context_sha256 = NEW.run_context_sha256
      AND final.input_sha256 = NEW.input_sha256
      AND final.run_version <= NEW.prior_head_version
      AND final.applied_at <= NEW.written_at
      AND final.lease_generation <= NEW.lease_generation
      AND (
          final.lease_generation < NEW.lease_generation
          OR final.lease_owner = NEW.lease_owner
      )
)
OR EXISTS (
    SELECT 1
    FROM chain_post_close_position_concept_rpc_occurrences AS occurrence
    LEFT JOIN chain_post_close_position_concept_rpc_finals AS final
      ON final.intent_id = occurrence.intent_id
     AND final.cache_material_run_version = occurrence.cache_material_run_version
     AND final.position_ordinal = occurrence.position_ordinal
    WHERE occurrence.intent_id = NEW.intent_id
      AND occurrence.cache_material_run_version = NEW.cache_material_run_version
      AND final.intent_id IS NULL
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_position_concept_rpc_finals AS earlier
    WHERE earlier.intent_id = NEW.intent_id
      AND earlier.cache_material_run_version = NEW.cache_material_run_version
      AND earlier.terminal_result_run_version < NEW.terminal_result_run_version
      AND (
          earlier.final_outcome != 'Available'
          OR NOT EXISTS (
              SELECT 1
              FROM chain_post_close_position_concept_cache_writes AS applied
              WHERE applied.intent_id = earlier.intent_id
                AND applied.cache_material_run_version = earlier.cache_material_run_version
                AND applied.position_ordinal = earlier.position_ordinal
                AND applied.final_run_version = earlier.run_version
                AND applied.final_sha256 = earlier.final_sha256
                AND applied.terminal_result_run_version = earlier.terminal_result_run_version
          )
      )
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_position_concept_cache_writes AS later_applied
    WHERE later_applied.intent_id = NEW.intent_id
      AND later_applied.cache_material_run_version = NEW.cache_material_run_version
      AND later_applied.terminal_result_run_version > NEW.terminal_result_run_version
)
OR NOT EXISTS (
    SELECT 1 FROM stock_concepts AS business_cache
    WHERE business_cache.code = NEW.code
      AND CAST(business_cache.concepts AS BLOB) = NEW.concepts_bytes
      AND CAST(business_cache.updated_at AS TEXT) = NEW.cache_updated_at
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
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id, run_version FROM chain_post_close_position_concept_cache_writes
    ) AS fact
    WHERE fact.intent_id = NEW.intent_id AND fact.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_cache_write_invalid');
END;

CREATE TRIGGER chain_post_close_position_concept_cache_writes_update
BEFORE UPDATE ON chain_post_close_position_concept_cache_writes
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_cache_write_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_cache_writes_delete
BEFORE DELETE ON chain_post_close_position_concept_cache_writes
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_cache_write_immutable');
END;
