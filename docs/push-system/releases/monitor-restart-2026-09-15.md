# monitor 重启验收（2026-09-15）

当前状态请看[09-16常驻运行验收](monitor-launchd-2026-09-16.md)：用户新授权后，已改由LaunchAgent托管同一已验收制品，09:35:24完成启动对账并进入上午盘；下文“未运行/未安装自启”均为09-15历史时点，不代表09-16当前状态。

后续单次核对（2026-09-15晚间）：在用户询问剩余工作时，pgrep及ps两项只读检查均未发现业务monitor，原PID59981不存在，仅行情gRPC PID761仍在。此实例日志末尾为14:04，包含descriptor_attestation_unavailable和来源超时，但未取得退出状态或确定原因；不能将该日志错误直接称为进程退出原因。本次未重启、未部署，也没有开启持续监控。下文09:32是历史启动验收，不代表当前仍在线。

状态（验证至09:32）：**monitor 已重启并进入上午盘循环，PID `59981`、持续 PTY 会话 `92295`。09:32:03 启动对账完成，09:32:06 自然触发的 DataMode 飞书消息回执校验成功，09:32:09 事件状态提交确认。** 使用原已验收扫描修复制品；重启成功不等于所有行情数据和推送业务均健康。

## 制品与操作边界

运行文件：`target/monitor-deploy-20260914-scan.9AjKKD/release/monitor`。启动前重新计算 SHA-256 为 `dea3dad22500dca238900add96d8bf820e3df325855ffd03166d191fcc60d7ae`，大小 35,981,248 字节、inode 151207492，与[9月14日扫描修复验收](monitor-scan-release-2026-09-14.md)完全一致。没有重新构建、覆盖制品或部署工作树中尚未验收的 Macro/schema 改动。

09:13只读检查曾确认monitor未运行；启动前再次检查系统进程，仍仅行情gRPC PID761存在，生产monitor实例锁未见持有者。因此本次不向任何旧PID发送信号，不重启行情服务，直接启动已验证制品。没有清除锁、删除审计、恢复数据库备份或人工重放未知投递。

启动工作目录为原项目根，原配置解析与正常生产扫描/推送政策保持。新日志使用noclobber排他创建，不覆盖旧日志：

```sh
set -o noclobber
exec target/monitor-deploy-20260914-scan.9AjKKD/release/monitor > logs/monitor-scan-restart-20260915-0930.log 2>&1
```

该命令经工具权限确认后在持续PTY执行；不是此前未能持续存活的nohup后台方式。日志文件名中的0930只是标签，实际首条日志为09:29:38。

## 本次已核对的运行身份

`ps`确认新PID59981存在，`lsof`确认：cwd为原根、txt为上述制品；核心数据库仍为 `data/stock_analysis.db`（inode107663956），投递数据库为 `data/durable_delivery.sqlite3`（inode140129846）。生产实例锁（inode143554981）、投递审计锁、event audit目录和正文归档锁（inode140175103）均在原根，没有转入开发工作树。

持续PTY最近一次poll仍返回运行中，首条推送完成后`ps`再次确认PID59981存在（运行时长03:08）。本次没有跳过历史审计或数据初始化来加速。

## 启动与自然消息验收

1. 09:29:38原生产根绑定；09:29:39投递审计预检healthy；09:29:41开始核心数据库初始化，09:32:02完成，耗时约141秒。没有通过修改校验或生产配置缩短等待。
2. 09:32:03启动固定点：`progress=0 resumed_sink_calls=0 foreign_lease_boundaries=0 manual_review_boundaries=9 schedule_hydrations=3`。原9条未确定投递保留人工判断边界，没有自动重发；随后进入正常上午盘业务循环。
3. 09:32:04正文写入原根，09:32:06飞书`receipt=validated`且`PushKind=DataMode pushed=true`，09:32:09记录`confirmed DataMode delivery committed for event state`。这是程序自然启动消息，没有人工测试发送。

新正文为`data/push_log/2026-09-15/093204_000000000000000018d55a7df9b63540_0000ea4d_0000000000000000.md`，实际789字节、inode151327996，SHA-256 `1ff66b94ed34dec906d8dcddc8e9e4ce13e9f01516e38ff83ed897d56166e8f1`。正文与日志核查只证明本条消息归档/投递，不泛化到其他推送类型。

## 日志与未完成项

[本次运行日志](../../../logs/monitor-scan-restart-20260915-0930.log)。启动、对账和一条自然消息已验证；日志仍显示账户快照过期、DataMode为Unsafe，以及RealtimeQuotes/BoardConstituents等请求`no_verified_batch`，因此相关扫描/业务推送仍会按原规则跳过或失败。selection-v2仍因proposal_missing关闭，cffex_futures_delivery仍unsupported，原边界没有放宽。不能将行情gRPC进程存在或External健康/能力检查成功当作真实数据批次可用。

这次重启不完成Macro同库恢复、完整盘后/定时器/全部52个业务单元迁移，也不修复尚未完成的SQLite FD复用问题。没有安装系统自启或自动拉起服务；开发继续，启动验收不扩展为持续监控任务。
