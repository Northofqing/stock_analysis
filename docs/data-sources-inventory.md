# 网络数据源全清单

> 项目内所有**需要网络请求**的数据获取路径与投递端点总览。
> 更新时间: 2026-08-12 (14:30 CST)。数据获取统一入口在 `src/data_gateway/**`
> (CLAUDE.md 架构规则), 上游供应商 HTTP/TCP 端点封装于 magic crates
> (见文末「上游依赖」), 本清单以项目内代码位置为准。

---

## 一、行情类

| 数据类别 | 具体内容 | 数据源/端点 | 提供方 | 关键代码位置 | 频率/触发 | 用途 |
|---|---|---|---|---|---|---|
| 实时报价 | 最新价/昨收/涨跌幅等 | Magic TDX (TCP) → Magic Tencent (HTTP) → Magic Sina (HTTP) 三级路由 | magic-tdx-rs / magic-tencent-rs / magic-sina-rs | `src/data_gateway/market_data.rs` (CAPABILITY: RealtimeMarketQuotes) | 消费者调用即拉取；逐条严格 `0..=5s` 新鲜度门 (BR-218)，未来/超龄即使在午休或盘后也显式拒绝；盘后收盘价仅走独立 `SettledClose` 合同 | 盘中实时价格、执行报价 |
| 五档盘口 | 买卖各五档价量 | TDX → Tencent → Sina 三级路由 | 同上 | `src/data_gateway/market_capabilities.rs:47` (ORDER_BOOK_PROVIDER_ORDER); `magic_tdx_t0.rs:44` (T0BookLevel 做T专用) | 消费者按需 | 盘口深度分析、做T |
| 分时/分钟均价 | 每分钟 price/cumulative_quantity/avg | TDX → Tencent → Sina 三级路由 | 同上 | `src/data_gateway/market_capabilities.rs:41` (MINUTE_PROVIDER_ORDER) | 消费者按需 | 分时走势 |
| 5 分钟 K 线 | at/open/high/low/close/volume/amount | Magic TDX (TdxHqClient, TCP; `cached_tdx_hq_client` 进程级复用) | magic-tdx-rs (KLINE_5MIN) | `src/data_gateway/magic_tdx_t0.rs:13,985` | 做T 扫描每 30s tick | T0 做T 证据链、日内形态 |
| 日 K 线 | 20+ 根日线 | Magic TDX → Tencent → Sina → Baidu 四级路由 (BR-092 跨股票一致性准入) | 四家 magic crates | `src/data_gateway/historical_bars.rs:5,48` (HistoricalDailyBars) | 消费者按需; 批量请求 | 趋势分析、指标、盘后选股 |
| Outcome 日 K 线 (选股专用) | 已验证 due binding 的证据前像 | **Magic TDX ONLY, 无路由无回退** | magic-tdx-rs (tdx-smart) | `src/data_gateway/outcome_daily_bars.rs:44` (OutcomeDailyBarsV2) | BR-174 盘后选股流程 | Schema-v2 正式选股 |
| 做T 证据链 | quote+五档+日K(20)+5分钟K+source_time+内容哈希+BR-231 已准入证据诊断缓存 | Magic TDX (TdxHqClient TCP; ALL_KNOWN_SERVERS/PRIMARY_SERVERS, connect_to_any 5s 超时) | magic-tdx-rs | `src/data_gateway/magic_tdx_t0.rs::fetch_magic_tdx_t0_batch_with_clock`, `src/data_gateway/magic_tdx_t0.rs::validate_quote_freshness` | 盘中 30s tick 批量；quote 逐票与批次完成均严格 `0..=5s`，未来/过期显式拒绝 | 反向 T 观察计划 (BR-153)；缓存仅诊断/重放检测，不放宽新鲜度 |

## 二、新闻类

| 数据类别 | 具体内容 | 数据源/端点 | 提供方 | 关键代码位置 | 频率/触发 | 用途 |
|---|---|---|---|---|---|---|
| 全局新闻聚合 (4 Feed) | 东财快讯/财联社电报/金十快讯/澎湃财经 各 ≤20 条 | 4 家供应商 HTTP 端点 (crate 内实现) | magic-eastmoney-rs (eastmoney-web) / magic-cls-rs (cls-v1) / magic-jin10-rs (jin10-flash-v1) / magic-thepaper-rs (thepaper-finance-v1) | `src/data_gateway/global_news.rs:19-43` (GlobalNewsProvider) | BR-166 轮询, 默认 120s (NEWS_POLL_INTERVAL) | 快讯共振 → 选股信号 (BR-174 ingress) |
| 新闻搜索 (宏观) | 4 家快讯聚合渲染综合宏观点评 + 经济日历章节 | 依赖 GlobalNewsGateway + EconomicCalendarGateway | 同上 4 家 + Jin10 | `src/search_service/service.rs:1369` (search_macro_news), `:337` (并发 join) | R-08 盘后复盘等触发 | LLM 宏观分析上游 |
| 个股新闻 | 指定代码+日期范围的公司新闻 | Magic Sina (HTTP) | magic-sina-rs (sina-company-news) | `src/data_gateway/sina_instrument_news.rs:20-21` (SinaInstrumentNews, limit 100) | BR-163 按需 | BR-066 个股新闻审计 |
| 市场公告 (CNInfo) | 交易日全市场公告 | 巨潮资讯网 (CninfoClient HTTP) | magic-cninfo-rs (cninfo-market) | `src/data_gateway/event_calendar.rs:18-19` (R-08-announcements) | BR-161 按交易日拉取 | R-08 大盘复盘、公告推送 |
| 盘面新闻事件 | 4 feed → market_event pipeline (simhash 去重) | GlobalNewsGateway | 同上 4 家 | `src/news/aggregator/feed.rs`, `src/news/aggregator/mod.rs` (SourceKind) | BR-166/BR-174 轮询 | 新闻→市场事件→产业链映射→选股 |

## 三、板块/资金类

| 数据类别 | 具体内容 | 数据源/端点 | 提供方 | 关键代码位置 | 频率/触发 | 用途 |
|---|---|---|---|---|---|---|
| 板块资金流 | 主力/超大单/大单/中单/小单净流入 | Eastmoney (HTTP) | magic-eastmoney-rs (eastmoney-web) | `src/data_gateway/board_runtime.rs:15` (board-flows) | 消费者按需 | BR-188 板块资金流分析 |
| 板块成分股 | 板块→成分股列表 (limit 10000) | TDX (TdxHqClient TCP, block files) | magic-tdx-rs (tdx-block-files) | `src/data_gateway/board_runtime.rs:14` (board-memberships); `board.rs:59` | 启动加载 + 按需刷新 | BR-159 产业链推演、选股板块绑定 |
| 板块目录 | 概念/行业/地域板块列表 | TDX (TdxHqClient TCP) | magic-tdx-rs (tdx-block-files) | `src/data_gateway/board_runtime.rs:13` (board-directory) | 启动时加载 | 板块发现+验证 |
| 概念板块主力净流入排行 | code/name/change_pct/main_inflow/量比/换手等 | **`https://push2.eastmoney.com/api/qt/clist/get`** (fid=f62/f3, fs=m:90+t:3) | Eastmoney 直连 (reqwest 同步) | `src/data_gateway/board_ranking.rs:18-19` (CLIST_ENDPOINT/TOKEN), `:53` (fetch_top) | I-09 按 fid+top_n 拉取 | 概念板块排行展示 |

## 四、列表类

| 数据类别 | 具体内容 | 数据源/端点 | 提供方 | 关键代码位置 | 频率/触发 | 用途 |
|---|---|---|---|---|---|---|
| 连板识别/涨停池 | 涨停代码/名称/连板数 | TDX (TdxSmartClient TCP, BlockService) | magic-tdx-rs | `src/data_gateway/chain_intelligence.rs:77-80,855,886` | BR-159/BR-160 按需 | A-10 产业链智分析 |
| 证券信息 (STOCK_LIST) | 名称/ST 标签/板块归属 | Tencent → Sina 双路由 | magic-tencent-rs / magic-sina-rs | `src/data_gateway/market_capabilities.rs:58` (SecurityMetadata; TDX 因缺 source timestamp 被排除) | 消费者按需 | 证券名称/ST 识别 |
| 交易日历 | 节假日/交易日判定 | 本地配置为主 + 交易所官网 URL 校验 | 本地逻辑 + 交易所 | `src/calendar.rs:20-33`; `src/data_gateway/exchange_calendar_authority.rs:9,43` | 启动时加载 | 交易日/交易时段判定 |

## 五、资金/财务/研报类

| 数据类别 | 具体内容 | 数据源/端点 | 提供方 | 关键代码位置 | 频率/触发 | 用途 |
|---|---|---|---|---|---|---|
| 个股资金流 | 1 分钟/日线主力净流入等 | Eastmoney (HTTP) | magic-eastmoney-rs (eastmoney-web) | `src/data_gateway/capital.rs:41` (FUND_FLOW_CAPABILITY) | 按需; Minute1 2 分钟老化门, Day1 30s 门 | 资金面分析 |
| 北向资金 (HKEX) | 沪股通/深股通额度与统计 | HKEX 官方 (HkexClient) | magic-exchange-rs (hkex-official) | `src/data_gateway/capital.rs:44` (NORTHBOUND_CAPABILITY) | BR-164 按需 | R-08 大盘复盘 |
| 卖方一致预期 | 净利润/营收/ROE 预测 (≤50 份研报/180 天) | Eastmoney (HTTP) | magic-eastmoney-rs | `src/data_gateway/consensus.rs:17` | BR-119/159/164 按代码 | 一致预期分析 |
| 研报 | 报告标题/机构/评级/目标价/PDF | Eastmoney (HTTP) | magic-eastmoney-rs | `src/data_gateway/research.rs:11` | BR-119/164 按代码 | 研报数据 |
| 财务报表 | 三表 (利润/资产负债/现金流) | Sina (HTTP) | magic-sina-rs | `src/data_gateway/company.rs:32` | 按需 | 基本面分析 |
| 市场统计 | PE/PB/市值/换手率 | Tencent (HTTP) | magic-tencent-rs | `src/data_gateway/company.rs:34` | 按需 | 估值分析 |
| 龙虎榜 | 席位买卖明细/金额/净额 | Eastmoney (HTTP) | magic-eastmoney-rs | `src/data_gateway/dragon_tiger.rs:16` (R-04) | BR-162 按交易日 | R-04 龙虎榜分析 |
| 大宗交易 | 成交价/折溢价/买卖方 | Eastmoney (RPT_DATA_BLOCKTRADE) | magic-eastmoney-rs | `src/data_gateway/block_trade.rs:16` | BR-223 盘后 | 盘后大宗复盘 |
| Provider Top-N 排行 | 量比/主力净流入 Top-20 | Eastmoney (单页 ≤20 条) | magic-market-composition | `src/data_gateway/capital.rs:42-43` | BR-164 按需 | 排名展示 |

## 六、指数/全球市场类

| 数据类别 | 具体内容 | 数据源/端点 | 提供方 | 关键代码位置 | 频率/触发 | 用途 |
|---|---|---|---|---|---|---|
| A 股盘中指数 | 上证/深证/创业板/科创 50 等 6 大指数 | Tencent (HTTP, 唯一满足契约的提供方) | magic-tencent-rs | `src/data_gateway/index.rs:21` (RealtimeIndexQuotes, 5s 新鲜度门) | MarketAnalyzer 按需 | 大盘监控 |
| 美股三大指数 | 道琼斯/纳指/标普 500 | Sina (global_indices) | magic-sina-rs (sina-web) | `src/data_gateway/global_market.rs:13-14` (R-08-global-indices) | R-08 盘后 | 美股对照 |
| 美元/人民币汇率 | rate/change | Sina (foreign_exchange) | magic-sina-rs (sina-web) | `src/data_gateway/global_market.rs:14` (R-08-global-fx) | R-08 盘后 | 汇率监控 |

## 七、期货类

| 数据类别 | 具体内容 | 数据源/端点 | 提供方 | 关键代码位置 | 频率/触发 | 用途 |
|---|---|---|---|---|---|---|
| 期货交割日历 | CFFEX 交割日/最后交易日 | 中金所官网 (CffexClient HTTP) | magic-exchange-rs (cffex-official-notice) | `src/data_gateway/futures_delivery.rs:13-14` (R-08-cffex-delivery) | BR-165/199 按年月 | 交割日预警 |

## 八、经济日历类

| 数据类别 | 具体内容 | 数据源/端点 | 提供方 | 关键代码位置 | 频率/触发 | 用途 |
|---|---|---|---|---|---|---|
| 宏观经济事件 | 各国经济数据发布/预期/实际值 | 金十数据 (Jin10Client HTTP) | magic-jin10-rs (jin10-flash-v1) | `src/data_gateway/economic_calendar.rs:13-14` (MAX_LIMIT 20) | BR-133/167 按 limit+国家 | R-08 宏观日历 |

## 九、搜索/研究类 (ResearchOnly)

| 数据类别 | 具体内容 | 数据源/端点 | 提供方 | 关键代码位置 | 频率/触发 | 用途 |
|---|---|---|---|---|---|---|
| 通用 Web 搜索 | 按 query 返回搜索结果 (不替代金融数据) | **Bocha** `https://api.bocha.cn/v1/web-search`; **Tavily** `https://api.tavily.com/search`; **SerpAPI** `https://serpapi.com/search` | reqwest 直连 (10s 超时), key 池轮转 | `src/data_gateway/general_web_research.rs:337,413,470` | BR-175 按 query | ResearchOnly 题材搜索 |

## 十、生命周期/证券治理类

| 数据类别 | 具体内容 | 数据源/端点 | 提供方 | 关键代码位置 | 频率/触发 | 用途 |
|---|---|---|---|---|---|---|
| 上市日期/公司行动 | 上市日、除权除息/送转配 | Magic TDX (TdxSmartClient) | magic-tdx-rs | `src/data_gateway/security_lifecycle.rs:28-30` | BR-171 按需 | 前复权、日K 校验 |
| 持仓板块归属 | code → 主板块+成员 | TDX 板块成分 (BoardDataGateway) | magic-tdx-rs | `src/data_gateway/position_chain.rs:13` | BR-085/170 按需 | 持仓个股板块归属 |

---

## 十一、外发投递 (网络请求, 非数据获取)

| 投递渠道 | 端点 | 提供方 | 关键代码位置 |
|---|---|---|---|
| MagicLaw 统一推送网关 | `127.0.0.1:18011` (MAGICLAW_API_ADDR), /api/v1/send 等 | 自研 Go daemon (外部二制) | `src/bin/monitor/main.rs:48` (DEFAULT_MAGICLAW_API_ADDR); `notify.rs:4027` (push_via_magiclaw_daemon) |
| 飞书群机器人 | `https://open.feishu.cn/open-apis/bot/v2/hook/xxx` (FEISHU_WEBHOOK_URL) | reqwest (SHARED_HTTP_CLIENT) | `src/bin/monitor/notify.rs:3697` (push_feishu_http); `src/push_l6/external_sinks.rs:263` (FeishuSink) |
| 企业微信群机器人 | `https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=xxx` (WECHAT_WEBHOOK_URL) | reqwest | `src/push_l6/external_sinks.rs:151-195` (WechatSink); `src/notification/service.rs` (send_to_wechat) |
| Telegram | `https://api.telegram.org/bot{TOKEN}/sendMessage` | reqwest | `src/notification/service.rs:536` |
| Pushover | `https://api.pushover.net/1/messages.json` | reqwest | `src/notification/service.rs:443-459` |
| Server酱 (个人微信) | `https://sctapi.ftqq.com/{KEY}.send` | reqwest | `src/notification/service.rs:411` |
| 钉钉机器人 | DINGTALK_WEBHOOK_URL (env) | reqwest | `src/notification/service.rs:483-509` |
| Slack | SLACK_WEBHOOK_URL (env) | reqwest | `src/notification/service.rs:559-579` |
| Discord | DISCORD_WEBHOOK_URL (env, 2000 字符上限) | reqwest | `src/notification/service.rs:604-634` |
| 自定义 Webhook | CUSTOM_WEBHOOK_URLS (env, 可多个; 可选 Bearer auth) | reqwest | `src/notification/service.rs:581-600` |
| 邮件 SMTP | SMTP_SERVER:SMTP_PORT (env) | lettre crate | `src/notification/service.rs` (send_to_email); `src/notification/config.rs:46-51` |

## 十二、LLM API (AI 模型调用)

| 模型 | 端点 | 提供方 | 关键代码位置 |
|---|---|---|---|
| DeepSeek | `https://api.deepseek.com/v1` (DEEPSEEK_BASE_URL 可覆盖) | async-openai | `src/llm/providers.rs:36`; `src/deep_analyzer.rs:245` |
| 豆包 (字节) | `https://ark.cn-beijing.volces.com/api/v3` (DOUBAO_BASE_URL) | async-openai | `src/deep_analyzer.rs:237` |
| Gemini (Google) | `https://generativelanguage.googleapis.com/v1beta/openai/` (GEMINI_BASE_URL) | async-openai | `src/deep_analyzer.rs:253` |
| MiniMax | `https://api.minimaxi.com/v1` | async-openai | `src/llm/providers.rs:107` |

## 十三、TDX 主站服务器池 (TCP, 非 HTTP)

| 项 | 内容 | 位置 |
|---|---|---|
| 服务器列表 | ALL_KNOWN_SERVERS (全量 101 台) / PRIMARY_SERVERS (优先 10 台), 含名称/IP/端口 | magic-tdx-rs `protocol::constants`; 引用点 `src/data_gateway/chain_intelligence.rs:886`、`magic_tdx_t0.rs:985`、`market_data.rs:593-599`、`src/bin/tdx_server_probe.rs:21` |
| 连接管理 | TdxSmartClient/TdxHqClient 内建故障转移+自动重连; `cached_tdx_hq_client`/`cached_tdx_smart_client` 进程级复用 | `magic_tdx_t0.rs:967-989`、`market_data.rs:598-599` |
| 用途 | 所有 TDX 数据获取的底层 TCP 传输 (通达信免费公共行情协议, zlib 压缩) | — |

## 十四、上游 magic crates (锁定版本)

全部 14 个 magic crates 锁定同一 Git 仓库 `https://github.com/Northofqing/magic-market-data-rs.git` 同一 revision `75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e` (v0.2.0)。供应商 HTTP 端点代码内嵌于各 crate 内部, 本项目只使用类型化 Provider Client。

| Crate | 传输 | 封装的供应商协议 |
|---|---|---|
| magic-tdx-rs | TCP (自定义 TDX 协议) | 通达信免费公共行情 |
| magic-tencent-rs | HTTP | 腾讯财经行情 |
| magic-sina-rs | HTTP | 新浪财经行情/新闻/全球市场 |
| magic-eastmoney-rs | HTTP | 东方财富 (行情/资金流/龙虎榜/大宗/研报/板块) |
| magic-baidu-rs | HTTP | 百度财经日 K 线 |
| magic-cls-rs | HTTP | 财联社电报 |
| magic-jin10-rs | HTTP | 金十数据快讯 + 经济日历 |
| magic-thepaper-rs | HTTP | 澎湃财经 |
| magic-cninfo-rs | HTTP | 巨潮资讯网市场公告 |
| magic-exchange-rs | HTTP | HKEX 北向资金 + CFFEX 期货交割 |
| magic-market-core / magic-market-router / magic-market-composition | (纯库) | 共享类型 / SourceRouter / Top-N 排行组装 |

## 附: 已排查非数据端点 (grep 确认过)

- 所有 `reqwest::Client::builder()` (28 处) 统一走 `SHARED_HTTP_CLIENT` (30s) / `SHARED_FAST_HTTP_CLIENT` (10s) 两个懒加载单例 (`src/http_client.rs:19-37`) — 进程内 HTTP 池, 非新端点。
- Prometheus metrics `http://localhost:9090/metrics` (`src/bin/monitor/metrics.rs:12`) — **入站**导出端口, 非外发请求。
- `calendar.rs` 交易日历纯本地计算, 仅引用交易所权威 URL 做 URL 校验 (`exchange_calendar_authority.rs:9`)。
- `data_provider/mod.rs:95` loopback_http_client — 仅测试用 mock。
- broker.rs 本身不发网络请求, 委托 RealtimeMarketQuotes (见一、1.1)。
