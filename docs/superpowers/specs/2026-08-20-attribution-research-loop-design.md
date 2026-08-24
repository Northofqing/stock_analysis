# 2026-08-20 Attribution Research Loop 设计

**状态**: Approved (2026-08-20, 用户批准, 含修订版「已实现 + 未实现浮盈」)
**分支**: `attribution-research-loop` @ `5d44613` (2026-08-20 创建)
**范围**: 交付物 A (虚拟盘绩效归因闭环) + 交付物 B (G5a 盘中异动归因接线)

---

## 1. 背景与目标

用户 (机构负责人视角) 核心痛点: **系统至今无法正向分析市场、归因、验证虚拟盘盈利**。

根因诊断 (2026-08-20 会话):
- 归因代码是孤儿 (`src/monitor/attribution.rs` 425 行, 全 src 零调用 — 证据 E1/E2)
- P&L 闭环只到一条日志 (`PerformanceEngine::daily_settlement` 15:05 跑完打 log, 无消费方 — 证据 E14)
- 成功标准错位: CLAUDE.md Completion Rule 全部验证「推送是否送达」, 无一条验证「盈亏从哪来」

**目标**: 建立每日归因闭环 — 每笔虚拟成交 (已实现) + 每个未平仓 lot (浮盈) 按入场信号族归因, 每日 15:05 计算 → 落库 → 落盘全文 → 推送摘要, 30 天滚动窗口回答「虚拟盘盈亏从哪来」; 同时把已写好的 G5a 异动归因接回生产路径。

## 2. 决策记录 (已获用户批准)

| 决策 | 选择 | 理由 |
|---|---|---|
| 分支 | 从 HEAD `5d44613` 直接开 `attribution-research-loop`, **grpc WIP (18 文件) 留在工作树不动**, 提交只 add 本分支文件 | 归因工作文件集与 grpc WIP 完全不相交, 零纠缠 |
| 归因范围 | **两者都要**: 交付物 A 绩效归因 + 交付物 B G5a 接线 | 用户明确选择 |
| 输出形态 | 每日 15:05 推送摘要 (新 PushKind::AttributionDaily, 与 19:00 复盘推送分离) + 全文落盘 `data/attribution/<date>.md` + 30 天滚动窗口附摘要 | 用户明确选择; 符合「默认值出声」原则 |
| 归因口径 (修订版) | **已实现 (FIFO 卖出) + 未实现浮盈 (未平仓 lot × 收盘价)** | 证据 E7 驱动: 100 笔卖出全部集中在 8/11–8/12, 698 个 lot 未平仓 (87%)。只归已实现 = 5 周 33 天空报告, 不回答问题 |
| 价格源 | 复用 19:00 复盘已验证路径 `market_data::fetch_position_quotes()` + `build_price_map` (main.rs:10711) | 同一机制, 不发明新价格通道 |
| 数据质量 | 可疑成交 (涨幅>25% 或 量比≤0) 归入原族但单独标注计数+影响金额, 报告第一段即数据质量审计 | 证据 E6: `涨幅+858.9%` × 27 笔真实存在 |

## 3. 现有代码事实 (含证据, 2026-08-20 采集)

### E1. attribution.rs 在 bin 零调用 (multiline-aware)

```bash
$ pcre2grep -rInM 'attribution|apply_attribution|AttributionRequested|attribute_event|handle_attribution' src/bin/ 2>/dev/null | head -5
(空输出)
```

### E2. 全 src 调用点计数

```bash
$ grep -rInE 'monitor::attribution|apply_attribution|attribute_event|handle_attribution_requested' src/ --include="*.rs" | grep -v '^src/monitor/attribution.rs' | wc -l
0
```

### E3. paper_trades 规模

```bash
$ sqlite3 data/stock_analysis.db "SELECT COUNT(*), MIN(ts), MAX(ts) FROM paper_trades;"
1074|2026-07-10 15:07:43|2026-08-14 06:59:19
$ sqlite3 data/stock_analysis.db "SELECT direction, COUNT(*) FROM paper_trades GROUP BY direction; SELECT status, COUNT(*) FROM paper_trades GROUP BY status;"
buy|974
sell|100
Filled|898
NotFilled|176
```

### E4. virtual_reason 分布 (前 12) — 信号族可直接提取

```bash
$ sqlite3 data/stock_analysis.db "SELECT virtual_reason, COUNT(*) FROM paper_trades GROUP BY virtual_reason ORDER BY 2 DESC LIMIT 12;"
NewsCatalyst|506
VolumeSurge|87
MainNetInflow|80
BR-234四大铁律卖出:结构止损（破中期趋势）|38
Breakout|28
盘后资金净流入Top10 收盘价买入: 主力+9.96亿 量比1.5 涨幅-2.9%|27
盘后资金净流入Top10 收盘价买入: 主力+9.57亿 量比1.6 涨幅+10.0%|27
盘后资金净流入Top10 收盘价买入: 主力+9.52亿 量比3.1 涨幅+5.6%|27
盘后资金净流入Top10 收盘价买入: 主力+9.52亿 量比1.6 涨幅+10.0%|27
盘后资金净流入Top10 收盘价买入: 主力+8.92亿 量比2.0 涨幅+20.0%|27
盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%|27
盘后资金净流入Top10 收盘价买入: 主力+20.84亿 量比1.1 涨幅+6.7%|27
```

### E5. 卖出 virtual_reason 全分布 (截断) — 出场原因结构清晰

`BR-234四大铁律卖出:结构止损（破中期趋势）` ×38, `BR-234四大铁律卖出:铁律4:14天不涨换股` ×7, `铁律5:布林上轨+MACD顶背离` ×4, `铁律3:跌破5日线止盈` ×3, `ATR动态止损(有效止损价 X.XX)` ×48 (每笔唯一)。

### E6. 可疑数据真实存在

```bash
$ sqlite3 data/stock_analysis.db "SELECT virtual_reason FROM paper_trades WHERE virtual_reason LIKE '%858.9%' OR virtual_reason LIKE '%量比0.0%' LIMIT 3;"
盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%
盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%
盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%
```

### E7. 卖出日期极度集中 — 修订版设计的关键证据

```bash
$ sqlite3 data/stock_analysis.db "SELECT date(ts) d, COUNT(*) FROM paper_trades WHERE direction='sell' AND status='Filled' GROUP BY d ORDER BY d DESC LIMIT 10;"
2026-08-12|63
2026-08-11|37
```

**100 笔卖出全部在 8/11–8/12 两天。** 未平仓 lot 规模 (E12): 买入 Filled 798 − 卖出 Filled 100 = **698 lot 未平仓 (87%)**。

### E8. business_rules.md 与 grpc WIP 纠缠

```bash
$ git diff --stat docs/business_rules.md
 docs/business_rules.md | 16 +++++++++++++++-
 1 file changed, 15 insertions(+), 1 deletion(-)
```

**后果**: 本分支不直接改 `docs/business_rules.md` (会把未提交的 grpc WIP 内容卷入提交)。BR 注册落在新文件 `docs/operations/2026-08-20-attribution-research-loop.md`, 待 grpc WIP 提交后再合并进 business_rules.md (见 §8 风险)。

### E9. DISPATCH_TABLE 位置

```bash
$ grep -rln "DISPATCH_TABLE" src/ --include="*.rs" | head -3
src/bin/monitor/notify.rs
$ grep -n "pub const DISPATCH_TABLE" src/bin/monitor/notify.rs
675:pub const DISPATCH_TABLE: &[(PushKind, DispatchRow)] = &[
```

### E10. 模板注册机制 (push_templates.rs)

渲染函数: `pub fn render_<name>(...) -> String` (如 `render_intraday_alert` @ push_templates.rs:520); 模板注册: presentation_registry.rs:81 按名字符串注册; stable template_id 形如 `R09_TEMPLATE_ID` / `"event_calendar_v1"`。

### E11. 估值机制已存在 (portfolio/closing_valuation.rs, BR-147)

`calculate_closing_valuation(items, prices, previous_closes, date, provider)` → `ClosingValuationView { covered, total, items[unrealized_pnl], ... }`, **部分覆盖语义**: 价格缺失项不报错, 计入未覆盖计数。本设计的「未估值 lot 出声不静默」沿用此语义。

### E12. 未平仓 lot 按族分布 (买入 Filled, 非 BR 前缀)

`NewsCatalyst|473, VolumeSurge|87, MainNetInflow|72, Breakout|28, 盘后资金流入×族|135` (验证: `SELECT COUNT(*) FROM paper_trades WHERE direction='buy' AND status='Filled' AND (virtual_reason LIKE '盘后%' OR virtual_reason LIKE '%收盘价买入%')` → 135)。

### E13. 收盘价格源 (19:00 复盘已验证路径)

main.rs:10711 (build_close_review_report):
```rust
let quotes = market_data::fetch_position_quotes()?;
let prices = build_price_map(&quotes);
```
`build_price_map: quotes → HashMap<code, price>` (main.rs:10429)。

### E14. PerformanceEngine 接线点与告警 fail-closed 现状

- `PerformanceEngine::daily_settlement()` @ main.rs:8171, 15:05 块内, `PERF_LAST_RUN: Mutex<Option<NaiveDate>>` 当日一次, 失败 warn「允许 30s 后重试」。归因接入紧随其后。
- 盘中告警: main.rs:8966 / 9472 `detector.scan_stock(&snap)` 循环, 产物进 `state_machine.process(e)`, 再进 `reject_unbound_alert_delivery` (main.rs:10418) — **BR-192 非涨停/跌停类目 fail-closed 拒推**。G5a 归因文本随 alert 审计落库是保证输出; 推送可见性取决于 BR-192 绑定状态 (见 §5.3)。

## 4. 交付物 A: 虚拟盘绩效归因

### 4.1 信号族提取 (SignalFamily)

枚举 (字符串表示, 稳定 snake_case 用于报告与 DB):

| SignalFamily | 提取规则 (virtual_reason 前缀/包含) | 证据 |
|---|---|---|
| `NewsCatalyst` | 前缀 `NewsCatalyst` | E4 (506) |
| `VolumeSurge` | 前缀 `VolumeSurge` | E4 (87) |
| `MainNetInflow` | 前缀 `MainNetInflow` | E4 (80) |
| `Breakout` | 前缀 `Breakout` | E4 (28) |
| `PostCloseFundInflow` | 前缀 `盘后资金净流入` 或包含 `收盘价买入` | E4 (~270) |
| `ExitByRule` | 前缀 `BR-` (卖出族) | E5 (100) |
| `Unknown` | 其余 → 归入 Unknown, **报告显式列出 Unknown 计数, 不静默** | — |

辅助解析 (用于数据质量标注):
- `parse_pct_after("涨幅")`: 从 virtual_reason 提取 `涨幅+X.X%` 数值。
- `parse_volume_ratio`: 提取 `量比X.X` 数值。
- **可疑规则**: `|涨幅| > 25` 或 `量比 ≤ 0` → 该 lot 标记 `suspicious = true` (A 股单日最大 ±20%, 25 为保守阈值; `涨幅+858.9%` ×27、`量比0.0` 即被捕获 — E6)。可疑 lot **仍计入所属族** PnL, 但报告「数据质量」节单独列出计数与影响金额 (不删除, 不静默 — 与「不重建历史数据」决策一致)。

### 4.2 模块 `src/performance/attribution.rs` (新文件)

数据结构:
```rust
pub struct TradeAttribution {
    pub sell_id: i64,
    pub code: String,
    pub pnl: f64,                    // 该笔卖出已实现盈亏
    pub entry_plan_id: String,       // 匹配到的入场 lot 的 plan_id
    pub entry_family: SignalFamily,  // 入场信号族 (归因维度)
    pub exit_reason: String,         // 卖出 virtual_reason
    pub suspicious: bool,            // 入场侧数据质量标注
}

pub struct OpenLotValuation {
    pub code: String,
    pub lots: i64,                   // 未平仓手数
    pub cost_price: f64,             // 加权成本
    pub close: Option<f64>,          // None = 未估值
    pub unrealized_pnl: Option<f64>,
}

pub struct FamilyAggregate {
    pub family: SignalFamily,
    pub realized_trades: i64,
    pub realized_pnl: f64,
    pub open_lots: i64,
    pub unrealized_pnl: f64,
    pub total_pnl: f64,              // realized + unrealized
    pub wins: i64,
    pub losses: i64,
    pub win_rate: Option<f64>,
    pub unvalued_lots: i64,          // 无收盘价 lot 数
    pub suspicious_lots: i64,        // 入场侧可疑 lot 数
}

pub struct DailyAttribution { pub date: NaiveDate, pub families: Vec<FamilyAggregate>, ... }
pub struct WindowAttribution { pub days: u32, pub families: Vec<FamilyAggregate>, ... }
```

核心函数:
- `fifo_attributed_pnl(rows: &[PaperFillRow]) -> Result<Vec<TradeAttribution>, String>` — FIFO 语义与 `performance/snapshot.rs::realized_pnls_for_date` **逐条对齐** (id>0, code 非空, price>0 finite, qty>0 且 %100==0, 时间序校验, oversell 拒绝, 非 finite PnL 拒绝), 区别仅在匹配时携带入场 lot 的 `plan_id / virtual_reason → family / suspicious` 归属。跨 lot 匹配时 PnL 按数量比例拆分归属。
- `signal_family_of(reason: &str) -> SignalFamily`
- `is_suspicious_reason(reason: &str) -> bool` (§4.1 规则)
- `compute_daily(date: NaiveDate, prices: &HashMap<String, f64>) -> Result<DailyAttribution, String>` — 已实现 (当日卖出) + 浮盈 (截至当日未平仓 lot × close, 缺失 close → `unvalued` 计数, 不报错不静默)
- `compute_window(end: NaiveDate, days: u32, prices: &HashMap<String, f64>) -> Result<WindowAttribution, String>` — 已实现累计 (窗口内每日卖出, FIFO 对历史 lot 全局匹配) + 期末浮盈 (窗口末日未平仓)

**一致性约束 (硬)**: 对任意日期, 新模块当日已实现合计必须等于 `PerformanceEngine::compute_snapshot` 当日 `total_pnl` (同日期同数据 → 同结果)。落地为单元测试 (AC-A7)。

### 4.3 表 `paper_attribution_daily` (新表, 与 snapshot 并行, immutable 追加)

```sql
CREATE TABLE IF NOT EXISTS paper_attribution_daily (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    date            TEXT NOT NULL,
    signal_family   TEXT NOT NULL,
    realized_trades INTEGER NOT NULL DEFAULT 0,
    realized_pnl    REAL NOT NULL DEFAULT 0.0,
    open_lots       INTEGER NOT NULL DEFAULT 0,
    unrealized_pnl  REAL NOT NULL DEFAULT 0.0,
    total_pnl       REAL NOT NULL DEFAULT 0.0,
    wins            INTEGER NOT NULL DEFAULT 0,
    losses          INTEGER NOT NULL DEFAULT 0,
    win_rate        REAL,
    unvalued_lots   INTEGER NOT NULL DEFAULT 0,
    suspicious_lots INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(date, signal_family)
)
```
写入: `INSERT OR REPLACE` (当日重算幂等, 与 snapshot 同模式); 不 UPDATE 历史行。

### 4.4 报告

**全文** → `data/attribution/<date>.md`:
1. 标题 + 日期
2. **数据质量审计** (第一段): 可疑 lot 计数/族/影响金额; 未估值 lot 计数; Unknown 族计数
3. **今日归因**: 每族一行 (已实现/浮盈/合计/笔数/胜率)
4. **30 天滚动窗口**: 每族一行 (已实现累计/期末浮盈/合计/胜率)
5. **Top 亏损/盈利交易明细** (当日, 各 ≤5 笔, 含 code/plan_id/盈亏/入场族)

**摘要** (~20 行, 推送):

```
📊 虚拟盘归因 2026-08-20
━━━━━━━━━━━━━━━━━━━━
【今日】已实现 +¥120  浮盈 -¥3,450
【30天】已实现 -¥12,340  期末浮盈 -¥56,800
━━━━━━━━━━━━━━━━━━━━
① NewsCatalyst    506笔  -¥8,120  胜率38%
② 盘后资金流入    270笔  -¥3,900  胜率31%
③ MainNetInflow    80笔  +¥2,100  胜率52%
④ VolumeSurge      87笔  -¥1,200  胜率29%
⑤ Breakout         28笔    -¥420  胜率36%
⑥ ExitByRule(卖)  100笔    +¥890  胜率44%
━━━━━━━━━━━━━━━━━━━━
⚠ 数据存疑 27笔 (+¥582k)  |  未估值 12 lot
```
(以上为格式示例, 数字非真实。30 天窗口覆盖 7/10 起数据, 首日即完整 30 天。)

### 4.5 30 天窗口算法

滚动 30 自然日 (含当日), 按族聚合; 已实现 = 窗口内每日卖出 FIFO 全局匹配 (对历史全部 lot), 浮盈 = 窗口末日未平仓 × 当日 close。窗口合计 = 已实现累计 + 期末浮盈 — 对未平仓占 87% 的盘, 这是正确口径 (已批准修订)。

### 4.6 main.rs 15:05 接线

位置: main.rs:8171 `PerformanceEngine` 块之后 (同 15:05 块内), 模式完全沿用:
- `ATTRIBUTION_LAST_RUN: Mutex<Option<NaiveDate>>` 当日一次
- 顺序: `compute_daily` → 写 `paper_attribution_daily` → `compute_window` → 生成全文 md (`data/attribution/<date>.md`) → 推送摘要 (新 PushKind)
- 价格: `market_data::fetch_position_quotes()` + `build_price_map` (复用 E13 路径; 19:00 路径内包裹于 `tokio::task::spawn_blocking` (main.rs:10680), 15:05 块以同方式调用)
- 失败: warn 出声「归因计算失败 (允许 30s 后重试)」, 不 panic, 不静默
- 空数据日 (无卖出且无持仓变化): 仍生成报告 (浮盈变化即内容), 推送摘要 — **空报告不静默跳过**

### 4.7 PushKind::AttributionDaily 注册链 (5 步)

1. `notify.rs` PushKind 枚举加 `AttributionDaily` (106 → 107)
2. `push_templates.rs` 加 `render_attribution_daily(params) -> String` + stable template_id `attribution_daily_v1`; presentation_registry.rs 注册
3. `v14_adapter.rs::map_push_kind` 加 `PushKind::AttributionDaily => (HoldingHealth, "attribution_daily", Severity::Normal)`
4. `notify.rs:675 DISPATCH_TABLE` 加 `(PushKind::AttributionDaily, DispatchRow {...})` — 启动 audit 自动覆盖
5. BR 注册: 新文件 `docs/operations/2026-08-20-attribution-research-loop.md` (business_rules.md 被 WIP 纠缠, E8)

## 5. 交付物 B: G5a 盘中异动归因接线

### 5.1 接线点

main.rs:8966 (早盘限价循环) 与 main.rs:9472 (盘中循环) 的 `for e in detector.scan_stock(&snap)` 循环内, **事件进入 state_machine 之前**:

```rust
for e in detector.scan_stock(&snap) {
    signal_count += 1;
    // G5a: 同步归因, 2s 预算, 失败出声不折叠
    if let Err(failure) = stock_analysis::monitor::attribution::apply_attribution(&mut e) {
        log::warn!("[G5a] attribution failed: {failure}");
    }
    // ... 原有 state_machine.process / signals 逻辑不变
}
```

### 5.2 语义 (全部沿用模块既有实现, 零改动)

- `apply_attribution(&mut event)` 同步, `ATTRIBUTION_BUDGET = 2s`, 超时 warn
- 回写 `event.detail.ai_decision` (如 `"半导体-PCB 异动催化 | 置信度A | 缺失:fund_flow"`)
- 审计: ai_decision 随 alert 落库 (沿用 `src/monitor/alert_log.rs` 现有写入机制, 实施时定位调用点)
- 失败语义不变: `ChainRulesUnavailable` 出声 warn, 事件照常流转 (不折叠成空归因)

### 5.3 BR-192 fail-closed 风险 (已知)

盘中告警非涨停/跌停类目当前被 `reject_unbound_alert_delivery` (main.rs:10418) 拒推。因此:
- **保证输出**: G5a 归因写入 alert 审计 (ai_decision)
- **推送可见性**: 取决于 BR-192 绑定状态; 实施时验证, 若仍 fail-closed 则归因文本仍完整落审计, 不算未完成 (AC-B2 只要求审计)

## 6. 测试与验收 (machine-checkable)

| AC | 命令 | 预期 |
|---|---|---|
| AC-A1 | `cargo test --lib performance::attribution` | 全绿 (新模块单测: 信号族提取 / FIFO 带归属 / 跨 lot 拆分 / 可疑标注 / 一致性) |
| AC-A2 | `cargo test --lib performance::snapshot` | 全绿 (原测试不动) |
| AC-A3 | `cargo test --lib monitor::attribution` | 全绿 (G5a 既有 11 个单测, 零改动) |
| AC-A4 | `cargo build --lib` | exit 0 |
| AC-A5 | `cargo build --release --bin monitor` | exit 0 |
| AC-A6 | `V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 \| grep -E 'attribution'` | ≥1 行 (attribution 计算/delivery 日志) |
| AC-A7 | `cargo test --lib performance::attribution consistency` | 通过: 同日期新模块 realized 合计 == `compute_snapshot.total_pnl` |
| AC-A8 | 生产: `ls data/attribution/$(date +%Y-%m-%d).md` | 文件存在 |
| AC-A9 | 生产: `grep -lE '^\[AttributionDaily\]' data/push_log/$(date +%Y-%m-%d)/ \| wc -l` | ≥ 1 |
| AC-A10 | 生产: `grep -c '"event_type":"push.delivery.audit"' data/event_bus/$(date +%Y-%m-%d).jsonl` 且含 attribution_daily_v1 | > 0 |
| AC-B1 | `V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 \| grep -E 'G5a'` | ≥1 行 (apply_attribution 进入路径) |
| AC-B2 | 生产 alert 审计: 任意 AlertEvent 的 ai_decision 非空 | 存在 (或 --test 下验证) |
| AC-B3 | `./target/release/monitor --test 2>&1 \| grep -E 'dispatch_table|DISPATCH'` | 新 PushKind 出现在启动 audit |
| AC-B4 | `grep -RInE '"(first|mock|stub|test kept|placeholder|fake|sample)"' data/push_log/$(date +%Y-%m-%d)/` | 0 命中 (v15 规则: 测试字符串不进生产) |

## 7. 明确不做 (Scope Guard)

- 不修 `review::factor_ic` / `review::failure_attribution` 孤儿 (记入 `docs/operations/2026-08-20-attribution-research-loop.md` 待办, 不实现)
- 不改 `paper_trades` schema (immutable 写入原则)
- 不重建/清洗历史虚拟盘数据 (只标注 suspicious, 不删除 — -104% 结论 30 天窗口内自然呈现)
- 不触碰 grpc WIP 文件 (E8 文件集不相交)
- 不发明新价格通道 (复用 E13)

## 8. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 新 FIFO 与 snapshot.rs 结果不一致 | AC-A7 一致性硬测试; 移植其 8 个既有测试为对照 |
| PushKind 注册链遗漏 (106 变体 + DISPATCH_TABLE) | 启动 audit (notify.rs:634) 自动报未注册; AC-B3 |
| business_rules.md 与 grpc WIP 纠缠 (E8) | 本分支不碰该文件, BR 注册落新文件; 后续会话在 grpc WIP 提交后合并 |
| 盘中告警 BR-192 fail-closed, G5a 文本不可见 | 归因审计是保证输出 (§5.3), AC-B2 只要求审计 |
| 15:05 块与 BR-226 快照提醒同点竞争 | 顺序执行, 无锁冲突 (各自独立 Mutex) |
| 未估值 lot 过多导致浮盈失真 | 报告「数据质量」节明示 unvalued 计数 (出声), 不静默填零 |

## 9. 2026-08-22 纸面卖出 T+1/FIFO 批次库存修订

### 9.1 状态、范围与旧模块关系

本修订修复 `paper_sell` 把全部历史买入混合摊薄、只扣总数量并以最早买入日放行整仓卖出的错误。范围只包括：从真实 `Filled` 成交重建批次库存、计算 T+1 可卖数量、向既有四铁律卖出规则提供正确的数量/成本/日期，以及把依据绑定到订单审计。

不修改卖出阈值、规则优先级、监控频率、数据库结构、历史成交、真实下单路径和 `src/bin/monitor/main.rs`。`paper_sell_paused` 默认暂停及 BR-201/BR-211 Disabled 状态保持不变；算法正确不构成恢复生产卖出的授权。

| 旧模块 | 处理 | 原因 |
|---|---|---|
| `paper_sell` | 采用 | 保留数据库、行情、规则和订单适配职责 |
| `position_tracker::evaluate_sell_rules` | 采用 | 保持既有四铁律语义和阈值 |
| `paper_trade::simulate` | 采用 | 保持订单安全、幂等和成交审计；增加仅供 FIFO 卖出的窄审计入口 |
| `performance::attribution::fifo_match` | 拒绝直接依赖 | 报告窗口和信号族语义不得进入资金安全执行边界 |
| `paper_engine` | 不变 | 该旧执行路径不作为本修订的新 owner |

触发数据红线 2.1、2.2、2.3、2.4、2.6、2.7 和 2.10；未修改配置阈值，因此 2.9 的 Threshold-Proof 为 N/A。

### 9.2 数据流与深模块接口

新增纯模块 `src/trading/paper_lot_ledger.rs`。其唯一入口接收按 `(occurred_at,id)` 排序的 `PaperFill` 和显式 `as_of_date`，输出按代码稳定排序的 `PaperPositionInventory`。模块内部维护代码隔离的 FIFO 批次，外部看不到状态转换细节。

处理顺序：

1. `paper_sell` 从 `paper_trades` 一次读取全部 `Filled` 行，读取原始 `CAST(ts AS TEXT)`，不调用 SQLite 日期修饰器。
2. Rust 仅接受完整、不可变的 `YYYY-MM-DD HH:MM:SS[.fraction]`，然后验证身份、顺序、方向、真实 `fill_price` 和 100 股整数手数量。
3. 买入形成带原始成交 ID、时间、价格和剩余数量的批次；卖出只按 FIFO 消耗 `buy_date < sell_date` 的批次。
4. 任一成交日期晚于 `as_of_date`、同日买卖、超卖或结构错误都使整批失败，不返回部分仓位。
5. 剩余批次中仅 `buy_date < as_of_date` 可卖；同日批次锁定。卖出规则只接收可卖数量、可卖批次加权成本和最早可卖日。
6. 触发规则后仍调用既有订单安全路径，但使用窄接口把 `BR134_FIFO_V1` 库存证据写入 `order_audit.decision_basis`；`paper_trades.virtual_reason` 保留原策略分类文本。

### 9.3 失败模式

以下情况在行情、订单和推送前显式失败：数据库读取失败、原始时间不完整或不可解析、重复身份、乱序、未来成交、空 ID/代码/名称、未知方向、缺失/非正/非有限成交价、非正或非 100 股整数手、数量溢出、超卖、卖出试图消耗同日买入批次。

禁止跳过坏行、用信号价/成本价/零值补 `fill_price`、把错误当空仓、由 SQLite 把 `now` 或 `12:34` 补造成时间，也禁止修改或删除历史事实来“修好”结果。

### 9.4 审计与历史数据

历史成交必须保留。它既是 FIFO/T+1 重建和策略复现的事实底账，也是规则 2.7 至少五年留存的审计依据。可删除或冷存的是可重算缓存，不是成交、仓位变化、行情输入或策略决策证据。

每次真正进入卖出订单尝试时，规范化证据绑定：评估日、该代码参与重建的成交 ID、所有剩余买入批次的成交 ID/时间/剩余数量/精确价格位模式、可卖或锁定状态、可卖总量和成本。订单审计链的既有不可更新、不可删除和哈希链机制继续生效。

结构性重建失败目前在进入订单前返回错误；该失败的独立持久审计设计见 §12。失败事件不得伪装成 `order_audit`。在 §12 的实现与验证完成前，生产卖出暂停闸不得解除。

### 9.5 验收与回滚

必须覆盖：混合隔夜/同日批次、部分卖出 FIFO、锁定批次排除成本、同日-only 零候选、同日卖出拒绝、未来卖出与未来回转拒绝、原始非法时间拒绝、超卖/坏价格/非法数量/重复乱序整批失败、多代码交错隔离、旧卖出规则与订单接口回归、审计证据与策略分类分离。

回滚按提交从新到旧执行 `git revert <sha>`；不得删除历史成交或改写数据库。任何 Gate 失败按 AGENTS 3.2 返回对应 Gate 修复。

## 10. 2026-08-23 R-12 事件研究语义修订

### 10.1 目的、边界与旧模块关系

R-12 当前把 `paper_trades` 的每条买入和卖出都当成独立信号，并用未来第 4/16 根十五分钟 K 线的终点涨跌计算同一套“胜率”。这只能回答“入场后短期价格是否上涨”，不能回答“现有买入与 BR-134 卖出组合扣除成本后是否盈利”。卖出原因也不是入场策略来源，二者必须从类型和报告上分开。

本修订把 R-12 收窄为**买入事件研究**：只评估已经真实写入 `paper_trades` 且状态为 `Filled` 的买入事件，按明确入场策略族分组，报告 4/16 根后的终点收益、上涨比例以及每个事件路径内的 MFE/MAE。完整经济持仓胜率、净收益、盈亏平衡、未平仓和右删失由后续经济持仓归因切片负责，R-12 不冒充该结论。

| 旧模块 | 处理 | 原因 |
|---|---|---|
| `review::backtest` | 修订 | 保留纯计算与取数薄壳，纠正事件边界和指标名称 |
| `HistoricalBarsGateway::fifteen_min_bars` | 保留禁用 | BR-239 已确认生产 TechnicalBars 能力未发布；本修订不授权上线 |
| `performance::attribution::SignalFamily` | 采用并补全 | 作为入场策略族单一映射，禁止 R-12 自建另一套来源口径 |
| `trading::paper_lot_ledger` | 不直接依赖 | R-12 是事件研究，不应复用执行库存类型冒充生命周期回测 |
| `paper_trades` 历史记录 | 只读保留 | 不删除、不修补、不以结果好坏选择样本 |

触发数据红线 2.1、2.2、2.3、2.4、2.7 和 2.10；未改配置阈值，2.9 的 Threshold-Proof 为 N/A。业务规则登记为 BR-247，且不改变 BR-239 的生产禁用状态。

### 10.2 输入合同与数据流

1. 数据库薄壳以参数绑定读取窗口内 `Filled` 买入，保留成交 ID、计划 ID、代码、名称、真实 `fill_price`、原始 `CAST(ts AS TEXT)` 和 `virtual_reason`；禁止使用请求价代替成交价。
2. 原始时间仅接受完整 `YYYY-MM-DD HH:MM:SS[.fraction]`；成交行 `id` 与业务幂等键 `plan_id` 必须分别非空/有效且在批内唯一，代码、名称、价格和入场族逐项校验。即使完整生产 schema 的 `uniq_paper_trades_plan_id` 正常阻止重复写入，R-12 公共解析边界也必须独立拒绝重复计划身份，禁止损坏或非规范输入把同一业务买入事件重复计入样本。未知策略族是显式数据缺口，不能并入 `Unknown` 后继续宣称全策略结果。
3. 入场族使用与经济持仓归因相同的封闭枚举，覆盖 `NewsCatalyst`、`VolumeSurge`、`MainNetInflow`、`Breakout`、`SectorLeader`、`AuctionAnomaly`、`LLMSelect`、`Momentum` 和盘后资金流入；BR-134 卖出原因永远不是入场族。
4. 已验证的十五分钟 K 线必须按时间严格升序、时间有效、OHLC/价格有限且大于零，并且完整序列须与一种稳定的十五分钟栅格一致：起点标记制为 `09:30..11:15 / 13:00..14:45`，终点标记制为 `09:45..11:30 / 13:15..15:00`。同一批次不得在两种栅格间切换；同日相邻 bar、午休跨段和跨交易日都必须连续，跨日使用仓库内不可变交易所日历验证。头尾可以是覆盖窗口的部分批次，内部缺口、重复、乱序、休市日或覆盖外日历均整批失败。
5. 上游 raw `KLINE_15MIN` 尚未发布 bar 起止语义，所以当前事件对齐只接受信号北京时间与某个来源 bar 时间戳完全相等。早于首根、晚于末根、任意非边界分钟、午休或收盘后信号保持“未对齐”，不得映射到此前 bar；未来 TechnicalBars 发布时必须以来源时间语义替换该保守合同，不得在本模块自行推断。
6. 每个窗口从信号边界之后的完整 4/16 根路径计算：终点收益使用最后一根 `close`，路径最大有利变动 MFE 使用所有未来 bar 的 `high`，路径最大不利变动 MAE 使用所有未来 bar 的 `low`。聚合值使用逐事件指标，不再把跨样本最高/最低终点收益或仅收盘路径冒充 MFE/MAE。`forward_observation/forward_return` 是公开计算缝，即使调用方没有先经过整批门禁，也必须逐根拒绝非有限、非正或 `high/low/open/close` 关系矛盾的 OHLC，禁止以坏路径计算终点、MFE 或 MAE。
7. `boll_macd` 只消费按时间排序的真实 `close/volume` 窄观察值；既有 `KlineData` 入口仅投影它真实拥有的字段。R-12 直接从 `SecurityBar` 构造窄观察值，禁止为了调用策略算法而填造 `pct_chg`、`settled`、涨跌停或停牌状态。
8. 即使所有事件均未对齐或右删失，R-12 也必须保留并审计 `exit_rows_excluded / unaligned_signals / censored_windows`，不得因统计组为空而降格成无计数的 NoData；只有全部统计组和三个计数都为零才是真正 NoData。
9. 卖出行不进入 R-12 上涨率。报告固定声明“事件上涨率不是完整策略胜率”；完整策略结论必须等待经济持仓归因、成本、基准和右删失处理完成。

### 10.3 失败与历史异常

固定日期 `2026-07-14..16` 和 `price < 1` 不是来源证据，必须从 R-12 删除。历史坏记录不能按硬编码规则静默丢弃：缺失/非法成交价、不可解析时间、未知方向/策略族、重复成交行 `id`、重复业务 `plan_id` 或非法 K 线使数据集显式不可用，并报告失败原因。仅“窗口未来 K 线尚不完整”与“信号在已验证 K 线覆盖外”可作为有计数的未对齐/右删失状态，不计入分母。

生产 TechnicalBars 仍由 BR-239 在 loader 前禁用，因此本切片只能完成确定性算法和契约测试，不能声称已取得真实十五分钟行情正向案例。能力发布必须另有数据源身份、时间语义、新鲜度和不可变审计设计。

### 10.4 指标与成功边界

R-12 的 `上涨比例` 仅为短期事件描述，样本数少于 200 时必须标记“样本不足”；即使达到 200，也不能替代至少 12 周、多个独立入场/退出日期和市场状态的完整经济持仓验证。策略成功标准保持冻结：扣成本净期望为正、相对基准 Alpha 为正，并报告聚类不确定性、未平仓和盈亏平衡。

### 10.5 验收与回滚

测试必须覆盖：早于首根/晚于末根/午休/收盘后/非边界分钟拒绝对齐、两种稳定栅格、内部缺口或切换栅格整批失败、非法或乱序 K 线整批失败、公开未来路径缝独立拒绝非有限/非正/关系矛盾的 OHLC、重复成交行 `id` 与重复业务 `plan_id` 分别整批失败、卖出不进入入场统计、九个入场族映射、未知族显式失败、真实 `fill_price`、high/low 逐路径 MFE/MAE、完整窗口右删失、全未对齐/右删失计数仍进入审计、200 样本门和报告免责声明。R-12、策略族映射和文档分别独立提交；回滚使用 `git revert <sha>`，不得恢复静默硬删除或改写历史事实。

## 11. 2026-08-23 经济仓位净收益归因

### 11.1 目的、统计单位与旧模块关系

现有 `performance::attribution` 把一笔卖出按 FIFO 买入 lot 拆成多个
`TradeAttribution`，随后把每个拆分片段都计为一次胜负。它适合解释盈亏来自哪类
入场，但不能作为策略胜率：同一卖出跨三个 lot 会被放大成三笔交易，部分卖出又会
在多天重复计数；同时当前值只含毛价差，没有逐成交费用证据。

本修订新增独立的**经济仓位生命周期**：同一代码从数量为零开始，经历一次或多次
买入、部分卖出和再次买入，直到数量重新归零，才形成一个完整胜负样本。未归零状态
是右删失开放仓位，不进入胜率分母。卖单和 FIFO 匹配片段只保留为审计/解释维度，
不得再冒充独立策略样本。

| 旧模块 | 处理 | 原因 |
|---|---|---|
| `performance::attribution` | 保留兼容，不作为完整胜率 | 继续服务既有日/窗口家族报告，避免在未验证前改动生产接线 |
| `performance::SignalFamily` | 采用 | 复用唯一入场族映射；未知买入族整批失败 |
| `trading::paper_lot_ledger` | 采用语义、不直接耦合输出 | FIFO/T+1/严格时间合同一致，但执行库存与研究生命周期保持不同接口 |
| `paper_trades` | 只读事实源 | 使用 `Filled.fill_price` 和原始时间；不删除、不回填、不按结果筛样本 |
| `position_tracker` 成本常量 | 拒绝作为证据 | 常量没有账户/生效期证据，且不能证明历史每笔实际费用 |
| `monitor/main.rs` 与现有推送 | 不变 | Gate B 只建立纯计算与只读薄壳，不授权生产发布或下单 |

触发数据红线 2.1、2.2、2.3、2.7 和 2.10；没有修改配置阈值，2.9 的
Threshold-Proof 为 N/A。业务规则登记为 BR-248。

### 11.2 输入合同与严格数据流

数据库薄壳读取全部 `Filled` 行的成交 ID、计划 ID、代码、名称、方向、真实
`fill_price`、数量、`CAST(ts AS TEXT)` 原始时间和 `virtual_reason`，禁止用 SQLite
`datetime/strftime/localtime/now` 解释或补造时间。Rust 复用严格
`YYYY-MM-DD HH:MM:SS[.fraction]` 解析器，验证完整批次后只把不晚于调用方显式
`as_of_date` 的行交给纯计算；纯计算入口若直接收到晚于评估日的行则失败。

每行必须满足：正且唯一的成交 ID、非空计划/代码/名称、严格 `(occurred_at,id)`
升序、方向仅为 buy/sell、有限正成交价、正且为 100 股整数手。买入还必须映射到
明确入场族；卖出原因只作为退出事实。任何缺失、重复、乱序、未知族、超卖、数量
溢出或非有限金额使整批失败，不返回部分统计。

每个代码维护一个隔离状态：

1. 空仓遇到买入时创建周期，记录首笔时间及全部来源成交 ID。
2. 后续买入累加数量、买入成交额和入场族组成；不得把混合策略强塞给首笔或最大族。
3. 卖出按 FIFO 消耗，且只能消费 `buy_date < sell_date` 的批次；同日消费与超卖失败。
4. 部分卖出只改变周期内部数量；剩余数量归零时一次性产出一个闭合经济仓位。
5. 截止评估日仍未归零的周期作为 `OpenPosition` 单列，数量和来源 ID 可追溯，
   不参与 wins/losses/breakeven 或平均净收益分母。

### 11.3 费用证据与净值状态

`paper_trades` 没有账户费用字段，因此禁止默认零费用，也禁止把仓内无来源常量包装成
真实成本。纯接口接受可选的完整 `FillCostLedger`：

- `basis_id` 必须非空并绑定来源/版本；`kind` 只能是 `Observed`（账户实证）或
  `Scenario`（明确情景假设）。
- 每个参与批次的成交 ID 必须恰有一条 `FillCostEvidence`，包含非负有限的总不利成本
  金额和非空证据 ID；重复、缺失或引用未知成交均失败。
- 总不利成本包含该成交已冻结口径下的佣金、税费及执行调整；生命周期模块只消费最终
  金额，不自行发明费率、生效日、最低佣金或滑点。
- 没有费用账本时仍可输出毛价差、闭合/开放数量和缺失原因，但所有净盈亏、净胜率、
  净期望和策略结论为 typed `Unavailable`，不得以 0 代替。
- `Scenario` 只能输出“情景净结果”，不能标为账户实际结果；`Observed` 才允许标为
  实证净结果。两种 basis 不得合并在同一分母。

当前仓库没有能够签发账户费用证据能力的真实适配器，因此公开纯计算入口在本切片中只
接纳 `Scenario`。仅凭调用方传入的非空 `basis_id` 不足以证明 `Observed`；任何
`Observed` 账本都必须显式失败并保持净指标不可用。未来只有在独立 Gate A 定义真实账户
费用来源、版本、不可变接收凭据及完整成交绑定，并由该适配器返回不可伪造的能力类型后，
才能开放 `Observed`，不得只增加字符串前缀、哈希或布尔开关冒充来源权威。

闭合周期的毛盈亏为卖出成交额减买入成交额；净盈亏再减该周期所有成交的总不利成本。
净收益率的分母固定为该周期累计买入成交额，并在名称中明确，不冒充资金加权 ROI。
净盈亏大于、等于、小于零分别为盈利、盈亏平衡、亏损；盈亏平衡单列，不能并入亏损。

### 11.4 汇总、成功边界与后续层

汇总至少报告：闭合经济仓位数、开放右删失数、盈利/亏损/盈亏平衡、净胜率（只以
盈利+亏损为分母并同时披露平衡数）、总/平均/中位净盈亏、累计买入成交额口径的
净收益率、首末闭环日期、入场族组成和费用 basis。混合入场族只报告数量/成交额贡献，
不产生伪造的单族“整仓胜率”。

少于 200 个闭合经济仓位或首末覆盖少于 84 个自然日（12 周）时固定为
`InsufficientSample`。即使样本达到门槛，本切片也不单独产生“策略成功”结论：完整结论
还必须有与每个周期精确对齐的基准收益、Alpha、按代码/入场日聚类的不确定性，以及
多个市场状态的来源证据。上述能力缺失时报告为 `ResearchOnly` 并列出缺口；净均值为正
不等于验证成功。

### 11.5 失败模式、审计与发布边界

所有失败发生在报告持久化、推送和订单之前。禁止跳坏行、改写历史、按日期/价格删除
不利样本、把费用缺失补零、把开放仓位按期末价强制平仓、把卖出片段当独立胜负，或把
情景成本描述为账户实证。计算结果必须携带 `as_of_date`、排序后的成交 ID、闭环 ID、
费用 basis/evidence ID 和规则版本，供后续不可变报告审计绑定。

本切片不修改生产推送、不写策略成功状态、不解除任何交易/行情能力闸。历史只读探针若
遇到坏记录，必须输出确切阻塞原因和可复现 ID；不得为了得到数字修表或回填成本。

### 11.6 验收与回滚

TDD 必须覆盖：一个跨多 lot/多卖单周期只计一个样本；平仓后再次买入形成第二周期；
混合入场族保留组成；开放仓位右删失；T+1、超卖、重复/乱序、坏身份/方向/价格/数量、
未知族和未来行整批失败；无费用时净值不可用；完整费用一对一绑定；费用缺失/重复/
未知引用失败；当前无真实费用适配器时 Observed 必须失败、Scenario 标签不得冒充实证；盈利/亏损/平衡分母；200 个闭环和 84 天门。

设计、RED/GREEN 实现和证据分别独立提交。回滚只使用 `git revert <sha>`；不得删除或
修改 `paper_trades`，不得把旧 lot 片段胜率重新包装成完整策略胜率。

## 12. 2026-08-23 纸面库存重建失败审计

### 12.1 目的、边界与旧模块关系

BR-134 在行情、策略判断和订单之前重建完整 `Filled` 成交批次；任一坏行、超卖或
T+1 违规都会整批失败。当前错误只返回调用方，重复扫描只留下易丢失的日志，不能证明
某次卖出为何没有进入订单，也不能在五年后复核当时读取到的原始事实。本修订为这个
**订单前失败边界**新增独立、只追加、哈希链接的 SQLite 审计，不改变成交事实、卖出
规则、阈值、推送、真实下单路径或 `paper_sell_paused`。

| 旧模块 | 处理 | 原因 |
|---|---|---|
| `trading::paper_sell` | 修订失败边界 | 在已经取得原始行后捕获解析、FIFO/T+1 重建和持仓投影失败 |
| `trading::paper_lot_ledger` | 保留 | 继续作为严格时间、FIFO 和 T+1 的唯一计算权威，不为了审计改变错误语义 |
| `database::order_audit` | 拒绝复用表 | 重建失败发生在订单尝试前，伪装成订单会污染订单审计语义 |
| `database::data_acquisition_audit` | 采用模式 | 复用 IMMEDIATE 事务、启动时全链校验、只追加触发器和五年留存语义 |
| `paper_trades` | 只读保留 | 它是来源事实；禁止删除、缩量、改时或跳过 `id=520` 来制造可用样本 |
| `monitor/main.rs` | 不变 | 用户已限定不修改推送与主循环，生产暂停闸继续有效 |

触发数据红线 2.2、2.3、2.7、2.8 和 2.10；没有阈值或配置变更，2.9 的
Threshold-Proof 为 N/A。去重和失败审计规则登记为 BR-249。

### 12.2 数据流与不可变记录

`paper_sell` 仍以参数绑定一次读取全部 `Filled` 行，并保留 `id/code/name/direction/`
`fill_price/quantity/CAST(ts AS TEXT)`。读取成功后，先生成规范化来源快照：文本按原始
UTF-8 保留，可空成交价显式标记，浮点价格使用 IEEE-754 位模式，整数使用固定宽度，
同时保存可读 JSON 和 SHA-256。禁止用 SQLite 时间函数或格式化后的浮点数改变事实。

失败分为 `parse_raw_fill`、`rebuild_fifo` 和 `project_sellable_position` 三个固定阶段。
审计行至少保存：schema 版本、评估日、阶段、固定原因码、原始诊断、来源行数、来源
成交 ID、规范来源 JSON、来源快照哈希、诊断哈希、观察时间、去重身份和至少五年留存
值。审计行及其链行都由 UPDATE/DELETE 触发器保护；每条记录哈希绑定前序哈希与完整
持久化行。进程启动时校验全链，追加时在 SQLite `IMMEDIATE` 事务中校验、写审计、写
链并回读回执；任一步失败全部回滚。

数据库读取失败没有完整来源快照，必须直接报告“来源不可用 + 审计未形成”；数据库
连接或审计追加失败同样显式返回组合错误。禁止把写日志视为持久审计成功，也禁止在
审计失败后继续行情、策略判断或订单。

### 12.3 精确去重与失败返回

同一坏事实可能被 30 秒扫描重复遇到。去重身份固定绑定：规则版本、评估日、失败阶段、
来源快照哈希和诊断哈希。完全相同的重放只返回已存在且重新校验过的审计回执，不新增
行；评估日、任一来源字段或诊断变化都形成新事件。去重只抑制重复审计写入，不把失败
转换为成功，也不缓存/回放持仓结果。

调用方收到的错误必须同时包含原始失败和 `audit_id/record_hash/disposition`。如果审计
本身失败，则返回原始失败和精确审计错误，不得只保留其中一个。由于失败发生在订单前，
不得生成 `order_audit`、`paper_trades` 卖单、行情请求或推送副作用。

### 12.4 验收、发布与回滚

TDD 必须覆盖：解析失败持久化、T+1/超卖重建失败持久化、精确重放不新增、来源或诊断
变化会追加、审计行和链行不可更新/删除、链篡改阻止后续追加、链写失败原子回滚，以及
失败发生后没有订单副作用。回归至少包括 paper FIFO/交易测试、`cargo clippy` 和
`cargo check --bin monitor`。

本修订完成只补齐失败可追溯性，不修复既有历史 T+1 违规，也不使完整策略胜率可用；
`paper_sell_paused` 保持暂停。回滚使用 `git revert <sha>` 回退代码与文档；已写审计表
及记录不得删除，旧程序可忽略该只追加表。任何校验失败按根因返回 Gate A/B，禁止用
清空审计或改写 `paper_trades` 通过测试。

## 13. 2026-08-23 真实验证证据盘点与暂不实施决定

### 13.1 只读实证

对生产数据库 `data/stock_analysis.db` 的 SQLite READ_ONLY 盘点得到：

| 证据 | 只读结果 | 结论 |
|---|---:|---|
| `paper_trades` 的 `Filled` | 898 行，2026-07-10 至 2026-08-14 | 无逐成交费用字段，且被 `id=520` T+1 违规先行阻断 |
| `trades` | 35 行，费用非零行 0 | 不是纸面成交费用来源 |
| `trades` 与 `paper_trades` 精确匹配 | 0 行 | 不能按代码/方向/价格/数量/时间嫁接 |
| `trades.signal_id = paper_trades.plan_id` | 0 行 | 没有不可变身份关联 |
| `stock_daily` | 5,006 行、57 个代码 | 均为个股；没有 `sh000300/000300/399300` 基准历史 |
| `paper_performance_snapshot` | 最近记录交易数、盈亏和风险指标均为 0 | 这是空汇总，不是正向验证证据 |

2026-08-24 对同一生产库再次以 READ_ONLY 复核：库大小 506322944 bytes、mtime
`2026-08-23T23:44:43+0800`、SHA-256
`835a4b0c7089e97a2c174f08466fb50207466ce875e9dc4799e010796eb514cb`，读取前后完全一致。
`paper_trades` 仍为 898 条 Filled 且止于 2026-08-14；`trades` 仍为 35 行，三类费用
非零行均为 0。`stock_daily` 已增长到 5,070 行并覆盖至 2026-08-21，但 57 个代码中仍无
沪深 300 历史。18 份 `user_position_snapshot` 的 `confirm_empty=1` 仍为 0，最新快照明确
非空且含 7 项，因此没有干净验证纪元的权威起点。只读经济仓位探针仍首先由 `id=520`
的 2026-08-11 同日买卖 T+1 违规失败关闭；最新绩效快照仍为 0 成交、0 盈亏。

现有 `pipeline::market_regime` 使用来源可追溯的沪深 300 **实时**指数门控；它不能倒推出
898 条历史成交的开平仓时点基准，也没有逐周期不可变证据 ID。现有回测代码能够在运行
时尝试获取日线基准，但日线收盘与盘中成交时点不等价，且当前数据库没有当时的指数
批次，禁止事后把实时或日线终值包装成精确时点 Alpha。

### 13.2 暂不增加推测性实现

在来源缺失时新增“默认费率”“基准回填”或“市场状态推断”只会扩大接口和表结构，不能
产生可验证结论，因此本轮不新增这些生产实现：

1. 不把 `trades` 的零费用关联到 `paper_trades`，也不把缺失费用补零。
2. 不把代码 `000001` 当上证指数；该库中的 `000001` 是普通 A 股身份。
3. 不用当前实时指数、事后日线收盘或自选股涨跌替代历史开平仓时点基准。
4. 不从策略收益本身反推市场状态，不用无来源标签满足多状态覆盖。
5. 不跳过、删除、缩量或改时 `id=520`；受其影响的存量链路继续整体不可采信。

这是一项控制技术债的 Gate A 决定，不是取消验证。已经完成的严格时间、FIFO/T+1、
经济仓位、费用 typed Unavailable 和 BR-249 失败审计继续保留。

### 13.3 解除阻塞所需的新事实

后续只有在以下事实进入项目后才实施对应适配器：

1. **干净验证纪元：** 从经过确认的空纸面账户状态开始，使用已修复执行器生成的新成交；
   旧 898 行保留作缺陷审计，不并入新纪元。没有不可变空仓确认，不得自行选日期切样本。
2. **费用依据：** 每个成交 ID 一对一的账户实证费用，或由用户明确批准、带来源版本和
   生效期的 Scenario 费率；Scenario 结果不得标成 Observed。
3. **基准依据：** 覆盖每个开平仓时点、具有 canonical instrument、provider、source time、
   observed time、batch/evidence ID 的历史基准价格。对齐/降采样规则须另行登记，禁止隐式
   前向填充或用收盘价代替盘中时点。
4. **市场状态依据：** 同一验证纪元内来源可追溯的指数与市场广度批次，先冻结分类规则，
   再生成逐周期状态标签；标签不得从策略结果反推。

取得以上事实后，先回到 Gate A 登记对齐、聚类和结论规则，再实现逐周期超额收益、按
`(code, entry_date)` 聚类的不确定性和多状态报告。样本仍须达到 200 个闭环、84 天覆盖；
任何条件不足都只能输出 `Unavailable/InsufficientSample/ResearchOnly`。

### 13.4 2026-08-24 现有采集能力完成性审计

对仓库与锁定的 `magic-market-data-rs` revision
`75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e` 做只读调用链审计后，三个阻塞项必须进一步
区分为“正式能力缺口”和“真实事实尚未到达”：

1. **逐成交费用仍是正式能力缺口。** 仓库没有成交/费用导入器；`trades` 的佣金、印花税、
   滑点列来自 `DEFAULT 0` 迁移，唯一真实 INSERT 不写这些列，也不写可与 `paper_trades`
   绑定的成交身份。现有 `TradeEventSource` 事件同样不含费用或结算凭据，运行时代码明确
   报告 broker trade-sync watermark 尚未连接。策略模块中的费率常量只能作为 Scenario，
   不能升级为 Observed。
2. **历史沪深300仍是正式 Gateway 缺口。** 当前 `IndexDataGateway` 只提供 5 秒实时指数
   quote，生产 `get_backtest_daily_data` 明确拒绝指数历史身份，所以
   `fetch_benchmark_series(sh000300)` 不能形成生产序列。锁定上游的 Tencent
   `HistoricalBars` 走只接受 Equity 的校验；TDX normalized `HistoricalBars` 虽未拒绝
   Index，却仍无条件调用股票 `security_bars`，未接入其独立 `get_index_bars/IndexBar`
   协议。底层原语可供未来设计，但当前没有经过身份、协议和真实源验证的指数历史能力。
3. **不可变空仓确认入口已经存在，缺的是事实。** `import_user_position_snapshot` 要求完整
   用户 JSON，只有空 `items` 与显式 `confirm_empty=true` 同时成立才接受；规范化证据哈希后，
   SQLite 以原子事务、唯一身份及禁止 UPDATE/DELETE 的触发器只追加保存。它明确是
   `user_confirmed_full_snapshot`，不是 30 秒券商账户证据，也不得从投影或交易增量推断。
   当前真实库没有任何空仓确认，因此代理不得自行生成或代用户导入。

本审计不新增实现：费用需要真实券商成交/结算来源；指数历史适配需要另立 Gate A，冻结
canonical instrument、指数专用协议、分页、时间对齐、持久证据及真实 provider 验收；空仓
入口只等待真实用户/账户事实。三者未齐前，BR-248 继续 `ResearchOnly`，不释放
`paper_sell_paused`，不启用 R-12 `TechnicalBars`，也不改写旧 898 条成交。

## 14. 2026-08-24 worktree 覆盖率路径归一化

### 14.1 目的与边界

Gate D 的 llvm-cov 报告使用绝对文件名。隔离 worktree 中的路径形如
`<repo>/.worktrees/<branch>/src/...`；现有检查器先按固定 `/stock_analysis/` 截断，得到
`.worktrees/<branch>/src/...`，因此核心前缀一个也匹配不到并错误返回“无核心模块”。本修订
只修复检查器对当前 checkout 的路径识别，不修改报告、覆盖计数、80%/95% 阈值或核心
目录集合，也不把未达标转换成成功。业务规则登记为 BR-250，Threshold-Proof 为 N/A。

### 14.2 归一化合同

检查器必须先把输入路径和当前工作目录都规范为绝对路径，并尝试取得相对当前 checkout
的路径。只要文件确实位于当前 checkout，`<cwd>/src/risk/limits.rs` 和
`<cwd>/src/bin/monitor/main.rs` 就必须分别归一化为 `src/risk/limits.rs` 与
`src/bin/monitor/main.rs`，无论 cwd 本身是否位于 `.worktrees/<name>` 下。

只有输入文件不属于当前 checkout 时，才允许使用既有重复仓库目录兼容逻辑，支持 CI 的
`.../stock_analysis/stock_analysis/src/...`。路径无法相对化且没有受支持仓库标记时保留
显式非核心结果；禁止猜测任意含 `src` 的外部路径属于本仓库。零匹配或零核心行仍必须
返回 exit 2，实际覆盖低于阈值仍必须返回 exit 1。

### 14.3 失败模式、验收与回滚

回归测试必须从当前 `CARGO_MANIFEST_DIR` 构造 worktree 形状的绝对核心文件名，并证明
检查器输出核心文件数量且因低覆盖返回 exit 1，而不是因零匹配返回 exit 2。既有普通
workspace、扩展核心目录和成功阈值测试必须继续通过；真实 coverage JSON 必须输出实际
全局/核心分子分母。实现失败只回滚检查器与测试，不得修改 coverage JSON 或降低阈值。

回滚使用 `git revert <sha>`。本修订不改变策略结果、生产数据、推送、交易或历史成交。
