# ADR-0001：按成交日与账本版本管理 A 股费用

**状态：** Accepted for development（未切换生产）

**日期：** 2026-09-28

**范围：** 回测、研究归因、模拟盘订单与不可变账本

## 背景

当前 `strategy::lot::STAMP_TAX_RATE`、`BacktestConfig` 默认值、RSI/Bollinger 配置、`performance::fee_evidence` 和 `PaperLedgerV1` 均按卖出金额的千一计算印花税。财政部、税务总局公告 2023 年 8 月 28 日起证券交易印花税减半；上交所当前股票投资说明为卖出方单边 0.5‰。因此，2026 年按当前配置产生的新研究净收益与模拟卖出费用会高估印花税。100 股、每股 10 元、平价往返，在既有双边最低佣金各 5 元的模型下，旧口径成本 11 元，现行税率对应 10.50 元。这仍是**模型成本**，不能把固定佣金假设写成实际券商收费。

同时，`PaperLedgerV1` 的账户 manifest、数据库 `CHECK(fee_model='lot-rates-v1')`、事件链、有效成交投影和派生证据均冻结了 `lot-rates-v1`。直接修改这个常量或同名函数的计算，会在保留旧身份的同时改变回放结果，破坏既有账本与报告的可追溯性。尚未激活的生产 PaperLedger 也不能由一次源码修改隐式 seed。

## 决定

1. 将 `lot-rates-v1` 固定为旧模拟假设：佣金模型万三、单边最低 5 元，卖出印花税 1‰。历史记录与已生成回执继续按其入账时模型读取，不能就地改写、重算或用当前费率重新解释。
2. 为新研究与新的模拟账本 generation 定义显式费用口径版本，至少包含适用市场、成交日期、印花税档期、佣金假设、金额取整规则及来源版本。此版 A 股股票模型仅覆盖 2008-09-19 起：至 2023-08-27 卖出印花税 1‰，从 2023-08-28 起为 0.5‰；买入印花税为零。更早日期返回 typed unavailable，待另立历史费率合同。日期依据**成交日**，不能依据查询日或报告生成日。佣金仍标为可配置的模拟假设。
3. 不以修改 `PaperLedgerV1` DDL、常量和历史事件的方式实施 v2。新 generation 使用独立且不可变的 manifest/schema 标识，显式 cutover 与期初持仓/现金对账；单个账户/策略在任一时刻只有一个有效成交及持仓 owner。旧 v1 提供只读历史和可复核结果。
4. 回测、模拟成交与归因共享同一费用计算入口和 fixture；每笔 fill 输出实际采用的版本、成交日和金额。研究结果必须区分 `legacy v1` 与 `policy-by-date v2`，跨口径比较需重新计算同一批原始 fill 并标记为新研究 run，不能覆盖旧结果。
5. 上线前核对交易品种边界、佣金/过户费假设、公司行动和入账精度；不适用本 ADR 的交易按 typed unavailable 拒绝，不能静默套用 A 股股票税率。

## 取舍

- **直接把全局 0.001 改为 0.0005：拒绝。** 改动小，但同名 v1 账本会得出不同 replay/净值。
- **保留旧费率直到全平台重写：拒绝。** 会继续生成明知偏高的 2026 年模拟税额。
- **版本化并显式切换：采用。** 需要新增 schema generation、历史区分与 cutover 工作，但能保持旧证据不可变，新结果采用现行口径。

## 实施与验收

1. 冻结 v1 golden fixture：小额/大额、买/卖、FIFO 分摊、事件 replay、旧报告复现；任何修改后这些字节与金额不变。
2. 实现纯函数 v2 费用模型和成交日边界测试（2008-09-18/19、2023-08-27/28），与公共费用证据共享。把模型身份传到所有新回测/归因结果；禁止调用方用隐式默认值混合 v1/v2。
3. 为模拟账本新 generation 建表/manifest/事件与有效投影，增量迁移仅在独立测试数据库执行；验证 `old read + new write`、失败回滚、重复命令、净现金、FIFO、重放与单 owner。
4. 在生产切换前取得账户显式 seed/cutover 授权，核对原始持仓、现金和旧 head；新旧同日影子对账。真实 activation 和运行日观察按平台路线图 M4/M5 的独立门禁执行。

相关入口：`src/strategy/lot.rs`、`src/performance/fee_evidence.rs`、`src/trading/paper_ledger.rs`、`src/trading/paper_ledger_execution.rs`、`src/database/paper_ledger_schema_v1.rs`、[完整路线图](../superpowers/plans/2026-09-28-platform-complete-roadmap.md)。

来源：[财政部、税务总局 2008 年单边征收公告](https://www.mof.gov.cn/zhengwuxinxi/caizhengxinwen/200809/t20080919_76432.htm)；[2023 年第 39 号减半公告](https://m.mof.gov.cn/czxw/202308/t20230827_3904226.htm)；[上交所股票投资费用说明](https://one.sse.com.cn/onething/gptz/)。
