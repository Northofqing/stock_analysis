# CLAUDE.md

Project context only. This file defines no repository-specific agent process or development gates.

## Commands

```bash
cargo build
cargo test --lib
cargo run --bin monitor
cargo run --bin monitor -- --test --push-dry-run
cargo run --bin monitor -- --review
```

## Architecture

The project is an event-driven live A-share trading monitor. Its main bounded contexts are:

| Context | Directory | 说明 |
| --- | --- | --- |
| Push pipeline | `push_foundation/`（约 23% 代码量）, `durable_delivery/` | 推送管道与持久化投递协调器（21 表状态机、fence、sha256 envelope 审计、幂等投递） |
| Database | `database/` | diesel + rusqlite 双栈并存；`selection_v2` 规模最大 |
| Monitor (bin) | `bin/monitor/` | 主进程：数据模式、新闻、竞价、盘后复盘、counted 接线 |
| Trading | `trading/` | 虚拟盘（paper）买卖、FIFO 批次账本、T+1、最小手数 |
| Selection / activation | `selection/` | BR-183 激活状态机；**改动 src/config 后重启必须重发 activation** |
| gRPC | `grpc_client/`, `grpc_contract/`, `contracts/` | 上游 magic-market VM (10.211.55.3:50051) 数据通道；LocalBridgeV1/ExternalV1 拆分 |
| Market | `data_gateway/`, `market_analyzer/` | fail-closed 数据准入（Admitted* + BatchEvidence） |
| Signal / Opportunity / Review / Decision / Risk | `signal/`, `opportunity/`, `review/`, `decision/`, `risk/` | 信号、候选池、复盘链、决策、风控 |
| Indicators | `indicators/` | MACD/KDJ/RSI/布林等（实现扎实但未接入下单） |
| News | `news/`, `bin/monitor/news_aggregator_init.rs` | 新闻聚合（NewsFlashGate/N-01/N-02）、NewsAI 分析链 |

## Configuration

- `.env`: `STOCK_LIST`, `DATABASE_PATH`（`WECHAT_SEND_SCRIPT` 是死配置；真实通道是外部 magiclaw 二进制）
- Runtime TOML inputs: `config/strategy.toml`, `config/chain.toml`
- 激活文件: `config/selection/selection_activation.v1.json` — 改 src/config 后必须重发（`selection_activation_prepare` 工具 + 未来 effective_from），否则生产拒绝启用 selection 配置

## 关键运行事实（2026-09-21 评估与处置后）

- **生产 monitor** 由 launchd（`com.stockanalysis.monitor`）管理；launchd 损坏时 nohup 直跑（重启 Mac 后 `launchctl load` 恢复）
- **counted 持久投递**：22 个 PushKind 已接线（durable_delivery catalog 45 kinds）；counted kind 走 `push_counted_with_binding`，generic governor 对 counted kind 拒绝（fail-closed）
- **交易能力边界**：无真实券商接入（虚拟盘 paper 交易）；买入门由 `compute_account_mode_metrics_blocking` 桩函数关闭（BR-103 水位未接线）；T-14/T-15 等真实券商回报 feed 缺失
- **上游数据问题记录**：`grpc_handoffs/`（竞价期数据源、VM 时钟、新闻时间戳等交接文档）
- **已知缺陷清单**：`docs/audits/2026-09-21-系统评估.md`（含修复优先级 A/B 档；B 档需产品决策）

## 修复/上线纪律

- 每 Unit 接线流程：前提核实（穷举 dispatch 家族）→ RED → 实现 → 回归 → dry-run（`--test --push-dry-run` EXIT 0）→ 复审 → 提交
- 改动 src/config → `cargo build --release` → 重发 activation（effective_from 未来时刻）→ 重启 monitor → 验证 `capability=disabled` 行为 0
- durable 层 Uncertain 决策需人工裁定（`resolve_stale_uncertain` 工具）；冷却头 Uncertain 会永续阻塞新决策
