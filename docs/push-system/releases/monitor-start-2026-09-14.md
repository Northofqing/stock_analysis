# 最新 monitor 启动验收（2026-09-14）

后续更新：扫描取消/关停修复已于 12:13:57 切换到 PID 92715，12:15:12 启动对账完成并取得新飞书回执，见[扫描修复发布验收](monitor-scan-release-2026-09-14.md)。下文 PID 30370 是前一运行版本，已于 12:05:07 优雅退出，历史证据原样保留。

状态：最新优化 release 已于 10:07:26 启动，PID 30370；10:08:28 完成核心数据库初始化及启动对账，进入上午盘业务循环。10:08:29 DataMode、10:10:01 IpoCatalyst 的真实飞书回执均校验通过，正文写入原生产目录；dev 实例 PID 14270 已结束。**最新版本已运行，但完整迁移及所有推送健康验收未完成。** 用户要求「并行完成，启动起最新的monitor」。

## 本次实际范围

使用最新开发工作树 `.worktrees/push-reliability-20260905`，HEAD `b3742abab641565bab9b6893b4fdb75bb81261de` 加现存工作区修改。不是旧 release 的复刻，也不是此前临时参考的两文件包。原项目的 160 个合并冲突不处理、不覆盖，旧 release 保留。

必要启动适配：构建变量 `STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT` 固定原生产根 `/Users/zhangzhen/Desktop/Quant/stock_analysis`。生产核心 DB、投递 DB、单实例锁、审计、板块及 selection 发布材料使用同一个根；测试命名空间保持开发目录。运行时环境/.env 不可改变这个固定身份。以原项目为工作目录启动，保留 dotenv/TOML/相对缓存的既有解析。没有绕过锁、schema、启动对账或开启尚未发布的 selection。

并行完成另一片修复：龙虎榜保存结果的运行编号、上下文摘要、输入摘要必须属于原运行；真实损坏测试已复现，相关 169 项回归及消费者 Clippy 已通过。该限定修复完成不等于完整盘后恢复任务完成。

## 启动前保护

备份目录：`backups/monitor-start-20260914.peMG7I`，目录权限 700。

| 备份 | SHA-256 | 检查 |
|---|---|---|
| stock_analysis.db | e004c1beb8c5b69b4c25f5fda838e9a1a1b7350db052e7bc278f07aa14714fa2 | SQLite 在线备份完成；独立快照 quick_check=ok |
| durable_delivery.sqlite3 | 742e0b6d43f6a460572c5084382c309a0a918b173e304ca7730b33f9dc09d55e | 在线备份完成；quick_check=ok，user_version=9 |
| monitor-and-audits.tar | 8fd57cc272b97a11156a6bf6d8badc88a58fd09373e5a15d637d4a2d24b0519f | 含旧 release、event_audit、durable_delivery_audit、selection 生产审计 |

旧 release SHA：`d27b52f532b86ee393124b3d944949279c7ceedadd5ee29f9ec05461e86e78ed`。未覆盖、未启动。数据库快照先用普通只读打开遇 SQLite WAL sidecar 问题；对无写者的独立备份使用 immutable 只读检查后通过，不将该方式用于活动生产 DB。

回滚只能先停止本次明确 PID、确认发送状态，再运行保留的旧制品；不能盲目重发、清空去重或在 grpc 服务仍使用数据库时自动恢复备份。备份恢复不是本次已经执行的操作。

## 验收记录

- 生产路径纯 std：默认根 2/2，通过；合成配置根 2/2，通过；真实 RED 为配置根错误回落到编译根。
- 两片限定独立审查：Spec/Quality 通过；不代表全分支或生产运行已验证。
- Cargo 相关回归：99257 于 08:36:31 北京时间退出 0，169/169 通过；614 项源码前后和日志摘要核一致。
- 路径/手动推送/单实例锁：69271 于 08:39:24 退出 0，16 项库测试及 12 项 monitor 测试通过（另含 1 次独立 cwd 子进程），614 项源码前后核一致。
- 构建：30143 于 08:40:27–08:42:47，以固定原生产根完成最新 dev 配置（非优化 release），exit0；614 项源码前后/当前核一致，log SHA 8fa8ce9dd3a71e3d003f35ffe17fb977c0cdc26b107b3ef4234c1a4a9d8de075。测试后仅调整 helper 格式换行，未改行为。
- dev 制品：target/monitor-deploy-20260914.4oWNlf/monitor，98,866,248 字节，SHA 2c63a7edf9c1c5aeeabab05fdfe96d97c9102f5090dbf62c553ecb39ab1efaa9，与构建输出逐字一致；旧 release 摘要未变。部署目录内 source-snapshot.tar 包含本次 src/Cargo/build/proto/.cargo 配置，不含 .env；SHA 27a7e2af1ba237acf59553e4a5e9ed112c29214dcd209e86d73705da7a0301e0。
- 首次 nohup 后台启动返回13386，但检查时进程不存在、日志为空，不能算成功。保持同一 binary/配置改为持续 PTY 会话30716，实际 PID14270 于09:35:24 启动，之后持续存活；支持问题在启动会话保持层的判断，不将后台 shell 的退出0当业务成功。
- lsof 已核：cwd 为原根；txt 为独立部署制品；核心 DB inode107663956，与原 grpc 服务所用相同；投递 DB、production/monitor-delivery.lock、event_audit、durable_delivery_audit 都在原根。09:35:35 delivery audit preflight healthy；09:44:43核心DB初始化完成，09:44:46启动固定点达成并进入正常上午盘循环。
- 启动恢复：progress=0、resumed_sink_calls=0、manual_review_boundaries=9、schedule_hydrations=3。9个未确定投递保留人工判断边界，没有擅自重发。09:44:49首条DataMode飞书消息receipt=validated，09:45:11对应事件提交确认。
- dev 阶段实际发现正文 push_log 的生产/测试混合 constructor 仍绑定编译根，写到了开发目录（notify.rs:1607）；该问题随后已补齐，见下方优化版切换记录。现有正文文件保留，不重写历史 DB/审计，首个有遗漏的 release 候选没有部署。
- 日志：[monitor-latest-20260914-4oWNlf.log](../../../logs/monitor-latest-20260914-4oWNlf.log)。selection-v2 保持 disabled / proposal_missing；cffex_futures_delivery 保持 unsupported，没有越过已有发布边界。
- 最新消费者 Clippy：52257 于09:40:19退出0，lib/stock_analysis/monitor检查通过（仅编译，不执行CLI）；614项源码前后/当前、日志摘要核一致，log SHA6165611e4d64faf2d5a0bb695c77e113196dba0d24771080b8c5b7de8d5cef94，lib231/monitor2条告警保留。
- 原临时参考制品仍不部署；其编译根错误不会靠复制/CWD 修正。

## 优化版切换与正文归档补漏

`notify.rs:1607` 的混合生产/测试 constructor 改用 `production_root::root_for_mode`；根目录与创建边界同时选择，Test 仍使用开发树。仅一处 initializer 变更，锁、nofollow、inode、单链接、同步落盘、终态验证均保留。独立审查 Spec/Quality 通过。定向回归会话 37502 于 09:57:08 退出 0：21 passed、1 ignored（跨进程 helper，由父测试另外执行 4 次并通过），738 filtered；614 项源码前后相同，日志 SHA `70e7193a07b6ff5c689cc647fd89cfad427072ffe3b30588330806224c286a4e`。测试采用默认隔离根，未编译到真实生产根执行测试。

修正后的 release 构建会话 48696 于 09:59:32–10:01:30 完成，exit 0；614 项源码前后和部署前当前快照一致，日志 SHA `1bb0b467194aaad6529e27b7052371845d67bfde25933049eaeb8e8d2cdb2024`。已保留的 Clippy 52257 在这一处 initializer 修改之前，不能写成最终源码再次全量 Clippy。

- 当前制品：`target/monitor-deploy-20260914.4oWNlf/release/monitor`，35,943,984 字节，SHA `e53ff250455cc246b8d600149dd15ad4d62f8ca0643c3bb329c9df7c4a63682d`，与最新构建逐字相同。原 dev 制品仍保留于同目录上一级的 `monitor`；原 `target/release/monitor` 亦未覆盖。
- 对应源码归档：部署目录 `source-release-snapshot.tar`，SHA `348503a73cd8ae8c9a153fb9c415647734b3fc84fec627531ce8a828bc85f413`，不含 `.env`。
- 旧实例收到 SIGINT 后数分钟未退出；采样确认主循环阻塞在 `paper_sell::scan_and_sell -> grpc_source::block_on_with_timeout -> ScopedJoinHandle::join`。随后仅向已核实 PID 14270 发送 SIGTERM，会话 30716 退出 1，PID 和生产/开发归档锁持有者均消失。**这不是优雅关闭成功**，未清空账本或不确定状态，必须由新进程完成启动对账。
- 第一次以 `monitor-release` 文件名启动，被严格 CLI bootstrap 以 argv[0] 不识别拒绝，exit 2，发生在生产初始化前。保留失败日志；改为独立 `release/monitor` 名称启动同一字节制品，不修改 CLI 合同。
- 新实例 PID 30370，持续 PTY 会话 80960，工作目录原根；10:07:29 生产根日志、10:07:30 delivery audit preflight healthy。`lsof` 已核实核心 DB inode 107663956、投递 DB、单实例锁、两类审计及正文归档锁全部位于原根；`.push_log.lock` 仍为原 inode 140175103。不是临时根、symlink 别名或第二套生产库。
- 新日志：[monitor-release-20260914-4oWNlf.log](../../../logs/monitor-release-20260914-4oWNlf.log)。未安装开机自启或进程退出后自动拉起服务。

最终本次启动验收：新日志第 17–23 行显示 10:07:30 开始核心 DB 初始化、10:08:28 完成，随后启动固定点 `progress=0/resumed_sink_calls=0/manual_review_boundaries=9/schedule_hydrations=3`，进入正常上午盘。与旧进程同为 9 条人工核查边界，没有自动重发。第 44–49 行显示 10:08:28 正文写入 R、10:08:29 飞书回执校验成功、10:08:31 DataMode 事件状态提交；第 125–128 行显示 10:09:59 第二份正文写入 R、10:10:01 IpoCatalyst 回执校验成功。这些是程序自然触发的消息，未手工制造测试推送。

生产日期目录已出现新 PID 段 `000076a2` 的 `100828_…_0000000000000000.md` 与 `100959_…_0000000000000001.md`；开发日期目录仍只有原 3 份旧 PID 正文。归档根修正已取得真实新写入 GREEN，而不仅是默认编译根下的测试通过。10:10:17 再次核实 PID 30370 存活。此证据覆盖启动和已观察的两类发送，不代表其余全部业务链路完成验收。

旧进程完全停止后，只将该部署生成的 3 份 Markdown 正文按原名称、原字节排他创建到 `data/push_log/2026-09-14`，每份 0600、目录 0700、文件及父目录 fsync；开发目录原件全部保留。没有复制锁、硬链接、覆盖已有正文或重写历史 DB/审计。内容校验如下（完整名称可在该日期目录核对）：

| 名称前缀 | 字节 | SHA-256 |
|---|---:|---|
| 094448（PID 段 000037be，序号 0） | 917 | 056f31eaf5522660415da4f83ac8d609b7d104c9c50a7ec04e73640501f7bf05 |
| 094835（同 PID，序号 1） | 561 | 6755756957c06ea5b37d85be4383040df5b4818d9c8d9d34ec38d7db88c894fa |
| 095233（同 PID，序号 2） | 122 | 5a973d83920c420b9b285056b6b2d5fdaf479bbd4ac59ef81b83a95944ee01b9 |

## 仍然未完成

本次实测启动瓶颈：生产 data_acquisition_audit 与 chain 在首轮启动检查时各 2,203,771 行。09:36 采样栈确认 run_migrations -> create_schema -> validate_data_acquisition_audit_chain_rows -> JSON/SHA256，不是仅凭无日志判定卡死。静态核查显示 benchmark_segments 的启动验证随后又完整验证同一采集链，至少两遍全量；正常采集 append 只校验尾部，不同于启动全量扫描。详见开发树 src/database/data_acquisition_audit.rs:450、:485、:608、:691 及 src/database/benchmark_segments.rs:1448。原校验没有删减，已切换优化版；本次初始化为 58 秒，旧 dev 为 543 秒。两次不是控制缓存/负载的性能实验，不能把全部差异归为编译优化。更长期的重复校验复用与进度观测另需设计/测试，不在生产临时跳过。

运行验收还暴露两项需要继续修复的问题，不能被“进程启动成功”掩盖：

1. 归因连接池反复出现 `descriptor_attestation_unavailable`。错误源于 `src/database/sqlite_descriptor_attestation.rs:408` 的 main FD 前后差集证明；普通业务池与归因池相互独立（`src/database/mod.rs:2488`、`:2497`），不能由此断言所有推送失败。10:45 补充：独立 TEST_CODE 连接关闭/重开实验已在实际系统 SQLite 3.51.0 上确认合法 main FD 复用使差集为零，项目真实 manager 回归和修复仍待；禁止因该错误放宽数据文件身份守卫。进度见[运行期修复](monitor-runtime-reliability-2026-09-14.md)。
2. 同步行情获取阻塞主 `join/select`，已导致 SIGINT 无法及时处理，也会延迟同组循环。证据为 `/private/tmp/monitor-stop-14270.sample.txt` 及 `src/bin/monitor/main.rs:4105`、`:5357`、`:8638`。需要独立的有界阻塞隔离/取消回归，不靠临时关闭业务配置掩盖。归档根修正及优化构建没有修复这个结构性问题。

数据条件仍需区分：持仓源快照已过期、opening live 数据无已验证批次、部分 gRPC 行情取消/超时、selection 发布材料缺失，均保留现有关闭/降级边界；未承诺全部推送健康、未伪造实时持仓、未强制测试发送。

完整 Task2–4/W15–W21/52 个业务单元迁移未完成；本次启动不等于全部新架构接管。宏观/搜索/模型/报告/逐目标发送恢复、完整 timer/manual/startup 接入、剩余故障矩阵和后续业务迁移继续按开发文档推进。可信身份/RBAC 不重新加入范围。
