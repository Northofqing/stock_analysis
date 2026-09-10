# W15 运行就绪实现记录（进行中）

## 1. 当前结论

W15 未完成，尚未接入生产。已交付候选判级、规范快照/重建、完整目录计数，以及通过独立审查的候选记录原子存储；真实来源认证与认证恢复、实际部署探针、W11/W14 联结仍未交付。生产 monitor 未启动、观察、替换或接线；开发限于 `codex/push-reliability-20260905` 隔离工作区。

完整目标仍为正式 WBS 的 W01--W21 与 52 个迁移单元。局部测试通过不能证明逐推送迁移、发送效果、当前生产健康或上线完成。

## 2. 已完成代码与证据

| 提交 | 行为 | 验证边界 |
| --- | --- | --- |
| bbe0ca6 | 缺少 OccurrenceInput 声明属于合同缺失，不误判临时输入阻断 | 纯判级回归 5/5 |
| fed906e | 显式不可用观察保留实际原因、来源版本和证据哈希 | 核心切片 6/6 |
| c2b7dc6 | 候选快照绑定完整上下文、正负依赖及证据，规范哈希不含自身 | snapshot 4 条测试 |
| c040072 | 独立统计 65 kind / 102 producer / 10 枚举外 producer，保留缺项/条件/回执强度 | inventory 7 条测试，独立 Spec/quality PASS |
| 1b30629 | 原因族必须匹配依赖角色，拒绝核心失败被错放进可部署的输入阻断 | 真实 RED 18 passed/1 failed；修复后 W15 23/23，独立复审 PASS |
| a1ca363 | 重读候选快照时重新判级和编码，拒绝重算哈希的伪 Ready、重复/额外字段及非规范输入 | codec 初始 4 条测试；两项覆盖缺口已由 6f255c0 补齐并复审 |
| 20b4409 | 独立 SQLite schema/open、无环事件身份、三类候选恢复与前后快照材料 | W15 37/37、Foundation 140/140；schema 两项审查问题进入修复 |
| 6f255c0 | 补齐错来源/错版本、14 种合法 unavailable 原因、30 项 typed mutation | W15 40/40（codec 7/7），独立 scoped 复审 Spec/quality PASS |
| bde6719 / 7617763 | 恢复事件按两端快照重新构造，核验全部派生内容与规范字节；补合法角色错绑反例 | event codec 4/4；Spec/quality 独立复审通过 |
| b6f5c99 | 原子认领初始化文件、核对路径身份；SQLite 查询前拒绝静态 WAL/未知/损坏文件头 | W15 47/47、Foundation 150/150；复审发现两项竞态未闭合，进入第二轮 |
| 64047dd | owned-file 镜像初始化、受控 SQLite 事务与实际句柄锁/头核验；候选 store 首条重启查询 | W15 53/53、Foundation 156/156；初始化 finding 关闭，模式稳定残留由下述第三轮关闭 |
| 81878ce | 原子回滚、历史重放、回复丢失后重查、显式恢复链、head 损坏拒绝、并发 CAS | store 六项实际通过；Task 3D 独立 Spec/quality PASS，一项中间链损坏覆盖 Minor 待补强 |
| 3fe1e62 | 移除 existing 主库普通文件头预读；独立进程锁竞争与严格子测试结果校验 | W15 59 passed/0 failed/1 ignored；schema 第三轮 Spec/quality PASS，两个原 Important 均关闭 |
| 1a4ee8e | 中间链损坏使 head/历史查询/追加全部拒绝且无文件副作用；清理两项新增 lint | Foundation 163 passed/0 failed/1 ignored（含 W15 60 项、store 7 项通过）；Minor 独立复审关闭，Clippy 目标零诊断 |

历史 `b6f5c99` 的完整 Foundation 命令：`cargo test --lib push_foundation:: -- --test-threads=1`，session 71160，exit 0，150 passed/0 failed，测试执行 9.25s。对应 W15 命令 `cargo test --lib w15_ -- --test-threads=1`，session 99470，exit 0，47 passed/0 failed，3.26s。编译为既有 43 条 warning，不是零告警；各轮结果只覆盖当时对应源码，最新结果见 §8。

相邻 `push_job` 命令 session 56855：52 passed/0 failed，1.76s。以上不是全仓测试、完整 W15 或生产验收。

独立复审确认 `OccurrenceInput + ActivationCoreUnready` 在判级前即被拒绝，合法 Core/Producer/Input 原因矩阵仍成立；修复范围未发现新增问题。

codec 原审查的两项测试缺口均已修复：错来源/错版本及合法 unavailable 原因的 roundtrip 有直接事实等值断言；namespace/build/generation 等 typed 输入故障矩阵已补齐。独立复审逐项确认关闭，无新增问题；这不证明外部认证或持久关联已经交付。

## 3. 剩余验收

| 任务 | 当前缺口 | 完成证据要求 |
| --- | --- | --- |
| Task 2 | expected authority、版本化 NotRequired 和 v2 候选已通过限定审查；持久 occurrence reader 已验收，但仍缺 source/deployment 绑定 | 实际 reader 校验与失败反例，不接受自由 bool；不能以候选或显式路径替代来源认证 |
| Task 3 | 候选 recovery/event codec、schema/open 与候选 store 均已通过限定范围独立评审及补强；真实来源认证与认证恢复仍缺 | 完成实际 evidence reader 与提交后认证重查才能发布权威快照；明确底层提交确认异常的处理，不以成功回复被丢弃替代全部 I/O 故障验收 |
| Task 4 | 当前只有纯计数，不是实际 probe | 同一已验证快照的 health/readiness/CLI；只读零副作用与损坏拒绝 |
| Task 5 | 只读接口审计完成，代码联结未完成 | 身份/版本/恢复来源精确绑定，输入阻断与有效窗口内恢复，过期/终态不重开 |
| Task 6 | 只有部分切片审查 | 完整 W15 双轴评审、目标与相邻回归、check/clippy/rustdoc、probe 与文档验证 |

W11 barrier 只证明固定点遍历完成，不含跨库身份。W14 当前没有 input block/recovered 提案入口；`ScheduleOccurrenceId` 与业务 `OccurrenceId` 不同，不能比较字符串后当作同一对象。

## 4. 下一步和并行边界

主代理统一维护接口、Cargo 调度、集成与文档；schema/open 原代理与主代理 store 开发已并行交付，随后由两名只读代理分别审查，未共同编辑文件。一个实现子代理已补齐链损坏测试和两项新 lint，主代理同步完成 docs 与验证。后续继续按独立文件分工，复用有效证据、仅复审改动及未关闭问题；编译测试保持一个队列。候选哈希、SQLite 连接或自由构造的恢复引用都不能升级为执行许可。

本轮采用 planning-with-files 保留可恢复记录；subagent-driven-development 将独立 schema 与核心事件材料分开，避免共享文件并发修改。拆分不减少完整 W15 的验收目标。

## 5. 20b4409 增量（不等于 Task 3 完成）

已新增独立 SQLite schema/open，以及精确绑定前后快照、三类恢复、来源声明和实际旧新版本的候选事件材料。真实反例先复现并修复了非法 hash 落库、sqliteX 未知对象漏检和 INSERT OR REPLACE 覆盖不可变记录。

- W15 session 11137：37 passed/0 failed，2.39s，43 warnings。
- Foundation session 10906：140 passed/0 failed，8.38s。
- `cargo clippy --lib --message-format=json` session 14122：exit 0，1m16s；对应制品诊断文件逐项核对为 163 条有位置 warning，与此前基线数量相同，Foundation/W15 目标文件零诊断。JSON 中另有一条“163 warnings emitted”汇总，不计为第 164 个 lint。未把非致命 Clippy 冒充 strict 全仓通过。
- `cargo test --doc` session 90104：16 passed/0 failed，4 ignored，3.30s。
- 相对 W15 起点 7608730，`src/bin/monitor`、`src/notification`、`config`、`Cargo.toml`、`Cargo.lock`、`migrations` 的源码 diff 为零；不以此推断当前生产健康。

该提交的独立审查发现两个问题：初始化检查后并发创建的空文件可被接管；READ_ONLY WAL 查询可能创建旁路文件（[SQLite 官方说明](https://www.sqlite.org/wal.html#read_only_databases)）。两项均由 b6f5c99 修复并进入复审，不按“测试已绿”放行。候选 recovery 行为通过审查，event_bytes 受保护字节说明的 Minor 已补。

codec 补测 session 65575 已结束并通过，见上表。完整事务/认证来源/只读权威 probe/W11-W14 联结仍未交付。

## 6. 恢复事件重读与 schema 修复（开发中）

恢复事件 codec 首次 RED session 2609 已 exit 101：E0583 缺少模块，随后补齐实现。联合 session 7624 已编译完成（2m04s、43 warnings），41 passed/2 failed，测试执行 2.62s：event codec 首个前后关联/重算 hash 篡改测试通过；两项失败恰为 schema 确定性竞争创建与 WAL 拒绝反例。该运行期 RED 已交给原 schema 代理修复，不将它描述为通过。

event codec 进一步覆盖三范围初始/持续/恢复记录、派生依赖差异、claims 错绑和时间、typed 输入及非规范字节；session 52197 exit 0，4/4、0.67s。复审要求补充语法合法但角色错误的 Manifest claim，7617763 已补，session 80960 exit 0，4/4、0.60s，独立 Spec/quality 复审通过。

schema 第一轮修复后 session 99470 W15 47/47，71160 Foundation 150/150；但 scoped 复审未关闭两项 Important：路径前后 inode 检查不能证明 SQLite 实际句柄打开的是初始化器持有的文件；检查头到第一次 SQLite 查询之间仍可切到 WAL。上述为静态交错分析，原测试未覆盖，已进入第二轮确定性反例与 interface 修复，不能把上一轮 GREEN 当闭合证据。event_bytes 文档 Minor 已关闭。

静态检查 session 73045：clippy exit 0、1m14s，完整制品核对 163 条有位置 warning、Foundation 目标零诊断；不是 strict 全仓零告警。rustdoc session 92421：16 passed/4 ignored、3.74s。完整 Task 3 仍需实际事务、重启查询与真实证据认证。

实际 reader 接线预检确认：现有 `DatabaseManager::init` 会设置 WAL 并运行迁移，不能为只读 probe 初始化全局数据库；retained readonly snapshot 的实现包含 checkpoint，不能只凭名称推断零文件写入。后续须在指定、已验证来源上接只读认证能力，不能用任意 URI 文件或自报 bool 填补 authority 缺口。

## 7. 第二轮实测修订（进行中）

新增竞态反例后 session 46660 为 W15 **48 passed/1 failed**，不是继续全绿。初始化 ABA 反例及 reader 模式竞争在实际链接的 Apple SQLite 3.51.0 上通过；只有 writer 拒绝断言失败，不能把两个静态风险都称为本机已复现。

最小诊断 session 89176：初始化 ABA 1/1，拒绝于 `enable_foreign_keys`，owned/competitor 文件均为 0 bytes。session 54797：模式竞争 0/1，reader 拒绝于 `schema_objects` 且目录字节不变；writer 返回成功且目录发生变化，确认存在实际缺口。错误分类定位了拒绝阶段，不声称取得底层 SQLite errno 或证明其他实现平台安全。

并行只读诊断还发现原 proposed raw lock guard 不能仅靠存活保证锁连续；首次 schema prepare 可能结束内部读事务。第二轮采用内存镜像写入 owned File、PRIVATECACHE/EXCLUSIVE locking mode 与受控事务，实际锁竞争和失败退出仍待验证。主代理并行新增首次 Pending 的存储/重启 tracer，完整原子存储仍在开发；无生产接线。

第二轮实现与存储首 tracer 已提交 `64047dd`。联合 `cargo test --lib w15_ -- --test-threads=1` session 77786 exit 0，**53 passed/0 failed**、3.86s；编译 2m18s、43 warnings。`cargo test --lib push_foundation:: -- --test-threads=1` session 44217 exit 0，**156 passed/0 failed**、9.96s。新锁竞争/释放、真实 hot-journal 拒绝、初始化 owned-file 和首条 Pending 的重启/head 查询均通过。schema 第二轮独立 scoped 复审进行中；存储后续事务故障、CAS/并发、确认丢失、损坏、真实来源认证仍未验收，不把该数字当完整 W15。

本次 Cargo.toml 仅给原 rusqlite 0.31 增加 `serialize` feature，Cargo.lock 未变；相对 W15 起点 `src/bin/monitor`、`src/notification`、`config`、`migrations` 仍无源码差异。没有查询生产运行情况，不据此推断现有推送健康。

独立复审 `b6f5c99..64047dd` 已结束：初始化所有权 finding 关闭；模式稳定 finding 仍有一项 Important 残留——普通文件描述符的关闭可能取消同进程其他 SQLite 连接的 POSIX 锁（[SQLite 官方 §2.2](https://www.sqlite.org/howtocorrupt.html#posix_advisory_locks_canceled_by_a_separate_thread_doing_close_)）。现有同进程竞争测试不足以证明跨进程保护，第三轮由原代理补独立新进程反例并移除既有库的普通文件头预读；这是静态残留，未声称在本机已复现。

存储故障 tracer：29554 因缺少测试故障入口得到编译期 RED；实现测试专用入口后 58569 exit 0，store **2/2**、0.61s。真实事务在 event、snapshot、head 三个写入点分别中断，均保留旧 head/历史、拒绝查询新记录，并能完整重试。提交后回复丢失、实际并发、恢复链和其他损坏覆盖仍在补齐，不把本结果称为完整 store 验收。

## 8. 并行收口与第三轮结果

`81878ce` 将 store 覆盖补到六项，联合 session 56464 为 **59 passed/0 failed/1 ignored**、10.68s。回复丢失覆盖是丢弃已成功提交的返回值，再重建 store 精确重查/重试；不是底层 COMMIT I/O 错误模拟。该运行还加入独立新进程锁竞争，但修复前也 GREEN，且初版 child 拒绝分类较宽；只能记为本机未复现，不能声称取得 POSIX 失锁 RED。

`3fe1e62` 结构性移除了 existing 主库普通文件预读/关闭，实际 SQLite 句柄的 `xRead` 成为唯一文件头读取路径；child 仅允许 BUSY/LOCKED 作为锁拒绝，父测试要求子进程确实运行一项测试并通过。session 31849 `cargo test --lib w15_ -- --test-threads=1` exit 0，**59 passed/0 failed/1 ignored**、11.26s；ignored 是由父测试显式启动的 helper，不是跳过该锁验收。WAL/畸形文件/部分 schema/hot-journal 零副作用拒绝均在本轮通过。

两条独立审查同时结束：schema `64047dd..3fe1e62` 第三轮确认残留 finding 已关闭，Spec/quality PASS、无新增问题；store `935e6f7..81878ce` Spec/quality PASS、无 Critical/Important。store 仅有中间链实际落库损坏覆盖的 Minor，当前补强不改认证边界。

最新静态检查 session 49886 `cargo clippy --lib --message-format=json` exit 0、1m25s，但并非零新增：完整 fingerprint 诊断为 **165** 条有位置 warning，其中目标新增 2 条（SQLite 手工 C 字符串、故障点枚举重复 After 前缀）。当前正在修正；此前 163 项基线不得冒充这版代码的结果。Foundation 最新完整 156/156 属于 `64047dd`，后续 store/第三轮修改需更新相邻回归证据。没有运行或观察生产 monitor，也未接线。

### 本批最终验证（代码 `1a4ee8e`）

上段 pending 项现已关闭：中间链损坏补强和两项 lint 修正由独立实现代理完成，限定复审 `3fe1e62..1a4ee8e` Spec/quality PASS，原 Minor ADDRESSED、无新增问题。测试先精确恢复不可变 trigger 并证明 schema 合法，再要求三种 store 操作返回事件 codec 错误；不是把打不开数据库当损坏检查通过。

- `cargo test --lib push_foundation:: -- --test-threads=1`，session 67731，exit 0：**163 passed/0 failed/1 ignored**，17.50s，编译 2m17s、43 项既有 lib-test warning。实际输出中 W15 60 项通过、store 7 项通过；ignored child helper 由父测试显式执行。
- `cargo clippy --lib --message-format=json`，session 83430，exit 0、1m22s。对应 `stock_analysis-5cb12cc6a07580ac/output-lib-stock_analysis` 完整诊断经 `jq` 核验 **163** 条有位置 warning、Foundation 目标诊断为空；两项新增告警已消除，仍不是 strict 全仓零告警。
- 相邻 `cargo test --lib monitor::push_job -- --test-threads=1`，session 51815，exit 0：**52 passed/0 failed**、1.77s，43 项既有 lib-test warning。
- 五个本批 Rust 文件使用 `rustfmt --edition 2021 --check` 通过，`git diff --check` 通过。相对 W15 起点，monitor/notification/config/Cargo.lock/migrations 源码 diff 仍为空；Cargo.toml 保留已验证的 rusqlite serialize feature。

本批仅完成候选持久化与连接安全切片；真实认证、权威查询、实际 probe/CLI 和 W11/W14 联结未交付。完整 W01--W21/52 Unit 目标继续保持进行中，没有把测试数量或 scoped 审查通过当整体交付。

## 9. 真实来源接线与提交确认异常（继续开发）

限定源码审计确认 BR-159 的真实 audit 没有 namespace、权威业务日和 SourceContractId/version；`provider_state_changed` 仅按 capability+provider 查询前态，不能独立签发某个 W15 scope 的恢复。MachineCatalog 只证明其实际登记的关系，RunContext 派生 ID 不证明 occurrence 已落库，终局投递 authority 也不证明当前输入能力。详细字段/函数证据留在本计划的 `task-3-authority-reader-findings.md`。

当前并行两条明确验收：Task 3E 在 COMMIT 确认异常后结束原写连接并完整重查同一候选，无法确认则返回明确未确认错误，不自动追加；Task 3F 在 database 模块复用 BR-159 原算法，从同一读事务取真实 audit/chain 并返回受保护原始字段，不接收调用方自报 observation，不创建数据库或初始化 Manager。3F 仍不证明 source identity、namespace/date/version 或外层事务提交状态，不能用于发布 W15 Ready；真实注册与来源绑定尚待接入。

首联合 RED session 68192：exit101，缺新reader函数 E0425 和确认丢失故障入口 E0599，两项均为编译期缺口。最小实现后 session 70694：**61 passed/1 failed/1 ignored**，11.06s，编译2m32s/43 warnings；审计原样只读查询与自动确认重查两个首tracer都通过，唯一失败是旧并发测试未允许新的 CommitUnconfirmed 错误分类。该断言已按合同调整并保留最多单winner、无部分写入和显式重试断言，尚待联合复验。

真实 deferred-FK COMMIT 拒绝测试及原始audit八outcome/错receipt/中间链损坏测试已追加。session 56313 exit101，仅缺 deferred-FK 测试入口 E0599；该结果不证明已模拟真实提交期故障，需看后续运行结果。

### 本批实现与最终门禁

- `f1fcb15`：真实 BR-159 audit/chain 原始事实只读查询，保留八种 outcome；四条新测试覆盖原字段、receipt 漂移和实际中段损坏。限定独立 Spec/quality 审查 Approved，无 Critical/Important；仅证明当前连接快照的完整性，不证明来源身份或外层事务已提交。
- `6fac7c7`：仅在 COMMIT 确认异常时关闭原写连接，再完整重查同一候选；未证实即返回脱敏 `CommitUnconfirmed`，不自动再次追加。真实 deferred-FK COMMIT 拒绝、提交后丢确认模拟、重查损坏/不可达与原有并发/回滚断言均通过。限定独立 Spec/quality 审查 Approved、无 Critical/Important，尚不按完整 Task3 交付。
- session 89676 曾为 67 passed/1 failed/1 ignored：唯一失败是测试将 Display 与 Debug 的大小写约定混用，原实现代理只修正该断言；不是生产 COMMIT 行为失败。
- 最终 Foundation session 67896，`cargo test --lib push_foundation:: -- --test-threads=1`，exit 0：**167 passed/0 failed/1 ignored**，18.19s，编译 2m27s；ignored child helper 由父测试显式执行。
- 采集审计相邻回归 session 98657，`cargo test --lib database::data_acquisition_audit:: -- --test-threads=1`，exit 0：**8 passed/0 failed**（旧4+新4），0.06s，缓存编译 1.80s。以上 lib-test 输出仍为 43 项既有 warning。
- Clippy session 8984，`cargo clippy --lib --message-format=json`，exit 0、1m23s；本次完整 fingerprint 经 jq 核验 **163** 条有位置 warning，Foundation 与采集审计文件均零诊断。三个本批 Rust 文件定向 rustfmt --check、git diff --check 通过；没有把非 strict 门禁称作全仓零告警。

提交异常验证区分了“真实 SQLite deferred-FK COMMIT 拒绝”和“真实提交后模拟确认丢失”；没有强制模拟 fsync、磁盘耗尽或 VFS 故障，不能夸大覆盖。完整 W15 仍缺可信 source descriptor/opener、版本化 NotRequired、KnownOccurrence、认证发布与恢复、权威 probe/CLI、W11/W14 联结和整体门禁。

用户要求继续提速后，按 subagent-driven-development 的任务边界让独立审查与统一验证交叠，复用未变化代码的有效证据，并集中更新同批文档；不通过重复开 agent、并发争抢 Cargo 或删减真实验收来声称提速。完整 W01--W21/52 Unit 目标不变，生产 monitor 与真实业务库仍未操作。

Task3E 独立审查保留一项非阻塞清理建议：仅测试调用的错误构造 helper 和两个私有 fault 变体尚未加条件编译，但实际注入入口已为 cfg(test)，不存在生产可调用故障口。记录为下次修改该文件时顺带清理，不为此重开已通过的事务行为审查；43/163 项既有告警背景继续如实保留。

## 10. 显式依赖合同与持久 occurrence（2026-09-08）

本节更新 §9 的历史剩余清单，不修改其历史测试结论。两条实现按独立文件并行，统一验证：

- `595f605`：声明新增预期 authority 和 Required/NotRequired；不适用声明与观察精确绑定依据、source ID 和版本，只允许 AuthorityArtifact 支持。不允许用同 SHA 的错误证据类别替代。候选格式升级为 v2，严格拒绝旧 v1、缺字段及重算 hash 后的派生状态漂移；不修改 operational schema、recovery event v1 或冻结 W07。限定独立 Spec/quality Approved，无 Critical/Important。
- `ac8a28b`：通过自有只读事务读取真实 intent 和完整转换链，复用 bundled catalog 验证 Unit/family/owner，支持 Ready、NoData、Disabled。schema 与实际内嵌冻结 DDL 的 25 对象定义逐字节比较，参考仅在内存生成。8 条真实 SQLite 测试覆盖错绑、未持久身份、中段损坏、自洽但非内嵌 schema、缺文件和 WAL；独立 Spec/quality Approved，无 Critical/Important。
- `cargo test --lib push_foundation:: -- --test-threads=1`，session10760，exit0：**182 passed/0 failed/1 helper ignored**，19.21s，编译2m21s；helper由父测试显式调用。全部新反例与原共享 schema/锁/hot-journal/store 回归通过。lib-test 仍有43项既有warnings。
- `cargo clippy --lib --message-format=json`，session43386，exit0、1m21s；完整制品诊断核对163项有位置既有warnings，Foundation/采集审计目标零诊断。定向 rustfmt 与 `git diff --check` 通过；相对4ce4a4f，monitor/notification/config/Cargo/冻结DDL无改动。

本批 TDD 区分实际失败：63225/20721 为缺模块/字段的编译期 RED；72590/13330 的 occurrence 测试拒绝于 DatabaseOpenFailed，修复测试临时目录为 canonical 路径，没有放宽生产 NOFOLLOW 规则。13330 的 v2 domain 是另一条实际运行期 RED；64739 为71/0/1的首 tracer GREEN，而非本批完整门禁，最终结果以上述10760为准。

持久 occurrence 没有 producer 列，结果中的 producer 只证明所选 catalog 关系吻合，不证明原始写入者。指定文件的持久完整性也不证明它属于当前 source version/build/generation。仍不能签发权威快照或 ReadyGate。

审查的跨任务证据由主控核对：缺声明非就绪仍由 operational_readiness_tests.rs:464/:488 的实际断言覆盖并在10760通过；operational DDL原文未改，recovery生产文件与冻结W07相对4ce4a4f无差异，旧store/recovery测试通过。真实版本化来源/basis认证仍列为后续集成验收，不由Task3G冒充完成。两个reviewer的Minor均为已登记43/163既有告警背景，没有新增阻塞finding。

## 11. 来源与部署的后续接线顺序

本节记录当时的接线缺口；其中“只有单manifest/generation”的结构限制已由后续v3集合路径补齐，见[最新边界核对](#13-v3集合与实际查询入口的最新边界-2026-09-10)。来源认证、生产授权和实际probe尚未因此完成，以下历史测试及当时分析不改写为新验收结果。

Task3H 已完成限定实现与审查，最终证据见 §12：在 database 模块给既有 BR159 完整链/receipt 验证增加 rusqlite 事务行加载适配器，与 Diesel reader 共用规则，不跨驱动重开路径。该适配器自身不打开文件、不改变事务生命周期，也不认证源 schema/注册；完整来源读取仍须外层安全 opener、实际 schema 与上下文绑定。

接线设计核对发现两项必须解决的前置条件：

1. `src/data_gateway/grpc_source.rs:1445` / `:1583` 的 opening 请求摘要没有采集时的 namespace、权威业务日或部署代。需要由实际采集流程产生绑定 audit ID/record hash 与这些上下文的不可变事实；不能向旧行倒填一个 descriptor 就称其已被认证。
2. RFC 的 manifest/generation 是逐 Unit，而当前 W15 context 只有一份 manifest/generation。全局 Core 不能取最大代、任选 Unit 或缩为单 Unit。该集合合同须在 W16 衔接时明确实现并验证，当前尚未解决。

因此接下来的顺序是：共享 reader 与真实闭集合同注册 → W16 的实际部署/批准/manifest/journal/owner 读取验证基础 → 来源 context 绑定与 W15 认证存储/恢复 → 同快照 probe 和 W11/W14 联结。W16 读取基础不以 W15 Ready 为前提，执行阶段再重验 fence，避免相互等待。仅有路径、approved_by 字符串或合法 hash 不替代真实认证。

完整目标仍为 W01--W21、52个迁移单元及真实发布门禁。W15整体、W16--W21、逐Unit迁移与上线验收均未完成；本批未启动、观察或替换生产monitor。

## 12. 采集审计事务适配器完成（2026-09-08）

`a538925` 仅修改 `src/database/data_acquisition_audit.rs`。调用者持有的 rusqlite 事务与原 Diesel reader 共用固定投影及原 receipt/全链/hash 验证内核，严格保留整数、可空文本、原始 created_at；入口不自行 open、BEGIN、COMMIT、ROLLBACK 或设置 pragma。

- session82943：`cargo test --lib database::data_acquisition_audit:: -- --test-threads=1`，exit0，**13 passed/0 failed/0 ignored**，测试0.13s、编译2m18s；43项既有 warning。
- session13184：`cargo clippy --lib --message-format=json`，exit0、1m25s。完整诊断核对163项有位置既有 warning，Foundation及采集审计目标零诊断；定向 rustfmt 与 diff 检查通过。
- 新增5条测试并扩展既有中段损坏测试：真实跨驱动已提交行、8种 outcome、4类 receipt 漂移、audit/chain 中段损坏、19个投影字段逐一错误 SQLite 类型、两张表分别缺失、调用者事务生命周期与只读目录字节不变。
- `w15_acquisition_transaction_review` 在原始 `595f605..a538925` 范围独立审查：Spec compliant / quality Approved，无 Critical/Important。告警背景列为已归因非阻塞项。

审查不能从差异证明的外层提交、schema/source 所有权及 W15 authority，均不在本适配器声明的证明范围，仍明确列为后续来源拥有者和部署验证器的必要验收，未据此签发 Ready。

下一条完整开发链按 [W16 实施计划](../superpowers/plans/2026-09-08-push-foundation-w16-activation.md) 推进：先同一只读事务加载全 Unit 的实际 manifest/journal、重算内容及完整链、区分未登记/待协调/持久一致，再接外部认证与当前 owner。读取一致不等于部署认证或允许发送；真实平台信任根、Shadow/legacy 共同授权等未决项保留。没有启动监控或生产操作。

## 13. v3集合与实际查询入口的最新边界（2026-09-10）

此次为隔离树源码只读核对，Rust/Cargo仍在`aef7972965f610ed418049593dfff1d55341772e`，没有重跑已关闭的v3/store/codec测试或启动实际probe。它更新下一步依赖判断，不为W15签发完成证书。

| 旧剩余项/入口 | 当前已有能力 | 仍需补齐的边界 |
| --- | --- | --- |
| 单一manifest/generation | [逐Unit部署声明](../../src/push_foundation/activation_readiness.rs#L31)及[完整集合](../../src/push_foundation/activation_readiness.rs#L161)已存在；[v3载荷](../../src/push_foundation/readiness_snapshot.rs#L538)包含完整deployment set及摘要 | 不能再按“尚无集合结构”重复开发；v2 scalar兼容分支仍保留，并非被删除 |
| 集合评估/快照 | [evaluate_for_deployment_set](../../src/push_foundation/operational_readiness.rs#L441)、[try_new_v3](../../src/push_foundation/readiness_snapshot.rs#L222)按同一catalog重算并核对assessment | 部署声明和证据仍须独立认证；[模块契约](../../src/push_foundation/activation_readiness.rs#L1)明确不认证source/owner/binary/执行权，hash相等不提升为批准 |
| 已持久化查询 | [load_head/load_record](../../src/push_foundation/readiness_store.rs#L144)验证并返回head链上的记录 | [StoredReadinessRecord](../../src/push_foundation/readiness_store.rs#L75)只证明持久receipt，内部仍是CandidateReadinessRecord；不是已认证运行快照 |
| W15 probe | [readiness_probe](../../src/push_foundation/readiness_probe.rs#L1)以catalog和调用者facts生成候选inventory | 尚非同一已认证store快照的消费者；不能用候选计数替代权威查询 |
| 现有gRPC probe | [run](../../src/bin/grpc_local_readiness_probe.rs#L117)直连外部健康/能力/业务查询 | 它不是W15 store查询入口，不能因名称相似就合并完成状态；本轮未执行任何RPC |
| monitor组合 | [Foundation声明/导出](../../src/push_foundation/mod.rs#L19)中这些模块仍为内部边界 | 限定检索monitor及该gRPC probe未找到这些W15记录/集合/store标识的直接consumer，不是全仓间接调用图证明，也不表示删除已有能力 |

限定检索标识为`ReadinessDeploymentSetContext`、`CandidateReadinessSnapshot`、`CandidateReadinessInventory`、`StoredReadinessRecord`、`ReadinessStore`、`readiness_store`、`readiness_snapshot`、`readiness_probe`、`try_new_v3`；范围为`src/bin/monitor`下源码及`src/bin/grpc_local_readiness_probe.rs`。后续应在已存在的全Unit集合能力上补来源/owner/build真实性、认证后的记录存储与恢复，再让query/probe/monitor消费同一认证快照；不要退回单Unit、自报bool或任意文件路径。具体生产信任材料与授权仍单独验收。
