-- v12 PR3-3.1 (BR-023/024) — paper_trades / execution_tracking / position_adjustments / chain_name
-- 设计见 v12-trading-assistant-implementation-2026-07-04.md §10
-- 2026-07-05

-- ============ 1. paper_trades ============
-- 虚拟腿只写此表; 真实减仓走 position_adjustments (BR-023)
CREATE TABLE IF NOT EXISTS paper_trades (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id         TEXT NOT NULL,        -- 计划 ID (幂等键)
    code            TEXT NOT NULL,
    name            TEXT NOT NULL,
    direction       TEXT NOT NULL CHECK(direction IN ('buy', 'sell')),
    price           REAL NOT NULL,
    quantity        INTEGER NOT NULL,
    status          TEXT NOT NULL CHECK(status IN ('SignalTriggered', 'Filled', 'NotFilled', 'Invalidated')),
    fill_price      REAL,                 -- 实际成交价 (Filled 时填)
    not_fill_reason TEXT,                 -- NotFilled 时填 (涨停不可买 / 跌停不可卖 / N 分钟未触达)
    virtual_reason  TEXT NOT NULL,        -- 主理由 (BR-016 枚举)
    account_mode    TEXT NOT NULL,        -- 触发时的账户模式快照
    data_mode       TEXT NOT NULL,        -- 触发时的数据模式快照
    ts              TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- partial unique index: 同一 plan_id 只允许 1 条主记录 (幂等)
CREATE UNIQUE INDEX IF NOT EXISTS uniq_paper_trades_plan_id
    ON paper_trades(plan_id);

CREATE INDEX IF NOT EXISTS idx_paper_trades_code ON paper_trades(code);
CREATE INDEX IF NOT EXISTS idx_paper_trades_status ON paper_trades(status);
CREATE INDEX IF NOT EXISTS idx_paper_trades_ts ON paper_trades(ts);

-- ============ 2. execution_tracking ============
-- 跟踪每条建议的执行情况 (T+1/T+3/T+5 实际涨跌)
CREATE TABLE IF NOT EXISTS execution_tracking (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    paper_trade_id      INTEGER NOT NULL,
    plan_id             TEXT NOT NULL,
    code                TEXT NOT NULL,
    expected_price      REAL NOT NULL,
    actual_change_t1    REAL,
    actual_change_t3    REAL,
    actual_change_t5    REAL,
    mfe                 REAL,               -- 最大有利偏移 (PR4 live_plan 用)
    mae                 REAL,               -- 最大不利偏移
    t1_special_case     TEXT,               -- 停牌/涨停/跌停/正常 (BC-3)
    created_at          TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at          TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(paper_trade_id) REFERENCES paper_trades(id)
);

CREATE INDEX IF NOT EXISTS idx_execution_tracking_plan_id ON execution_tracking(plan_id);
CREATE INDEX IF NOT EXISTS idx_execution_tracking_code ON execution_tracking(code);

-- ============ 3. position_adjustments ============
-- 人工确认减仓 + 同日加仓记录 (BR-024)
-- source ∈ {'manual_confirm', 'import'}
-- delta 负数即时生效, 正数 T+1 生效
CREATE TABLE IF NOT EXISTS position_adjustments (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    code            TEXT NOT NULL,
    delta           INTEGER NOT NULL,        -- 正 = 加仓, 负 = 减仓
    source          TEXT NOT NULL CHECK(source IN ('manual_confirm', 'import')),
    reason          TEXT NOT NULL DEFAULT '',
    effective_date  TEXT NOT NULL,           -- ISO date (T+1 生效用)
    applied_immediately INTEGER NOT NULL DEFAULT 0,  -- delta<0 时 = 1 (即时), delta>0 时 = 0 (T+1)
    operator        TEXT,
    created_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_position_adjustments_code ON position_adjustments(code);
CREATE INDEX IF NOT EXISTS idx_position_adjustments_effective ON position_adjustments(effective_date);

-- ============ 4. stock_position ADD COLUMN chain_name ============
-- BR-015 偿还 + BR-022 集中度检查接入
-- 注: SQLite ALTER TABLE ADD COLUMN 不支持 IF NOT EXISTS, 用 add_column_if_missing 包装
ALTER TABLE stock_position ADD COLUMN chain_name TEXT DEFAULT '其他';

CREATE INDEX IF NOT EXISTS idx_stock_position_chain_name ON stock_position(chain_name);

-- ============ 注释 (字段语义) ============
-- paper_trades.virtual_reason: VirtualReason 枚举值 (Breakout/VolumeSurge/MainNetInflow/SectorLeader/NewsCatalyst/AuctionAnomaly, BR-016)
-- paper_trades.status:
--   SignalTriggered: 信号触发, 待模拟成交
--   Filled: 模拟成交 (fill_price 必填)
--   NotFilled: 未成交 (not_fill_reason 必填: 涨停不可买 / 跌停不可卖 / N分钟未触达)
--   Invalidated: 失效 (基本面/排雷命中)
-- paper_trades.account_mode/data_mode: 触发时的快照, 用于 audit 分析
--
-- position_adjustments.delta:
--   负数: 减仓, applied_immediately=1, available_shares() 立即减
--   正数: 加仓, applied_immediately=0, available_shares() T+1 增 (effective_date <= today 才计入)
--
-- stock_position.chain_name: 用于 BR-015 集中度检查 (同 chain 持仓总市值不能超 single_sector_max)