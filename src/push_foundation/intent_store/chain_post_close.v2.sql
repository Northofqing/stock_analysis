CREATE TABLE chain_post_close_layouts (
    layout_version INTEGER PRIMARY KEY CHECK (layout_version >= 2),
    predecessor_layout_version INTEGER NOT NULL CHECK (
        predecessor_layout_version = layout_version - 1
    ),
    predecessor_bundle_sha256 TEXT NOT NULL CHECK (
        length(predecessor_bundle_sha256) = 64
        AND predecessor_bundle_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_codec_version INTEGER NOT NULL CHECK (artifact_codec_version >= 1),
    input_codec_version INTEGER NOT NULL CHECK (input_codec_version >= 1),
    stage_codec_version INTEGER NOT NULL CHECK (stage_codec_version >= 1),
    description TEXT NOT NULL CHECK (length(description) > 0),
    bundle_sha256 TEXT NOT NULL UNIQUE CHECK (
        length(bundle_sha256) = 64 AND bundle_sha256 NOT GLOB '*[^0-9a-f]*'
    )
);

CREATE TABLE chain_post_close_layout_objects (
    layout_version INTEGER NOT NULL,
    name TEXT NOT NULL CHECK (length(name) > 0),
    object_type TEXT NOT NULL CHECK (object_type IN ('table', 'index', 'trigger')),
    definition TEXT NOT NULL CHECK (length(definition) > 0),
    PRIMARY KEY (layout_version, name),
    FOREIGN KEY (layout_version) REFERENCES chain_post_close_layouts(layout_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TRIGGER chain_post_close_layouts_seal
BEFORE INSERT ON chain_post_close_layouts
WHEN NEW.layout_version != COALESCE(
        (SELECT MAX(layout_version) + 1 FROM chain_post_close_layouts), 2
    )
    OR NOT EXISTS (
        SELECT 1 FROM chain_post_close_layout_objects
        WHERE layout_version = NEW.layout_version
    )
    OR (
        NEW.layout_version = 2
        AND (
            NEW.predecessor_layout_version != 1
            OR NEW.predecessor_bundle_sha256 !=
                'cfaedcafa3bda35942404b874e954a3b88c764a1600e9060a496163721742cb5'
        )
    )
    OR (
        NEW.layout_version > 2
        AND NOT EXISTS (
            SELECT 1 FROM chain_post_close_layouts AS predecessor
            WHERE predecessor.layout_version = NEW.predecessor_layout_version
              AND predecessor.bundle_sha256 = NEW.predecessor_bundle_sha256
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.layout_invalid');
END;

CREATE TRIGGER chain_post_close_layouts_update
BEFORE UPDATE ON chain_post_close_layouts
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.layout_immutable');
END;

CREATE TRIGGER chain_post_close_layouts_delete
BEFORE DELETE ON chain_post_close_layouts
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.layout_immutable');
END;

CREATE TRIGGER chain_post_close_layout_objects_insert
BEFORE INSERT ON chain_post_close_layout_objects
WHEN EXISTS (
    SELECT 1 FROM chain_post_close_layouts
    WHERE layout_version = NEW.layout_version
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.layout_registry_sealed');
END;

CREATE TRIGGER chain_post_close_layout_objects_update
BEFORE UPDATE ON chain_post_close_layout_objects
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.layout_registry_immutable');
END;

CREATE TRIGGER chain_post_close_layout_objects_delete
BEFORE DELETE ON chain_post_close_layout_objects
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.layout_registry_immutable');
END;

CREATE TABLE chain_post_close_runs (
    intent_id TEXT PRIMARY KEY CHECK (
        length(intent_id) = 64 AND intent_id NOT GLOB '*[^0-9a-f]*'
    ),
    run_id TEXT NOT NULL UNIQUE CHECK (length(run_id) BETWEEN 1 AND 512),
    context_codec_version INTEGER NOT NULL CHECK (context_codec_version = 1),
    context_bytes BLOB NOT NULL CHECK (
        typeof(context_bytes) = 'blob' AND length(context_bytes) > 0
    ),
    context_length INTEGER NOT NULL CHECK (context_length = length(context_bytes)),
    run_context_sha256 TEXT NOT NULL CHECK (
        length(run_context_sha256) = 64
        AND run_context_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    namespace TEXT NOT NULL CHECK (namespace = 'Production'),
    unit_id TEXT NOT NULL CHECK (unit_id = 'MU-chain-post-close'),
    producer_id TEXT NOT NULL CHECK (producer_id = 'chain-post-close-timer'),
    phase TEXT NOT NULL CHECK (phase = 'Postclose'),
    occurrence_id TEXT NOT NULL CHECK (
        length(occurrence_id) = 64 AND occurrence_id NOT GLOB '*[^0-9a-f]*'
    ),
    occurrence_family TEXT NOT NULL CHECK (
        occurrence_family =
            'calendar date / 15:30≤t<15:35 / latest completed business date'
    ),
    occurrence_key TEXT NOT NULL CHECK (length(occurrence_key) BETWEEN 1 AND 512),
    calendar_date TEXT NOT NULL CHECK (
        length(calendar_date) = 10
        AND substr(calendar_date, 5, 1) = '-'
        AND substr(calendar_date, 8, 1) = '-'
    ),
    business_date TEXT NOT NULL CHECK (
        length(business_date) = 10
        AND substr(business_date, 5, 1) = '-'
        AND substr(business_date, 8, 1) = '-'
    ),
    completion_owner TEXT NOT NULL CHECK (
        completion_owner = 'monitor_loop::CHAIN_POST_LAST[calendar_date]'
    ),
    source_contract_id TEXT NOT NULL CHECK (
        source_contract_id = 'chain-post-close-passed-input-v1'
    ),
    source_contract_version TEXT NOT NULL CHECK (source_contract_version = '1'),
    template_version TEXT NOT NULL CHECK (template_version = 'chain-analysis-prepared-v1'),
    layout_version INTEGER NOT NULL CHECK (layout_version = 2),
    input_codec_version INTEGER NOT NULL CHECK (input_codec_version = 1),
    input_bytes BLOB NOT NULL CHECK (
        typeof(input_bytes) = 'blob' AND length(input_bytes) > 0
    ),
    input_length INTEGER NOT NULL CHECK (input_length = length(input_bytes)),
    input_sha256 TEXT NOT NULL CHECK (
        length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    lease_until INTEGER NOT NULL CHECK (lease_until >= 0),
    head_version INTEGER NOT NULL CHECK (head_version >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (
        updated_at >= created_at AND lease_until > updated_at
    ),
    FOREIGN KEY (layout_version) REFERENCES chain_post_close_layouts(layout_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TRIGGER chain_post_close_runs_update
BEFORE UPDATE ON chain_post_close_runs
WHEN NEW.intent_id IS NOT OLD.intent_id
    OR NEW.run_id IS NOT OLD.run_id
    OR NEW.context_codec_version IS NOT OLD.context_codec_version
    OR NEW.context_bytes IS NOT OLD.context_bytes
    OR NEW.context_length IS NOT OLD.context_length
    OR NEW.run_context_sha256 IS NOT OLD.run_context_sha256
    OR NEW.namespace IS NOT OLD.namespace
    OR NEW.unit_id IS NOT OLD.unit_id
    OR NEW.producer_id IS NOT OLD.producer_id
    OR NEW.phase IS NOT OLD.phase
    OR NEW.occurrence_id IS NOT OLD.occurrence_id
    OR NEW.occurrence_family IS NOT OLD.occurrence_family
    OR NEW.occurrence_key IS NOT OLD.occurrence_key
    OR NEW.calendar_date IS NOT OLD.calendar_date
    OR NEW.business_date IS NOT OLD.business_date
    OR NEW.completion_owner IS NOT OLD.completion_owner
    OR NEW.source_contract_id IS NOT OLD.source_contract_id
    OR NEW.source_contract_version IS NOT OLD.source_contract_version
    OR NEW.template_version IS NOT OLD.template_version
    OR NEW.layout_version IS NOT OLD.layout_version
    OR NEW.input_codec_version IS NOT OLD.input_codec_version
    OR NEW.input_bytes IS NOT OLD.input_bytes
    OR NEW.input_length IS NOT OLD.input_length
    OR NEW.input_sha256 IS NOT OLD.input_sha256
    OR NEW.created_at IS NOT OLD.created_at
    OR NEW.head_version != OLD.head_version + 1
    OR NEW.lease_generation NOT IN (OLD.lease_generation, OLD.lease_generation + 1)
    OR (
        NEW.lease_generation = OLD.lease_generation
        AND NEW.lease_owner IS NOT OLD.lease_owner
    )
    OR NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.run_transition_invalid');
END;

CREATE TRIGGER chain_post_close_runs_delete
BEFORE DELETE ON chain_post_close_runs
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.run_immutable');
END;

CREATE TABLE chain_post_close_stage_begins (
    intent_id TEXT NOT NULL,
    effect_kind TEXT NOT NULL CHECK (effect_kind = 'ConceptProvider'),
    effect_ordinal INTEGER NOT NULL CHECK (effect_ordinal >= 0),
    effect_key TEXT NOT NULL CHECK (length(effect_key) BETWEEN 1 AND 512),
    request_codec_version INTEGER NOT NULL CHECK (request_codec_version = 1),
    request_bytes BLOB NOT NULL CHECK (
        typeof(request_bytes) = 'blob' AND length(request_bytes) > 0
    ),
    request_length INTEGER NOT NULL CHECK (request_length = length(request_bytes)),
    request_sha256 TEXT NOT NULL CHECK (
        length(request_sha256) = 64 AND request_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    run_version INTEGER NOT NULL CHECK (run_version >= 1),
    begun_at INTEGER NOT NULL CHECK (begun_at >= 0),
    PRIMARY KEY (intent_id, effect_kind, effect_ordinal),
    UNIQUE (intent_id, effect_kind, effect_key),
    UNIQUE (intent_id, run_version),
    UNIQUE (intent_id, effect_kind, effect_ordinal, lease_owner, lease_generation),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TRIGGER chain_post_close_stage_begins_guard
BEFORE INSERT ON chain_post_close_stage_begins
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.lease_until > NEW.begun_at
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.stage_begin_stale_run');
END;

CREATE TRIGGER chain_post_close_stage_begins_update
BEFORE UPDATE ON chain_post_close_stage_begins
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.stage_begin_immutable');
END;

CREATE TRIGGER chain_post_close_stage_begins_delete
BEFORE DELETE ON chain_post_close_stage_begins
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.stage_begin_immutable');
END;

CREATE TABLE chain_post_close_stage_results (
    intent_id TEXT NOT NULL,
    effect_kind TEXT NOT NULL CHECK (effect_kind = 'ConceptProvider'),
    effect_ordinal INTEGER NOT NULL CHECK (effect_ordinal >= 0),
    outcome TEXT NOT NULL CHECK (outcome IN ('Returned', 'BusinessError')),
    result_codec_version INTEGER NOT NULL CHECK (result_codec_version = 1),
    result_bytes BLOB NOT NULL CHECK (typeof(result_bytes) = 'blob'),
    result_length INTEGER NOT NULL CHECK (result_length = length(result_bytes)),
    result_sha256 TEXT NOT NULL CHECK (
        length(result_sha256) = 64 AND result_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    run_version INTEGER NOT NULL CHECK (run_version >= 1),
    returned_at INTEGER NOT NULL CHECK (returned_at >= 0),
    committed_at INTEGER NOT NULL CHECK (committed_at >= returned_at),
    PRIMARY KEY (intent_id, effect_kind, effect_ordinal),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (
        intent_id, effect_kind, effect_ordinal, lease_owner, lease_generation
    ) REFERENCES chain_post_close_stage_begins(
        intent_id, effect_kind, effect_ordinal, lease_owner, lease_generation
    ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TRIGGER chain_post_close_stage_results_guard
BEFORE INSERT ON chain_post_close_stage_results
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.lease_until > NEW.committed_at
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.stage_result_stale_run');
END;

CREATE TRIGGER chain_post_close_stage_results_update
BEFORE UPDATE ON chain_post_close_stage_results
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.stage_result_immutable');
END;

CREATE TRIGGER chain_post_close_stage_results_delete
BEFORE DELETE ON chain_post_close_stage_results
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.stage_result_immutable');
END;
