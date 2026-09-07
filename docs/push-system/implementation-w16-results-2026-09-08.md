# W16 激活与所有权实施记录

日期：2026-09-08。完整 W16 未完成；此记录区分规范纠错、完整性读取、实际认证与生产接管，不以局部绿灯替代全目标。

## 已完成：Shadow 范围纠错

`e7bf3d5` 将 RFC 与机器校验器的 Shadow owner=None 限定到新 shadow actor，保留 Unit 实际 incumbent；初始 Disabled 与排空后 Disabled 的准入从不可变已执行历史和真实批准推导，不新增状态表，不复用旧 token。

依据及矩阵见 [合同裁决](activation-contract-decisions-2026-09-08.md)。原 Q13/Q17 和八份输入字节保留；原始准入投影见后文，真实身份及执行许可仍需实现。

- 公共 CLI 回归先失败再通过：1 run / 11 assertions，全绿，接受正确 scope 后突变回旧 whole-Unit scope 并验证拒绝。
- `ruby scripts/architecture-docs/test/rfc_spec_test.rb`：session79925，exit0，296 runs / 3884 assertions，0 failures / errors / skips，165.568s。
- 实际输入哈希门禁 `check-rfc-inputs.rb --root .` 与 `check-rfc.rb --root . --draft` 均通过；不是 strict 发布通过。
- 独立限定 Spec/quality 审查 Approved，Critical/Important/Minor 均无。

## 已完成限定实现：全 Unit 激活事实读取

T1 实现通过自有安全只读事务读取完整 manifest/journal、校验内嵌 schema、全部原始字段、内容/稳定身份 SHA、连续代际链、合法边与完整关联。公开入口自行加载 bundled catalog，先核验全库再选择 Unit；未登记、journal 跟齐、一个未执行末代分别表达，缺行不默认为 Disabled。

内容 domain 为 ActivationManifestV1 / PromotionJournalV1，稳定身份沿用 PromotionV1；历史版本字段保留原值，不假装与当前部署已认证相等。RawActivationFacts 不是部署认证、配额许可、当前 owner fence 或 W15 Ready。

首轮 session44265：编译2m20s，12项为10 passed / 2 failed，3.22s，43项既有 warning。实际失败是 hot journal 在 schema 阶段拒绝的分类与测试预期不同，以及造损坏 fixture 修改 sqlite_master 被当前 SQLite 拒绝。该轮没有执行成功的 hot-journal 目录字节断言，不能声称已证明该反例零写入。

原实现代理改用真实同名表重建，按实际错误传播验证 hot journal 的拒绝和目录字节；另外补充完整六代已执行循环/同 Unit 合法回滚正例及中间 predecessor 损坏反例。生产只读/schema 内核没有为测试放宽。

源码提交 `1c16380`，原始任务 BASE `f48ddfc`，五个新文件及 module 声明；中途 `e7bf3d5` 是独立已审查的文档/Ruby 任务，不混入 Rust 验收范围。

- `cargo test --lib push_foundation::activation_facts_tests -- --test-threads=1`，session57866，exit0，**14 passed/0 failed/0 ignored**，3.54s，编译2m24s。完整六代、合法回滚、实际中段断链、表替换和 hot-journal 目录字节断言均执行通过。
- `cargo test --lib push_foundation:: -- --test-threads=1`，session99791，exit0，**196 passed/0 failed/1 helper ignored**，23.32s，缓存编译1.75s。ignored 是由父 POSIX 锁回归显式运行的 helper，不是跳过业务验收；共享 schema/锁/store/recovery 相邻回归通过。
- `cargo clippy --lib --message-format=json`，session45449，exit0，1m22s。完整当前 fingerprint 经 jq 核对163项有位置既有 warning，Foundation/采集审计目标零诊断。上述 lib-test 仍报告43项既有 warning，不称全仓零告警。
- 六文件定向 `rustfmt --edition 2021 --check --config skip_children=true` 与 `git diff --check` 均通过。

限定独立 Spec/quality 审查 Approved，无 Critical/Important。T1 按完整性读取范围收口，不据此标 W16 或生产认证完成。审查和主控裁决保留两项证据限制：

1. 这里实际测量的是数据库/sidecar 字节和目录变化，没有 provider/sink 计数器。入口只接受 path/Unit，实际调用为 bundled catalog→安全读事务→schema/FK/原始行/全链校验；没有对应外部执行端口。不新增与调用链无关的计数器来假证零调用，完整W16/T7和W17的真实端口计数仍待实现。
2. 同名表替换 fixture 同时删除了三个触发器，因此该测试证明组合 schema 损坏被拒绝，不能单独证明由列类型替换触发拒绝。下次触碰该 fixture 时恢复原触发器以隔离这个反例；记为非阻塞测试改进，不抹去此限制。原内嵌schema精确比较实现未变，相邻共享schema回归已通过。

外部部署真实性、实际 owner 权限、生产source配置与整个W16仍未验证，是明确的后续范围。monitor/config/Cargo/冻结SQL及蓝图两份输入相对原始BASE无差异；未以静态检查推断生产运行健康。

## 新增限定实现：事务写入与准入历史投影

源码 `10f7e03`，原始 BASE `06a6633`。主控与一个实现 agent 分文件并行，完成内部事务引擎、原始准入投影及其接线；不是公开生产 writer、真实操作批准或整个 T3/T4 完成。

- `activation_transaction.rs::apply_activation_candidate` 持有同一 `BEGIN IMMEDIATE`，重验全 Unit 完整历史、未协调末代、generation 和候选全部持久字段；全 Unit 当日 `Activate/Rollback` 任一记录阻止新的普通 Activate。同 owner 文本不豁免 Activate，Rollback 不限额但留下阻止随后晋级的记录。
- 同事务依次写 manifest、调用内部 paused-owner 确认边界、写 journal，再重读全链、复核窗口/时间并提交。无效候选在 owner 调用前拒绝；持锁后和提交前检查过去/未来时间及回拨。真实认证/时钟/监督器实现仍未提供，测试边界不等于它们已交付。
- `activation_store.rs::inspect_activation_transaction` 复用同一读取、schema、字段和链校验，公开 rollback-only reader 不改为写入口。`AlreadyRecorded` 只比较两表全部字段；独立 command_id 和日历区间不在冻结两表中，T2/T5 仍必须验证外部持久批准包，不能以行相同冒充完整请求或 Ready。
- `activation_owner.rs::project_owner_admission` 区分初始 Disabled/Shadow 的待认证原批准范围、正式排空后的关闭状态和精确历史目标回滚；检查完整历史中的 owner 保留规则，保留目标路径上每次 rollback 的批准引用。当前元组使用新代，不返回旧 token；None、未登记和待协调不能授予权限。

第三次合批验证 session22963：`cargo test --lib push_foundation:: -- --test-threads=1` exit0，**221 passed / 0 failed / 2 helper ignored**，28.43s，compile2m27s、43项既有 warning。其中17项事务测试、8项准入测试与原196项均通过；两个helper由父测试实际执行，新三组真实进程竞争合计执行6次子helper，均通过。包括真实第二连接 SHARED 锁导致 COMMIT BUSY、无自动再次 owner 调用、关闭连接后公开 T1 读回无半条记录的反例。

首轮63433因新测试辅助函数被同名变量遮蔽产生3个E0618，0项测试执行；第二轮70996为219 passed/1 failed/2 helper ignored，失败是Pending先进入准入投影而被归类为InvalidHistory。原agent修正优先级，未放宽预期；第三轮才全绿。helper也改为显式ignore、父进程指定执行及有界启动等待，没有把无断言返回算作业务通过。

Clippy75692 exit0，1m24s；当前完整fingerprint为163项有位置既有warning，Foundation/采集审计目标零诊断。六精确Rust文件格式和diff检查通过；冻结SQL、八份输入、monitor/config/Cargo均未改。

独立范围审查要求补“真实等待写锁期间批准过期”的反例，同批完善进程锁竞争握手、同日额度已占用后的连续Rollback覆盖。修正提交 `5a78dd6` 只改测试文件，生产逻辑不变：

- 真实 SQLite BUSY 回调确认 contender 正在竞争写锁，随后推进受控测试时钟到批准窗口终点，释放锁后必须拒绝、只进行一次锁后检查、pause调用为0且历史为空。
- 等待方子进程通过实际 BUSY marker 握手；父进程观察后才释放Rollback，不再用100ms睡眠猜测已经发生竞争。
- 同日Activate先占额度，两次连续同日Rollback均成功，随后另一个Unit的Activate仍被拒绝。

session40241：`cargo test --lib push_foundation::activation_transaction_tests -- --test-threads=1` exit0，**18 passed / 0 failed / 1 helper ignored**，4.93s，compile2m26s、43项既有warning；三组进程的6次helper实际执行均通过。未改的生产/准入代码和原相邻回归沿用22963，未改lib的Clippy沿用75692，没有把两次不同范围命令冒充一次新的全仓测试。

限定复核 `10f7e03..5a78dd6`：三项全部ADDRESSED，新Critical/Important/Minor均无；最终 **Spec Approved / Code quality Approved**。批准范围仅内部事务引擎、原始准入投影与本批测试，不是整个T3/T4、真实身份或生产owner接管。

## 已完成限定实现：真实身份/制品观察与日历声明绑定

源码 `9722979`，原始 BASE `c1aee09`。身份适配 agent 与主控按文件并行，本批增加四个模块文件及一个窄日历读取接口，尚不提供生产认证或实际 owner 切换。

- `activation_authorization.rs::observe_unix_peer` 从实际 Unix 连接查询内核 UID/GID/PID，字段私有、不可克隆、绑定连接借用生命周期，Debug 隐去原身份。它不是 PAM 人类认证，也不证明监督器实例。
- 原始批准声明逐项比对 command、namespace、Unit、action、generation、目标 manifest、evidence、窗口、approval ID 与策略版本；检查角色/范围、撤销及独立 Rollback 权限。唯一可创建策略来自测试编译下的临时 listener，拒绝 Production；同 UID 的两个连接不能满足 DualControl。生产根未配置始终拒绝，原始声明相等不授权执行，approval ID 不证明跨重启防重放。
- `activation_deployment.rs` 保持同一个 File 描述符，用实际 metadata 和有界 bytes/hash 观察、重验文件；路径替换不会使它改读新路径，字节/权限变化会拒绝。原始 FD 不证明来自受保护根，尚不验证祖先目录/ACL 或运行中 binary；自洽克隆不能因此成为可信来源。
- 日历声明精确连接 namespace、catalog、全部52个已登记 Unit、CalendarId、不可变日历摘要与上海时区。从显式 UTC 微秒计算上海自然日 `[00:00,次日00:00)`；普通 Activate 休市拒绝，Rollback 使用覆盖年内的实际当日，不映射到下一交易日。覆盖年外、缺/重/额外 Unit、错日历/namespace、过期/回拨/跨日或锁后上下文改变均拒绝。
- `calendar.rs::verified_a_share_calendar_authority_hash` 仅增加不可变的覆盖日摘要读取，允许休市日取得同一 authority；原 replay 空日期范围仍拒绝，未改内嵌 CSV 或现有日期语义。catalog 尚无 CalendarId 字段，期望映射来自待认证的部署声明，代码未虚构默认批准映射。

首轮合批 `cargo test --lib push_foundation:: -- --test-threads=1`，session63832，exit0：**244 passed / 0 failed / 2 helper ignored**，28.34s，编译2m24s。包括新增22项（8身份/14部署日历）与原222项；两个helper由父测试实际执行。44项warning中43既有，一个为非Unix错误分支在Unix未使用；原agent改为一致平台cfg。

修正后session97782身份专项 **8/8**，0.01s、编译2m24s，仅43项既有warning；未改的部署和相邻代码复用63832结果。session92155运行不可变calendar后续交易日两项，**2/2**，0.00s、缓存1.72s。六文件定向格式检查通过。

Clippy session60834 exit0、1m22s；完整JSON流在pipefail下汇总，build-finished success=true，163项有位置既有warning，Foundation/calendar/采集审计目标零诊断。不是根据截断日志推断没有新增问题。

固定 `c1aee09..9722979` 六文件限定独立审查 **Spec compliant / Code quality Approved**，Critical/Important均无。非阻塞Minor：authorization中的 `std::fmt` 导入仅Unix使用，非Unix编译可能告警，随下次平台适配处理；本轮未执行非Unix构建。原始UID/FD/日历检查不是生产身份、受保护根、运行binary或防重放证明，完整T2仍未完成。

## 已完成限定实现：全 Unit 部署候选集合

源码 `8f1b4d2`、修正 `838a947` 和 `e0cdd0d` 提供内部 `ActivationDeploymentSet/v1`：复用T1全库读取，显式保留52Unit状态，绑定启用/恢复集合、来源声明、六项共享依赖和日历；未登记不等于Disabled，任一Pending拒绝构造，重读比较全集合而非最大代。集合是候选事实，不是认证或执行许可；v3 snapshot/material、stream v2消费仍待。

验证历程：83215在测试二元夹具解构处编译失败；修正后80948为253 passed/1测试位置断言失败/2父测试执行helpersignored。改按精确UnitId验证后20521专项10/10，4.08s。独立审查随后发现共享依赖使用enum顺序而非合同文本顺序，导致全Core6摘要不合规范；不能以此前测试绿灯替代合同检查。

- 88322真实全库读取顺序回归先得到1项行为失败（0.23s），随后按`dependency.kind.as_str()`排序；固定字节样本扩为六项字面顺序，不从被测encoder生成期望值。
- `cargo test --lib push_foundation::activation_readiness_tests -- --test-threads=1`，1223 exit0，**11 passed/0 failed/0 ignored**，5.27s，compile2m22s、43项既有warning。包含精确来源join、依赖ID/version/hash、依赖重排、日历ID、仅观察时间改变保持身份，以及未选中Unit Pending/代次漂移。
- 最后的owner/build_commit变体改为实际传入encoder，与独立字面样本逐字节比较。93935仅重跑完整golden测试名并指定`--exact`：**1/1**，0.00s，compile2m21s；其余10项和生产代码未变，不重复编译相邻模块来制造“新全量通过”。
- 最终lib Clippy19760 exit0，1m24s；完整JSON流成功，163项既有有位置warning、Foundation/calendar/采集审计目标零诊断。该轮生产排序已冻结，随后只改cfg(test)。定向格式及diff检查通过。
- 独立限定复核`8f1b4d2..e0cdd0d`：排序、六项golden、具体字段/非选中Unit/时间稳定性证据均ADDRESSED，无新增Critical/Important，无范围外遗留。本片最终Spec/quality通过，不代表完整T6或W16完成。

旧T1读取/schema及W15 v2/v1/recovery/store未改，80948中244项既有测试通过的证据保留；不是把不同范围运行合称一次新的全Foundation结果。来源声明真实性、生产owner和跨库原子性均未由此证明。

## 完整剩余范围

### T4 实际接线核查（证据基线 f87e2b8，尚无执行实现）

只读核查确认不能仅在表面入口增加一个检查。以下是后续必须覆盖的真实位置，未运行monitor或进程实验：

| 位置 | 现有行为 | 后续必须覆盖 |
| --- | --- | --- |
| `phase_scheduler.rs:554`、`intent_store.rs:1089` | 前者仅返回CreateExpected；后者才写初始intent | 实际采集/prepare至初始commit的许可，不将纯proposal当已登记 |
| `generic_transport.rs:204,217,221`、`durable_delivery/coordinator.rs:3835` | prepare→send→结果记录，随后还有全pending恢复 | 许可覆盖整个attempt寿命；跨Unit恢复逐作用范围验证，单Unit许可不能包全局扫表 |
| `dedicated_transport.rs:103`、`monitor/main.rs:4993,5483,7790` | 前者仅查terminal；后者存在实际补偿/调度/快讯发送入口 | 专用发送另行映射，不能把terminal查询测试当已覆盖专用物理发送 |
| `business_finalizer.rs:418,809,829`、`reconciler.rs:628` | 准备、冲突、错误和恢复租约均可能写库 | 每个写入分支都在当前许可内，嵌套恢复复用同一个scope，不只包成功完成分支 |
| `monitor/main.rs:4958,5829` | 启动恢复可发送；超时的review worker可继续执行 | 启动/手工入口先关门；跟踪真实effect/worker结束，外层abort/join不是排空证明 |

因此下一隔离broker应拥有typed effect及许可生命周期，客户端断连后仍保持在途状态；以实际Child/资源约束和稳定operation ID查回，重启默认关闭。旧binary可绕过broker直接访问sink/DB时仍不能批准接管，需真实监督器与权限撤销证据。这是下一实现要求，不是新增生产保护已生效。

- T2：本批仅完成真实内核/FD观察、批准声明约束和原始日历join；生产规范操作员、受保护根/opener/ACL、实际部署/source package、可信时钟和批准持久/撤销/防重放仍待，生产平台及根配置尚未给定。
- T3：内部同事务引擎已实现；真实认证 opener、可信业务日及外部批准包精确请求绑定、T5协调接线仍待。
- T4/T5：原始准入投影已实现；legacy/new 四类 actor 的共同当前 fence、真实监督器和旧 binary 撤权、批准范围认证、非原子切换/恢复/rollback仍待。
- T6/T7：全52Unit原始候选集合已实现；v3 snapshot/material、stream v2与恢复衔接、真实来源/部署认证、同快照查询/启动、操作员入口和完整门禁仍待。
- W15 真正来源上下文/认证/恢复及调度联结，W17–W21，52个 Unit 的纵向迁移与真实发布证据仍未完成。

开发和测试均隔离于 worktree/临时库；本轮没有启动、观察、替换生产 monitor，没有调用真实 provider/sink/PAM，也未进行 owner 晋级或真实库修改。
