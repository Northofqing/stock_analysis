# `magic-market-data-rs` 外部行情需求交接

## 目标与边界

本项目只需要公共市场数据；不接入券商账户、资金、持仓或下单接口。真实持仓由用户快照提供，虚拟仓由 `stock_analysis` 自己记账。`magic-market-data-rs` 只负责可追溯的行情批次，不负责推送、AI、账户估值或交易决策。

现有 v18/v19 已有调度器和风险门，市场数据库不应新增第二套调度器。交付以 provider/service API 为主，调用方按盘中、集合竞价、盘后窗口请求。

## P0：必须提供的数据

### 1. 实时 Quote（盘中 5 秒门）

每个证券返回：`code`、`name`、`exchange/board`、`last`、`prev_close`、`open`、`high`、`low`、`change_pct`、`volume`、`amount`、`source_at`、`observed_at`、`provider`、`batch_id`。可用时附带买卖一至五档价格和数量。

要求：价格必须为有限正数；成交量/额不得为负；缺字段显式 `Unavailable`，不得用 0 或昨收代替；`source_at` 缺失不能通过 5 秒订单/纸面成交门；批次缺行、重复代码、超时或跨源冲突返回错误。板块真实涨跌幅（包括创业板 30%、科创板 20%、北交所 30%、ST 5%）必须原样保留，超出 20% 只做数据质量提示，不得硬拦截。

### 2. K 线（日线及分钟线）

支持 `1m/5m/15m` 盘中线和 `Day` 日线。每根返回：`code`、`interval`、`bar_start`、`bar_end`、`open/high/low/close`、`volume`、`amount`、复权标记、`source_at`、`provider`、`batch_id`。

日线最新日期最多落后一个交易日（节假日按交易日历排除）；盘后 AI 需要每股约 250 根有效日线。必须检测重复、倒序、日期断档、OHLC 不一致和非法价格；失败时返回完整错误，不能拼接静态或模拟数据。

### 3. Money Flow（盘中和盘后）

返回 `main_net`、超大/大/中/小单净额及金额、单位、`source_at`、`observed_at`、`provider`、`batch_id`。没有源数据必须是 `Unavailable`，不能写成 0；批次要能按证券集合原子校验。盘后 15:35 的主力净流 Top10 还需返回 `rank`、收盘价、涨跌幅和对应板块限价元数据。

### 4. Order Book

返回时间戳、买卖一至五档（可扩展十档）的 `price/quantity`、总深度和 provider/batch 证据。未支持时返回 `Unsupported`，不要把“不支持”伪装成 `Unsafe` 或空盘口；支持时仍须通过实时新鲜度门。

### 5. 集合竞价（09:15–09:25，重点 09:20–09:25）

按约 30 秒采样返回：`code/name`、撮合价、昨收、真实涨跌幅、匹配量/金额、未匹配买量/卖量、量比、`source_at`、`observed_at`、板块/涨跌停元数据及 `batch_id`。必须允许撤单冻结后的真实变化，缺字段逐行报错；不得因涨跌幅大于 20% 丢弃创业板、科创板或北交所数据。

## P1：增强能力

- 证券元数据：交易所、板块、ST 标记、上市日期、涨跌停规则版本（主板 10%、创业板 30%、科创板 20%、北交所 30%、ST 5%，新股/特殊规则以源元数据为准）。
- 交易明细（可选）：成交时间、价、量、方向和来源时间，用于盘后承接分析。
- 指数/板块快照：指数点位、涨跌幅、成交额及来源批次，供市场状态分析。

财报、分析师一致预期、公告和新闻不属于本次行情交接；继续由现有 v17 source/provider 负责，并且只在盘后分析窗口拉取。

## 统一协议要求

建议扩展现有 `magic-market-core::provider`：`RealtimeQuotes`、`HistoricalBars`，并增加 `MoneyFlow`、`OrderBook`、`AuctionSnapshot`、`PostCloseFlow` 能力。所有方法返回带 `batch_id` 和 provenance 的 `DataBatch<T>`，状态至少区分 `Available`、`Unavailable`、`Stale`、`Conflicted`、`Unsupported`。

- 批次必须记录 provider、请求时间、源时间、端点、重试次数、证券数和完整性。
- 严格请求与分块请求分开；分块失败不得静默返回部分结果，除非 API 明确标注 `Partial`。
- 网络层提供有界超时、重试/退避、限速和连接池；异步上下文禁止直接 drop Tokio runtime。
- 不提供 mock、成本价、0 值、昨收回退或“成功但无证据”的结果。

## 验收样本与交接顺序

覆盖 `000001`、`002208`、`600703`、`300xxx`、`688xxx`、北交所代码及 ST 代码；验证 5 秒实时门、30 秒账户数据门（由调用方负责）、日线一交易日门、20% 以上真实涨跌幅保留、09:20 集合竞价、15:35 资金流批次、重复/断档/缺字段错误和跨源冲突。

实施顺序：P0 Quote/Kline → P0 MoneyFlow/Auction → P0 OrderBook → P1 元数据/交易明细。每项先补 core contract 与 loopback fixture，再接 TDX/Smart client，最后由 `stock_analysis` 做只读适配；不得在 provider 项目内添加账户或推送副作用。

关联规则：stock_analysis `BR-115`（来源时间）、`BR-134`（风险门）、`BR-137`（来源事件）、`BR-152`（盘后昂贵数据）、`BR-154`（盘后行情缺失隔离）；数据红线 2.1、2.2、2.3、2.4、2.7、2.10。
