
CREATE TABLE IF NOT EXISTS data_acquisition_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version INTEGER NOT NULL CHECK(schema_version = 1),
    capability TEXT NOT NULL,
    provider TEXT NOT NULL,
    source TEXT NOT NULL,
    request_hash TEXT NOT NULL CHECK(length(request_hash) = 64),
    source_at TEXT,
    observed_at TEXT NOT NULL,
    batch_id TEXT,
    outcome TEXT NOT NULL CHECK(outcome IN (
        'available', 'verified_empty', 'invalid_request', 'unavailable', 'stale',
        'partial', 'conflict', 'unsupported'
    )),
    request_count INTEGER NOT NULL CHECK(request_count >= 0),
    accepted_count INTEGER NOT NULL CHECK(accepted_count >= 0),
    rejected_count INTEGER NOT NULL CHECK(rejected_count >= 0),
    reason_code TEXT NOT NULL,
    retryable INTEGER NOT NULL CHECK(retryable IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    )
);
CREATE INDEX IF NOT EXISTS idx_data_acquisition_audit_provider
    ON data_acquisition_audit(capability, provider, id);
CREATE TRIGGER IF NOT EXISTS trg_data_acquisition_audit_no_update
    BEFORE UPDATE ON data_acquisition_audit
    BEGIN SELECT RAISE(ABORT, 'BR-159 acquisition audit is immutable'); END;
CREATE TRIGGER IF NOT EXISTS trg_data_acquisition_audit_no_delete
    BEFORE DELETE ON data_acquisition_audit
    BEGIN SELECT RAISE(ABORT, 'BR-159 acquisition audit retention is at least five years'); END;
CREATE TABLE IF NOT EXISTS data_acquisition_audit_chain (
    acquisition_audit_id INTEGER PRIMARY KEY NOT NULL,
    previous_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    ),
    FOREIGN KEY(acquisition_audit_id) REFERENCES data_acquisition_audit(id)
);
CREATE TABLE IF NOT EXISTS news_ai_global_critical_v1 (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    assessment_id TEXT NOT NULL UNIQUE CHECK(length(assessment_id)=64),
    evidence_json TEXT NOT NULL CHECK(length(evidence_json)>0),
    evidence_sha256 TEXT NOT NULL CHECK(length(evidence_sha256)=64),
    previous_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL CHECK(length(record_hash)=64)
);
CREATE TRIGGER IF NOT EXISTS trg_global_critical_no_update BEFORE UPDATE ON news_ai_global_critical_v1 BEGIN SELECT RAISE(ABORT,'immutable global N01'); END;
CREATE TRIGGER IF NOT EXISTS trg_global_critical_no_delete BEFORE DELETE ON news_ai_global_critical_v1 BEGIN SELECT RAISE(ABORT,'immutable global N01'); END;
