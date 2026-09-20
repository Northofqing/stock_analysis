CREATE TABLE chain_post_close_board_status_materials (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    kind TEXT NOT NULL CHECK (kind IN ('Industry', 'Concept')),
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal >= 1),
    result_run_version INTEGER NOT NULL CHECK (result_run_version >= 1),
    result_sha256 TEXT NOT NULL CHECK (
        length(result_sha256) = 64 AND result_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    provenance TEXT NOT NULL CHECK (provenance IN ('Captured', 'LegacyV5Absent')),
    projection_version INTEGER,
    material_codec_version INTEGER,
    material_bytes BLOB,
    material_length INTEGER,
    material_sha256 TEXT,
    run_id TEXT,
    run_context_sha256 TEXT,
    input_sha256 TEXT,
    lease_owner TEXT,
    lease_generation INTEGER,
    prior_head_version INTEGER,
    run_version INTEGER,
    captured_at INTEGER,
    legacy_layout_version INTEGER,
    CHECK (
        (
            provenance = 'Captured'
            AND projection_version IS NOT NULL AND projection_version = 1
            AND material_codec_version IS NOT NULL AND material_codec_version = 1
            AND typeof(material_bytes) = 'blob'
            AND length(material_bytes) > 0
            AND material_length IS NOT NULL
            AND material_length = length(material_bytes)
            AND material_sha256 IS NOT NULL
            AND length(material_sha256) = 64
            AND material_sha256 NOT GLOB '*[^0-9a-f]*'
            AND run_id IS NOT NULL AND length(run_id) BETWEEN 1 AND 512
            AND run_context_sha256 IS NOT NULL
            AND length(run_context_sha256) = 64
            AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
            AND input_sha256 IS NOT NULL
            AND length(input_sha256) = 64
            AND input_sha256 NOT GLOB '*[^0-9a-f]*'
            AND lease_owner IS NOT NULL AND length(lease_owner) BETWEEN 1 AND 512
            AND lease_generation IS NOT NULL AND lease_generation >= 1
            AND prior_head_version IS NOT NULL AND prior_head_version >= 1
            AND run_version IS NOT NULL AND run_version = prior_head_version + 1
            AND captured_at IS NOT NULL AND captured_at >= 0
            AND legacy_layout_version IS NULL
        )
        OR (
            provenance = 'LegacyV5Absent'
            AND projection_version IS NULL
            AND material_codec_version IS NULL
            AND material_bytes IS NULL
            AND material_length IS NULL
            AND material_sha256 IS NULL
            AND run_id IS NULL
            AND run_context_sha256 IS NULL
            AND input_sha256 IS NULL
            AND lease_owner IS NULL
            AND lease_generation IS NULL
            AND prior_head_version IS NULL
            AND run_version IS NULL
            AND captured_at IS NULL
            AND legacy_layout_version IS NOT NULL
            AND legacy_layout_version = 6
        )
    ),
    PRIMARY KEY (intent_id, kind, attempt_ordinal),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (intent_id, kind, attempt_ordinal)
        REFERENCES chain_post_close_board_attempt_results(intent_id, kind, attempt_ordinal)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, result_run_version)
        REFERENCES chain_post_close_board_attempt_results(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_board_error_materials (
    intent_id TEXT NOT NULL CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    kind TEXT NOT NULL CHECK (kind IN ('Industry', 'Concept')),
    terminal_attempt_ordinal INTEGER NOT NULL CHECK (terminal_attempt_ordinal >= 1),
    terminal_result_run_version INTEGER NOT NULL CHECK (terminal_result_run_version >= 1),
    terminal_result_sha256 TEXT NOT NULL CHECK (
        length(terminal_result_sha256) = 64
        AND terminal_result_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
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
        (status_material_run_version IS NULL AND status_material_sha256 IS NULL)
        OR (
            status_material_run_version IS NOT NULL
            AND status_material_run_version >= 1
            AND status_material_sha256 IS NOT NULL
            AND length(status_material_sha256) = 64
            AND status_material_sha256 NOT GLOB '*[^0-9a-f]*'
        )
    ),
    PRIMARY KEY (intent_id, kind),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (intent_id, kind, terminal_attempt_ordinal)
        REFERENCES chain_post_close_board_attempt_results(intent_id, kind, attempt_ordinal)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, terminal_result_run_version)
        REFERENCES chain_post_close_board_attempt_results(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, status_material_run_version)
        REFERENCES chain_post_close_board_status_materials(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TRIGGER chain_post_close_board_status_materials_guard
BEFORE INSERT ON chain_post_close_board_status_materials
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_board_attempt_results AS result
    WHERE result.intent_id = NEW.intent_id
      AND result.kind = NEW.kind
      AND result.attempt_ordinal = NEW.attempt_ordinal
      AND result.run_version = NEW.result_run_version
      AND result.result_sha256 = NEW.result_sha256
      AND result.request_sha256 = NEW.request_sha256
      AND result.wire_outcome = 'Status'
)
OR (
    NEW.provenance = 'LegacyV5Absent'
    AND EXISTS (SELECT 1 FROM chain_post_close_layouts WHERE layout_version >= 6)
)
OR (
    NEW.provenance = 'Captured'
    AND (
        NEW.result_run_version != NEW.prior_head_version
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
            SELECT 1 FROM chain_post_close_board_attempt_results AS result
            WHERE result.intent_id = NEW.intent_id
              AND result.kind = NEW.kind
              AND result.attempt_ordinal = NEW.attempt_ordinal
              AND result.run_version = NEW.result_run_version
              AND result.run_id = NEW.run_id
              AND result.run_context_sha256 = NEW.run_context_sha256
              AND result.input_sha256 = NEW.input_sha256
              AND result.lease_owner = NEW.lease_owner
              AND result.lease_generation = NEW.lease_generation
              AND result.committed_at <= NEW.captured_at
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
                UNION ALL SELECT run_version FROM chain_post_close_board_error_materials WHERE intent_id=NEW.intent_id
            ) AS facts WHERE facts.run_version=NEW.run_version
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_status_material_invalid');
END;

CREATE TRIGGER chain_post_close_board_status_materials_update
BEFORE UPDATE ON chain_post_close_board_status_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_status_material_immutable');
END;

CREATE TRIGGER chain_post_close_board_status_materials_delete
BEFORE DELETE ON chain_post_close_board_status_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_status_material_immutable');
END;

CREATE TRIGGER chain_post_close_board_error_materials_guard
BEFORE INSERT ON chain_post_close_board_error_materials
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_board_attempt_results AS result
    WHERE result.intent_id = NEW.intent_id
      AND result.kind = NEW.kind
      AND result.attempt_ordinal = NEW.terminal_attempt_ordinal
      AND result.run_version = NEW.terminal_result_run_version
      AND result.result_sha256 = NEW.terminal_result_sha256
      AND result.request_sha256 = NEW.request_sha256
      AND result.continuation = 'Terminal'
      AND result.run_id = NEW.run_id
      AND result.run_context_sha256 = NEW.run_context_sha256
      AND result.input_sha256 = NEW.input_sha256
      AND result.run_version < NEW.run_version
      AND result.committed_at <= NEW.captured_at
      AND result.lease_generation <= NEW.lease_generation
      AND (
          result.lease_generation < NEW.lease_generation
          OR result.lease_owner = NEW.lease_owner
      )
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
OR EXISTS (
    SELECT 1 FROM chain_post_close_board_attempt_begins AS later
    WHERE later.intent_id=NEW.intent_id AND later.kind=NEW.kind
      AND later.attempt_ordinal>NEW.terminal_attempt_ordinal
)
OR (
    (SELECT wire_outcome FROM chain_post_close_board_attempt_results
     WHERE intent_id=NEW.intent_id AND kind=NEW.kind
       AND attempt_ordinal=NEW.terminal_attempt_ordinal) = 'Status'
    AND NOT EXISTS (
        SELECT 1 FROM chain_post_close_board_status_materials AS status
        WHERE status.intent_id=NEW.intent_id AND status.kind=NEW.kind
          AND status.attempt_ordinal=NEW.terminal_attempt_ordinal
          AND status.provenance='Captured'
          AND status.run_version=NEW.status_material_run_version
          AND status.material_sha256=NEW.status_material_sha256
          AND status.result_run_version=NEW.terminal_result_run_version
          AND status.result_sha256=NEW.terminal_result_sha256
          AND status.request_sha256=NEW.request_sha256
    )
)
OR (
    (SELECT wire_outcome FROM chain_post_close_board_attempt_results
     WHERE intent_id=NEW.intent_id AND kind=NEW.kind
       AND attempt_ordinal=NEW.terminal_attempt_ordinal) = 'Response'
    AND (NEW.status_material_run_version IS NOT NULL OR NEW.status_material_sha256 IS NOT NULL)
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
        UNION ALL SELECT run_version FROM chain_post_close_board_status_materials
            WHERE intent_id=NEW.intent_id AND run_version IS NOT NULL
    ) AS facts WHERE facts.run_version=NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_error_material_invalid');
END;

CREATE TRIGGER chain_post_close_board_error_materials_update
BEFORE UPDATE ON chain_post_close_board_error_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_error_material_immutable');
END;

CREATE TRIGGER chain_post_close_board_error_materials_delete
BEFORE DELETE ON chain_post_close_board_error_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.board_error_material_immutable');
END;
