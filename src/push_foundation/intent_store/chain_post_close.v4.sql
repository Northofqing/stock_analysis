CREATE TABLE chain_post_close_cluster_configurations (
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
    configuration_codec_version INTEGER NOT NULL CHECK (
        configuration_codec_version = 1
    ),
    configuration_bytes BLOB NOT NULL CHECK (
        typeof(configuration_bytes) = 'blob' AND length(configuration_bytes) > 0
    ),
    configuration_length INTEGER NOT NULL CHECK (
        configuration_length = length(configuration_bytes)
    ),
    configuration_sha256 TEXT NOT NULL CHECK (
        length(configuration_sha256) = 64
        AND configuration_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version >= 0),
    run_version INTEGER NOT NULL CHECK (run_version = prior_head_version + 1),
    fixed_at INTEGER NOT NULL CHECK (fixed_at >= 0),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_cluster_materials (
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
    concept_map_codec_version INTEGER NOT NULL CHECK (
        concept_map_codec_version = 1
    ),
    concept_map_bytes BLOB NOT NULL CHECK (
        typeof(concept_map_bytes) = 'blob' AND length(concept_map_bytes) > 0
    ),
    concept_map_length INTEGER NOT NULL CHECK (
        concept_map_length = length(concept_map_bytes)
    ),
    concept_map_sha256 TEXT NOT NULL CHECK (
        length(concept_map_sha256) = 64
        AND concept_map_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    concept_state_through_head_version INTEGER NOT NULL CHECK (
        concept_state_through_head_version >= 0
    ),
    configuration_run_version INTEGER NOT NULL CHECK (
        configuration_run_version >= 1
    ),
    configuration_sha256 TEXT NOT NULL CHECK (
        length(configuration_sha256) = 64
        AND configuration_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    material_codec_version INTEGER NOT NULL CHECK (material_codec_version = 1),
    material_bytes BLOB NOT NULL CHECK (
        typeof(material_bytes) = 'blob' AND length(material_bytes) > 0
    ),
    material_length INTEGER NOT NULL CHECK (
        material_length = length(material_bytes)
    ),
    material_sha256 TEXT NOT NULL CHECK (
        length(material_sha256) = 64
        AND material_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (
        prior_head_version = concept_state_through_head_version
    ),
    run_version INTEGER NOT NULL CHECK (
        run_version = prior_head_version + 1
        AND configuration_run_version < run_version
    ),
    materialized_at INTEGER NOT NULL CHECK (materialized_at >= 0),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, configuration_run_version)
        REFERENCES chain_post_close_cluster_configurations(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE chain_post_close_chain_daily_applications (
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
    business_date TEXT NOT NULL CHECK (
        length(business_date) = 10
        AND substr(business_date, 5, 1) = '-'
        AND substr(business_date, 8, 1) = '-'
    ),
    material_run_version INTEGER NOT NULL CHECK (material_run_version >= 1),
    material_sha256 TEXT NOT NULL CHECK (
        length(material_sha256) = 64
        AND material_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lifecycle_codec_version INTEGER NOT NULL CHECK (lifecycle_codec_version = 1),
    lifecycle_bytes BLOB NOT NULL CHECK (
        typeof(lifecycle_bytes) = 'blob' AND length(lifecycle_bytes) > 0
    ),
    lifecycle_length INTEGER NOT NULL CHECK (
        lifecycle_length = length(lifecycle_bytes)
    ),
    lifecycle_sha256 TEXT NOT NULL CHECK (
        length(lifecycle_sha256) = 64
        AND lifecycle_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    prior_head_version INTEGER NOT NULL CHECK (prior_head_version >= 0),
    run_version INTEGER NOT NULL CHECK (
        run_version = prior_head_version + 1
        AND material_run_version < run_version
    ),
    applied_at INTEGER NOT NULL CHECK (applied_at >= 0),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, material_run_version)
        REFERENCES chain_post_close_cluster_materials(intent_id, run_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TRIGGER chain_post_close_cluster_configurations_guard
BEFORE INSERT ON chain_post_close_cluster_configurations
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.run_id = NEW.run_id
      AND run.run_context_sha256 = NEW.run_context_sha256
      AND run.input_sha256 = NEW.input_sha256
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.updated_at = NEW.fixed_at
      AND run.lease_until > NEW.fixed_at
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_stage_begins
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_stage_results
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_cache_writes
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_cluster_materials
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_chain_daily_applications
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.cluster_configuration_invalid');
END;

CREATE TRIGGER chain_post_close_cluster_configurations_update
BEFORE UPDATE ON chain_post_close_cluster_configurations
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.cluster_configuration_immutable');
END;

CREATE TRIGGER chain_post_close_cluster_configurations_delete
BEFORE DELETE ON chain_post_close_cluster_configurations
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.cluster_configuration_immutable');
END;

CREATE TRIGGER chain_post_close_cluster_materials_guard
BEFORE INSERT ON chain_post_close_cluster_materials
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
    SELECT 1 FROM chain_post_close_cluster_configurations AS configuration
    WHERE configuration.intent_id = NEW.intent_id
      AND configuration.run_id = NEW.run_id
      AND configuration.run_context_sha256 = NEW.run_context_sha256
      AND configuration.input_sha256 = NEW.input_sha256
      AND configuration.run_version = NEW.configuration_run_version
      AND configuration.configuration_sha256 = NEW.configuration_sha256
      AND configuration.fixed_at <= NEW.materialized_at
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_stage_begins
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_stage_results
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_cache_writes
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_cluster_configurations
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_chain_daily_applications
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.cluster_material_invalid');
END;

CREATE TRIGGER chain_post_close_cluster_materials_update
BEFORE UPDATE ON chain_post_close_cluster_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.cluster_material_immutable');
END;

CREATE TRIGGER chain_post_close_cluster_materials_delete
BEFORE DELETE ON chain_post_close_cluster_materials
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.cluster_material_immutable');
END;

CREATE TRIGGER chain_post_close_chain_daily_applications_guard
BEFORE INSERT ON chain_post_close_chain_daily_applications
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.run_id = NEW.run_id
      AND run.run_context_sha256 = NEW.run_context_sha256
      AND run.input_sha256 = NEW.input_sha256
      AND run.business_date = NEW.business_date
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.updated_at = NEW.applied_at
      AND run.lease_until > NEW.applied_at
)
OR NOT EXISTS (
    SELECT 1 FROM chain_post_close_cluster_materials AS material
    WHERE material.intent_id = NEW.intent_id
      AND material.run_id = NEW.run_id
      AND material.run_context_sha256 = NEW.run_context_sha256
      AND material.input_sha256 = NEW.input_sha256
      AND material.run_version = NEW.material_run_version
      AND material.material_sha256 = NEW.material_sha256
      AND material.materialized_at <= NEW.applied_at
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_stage_begins
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_stage_results
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_concept_cache_writes
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_cluster_configurations
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
OR EXISTS (
    SELECT 1 FROM chain_post_close_cluster_materials
    WHERE intent_id = NEW.intent_id AND run_version = NEW.run_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.chain_daily_application_invalid');
END;

CREATE TRIGGER chain_post_close_chain_daily_applications_update
BEFORE UPDATE ON chain_post_close_chain_daily_applications
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.chain_daily_application_immutable');
END;

CREATE TRIGGER chain_post_close_chain_daily_applications_delete
BEFORE DELETE ON chain_post_close_chain_daily_applications
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.chain_daily_application_immutable');
END;
