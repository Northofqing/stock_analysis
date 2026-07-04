-- v12 PR1-1.5 (BR-021) — account_mode_log 表
-- 落 BR-021: 账户模式三态变更落库, 每条含 prev/new + 触发原因 + 当时 portfolio 指标快照
-- 2026-07-05

CREATE TABLE IF NOT EXISTS account_mode_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ts              TIMESTAMP NOT NULL,           -- 变更时间 (本地时区 ISO 字符串)
    prev_mode       TEXT NOT NULL,                -- 'Normal' / 'ReduceOnly' / 'Frozen'
    new_mode        TEXT NOT NULL,                -- 同上
    trigger_reason  TEXT NOT NULL,                -- 人类可读触发原因 (供 T-01 推送文案)
    today_pnl_pct   REAL,                         -- 变更时当日盈亏快照 (NULL 表示数据缺失)
    consecutive_n   INTEGER,                      -- 变更时连续止损笔数快照
    total_pos_cheng INTEGER,                      -- 变更时总仓位成数快照
    data_complete   INTEGER NOT NULL DEFAULT 1,   -- 0=数据缺失, 1=完整
    pushed          INTEGER NOT NULL DEFAULT 0,   -- 0=未推 T-01, 1=已推 (失败重试不重复推)
    push_attempted_at TIMESTAMP                   -- 推送尝试时间 (供重试/审计)
);

CREATE INDEX IF NOT EXISTS idx_account_mode_log_ts ON account_mode_log(ts);
CREATE INDEX IF NOT EXISTS idx_account_mode_log_new_mode ON account_mode_log(new_mode);