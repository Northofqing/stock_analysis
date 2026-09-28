# ADR-0002：投资事实时间与决策身份

**状态：** Accepted for development（生产行动入口尚未切换）

**日期：** 2026-09-29

**范围：** M4 的数据准入、投资决策、研究回放与独立 PaperLedger

## 背景

`data_gateway::BatchEvidence` 已保留 provider、source、`source_at`、`observed_at` 和 batch ID；`qualified_trading_facts` 能逐字段拒绝缺失、覆盖不足、过期、冲突和身份错配，但生产来源合同尚未交付。`monitor::data_mode` 的 Full/Degraded/Unsafe 是运行健康聚合，不能证明某个投资行动所需的每项事实合格。`decision` 中的现有卡片和计划也没有覆盖整个候选集合的不可变投资决策身份。`durable_delivery` 的 decision identity 属于通知投递，`PaperLedgerV1` 的 seed、费用模型和事件链已有独立身份。

因此，同一天的展示值、拉取时间或推送回执都不足以证明研究回放在当时可见、候选可交易或模拟成交已发生。

## 决定

1. **时间分开保存。** 新的投资事实合同显式记录市场生效时间/日期 `effective_on`、来源发布或产生时间 `source_at`、本系统取得时间 `observed_at`、被准入的批次版本与决策截止时间 `as_of`。历史研究还须证明该**版本**在 `as_of` 前可取得；只知道当前 `observed_at` 的追溯数据不得自动用于过去的决策。时间精度和时区由版本化来源合同定义，比较前转为同一 UTC 时刻及经交易日历验证的上海业务日。不能用抓取时间代替来源时间，也不能用“日期相同”跳过盘中截止点。
2. **每项必需事实给出五态裁定。** 一个行动批次的 `DataHealthSnapshot` 按需求字段和来源版本记录 `Admitted`、`Missing`、`Stale`、`Conflict`、`Unqualified` 之一及稳定 reason code。`Admitted` 包括由来源合同证明完整覆盖的真实零值或空事件；`Missing` 表示缺少应有事实；`Stale` 表示事实有效但不满足截止或新鲜度；`Conflict` 表示时间、值或身份矛盾；`Unqualified` 表示来源合同、身份锚、完整窗口或品种资格不足。未交付的 D14/D17/D20 合同属于 `Unqualified`。冲突与不合格同时出现时仍保存全部原因，行动资格一律为否。此五态是**行动事实**裁定，不改写既有三态运行健康。
3. **完整批次准入。** 一个策略评估先冻结 universe、数据集版本、所需字段、截止时间、日历版本及每个候选的 disposition。任何必需事实非 `Admitted` 时，受影响候选或整个依赖该事实的批次明确拒绝；不把缺失值填零、把未知空窗当 verified-empty、把未标注兼容数据当候选或把迟到修订写回旧快照。研究探索可读隔离的未准入数据，但输出必须标为 exploratory，不能进入模拟订单或晋级证据。
4. **投资与推送身份分开。** `InvestmentDecisionId` 是新类型和命名空间，由版本化规范编码的策略/模型/配置版本、评估键、冻结 universe、已准入事实快照、业务日和截止时间导出；同一输入重放得到同一 ID，不同事实版本生成新 ID。不可变决策记录保存每个候选的通过/拒绝原因、成本和流动性假设、风险 veto 与人工裁定引用。通知使用独立的 `PushDecisionId`，只允许持有对投资决策的类型化引用。发送成功、人工确认和 paper fill 各有自己的终态，不能反向改变投资裁定。
5. **单一行动门。** `decision` 消费同一份冻结 `DataHealthSnapshot` 和完整候选 disposition；通过后才构造带 `InvestmentDecisionId` 的 paper order intent。`trading::paper_ledger` 仍是模拟成交和持仓的唯一有效 owner，保留其显式 seed/cutover、事件重放与费用 generation 约束。对账只通过不可变引用连接决策、parent order、fill、费用、持仓与结果，不凭文本、股票代码和日期猜关联。

## 取舍与后果

- **采用逐行动、逐字段快照。** 可以准确说明拒绝原因，并复用现有 Gateway 的 `BatchEvidence` 与字段级拒绝。代价是需要保存版本、完整候选集合及快照引用，旧卡片和历史回测不能自动获得投资决策资格。
- **不复用运行健康三态或推送 decision ID。** 它们各自回答系统可用性与投递的问题，粒度和终态不同。代价是新增类型及关联表，而不是复用一个字符串列。
- **迟到修订生成新研究 run。** 原决策及当时可见版本保持可回放；报告要同时标出原始口径与重算口径，不能静默覆盖。

## 实施与验收

1. 在一个无券商策略 vertical slice 上实现版本化事实 envelope、五态行动快照和完整候选 disposition；来源 adapter 需证明身份、时间、覆盖与 verified-empty。D14/D17/D20 在其权威证据到齐前继续 fail closed。
2. 固定 `InvestmentDecisionId` 的规范编码和 golden，测试同输入重放稳定、事实版本/截止时间变化换 ID、缺失/迟到/冲突/伪空窗拒绝，以及所有候选均有 disposition。
3. 把一个投资决策连接到唯一的 paper order/fill owner；验证重复、逆序、no-fill、部分成交、费用版本和重放对账。`PaperLedgerV1` 字节及费用历史由 [ADR-0001](0001-versioned-a-share-fee-schedule.md) 保持冻结。
4. 生产切换另需来源合同、实际数据集/健康快照、seed/cutover、逐日对账及 Gate P 外部保留与恢复回执；本 ADR 只确定开发边界，不表示这些验收已完成。

相关代码：`src/data_gateway/review.rs`、`src/data_gateway/qualified_trading_facts.rs`、`src/monitor/data_mode.rs`、`src/decision/`、`src/durable_delivery/model.rs`、`src/trading/paper_ledger.rs`；[平台路线图](../superpowers/plans/2026-09-28-platform-complete-roadmap.md)。
