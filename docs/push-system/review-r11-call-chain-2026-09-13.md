# MU-review-r11：持仓复盘四入口与恢复缺口

日期：2026-09-13。静态源码核查，不是生产复现或迁移验收。主控已全文核独立报告、复核关键源与12项输入摘要；当前累计17个Unit已追链、35个尚未逐链，不等于17个已迁移。单用户范围与全部52个Unit目标不变。

## 四入口与共同完成键

目录中的四个producer归属同一业务日的 `PositionReview/None/GLOBAL`、`BusinessDateOnce`，任务身份为 `review_task_identity(date,R11)`；四入口不是四个完成owner。依据：[目录](push-capability-catalog.v1.json)、[通知政策](../../src/durable_delivery/model.rs#L447)。

| 入口 | 实际调用与差异 | 源码 |
| --- | --- | --- |
| 自动 | 交易日19:00起、60秒轮询；先从持久结果恢复调度状态，再算due。实际attempt用at_manual，R11无额外发布时间门 | [时间门](../../src/bin/monitor/main.rs#L5656)、[attempt](../../src/bin/monitor/main.rs#L5717)、[调度提交](../../src/bin/monitor/main.rs#L6244) |
| 手动 --review | 13项一起尝试；按当前时刻推导最近已完成交易日、开启override。Complete与Partial均可使CLI成功，不能据此证明R11送达 | [手动入口](../../src/bin/monitor/main.rs#L5527)、[退出分类](../../src/bin/monitor/main.rs#L5574) |
| 历史补推 | 最近5个已验证历史交易日，不含今天、旧到新；8项集合含R11。先补收盘估值，失败仍继续任务。已有Delivered直接跳过，无claim才按原日重新调用dispatcher | [日期及任务集](../../src/bin/monitor/main.rs#L5735)、[估值前置](../../src/bin/monitor/main.rs#L5815)、[单项恢复](../../src/bin/monitor/main.rs#L5930) |
| 启动恢复 | 核心库绑定后、producer启动前扫描全部日期的pending decision；恢复原信封，不重读账户/估值或重跑AI。不是新完成键；P01独占补偿跳过此全局门 | [启动入口](../../src/bin/monitor/main.rs#L4810)、[恢复运行时](../../src/bin/monitor/durable_delivery_runtime.rs#L1166) |

单项历史补推仍进入完整dispatcher，可能附带prediction回填、block-trade和IPO侧路；不能把singleton due当作没有其他副作用。[dispatcher](../../src/bin/monitor/push_templates.rs#L9891)

## 新消息的业务逻辑

1. 先解析业务日，查询该日R11既有decision。若存在则直接映射结果，尚未读账户、估值、持仓或调用AI；查询错误按可重试失败处理。[preflight](../../src/bin/monitor/push_templates.rs#L9150)
2. 依次读最新用户确认账户摘要、指定业务日的最新持久收盘估值、当前持仓。摘要不是按业务日查询；估值必须精确日期，不能拿前一日代替。摘要/估值缺失为NoData，读库或任务错误为可重试失败。[账户查询](../../src/database/user_account_summary.rs#L83)、[估值查询](../../src/database/closing_valuation.rs#L120)、[调用顺序](../../src/bin/monitor/push_templates.rs#L9185)
3. 估值覆盖须完整，市值/收盘价/盈亏及相关可选指标须通过原有限数值检查；失败为NoData。个股按市值降序，不截断；行业按当前Holding聚合、市值占比降序，前5之外并为“其他”。[估值准入](../../src/bin/monitor/push_templates.rs#L8860)、[行业投影](../../src/bin/monitor/push_templates.rs#L9247)
4. 模板包含账户汇总、全部个股、行业分布。可选AI取市值Top-N（默认3），每只90秒；缺key、失败、超时均跳过，不阻断事实消息。成功AI先落详细报告，推送正文附报告路径。[正文](../../src/bin/monitor/push_templates.rs#L9290)、[AI规则](../../src/bin/monitor/push_templates.rs#L8995)
5. source canonical实际只保存date及每项code/quantity/cost_price/close/unrealized_pnl；task binding的snapshot_size为items.len()，subject为日期加position-review。进入counted通知，依原durable decision恢复，而非本次新Foundation已接管。[来源绑定](../../src/bin/monitor/push_templates.rs#L9320)、[任务绑定](../../src/bin/monitor/push_templates.rs#L9718)、[通知入口](../../src/bin/monitor/notify.rs#L2889)

## 仍未修复的具体缺口

- 空持仓允许首次投递，snapshot_size因此可以为0。但dispatcher再次复用Delivered时，零snapshot只豁免R08，R11会被映射为永久Failed。此问题限定于该preflight映射路径，不能概括成所有启动/补推都失败：补推已有Delivered直接返回AlreadyDelivered。[允许空持仓](../../src/bin/monitor/push_templates.rs#L9137)、[零快照拒绝](../../src/bin/monitor/push_templates.rs#L6936)、[补推直接跳过](../../src/bin/monitor/main.rs#L5973)
- source canonical未覆盖实际渲染的账户摘要及时间、当前持仓/行业、估值批次元数据/总额和AI结果；历史无claim补推仍读取现在最新摘要与持仓。原信封可以固定已经生成的正文，但不能由上述局部来源绑定证明完整原日同批事实。[局部来源字段](../../src/bin/monitor/push_templates.rs#L9320)、[实际渲染输入](../../src/bin/monitor/push_templates.rs#L9290)
- AI在通知前写 `reports/details/{当前日}_{code}.md`，使用Local::now而非复盘业务日，且直接覆盖同名文件；失败重试/历史补推可能重复生成或覆盖。R11没有通知后的独立业务保存receipt，不能把通知decision当成这些前置效果的恢复日志。[AI调用](../../src/bin/monitor/push_templates.rs#L9070)、[文件写入](../../src/deep_analyzer.rs#L576)
- 调度Terminal包含NoData、Disabled、永久Failed，持久恢复也包含Rejected/Uncertain等分类；Terminal不等于Delivered。[调度分类](../../src/bin/monitor/review_batch.rs#L1236)、[持久结果恢复](../../src/bin/monitor/review_batch.rs#L1433)

这些是后续真实接线、固定来源、前置效果恢复及故障测试的输入。本次没有修改业务策略、操作生产库、运行provider/AI/sink/monitor或增加完整迁移认证数。

## 核对时源码身份

```text
src/bin/monitor/main.rs c87fa4bb56df1c786e11516f754316e0a3fb9d9f5f75aad8aa2a7cb02016488c
src/bin/monitor/review_batch.rs 8006c7bc6143bd410087b832c81636bc81bb1c047709fd0b0eb100ee77a56b2c
src/bin/monitor/push_templates.rs 5096e5697f67338b45346d11efa8f9ec988774c17a8f9623b280adeaeb2044f4
src/bin/monitor/notify.rs ce2112226a4c71cb12426edd151444e239c4844ccf50d49252a67be3af7c7bfa
src/bin/monitor/durable_delivery_runtime.rs 53df56b8078453e9ab141766efed8ff72709ac821298adfd4f05bdeb4c92ec78
src/durable_delivery/model.rs b14f970f6c3d28123d97d287564aa870eb07bead06c2dcf44ac2be536c86d0b9
src/database/user_account_summary.rs 16974623804f08c742828439f4dfd956b94f2213a150eb75d0a39ad2d4733207
src/database/closing_valuation.rs f06c7e00d67a544a1102330e2d9e3e031ddd59ddf4a6951367f650faad60f0a6
src/portfolio/mod.rs a89124894399b5cd1562d47a863eee759b3b6ed3f3aae9e84ea5f83af91cb025
src/deep_analyzer.rs 33945c38102aea5fe09bfdc4bca36aca40c5fd6589f212419cc721feceb11c15
```

独立报告初稿误将同隔离树 `src/main.rs` 的摘要标为monitor入口；主控摘要复核发现后，作者重新核对精确路径和四入口，正文结论保持。上述列出的是更正后的实际源码身份，不混用两个main文件。
