CREATE TABLE chain_post_close_concept_cache_writes (
    intent_id TEXT NOT NULL,
    effect_kind TEXT NOT NULL CHECK (effect_kind = 'ConceptProvider'),
    effect_ordinal INTEGER NOT NULL CHECK (effect_ordinal >= 0),
    code TEXT NOT NULL CHECK (length(code) BETWEEN 1 AND 512),
    provider_result_run_version INTEGER NOT NULL CHECK (
        provider_result_run_version >= 1
    ),
    concepts_codec_version INTEGER NOT NULL CHECK (concepts_codec_version = 1),
    concepts_bytes BLOB NOT NULL CHECK (
        typeof(concepts_bytes) = 'blob' AND length(concepts_bytes) > 0
    ),
    concepts_length INTEGER NOT NULL CHECK (
        concepts_length = length(concepts_bytes)
    ),
    concepts_sha256 TEXT NOT NULL CHECK (
        length(concepts_sha256) = 64
        AND concepts_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    cache_updated_at TEXT NOT NULL CHECK (length(cache_updated_at) = 19),
    lease_owner TEXT NOT NULL CHECK (length(lease_owner) BETWEEN 1 AND 512),
    lease_generation INTEGER NOT NULL CHECK (lease_generation >= 1),
    run_version INTEGER NOT NULL CHECK (run_version >= 1),
    written_at INTEGER NOT NULL CHECK (written_at >= 0),
    PRIMARY KEY (intent_id, effect_kind, effect_ordinal),
    UNIQUE (intent_id, code),
    UNIQUE (intent_id, run_version),
    FOREIGN KEY (intent_id) REFERENCES chain_post_close_runs(intent_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (intent_id, effect_kind, effect_ordinal)
        REFERENCES chain_post_close_stage_results(
            intent_id, effect_kind, effect_ordinal
        ) ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TRIGGER chain_post_close_concept_cache_writes_guard
BEFORE INSERT ON chain_post_close_concept_cache_writes
WHEN NOT EXISTS (
    SELECT 1 FROM chain_post_close_runs AS run
    WHERE run.intent_id = NEW.intent_id
      AND run.lease_owner = NEW.lease_owner
      AND run.lease_generation = NEW.lease_generation
      AND run.head_version = NEW.run_version
      AND run.lease_until > NEW.written_at
)
OR NOT EXISTS (
    SELECT 1
    FROM chain_post_close_stage_results AS result
    JOIN chain_post_close_stage_begins AS begun
      ON begun.intent_id = result.intent_id
     AND begun.effect_kind = result.effect_kind
     AND begun.effect_ordinal = result.effect_ordinal
    WHERE result.intent_id = NEW.intent_id
      AND result.effect_kind = NEW.effect_kind
      AND result.effect_ordinal = NEW.effect_ordinal
      AND begun.effect_key = NEW.code
      AND result.outcome = 'Returned'
      AND result.run_version = NEW.provider_result_run_version
      AND result.run_version < NEW.run_version
      AND result.committed_at <= NEW.written_at
)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_cache_write_invalid');
END;

CREATE TRIGGER chain_post_close_concept_cache_writes_update
BEFORE UPDATE ON chain_post_close_concept_cache_writes
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_cache_write_immutable');
END;

CREATE TRIGGER chain_post_close_concept_cache_writes_delete
BEFORE DELETE ON chain_post_close_concept_cache_writes
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.concept_cache_write_immutable');
END;
