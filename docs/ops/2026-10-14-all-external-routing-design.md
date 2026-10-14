# 2026-10-14 「全经过 Windows gRPC」改造设计（草案，未开工）

> 依据：operator 2026-10-14 明确「全经过 windows grpc 端口」；
> 事实基础见 `client-bundle/MAC_DATA_CHANNEL_TOPOLOGY_MEASURED_VS_STATED_20261014_V1.md`（含实测拓扑与 bridge 定位）。
> 本文件是设计草案；**未改任何代码**。

## 1. 结论先说：改动比预想小得多

对两份合同的 `Operation` 枚举逐项比对（`contracts/local_bridge_v1/market.proto` vs `contracts/external_v1_current/market.proto`）：

| 项 | 数量 | 说明 |
| --- | ---: | --- |
| 本地 op | 63 | 含 `UNSPECIFIED` |
| 外部 op | 66 | 含 `UNSPECIFIED` |
| **同名共有** | **61** | **且序号逐项相同（0–60 全部对齐，0 个不同）** |
| 仅本地 | 2 | `CHAIN_BATCH`(61)、`BENCHMARK_BARS`(62) |
| 仅外部 | 5 | `CURRENT_AUCTION_OBSERVATIONS`(61)、`ECONOMIC_RELEASE_OBSERVATIONS`(62)、`ECONOMIC_RELEASE_SCHEDULE`(63)、`OFFICIAL_PUBLICATIONS`(64)、`OFFICIAL_PUBLICATION`(65) |

**更正**：交接文档 §8.3 曾写「序号不可直接对齐」——该说法只对 2 个本地专有 op 成立；
61 个共有 op 的序号完全一致，可按序号（或按名）直接对齐，不需要逐 op 手写映射表。

## 2. 当前实现（改造前的真实边界）

按 op 选路只有一个函数，网关侧只是转发：

```
src/data_gateway/grpc_source.rs:976  fn external_query_method(operation)
        └─ 转发到 → src/grpc_contract/methods.rs:99  MethodIdentity::from_client_operation(ExternalV1, op)
                     └─ → methods.rs:66  ExternalMethod::try_from_local_compat(op)   ← 真正的白名单
```

`try_from_local_compat` 当前只放行 **4 个 op**：
`SecurityMetadata` / `MarketAnnouncements` / `GlobalNews` / `InstrumentNews`。
其余在 **IO 之前**返回 `UnknownMethod`，网关映射为
`invalid_request` / `external_contract_rejected`（消息文案："ExternalV1 operation 未在当前三项 query allowlist 中"）。

另有能力族门槛表 `STATIC_OPENING_CAPABILITY_FAMILIES`（`grpc_source.rs:1049` 起），
按 route 名声明所需的 `ExternalOperation` 族——扩展路由时该表需同步评估。

### 2.1 更正（2026-10-14 15:0x 代码复核后）：白名单不是决定路由的闸门

上面把 `try_from_local_compat` 称为「真正的白名单」**不准确**。按调用链逐段核对
（`grpc_source.rs:3256 query_external_op`），一个 op 真要走外部要连过**三道门，且是串联的**：

| 序 | 闸门 | 位置 | 当前放行面 |
| --- | --- | --- | --- |
| 1 | `external_query_method` → `try_from_local_compat` | `grpc_contract/methods.rs:66` | 本次已 4 → 61 |
| 2 | **`build_external_query_request`（请求信封构造）** | `grpc_client/external_v1.rs:119` | **只 6 个 op**：`BoardFlows` / `GlobalNews` / `InstrumentNews` / `MarketAnnouncements` / `MoneyFlows` / `SecurityMetadata` |
| 3 | 能力声明门控 | `grpc_source.rs:1082` | 由 Windows 侧 `GetCapabilities` 逐 op 决定 |

**闸门 2 才是绑定约束**：白名单第 7 个 op 起，`query_external_op` 会在构造请求时
返回 `ExternalContractError::InvalidParameters`（文案「ExternalV1 operation 或参数未在交付合同中冻结」），
**仍到不了 RPC**。

**并且生产侧根本没人调用**：全部非测试的 `query_external_op` 调用点只有四处
——`SecurityMetadata`(`:3506`)、`MarketAnnouncements`(`:3593`)、`GlobalNews`(`:3650`)、
`InstrumentNews`(`:4118`/`:4150`)——正好等于扩白名单之前的 4 个 op。
`MoneyFlows`(`:3469`) 与 `BoardFlows`(`:3876`) 的 typed 包装是**硬编码走本地** `query_op`，
不随白名单变化。

→ **结论（重要）**：把白名单从 4 扩到 61，在今日生产行为上是**惰性的**，
既不放行任何行情，也不改变任何 op 的取数路线。它的价值是**前置条件与合同文档化**
（守卫测试钉住 61 个共有 op 同名同号）；要真正改路线还必须逐 op：
① 在 `build_external_query_request` 增加该 op 的请求信封 + `schema_version`；
② 让对应 typed 包装按 `external_bundle` 选路（或新增外部专用包装）。

行情类（`RealtimeQuotes` / `MinuteData` / `TechnicalBars` …）这两项都缺，
因此「全经过 Windows」对行情而言**尚未开工**。

**另一道运行时门槛（决定"直接接入"能否成功，逐 op 生效）**：
`ensure_external_connected` → `require_external_capability(capabilities, method)` → `require_external_capability_family`
（`grpc_source.rs:1082` 起）要求**服务端 `GetCapabilities` 把该 op 声明为
`repository_admission == Admitted` 且 `runtime_available == true`**，否则 fail-closed，reason_code 分别为：

| 情形 | reason_code | retryable |
| --- | --- | --- |
| 服务端未发布该 op | `external_capability_missing` | false |
| 已发布但未准入 | `external_capability_unadmitted` | false |
| 已准入但 runtime 暂不可用 | `external_capability_runtime_unavailable` | true |

→ 含义：把白名单从 4 扩到 61 后，**每个 op 是否真走通由 Windows 侧的能力声明逐项门控**，
失败是显式 reason_code 而不是静默降级。这也正是原计划"先探测"想拿到的信息，
现改为由运行时产出（代价是首批调用会出现一批 `external_capability_missing`）。

## 3. 改造方案（按最小改动排序）

### 3.1 核心改动：把白名单从 4 扩到 61 —— **已于 2026-10-14 实施（提交 `41cb2bbfc`）**

> 实施后的实测更正见 §2.1：该改动是**前置条件**，单独实施不改变任何取数路线。

`src/grpc_contract/methods.rs` 的 `try_from_local_compat` 改为：

```rust
fn try_from_local_compat(operation: LocalOperation) -> Result<Self, UnknownMethod> {
    // 本地专有 op：外部目录无对应名，必须显式拒绝（否则 raw 61/62 会撞成
    // 外部的 CURRENT_AUCTION_OBSERVATIONS / ECONOMIC_RELEASE_OBSERVATIONS）
    if matches!(operation, LocalOperation::ChainBatch | LocalOperation::BenchmarkBars) {
        return Err(UnknownMethod);
    }
    let raw = operation as i32;                       // 共有 op 序号一致，可直接对齐
    ExternalOperation::try_from(raw)
        .map_err(|_| UnknownMethod)
        .and_then(Self::try_from_operation)
}
```

配套测试（替换/扩充现有 `grpc_dual_contract_external_method_compatibility_is_closed_before_io`）：
- 断言 61 个共有 op 在 ExternalV1 profile 下**全部** `is_ok`，且映射后的 `as_str_name()` 与本地同名；
- 断言 `ChainBatch` / `BenchmarkBars` 仍 `Err(UnknownMethod)`；
- 新增一致性守卫：两份 proto 的共有 op **名字集合与序号**必须一致（防未来某份 proto 单独改动后静默错配）。

### 3.2 分流策略（关键：不要一次全翻）

**风险**：把 61 个 op 一次性全路由到 Windows，会**动到现在还能用的一部分能力**。
证据：本地路线当前仍有可用项（新实例内 `board-memberships available 398`、
`provider=Tdx source=tdx-block-files`），而 ExternalV1 侧我们**只验证过身份与部分 route**，
op 级可用性**未验证**。

因此建议**分段放行**：

1. **先探测**：对 ExternalV1 逐个 op 做真实调用（沿用既有 mTLS + token + `QueryRequest` 的探测法），
   产出「Windows 侧 op 级可用性表」；
2. **再按表放行**：把探测为可用的 op 加入白名单（可分批：新闻/公告类 → 行情类 → 板块类 → 其余）；
3. **本地路线保留为降级**（见 3.4），不做硬切，便于逐 op 对比与回退。

### 3.3 2 个本地专有 op 的归属（需 operator 决策）

`CHAIN_BATCH`(61) / `BENCHMARK_BARS`(62) 在外部目录**无对应名**。三条路：

- (甲) 继续走本地 bridge（改动最小，与「全经过 Windows」不完全一致）；
- (乙) 请 Windows 侧在外部合同里增加对应 op（需要双方合同变更 + 版本冻结）；
- (丙) 停用（需确认无消费方；`ChainBatch` 是 monitor 复盘消费的 A-10 批次，**不可轻停**）。

建议 (甲) 先保留并在文档标注为例外，同时把 (乙) 列为双方的后续合同项。

### 3.4 本地 provider 栈（`grpc_market_server`）的定位（需 operator 决策）

事实：它是 worktree `.worktrees/tdx-local-provider-20260923` 里的「数据提供者宿主」，
`--features magic-gateway`，依赖与 Windows 同一个私有仓库 `magic-market-data-rs`（rev `75ee2a2b…`），
**自成一个 provider 实例**，直连公网 TDX/web。

选项：
- (甲) 保留为**降级路径**：External 连续失败 N 次后回落，外部恢复后回切（推荐，风险最低）；
- (乙) 保留但仅服务 2 个本地专有 op（3.3 甲）；
- (丙) 停用：运行根少一个进程、少一套审计写入，但失去降级能力，且要处理它的 launchd job 与审计归属。

### 3.5 审计/证据链归属（需 operator 决策）

本地 bridge 当前写入 `data_acquisition_audit`（今日 `review-data-gateway` 997 / `opening-readiness` 537 行）
并承担 descriptor attestation。改走 ExternalV1 后：

- monitor 侧的采集审计**仍会记录**（今日新实例已产生 ExternalV1 来源记录）；
- 但 bridge 侧那部分记录会停止增长 → **审计连续性与历史可比性需要明确规则**
  （建议：保留 bridge 审计表不动，新增以来源区分的字段/视图，而不是混写同一序列）。

## 4. 建议的执行顺序（每步独立可验收）

| 步 | 内容 | 验收 |
| --- | --- | --- |
| 0 | 已完成：ExternalV1 身份钉死发布（`a05eecc0…` / config_hash `94b2b731…` / activation `f56cd085…`） | `ACTUAL_RUNTIME_TYPED_HASH_AND_GATE_MATCH` + ExternalV1 开始供数 |
| 1 | ExternalV1 **op 级可用性探测**（只读，不改代码） | 61 个共有 op 的 Windows 侧可用性表 |
| 2 | 扩白名单到「探测为可用」的 op（分批）+ 一致性守卫测试 | **已做**（提交 `41cb2bbfc`，一次性 4→61）：`grpc_contract::methods` 5/5；但按 §2.1，**实际放行面仍是 6 个 op** |
| 2b | **补 `build_external_query_request` 的逐 op 请求信封**（真正的绑定闸门，含 `schema_version`） | 每个新增 op 一次真实调用（今日 `t0_evidence` 需 `schema_version=2` 即此类坑） |
| 2c | 让 typed 包装按 `external_bundle` 选路（行情类当前无外部包装） | 该 route 的 `source=grpc-mtls` 计数 + 数据新鲜度抽查 |
| 3 | 3.3/3.4/3.5 三项决策落地（按 operator 选择） | 每项单独验收 |
| 4 | 合并为一次发布（新 manifest + 新 activation + 切换 + 验收） | typed 哈希 + 门 + 同 boot 心跳 + op 级抽查 |

**注意**：步 2 与步 4 都改动 `src/`，会使 `config_hash` 变化 → 每次都要新的 activation 与人工批准
（`config_activation_v2` 冻结时序：`reviewed_at ≤ activated_at ≤ effective_from < board 制品到期`）。
board 制品 **2026-10-22T12:00Z** 到期，是这条线的硬时间窗。

## 5. 未验证项（写清楚，不当结论）

- Windows 侧 **op 级可用性**：只验证了身份（四字段）与部分 route（新闻/开盘/自选身份），其余未测。
- Windows 侧**数据质量/新鲜度**：来源时间戳、覆盖率门槛未核。
- 「全经过 Windows」后**延迟与吞吐**：本地 bridge 是进程内/本机，ExternalV1 经 Parallels NAT + Windows；
  对分钟级任务的时延影响未评估。
- 3.3/3.4/3.5 的结论依赖 operator 决策，未定。
