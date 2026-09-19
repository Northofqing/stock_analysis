CREATE TABLE chain_post_close_position_materials (
    intent_id TEXT PRIMARY KEY CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (
        length(run_context_sha256) = 64
        AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    input_sha256 TEXT NOT NULL CHECK (
        length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    material_codec_version INTEGER NOT NULL CHECK (material_codec_version = 1),
    material_bytes BLOB NOT NULL CHECK (
        typeof(material_bytes) = 'blob' AND length(material_bytes) > 0
    ),
    material_length INTEGER NOT NULL CHECK (material_length = length(material_bytes)),
    material_sha256 TEXT NOT NULL CHECK (
        length(material_sha256) = 64
        AND material_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version >= 0),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    observed_at INTEGER NOT NULL CHECK (observed_at >= 0),
    committed_at INTEGER NOT NULL CHECK (committed_at >= observed_at),
    row_count INTEGER NOT NULL CHECK (row_count >= 0),
    UNIQUE (intent_id, run_version),
    UNIQUE (intent_id, run_version, material_sha256),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_position_concept_materials (
    intent_id TEXT PRIMARY KEY CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 512),
    run_context_sha256 TEXT NOT NULL CHECK (
        length(run_context_sha256) = 64
        AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    input_sha256 TEXT NOT NULL CHECK (
        length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    material_codec_version INTEGER NOT NULL CHECK (material_codec_version = 1),
    material_bytes BLOB NOT NULL CHECK (
        typeof(material_bytes) = 'blob' AND length(material_bytes) > 0
    ),
    material_length INTEGER NOT NULL CHECK (material_length = length(material_bytes)),
    material_sha256 TEXT NOT NULL CHECK (
        length(material_sha256) = 64
        AND material_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version >= 0),
    run_version INTEGER NOT NULL CHECK (
        run_version = prior_head_version + 1
        AND positions_run_version < run_version
    ),
    observed_at INTEGER NOT NULL CHECK (observed_at >= 0),
    committed_at INTEGER NOT NULL CHECK (committed_at >= observed_at),
    batch_kind TEXT NOT NULL CHECK (batch_kind = 'PositionConcepts'),
    positions_run_version INTEGER NOT NULL CHECK (positions_run_version >= 1),
    positions_sha256 TEXT NOT NULL CHECK (
        length(positions_sha256) = 64
        AND positions_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    requested_count INTEGER NOT NULL CHECK (requested_count > 0),
    cache_row_count INTEGER NOT NULL CHECK (cache_row_count >= 0),
    cache_max_age_days INTEGER NOT NULL CHECK (cache_max_age_days = 7),
    cache_cutoff TEXT NOT NULL CHECK (length(cache_cutoff) > 0),
    local_offset_seconds INTEGER NOT NULL CHECK (
        local_offset_seconds BETWEEN -86400 AND 86400
    ),
    cutoff_local_offset_seconds INTEGER NOT NULL CHECK (
        cutoff_local_offset_seconds BETWEEN -86400 AND 86400
    ),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, positions_run_version, positions_sha256)
        REFERENCES chain_post_close_position_materials(intent_id, run_version, material_sha256)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TRIGGER chain_post_close_position_materials_guard
BEFORE INSERT ON chain_post_close_position_materials
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
    SELECT 1 FROM chain_post_close_chain_daily_applications AS application
    WHERE application.intent_id = NEW.intent_id
      AND application.run_version < NEW.run_version
      AND application.applied_at <= NEW.observed_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_board_directory_materials AS directory
    WHERE directory.intent_id = NEW.intent_id
      AND directory.fold_outcome = 'Available'
      AND directory.run_version < NEW.run_version
      AND directory.materialized_at <= NEW.observed_at
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_stage_results WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_selections WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_status_materials WHERE intent_id=NEW.intent_id AND run_version IS NOT NULL
        UNION ALL SELECT run_version FROM chain_post_close_board_error_materials WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_occurrences WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_attempt_begins WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_attempt_results WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_status_materials WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_error_materials WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_finals WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_position_concept_materials WHERE intent_id=NEW.intent_id
    ) AS facts WHERE facts.run_version=NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_material_invalid');
END;

CREATE TRIGGER chain_post_close_position_materials_update
BEFORE UPDATE ON chain_post_close_position_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_material_immutable');
END;

CREATE TRIGGER chain_post_close_position_materials_delete
BEFORE DELETE ON chain_post_close_position_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_material_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_materials_guard
BEFORE INSERT ON chain_post_close_position_concept_materials
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id=NEW.intent_id
      AND run.run_id=NEW.run_id
      AND run.run_context_sha256=NEW.run_context_sha256
      AND run.input_sha256=NEW.input_sha256
      AND run.lease_owner=NEW.lease_owner
      AND run.lease_generation=NEW.lease_generation
      AND run.head_version=NEW.run_version
      AND run.updated_at=NEW.committed_at
      AND run.lease_until>NEW.committed_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_position_materials AS positions
    WHERE positions.intent_id=NEW.intent_id
      AND positions.run_id=NEW.run_id
      AND positions.run_context_sha256=NEW.run_context_sha256
      AND positions.input_sha256=NEW.input_sha256
      AND positions.run_version=NEW.positions_run_version
      AND positions.material_sha256=NEW.positions_sha256
      AND positions.row_count>0
      AND positions.committed_at<=NEW.observed_at
      AND positions.lease_generation<=NEW.lease_generation
      AND (positions.lease_generation<NEW.lease_generation OR positions.lease_owner=NEW.lease_owner)
)
OR EXISTS (
    SELECT 1 FROM (
        SELECT run_version FROM chain_post_close_stage_begins WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_stage_results WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_cache_writes WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_configurations WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_cluster_materials WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_chain_daily_applications WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_begins WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_attempt_results WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_kind_finals WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_directory_materials WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_selections WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_board_status_materials WHERE intent_id=NEW.intent_id AND run_version IS NOT NULL
        UNION ALL SELECT run_version FROM chain_post_close_board_error_materials WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_occurrences WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_attempt_begins WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_attempt_results WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_status_materials WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_error_materials WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_concept_rpc_finals WHERE intent_id=NEW.intent_id
        UNION ALL SELECT run_version FROM chain_post_close_position_materials WHERE intent_id=NEW.intent_id
    ) AS facts WHERE facts.run_version=NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_material_invalid');
END;

CREATE TRIGGER chain_post_close_position_concept_materials_update
BEFORE UPDATE ON chain_post_close_position_concept_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_material_immutable');
END;

CREATE TRIGGER chain_post_close_position_concept_materials_delete
BEFORE DELETE ON chain_post_close_position_concept_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.position_concept_material_immutable');
END;
