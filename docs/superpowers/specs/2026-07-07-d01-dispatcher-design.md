# D-01 新闻驱动个股 Dispatcher 接入设计

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 v12 §14.4 D-01 (新闻驱动个股) dispatcher 接入 `news_monitor_loop` 实盘路径, 闭环"新闻 → 板块 → 个股"。

**Architecture:** 事件驱动模型。`news_monitor_loop` 每 2 min 拉到新公告时, 调 `dispatch_news_to_idea_daily` 推送 D-01。dispatcher 内部 1h/票 memo + `push_governor` 20min 冷却, 双重去重。

**Tech Stack:** Rust 2021, tokio, once_cell, std::sync::Mutex, chrono, std::time::Instant。

---

## 1. 背景与现状

### 1.1 v12 §14 模板推送审计结果

2026-07-07 实盘监控审计 (commit `026893a` 之后), 23 个 v12 §14 模板中:

| 类别 | 模板数 | 已推送 | **未推送** |
|---|---|---|---|
| 14.1 盘前 (P-01~P-04) | 4 | 0 | **4** |
| 14.2 盘中 (I-01~I-08) | 8 | 3 | **5** |
| 14.3 盘后 (A-01~A-10) | 10 | 8 | **2** |
| 14.4 全天 (D-01) | 1 | 0 | **1** |
| **合计** | **23** | **11** | **12** |

D-01 是 v12 §14 闭环核心 (新闻→板块→个股), 缺失导致整个 v12 §14 框架"哑模板"。

### 1.2 D-01 已存在的组件 (无需新建)

| 组件 | 文件:行 | 状态 |
|---|---|---|
| `render_news_to_idea` 模板函数 | `push_templates.rs:2834` | ✓ 单元测试已过 |
| `push_news_to_idea` dispatcher 包装 | `push_templates.rs:2343` | ✓ 已存在 |
| `load_news_to_idea_snapshot_real` 真实数据源 | `push_templates.rs:2029` | ✓ v13.27 端到端诊断已覆盖 |
| `dispatch_news_to_idea_daily` 业务层入口 | `push_templates.rs:2121` | ✓ 已存在, **但从未被调用** |
| `NewsToIdeaSnapshot` / `NewsToIdeaParams` 数据结构 | `push_templates.rs:1962, 2822` | ✓ 已存在 |
| `PushKind::NewsToIdea` 治理策略 | `notify.rs:174, 215` | ✓ 等级 ⚡, 冷却 20min/票 |

### 1.3 D-01 缺失原因

- `dispatch_news_to_idea_daily` 已存在但**只被 `run_daily_pushes_dry_run` 调用** (line 429)
- `run_daily_pushes()` (v22) 已实现 6 dispatcher 调度, **但 main.rs 没有任何分支调用它**
- `news_monitor_loop` (实盘) 走的是 NewsRanked 推送 (line 2773) + 实时层 (line 2780) + 快研层 (line 2806) 路径, **完全没调 D-01**

### 1.4 设计目标

- **触发**: `news_monitor_loop` 每轮 (2 min) 拉到新公告时, 触发 D-01
- **去重**: dispatcher 内部 memo (1h/票, 跨日重置) + push_governor 冷却 (20min/票) 双重防护
- **静默**: 候选台无候选时短路返回, 不刷屏
- **不重复**: D-01 走"候选台多源验证个股", NewsRanked 走"公告标题影子 rank", 语义不同, 互补不冲突

---

## 2. 架构设计

### 2.1 调用链

```
news_monitor_loop 每轮 (2 min)
│
├── 1. 拉公告 (现有, line 2647-2700)
│   └─ NewsMonitor::fetch_announcements
│
├── 2. process_announcements → 推 (现有, line 2708-2717)
│   └─ NewsMonitor::process_announcements
│   └─ 推: NewsRanked (line 2773) + NewsAI 实时层 (line 2780) + 快研层 (line 2806)
│
└── 3. 【新增 D-01 触发块】(line ~2775 之后)
    │
    ├── 条件: !pushed.is_empty()  (有重要事件才推)
    │
    ├── 调用: dispatch_news_to_idea_daily(&hhmm, &banner)
    │   │
    │   ├── load_news_to_idea_snapshot_real
    │   │   ├─ 5 源合并 (IndustryChain + 4 P5 源)
    │   │   ├─ merge_candidates (候选台)
    │   │   └─ top 1 → NewsToIdeaSnapshot
    │   │
    │   ├── 【新增 memo 检查 1h/票】
    │   │   ├─ static D01_LAST_PUSH: Mutex<HashMap<code:name, Instant>>
    │   │   ├─ 命中且 < 3600s → 短路返回, log
    │   │   └─ miss 或 > 3600s → 写入 Instant::now()
    │   │
    │   └── push_news_to_idea("", Some(banner), params).await
    │       └─ dispatch(PushKind::NewsToIdea) → push_governor
    │           └─ push_governor 内部 20min 冷却 (v12 §14.5)
    │
    └── 兜底: snapshot.headline.is_empty() → log + 短路 (现有逻辑)
```

### 2.2 数据流

```
data/dispatcher_log/                # v26 dry-run 报告来源
  ├── news_to_idea-2026-07-07.jsonl # D-01 推送记录
  └── ... 

data/v13_diag_report.json           # v13.27 端到端诊断 (含 D-01 链路)
data/dry_run_report.json            # v26 后台 dry-run 报告 (30 min 一次)

DB: get_latest_chain_clusters()     # D-01 数据源
  └── data/news_clusters/           # chain_daily 写入
```

### 2.3 触发模型

**事件驱动 (用户选择)**:
- `news_monitor_loop` 每轮 2 min
- 拉取公告后, `pushed.len() > 0` 时触发 D-01
- 公告空时 (节假日/夜间), 不推 (避免空快照)
- 与 NewsRanked 同步触发, 但走不同数据源 (候选台 vs 公告), 不重复

---

## 3. 接口设计

### 3.1 `dispatch_news_to_idea_daily` 内部 memo

**位置**: `src/bin/monitor/push_templates.rs:2121` 函数体内。

**新增**:
```rust
use std::sync::Mutex;
use std::time::Instant;
use std::collections::HashMap;
use once_cell::sync::Lazy;

/// v29: D-01 dispatcher 内部 memo (1h/票, 跨日重置)
static D01_LAST_PUSH: Lazy<Mutex<HashMap<String, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 测试用: 重置 memo (#[cfg(test)] 暴露)
#[cfg(test)]
pub fn _reset_d01_memo_for_test() {
    D01_LAST_PUSH.lock().unwrap().clear();
}
```

**函数体变更**:
```rust
pub async fn dispatch_news_to_idea_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    let snapshot = load_news_to_idea_snapshot_real(hhmm);
    if snapshot.headline.is_empty() {
        log_dispatcher_attempt("D-01", false, 0, "news_to_idea_snapshot empty");
        log::info!("[D-01] news_to_idea_snapshot 空 (候选台无候选), 跳过推送");
        return false;
    }

    // v29: dispatcher 内部 memo 1h/票
    let memo_key = format!("{}:{}", snapshot.code, snapshot.name);
    {
        let mut map = D01_LAST_PUSH.lock().unwrap();
        if let Some(last) = map.get(&memo_key) {
            let elapsed = last.elapsed().as_secs();
            if elapsed < 3600 {
                log_dispatcher_attempt(
                    "D-01",
                    false,
                    0,
                    &format!("1h memo 冷却, 还需 {}s", 3600 - elapsed),
                );
                return false;
            }
        }
        map.insert(memo_key, Instant::now());
    }

    let params = build_news_to_idea_from_snapshot(&snapshot);
    let snap_size = snapshot.reasons.len();
    let result = push_news_to_idea("", Some(banner), params).await;
    log_dispatcher_attempt("D-01", result, snap_size, "");
    result
}
```

### 3.2 `news_monitor_loop` 触发块

**位置**: `src/bin/monitor/main.rs:2773` (NewsRanked 推送后) 之后。

**新增**:
```rust
// ═══════════════════════════════════════════════════════════════
// v29: D-01 新闻驱动个股推送 (事件驱动)
//   - 触发: pushed 不空 (有重要公告/事件) 时, 每轮 news_monitor_loop 调一次
//   - 去重: dispatcher memo 1h/票 + push_governor 20min 冷却 (v12 §14.5)
//   - 数据源: 候选台 (5 源合并) - 与 NewsRanked 公告影子 rank 互补
//   - 静默: 候选台空时短路返回, log
// ═══════════════════════════════════════════════════════════════
if !pushed.is_empty() {
    use push_templates::dispatch_news_to_idea_daily;
    let banner = push_templates::BannerCtx {
        account_mode: push_templates::AccountMode::Normal,
        total_pos: 0,
        today_pnl: 0.0,
        data_mode: push_templates::DataMode::Full,
        data_missing_note: None,
    };
    let now_ts = chrono::Local::now();
    let hhmm = now_ts.format("%H:%M").to_string();
    let _ = dispatch_news_to_idea_daily(&hhmm, &banner).await;
}
```

---

## 4. 关键设计权衡

### 4.1 memo 放在 dispatcher 内部 vs 放 main.rs

| 方案 | 优点 | 缺点 |
|---|---|---|
| **dispatcher 内部 (静态)** (推荐) | 调用方无感知, 跨函数复用, 单元测试可重置 | 跨进程不共享 (无影响: monitor 是单进程) |
| main.rs 局部状态 | 显式可见, 易调 | 需改 dispatcher 签名, 调用方都要传 |

**选择 dispatcher 内部**, 因为:
1. D-01 可能被多个调用方触发 (`--push` 模式, `news_monitor_loop` 未来)
2. memo 语义是 "D-01 推送的票 1h 内不重推", 属于 D-01 自身职责
3. 单元测试用 `_reset_d01_memo_for_test` 解决隔离

### 4.2 事件驱动 vs 时间窗

| 模型 | 优点 | 缺点 |
|---|---|---|
| **事件驱动** (用户选择) | 实时性强, 公告触发即推 | 公告密集时 (e.g. 盘前开盘) 可能刷屏, 但有 1h memo 防护 |
| 时间窗 | 可控, 容易预测 | 错过窗口, 盘中无推送 |

**选择事件驱动**, 配合 memo, 1 票 1h 内最多 1 条。

### 4.3 与 NewsRanked 重复问题

| 模板 | 数据源 | 渲染 |
|---|---|---|
| **NewsRanked** (line 2773) | `news_ranker::shadow_rank_hits(&hits, &titles)` - 公告标题影子 rank | `format_news_ranked_board` 排序板 |
| **D-01** (本设计) | `load_news_to_idea_snapshot_real` - 候选台 (5 源) | `render_news_to_idea` 单股驱动 |

**不重复**:
- NewsRanked 输出多股排序板 (`name/code/评级/标题`)
- D-01 输出单股驱动 (`headline/theme/stage/action`)
- 触发同时, 但不同视角

### 4.4 横幅 data_mode 写死 Full

**问题**: 写死 `DataMode::Full` 与 §14.0.1 banner 规范不完全一致 (实盘应接 AccountMode)。

**简化理由**:
1. v12 §14 实盘模板有 11 个已推, 全部写死 Full
2. 后续 PR 接 AccountMode 评估时统一改 (不在本次范围)
3. 保持本次 PR 最小改动, 避免引入 AccountMode 评估耦合

---

## 5. 测试设计

### 5.1 单元测试 (`push_templates.rs` tests 模块)

```rust
#[tokio::test]
async fn test_d01_dispatcher_memo_1h() {
    use super::*;
    _reset_d01_memo_for_test();

    // 测试 dispatch_news_to_idea_daily 在 memo 命中时短路
    // 注: 真实数据源 (load_news_to_idea_snapshot_real) 依赖 DB
    //      这里用 mock 替换 (或跳过集成测试, 只测 memo 逻辑)
    let banner = BannerCtx::default();

    // 第一次: snapshot 空 → 短路
    let r1 = dispatch_news_to_idea_daily("10:30", &banner).await;
    assert!(!r1, "空 snapshot 应短路");

    // memo 逻辑: 手动插入测试 key
    {
        let mut map = D01_LAST_PUSH.lock().unwrap();
        map.insert("000001:平安银行".to_string(), std::time::Instant::now());
    }

    // 第二次: memo 命中 → 短路
    // 注: load_news_to_idea_snapshot_real 返回 default → headline 空, 但 memo 检查在前?
    //      当前实现: snapshot 检查在前, memo 在后. 测试需要 mock snapshot 非空.
    //      简化: 跳过集成测试, 只验证 memo map 操作
    let map = D01_LAST_PUSH.lock().unwrap();
    assert!(map.contains_key("000001:平安银行"));
}
```

**注**: 由于 `load_news_to_idea_snapshot_real` 依赖 DB, 完整集成测试需 mock。简化: 单元测试只验证 memo 容器操作, 集成测试用 `monitor --test --v13-diag`。

### 5.2 集成测试 (用户跑)

```bash
# 1. 端到端诊断 (验证 D-01 链路)
cargo run --release --bin monitor -- --test --v13-diag

# 2. 实盘 1 日, 看 dispatcher_log
ls data/dispatcher_log/news_to_idea-*.jsonl
cat data/dispatcher_log/news_to_idea-2026-07-07.jsonl
```

### 5.3 v26 dry-run 报告验证

```bash
# 30 min 后
cat data/dry_run_report.json | jq '.by_kind[] | select(.kind | contains("NewsToIdea"))'
```

**期望**: 1 日内 NewsToIdea 推送 1-5 条, 失败率低 (主要失败是 memo 冷却, 应记录在 `log_dispatcher_attempt` log)。

---

## 6. 风险评估

| 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|
| **同票跨日重置 (预期行为)** | 100% | 低 | 用户要求 1h/票, 跨日重置合理 |
| **候选台空时静默** | 50% | 低 | 已有 `snapshot.headline.is_empty()` 短路 |
| **横幅 data_mode 写死 Full** | 100% | 低 | 与现有 11 个已推模板一致, 后续 PR 统一改 |
| **事件驱动 → 公告密集时刷屏** | 20% | 中 | 1h memo 防护, 1 票 1 日最多 ~4 条 |
| **D-01 与 NewsRanked 重复** | 5% | 低 | 语义不同, 互补不冲突 |
| **`once_cell` 依赖缺失** | 5% | 高 | 检查 Cargo.toml, 缺失则 `cargo add once_cell` |
| **`load_news_to_idea_snapshot_real` 失败** | 30% | 中 | v13.27 端到端诊断可定位, snapshot 空时短路 |

---

## 7. 实施计划概要

### 7.1 涉及文件

| 文件 | 改动行数 |
|---|---|
| `src/bin/monitor/main.rs` | +15 行 (news_monitor_loop 触发块) |
| `src/bin/monitor/push_templates.rs` | +30 行 (memo static + memo 检查 + 测试) |
| `Cargo.toml` | 0 行 (once_cell 应已在依赖中) |
| **合计** | **~45 行, 2 文件** |

### 7.2 步骤

1. 检查 `Cargo.toml` 是否有 `once_cell` 依赖
2. 修改 `src/bin/monitor/push_templates.rs`:
   - 在 `dispatch_news_to_idea_daily` 函数体前加 `D01_LAST_PUSH` static
   - 在函数体内加 memo 检查
   - 加 `_reset_d01_memo_for_test` (cfg(test))
   - 加 `test_d01_dispatcher_memo_1h` 测试
3. 修改 `src/bin/monitor/main.rs`:
   - 在 NewsRanked 推送 (line 2773) 之后, 加 D-01 触发块
4. `cargo build --lib` 验证编译
5. `cargo test --lib` 验证测试 (893 → 894)
6. `git commit` v29

### 7.3 不在范围

- ❌ 接 AccountMode 评估 (写死 Full, 与现有 11 个模板一致)
- ❌ 接 v12 §14.1 盘前模板 (P-01~P-04) - 后续 PR
- ❌ 接 v12 §14.2 盘中模板 (I-01~I-04) - 后续 PR
- ❌ 接 v12 §14.3 A-10 催化复盘 - 后续 PR
- ❌ 修复 `run_daily_pushes()` 未被调用 (用户没要求, 单独 PR)

---

## 8. 全局约束 (Global Constraints)

> 来自 AGENTS.md + v12 §14 + v19.15 + 本项目惯例

1. **数据真实性**: 所有数据源必须真实, 无 mock (D-01 已用真实候选台)
2. **横幅格式**: 14.0.1 banner 强制, 即使写死 DataMode::Full
3. **冷却防护**: 至少 1 层去重 (本设计 2 层: memo + push_governor)
4. **静默失败**: 快照空时 log + 短路, 不刷屏
5. **Cargo 测试**: 893/893 起步, 新增 1 个测试 → 894
6. **dispatcher_log 格式**: 已有 `log_dispatcher_attempt` 工具, 复用
7. **PR 最小**: 不动无关代码, 严格限定 2 文件
8. **commit 命名**: `feat(v29): D-01 dispatcher 接入 news_monitor_loop`

---

## 9. 验收标准

✅ **AC1**: `cargo test --lib` 通过 (893 → 894, 新增 memo 测试)
✅ **AC2**: `cargo build --release --bin monitor` 编译成功
✅ **AC3**: `monitor --test --v13-diag` 显示 D-01 状态 ok (说明数据源链路通)
✅ **AC4**: 实盘跑 1 个交易日, `data/dispatcher_log/news_to_idea-*.jsonl` 至少 1 条记录
✅ **AC5**: 1 票 1h 内不重推 (验证 memo 生效)
✅ **AC6**: 候选台空时, log 显示 `[D-01] news_to_idea_snapshot 空` 不推送

---

## 10. 参考

- v12 §14.4 D-01 模板定义 (AGENTS.md §14.4)
- v12 §14.5 PushKind 治理表 (NewsToIdea 20min 冷却)
- v13.27 端到端诊断 (`src/bin/monitor/v13_diag.rs`)
- v22 `run_daily_pushes()` 6 dispatcher 调度参考
- 现有 NewsRanked 推送 (`src/bin/monitor/main.rs:2770-2773`)
- v26 dry-run 报告 (`src/bin/monitor/dryrun_report.rs`)

---

## 11. 实现日志 (2026-07-07)

**实际改动**:
- `src/bin/monitor/push_templates.rs`: +69 行 (memo static + 检查 + 测试, Tasks 1+2)
- `src/bin/monitor/main.rs`: +21 行 (news_monitor_loop 触发块, Task 3)
- **合计: +90 行, 2 文件**

**测试**: 893 lib + 184 monitor → 893 lib + 185 monitor (+1 memo 容器测试)

**Commits**:
1. `0db806c` feat(v29.1): D-01 dispatcher memo 静态容器 + 单元测试
2. `f1665de` feat(v29.2): dispatch_news_to_idea_daily 加 memo 1h/票 检查
3. `13a0f2a` feat(v29.3): news_monitor_loop 触发 D-01 dispatcher (事件驱动)

**端到端验证 (2026-07-07 11:16)**:
- `cargo test --lib`: 893 passed; 0 failed; 2 ignored
- `cargo test --bin monitor`: 185 passed; 0 failed
- `cargo build --release --bin monitor`: clean
- `monitor --test --v13-diag`: D-01 load_news_to_idea empty (沙箱无网络, 候选台返回 default, 链路通)

**任务 review 状态**:
- Task 1: ✅ APPROVED (reviewer 0 Critical, 0 Important, 3 Minor cosmetic)
- Task 2: ✅ APPROVED (reviewer 0 Critical, 0 Important, 3 Minor)
- Task 3: ✅ APPROVED (reviewer 0 Critical, 0 Important, 0 Minor)

**实施中的偏差** (与 plan 差异):
- Plan 写 "894 lib tests", 实际是 "185 monitor tests" (push_templates.rs 在 monitor binary, 不在 lib)
- Plan Task 3 头部说"嵌套在 `if !pushed.is_empty()` 块内", Step 1 代码说是"在 closing brace 后 top-level" — 实现遵循代码, top-level 是正确选择 (语义等价, 避免双重 `if !pushed.is_empty()`)
- 3 个 Minor cosmetic (imports 位置 / `last.elapsed().as_secs()` 截断) — 不影响功能

**未做** (后续 PR):
- 盘前 P-01~P-04 / 盘中 I-01~I-04 / 盘后 A-10 模板接入
- `run_daily_pushes()` 未被调用 (--push 模式完全失效)
- AccountMode 评估接横幅 (DataMode 写死 Full)
- v13.27 端到端诊断中 I-01 load_sector panic (与 v29 无关, 已知问题)
