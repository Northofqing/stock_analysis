# W19 持久化全量指标与覆盖范围

日期：2026-09-08。状态：本计划Task1完成。原始基线3af4fb2，初版8094ffc、修复c10ec78；最终107项相关测试及Clippy通过，限定复审I1关闭、无新增问题。中文证据见[全量库存实施记录](../../push-system/implementation-w19-inventory-results-2026-09-08.md)。本计划仅推进W19实际库存/指标读取，不改变完整W01–W21、52Unit迁移与生产门禁目标。

## 目标与来源

把已完成的单intent检查接到指定namespace的全部持久化intent库存，而不是接收调用方手选的一组成功report。按RFC:771,776,791,1251,1253,1338–1339保留实际接受、人工处置、业务完成、未决、冲突和失败的独立统计；预算不足/来源不完整不能呈现无积压。

当前证据：intent_store.rs:1290 的scan_recovery_page仅cfg(test)，只扫描四种恢复状态，且不建立跨页一致读取，不能复用为全量指标。finalization_sla.rs:248及修复6f8713c已实现三类实际来源/时间/全链历史检查。catalog.rs只有身份/owner/occurrence关系，不提供完整CompletionPolicy/实际source注册。当前指标输入仍需显式typed绑定，不得从MachineCatalog的静态存在推导生产路由已就绪。

## Global Constraints

- 仅工作树 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905` / 分支codex/push-reliability-20260905；根工作树、生产monitor、真实数据库/provider/LLM/sink/订单/PAM/owner/.env/部署不操作。
- 不改冻结SQL、八份RFC输入、catalog/WBS/蓝图、既有canonical字节/哈希域/磁盘协议，不增加依赖，不添加自动发送/恢复/清理/晋级或默认认证。
- 父线独占Cargo/Git/mod.rs注册及中文docs；唯一实现代理只改授权源码/测试/report，不跑Cargo/Git/网络/生产、不起子代理。apply_patch编辑，定向rustfmt，禁止全仓cargo fmt。
- 原单条SLA行为、N02局部路由限制及6f8713c历史一致性修复必须保留；实际源验证、时间计算和历史一致性判断保持唯一实现。

## Task 1: 同一库存读事务中的实际SLA指标

### 文件与边界

- 新增 `src/push_foundation/finalization_metrics.rs` 与 `finalization_metrics_tests.rs`，父线注册crate内部模块。
- 修改 `intent_store.rs` 的读取层，提供source-owned DEFERRED只读事务、稳定namespace库存分页和实际snapshot/全链校验。复用query_intent/query_transition_chain，不另建SQL验证栈/写入opener。可新增私有child读取模块，先通知父线其路径和归属。
- 修改 `finalization_sla.rs`，让单条入口和库存入口共享同一已验证读取结果的计算路径；用只能由实际读取构造的私有proof/view传递snapshot+chain，不能开放接受任意snapshot/chain/report组合的成功构造器。原公共/内部单条查询使用方式和语义保留。
- 可定向调整 `finalization_sla_tests.rs` 的fixture可见性或扩展参数以复用三类真实writer，不改原测试期望/覆盖，不复制一整套同样的千行fixture。需要全新组合fixture时保持本任务文件内且通过真实存储接口。
- 不修改durable reader、业务writer/finalizer、reconciler或生产调度接口。定向核对确认 N02 冷读会创建年度lock，特例授权 `src/event/dispatcher.rs` 的 `read_authoritative_year` 只打开既存lock：复用现有 `open_existing_audit_file`，保留正常读取、完整链、保留目录与叶节点身份校验；不改append、文件格式或写入路径。先在单条/批量真实入口跑缺失lock反例RED，再最小修复；其他接口缺口仍须先报告。

### 可验收行为

1. 输入显式namespace、观察时钟、非零精确微秒周期、读取预算和typed来源绑定集。查询从DB枚举该namespace下全部业务日/所有状态intent，不接收手选intent列表、不默认只查今天/未决。另一个namespace不进入本次统计；报告仅声明这个明确选择范围，不声称全部DB/生产部署已覆盖。
2. 库存总数、稳定ID分页、每条snapshot与完整chain来自同一个source-owned DEFERRED事务，页大小1..=1000，最大检查数1..=100000；配置非法提前拒绝。流式处理每页，不保留所有payload/正文/完整链。事务结束rollback，不改PRAGMA/不偷提交嵌套调用方事务，不建立新库，不复制/checkpoint WAL。
3. 报告包含实际库存总数、已检查数、未检查数和Complete/Limited覆盖状态。到预算上限仍有数据时明确Limited，不能作为完整无积压报告；恰好全量等于预算可以Complete。分页不丢/重数，空库存是空库存事实，不是source配置或部署健康通过。DB枚举/事务失败返回脱敏错误，不返回看似完整的半份结果。
4. 每个已枚举intent必须有一个结果计数：实际SLA状态或明确业务损坏/路由缺失/不支持/验证错误。不能catch后continue漏分母。损坏单条不冒充NoData；保留其错误计数，是否继续枚举按读取接口能否安全定位下一ID处理；不能安全继续时整个读取失败。由SQL/错误字符串带来的任意文本不得进入Debug/Error。
5. 显式绑定集映射真实Unit/template与CompletionPolicy/实际来源；以(Unit,持久化template hash)精确选择，允许同Unit的不同历史模板并存，重复同pair或歧义来源提前拒绝，未知Unit/模板/来源不能回退到任意Generic或忽略。不新增从调用方布尔生成Verified/SLA成功的接口。来源必须为实际coordinator/AuditDispatcher，当前source读取不由可任意回调的用户report/provider端口代替。
6. N02仅在已支持family下，从持久化occurrence key通过现有NewsFlashWindow::parse选择窗口，再让原SLA精确核对；不能从观察时钟或显示文本猜窗口，不能接受任意调用方family/key/window自声明映射。未知约定保持UnsupportedOccurrenceRoute或明确route错误。这仍不是生产注册证明。
7. 聚合同时保留业务状态计数、SLA状态/错误计数以及观察到的terminal disposition计数；TransportAccepted与人工接受/不投递/失败分开。只把当前Completed且真实Accepted的匹配样本计作正常完成；当前ResolutionRequired、Conflict、ClockUncertain等不能因有历史Completed被计作正常完成。
8. 两周期/硬上限与未决阻断计数复用单条report事实；接受样本的最大已完成延迟与当前未决接受年龄分开。尤其历史Completed后当前ResolutionRequired时，elapsed仍冻结于旧完成，但当前未决年龄必须取观察时间减原始Accepted。时钟不确定不计作有效时间样本；冲突与不完整总量必须显式可见，不能产生可被误当promotion权限的健康成功布尔。不得发明比RFC更宽松的通过标准。
9. 结果私有构造/只读、确定顺序，披露最小身份/计数/时间/覆盖元数据，保留本次observed_at及cycle/target以区分查询时点与SLA阈值，不携带正文/完整链/回执message_id/path/raw DB错误。可以有有界问题摘要，但总错误计数不得随摘要截断。不得新增高基数的每intent监控标签或把诊断文本当标签。
10. 查询本身不获取lease、不发送/append、不调用reconcile/finalizer、不推进cursor、不删除；事务/迭代budget边界及异常均无写入。业务库存是同一快照，来源库仍为实际逐次读取，不声称跨库原子、外部可信时钟、认证生产健康或晋级证据。

### 测试与证据

至少一个实际库存fixture含多页、同namespace多个intent与多业务日，混合正常完成、Accepted待完成/确认丢失、人工处置、来源Missing/PendingSeal、当前ResolutionRequired/历史冲突，另一个namespace的记录隔离。至少覆盖Generic/P01/N02三种实际source进入批量入口，不用手造SLA report代替；允许复用既有隔离Test fixture并最小扩展共享namespace/业务库参数。

断言独立预期计数/总数/覆盖/最大年龄，明确人工不冲抵实际接受、未决不被旧完成遮蔽；故意不注册一个已存intent的路由必须留错误计数。覆盖重复绑定、未知template、N02不支持/错窗口、单条损坏、预算前/等于/不足、非法配置和空库存。通过实际第二连接写入或其他已有事务边界证明跨页库存一致性，而非只断言新函数名。保持实际DB行、audit文件、sink/append计数不变及重启后统计一致；不读取真实数据。

父线冻结后运行：

```sh
env CARGO_PROFILE_TEST_INCREMENTAL=true cargo test --lib -- --test-threads=1 push_foundation::finalization_metrics_tests:: push_foundation::finalization_sla_tests:: push_foundation::generic_transport_tests:: push_foundation::dedicated_transport_tests:: push_foundation::terminal_authority_tests:: push_foundation::business_finalizer_tests:: push_foundation::tests::w08_
cargo clippy --lib --no-deps --message-format=json
```

N02 冷读修复后，同一最终合批另加入 `event::delivery_observation_tests::br244_` 和 `event::delivery_observation_tests::w13_n02_`，覆盖年度reader的正常窗口读取与业务日reconcile消费者。新反例在隔离fixture把年度lock重命名保留，分别从单条SLA/全量指标读取，验证SourceInvalid/错误分母以及目录文件名和字节不变；不得仅测试有lock正例，也不把编译失败当行为RED。

实际测试数量必须>0；六个既有相邻组上一批78项是基线，不当新代码验证。父线定向fmt/diff/RFC输入检查，43条test/163条Clippy既有warning单列，本批不新增诊断。原实现代理修复失败；固定原始BASE..HEAD独立Spec/Quality审查，不能用可控fixture绿灯替代来源/库存真实性。

### 报告与保留范围

实现代理写同名SDD的task-1-report.md，记录真实数据流、内部接口/注册名、文件、测试、自审、已知限制；未跑Cargo须明示。父线验证/提交/独立审查后中文docs收口。

W19仍需生产健康/晋级消费者、Uncertain人工响应SLA、保留期/法律保留/备份完整性/安全审计；完整W15/W16/W17接线、W18/W20/W21、52Unit迁移及生产批准不由本计划关闭。未知生产身份根只影响对应adapter，不授予默认权限。
