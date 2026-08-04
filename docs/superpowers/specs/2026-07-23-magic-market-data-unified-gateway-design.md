# Magic Market Data 统一数据网关设计

**日期**：2026-07-23

**状态**：历史总体设计；当前实现事实以
`2026-07-25-unified-data-final-cutover-design.md` 为准

**适用仓库**：

- `/Users/zhangzhen/Desktop/Quant/magic-market-data-rs`
- `/Users/zhangzhen/Desktop/Quant/stock_analysis`

## 1. 目标

将 `stock_analysis` 使用的公共金融数据和新闻数据统一收口到
`magic-market-data-rs` 的强类型合同、真实 Provider 和证据保留路由中。
`stock_analysis` 不再自行维护金融网站协议、请求签名、响应解析或跨源回退。

最终生产数据链路为：

```text
外部真实数据源
  → magic-market-data-rs Provider
  → magic-market-core DataBatch / SourceEvidence
  → magic-market-router 完整批次路由
  → stock_analysis::data_gateway
  → 选股 / 复盘 / 风控 / 新闻分析 / 推送
```

本设计必须满足 `AGENTS.md` 2.1、2.2、2.3、2.4、2.7、2.8 和 2.10。

这是跨仓库迁移的总体架构设计，不授权一次性实现全部 Slice。用户审核本文件后，
第一份实施计划只覆盖 Slice 0。Slice 1 至 Slice 4 必须在前一 Slice 至少通过
Gate C 后，分别完成代码事实复核、聚焦设计、实施计划和用户审核；禁止用本总体
设计替代后续 Slice 的具体 Gate A。

## 2. 非目标

- 不把账户、持仓、现金、净值或订单接口放入 `magic-market-data-rs`。
- 不把调度器、数据库、AI、推送、选股或交易决策放入 Provider 仓库。
- 不用一个网站强行覆盖所有能力。
- 不在迁移期间混合新旧来源字段生成一个伪原子批次。
- 不将一般网页搜索结果直接升级为可交易金融事实。
- 不保证公开网页端点具有厂商 SLA、再分发授权或长期协议稳定性。

## 3. 已核实代码事实

### 3.1 stock_analysis 当前只直接依赖 Magic TDX

命令：

```bash
rg -n '^magic-.*=' Cargo.toml
```

2026-07-23 输出：

```text
54:magic-tdx-rs = { package = "magic-tdx-rs", path = "../magic-market-data-rs/crates/magic-tdx-rs" }
```

其他财务、资金流、公告和新闻请求仍由 `src/data_provider/` 与
`src/search_service/providers/` 的本地实现发起。

### 3.2 monitor 当前注册七个本地新闻抓取器

命令：

```bash
rg -n \
  'Jin10FlashFeed|WallStreetCnFeed|ClsFlashFeed|SinaFlashFeed|WeiboHotFeed|GelonghuiFeed|KcbDailyFeed' \
  src/bin/monitor/news_aggregator_init.rs
```

2026-07-23 输出包含：

```text
61:Arc::new(feed::Jin10FlashFeed {
64:Arc::new(feed::WallStreetCnFeed {
68:Arc::new(feed::ClsFlashFeed {
71:Arc::new(feed::SinaFlashFeed {
74:Arc::new(feed::WeiboHotFeed {
77:Arc::new(feed::GelonghuiFeed {
80:Arc::new(feed::KcbDailyFeed {
```

### 3.3 magic-market-data-rs 已存在的 Provider 实现

命令：

```bash
for d in crates/magic-{tdx,tencent,sina,eastmoney,cls,cninfo,ths,iwencai,baidu,emquant}-rs
do
  echo "## $d"
  rg -n --glob '*.rs' \
    '^impl [A-Za-z][A-Za-z0-9_]*(Provider|Data|Reports|Statements|Questions|Search|Pools|Flows|Trades|Actions|Options|Announcements)? for ' \
    "$d/src"
done
```

核实到的实现包括：

- Magic TDX：证券元数据、日线、分钟线、报价、逐笔和盘口；
- 腾讯：报价、K 线、分钟线、盘口、统计和证券元数据；
- 新浪：报价、K 线、分钟线、盘口、财务三表和 ETF 期权；
- 东财：研报、个股新闻、资金流、龙虎榜、涨跌停池、两融、大宗交易、
  股东户数、解禁、分红和热度；
- 财联社：全球电报；
- 巨潮：公告和互动问答；
- 同花顺：一致预期、强势原因、涨停池和热榜；
- i问财：授权语义搜索；
- 百度：技术 K 线；
- EMQuant：报价、K 线和资金流，其他若干能力明确返回 `Unsupported`。

以上是 2026-07-23 对上游工作树的历史核实，不表示这些 crate 已被
`stock_analysis` 的生产 Gateway 链接。当前生产边界没有链接 THS、i问财、
互动问答或政府政策 Gateway；当前能力与来源必须以最终切换设计和实际
`src/data_gateway/**` 为准。

### 3.4 上游 Provider 尚不是稳定发布基线

命令：

```bash
git status --short
```

2026-07-23 的 `magic-market-data-rs` 工作区显示
`magic-eastmoney-rs`、`magic-cls-rs`、`magic-cninfo-rs`、
`magic-ths-rs`、`magic-iwencai-rs` 和 `magic-baidu-rs` 仍为未跟踪目录。
因此本设计把它们视为**待完成并验收的上游实现**，不能把目录存在写成生产可用。

### 3.5 版本现状

命令：

```bash
for f in crates/*/Cargo.toml
do
  rg -n '^(name|version)\s*=' "$f" | head -2
done
```

2026-07-23 结果显示公共 Provider 与 Core/Router 为 `0.2.0`，
`magic-market-analysis` 仍为 `0.1.0`。统一版本是上游 Gate B 的一部分。

## 4. 方案选择

### 方案 A：按数据域、上游优先切换（采用）

先稳定一个上游 Provider 数据域，再在 `stock_analysis` 进行 shadow 双读，
验收后切换并删除旧抓取器。

优点：

- 每个数据域可独立验证、提交和回滚；
- 不会把未提交 Provider 引入生产；
- 失败只影响当前迁移域；
- 可逐步删除重复协议代码。

代价：迁移期间暂时存在新旧两条读取路径，但只有一条生产决策路径。

### 方案 B：上游全部完成后一次切换

先完成所有 Provider，再一次性修改 `stock_analysis`。

优点是最终切换次数少；缺点是验证周期长、失败面大、回滚困难。

### 方案 C：直接让业务模块调用各 Magic Provider

改动最少，但 Provider 类型、错误、路由和协议知识会扩散到大量调用方，
形成浅模块和多套降级语义。本方案拒绝。

## 5. 仓库职责

### 5.1 magic-market-data-rs

负责：

- 真实公共数据获取；
- 强类型请求和记录；
- 单位、身份、时间和字段验证；
- `DataBatch<T>`、`SourceEvidence` 与来源状态；
- Provider 能力声明；
- 完整批次级路由；
- 有界超时、限流和传输安全；
- fixture、错误测试、live probe 和 load probe。

不负责：

- `stock_analysis` 调度；
- 业务缓存和数据库；
- 账户、持仓、订单；
- AI、推送和选股；
- 隐藏重试、跨源补值或模拟数据。

### 5.2 stock_analysis

负责：

- Provider 组合和来源优先级；
- 请求调度、缓存、持久化和审计；
- Magic 类型到现有领域模型的映射；
- 新闻去重、选股、分析、风控和推送；
- shadow 对比、切换状态和生产回滚；
- 业务窗口，例如盘中、15:35 和盘后复盘。

## 6. 统一数据网关

以下模块与接口均为**待实现（TO BE BUILT）**。

```text
src/data_gateway/
├── mod.rs
├── hub.rs
├── market.rs
├── company.rs
├── signals.rs
├── content.rs
├── error.rs
├── audit.rs
└── adapters/
```

`DataHub` 由 monitor composition root 构造，只负责持有四个深模块：

### 6.1 MarketData

职责：

- 实时报价；
- 日线和分钟线；
- 盘口；
- 逐笔；
- 证券元数据。

它隐藏 Provider 构造、来源优先级、完整批次校验和 Magic 类型映射。

### 6.2 CompanyData

职责：

- 财务三表；
- 研报；
- 一致预期；
- 公司资本事件；
- 公司/行业基础事实。

现有财务质量、估值、行业比较和研报衍生计算保留在业务模型中，
网络和协议实现移出。

### 6.3 MarketSignals

职责：

- 个股和板块资金流；
- 龙虎榜；
- 涨停、炸板、跌停和昨日涨停池；
- 强势原因和热度；
- 两融、大宗交易、股东户数、解禁和分红。

### 6.4 ContentData

职责：

- 个股新闻；
- 全球快讯；
- 公告；
- 互动问答；
- 政策新闻；
- 授权语义搜索。

`ContentData` 返回来源记录，不执行推送治理或选股。

### 6.5 外部 seam

生产调用方只认识这四个模块的类型化接口和统一错误，不认识具体 Provider。
只有 `src/data_gateway/adapters/` 可以构造或直接引用
`magic-*-rs` Provider。

架构测试必须阻止：

- `src/data_gateway/` 之外构造 Magic Provider；
- 新增金融数据 URL；
- 新增本地金融协议解析；
- 业务模块重新实现跨源 fallback。

一般通知、LLM 或非金融基础设施 URL 不属于该架构测试的金融数据 URL 列表。

## 7. 来源路由

| 数据域 | 主源 | 备用或交叉验证 |
| --- | --- | --- |
| 实时报价、盘口 | Magic TDX（首个路由候选） | 腾讯、新浪；TDX 实时报价当前不能证明五秒内的高精度 `source_at`，严格路由会继续尝试后备 |
| 日线、分钟线 | Magic TDX | 腾讯、新浪、百度 |
| 财务三表 | 新浪 | 无可验证备用时显式失败 |
| 个股和行业研报 | 东财 | PDF 只保留真实 URL |
| 一致预期 | 东财 | 缺失或非法 provider 发布日期时显式拒绝 |
| 个股、板块资金流 | 东财 | 其他源只作独立交叉验证 |
| 龙虎榜和资本数据 | 东财 | 无完整席位时降级为已验证事实 |
| 涨停复盘输入 | 东财 | 未实现的强势原因/THS 排名保持 unsupported |
| 个股新闻 | 新浪 | 按证券和时间窗口形成独立批次 |
| 全球快讯 | 东财、财联社、金十、澎湃 | 每个来源独立完整批次 |
| 公告 | 巨潮 | 互动问答当前没有生产 Gateway |
| 语义搜索 | 未接入 | i问财没有链接到生产 Gateway，不得由通用搜索冒充 |

Magic TDX 是实时行情首个路由候选，不是无条件固定主源；只有 Magic Router
可以依据同一严格批次合同选择已登记的腾讯或新浪后备。公开网页 Provider
只承担其已验证的数据域，不得冒充五秒实时报价 SLA。

## 8. 新闻源准入与迁移

用户选择“保留有独特价值且证据完整的来源，将实现迁入 Magic；删除重复、长期
stale 或字段不完整的来源”。

### 8.1 准入条件

一个新闻源只有同时满足以下条件才能进入生产路由：

1. Provider 返回真实源端发布时间，抓取时间不得替代；
2. 存在稳定源 ID 或规范 URL；
3. 标题、正文/摘要、来源和语言字段通过严格校验；
4. 批次保留 Provider、源时间、观察时间和 batch ID；
5. fixture、错误 fixture、live probe 和有界 load probe 通过；
6. 相比已启用来源具有明确独特覆盖；
7. 版权和使用边界记录在 integration 文档。

### 8.2 初始处置

| 来源 | 处置 |
| --- | --- |
| 财联社 | 采用现有 Magic Provider，完成上游 Gate 后启用 |
| 东财新闻 | 采用现有 Magic Provider，完成上游 Gate 后启用 |
| 巨潮 | 采用现有 Magic Provider，完成上游 Gate 后启用 |
| Jin10 | 保留宏观/财经日历价值；将抓取实现迁入 Magic 后验收 |
| 华尔街见闻 | 保留政策/市场覆盖；将抓取实现迁入 Magic 后验收 |
| 新浪个股新闻 | 保留持仓新闻回溯价值；迁入 Magic 后验收 |
| 科创板日报 | 保留科创板独特覆盖；具备完整源时间后迁入 |
| 微博热搜 | 当前不进入事实路由；只有满足全部准入条件才迁入 |
| 格隆汇 | 当前不进入事实路由；只有满足全部准入条件才迁入 |
| 雪球 | 仅可作为舆情，不得直接升级为公司事实 |
| 通用搜索 Provider | 作为发现工具；若用于金融新闻生产链，必须迁入 Magic 或删除 |
| gov.cn / MIIT | Parser 未实现时保持 `disabled=no_producer` |

## 9. 数据流

```text
业务模块发起类型化请求
  → Gateway 附带目标、查询时间、as-of、严格性和新鲜度要求
  → Router 按能力选择 Provider
  → Provider 获取并验证完整原始批次
  → Router 接受一个完整批次或返回类型化失败
  → Gateway 映射为 stock_analysis 领域模型
  → acquisition audit 记录请求与路由尝试
  → 业务模块消费
```

规则：

- 以完整批次为切换单位；
- 禁止跨 Provider 拼接同一记录的字段；
- 不同数据族可在下游联合分析，但各自保留来源证据；
- `source_at`、`observed_at` 永远分开；
- 只有源端明确证明的空批次才是 Verified Empty；
- 缓存读取不能刷新原始数据年龄；
- `Available`、`Unavailable`、`Stale`、`Partial`、`Conflict` 和
  `Unsupported` 不得互相折叠。

## 10. 失败和降级

统一错误分类：

- `Unsupported`：能力或请求组合不受支持；
- `Unavailable`：当前无数据或必要配置缺失；
- `Stale`：数据存在但超过业务新鲜度；
- `Invalid`：字段、身份、时间、单位、连续性或协议失败；
- `Partial`：严格批次不完整；
- `Conflict`：关键事实冲突；
- `Authentication`：授权缺失或被拒绝；
- `Transport` / `RateLimited`：传输或限流失败。

错误必须携带：

- provider；
- capability；
- batch/request identity；
- observed time；
- source time（若真实存在）；
- reason code；
- retryable；
- 脱敏目标身份。

### 10.1 降级边界

- 实时行情超过五秒立即拒绝，不用昨收、成本价或旧缓存替代；
- 盘后估值只消费已验证当日收盘批次；
- 资金流失败时可以生成独立龙虎榜章节，但龙虎榜不得冒充资金流；
- 财报与一致预期独立记录，要求完整一致预期的策略必须拒绝运行；
- 缺真实发布时间的新闻不得进入即时新闻、政策催化或选股；
- 龙虎榜席位不完整时只输出已验证事实，不渲染缺失金额、集中度或成功率；
- 失败不得转换为空集合、零、当前时间或默认对象。

### 10.2 异步执行

- Gateway 生产接口全部异步；
- 同步 Provider 只在统一 Adapter 内通过 `spawn_blocking` 调用；
- 禁止 Tokio runtime 内创建、`block_on` 或销毁另一套 runtime；
- 所有请求有界超时并支持取消；
- monitor 关闭时先停止生产者，再等待数据任务退出。

## 11. 日志和审计

预留业务规则：

- **BR-158**：统一数据网关、完整批次路由、状态保持与跨源拼字段禁令；
- **BR-159**：Provider 准入、批次日志汇总、相同原因去重和状态变化告警。

这些规则必须在实施第一步写入 `docs/business_rules.md`，然后才能实现相关逻辑。

每个 Provider 批次只输出一条结构化摘要：

- 请求数；
- 成功数；
- 拒绝数；
- 原因分类；
- provider；
- batch ID。

同一批次、同一 reason code 只输出一条 WARN，并附最多三个脱敏样本；
逐记录详情为 DEBUG。跨批次持续失败只在状态发生变化时再次 WARN，持续状态由
metrics 和 DEBUG 保持可见；系统级持续异常提醒复用 BR-135，不新增另一套时间
冷却阈值。

所有请求、路由尝试、拒绝和消费者结果写入强类型 acquisition audit。
审计路径沿用测试/生产物理隔离、SHA-256、跨进程锁、全链验证和 `sync_data`，
保留不少于五年。

## 12. Shadow 切换

每个数据域经历：

1. 上游 Provider Gate；
2. 下游 Adapter Gate；
3. shadow 双读；
4. 差异分类；
5. 新路径生产切换；
6. 旧抓取器删除；
7. 生产证据确认。

shadow 期间：

- 旧路径仍是唯一生产决策输入；
- 新路径只写隔离对比审计；
- 两条路径不拼接；
- 不重复推送；
- 不把 shadow 数据写入账户、持仓或交易状态。

对比至少覆盖：

- 证券身份；
- provider/source time；
- 关键数值和单位；
- 记录数与完整性；
- 缺失状态；
- 排序和重复身份；
- 新鲜度。

差异必须标记为口径差异、源端差异或实现缺陷。未分类差异阻止切换。

## 13. 迁移切片

### Slice 0：上游基线

- 提交并验收所有新 Provider crate；
- 全 workspace crate 统一 `0.2.0`；
- Core/Router/Provider 测试与 live probe 通过；
- 生成可供下游固定的 commit SHA；
- 不把未跟踪目录作为 path dependency。

Slice 0 是本设计通过用户书面审核后唯一进入 `writing-plans` 的范围。

### Slice 1：财务、研报、新闻、公告（历史计划）

- CompanyData 与 ContentData；
- 新浪财报、东财研报和东财一致预期；
- 东财/财联社新闻、巨潮公告；
- 迁入符合准入条件的独特新闻源；
- 替换 `v17_sources`、NewsAggregator 和盘后复盘取数。

### Slice 2：资金、龙虎榜、涨跌停池和资本数据

- MarketSignals；
- 修复龙虎榜字段完整性与重复记录；
- 资金流不可用和龙虎榜独立降级；
- 替换本地东财协议。

### Slice 3：行情、K 线、盘口和证券元数据

- MarketData；
- Magic TDX 主源；
- 腾讯、新浪、百度备用；
- 替换本地市场 HTTP Provider 和 fallback。

### Slice 4：删除和收口

- 删除已替代抓取器；
- 删除重复协议测试和不再需要的依赖；
- 架构测试覆盖所有生产模块；
- 文档、部署和运维说明更新。

## 14. 旧模块处置

| 旧模块 | adopt/reject | 处置 |
| --- | --- | --- |
| `magic_tdx_provider.rs` | adopt | 收入 MarketData Adapter，保留领域映射 |
| `magic_tdx_t0.rs` | adopt | 保留 T0 验证/计算，删除直接取数 |
| `financials.rs` | adopt | 保留模型和质量分析，删除 HTTP/解析 |
| `consensus.rs` | adopt | 保留衍生指标，删除 HTTP/解析 |
| `money_flow.rs` | adopt | 保留形态计算，删除东财抓取 |
| `announcement.rs` | adopt | 保留生命周期和分类，删除协议实现 |
| `lhb_analyzer.rs` | adopt | 保留分析，输入改由 MarketSignals 提供 |
| `search_service/providers/*` | partial | 已迁移源删除抓取器；保留必要结果投影 |
| `eastmoney_provider.rs` | reject after parity | MarketData 通过验收后删除 |
| `gtimg_provider.rs` | reject after parity | 腾讯 Magic Provider 通过验收后删除 |
| `sina_provider.rs` | reject after parity | 新浪 Magic Provider 通过验收后删除 |
| `baostock_provider.rs` | reject after parity | Magic 盘后 K 线完整后删除 |
| 新闻 feed wrappers | adopt | 只调用 ContentData，不创建 HTTP Provider |

## 15. 测试与验收

### 15.1 上游

每个能力需要：

- 正常 fixture；
- 字段缺失、坏时间、坏 URL、错误码和超限 fixture；
- Provider 能力声明测试；
- 来源证据测试；
- live probe；
- 有界、低并发 load probe。

样本覆盖主板、创业板、科创板、北交所、ST、空结果和异常响应。

### 15.2 下游

- Gateway 接口单元测试；
- Adapter 映射和单位测试；
- 完整批次/Partial/Conflict 测试；
- 新鲜度和缓存年龄测试；
- shadow 对比测试；
- 架构测试；
- 嵌套 runtime 回归测试；
- monitor 优雅关闭测试；
- 生产/test 身份和存储隔离测试。

### 15.3 强制命令

两个仓库按适用范围执行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

运行验证使用待实现的隔离脚本（**TO BE BUILT**）。脚本必须创建临时数据库、
event bus、delivery audit、push log 和 dispatcher log 根目录，清除生产路径
override，启用通知 dry-run，并在退出后证明生产目录未变化：

```bash
bash tools/validation/run_magic_gateway_review.sh
bash tools/validation/run_magic_gateway_monitor.sh
```

第一条脚本内部执行 `cargo run --bin monitor -- --review`；第二条内部执行
`cargo run --bin monitor`，运行到覆盖目标调度路径后发送 SIGINT。两者必须无
panic、无 runtime shutdown 错误、无 stale 日志洪泛并优雅退出。严格复盘在真实
数据不足时可以按已登记合同非零退出，但不得因 Gateway 自身 panic 或破坏隔离。

覆盖率：

- workspace 总行覆盖率不低于 80%；
- 核心 Provider、Gateway、路由、验证和审计路径不低于 95%。

### 15.4 生产证据

每个已启用能力至少保留一次：

```text
真实 Provider
→ DataBatch / SourceEvidence
→ Gateway
→ 真实 consumer
→ acquisition audit
```

若能力没有生产者，启动时必须输出 `disabled=no_producer`；
不得用测试 fixture 或 `--test` 推送冒充生产证据。

## 16. Gate

### Gate A

- 本设计获用户确认；
- 两仓职责、失败模式、旧模块和回滚明确；
- 阻塞性架构异议为零。

### Gate B

- 当前 Slice 实现、故障路径和测试完成；
- 上游 Provider 不是未跟踪文件；
- 不存在生产 mock、静默空值或嵌套 runtime。

### Gate C

- fmt、Clippy、全量测试和合规全部通过；
- 数据新鲜度通过；
- BR-158/BR-159 已登记并由检查脚本验证。

### Gate D

- 覆盖率满足 80%/95%；
- live probe 和 shadow 证据完整；
- monitor 两种命令通过隔离验证；
- 独立审查无 Critical/Important 未决项；
- PR 证据字段完整。

## 17. PR 证据模板

```markdown
### Refs
- spec: `docs/superpowers/specs/2026-07-23-magic-market-data-unified-gateway-design.md`

### Data-Redlines
- [2.1] 只使用真实 Provider；失败不回退模拟数据
- [2.2] 缺失保持缺失
- [2.3] 价格、时间、连续性、单位和批次严格校验
- [2.4] 实时五秒、日线一交易日等新鲜度门
- [2.7] acquisition audit 可追溯
- [2.8] Provider/Gateway 实际读取目标数据源
- [2.10] BR-158、BR-159

### OldModules
| module | adopt/reject | reason |
| --- | --- | --- |
| `src/data_provider/financials.rs` | adopt | 保留分析模型，删除已被替代的网络和解析实现 |

### Threshold-Proof
- 本 Slice 未修改阈值；若修改，引用对应 spec/config 字段和 clamp 证明

### Business-Rules
- BR-158
- BR-159

### Validation
- `cargo test --workspace --all-targets --all-features -- --test-threads=1`: PASS

### Rollback
- 记录该 Slice 的实际 merge SHA，并对该 SHA 执行 `git revert`
```

## 18. 回滚

- 每个数据域独立 PR 和独立切换；
- shadow 阶段关闭新读取不影响生产决策；
- 切换后通过 Git revert 恢复上一条已验证路径；
- 架构或数据流错误返回 Gate A；
- 实现错误返回 Gate B；
- 数据红线失败返回 Gate B，并重审 Gate A 失败模式；
- 不删除历史行情、审计、账户、持仓或交易证据。

## 19. 完成定义

只有以下全部成立，才能称为“所有金融和新闻数据已统一到
magic-market-data-rs”：

1. `stock_analysis` 生产金融/新闻调用只通过 `data_gateway`；
2. 所有启用 Provider 已提交、版本一致并通过 live probe；
3. 旧金融/新闻 HTTP 抓取器和协议解析已删除；
4. 未支持能力明确 `Unsupported/Unavailable`；
5. fmt、Clippy、测试、合规和覆盖率通过；
6. `monitor --review` 和常驻 monitor 验证通过；
7. 每个已启用能力有真实生产链路和审计证据；
8. PR 字段完整且独立审查通过。
