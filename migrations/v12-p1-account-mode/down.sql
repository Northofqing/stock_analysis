-- v12 PR1-1.5 (BR-021) — account_mode_log 回滚
-- 2026-07-05

DROP INDEX IF EXISTS idx_account_mode_log_ts;
DROP INDEX IF EXISTS idx_account_mode_log_new_mode;
DROP TABLE IF EXISTS account_mode_log;