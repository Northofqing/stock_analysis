# 2026-08-20 Attribution Research Loop

## BR 注册 (待 grpc WIP 提交后合并进 business_rules.md — spec §8)

- BR-247: 虚拟盘绩效归因日推 (PushKind::AttributionDaily, 每日 15:05, 默认出声)
- BR-248: G5a 盘中异动归因接线 (apply_attribution + alert_log 审计)

> 注: business_rules.md 当前与 grpc WIP (src/data_gateway/, src/grpc_client/,
> src/grpc_server/, build.rs) 纠缠 (spec E8), 本分支不触碰该文件; BR-247/BR-248
> 在本文件注册, 待 grpc WIP 提交后由后续会话合并进 business_rules.md。

## 已确认孤儿 (本分支不修, 留待后续)

- src/review/factor_ic.rs (637 行, 零生产调用者; PushKind::FactorIC 无生产者)
- src/review/failure_attribution.rs (136 行, R-06 设计, 零生产调用者)

## 生产验证清单 (AC)

spec §6 全量 AC 于 2026-08-20 (Task 9) 执行, 结果见下表。AC-A8/A9/A10/B2 依赖
生产环境 15:05 cron / scan_stock 循环, `--test` 不覆盖这些块 (两次独立 review
确认 sibling 块 0 命中), 且当前运行中的生产 monitor 二进制早于本分支代码 —
**AC-A8/A9/A10/B2 无法在部署前于本日满足, 待下一交易日 15:05 生产补验**。

| AC | 命令 | 2026-08-20 结果 | 生产补验证据 (下一交易日 15:05 后) |
|---|---|---|---|
| AC-A1 | `cargo test --lib performance::attribution` | 全绿 (见下方输出) | — |
| AC-A2 | `cargo test --lib performance::snapshot` | 全绿 (见下方输出) | — |
| AC-A3 | `cargo test --lib monitor::attribution` | 全绿 (见下方输出) | — |
| AC-A4 | `cargo build --lib` | exit 0 | — |
| AC-A5 | `cargo build --release --bin monitor` | exit 0 | — |
| AC-A6 | `V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 \| grep -E 'attribution'` | 见下方输出 | — |
| AC-A7 | `cargo test --lib performance::attribution consistency` | 通过 | — |
| AC-A8 | 生产: `ls data/attribution/$(date +%Y-%m-%d).md` | **待补验** | 文件存在 + 日志行 `[attribution] 15:05 归因推送完成` |
| AC-A9 | 生产: `grep -lE '^\[AttributionDaily\]' data/push_log/$(date +%Y-%m-%d)/ \| wc -l` | **待补验** | ≥ 1 (PushKind AttributionDaily) |
| AC-A10 | 生产: `grep -c '"event_type":"push.delivery.audit"' data/event_bus/$(date +%Y-%m-%d).jsonl` 且含 attribution_daily_v1 | **待补验** | > 0 且含 attribution_daily_v1 |
| AC-B1 | `V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 \| grep -E 'G5a'` | 0 命中 (--test 不进 scan_stock 循环, 已记录事实) | `[G5a] attribution failed: ...` warn 仅失败时出现; 成功路径看 AC-B2 审计字段 |
| AC-B2 | 生产 alert 审计: ai_decision 非空 | **待补验** | `reports/alerts/YYYY-MM-DD.jsonl` 含 `attribution_decision` 字段 (成功); 失败时 `[G5a]` warn 出声 |
| AC-B3 | `./target/release/monitor --test 2>&1 \| grep -E 'dispatch_table|DISPATCH'` | 见下方输出 | — |
| AC-B4 | `grep -RInE '"(first|mock|stub|test kept|placeholder|fake|sample)"' data/push_log/$(date +%Y-%m-%d)/` | 0 命中 (见下方输出) | — |

### 生产补验触发条件 (BR-183 注意)

部署本分支 (commit 007cce2) 后 **必须重发 BR-183 activation** — 项目 memory 已确认
activation hash 覆盖全部 `src/**` (非仅 config), 重启不重发激活会静默禁用
NewsAggregator。补验时间: 部署后下一个交易日 15:05 (归因日推) 与盘中 (G5a)。

## 已知限制 (2026-08-20 记录)

- **15:05 块无交易日守卫**: 周末/节假日 15:05 也会执行 (产出空报告推送 + 空 md
  文件); 与 PERF v16.4 兄弟块字节级一致 (brief 强制 verbatim 对齐), 留待后续
  spec 决策。
- **推送失败不重试**: 仅计算失败重试 (30s tick); 推送失败经 outcome 日志 +
  `push.delivery.audit` 出声, 不静默。

## 2026-08-20 Task 9 验证输出

### Layer 1: 模块测试 + build (2026-08-20 实跑)

```bash
$ cargo test --lib performance:: 2>&1 | tail -3
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 2703 filtered out; finished in 0.00s

$ cargo test --lib performance::attribution 2>&1 | tail -3
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 2713 filtered out; finished in 0.00s

$ cargo test --lib performance::attribution::tests::fifo_carries_lot_attribution 2>&1 | tail -3
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2724 filtered out; finished in 0.00s
# AC-A7 一致性锚点: 同日期新模块 realized 合计 == 800.0 (snapshot.rs 已知结果)

$ cargo test --lib performance::snapshot 2>&1 | tail -3
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 2718 filtered out; finished in 0.00s

$ cargo test --lib monitor::attribution 2>&1 | tail -3
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 2714 filtered out; finished in 0.03s

$ cargo build --lib
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.08s   # exit 0
```

### Layer 2: 集成 grep (Completion Rule §2) — 12 命中 (floor ≥3)

```bash
$ grep -RInE 'use stock_analysis::performance::|performance::attribution|PushKind::AttributionDaily' src/bin/monitor/ | wc -l
12

$ grep -RInA2 'PushKind::AttributionDaily' src/bin/monitor/ | head -20
src/bin/monitor/notify.rs:341:            PushKind::AttributionDaily => PushLevel::Info,
src/bin/monitor/notify.rs-342-            // ℹ️参考 (降级 + ForbiddenOps/PaperTrade)
src/bin/monitor/notify.rs:452:            PushKind::AttributionDaily => None,
src/bin/monitor/notify.rs:527:            PushKind::AttributionDaily => "虚拟盘归因",
src/bin/monitor/notify.rs:885:        PushKind::AttributionDaily,
src/bin/monitor/notify.rs-886-        DispatchRow {
src/bin/monitor/presentation_registry.rs:87:        PushKind::AttributionDaily,
src/bin/monitor/presentation_registry.rs-88:        "attribution_daily_dispatcher",
src/bin/monitor/v14_adapter.rs:1177:        PushKind::AttributionDaily => (HoldingHealth, "attribution_daily", Severity::Normal),
src/bin/monitor/br196_test_delivery.rs:690:        PushKind::AttributionDaily,
```

代表命中 (multiline-aware, main.rs 15:05 块 + 2 个 G5a 块):
- main.rs:8372-8408 — `use stock_analysis::performance::attribution::{...}` + `ATTRIBUTION_LAST_RUN` + `compute_daily/compute_window/persist_daily` + `push_governor_v3(&text, PushKind::AttributionDaily, None)`
- main.rs:9192-9204 与 main.rs:9713-9725 — 两处 `stock_analysis::monitor::attribution::apply_attribution(&mut e)` + `[G5a]` warn

### Layer 3: release build + --test 冒烟 (2026-08-20 实跑)

```bash
$ cargo build --release --bin monitor
    Finished `release` profile [optimized] target(s) in 1.15s             # exit 0

$ V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 | grep -E 'attribution|G5a' | head -10
# (0 行 — 事实记录: --test 目录不进入 15:05 cron 与 scan_stock 循环,
#  两次独立 review 已确认 sibling 块同为零命中; AC-A6/B1 证据待生产)
$ grep -cE 'attribution|G5a' /tmp/t9_smoke.log   # 0

$ ./target/release/monitor --test 2>&1 | grep -E 'dispatch_table|DISPATCH' | head -5
[22:36:01 INFO] [v17.x] DISPATCH_TABLE init: 20 rows (Emergency=1 Important=11 Info=8); 逐行 metadata 见运行时 push_governor_inner
# AC-B3 ✔ — 启动 audit 行存在 (AttributionDaily 已在 DISPATCH_TABLE, 单测 dispatch_table_size_is_twenty=20 锁定)

$ grep -RInE '"(first|mock|stub|test kept|placeholder|fake|sample)"' data/push_log/$(date +%Y-%m-%d)/ | wc -l
0   # AC-B4 ✔ — v15 规则: 测试字符串不进生产
```

--test 完整 run 退出码 2 = BR-108/BR-196 门 (`live_acceptance_not_opted_in`, 文档化
预期路径; 目录推送经 dry_run 写入 test 命名空间 push_log, 生产 push_log 未污染)。

### Layer 4: 生产证据

**已推迟 (2026-08-20 记录)**: 生产 AC (A8/A9/A10/B2) 见上表「待补验」列 — 2026-08-20
为交易日但运行中的生产 monitor 早于本分支代码 (commit 007cce2), 未部署前无法产生
生产证据。**补验触发条件: 部署后重发 BR-183 activation (hash 覆盖全部 src/**, 重启
不重发激活会静默禁用 NewsAggregator), 补验时间为部署后下一个交易日 15:05**。

### brooks-test 结果 (RULES-2, 2026-08-20)

Scope: src/performance/attribution.rs (12 tests) + src/performance/report.rs (3 tests)
+ src/bin/monitor/br196_test_delivery.rs (7 tests) — 共 22 tests, 全 unit, 无 mock。
Health Score: **88/100**, 2 Warnings + 2 Suggestions, 无 Critical:

- 🟡 T5 Coverage Illusion — fifo_match 7 个错误分支仅 2 个在模块内覆盖 (oversell /
  missing price); 其余 (identity / timestamp / not-ordered / quantity / non-finite /
  direction) 依赖 sibling snapshot.rs 的等价移植测试兜底 — 移植副本分歧风险无本地
  测试拦截。
- 🟡 T5 Coverage Illusion — compute_window (spec §4.5 修订版核心算法) 零直接单测;
  `sell_date >= start` 窗口过滤分支仅生产 15:05 块可达, --test 也不覆盖。
- 🟢 T3 Test Duplication — 同一 3-fill FIFO 场景在 fifo_carries_lot_attribution 与
  aggregate_families_sums_realized_and_unrealized 重复 (2 处, 本地 fill() helper 已有)。
- 🟢 T2 Brittleness — ddl/persist 两个 SQL 文本断言对格式化变化脆弱 (设计上为无 DB
  幂等锚点, 注释已说明; 属可接受的廉价护栏)。

以上为记录, 本分支不修 (spec §7 范围), 建议后续 spec 评估。完整报告见
.superpowers/sdd/2026-08-20-attribution-research-loop/task-9-report.md。
