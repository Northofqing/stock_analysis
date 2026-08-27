# 设计文档：gRPC 数据通道 + TDX 异动监听 → 飞书

- 日期：2026-08-13
- 状态：Approved（用户已确认方案 A：服务端委托 data_gateway）
- 合同源：`grpc/market.proto`（magic.market.v1，协议版本 1）+ `grpc/grpc-external-api.md`
- 上游事实：服务端当前不存在（用户确认"现在还没有 GRPC"），需在本仓库自建 mock 服务端用于开发测试

## 1. 背景与目标

`grpc/` 目录提供了一份 Magic Market gRPC 对接合同：`market.proto` 定义 4 个 service
（SystemService / MarketDataService 54 个只读 unary RPC / MarketEventService Subscribe+Replay+GetListenerStatus /
TdxAgentService.OpenStream），`grpc-external-api.md` 定义 QueryRequest/QueryResponse 信封、
认证（Bearer metadata）、TDX 异动订阅与断线恢复语义、错误码映射（§10）。

目标：

1. **现有数据请求改 gRPC**：把生产实际用到的数据族（data_gateway 22 个 Gateway / 43 处 client 构造）
   迁移到 gRPC 客户端调用，feature flag 后置切换，默认行为不变。
2. **小心监听**：通过 `MarketEventService.Subscribe` 订阅 TDX 价格/成交量/成交额异动，
   断线重连 + generation/sequence cursor 持久化，监听到的事件格式化后经现有 push 体系推飞书。
3. **现有程序不改变**：开发测试阶段只新增文件；data_gateway 开关接入为最后一步且默认关闭；
   monitor 二进制、14 处 `push_governor_v3` 调用、现有单测全部不动。

## 2. 范围与边界

### 做

- 用 tonic + prost-build 编译 `grpc/market.proto`，生成客户端与服务端骨架（同一 proto 双端生成）。
- `GrpcMarketClient`：54 op→方法映射、信封构造、Bearer metadata、request_id 生成、
  GetHealth/GetCapabilities、Subscribe/Replay/GetListenerStatus、§10 错误码映射、重试。
- `grpc_market_server` 二进制：mock 服务端，handler 委托 `data_gateway`（spawn_blocking）。
  先实现生产实际用到的 ~20+ op；未实现的返回 UNIMPLEMENTED 并在 capability 表中标注。
- `MarketEventService`：服务端轮询 watchlist 行情 → 快照 diff → 生成 `MarketEventEnvelope`
  （cursor generation+sequence 单调递增）。轮询间隔与异动阈值可配置。
- `grpc_event_listener` 二进制：Subscribe 流 → cursor 持久化 → 断线重连 + Replay 补漏 →
  格式化中文 → 库侧 push_l4-l7 推飞书（新增 PushKind）。
- schema 注册表（`src/grpc_contract/`）：每个 op 的 canonical JSON schema 名 + 版本 +
  结构定义 + 校验，服务端与客户端共享（同一 crate）。
- 测试：模块单测 + 集成测试（真起 server + client）+ e2e dry-run。

### 不做（明确排除）

- `TdxAgentService.OpenStream`：合同 §9 只供同仓库 Windows Agent 使用 → 返回 UNIMPLEMENTED。
- 54 个 op 全实现：生产未用到的数据族只进 capability 表，不实现 handler。
- TLS / mTLS：dev 服务端仅 loopback 明文；远程 TLS 要求（合同 §3/§4）留待真实服务端对接时做，
  客户端预留连接配置位（tls: bool / ca_path: Option）。
- 修改现有 push_governor_v3 调用链、修改 monitor 二进制、修改现有 289 个单测。
- 不把 proto 复制到别处修改（合同 §2：唯一合同源，升级以仓库内文件为准）。

## 3. 架构总览

```
【现有程序（零改动）】
monitor 二进制 → data_gateway (22 Gateway / 43 client) → magic-* crates（真实数据）

【新增 gRPC 侧】
┌───────────────────┐   gRPC    ┌──────────────────────────┐
│ GrpcMarketClient  │ ────────► │ grpc_market_server       │
│ src/grpc_client/  │           │ (tonic, 127.0.0.1:18082) │
└───────────────────┘           │ handler → spawn_blocking │
                                │ → data_gateway(进程内)     │
                                └──────────────────────────┘
                                        ▲
                                        │ Subscribe 流 (MarketEventService)
                                ┌───────┴──────────────┐
                                │ grpc_event_listener  │  cursor 持久化
                                │ → 格式化 → push_l4-l7 │  → 飞书
                                └──────────────────────┘
```

- 新库模块：`src/grpc_contract/`（schema 注册表）、`src/grpc_client/`（网络层）、`src/push_forward/`（监听器推送 glue）。
- 新二进制：`src/bin/grpc_market_server.rs`、`src/bin/grpc_event_listener.rs`。
- 新构建：`build.rs`（tonic-prost-build 编译 `grpc/market.proto`）。
- `Cargo.toml`：新增 `tonic`、`prost`、`tokio-stream`、`tonic-build`(build-dependency)。

## 4. 组件详述

### 4.1 build.rs + 生成代码

```rust
fn main() {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(&["grpc/market.proto"], &["grpc/"])
        .expect("compile market.proto");
}
```

- proto 是唯一合同源，生成代码进 OUT_DIR（不提交）。
- `src/grpc_client/pb.rs`：`include!(concat!(env!("OUT_DIR"), "/magic.market.v1.rs"))`。
- 同一份生成代码双端使用（server 二进制与 client 模块）。

### 4.2 src/grpc_contract/（schema 注册表，服务端与客户端共享）

| 文件 | 内容 |
|------|------|
| `schema.rs` | `const SCHEMAS: &[OpSchema]`：Operation → schema 名 + 版本 + 内容类型 + JSON 结构 + 校验函数。初始以现有 data_gateway 返回类型的 JSON 为准（如 TopStock → `market_data.realtime_quotes` v1） |
| `ops.rs` | Operation 枚举 ↔ proto 方法名映射表（54 op 全列出；`impl_mapped()` 标记哪些已实现） |
| `validate.rs` | canonical payload 校验（未知 schema/version 必须拒绝解析，合同 §5：不能忽略或猜字段） |

schema 注册表是 dev server 自身定义的（dev server 对自己的 schema 是权威）；
将来真实服务端发布后按 §13 交付的 fixture 对齐。

**P2 op 清单起点**（以 2026-08-13 探索结果为据，P2 时从 data_gateway 实际调用逐一核实冻结）：
RealtimeQuotes / HistoricalBars / MinuteData / OrderBooks / MoneyFlows / SecurityMetadata /
Announcements（cninfo 公告）/ GlobalNews / EconomicCalendar / FuturesDelivery / GlobalIndices /
BoardDirectory / BoardConstituents / BoardFlows / LimitPools / StrongStockReasons /
DragonTiger / MarketDragonTiger / MarketRankings / ConceptHits / Consensus /
ResearchReports / BlockTrades / NorthboundDaily（资金流）。共 24 个。

### 4.3 src/grpc_client/（网络层）

| 文件 | 内容 |
|------|------|
| `client.rs` | `GrpcMarketClient`：`query(op, json) -> GrpcBatch`、`get_health()`、`get_capabilities()`、`subscribe(filter, after)`、`replay(filter, after)` |
| `envelope.rs` | QueryRequest 构造（protocol_version=1、request_id 非空唯一、preferred_provider 恒为空）、QueryResponse 解析 |
| `auth.rs` | Bearer token 从 `.env GRPC_MARKET_TOKEN` 读，只进 gRPC metadata（合同 §4：不进请求体/URL/日志） |
| `retry.rs` | §10 错误映射 + UNAVAILABLE 指数退避（重查 health）、DEADLINE_EXCEEDED 有界重试（保留原 request_id） |
| `errors.rs` | gRPC status code → 项目错误类型；ErrorDetail（request_id/operation/provider/reason_code/retryable）解析，不依赖自然语言 message 分支 |

核心语义（合同 §6/§10）：

- `admission=ADMITTED` 才能作为生产数据使用；否则按影子事件处理。
- `complete=false` 不能被当作成功完整批次。
- `source_at` 为空表示无可信源时间，不能用 `observed_at` 代替。
- `batch_id`/provider/单位/来源证据原样保留。

### 4.4 src/bin/grpc_market_server.rs（mock 服务端，方案 A）

- tonic Server，默认监听 `127.0.0.1:18082`（`--port` / `GRPC_MARKET_PORT` 可配；18082 避开 MagicLaw 的 18011）。
- SystemService：GetHealth（live/ready/state）、GetCapabilities（每 op：repository_admission /
  runtime_available / provider / exact_scope / blocker）。
- MarketDataService：QueryRequest → 校验（protocol_version=1、request_id 非空、schema 已注册）→
  spawn_blocking 调 data_gateway 对应 Gateway → 序列化 canonical JSON → QueryResponse
  （admission=ADMITTED、complete=true、observed_at=now、source_at 原样、batch_id 由 evidence 生成）。
- 未实现 op → `UNIMPLEMENTED` + capability 表标注 blocker。
- MarketEventService.Subscribe：事件生成器（见 4.5）→ 每订阅者一个流；
  Replay：有界、同 generation、best-effort（合同 §8）；
  GetListenerStatus：state / terminal_generation / latest cursor。
- TdxAgentService.OpenStream → UNIMPLEMENTED。
- 认证：dev 服务端不强制 token；预留 metadata 校验位（`GRPC_MARKET_TOKEN` 设置后即校验，方便测试）。

### 4.5 服务端事件生成器（TDX 异动检测）

- 轮询：每 `EVENT_POLL_INTERVAL_MS`（默认 3000，可配）调 data_gateway 实时行情，
  覆盖 watchlist（STOCK_LIST + `--instruments` 追加）。
- diff：与上一快照比较，检测：
  - `price`：涨跌幅变化 ≥ `EVENT_PRICE_THRESHOLD_PCT`（默认 0.5%）
  - `volume` / `amount`：相对 N 日均量突增 ≥ `EVENT_VOLUME_THRESHOLD_X`（默认 1.5x）
  - `status`：状态变化（停牌/复牌、涨跌停封板/开板）
  - `reset`：gRPC 流重置/服务重启
- 每个检测到的事件生成 `MarketEventEnvelope`：event_id（sha256 稳定串）、cursor(generation=进程代次,
  sequence 单调递增)、event_kind、provider="tdx-dev", instrument、observed_at/source_at、admission、
  payload=canonical JSON（含 code/name/price/prev_close/change_pct/volume/amount/原因）。

**admission 语义**：dev server 自身是轮询与校验权威，事件标 `ADMITTED`；
同时支持 `GRPC_EVENTS_SHADOW=1` 标 UNADMITTED（测试影子隔离用）。
listener 对 UNADMITTED 事件只记日志不推送（合同 §8 影子事件隔离）。

### 4.6 src/bin/grpc_event_listener.rs（小心监听）

启动流程：GetHealth → GetCapabilities → 读 cursor 文件 → Subscribe(filter, after=cursor) 。

主循环（逐事件）：

1. 校验 protocol_version、admission；UNADMITTED → 只写日志，不推送。
2. 格式化中文消息（示例）：
   ```
   【TDX 异动】600519 贵州茅台 +2.34% → 1500.00（成交量突增 2.1x）
   observed 09:35:01 | provider tdx-dev | seq 123
   ```
3. 经 `src/push_forward/` 推送（4.7）。
4. **成功后**原子写 cursor（temp 文件 + rename），失败不推进。

断线恢复（合同 §8）：

- 流断开 → 指数退避重连（1s→2s→4s…上限 60s，可配）→ `Replay(last_cursor)` 补漏。
- `OUT_OF_RANGE`（cursor 早于重放窗口）→ 记明确 gap 日志 + 重新从最新订阅。
- `FAILED_PRECONDITION`（generation 不匹配/连续性重置）→ 记录后重建订阅，不拼接新旧 generation。
- `RESOURCE_EXHAUSTED` → 按服务端策略退避，记录 gap。
- 持久化要求：cursor.json 含 `generation` + `sequence` + `updated_at`。

### 4.7 src/push_forward/（监听器推送 glue）

- `push_governor_v3` 在 monitor bin 内（`src/bin/monitor/notify.rs:2392`），库侧二进制无法复用。
- 库侧组装 push_l4-l7 最小链路：Dispatcher（L4 去重）→ GovernanceEngine（L5）→
  SinkRouter（L6，复用现有 `MagiclawSink`/Feishu 路由）→ analytics（L7）。
- 新增事件推送标识 `MarketEventAlert`（库侧 push_l4 dispatcher / push_l2 模板体系中的活跃项，
  生产者 = grpc_event_listener；具体枚举位置由实施者定位后冻结，本 spec 不预设未核实的位置）。
- 事件级去重：同 instrument + event_kind 在 dedup 窗口（默认 60s）内只推一次（防轮询抖动）。
- 遵循 v15.x 规则：默认值出声、dry-run 可验（`V10_DRY_RUN_PUSH=1` 只记日志）。

## 5. 数据流

```
【迁移后（P4，开关默认关闭）】
monitor → data_gateway[GRPC开关] → GrpcMarketClient.query(op, schema_json)
        → gRPC → grpc_market_server → data_gateway(服务端进程) → magic-* → 真实数据
        ← QueryResponse ← 反序列化回现有类型 ←

【监听（P3 起可跑）】
grpc_market_server（poll → diff）→ Subscribe 流 → grpc_event_listener
        → cursor 持久化 → 格式化 → push_l4-l7 → 飞书
```

data_gateway 开关（P4）：每个 Gateway 内部加分支——`GRPC_MARKET_ADDR` 未设置 → 现有路径
（默认行为完全不变）；设置后 → gRPC 路径。逐个 Gateway 迁移、逐个验证（grpc_gateway_probe 二进制
逐 op 对比新老路径输出）。

## 6. 可靠性设计

| 场景 | 行为 |
|------|------|
| INVALID_ARGUMENT / UNAUTHENTICATED / PERMISSION_DENIED / UNIMPLEMENTED | 不重试；日志 + 启动 banner（"能力未准入"必须出声） |
| DEADLINE_EXCEEDED | 有界重试（保留原 request_id） |
| UNAVAILABLE | 指数退避 + 重新 GetHealth/GetCapabilities |
| RESOURCE_EXHAUSTED | 按服务端策略退避；流消费者记录 gap |
| FAILED_PRECONDITION | 数据完整性/连续性失败，不能当空成功 |
| INTERNAL | 记录 request_id，停止无界重试 |
| complete=false | 不作为成功完整批次 |
| source_at 缺失 | 不填充 |
| UNADMITTED 影子事件 | 只记日志，不推送、不用于决策 |
| 流断开 | 指数退避重连 + Replay 补漏 |
| generation 变化 | 连续性重建，不拼接 |

## 7. 现有程序不改变的保证

1. 开发测试阶段（P0-P3）**只加文件**：build.rs、src/grpc_contract/、src/grpc_client/、
   src/push_forward/、src/bin/grpc_market_server.rs、src/bin/grpc_event_listener.rs、
   src/bin/grpc_gateway_probe.rs（可选验证工具）。
2. `Cargo.toml` 只加 tonic/prost/tokio-stream 依赖（构建期影响，无运行期行为影响）。
3. P4 是唯一触碰现有文件的阶段：data_gateway 各 Gateway 加开关分支，**默认关闭**，
   `GRPC_MARKET_ADDR` 未设置时行为与今天逐字节一致。
4. monitor 二进制、14 处 push_governor_v3 调用、现有 289 单测全部不动。
5. 每个提交跑 `cargo test --lib` 确认不回归。

## 8. 测试策略

| 层级 | 内容 |
|------|------|
| 单元 | 信封编解码、request_id 重试语义、错误码映射、cursor 序列化/原子写、diff 检测器（价格/量/状态）、schema 校验 |
| 集成 | 真起 grpc_market_server（随机端口）→ client 调 6 个代表性 op（realtime_quotes / historical_bars / minute_data / announcements / global_news / security_metadata）→ 断言 admission/complete/records；Subscribe 流事件注入测试（server 内置事件注入接口或测试注入 poll 结果） |
| e2e | grpc_event_listener 连 dev server，`V10_DRY_RUN_PUSH=1` 验证格式化与推送链路（不真推飞书） |
| 回归 | `cargo test --lib` 全绿（注意全量 lib 预存 flaky：23-28 个顺序依赖失败为 pre-existing，判断回归用模块级测试） |

集成测试避免连真实网络：server 的 data_gateway 委托层加测试注入位
（`GRPC_GATEWAY_TEST_FIXTURE=1` 时 gateway 层返回 fixture），保证离线确定性；
日常手工跑 server 二进制才是真实 provider 路径。

## 9. 实施阶段

- **P0**：build.rs + 生成代码 + src/grpc_contract（schema 注册表骨架）+ src/grpc_client 骨架
  （envelope/auth/errors/retry）+ 单测。验收：`cargo build --lib` 0；模块级测试绿。
- **P1**：grpc_market_server 打通 6 个代表性 op + SystemService + 集成测试。
  验收：集成测试全绿（fixture 模式）；真实模式手工 `grpc_market_server --port 18082` + probe 调通。
- **P2**：服务端补全生产实际用到的全部 op（以 data_gateway 实际调用为准，约 20+）+ capability 表 +
  schema 冻结 + 事件生成器（diff 检测 + Subscribe/Replay/GetListenerStatus）+ 单测。
- **P3**：grpc_event_listener（订阅/重连/cursor/影子隔离）+ src/push_forward + 飞书 dry-run 验证。
  验收：listener 连 dev server 收到真实异动事件 → dry-run 日志显示格式化消息与推送路径。
- **P4**：data_gateway 开关（默认关闭）+ grpc_gateway_probe 逐 op 新旧路径对比 + 逐个 Gateway 迁移。
- **P5**：文档（真实服务端地址/TLS/认证切换说明）+ 交接。

## 10. 验收标准（机器可查）

1. `cargo build --lib` / `cargo build --release --bin grpc_market_server` / `--bin grpc_event_listener` 均 0。
2. 新模块单测全绿；`cargo test --lib` 与改动前无新增失败。
3. 集成测试（fixture 模式）全绿：6 个代表性 op 的 QueryResponse 满足 §6 语义断言。
4. e2e dry-run：listener 收到注入事件 → `data/push_log/<date>/` 出现 `[MarketEventAlert]` 记录
   （dry-run 模式仅日志，不真推飞书）。
5. `grep -RIn 'GRPC_MARKET_ADDR' src/data_gateway/` 显示开关存在且默认分支走现有路径；
   未设置 env 时 monitor 行为与迁移前一致（旧 push log 与数据不受影响）。
6. proto 文件未修改（`git diff grpc/market.proto` 为空）。

## 11. 2026-08-27 修订：T0 批次时间合同与本地双进程切换

### 11.1 背景与已确认事实

2026-08-27 盘中真实探针确认，monitor 同时消费两条彼此独立的 gRPC 路径：

- `LocalBridgeV1` 指向 `127.0.0.1:18082`，承载 `T0Evidence`、`OrderBooks`、
  `HistoricalBars` 等本地委托；
- authenticated `ExternalV1` 使用 client bundle 的独立远端 endpoint，承载静态开盘能力。

因此，远端服务恢复不能证明本地 `grpc_market_server` 已部署。事发时本地 release
进程与二进制均早于当前 checkout；真实 `T0Evidence` RPC 虽返回 `ADMITTED`、
`complete=true` 和非空记录，但 `completed_five_minute[].at` 是
`YYYY-MM-DDTHH:MM:SS` 无 offset 字符串。服务端直接 `serde_json::to_value` 一个包含
`NaiveDateTime` 的领域对象，而客户端按 RFC3339 instant 严格解析，最终以
`invalid_evidence` 拒绝整批。客户端拒绝是正确的 AGENTS 2.3 fail-closed 行为。

同一时段的精确 RPC 还确认：

- `OrderBooks` 可由真实 Tencent provider 返回完整五档、批次身份和来源时间；
- `MoneyFlows` 返回非重试 `unsupported_contract`，原因是规范化资金流所需的
  `magic-emquant-rs` licensed bridge 未链接；
- `BoardFlows` 与逐证券 `MoneyFlows` 是不同合同，禁止互相冒充。

### 11.2 方案选择

采用“显式 T0 wire DTO + 严格 consumer + 候选端口联合切换”。

拒绝以下方案：

1. **只重启当前服务**：源码边界仍会输出无 offset 时间，不能消除根因。
2. **客户端猜测无时区字符串为北京时间**：把坏合同解释为真值，违反 2.3/BR-238。
3. **以 BoardFlows 或零值填补 MoneyFlow**：伪造能力，违反 2.1/2.2/2.8。

### 11.3 `market.t0_evidence` v2 wire 合同

`MagicTdxT0Evidence` 继续作为进程内领域对象；`grpc_server::delegate` 不再直接序列化
该对象，而是投影为服务端拥有的显式 wire DTO。v2 payload 固定包含：

```text
T0EvidenceBatchWireV2
├── requested_at       RFC3339 instant
├── source_at          RFC3339 instant，必须与 envelope 一致
├── observed_at        RFC3339 instant，必须与 envelope 一致
├── batch_id           非空，必须与 envelope 一致
├── time_untrustworthy bool
├── records[]
└── rejections[]
```

每个 `completed_five_minute[].at` 必须是带 `+08:00` offset 的 RFC3339 时间。TDX
五分钟标签在 provider 适配层已被验证为 A 股交易日历的中国标准时间 civil label；
wire 投影使用固定 `+08:00` 表达该既有语义，不依赖部署主机的 `Local` 时区。

客户端必须：

1. 只接纳 v2 已登记 schema；无时区、错误 offset、非法日期一律 `invalid_evidence`；
2. 校验 payload 批次身份与 gRPC envelope 完全一致；
3. 将已验证的 `+08:00` civil label 无损恢复为现有领域对象的 `NaiveDateTime`，不得用
   `.naive_utc()` 改变交易时刻；
4. 空 `records` 时从批级 `requested_at` 恢复批次，保留全部 rejection，禁止用
   `observed_at` 或 consumer 当前时间代填；
5. `time_untrustworthy` 采用“服务端已标记 OR 客户端按真实接收时间复核后标记”；未来
   时间、时间倒置和证据冲突仍是硬错误。

v1 与 v2 不做猜测兼容。旧 monitor 与新 server、或新 monitor 与旧 server 的 schema
不匹配必须显式失败，因此生产切换必须把 server 与 monitor 作为一个发布单元。

### 11.4 OrderBook 能力归因

MoneyFlow 保持 `Unsupported/Missing`，直到规范化逐证券真实 provider 被单独接入并通过
准入，不在本修订中改变。

OrderBook 只允许在 monitor 进程内完成下列全部条件后刷新成功时间：

- T0 响应是 `complete=true`、`ADMITTED` 且批次证据一致；
- 请求代码集合无重复，返回 record/rejection 集合与请求集合一一对应；
- 所有请求代码均为成功 record，rejections 为空；
- 每条 record 的五档盘口、价格和时间均通过现有严格校验。

任一代码失败时不得用其他代码的成功把全局 OrderBook 标成健康。该“全请求集合原子
成功后才刷新能力”的规则在实现前登记为 **BR-253**。不新增独立轮询器、不增加 provider
流量，也不修改推送节流；复用现有 30 秒 T0 真实读取结果。

BR-253 的成功标记是 monitor 进程的真实 provider-backed OrderBook 准入，因而触发
BR-216：OrderBook 必须作为关键 DataMode capability。其成功时间缺失或超过既有 600 秒
预算时进入 `Degraded`，而不是 `Unsafe`；Quote 仍是唯一直接进入 `Unsafe` 的 capability。
MoneyFlow 未接入其独立逐证券真实 provider，继续保持辅助能力。这里“不修改推送节流”
只排除 dedup/rate-limit 变更，不能压制 DataMode 的既有安全后果。

### 11.5 数据流与失败处理

```text
TDX 中国标准时间 civil bar
  -> MagicTdxT0Evidence（领域校验）
  -> T0EvidenceBatchWireV2（显式 +08:00 / 批级证据）
  -> LocalBridgeV1 gRPC envelope
  -> v2 consumer（schema + 批次身份 + 时间 + 数值校验）
  -> MagicTdxT0Batch
  -> exact requested outcomes 校验
  -> 买卖策略；仅全批成功时刷新 OrderBook capability
```

任何 DTO 构造、时间绑定、schema、批次身份、代码集合、数值或 freshness 失败均返回
结构化错误并保留 provider/reason/retryable 审计；不得降级为本地 library、空成功、旧
schema 或默认时间。MoneyFlow 的 `unsupported_contract` 是真实能力状态，不参与 T0
成功判定，也不得被 BoardFlows 成功覆盖。

### 11.6 测试与真实验证

实现按失败测试先行：

- 非空五分钟 bar 的 server/client round-trip 保持 `13:05 +08:00` 对应的原交易标签；
- v2 空 records + 非空 rejections 可无损恢复 `requested_at`；
- 无 offset、非 `+08:00`、未来时间、时间倒置、envelope 冲突全部拒绝；
- 服务端 `source_at/observed_at/batch_id` 与 payload 冲突时拒绝；
- 只有 exact 全请求成功才刷新 OrderBook，部分成功、重复、缺失、额外代码均不刷新；
- 既有 quote 5 秒门和 T0 `time_untrustworthy` 规则不被放宽。

测试身份与完整 round-trip 证据必须分层解释。Task 3 的 server producer DTO 单测和 Task 4
的 consumer converter 单测分别用 `TEST_CODE_` 证明两端的测试身份语义；Task 6 的隔离 fixture
只通过 raw `GrpcMarketClient::query` 证明 `ADMITTED`、schema/version、批次身份、
`TEST_CODE_` payload 与中国 civil-time wire 标签。部分 high-level bridge adapter 会在 RPC 前
按设计通过 production identity resolver 拒绝 `TEST_CODE_`，因此禁止为离线 E2E 放宽
converter/resolver，也不得把 Task 6 raw fixture 合同测试称为 typed domain round-trip 或 live
evidence。只有 Task 8 在未开启 fixture 的候选端口上完成真实 provider typed probe，才证明
完整 server/client typed round-trip。

离线 Gate B/C 全部通过后，在独立临时 target 构建候选 server/monitor，并在备用端口启动
真实 server。候选探针必须覆盖 `T0Evidence`、`RealtimeQuotes`、`OrderBooks`、
`HistoricalBars`，输出只保留脱敏计数、provider、admission、freshness 和结构化错误。

候选通过后停止旧 monitor，再停止旧 server，安装并启动两份候选二进制。切换后先验证
health/capability/T0，再恢复常驻监控；任一检查失败立即成对回滚。随后运行登记的真实
日线 backfill，要求 `stock_daily.MAX(date)` 满足不落后超过一个交易日，并连续观察两个
严格五分钟窗口。未通过 freshness、审计或 live 探针时状态保持 In Progress/Blocked。

### 11.7 回滚与旧模块关系

| 模块 | 处理 | 理由 |
|---|---|---|
| `MagicTdxGateway::get_t0_evidence_batch` | adopt | 保留真实 provider、质量门和完整 outcome 语义 |
| `delegate::fetch_t0_evidence` 直接 `serde_json::to_value` | reject | 泄漏 `NaiveDateTime`，无法形成稳定 wire 合同 |
| `convert::t0_evidence_batch` 严格时间/证据校验 | adopt + deepen | 保持 fail-closed，补充 v2 批级身份和正确 civil-time 恢复 |
| `BoardFlows` 作为 MoneyFlow | reject | 数据族和方法学不同，不具备来源证明 |
| 独立 OrderBook capability 轮询 | reject | 现有 T0 已有真实读取，避免新增流量与并发状态 |

切换前记录旧 PID、启动参数、二进制 SHA-256，并把旧 server/monitor 复制到唯一临时备份
目录。回滚恢复两份旧二进制及原启动参数；源码通过 PR 的 `git revert <sha>` 回滚。
日线 backfill 只追加/幂等接纳已验证真实批次，不以删除历史记录作为常规回滚手段。

### 11.8 非目标

本修订不处理 authenticated ExternalV1 的 ThePaper provider、持仓台账、durable delivery
拒绝或损坏的历史 dispatcher JSONL；这些问题拥有独立根因和审计/回滚边界，不与 T0
数据合同混合修改。本修订不解除订单门禁、不授权真实下单，也不改变策略阈值。
