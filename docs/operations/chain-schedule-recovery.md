# 盘前／盘后产业链报告跨重启状态

monitor 分别按自然日和 `preopen`、`postclose` 记录发送状态，数据库位于生产工作目录的 `data/chain_schedule.sqlite3`。报告保存在 `reports/chain_analysis_schedule_YYYYMMDD_<phase>.md`。CLI `--chain-analysis` 仍是单次调用，不使用这份调度状态。

在通知发送前，monitor 写入 `sending`。渠道只提供弱接受结果：如果进程崩溃或发送返回失败，`sending` 保持不变，重启后也不自动重发，以免外部已经接收而再次推送。若至少一个渠道返回接受，状态转为 `weak_accepted`，同一天同一时段不再运行。分析、取数、保存报告或无可用渠道在写入 `sending` 前失败时，窗口内可自动重试。

查看状态：

```bash
target/release/chain_schedule_resolve --phase preopen --date 2026-09-24
```

若状态为 `Uncertain`，先核对对应时段报告和所有已配置渠道的发送日志，再人工裁定。确认已送达时封口；确认未送达时授权窗口内重试。两种裁定都要求写明核对依据，裁定历史保留在数据库中。窗口已结束时，`retry` 不会补发。

```bash
target/release/chain_schedule_resolve --phase preopen --date 2026-09-24 --resolution delivered --note '核对渠道回执：已接收'
target/release/chain_schedule_resolve --phase postclose --date 2026-09-24 --resolution retry --note '核对各渠道日志：均未接收'
```

`weak_accepted` 只是渠道方法的接受信号，不证明所有目标最终收取或阅读。该状态机制解决的是调度重复执行与不确定结果自动重发问题；它不能替代带稳定目标身份和权威回执的 counted 投递。
