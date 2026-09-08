# 恢复分类 Uncertain 的真实终态读取修复

日期：2026-09-08。状态：待实施。原始BASE `3cca5cc9a0bc9239d8900585593ade6a492af4fa`。前批来源观察保留已完成，不重做。

## 目标与已经核实的缺口

使真实过期attempt恢复生成的Uncertain，在原恢复事件、fence与disposition严格绑定后，经实际Generic/P01终态reader及已有Foundation消费者读取为Uncertain，而不是SourceInvalid。它仍是未解决隔离状态，绝不成为Accepted、Completed、NotDelivered或重发许可。不是新增一套恢复写入协议，不是完整Q39告警/解决SLA。

依据：既有W09/W11/W12的精确authority读取/未知结果隔离；RFC第342、775、1177、1253行；`.planning/2026-09-06-push-foundation-runtime/w19-uncertain-sla-source-map.md`。当前`src/durable_delivery/coordinator.rs:5535–5589`真实保存FenceRevoked、RecoveryClassified和Uncertain disposition（其evidence hash指向fence evidence）；`:6443,6545`却对全部Uncertain要求authoritative sink result。现有`tests.rs:7013`证明过期恢复，`:8530–8598`只覆盖sink返回Uncertain，两者没有贯通测试。

父线已读真实下游：`generic_transport.rs:510–584`按已验证disposition映射并保留原证据；P01 `dedicated_transport.rs:522–584`只为Accepted解析receipt；共享`terminal_authority.rs`重验身份/字节哈希；reconciler将Uncertain转ResolutionRequired；SLA/metrics只对Accepted计算接受后最终化时间。不能让修复绕开这些实际接口。

## Global Constraints

- 仅隔离树`.worktrees/push-reliability-20260905`，分支codex/push-reliability-20260905；根工作树、生产monitor、真实业务库/provider/LLM/sink/订单/PAM/.env/owner/批准/部署不操作，不联网。
- 不修改冻结Foundation SQL、durable schema、八份RFC输入、catalog/WBS/蓝图、已有canonical字节/哈希域或磁盘写入协议；不增加依赖、成功默认值、伪造sink结果、自动发送/重试/人工处置或新的权限。
- 只用一个实施代理；父线独占Cargo/Git/中文docs/必要模块注册。代理只编辑本Task源文件/测试/报告，定向rustfmt；不运行Cargo/Git、不起子代理、不全仓格式化。固定原始BASE做一次完整Task审查，修复回原代理。
- 读取接口必须保持只读；审计尚未封口继续PendingSeal，不把缺失或损坏记录当恢复证据。源错误不得宽松回退另一类来源；旧Accepted/Rejected/人工处置/typed sink Uncertain及原独立断言全部保留。

## Task 1: 让真实恢复证据穿过实际终态读取和消费

### 文件与范围

- 实施代理：`src/durable_delivery/coordinator.rs`、`src/durable_delivery/tests.rs`、`src/push_foundation/finalization_sla_tests.rs`、`src/push_foundation/finalization_metrics_tests.rs`。消费者生产代码应复用原逻辑；若实证还需最小修改，先报告父线具体缺口与文件，不静默扩散。
- 可在coordinator内增加聚焦私有校验函数/类型。不要为只读修复新增公共成功构造器或另一条writer，不改原过期attempt恢复算法。
- 父线维护本计划、SDD证据和中文docs；没有新模块注册需求时不改mod.rs。当前原始BASE没有未提交改动。

### 必须成立的行为

1. 首先通过真实Test命名空间coordinator完成prepare→begin_attempt→租约到期→reconcile→terminal inspector，取得“已封口Uncertain却因零authoritative sink result读取失败”的真实行为RED。不要手工插入成功终态或用编译失败充当RED。
2. 在同一个既有terminal验证接点区分sink权威结果与恢复分类。原sink路径保持原严格校验；不能catch sink校验错误后就尝试更宽松的recovery。恢复路径须证明对应decision/current attempt/state、原fence与当前撤销后generation、真实到期/撤销时间，以及只有这一组准确的恢复事件。
3. 重验FenceRevoked与RecoveryClassified原始canonical/hash/稳定事件身份、相互hash引用、真实attempt/decision归属及关联audit身份/已封口证据；验证classification=Uncertain、automatic_resend=false、persisted_receipt=false。时间须来自持久记录且与lease/revocation/disposition一致，不能使用本机now填充。缺失/重复/错绑定/改值即拒绝。
4. 复用原`validate_current_disposition_canonical`核对既有disposition原字节/hash/引用、当前decision/envelope/attempt、时间与fence evidence hash；返回实际已持久化的证据字节及对应hash，不能把fence hash说成sink-result hash，不能新合成伪sink payload或改写磁盘协议。返回格式须兼容现有Generic/P01共同terminal验证。
5. 恢复后合法迟到且非authoritative的sink结果不得把恢复状态变成Accepted，也不能被当作所缺的原sink权威证据；原Uncertain仍待人工处理。存在冲突的authoritative结果或被破坏的恢复链时拒绝，不能“无sink行所以成功”。
6. 真实Generic/P01终态inspector都覆盖；至少一条跨实际Foundation route→SLA/metrics的回归证明两类来源的恢复Uncertain被计入正确disposition而非SourceInvalid，没有Accepted样本、完成推进或新增发送。业务若已ResolutionRequired，status可为ResolutionRequired而disposition仍Uncertain，不能写死错误状态预期。
7. 重启/重复读取保持相同证据与未决状态；终态读取不写决策、审计、发送或游标。原PendingSeal与现有sink/Rejected/Accepted/人工终态防篡改回归不削弱。输出错误使用有限固定说明，不打印原始payload/账户/凭据。

### 垂直实施与验证

- 先只交付一个最小Generic真实恢复→读取反例，冻结给父线运行实际RED；取得行为失败后实施最小修复并运行同一GREEN。再按新发现补P01、来源篡改/冲突/迟到、重启及SLA/metrics消费回归；有旧代码可观察行为问题时按RED→GREEN，不为新私有函数不存在制造伪RED。
- 现有durable `Fixture` 在隔离树`data/test/TEST_CODE_BR192_<label>_<pid>_<sequence>`建库并按原inode回收自己拥有对象；它只stat隔离树既有生产命名路径以证明未改动，不读取其数据库内容。`Case`使用`data/test/TEST_CODE_W19_SLA_*`临时双库和受限测试审计。沿用这些真实测试seam，不用全局DB初始化、固定test.db或真实外部连接。
- 新源层测试统一`durable_delivery::tests::w19_recovered_uncertain_`前缀；新增消费者测试使用对应模块下`recovered_uncertain_`前缀。首条命令为`env CARGO_PROFILE_TEST_INCREMENTAL=true cargo test --lib -- --exact durable_delivery::tests::w19_recovered_uncertain_terminal_is_readable_after_expiry --test-threads=1`，实际命名一致后执行，0tests不能算通过。
- 最终父线合批安全新前缀＋既有`durable_delivery::tests::w12_terminal_`/完整SLA/metrics模块与已核对过期/作用域恢复邻域；先核对新增测试副作用再执行，不直接跑整个durable/全仓测试。保存唯一session，超时只轮询原session，不重启。
- 最后`cargo clippy --lib --no-deps --message-format=json`、改动文件`rustfmt --check --edition 2021 --config skip_children=true`、`git diff --check`及`ruby scripts/architecture-docs/check-rfc-inputs.rb --root .`。警告分既有与新增，不加allow掩盖。固定BASE..SOURCE独立Spec/Quality审查；若有修复，只复查FIX_BASE..fix。

### 交付与完整剩余范围

代理在同basename SDD的task-1-report.md记录完整实际数据流、来源分流规则、校验字段、正/反例及未执行项。父线记录RED/GREEN和最终原始验证/独立审查、中文docs及提交。本Task不具备完整告警/解决SLA所需的告警、可信severity或eligible-session证据，不认定全W19完成。完整W15/W16/W17生产接线、W18/W20/W21、全部52Unit、HTML/checker/CI及真实上线门禁继续保留；不操作生产monitor。
