# monitor 用户级常驻运行（2026-09-16）

用户明确要求“持续运行monitor”。此前09-16 08:16单次只读检查未发现monitor，仅行情gRPC PID761；旧monitor日志止09-15 14:04，退出原因未确定。本次经权限确认安装用户级LaunchAgent，未部署开发树中的Macro/schema改动。

## 当前已验证状态

- 22:59只读复查：`ps`确认PID2641、PPID1仍为同一归档monitor，已运行3小时34分；stderr日志推进到22:59:05。该轮持仓产业链`requested=6 assigned=6 verified_empty=0 failed=0`；公告接收100条、过滤跳过100条、推送0条，不是未收到公告数据。日志仍报实时账户快照过期（30秒门未放宽）。本次没有启停、部署、改配置或手动推送；未重新验证全套行情能力，也不把进程存活当作所有推送健康。
- 19:43:37再次只读核查：当前LaunchAgent仍`state=running`/`keepalive`、本次加载`runs=1`/`never exited`，PID已变为2641（PPID1，19:24:21创建），不是上午的64960；`ps`与生产实例锁`lsof`一致。系统uptime仅28分钟，因此上午进程不能当作一直未中断的证据；本轮没有人工启停或部署。执行制品路径仍为下节固定归档。19:43:16盘后循环继续写日志，但gRPC `127.0.0.1:18082`不可达、DataMode Unsafe，且仍出现`descriptor_attestation_unavailable`；运行托管有效不代表行情/数据库所有业务健康。
- 09:33:11创建monitor PID64960，父进程PID1；`launchctl print gui/501/com.stockanalysis.monitor`已显示`state=running`、`runs=1`、`last exit code=(never exited)`、`properties=keepalive`。
- 09:33:17绑定原生产根，09:33:18投递审计预检healthy，09:33:20开始初始化原数据库，09:35:22初始化完成；09:35:24启动对账完成并进入上午盘正常循环。启动固定点为`progress=0 resumed_sink_calls=0 foreign_lease_boundaries=0 manual_review_boundaries=11 schedule_hydrations=3`，11条不确定决策保留人工判断、未重发。
- 09:35:32自然触发的DataMode飞书消息`receipt=validated`，09:35:34确认投递事件状态提交。09:35:42、09:36:13有后续盘中扫描日志；09:36:35查询仍为同一PID64960、`runs=1`、`last exit code=(never exited)`。仅证明本条自然消息与循环推进，不泛化到所有推送。
- `lsof`核对cwd为原根，txt为已验收制品（inode151207492），生产实例锁由此PID持有（inode143554981）。启动前系统进程与锁检查均未发现旧monitor，不杀旧PID、不删锁。

## 固定制品与配置

- 制品：`target/monitor-deploy-20260914-scan.9AjKKD/release/monitor`，35981248字节，SHA256 `dea3dad22500dca238900add96d8bf820e3df325855ffd03166d191fcc60d7ae`，与09-14验收一致。
- 原生产根：`/Users/zhangzhen/Desktop/Quant/stock_analysis`。沿用原`.env`/数据库/审计/推送策略，不复制配置或凭据。
- 仓库配置：[com.stockanalysis.monitor.plist](../../../scripts/launchd/com.stockanalysis.monitor.plist)；安装位置：`/Users/zhangzhen/Library/LaunchAgents/com.stockanalysis.monitor.plist`，0600。安装前精确目标不存在，不覆盖任何旧服务文件。
- 两份plist相同SHA256：`d3e7c703357c16209fa5680e1e4785d6c3d7f53fe66ab14138772c76d2792145`；`plutil -lint`通过。无shell包装，直接执行绝对制品路径，WorkingDirectory固定为原根；PATH仅标准工具目录，无凭据；日志新建权限受Umask077约束。

## 常驻语义与边界

`KeepAlive=true`由系统保持服务，退出后再次启动；`ThrottleInterval=60`限制频繁短命进程重启，避免紧密循环。配置放在当前用户LaunchAgents，登录时加载；不依赖Codex的PTY会话。未故意杀死生产进程来测试拉起，不声称已经实测崩溃恢复。

这是用户登录会话中的进程托管，不是系统级守护或全天候硬件保证：注销/休眠/关机会中断工作，重新登录后会加载；不修改电脑电源策略。KeepAlive只处理进程退出，不证明业务线程无卡顿、行情可用或每种推送都成功。本次对账实际有11条未知投递边界，遵守应用自身恢复规则，不人工重放或改库。

当前业务限制仍在：启动health检查的`perf_recent=false`导致该检查失败，告警webhook未配置；账户快照过期，DataMode为Unsafe；RealtimeQuotes、LimitPools、T0Evidence等实际扫描返回`no_verified_batch`，依赖这些批次的计算被按原规则跳过。只记录现场结果，未据此擅自重启行情服务、修改账户时间或放松数据门。

## 日志与运维命令

- [stdout](../../../logs/monitor-launchd-20260916.stdout.log)
- [stderr／业务日志](../../../logs/monitor-launchd-20260916.stderr.log)

只读查看：

```sh
launchctl print gui/501/com.stockanalysis.monitor
tail -n 50 logs/monitor-launchd-20260916.stderr.log
```

若需要停止，先由用户确认；不要只kill PID，因为KeepAlive会重新拉起。临时卸载可用`launchctl bootout gui/501/com.stockanalysis.monitor`；若要后续登录也不启动，则先`launchctl disable gui/501/com.stockanalysis.monitor`再卸载。本文仅记录操作方式，没有执行停止/禁用。

该启动不完成完整Macro、模型/报告/渠道/定时器恢复、全部52项迁移、SQLite FD修复或gRPC数据合同验收。源码开发继续留在隔离树。
