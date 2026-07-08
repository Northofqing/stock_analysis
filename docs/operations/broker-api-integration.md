# broker API 接入方案

> **状态**: 调研文档 (v14.1 task #168)
> **代码层**: `src/broker.rs` NoopBroker stub 已落地 (#165)
> **下次接入 PR**: 待评估

## 1. 目标

把券商的 4 类实时推送接进 monitor:

| 类型 | 触发时机 | 用途 |
|------|----------|------|
| 委托回报 | 下单后 100ms 内 | T-04/T-14 推送状态更新 |
| 成交回报 | 撮合后 | trade 落库 + T-15 |
| ST 状态变更 | 公告日 | F7 `stock_position.st_type` 同步 |
| 报价 Tick | 实时 | K 线 + 实时风控 |

## 2. 候选方案

### 方案 A: QMT (迅投) — 推荐
- **接入方式**: 共享内存 + TCP (本地 IPC)
- **SDK**: `xtp` (Python) / 官方 C++ 库
- **优**: A 股覆盖全、延迟 < 1ms、风控 SDK 完整
- **劣**: 需券商授权 QMT 账号、共享内存易冲突
- **cost**: 券商月费 200-500 RMB

### 方案 B: ptrade (恒生)
- **接入方式**: HTTP REST + WebSocket
- **SDK**: 官方 Python SDK
- **优**: 部署简单 (远端)、HTTP 标准化
- **劣**: 延迟 50-200ms、不适合高频
- **cost**: 月费 100-300 RMB

### 方案 C: 模拟盘 (magiclaw 当前路径)
- **接入方式**: magiclaw HTTP API (已有 `DEFAULT_MAGICLAW_API_ADDR`)
- **优**: 零成本、不依赖券商、测试用
- **劣**: 不真下单 (simulation)、无真实 ST/quote 推送
- **status**: 已实现 (`src/bin/monitor/main.rs:23-35`)

## 3. 落地路径 (按方案 A 展开)

### Phase 1: 接口抽象 (v14.1 #165 ✅)
- `src/broker.rs` BrokerPush trait + NoopBroker stub
- `lib.rs` 加 `pub mod broker`
- 3 个单测覆盖

### Phase 2: 真实接入 (未来 PR)
1. `Cargo.toml` 加 broker SDK 依赖 (e.g. `qmt = "0.1"`)
2. `src/broker/qmt.rs` 写 `QmtBroker` impl BrokerPush
3. `src/broker/registry.rs` 加 `register_from_env()`: 读 `BROKER_IMPL` env 选实现
4. `main.rs` 启动时调 `broker::register_from_env()` 替换 NoopBroker
5. trading::open_position 改调 `broker::with(|b| b.push_st_type(code, st))`

### Phase 3: 风控 + 监控
- 接入断线重连 (QMT SDK 自动重连 + 应用层心跳)
- 推送延迟监控 (> 5s 报警)
- 数据落 broker_log 表 (跟 push_log 同模式)

## 4. 风险点 (AGENTS 2.5 / 2.7)

| 风险 | 缓解 |
|------|------|
| 测试 / 实盘账户串 | `env_guard` 已在 positions.rs 拒绝实盘代码入 test DB |
| 推送数据缺失 | 跟 stock_daily 一样, 缺失 → warn, 不静默填充 (AGENTS 2.2) |
| 断网 / broker 宕 | 启动时 broker::with 不 panic (NoopBroker 兜底), 后续推送降级 |
| 鉴权 token 泄露 | magiclaw token 已经在 `MAGICLAW_TOKEN_MEM_CACHE` 加密, 复用同模式 |
| 限流 | broker 推送 > 100/s 触发本地 throttle (跟 magiclaw push_governor 同型) |

## 5. 测试隔离

- 单元测试默认走 NoopBroker (broker::register 不调)
- 集成测试加 `BROKER_IMPL=test` 显式注入 mock 实现
- E2E (cargo run -- --e2e) 走 NoopBroker, broker 推送不污染测试

## 6. 落地时间表 (估算)

- Phase 1 (stub): ✅ 1 commit
- Phase 2 (真接): 2-3 周 (SDK 学习 + 接口对接 + 单测/集成测试)
- Phase 3 (风控): 1 周
- 总计: 1 个月 (1 dev, 兼职)

## 7. 决策点 (待用户拍板)

1. **选哪个方案**: QMT (低延迟, 月费) / ptrade (简单, 中延迟) / 保持 magiclaw 模拟?
2. **预算**: 月费 ≤ 300 RMB 行不行?
3. **优先级**: 4 类推送先接哪个? (建议: ST 状态 > 委托回报 > 成交 > 报价)
4. **是否真接 vs 永远 NoopBroker**: 当前 stub 已足够支持 F7/回填, broker 真接是 optional

## 8. 相关文件

- `src/broker.rs` — stub 实现 (#165)
- `src/database/positions.rs:45` — save_position 注释 "// broker 推送更新时同步写"
- `src/bin/monitor/main.rs:23-35` — magiclaw token 缓存 (可复用鉴权模式)
- `src/database/mod.rs:113-123` — stock_concepts 表 (broker 推 concepts 也可写这)
