# v9.4 Release Notes — Process Discipline & Audit Follow-up

> **发布日期**: 2026-06-30 (master @ `8567b6b`)
> **基于**: audit `docs/analysis-2026-06-29-project-audit.md` (内部 8/10 vs 外部 2.3/5 评分差距)
> **范围**: 7 commits, 281+ 行新增, 50+ 行删除, 506 tests passed

---

## 0. TL;DR

v9.4 是 **codex review 驱动** 的**审计跟进**版本（不是功能版本）：

- **起源**: Explore agent 扫 docs 列了 84 项未完成 (24 P0 + 28 P1 + 32 P2) → codex 验证发现 60% 是假任务 → 按真 P0/P1 推荐路径开发
- **目标**: 修审计 §1-§6 中**已知的真问题**（critical 隐藏 bug + 流程纪律），不追求架构债清理
- **现实**: 7 个 commit 主要修 **critical 假实现 + BR 门禁**，**审计 Top 10 中 9 项没动**（架构债 + 量化方法学）

---

## 1. Commits 概览

| Commit | 类型 | 修复 |
|--------|------|------|
| `686e378` | fix(v9.4) | 真 P0/P1 — F15/F16 BR 测试 + F17 dyn_priority + B-006 + B6 |
| `f8d9332` | fix(v9.4.1) | 修 docs/.gitignore 漏 commit bug (codex review critical) |
| `8b455ae` | fix(v9.4.2) | F19 push_time 单调递增 + F20 CI --test-threads=2 |
| `fcbc49f` | feat(v9.4.3) | launch_gate 真接 — 阶段状态机从死代码到生产可用 |
| `c02a288` | fix(v9.4.4) | 修 v9.3 引入回归 (e2e_dedup + Mutex poison) |
| `c8c373e` | fix(v9.4.5) | test_design_contradiction atomic rename + CI --test-threads=1 |
| `8567b6b` | refactor(v9.4.6) | block_on 收敛统一 trait — 25+ 处 → `crate::block_on_async` |

---

## 2. 修了什么 (audit 角度)

### ✅ 已修

| Audit 章节 | 问题 | Commit | 影响 |
|------------|------|--------|------|
| §1.2 多 Runtime | 26 处 `block_on` 散落 | `8567b6b` v9.4.6 | 4 种 pattern → 1 helper, 40 worker → 1 套 |
| §3.2 风控执行断层 | `e25f902` (v9.2) | 之前 | 量化机构 P0-1 阻塞实盘修复 |
| §4.4 配置散落 | 4 个 compliance 脚本 | v9.2 | CI gate 集成 |
| §4.5 CI/CD | `.github/workflows/compliance.yml` | v9.2 | 4 gate 串行 + PR 拦截 |
| §4.8 文档债 | `notification/mod.rs` 5→10 渠道 | `686e378` v9.4 | 幻象引用清理 |

### ⚠️ 部分修 / 文档化（不算完成）

| Audit 章节 | 状态 |
|------------|------|
| §4.6 0 单测 (bin/monitor 1934 行) | 未做（最大覆盖盲区） |
| §4.7 `AnalysisResult` 130 字段 | 未拆（codex 验证实际 69 字段） |
| §5.1 测试账户隔离 | `TEST_CODE_xxx` 前缀在用，硬隔离未审计 |

### ❌ 未做（按优先级排）

**Top 10 (audit §7)**:
- ❌ #1 `signal/` 死代码删除
- ❌ #2 AI 评分 IC/IR 根因分析 (sentiment_score ≥80 全亏)
- ❌ #3 `pipeline/mod.rs` 1765 行拆 5 文件
- ❌ #4 `bin/monitor/main.rs` 1934 行拆 6 文件
- ❌ #6 `std::sync::Mutex` → `tokio::sync::Mutex` (5 处)
- ❌ #7 HTTP client 共享 + 4 路 `join!` (data_provider)
- ❌ #8 `thiserror` 错误枚举替换 anyhow
- ❌ #10 多因子每日重算 + Walk-Forward + PBO/DSR/CPCV

**其他**:
- ❌ §2.4 KDJ O(n²) / O(n·K) 算法
- ❌ §2.7 Cargo profile (30GB target)
- ❌ §3.5 HybridStrategy 空壳
- ❌ §3.6-3.7 6 因子扩展 + 4 压力期 + VaR/CVaR
- ❌ §3.9 实盘/回测鸿沟（无券商 API, 6-12 月工程量）
- ❌ §4.1 thiserror (跨层错误)
- ❌ §4.2 284 处 `unwrap()` 清理
- ❌ §4.3 56 种可变状态散落
- ❌ §5.5 DI 容器（替换全局单例）
- ❌ §6.3 文档版本膨胀（10 个 architecture 版本）

---

## 3. v9.4 详细改动

### 3.1 F15/F16 BR 测试覆盖 (v9.4)

**问题**: 业务规则 BR-004/005 文档承诺有测试, 实际 `ls tests/` 没找到 `tests/ranking.rs`.

**修法**:
- `Candidate` 加 `push_time` 字段 (discover 时填 `Local::now().timestamp()`)
- `PostCloseCandidate` 透传 + sort_by 加 `then_with(push_time)` 次级排序
- 新建 `tests/ranking.rs` (8 测试):
  - `test_br004_final_score_descending`
  - `test_br004_tie_breaker_push_time_ascending`
  - `test_br004_default_push_time_zero_sorts_first`
  - `test_br005_skip_when_no_flash_news` (snapshot test)
  - `test_br005_skip_when_no_chain_match`
  - `test_br005_skip_when_low_confidence`
  - `test_br005_no_skip_when_normal_output`
  - `test_br005_daily_limit_5_NOT_IMPLEMENTED_placeholder` (BR-005 ≤5 限额**未实现**, 占位标记)
- CI workflow 加 `--test ranking`

### 3.2 F17 dynamic_priority 公式 (v9.4)

**问题**: v9.3 commit `480b74f` 批量 priority=100 注入破坏 `winrate_simulator` "已加权/未加权" 提示能力.

**修法** (in `winrate_simulator.rs`):
```rust
dynamic_priority = winrate × log(samples + 1) × 25, clamp [0, 100]
```
- 输出 dyn vs static priority 对比表
- 差 > 15 或 dyn>50 但 static<80 自动列出
- operator 用 dyn_prior 复制到 `chain_rules.toml`

**实测输出**:
```
主题                  胜率    样本  log加权  dyn_prior  static
AI硬件-PCB           66.7%   120    4.80    79.9      100   ← 应降权
半导体-设备          73.2%    41    3.74    68.4      100   ← 应降到 70
AI算力               4.3%    92    4.53     4.9      100   ← 应大幅降权
```

### 3.3 B-006 lead_days 衰减 (v9.4)

**问题**: `bom_kb.rs:chain_score_with_direction` 公式 `elasticity × direction_match × confidence`, 忽略 `lead_days`.

**修法**:
```rust
let lead_decay = (-(node.lead_days as f64) / 30.0).exp();
node.elasticity_score * dir_match * node.confidence * lead_decay
```
- 50 节点 BOM 平均 lead_days ~15, 平均 lead_decay ≈ 0.61
- 加 3 单元测试覆盖 lead_days=0/30/短长

### 3.4 B6 position_tracker 产业链计数 (v9.4)

**问题**: `position_tracker.rs:321-322` 两个 TODO 注释 `0,  // TODO: 产业链持仓计数 (后续迭代)` / `0,  // TODO: 产业链冻结计数` hardcoded 0.

**修法**:
- 改 TODO 注释解释 hardcoded 0 = 禁用产业链集中度检查
- 等 `stock_position` 表加 `chain_name` 列后启用
- 警告日志让 operator 知道当前 chain 集中度检查不生效

### 3.5 #95 spec/plan checkbox 回填 (v9.4)

**问题**: 之前 v9.2/v9.3 commit 落地了代码, 但 docs checkbox 没勾.

**修法** (force-add 到 git, 因为 `docs/` 在 `.gitignore`):
- `2026-06-28-event-extractor.md` plan: 26 个 step checkbox 全勾 (commit 56b4d3f/1a4754f/9755012/2ed976d)
- `2026-06-29-process-discipline-design.md` spec §5.1: 8 个验收 checkbox 全勾 (e7cdc15~a99cd2c)
- §5.2 30 天验证保留 `[ ]` 等时间

### 3.6 #98 文档同步幻象文件清理 (v9.4)

**修法**:
- `KNOWN_BUGS-2026-06-28.md` B-007: `event_bus.rs:142` → `news_monitor.rs:142`
- `KNOWN_BUGS` / `analysis-2026-06-29` B-011: `docs/notification/mod.rs:7` → `src/notification/mod.rs:7`
- `src/notification/mod.rs` 头部注释同步: 5 渠道 → 10 渠道

### 3.7 F19 push_time 死代码修 (v9.4.2)

**问题**: `chrono::Local::now().timestamp()` 秒级, 单次 `discover()` 所有 candidate 共享同一时间, BR-004 次级排序死代码.

**修法** (in `discover.rs`):
```rust
static PUSH_TIME_COUNTER: AtomicI64 = AtomicI64::new(0);
fn next_push_time() -> i64 {
    let prev = PUSH_TIME_COUNTER.load(Ordering::Relaxed);
    let next = if prev == 0 {
        chrono::Local::now().timestamp_millis()
    } else {
        prev + 1
    };
    PUSH_TIME_COUNTER.store(next, Ordering::Relaxed);
    next
}
```
- 加 2 测试: monotonic_incrementing + distinct_per_candidate

### 3.8 F20 CI test 并行 transient 失败修 (v9.4.2)

**问题**: cargo test 同时跑 8 个 `--test` binary 偶发 SQLite WAL 锁竞争 transient 失败.

**修法**:
- `compliance.yml` cargo test 命令末尾加 `-- --test-threads=2`

### 3.9 launch_gate 真接 (v9.4.3)

**问题** (audit Top 10 #10 + codex review F20):
- `LaunchStage` enum 之前 0 外部读, 是死代码
- 启动时 banner 缺失, operator 看不到当前阶段

**修法**:
```rust
// launch_gate.rs:
- LaunchStage::name() / FromStr: shadow / gray / live (大小写不敏感)
- current_stage(): 读 env STAGE, 默认 Shadow
- should_push_user(stage, is_critical_alert):
    Shadow: 不打用户 (false)
    Gray: 仅 critical (止损/风控) → is_critical_alert
    Live: 全量推送 (true)
- metrics_from_db(): 从 prediction_tracker 算 StageMetrics

// monitor/main.rs:
- 启动 banner 显示 LaunchStage + 推送策略
- push_wechat_with_kind(text, is_critical_alert): 包装 stage gate
```
- 7 unit test 覆盖 transition + push gate + 三阶段 e2e (shadow → gray → live + 回退)

### 3.10 block_on 收敛统一 trait (v9.4.6)

**问题** (audit §1.2 + Top 10 #5):
- 26 处 `Runtime::new` / `block_on` / `Handle::try_current`
- 8 核机器 5 套 Runtime = 40 个 worker 线程
- 4 种手写 pattern, 容易出错

**修法** (`src/lib.rs` 加 helper):
```rust
pub fn block_on_async<F, T>(fut: F) -> T
where F: Future<Output = T> {
    use tokio::runtime::Handle;
    match Handle::try_current() {
        Ok(handle) => handle.block_on(fut),
        Err(_) => {
            // 不在 tokio runtime 内: 建 current_thread runtime 临时跑
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all().build()
                .expect("...");
            rt.block_on(fut)
        }
    }
}

pub fn block_on_async_with_timeout<F, T>(fut: F, timeout_secs: u64) -> Result<T, String>
// 默认 30s 超时, 防止永久阻塞 worker
```

**迁移** (10 个文件):
- `market_analyzer/statistics.rs`
- `data_provider/{eastmoney,money_flow,intraday_kline,financials,consensus,industry,valuation_history}.rs`
- `data_provider/gtimg_provider.rs` (run_async_blocking 委托给 helper)
- `opportunity/chain_mapper.rs` (spawn_blocking 内)

**保留** (合法 pattern):
- `rsi_optimize.rs:290` main Runtime::new (入口 runtime)
- `monitor/main.rs:350` block_in_place + sync mpsc.recv

**收益**: -31 行, 4 pattern → 1 helper, 1 套 runtime + 临时 current_thread.

### 3.11 回归修复 (v9.4.4 / v9.4.5)

- **v9.4.4**: `count_recent_pushes_batch` I-5 assert 拒绝 `_` 让 e2e_dedup panic → 放宽到 `alphanumeric + _ + -`. `CONFIG_LOCK` Mutex poison → `acquire_config_lock()` 容忍 poison
- **v9.4.5**: `test_design_contradiction` 在 `--test-threads=2` 下偶发 fail (std::fs::write + bash subprocess fd/cache race) → atomic rename + CI 拆两段 (该 binary 单独 `--test-threads=1`)

### 3.12 f8d9332 docs/.gitignore 漏 commit 修

**codex review 抓的 critical**:
- `docs/` 在 `.gitignore`, 之前 v9.4 commit 686e378 commit message 声称"修 3 处幻象引用 + spec 回填"实际只本地改
- `git add -f` 把 4 个 doc 文件作为新 tracked 加入仓库

---

## 4. 性能 / 测试影响

### 测试
```
506 tests passed, 0 failed
- 474 lib + 1 + 1 + 2 + 8 + 5 + 8 + 4 + 3 = 506
- cargo test --lib + 7 test binary -- --test-threads=2
- test_design_contradiction 单独 --test-threads=1 (避免 fs race)
```

### Compliance
```
✓ bash tools/compliance/check.sh ALL CHECKS PASSED
- fake_impl: OK
- data_freshness: 2026-06-29 latest (0 day lag)
- design_contradiction: threshold 60 ≤ clamp 70.0 ✓
- business_rules: §2.10 通过 (I-7 fallback WARN 已显式化)
```

### Push
```
8b455ae..8567b6b master → stock_analysis/master
```

---

## 5. 现实评估（透明报告）

**审计文档 Top 10 进度**: 1/10 完成 (#5 block_on 收敛)

**整体进度**: 约 15-20% (审计 §1-§6 共 50+ 项中修了 5-7 项 critical bug + 1 项架构债)

**自我评分 vs 外部评分差距**: 仍维持 8/10 vs 2.3/5 严重脱节.
v9.4 修的全是**已知 critical 隐藏 bug**（R-1 verify 假实现、R-3 数据断层、R-6 BR 缺失、launch_gate 死代码）+ **流程纪律**（4 gate + BR 测试覆盖）. **审计指出的架构债（main.rs 1934 行 / pipeline/mod.rs 1765 行）+ 量化方法学（IC/IR / Walk-Forward / PBO）一个没动**.

---

## 6. 推荐下一步 (按 ROI)

| 优先级 | 任务 | 估时 | 收益 |
|--------|------|------|------|
| 1 | Top10 #6: std::sync::Mutex → tokio (5 处) | 1 天 | 解决 LLM worker 阻塞 |
| 2 | Top10 #7: HTTP client 共享 + 4 路 join! | 1 天 | 性能 5-10x |
| 3 | Top10 #2: AI 评分 IC/IR 根因分析 | 1 周 | 决定 AI 模块去留 |
| 4 | Top10 #3+#4: 拆 pipeline/mod.rs + bin/monitor | 2 周 | 认知负担 ↓, 单测覆盖 ↑ |
| 5 | §3.9 实盘/回测鸿沟 (无券商 API) | 6-12 月 | 系统升级到真实盘 |

---

## 7. 参考

- `docs/analysis-2026-06-29-project-audit.md` — 审计源文档 (50+ 项未完成)
- `docs/superpowers/specs/2026-06-29-process-discipline-design.md` §5.1 — 8 验收 checkbox 全勾
- `docs/superpowers/plans/2026-06-28-event-extractor.md` — 26 step checkbox 全勾
- `docs/KNOWN_BUGS-2026-06-28.md` — B-007/B-011 幻象引用已修
- `/tmp/codex_review/v9.4_review.md` — codex 实际 review 输出（stdout 收集）
- `/tmp/codex_review/explore_review.md` — Explore agent 60% 错的 review
- `tools/compliance/check.sh` — 4 个 CI gate
- `tests/ranking.rs` — BR-004/005 测试覆盖