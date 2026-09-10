# W17：同次影子执行比较完整业务提案并保留旧输出

日期：2026-09-10。状态：计划及独立只读预检完成，四项interface澄清已通过限定复核；未派发、未实现、BASE未固定。须先完成当前P-02选集诊断任务及独立审查，交接唯一Rust/Cargo队列后再执行。本任务不重开已完成的W17语义比较内核，不代表完整P-02 adapter或生产迁移完成。

## 目标与权威

落实[完整P-02业务提案接线前置](../../push-system/p02-shadow-integration-handoff-2026-09-10.md)及[RFC影子精确比较/副作用合同](../../push-system/push-system-implementation-rfc.md#影子精确比较proposed)：在同一次影子执行内比较两侧实际完整业务提案，保留被比较的旧提案供原合法owner后续消费。Match不授予发送、完成、认证或晋级权限；新路径失败不能自行关闭或恢复旧owner。

已读取1931014相关源码：`src/monitor/push_job/shadow.rs:127–172`的ShadowObservation无业务载荷；`:279–345`只返回ShadowReport；`:409`之后精确比较已覆盖的决策/语义/字节/完成提案。`src/bin/monitor/push_templates.rs`的PreparedAuctionVolumeDispatch已包含消息、有序逐票记录及通知集合，其PartialEq覆盖所有业务字段。消息字节相同而记录价格/指标或通知集合不同的独立旧测试已存在，不能将消息相等提升为完整提案相等。

## Global Constraints

- 仅在`/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`的`codex/push-reliability-20260905`开发；不访问根项目checkout、真实.env/data、生产DB/monitor/provider/sink/订单/PAM/凭据、网络、远端Git或部署。
- 一个实现者独占Rust/Cargo；主线拥有Git/docs/独立审查。实现者不派子agent、不写Git。前置任务释放ownership后才能派发。
- 不改P-02 selector、真实loader、main、dispatcher、来源/量比合同、通知集合推进、模板、schema/canonical/冻结输入、激活及认证权限。
- 不改Cargo/target/RUSTFLAGS/incremental；仅格式化本任务改动，不全仓重排。已有同源码有效测试不为报告重复运行。
- 不把框架泛型比较结果当完整Unit认证；既有全局I/O无法由纯callback接口沙箱化的限制必须继续明确。

## Task 1: 同次执行的完整提案比较与所有权

### Interface选择及边界

选择在原W17执行模块内增加“带业务提案的执行”interface，并共享原验证/执行逻辑；不建立第二套独立影子引擎，也不让调用方先运行旧引擎再拼接一个自报相等bool/hash。保留现有execute_shadow及其调用者合同，旧接口的Match仍只证明其原先覆盖范围。旧ShadowReport及公开闭集ShadowInvalidBinding/ShadowDifference/ShadowPathStatus保持不变，不为本任务添加会破坏外部穷尽match的变体；新增业务报告使用独立闭集类型承载原差异与业务提案差异。

完整业务提案类型由实际adapter拥有；这里使用受约束的泛型载荷`P: PartialEq`，以直接比较同次返回的实际值。无需引入新的持久化canonical/schema，不复制P-02逐票记录结构到Foundation。该机制只能证明“提供给本次执行的载荷按其判等规则一致”：实际接线还必须审查P-02真实完整类型及两个adapter，不能用空载荷、漏字段的替代类型、相同结果克隆两份或不可信PartialEq宣称业务一致。

建议公开类型/入口为`ShadowBusinessObservation<'a, P>`、`ShadowBusinessExecution<P>`、`execute_shadow_with_proposals`，实际实现可调整命名以符合现有代码，但须保留以下合同：

- 每侧callback至多调用一次；仅初始context/facts绑定通过后，两侧各调用一次，任一侧返回失败也不能跳过另一侧。每侧一次返回原ShadowObservation与其拥有的`Option<P>`；不要求P实现Clone、Copy或Debug。
- Ready必须带Some实际业务提案；非Ready不得带发送业务提案。缺失/错误存在性即使两侧一样也不能Match，产生固定、可区分path的InvalidObservation事实。
- 只有两个callback均返回提案时才比较其实际值；不同产生固定BusinessProposal差异。不能只比较调用方提供的摘要。原有错误、绑定、效果计数和语义差异仍保留。
- 返回对象同时保留此次report与旧callback产生的原提案，并允许一次性取出；没有旧结果时为None，不重新prepare、不Clone，不返回新侧提案当旧侧。只有旧侧观察通过绑定和存在性验证时才保留旧提案；新侧失败或提案不一致不得自动丢弃有效旧提案。
- 返回旧提案只转交普通业务数据，不是执行许可；它不以整个report的Match作为旧owner是否继续工作的授权决定。后续真正dispatcher仍须消费原合法owner/fence，不新增绕过激活的发送入口。
- 新接口返回独立的只读业务report，将原有证据与新增提案证据合并到其is_match/differences/reasons。业务report中的每侧status描述执行/观察有效性：缺失或错误存在性使该侧InvalidObservation；两个有效提案仅内容不同则两侧仍Completed，但整体不Match并带语义差异reason。保留原path计数和所有原差异，不把内容差异误报成callback执行失败。
- 不向新调用者暴露可独立is_match为true的内部基础ShadowReport；需要的原证据通过业务report的只读查询统一提供，取走旧提案后保留的也是这份完整业务report。不得只返回基础report加一个可被遗漏的payload_equal字段。
- Debug只显示类型、存在性、path/status/差异或数量；不调用P的Debug，不打印提案正文、股票、价格、URI或任意回调错误。

代价是增加一个模块级执行interface及类型，保持语义版兼容；收益是完整业务输出的比较和所有权能被同次执行约束。尚缺真实P-02注册、context/capture/projector工厂接线、八类实际效果端口纳管及部署/fence，本任务不补造它们。

### 文件与实现

仅修改`src/monitor/push_job/shadow.rs`、公开导出所在`src/monitor/push_job.rs`和纯测试`src/monitor/push_job/shadow_tests.rs`。没有必要修改真实monitor或另建业务模型；如果发现必须跨出范围，先报告具体interface原因。

在现有模块内实现上面的interface。复用原context/facts初检、两个callback各一次执行、Ready/NoData绑定校验、失败状态和八种先计数再拒绝能力；新旧入口不得复制整段执行/验证/计数算法。原接口保持返回类型及行为，原有独立测试不删除或弱化。

新执行报告同时覆盖业务提案存在性和内容差异。旧侧完整有效提案以所有权移动保存；新侧失败、被拒效果或不一致时仍保留该旧数据，同时report不能Match。旧侧未执行/回调失败/观察无效时不产出旧提案。不能把“旧数据可取出”命名或包装成发送capability。

### TDD与验收

用现有真实context/facts/projector的纯fixture，从新增执行interface取结果。新增测试名以`business_proposal_`开头；合成载荷包含消息、有序逐票记录、通知集合并使用明确完整判等，不能用两次同函数调用作为预期。

至少覆盖：

1. 两侧独立产生相同完整提案，report Match；旧提案可一次移动取出，输入同实例、每侧只执行一次。使用不实现Clone/Debug的载荷，通过旧侧已拥有堆对象的地址等测试观察证明返回的是原对象，不是新侧/重建值；实际业务字段全部参与判等，不新增业务比较排除项。
2. 消息完全相同而价格bits、隐藏原始指标、记录顺序、通知集合各自改变，分别使report出现固定业务差异；旧侧完整提案仍可取回。预期使用固定字面量，不由被测比较器计算。
3. Ready缺载荷、非Ready带载荷，两侧同时犯相同错误也不能Match；明确旧/新path、InvalidObservation、固定binding及旧输出存在性。
4. 新侧callback失败仍保存已完成且有效的旧提案；旧侧失败不保存；两侧失败均报告。不得跳过另一侧或重执行旧callback。
5. context/facts不匹配时两侧均未执行、无提案；错误context/facts实例、无效Ready/NoData观察不能被载荷相等掩盖。
6. 两侧八种实际提供的拒绝capability，任一调用都先计数后拒绝；即使提案相等或callback忽略拒绝也不能Match，保留精确path计数。此计数不证明任意全局I/O已纳管。
7. 合法非Ready分支无提案可比较为原来一致状态，不制造发送业务数据；旧接口17项既有行为保持。
8. Debug/errors包含敏感合成字符串、特殊价格/记录时均不泄露正文或调用P的Debug；取走载荷后report的证据仍能独立保留且不改变。

记录实际接口RED和GREEN，编译缺类型与运行期反例分开说明；不制造错误实现补过程。最终固定相关源码后：

- `cargo test --lib monitor::push_job::shadow_tests -- --test-threads=1`：覆盖既有17项和新增测试，匹配测试数必须非零、通过时真实退出码必须为0，并记录实际总数及完整stdout/stderr；这里为纯内存测试，不运行monitor。
- `cargo clippy --lib --message-format=json`：完整保留诊断，区分改动目标及非目标；无BASE对照不得称非目标全部既有。
- 对改动块做定向格式检查、`git diff --check`，记录最终源码hash及验证前后未漂移；不重排大型pub-use文件的无关部分。

主线提交SOURCE后固定BASE..SOURCE，进行独立Spec/Quality任务审查，再关闭此Task。该Task完成只表示W17可覆盖并保留真实传入的业务提案；真实P-02 adapter尚未由此接入，不可把它记为完成Unit迁移。

## 后续依赖与回滚

下一层必须让实际P-02旧/新adapter在同一次context/facts/captured banner下返回完整PreparedAuctionVolumeDispatch，让真正dispatcher消费此次保留的旧提案，并建立真实身份、来源与效果端口证据。不得重造已完成来源观察或冻结准备，不得把真实缺量比/未认证输入提升为Ready。

回滚只撤销新增执行interface/类型和相应纯测试，保留原execute_shadow合同；本任务无持久化迁移或生产状态需要回滚。
