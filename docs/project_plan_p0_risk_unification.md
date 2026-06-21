# P0 风控统一与信号诊断 — 项目计划

> 版本: v1.0 | 日期: 2026-06-21 | 关联: `docs/architecture_p0_risk_unification.md`
> 状态: Step 3 项目计划 (AGENTS.md 流程第 3 步)

---

## Project: P0 风控统一改进

**Goal**: 修复 4 个阻塞实盘的 P0 问题,所有改动可独立验收、可灰度上线、可紧急回滚
**Timeline**: 4 个 Phase,串行推进 (部分 Phase 内任务可并行)
**Constraints**: 不破坏现有 321 个测试;不修改 monitor/risk.rs 公开 API;所有新增配置支持 SIGHUP 热加载

---

## Milestones

| # | Milestone | Phase | Success Criteria |
|---|-----------|-------|-----------------|
| M1 | VetoChain 集成上线 (dry_run) | Phase 1 | 3 条规则在 dry_run 模式下运行,日志可见拦截结果,零误拦 |
| M2 | 风控执行统一 | Phase 2 | position_tracker 使用 StopLoss+PositionSizer,旧四大铁律兼容 |
| M3 | AI 评分诊断报告 | Phase 3 | `--review` 输出 sentiment_score 及各因子 IC/IR 报告 |
| M4 | 核心路径测试覆盖 | Phase 4 | position_tracker ≥95% 行覆盖, VetoChain ≥90% 行覆盖 |

---

## Phase 1: VetoChain 风控拦截恢复 (P0-1)

> 目标: 3 条被注释的拦截规则以策略模式恢复,默认 dry_run 模式运行

### Task 1.1: 创建 VetoChain 核心框架
| 属性 | 内容 |
|------|------|
| 文件 | `src/risk/veto_chain.rs` (新增) |
| 工时 | 4h |
| 依赖 | 无 |
| 数据红线 | 2.3 (VetoContext 中价格字段需 >0 校验) |

**Done Criteria**:
- [ ] `VetoRule` trait 定义完成 (name/evaluate/priority)
- [ ] `VetoVerdict` struct (risk_flags/score_penalty/force_hold)
- [ ] `VetoContext` struct (使用基础类型,不依赖 trend_analyzer)
- [ ] `VetoChain` struct (rules Vec + evaluate_all 方法)
- [ ] `VetoChain` 通过 `Arc<RwLock<VetoChainConfig>>` 支持热加载
- [ ] `std::panic::catch_unwind` 包裹每个规则执行
- [ ] 单元测试: 空规则链不 panic, 单规则 panic 不传播

### Task 1.2: 实现 3 条具体规则
| 属性 | 内容 |
|------|------|
| 文件 | `src/risk/veto_rules_live.rs` (新增) |
| 工时 | 4h |
| 依赖 | Task 1.1 |
| 数据红线 | 2.2 (MoneyFlow 缺失不静默填充), 2.3 (PE 范围检查) |

**Done Criteria**:
- [ ] `BiasRateRule`: bias_ma5 > 5% → force_hold=true, score_penalty=-5
- [ ] `BiasRateRule`: is_bearish=true → force_hold=true (空头排列拦截)
- [ ] `MainFlowRule`: main_net < -5000万 → force_hold=true
- [ ] `MainFlowRule`: pct_chg > 4% + main_net < -1000万 (诱多) → force_hold=true
- [ ] `FundamentalDeteriorationRule`: pe<0 或 pe>300 + net_profit_yoy < -30% → force_hold=true
- [ ] 每条规则在数据缺失时返回空 VetoVerdict (不否决) + warn 日志
- [ ] 单元测试: 每条规则的触发/不触发/数据缺失三种场景

### Task 1.3: 添加配置支持
| 属性 | 内容 |
|------|------|
| 文件 | `src/config.rs` (修改), `config/monitor.toml` (修改) |
| 工时 | 2h |
| 依赖 | Task 1.1 |
| 数据红线 | 无直接红线,配置属于基础设施 |

**Done Criteria**:
- [ ] `VetoConfig` struct 含 enabled/mode/bias_rate_enabled/bearish_enabled/main_flow_enabled/fundamental_enabled
- [ ] `config/monitor.toml` 新增 `[live_veto]` section
- [ ] mode 支持 "dry_run" / "live" 双模式
- [ ] SIGHUP 热加载: 修改 toml → 发送 SIGHUP → VetoChain 配置更新
- [ ] 配置缺失时使用代码内 Default (全部开启 + dry_run)

### Task 1.4: 调整 pipeline 数据获取时序 + 集成 VetoChain
| 属性 | 内容 |
|------|------|
| 文件 | `src/pipeline/mod.rs` (修改) |
| 工时 | 5h |
| 依赖 | Task 1.2, 1.3 |
| 数据红线 | 2.1 (MoneyFlow 获取失败显式报错), 2.4 (数据新鲜度) |

**Done Criteria**:
- [ ] 调整 `process_stock_inner` 流程顺序: trend_analyzer → boll_macd → **并行获取 MoneyFlow/Financials/News (提前)** → fundamental adjustment → VetoChain → AI analysis
- [ ] VetoChain 替换原注释块 (line 686-740)
- [ ] dry_run 模式: 规则评估 → 日志记录拦截详情 → **不修改** signal_score/buy_signal
- [ ] live 模式: 规则评估 → 触发时 signal_score=55, buy_signal=Hold
- [ ] 拦截事件记录到 result.veto_flags
- [ ] MoneyFlow 获取失败不影响 VetoChain 继续执行 (MainFlowRule 返回空)
- [ ] 日志格式: `[{code}] VetoChain[{mode}] {rule_name}: {verdict}`

### Task 1.5: VetoChain 集成测试 + 规则效力回测
| 属性 | 内容 |
|------|------|
| 文件 | `tests/risk/veto_chain_tests.rs` (新增), `src/review/` (修改) |
| 工时 | 3h |
| 依赖 | Task 1.4 |
| 数据红线 | 2.5 (测试用 TEST_CODE 前缀) |

**Done Criteria**:
- [ ] VetoChain 集成测试: 多条规则同时触发 → flags 正确汇聚
- [ ] dry_run vs live 行为差异测试
- [ ] `--review` 路径新增规则效力统计: 每条规则的拦截数/拦截交易中亏损比例
- [ ] 测试中的股票代码使用 `TEST_CODE` 前缀

### Phase 1 小计: 18h

---

## Phase 2: 统一风控执行 (P0-2)

> 目标: position_tracker 从硬编码四大铁律迁移到 Risk Context 组件

### Task 2.1: 封装 RiskContext
| 属性 | 内容 |
|------|------|
| 文件 | `src/pipeline/position_tracker.rs` (修改) |
| 工时 | 2h |
| 依赖 | Phase 1 完成 |
| 数据红线 | 2.3 (ATR/价格 >0 校验) |

**Done Criteria**:
- [ ] `RiskContext` struct: StopLoss + PositionSizer + MarketRegime + HardLimits
- [ ] `RiskContext::from_env_and_data()` 构造函数 (从环境变量 + 市场数据构建)
- [ ] `track_position` 签名新增 `risk_ctx: &RiskContext` 参数
- [ ] 向后兼容: 旧 `position_shares()` 函数保留,标记 `#[allow(dead_code)]`

### Task 2.2: 买入路径接入 PositionSizer + MarketRegime
| 属性 | 内容 |
|------|------|
| 文件 | `src/pipeline/position_tracker.rs` (修改) |
| 工时 | 3h |
| 依赖 | Task 2.1 |
| 数据红线 | 2.6 (单笔金额 ≤ 账户可用资金) |

**Done Criteria**:
- [ ] 买入前检查 `MarketRegime::allow_new_position()` — Crash 拒绝买入
- [ ] `PositionSizer::max_position()` 计算动态仓位上限
- [ ] 买入量 = min(PositionSizer结果, HardLimits.single_stock_max_pct × total_capital)
- [ ] 买入量向下取整 100 股整数倍
- [ ] 同股当日已持仓 → 拒绝重复买入 (PositionSizer 已支持)
- [ ] T+1 锁仓检查: 买入后标记 `PositionType::Locked`
- [ ] 配置 `[position_sizing] use_dynamic = false` 可回退到旧逻辑

### Task 2.3: 卖出路径接入 StopLoss + check_stops
| 属性 | 内容 |
|------|------|
| 文件 | `src/pipeline/position_tracker.rs` (修改) |
| 工时 | 3h |
| 依赖 | Task 2.1 |
| 数据红线 | 2.3 (止损价 > 0 校验) |

**Done Criteria**:
- [ ] `StopLoss::triggered(current_price)` 替代铁律1 (8% 固定止损)
- [ ] `check_stops()` 三级止损作为补充检查
- [ ] T+1 锁仓时: 触发止损 → 日志告警 "T+1锁仓无法卖出,建议次日竞价挂单",不执行 DB 平仓
- [ ] 保留铁律2/3/4/5 作为额外保护层
- [ ] 止损触发后 result.veto_flags 追加止损标记
- [ ] 配置回退: 若 ATR 数据缺失,StopLoss 回退到 8% 固定止损

### Task 2.4: 端到端集成验证
| 属性 | 内容 |
|------|------|
| 文件 | `src/pipeline/mod.rs` (修改) |
| 工时 | 2h |
| 依赖 | Task 2.2, 2.3 |
| 数据红线 | 2.1 (全路径无 mock 残留) |

**Done Criteria**:
- [ ] RiskContext 在 process_stock_inner 中构造并传递给 track_position
- [ ] MarketRegime 判定所需"上涨家数占比"从现有 market_regime 模块获取
- [ ] `cargo build` 通过
- [ ] `cargo run --bin monitor -- --test` 全链路烟测通过
- [ ] 手动 review: position_tracker.rs 中无硬编码止损阈值 (8%/-20%/14天) — 除铁律2/3/4/5保留外

### Phase 2 小计: 10h

---

## Phase 3: AI 评分因子分析 (P0-3)

> 目标: 实现 FactorIC 计算,在 `--review` 模式下输出诊断报告

### Task 3.1: 实现 Spearman Rank IC 计算核心
| 属性 | 内容 |
|------|------|
| 文件 | `src/review/factor_ic.rs` (新增) |
| 工时 | 4h |
| 依赖 | 无 (纯计算模块) |
| 数据红线 | 2.2 (缺失因子值跳过该期,不静默填充) |

**Done Criteria**:
- [ ] `FactorIC` struct: factor_name/ic_series/cumulative_ic/information_ratio/ic_win_rate/ic_decay/t_stat
- [ ] `compute_ic(factor_values: &[f64], forward_returns: &[f64]) -> FactorIC`
- [ ] Spearman Rank IC: 先分别排序得秩次,再算秩次 Pearson 相关
- [ ] IC 衰减: lag=1,2,3,4 的 IC 均值
- [ ] t_stat = mean(IC) / (std(IC)/√n)
- [ ] 数据不足 (< 30 期) → 返回 None
- [ ] NaN/Inf 安全处理
- [ ] 单元测试: 正相关序列 → IC>0.5; 负相关 → IC<-0.5; 随机 → IC≈0

### Task 3.2: 从持仓历史提取因子值与前向收益
| 属性 | 内容 |
|------|------|
| 文件 | `src/review/factor_ic.rs` (扩展) |
| 工时 | 3h |
| 依赖 | Task 3.1 |
| 数据红线 | 2.4 (使用当日有效数据), 2.1 (不做数据编造) |

**Done Criteria**:
- [ ] 从 `stock_position` 表读取已平仓交易
- [ ] 从 `analysis_result` 表读取对应日期的 score_breakdown (5 维度)
- [ ] 计算每笔交易的 forward_return (T+1, T+5, T+10)
- [ ] 构建因子值向量 × 前向收益向量的配对数据
- [ ] 对 sentiment_score 单独计算 IC (验证方向性错误)
- [ ] 对 ScoreBreakdown 5 个维度分别计算 IC
- [ ] 输出因子间相关性矩阵
- [ ] 样本量不足 → 跳过并报告

### Task 3.3: 生成 Markdown 诊断报告
| 属性 | 内容 |
|------|------|
| 文件 | `src/review/factor_report.rs` (新增) |
| 工时 | 3h |
| 依赖 | Task 3.2 |
| 数据红线 | 无直接红线 |

**Done Criteria**:
- [ ] `generate_factor_report(analyses: &[FactorIC], corr_matrix: &[Vec<f64>]) -> String`
- [ ] 报告包含: 执行摘要 (哪个因子方向性最差)
- [ ] IC 表格: 因子名 / IC / IR / 胜率 / t-stat / 评估 (🟢🟡🔴)
- [ ] IC 衰减图 (ASCII art)
- [ ] 因子相关性矩阵 (Markdown table)
- [ ] sentiment_score 方向性诊断: IC<0 → "AI评分与未来收益负相关,建议继续使用B方案"
- [ ] 集成到 `--review` 输出

### Phase 3 小计: 10h

---

## Phase 4: position_tracker 测试覆盖 (P0-4)

> 目标: 核心买卖路径 ≥95% 行覆盖

### Task 4.1: 测试基础设施搭建
| 属性 | 内容 |
|------|------|
| 文件 | `tests/pipeline/position_tracker_tests.rs` (新增) |
| 工时 | 2h |
| 依赖 | Phase 2 完成 |
| 数据红线 | 2.5 (TEST_CODE 前缀,测试与实盘硬隔离) |

**Done Criteria**:
- [ ] 测试 helper 函数: `make_kline_data()` / `make_analysis_result()` / `make_risk_context()`
- [ ] 所有测试股票代码使用 `TEST_CODE` 前缀
- [ ] 测试不访问真实 DB — 使用 `DatabaseManager` 的测试模式或 mock
- [ ] 测试不访问网络

### Task 4.2: 买入路径测试
| 属性 | 内容 |
|------|------|
| 文件 | `tests/pipeline/position_tracker_tests.rs` |
| 工时 | 3h |
| 依赖 | Task 4.1 |
| 数据红线 | 2.5 |

**Done Criteria**:
- [ ] `test_buy_bottom_buy_triggered` — BollMacdAction::BottomBuy → 买入
- [ ] `test_buy_uptrend_start_triggered` — BollMacdAction::UptrendStart → 买入
- [ ] `test_buy_contrarian_triggered` — contrarian_signal=true → 买入
- [ ] `test_buy_no_signal_no_buy` — 无共振信号 → 不买入
- [ ] `test_buy_crash_regime_blocked` — MarketRegime::Crash → 拒绝买入
- [ ] `test_buy_already_held_blocked` — 同股已持仓 → 拒绝重复买入
- [ ] `test_buy_position_sizer_respected` — 仓位计算正确
- [ ] `test_buy_lot_rounded_to_100` — 100 股整数倍

### Task 4.3: 卖出路径测试
| 属性 | 内容 |
|------|------|
| 文件 | `tests/pipeline/position_tracker_tests.rs` |
| 工时 | 3h |
| 依赖 | Task 4.1 |
| 数据红线 | 2.5 |

**Done Criteria**:
- [ ] `test_sell_stop_loss_triggered` — StopLoss.triggered → 卖出
- [ ] `test_sell_tiered_stop_check` — check_stops 三级止损触发
- [ ] `test_sell_profit_trend_exit` — 盈利≥20% + 跌破5日线 → 卖出
- [ ] `test_sell_timeout_loss` — >14天仍亏损 → 卖出
- [ ] `test_sell_boll_top_sell` — 布林上轨+MACD顶背离 → 卖出
- [ ] `test_sell_profit_below_20_no_sell` — 盈利<20% 不主动止盈
- [ ] `test_sell_t1_locked_prevents_sell` — T+1 锁仓阻止卖出
- [ ] `test_sell_multiple_reasons` — 多原因同时触发 → 输出正确的首要原因

### Task 4.4: 边界与异常测试
| 属性 | 内容 |
|------|------|
| 文件 | `tests/pipeline/position_tracker_tests.rs` |
| 工时 | 2h |
| 依赖 | Task 4.2, 4.3 |
| 数据红线 | 2.5 |

**Done Criteria**:
- [ ] `test_zero_price_no_panic` — current_price=0 不 panic
- [ ] `test_empty_data_no_panic` — data 为空不 panic
- [ ] `test_db_unavailable_graceful` — DB 不可用 → warn 日志,不 panic
- [ ] `test_atr_nan_fallback` — ATR=NaN → 回退到默认值
- [ ] `test_position_sizer_zero_capital` — 总本金=0 → 买入量=0,不 panic
- [ ] `test_negative_return` — 收益率为负但未触发止损 → 正确判断

### Task 4.5: 覆盖率验证
| 属性 | 内容 |
|------|------|
| 文件 | 无代码变更 |
| 工时 | 1h |
| 依赖 | Task 4.2-4.4 |

**Done Criteria**:
- [ ] `cargo test --lib pipeline::position_tracker` 全部通过
- [ ] `cargo tarpaulin --lib --line` position_tracker.rs ≥95%
- [ ] VetoChain ≥90% 行覆盖
- [ ] 所有 321 个已有测试仍然通过
- [ ] `cargo clippy` 无新增 warning

### Phase 4 小计: 11h

---

## Dependencies Map

```
Phase 1 (VetoChain) ──────────────────────┐
  T1.1 ──> T1.2 ──> T1.4 ──> T1.5        │
            T1.3 ──┘                       │
                                           ├──> Phase 4 (Tests)
Phase 2 (Risk Unification) ───────────────┘
  T2.1 ──> T2.2 ──> T2.4                 
         ──> T2.3 ──┘                     

Phase 3 (FactorIC) ─── 独立并行 ──────────
  T3.1 ──> T3.2 ──> T3.3
```

**Critical Path**: Phase 1 → Phase 2 → Phase 4
**Parallelizable**: Phase 3 可与 Phase 1+2 并行开发

---

## 总工时估算

| Phase | 乐观 | 最可能 | 悲观 | 期望 (PERT) |
|-------|------|--------|------|-------------|
| Phase 1: VetoChain | 14h | 18h | 24h | 18.3h |
| Phase 2: 统一风控 | 8h | 10h | 14h | 10.3h |
| Phase 3: FactorIC | 8h | 10h | 14h | 10.3h |
| Phase 4: 测试 | 9h | 11h | 15h | 11.3h |
| **总计** | **39h** | **49h** | **67h** | **50.3h** |

约 **6-8 个工作日** (单人开发,含 buffer)。

---

## 数据红线覆盖矩阵

| 红线条款 | Phase 1 | Phase 2 | Phase 3 | Phase 4 |
|---------|---------|---------|---------|---------|
| 2.1 生产禁 mock | T1.4 (MoneyFlow 真数据) | T2.4 (全路径无 mock) | T3.2 (DB 真数据) | T4.1 (TEST_CODE) |
| 2.2 缺失不静默填 | T1.2 (规则数据缺失) | — | T3.1 (因子缺失跳过) | — |
| 2.3 坏数据校验 | T1.1 (价格>0) | T2.1 (ATR>0), 2.3 (止损价>0) | — | — |
| 2.4 数据时效 | T1.4 (MoneyFlow 新鲜度) | — | T3.2 (当日有效) | — |
| 2.5 测试实盘隔离 | T1.5 | — | — | T4.1-T4.4 (TEST_CODE) |
| 2.6 写入防护 | — | T2.2 (单笔金额校验) | — | — |
| 2.7 审计留痕 | T1.4 (拦截日志) | T2.3 (止损标记) | T3.2 (DB 来源) | — |

---

## Risk & Mitigation

| Risk | Impact | Prob | Mitigation |
|------|--------|------|------------|
| VetoChain 参数过紧导致大量误拦 | High | Medium | 首周 dry_run,观察拦截率;若 >30% 则调整阈值 |
| 数据获取时序调整引入 bug | High | Low | 充分测试 process_stock_inner;保留旧代码路径注释 |
| FactorIC 样本量不足无法可靠分析 | Low | High | 设定最小样本 30 笔,不足时跳过并提示 |
| DB mock 困难导致 position_tracker 测试写不了 | Medium | Medium | 若无法 mock DB,改用集成测试 + 测试数据库 |
| SIGHUP 热加载并发问题 | Low | Low | Arc<RwLock> 原子切换,现有 config 已有此模式 |

---

*下一步: Step 4 编码 (AGENTS.md 流程第 4 步)*
