# Magic TDX 真实持仓做T价格区间设计

日期：2026-07-23

状态：Gate A — 待用户书面审阅

规则：数据红线 2.1、2.2、2.3、2.4、2.6、2.7、2.10；BR-047、BR-097、BR-116、BR-151、BR-153

## 1. 目标

把当前“主力净流触发 + 当前涨跌幅对称 ±N%”的做T消息替换为可验证的反T观察计划。计划必须结合真实日线趋势、当日 5 分钟量价、量能节奏和五档盘口，给出窄价格区间、触发条件、失效条件和相同数量的高抛/接回腿。

本功能只生成辅助观察建议，不提交订单、不修改真实持仓、不推断券商可卖数量，也不因数据缺失生成机械点位。

## 2. 已确认方案与备选

采用方案 A：

- 日线结构确定趋势和 ATR14。
- 5 分钟量价确定盘中摆动点、量能节奏和触发状态。
- Magic TDX 五档盘口确认买卖侧承接。

未采用方案：

- 方案 B（仅日线）：稳定，但盘中点位过粗。
- 方案 C（仅 5 分钟）：灵敏，但更容易受单根 K 线噪声影响。

## 3. 当前代码事实

### 3.1 现有生产路径使用机械百分比

复现命令：

```bash
rg -n -A120 "if last_t0_scan.elapsed" src/bin/monitor/main.rs
```

HEAD 证据：

```text
src/bin/monitor/main.rs:7972:if last_t0_scan.elapsed().as_secs() >= 30 {
src/bin/monitor/main.rs:8061:for e in detector_local.scan_stock(&snap) {
src/bin/monitor/main.rs:8093:"高抛: +{:.1}% 卖出约{}股"
src/bin/monitor/main.rs:8094:"低吸: -{:.1}% 回补约{}股"
src/bin/monitor/main.rs:8097:snap.change_pct.abs().max(2.0)
```

这条路径没有计算支撑、压力、ATR、盘中均价或 5 分钟量能；买回数量也可能与卖出数量不同。

### 3.2 已有安全决策模块未接入生产 T0 producer

复现命令：

```bash
rg -n "pub fn evaluate|sell_zone|buy_zone|render_t0_advice" \
  src/decision/t0_advisor.rs src/bin/monitor/push_templates.rs src/bin/monitor/main.rs
```

HEAD 证据：

```text
src/decision/t0_advisor.rs:112:pub fn evaluate(input: &T0Input) -> T0Verdict
src/decision/t0_advisor.rs:159:let sell_zone = ...
src/decision/t0_advisor.rs:163:let buy_zone = ...
src/bin/monitor/push_templates.rs:535:pub fn render_t0_advice(...)
```

`main.rs` 的真实持仓 producer 没有调用该评估器或模板，而是在大函数里直接格式化字符串。

### 3.3 Magic TDX 原始能力足够

复现命令：

```bash
rg -n "KLINE_5MIN|KLINE_RI_K|get_security_bars|get_security_quotes|get_minute_time_data" \
  ../magic-market-data-rs/crates/magic-tdx-rs/src
rg -n "bid[1-5]|ask[1-5]|bid_vol[1-5]|ask_vol[1-5]" \
  ../magic-market-data-rs/crates/magic-tdx-rs/src/protocol/types.rs
```

HEAD 证据：

```text
protocol/constants.rs:96:pub const KLINE_5MIN: u8 = 0;
protocol/constants.rs:105:pub const KLINE_RI_K: u8 = 9;
net/client.rs:729:pub fn get_security_bars(...)
net/client.rs:1231:pub fn get_minute_time_data(...)
protocol/types.rs:62..81:五档 bid/ask price 与 volume 字段
```

现有 `MagicTdxProvider` 已接入真实日线和报价，但丢弃了 5 分钟线、分时均价及五档盘口，需要在同一 provider 内补充强类型 T0 证据。

## 4. 架构

### 4.1 数据获取边界

`MagicTdxProvider::get_t0_evidence_batch(codes)` 在一个 blocking worker 内完成：

1. 建立一次 Magic TDX 连接。
2. 批量获取所有持仓实时行情与五档盘口。
3. 对每只股票获取 30 根未复权日线、300 根 5 分钟 K 线和当日分时均价。
4. 报价批次失败则整批失败；单票日线、分钟线或盘口失败只隔离该票，其他完整票继续。
5. 每条完整记录保留 provider、provider source time、local observed time 和基于原始关键字段计算的稳定 batch ID。

新增强类型：

```rust
pub struct MagicTdxT0Evidence {
    pub code: String,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub quote: MagicTdxT0Quote,
    pub settled_daily: Vec<MagicTdxDailyBar>,
    pub completed_five_minute: Vec<MagicTdxFiveMinuteBar>,
    pub intraday_average_price: f64,
}

pub struct MagicTdxT0Batch {
    pub records: Vec<MagicTdxT0Evidence>,
    pub rejections: Vec<MagicTdxT0Rejection>,
}
```

不向通用 `DataProvider` trait 塞入 T0 专用字段，避免影响其他数据源和既有 fallback 语义。

### 4.2 纯决策边界

`decision::t0_advisor` 接收一条完整市场证据与一条用户确认持仓，输出：

```rust
pub enum T0PlanDecision {
    Advice(T0StructuredPlan),
    Forbidden(T0ForbiddenPlan),
    Rejected(T0Rejection),
}
```

- `Advice`：完整价格区间、触发状态、失效条件和证据摘要。
- `Forbidden`：证据完整，但因主升核心、退潮或价差不足而明确禁做。
- `Rejected`：证据缺失、过期或非法；只写诊断，不渲染推送。

所有技术指标和点位计算保持纯函数，网络、数据库、日志和推送不进入决策模块。

### 4.3 生产集成

`main.rs` 保留现有 30 秒调度和 `PushKind::T0Advice` 票级治理，但把内联 `Detector` 路径替换为：

```text
用户确认完整快照
  → blocking Magic TDX T0 batch
  → 逐票严格校验
  → t0_advisor 纯决策
  → push_templates 结构化渲染
  → push_governor_v3(..., Some(code))
  → delivery audit
```

若不存在用户确认快照，旧持仓源只保留身份兼容；缺成本、总持仓或数据新鲜度时不生成结构化 T0 点位。

## 5. 数据合同与校验

### 5.1 实时报价与盘口

- 价格、昨收、当日高低必须有限且大于 0。
- `low <= min(open, price) <= max(open, price) <= high`。
- provider source time 必须可解析，且距当前不超过 5 秒。
- 五档价格必须正且单调：bid1 ≥ ... ≥ bid5，ask1 ≤ ... ≤ ask5，bid1 < ask1。
- 五档数量必须有限且非负；任一档缺失或非法拒绝该票。
- `bid_ask_ratio = sum(bid_vol1..5) / sum(ask_vol1..5)`；分母为 0 时拒绝，不补默认比率。

### 5.2 日线

- 只使用最近已完成交易日的未复权日线；盘中当天未结算日 K 排除。
- 至少 20 根，通过既有价格正数、OHLC、日期重复、交易日连续和相邻变化 ±20% 校验。
- ATR14 使用真实范围 `max(high-low, abs(high-prev_close), abs(low-prev_close))` 的 14 日均值。
- MA5、MA10、MA20 只使用已结算收盘价。

### 5.3 5 分钟量价

- 只使用 quote source time 之前已完成的 5 分钟 K 线，当前形成中 K 线排除。
- OHLC 必须正且满足低高关系；volume、amount 必须有限且非负。
- 同一交易日时间戳不得重复，必须处于 09:30–11:30 或 13:00–15:00，并按 5 分钟槽连续。
- 当日至少 6 根已完成 K 线。
- 量能节奏比：

```text
pace_ratio =
  当日前 N 个已完成 5 分钟槽累计成交量
  / 最近 5 个可用历史交易日前 N 槽累计成交量均值
```

历史同槽交易日至少 3 个，否则拒绝。

- 最近 5 分钟量比：

```text
last_bar_volume_ratio =
  最近已完成 5 分钟成交量
  / 历史同一时间槽成交量均值
```

- 当日均价直接使用 Magic TDX 分时协议的 `avg_price`，并验证其位于当日已完成 K 线最低价与最高价之间。

## 6. 趋势、点位与触发

### 6.1 趋势

- 主升核心：`price > MA5 > MA10 > MA20`、5 日收益 ≥ 8%、`pace_ratio ≥ 1.5`；禁止反T，防止卖飞。
- 主升：`price > MA10` 且 `MA5 > MA10 > MA20`。
- 退潮：`price < MA20` 且 `MA5 < MA10 < MA20`；禁止做T。
- 走弱：`price < MA10` 或 `MA5 < MA10`。
- 其余为震荡。

5 日收益使用最新已结算收盘与第 6 根已结算收盘计算，不用盘中价格冒充结算价。

### 6.2 结构位

候选结构位：

- 最近 20 个交易日、左右各两根 K 线确认的摆动高点/低点。
- 当日已完成 5 分钟、左右各一根确认的摆动高点/低点。
- Magic TDX 当日分时均价。
- 当真实摆动点不存在时，使用 `current_price ± 0.5×ATR14` 技术投影，并在证据摘要中标记为 ATR 投影。

压力中心取现价上方最近且距离至少 0.3% 的候选；支撑中心取现价下方最近且距离至少 0.3% 的候选。

区间半宽：

```text
half_width = clamp(
  0.1 × ATR14,
  current_price × 0.15%,
  current_price × 0.35%
)
```

要求：

- 所有区间价格有限且大于 0。
- `sell_zone.low > buy_zone.high`。
- `(sell_zone.low / buy_zone.high - 1) × 100 ≥ 1.5%`。
- 不满足则 `Forbidden("结构价差不足以覆盖往返成本和滑点")`，不得缩窄风险阈值凑结果。

### 6.3 触发和失效

高抛触发：

- 当前价进入高抛区；
- 最近完成 5 分钟量比 ≥ 1.2；
- 五档卖量/买量 ≥ 1.2。

接回触发：

- 当前价进入接回区；
- 回踩阶段最近完成 5 分钟量比 ≤ 0.8；
- 最近完成 5 分钟 K 线收盘高于开盘；
- 五档买量/卖量 ≥ 1.2。

失效：

- 连续两根已完成 5 分钟收盘站上高抛区上沿：取消高抛计划，视为放量突破。
- 最近完成 5 分钟收盘跌破接回区下沿：取消接回计划，禁止接下跌刀。

未触发时仍可展示“等待”状态和具体条件，但仅当完整结构价差存在且当前价距任一观察区不超过 `0.5×ATR14`；距离更远则不推送，避免无行动价值的噪声。

## 7. 数量与资金安全

- 只生成反T（先高抛、后接回），不生成需要新增现金的正T。
- `leg_quantity = floor((confirmed_total_quantity / 3) / 100) × 100`。
- `leg_quantity < 100`：拒绝，不渲染。
- 高抛与接回数量严格相同。
- 用户快照总持仓不得显示为“可卖底仓”；文案必须写“观察数量”，并提示实际可卖数量以券商为准。
- 用户快照不含买入日期和券商可卖数量，因此本计划不声称通过 T+1 可卖校验。任何实际执行都必须另行取得 30 秒内券商可卖批次；本功能没有该批次，故永远停留在观察建议。
- 本功能不调用订单接口，因此不创建 business order ID，也不写真实或纸面成交。

## 8. 消息合同

允许计划示例：

```text
🔄 做T观察【真实持仓】 华电辽能(600396)
持仓: 500股 | 观察数量: 100股（实际可卖以券商为准）
现价: ¥16.12 | 趋势: 主升
量价: 节奏比1.46 | 最近5分钟量比1.31 | TDX均价¥15.83 | ATR14 ¥0.64
盘口: 买/卖量比0.78（卖压1.28）
高抛区: ¥16.28~¥16.38
  触发: 进入区间 + 5分钟量比≥1.2 + 卖压≥1.2
接回区: ¥15.74~¥15.84
  触发: 缩量≤0.8后5分钟翻红 + 买压≥1.2
失效: 连续2根5分钟站上¥16.38取消高抛；跌破¥15.74取消接回
当前: 等待高抛触发
证据: Magic TDX 09:45:58 | batch=<短哈希>
辅助观察，不是下单指令
```

禁做计划展示完整证据与明确原因；数据拒绝不推送，只记录脱敏 batch、reason code 和 retryable。

## 9. 失败模式

- Magic TDX 连接或批量报价失败：整轮失败，30 秒后重试。
- 单票日线/5 分钟线/分时/盘口失败：隔离该票，不影响其他完整持仓。
- 报价超过 5 秒、休市、午间暂停或非连续竞价：不生成计划。
- 用户快照缺成本、数量不足、身份不一致：逐票拒绝。
- 不能计算历史同槽量比、ATR 或结构位：逐票拒绝。
- 推送失败：保持立即重试资格；Deduped 视为已有同一业务建议，不重复物理投递。
- 不允许回退到东财/新浪行情、主力净流、成本价、涨跌幅百分比或默认点位。

## 10. 旧模块处置

| 模块 | 处置 | 原因 |
|---|---|---|
| `monitor::detector` | T0 路径拒绝，其他通用异动告警保留 | 只能产生异动事件，不能提供结构点位 |
| `decision::t0_advisor` | 采用并深化 | 保留主升核心、退潮和结构价差安全语义；把不可从用户快照验证的 T+1/可卖状态从市场观察计划中明确分离 |
| `push_templates::render_t0_advice` | 采用并扩展 | 统一结构化文案，替代 `main.rs` 内联格式化 |
| `MagicTdxProvider` | 采用并扩展 | 作为唯一 T0 市场证据源 |
| `fetch_eastmoney_quotes` / `fetch_sina_quotes` | T0 路径拒绝，其他路径保留 | 防止不同源时间与字段混批 |
| 用户确认持仓快照 | 采用 | 仅作为身份、成本和总持仓事实，不冒充券商可卖数量 |
| 全局 `DataMode::OrderBook` | 本次不修改 | T0 使用票级 Magic TDX 深度证据；全局能力探针是独立任务 |

## 11. 测试与验收

模块测试：

- Magic TDX 原始报价、盘口、日线、5 分钟线和分时均价的严格转换。
- 5 秒 freshness、形成中 K 线排除、午间时间槽、重复/缺口、坏 OHLC、坏盘口拒绝。
- 历史同槽量能节奏、ATR14、MA 趋势、摆动点、ATR 投影和区间宽度。
- 主升核心/退潮禁做、价差不足禁做、数量不足拒绝、两腿数量相等。
- 触发、等待、突破失效和跌破失效。
- source audit：生产 T0 路径不再包含 `snap.change_pct.abs().max(2.0)`，且存在 `t0_advisor` 与 `render_t0_advice` 调用。

Gate C：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

Gate D：

```bash
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

隔离 E2E 必须到达 `[v70] E2E 完成`；真实数据探针必须显示 Magic TDX 完整或显式逐票拒绝，不能出现机械百分比。最终生产证据要求：

```bash
rg -n -A3 "T0Advice|做T观察|BR-153" src/bin/monitor
rg -l "^🔄 做T观察" data/push_log/$(date +%Y-%m-%d) | head -3
rg -c '"kind":"t0_advice_v1".*"outcome":"Pushed"' \
  data/event_audit/$(date +%Y).jsonl
```

若当日没有真实完整触发，则状态保持 In Progress；不得用测试推送冒充生产证据。

## 12. 发布与回滚

发布前停止当前 debug monitor，构建 release 后只启动一个实例。观察至少一个 30 秒 T0 周期，确认无 Tokio runtime-drop panic、无双实例、无机械百分比消息。

回滚按实现提交逆序执行 `git revert` 并重建 release。若回滚会恢复机械百分比 T0 producer，必须同时禁用该 producer，而不是继续推送旧建议。任何回滚不得删除用户快照、行情证据、push log 或 delivery audit。
