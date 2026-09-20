CREATE TABLE chain_post_close_board_attempt_begins (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    kind TEXT NOT NULL CHECK (kind IN ('Industry', 'Concept')),
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal >= 1),
    request_id TEXT NOT NULL CHECK (length(request_id) BETWEEN 1 AND 512),
    request_codec_version INTEGER NOT NULL CHECK (request_codec_version = 1),
    request_bytes BLOB NOT NULL CHECK (
        typeof(request_bytes) = 'blob' AND length(request_bytes) > 0
    ),
    request_length INTEGER NOT NULL CHECK (request_length = length(request_bytes)),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    previous_result_run_version INTEGER CHECK (
        (attempt_ordinal = 1 AND previous_result_run_version IS NULL)
        OR (
            attempt_ordinal > 1
            AND previous_result_run_version IS NOT NULL
            AND previous_result_run_version >= 1
        )
    ),
    previous_result_sha256 TEXT,
    industry_final_run_version INTEGER,
    industry_final_sha256 TEXT,
    chain_daily_application_run_version INTEGER NOT NULL CHECK (
        chain_daily_application_run_version >= 1
    ),
    lifecycle_sha256 TEXT NOT NULL CHECK (
        length(lifecycle_sha256) = 64 AND lifecycle_sha256 NOT GLOB '*[^0-9a-f]*'
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
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    begun_at INTEGER NOT NULL CHECK (begun_at >= 0),
    CHECK (
        (
            kind = 'Industry'
            AND industry_final_run_version IS NULL
            AND industry_final_sha256 IS NULL
        )
        OR (
            kind = 'Concept'
            AND industry_final_run_version IS NOT NULL
            AND industry_final_run_version >= 1
            AND industry_final_sha256 IS NOT NULL
            AND length(industry_final_sha256) = 64
            AND industry_final_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        (
            attempt_ordinal = 1
            AND previous_result_sha256 IS NULL
        )
        OR (
            attempt_ordinal > 1
            AND previous_result_sha256 IS NOT NULL
            AND length(previous_result_sha256) = 64
            AND previous_result_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    ),
    PRIMARY KEY (intent_id, kind, attempt_ordinal),
    UNIQUE (intent_id, run_version),
    UNIQUE (
        intent_id, kind, attempt_ordinal, run_version,
        lease_owner, lease_generation
    ),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, chain_daily_application_run_version)
        REFERENCES chain_post_close_chain_daily_applications(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, industry_final_run_version)
        REFERENCES chain_post_close_board_kind_finals(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_board_attempt_results (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    kind TEXT NOT NULL CHECK (kind IN ('Industry', 'Concept')),
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
    PRIMARY KEY (intent_id, kind, attempt_ordinal),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (
        intent_id, kind, attempt_ordinal, begin_run_version,
        lease_owner, lease_generation
    ) REFERENCES chain_post_close_board_attempt_begins(
        intent_id, kind, attempt_ordinal, run_version,
        lease_owner, lease_generation
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_board_kind_finals (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    kind TEXT NOT NULL CHECK (kind IN ('Industry', 'Concept')),
    chain_daily_application_run_version INTEGER NOT NULL CHECK (
        chain_daily_application_run_version >= 1
    ),
    lifecycle_sha256 TEXT NOT NULL CHECK (
        length(lifecycle_sha256) = 64 AND lifecycle_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    industry_final_run_version INTEGER,
    industry_final_sha256 TEXT,
    terminal_origin TEXT NOT NULL CHECK (
        terminal_origin IN ('Attempt', 'LocalPreDispatch')
    ),
    terminal_attempt_ordinal INTEGER,
    terminal_result_run_version INTEGER,
    terminal_result_sha256 TEXT,
    attempt_count INTEGER NOT NULL CHECK (attempt_count >= 0),
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
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version >= 0),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    applied_at INTEGER NOT NULL CHECK (applied_at >= 0),
    CHECK (
        (
            kind = 'Industry'
            AND industry_final_run_version IS NULL
            AND industry_final_sha256 IS NULL
        )
        OR (
            kind = 'Concept'
            AND industry_final_run_version IS NOT NULL
            AND industry_final_run_version >= 1
            AND industry_final_sha256 IS NOT NULL
            AND length(industry_final_sha256) = 64
            AND industry_final_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        (
            terminal_origin = 'Attempt'
            AND terminal_attempt_ordinal IS NOT NULL
            AND terminal_attempt_ordinal >= 1
            AND terminal_result_run_version IS NOT NULL
            AND terminal_result_run_version >= 1
            AND terminal_result_sha256 IS NOT NULL
            AND length(terminal_result_sha256) = 64
            AND terminal_result_sha256 NOT GLOB '*[^0-9a-f]*'
            AND attempt_count = terminal_attempt_ordinal
        )
        OR (
            terminal_origin = 'LocalPreDispatch'
            AND terminal_attempt_ordinal IS NULL
            AND terminal_result_run_version IS NULL
            AND terminal_result_sha256 IS NULL
            AND final_outcome = 'Error'
        )
    ),
    PRIMARY KEY (intent_id, kind),
    UNIQUE (intent_id, run_version),
    UNIQUE (audit_id),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, chain_daily_application_run_version)
        REFERENCES chain_post_close_chain_daily_applications(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, industry_final_run_version)
        REFERENCES chain_post_close_board_kind_finals(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (audit_id) REFERENCES data_acquisition_audit(id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (audit_id)
        REFERENCES data_acquisition_audit_chain(acquisition_audit_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_board_directory_materials (
    intent_id TEXT PRIMARY KEY CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    industry_final_run_version INTEGER NOT NULL CHECK (
        industry_final_run_version >= 1
    ),
    industry_final_sha256 TEXT NOT NULL CHECK (
        length(industry_final_sha256) = 64
        AND industry_final_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    concept_final_run_version INTEGER,
    concept_final_sha256 TEXT,
    fold_outcome TEXT NOT NULL CHECK (fold_outcome IN ('Available', 'Unavailable')),
    directory_codec_version INTEGER NOT NULL CHECK (directory_codec_version = 1),
    directory_bytes BLOB NOT NULL CHECK (
        typeof(directory_bytes) = 'blob' AND length(directory_bytes) > 0
    ),
    directory_length INTEGER NOT NULL CHECK (
        directory_length = length(directory_bytes)
    ),
    directory_sha256 TEXT NOT NULL CHECK (
        length(directory_sha256) = 64
        AND directory_sha256 NOT GLOB '*[^0-9a-f]*'
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
        run_version = prior_head_version + 1
        AND industry_final_run_version < run_version
        AND (
            concept_final_run_version IS NULL
            OR concept_final_run_version < run_version
        )
    ),
    materialized_at INTEGER NOT NULL CHECK (materialized_at >= 0),
    CHECK (
        (concept_final_run_version IS NULL AND concept_final_sha256 IS NULL)
        OR (
            concept_final_run_version IS NOT NULL
            AND concept_final_run_version >= 1
            AND concept_final_sha256 IS NOT NULL
            AND length(concept_final_sha256) = 64
            AND concept_final_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    ),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, industry_final_run_version)
        REFERENCES chain_post_close_board_kind_finals(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, concept_final_run_version)
        REFERENCES chain_post_close_board_kind_finals(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_board_selections (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    cluster_ordinal INTEGER NOT NULL CHECK (
        cluster_ordinal >= 0 AND cluster_ordinal < 20
    ),
    directory_run_version INTEGER NOT NULL CHECK (directory_run_version >= 1),
    directory_sha256 TEXT NOT NULL CHECK (
        length(directory_sha256) = 64
        AND directory_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    cluster_material_run_version INTEGER NOT NULL CHECK (
        cluster_material_run_version >= 1
    ),
    cluster_material_sha256 TEXT NOT NULL CHECK (
        length(cluster_material_sha256) = 64
        AND cluster_material_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    cluster_concept TEXT NOT NULL,
    selection_outcome TEXT NOT NULL CHECK (
        selection_outcome IN ('Selected', 'NoMatch')
    ),
    selected_code TEXT,
    selection_codec_version INTEGER NOT NULL CHECK (selection_codec_version = 1),
    selection_bytes BLOB NOT NULL CHECK (
        typeof(selection_bytes) = 'blob' AND length(selection_bytes) > 0
    ),
    selection_length INTEGER NOT NULL CHECK (
        selection_length = length(selection_bytes)
    ),
    selection_sha256 TEXT NOT NULL CHECK (
        length(selection_sha256) = 64
        AND selection_sha256 NOT GLOB '*[^0-9a-f]*'
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
        run_version = prior_head_version + 1
        AND directory_run_version < run_version
        AND cluster_material_run_version < run_version
    ),
    selected_at INTEGER NOT NULL CHECK (selected_at >= 0),
    CHECK (
        (
            selection_outcome = 'Selected'
            AND selected_code IS NOT NULL
        )
        OR (selection_outcome = 'NoMatch' AND selected_code IS NULL)
    ),
    PRIMARY KEY (intent_id, cluster_ordinal),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, directory_run_version)
        REFERENCES chain_post_close_board_directory_materials(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, cluster_material_run_version)
        REFERENCES chain_post_close_cluster_materials(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TRIGGER chain_post_close_board_attempt_begins_guard
BEFORE INSERT ON chain_post_close_board_attempt_begins
WHEN NOT EXISTS (
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
    SELECT 1 FROM chain_post_close_chain_daily_applications AS application
    WHERE application.intent_id = NEW.intent_id
      AND application.run_version = NEW.chain_daily_application_run_version
      AND application.lifecycle_sha256 = NEW.lifecycle_sha256
      AND application.run_id = NEW.run_id
      AND application.run_context_sha256 = NEW.run_context_sha256
      AND application.input_sha256 = NEW.input_sha256
      AND application.run_version < NEW.run_version
      AND application.applied_at <= NEW.begun_at
      AND application.lease_generation <= NEW.lease_generation
      AND (
          application.lease_generation < NEW.lease_generation
          OR application.lease_owner = NEW.lease_owner
      )
)
OR (
    NEW.kind = 'Concept'
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_board_kind_finals AS industry
        WHERE industry.intent_id = NEW.intent_id
          AND industry.kind = 'Industry'
          AND industry.run_version = NEW.industry_final_run_version
          AND industry.final_sha256 = NEW.industry_final_sha256
          AND industry.final_outcome = 'Available'
          AND industry.run_id = NEW.run_id
          AND industry.run_context_sha256 = NEW.run_context_sha256
          AND industry.input_sha256 = NEW.input_sha256
          AND industry.run_version < NEW.run_version
          AND industry.applied_at <= NEW.begun_at
          AND industry.lease_generation <= NEW.lease_generation
          AND (
              industry.lease_generation < NEW.lease_generation
              OR industry.lease_owner = NEW.lease_owner
          )
    )
)
OR (
    NEW.attempt_ordinal > 1
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_board_attempt_results AS previous
        JOIN chain_post_close_board_attempt_begins AS previous_begin
          ON previous_begin.intent_id = previous.intent_id
         AND previous_begin.kind = previous.kind
         AND previous_begin.attempt_ordinal = previous.attempt_ordinal
        WHERE previous.intent_id = NEW.intent_id
          AND previous.kind = NEW.kind
          AND previous.attempt_ordinal = NEW.attempt_ordinal - 1
          AND previous.run_version = NEW.previous_result_run_version
          AND previous.result_sha256 = NEW.previous_result_sha256
          AND previous.continuation = 'Retry'
          AND previous.run_id = NEW.run_id
          AND previous.run_context_sha256 = NEW.run_context_sha256
          AND previous.input_sha256 = NEW.input_sha256
          AND previous.run_version < NEW.run_version
          AND previous.lease_generation <= NEW.lease_generation
          AND (
              previous.lease_generation < NEW.lease_generation
              OR previous.lease_owner = NEW.lease_owner
          )
          AND previous_begin.request_id = NEW.request_id
          AND previous.request_sha256 = NEW.request_sha256
          AND previous.committed_at <= NEW.begun_at
    )
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_board_kind_finals
    WHERE intent_id = NEW.intent_id AND kind = NEW.kind
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_board_directory_materials
    WHERE intent_id = NEW.intent_id
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_stage_results WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_selections WHERE intent_id = NEW.intent_id
    ) AS facts WHERE facts.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_attempt_begin_invalid');
END;

CREATE TRIGGER chain_post_close_board_attempt_begins_update
BEFORE UPDATE ON chain_post_close_board_attempt_begins
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_attempt_begin_immutable');
END;

CREATE TRIGGER chain_post_close_board_attempt_begins_delete
BEFORE DELETE ON chain_post_close_board_attempt_begins
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_attempt_begin_immutable');
END;

CREATE TRIGGER chain_post_close_board_attempt_results_guard
BEFORE INSERT ON chain_post_close_board_attempt_results
WHEN NOT EXISTS (
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
    SELECT 1 FROM chain_post_close_board_attempt_begins AS begun
    WHERE begun.intent_id = NEW.intent_id
      AND begun.kind = NEW.kind
      AND begun.attempt_ordinal = NEW.attempt_ordinal
      AND begun.run_version = NEW.begin_run_version
      AND begun.request_sha256 = NEW.request_sha256
      AND begun.run_id = NEW.run_id
      AND begun.run_context_sha256 = NEW.run_context_sha256
      AND begun.input_sha256 = NEW.input_sha256
      AND begun.lease_owner = NEW.lease_owner
      AND begun.lease_generation = NEW.lease_generation
      AND begun.begun_at <= NEW.returned_at
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_stage_results WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_selections WHERE intent_id = NEW.intent_id
    ) AS facts WHERE facts.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_attempt_result_invalid');
END;

CREATE TRIGGER chain_post_close_board_attempt_results_update
BEFORE UPDATE ON chain_post_close_board_attempt_results
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_attempt_result_immutable');
END;

CREATE TRIGGER chain_post_close_board_attempt_results_delete
BEFORE DELETE ON chain_post_close_board_attempt_results
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_attempt_result_immutable');
END;

CREATE TRIGGER chain_post_close_board_kind_finals_guard
BEFORE INSERT ON chain_post_close_board_kind_finals
WHEN NOT EXISTS (
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
    SELECT 1 FROM chain_post_close_chain_daily_applications AS application
    WHERE application.intent_id = NEW.intent_id
      AND application.run_version = NEW.chain_daily_application_run_version
      AND application.lifecycle_sha256 = NEW.lifecycle_sha256
      AND application.run_id = NEW.run_id
      AND application.run_context_sha256 = NEW.run_context_sha256
      AND application.input_sha256 = NEW.input_sha256
      AND application.run_version < NEW.run_version
      AND application.applied_at <= NEW.applied_at
      AND application.lease_generation <= NEW.lease_generation
      AND (
          application.lease_generation < NEW.lease_generation
          OR application.lease_owner = NEW.lease_owner
      )
)
OR (
    NEW.kind = 'Concept'
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_board_kind_finals AS industry
        WHERE industry.intent_id = NEW.intent_id
          AND industry.kind = 'Industry'
          AND industry.run_version = NEW.industry_final_run_version
          AND industry.final_sha256 = NEW.industry_final_sha256
          AND industry.final_outcome = 'Available'
          AND industry.run_id = NEW.run_id
          AND industry.run_context_sha256 = NEW.run_context_sha256
          AND industry.input_sha256 = NEW.input_sha256
          AND industry.run_version < NEW.run_version
          AND industry.applied_at <= NEW.applied_at
          AND industry.lease_generation <= NEW.lease_generation
          AND (
              industry.lease_generation < NEW.lease_generation
              OR industry.lease_owner = NEW.lease_owner
          )
    )
)
OR (
    NEW.terminal_origin = 'Attempt'
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_board_attempt_results AS terminal
        WHERE terminal.intent_id = NEW.intent_id
          AND terminal.kind = NEW.kind
          AND terminal.attempt_ordinal = NEW.terminal_attempt_ordinal
          AND terminal.run_version = NEW.terminal_result_run_version
          AND terminal.result_sha256 = NEW.terminal_result_sha256
          AND terminal.continuation = 'Terminal'
          AND terminal.run_id = NEW.run_id
          AND terminal.run_context_sha256 = NEW.run_context_sha256
          AND terminal.input_sha256 = NEW.input_sha256
          AND terminal.run_version < NEW.run_version
          AND terminal.committed_at <= NEW.applied_at
          AND terminal.lease_generation <= NEW.lease_generation
          AND (
              terminal.lease_generation < NEW.lease_generation
              OR terminal.lease_owner = NEW.lease_owner
          )
    )
)
OR (
    SELECT COUNT(*) FROM chain_post_close_board_attempt_begins
    WHERE intent_id = NEW.intent_id AND kind = NEW.kind
) != NEW.attempt_count
OR (
    SELECT COUNT(*) FROM chain_post_close_board_attempt_results
    WHERE intent_id = NEW.intent_id AND kind = NEW.kind
) != NEW.attempt_count
OR COALESCE((
    SELECT MAX(attempt_ordinal) FROM chain_post_close_board_attempt_begins
    WHERE intent_id = NEW.intent_id AND kind = NEW.kind
), 0) != NEW.attempt_count
OR COALESCE((
    SELECT MAX(attempt_ordinal) FROM chain_post_close_board_attempt_results
    WHERE intent_id = NEW.intent_id AND kind = NEW.kind
), 0) != NEW.attempt_count
OR EXISTS (
    SELECT 1 FROM chain_post_close_board_attempt_results
    WHERE intent_id = NEW.intent_id
      AND kind = NEW.kind
      AND attempt_ordinal < NEW.attempt_count
      AND continuation != 'Retry'
)
OR (
    NEW.terminal_origin = 'LocalPreDispatch'
    AND EXISTS (
        SELECT 1 FROM chain_post_close_board_attempt_results
        WHERE intent_id = NEW.intent_id
          AND kind = NEW.kind
          AND continuation != 'Retry'
    )
)
OR NOT EXISTS (
    SELECT 1 FROM data_acquisition_audit AS audit
    JOIN data_acquisition_audit_chain AS chain
      ON chain.acquisition_audit_id = audit.id
    WHERE audit.id = NEW.audit_id
      AND audit.capability = 'board-directory'
      AND audit.request_hash = CASE NEW.kind
          WHEN 'Industry' THEN 'a60e8f768348b8c1cffcc9127857b61f109c7e5fc60f55fbddda98169d1d420f'
          WHEN 'Concept' THEN 'fe96ce4cfcabd4b4ca6fd5589792b7b1cb990ef3f445bdf688c6993950b0f6e5'
      END
      AND audit.outcome = NEW.current_outcome
      AND (
          (NEW.final_outcome = 'Available' AND audit.outcome = 'available')
          OR (
              NEW.final_outcome = 'VerifiedEmpty'
              AND audit.outcome = 'verified_empty'
          )
          OR (
              NEW.final_outcome = 'Error'
              AND audit.outcome NOT IN ('available', 'verified_empty')
          )
      )
      AND NEW.previous_outcome IS (
          SELECT previous.outcome
          FROM data_acquisition_audit AS previous
          WHERE previous.id < audit.id
            AND previous.capability = audit.capability
            AND previous.provider = audit.provider
          ORDER BY previous.id DESC
          LIMIT 1
      )
      AND chain.record_hash = NEW.audit_record_hash
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_stage_results WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_selections WHERE intent_id = NEW.intent_id
    ) AS facts WHERE facts.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_kind_final_invalid');
END;

CREATE TRIGGER chain_post_close_board_kind_finals_update
BEFORE UPDATE ON chain_post_close_board_kind_finals
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_kind_final_immutable');
END;

CREATE TRIGGER chain_post_close_board_kind_finals_delete
BEFORE DELETE ON chain_post_close_board_kind_finals
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_kind_final_immutable');
END;

CREATE TRIGGER chain_post_close_board_directory_materials_guard
BEFORE INSERT ON chain_post_close_board_directory_materials
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.run_id = NEW.run_id
      AND run.run_context_sha256 = NEW.run_context_sha256
      AND run.input_sha256 = NEW.input_sha256
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.updated_at = NEW.materialized_at
      AND run.lease_until > NEW.materialized_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_board_kind_finals AS industry
    WHERE industry.intent_id = NEW.intent_id
      AND industry.kind = 'Industry'
      AND industry.run_version = NEW.industry_final_run_version
      AND industry.final_sha256 = NEW.industry_final_sha256
      AND industry.run_id = NEW.run_id
      AND industry.run_context_sha256 = NEW.run_context_sha256
      AND industry.input_sha256 = NEW.input_sha256
      AND industry.run_version < NEW.run_version
      AND industry.applied_at <= NEW.materialized_at
      AND industry.lease_generation <= NEW.lease_generation
      AND (
          industry.lease_generation < NEW.lease_generation
          OR industry.lease_owner = NEW.lease_owner
      )
)
OR (
    NEW.concept_final_run_version IS NOT NULL
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_board_kind_finals AS concept
        WHERE concept.intent_id = NEW.intent_id
          AND concept.kind = 'Concept'
          AND concept.run_version = NEW.concept_final_run_version
          AND concept.final_sha256 = NEW.concept_final_sha256
          AND concept.run_id = NEW.run_id
          AND concept.run_context_sha256 = NEW.run_context_sha256
          AND concept.input_sha256 = NEW.input_sha256
          AND concept.run_version < NEW.run_version
          AND concept.applied_at <= NEW.materialized_at
          AND concept.lease_generation <= NEW.lease_generation
          AND (
              concept.lease_generation < NEW.lease_generation
              OR concept.lease_owner = NEW.lease_owner
          )
    )
)
OR (
    NEW.concept_final_run_version IS NOT NULL
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_board_kind_finals AS industry
        WHERE industry.intent_id = NEW.intent_id
          AND industry.kind = 'Industry'
          AND industry.run_version = NEW.industry_final_run_version
          AND industry.final_outcome = 'Available'
    )
)
OR (
    NEW.fold_outcome = 'Available'
    AND (
        NEW.concept_final_run_version IS NULL
        OR NOT EXISTS (
            SELECT 1 FROM chain_post_close_board_kind_finals AS industry
            JOIN chain_post_close_board_kind_finals AS concept
              ON concept.intent_id = industry.intent_id
            WHERE industry.intent_id = NEW.intent_id
              AND industry.kind = 'Industry'
              AND concept.kind = 'Concept'
              AND industry.run_version = NEW.industry_final_run_version
              AND concept.run_version = NEW.concept_final_run_version
              AND industry.final_outcome = 'Available'
              AND concept.final_outcome = 'Available'
        )
    )
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_stage_results WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_selections WHERE intent_id = NEW.intent_id
    ) AS facts WHERE facts.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_directory_material_invalid');
END;

CREATE TRIGGER chain_post_close_board_directory_materials_update
BEFORE UPDATE ON chain_post_close_board_directory_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_directory_material_immutable');
END;

CREATE TRIGGER chain_post_close_board_directory_materials_delete
BEFORE DELETE ON chain_post_close_board_directory_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_directory_material_immutable');
END;

CREATE TRIGGER chain_post_close_board_selections_guard
BEFORE INSERT ON chain_post_close_board_selections
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.run_id = NEW.run_id
      AND run.run_context_sha256 = NEW.run_context_sha256
      AND run.input_sha256 = NEW.input_sha256
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.updated_at = NEW.selected_at
      AND run.lease_until > NEW.selected_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_board_directory_materials AS directory
    WHERE directory.intent_id = NEW.intent_id
      AND directory.run_version = NEW.directory_run_version
      AND directory.directory_sha256 = NEW.directory_sha256
      AND directory.fold_outcome = 'Available'
      AND directory.run_id = NEW.run_id
      AND directory.run_context_sha256 = NEW.run_context_sha256
      AND directory.input_sha256 = NEW.input_sha256
      AND directory.run_version < NEW.run_version
      AND directory.materialized_at <= NEW.selected_at
      AND directory.lease_generation <= NEW.lease_generation
      AND (
          directory.lease_generation < NEW.lease_generation
          OR directory.lease_owner = NEW.lease_owner
      )
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_cluster_materials AS material
    WHERE material.intent_id = NEW.intent_id
      AND material.run_version = NEW.cluster_material_run_version
      AND material.material_sha256 = NEW.cluster_material_sha256
      AND material.run_id = NEW.run_id
      AND material.run_context_sha256 = NEW.run_context_sha256
      AND material.input_sha256 = NEW.input_sha256
      AND material.run_version < NEW.run_version
      AND material.materialized_at <= NEW.selected_at
      AND material.lease_generation <= NEW.lease_generation
      AND (
          material.lease_generation < NEW.lease_generation
          OR material.lease_owner = NEW.lease_owner
      )
)
OR (
    NEW.cluster_ordinal > 0
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_board_selections AS previous
        WHERE previous.intent_id = NEW.intent_id
          AND previous.cluster_ordinal = NEW.cluster_ordinal - 1
          AND previous.directory_run_version = NEW.directory_run_version
          AND previous.directory_sha256 = NEW.directory_sha256
          AND previous.cluster_material_run_version = NEW.cluster_material_run_version
          AND previous.cluster_material_sha256 = NEW.cluster_material_sha256
          AND previous.run_id = NEW.run_id
          AND previous.run_context_sha256 = NEW.run_context_sha256
          AND previous.input_sha256 = NEW.input_sha256
          AND previous.run_version < NEW.run_version
          AND previous.selected_at <= NEW.selected_at
          AND previous.lease_generation <= NEW.lease_generation
          AND (
              previous.lease_generation < NEW.lease_generation
              OR previous.lease_owner = NEW.lease_owner
          )
    )
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_stage_results WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id = NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id = NEW.intent_id
    ) AS facts WHERE facts.run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_selection_invalid');
END;

CREATE TRIGGER chain_post_close_board_selections_update
BEFORE UPDATE ON chain_post_close_board_selections
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_selection_immutable');
END;

CREATE TRIGGER chain_post_close_board_selections_delete
BEFORE DELETE ON chain_post_close_board_selections
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_selection_immutable');
END;
