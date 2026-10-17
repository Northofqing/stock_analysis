# 对《Windows 侧数据缺口清单与盘中异动监控需求》的评估回执（Windows 侧）

- 对应来单：`2026-10-17-windows-data-gaps-and-intraday-alert-request.md`
- 回执方：Windows 侧 ExternalV1 数据面（现役部署身份与复现命令见 §7）
- 回执日期：2026-10-17（Asia/Shanghai）
- 性质：评估与信息补全。**不含新协议批准、不含未拍板的生产修复承诺**；本文所有"实测"仅代表 2026-10-17 凌晨对现役实例的有界探针状态，响应帧与命令均可复现（§7）。

## 0. 一页结论

| 来单条目 | 我方核实结论 | 归属 / 下一步 |
| --- | --- | --- |
| P0-1 QualifiedTradingFacts | 按现规格无法交付（本侧无权威源，详见 §2.1） | 需 Windows 侧 operator 拍板路径（立项新授权源 / 来单方发新规格单）；编号纠正如 §1.1 |
| P0-2 complete=true | 现上限内不可能为 true；属覆盖 v2 的设计内行为（§1.2） | 二选一：我方提额（新 Gate A）或来单方按 v2 覆盖语义消费；待 operator 拍板 |
| P0-3 op 61 `stage=live` | live 通路本身可用；"与 stage=final 一致"其实**已成立**；报错源于来单方解析假设（§1.3） | 来单方解析器改吃 `{value,unit}`；我方补合同文档样例；周一窗口双向留证 |
| P0-4 T0Evidence | 我方侧可用（实测 `records=1`，§2.4） | 待来单方盘中复现 `records=0` 并附原始片段 |
| P1-1 BlockTrades | 我方侧可用（实测 10 条，§2.5） | 来单方请求信封接线；若接线后仍失败请给 reason_code 细类 |
| P1-2/3 汇率 / 全球指数 | 我方侧可用（6 指数 + 8 汇率全绿，§2.6） | 同上 |
| P1-4/5/6 push2 族 / R-09 | 真故障，根因在网络层；我方侧无法修复（证据链 §2.7） | 网络/代理层修复（Mac 侧）；我方出证据链面交件 |
| P1-7 cffex confirmed | Confirmed 为设计边界；planned 部分已通（§2.8） | 若确需 Confirmed，属新权威源立项 |
| 第四章 盘中异动监控 | 基础（监控循环 / 订阅服务 / 三类异动准入）**已存在**；路线 A 不可行；路线 B 需增量设计（§5） | 我方起草 Gate A 设计；来单方确认条件模型与 block_trade 预期 |

**真正需要 Windows 侧动手的只有三项**：op-61 合同文档样例补丁、push2 证据链面交件、第四章 Gate A 设计。其余条目为来单方接线/解析动作，或 operator 决策。

## 1. 三处先纠正

### 1.1 编号：Operation 66 已被 TradingCalendar 占用

来单 P0-1 记 "`QualifiedTradingFacts`（Operation 66）"。现役合同（`dba66a27…`，随 2026-10-16.1 client-bundle 交付）中 **Operation 66 = `TradingCalendar`**（2026-10-15 启用，SSE 签入修订）。若 QualifiedTradingFacts 立项，号应自 **67** 起。请来单方核对自身 pin 的合同四字段（§7）——若 pin 仍是旧包，其 op 视图与已交付合同不一致，建议先行重钉再对表。

### 1.2 P0-2 是设计内行为，不是回归；"分页约定"已有答案

覆盖 v2 设计原文（本仓 `docs/integrations/grpc-market-announcements-coverage-v2.md`）："A bounded prefix must not advertise complete coverage of the source's declared result set." 现上限 = 页大小 30 × 最多 10 页 = 300 条/查询，caller limit ≤ 300。

实测（请求区间 2026-10-15，执行于 2026-10-17 凌晨；limit=50）：源侧声明 1097 条，实际翻 2 页 60 条，`complete=false`；quality issues 原文：`source pagination incomplete: inspected 60 of 1097 declared rows`。**当天 1097 条 > 300 上限，故 `complete=true` 在现合同下不可能出现。**

来单中"若协议上分批是本意，请告知分页约定"一项——**已有答案**：`schema_version=2` 的覆盖 envelope 已在线上（记录 schema `magic.market.market_announcements.coverage` v2；实测响应含 `coverage_scope` / `exchange_event_universe_complete` / `pit_guarantee` 与逐页 SHA 证据），可直接消费。

两条可行路线（待我方 operator 拍板后在后续回执中选择）：
- (a) **提额**：页预算 10→40、caller limit 300→1200（并复核部署 payload 上限 512 KiB 是否需同批调整）——典型日可达 `complete=true`；**极端峰值日仍可能超界，无界保证不存在**；
- (b) **前缀消费**：来单方按 v2 覆盖语义接受"有界前缀 + 明确覆盖面"。我方不会以求真名义把 `complete` 假报为 true。

### 1.3 P0-3："与 stage=final 一致"按实测不成立——但真实结论对来单方有利

实测（2026-10-17 凌晨，`stage=final` 帧）：记录中 **4 个比率字段为 `{"value","unit"}` 映射**——`auction_pct`（Percent）、`auction_turnover_pct`（Percent）、`auction_volume_ratio`（Decimal）、`auction_yesterday_ratio_pct`（Percent）；其余 8 个数值字段（auction_price / pre_close_price / auction_volume_shares / auction_amount / auction_unmatched / float_market_cap / last_price / open_price）为裸 f64 或 null。

同一合同文档（`grpc-external-api.md:612、:651`）对 `change_percent` 等同类字段**明文就是 `{"value","unit"}` 形态**——这是全 API 的既有形状，并非 live 特有。`stage=live` 与 `stage=final` 由同一构造器产出，字段形态一致；来单报错 `invalid type: map, expected f64` 是 serde 对 `Option<f64>` 反序列化映射时的标准报错，指向来单方解析器把 4 个比率字段声明成了标量。

**建议修复（零契约变更）**：来单方解析器对这 4 个字段接受 `{"value": f64, "unit": "Percent"|"Decimal"}` 或 null；我方把"非空比率字段"样例补进 op-61 合同段落（文档补丁，不晚于下周）。字面 `map → f64` 改法不推荐：会丢 unit 语义、与全 API 现有形状冲突，属记录合同变更（需版本化与双侧重钉）。

**周一（10-19）双向留证**：来单方按计划输出字段级观测（首个不符字段名 + 实际类型 + 原值片段）；我方已在窗口内排定 09:19 / 09:37 的 live / final 探针，可提供 live 原始帧对照。

## 2. 逐条核对（证据）

### 2.1 P0-1

立场与我方 2026-10-15 回执一致（`client-bundle/WINDOWS_QUALIFIED_TRADING_FACTS_AUTHORITY_REPLY_20261015_V1.md`）：本侧没有任何停牌来源；`listed_on`（TDX 财务 / 同花顺元数据）与价格带（行情自带批次值、TDX 显式标注不可用）非权威且无源时间。按来单 §2 的证据要求**无法交付**。

可行路径二选一（均需 operator 拍板）：
- 立项新权威源（候选见本仓 `docs/integrations/research/2026-10-08-h08-historical-status-product-requirements.md`：上交所 SSEINFO 历史/静态产品、深交所 FTS 静态交易参考——**均未在授权内**）；
- 来单方发新规格单，接受"provider-sourced 观察值"中间态（语义、字段白名单、校验放宽点写清）。

请与本交接目录 `2026-09-27-d17-d20-authority-trading-facts-vm-handoff.md` 的验收要求对齐后更新该单状态。

### 2.2 P0-2

见 §1.2（含实测帧与两条路线）。

### 2.3 P0-3

见 §1.3（含最终/实测字段形态与修复建议）。

### 2.4 P0-4 T0Evidence

实测（2026-10-17 02:22:38 +08；600519；request schema v2）：`records=1`，含 quote（price / previous_close / OHLC / change_percent / volume / amount）与五档 `order_book`，`complete=true`。

服务端日志同期有一条 `t0_evidence_degraded` WARN：`structurally absent fields: security_name, source_at`——这是源侧结构性缺失（TDX 不提供源时间与证券名称字段），按 BR-050 保持 null，**不是数据缺口**；若来单方把 null 计为缺数据，请按此口径区分（不得把结构性 null 与 Unavailable 混成同一状态）。

"Mac 侧 `records=0`"待周一盘中复现；请附原始请求/响应片段与服务端 request_id（以便在 `grpc-server.stderr.log` 对位）。

### 2.5 P1-1 BlockTrades

实测：`{"instrument": 600519, "limit": 10}` → `grpc_status=0`、`complete=1`、`records=10`（trading_date 2026-10-15 / 10-14 / 08-31 / 08-03…，含 price / close_price / premium_ratio / volume / amount / 买卖营业部与完整 evidence）。**我方侧无故障**。

来单方请求信封（其 `2026-10-14-all-external-routing-design.md` 所列 `build_external_query_request`，当时仅 6 个 op）未含该 op——请先接线；若接线后仍 unavailable，请给 reason_code 细类（`external_capability_missing` / `unadmitted` / `runtime_unavailable` 或 provider 错误）与原始片段。

### 2.6 P1-2 / P1-3

GlobalIndices：6 指数、`complete=1`、provider=Sina；ForeignExchange：8 汇率对、`complete=1`、provider=Sina（带 source_at）。同 §2.5，属来单方接线侧。

### 2.7 P1-4/5/6（push2 族）与 R-09

根因证据（2026-10-17 凌晨，本机直测；与任何客户端实现无关）：

| 目标 | 结果 |
| --- | --- |
| `push2.eastmoney.com` HTTPS / HTTP | 连接不可用（含真实 IP 直连） |
| `push2delay.eastmoney.com`（含真实 IP 直连） | 连接不可用 |
| `push2.eastmoney.com` IPv6 | 不可用 |
| 同族 `push2his.eastmoney.com` | 200 |
| 同族 `push2ex.eastmoney.com` | 200 |
| 对照 `datacenter-web.eastmoney.com` | 200 |

DNS 侧：本 VM 的系统解析（Parallels 本地 DNS，即 Mac 宿主网络）将 push2 系列解析为 `198.18.x.x`（fake-IP）；即使绕过 DNS 直连真实 IP 亦失败。TLS 阶段现象：两次 renegotiation 后 `peer closed connection without sending TLS close_notify`。

受影响 op 族（不止来单所列两个）：`ProviderTopNRankings`、`BoardFlows`、`PostCloseFlows`、`MarketRankings`、`Popularity`、`FundFlowSeries` 分钟线。**修复点在网络/代理层（Mac 侧路由规则）**，我方数据面代码无需修改；我方将整理完整证据链（含复现命令）另件面交。

### 2.8 P1-7

FuturesDelivery v2 实测：`complete=1`（按 v2 合同为 IF/IH/IC/IM 四条品种的 2026-10 计划交割日，batch `cffex-equity-index-planned-delivery-2026-v2:10`）。**Confirmed 属设计边界**：wire 文档（`grpc-futures-delivery-v2.md`）原文 "This v2 response authorizes planned-calendar display only. It must not authorize a same-day 'official notice', 'confirmed delivery', …"。如需 Confirmed，属新权威源立项（同 §2.1 路径）。

## 3. 已就绪清单（需要来单方动作）

1. **P0-3**：解析器改吃 `{value,unit}`（4 个比率字段，可空）；我方同步补文档样例；
2. **P0-4**：盘中复现并附原始片段 + request_id；
3. **P1-1 / P1-2 / P1-3**：把三个 op 接进请求信封（或给接线后的 reason_code 以定界）；
4. **合同重钉确认**：pin 的四字段与 op 视图（§1.1）；
5. **第四章**：确认条件模型（§5.4）与 block_trade 预期（§5.3-5）。

## 4. 做不了清单（原因与替代）

| 项 | 做不了的原因 | 替代路径 |
| --- | --- | --- |
| P0-1（按现规格） | 本侧零停牌源；上市日/价带无权威+无源时间 | 立项新授权源 / 来单方新规格单（均待拍板） |
| P0-2（无条件 complete=true / 一次性全量） | 300 条上限 vs 1097 条/天；提额也只是有界覆盖 | 提额（典型日）或 v2 前缀消费 |
| P1-4/5/6（push2 族，含 R-09） | 已证明与客户端无关（裸 HTTP / 真实 IP / IPv6 全断） | 网络/代理层修复（Mac 侧）；我方出证据链 |
| P1-7（Confirmed 原件） | 需交易所月度 Confirmed 授权源 | 立项（同 P0-1 路径）或按 planned-only 消费 |
| 第四章路线 A | TQ-Local 无预警条件读写接口；GUI 自动化不在允许面 | 取路线 B（§5） |
| 第四章 block_trade 盘中触发 | 现准入源为（半）日频数据 | "新记录出现即报"；或澄清预期后新增源 |
| P0-3 字面 map→f64 | 能做但**不应做**：丢 unit、破全 API 形状、动合同 | 来单方改解析（§1.3） |

## 5. 第四章评估：盘中异动监控

### 5.1 路线判定

- **路线 A（TDX 原生预警抓取）：不可行。** 本仓对 TDX 的接入仅限官方 TQ-Local loopback 行情接口（固定 `127.0.0.1:17709`），没有预警条件的程序化读写通道；GUI 自动化不在允许面内，也不满足可复现证据要求。
- **路线 B（Windows 侧自算）：可行，且基础已存在。**

### 5.2 已有基础（不需要新开发的部分）

- `MarketEventService`（v1 proto 已冻结、已部署）：`SetWatchlist`（全量替换）、`Subscribe`（含 instrument 过滤）、`Replay`、`GetListenerStatus`（max watchlist、`admitted_event_families`、replay 窗口与计数）；
- 盘中监控循环：tdx-agent + 监控器已在运行；行情源 = TQ-Local（与 `RealtimeQuotes` / `T0Evidence` 同源）；
- 三类 LocalAnalysis 异动（价格 / 金额 / 量）带 Core 证据的 trigger/rearm **已准入**；状态类消息仍 `UNADMITTED`；
- 2026-08-15 有真实联调证据（watchlist 替换 → 新 generation → Replay 收帧）。

### 5.3 与来单规格的差距（需设计/开发的部分）

1. `alert_type` 集合：`price_breakout` / `block_trade` / `custom` 不在现有三族内（现有为价格/金额/量变动）；需映射或扩展规则族（扩展族 = 新准入）；
2. 条件下发模型：来单为逐条件（threshold / direction / `expire_at`）+ `replace_all`，与现有"全局 watchlist + 规则策略"不同——需设计承诺语义（最大条件数、冷启动、非交易时段、过期清理）；
3. 事件形状：来单 `tdx_intraday_alert` 字段集（含带日期的 ISO `alert_at`）与现有 AnomalyEvent 帧不同——需新增事件 kind 或投影层（在设计中定）；
4. 节奏与验收：需在设计中钉死"触发后 30 秒内到达"的计量口径（触发时刻→Mac 收到）；
5. `block_trade` 子项：现准入源的大宗交易为（半）日频，做不到 30 秒级真实成交触发；可做"新记录出现即报"——请澄清预期。

### 5.4 对来单 §4.5 五问的答复

1. 路线 A 的预警条件**不可编程**；建议直接取路线 B。
2. 事件 kind：可在现有订阅通道上承载，但新 payload 形状需在新设计中定义（不在无设计状态下承诺"复用"）。
3. 现成能力**有**：见 §5.2；行情源为 TQ-Local。
4. `set_intraday_alerts` 频率：盘后一次 + 观察池变更一次可接受（替换式）；限频与幂等语义在设计里定。
5. 见 §5.3-5。

### 5.5 下一步

建议由我方起草《盘中异动预警转发》Gate A 设计（含：条件下发的承载方式——新 RPC 或既有服务扩展；事件 schema 与 kind；30 秒验收口径；容量上限；非交易时段与过期语义），来单方确认条件模型后进入实现。**这是本回执中唯一需要新合同变更的工作项。**

## 6. 需要来单方补充的信息

1. pin 的合同四字段（§1.1）；
2. P0-3 字段级观测输出 + 一次 `stage=final` 的严格逐字段解析样本（若 final 也失败，同样是解析器问题）；
3. P1-1/2/3 的 reason_code 细类与原始片段（或接线变更说明）；
4. P0-4 `records=0` 原始请求/响应 + 服务端 request_id；
5. 第四章：block_trade 预期 + 观察池容量上限 + `expire_at` 语义确认；
6. （附注，不在清单内）我方服务端日志另有两条疑似来单方调用的失败记录：`instrument_news`（request_id `1792176259018-45362-3505`，2026-10-17 02:44 +08）与 `global_news`（request_id `1792184406206-45362-13486`，05:00 +08），均 `provider_unavailable`；如非预期请一并核查。

## 7. 现役身份与复现

- 部署身份（GetHealth 四字段）：`0.2.0` / `8523c1edb4ae4710f727b27b79026240e9cf47a6` / `dba66a27d82b81e0d0ecee60cfed706cc42eedeb6ab0e404a44a00fdf0ee1d81` / `c156c2413c36366dcee15bbe978487cb81cd2f41149acc2118bb7696fbb7b6d5`；
- 探针复现（示例，均可直接运行）：
  - `python query_op_probe.py BlockTrades magic.market.block_trades.request '{"instrument":{"exchange":"Shanghai","code":"600519","asset_class":"Equity"},"limit":10}'`
  - `python query_op_probe.py GlobalIndices magic.market.global_indices.request '{"indices":["DowJones","NasdaqComposite","Sp500","Nikkei225","HangSeng","Ftse100"]}'`
  - `python query_op_probe.py MarketAnnouncements magic.market.market_announcements.request '{"start":"2026-10-15","end":"2026-10-15","limit":50}' "" 2`
  - `python query_op_probe.py T0Evidence magic.market.t0_evidence.request '{"instruments":[{"exchange":"Shanghai","code":"600519","asset_class":"Equity"}],"daily_bar_count":20,"five_minute_bar_count":20,"requested_at":"<RFC3339>"}' "" 2`
- push2 证据链（curl 复现命令）随面交件交付。

## 8. 边界声明

本回执不授予任何数据权利、不改任何既有合同、不承诺未拍板的时间表；"实测"仅代表 2026-10-17 凌晨对现役实例（§7）的有界探针状态。所有"做不了"项均列有替代路径；需要 operator 拍板的两项（P0-1 与 P0-2）在本回执中保持开放。建议本交接目录 README 中该条由"待对端评估"更新为"已评估（见本回执）"。
