-- v12 PR3-3.1 回滚
-- 2026-07-05

DROP INDEX IF EXISTS idx_paper_trades_code;
DROP INDEX IF EXISTS idx_paper_trades_status;
DROP INDEX IF EXISTS idx_paper_trades_ts;
DROP INDEX IF EXISTS uniq_paper_trades_plan_id;
DROP TABLE IF EXISTS paper_trades;

DROP INDEX IF EXISTS idx_execution_tracking_plan_id;
DROP INDEX IF EXISTS idx_execution_tracking_code;
DROP TABLE IF EXISTS execution_tracking;

DROP INDEX IF EXISTS idx_position_adjustments_code;
DROP INDEX IF EXISTS idx_position_adjustments_effective;
DROP TABLE IF EXISTS position_adjustments;

DROP INDEX IF EXISTS idx_stock_position_chain_name;
-- SQLite 3.35+ 支持 DROP COLUMN, 老版本需 recreate table. 这里保留兼容性.
-- ALTER TABLE stock_position DROP COLUMN chain_name;