CREATE TABLE chain_post_close_schema (
    schema_version INTEGER PRIMARY KEY CHECK (schema_version = 1),
    artifact_codec_version INTEGER NOT NULL CHECK (artifact_codec_version = 1),
    description TEXT NOT NULL CHECK (description = 'chain-post-close-schema-v1'),
    bundle_sha256 TEXT NOT NULL CHECK (
        length(bundle_sha256) = 64 AND bundle_sha256 NOT GLOB '*[^0-9a-f]*'
    )
);

CREATE TABLE chain_post_close_objects (
    name TEXT PRIMARY KEY NOT NULL,
    object_type TEXT NOT NULL CHECK (object_type IN ('table', 'trigger')),
    definition TEXT NOT NULL CHECK (length(definition) > 0)
);

CREATE TRIGGER chain_post_close_schema_insert
BEFORE INSERT ON chain_post_close_schema
WHEN EXISTS (SELECT 1 FROM chain_post_close_schema)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.schema_sealed');
END;

CREATE TRIGGER chain_post_close_schema_update
BEFORE UPDATE ON chain_post_close_schema
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.schema_immutable');
END;

CREATE TRIGGER chain_post_close_schema_delete
BEFORE DELETE ON chain_post_close_schema
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.schema_immutable');
END;

CREATE TRIGGER chain_post_close_objects_insert
BEFORE INSERT ON chain_post_close_objects
WHEN EXISTS (SELECT 1 FROM chain_post_close_schema)
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.registry_sealed');
END;

CREATE TRIGGER chain_post_close_objects_update
BEFORE UPDATE ON chain_post_close_objects
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.registry_immutable');
END;

CREATE TRIGGER chain_post_close_objects_delete
BEFORE DELETE ON chain_post_close_objects
BEGIN
    SELECT RAISE(ABORT, 'chain_post_close.registry_immutable');
END;
