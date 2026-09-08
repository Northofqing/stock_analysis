# P-02 竞价量比与来源证据缺口

日期：2026-09-08。这是隔离开发树的源码核对，不是生产运行观察或数据提供方认证。所引上游文件在本轮HEAD `617f37f`未修改；进行中的横幅/提案改造仅触及`push_templates.rs`。没有调用真实provider、数据库或推送通道。

## 结论

当前涨停池路径不能提供P-02要求的有限正量比；这不是补一个字段即可解除的代码缺陷。BR-213/BR-220明确只允许跨批补展示名称，禁止跨批补量比、主力净流。现有`MarketStatistics`虽然有optional量比，但尚不能证明同一竞价时段、精确证券集合及量比计算口径。不能直接拼入旧`TopStock`，也不能把缺字段或未接能力当作provider证实的空结果。

## 已有路径与边界

| 路径 | 实际证明的范围 | 尚未证明的范围 | 源码/规则证据 |
| --- | --- | --- | --- |
| 当前日涨停池 | 允许provider、完整批次、唯一代码、Upper、指定交易日、逐记录与批次证据一致 | 竞价阶段的成员是否有效、精确竞价时点、量比 | `src/data_gateway/review.rs:733`、`:758`；`src/market_analyzer/limit_up.rs:186`明确`volume_ratio: None` |
| 名称补充 | 独立分片、每片provider/batch/observed_at一致、合并后代码集与涨停池完全一致 | 不授权行情/量比补值；逐记录source_at不要求等于批次source_at | `limit_up.rs:130`、`:165`；`docs/business_rules.md:151` BR-220、`:170` BR-213 |
| MarketStatistics | 受准入约束的响应、证券身份解析、optional有限量比 | 请求集精确一致、逐证券原始source time、竞价session及量比定义 | `src/data_gateway/company.rs:106`、`grpc_source.rs:3063`、`grpc_source/convert.rs:2353`、`:2390` |
| Auctions | 本地协议存在操作名 | 未在实际操作实现列表，client拒绝未实现操作；本仓没有可复用的真实采集链 | `src/grpc_contract/ops.rs:15`、`:77`；`src/grpc_client/client.rs:196` |
| VolumeRatio Top-N | 特定来源的量比排名页 | 不覆盖指定全部涨停池成员，不证明竞价session；板块量比也不是个股量比 | `src/data_gateway/capital.rs:59`、`:178`；`grpc_source/convert.rs:2197` |

`CompanyDataGateway`中关于BR-205 Gate-A的注释仅说明它不是动态委托价格权威，不能扩大解释为所有市场统计均禁止展示；也不能反向当作P-02竞价准入已通过。`convert.rs:1732`把批次证据复制给各条记录，不等于取得逐证券原始时间。请求关联校验也不等于请求证券集与返回证券集完全一致。

未核对外部provider服务实现；本checkout没有文档引用的`src/grpc_server/delegate.rs`。不能从客户端可选字段或协议名字推断服务端支持与数据语义。

## 可以先完成的证据保留工程

现有采集与审计已经产生真实证据，后续应保留而不是再调用一次来补凭证：

1. `src/data_gateway/review.rs:1288`已有同时返回批次和`DataAcquisitionAuditReceipt`的内部helper；`:1304`兼容出口丢弃receipt。增加窄的证据保留出口时必须复用同次采集、同次审计，不增加provider请求或重复审计写入。
2. 保留涨停池原始记录及顺序、真实批次证据、实际请求身份/请求hash及receipt；名称按原请求分片保留代码列表、原始记录、各自批次和各自receipt。不能只存最后的`Vec<TopStock>`或合并后的一个批次身份。
3. `limit_up.rs:271`已调用composition audit，但`:303`最终只返回stocks。应保留真实composition receipt，以及它确实绑定的日期、批次身份和记录数；不把现有摘要宣称为逐票量比组合证明。
4. `DataAcquisitionAuditReceipt`的真实字段是`audit_id`、`record_hash`、`previous_outcome`、`current_outcome`（`src/database/data_acquisition_audit.rs:38`），不是provider签名、生产身份根或竞价session认证。既有canonical前像不改写。
5. 新出口需由实际`MarketAnalyzer`/loader消费并保持旧兼容路径，验证Available与VerifiedEmpty区分、空结果不请求名称、各分片精确归属、完整代码集、审计失败整批拒绝以及没有额外采集。仅新建一个无人消费的包装类型不能算交付。

这些是下一任务的工程边界，尚未实现或验证；当前冻结业务准备任务不据此扩大文件所有权。

## 接入真实量比前需要补齐的合同

以下缺口不靠本地假数据、调用时刻或成功布尔补足：

- 产品合同：是否允许为P-02新增独立的跨来源组合合同，以及它如何窄化现行BR-213/BR-220，而不改变其他消费者。
- 提供方事实：真实可用的provider/operation、竞价量比的分子与分母、回看区间、对应原始时刻；是否包含竞价匹配量或累计成交量。
- 时间与身份：竞价阶段边界、同交易日/同session校验、允许的跨源偏差、新鲜度与未来时间规则；精确请求代码集、返回/缺失集合、重复/额外代码及跨片冲突处理。
- 业务完整性：当前日涨停池在竞价阶段能否产生有效成员；缺量比时是逐票排除还是整个采集失败。现在的过滤行为不能自动成为新来源合同的完整性决定。
- 可审计性：逐票值保留其真实来源与分片关系；新增组合采用独立版本化的精确绑定，不修改旧摘要协议或覆盖未知source time。

RFC `docs/push-system/push-system-implementation-rfc.md:1116`要求共享一次捕获的事实，`:1615`要求同批业务绑定与有限正量比；它们没有指定MarketStatistics join，也没有直接撤销BR-213/BR-220的禁令。

当前可以继续冻结业务提案、保留来源证据及其他独立Foundation工程。没有完整来源合同前，不启用P-02新来源或声称完整W17/Unit迁移成功；生产身份、操作员权限、全局效果纳管及发布验收仍分别需要真实证据。
