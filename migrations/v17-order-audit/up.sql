-- BR-086 / Data Redline 2.7: immutable order-attempt audit.
-- Runtime bootstrap in src/database/mod.rs creates the same objects for legacy databases.

CREATE TABLE IF NOT EXISTS order_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    business_order_id TEXT NOT NULL,
    source TEXT NOT NULL,
    decision_basis TEXT NOT NULL,
    side TEXT NOT NULL CHECK(side IN ('buy', 'sell', 'cancel')),
    code TEXT NOT NULL,
    requested_price REAL NOT NULL,
    execution_price REAL,
    quantity INTEGER NOT NULL,
    quote_observed_at TEXT,
    outcome TEXT NOT NULL CHECK(outcome IN ('Filled', 'Rejected', 'Canceled')),
    failure_reason TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_order_audit_business_id
    ON order_audit(business_order_id, created_at);

CREATE TRIGGER IF NOT EXISTS trg_order_audit_validate_insert
BEFORE INSERT ON order_audit
WHEN trim(NEW.business_order_id) = ''
  OR trim(NEW.source) = ''
  OR trim(NEW.decision_basis) = ''
  OR trim(NEW.code) = ''
  OR (NEW.outcome = 'Filled' AND (
        NEW.requested_price <= 0
        OR NEW.execution_price IS NULL
        OR NEW.execution_price <= 0
        OR NEW.quantity <= 0
        OR NEW.quantity % 100 != 0
        OR NEW.quote_observed_at IS NULL
        OR trim(NEW.quote_observed_at) = ''
     ))
  OR (NEW.outcome = 'Rejected' AND (
        NEW.failure_reason IS NULL OR trim(NEW.failure_reason) = ''
     ))
BEGIN SELECT RAISE(ABORT, 'BR-086 invalid order_audit record'); END;

CREATE TRIGGER IF NOT EXISTS trg_order_audit_no_update
BEFORE UPDATE ON order_audit
BEGIN SELECT RAISE(ABORT, 'BR-086 order_audit is immutable'); END;

CREATE TRIGGER IF NOT EXISTS trg_order_audit_no_delete
BEFORE DELETE ON order_audit
BEGIN SELECT RAISE(ABORT, 'BR-086 order_audit retention is at least five years'); END;

CREATE TABLE IF NOT EXISTS order_audit_chain (
    order_audit_id INTEGER PRIMARY KEY NOT NULL,
    previous_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(order_audit_id) REFERENCES order_audit(id)
);

CREATE TRIGGER IF NOT EXISTS trg_order_audit_chain_no_update
BEFORE UPDATE ON order_audit_chain
BEGIN SELECT RAISE(ABORT, 'BR-086 order audit hash chain is immutable'); END;

CREATE TRIGGER IF NOT EXISTS trg_order_audit_chain_no_delete
BEFORE DELETE ON order_audit_chain
BEGIN SELECT RAISE(ABORT, 'BR-086 order audit hash chain retention is at least five years'); END;
