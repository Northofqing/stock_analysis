-- v10 P0.1 (G0) — prediction_tracker 回滚 12 列
-- 注意: SQLite 不支持 DROP COLUMN 直接删 (3.35+ 支持, 但旧版本需 recreate table)
-- 实际回滚见 tools/one_shot/migrate_rollback_v10_p0_1.sh
-- 2026-07-01

-- 方案 A: SQLite 3.35+ DROP COLUMN (推荐, 干净)
ALTER TABLE prediction_tracker DROP COLUMN reason;
ALTER TABLE prediction_tracker DROP COLUMN reason_secondary;
ALTER TABLE prediction_tracker DROP COLUMN actual_change_t1;
ALTER TABLE prediction_tracker DROP COLUMN actual_change_t3;
ALTER TABLE prediction_tracker DROP COLUMN actual_change_t5;
ALTER TABLE prediction_tracker DROP COLUMN hit_t1;
ALTER TABLE prediction_tracker DROP COLUMN hit_t3;
ALTER TABLE prediction_tracker DROP COLUMN hit_t5;
ALTER TABLE prediction_tracker DROP COLUMN market_up_rate_t1;
ALTER TABLE prediction_tracker DROP COLUMN market_up_rate_t3;
ALTER TABLE prediction_tracker DROP COLUMN market_up_rate_t5;
ALTER TABLE prediction_tracker DROP COLUMN t1_special_case;
