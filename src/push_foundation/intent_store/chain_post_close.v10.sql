-- Controlled chain-post-close layout v10 bundle; installed only by the attested v9-to-v10 migration.
-- This revision expands every INSERT guard and the complete v1-v10 run-version collision
-- UNION for controller review. Rust must independently validate the same decoded codecs,
-- parent cardinalities, terminal material and audit receipt before and after each write.

CREATE TABLE chain_post_close_dragon_tiger_occurrences (
    intent_id TEXT PRIMARY KEY CHECK (
        length(intent_id)=64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    parent_kind TEXT NOT NULL CHECK (
        parent_kind IN ('EmptyPositions','AllCached','FetchedAndCached')
    ),
    positions_run_version INTEGER NOT NULL CHECK (positions_run_version>=1),
    positions_sha256 TEXT NOT NULL CHECK (
        length(positions_sha256)=64 AND positions_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    position_concept_run_version INTEGER,
    position_concept_sha256 TEXT,
    parent_completion_run_version INTEGER NOT NULL CHECK (parent_completion_run_version>=1),
    parent_codec_version INTEGER NOT NULL CHECK (parent_codec_version=1),
    parent_bytes BLOB NOT NULL CHECK (typeof(parent_bytes)='blob' AND length(parent_bytes)>0),
    parent_length INTEGER NOT NULL CHECK (parent_length=length(parent_bytes)),
    parent_sha256 TEXT NOT NULL CHECK (
        length(parent_sha256)=64 AND parent_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    request_observed_at TEXT NOT NULL CHECK (length(request_observed_at) BETWEEN 20 AND 64),
    request_local_offset_seconds INTEGER NOT NULL CHECK (
        request_local_offset_seconds BETWEEN -86400 AND 86400
    ),
    request_date TEXT NOT NULL CHECK (
        length(request_date)=10 AND request_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
    ),
    operation TEXT NOT NULL CHECK (operation='DragonTiger'),
    disclosure_limit INTEGER NOT NULL CHECK (disclosure_limit=100),
    stock_limit INTEGER NOT NULL CHECK (stock_limit=5000),
    request_id TEXT NOT NULL CHECK (length(request_id) BETWEEN 1 AND 512),
    request_codec_version INTEGER NOT NULL CHECK (request_codec_version=1),
    request_bytes BLOB NOT NULL CHECK (typeof(request_bytes)='blob' AND length(request_bytes)>0),
    request_length INTEGER NOT NULL CHECK (request_length=length(request_bytes)),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256)=64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    acquisition_request_hash TEXT NOT NULL CHECK (
        length(acquisition_request_hash)=64
        AND acquisition_request_hash NOT GLOB '*[^0-9a-f]*'
    ),
    profile TEXT NOT NULL CHECK (profile IN ('LocalBridgeV1','ExternalV1')),
    acquisition_authority TEXT CHECK (
        acquisition_authority IS NULL OR length(acquisition_authority) BETWEEN 1 AND 512
    ),
    retry_max_attempts INTEGER NOT NULL CHECK (retry_max_attempts>=1),
    retry_base_delay_ms INTEGER NOT NULL CHECK (retry_base_delay_ms>=0),
    retry_max_delay_ms INTEGER NOT NULL CHECK (retry_max_delay_ms>=0),
    retry_jitter_ms INTEGER NOT NULL CHECK (retry_jitter_ms>=0),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (
        length(run_context_sha256)=64 AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    input_sha256 TEXT NOT NULL CHECK (
        length(input_sha256)=64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation>=1),
    prior_head_version INTEGER NOT NULL CHECK (
        prior_head_version>=parent_completion_run_version
    ),
    run_version INTEGER NOT NULL CHECK (run_version=prior_head_version+1),
    planned_at INTEGER NOT NULL CHECK (planned_at>=0),
    CHECK (
        (parent_kind='EmptyPositions'
         AND position_concept_run_version IS NULL AND position_concept_sha256 IS NULL
         AND parent_completion_run_version=positions_run_version)
        OR
        (parent_kind IN ('AllCached','FetchedAndCached')
         AND position_concept_run_version IS NOT NULL AND position_concept_run_version>=1
         AND position_concept_sha256 IS NOT NULL AND length(position_concept_sha256)=64
         AND position_concept_sha256 NOT GLOB '*[^0-9a-f]*'
         AND parent_completion_run_version>=position_concept_run_version)
    ),
    UNIQUE (intent_id,run_version),
    UNIQUE (intent_id,run_version,request_sha256),
    UNIQUE (intent_id,request_sha256),
    UNIQUE (intent_id,parent_sha256),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id,positions_run_version,positions_sha256)
        REFERENCES chain_post_close_position_materials(intent_id,run_version,material_sha256)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id,position_concept_run_version)
        REFERENCES chain_post_close_position_concept_materials(intent_id,run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_dragon_tiger_attempt_begins (
    intent_id TEXT NOT NULL,
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal>=1),
    occurrence_run_version INTEGER NOT NULL CHECK (occurrence_run_version>=1),
    request_id TEXT NOT NULL CHECK (length(request_id) BETWEEN 1 AND 512),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256)=64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    previous_attempt_ordinal INTEGER,
    previous_result_run_version INTEGER,
    previous_result_sha256 TEXT,
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (length(run_context_sha256)=64),
    input_sha256 TEXT NOT NULL CHECK (length(input_sha256)=64),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation>=1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version>=occurrence_run_version),
    run_version INTEGER NOT NULL CHECK (run_version=prior_head_version+1),
    begun_at INTEGER NOT NULL CHECK (begun_at>=0),
    CHECK (
        (attempt_ordinal=1 AND previous_attempt_ordinal IS NULL
         AND previous_result_run_version IS NULL AND previous_result_sha256 IS NULL)
        OR
        (attempt_ordinal>1 AND previous_attempt_ordinal=attempt_ordinal-1
         AND previous_result_run_version IS NOT NULL
         AND previous_result_sha256 IS NOT NULL
         AND length(previous_result_sha256)=64
         AND previous_result_sha256 NOT GLOB '*[^0-9a-f]*')
    ),
    PRIMARY KEY (intent_id,attempt_ordinal),
    UNIQUE (intent_id,run_version),
    UNIQUE (intent_id,attempt_ordinal,run_version,request_sha256,lease_owner,lease_generation),
    FOREIGN KEY (intent_id,occurrence_run_version,request_sha256)
        REFERENCES chain_post_close_dragon_tiger_occurrences(
            intent_id,run_version,request_sha256
        ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id,previous_attempt_ordinal,previous_result_run_version,previous_result_sha256
    ) REFERENCES chain_post_close_dragon_tiger_attempt_results(
        intent_id,attempt_ordinal,run_version,result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_dragon_tiger_attempt_results (
    intent_id TEXT NOT NULL,
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal>=1),
    begin_run_version INTEGER NOT NULL CHECK (begin_run_version>=1),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256)=64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    wire_outcome TEXT NOT NULL CHECK (wire_outcome IN ('Response','Status')),
    result_codec_version INTEGER NOT NULL CHECK (result_codec_version=1),
    result_bytes BLOB NOT NULL CHECK (typeof(result_bytes)='blob' AND length(result_bytes)>0),
    result_length INTEGER NOT NULL CHECK (result_length=length(result_bytes)),
    result_sha256 TEXT NOT NULL CHECK (
        length(result_sha256)=64 AND result_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    continuation TEXT NOT NULL CHECK (continuation IN ('Retry','Terminal')),
    retry_decision TEXT NOT NULL CHECK (
        retry_decision IN ('RetryBackoff','RetryBounded','NoRetry')
    ),
    backoff_ms INTEGER,
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (length(run_context_sha256)=64),
    input_sha256 TEXT NOT NULL CHECK (length(input_sha256)=64),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation>=1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version>=begin_run_version),
    run_version INTEGER NOT NULL CHECK (run_version=prior_head_version+1),
    returned_at INTEGER NOT NULL CHECK (returned_at>=0),
    committed_at INTEGER NOT NULL CHECK (committed_at>=returned_at),
    CHECK (
        (wire_outcome='Status' AND continuation='Retry'
         AND retry_decision IN ('RetryBackoff','RetryBounded')
         AND backoff_ms IS NOT NULL AND backoff_ms>=0)
        OR
        (continuation='Terminal' AND backoff_ms IS NULL)
    ),
    PRIMARY KEY (intent_id,attempt_ordinal),
    UNIQUE (intent_id,run_version),
    UNIQUE (intent_id,attempt_ordinal,run_version,result_sha256),
    FOREIGN KEY (
        intent_id,attempt_ordinal,begin_run_version,request_sha256,lease_owner,lease_generation
    ) REFERENCES chain_post_close_dragon_tiger_attempt_begins(
        intent_id,attempt_ordinal,run_version,request_sha256,lease_owner,lease_generation
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_dragon_tiger_status_materials (
    intent_id TEXT NOT NULL,
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal>=1),
    result_run_version INTEGER NOT NULL CHECK (result_run_version>=1),
    result_sha256 TEXT NOT NULL CHECK (length(result_sha256)=64),
    request_sha256 TEXT NOT NULL CHECK (length(request_sha256)=64),
    provenance TEXT NOT NULL CHECK (provenance='Captured'),
    projection_version INTEGER NOT NULL CHECK (projection_version=1),
    material_codec_version INTEGER NOT NULL CHECK (material_codec_version=1),
    material_bytes BLOB NOT NULL CHECK (typeof(material_bytes)='blob' AND length(material_bytes)>0),
    material_length INTEGER NOT NULL CHECK (material_length=length(material_bytes)),
    material_sha256 TEXT NOT NULL CHECK (length(material_sha256)=64),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (length(run_context_sha256)=64),
    input_sha256 TEXT NOT NULL CHECK (length(input_sha256)=64),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation>=1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version>=result_run_version),
    run_version INTEGER NOT NULL CHECK (run_version=prior_head_version+1),
    captured_at INTEGER NOT NULL CHECK (captured_at>=0),
    PRIMARY KEY (intent_id,attempt_ordinal),
    UNIQUE (intent_id,run_version),
    UNIQUE (intent_id,attempt_ordinal,run_version,material_sha256),
    FOREIGN KEY (intent_id,attempt_ordinal,result_run_version,result_sha256)
        REFERENCES chain_post_close_dragon_tiger_attempt_results(
            intent_id,attempt_ordinal,run_version,result_sha256
        ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_dragon_tiger_error_materials (
    intent_id TEXT PRIMARY KEY,
    terminal_attempt_ordinal INTEGER NOT NULL CHECK (terminal_attempt_ordinal>=1),
    terminal_result_run_version INTEGER NOT NULL CHECK (terminal_result_run_version>=1),
    terminal_result_sha256 TEXT NOT NULL CHECK (length(terminal_result_sha256)=64),
    request_sha256 TEXT NOT NULL CHECK (length(request_sha256)=64),
    status_material_attempt_ordinal INTEGER,
    status_material_run_version INTEGER,
    status_material_sha256 TEXT,
    material_codec_version INTEGER NOT NULL CHECK (material_codec_version=1),
    material_bytes BLOB NOT NULL CHECK (typeof(material_bytes)='blob' AND length(material_bytes)>0),
    material_length INTEGER NOT NULL CHECK (material_length=length(material_bytes)),
    material_sha256 TEXT NOT NULL CHECK (length(material_sha256)=64),
    observed_fallback TEXT NOT NULL CHECK (length(observed_fallback) BETWEEN 20 AND 64),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (length(run_context_sha256)=64),
    input_sha256 TEXT NOT NULL CHECK (length(input_sha256)=64),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation>=1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version>=terminal_result_run_version),
    run_version INTEGER NOT NULL CHECK (run_version=prior_head_version+1),
    captured_at INTEGER NOT NULL CHECK (captured_at>=0),
    CHECK (
        (status_material_attempt_ordinal IS NULL AND status_material_run_version IS NULL
         AND status_material_sha256 IS NULL)
        OR
        (status_material_attempt_ordinal=terminal_attempt_ordinal
         AND status_material_run_version IS NOT NULL
         AND status_material_sha256 IS NOT NULL AND length(status_material_sha256)=64)
    ),
    UNIQUE (intent_id,run_version),
    UNIQUE (intent_id,run_version,material_sha256),
    FOREIGN KEY (
        intent_id,terminal_attempt_ordinal,terminal_result_run_version,terminal_result_sha256
    ) REFERENCES chain_post_close_dragon_tiger_attempt_results(
        intent_id,attempt_ordinal,run_version,result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id,status_material_attempt_ordinal,status_material_run_version,status_material_sha256
    ) REFERENCES chain_post_close_dragon_tiger_status_materials(
        intent_id,attempt_ordinal,run_version,material_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

-- One final envelope independently contains (a) complete GatewayBatch or GatewayError and
-- (b) ProjectionOutcome::{Available,Failed} with the exact lhb map + SourceObservation or the
-- original projection failure. Gateway success + projection failure remains a successful batch
-- and R-04 audit; it is never rewritten as GatewayError/Unavailable. The codec cross-validates
-- both halves against the original terminal wire.
CREATE TABLE chain_post_close_dragon_tiger_finals (
    intent_id TEXT PRIMARY KEY,
    occurrence_run_version INTEGER NOT NULL CHECK (occurrence_run_version>=1),
    occurrence_request_sha256 TEXT NOT NULL CHECK (length(occurrence_request_sha256)=64),
    terminal_attempt_ordinal INTEGER NOT NULL CHECK (terminal_attempt_ordinal>=1),
    terminal_result_run_version INTEGER NOT NULL CHECK (terminal_result_run_version>=1),
    terminal_result_sha256 TEXT NOT NULL CHECK (length(terminal_result_sha256)=64),
    error_material_run_version INTEGER,
    error_material_sha256 TEXT,
    final_outcome TEXT NOT NULL CHECK (final_outcome IN ('Available','VerifiedEmpty','Error')),
    final_codec_version INTEGER NOT NULL CHECK (final_codec_version=1),
    final_bytes BLOB NOT NULL CHECK (typeof(final_bytes)='blob' AND length(final_bytes)>0),
    final_length INTEGER NOT NULL CHECK (final_length=length(final_bytes)),
    final_sha256 TEXT NOT NULL CHECK (length(final_sha256)=64),
    audit_id INTEGER NOT NULL CHECK (audit_id>=1),
    audit_record_hash TEXT NOT NULL CHECK (length(audit_record_hash)=64),
    previous_outcome TEXT,
    current_outcome TEXT NOT NULL CHECK (current_outcome IN (
        'available','verified_empty','invalid_request','unavailable',
        'stale','partial','conflict','unsupported'
    )),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (length(run_context_sha256)=64),
    input_sha256 TEXT NOT NULL CHECK (length(input_sha256)=64),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation>=1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version>=terminal_result_run_version),
    run_version INTEGER NOT NULL CHECK (run_version=prior_head_version+1),
    applied_at INTEGER NOT NULL CHECK (applied_at>=0),
    CHECK (
        (final_outcome IN ('Available','VerifiedEmpty')
         AND error_material_run_version IS NULL AND error_material_sha256 IS NULL)
        OR
        (final_outcome='Error' AND error_material_run_version IS NOT NULL
         AND error_material_sha256 IS NOT NULL AND length(error_material_sha256)=64)
    ),
    UNIQUE (intent_id,run_version),
    UNIQUE (audit_id),
    UNIQUE (intent_id,run_version,final_sha256),
    FOREIGN KEY (intent_id,occurrence_run_version,occurrence_request_sha256)
        REFERENCES chain_post_close_dragon_tiger_occurrences(
            intent_id,run_version,request_sha256
        ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (
        intent_id,terminal_attempt_ordinal,terminal_result_run_version,terminal_result_sha256
    ) REFERENCES chain_post_close_dragon_tiger_attempt_results(
        intent_id,attempt_ordinal,run_version,result_sha256
    ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id,error_material_run_version,error_material_sha256)
        REFERENCES chain_post_close_dragon_tiger_error_materials(
            intent_id,run_version,material_sha256
        ) ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (audit_id) REFERENCES data_acquisition_audit(id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (audit_id) REFERENCES data_acquisition_audit_chain(acquisition_audit_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

-- The six guards below admit no row unless the run update, historical parent and global
-- run-version namespace agree. A run with all six families empty remains legal: these predicates
-- execute only when a DragonTiger fact is inserted. Codec decoding and exact parent byte/hash
-- recomputation stay mandatory in Rust because SQLite cannot interpret the canonical envelopes.
-- In particular, v8 cache_row_count counts every fresh source row and can include codes outside
-- requested_codes; it is never used as work cardinality. Rust derives work from decoded requested
-- codes minus parsed cache keys, while SQL only enforces no work for AllCached or nonempty,
-- fully-finalized-and-written work for FetchedAndCached.

CREATE TRIGGER chain_post_close_dragon_tiger_occurrences_guard
BEFORE INSERT ON chain_post_close_dragon_tiger_occurrences
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_layouts WHERE layout_version=10
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id=NEW.intent_id
      AND run.run_id=NEW.run_id
      AND run.run_context_sha256=NEW.run_context_sha256
      AND run.input_sha256=NEW.input_sha256
      AND run.lease_owner=NEW.lease_owner
      AND run.lease_generation=NEW.lease_generation
      AND run.head_version=NEW.run_version
      AND run.updated_at=NEW.planned_at
      AND run.lease_until>NEW.planned_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_position_materials AS positions
    WHERE positions.intent_id=NEW.intent_id
      AND positions.run_id=NEW.run_id
      AND positions.run_context_sha256=NEW.run_context_sha256
      AND positions.input_sha256=NEW.input_sha256
      AND positions.run_version=NEW.positions_run_version
      AND positions.material_sha256=NEW.positions_sha256
      AND positions.committed_at<=NEW.planned_at
      AND positions.lease_generation<=NEW.lease_generation
      AND (positions.lease_generation<NEW.lease_generation
           OR positions.lease_owner=NEW.lease_owner)
      AND (
          (NEW.parent_kind='EmptyPositions'
           AND positions.row_count=0
           AND NEW.position_concept_run_version IS NULL
           AND NEW.position_concept_sha256 IS NULL
           AND NEW.parent_completion_run_version=positions.run_version
           AND NOT EXISTS (
               SELECT 1 FROM chain_post_close_position_concept_materials AS unexpected
               WHERE unexpected.intent_id=NEW.intent_id
           ))
          OR
          (NEW.parent_kind='AllCached'
           AND positions.row_count>0
           AND EXISTS (
               SELECT 1 FROM chain_post_close_position_concept_materials AS material
               WHERE material.intent_id=NEW.intent_id
                 AND material.run_id=NEW.run_id
                 AND material.run_context_sha256=NEW.run_context_sha256
                 AND material.input_sha256=NEW.input_sha256
                 AND material.run_version=NEW.position_concept_run_version
                 AND material.material_sha256=NEW.position_concept_sha256
                 AND material.positions_run_version=positions.run_version
                 AND material.positions_sha256=positions.material_sha256
                 AND material.requested_count=positions.row_count
                 AND material.committed_at<=NEW.planned_at
                 AND NEW.parent_completion_run_version=material.run_version
                 AND material.lease_generation<=NEW.lease_generation
                 AND (material.lease_generation<NEW.lease_generation
                      OR material.lease_owner=NEW.lease_owner)
                 AND NOT EXISTS (
                     SELECT 1 FROM chain_post_close_position_concept_rpc_occurrences AS work
                     WHERE work.intent_id=NEW.intent_id
                       AND work.cache_material_run_version=material.run_version
                 )
           ))
          OR
          (NEW.parent_kind='FetchedAndCached'
           AND positions.row_count>0
           AND EXISTS (
               SELECT 1 FROM chain_post_close_position_concept_materials AS material
               WHERE material.intent_id=NEW.intent_id
                 AND material.run_id=NEW.run_id
                 AND material.run_context_sha256=NEW.run_context_sha256
                 AND material.input_sha256=NEW.input_sha256
                 AND material.run_version=NEW.position_concept_run_version
                 AND material.material_sha256=NEW.position_concept_sha256
                 AND material.positions_run_version=positions.run_version
                 AND material.positions_sha256=positions.material_sha256
                 AND material.requested_count=positions.row_count
                 AND material.committed_at<=NEW.planned_at
                 AND material.lease_generation<=NEW.lease_generation
                 AND (material.lease_generation<NEW.lease_generation
                      OR material.lease_owner=NEW.lease_owner)
                 AND EXISTS (
                     SELECT 1 FROM chain_post_close_position_concept_rpc_occurrences AS work
                     WHERE work.intent_id=NEW.intent_id
                       AND work.cache_material_run_version=material.run_version
                 )
                 AND NOT EXISTS (
                     SELECT 1
                     FROM chain_post_close_position_concept_rpc_occurrences AS work
                     LEFT JOIN chain_post_close_position_concept_rpc_finals AS final
                       ON final.intent_id=work.intent_id
                      AND final.cache_material_run_version=work.cache_material_run_version
                      AND final.position_ordinal=work.position_ordinal
                      AND final.final_outcome='Available'
                     LEFT JOIN chain_post_close_position_concept_cache_writes AS cache_write
                       ON cache_write.intent_id=final.intent_id
                      AND cache_write.cache_material_run_version=final.cache_material_run_version
                      AND cache_write.position_ordinal=final.position_ordinal
                      AND cache_write.final_run_version=final.run_version
                      AND cache_write.final_sha256=final.final_sha256
                     WHERE work.intent_id=NEW.intent_id
                       AND work.cache_material_run_version=material.run_version
                       AND (final.intent_id IS NULL OR cache_write.intent_id IS NULL
                            OR cache_write.written_at>NEW.planned_at)
                 )
                 AND NEW.parent_completion_run_version=(
                     SELECT max(cache_write.run_version)
                     FROM chain_post_close_position_concept_rpc_occurrences AS work
                     JOIN chain_post_close_position_concept_rpc_finals AS final
                       ON final.intent_id=work.intent_id
                      AND final.cache_material_run_version=work.cache_material_run_version
                      AND final.position_ordinal=work.position_ordinal
                      AND final.final_outcome='Available'
                     JOIN chain_post_close_position_concept_cache_writes AS cache_write
                       ON cache_write.intent_id=final.intent_id
                      AND cache_write.cache_material_run_version=final.cache_material_run_version
                      AND cache_write.position_ordinal=final.position_ordinal
                      AND cache_write.final_run_version=final.run_version
                      AND cache_write.final_sha256=final.final_sha256
                     WHERE work.intent_id=NEW.intent_id
                       AND work.cache_material_run_version=material.run_version
                 )
           ))
      )
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
    ) AS fact
    WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_occurrence_invalid');
END;

CREATE TRIGGER chain_post_close_dragon_tiger_attempt_begins_guard
BEFORE INSERT ON chain_post_close_dragon_tiger_attempt_begins
WHEN NOT EXISTS (SELECT 1 FROM chain_post_close_layouts WHERE layout_version=10)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id=NEW.intent_id AND run.run_id=NEW.run_id
      AND run.run_context_sha256=NEW.run_context_sha256
      AND run.input_sha256=NEW.input_sha256
      AND run.lease_owner=NEW.lease_owner
      AND run.lease_generation=NEW.lease_generation
      AND run.head_version=NEW.run_version AND run.updated_at=NEW.begun_at
      AND run.lease_until>NEW.begun_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_dragon_tiger_occurrences AS occurrence
    WHERE occurrence.intent_id=NEW.intent_id
      AND occurrence.run_version=NEW.occurrence_run_version
      AND occurrence.request_id=NEW.request_id
      AND occurrence.request_sha256=NEW.request_sha256
      AND occurrence.run_id=NEW.run_id
      AND occurrence.run_context_sha256=NEW.run_context_sha256
      AND occurrence.input_sha256=NEW.input_sha256
      AND occurrence.run_version<NEW.run_version
      AND occurrence.planned_at<=NEW.begun_at
      AND NEW.attempt_ordinal<=occurrence.retry_max_attempts
      AND occurrence.lease_generation<=NEW.lease_generation
      AND (occurrence.lease_generation<NEW.lease_generation
           OR occurrence.lease_owner=NEW.lease_owner)
)
OR (NEW.attempt_ordinal=1 AND EXISTS (
    SELECT 1 FROM chain_post_close_dragon_tiger_attempt_results AS result
    WHERE result.intent_id=NEW.intent_id
))
OR (NEW.attempt_ordinal>1 AND NOT EXISTS (
    SELECT 1 FROM chain_post_close_dragon_tiger_attempt_results AS result
    WHERE result.intent_id=NEW.intent_id
      AND result.attempt_ordinal=NEW.previous_attempt_ordinal
      AND result.run_version=NEW.previous_result_run_version
      AND result.result_sha256=NEW.previous_result_sha256
      AND result.request_sha256=NEW.request_sha256
      AND result.continuation='Retry'
      AND result.run_version<NEW.run_version
      AND result.committed_at<=NEW.begun_at
))
OR EXISTS (
    SELECT 1 FROM chain_post_close_dragon_tiger_finals AS final
    WHERE final.intent_id=NEW.intent_id
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_attempt_begin_invalid');
END;

CREATE TRIGGER chain_post_close_dragon_tiger_attempt_results_guard
BEFORE INSERT ON chain_post_close_dragon_tiger_attempt_results
WHEN NOT EXISTS (SELECT 1 FROM chain_post_close_layouts WHERE layout_version=10)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id=NEW.intent_id AND run.run_id=NEW.run_id
      AND run.run_context_sha256=NEW.run_context_sha256
      AND run.input_sha256=NEW.input_sha256
      AND run.lease_owner=NEW.lease_owner
      AND run.lease_generation=NEW.lease_generation
      AND run.head_version=NEW.run_version AND run.updated_at=NEW.committed_at
      AND run.lease_until>NEW.committed_at
)
OR NOT EXISTS (
    SELECT 1
    FROM chain_post_close_dragon_tiger_attempt_begins AS begun
    JOIN chain_post_close_dragon_tiger_occurrences AS occurrence
      ON occurrence.intent_id=begun.intent_id
     AND occurrence.run_version=begun.occurrence_run_version
    WHERE begun.intent_id=NEW.intent_id
      AND begun.attempt_ordinal=NEW.attempt_ordinal
      AND begun.run_version=NEW.begin_run_version
      AND begun.request_sha256=NEW.request_sha256
      AND begun.run_id=NEW.run_id
      AND begun.run_context_sha256=NEW.run_context_sha256
      AND begun.input_sha256=NEW.input_sha256
      AND begun.lease_owner=NEW.lease_owner
      AND begun.lease_generation=NEW.lease_generation
      AND begun.begun_at<=NEW.returned_at
      AND NEW.prior_head_version=begun.run_version
      AND (NEW.continuation='Terminal'
           OR NEW.attempt_ordinal<occurrence.retry_max_attempts)
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_dragon_tiger_attempt_begins AS later
    WHERE later.intent_id=NEW.intent_id AND later.attempt_ordinal>NEW.attempt_ordinal
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_attempt_result_invalid');
END;

CREATE TRIGGER chain_post_close_dragon_tiger_status_materials_guard
BEFORE INSERT ON chain_post_close_dragon_tiger_status_materials
WHEN NOT EXISTS (SELECT 1 FROM chain_post_close_layouts WHERE layout_version=10)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id=NEW.intent_id AND run.run_id=NEW.run_id
      AND run.run_context_sha256=NEW.run_context_sha256
      AND run.input_sha256=NEW.input_sha256
      AND run.lease_owner=NEW.lease_owner
      AND run.lease_generation=NEW.lease_generation
      AND run.head_version=NEW.run_version AND run.updated_at=NEW.captured_at
      AND run.lease_until>NEW.captured_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_dragon_tiger_attempt_results AS result
    WHERE result.intent_id=NEW.intent_id
      AND result.attempt_ordinal=NEW.attempt_ordinal
      AND result.run_version=NEW.result_run_version
      AND result.result_sha256=NEW.result_sha256
      AND result.request_sha256=NEW.request_sha256
      AND result.wire_outcome='Status'
      AND result.run_id=NEW.run_id
      AND result.run_context_sha256=NEW.run_context_sha256
      AND result.input_sha256=NEW.input_sha256
      AND result.lease_owner=NEW.lease_owner
      AND result.lease_generation=NEW.lease_generation
      AND result.committed_at<=NEW.captured_at
      AND NEW.prior_head_version=result.run_version
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_status_material_invalid');
END;

CREATE TRIGGER chain_post_close_dragon_tiger_error_materials_guard
BEFORE INSERT ON chain_post_close_dragon_tiger_error_materials
WHEN NOT EXISTS (SELECT 1 FROM chain_post_close_layouts WHERE layout_version=10)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id=NEW.intent_id AND run.run_id=NEW.run_id
      AND run.run_context_sha256=NEW.run_context_sha256
      AND run.input_sha256=NEW.input_sha256
      AND run.lease_owner=NEW.lease_owner
      AND run.lease_generation=NEW.lease_generation
      AND run.head_version=NEW.run_version AND run.updated_at=NEW.captured_at
      AND run.lease_until>NEW.captured_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_dragon_tiger_attempt_results AS result
    WHERE result.intent_id=NEW.intent_id
      AND result.attempt_ordinal=NEW.terminal_attempt_ordinal
      AND result.run_version=NEW.terminal_result_run_version
      AND result.result_sha256=NEW.terminal_result_sha256
      AND result.request_sha256=NEW.request_sha256
      AND result.continuation='Terminal'
      AND result.run_id=NEW.run_id
      AND result.run_context_sha256=NEW.run_context_sha256
      AND result.input_sha256=NEW.input_sha256
      AND result.lease_owner=NEW.lease_owner
      AND result.lease_generation=NEW.lease_generation
      AND result.committed_at<=NEW.captured_at
      AND ((result.wire_outcome='Response'
            AND NEW.status_material_attempt_ordinal IS NULL
            AND NEW.status_material_run_version IS NULL
            AND NEW.status_material_sha256 IS NULL
            AND NEW.prior_head_version=result.run_version)
           OR
           (result.wire_outcome='Status' AND EXISTS (
               SELECT 1 FROM chain_post_close_dragon_tiger_status_materials AS status
               WHERE status.intent_id=NEW.intent_id
                 AND status.attempt_ordinal=NEW.status_material_attempt_ordinal
                 AND status.run_version=NEW.status_material_run_version
                 AND status.material_sha256=NEW.status_material_sha256
                 AND status.result_run_version=result.run_version
                 AND status.request_sha256=NEW.request_sha256
                 AND status.run_id=NEW.run_id
                 AND status.run_context_sha256=NEW.run_context_sha256
                 AND status.input_sha256=NEW.input_sha256
                 AND status.lease_owner=NEW.lease_owner
                 AND status.lease_generation=NEW.lease_generation
                 AND status.run_version=NEW.prior_head_version
                 AND status.captured_at<=NEW.captured_at
           )))
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_dragon_tiger_attempt_begins AS later
    WHERE later.intent_id=NEW.intent_id
      AND later.attempt_ordinal>NEW.terminal_attempt_ordinal
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_error_material_invalid');
END;

CREATE TRIGGER chain_post_close_dragon_tiger_finals_guard
BEFORE INSERT ON chain_post_close_dragon_tiger_finals
WHEN NOT EXISTS (SELECT 1 FROM chain_post_close_layouts WHERE layout_version=10)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id=NEW.intent_id AND run.run_id=NEW.run_id
      AND run.run_context_sha256=NEW.run_context_sha256
      AND run.input_sha256=NEW.input_sha256
      AND run.lease_owner=NEW.lease_owner
      AND run.lease_generation=NEW.lease_generation
      AND run.head_version=NEW.run_version AND run.updated_at=NEW.applied_at
      AND run.lease_until>NEW.applied_at
)
OR NOT EXISTS (
    SELECT 1
    FROM chain_post_close_dragon_tiger_occurrences AS occurrence
    JOIN chain_post_close_dragon_tiger_attempt_results AS terminal
      ON terminal.intent_id=occurrence.intent_id
     AND terminal.request_sha256=occurrence.request_sha256
    WHERE occurrence.intent_id=NEW.intent_id
      AND occurrence.run_version=NEW.occurrence_run_version
      AND occurrence.request_sha256=NEW.occurrence_request_sha256
      AND terminal.attempt_ordinal=NEW.terminal_attempt_ordinal
      AND terminal.run_version=NEW.terminal_result_run_version
      AND terminal.result_sha256=NEW.terminal_result_sha256
      AND terminal.continuation='Terminal'
      AND terminal.run_id=NEW.run_id
      AND terminal.run_context_sha256=NEW.run_context_sha256
      AND terminal.input_sha256=NEW.input_sha256
      AND terminal.committed_at<=NEW.applied_at
      AND terminal.lease_generation<=NEW.lease_generation
      AND (terminal.lease_generation<NEW.lease_generation
           OR terminal.lease_owner=NEW.lease_owner)
      AND ((NEW.final_outcome IN ('Available','VerifiedEmpty')
            AND terminal.wire_outcome='Response'
            AND NEW.error_material_run_version IS NULL
            AND NEW.error_material_sha256 IS NULL)
           OR
           (NEW.final_outcome='Error' AND EXISTS (
               SELECT 1 FROM chain_post_close_dragon_tiger_error_materials AS error
               WHERE error.intent_id=NEW.intent_id
                 AND error.run_version=NEW.error_material_run_version
                 AND error.material_sha256=NEW.error_material_sha256
                 AND error.terminal_attempt_ordinal=terminal.attempt_ordinal
                 AND error.terminal_result_run_version=terminal.run_version
                 AND error.terminal_result_sha256=terminal.result_sha256
                 AND error.request_sha256=occurrence.request_sha256
                 AND error.run_id=NEW.run_id
                 AND error.run_context_sha256=NEW.run_context_sha256
                 AND error.input_sha256=NEW.input_sha256
                 AND error.run_version<=NEW.prior_head_version
                 AND error.captured_at<=NEW.applied_at
                 AND error.lease_generation<=NEW.lease_generation
                 AND (error.lease_generation<NEW.lease_generation
                      OR error.lease_owner=NEW.lease_owner)
           )))
      AND terminal.run_version<=NEW.prior_head_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_dragon_tiger_attempt_begins AS later
    WHERE later.intent_id=NEW.intent_id
      AND later.attempt_ordinal>NEW.terminal_attempt_ordinal
)
OR NOT EXISTS (
    SELECT 1
    FROM data_acquisition_audit AS audit
    JOIN data_acquisition_audit_chain AS chain
      ON chain.acquisition_audit_id=audit.id
    WHERE audit.id=NEW.audit_id
      AND audit.schema_version=1
      AND audit.capability='R-04'
      AND length(audit.provider)>0
      AND length(audit.source)>0
      AND audit.request_hash=(
          SELECT occurrence.acquisition_request_hash
          FROM chain_post_close_dragon_tiger_occurrences AS occurrence
          WHERE occurrence.intent_id=NEW.intent_id
      )
      AND audit.observed_at!=''
      AND audit.outcome=NEW.current_outcome
      AND audit.request_count=1
      AND chain.record_hash=NEW.audit_record_hash
      AND NEW.previous_outcome IS (
          SELECT previous.outcome FROM data_acquisition_audit AS previous
          WHERE previous.id<audit.id
            AND previous.capability=audit.capability
            AND previous.provider=audit.provider
          ORDER BY previous.id DESC LIMIT 1
      )
      AND ((NEW.final_outcome='Available'
            AND audit.outcome='available'
            AND audit.accepted_count>=1 AND audit.rejected_count=0
            AND audit.reason_code='accepted' AND audit.retryable=0)
           OR (NEW.final_outcome='VerifiedEmpty'
            AND audit.outcome='verified_empty'
            AND audit.accepted_count=0 AND audit.rejected_count=0
            AND audit.reason_code='verified_empty' AND audit.retryable=0)
           OR (NEW.final_outcome='Error'
            AND audit.provider='Eastmoney'
            AND audit.outcome NOT IN ('available','verified_empty')
            AND audit.accepted_count=0 AND audit.rejected_count=1
            AND length(audit.reason_code)>0))
)
OR EXISTS (SELECT 1 FROM chain_post_close_board_kind_finals WHERE audit_id=NEW.audit_id)
OR EXISTS (SELECT 1 FROM chain_post_close_concept_rpc_finals WHERE audit_id=NEW.audit_id)
OR EXISTS (SELECT 1 FROM chain_post_close_position_concept_rpc_finals WHERE audit_id=NEW.audit_id)
OR EXISTS (
    SELECT 1 FROM (
        SELECT intent_id,run_version FROM chain_post_close_stage_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_stage_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_configurations
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_cluster_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_chain_daily_applications
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_kind_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_directory_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_selections
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_status_materials WHERE run_version IS NOT NULL
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_board_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_rpc_finals
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_position_concept_cache_writes
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_occurrences
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_begins
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_attempt_results
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_status_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_error_materials
        UNION ALL SELECT intent_id,run_version FROM chain_post_close_dragon_tiger_finals
    ) AS fact WHERE fact.intent_id=NEW.intent_id AND fact.run_version=NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_final_invalid');
END;

CREATE TRIGGER chain_post_close_dragon_tiger_occurrences_update
BEFORE UPDATE ON chain_post_close_dragon_tiger_occurrences
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_occurrence_immutable'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_occurrences_delete
BEFORE DELETE ON chain_post_close_dragon_tiger_occurrences
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_occurrence_immutable'); END;

CREATE TRIGGER chain_post_close_dragon_tiger_attempt_begins_update
BEFORE UPDATE ON chain_post_close_dragon_tiger_attempt_begins
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_begin_immutable'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_attempt_begins_delete
BEFORE DELETE ON chain_post_close_dragon_tiger_attempt_begins
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_begin_immutable'); END;

CREATE TRIGGER chain_post_close_dragon_tiger_attempt_results_update
BEFORE UPDATE ON chain_post_close_dragon_tiger_attempt_results
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_result_immutable'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_attempt_results_delete
BEFORE DELETE ON chain_post_close_dragon_tiger_attempt_results
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_result_immutable'); END;

CREATE TRIGGER chain_post_close_dragon_tiger_status_materials_update
BEFORE UPDATE ON chain_post_close_dragon_tiger_status_materials
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_status_immutable'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_status_materials_delete
BEFORE DELETE ON chain_post_close_dragon_tiger_status_materials
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_status_immutable'); END;

CREATE TRIGGER chain_post_close_dragon_tiger_error_materials_update
BEFORE UPDATE ON chain_post_close_dragon_tiger_error_materials
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_error_immutable'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_error_materials_delete
BEFORE DELETE ON chain_post_close_dragon_tiger_error_materials
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_error_immutable'); END;

CREATE TRIGGER chain_post_close_dragon_tiger_finals_update
BEFORE UPDATE ON chain_post_close_dragon_tiger_finals
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_final_immutable'); END;
CREATE TRIGGER chain_post_close_dragon_tiger_finals_delete
BEFORE DELETE ON chain_post_close_dragon_tiger_finals
BEGIN SELECT RAISE(ABORT,'chain_post_close.dragon_tiger_final_immutable'); END;
