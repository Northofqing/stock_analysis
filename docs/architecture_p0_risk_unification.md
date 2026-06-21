# P0 风控统一与信号诊断 — 架构设计文档

> 版本: v1.0 | 日期: 2026-06-21 | 关联审查: `docs/quant_institutional_review.md`
> 状态: Step 1 架构设计 (AGENTS.md 流程第 1 步)

---

## 1. 设计目标

基于机构审查报告的 4 个 P0 阻塞项,在不破坏现有 DDD 7+1 Context 架构的前提下,增量修复:

| # | P0 项 | 目标 |
|---|-------|------|
| P0-1 | 恢复风控拦截 | 将注释掉的 3 段拦截改为可配置的策略链,融入主流程 |
| P0-2 | 统一风控执行 | position_tracker 接入 StopLoss + PositionSizer + MarketRegime |
| P0-3 | AI 评分根因分析 | 对各组成因子做 IC/IR 分析,定位方向性错误的来源 |
| P0-4 | position_tracker 测试 | 核心买卖路径达到 ≥95% 行覆盖 |

---

## 2. 数据流图

### 2.1 当前数据流 (AS-IS)

```
┌──────────┐    ┌─────────────────┐    ┌──────────────────┐
│ Market   │    │ Pipeline        │    │ Portfolio        │
│ Context  │    │ (mod.rs)        │    │ (DB)             │
│          │    │                 │    │                  │
│ Kline ───┼───→│ process_stock   │    │ stock_position   │
│ MoneyFlow│    │ _inner()        │    │ analysis_result  │
│ Financial│    │                 │    │ trades           │
│ News     │    │  1. trend_analyzer  │ ledger           │
└──────────┘    │  2. boll_macd      │                  │
                │  3. fundamental     │                  │
                │  4. AI analysis     │                  │
                │  5. ~~risk veto~~ ❌ │  ← 被注释        │
                │  6. score_breakdown │                  │
                │  7. veto_rules      │                  │
                │                    │                  │
                │  position_tracker  │                  │
                │  ┌───────────────┐ │                  │
                │  │ 四大铁律(硬编码)│ │─────────────────→│
                │  │ StopLoss ❌    │ │  开/平仓         │
                │  │ PositionSizer❌│ │                  │
                │  └───────────────┘ │                  │
                └─────────────────┘                    │
                                                       │
┌──────────┐    ┌─────────────────┐                    │
│ Risk     │    │ monitor/risk.rs │                    │
│ Context  │    │                 │                    │
│          │    │ StopLoss ✅     │ ← 已实现但未接入    │
│ limits   │    │ PositionSizer ✅│   主交易路径        │
│ stop_loss│    │ MarketRegime ✅ │                    │
│ cash_grrd│    └─────────────────┘                    │
└──────────┘
```

### 2.2 目标数据流 (TO-BE)

```
┌──────────┐    ┌─────────────────────────────────────┐    ┌──────────┐
│ Market   │    │ Pipeline (mod.rs)                    │    │ Portfolio│
│ Context  │    │                                      │    │ (DB)     │
│          │    │ process_stock_inner()                │    │          │
│ Kline ───┼───→│                                      │    │          │
│ MoneyFlow│    │  1. trend_analyzer                    │    │          │
│ Financial│    │  2. boll_macd                         │    │          │
│ News     │    │  3. fundamental adjustment            │    │          │
└──────────┘    │  4. AI analysis                       │    │          │
                │                                      │    │          │
                │  ┌─────────────────────────────┐     │    │          │
                │  │ 5. VetoChain (NEW)           │     │    │          │
                │  │  ┌─────────────────────────┐ │     │    │          │
                │  │  │ TechnicalVetoRule        │ │     │    │          │
                │  │  │  - 空头排列拦截           │ │     │    │          │
                │  │  │  - 乖离率>5%拦截          │ │     │    │          │
                │  │  ├─────────────────────────┤ │     │    │          │
                │  │  │ MoneyFlowVetoRule        │ │     │    │          │
                │  │  │  - 主力单日净流出>5kw     │ │     │    │          │
                │  │  │  - 价涨量增+主力流出(诱多)│ │     │    │          │
                │  │  ├─────────────────────────┤ │     │    │          │
                │  │  │ FundamentalVetoRule      │ │     │    │          │
                │  │  │  - PE异常+利润大幅下滑    │ │     │    │          │
                │  │  └─────────────────────────┘ │     │    │          │
                │  │  每个规则独立裁决 → 汇聚      │     │    │          │
                │  └─────────────────────────────┘     │    │          │
                │                                      │    │          │
                │  6. score_breakdown                   │    │          │
                │  7. veto_rules (existing Phase1/3)    │    │          │
                │                                      │    │          │
                │  position_tracker (REFACTORED)        │    │          │
                │  ┌───────────────────────────────┐   │    │          │
                │  │ 买入: PositionSizer.max_pos()  │   │──────────→│
                │  │       + MarketRegime 门控      │   │    │ 开仓   │
                │  │ 卖出: StopLoss.effective()     │   │──────────→│
                │  │       + check_stops() 三级止损  │   │    │ 平仓   │
                │  │ T+1:  PositionType 锁仓检查    │   │    │          │
                │  └───────────────────────────────┘   │    │          │
                └─────────────────────────────────────┘    └──────────┘

┌──────────────┐    ┌──────────────────────────┐
│ Signal       │    │ analysis/factor_ic.rs    │
│ Context (NEW)│    │ (P0-3: AI 评分诊断)       │
│              │    │                          │
│ FactorIC ────┼───→│ IC(t) = corr(factor,     │
│ FactorIR     │    │         fwd_return)       │
│ ICHistory    │    │ IR = mean(IC)/std(IC)     │
└──────────────┘    │ 5 因子 × 滚动窗口        │
                    │ → IC 衰减曲线            │
                    │ → 因子相关性矩阵          │
                    └──────────────────────────┘
```

---

## 3. 架构决策

### ADR-1: VetoChain 策略模式 — 风控拦截重构

**决策**: 用 `VetoRule` trait + `VetoChain` 责任链替代注释掉的内联代码。

```rust
// src/risk/veto_chain.rs (NEW)

/// 否决规则的统一接口
pub trait VetoRule: Send + Sync {
    /// 规则名称 (用于日志和审计)
    fn name(&self) -> &'static str;
    /// 评估是否触发否决
    fn evaluate(&self, ctx: &VetoContext) -> VetoVerdict;
    /// 规则优先级 (数字越小越先执行)
    fn priority(&self) -> u8 { 50 }
}

#[derive(Debug, Clone)]
pub struct VetoVerdict {
    /// 触发的风险标签
    pub risk_flags: Vec<String>,
    /// 评分下调量 (若需要)
    pub score_penalty: i32,
    /// 是否强制降级买入信号为 Hold
    pub force_hold: bool,
}

#[derive(Debug, Clone)]
pub struct VetoContext {
    pub code: String,
    pub current_price: f64,
    pub signal_score: i32,
    pub buy_signal: BuySignal,
    pub bias_ma5: f64,
    pub trend_status: TrendStatus,
    /// 资金流数据 (可选,规则自行判断)
    pub money_flow_days: Option<Vec<MoneyFlowDay>>,
    /// 基本面数据
    pub pe_ratio: Option<f64>,
    pub net_profit_yoy: Option<f64>,
}
```

**具体规则实现** (内聚在 `src/risk/veto_rules_live.rs`):

| 规则 Struct | 原注释位置 | 触发条件 |
|------------|-----------|---------|
| `BiasRateRule` | mod.rs:688-704 | bias_ma5 > 5% 或 空头排列 |
| `MainFlowRule` | mod.rs:706-728 | 主力单日净流出 > 5kw 或 诱多形态 |
| `FundamentalDeteriorationRule` | mod.rs:730-740 | PE<0 或 >300 且 净利同比 < -30% |

**VetoChain 执行逻辑**:
```rust
pub struct VetoChain {
    rules: Vec<Box<dyn VetoRule>>,
}

impl VetoChain {
    pub fn evaluate_all(&self, ctx: &VetoContext) -> VetoOutcome {
        let mut outcome = VetoOutcome::default();
        for rule in &self.rules {
            let verdict = rule.evaluate(ctx);
            if !verdict.risk_flags.is_empty() {
                outcome.flags.extend(verdict.risk_flags);
                outcome.total_penalty += verdict.score_penalty;
                if verdict.force_hold {
                    outcome.force_hold = true;
                }
            }
        }
        outcome
    }
}
```

**为什么不用已有 `pipeline/veto_rules.rs`**:
- 现有 `veto_rules.rs` 侧重**基本面估值否决** (营收连降/PE分位/CFO含金量),属于 Phase 1/3
- 被注释的 3 段是**实时技术面+资金面否决**,属于 Phase 2 (核心拦截器)
- 两者互补,不冲突: VetoChain 先执行 → 然后 veto_rules::evaluate 补充

### ADR-2: position_tracker 接入 Risk 模块

**决策**: position_tracker 从"自包含硬编码"改为"依赖注入 Risk 组件"。

```rust
// 修改后的 track_position 签名
pub(super) fn track_position(
    code: &str,
    data: &[KlineData],
    result: &mut AnalysisResult,
    risk_config: &RiskConfig,  // NEW: 风控配置注入
) { ... }
```

**买入路径变更**:
```
当前: position_shares(price)  → 总本金/最大仓位数, 向下取整百股
改为: PositionSizer::max_position(regime, volatility, chain_pos, chain_frozen, already_held)
      + MarketRegime::allow_new_position() 门控
      + 取 min(PositionSizer结果, HardLimits.single_stock_max_pct × total_capital)
```

**卖出路径变更**:
```
当前: 四大铁律硬编码 (8%/-20%/5日线/14天/布林上轨)
改为: StopLoss::triggered(current_price)           ← 替代 铁律1 (8%固定止损)
      + check_stops(code,name,price,cost,hard,ma20,ma60) ← 三级止损补充
      + 保留铁律2/3/4/5 作为额外保护层
```

**T+1 锁仓检查 (新增)**:
```
买入时: 检查 PositionType::Locked { unlock_date }
卖出时: 若 PositionType::is_locked() → 阻止卖出,输出提醒
```

### ADR-3: AI 评分因子分析模块

**决策**: 新增 `src/analysis/` 目录 (Signal Context 扩展),纯诊断模块,不参与交易路径。

```
src/analysis/
├── mod.rs              # 模块入口
├── factor_ic.rs        # IC/IR 计算核心
└── factor_report.rs    # Markdown 报告生成
```

**数据结构**:
```rust
/// 单因子 IC 分析结果
pub struct FactorIC {
    pub factor_name: String,
    /// IC 序列 (每个调仓周期的 rank IC)
    pub ic_series: Vec<f64>,
    /// 累计 IC
    pub cumulative_ic: f64,
    /// Information Ratio = mean(IC) / std(IC)
    pub information_ratio: f64,
    /// IC 胜率 (IC > 0 的比例)
    pub ic_win_rate: f64,
    /// IC 衰减: lag=1,2,3,4 期的 IC 均值
    pub ic_decay: [f64; 4],
    /// t 统计量
    pub t_stat: f64,
}

/// 多因子分析汇总
pub struct FactorAnalysisReport {
    pub analysis_date: NaiveDate,
    pub sample_stocks: usize,
    pub sample_periods: usize,
    pub factors: Vec<FactorIC>,
    pub factor_correlation: Vec<Vec<f64>>,  // 因子间相关性矩阵
    pub sentiment_score_ic: Option<FactorIC>, // AI 综合评分本身的 IC
}
```

**分析方法**:
- 使用 Spearman Rank IC (更稳健,不要求正态分布)
- 对 ScoreBreakdown 的 5 个维度 (technical/fundamental_quality/valuation_safety/capital_flow/growth_sustainability) 分别计算 IC
- 对 sentiment_score (AI 综合评分) 计算 IC — 预期为负,验证方向性错误
- 通过 `cargo run --bin monitor -- --review` 触发分析

### ADR-4: 测试架构

**决策**: position_tracker 的测试遵循 Clean Architecture — 通过注入 DB mock 隔离外部依赖。

```rust
// src/pipeline/position_tracker.rs 底部或 tests/position_tracker_tests.rs
#[cfg(test)]
mod tests {
    use super::*;

    // 测试场景矩阵:
    // 买入路径:
    //   - BottomBuy 触发买入
    //   - UptrendStart 触发买入
    //   - contrarian_signal 触发买入
    //   - 无信号不买入
    //   - 崩盘 MarketRegime 拒绝买入
    //   - 同股已持仓拒绝重复买入
    // 卖出路径:
    //   - 铁律1: 亏损 ≥ 8% 触发卖出
    //   - 铁律2: 盈利 < 20% 不卖出
    //   - 铁律3: 盈利 ≥ 20% + 跌破5日线触发卖出
    //   - 铁律4: >14天仍亏损触发卖出
    //   - 铁律5: 布林上轨+MACD顶背离触发卖出
    //   - StopLoss.triggered() 触发卖出
    //   - T+1 锁仓阻止卖出
    // 边界:
    //   - current_price = 0 不 panic
    //   - 空 data 不 panic
    //   - DB 不可用优雅降级
}
```

---

## 4. 失败模式分析

### 4.1 VetoChain 执行失败

| 失败场景 | 处理策略 | 降级行为 |
|---------|---------|---------|
| MoneyFlow 数据源不可用 | warn 日志, MainFlowRule 返回空 VetoVerdict | 跳过资金面拦截,其他规则继续 |
| KlineData 缺少 PE/净利润 | warn 日志, FundamentalRule 返回空 | 跳过基本面拦截 |
| VetoChain 规则 panic | `std::panic::catch_unwind` 包裹每个规则 | 单规则失败不影响其他规则 |
| 所有规则都失败 | error 日志 | 不否决任何信号 (宁可放过不可错杀的原则不适用——改为保守:全部失败则强制 Hold) |

### 4.2 PositionSizer/StopLoss 计算失败

| 失败场景 | 处理策略 |
|---------|---------|
| ATR 计算返回 0 或 NaN | 回退到默认 3% ATR, 记录 warn |
| MarketRegime 无法判定 (上涨家数数据缺失) | 默认为 Structural (中性), 记录 warn |
| 产业链持仓计数失败 | chain_penalty = 0, 不做集中度折扣 |

### 4.3 FactorIC 分析失败

| 失败场景 | 处理策略 |
|---------|---------|
| 历史交易数据不足 (< 30 笔) | 跳过分析,输出 "样本不足" |
| 因子数据缺失 | 该因子 IC 标记为 None, 不参与相关性矩阵 |
| 计算溢出 | f64 饱和处理, NaN → 报错跳过 |

---

## 5. 回滚方案

### 5.1 VetoChain 回滚

VetoChain 通过 `config/veto_rules.toml` 的 `[live_veto]` section 控制:

```toml
[live_veto]
enabled = true                # 主开关: false 则完全跳过 VetoChain
bias_rate_enabled = true      # 乖离率拦截开关
bearish_alignment_enabled = true  # 空头排列拦截开关
main_flow_enabled = true      # 主力资金拦截开关
fundamental_enabled = true    # 基本面拦截开关
```

- 紧急回滚: 设置 `enabled = false` (支持 SIGHUP 热加载,无需重启)
- 单规则回滚: 关闭对应 `*_enabled = false`
- 代码回滚: VetoChain 在 `process_stock_inner` 中作为独立调用块,可一行 `if false` 绕过

### 5.2 PositionSizer 回滚

```toml
[position_sizing]
use_dynamic = true            # false = 回退到旧 position_shares()
```

保留旧 `position_shares()` 函数,通过 feature flag 切换。

### 5.3 FactorIC 回滚

FactorIC 是完全独立的诊断模块,不影响任何生产路径。回滚只需不调用。

---

## 6. 与旧模块的关系说明

| 旧模块 | 位置 | 关系 | 决策 |
|--------|------|------|------|
| `pipeline/veto_rules.rs` | Phase 1/3 否决规则 | **互补** — VetoChain 做实时技术/资金面否决,veto_rules 做基本面估值否决 | 不修改 veto_rules.rs,在流程中先执行 VetoChain 再执行 veto_rules::evaluate |
| `monitor/risk.rs` | StopLoss, PositionSizer, MarketRegime | **直接复用** — position_tracker 接入 | 不修改 monitor/risk.rs,仅在 position_tracker 中使用其公开 API |
| `risk/stop_loss.rs` | check_stops 三级止损 | **接入** — 作为卖出路径补充 | 不修改,在 position_tracker 卖出逻辑中调用 |
| `risk/limits.rs` | HardLimits 硬约束 | **接入** — 买入前做单票仓位上限检查 | 不修改,在 position_tracker 买入逻辑中调用 |
| `pipeline/position_tracker.rs` | 买卖执行核心 | **重构** — 从硬编码改为依赖注入 | 主修改目标 |
| `pipeline/mod.rs` | 主流程调度 | **修改** — 插入 VetoChain 调用,传递 RiskConfig | 小范围修改 |
| `strategy/` 各策略 | 信号生成 | **不修改** — VetoChain 在信号生成后执行 | 无变更 |
| `config/monitor.toml` | 监控配置 | **扩展** — 新增 veto 和 position_sizing section | 增加配置项,保留旧默认值 |

---

## 7. 文件变更清单

### 新增文件

| 文件 | 说明 |
|------|------|
| `src/risk/veto_chain.rs` | VetoRule trait + VetoChain + VetoContext/VetoVerdict |
| `src/risk/veto_rules_live.rs` | 3 个具体规则实现 (BiasRate/MainFlow/Fundamental) |
| `src/analysis/mod.rs` | 分析模块入口 |
| `src/analysis/factor_ic.rs` | IC/IR 计算 |
| `src/analysis/factor_report.rs` | Markdown 报告生成 |
| `tests/pipeline/position_tracker_tests.rs` | position_tracker 全面测试 |
| `tests/risk/veto_chain_tests.rs` | VetoChain 单元测试 |
| `config/veto_rules.toml` | 否决规则配置 |

### 修改文件

| 文件 | 变更范围 |
|------|---------|
| `src/risk/mod.rs` | 添加 `pub mod veto_chain; pub mod veto_rules_live;` |
| `src/pipeline/mod.rs` | 1) 插入 VetoChain 调用 (替换注释块 686-740) 2) 传递 RiskConfig 给 position_tracker |
| `src/pipeline/position_tracker.rs` | 1) 买入: PositionSizer 替代 position_shares 2) 卖出: StopLoss + check_stops 增强 3) T+1 锁仓检查 |
| `src/config.rs` | 新增 VetoConfig / RiskConfig 反序列化 |
| `config/monitor.toml` | 新增 [live_veto] 和 [position_sizing] section |
| `src/lib.rs` 或 `src/main.rs` | 注册 analysis 模块 |

---

## 8. 配置热加载设计

遵循现有 SIGHUP 机制 (`src/config.rs` 中的 `GlobalConfig`):

```
SIGHUP → GlobalConfig::reload()
  ├── monitor.toml [live_veto]     → VetoChain 重建规则列表
  ├── monitor.toml [position_sizing] → PositionSizer 参数更新
  └── veto_rules.toml              → 各规则阈值热更新
```

热加载不重建 VetoChain 实例 (避免并发问题),而是通过 `Arc<RwLock<VetoChainConfig>>` 原子切换配置。

---

## 9. 不变量与约束

- **MUST** VetoChain 的执行必须在 `sentiment_score` 最终确定之后、`score_to_advice` 映射之前
- **MUST** position_tracker 的 StopLoss 检查在 DB 操作之前完成 (先判断再写入,避免写入后又回滚)
- **MUST** AI 评分 FactorIC 分析仅使用已平仓交易的历史数据,不基于未平仓持仓 (避免幸存者偏差)
- **MUST NOT** 修改 `monitor/risk.rs` 的公开 API (其他调用方依赖)
- **MUST NOT** 在 FactorIC 分析路径中引入任何 mock 数据

---

*下一步: Step 2 四角色挑战 (AGENTS.md 流程第 2 步)*

---

## 附录 A: 四角色挑战记录 (Step 2)

> 日期: 2026-06-21 | 轮次: 1/3 | 结论: 6 Blocking 已闭环,通过

### Blocking 异议及决议

#### B1 (PM): 全部规则失败 → 强制 Hold 过于激进

**决议**: 修改失败模式处理策略。规则无法判定 (数据缺失) 时 pass-through + warn;仅规则明确触发时才否决。

> 更新 4.1 VetoChain 执行失败表:
> | 所有规则都失败 | error 日志 | **pass-through (不否决)** + 推送告警,等待人工判断 |

#### B2 (PM): 缺少灰度 dry-run 模式

**决议**: 上线首周默认 `mode = "dry_run"`,仅记录不拦截;第二周起 `mode = "live"`。

```toml
[live_veto]
enabled = true
mode = "dry_run"   # "dry_run" | "live"
# dry_run: 评估所有规则,记录日志但不修改 signal_score
# live:     评估并实际拦截
```

#### B3 (Dev): MoneyFlow 获取时序与 VetoChain 执行冲突

**决议**: 调整 `process_stock_inner` 流程 —— 先并行获取所有外部数据 (MoneyFlow/财务/新闻),再依次执行布林MACD → 基本面修正 → VetoChain → AI 分析。VetoChain 执行时需要的数据必须已就绪。

```
调整后流程:
  1. trend_analyzer
  2. boll_macd
  3. 并行获取: MoneyFlow + Financials + News  ← 提前
  4. fundamental adjustment
  5. VetoChain (此时所有数据已就绪)           ← 原注释位置
  6. AI analysis
  7. score_breakdown
  8. veto_rules (Phase 1/3)
```

#### B4 (Analyst): Veto 规则阈值未经数据验证

**决议**: 在 `--review` 路径新增规则回测验证,对每条规则计算:
- 拦截交易数 / 总交易数
- 拦截交易中实际亏损比例 (Precision)
- 亏损交易中被拦截比例 (Recall)
- 若 Precision < 50%,输出告警建议调整阈值

#### B5 (Architect): VetoContext 依赖 trend_analyzer 类型

**决议**: VetoContext 使用基础类型,Pipeline 层负责转换:

```rust
// 修改后 (不依赖 trend_analyzer)
pub struct VetoContext {
    pub code: String,
    pub current_price: f64,
    pub signal_score: i32,
    pub is_strong_buy: bool,      // 代替 BuySignal::StrongBuy
    pub bias_ma5: f64,
    pub is_bearish: bool,         // 代替 TrendStatus::StrongBear|Bear
    pub money_flow_days: Option<Vec<MoneyFlowDay>>,
    pub pe_ratio: Option<f64>,
    pub net_profit_yoy: Option<f64>,
}
```

#### B6 (Architect): analysis/ 放 Signal Context 不合适

**决议**: FactorIC 移至 Review Context (`src/review/factor_ic.rs`),与 `review/falsify.rs` 同层,明确"事后诊断"定位。

### 非 Blocking 建议 (记录,后续迭代)

| # | 建议 | 处理 |
|---|------|------|
| S1 | VetoChain 拦截事件纳入每日推送摘要 | P1 backlog |
| S2 | 封装 RiskContext struct 避免参数膨胀 | 本次实现时一并封装 |
| S3 | FactorIC 加数据量上限 (500 笔) | 本次实现 |
| S4 | StopLoss 与 铁律1 去重 — 选用 StopLoss | 本次实现 |
| S5 | 明确 monitor/risk vs risk/ 分工文档 | 补充至 CLAUDE.md |

---

## 附录 B: 更新后的文件变更清单

### 新增文件

| 文件 | 说明 | 变更 |
|------|------|------|
| `src/risk/veto_chain.rs` | VetoRule trait + VetoChain (VetoContext 用基础类型) | - |
| `src/risk/veto_rules_live.rs` | 3 个具体规则 (BiasRate/MainFlow/Fundamental) | - |
| `src/review/factor_ic.rs` | IC/IR 计算 (从 analysis/ 移至 review/) | **位置调整** |
| `src/review/factor_report.rs` | Markdown 报告生成 | **位置调整** |
| `tests/pipeline/position_tracker_tests.rs` | position_tracker 全面测试 | - |
| `tests/risk/veto_chain_tests.rs` | VetoChain 单元测试 | - |
| `config/veto_rules.toml` | 否决规则配置 (含 mode=dry_run) | **新增 mode 字段** |

### 修改文件

| 文件 | 变更范围 |
|------|---------|
| `src/risk/mod.rs` | 添加 `pub mod veto_chain; pub mod veto_rules_live;` |
| `src/review/mod.rs` | 添加 `pub mod factor_ic; pub mod factor_report;` |
| `src/pipeline/mod.rs` | 1) 调整数据获取时序 2) 插入 VetoChain 调用 3) 传递 RiskContext |
| `src/pipeline/position_tracker.rs` | 1) 买入: PositionSizer + MarketRegime 2) 卖出: StopLoss 替代铁律1 3) T+1 锁仓 |
| `src/config.rs` | 新增 VetoConfig / RiskConfig 反序列化 |
| `config/monitor.toml` | 新增 `[live_veto]` (含 mode) 和 `[position_sizing]` section |
| `CLAUDE.md` | 补充 monitor/risk vs risk/ 分工说明 |
