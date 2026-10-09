独立看门狗手机告警配置
======================

`monitor_watchdog.py` 保留原本的本地告警和独立 launchd 运行方式。手机侧目前实现 Bark；默认没有配置，不产生网络调用。这个功能只负责独立运维告警，不进入业务推送 owner，也不改数据库。

配置必须由用户选定通道并明确启用。将 `watchdog-mobile.example.json` 复制到仓库/版本包外的用户私有目录（目录 `0700`，文件 `0600`，均不可为符号链接），填写自己的 Bark `device_key`，设 `enabled=true`，再向 watchdog 命令显式增加 `--mobile-config /Users/zhangzhen/.local/share/stock-analysis-runtime/data/private_config/watchdog-mobile.json`。正式 launcher 已固定使用这个可缺省的私有路径；未来只需明确配置该文件。不要把凭证放进 `.env`、plist 参数、URL 或版本包，也不要写入运行根 `config/`：该树全部属于 selection activation 的编译输入，新增凭证文件会使主链激活失配。模板处于禁用状态，空密钥不能启用；本次开发未创建配置、账户或发送真实手机消息。

默认仅发送 P0/P1；P2 继续留在本地。`severities` 可以显式增加 P2，启用配置必须保留 P0。`daily_self_check=true` 允许每日一次本地落档/回读成功的手机自检；这是本地工具的自检，不表示监控业务正常。每天的原始本地自检文件保留 `mobile_delivery=not_attempted`，后续手机结果独立记在 `latest-mobile.json` 和 `mobile-receipts/`。

接线使用 [Bark 官方 JSON API](https://github.com/Finb/bark-server/blob/master/docs/API_V2.md) 的 `POST /push`，密钥在 JSON body 内。仅 HTTPS；测试夹具可显式允许 `http://127.0.0.1:<port>/push`，不允许远端明文。客户端没有代理、重定向或隐式重试，每次请求硬期限 5 秒、单 socket 操作 2 秒、响应体上限 2 KiB，每轮最多 3 次请求。

事件先落档，再由成功持久化的本地 `state.json.mobile_outbox` 引用，之后才允许网络发送。未提交的孤立事件不发送。每个事件 hash 对应私有 receipt；发送前先持久 `InFlight`，重启发现未完成尝试时记 `Unknown`，不会重新发送。整个过程持有原 watchdog 锁，重叠运行不会形成第二个 owner。

状态口径：

| 状态 | 证据与后续 |
| --- | --- |
| `Accepted` | HTTP 200 且有限 JSON 解析得到整数 `code=200`；只表示 Bark 服务受理，手机送达及已读未观测。 |
| `FailedRetryable` | 子进程没有启动，或 TCP/TLS 连接阶段确定失败且 HTTP request 尚未调用；60 秒、240 秒退避，最多 3 次。 |
| `FailedFinal` | TLS 证书验证拒绝；或 [官方单设备路由](https://github.com/Finb/bark-server/blob/master/route_push.go) 的 HTTP 400 + `code=400` 发送前拒绝；或确定失败已达重试上限。 |
| `Unknown` | 请求调用后异常、超时、进程中断、500、重定向、不合格/过大响应；留 `manual_review_required=true`，只供核对，不自动重发。配置变化也不解除。 |
| `Expired` / `Suppressed` | 事件超过本地 15 分钟有效期、时间在未来、当前事件已恢复/升级，或不在手机分级策略；不补发。 |

Bark 的 `ttl` 是归档保留时间，不能用作发送资格；有效期由本地消费者独立检查。既有 `Accepted` 和 `Unknown` 不因过期、重新导入配置或重复事件引用而重发。同一次事故原因/级别的 `Unknown` 还会阻止下一次小时提醒，持久 subject 指针不会随着 24 小时 outbox 清理失效；新的级别升级、已核实恢复，以及恢复后再次发生的事故是新的观察范围，后者由持久 `generation_sha256` 区分。各 receipt 和 `latest-mobile.json` 只包含固定状态/原因、事件 hash、尝试数和时间；不存 endpoint、密钥、远端响应正文、原始健康输出或持仓。`Unknown` 的人工核对材料就是原事件和该 receipt，不提供一键清空重发命令。

本地结果与手机结果分列：`local_observation_persisted` 在任何手机配置读取/调用之前产生；未传配置和显式配置路径文件不存在均显示 `configuration_status=unconfigured`，配置权限、符号链接或内容不合格显示 `invalid`，均零网络。手机 receipt 写入失败时 exit 2，并明确提示本地观察保留。尚未配置手机凭证时，不能宣称手机告警已上线或手机验收完成。

相关验证：从 `scripts/` 运行 `python3 -m unittest test_monitor_watchdog test_watchdog_mobile`。HTTP 用自有 loopback 假服务验证，无真实推送。
