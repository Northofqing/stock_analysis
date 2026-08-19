# Stock Analysis

面向 A 股的实时数据、新闻分析、持仓监控、受控推送与盘后复盘系统。
生产运行采用“数据服务端 + monitor”双进程：服务端统一接入固定 revision
的 Magic Market providers，monitor 只通过 gRPC 消费已验证数据，并将关键
决策、投递回执和状态迁移写入不可变审计链。

> 这是实盘数据系统。任何缺失、过期、冲突或来源不明的数据都必须显式失败，
> 不允许用 mock、默认值、代码前缀推断或静默降级冒充真实依据。开发与发布先读
> [`AGENTS.md`](AGENTS.md) 和
> [`docs/ENGINEERING_RULES_V2.md`](docs/ENGINEERING_RULES_V2.md)。

## 当前状态

- 2026-08-19 的纠正版源码已通过 fresh Gate C：全仓格式化、严格全 feature
  Clippy、全工作区测试和 compliance 均为零退出；release server、no-default
  monitor 与认证 probe 也已重新构建。
- 认证 probe 已分别在隔离端口和生产端口验证 exact `LimitPools`，五条必需的
  非新闻路线全部通过；四条 GlobalNews 全部尝试并由两个独立 provider 达到
  quorum，另外两条保留为显式 degraded，没有改标或补值。
- 当前纠正版 `grpc_market_server` 与 `monitor` 已按受控顺序切换，分别独占
  18082 listener 和生产投递 lease。发布状态仍是
  **In Progress / Live Receipt Pending / Do Not Merge**：必须取得纠正版进程的
  一条真实 provider-backed 业务投递、typed Feishu receipt 和 durable/audit
  精确关联后，才可完成受控 Gate D、PR 和 `master` 合并。
- 历史 P-01 盘前新闻热点取得过真实 Feishu Accepted receipt，并完成 durable
  decision、BusinessDateOnce claim、sink result、immutable audit 与 JSONL
  delivery audit 的精确关联；该证据只证明该次 P-01，不证明修复后候选版本或
  opening release gate。
- 纠正版 Gate C 证据只覆盖当前构建源码；后续若再改 Rust，必须重新运行完整
  Gate C，不能沿用本次结果。
- BR-202 正式 coverage authority 尚未实现；即使其他门重新通过，也只能由有时限
  的受控例外覆盖这一项，不能表述为普通 Gate D 完成。详见
  [`2026-08-19 release cutover controlled exception`](docs/operations/2026-08-19-release-cutover-controlled-exception.md)。
- 实时持仓/现金仍要求 30 秒内的真实账户快照；旧截图或数据库历史记录不能满足
  盘中账户 authority。其他活动问题见
  [`gRPC known issues`](docs/operations/2026-08-18-data-grpc-known-issues.md)。

## 架构

```text
Magic Market providers / remote client-bundle
                    │
                    ▼
          grpc_market_server (default features)
                    │  typed gRPC + evidence
                    ▼
          monitor (--no-default-features)
             │       │        │
             │       │        └─ news / review / intraday producers
             │       └─ data freshness and admission gates
             └─ durable delivery coordinator
                         │
                         ├─ typed Feishu receipt
                         ├─ SQLite decision/attempt/sink authority
                         └─ retained hash-chained JSONL audit
```

主要边界：

- `src/data_gateway/`：公共金融和新闻数据的统一准入、证据与新鲜度校验。
- `src/grpc_server/`、`src/grpc_client/`：本地服务端、ExternalV1 与 LocalBridge 合同。
- `src/bin/monitor/`：常驻调度、P-01、NewsFlash、NewsAI、盘中与复盘生产者。
- `src/durable_delivery/`：计数型投递、去重、lease、恢复、typed receipt 与不可变状态。
- `src/event/`：事件 envelope、投递审计和 retained hash chain。
- `docs/business_rules.md`：去重、过滤、排序、限额和互斥规则的权威注册表。

## 构建

需要 Rust stable、SQLite、可用的 C/C++ 工具链，以及能解析锁定 Git 依赖的环境。

```bash
# 数据 provider 宿主：默认 feature 包含 magic-gateway
cargo build --release --bin grpc_market_server

# 生产 monitor 与认证探针：不链接 provider 实现，只走 gRPC
cargo build --release --no-default-features \
  --bin monitor --bin grpc_bundle_probe
```

禁止把 `monitor` 构建成默认 feature 后再声称完成生产数据隔离。

## client-bundle

`client-bundle/` 保存 mTLS CA、客户端证书/私钥、Bearer token、proto 与连接描述。
它是本机私有运行资产，不是源码：

```bash
chmod 700 client-bundle
chmod 600 client-bundle/bearer-token.txt \
  client-bundle/client-key.pem \
  client-bundle/client.pem \
  client-bundle/connection.txt
```

- 不提交、复制到日志、终端回显或写入 README。
- 证书、私钥、token 和 connection 必须由同一已授权 bundle 提供。
- 更新 bundle 后先运行认证 probe；transport 成功不等于数据准入成功。

## 生产启动顺序

`.env` 至少应明确 `MONITOR_ENABLED=true`、`DATA_GATEWAY_GRPC=1`、
`DATABASE_PATH` 和 `GRPC_MARKET_CLIENT_BUNDLE`。不要把账户或 token 值写进命令历史。

1. 确认没有旧 monitor 持有
   `data/locks/production/monitor-delivery.lock`，且 18082 没有旧 listener。
2. 启动 default-feature `grpc_market_server`：

   ```bash
   GRPC_MARKET_PORT=18082 \
   DATABASE_PATH=/absolute/path/to/data/stock_analysis.db \
   ./target/release/grpc_market_server
   ```

3. 在启动 monitor 前执行认证 opening probe：

   ```bash
   GRPC_MARKET_ADDR=http://127.0.0.1:18082 \
   ./target/release/grpc_bundle_probe \
     --bundle /absolute/path/to/client-bundle --opening
   ```

4. 只有 probe exit 0 且输出 `opening_static_ready=true` 时才启动唯一 monitor：

   ```bash
   MONITOR_ENABLED=true \
   DATA_GATEWAY_GRPC=1 \
   GRPC_MARKET_ADDR=http://127.0.0.1:18082 \
   GRPC_MARKET_CLIENT_BUNDLE=/absolute/path/to/client-bundle \
   DATABASE_PATH=/absolute/path/to/data/stock_analysis.db \
   ./target/release/monitor
   ```

5. 验收必须看到真实 provider-backed Accepted receipt 以及数据库/JSONL 精确关联。
   listener、banner、普通日志或 transport handshake 都不能单独作为发布成功证据。

生产切换、失败回滚和证据格式以
[`controlled exception`](docs/operations/2026-08-19-release-cutover-controlled-exception.md)
及后续正式 release runbook 为准。

## 数据与安全红线

- 实时行情最多 5 秒；持仓/现金最多 30 秒；净值必须同交易日；日线最多落后 1 个交易日。
- 价格必须为正；时间序列 gap/duplicate、证据冲突或批次不完整均显式失败。
- 涨跌停和订单价格范围只能使用同标的、同交易日、来源明确的上下限或显式
  `NoLimit`；禁止按代码、板块、ST 名称或默认百分比推断。
- `TEST_CODE` 只能进入测试环境；生产必须拒绝，测试环境也必须拒绝真实证券订单。
- 订单还必须满足现金、100 股整数倍、单笔 100 万元上限、60 秒幂等，以及
  50 万元以上二次确认。
- 所有关键数据流、订单和权威投递保留来源、时间、决策依据与至少五年的防篡改审计。

完整规则见 [`AGENTS.md`](AGENTS.md)。

## 开发与验证

每项修改按 Gate A → B → C → D 推进。涉及去重、mutex、过滤、排序或限额时，
必须先登记 [`docs/business_rules.md`](docs/business_rules.md) 中的 BR 编号。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

数据新鲜度失败时运行官方回填入口，然后重新执行 compliance：

```bash
bash tools/one_shot/backfill_daily.sh
```

任务只有在测试、失败路径、审计证据和合规检查全部满足后才是 Done；否则必须标记
为 In Progress 或 Blocked。当前 BR-202 coverage authority 缺口仍属于发布后续项。

## 文档入口

- [`docs/README.md`](docs/README.md)：文档索引与版本演进。
- [`docs/business_rules.md`](docs/business_rules.md)：业务规则注册表。
- [`docs/data-sources-inventory.md`](docs/data-sources-inventory.md)：数据源与能力清单。
- [`docs/operations/2026-08-18-data-grpc-known-issues.md`](docs/operations/2026-08-18-data-grpc-known-issues.md)：当前数据/gRPC 问题。
- [`docs/superpowers/specs/2026-08-17-client-bundle-opening-readiness-design.md`](docs/superpowers/specs/2026-08-17-client-bundle-opening-readiness-design.md)：client-bundle 与 opening readiness 设计。

## 许可证与责任

本仓库当前未声明开源许可证。它包含真实交易系统逻辑，仅供已授权环境使用；任何
自动交易、实盘推送或账户接入都必须由操作员独立审查并遵守适用法律、券商规则和
仓库安全门禁。
