# W15 运行就绪实现记录（进行中）

## 1. 当前结论

W15 未完成，尚未接入生产。已交付候选判级、规范快照/重建及完整目录计数；独立持久化与认证恢复、实际部署探针、W11/W14 联结还在开发。生产 monitor 未启动、观察、替换或接线；开发限于 `codex/push-reliability-20260905` 隔离工作区。

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

当前 `b6f5c99` 的完整 Foundation 命令：`cargo test --lib push_foundation:: -- --test-threads=1`，session 71160，exit 0，150 passed/0 failed，测试执行 9.25s。对应 W15 命令 `cargo test --lib w15_ -- --test-threads=1`，session 99470，exit 0，47 passed/0 failed，3.26s。编译为既有 43 条 warning，不是零告警；独立评审未关闭前不将切片标记验收完成。

相邻 `push_job` 命令 session 56855：52 passed/0 failed，1.76s。以上不是全仓测试、完整 W15 或生产验收。

独立复审确认 `OccurrenceInput + ActivationCoreUnready` 在判级前即被拒绝，合法 Core/Producer/Input 原因矩阵仍成立；修复范围未发现新增问题。

codec 原审查的两项测试缺口均已修复：错来源/错版本及合法 unavailable 原因的 roundtrip 有直接事实等值断言；namespace/build/generation 等 typed 输入故障矩阵已补齐。独立复审逐项确认关闭，无新增问题；这不证明外部认证或持久关联已经交付。

## 3. 剩余验收

| 任务 | 当前缺口 | 完成证据要求 |
| --- | --- | --- |
| Task 2 | 预期 authority、版本化 NotRequired、已知 occurrence 真实性 | 实际 reader 校验与失败反例，不接受自由 bool |
| Task 3 | 候选 recovery/event 重读通过评审；schema 复审两项竞态仍开放，完整 store/认证恢复未完成 | 真实 SQLite 原子快照+事件+head CAS，重启、损坏、并发、确认丢失重查 |
| Task 4 | 当前只有纯计数，不是实际 probe | 同一已验证快照的 health/readiness/CLI；只读零副作用与损坏拒绝 |
| Task 5 | 只读接口审计完成，代码联结未完成 | 身份/版本/恢复来源精确绑定，输入阻断与有效窗口内恢复，过期/终态不重开 |
| Task 6 | 只有部分切片审查 | 完整 W15 双轴评审、目标与相邻回归、check/clippy/rustdoc、probe 与文档验证 |

W11 barrier 只证明固定点遍历完成，不含跨库身份。W14 当前没有 input block/recovered 提案入口；`ScheduleOccurrenceId` 与业务 `OccurrenceId` 不同，不能比较字符串后当作同一对象。

## 4. 下一步和并行边界

主代理统一维护接口、Cargo 调度、集成与文档；一个实现子代理负责独立 schema/open 文件，另一只读代理复核 codec。先形成完整事件与快照材料，再集成持久事务和真实证据 reader。候选哈希、SQLite 连接或自由构造的恢复引用都不能升级为执行许可。

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
