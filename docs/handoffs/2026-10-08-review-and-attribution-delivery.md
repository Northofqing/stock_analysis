# 复盘与归因投递排查（2026-10-08）

用户补充“复盘也没有，归因也没有”。继续保留 H08/outcome/monitor、Windows841与原Schema9；H04–H06现有paper、H16周报，H01/H02/H03/H09/H10/H14/H17冻结。本文接续[前次发布](2026-10-08-outcome-backfill-and-monitor-push.md)。

## 实际缺失边界

- **paper归因**：今天15:08:47原正式实例已有 `AttributionDaily` 接纳回执。17:00前后用飞书消息GET核对，标题“📊 虚拟盘归因 2026-10-08”，343字节正文与原不可变envelope逐字节一致；消息未删除。配置指向Stock机器人p2p，成员名称zhangzhen；17:00 read_users=0。19:15再次只读核对原消息，原正文/目标/状态保持、read_users已为1。平台显示已读仍不代替人确认其当前客户端/账号；会话问题未答前保持既有目标。
- **晚间复盘**：当日调度从19:00开始。此前五个交易日40项补推扫描为23项已有Delivered、16项failed、1项NoData；本轮实际新增投递0。旧启动分支把该结果打印成“无欠账”，是错误状态描述。
- **收盘/策略/深链**：普通收盘复盘缺账户计数绑定，R02缺完整盘面源，R05/R06缺信号及分类失败结果源；G5bv2使用冻结平台owner。paper日报的回执不能证明这些能力已恢复。

## 真实取数复现

本次用当前LocalBridgeV1冻结请求，只读调用Mac `127.0.0.1:18082`，没有发送业务消息或改Windows服务。三次返回分别为：

| 复盘来源 | 实际结果 |
| --- | --- |
| TechnicalBars / 300274 / 48条 | provider未返回可保真的15分钟batch evidence；no_verified_batch |
| ProviderTopNRankings / 2026-09-30 | Eastmoney注册市场来源耗尽 |
| DragonTiger / 2026-09-30 | 原数据净额不等于买额减卖额，core金额校验拒绝 |

这些是Mac本地桥的结果。Windows任务随后只读核对7个公开合同、18个源文件快照，确认TechnicalBars正式语义为百度未复权日K与可选均线，不可替代15分钟K；HistoricalBars的Tdx/Sina/Tencent有Minute15尾部读取，但不提供任意起止日期档案。Mac核验报告SHA与JSON并回填仅限原件收件的ACK，未签发资格。

17:37 Mac用当前841的mTLS、冻结proto和原请求独立核对：Tdx/HistoricalBars/Minute15/300274/48成功返回48行，9/29、9/30、10/8各16行，原batch `tdx-smart:1791452250:5155`。原 `bar_start==bar_end`，仅保留15分钟端点，不推造区间起点、volume倍率或历史available_at。该原件只证明这一证券三个交易日，不能证明全部paper证券或完整30日。

同时，Windows的MarketDragonTiger与ProviderTopNRankings在9/30及今天10/8都独立复现净额校验拒绝和全部HTTPS来源失败。因此两项来源故障仍存在；分钟线则有真实来源，缺少Mac R12接收适配。首次手工探针把content-type写为`application/json`而被拒绝，按现有SDK实际冻结值`application/json; charset=utf-8`纠正后才得到上述结果；失败原件保留，不当成provider失败。

Windows追加原件包 `windows-h08-source-diagnosis-841e4ae-20261008.1` 已由Mac核验21成员与manifest `368aba84ace4f7b54a5a3c567a48be94ce703d1f3ad0de4fb27b8d8a86cec667`；重建原始响应96220字节且SHA相同，只回填收件ACK，未签资格。原9/30源页中688137.SH/TRADE_ID100421554的买入减卖出为-100元而源净额0；既有Core拒绝与最小重放一致。ProviderTopN的两个已登记HTTPS地址在Windows独立传输、Mac同URL/fields/header每址一次均提前关闭；Mac两次curl exit52/HTTP000/0字节，不能据此认定全局源站故障。金额/TLS/合同不放宽，841源版本保持。

## 已完成源码修正

行为提交 `6b37e5039290783623b68ab021a56457d925c327` 已推送master：

1. R07将必需的龙虎榜批次提前取得。来源失败按原GatewayError的类别、原因和retryable进入Failed，不再掉落NoData封日；必需来源失败即停止后续无法形成报告的取数。原permanent拒绝、已投递/Reserved不重取、有效空批次和计数绑定仍保持。
2. 启动补推有失败、缺数、跳过或估值失败时明确记录未投递欠账；有部分进展也不冒称全部完成。

定向 `cargo test --bin monitor r07_ -- --test-threads=1` 为4通过，覆盖临时来源失败1分钟后仍due、永久失败保持不重试、已有Reserved不重复取数及原渲染。首次测试调用私有构造器的编译错误已修正；未扩大测试或追加同目标check/build/clippy。公共合同、底层owner、Schema9目录/49政策与quote物理防护的源码均与前次发布精确相同，复用对应原canary证据。

## paper 完整性边界

现有生产R12在行情取数之前已因 `attribution_epoch_cumulative_oversell` 失败。只读原Filled算术定位两处首次超卖：002916的fill558在8/11卖300、原序列此前仅余100；002463的fill1342在9/1卖3200、此前余1500。两条对应的order_audit均为同量Filled卖出，未发现可补足缺口的既有买入审计；此为诊断，不是对成交事实作裁决。禁止改量、删单或制造seed来绕过校验。

独立的当日effective-read探针使用已发布6b37普通release库和只读detached会话，却在BR-251私有数据库快照边界失败（DELETE规范化后仍有32768字节-shm），尚未执行effective投影。此结果不能被归责于行情，也不能充当当日paper完整性通过证据。随后用 SQLite readonly backup 建立一致、DELETE journal 的私有副本；同一6b37普通release库、同一harness和当日asof只读查询明确失败 `attribution_epoch_cumulative_oversell`，原库inode未变、无provider/delivery/seed调用。因此当日R12同样受纸面完整性阻断。两次失败边界与原始账本诊断分别留存私有目录。

## 发布与待验收

`6b37e5039`普通release构建成功（1164.6秒），61家族同制品与独立launchd shadow dryrun均通过，外部进程尝试和审计回执追加均0。17:33安装静态源码及产物，future activation为17:37:32.538；保留原本地桥二进制和Windows841。原桥17:40:50完成主库初始化，18082可连接；正式monitor51723于17:41:06启动，17:42:25数据库初始化完成，17:42:26固定点恢复为`resumed_sink_calls=0 / manual_review_boundaries=78 / schedule_hydrations=17`。

monitor SHA为`481a42f04c44ddab52c4399c8c3d37c3aeb65ebd1772d1af3d195f563fbfc39a`，bridge PID50100。17:43原库inode、Schema9目录与49政策、5088原决策及envelope精确保持，78Uncertain精确保持；未新增决策。17:45飞书只读核对该次安装后无新接纳消息。Health仍unhealthy，源/风险资格缺口未由状态修正解决。17:58启动扫描最终为17项failed、0项NoData/skip/valuation_failed，明确“保留欠账，不记作完成”；17:54 R07原龙虎榜错误确实进入Failed，已有生产证据。

R12窄Minute15接收行为提交 `788d060a50c47ad5e587751485a9f06ba2daa9f1`，11文件。复用原descriptor/mTLS传输、锚定0700只读原件存储、原日线观察路径、持久owner及政策。封闭Tdx尾部请求不带起止日期，接收实际证券/Minute15/未复权/native端点、原OHLCV/batch/source_at/observed_at；拒绝未来观测，保留全部原wire及状态后解释。不可外部构造的receipt及bars SHA绑定现有R12 canonical，历史业务日排除未来价格，boll_macd统计限定同一30自然日业务窗；不以返回尾部声明完整30日或PIT。R12保持先验证paper再取provider，typed transient ledger/source失败保留retryable，完整性失败保留永久Failed和provider0。

定向验证：Minute15 lib11、backtest22、旧Day store12、旧Day mTLS reader1、真实hook1、monitor R12最终3项通过。首次monitor目标因旧展示fixture缺新增字段编译失败，补默认字段后只重跑该目标；lib未改，结果复用。18:15唯一debug真实receiver canary成功（300274/48，三日期各16根），原件437555字节/0400，batch `tdx-smart:1791454533:187`、artifactSHA `df6883fa2137c7acca20e4bd4866e07a48998d1a232114cced1e8ffebde02261`。receipt的descriptorSHA是Mac已编译client `41db4b...`，原Health/BuildIdentity中的Windows contractSHA是 `abf28a...`，两者范围不同，不能混用。

788d普通release成功（1947.1秒，1715个Git输入前后精确），monitor SHA `fb74f6f7b2f9b1b3177fc66d36fe9001487d466de239233f7acdab34dfe1aa8b`。同一优化库18:57真实接收300274/48成功，Windows原BuildIdentity为841/abf28a/7407a，Mac client descriptor仍41db4；48根日期与原bars SHA保持，原批次 `tdx-smart:1791457029:823`、artifact SHA `5a92256586f1e38846bd75bb9309a15b460d889e54f2bf2a84969a9aaaf2a4b5`。同一优化库重开真实Schema9副本后，93个catalog对象、49政策和所有表行精确保持，原库inode未变，未调用sink/reconcile。61同制品CLI与独立launchd shadow均通过，外部尝试/审计追加0，shadow卸载。

18:58安装788d，future activation为19:02:45.492709，expected_config_hash `b43e2969132d66dc724ea805ca1b244c409042febcd8a6fbc77d87ff38b0b515`；旧pids51723/50100退出。保留桥binary、原库/.env/所有原决策/catalog；monitor plist仅追加 `R12_MINUTE15_EVIDENCE_DIR` 至独立0700当前uid目录。bridge63046开始初始化，正式monitor待桥init/18082/activation齐备后启动。源码已快进并推master788d、远端核对。bridge19:06:20初始化完成、18082就绪，monitor64021于19:06:21启动，19:09:34初始化完成、19:09:36 fixed point为progress0/resumed_sink_calls0/foreign0/manual78/hydrate17。19:13回读原main/durable inode、Schema9 catalog/49政策、原5088决策全部字段与envelope、原78Uncertain精确保持。原mon plist新证据env在实际launchctl进程中确认，0700根inode保持。Health仍unhealthy，快照/心跳fresh、AccountFrozen/DataUnsafe/缺账户计数及Quote/MoneyFlow/OrderBook；业务欠缺不能被部署成功抹掉。

新正式实例正常生成两条复盘。19:15飞书GET200/code0逐字节对齐不可变envelope，且目标匹配原Stock p2p：题材催化复盘于19:11:03接纳（486字节，SHA6534693a...，消息om_x100b6343eab8f0a4c4556e4b1737efa），持仓复盘于19:11:58接纳（1174字节，SHA032fb6a3...，消息om_x100b6343e92108acdd88ab3f157019b）。两条及原下午虚拟盘归因均read_users=1；没有手工测试、重发或改接收目标。19:13–19:14正式R12实际失败为paper_backtest_failed、retryable=false、原BR-255超卖原因保持；当日R07为source_not_published ExpectedWait，retry_at21:00，未作NoData或完成。过去日期R07的源金额失败仍需源更正。

本次没有恢复缺失来源、冻结平台或正式资金链；没有制造历史资格、手工发送飞书测试、重发原Delivered/Uncertain或更改接收目标。两项普通复盘已取得真实平台接纳与已读状态；严格R12/部分来源恢复与用户指定客户端会话仍未验收。

私有原件：前次发布目录 `retained-monitor-push-release-20261008` 的 `review-after-release-diagnostics.json`、`review-diagnostic-*`、`feishu-review-attribution-1655-readback.json`、`feishu-review-recipient-readback.json`；第一次发布目录 `review-attribution-release-20261008`，第二次Minute15正式发布目录 `r12-minute15-release-20261008`，包含普通release真实接收、Schema9副本、正式init/fixed point与平台回读原件。开发工作树 `.planning/2026-10-08-review-attribution/` 留存定向验证和发布脚本；原库、凭据及未脱敏原始产物不推Git。
