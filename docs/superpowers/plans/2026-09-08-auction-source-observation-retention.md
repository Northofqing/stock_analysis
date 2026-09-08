# P-02 真实采集观察与审计回执保留

日期：2026-09-08。状态：待实施。原始BASE `250932d61152b348144625319f3597e6eb306f95`。

## 目标与当前合同

将现有实际Gateway→MarketAnalyzer→P-02 tick路径中被丢弃的原始记录、分片身份及本地审计回执保留到消费阶段，作为完整W17来源绑定的必要输入。不是重新实现前批冻结业务准备，也不通过一个无人调用的包装类型交付。本批只保留事实，不认证生产来源或赋予发送许可。

依据：[来源缺口核对](../../push-system/auction-source-evidence-gaps-2026-09-08.md)；BR-159/BR-213/BR-220/BR-221既有采集审计、exact-date涨停池、名称专用补充及独立分片合同；Foundation RFC §共享事实与P-02同批门禁。现行来源没有量比，跨批补量比仍被禁止。本批不解决该产品/提供方合同缺口，不让来源观察对象冒充PreparedFacts、生产认证或VerifiedEmpty。

当前源码：`review.rs:1288`已有真实批次+receipt helper，`:1304`与`:1314`兼容出口丢receipt；`:678`当日涨停池用该出口，另有P01调用者。`market_capabilities.rs:292`名称接口也丢receipt。`limit_up.rs:18`仅保存投影/批次证据，原记录在组装中被消费，`:271`真实composition receipt仅用于日志，`:303`最终丢证据。`push_templates.rs:6030`的tick helper与`:6050`真实loader仅消费Vec；`main.rs:9710`直接拆开tick字段，新增证据若不修此接点仍会被丢弃。

## Global Constraints

- 仅隔离工作树`.worktrees/push-reliability-20260905`/分支codex/push-reliability-20260905；根工作树、生产monitor、真实数据库/provider/LLM/sink/order/PAM/.env/owner/批准/部署不操作。
- 冻结SQL、八份RFC输入、catalog/WBS/蓝图、既有canonical前像/哈希域/磁盘协议不修改；不增加依赖、来源回退、量比补值、自动发送/重试/恢复或权限默认值。
- 仅一个实施代理；父线独占Cargo/Git/中文docs及主入口注册。代理仅apply_patch/定向rustfmt，不Cargo/Git/网络/生产、不起子代理；不全仓cargo fmt。
- 保留现有BR-213/BR-220/BR-221准入、原始成员顺序、名称分片上限50、名称仅展示、可选量比/主力净流None、空池不请求名称与审计失败整批拒绝。P-02原时间窗、稳定筛选、冻结提案、注册元组、sink/recorder/通知集合与持仓消费语义不改变。

## Task 1: 从真实采集保留完整观察，并贯穿实际竞价tick

### 文件与所有权

- 实施代理：`src/data_gateway/review.rs`、`src/data_gateway/market_capabilities.rs`、`src/market_analyzer/limit_up.rs`、`src/bin/monitor/push_templates.rs`及上述文件相关测试。可新增聚焦的`src/market_analyzer/limit_up_observation.rs`保存只读观察类型/相关组装；先报告父线名称和注册方式，不复制整套旧组装算法。
- 父线：`src/market_analyzer/mod.rs`的必要公开导出，以及`src/bin/monitor/main.rs`竞价分支保留完整tick的最小调用适配。代理报告最终interface后父线修改；不得各自编辑同一文件。
- `current_upper_limit_pool`的P01兼容调用在`src/pipeline/chain_analysis/p01_projection.rs:312`，此次不改它。其他Gateway/实时准入/provider/client/认证合同不改。

### 行为合同

1. **同次实际审计的窄出口。** 涨停池和名称各提供保留批次、实际请求hash及实际`DataAcquisitionAuditReceipt`的内部出口；旧接口委托同一路径并明确投影回旧返回值。实际request hash必须来自当前计算，不在消费者重算，不改旧canonical字段顺序/域。请求失败仍按原路径审计并返回显式错误；任何append失败不得返回成功观察。不为了拿receipt再采集或再追加审计。
2. **只读完整来源观察。** MarketAnalyzer的实际新入口返回私有字段、无任意成功构造器/Default/可变访问器的观察对象。保存指定交易日、同次原始涨停池`GatewayBatch<LimitPoolEntry>`及记录顺序/全部可选字段、请求hash与receipt；每个名称分片分别保存原请求代码列表、原始`MarketSecurityIdentity`记录/批次证据、请求hash与receipt；保存原有投影股票及Available时真实composition receipt。只读访问和必要Clone/共享所有权可用，不新增持久化协议或认证标签。
3. **不伪造空结果与回执。** VerifiedEmpty只来自实际完整空涨停池，保留它自己的采集观察，名称调用/分片数必须为0，composition调用保持原来0次；不生成占位composition receipt。Available必须通过原有非空/身份/代码集/名称/分片校验及一次composition审计。Unavailable/Partial/Conflict/名称缺失/审计失败仍整批Err，不折成空观察。
4. **保留真实分片语义。** 每条名称只与其实际分片provider/batch/observed_at绑定；不得把逐记录source_at强改成批次source_at，也不合并/synthesize批次身份。保存的请求代码分片来自实际发出的分片，不能事后按返回顺序猜测。保持exact-code整体校验与既有gateway每次请求校验；测试需覆盖错片/重复/漏码，不以名称常量补值。
5. **兼容路径仍来自同一实现。** `get_limit_up_stocks`和原P01/current_upper_limit_pool返回值/准入/请求次数/审计次数保持兼容。新观察入口与旧路径共用真实采集和同一组装算法，不新增第二套更宽松projection；若保留旧私有helper给测试，应由生产路径实际调用，而不是仅旧测试调用。
6. **实际P-02 loader消费而不是声明。** `load_auction_volume_tick_real`调用新的MarketAnalyzer观察入口，从该观察的stocks生成现有快照；tick保留同一完整观察。原合成股票seam可为纯算法回归明确保留“无来源观察”，不得因此构造伪回执或伪VerifiedEmpty，也不能把无观察当生产认证。即使量比仍None导致snapshot为Err，原始股票、Available/VerifiedEmpty区别及来源观察仍保留在成功取得的tick中。
7. **主入口不丢弃。** main竞价分支保存完整`AuctionVolumeTickData`，从它借用选中快照及原始股票，覆盖实际P-02调用和后面的持仓detector消费阶段；不要只绑定一个无消费者的`_receipt`，也不立刻into_parts后丢来源。保持原出错日志和失败继续行为，不改变其它竞价推送或sleep。来源对象的保留不等于把它宣称为Foundation认证上下文。
8. **无新增披露。** 新观察/内部回执容器Debug仅给类型、状态、数量，不打印原始records、账户信息、正文、请求代码、完整hash/receipt或原始外部错误。显式只读业务访问可取事实；诊断默认不展示。保留原审计日志，不增一套来源日志。

### 实施顺序与验证

先完成同次audit出口和只读观察组装，再接真实MarketAnalyzer与P-02 loader，最后父线适配main并验证整个调用链；作为一个完整Task审查，不为每个getter单独提交/审查。新功能测试与实现一起收敛；若发现真正需要改变旧行为的回归，先报父线并取得实际行为RED，编译失败不算RED。

测试需从生产共用seam进入，替换仅实际外部采集；审计正例使用现有`AttributionDatabaseSession`或同等任务独有临时SQLite及真实`record_data_acquisition`，不能手填成功receipt/report或使用`DatabaseManager::init(None)`、固定test.db。允许把现有audit算法提取为接受显式测试数据库的内部函数，生产仍传真实既有数据库；不新增任意公开source-auth构造器来迁就跨crate测试。lib集成用例证明真实观察/原始字段/回执，bin原算法用例和实际调用核对证明loader/main接线；未运行真实provider的部分明确记录，不能声称生产通过。

独立预期至少覆盖：

- 一次完整涨停池＋多片名称→每片原请求/记录/证据/真实request hash和receipt逐项保留；同次实际审计行数=池1＋名称片数＋composition1，旧兼容投影不重复写；记录全可选字段和顺序不丢。
- provider证实空池→保留空池receipt、0次名称和composition；有非空池但全部量比缺失→仍是Available来源/原始股票，P02 snapshot Err，不是VerifiedEmpty。
- 名称错片/重复/漏码、第二片失败、composition写入失败→拒绝，保留既有已写审计，不重试/重发/回滚；无占位成功对象。
- 名称逐记录source_at与批次不同但符合旧合法归属时仍接受；缺失量比/主力净流仍None。
- 新只读类型Debug脱敏；生产入口/兼容入口共享同一采集和组装，原P02全部独立消息/通知集合断言保留，不能用新helper相等性替代原literal。

父线最终命令按实际测试名安全组合：新lib用例统一`br213_observation_`、`br221_observation_`前缀，保留已核对纯`market_analyzer::limit_up::tests::`。不要全跑`data_gateway::review::tests::`，其旧用例有全局DB初始化和网络调用。改动的audit兼容邻域由父线逐个核对副作用后补入；bin沿前批十组安全前缀＋旧有限正值纯函数，显式绑定新建的专用DISPATCHER_LOG_DIR。lib与bin Cargo串行、保留唯一session；再`cargo clippy --bin monitor --no-deps --message-format=json`、逐文件rustfmt check、diff检查及`ruby scripts/architecture-docs/check-rfc-inputs.rb --root .`。

既有test/Clippy诊断按目标和基线分列，不新增allow掩盖。main有基线格式差异，不能因定向检查重排无关行；主入口只改所需hunk。最终以完整原始BASE..SOURCE独立Spec/Quality审查，修复回原实施者，复审只针对FIX_BASE..fix。

### 交付与剩余完整范围

代理将全数据流、真实调用者、全部变更文件/函数、测试列表及未运行项写同名SDD的task-1-report.md；父线写中文docs与验收证据。仅完成本Task，不称P-02已能发送或完整W17完成。仍须真实竞价量比合同、生产来源身份/受约束context与factory、完整新旧业务比较及全局副作用纳管、业务最终化/游标、完整W15/W16/W18–W21、其他Unit迁移、文档工具和真实发布门禁。
