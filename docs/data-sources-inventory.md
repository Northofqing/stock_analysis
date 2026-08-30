# 数据源与准入边界

> 更新日期：2026-08-31。本文描述 `stock_analysis` 当前消费边界，不描述外部 provider-host 的内部 crate 或协议实现。

## 总原则

- 所有生产市场数据经版本化 gRPC 合同进入本仓库。
- `stock_analysis` 不拥有 provider client、provider router 或生产 provider server。
- provider/source/batch/time 是数据证据的一部分，允许保留历史名称，但不能授权本地 fallback。
- 不完整、过期、重复、冲突或身份不匹配的数据显式拒绝。
- 测试 fixture 使用 `TEST_CODE` identity，且只在 integration-test target 中编译。

## 市场与证券数据

| 能力 | gRPC operation / 本地边界 | 本仓准入重点 | 主要消费者 |
| --- | --- | --- | --- |
| 实时报价 | `RealtimeQuotes` / `market_data` | 精确代码集、有限正价格、record/envelope 证据一致、消费时新鲜度 | monitor、开盘门、执行报价 |
| 五档盘口 | `OrderBooks` / `market_capabilities` | 五档结构、价格/数量、代码与批次一致、消费时新鲜度 | T0、竞价与盘中分析 |
| 历史 K 线 | `HistoricalBars` / `historical_bars` | 日期区间、OHLC、排序、重复、完整性和 settled 语义 | 技术分析、复盘 |
| 技术 K 线 | `TechnicalBars` | 时间顺序、OHLC、量额、请求代码集 | 指标与形态 |
| 分钟数据 | `MinuteData` | 交易时段、累计量额、source time | 盘中分析 |
| T0 证据 | `T0Evidence` / `t0_evidence` | quote、盘口、日线、五分钟线、批次身份和逐条拒绝项 | 反向 T 观察计划 |
| 日内形态 | `IntradayShape` | 单一已准入批次投影，禁止跨批次拼接 | 盘中形态判断 |
| 指数行情 | `IndexQuotes`, `GlobalIndices` | 指数 identity、数值和时间证据 | 大盘监控与复盘 |
| 市场统计 | `MarketStatistics` | 估值、市值、换手和价格限制字段的完整性 | 公司与市场分析 |
| 证券元数据 | `SecurityMetadata` | canonical identity、名称、板块、ST、上市日 | 展示、限制与选择 |
| 公司行动 | `CorporateActions` | 日期窗口、类别、条款和证据 | 复权与异常波动确认 |

## 新闻、公告与研究

| 能力 | gRPC operation / 本地边界 | 本仓准入重点 | 主要消费者 |
| --- | --- | --- | --- |
| 全局新闻 | `GlobalNews` / `global_news` | 请求 provider 必须与响应 provider/source 精确一致；各源独立失败 | 新闻聚合与共振 |
| 个股新闻 | `InstrumentNews` | canonical code、时间范围、内容 identity/hash | 个股新闻审计 |
| 市场公告 | `Announcements`, `MarketAnnouncements` | 公告 ID、交易日、URL 和批次证据 | 盘后复盘与推送 |
| 研报 | `ResearchReports`, `ResearchDocuments` | 代码、发布时刻、机构、文档链接 | 基本面与专题分析 |
| 一致预期 | `Consensus`, `TargetPrices` | 报告数、机构数、预测分布和时间窗口 | 盈利预测与估值 |
| 网页研究 | `SemanticSearch` | provider 精确匹配、每条 evidence 与批次一致、仅 ResearchOnly | R-11 研究上下文 |

## 板块、资金与盘后数据

| 能力 | gRPC operation / 本地边界 | 本仓准入重点 |
| --- | --- | --- |
| 板块目录/成分 | `BoardDirectory`, `BoardConstituents`, `BoardMemberships` | 板块和证券 canonical identity、完整批次、唯一成员 |
| 板块资金流/排行 | `BoardFlows`, `MarketRankings`, `ConceptHits` | 排序、limit、批次证据和 metric 语义 |
| 个股资金流 | `MoneyFlows`, `FundFlowSeries` | interval、时间、量纲和数值有限性 |
| Provider Top-N | `ProviderTopNRankings` | 两种 metric 各自保留独立 evidence，禁止复用批次身份 |
| 北向资金 | `NorthboundDaily` | 交易日、channel、额度与 Top turnover |
| 龙虎榜 | `DragonTiger`, `MarketDragonTiger`, `DragonTigerDiscovery` | 日期、代码、披露项和席位金额 |
| 大宗交易 | `BlockTrades` | 代码、时间、成交价、收盘价、折溢价和双方身份 |
| 涨停池 | `LimitPools`, `UpperLimitPoolReview` | exact date、Upper kind、完整批次和代码唯一性 |
| 产业链批次 | `ChainBatch` | 输入批次、成员、内容 hash 和 rejection 全量保留 |

## 宏观与交易所数据

| 能力 | gRPC operation | 准入重点 |
| --- | --- | --- |
| 汇率 | `ForeignExchange`, `OfficialFxFixings` | pair、rate、source time 和官方 fixing identity |
| 经济日历/序列 | `EconomicCalendar`, `EconomicSeries` | 指标 identity、计划/发布时刻、period 和修订值 |
| 参考利率 | `ReferenceRates` | authority、tenor、日期和数值 |
| 期货交割 | `FuturesDelivery` | 合约、品种、最后交易日与交割日 |
| 公司文件 | `CompanyFilings` | issuer、document identity、发布日期和链接 |
| 财务报表 | `FinancialStatements` | instrument、报表类型、报告期、币种和行项目 |

## 传输与认证

- `GRPC_MARKET_ADDR` 选择外部服务地址。
- `GRPC_MARKET_CLIENT_BUNDLE` 可提供 CA、客户端证书/私钥、TLS server name、Bearer token 和连接描述。
- opening readiness 先校验健康、能力、认证和静态数据，再由实时 consumer 自行执行实时证据门。
- 未配置 bundle 时允许明确的开发 loopback 连接；这不创建本地 provider 实现。

## 历史身份

数据库、审计和 wire 中的 provider/source 字符串用于解释既有数据。删除本地依赖不会改写这些记录，
也不会改变已冻结 schema。判断仓库是否仍有 provider 实现，应查看 Cargo 依赖图、`src` 路径和
`scripts/check-no-magic-dependencies.sh`，而不是搜索历史数据标签。
