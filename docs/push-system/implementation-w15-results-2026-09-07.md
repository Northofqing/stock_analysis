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
| a1ca363 | 重读候选快照时重新判级和编码，拒绝重算哈希的伪 Ready、重复/额外字段及非规范输入 | codec 4 条测试；独立审查发现两项测试缺口，尚未通过 |

最近一次已结束的完整 Foundation 命令：`cargo test --lib push_foundation:: -- --test-threads=1`，session 62542，exit 0，126 passed/0 failed，测试执行 7.77s。此前 W15 命令 `cargo test --lib w15_ -- --test-threads=1`，session 15705，exit 0，23 passed/0 failed，2.22s。编译为既有 43 条 warning，不是零告警。

相邻 `push_job` 命令 session 56855：52 passed/0 failed，1.76s。以上均为候选切片的测试证据，不包含本记录之后尚未验证的 Task 3 新源码，也不是全仓测试、完整 W15 或生产验收。

独立复审确认 `OccurrenceInput + ActivationCoreUnready` 在判级前即被拒绝，合法 Core/Producer/Input 原因矩阵仍成立；修复范围未发现新增问题。

codec 审查的两项待修复：错来源/错版本及合法 unavailable 原因的 roundtrip 缺少直接断言；namespace/build/generation 等 typed 输入故障矩阵不完整。审查未发现当前生产 parser 的绕过，但在补齐规范要求的执行证据前不标记通过。

## 3. 剩余验收

| 任务 | 当前缺口 | 完成证据要求 |
| --- | --- | --- |
| Task 2 | 预期 authority、版本化 NotRequired、已知 occurrence 真实性 | 实际 reader 校验与失败反例，不接受自由 bool |
| Task 3 | schema/open 正在开发；完整 store/认证恢复未完成 | 真实 SQLite 原子快照+事件+head CAS，重启、损坏、并发、确认丢失重查 |
| Task 4 | 当前只有纯计数，不是实际 probe | 同一已验证快照的 health/readiness/CLI；只读零副作用与损坏拒绝 |
| Task 5 | 只读接口审计完成，代码联结未完成 | 身份/版本/恢复来源精确绑定，输入阻断与有效窗口内恢复，过期/终态不重开 |
| Task 6 | 只有部分切片审查 | 完整 W15 双轴评审、目标与相邻回归、check/clippy/rustdoc、probe 与文档验证 |

W11 barrier 只证明固定点遍历完成，不含跨库身份。W14 当前没有 input block/recovered 提案入口；`ScheduleOccurrenceId` 与业务 `OccurrenceId` 不同，不能比较字符串后当作同一对象。

## 4. 下一步和并行边界

主代理统一维护接口、Cargo 调度、集成与文档；一个实现子代理负责独立 schema/open 文件，另一只读代理复核 codec。先形成完整事件与快照材料，再集成持久事务和真实证据 reader。候选哈希、SQLite 连接或自由构造的恢复引用都不能升级为执行许可。

本轮采用 planning-with-files 保留可恢复记录；subagent-driven-development 将独立 schema 与核心事件材料分开，避免共享文件并发修改。拆分不减少完整 W15 的验收目标。
