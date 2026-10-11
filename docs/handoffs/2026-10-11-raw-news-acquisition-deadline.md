# raw 新闻采集期限修复与恢复接续（2026-10-11）

范围沿 [当前裁定](2026-10-07-active-scope.md)：H08、outcome 和 monitor 稳定性。冻结平台工程不作为恢复前置。

## 新发现与最小修复

晨间正式 monitor 自然日志显示四路新闻均 `external_transport_unavailable`。一轮 registry attempt 为 00:24:31 UTC，四路 failure 分别在 00:29:42、00:32:12、00:34:42、00:37:12；最长墙钟间隔约 12 分 41 秒。08:36 的 health 回读曾因超过原 10 分钟期限而报 `raw_news_source_snapshot_stale`，随后原循环又发布四路失败快照。不是进程退出或恢复成功。

源码中，raw batch 的 `join_all` 等待全部来源；External qualification 排队及连接建立不受 unary 35 秒 deadline 覆盖。本轮给每个原 raw acquisition 加上 **35 秒 cooperative 期限**，包括等待 port 的连接、资格检查和查询。不同来源并行执行，原已完成的成功或 typed error 仍保留。

超期返回 `global_news_acquisition_timeout`，stage 为 `global_news_acquisition_deadline`，retryable=true、无来源证据、无记录；在返回前完成原 breaker permit 的失败记录。原失败阈值 10、冷却 60 秒、半开单探测与后续恢复规则保留。health 恢复文件的 reason allowlist 认识这一明确本地原因。

这个期限仅限制可取消的异步等待，不是任意同步 SQL/I/O 的硬截止，不证明 Windows 已开始的 worker 停止或服务端 permit 释放。超期不能冒充原 RPC code、VerifiedEmpty、可交易新闻或上游历史资格。

## 当前安装身份与自然任务

- 本轮开始时主仓库及当前 business 分支均 `0771fd7fa091594abc747ebbc2bb6b97bb112917`，无未提交改动；修复在已有隔离工作树进行。
- 当前安装仍是 [10-10 发布](2026-10-10-push-clarity-and-account-details.md) 的行为源码 `5fb81f1f`：monitor PID89475 / SHA `d459ec8e…`，独立 bridge PID88699 / SHA `a13ed075…`；实际 command、executable inode 和磁盘 hash 匹配。activation SHA `2b2ca75f…` 未变。本轮修复尚未安装。
- 08:36 只读 health exit1，但进程、心跳及 banner 新鲜；Frozen/Unsafe、Quote/Kline/MoneyFlow/News/OrderBook 缺失。durable 1115 Delivered、4006 RejectedDurable、76 Uncertain、10 ManualResolvedRejected、pending/deliverable0。
- 10-10 日任务自然 21:17:00–21:43:23：selection0、daily0、prediction1、wrapper1，13 deferred、24 deferred windows、verified0。id37–40 的 7/11–12 周末起点和原收益保持不变；不能把 exit1 改成成功或擅自移动原日期。
- bridge 的 main descriptor 错误已有历史记录；本轮保留，不据相邻日志认定其造成 Windows 停服或恢复冻结 DB 平台工程。

## Windows 接续已读取

原 chat `R08 FuturesDelivery 上游合同与部署`（`01a0e0cf-2276-7512-96ee-3a94bdfa8ca5`）正在执行新 turn `01a12839-ef29-76e3-8bad-74158bfdd524`。V14 公共原件实际 2214 bytes / SHA `dd19717fb62e575203868f0753d12316bf0de9277c94cd38e4b2e917d4f99aad`：上游 50051 无监听、原 runtime 尚未恢复，canonical v7 在测量，sender_ready/release_go=false。

原 10-10 心跳回执实际 82621 bytes / SHA `db5ff88ad2a47be78b6ad0333fbec2219b79f6522bd397a43431a0ea10d50ac9`，明确核过昨日 Mac 26806-byte 包的精确 hash，并交付 heartbeat stop/handle 成组和唤醒等待的开发补丁；这是源码回归与接收证据，不是事故根因或发布完成。

历史 canonical v4 指标 overall 85.42%、critical 91.21% 仍保留。最新 V15 原件为 2048 bytes / SHA `eb95c58295fa32ecca487ead976350813c0bd7c3bde98dcebc4e6fe862bd0e35`，记录当时进程组仍有 1 个成员及公开 SSH 签名核验失败；它不是后续收尾回执。随后原 chat 的 cursor10 反馈测量已自然结束、overall 85.64%、critical **91.62% < 95%**，继续在已批准范围补关键模块回归。这个最新分数是对端状态反馈，Mac尚未收到该最终完整报告与原生产物料，不冒充独立逐件核证。

193 项回归与测量进展不能替代当前 95%、完整 release 检查、认证交接和业务恢复。Windows 原任务已收到清点 5→8 秒及精确系统 SSH 例外的直接人审回复；Mac 保持现有连接及私有材料，等待后续明确 sender-ready 协调。

## 验证与下一步

两项新增 pending-port 回归在旧生产 acquisition 实现上真实失败（0 passed / 2 failed）：36 秒虚拟外层守卫到期。最终代码与相关输入 hash 在检查前后相同，最小充分验证为：

- `cargo test --lib news::aggregator::raw_v2:: -- --test-threads=1`：41 passed，包括新期限、future drop、无证据拒绝、其他来源结果保留、熔断/半开恢复和原内容接纳路径。
- `cargo test --lib external_cached_ -- --test-threads=1`：5 passed，包括真实客户端资格取消释放、原缓存身份/能力拒绝与连接重启恢复。复用前一编译结果。
- 原 RED 日志 SHA `7f5bb2337de3bd112ac7710d36d9b3882ba48aa50fb56e7b0ca84a8a654a47d3`，最终 raw 日志 SHA `eb9366b9d04cd02544aa17843030c9140135f6802f4d710842f7d57911fccbff`，cached 日志 SHA `5f73445b64252186437bf6c5d52a50496f0a2a1f44109685982c3b216f75b279`；153 条现有告警保留。

本轮限定 diff 自审、内容/相对链接/格式和 diff 检查；未另启 agent 独立复核。未运行全量 suite、monitor bin 测试、release 或新业务探针，未把源码通过当作生产恢复。

后续先收取 Windows 当前完整质量与精确同版恢复材料，匹配新的可审 source/descriptor/executable 后完成 Mac 必要窄候选、兼容回退、精确 activation 人审及单实例发布，观察实际采集恢复。H08 数据合格后再有界回填；新版自然周报仍待 10-16。已有账户初始化、19 条恢复和四条人工取消不重做；剩余 76 条 Uncertain 保留。

私有证据与任务计划：开发工作树 `.planning/2026-10-11-retained-observation/`。本轮未进行新业务 RPC、发送测试消息、修改运行配置、回填、生产部署或新人工裁定。

## 提交与新增同步结果

源码/测试/本交接及主交接五件已提交 `b6063450677496d28a6ef3c4e1418daec4f10f31`、精确快进主目录 master 并 atomic push exit0；master/current business 分支远端完整 OID 实际回读一致，两个 worktree clean。原生产仍为 `5fb81f1f`。

新公共包 `client-bundle/MAC_RAW_NEWS_ACQUISITION_DEADLINE_20261011.json` 实际 12394 bytes / SHA `2d620ebcf5db155f70adc28c06e2139f6ed577fe17f1b05821635e92209027e2`，含源码身份、46项结果、原自然采集故障及 V14/V15 消费状态；本机完整写入及 JSON 回读通过，原件不纳入 Git。

发送续办消息时应用返回 `An earlier turn submission is not yet confirmed`，没有新 turn/message 标识。随后实际读回原 Windows turn 仍 inProgress，正在核验完整测量原件，尚未提及新包。**新消息送达和对端消费未确认**，不能把共享文件存在当作送达，未立即重复发送；原昨日包的精确接收回执仍有效。后继先读回队列及原任务再决定一次续办，不重复已获人审的动作或停止正在进行的回归。
