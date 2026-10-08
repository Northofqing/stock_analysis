# 保留 monitor 的竞价与盘内报价观察

用户范围：保留 H08 / outcome / monitor；冻结平台工程。Windows 841 gRPC 保持，正式资金链/P05 Unit/G5b v2 不恢复。本变更尚未部署；发布、激活、launchd 切换由根任务统一协调。

## 已核实的缺推原因

- 2026-10-08 09:20–09:24 竞价窗口的原实例无法连接 `127.0.0.1:18082`；原日志 A-02/P-05 均未确认成功。这是早上旧实例记录，13:20 的新实例尚未经历新的竞价窗口。
- 原盘中路径取得报价后等待资金流及其他工作，持仓 Scanner 在消费时看到源报价已经约 10–12 秒旧，超过原 5 秒门；时效规则不放宽。
- 账户 Frozen 拦住 I-01/T0/持仓建议；内存 alert 无 durable occurrence 的 BR-192 入口也保持拒绝。没有借用户想看消息的要求开启交易建议或伪造新账户。

## 变更

`retained_market_observation` 是独立 quote-only 调度，与资金流/LLM/私有平台任务分离；只采用 `MarketDataGateway` 的 original admitted quote。

- 当日 09:20 ≤ 时间 < 09:25：集合竞价报价观察；不回放过去窗口。
- 09:30–11:30 / 13:00–15:00：15 分钟槽定义发生身份，报告采集当时的逐条报价，不是整个时间槽统计。
- 正文使用真实账户 banner，标明 Frozen/账户缺项/数据缺项；名单附最近用户快照日期。本地持仓投影没有 broker source timestamp 时显示来源日期缺失，不把 buy_date/updated_at 变成来源证据或当前持仓资格。
- 原 quote 在 factory、治理 gate、durable 入口前按原 5 秒规则复核；缺报价、过期、未来、跨 source 日或跨调度窗均不发送。不补量比/资金流/涨跌停资格，不生成买卖指令。
- 私有字段的 `PreparedObservation` 保留原 admitted records。canonical 包含真实 provider/source/batch_id、每条 source_at/observed_at、原请求代码、逐条价格/昨收/涨幅、名单来源说明、正文 sha256。Provider origin 的 observed_at 来自真实观测、日期由原 quote 的上海交易日校验；时槽只是 producer occurrence。
- 专用 opaque gate 只豁免这份 quote-only 卡的 Frozen/global DataMode 拦截；默认 IntradayMarket/T0/AuctionVolume 建议 profile 不变。
- 复用原 Schema9 的 IntradayMarket kind / 900 秒 rolling cooldown / daily budget / immutable audit / authoritative sink / terminal settlement，policy、catalog、DDL、私有 platform origin 全部不变。
- 发送前查原 durable `inspect_exact_occurrence_owner`。已有任意 owner 关闭本槽，包括 Delivered、Uncertain、RejectedDurable，重启后新报价不能重放同槽。明确采用保守临时观察语义：RejectedDurable 当槽不再尝试；中途首次发送可能因原 900 秒冷却跨下个槽，不承诺每整刻固定送达。
- 原 startup 会恢复 Reserved 决定，因此在 `MagiclawAuthoritativeSink` 调外部进程前另做拒绝性原件复核：只读原决定 BLOB 和 envelope hash，核正文/source canonical hash、Provider 的 observed_at/as_of/原 batch_id，以及每条原 source_at/observed_at 的 5 秒、非未来、同源日、当日原窗条件。过窗、同槽已旧、缺项或非法均 TypedRejected/retry=false，经原底层结算；存储 JSON 不构造 Admitted capability、不解除 L5。其他 kind 保持原路径。
- Tencent raw batch observed_at 是 Unix 小数秒；exact raw 字节保留且 hash 绑定，只检查原非空，不重新解析为资格时间。时效资格只取 sealed quote 的 original normalized source_at/observed_at 与对应 Provider observed_at，仍按物理调用时点的原 5 秒规则检查。

## 验证与证据

证据目录：`/Users/zhangzhen/.local/share/stock-analysis-candidates/monitor-observation-20261008`。

- 最终 monitor 检查 7/7（含物理 guard）通过，2 个显式 opt-in ignored 不计入普通通过数；完整证券名单 2/2、Schema9 7/7、Scanner 原 5 秒相关检查 6/6 通过。`physical-guard-monitor-receipt.json` 记录最终目标；原 101 编译错误回执保留，修正了 TypedRejection 直接字段访问。
- actual copy readback 只有显式 opt-in ignored test；测试数据库和 quote 审计主库位于 `data/test/TEST_CODE_RETAINED_OBSERVATION_REAL_READBACK*`，original DB 只做 RO backup。行情实际调用真实 Gateway；发送端是 TEST_CODE memory sink，没有飞书调用。
- actual copy readback 1/1 通过（89.8 秒）：三个独立原历史 Schema9 copy 分别模拟 Accepted / Uncertain / Rejected，opaque source / Provider origin 在实际 Schema9 正向准入并结算为 Delivered / UncertainManualReview / RejectedDurable；重开 exact owner 一致，各 sink 仅调用一次，6 秒后原始 quote 全部拒绝。
- 三批实际 admitted Tencent quote 各 5 条，source_at 分别为 14:04:06 / 14:04:36 / 14:05:09 CST，observed_at 为 14:04:09.751310 / 14:04:39.154638 / 14:05:12.698267 CST；canonical 保留每条原始值和 `tencent-web` 原批次 ID。14:00 槽只是 occurrence，不是源时刻。
- `live-readback-receipt.json` 逐项核对：每份 copy 的原 5,082 decision canonical 和 state 全保留，原 78 Uncertain 全保留；catalog SHA256 `189981e847cea1bed843fde41bd8ef790f4af36bb7e89fbe2d48a780c7621b2e`、49 policy、Schema9 不变；正式 main/durable 的 dev/inode 均保持，飞书调用 0，没有账户或持仓种子。该证明限定隔离底层行为，不代表生产已投递或客户端已收到。
- `original-schema9-owner-preflight.json` 另以 `mode=ro` / `query_only` 查正式原库：Schema9 / 49 policy / 78 Uncertain；当日 IntradayMarket/NONE/GLOBAL 决定 0、新 retained occurrence owner 0；原 policy 900 秒且计入 daily budget。正式 durable dev 16777220 / inode 154266271；正向准入限定上面的原库副本 canary。
- 第四份原库 copy 的最终真实物理 guard canary 1/1 通过（15.57 秒）：实际 Tencent source_at 14:31:49、observed_at 14:31:53.273194 CST，14:31:53.285379 以真实 `Utc::now` 对原 canonical 正向通过；Provider 原准入后成为 Reserved，等待 6 秒后 14:32:02.583737 仍在同一 14:30 槽，经真实 `MagiclawAuthoritativeSink` 在外部调用前拒绝为 `retained_observation_original_quote_expired` / retry=false，并原底层结算 RejectedDurable。新 fingerprint `9e5fcb2471ac6517a1a17548ff0eda040a446a490ced8a93fec4ba4ead93c347`；`physical-guard-readback-receipt.json` 核对原 5,083 决定和 78 Uncertain、catalog、49 policy、Schema9 全保留；飞书调用 0。Test transport 保护无外部副作用，source origin 仍是原 Provider，没有伪时刻。
- 真实 readback 初次的 raw decimal 格式误判与随后一批原报价陈旧 9.526821 秒均留有 `failed-native-time-*` / `failed-stale-native-*` 失败证据；都在 prepare 前失败，原第四 copy 无新 Reserved，新取真实 fresh 原件完成上述验证，不是重发。
- 默认 Frozen 门的证据来自具体 metadata 检查，不用 canary 对泛型账户 gate 做无证据断言。新入口没有修原 Scanner 的 MoneyFlows 前置延迟或 BR-192 无绑定 alert；止损与交易建议依旧需要真实账户/报价资格。
- 正式上线还需普通 release、activation、同库/同 inode 单实例切换及自然盘中回执；下个真实竞价窗口才能观察线上竞价。
