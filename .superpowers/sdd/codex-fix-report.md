# Codex 第二轮 Review Fix Report

Date: 2026-06-29
Review source: Codex CLI 第二轮 review on v9.2 process discipline commit (e7cdc15)
Findings fixed: 4 (F1 P1, F2 P1, F3 P2, F4 P2)

---

## F1 (P1) — BR-001 calendar days documentation fix

**Decision:** Option C (YAGNI, accept calendar-day reality, document the deviation).
引入 trading-day 计算需要 holiday 数据, 复杂度与当前 BR-001 业务价值不匹配 —
周末不推送, 跳过周期可接受。

**Files changed:**
- `docs/business_rules.md`: BR-001 描述从 "近 3 个交易日" 改 "近 3 个日历日", 加 YAGNI 说明
- `src/opportunity/discover.rs:116-117`: `is_recently_pushed` 注释同步更新
- `tests/e2e_dedup.rs:1`: 文件头注释同步

**Implementation:** 未改 (按 YAGNI 选项 C), `count_recent_pushes` 继续用 `chrono::Duration::days(3)`。

---

## F2 (P1) — e2e TST00x → TEST_CODE_xxx prefix rename

**Why critical:** `src/risk/env_guard.rs:21` 检查 `code.starts_with("TEST_CODE")`.
TST00x 不识别, 测试残留票可能进入生产路径, 违反 AGENTS §2.5 测试隔离。

**Files changed:**
- `tests/e2e_prediction_verify.rs`: TST001→TEST_CODE_001 (13 处), TST002→TEST_CODE_002 (12 处), 头部注释更新
- `tests/e2e_dedup.rs`: TST002→TEST_CODE_002 (5 处), 头部注释更新

**Verified clean (no TST/TEST_CODE refs):**
- `tests/chain_exclusive.rs` (业务纯函数测试, 无 DB)
- `tests/flash_filter.rs` (仅断言标题不含宏观关键词, 无 DB)
- `tests/test_data_freshness_check.rs` (用 temp file, 不涉及股票代码)
- `tests/test_design_contradiction.rs` (不涉及)

**Also added:** `use diesel::RunQuerySql;` import in e2e_dedup.rs (新增 cleanup 块需要).

---

## F3 (P2) — discover.rs ScoreBreakdown comment math correction

**Bug:** 注释写 "拆分后链路分最大 24 → Rule 24, AI 20, 差 4 分",
但 ScoreBreakdown 实际是 source(10) + keyword(9) + fund(10) + position(5) = **34 max**.
边界证明与代码不符, 违反 AGENTS §2.9 纪律。

**File changed:** `src/opportunity/discover.rs:42-48`

**New comment (correct math):**
```
Rule max      = 10 + 9 + 10 + 5  = 34
AI max        =  6 + 9 + 10 + 5  = 30
AI 降级 max   =  0 + 9 + 10 + 5  = 24
Rule vs AI 差 4 分 (逻辑硬度项)
```

巧合: 之前的"差 4 分"结论仍正确 (10-6=4), 仅"24 / 20"边界值错。

---

## F4 (P2) — e2e_dedup cleanup block

**Bug:** e2e_dedup 在 line 24 save_prediction("TEST_CODE_002") 但测试末尾没 DELETE.
prediction_tracker 是 OnceCell 全局 (test_data/test.db), 后续测试运行会被污染。

**Fix:** 加 cleanup 块 (e2e_dedup.rs:46-49):
```rust
let _ = diesel::sql_query("DELETE FROM prediction_tracker WHERE stock_code = 'TEST_CODE_002'")
    .execute(&mut *db.get_conn().unwrap());
```

**Note:** 同时 (F2) 引入了 `diesel::RunQueryDsl` import。

---

## Verification

```
cargo test --lib --test e2e_prediction_verify --test e2e_dedup --test chain_exclusive --test flash_filter
→ 459 lib tests pass, 4 e2e tests pass, 0 failures

bash tools/compliance/check.sh
→ ALL CHECKS PASSED
  - check_fake_impl: PASS
  - check_data_freshness: PASS (stock_daily 滞后 0 天)
  - check_design_contradiction: PASS
  - check_business_rules: PASS (warn: 0)
```

---

## Deviations

无 deviation — 按用户 brief 指定方案实施。

## Out of scope (per brief)

- search_service/* (用户 WIP)
- main.rs verify_predictions 之外的逻辑
- P3 findings (sync DB I/O, chain_mapper.rs:108 dead code, e2e_dedup retry_db)