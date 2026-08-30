# Stock Analysis

面向 A 股的行情消费、新闻分析、持仓监控、机会识别、受控推送和盘后复盘系统。
项目以 Rust 编写；生产进程只通过版本化 gRPC 合同消费外部 provider-host 提供的市场数据。

## 主要能力

- 消费行情、历史 K 线、公告、全球新闻、板块、资金流和研究数据。
- 持续监控持仓、自选股、新闻事件和市场状态。
- 生成盘前、盘中、盘后及专题分析结果。
- 通过持久化投递协调器处理去重、租约、投递结果和恢复。
- 保存结构化事件、决策记录、投递回执与审计数据。

## 架构

```text
external provider-host
        │
        │ typed gRPC / optional mTLS client bundle
        ▼
     monitor / CLI
        │
        ├── data_gateway：身份、完整性、新鲜度和批次证据准入
        ├── analysis / selection / decision / risk
        └── durable delivery：SQLite、JSONL 审计和外部通知
```

本仓库不包含 provider 实现、provider server target 或本地 provider fallback。服务不可达、
合同不匹配或证据不合格时，数据网关显式失败，不会切换到本地采集库。

## 代码结构

| 路径 | 用途 |
| --- | --- |
| `src/data_gateway/` | 远程市场数据转换、准入和领域网关 |
| `src/grpc_client/` | gRPC 网络、认证、重试和 envelope |
| `src/grpc_contract/` | operation、参数和 schema 合同 |
| `src/market_domain/` | 本仓拥有的 provider-neutral 市场领域类型 |
| `src/bin/monitor/` | 常驻监控、推送、盘中任务和复盘任务 |
| `src/portfolio/` | 持仓、交易和账户快照 |
| `src/selection/` | 选股合同、决策与审计 |
| `src/review/` | 日度、周度和策略复盘 |
| `src/durable_delivery/` | 持久化投递、去重、租约和恢复 |
| `tests/support/grpc_fixture/` | 仅集成测试编译的本地 tonic fixture |

## 环境要求

- Rust stable toolchain
- SQLite
- C/C++ 构建工具链
- Protocol Buffers 编译环境
- 可访问的外部 market-data gRPC 服务
- macOS/Linux；PAM 相关功能仅在 Unix 平台启用

## 配置

从示例文件开始配置本地环境：

```bash
cp .env.example .env
```

常用配置项：

| 变量 | 用途 |
| --- | --- |
| `STOCK_LIST` | 逗号分隔的自选证券代码 |
| `DATABASE_PATH` | 主业务数据库路径 |
| `MONITOR_ENABLED` | 是否启用常驻 monitor |
| `GRPC_MARKET_ADDR` | 外部数据服务地址；未设置时为 `http://127.0.0.1:18082` |
| `GRPC_MARKET_CLIENT_BUNDLE` | 可选的 mTLS、Bearer token 和连接配置目录 |

运行时 TOML 输入包括 `config/strategy.toml` 和 `config/chain.toml`。完整示例见
[`.env.example`](.env.example)。

## 构建与测试

```bash
cargo build --release --bin monitor --bin grpc_bundle_probe
cargo check --locked --offline --lib
cargo check --locked --offline --bins
cargo test --locked --offline --lib
```

gRPC 集成测试使用 `tests/support` 中的 provider-neutral fixture，不需要生产 provider：

```bash
cargo test --locked --offline --test grpc_channel_e2e
cargo test --locked --offline --test grpc_bridge_e2e
```

## 生产启动

provider-host 在本仓库之外独立构建和部署。先确认其地址及客户端凭据，再运行探针和 monitor：

```bash
GRPC_MARKET_ADDR=https://market-data.example.internal:443 \
GRPC_MARKET_CLIENT_BUNDLE=/absolute/path/to/client-bundle \
./target/release/grpc_bundle_probe \
  --bundle /absolute/path/to/client-bundle --opening

MONITOR_ENABLED=true \
GRPC_MARKET_ADDR=https://market-data.example.internal:443 \
GRPC_MARKET_CLIENT_BUNDLE=/absolute/path/to/client-bundle \
DATABASE_PATH=/absolute/path/to/data/stock_analysis.db \
./target/release/monitor
```

开发环境也可用 `grpc_local_readiness_probe --addr <URL>` 检查一个已运行的外部服务。
账户、证书、token、持仓列表和 webhook 等敏感值不要写入 README、命令历史或日志。

## 已知问题

| 问题 | 状态 |
| --- | --- |
| GlobalNews-ThePaper 失败被折叠为 `internal` (audit reason_code 刷屏) | 预存, 未修 |
| BR-178: selection-v2 recovery 每 60s GlobalSchema authority 失配刷屏 | 预存, 低优先 |
| CompanyFinancialStatements/Sina 证据不匹配 → `invalid_evidence` | 预存数据质量问题 |

2026-08-31: consensus/research 数据问题已修复并部署验证 —
缺评级记录不再整批拒绝 (BR-119/688548, 剔除后其余记录保留); 上游空响应
(`data=[]`) 按 VerifiedEmpty 业务态分类而非 Protocol/invalid_evidence
(605178/300128); `no_current_reports` 业务态在 gRPC wire 保真, 不再折叠为
`internal`。上游修复位于 magic-market-data-rs 48ae41b。

## 常用工具

| 命令 | 用途 |
| --- | --- |
| `cargo run --bin lhb_query` | 龙虎榜查询 |
| `cargo run --bin rsi_optimize` | RSI 参数优化 |
| `cargo run --bin winrate_simulator` | 胜率模拟 |
| `cargo run --bin strategy_attribution` | 策略归因 |
| `cargo run --bin backfill_daily` | 日线数据回填 |
| `cargo run --bin grpc_local_readiness_probe` | 外部 gRPC readiness 检查 |
| `cargo run --bin gateway_quote_probe` | 行情网关探针 |

## 文档

- [当前架构](docs/ARCHITECTURE.md)
- [业务规则注册表](docs/business_rules.md)
- [数据源边界](docs/data-sources-inventory.md)
- [gRPC 已知问题](docs/operations/2026-08-18-data-grpc-known-issues.md)

历史设计文档保留当时的 provider 名称和迁移命令作为审计记录；它们不是当前构建或部署说明。

## 许可证与责任

本仓库当前未声明开源许可证，仅供已授权环境使用。自动交易、实盘推送和账户接入涉及真实资金与外部系统，使用者应独立评估风险并遵守适用法律、券商和平台要求。
