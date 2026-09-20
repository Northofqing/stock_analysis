# monitor 扫描修复发布验收（2026-09-14）

后续运行状态：2026-09-15用户确认重启后，同一摘要制品已由新PID59981启动，本次运行验收见[9月15日重启记录](monitor-restart-2026-09-15.md)。下文PID92715及时间为9月14日历史证据，不代表该旧PID仍在运行。

状态截至 12:15（北京时间）：**扫描修复已上线。PID 92715 于 12:13:57 启动，12:15:12 完成启动对账，12:15:13 的真实 DataMode 飞书回执校验通过。** 这是逐步替换的一批，不代表全部 52 个业务单元迁移完成。

## 本次改动

- 实际盘中、盘后虚拟盘扫描共用受监督的阻塞任务，慢行情不会再由该扫描占住主监督 future。
- 关停取消尚未开始的证券/行情/估值/成交；保留并等待在途工作，之后关闭审计 writer。取消、后续 panic 时保留已经完成的卖出结果。
- 在取得数据库连接之后、进入真实成交事务之前再次检查取消；嵌套估值中也停止后续证券及日 K 降级请求。
- 缺失 writer 的错误分支同样先取消、排空扫描和后台任务，再返回错误。

与前一运行版本的实际源码归档逐字比较：618 个文件到 621 个文件，恰为 `main.rs`、`paper_sell.rs`、`paper_trade.rs` 三个生产源码文件修改和三个测试文件新增。未改 schema、依赖、配置、行情政策、T+1/FIFO 或 Unknown 不重发约束。原根的合并冲突与用户修改没有处理或覆盖。

## 验证与制品

| 证据 | 结果 / SHA-256 |
| --- | --- |
| 定向回归 27289，11:29:30–11:37:44 | exit 0；lib 8 passed、monitor 14 passed，另两个隔离子进程各 1 passed；日志 `6a26293e6964f89c14cb7fcb3e59d53178b3f6b1e2f19ddc57d2de741ed60be7` |
| 独立 Task1 审查 | Spec compliant / Quality Approved，无 Critical/Important；关停多错误汇总为 Minor |
| 消费者 Clippy 29199，11:40:20–11:44:10 | lib、monitor、stock_analysis 检查通过，exit 0；日志 `c44da876a2577b46bc0a2160d679c4ead2eb17283921e27d5c3125e822e6376f` |
| release 构建 98936，11:45:39–11:55:26 | 固定原生产根，exit 0；日志 `95698d61ce9263277b6af04ebf324332c9b395f426cbe8e416cce2027d2a7ce5` |
| 新制品 | `target/monitor-deploy-20260914-scan.9AjKKD/release/monitor`；35,981,248 字节；`dea3dad22500dca238900add96d8bf820e3df325855ffd03166d191fcc60d7ae` |
| 源码归档 | 同部署目录 `source-snapshot.tar`，不含 `.env`；`318af1a6f4b97caaf4563ad718bcb6705fb9343ae8c961c7162e89927e5bc62b` |

测试、消费者检查与构建各自的 621 项源码前后/当前摘要及日志 SHA 已由主控核对。部署目录另保留 `test-result.json`、`build-result.json`。测试使用隔离根/自有 SQLite/合成输入，不执行生产配置的测试二进制；只有 release 构建显式设置 `STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT=/Users/zhangzhen/Desktop/Quant/stock_analysis`。

不是零告警验收：普通 lib 编译 123、lib test 56；Clippy lib 233、monitor 2，其中新增两个参数数量告警在 `paper_trade.rs:1088`、`:1232`。最终整分支审查仍须处理或明确裁定已登记的 Minor。

## 实际切换

1. 新制品构建、摘要核对和源码归档完成后，才向已核实的旧 PID 30370 发送 SIGINT。12:05:07 日志记录收到信号、writer 正常关闭、监控安全关闭；旧 PTY 会话 80960 实际 exit 0，PID 消失，生产实例锁无持有者。没有 SIGTERM、强杀或双实例重叠。
2. 新进程由持续 PTY 会话 82018 启动（shell 92709、monitor 92715），cwd 为原项目根。启动与权限确认之间存在间隙：旧进程 12:05:07 退出，新进程 12:13:57 创建；不能称无缝/零停机切换。
3. 12:14:01 原生产根绑定及 delivery audit preflight healthy；12:14:02 开始核心 DB 初始化，12:15:11 完成。全量历史审计校验仍执行，没有跳过校验来加速。
4. 12:15:12 启动固定点：`progress=0 resumed_sink_calls=0 foreign_lease_boundaries=0 manual_review_boundaries=9 schedule_hydrations=3`。原 9 条不确定投递未重发，恢复的计划状态仍为 3 项。进入交易日午休阶段。
5. `lsof` 核对：新制品 inode 151207492；核心 DB 仍为原 inode 107663956；投递 DB、生产实例锁、两类审计和正文锁均在原根。正文锁 inode 140175103 保持，没有另建开发目录数据库或锁。
6. 12:15:12 新正文写入原 `data/push_log/2026-09-14/121512_000000000000000018d514d0711e0ca0_00016a2b_0000000000000000.md`；12:15:13 飞书 `receipt=validated`，L7 记录 `PushKind=DataMode pushed=true`。这是自然启动触发的消息，不是人工测试发送。

实际日志：[monitor-scan-20260914-9AjKKD.log](../../../logs/monitor-scan-20260914-9AjKKD.log)。目前依赖持续 PTY 会话，未安装系统自启或自动拉起服务。

## 回退与剩余边界

前一制品 `target/monitor-deploy-20260914.4oWNlf/release/monitor` 原字节保留，SHA `e53ff250455cc246b8d600149dd15ad4d62f8ca0643c3bb329c9df7c4a63682d`；更早的原 `target/release/monitor` 也没有覆盖。历史备份见[首轮启动验收](monitor-start-2026-09-14.md)。回退须先核目标 PID/在途责任、退出并确认锁释放，再启动保留制品；本次没有恢复旧数据库、删除审计或重放未知消息。

这批只修复 paper scan 的阻塞/取消/排空。其他同步 `monitor.tick`、晚间复盘、任意 SQL/自定义同步 provider 的硬截止仍未完成；保持任务所有权不是承诺可以抢占任意同步调用。SQLite FD 复用误拒绝的生产修复仍待，来源不可用、过期账户快照、selection 发布材料与 futures 支持条件也没有被放宽。

完整盘后宏观/搜索/模型/报告/逐目标发送恢复、真实定时器与强完成、W15–W21/52 Unit 迁移仍继续，见[运行期进展](monitor-runtime-reliability-2026-09-14.md)。复杂可信身份/RBAC 不重新纳入范围。
