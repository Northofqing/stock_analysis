-- Intentionally non-destructive.
-- Rolling application code back does not authorize deleting regulated order audit evidence.
-- A future archival migration may move records and their hash-chain evidence only after
-- preserving at least five years.
-- BR-086: before deploying a chain-unaware application, freeze all order/paper writers or
-- deploy a compatible writer that appends and validates order_audit_chain in the same transaction.
SELECT 1;
