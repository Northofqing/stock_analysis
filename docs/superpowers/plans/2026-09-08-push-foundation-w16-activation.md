# W16 activation 完整实施计划

日期：2026-09-08。状态：待实施，关键外部选择 needs-context。设计：[W16 activation 设计](../specs/2026-09-08-push-foundation-w16-activation-design.md)。本文件不是执行记录。

## 范围与执行约定

工作目录固定 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`。WBS W16 正式依赖 W06/W08/W11/W12，不依赖 W15 Ready。交付授权/部署 facts、store/CAS/配额、共同 fence、真实 supervisor adapter、操作员控制、非原子恢复/rollback，以及 W15 全 Unit 消费合同。各 Unit cutover 和生产批准不在此计划执行权限中。

使用 brainstorming 决策、writing-plans 依赖排序、codebase-design 的 deep module/interface 设计。复用已闭合 W15 reader/codec/store 证据，不重开其审计。所有下列新 Rust 路径明确标“新建”，当前没有 activation 运行时；文档中的方法名为拟定 interface。实现时用 apply_patch，保护并发工作；同一文件仅交当前任务 owner 编辑，跨任务改动串行集成。不开启生产发送来证明框架。

依赖顺序：T0 冻结必要合同；T1 完整只读事实可独立完成；T2 认证部署使 T1 输出成为可认证来源；T3 完成事务写入引擎；T4 完成跨进程共同 fence 与 paused owner；T5 把 T2/T3/T4 合成真实 apply/reconcile/rollback；T6 在 T1/T2 的基础并行推进 W15 全范围消费，并与 T5 联调；T7 收口监控/CLI 与验收。T3 可先使用测试身份，但不得公开“已认证”构造器。T4/T5 未完成时 T1–T3 只能称基础切片。

## T0 — 冻结认证、owner 投影和版本化合同

结果：不写实现也能明确正确执行者、数据语义和验收预期。主控读取设计 needs-context A–E 并记录结论；开发授权不需要再次批准，但真实平台/身份发行方、DualControl 外部策略不能靠程序猜测。

文件 ownership：本设计及本计划；后续实施时仅按主控裁决编辑 `docs/push-system/push-system-implementation-rfc.md` 和 `docs/Project_Architecture_Blueprint.md` 的相关段，不改冻结 SQL。先在冻结 Task2 输入中定位 canonical manifest/journal domain；不存在则冻结 domain + golden vectors。Shadow `physical_owner=None` 与 legacy actor 匹配 manifest owner 的 common_fence 存在未闭合矛盾：另存 owner 登记不足以解开等式。须同时满足蓝图 1502 默认 owner 不变、1537/1568 唯一 live owner，明确 Shadow/legacy 真实授权合同后才能交付联合许可；不能自行更改 RFC。另决定 paused owner 是否足够构成已切换事实、外部持久批准包与同事务写入含义、无 owner 变化 release 如何合法表达。

开放选择：推荐平台受保护 supervisor + 强制 PAM/服务身份 allowlist（设计 A）；推荐外部批准包、事务内 paused 安装/journal、提交后重查开门（C，尚未获得生产批准）；推荐新增 W15 集合 snapshot/material v3 与独立 deployment-set/v1（D），保留当前 595f605 的 v2 候选语义及 legacy v1 拒绝策略；推荐 DualControl preparer≠approver 且 approver 执行（E）。若外部策略不同，标 needs-context 并只暂停相关 adapter，继续纯验证/隔离任务。不得默认“Verified struct 存在”已解决这些选择。

只读核验命令：

```bash
jq '.foundation_work_packages[] | select(.id=="W16")' docs/push-system/push-system-wbs.v1.json
rg -n 'PromotionV1|canonical|Shadow|quota_transaction|Codex|DualControl' docs/push-system/push-system-implementation-rfc.md
rg -n 'push_activation_manifests|push_promotion_journal' docs/push-system/push-system-foundation.v1.sql
```

验收：上述每个选择有“已确定/needs-context + 证据 + 影响任务”，没有假设外部根已配置。反例：允许 env 中非空 operator 成为 approver、为 Shadow 改 SQL 允许 NULL、先插成功 journal 后切 owner，均应否决。

## T1 — 完整 manifest/journal 只读 inspector

依赖 T0 中编码/owner 语义；不等 W15 Ready 或生产配置。新建 `src/push_foundation/activation.rs`、`activation_facts.rs`、`activation_codec.rs`、`activation_store.rs`、`activation_facts_tests.rs`；编辑 `src/push_foundation/mod.rs` 声明（本任务唯一 owner）。复用 `monitor::push_job` canonical 与 MachineCatalog；调用既有 `with_rollback_read_only` 和内嵌 Foundation schema 验证，必要时只调整可见性，不重写 VFS 锁算法。

通过一个 inspector interface 加载全部 Unit 的完整链及指定 Unit 的期望/已执行差异，逐列类型检查、canonical SHA、`PromotionV1` 身份、前驱、所有版本/FK 等式、action/reason/窗口、rollback 目标、不可变 schema/triggers。产生 `RawActivationFacts` 一类明确非认证输出。缺 journal 返回待协调；缺 Unit 行不自动填 Disabled；错误/损坏不返回空集合。读路径不打开写连接、不迁移数据库。

测试在 `tempfile` Test namespace 创建冻结 DDL 合成数据库；fixture hash 使用独立固定 golden bytes，不能测试期待值复用被测 codec。覆盖每个 manifest/journal 字段改变、NUL/非法 SQLite 类型、同名 schema/trigger 替换、断链/重复/跳代、错误 actor/窗口/reason、跨 Unit rollback、自洽克隆源库仅能得 raw facts。WAL/hot journal/不存在路径沿已知 reader 合同拒绝，不能宣称新增支持。

命令与可观察结果：

```bash
cargo test --lib push_foundation::activation_facts_tests -- --test-threads=1
```

必须打印实际测试数量大于零且通过；精确确认 inspector 前后 DB bytes/sidecar 不变、零副作用计数。T1 完成报告明确“不含认证、写入、配额、owner 接管”。

## T2 — 认证部署 inspector 与操作批准

依赖 T1 与 needs-context A/E 已决部分。新建 `activation_authorization.rs`、`activation_deployment.rs`、`activation_authorization_tests.rs`、`activation_deployment_tests.rs`；声明变更由本任务串行接管 `mod.rs`。`src/auth/operator.rs` 只在确定平台后增加能返回真实认证主体的专用路径，保留现有 monitor auth 行为；W16 不使用可跳过的 `Result<()>` 作证明。平台信任配置文件的实际路径在 T0 确认后登记，不能预造仓库内 production allowlist 为权威。

实现认证主体/权限/批准证据验证及精确 request binding；部署 inspector 验证实际 binary、Git/build/schema/catalog/template/source package、host/service instance、namespace、目录 owner/权限与撤销/过期。认证结果只能由生产验证流程构造，测试创建接口 `cfg(test)`；调用者不能注入任意 verifier、URI/hash、成功 bool 或 root 自报 manifest。闭集来源规则沿 W15 接线提案，不替 source owner 补历史 acquisition binding。

日历 adapter 使用 catalog CalendarId 和版本化已验证数据产生业务日 UTC `[start,end)`，校验请求时间、执行时间及批准窗口；不能用 `today_is_trading_day`、receipt 或当前本地日期代替。source 默认 WAL 的兼容差异保持显式不就绪，除非来源负责的 WAL reader/认证导出已交付。

验证：伪主体、PAM 关闭、wrong host/service、可写 allowlist、过期/撤销批准、换 command/Unit/generation/evidence、自洽克隆 DB、路径替换、source package/hash/namespace 错配、DualControl 同身份、未授 rollback 权限均拒绝；正例经隔离平台 adapter 的真实验证流程，不能只用可自由构造的 Verified 类型。平台测试使用临时身份材料与模拟服务端，不向真实 PAM 输入或读取凭据。

```bash
cargo test --lib push_foundation::activation_authorization_tests -- --test-threads=1
cargo test --lib push_foundation::activation_deployment_tests -- --test-threads=1
```

验收：`inspect` 已认证期望/执行/实物三者并清楚返回不一致；未开 owner 或发消息。外部配置未定则明确 T2 生产 adapter needs-context，其他子项可交付。

## T3 — 同事务 CAS、跨 Unit 配额与 append-only 写入

依赖 T1、T2 的批准与日历合同；owner 接口可先由测试 harness 实现。本任务接管 `activation_store.rs`，新建 `activation_transaction_tests.rs`。不编辑冻结 Foundation SQL，不复用 rollback-only reader 做写入。写连接对已认证 activation DB 打开并验证 schema，显式 `BEGIN IMMEDIATE`，重验 expected generation、前驱与 pending 代，再查询全 Unit journal、写 manifest/journal、提交。

writer 内部事务 interface 允许 T5 在 manifest INSERT 与 journal INSERT 之间执行有界 paused-owner 确认，但不暴露任意 SQL/不让外部 caller 自行漏掉 quota。正常 owner-changing promote 查询全部 Unit 的 `Activate/Rollback`，按认证 authority business-date UTC 半开区间；rollback 无名额限制但同事务写新代并阻断当日后续 promote。shadow/无 owner 变化动作不消耗名额，但不能制造冻结状态机不允许的同态边。对同一命令结果精确比对事件和请求，提交确认丢失只能重查。

并发反例：两个独立连接/进程同时对同一 Unit expected generation；两个不同 Unit 抢同一业务日；rollback 与 promote 抢锁；进事务前通过、拿锁后过期；业务日区间端点/跨日；过去日 receipt；一 Unit pending manifest；重复 command 改字段；事务中任一步失败。预期最多一个正常晋级成功、无半条历史、失败时无越权 owner 调用。数据库锁超时不能退化为无事务路径。

```bash
cargo test --lib push_foundation::activation_transaction_tests -- --test-threads=1
```

验收同时记录独立进程竞争案例，不能仅单线程顺序测试或 `Mutex` 下验证。写入后调用 T1 inspector 独立重算完整链。

## T4 — 四类 actor 的共同 fence 与真实 owner adapter

依赖 T2 认证部署，T0/C supervisor 平台选择；Shadow/legacy 联合许可另依赖 T0/B owner 等式矛盾的裁决。新建 `activation_owner.rs`、`activation_fence.rs`、`activation_fence_tests.rs`；新建 `src/bin/monitor/activation_runtime.rs` 和其本地测试 module（真实路径均为新建）。编辑已有 `src/bin/monitor/main.rs` 注册受监督生命周期；接管 `phase_scheduler.rs`、`generic_transport.rs`、`dedicated_transport.rs`、`business_finalizer.rs`、`reconciler.rs` 的共同执行 seam。`intent_store.rs` 仅在使当前执行许可覆盖业务事务确有必要时修改，保留 lease/version 原义。common module 的公开可见性变更串行交接 `mod.rs`。

实现完整 `(unit,generation,manifest,owner)` 当前检查与撤销共享的执行许可；跨进程的 quiesce/inspect/install-paused/resume 由认证 supervisor 驱动。旧进程确认死亡/撤权、在途许可结束后才能切换；进程身份需防 PID 复用。未适配旧 binary 不允许混跑。日志、旧 token、重启新 run_id 不可授予权限。

四类 actor 均放在实际副作用前，不能只在 scheduler 入口检查：scheduler 创建 occurrence；producer 外部采集/prepare/intent；dispatcher 真实 transport；finalizer business completion/cursor。保留 recovery-only capability：正式 Draining/Disabled 转换关闭该 Unit 新工作，原 pending 查询/finalize/reconcile/quarantine 继续；shadow 执行路径拒绝 provider 重取、LLM、业务/durable 写、sink/order/cursor。Foundation 默认 Disabled 只关闭新框架路径，不得改变原 physical owner；其 legacy 当前 fence 和 Shadow live owner 的共同授权由 T0/B 明确，未决时仅交付拒绝/隔离实现，不宣称完整联合许可通过。

每个 legacy/new actor × 四种状态 × stale/current fence 是行为测试矩阵；使 actor 在检查后、动作前挂起，另一进程请求切换，验证切换必须等待或拒绝旧动作，不能发生 TOCTOU 双 owner。进程 crash、signal acknowledgement 丢失、owner 查询超时、旧 binary 重启均保持 gate 关闭。测试本地子进程、临时 IPC/DB、计数拒绝 sink，不运行真实 monitor。

```bash
cargo test --lib push_foundation::activation_fence_tests -- --test-threads=1
cargo test --bin monitor activation_runtime_tests -- --test-threads=1
```

验收：真实平台 adapter 在隔离 harness 可运行，只有 trait/fake 不算完成。catalog 全 52 Unit 的 actor 映射有覆盖状态；W16 共用执行 seam 全接线，具体 Unit 未迁移路径显式拒绝晋级并交其后续 cutover，不把一条 demo 当全局证明。

## T5 — apply、非原子中断协调与 rollback 纵向闭环

依赖 T2/T3/T4。新建 `activation_execution.rs`、`activation_execution_tests.rs`；串行接管 `activation.rs` 对外 interface、`activation_store.rs` 的事务组合及 `activation_runtime.rs` 的 owner 安装/重查。独立控制面操作存储 adapter 实际位置随 T0/C 确定；不得向冻结 promotion journal 写 Pending/失败伪事件。

实现“外部批准包→准备/旧 owner quiesce→BEGIN IMMEDIATE 重验配额/CAS→manifest INSERT→目标 owner paused 安装与确认→journal INSERT/commit→独立重查→允许当前 fence”。所有异常先阻断就绪；持久控制面记录精确 command/实际 owner 协调线索，但日志不是激活权威。旧 actor 已停、事务失败时恢复旧 owner 也必须重新认证/重查，禁止自动假定 rollback 成功。

在设计故障表每一行及 manifest/journal/commit 前后注入中断；重启用新的 store/进程实例重查。验证 commit ack 丢失不会二次推进 generation；manifest 孤代必须协调而不能跳代；外部 owner 已变且事务回滚仍返回不一致；journal 存在但实际 owner 未匹配不返回 Applied/Ready。

rollback 测试同 Unit 兼容旧目标、N+1 generation、实际 owner 恢复、原 AcceptedPending/Uncertain/ResolutionRequired 和 exact bytes 不变；拒绝跨 Unit、无 N/N−1 兼容证明、破坏性 schema 降级、删除 pending。N−1 不实现共同 fence 的测试进程必须由已验证 supervisor 保持暂停/拒绝启动，不得仅因 DB 代更新就当它已受控；rollback 的当日 journal 阻断随后 promote。Disabled/Draining 恢复无新增发生工作；外部 Accepted 不撤销，Uncertain 无第二条发送。

```bash
cargo test --lib push_foundation::activation_execution_tests -- --test-threads=1
```

验收：对所有中断点可精确分类持久/实际 owner 状态并安全重查；完成协调前没有 current authority。测试“进程结束”而非仅 Result::Err 返回，确认控制面恢复线索跨重启存在。

## T6 — W15 全 Unit 部署集合与只读消费接线

依赖 T1/T2，新增集合 domain 文档选择 T0/D；执行联调再依赖 T5。新建 `src/push_foundation/activation_readiness.rs`、`activation_readiness_tests.rs`；串行接管 `readiness_snapshot.rs`、`readiness_snapshot_codec.rs`、`readiness_recovery.rs`、`readiness_recovery_codec.rs`、`readiness_store.rs`、`readiness_probe.rs`、`operational_readiness.rs` 及相关既有 tests。基线 595f605 已使用 snapshot/material v2；保留该版本语义，新增集合建议使用 snapshot/material v3 及独立 deployment-set/v1。冻结 recovery event/schema 和 Foundation SQL 保持不变；只有实际不兼容证明及专项审查才能提出额外版本修订，不重写已闭合锁算法。

按设计完整部署集合绑定 Unit 逐代 manifest/journal/owner、build/source package、启用 producer 配置、共享 namespace/catalog/calendar 与恢复责任。集合 canonical 使用新 snapshot/material domain（建议 v3）；stream identity 使用新 domain/version 并绑定集合 hash。snapshot/material、recovery record/stream 按明确版本 dispatch：当前 v2 候选 bytes/hash/字段保持原语义、不自动认证全范围；legacy snapshot v1 继续拒绝，未知版本拒绝，不恢复旧 reader、不让 v3 内容进入 v2 domain。现有 recovery event 保留原 domain/schema，仅在引用解析处显式分派 snapshot/material 版本；跨 stream/domain 的恢复衔接须显式定义，不能静默续链。全 Core 精确覆盖所有启用 Unit，缺某个代/owner/journal 则失败；单 producer/occurrence 仍精确绑定所属 Unit。

source/deployment 认证按接线提案的闭集 source contract 与真实 binding 消费，不批量 NotRequired，不追认历史 audit。BR159 默认 WAL 与当前 reader 不兼容是来源任务待交付事实；可用合成且认证测试来源联调，但不能因此宣布生产 source 已准备。W16 inspector 读取不要求 W15 Ready；T5 切换完成后的新集合再触发 W15 评估，promotion 门禁只使用该次合法范围/上下文的证据。

测试至少两个 Unit 不同 generation/build、全 52 登记状态闭合、遗漏/重复/额外 Unit、配置启用集删减、恢复 pending 的 Disabled Unit、单 Unit 污染 Core、只取最大代、请求换 manifest、读取期间/存储提交期间/查询时某 Unit generation 漂移、跨库 binding 中断。新集合版（建议 v3）拒绝无效认证；现有 v2 golden bytes/hash/解码语义保持不变，legacy v1 fixture 仍拒绝；错 domain/schema_version、v2 包裹集合内容、未知 recovery/stream 版本和隐式跨 stream 续链均拒绝。冻结 event/schema 不因该任务自动升级。

```bash
cargo test --lib push_foundation::activation_readiness_tests -- --test-threads=1
cargo test --lib push_foundation::readiness_snapshot_codec_tests -- --test-threads=1
cargo test --lib push_foundation::readiness_recovery_codec_tests -- --test-threads=1
cargo test --lib push_foundation::readiness_probe_tests -- --test-threads=1
```

验收：health/readiness/CLI 指向同一新集合版（建议 v3）snapshot，公开完整集合 hash/逐 Unit 代，不输出伪 scalar Core generation。上述 codec/probe 定向命令同时验证现有 v2 候选兼容、v1 拒绝、新集合版本和 recovery/stream 显式分派；记录各版本测试数与结果。跨库非原子读取通过版本重查拒绝漂移，不宣称全库原子。

## T7 — 操作员 wire、默认关闭启动与整体验收

依赖 T5/T6；接管 `activation.rs`，新建 `activation_operator.rs`、`activation_operator_tests.rs`；monitor 入口由本任务接管 `src/bin/monitor/main.rs`、`activation_runtime.rs`。若独立 binary 比 monitor 子命令更适配，需明确新建 `src/bin/push_activation.rs`、登记 `Cargo.toml` 并在文档固定命令；默认推荐 monitor 中只读/准备入口与授权执行 interface，避免预先假造已存在命令。

实现 RFC request/response 全字段和 canonical hash；inspect/dry-run/refusal affected_rows=0、mutation ref=NULL，仅返回独立 control-plane audit envelope。apply 拒绝未经批准的直接 SQLite 修改路径。promote 验证六新鲜门禁、wave rank、线上批准、每日限额与阻断条件，rollback 验证专属权限/兼容矩阵；未知/缺依赖不假装成功。其他 intent 命令只委派已认证 authority，未接线则稳定拒绝。

按蓝图启动顺序把 activation 校验放在 producer/scheduler 开始前，durable startup reconciliation 完成后再 W15 capability readiness。启动失败非零；运行中 Core 失败停止新工作、保留恢复/隔离后受控非零；ProducerUnready 只隔离对应 producer、部署门禁失败；BlockedOnInput 遵守 RFC 不自动使全局失败。既有默认 Disabled 和零物理 owner 变化必须保持可验证，不能因 catalog/manifest 存在自动开启。

```bash
cargo test --lib push_foundation::activation_operator_tests -- --test-threads=1
cargo test --bin monitor activation_runtime_tests -- --test-threads=1
cargo test --lib push_foundation:: -- --test-threads=1
rustfmt --edition 2021 --check --config skip_children=true src/push_foundation/activation.rs src/push_foundation/activation_facts.rs src/push_foundation/activation_codec.rs src/push_foundation/activation_store.rs src/push_foundation/activation_facts_tests.rs src/push_foundation/activation_authorization.rs src/push_foundation/activation_deployment.rs src/push_foundation/activation_authorization_tests.rs src/push_foundation/activation_deployment_tests.rs src/push_foundation/activation_transaction_tests.rs src/push_foundation/activation_owner.rs src/push_foundation/activation_fence.rs src/push_foundation/activation_fence_tests.rs src/push_foundation/activation_execution.rs src/push_foundation/activation_execution_tests.rs src/push_foundation/activation_readiness.rs src/push_foundation/activation_readiness_tests.rs src/push_foundation/activation_operator.rs src/push_foundation/activation_operator_tests.rs src/bin/monitor/activation_runtime.rs
cargo clippy --lib --bin monitor --no-deps
```

执行前逐个检查新增测试只使用 tempfile/Test namespace/拒绝外部效果 adapter；所有测试命令必须报告非零测试数。格式检查仅覆盖上面明确的新文件；实际修改的既有文件按精确路径补充定向 rustfmt，复用主控已有格式基线记录，避免全仓无关格式噪声。只安排上面一次目标 Clippy，复用主控已记录 warning 基线，核对本次相关增量，不要求修复无关历史告警。必要编译验证由以上测试覆盖，不额外运行真实 monitor、生产 DB 或传输命令。若已有 broad suite 有非隔离案例，仅运行可证明隔离的相关模块并明确未运行范围；不能打开真实网络来令测试变绿。

文档 ownership：更新 RFC、蓝图 §24.15/owner 章节、W16 完成证据及 W15 缺口状态；只在真实满足后更新 WBS 实施状态，不改正式依赖。提交可核验的零 provider/LLM/sink/order/真实 DB 计数、崩溃/竞争测试结果和仍 needs-context 项。完整 W16 工程实现与真实生产配置/Unit Production Verified 分别报告。

## 规则到任务追踪

| 权威规则 | 实现任务 | 核心失败反例 |
| --- | --- | --- |
| WBS W16 依赖 W06/W08/W11/W12；完整而非 facts 切片 | T0–T7 | 把 T1 通过标 W16 完成；依赖 W15 Ready 造成循环 |
| RFC 610–648 canonical/全部版本/actor 外部认证 | T0/T1/T2 | 非空 hash/actor、自洽克隆库、遗漏持久字段 |
| RFC 753–762 六合法边、新 generation rollback | T1/T3/T5 | 跳代、同态边、改历史、跨 Unit 目标 |
| 冻结两表与 triggers：逐代唯一、FK/action/reason/时间/不可变 | T1/T3 | 缺 trigger、错前驱、缺 journal 假成功 |
| RFC 1057–1063 四类 actor common/current fence | T4/T5 | 检查后切换竞态、旧 binary/缓存重启逃逸 |
| Shadow 无 owner、默认 physical owner 不变、唯一 live owner、共同 fence | T0/B 裁决后 T4/T7 | None 无法匹配 legacy owner 却授予执行、shadow 重取/写库 |
| Draining/Disabled 保留恢复、原稳定 identity | T4/T5 | 停止 finalizer、丢 pending、Uncertain 盲重发 |
| RFC 1064–1080 全 Unit 日额、BEGIN IMMEDIATE、日历 UTC 区间 | T2/T3/T5 | 不同 Unit 并发同日 promote、rollback 后再 promote |
| RFC 637–643 非原子切换/确认丢失重查/禁止跳代 | T0/T5 | 预写成功 journal、未知 owner 仍 Ready |
| RFC 993–1048 全范围同源 snapshot/query/recovery | T6/T7 | scalar 最大代/单 Unit 冒充 Core、任一 Unit 漂移 |
| 蓝图 §24.15 受控重启、启动先 activation | T4/T5/T7 | 热加载自动批准、生产者先启动后校验 |
| RFC 1130 起操作员 wire/认证/Single/DualControl/Codex | T0/T2/T7 | dry-run 写 DB、自由文本身份、Codex approve/apply |
| RFC 1225 起六门禁/波次/新鲜绑定/阻断条件 | T2/T7 | 复用旧 build 或另一 Unit 绿灯、未裁定 Uncertain 晋级 |
| W15 接线提案：真实 source binding、WAL、全 Unit 缺口 | T2/T6 | 补推历史 audit、任意 URI/SHA、缺 WAL 事实 |

## 完成声明边界

验收证据必须同时覆盖完整读写、真实选定平台的认证与 owner adapter、共同 fence、事务配额、非原子中断和 rollback、新集合版（建议 v3）全 Unit 消费以及 operator wire；保持现有 v2 候选语义、legacy v1 拒绝和冻结 event/schema，验证 recovery/stream 版本分派。测试成功不授予生产批准；外部信任根/配置未配置可报告“工程实现通过，生产认证未就绪”，但 Shadow/legacy 等式未裁决、平台 adapter 尚未实现、只读切片或全 Unit 接线缺失时不能报告完整 W16 已实现。计划中的生产权限边界来源为 RFC 操作员合同，不是重复请求已有开发授权。
