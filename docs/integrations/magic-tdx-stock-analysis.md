# 统一 Gateway 与 Magic TDX 首候选

**状态：Gate B / In Progress**

本文记录当前公共金融和新闻数据边界。模块、测试或编译单独通过都不代表 Gate D 完成。

## 所有权

`src/data_gateway/**` 是 `magic-market-data-rs` Provider 的唯一所有者。业务模块只能消费 Gateway 返回的强类型记录、状态和批次证据。

业务模块不得导入 `magic_*_rs` Provider、构造金融 HTTP transport、保留金融源 URL/协议 parser，或重建本地 fallback。

统一流向：

```text
官方或公共来源
  -> 固定 Git revision 的 Magic Provider
  -> DataBatch + SourceEvidence
  -> Router 完整批次 admission
  -> stock_analysis::data_gateway
  -> 业务投影和不可变采集审计
```

## 行情路由

Magic TDX 是 A 股行情的第一个路由候选，不保证每次请求都胜出。当前 Gateway 的固定候选顺序是：

| 能力 | 路由 |
| --- | --- |
| 实时 A 股报价 | Magic TDX → Magic Tencent → Magic Sina；TDX 当前因缺少足以证明 5 秒 SLA 的高精度 `source_at` 而不能赢得严格路由 |
| 未复权日线 | Magic TDX → Magic Tencent → Magic Sina → Magic Baidu |
| 分钟线 | Magic TDX → Magic Tencent → Magic Sina |
| 五档盘口 | Magic TDX → Magic Tencent → Magic Sina |
| A 股指数 | Magic Tencent |
| A 股证券身份 | Magic Tencent → Magic Sina；完整证券主数据仍可能 Unsupported |
| 上市日 / 公司行动生命周期 | Magic TDX；身份解析器不得补造生命周期证据 |

来源只能以完整批次获胜。严格路由拒绝缺 `source_at`、身份不一致、部分或过期的批次，然后按登记顺序尝试下一个 Magic Provider。

盘中和盘后日线共用 `HistoricalBarsGateway`。不存在盘后专用的本地短路，也不存在 consumer-owned HTTP fallback。

## 其他公共数据

| 领域 | Gateway | 强类型上游 |
| --- | --- | --- |
| 财务报表 / 市场统计 | `CompanyDataGateway` | Magic Sina / Tencent |
| 个股及盘后资金 / 北向统计 | `CapitalDataGateway` | Magic Eastmoney / HKEX |
| 研报 / 一致预期 | `ResearchDataGateway` / `ConsensusDataGateway` | Magic Eastmoney |
| 板块 | `BoardDataGateway` | Magic TDX（目录/成员）/ Eastmoney（日资金流） |
| 龙虎榜 | `DragonTigerGateway` | Magic Eastmoney |
| 全市场公告 | `EventCalendarGateway` | Magic CNInfo |
| 全球财经新闻 | `GlobalNewsGateway` | Magic Eastmoney / CLS / Jin10 / The Paper |
| 个股新闻 | `SinaInstrumentNewsGateway` | Magic Sina |
| 宏观已发布数据 | `EconomicCalendarGateway` | Magic Jin10 |
| 股指期货交割通知 | `FuturesDeliveryGateway` | Magic CFFEX；当前真实 admission 未通过，生产保持 Disabled/Unsupported |

上表只说明所有权和类型边界。每个调用方、失败路径、真实网络门禁和生产证据仍必须单独验收。

## 证据与失败

接纳批次必须保留 provider、source、provider 时间（上游提供时）、本地观察时间和非空 batch ID。

`observed_at` 不能替代 `source_at`。缺失可选字段保持缺失；缺失必填字段、坏身份、坏价格、时间重复/缺口、部分分页或不明空批次必须拒绝。

实时报价不超过 5 秒。日线最多落后 1 个交易日。每次请求按 BR-159 记录一条不可变采集结果；审计失败时 Gateway 失败关闭。

当前必须显式保留的限制：

- 上游证券主数据不能证明完整时返回 exhausted/unsupported；
- 标准化通用 `MoneyFlow` 未链接授权 Provider，不由其他资金字段冒充；
- 通用逐笔、THS/iWencai 自然语言搜索、投资者问答和国务院/工信部政策当前没有生产 Gateway，保持 unsupported；THS exact-date 涨停池仅按 R-03 的完整批次回退使用；
- CFFEX 只覆盖 IF/IH/IC/IM，不推导其他交易所交割日；
- 网络、认证或协议失败不改写成 verified empty。

## BR-171 人工数据准入

`HistoricalBarsGateway` 遇到相邻有效收盘变化绝对值超过 20% 时不会把
股票判为坏票，也不会自动放行。操作员先运行只读审查：

```bash
cargo run --bin confirm_daily_change -- --code 600396 --days 60
```

确认时必须显式传入 `--confirm`、输出中的相邻日期与
`evidence_token`、数据库路径、操作员身份和非空理由。CLI 会重新采集
当前日线及 Magic TDX 生命周期证据；任何字段或批次变化都使确认失败。
成功记录追加到不可更新/删除的哈希链账本，之后只有完全匹配该证据的
日线批次可以通过准入。完整参数以
`cargo run --bin confirm_daily_change -- --help` 为准。

## 验证

```bash
rg -n "magic_[a-z0-9_]+_rs" src \
  --glob '*.rs' --glob '!src/data_gateway/**'

cargo test --test unified_data_architecture \
  br164_financial_and_news_acquisition_is_gateway_owned -- --exact

cargo run --bin monitor -- --test --review

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

零违规命中、编译和测试仍不等于发布完成。Gate D 还要求覆盖率、真实数据门禁、独立审查和生产链证据。

## 回滚

按数据域提交并使用 `git revert <domain-sha>` 回滚。不得恢复已删除的本地采集器或默认数据作为 fallback。

代码回滚不能删除或改写 `stock_daily`、账户快照、持仓、订单、收盘估值和审计记录。
