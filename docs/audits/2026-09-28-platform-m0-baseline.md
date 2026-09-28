# 平台完整路线图 M0：第一轮事实基线（2026-09-28）

> 采样时刻：2026-09-28 21:58 CST。范围为本机只读源码/launchd/运行根/日志检查；未操作生产 DB、未触发投递或重启。此文是 M0 的第一轮证据，尚不是完整生产验收。

## 1. 源码、运行制品与目录

| 项 | 第一轮观测 |
| --- | --- |
| 本仓 | `master@ac089275b5057d4d90478121ab80f1fcdd64f596`；完整路线图在当前任务中待提交。 |
| monitor | `gui/501/com.stockanalysis.monitor` 为 `running`，PID `17089`、`runs=2`，program 与工作目录均在 `/Users/zhangzhen/.local/share/stock-analysis-runtime`；binary SHA-256 `a2589f0115d6f3ee89bf11d7714c7bd4bce58f6d3e5bcbc12b06831ffc5b8b34`。 |
| 本地桥接 | `gui/501/com.northofqing.grpc-market-server` 为 `running`，PID `56417`、`runs=1`，同一运行根；binary SHA-256 `2546b74d3af6929b5a08de4303f506232c9988b8ff030ec994f45b642cd4b1e4`。 |
| 激活 | 运行根 `config/selection/selection_activation.v1.json` 原始文件 SHA-256 `2897d98558146b001eab3c26193882093adc2c097c29a989115cb6906283c814`。这里只证明文件身份，尚未重新探测生效条件。 |
| 公开 bundle | 运行根 `client-bundle/manifest.sha256` 原始文件 SHA-256 `cc3d97239da5bc224e487eef355b7154901739bafd76d64f78ab6337cad9f89f`；metadata 版本 `2026-09-28.2`、source revision `4e4995f8d3f2c7cd504d1dec0f238e6d4b4fc02c`、raw descriptor `0c4485545dbfd0979a7d5ea206c840f39fd504ed62fb7eef92f1940bdc9c2f41`、VM binary 身份 `517e0b4c31bb42330f4bc2a0395e3af385a164212414a65775bed40c9eb87ae3`。这只是本机公开元数据，真实 VM Health 仍须同请求核对。 |
| DB | 运行根主库 `stock_analysis.db` 约 2.22 GB，durable DB 约 79.8 MB；二者均可能有活动写入，本轮未对活动 DB 做 hash 或 `integrity_check`。 |

复核入口：`git rev-parse HEAD`、`launchctl print gui/501/<job>`、`shasum -a 256`、运行根公开 metadata。生产切换历史见 [launchd 迁移记录](../ops/2026-09-28-monitor-launchd-desktop-tcc-recovery.md)。

## 2. 推送目录版本漂移

使用 `notify.rs` 的 `PushKind` enum 定义与冻结 JSON 的 `kinds` 集合做集合差分：源码 **66**，冻结 [catalog v1](../push-system/push-capability-catalog.v1.json) **65**，唯一新增项为 `NewsAiAnalysis`；旧 65 项均未消失。冻结目录另有 102 producer、52 Unit，已有 `news-ai-same-tick → MU-news-ai`，但仍把该 producer 的 kind 记成旧 `NewsToIdea`；[current-source v2 状态增量](../push-system/push-current-capability-status.v2.md)也不覆盖新 kind。下一步必须版本化对账 kind 到现有 Unit/owner 的变化，不修改冻结 v1。

代码中有 NewsAI counted binding 与审计链；“counted kind 已接线”不等于 `MachineCatalog` 已覆盖、生产接收或 52 Unit 中的某项完成。可重跑的 `python3 scripts/push_catalog_drift.py` 已通过：固定旧目录 SHA，逐项核对当前 enum、唯一新增 kind、原 producer/Unit 关系及源码证据片段。[v2 当前 kind 增量](../push-system/push-current-kind-delta.v2.json)将 `NewsAiAnalysis → news-ai-same-tick → MU-news-ai` 标为源码 ACTIVE、生产 UNVERIFIED；运行时 `MachineCatalog::bundled()` 仍固定历史 v1，尚未完成当前目录接管或真实渠道对账。B02 的部署/回执两栏继续待证。

## 3. 2026-09-21 审计十二项的第一轮重核

下表只判代码形态；部署与自然运行证据另核。原审计是历史发现，不能照抄为当前缺陷。

| 原项 | 2026-09-28 第一轮状态 | 当前证据与剩余验收 |
| --- | --- | --- |
| #1 模拟盘逐笔成本 | 代码已修，生产口径待核 | `src/performance/fee_evidence.rs`、`src/trading/paper_sell.rs` 已按 FIFO 分摊买费并输出净收益；需与当日卡片及坏价裁定对账。 |
| #2 回填/回测同成本 | 部分修复 | `position_tracker.rs` 使用 `fee_evidence`；隔离工作树的 `strategy/core.rs` 已去掉 realistic 包装层重复买卖费调整，100 股 10 元零滑点往返净现金 `-11` 元与 `fee_evidence` 一致，定向 8 个 realistic/qualified 测试通过。尚未形成共享 FillModel、全部价量/印花税配置 parity 或生产证据。 |
| #3 DB 初始化/测试隔离 | 代码已修，测试稳态待核 | `DatabaseManager::init` 在非测试模式尊重显式路径，测试模式拒绝显式路径并提供隔离入口；仍需定向验证原 flaky 家族。 |
| #4 NewsAI counted binding | 代码已修，生产送达待核 | `notify.rs` 的 `NewsAiAnalysis` 走 counted 准入和专用审计；补机器目录及真实 receipt。 |
| #5 NewsAI 批次重复身份 | v3 路径已修，旧记录待核 | `news_ai.rs` 有 `NewsAiIdentityV3` 和恢复 envelope；旧版本兼容/重复计数及生产结果待核。 |
| #6 Gateway 错误归类 | 关键已知码已修，穷举未完成 | `raw_v2.rs::classify_gateway_error` 已单列 route exhausted/stopped、source precondition、external transport 等，仍保留未知码 fallback；需从当前失败样本验证原始 retryable 保真。 |
| #7 指标测试 | 代码已补基础测试，覆盖待核 | RSI/MACD/KDJ/cross/divergence 五文件均有测试；不据存在性宣称边界充分。 |
| #8 NewsFlash gate 拒绝审计 | 代码已修，生产计数待核 | `news_aggregator_init.rs` 在校验失败处调用 `record_gate_rejection`；需用运行数据核分母与原因。 |
| #9 铜箔文案/规则 | 代码已修 | `config/chain.toml` 已使用“铜箔”；来源规则有效性需正常样本验证。 |
| #10 项目上下文 | 代码已修 | `CLAUDE.md` 已列主要模块、运行根、部署和无券商边界。 |
| #11 Frozen/数据质量否决 | 代码已修，端到端待核 | `GovernanceEngine` 对 `frozen_mode_respect` 与低于 `data_mode_min` 的交易动作类拒绝，`v14_adapter` 对动作类配置 Degraded、其它保持出声；需调用路径/真实 mode 样本验证。 |
| #12 账户指标桩函数 | 代码已修，生产状态待核 | `compute_account_mode_metrics_blocking` 读取账户摘要、时效和 paper 账本，旧“无条件 Err”结论过期；缺数、周末与真实持仓摘要时间仍须验证。 |

## 4. 当晚生产数据通道样本

22:13 CST 执行运行根现有 `target/release/monitor --health --json`（只读，退出码 1）：`status=unhealthy`、`reason_code=banner_unhealthy`、`monitor_running=true`、`snapshot_fresh=true`、`account_mode=Frozen`、`data_mode=Unsafe`、`account_metrics_complete=true`，缺 `Quote/MoneyFlow/News/OrderBook`；该命令明确只覆盖 banner account/data。相邻运行日志每 30 秒仍出现 `PaperLedger is not activated: explicit seed/cutover binding required`。这证明当前模拟盘账本尚未完成显式激活，且夜间健康不佳；不能由单次 banner 推断盘中持续故障或给出自动 seed。需把夜间 Quiet 语义与账本切换分别验收。

只读解析运行根 `logs/monitor-launchd.stderr.log` 末尾 4 MB，按 `[DataGateway]` route/outcome/reason 统计；它不是完整交易日分母。`GlobalNews-Eastmoney` 在样本中有 `available/accepted`，符合 19:12 后恢复记录；另有以下未关闭现象：

| Route | 末尾样本 | 解释边界 |
| --- | --- | --- |
| `R-08-announcements` | 初采尾窗 102 次 `invalid_request`，21:50:41 CST 最后一次；稍后仍重复。本地下游请求为 `{"start":"2026-09-28","end":"2026-09-28","limit":300}`。VM 用相同参数直接调已部署服务返回 `ADMITTED/complete/300`；本机代码的 ExternalV1 adapter 在发 RPC 前把该 operation 判 `Unimplemented`，故原 `invalid_request` 是本机合同接线缺口。 | 本机需按公开 schema 补 `external_v1` builder、方法映射及 transport 路径，并用同版 bundle 与真实 RPC 复验；之前的日志不证明服务端故障或无公告。 |
| `BlockTrades` | 54 次 `no_verified_batch`；21:50:41 旁路日志见 `The operation was cancelled`。 | 尚不能判定服务端、桥接还是调用并发取消；先保留 request_id/同源 trace。 |
| `R-08-global-indices` | 17 次 `invalid_evidence`，最近 19:21:52 CST。 | 需要拆原始字段/客户端 converter 规则；不能直接归咎 VM。 |
| `R-08-global-fx` | 17 次 `no_verified_batch`，最近 19:22:00 CST。 | 先核当次来源可用性及覆盖，不能用空结果替代。 |
| `Consensus` | `no_current_reports` 与 `invalid_evidence` 均出现。 | 前者可能是合法无当期报告，后者需要逐证券核验；聚合计数不能代表全局故障。 |

另两组既有上游合同缺口（普通 HistoricalBars 异常日线发现、D17/D20 权威交易/停复牌事实）已交 VM gRPC Codex。VM 第一轮核查只证明 300005 的 HithinkFinance 已准入 63 条及 688277 日线断档，尚不能证明被拒 TDX 原始批次或逐日权威停复牌；已补发精确 TDX 请求及证据要求请其继续开发。下游保持 fail closed，收到服务端代码、公开 bundle 和真实 RPC 回执后再做消费端同版接线。

## 5. 下一批工作

1. B02：当前源码 enum/catalog/Unit 差分已有脚本与 v2 增量；继续补生产部署、真实回执、enum 外路径和物理 owner 对账，不由静态目录推断生产送达。
2. B03：为上述 `部分修复/待核` 项补最小相关测试和生产只读证据；已修且证据充分的项停止重复开发。
3. B04/B05：核对启动链完整性验证耗时、桥接就绪前欠账重试以及当晚各 route 的具体请求错误。日志样本要用时间窗口和 request_id，避免仅按末尾 4 MB 做因果判断。
4. 等 VM 回复 D14、D17/D20、MarketAnnouncements 的唯一 owner、合同身份及可用字段；本仓只消费有来源证明的同版结果。
