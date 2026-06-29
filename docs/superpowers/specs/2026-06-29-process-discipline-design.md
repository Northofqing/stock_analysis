# Process Discipline Design — 2026-06-29

## 1. 背景与动机

### 1.1 现状问题

经过 2026-06-29 对当前系统的根因调查，发现 7 个独立 bug 漏过了完整的开发流程（设计 → 四角挑战 → 计划 → 编码 → review → 测试）。bug 类型横跨"假实现""设计矛盾""数据断层""业务规则缺位"4 大类。

具体 bug 清单（详见 `docs/KNOWN_BUGS-2026-06-28.md`）：

| 编号 | 类型 | 描述 | 严重度 |
|------|------|------|--------|
| R-1 | 假实现 | `verify_predictions` 硬编码 0.0, false，胜率统计是空壳 | P0 致命 |
| R-2 | 设计矛盾 | 推送门 75 > NS3 评分封顶 70，沙盘阶段不可达 | P0 致命 |
| R-3 | 数据断层 | `stock_daily` 停更 6-15，但推送从 6-16 开始 | P0 致命 |
| R-4 | 业务规则缺位 | 600522 被推 33 次，无去重逻辑 | P1 严重 |
| R-5 | 业务规则缺位 | 关键词规则"一词多义"过载，无互斥 | P1 严重 |
| R-6 | 业务规则缺位 | 宏观新闻混入 chain_mapper，无过滤 | P1 严重 |
| R-7 | 设计矛盾 | 评分权重"链路分 47% 含常量"，无边界证明 | P1 严重 |

### 1.2 根因（why 这些 bug 没被识别）

| 根因 | 占比 | 描述 |
|------|------|------|
| 测试只验证"语法"不验证"语义" | 主导 | `verify_predictions` 函数体 100% 覆盖，但实际是空壳 |
| 时间维度集成测试缺位 | 重要 | "今天推→明天收→后天 verify" 跨日链路没 e2e |
| 设计文档与代码不同步 | 重要 | spec/config 改动无双向引用机制 |
| AGENTS.md 条款未严格执行 | 关键 | §2.1 禁 mock / §2.7 留痕 都被绕过 |

### 1.3 设计目标

建立**可执行 + 可自动门禁 + 可持续**的开发纪律，使得：
- 任何"假实现"在 merge 前被拦截
- 任何"设计矛盾"在 merge 前被拦截
- 任何"数据契约断裂"在 CI 上报警
- 任何"业务规则缺位"在 review 表里必查

---

## 2. 设计

### 2.1 总览：AGENTS.md 新增 §2.8 / §2.9 / §2.10 + 自动化门禁脚本

```
AGENTS.md (新增 3 条款)
  ↓
  规则
  ↓
tools/compliance/check.sh (bash 主入口)
  ↓  ┌─ lib/check_fake_impl.sh          (§2.8 拦截假实现)
     ├─ lib/check_design_contradiction.sh (§2.9 拦截设计矛盾)
     ├─ lib/check_data_freshness.sh     (§2.4 拦截数据断层)
     └─ lib/check_business_rules.sh     (§2.10 拦截业务规则缺位)
  ↓
GitHub Actions .github/workflows/compliance.yml
  ↓
PR 合并阻断
```

### 2.2 AGENTS.md 新增条款

#### §2.8 假实现禁令（防 R-1 / R-3）

**MUST** 任何"写数据 / 验证 / 通知 / 同步"类函数（命名含 `verify`、`save`、`notify`、`push`、`sync`、`update_result`、`reconcile`）必须真实操作目标数据源。仅写日志不操作数据的实现视为 **假实现**，合并阻断。

**反模式**：
```rust
// ❌ 假实现
match db.update_prediction_result(&today, None, 0.0, false) { ... }
```

**正例**：
```rust
// ✅ 真实 verify
let next_close = stock_daily::fetch_close(stock_code, target_date).await?;
let actual = (next_close - prev_close) / prev_close;
db.update_prediction_result(&today, Some(code), actual, hit)?;
```

**验证手段**：
- `grep` 模式 `update_.*result.*0\.0.*false` → 命中即 fail
- 必须存在 `tests/e2e_prediction_verify.rs` 之类的 e2e 测试

#### §2.9 设计矛盾禁令（防 R-2 / R-7）

**MUST** 任何评分 / 阈值 / 门控的设置必须满足：
1. 上下游互相引用：改动 `config/*.toml` 阈值必须 PR 描述引用 spec 章节号；改动 spec 必须引用 config 字段名
2. 边界证明：`event_*_threshold`、`*_max`、`*_min`、`*_clamp` 必须注释证明"为什么是这个值"
3. 矛盾检测：CI 解析 toml 与 rust 源码，若 `threshold > clamp_max` 即 fail

**反模式**（R-2）：
```toml
[push]
event_risk_score_threshold = 75     # 推送门
```
```rust
if inputs.winrate_score.is_none() {
    event_risk_score_clamped = event_risk_score_clamped.min(70.0);  // 封顶 70
}
// 沙盘阶段 winrate=None → 永远 ≤70，根本过不了 75 门 → 矛盾！
```

**验证手段**：
- 解析 `config/*.toml` 提取 `event_*_threshold` 类
- 解析 `src/**/*.rs` 提取 `clamp(N)` / `min(N)` 临近 event_risk 上下文
- 若 threshold > clamp_max，CI fail
- PR 描述必须含 `Refs: spec §X.X` 或 `Refs: config XXX`

#### §2.10 业务规则文档化（防 R-4 / R-5 / R-6）

**MUST** 涉及"去重 / 互斥 / 过滤 / 排序 / 限额"的业务规则必须在 `docs/business_rules.md` 列清单。每条规则包含：编号、规则描述、对应代码位置、测试位置、最后审核日期。

**MUST** 任何新代码涉及上述类别，必须先在 `business_rules.md` 登记再写实现。

**MUST** Review 检查表第 5 步加项："本 PR 涉及的 5 类业务规则是否登记"。

**5 类规则初始登记**（R-4 / R-5 / R-6 对应）：

| 编号 | 类别 | 规则 | 代码位置 |
|------|------|------|---------|
| BR-001 | 去重 | 同一只票近 3 个交易日最多推送 1 次 | `src/opportunity/discover.rs` |
| BR-002 | 互斥 | 一条快讯最多命中 1 条产业链。例外：AI 推理明确给出 ≥2 条**独立**产业链（chain 名之间无包含关系、关键词不重叠），可保留。 | `src/opportunity/chain_mapper.rs` |
| BR-003 | 过滤 | 宏观新闻（美联储/美股/汇率/大宗）入 macro 通道，不入 chain_mapper | `src/search_service/service.rs` |
| BR-004 | 排序 | 推送 TopN 按 final_score 降序，同分按发布时间升序 | `src/opportunity/mod.rs` |
| BR-005 | 限额 | 每天推送机会数 ≤ 5，超过入候选池 | `src/bin/monitor/main.rs` |

### 2.3 自动化门禁脚本

#### 目录结构

```
tools/compliance/
├── check.sh              # 主入口
├── lib/
│   ├── check_fake_impl.sh
│   ├── check_design_contradiction.sh
│   ├── check_data_freshness.sh
│   └── check_business_rules.sh
└── README.md
```

#### check.sh 主入口

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
FAIL=0
for f in "$SCRIPT_DIR"/lib/check_*.sh; do
  echo "── $(basename "$f") ──"
  if ! bash "$f"; then FAIL=1; fi
done
[ "$FAIL" -eq 0 ] && echo "✓ compliance check pass" || { echo "✗ compliance check fail"; exit 1; }
```

#### check_fake_impl.sh（拦 R-1）

检查项：
1. `grep` 硬编码 `update_prediction_result.*0\.0.*false` → 命中 fail
2. `verify` 类函数必须含 DB/网络/文件操作（不能只 log）
3. 必须存在 `tests/e2e_prediction_verify.rs`
4. `notify` / `push` 类函数必须含真实网络/DB 调用

#### check_design_contradiction.sh（拦 R-2 / R-7）

检查项：
1. 解析 `config/opportunity.toml` 提取 `event_risk_score_threshold`
2. 解析 `src/opportunity/score.rs` 提取 `min(N.0)` / `clamp(N)` 在 event_risk 上下文
3. 比较：`threshold > clamp_max` → fail
4. 评分权重含未解释的常量项 → warn

#### check_data_freshness.sh（拦 R-3）

检查项：
1. `sqlite3` 查询 `SELECT MAX(date) FROM stock_daily`
2. 与 today 比较，超过 1 个交易日 → fail

#### check_business_rules.sh（拦 R-4 / R-5 / R-6）

检查项：
1. `docs/business_rules.md` 必须存在
2. 5 类规则（去重/互斥/过滤/排序/限额）必须全有
3. 关键函数（discover, map_news_to_chains, fetch_flash_titles 等）必须引用规则编号

### 2.4 CI 集成

`.github/workflows/compliance.yml`：
- 触发：所有 pull_request
- 步骤：
  1. `tools/compliance/check.sh` 跑全套
  2. `cargo test --lib --test e2e` 跑单元 + e2e
  3. PR body grep `Refs: spec §|Refs: config` → 必须含
- 退出码非 0 → merge 阻断

### 2.5 实施顺序：4 个 PR 串行

```
PR-1 (0.5d) → PR-2 (1d) → PR-3 (1d) → PR-4 (1d)
   ↓            ↓            ↓            ↓
 修 R-1       修 R-3       修 R-2+7     修 R-4/5/6
e2e 测试     数据回灌     阈值对齐     业务规则文档
AGENTS§2.8  AGENTS§2.8.1  AGENTS§2.9   AGENTS§2.10
```

每个 PR 独立可合并、跑通测试、有可观测收益。

#### PR-1：修 R-1

| 步骤 | 改动 |
|------|------|
| 1 | 改写 `verify_predictions`：从 `stock_daily` 拉收盘价，算 actual_change，判定 hit |
| 2 | 写 `tests/e2e_prediction_verify.rs` |
| 3 | 修 AGENTS.md 新增 §2.8 |
| 4 | 写 `tools/one_shot/backfill_predictions.sh` 回填过去 14 天 |

DoD：
- `cargo test --test e2e_prediction_verify` 通过
- `backfill_predictions.sh` 后 `prediction_tracker` 至少有一批 hit=1
- AGENTS.md §2.8 落地
- review 表勾选"§2.8 合规"

#### PR-2：修 R-3 + §2.4 数据新鲜度

| 步骤 | 改动 |
|------|------|
| 1 | 查 rustdx / 调度器为什么 6-15 后停更 |
| 2 | 修复 + 回灌 6-16~6-29 数据 |
| 3 | 写 `tools/compliance/lib/check_data_freshness.sh` |
| 4 | 写 `tools/compliance/check.sh` 主入口（只挂 §2.4） |
| 5 | 修 AGENTS.md §2.4 强化 + §2.8.1 引用 |

DoD：
- `stock_daily` MAX(date) >= 昨天
- `tools/compliance/check.sh` 跑通
- `tests/test_data_freshness_check.rs` 通过

#### PR-3：修 R-2 + R-7 + §2.9

| 步骤 | 改动 |
|------|------|
| 1 | 改 `config/opportunity.toml`: threshold 75 → 60，注释引用 spec §0 NS3 |
| 2 | 改 `src/opportunity/discover.rs`: `source_score` 拆为 Rule=10/Ai=6/AiDegraded=0，加注释证明 |
| 3 | 写 `tools/compliance/lib/check_design_contradiction.sh` |
| 4 | `check.sh` 主入口挂 §2.9 check |
| 5 | 修 AGENTS.md §2.9 落地 |
| 6 | 写 `tests/test_design_contradiction.rs`（故意 threshold=80 断言 fail） |

DoD：
- `cargo run --bin monitor -- --review` 跑通
- `check.sh` §2.9 通过
- 故意把 threshold 调 80，§2.9 报失败

#### PR-4：修 R-4 / R-5 / R-6 + §2.10

| 步骤 | 改动 |
|------|------|
| 1 | 建 `docs/business_rules.md`，写 5 条规则 |
| 2 | 修 R-4：加"近 3 交易日已推"硬去重 |
| 3 | 修 R-5：chain_mapper 加"一条快讯最多 1 条产业链"互斥（例外：AI 给出 ≥2 条独立产业链时保留，见 BR-002） |
| 4 | 修 R-6：`fetch_flash_titles` 加 A 股过滤（macro 通道分离） |
| 5 | 写 `tools/compliance/lib/check_business_rules.sh` |
| 6 | 写 e2e：`tests/e2e_dedup.rs` + `tests/chain_exclusive.rs` + `tests/flash_filter.rs` |
| 7 | 修 AGENTS.md §2.10 落地 |
| 8 | `check.sh` 主入口挂 §2.10 check + 接入 CI |

DoD：
- 5 条业务规则全部登记且被代码引用
- 3 个 e2e 测试通过
- `check.sh` §2.10 通过
- 故意把 600522 连续 3 天推，dedup 生效
- `.github/workflows/compliance.yml` 落地

---

## 3. 测试策略

### 3.1 单元测试

不动。已存在的 ~289 个单测继续跑。

### 3.2 新增 e2e 测试（4 个 PR 共 5 个）

- `tests/e2e_prediction_verify.rs` (PR-1)
- `tests/test_data_freshness_check.rs` (PR-2)
- `tests/test_design_contradiction.rs` (PR-3)
- `tests/e2e_dedup.rs` + `tests/chain_exclusive.rs` + `tests/flash_filter.rs` (PR-4)

### 3.3 门禁脚本测试

每个 `check_*.sh` 必须支持"故意制造失败 → 脚本报错退出码 1"的回归测试。CI 必须跑这些回归。

### 3.4 验证矩阵

| 测试类型 | 拦 R-1 | 拦 R-2 | 拦 R-3 | 拦 R-4/5/6 | 拦 R-7 |
|---------|--------|--------|--------|-----------|--------|
| e2e_prediction_verify | ✓ | | | | |
| test_data_freshness | | | ✓ | | |
| test_design_contradiction | | ✓ | | | ✓ |
| e2e_dedup | | | | ✓ | |
| chain_exclusive | | | | ✓ | |
| flash_filter | | | | ✓ | |

---

## 4. 风险与缓解

| 风险 | 缓解 |
|------|------|
| 门禁脚本误报 | §2.9 / §2.10 加 `--strict` 与 `--warn-only` 两档 |
| 维护负担 | check.sh 单文件 + 模块化 lib/，加新规则 < 30 行 |
| CI 跑慢 | check.sh 设计为 < 5s 总耗时，纯 grep / sqlite 解析 |
| 故意违反纪律 | 走 AGENTS.md §三"受控例外通道"，先改文档再合代码 |
| 团队抵触 | 单开发者项目无团队问题；多人时升级到方案 C（reviewer 训练） |

---

## 5. 验收标准

### 5.1 4 个 PR 全部合入后 (v9.2 完成, commit 8645d71 + e7cdc15 + 380709e + 6d15608 + a99cd2c 落地)

- [x] `prediction_tracker` 表有真实 hit 数据，hit_rate 在 0~100% 区间反映真实 (R-1 修复, 8645d71)
- [x] `stock_daily` MAX(date) 自动保鲜 (R-3 修复 + check_data_freshness.sh 门禁, e7cdc15)
- [x] 同一只票 7 天内最多推送 1 次 (BR-001 batch query, fe2ec73)
- [x] 一条快讯最多命中 1 条产业链 (BR-002 互斥, chain_mapper.rs:133, e7cdc15 + C-3 测试加固 8645d71)
- [x] 宏观新闻入 macro 通道，不入 chain_mapper (BR-003 filter_macro_titles 纯函数, 3f6db3e)
- [x] `tools/compliance/check.sh` 在本地 + CI 跑通 (4 个 gate 脚本, e7cdc15)
- [x] AGENTS.md §2.8 / §2.9 / §2.10 落地 (e7cdc15)
- [x] `.github/workflows/compliance.yml` 拦截 PR (e7cdc15 + 8645d71 加 e2e_prediction_verify 到 CI)

### 5.2 上线后 30 天

- [ ] 新合入的 20+ PR 全部过门禁
- [ ] 新出现的"假实现/设计矛盾/数据断层/业务规则缺位" bug 数 = 0
- [ ] 预测闭环 hit_rate 数字真实可读

---

## 6. 范围外（Out of Scope）

以下内容**不在本次方案内**，留作未来：

- 重构 `prediction.rs` 整体结构（本次只动 verify 函数体）
- 引入新数据源（本次不增加 provider）
- 升级到方案 C（reviewer 训练 + checklist）——等团队 > 2 人再做
- 替换 bash 脚本为 Rust 工具链——除非性能瓶颈出现
- 引入 property-based testing（proptest）——超出纪律改进范围

---

## 7. 关联文档

- `AGENTS.md`（被修改：新增 §2.8 / §2.9 / §2.10）
- `docs/KNOWN_BUGS-2026-06-28.md`（7 个 bug 的根因记录）
- `docs/PROJECT_DESIGN-2026-06-28.md`（项目整体设计）
- `docs/architecture/`（v9.1 机会 pipeline 架构）

---

## 8. 决策记录

| 日期 | 决策 | 理由 |
|------|------|------|
| 2026-06-29 | 选方案 B（AGENTS.md + 门禁脚本） | 方案 A 太弱（无强制力），方案 C 太重（团队训练） |
| 2026-06-29 | PR-1 先修 R-1 | 最高 ROI，知道真实胜率才能调其它 |
| 2026-06-29 | 门禁脚本用 bash 而非 Rust | 易读易改，无需编译，性能足够（< 5s） |
| 2026-06-29 | 业务规则编号 BR-001~BR-005 | 与 AGENTS.md §2.10 对齐，5 类初始登记 |
