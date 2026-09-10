# 持仓计划同快照、同行情批次来源接线

日期：2026-09-10。源码 `112ff8f9d15834a4a29de5439909f01413d08ad4`，基线 `ca902d8112db187faa56b2f87138f22e6d12e808`。本切片实现及离线验证已完成，独立 Spec compliant / Quality Approved，0 Critical、0 Important、1 Minor；不是整个 Unit 或 Foundation 迁移完成，没有部署或生产推送验证。[实施计划](../superpowers/plans/2026-09-10-holding-plan-frozen-source.md)保留完整范围与后续门禁。

## 实际修复与证据

[真实准备入口](../../src/bin/monitor/main.rs#L8392)现在直接传入用户快照读取、保留证据的行情采集及本地时钟三个实际依赖。[手动入口](../../src/bin/monitor/manual_push.rs#L159)与[周期入口](../../src/bin/monitor/main.rs#L10678)仍复用它；没有增加另一条仅供测试使用的准备路径。

| 改动 | 当前代码与可观察证据 |
| --- | --- |
| 一轮固定一份持仓，不再为取行情重读最新持仓 | [准备操作](../../src/bin/monitor/holding_plan.rs#L7)读取一次快照并从其 items 构造 requested_codes；直接调用 [fetch_realtime_quote_batch](../../src/bin/monitor/market_data.rs#L227)，不再经过会重读/回退的 fetch_position_quotes。[A/B 回归](../../src/bin/monitor/holding_plan.rs#L579)让外部最新快照变成 B，实际请求、正文和 binding 仍属于选中的 A。 |
| 来源随真实 counted binding 保存 | [canonical 构造](../../src/bin/monitor/holding_plan.rs#L82)保存快照行号/ID/确认与生效时间/来源/证据摘要、该票名称、请求代码及行情 provider/source/source_at/observed_at/batch_id；[返回绑定](../../src/bin/monitor/holding_plan.rs#L119)使用这些字节。已有 [envelope 构造](../../src/bin/monitor/durable_delivery_runtime.rs#L2199)原样携带 canonical，不依赖日志拼接。 |
| 区分本地准备时间与提供方时间 | [单次时钟捕获](../../src/bin/monitor/holding_plan.rs#L13)同时供业务日、正文时分和本地 observed_at 使用；[两票回归](../../src/bin/monitor/holding_plan.rs#L469)验证一致性。行情证据中的时间字符串保留原值，[缺 source_at 回归](../../src/bin/monitor/holding_plan.rs#L508)验证字段存在且为 null，不用本地时间补造。 |
| 保持原业务与失败行为 | [原计算和渲染参数](../../src/bin/monitor/holding_plan.rs#L37)保持 >5 / <-3 / 其余三分支及成本比例区间；[固定正文样例](../../src/bin/monitor/holding_plan.rs#L396)验证 Reduce/Add/Hold、数量及价格。[缺失/错误/空持仓](../../src/bin/monitor/holding_plan.rs#L281)、[缺行情](../../src/bin/monitor/holding_plan.rs#L344)与[非正成本](../../src/bin/monitor/holding_plan.rs#L366)保留原处理。 |

来源版本为 `HOLDING_PLAN_SOURCE_BINDING_V1`。同一正文但快照或行情批次身份不同，会产生不同的 source bytes/fingerprint，[专门反例](../../src/bin/monitor/holding_plan.rs#L532)对此有断言；这只证明来源可区分，不认证外部来源，不自动获得再次发送资格。

## 验证结果及范围

原 canonical 提取到实际使用的准备操作后，先运行来源测试：编译成功，断言实际返回的 schema_version 为 Null 而非要求值，退出 101；加入来源绑定后，同一测试退出 0。这个 RED 证明来源未保留，不冒称执行过旧生产并发竞态。A/B 用例是离线受控变化。

最终源码运行以下定向检查，27 项全部通过：

| 检查 | 结果 | 能证明什么 |
| --- | --- | --- |
| `holding_plan::tests::` | 12 通过 | 实际准备操作、真实 renderer/binding、来源/错误/时钟/选中快照一致性 |
| `manual_push::tests::` | 10 通过 | 内存效果 adapter 下原手动批次编排不回退；不是生产 dispatcher 执行 |
| BR-159 `br159_top_stock_projection_retains_evidence_and_rejects_bad_market_data_without_network` | 1 通过 | 已有 Gateway 批次投影的证据保留与非法行情拒绝 |
| `br210_projection_` | 3 通过 | 整数/小数观察时间编码及非法编码拒绝 |
| `br192_internal_binding_uses_exact_canonical_source_hash` | 1 通过 | 已有 HoldingPlan envelope 携带精确 canonical/hash；不写业务库或发送 |

上述过滤器均使用 `cargo test --offline --bin monitor <filter>`；BR-159 的实际调用另带 `-- --exact market_data::quote_batch_tests::br159_top_stock_projection_retains_evidence_and_rejects_bad_market_data_without_network`。原始命令、输出及运行前后摘要留在本计划本地工作目录 `.superpowers/sdd/2026-09-10-holding-plan-frozen-source/`，没有纳入 Git。

两份改动文件的 `rustfmt --edition 2021 --config skip_children=true --check` 与 `git diff --check` 均通过。`cargo clippy --offline --bin monitor --message-format=json` 退出 0：190 条告警、0 错误，与本轮精确基线按 target/code/message/主文件比较完全一致，新增/消失均为 0。测试构建保留 113 条既有 lib 告警，不称无告警。

独立审查的唯一 Minor 是上述既有告警噪音，另行跟踪，不扩大本次源码范围。审查不能从两文件 diff 核验的两项已由主 agent 补核：实际 manual/periodic 调用仍共用准备入口；计划、实施记录及索引已更新。真实上线门禁和生产窗口明确属于完整目标后续，未被此补核认证为通过。完整分支审查仍留在最终集成前，不重复对本局部切片宣称整分支批准。

过程边界：首次 Clippy 基线误带 `--tests`，额外编译了其他测试 target；仅编译，未执行那些测试，随后取得并使用 monitor-only 基线。上轮已识别的六文件全仓格式差异本批未改，也未重跑全仓 fmt；只声明改动文件格式通过。最终每次捕获的两份 Rust SHA-256 前后相同：

- main.rs：`c87fa4bb56df1c786e11516f754316e0a3fb9d9f5f75aad8aa2a7cb02016488c`
- holding_plan.rs：`cdce768e2d00f19d0c16909b2e158083e6f0434c6c09634b976163b71da569a9`

文档收尾校验覆盖本轮 5 份文件：134 个本地链接、49 个数字行号范围及上述两份源码摘要均通过；跳过 1 个其他计划的私有链接，标题锚点未纳入自动校验，不把此结果称为全仓文档门禁通过。

## 仍未完成，不能由本切片推断

- [周期日表过滤](../../src/bin/monitor/main.rs#L10681)与[发送后记日表](../../src/bin/monitor/main.rs#L10736)仍未和 durable 原子统一，manual/startup 也不是同一日表 owner；读日表失败返回空、写失败只记日志的旧缺口仍在。没有修改 Rolling 1800 秒、日预算、重发权限或定时器。
- [既有修订与恢复裁决](holding-plan-call-chain-2026-09-10.md#既有频次与恢复裁决补核)仍有效：真实修订不能被日级展示名吞掉，但仅时间/来源 hash 改变不是获准重发；有效修订及再次发送资格仍需完整合同。启动恢复不读取新来源替换旧 immutable bytes/decision。
- 这不是新鲜度认证。没有改变持仓新鲜度政策、补造行情 source_at、增加来源权限或默认健康状态；没有冻结 [BannerCtx 外部说明读取](../../src/bin/monitor/push_templates.rs#L267)。测试用完整账户 fixture 避免真实 DB 读取，不能证明生产完整账户/行情快照已认证。
- 新 canonical 会改变新 decision 的 source/subject hash；本次不重写旧记录、不部署。后续上线必须验证新旧身份共存、恢复及切换门禁，不能靠回退源码假定新旧 hash 相同。
- [全量剩余证据](remaining-migration-evidence-2026-09-08.md)仍为 10 个 Unit 已追到旧调用链、42 个尚未逐链追完，不是迁移百分比；W15/W16 认证与共同 fence、真实业务适配、52 Unit 六门禁及生产窗口仍待。

本轮执行裁决：按已确认来源缺口修复精确采集和证据保留，不擅加业务频次/新鲜度规则；若后续来源合同不同，需要版本化兼容与重新取证。验证限定已审计离线组，未覆盖的生产性质不作通过声明。`tdd` 促成了实际失败反例，独立实现/验证分工保持单 Rust 写入者、单 Cargo 队列。
