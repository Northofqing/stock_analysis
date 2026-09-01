# Stock Analysis

面向 A 股的行情消费、新闻分析、持仓监控、机会识别、受控推送和盘后复盘系统。
项目以 Rust 编写；生产进程只通过版本化 gRPC 合同消费外部 provider-host 提供的市场数据。

## 主要能力

- 消费行情、历史 K 线、公告、全球新闻、板块、资金流和研究数据。
- 持续监控持仓、自选股、新闻事件和市场状态。
- 生成盘前、盘中、盘后及专题分析结果（含 AI 新闻证据卡片与产业链报告）。
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
| `src/data_provider/` | 进程级抓取缓存层（委托统一 Gateway，非 provider 实现） |
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
| `WECHAT_WEBHOOK_URL` | 企业微信机器人 webhook 地址 |
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

> 注：全量 `cargo test --lib` 存在预存 flaky（顺序依赖失败的集合每次不同，与改动无关）；
> 判断回归时优先运行被改模块的模块级测试。

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

### 配置激活（BR-183 部署仪式）

selection 配置受哈希激活保护：**每次改动 `src/` 后重启前必须重新生成 activation**
（`config/selection/selection_activation.v1.json`，`effective_from` 必须为未来时刻，等待窗口
过后再重启），否则生产进程拒绝启用 selection 配置并静默关闭相关 producer：

```bash
./target/release/selection_activation_prepare print-activation <reviewed_by> <RFC3339未来时刻>
```

重启后验证 banner 中 `producer_scheduling=enabled` 及对应 producer 状态。

### 自检与复盘

```bash
cargo run --bin monitor -- --test --push-dry-run   # 推送链路自检（不真实发送）
cargo run --bin monitor -- --review                # 盘后复盘任务
```

## 数据源已知问题与处置

| 问题 | 处置 |
| --- | --- |
| GlobalNews-ThePaper 失败折叠为 `internal` (audit reason_code) | 已归因：magic-market-data-rs 服务端偶发 500，经 reason_code 保真传递；本地 fail-closed 正确，生产已恢复（8/31 后无复现） |
| BR-178: selection-v2 recovery 每 60s GlobalSchema authority 失配刷屏 | **已修复** (6e69fea)：authority 未接线时明确跳过 recovery/due tick 并打一次性 banner，不再每 60s failed closed |
| CompanyFinancialStatements/Sina 证据不匹配 → `invalid_evidence` | 已归因并部分修复：consensus/research 缺评级整批拒绝已修复（2026-08-31）；R-08 CFFEX 为上游 FuturesDelivery 服务端 500（本地 fail-closed 正确，复盘失败审计每次出声属设计，待上游修复） |

2026-08-31: consensus/research 数据问题已修复并部署验证 —
缺评级记录不再整批拒绝 (BR-119/688548, 剔除后其余记录保留); 上游空响应
(`data=[]`) 按 VerifiedEmpty 业务态分类而非 Protocol/invalid_evidence
(605178/300128); `no_current_reports` 业务态在 gRPC wire 保真, 不再折叠为
`internal`。上游修复位于 magic-market-data-rs 48ae41b。

2026-09-01: BR-178 修复部署 (6e69fea, PID 15810) — 启动打印一次性
`[selection-v2][BR-178] GlobalSchema authority 未接线` banner 后，每 60s 刷屏终止；
selection-v2 未发布阶段该模块明确跳过而非反复失败。

## 常用工具

| 命令 | 用途 |
| --- | --- |
| `cargo run --bin lhb_query` | 龙虎榜查询 |
| `cargo run --bin rsi_optimize` | RSI 参数优化 |
| `cargo run --bin winrate_simulator` | 胜率模拟 |
| `cargo run --bin strategy_attribution` | 策略归因 |
| `cargo run --bin backfill_daily` | 日线数据回填 |
| `cargo run --bin selection_activation_prepare` | 生成/打印 selection 配置激活（部署仪式用） |
| `cargo run --bin grpc_bundle_probe` | 外部 gRPC 合同与链路探针 |
| `cargo run --bin grpc_local_readiness_probe` | 外部 gRPC readiness 检查 |
| `cargo run --bin gateway_quote_probe` | 行情网关探针 |

`src/bin/` 下还有回填（`backfill_catalyst_watchlist` 等）、导入（账户/持仓快照）与
专题探针等工具；`cargo build --release` 会一并产出。

## 文档

- [当前架构](docs/ARCHITECTURE.md)
- [业务规则注册表](docs/business_rules.md)
- [数据源边界](docs/data-sources-inventory.md)
- [gRPC 已知问题](docs/operations/2026-08-18-data-grpc-known-issues.md)

历史设计文档保留当时的 provider 名称和迁移命令作为审计记录；它们不是当前构建或部署说明。

## 许可证与责任

本仓库当前未声明开源许可证，仅供已授权环境使用。自动交易、实盘推送和账户接入涉及真实资金与外部系统，使用者应独立评估风险并遵守适用法律、券商和平台要求。
