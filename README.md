# stock_analysis

`stock_analysis` 是一个面向 A 股的研究、监控、纸面交易和盘后复盘系统。公共金融与新闻事实统一经过强类型 Gateway，账户事实走独立证据边界。

当前统一数据迁移仍处于 **Gate B / In Progress**。模块存在或能够编译不代表发布完成；在全量测试、合规、覆盖率和真实数据门禁通过前，不宣称 Gate D 就绪。

## 系统边界

系统用于研究市场事件、筛选候选、执行风险门、记录纸面决策并复原审计链。它不承诺收益，也不把 AI、回测或公共行情当作券商成交确认。

当前没有自动实盘订单出口。IB、TQS、QMT 等真实券商合同未接入时，账户、现金、持仓、净值和订单能力必须显式不可用。

本地导入的真实账户快照只是一份历史证据，不是持续券商连接。超过 30 秒的持仓或现金证据不能授权实时动作。

## 统一数据架构

```text
官方或公共来源
  -> magic-market-data-rs 固定 Git revision 的 Provider
  -> magic-market-core DataBatch + SourceEvidence
  -> magic-market-router 完整批次 admission
  -> stock_analysis::data_gateway
  -> persistence / research / selection / risk / review / notification
```

只有 `src/data_gateway/**` 可以构造 Magic Provider。业务模块不得保留金融源 URL、HTTP transport、协议 parser 或自建跨源 fallback。

Magic TDX 是 A 股行情路由的第一个候选，不是每次请求都必然采用的来源。只有通过完整性、新鲜度和证据门禁的批次才能胜出；其他公开候选只由 Magic Router 选择。

消费者不能直接连接腾讯、Sina、百度、东方财富、巨潮或交易所接口。

每次请求只接纳一个完整批次。缺失、过期、部分、冲突和未支持必须保持显式，禁止跨来源拼字段、补零或用本地观察时间冒充 provider 时间。

### 当前 Gateway 能力

下表描述当前代码边界，不代表 Gate D 已通过：

| 领域 | Gateway | 上游合同 | 当前语义 |
| --- | --- | --- | --- |
| A 股实时行情 | `MarketDataGateway` | Magic TDX → Tencent → Sina | TDX 当前缺少足以证明 5 秒 SLA 的高精度 `source_at`，严格路由会继续尝试 Tencent/Sina |
| A 股日线 | `HistoricalBarsGateway` | Magic TDX → Tencent → Sina → Baidu | 未复权完整 OHLCV/amount；日线新鲜度不超过 1 个交易日 |
| A 股指数 | `IndexDataGateway` | Magic Tencent | 强类型指数报价；不保留本地腾讯协议解析器 |
| 分钟线、五档 | `MarketCapabilitiesGateway` | Magic TDX → Tencent → Sina | 任一来源不能证明完整批次时返回显式失败；不降级填值 |
| A 股证券身份 | `MarketCapabilitiesGateway` | Magic Tencent → Magic Sina | 解析代码、名称与交易所；完整证券元数据仍可能 Unsupported |
| 上市日与公司行动 | `SecurityLifecycleGateway` | Magic TDX | 上市日、除权除息等生命周期证据逐批次校验，不由身份解析器补造 |
| 财务报表、估值统计 | `CompanyDataGateway` | Magic Sina / Tencent | 原始报表行与可选 PE/PB/市值等字段保持上游语义 |
| 个股/盘后资金、北向统计 | `CapitalDataGateway` | Magic Eastmoney / HKEX | 资金分层与官方北向统计分开建模，禁止把成交额改称净买额 |
| 研报、一致预期 | `ResearchDataGateway` / `ConsensusDataGateway` | Magic Eastmoney | 报告与一致预期使用各自强类型批次，不把研报预测拼成共识 |
| 板块 | `BoardDataGateway` | Magic TDX / Eastmoney | TDX 提供目录/成员，Eastmoney 提供日资金流；两类证据不拼字段 |
| 龙虎榜 | `DragonTigerGateway` | Magic Eastmoney | 当前没有交易所直连来源；不完整席位保持缺失 |
| 公告 | `EventCalendarGateway` | Magic CNInfo | 全市场公告身份、代码、标题、provider 时间和规范 URL |
| 全球财经新闻 | `GlobalNewsGateway` | Magic Eastmoney / CLS / Jin10 / The Paper | 各源独立完整批次；缺摘要或正文保持缺失 |
| 个股新闻 | `SinaInstrumentNewsGateway` | Magic Sina | 按证券和时间窗口读取，区分空批次与来源不可用 |
| 宏观发布 | `EconomicCalendarGateway` | Magic Jin10 | 当前是已发布经济数据，不冒充未来事件日历 |
| 股指期货交割 | `FuturesDeliveryGateway` | Magic CFFEX 官方通知 | 当前真实 admission 未通过，生产保持 Disabled/Unsupported；未来也只覆盖 IF/IH/IC/IM，不使用公式推算 |
| 全球市场 | `GlobalMarketGateway` | Magic Sina | 实时外汇保留上游时间；指数包当前缺 provider `source_at`，因此不能冒充完成时段/隔夜批次，R-08 保持显式降级 |
| 通用 Web 研究 | `GeneralWebResearchGateway` | 已登记搜索 Provider | 研究结果保持搜索证据，不提升为金融成交事实 |
| 盘后复盘 | `ReviewDataGateway` | Magic Eastmoney / Tonghuashun | 复盘事实按来源独立准入，不跨来源拼批次 |

### 显式未支持或尚未验收

- 严格完整的证券主数据若上游批次不完整，会返回 exhausted/unsupported，不使用本地名称缓存补齐。
- 通用标准化 `MoneyFlow` 需要当前未链接的授权 Provider；已接入的 Eastmoney `FundFlowSeries` 是独立合同。
- 通用逐笔、THS/iWencai 自然语言搜索、投资者问答和国务院/工信部政策当前没有生产 Gateway；保持 unsupported，不把搜索结果提升为金融事实。THS 的 exact-date 涨停池只作为 R-03 已登记的完整批次回退。
- CFFEX、交易所或 Eastmoney 的真实网络门禁失败仍是 Gate D 阻塞项；代码不会把网络失败改写成空数据。
- CFFEX 之外的期货交易所交割日没有在本项目中宣称覆盖。

## 数据与资金安全

这些约束来自 [AGENTS.md](AGENTS.md)，对开发、测试和发布都是阻塞门：

| 领域 | 合同 |
| --- | --- |
| 数据真实性 | 生产路径禁止 mock；来源失败返回显式错误，缺失字段保持空值或告警 |
| 数据质量 | 价格必须大于 0；时间缺口/重复和拆分、分红异常必须显式拒绝；BR-171 对相邻收盘变化超过 ±20% 的批次要求人工证据准入 |
| 新鲜度 | 实时报价 ≤5 秒；持仓/现金 ≤30 秒；净值同交易日；日线最多落后 1 个交易日 |
| 测试隔离 | 测试使用 `TEST_CODE` 和物理隔离的数据库、日志、审计与通知路径 |
| 订单安全 | 数量为正且是 100 股整数倍；金额不超过可用现金和 100 万元；60 秒业务 ID 防重；50 万元起二次确认 |
| 审计 | 关键采集、决策、投递和订单保留来源、时间、依据与不可逆身份；强审计保留不少于 5 年 |
| 规则登记 | 去重、互斥、过滤、排序和 limit 必须先登记到 `docs/business_rules.md` |

## 持久化职责

项目有三个职责不同的 SQLite 数据库，不能相互替代：

| 路径 | 职责 |
| --- | --- |
| `data/stock_analysis.db` | 主业务库：行情缓存、研究结果、纸面交易、用户快照、复盘和采集审计 |
| `data/push_analytics.db` | 推送分析库：L7 投递与治理统计。测试使用 `data/test/push_analytics.db` |
| `data/durable_delivery.sqlite3` | BR-192 计数型投递账本：预算、冷却、去重、尝试、receipt、恢复和人工处置。路径固定在编译期仓库根，不能由 `DATABASE_PATH`、运行时 CWD 或环境覆盖 |

JSONL 事件、投递、选择和复盘审计是独立文件边界，不属于上述 SQLite。测试使用独立的
`data/test/TEST_CODE*/...` 数据库、日志和审计根；生产与测试必须物理隔离，回滚不得删除任何账户、持仓、订单或审计证据。

## 配置

需要可用的 Rust/Cargo、SQLite CLI 和真实网络访问。仓库不限定单独的 `rust-toolchain` 文件；实际兼容性以锁文件、CI 和本地门禁为准。

```bash
cp .env.example .env
cargo build --workspace --all-features
```

常用配置：

- `STOCK_LIST`：六位 A 股代码，逗号分隔；
- `DATABASE_PATH`：monitor、通知与运行时主业务库路径；
- `STOCK_DB`：仅供显式读取它的回填、模拟器和合规脚本等离线工具使用；monitor 不读取此变量；
- `MONITOR_ENABLED`：仅控制裸 `monitor` 是否进入长驻服务，默认 `false`；`--test`、`--review` 等显式终端命令不受此开关拦截；
- `MONITOR_REVIEW_TIMEOUT_SECS`：严格复盘顶层超时，默认 300 秒；
- `SCHEDULE_ENABLED`、`LHB_MODE`、`MARKET_REVIEW_ENABLED`：属于 `stock_analysis` 二进制的模式选择，不控制 `monitor`；同名 CLI 参数与环境开关按实现的优先级解析，并非全局互斥配置；
- `BROKER_SOURCE`：执行行情入口，当前 `magic_tdx`、`magic`、`public` 都指向同一统一 Gateway；
- `NEWS_POLL_INTERVAL`：统一新闻 Gateway 的轮询间隔，默认 120 秒；
- `STOCK_ANALYSIS_NEWS_AI_SHADOW_ENABLE`：BR-172 不推送的 NewsAI 不可变审计影子链路，默认 `false`；启用时需配置 `LLM_ROLE_NEWS_AI` 及对应真实模型凭据；
- LLM、搜索和通知变量：只在对应能力实际启用时填写。

不要提交 `.env`、API Key、Webhook、账户截图或本地账户证据。未配置的能力保持不可用，不能使用示例值进入生产。

QMT 仅作为尚未接入的券商执行边界保留，不包含 `qmt-parser` 数据解析依赖。

## 运行

先查看当前参数，避免依赖历史文档中的旧命令：

```bash
cargo run --bin stock_analysis -- --help
cargo run --bin monitor -- --help
```

常用入口：

```bash
# 隔离 E2E：完整渲染 48 个 active monitor 模板，并向独立的非生产飞书
# 会话发送带 TEST_CODE 标签的验收批次；任一回执缺失即非零退出。
# 运行前必须配置 BR196_FEISHU_TENANT_ID / BR196_FEISHU_APP_ID /
# BR196_FEISHU_CONVERSATION_ID，将该目标的身份哈希登记到 release-pinned
# non_production_acceptance allowlist；MAGICLAW_BIN/HOME 必须指向同一独立
# 非生产 app/account 配置，并显式开启验收外发。
BR196_LIVE_FEISHU_ACCEPTANCE=1 cargo run --bin monitor -- --test

# 同一完整模板目录只渲染并写 TEST_CODE 审计，不向飞书发送
cargo run --bin monitor -- --test --push-dry-run

# 隔离验证严格复盘会在没有真实账户证据时失败关闭；不访问生产库、不外发
cargo run --bin monitor -- --test --review

# 严格盘后复盘；可能访问真实数据并使用已配置的通知通道
cargo run --bin monitor -- --review

# 常驻监控；会访问真实数据并可能产生外部通知
MONITOR_ENABLED=true cargo run --bin monitor

# 日线超过一个交易日时执行真实回填
STOCK_DB=data/stock_analysis.db STOCK_LIST=000001 \
  bash tools/one_shot/backfill_daily.sh
```

登记独立测试目标时，只输出域分离后的目标哈希，不输出三个原始标识：

```bash
printf '%s\0tenant_id=%s\napp_id=%s\nconversation_id=%s\n' \
  'stock_analysis.br196.feishu_target_identity.v1' \
  "$BR196_FEISHU_TENANT_ID" "$BR196_FEISHU_APP_ID" \
  "$BR196_FEISHU_CONVERSATION_ID" | shasum -a 256
```

把得到的 64 位小写哈希登记到
`config/br196_non_production_feishu_targets.toml` 后，还必须同步更新并评审
`br196_transport.rs` 中的 release-pinned allowlist 文件哈希。没有独立测试会话时，
裸 `--test` 按设计退出 2；使用 `--test --push-dry-run` 完成本地全模板检查。
三个 `BR196_FEISHU_*` 字段不会回退读取默认 MagicLaw 或仓库 `.env`；每批外发前
还会复验 `MAGICLAW_HOME/.env` 中的 `FEISHU_ACCOUNT_ID`、`FEISHU_APP_ID` 与许可
目标一致。测试环境即使继承 `ALERT_WEBHOOK_URL`，启动健康告警也会在解析 URL
或构造 HTTP 请求前隔离。

`monitor --review` 只执行已登记的严格盘后 dispatcher 子集，不等于常驻服务中的完整 19:00 深度 AI 分析。它可能请求真实公开数据、读写主业务库和审计，并通过已配置的真实通知通道投递。

`V10_DRY_RUN_PUSH=1` 只属于 `--test` 的 TEST_CODE 隔离运行时；`--test`
会自动设置该变量并把 durable delivery 固定到
`data/test/TEST_CODE*/durable_delivery.sqlite3`。普通 `--review` 或常驻生产
monitor 携带该变量会在打开 durable delivery 数据库、预留计数投递或调用
authoritative sink 之前显式退出（exit 2），避免测试回执进入生产审计。BR-196
只有在 TEST_CODE 命名空间、显式 live-acceptance opt-in、三个精确测试目标字段
以及 release-pinned 非生产目标 allowlist 全部成立时，才允许 `--test` 的模板验收
批次绕过该 dry-run 开关。普通生产飞书目标在 denylist 中，不能用于模板测试。
每批均要求飞书 CLI 返回可验证的 `message_id` 与 `platform_msg_id`，且不会写入
生产 counted-delivery 审计。没有独立测试目标时使用
`--test --push-dry-run`，它仍会完整渲染并核对 48 个 active 模板，但不进行任何
外部发送。`--test --review` 是严格复盘的测试隔离入口，不等价于完整模板检查。

`monitor --test`、`monitor --review` 和常驻 monitor 的成功退出含义不同。严格复盘没有任何确认投递时可以按合同非零退出，不能把“无数据”改写成成功。

导入本地真实账户历史证据：

```bash
cargo run --bin import_real_account_snapshot -- \
  --database data/stock_analysis.db \
  --evidence <ignored-local-manifest.json>
```

BR-171 日线跳变确认是人工数据准入，不是选股过滤。先只读取得当前
日线与证券生命周期证据：

```bash
cargo run --bin confirm_daily_change -- \
  --code 600396 --days 60 \
  --database data/stock_analysis.db
```

输出中的 `evidence_token` 绑定代码、相邻日期/价格、涨跌幅、日线
provider/source/batch 以及上市日/公司行动批次。复核后必须显式提交完全
相同的日期和 token；命令会重新采集并在证据变化时拒绝写入：

只读审查的 `--database` 可省略，此时使用 `DATABASE_PATH` 或默认主库；
`--confirm` 写入不可变确认记录时必须显式提供 `--database`。

```bash
cargo run --bin confirm_daily_change -- \
  --code 600396 --days 60 --confirm \
  --previous-date 2026-07-23 --current-date 2026-07-24 \
  --evidence-token <64位小写SHA-256> \
  --database data/stock_analysis.db \
  --operator <操作员身份> \
  --reason <复核依据>
```

确认记录只追加、不可更新或删除。禁止用环境变量、静态 IPO/除权缓存
或自动脚本代替这次显式人工决定。

## 开发与发布门禁

统一数据切换遵循 BR-158、BR-159、BR-164 和 BR-168。当前设计见 [最终切换设计](docs/superpowers/specs/2026-07-25-unified-data-final-cutover-design.md)。

最低验证命令：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
git diff --check

cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Gate D 还要求全仓行覆盖率至少 80%、核心交易/数据链路至少 95%、真实数据门禁、独立审查和完整 PR 证据。任一项未通过时，状态只能是 In Progress 或 Blocked。

## 文档

- [工程规则](docs/ENGINEERING_RULES_V2.md)
- [业务规则注册表](docs/business_rules.md)
- [文档索引](docs/README.md)
- [统一 Gateway / Magic TDX 接入](docs/integrations/magic-tdx-stock-analysis.md)
- [最终数据切换设计](docs/superpowers/specs/2026-07-25-unified-data-final-cutover-design.md)

版本目录记录历史决策，不代表当前生产路径。生产行为以当前代码、业务规则、合规脚本和最近一次可复验门禁结果为准。
