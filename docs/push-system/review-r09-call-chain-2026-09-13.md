# MU-review-r09：四入口只读调用链

日期：2026-09-13。范围：静态核对`review-r09-auto/manual/backfill`与`startup-resume-review-provider-top-n`。主控全文核报告、复核14项输入摘要和关键源码路径；未运行本单元的provider、网络、数据库或monitor。补齐本单元后累计16个Unit已定位旧入口及主要完成门、36个尚未逐链完成，不等于已迁移16个。

## 目录声明与真实入口

- catalog登记四入口共用`review_task_identity(date,R09)`和原业务日，完成owner为`business_date_once_claims(business_date,ReviewProviderTopN,None,GLOBAL)`关联的immutable decision；目录只是定位索引。[catalog](push-capability-catalog.v1.json):5367、5447、5526、8244、9243。
- R09是`SourceOnly`，与R04/R08处于首个封闭并发组，先于后段账户缺失门，因此不依赖账户指标。[review_batch.rs](../../src/bin/monitor/review_batch.rs):481-513；[push_templates.rs](../../src/bin/monitor/push_templates.rs):9927-9964、10072-10096。
- 自动scheduler每60秒tick，仅A股交易日19:00后运行；当日schedule初始Pending。R09自身15:35门在此时已开，未来业务日则永久Failed。[main.rs](../../src/bin/monitor/main.rs):5656-5660、6122-6135、6179-6183、6244-6269；[review_batch.rs](../../src/bin/monitor/review_batch.rs):1207-1213、1729-1751。
- 自动attempt实际构造`at_manual(now)`，但R09门读取`eligibility_time`而非dispatcher为R04计算的23:59:59变量；因此manual override不绕过R09的同日15:35门。[main.rs](../../src/bin/monitor/main.rs):5717-5731；[review_batch.rs](../../src/bin/monitor/review_batch.rs):32-39、86-92、1729-1746；[push_templates.rs](../../src/bin/monitor/push_templates.rs):9900-9909。
- 手动`--review`一次纳入全部13任务，业务日是当前时刻最近已完成交易日；若业务日就是今天且早于15:35，R09为ExpectedWait，否则可跑。手动另建临时schedule做hydrate/audit；整个批次Partial也返回Ok，不能据CLI成功断言R09 Delivered。[main.rs](../../src/bin/monitor/main.rs):5527-5541、5556-5567、5579-5605。
- backfill扫描最近5个已验证历史交易日（不含今天，旧到新）的8任务集合，R09映射`ReviewProviderTopN`；历史日不受15:35等待门。[main.rs](../../src/bin/monitor/main.rs):5735-5765、5787-5807、5930-5948。
- backfill先resume原`date/kind/task_identity`；无decision才重取来源。Delivered跳过，RejectedDurable显式授权再resume，其他状态（含Uncertain/ManualRejected）不盲发；零投递统计不是通知终态。[main.rs](../../src/bin/monitor/main.rs):5904-5927、5930-6009。
- 普通启动在core DB绑定后、常规producer前运行all-date fixed-point reconciliation，成功才打开`producer_ready`；它恢复既存信封，不受交易日、19:00、15:35或5日扫描限制，也不调用R09 loader。[main.rs](../../src/bin/monitor/main.rs):4810-4849；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):1166-1184、2074-2123。

## 新准备的来源请求与准入

1. dispatcher先只读inspect精确R09 occurrence；已有decision直接解释状态，不调用provider。无decision才调用`CapitalDataGateway::provider_top_n_pair(review_date)`。[push_templates.rs](../../src/bin/monitor/push_templates.rs):6982-7048、7110-7122；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):1533-1587。
2. 本地构造两份request evidence：`VolumeRatio`与`MainNetInflow`，同一交易日、固定limit=20、固定A股filter，各有按capability+canonical请求生成的hash。[capital.rs](../../src/data_gateway/capital.rs):23-32、178-194、319-346。
3. 实际gRPC桥只发一次`ProviderTopNRankings`，params只有`date`；response converter再按metric拆成两份batch。固定limit/filter并非RPC显式字段，而由本地request evidence及返回行的后续校验约束。[grpc_source.rs](../../src/data_gateway/grpc_source.rs):3526-3547；[schema.rs](../../src/grpc_contract/schema.rs):173-177；[params.rs](../../src/grpc_contract/params.rs):5-23。
4. converter要求每行携带独立evidence，单metric内evidence完全一致；两metric交易日相同、provider/source相同而batch ID不同，缺任一metric或空总response都作为`invalid_evidence`拒绝。[convert.rs](../../src/data_gateway/grpc_source/convert.rs):2512-2636、2639-2732。
5. gateway对两路分别写采集审计，但只返回完整pair；任一路审计/校验失败即整体Err。两侧必须非空，Eastmoney/`eastmoney-web`、`source_at=None`、固定metric/unit/date、ordinal从1连续且batch ID不同。[capital.rs](../../src/data_gateway/capital.rs):198-275、348-423。
6. `invalid_evidence`和`invalid_request`明确不可重试；普通unavailable保留其typed retryability。dispatcher按GatewayError原retryability分类来源失败。[review.rs](../../src/data_gateway/review.rs):255-317；[push_templates.rs](../../src/bin/monitor/push_templates.rs):7037-7047。

## 排序、截断、渲染与信封

- R09不在客户端重新排序或截断：仅接受每路1..=20行并验证当前vector顺序恰为source ordinal 1..N；所以正文保留provider顺序，少于20行仍标作Top20并注明本响应条数。[push_templates.rs](../../src/bin/monitor/push_templates.rs):6571-6622、6624-6667。
- schema还核每行metric/unit、业务日、A股Equity、非空代码/名称、固定filter；两路batch ID必须不同。量比单位Multiple、主力净流入单位Yuan。[push_templates.rs](../../src/bin/monitor/push_templates.rs):6477-6568、6670-6702。
- canonical binding包含两request、两batch、顺序投影及hash、完整rendered bytes/hash、task transition basis；source fingerprint绑定业务日、两request hash、两batch ID与投影hash。[push_templates.rs](../../src/bin/monitor/push_templates.rs):6380-6468、6703-6768。
- envelope固定`ReviewProviderTopN/None/GLOBAL`、业务日、task occurrence、source/subject/content hash和两原batch ID，且`retry_authorized=false`；构建后再次核source/content/subject不漂移。[push_templates.rs](../../src/bin/monitor/push_templates.rs):6776-6840。
- canonical preparation失败一律映射可重试Failed；envelope或presentation token构建失败为永久Failed；durable调用自身Err映射可重试。这些分类不是由同一个typed错误枚举统一推导。[push_templates.rs](../../src/bin/monitor/push_templates.rs):7049-7107。
- 空榜单不会成为NoData：gateway把任一空侧判`invalid_evidence`，最终是不可重试Failed且当前进程schedule Terminal。[capital.rs](../../src/data_gateway/capital.rs):378-391；[review.rs](../../src/data_gateway/review.rs):281-303；[review_batch.rs](../../src/bin/monitor/review_batch.rs):1247-1273。

## 通知owner、恢复、hydration与附带效果

- 与R07/R08不同，R09没有调用`notify.rs`的`push_counted_*`入口；presentation token后直接调用`durable_delivery_runtime::deliver_presented_envelope`，这里只校验token kind与envelope kind/subkind再交coordinator。[push_templates.rs](../../src/bin/monitor/push_templates.rs):7085-7102；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):1354-1382。`notify.rs`仅提供该kind的枚举/标签，不是本链完成owner。[notify.rs](../../src/bin/monitor/notify.rs):130-131、530。
- 编译政策明确Global/BusinessDateOnce/86400秒且不计日预算；完成权是原业务日claim关联的immutable decision，不是两份采集审计、presentation token、共享预算或schedule state。[model.rs](../../src/durable_delivery/model.rs):530-539。
- 新投递次序：完整来源与renderer → envelope → durable `prepare` → 本地reconcile → Reserved才resume唯一sink → 再reconcile → 读取decision终态并排队hydration。[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):2126-2176。
- Delivered必须带hydration才算强成功；Delivered缺hydration为可重试失败。Rejected/ManualRejected/Uncertain为永久失败，其他未定状态为可重试失败；Terminal因此不等于Delivered。[push_templates.rs](../../src/bin/monitor/push_templates.rs):6843-6884、6887-6979。
- schedule消费hydration时核transition/basis/hash、业务日、task identity；Accepted/Rejected/Uncertain/ManualRejected均置Terminal。自动路径先hydrate再due，attempt后clone→hydrate→剔除durable任务→写legacy audit→commit。[review_batch.rs](../../src/bin/monitor/review_batch.rs):1433-1556；[main.rs](../../src/bin/monitor/main.rs):5465-5488、6257-6303。
- 无claim的单任务backfill仍调用完整批次dispatcher，后段会执行预测样本回填、大宗交易与IPO侧路；外层还会为每个历史日尝试R07所需closing valuation，虽R09本身不依赖它。[push_templates.rs](../../src/bin/monitor/push_templates.rs):10032-10071；[main.rs](../../src/bin/monitor/main.rs):5815-5867、5940-5948。
- 启动恢复只消费原immutable envelope，不重新运行日期门、request/schema准入、provider、排序或renderer；既存authority不证明当前来源仍有效。[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):1625-1737、2074-2123。

## 最重要迁移接点与缺口

- 最小接点是`dispatch_r09_provider_top_n_outcome_with_loader`中“preflight无decision”与`deliver_presented_envelope`之间：保留`ProviderTopNPair → canonical binding → envelope`整体，替换/扩展其父任务状态持久化，不要复制排序、request hash或通知owner。[push_templates.rs](../../src/bin/monitor/push_templates.rs):6996-7107。
- 新发现的静态恢复缺口：provider空侧/invalid evidence等不可重试失败发生在通知`prepare`之前；它只进入进程内schedule Terminal和文件task audit，没有可由startup reconciliation恢复的R09 decision/task版本。重启后当日schedule重建为Pending，可能再次请求。迁移不能用两份采集audit冒充父任务完成，也不能事后补造旧failure artifact。[review_batch.rs](../../src/bin/monitor/review_batch.rs):396-405、1207-1213、1247-1259；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):2074-2123。
- 合同边界：gRPC实际请求只含date，而limit/filter request evidence在客户端构造；迁移若把request evidence当作“远端实际接收字段”会扩大证据含义。应继续以返回行filter/order与两batch evidence的准入结果证明固定口径。

## 未验证项与文件身份

- 未查询真实采集audit、decision、claim、hydration或业务表；未验证部署、真实provider响应、TransportAccepted、远端接收或用户已读。
- 未证明生产存在R09历史信封；启动恢复仅为源码可达性。未运行任何动态验证，以上缺口是静态路径推论。
- 本链未见R09最终业务表保存；gateway的两份采集审计和durable decision/hydration是不同owner。

```text
81f0cdd551540f6ea25fb46c36efae1af997b711d0739fab8f4d60993d632b13  .superpowers/sdd/2026-09-11-chain-post-close-recovery/task-2-review-r09-trace-brief.md
0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3  docs/push-system/push-capability-catalog.v1.json
c87fa4bb56df1c786e11516f754316e0a3fb9d9f5f75aad8aa2a7cb02016488c  src/bin/monitor/main.rs
8006c7bc6143bd410087b832c81636bc81bb1c047709fd0b0eb100ee77a56b2c  src/bin/monitor/review_batch.rs
5096e5697f67338b45346d11efa8f9ec988774c17a8f9623b280adeaeb2044f4  src/bin/monitor/push_templates.rs
ce2112226a4c71cb12426edd151444e239c4844ccf50d49252a67be3af7c7bfa  src/bin/monitor/notify.rs
53df56b8078453e9ab141766efed8ff72709ac821298adfd4f05bdeb4c92ec78  src/bin/monitor/durable_delivery_runtime.rs
b14f970f6c3d28123d97d287564aa870eb07bead06c2dcf44ac2be536c86d0b9  src/durable_delivery/model.rs
4b803f60bbabe3f67725f0eaf63ceffbe7ef1e1f1220d9d0cb764129632616d0  src/data_gateway/capital.rs
358ad584fc546e18194871900d214920762c93cf611a4cf3d05f622ddf3916d7  src/data_gateway/grpc_source.rs
d1bef9d1ed38d5c97e9a59577f9d48bbee84a44a0bdab1bd2e282e51ee82b375  src/data_gateway/grpc_source/convert.rs
e637a4702821fcbd59eef5e482233098b4d3bfa6ed2a48550a6133ad8f3e8cfc  src/data_gateway/review.rs
6879a721eb57c85e431731d8a928ff287c08311ca7bb3c8e3d51f927b45b65e0  src/grpc_contract/schema.rs
17f0989ffe136c35f5d37e2d041e4f5cfb47a3fe424c0c35da16068987d291c8  src/grpc_contract/params.rs
```

完整剩余范围见[迁移证据清单](remaining-migration-evidence-2026-09-08.md)，本记录不关闭盘后持久准备、真实调度、逐目标通知或切换验收。
