-- v10 P0.1 (G0) — prediction_tracker 加 12 列
-- 落 BR-016/017/020 (v10 §10.4 已登记)
-- 2026-07-01

-- 1+1 = reason / reason_secondary (主/副理由, 枚举, v10 §10.3 VirtualReason)
ALTER TABLE prediction_tracker ADD COLUMN reason TEXT;
ALTER TABLE prediction_tracker ADD COLUMN reason_secondary TEXT;

-- 3 = actual_change_t1/t3/t5 (T+1/T+3/T+5 实际涨跌幅, BC-3)
ALTER TABLE prediction_tracker ADD COLUMN actual_change_t1 REAL;
ALTER TABLE prediction_tracker ADD COLUMN actual_change_t3 REAL;
ALTER TABLE prediction_tracker ADD COLUMN actual_change_t5 REAL;

-- 3 = hit_t1/t3/t5 (三窗口命中布尔, BC-3)
ALTER TABLE prediction_tracker ADD COLUMN hit_t1 INTEGER;
ALTER TABLE prediction_tracker ADD COLUMN hit_t3 INTEGER;
ALTER TABLE prediction_tracker ADD COLUMN hit_t5 INTEGER;

-- 3 = market_up_rate_t1/t3/t5 (同日同窗市场基准, BC-1, Q2=B 全市场上涨家数占比)
ALTER TABLE prediction_tracker ADD COLUMN market_up_rate_t1 REAL;
ALTER TABLE prediction_tracker ADD COLUMN market_up_rate_t3 REAL;
ALTER TABLE prediction_tracker ADD COLUMN market_up_rate_t5 REAL;

-- 1 = t1_special_case (停牌/涨停/跌停/正常, BC-3)
ALTER TABLE prediction_tracker ADD COLUMN t1_special_case TEXT;

-- 注释 (v10 §5.2 字段语义):
--   reason / reason_secondary: VirtualReason 枚举值 (Breakout/VolumeSurge/MainNetInflow/SectorLeader/NewsCatalyst/AuctionAnomaly)
--   actual_change_tN: 实际涨跌幅 = (next_N_day_close - buy_price) / buy_price * 100
--   hit_tN: 1 = hit, 0 = miss, NULL = N/A (停牌/数据缺失)
--   market_up_rate_tN: 当日上涨家数 / 全市场 ~5400 = real_alpha 对标基准
--   t1_special_case: 'suspended' / 'limit_up' / 'limit_down' / 'normal'
