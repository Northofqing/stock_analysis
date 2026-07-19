-- BR-103: append-only real-account evidence with truthful nullable fields.
CREATE TABLE IF NOT EXISTS real_account_snapshot (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    snapshot_date TEXT NOT NULL,
    evidence_class TEXT NOT NULL,
    environment TEXT NOT NULL,
    total_assets REAL NOT NULL,
    securities_market_value REAL NOT NULL,
    available_cash REAL NOT NULL,
    withdrawable_cash REAL,
    holding_pnl REAL,
    daily_pnl REAL,
    daily_pnl_status TEXT NOT NULL,
    position_ratio_pct REAL,
    source_provider TEXT NOT NULL,
    source_account_type TEXT NOT NULL,
    ownership_attestation TEXT NOT NULL,
    currency TEXT NOT NULL,
    source_captured_at TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    account_mode TEXT,
    account_ref TEXT,
    account_ref_status TEXT NOT NULL,
    evidence_sha256 TEXT NOT NULL UNIQUE,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (total_assets >= 0),
    CHECK (securities_market_value >= 0),
    CHECK (available_cash >= 0),
    CHECK (withdrawable_cash IS NULL OR withdrawable_cash >= 0),
    CHECK (position_ratio_pct IS NULL OR (position_ratio_pct >= 0 AND position_ratio_pct <= 100))
);
CREATE INDEX IF NOT EXISTS idx_real_account_snapshot_observed
    ON real_account_snapshot(observed_at DESC, id DESC);
CREATE TRIGGER IF NOT EXISTS trg_real_account_snapshot_no_update
BEFORE UPDATE ON real_account_snapshot
BEGIN SELECT RAISE(ABORT, 'BR-103 real_account_snapshot is immutable'); END;
CREATE TRIGGER IF NOT EXISTS trg_real_account_snapshot_no_delete
BEFORE DELETE ON real_account_snapshot
BEGIN SELECT RAISE(ABORT, 'BR-103 real_account_snapshot retention is at least five years'); END;
