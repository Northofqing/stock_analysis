# monitor 运行期修复进展（2026-09-14）

状态截至 12:15（北京时间）：**扫描修复 Task1 已验收并上线，PID 92715 完成启动对账、取得真实 DataMode 飞书回执，见[扫描修复发布验收](monitor-scan-release-2026-09-14.md)。完整迁移没有完成。** 旧 PID 30370 在发布验收后于 12:05:07 优雅退出；开发/测试期间未停止它。以下历史测试点只对应各自候选，当前制品与剩余边界以新发布记录为准。

## 并行工作的实际分工

- 开发 agent：隔离真实虚拟盘扫描，补取消、已完成结果保留，以及盘中/盘后和监督层接线。
- 诊断 agent：用独立 TEST_CODE SQLite 库复现连接关闭/重开时的描述符复用；实验已交付，独立 Spec/Quality 审查已通过。这只关闭诊断，不代表连接池修复完成。
- 主控：串行执行编译测试、核对证据和更新物理 docs；保留当前运行制品。普通 Rust 文件由一名实施者修改，避免并行覆盖。

## 已取得的证据

| 项目 | 实际结果 | 仍未证明的内容 |
|---|---|---|
| 扫描阻塞复现 | 受控慢行情下，真实扫描使同组心跳/关停无法推进；原断言真实失败 | 不代表所有同步业务路径均已定位 |
| 首项隔离修复 | 同一真实扫描和原断言通过：1 passed；使用受监督 `spawn_blocking` 句柄 | 只是库级首片，尚未接生产两个调用点 |
| 关停后继续请求 | 原反例转绿：关闭 session 后，仅已开始的 A 收尾，不请求 B | 数据库连接等待后的取消复查、嵌套估值仍待补验 |
| 已完成成交保留 | 私有内存 SQLite、真实规则/模拟成交/审计路径验证成功：普通失败后继续，后续 panic 或关停保留先前成交 | 不代表生产行情与真实账户数据条件已经可用 |
| 真实监督层关停 | 独立子进程、显式临时 DB、真实扫描和受控慢行情：正常关停按行情结束→后台退出→writer 关闭；缺 writer 原反例修复后亦先排空再报错 | 不覆盖其他同步扫描或任意 SQL 的硬截止，不代表本修复已上线 |
| SQLite 合法 FD 复用 | 保留读事务、关闭另一连接再重开，SQLite 成功而 main FD 差集为 0；单连接对照为 1 | 尚非项目真实 Rust manager 的修复回归 |
| 实际依赖对齐 | 当前制品链接系统 SQLite；系统运行库 3.51.0 上亦复现。crate-carried 3.45.0 仅是另一份对照 | 不能把系统库与 crate 自带源码说成同一版本 |

测试使用隔离输入与受控 I/O，没有调用生产 DB、真实行情或发送测试消息。前三次运行的 619 项源码输入前后相同；最新候选新增一个测试文件后为 620 项，前后同样一致。最新保留 57 条 lib test 告警（此前 56 条）和 126 条普通 lib 编译告警，不称无告警验收。

| 测试记录 | 结果 | 日志 SHA-256 |
|---|---|---|
| paper-scan-heartbeat-red（10:22:55–10:26:58） | exit 101；0 passed / 1 failed | `25e627b384bbccf0aaa5d0b6a27c77c21232b9504d165786bab383341b204edc` |
| paper-scan-heartbeat-green（10:31:15–10:35:35） | exit 0；1 passed | `e1e07b05d6e34301cb37b2ddf0e544cf4ac864168ec482ffe0b822687897c0cc` |
| paper-scan-cancellation-red（10:42:21–10:44:32） | exit 101；0 passed / 1 failed | `497c3c962728d3e368a833c436a9316229d28ff3b1ff5e677d22f6ece6229bfa` |
| paper-scan-cancellation-green（10:57:30–11:03:50） | exit 0；lib 7 passed / 1 ignored，monitor 8 passed；审计 helper 另执行 4 次并通过 | `5d4029e9d7e6e2bdba5cbc17812e78bda46e90f67a389bae60f1868399c6ee54` |
| paper-scan-missing-writer-red（11:10:57–11:11:31） | exit 101；真实子进程进入行情后，监督层取消/排空断言失败 | `fd8f2d55a3ea41631b42252d43246b83d2678bf000a83fce314fcdc55be30d2e` |
| paper-scan-missing-writer-green（11:13:38–11:14:20） | exit 0；monitor 14 passed；两个隔离子进程各 1 passed | `8134151a893d29035f4a272937075fe6774a9ece305d9d7c123782fb99797b12` |
| paper-scan-checkout-red（11:16:40–11:18:53） | exit 101；取得连接时已取消，原实现仍写入真实 Filled 与审计回执，断言失败 | `50754c1149c420df39654ee21e6d99776795ee575399385264ed4106e80697aa` |
| paper-scan-final-candidate-green（11:29:30–11:37:44） | exit 0；lib 8 passed、monitor 14 passed，两个子进程各 1 passed；checkout 原断言转绿 | `6a26293e6964f89c14cb7fcb3e59d53178b3f6b1e2f19ddc57d2de741ed60be7` |
| paper-scan-consumer-clippy（11:40:20–11:44:10） | exit 0；lib、monitor、stock_analysis 编译检查通过，未执行 CLI；lib 233 / monitor 2 告警 | `c44da876a2577b46bc0a2160d679c4ead2eb17283921e27d5c3125e822e6376f` |

首次取消 GREEN 覆盖 5 个扫描用例、8 个 monitor 监督层用例。`br141_` 还匹配了 lib 的两项审计回归及其跨进程 helper，故该次总数并非 13。最新监督层 GREEN 已限定完整模块名，覆盖新模块 3 项、原监督层 8 项及签名相关 source 检查 3 项；621 项源码输入前后相同。命令及子进程明确固定测试风险参数；父进程持有临时目录直到子进程退出，未初始化生产库或访问真实行情。

原始记录在开发树 `.superpowers/sdd/2026-09-14-monitor-runtime-reliability/`，每次含命令、源码摘要、完整日志和退出状态；[实施计划](../../../.worktrees/push-reliability-20260905/docs/superpowers/plans/2026-09-14-monitor-runtime-reliability.md)记录验收边界。

## 剩余工作与边界

1. 扫描：Task1 工程验收与本批发布完成。取消、部分成交保留、writer 缺失分支、真实监督层排空、数据库 checkout 后的成交前取消复查及嵌套估值取消/正常计算均通过。621 项源码前后及当前、日志摘要已独立核对；最终普通编译为 lib 123、lib test 56 告警。独立审查 Approved，保留关停多错误汇总的 Minor；Clippy 新增两个窄 seam 参数数量告警（paper_trade.rs:1088、:1232），不称零告警。其他同步路径及任意 SQL/provider 的硬截止仍不在此完成声明内。
2. 连接池：差集假设缺陷已证实，但当前系统 SQLite / Diesel 公共接口不足以直接提供当前连接专属的完整 fd 证明。候选方案涉及原生 SQLite 适配和依赖构建，尚未实施；不静默更换底层库、不放宽文件身份校验、不借任意同 inode fd 放行。
3. 发布：扫描修复的 release 构建会话 98936 已退出 0，源码与制品验证后切换到 PID 92715。逐字比较前一运行版本归档（618 文件）与当前（621 文件），恰三生产文件修改、三测试文件新增，无 schema/依赖/配置变化；旧制品保留。后续业务迁移仍须分别验证后再逐批发布。
4. 全局：其他同步路径、任意 SQL 的硬关停期限、数据源不可用，以及 W15–W21 / 52 Unit、盘后恢复后续阶段仍未全部完成。35 秒单次 gRPC 期限和 5 秒 SQLite busy timeout 都不是整个扫描的总关停上界。

本片采用并行开发/诊断和单队列验证；没有把“启动成功”“局部测试通过”计为整个重构完成。
