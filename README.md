# Stock Analysis

面向 A 股的实时数据接入、新闻分析、持仓监控、机会识别、受控推送和盘后复盘系统。
项目以 Rust 编写，生产环境采用数据服务端与监控进程分离的双进程架构。

## 主要能力

- 统一接入行情、历史 K 线、公告、全球新闻、板块、资金流和研究数据。
- 对持仓、自选股、新闻事件和市场状态进行持续监控。
- 生成盘前、盘中、盘后及专题分析结果。
- 通过持久化投递协调器处理去重、租约、投递结果和恢复。
- 保存结构化事件、决策记录、投递回执与审计数据。
- 提供龙虎榜、RSI、胜率模拟、策略归因、回填和数据探针等命令行工具。

## 系统架构

```text
Magic Market providers / remote client-bundle
                    │
                    ▼
          grpc_market_server
          (provider host)
                    │
                    │ typed gRPC
                    ▼
                 monitor
        ┌───────────┼───────────┐
        ▼           ▼           ▼
     market       news       review
    producers    producers   producers
        └───────────┼───────────┘
                    ▼
       durable delivery coordinator
                    │
          ┌─────────┼─────────┐
          ▼         ▼         ▼
       Feishu    SQLite     JSONL audit
       receipt   authority  hash chain
```

生产部署中：

- `grpc_market_server` 使用默认 feature，承载固定 revision 的 Magic Market providers。
- `monitor` 使用 `--no-default-features` 构建，只通过 gRPC 消费数据。
- 两个进程使用同一个业务数据库，但职责和 provider 依赖相互隔离。

## 代码结构

| 路径 | 用途 |
| --- | --- |
| `src/data_gateway/` | 公共金融、行情和新闻数据的统一接入层 |
| `src/grpc_server/` | 数据服务端实现 |
| `src/grpc_client/` | monitor 使用的 gRPC 客户端 |
| `src/bin/monitor/` | 常驻监控、推送、盘中任务和复盘任务 |
| `src/portfolio/` | 持仓、交易和账户快照 |
| `src/signal/` | 信号与信号集合 |
| `src/opportunity/` | 新闻、产业链和候选机会发现 |
| `src/review/` | 日度、周度和策略复盘 |
| `src/decision/` | 排除、轮动和决策支持 |
| `src/risk/` | 订单、资金和仓位风险逻辑 |
| `src/durable_delivery/` | 持久化投递、去重、租约和恢复 |
| `src/event/` | 事件 envelope、投递记录和审计链 |
| `docs/` | 架构、历史版本、运行手册和业务文档 |

## 环境要求

- Rust stable toolchain
- SQLite
- C/C++ 构建工具链
- Protocol Buffers 编译环境
- 可访问 Cargo 与锁定 Git 依赖的网络环境
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
| `DATA_GATEWAY_GRPC` | monitor 是否通过 gRPC 数据通道运行 |
| `GRPC_MARKET_ADDR` | 数据服务端地址，默认 `http://127.0.0.1:18082` |
| `GRPC_MARKET_CLIENT_BUNDLE` | 客户端证书与连接配置目录 |

运行时 TOML 输入为：

- `config/strategy.toml`
- `config/chain.toml`

完整示例和注释见 [`.env.example`](.env.example)。

## 构建

构建默认命令行程序：

```bash
cargo build
```

构建生产数据服务端：

```bash
cargo build --release --bin grpc_market_server
```

构建不链接 provider 实现的生产 monitor 和认证探针：

```bash
cargo build --release --no-default-features \
  --bin monitor --bin grpc_bundle_probe
```

## 本地运行

运行默认程序：

```bash
cargo run --bin stock_analysis
```

运行隔离的 monitor 测试流程，不执行外部投递：

```bash
cargo run --no-default-features --bin monitor -- --test --push-dry-run
```

执行手动盘后复盘：

```bash
cargo run --no-default-features --bin monitor -- --review
```

常用检查：

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --lib
```

## client-bundle

`client-bundle/` 保存 mTLS CA、客户端证书、私钥、Bearer token、proto 和连接描述。
它是本机私有运行资产，不属于源码，不应提交到 Git 或输出到日志。

建议权限：

```bash
chmod 700 client-bundle
chmod 600 client-bundle/bearer-token.txt \
  client-bundle/client-key.pem \
  client-bundle/client.pem \
  client-bundle/connection.txt
```

## 生产启动

生产环境先启动数据服务端，再验证连接，最后启动唯一 monitor 实例。

1. 启动数据服务端：

   ```bash
   GRPC_MARKET_PORT=18082 \
   DATABASE_PATH=/absolute/path/to/data/stock_analysis.db \
   ./target/release/grpc_market_server
   ```

2. 执行 client-bundle opening probe：

   ```bash
   GRPC_MARKET_ADDR=http://127.0.0.1:18082 \
   ./target/release/grpc_bundle_probe \
     --bundle /absolute/path/to/client-bundle --opening
   ```

3. 启动 monitor：

   ```bash
   MONITOR_ENABLED=true \
   DATA_GATEWAY_GRPC=1 \
   GRPC_MARKET_ADDR=http://127.0.0.1:18082 \
   GRPC_MARKET_CLIENT_BUNDLE=/absolute/path/to/client-bundle \
   DATABASE_PATH=/absolute/path/to/data/stock_analysis.db \
   ./target/release/monitor
   ```

生产环境应确保 18082 端口只有一个数据服务端监听，并且只有一个 monitor 持有投递租约。
账户、证书、token、持仓列表和 webhook 等敏感值不要写入 README、命令历史或日志。

## 常用工具

| 命令 | 用途 |
| --- | --- |
| `cargo run --bin lhb_query` | 龙虎榜查询 |
| `cargo run --bin rsi_optimize` | RSI 参数优化 |
| `cargo run --bin winrate_simulator` | 胜率模拟 |
| `cargo run --bin strategy_attribution` | 策略归因 |
| `cargo run --bin backfill_daily` | 日线数据回填 |
| `cargo run --bin grpc_local_readiness_probe` | 本地 gRPC readiness 检查 |
| `cargo run --bin gateway_quote_probe` | 行情网关探针 |

更多可执行程序见 `src/bin/`。

## 文档

- [文档总索引](docs/README.md)
- [业务规则注册表](docs/business_rules.md)
- [数据源清单](docs/data-sources-inventory.md)
- [gRPC 已知问题](docs/operations/2026-08-18-data-grpc-known-issues.md)
- [Agent 规则退役说明](RULES_RETIREMENT.md)

## 仓库级 Agent 指令

仓库不再提供 `AGENTS.md`、工程规则伴随文件或 Copilot 项目指令。
[`CLAUDE.md`](CLAUDE.md) 只保留项目命令、架构和配置事实，不定义额外开发流程。
历史文档中出现的旧规则名称仅用于记录当时背景。

## 许可证与责任

本仓库当前未声明开源许可证，仅供已授权环境使用。自动交易、实盘推送和账户接入
涉及真实资金与外部系统，使用者应独立评估风险并遵守适用法律、券商和平台要求。
