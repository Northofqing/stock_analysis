# 数据提供者宿主独立仓库拆分设计

**状态：** 设计已批准；`stock_analysis` 的 Phase 5 删除工作已于 2026-08-31 完成。跨仓发布和生产证据仍由外部 provider-host 仓库负责。

## 实施状态（2026-08-31）

- `Cargo.toml`、`Cargo.lock` 和生效 Cargo 依赖图已不含 `magic-*` 包；
- 本地 provider server、provider-only probes、旧 feature gate 和所有本地 fallback 已删除；
- `stock_analysis` 只保留 provider-neutral 领域类型、gRPC 客户端和准入校验；
- gRPC 集成测试使用 `tests/support/grpc_fixture.rs` 中的测试专用服务，不恢复生产 provider host；
- 本文其余“当前事实”保留为 2026-08-23 的设计基线和迁移审计记录。

## 1. 目标

从 `stock_analysis` 仓库删除所有生效的 `magic-*` 包和源码引用。公共行情与新闻数据采集迁入独立构建、独立部署的 `magic-market-provider` 仓库。

`stock_analysis` 继续负责证据准入、新鲜度、交易决策、订单安全和持久审计。

最终状态必须满足：

- `stock_analysis` 的源码、`Cargo.toml`、`Cargo.lock` 和 Cargo 依赖图不存在 `magic-*` 包；
- `stock_analysis` 无法调用本地 provider 库，也不存在静默回退路径；
- `provider-host` 是唯一允许依赖 `magic-*` 适配器的模块（Module）；
- 两个仓库只通过版本化、provider-neutral 的 gRPC 合同通信；
- 数据源、新鲜度、校验和审计失败始终显式返回；
- 修改 `stock_analysis` 的普通业务代码不再重新编译 provider 实现。

## 2. 设计时基线（2026-08-23）

本设计基于 2026-08-23 对当前 HEAD 的只读检查：

- `cargo metadata --no-deps --format-version 1` 返回 82 个 Cargo target；
- `rg -n '#\[(tokio::)?test\]' src tests` 约有 3,765 个测试；
- `du -sh target` 返回 41 GiB；
- 单独运行一个库测试时，`stock_analysis` 测试 target 编译耗时 5 分 43 秒，测试执行耗时 0.00 秒，总耗时 `357.24s`；
- 不修改代码重复运行同一命令，总耗时 `2.54s`；
- `cargo tree --no-default-features` 不含 `magic-*`；
- 默认 Cargo 依赖图仍包含 14 个直接 `magic-*` 依赖及其传递依赖；
- `src/data_gateway/grpc_source.rs::KEEP_LOCAL_OPS` 仍包含 `strong_stock_reasons`；
- `docs/superpowers/plans/2026-08-15-p4-migration.md` 把彻底删除 manifest 依赖定义为 M5 终态，但当前只落地了 feature 隔离。

因此，当时实现只把 provider 排除在生产 monitor 构建之外，并未把 provider 从仓库或默认开发、测试依赖图中移除；该基线已由上述 Phase 5 完成状态取代。

## 3. 范围

### 3.1 包含

- 新建独立 `magic-market-provider` 仓库；
- 建立 provider-neutral 的 `market-contract` 包；
- 迁移 `grpc_market_server`、provider 适配器和 provider probes；
- 在 `stock_analysis` 建立 `MarketEvidencePort` 接缝（Seam）；
- 完成 `strong_stock_reasons` gRPC 接桥；
- 删除本地 provider/library fallback；
- 删除 `magic_compat`、`magic-gateway`、所有 `magic-*` 依赖和 provider-only target；
- 建立跨仓合同兼容、shadow、生产验证和回滚证据；
- 更新仓库指令和架构文档；
- provider 删除后重新测量编译时间。

### 3.2 不包含

- 改变信号、选股、排序或订单业务语义；
- 改变新鲜度窗口、仓位阈值或订单阈值；
- 新增通用事件总线或第二种远程传输；
- 允许 provider DTO 直接进入交易或决策代码；
- 在拆分过程中更新 `magic-market-data-rs` 固定 revision；
- 在生产路径使用 mock、fake、默认值或假数据；
- 为缩短开发时间而削弱完整 Gate C/D。

## 4. 方案对比

### 4.1 采用：两个仓库，合同包位于 provider 仓库

`magic-market-provider` 是一个 Cargo workspace，包含轻量 `market-contract` 包和深模块 `provider-host`。`stock_analysis` 只固定依赖 `market-contract`，不加入 provider workspace。

该方案只有两个仓库，gRPC 合同具有唯一权威来源，并且 `stock_analysis` 的 workspace 命令不会编译 provider 实现。

### 4.2 拒绝：建立第三个合同仓库

独立合同仓库的归属更中立，但会增加一套发布、兼容和 CI 流程。目前不存在第二个独立合同生产者，没有足够收益。

只有出现第二个非 provider 合同生产者时，才重新评估该方案。

### 4.3 拒绝：两个仓库各保留一份 proto

复制 proto 初期简单，但会让合同漂移成为常态。descriptor hash 只能在漂移发生后报警，不能提供唯一所有者，不适合实盘数据接缝。

## 5. 目标架构

```text
第三方行情/新闻 provider
              │
              ▼
┌─────────────────────────────────────────────┐
│ magic-market-provider 仓库                  │
│                                             │
│ provider-host 模块                          │
│ - magic-* 适配器                            │
│ - provider 路由                             │
│ - 原始响应校验                              │
│ - provider 证据与采集审计                   │
│                                             │
│ market-contract 包                          │
│ - proto 与生成 DTO                          │
│ - 闭合错误类型                              │
│ - operation/capability 目录                 │
│ - 版本与 descriptor hash                    │
└─────────────────────┬───────────────────────┘
                      │ mTLS gRPC
                      ▼
┌─────────────────────────────────────────────┐
│ stock_analysis 仓库                         │
│                                             │
│ MarketEvidencePort 接口（Interface）        │
│ - GrpcMarketAdapter：生产                   │
│ - InMemoryTestAdapter：cfg(test)/TEST_CODE  │
│                                             │
│ data_gateway 深模块                         │
│ - canonical identity                        │
│ - freshness/evidence admission              │
│ - 构造 Admitted<T>                          │
│                                             │
│ trading / decision / monitor / push         │
└─────────────────────────────────────────────┘
```

gRPC 是自有远程依赖。`MarketEvidencePort` 是真实接缝，因为它存在两个合理适配器（Adapter）：生产 gRPC 适配器和仅测试使用的内存适配器。

provider 内部适配器属于实现细节，不通过该接口暴露。

## 6. 模块职责

### 6.1 `market-contract`

该包只拥有两个仓库之间最小且完整的接口：

- provider-neutral 请求与响应 DTO；
- canonical operation ID 和 capability 声明；
- 合同版本与确定性的 descriptor/catalog hash；
- typed evidence metadata；
- 闭合错误与 disposition 类型；
- source-backed 价格限制状态：`Bounded`、`NoLimit`、`Unavailable`。

该包不得包含 provider 实现、环境变量读取、数据库访问、业务计算、`magic-*` 类型或生产 fallback。

新增字段默认采用 additive 兼容方式，并至少兼容两个已部署版本。删除字段或改变既有语义必须升级合同 major version，并另行通过跨仓切换设计。

### 6.2 `provider-host`

该模块把所有 provider 复杂度隐藏在 gRPC 接口后，负责：

- `magic-*` 依赖及固定 revision 策略；
- provider-specific ID、客户端和错误转换；
- provider 路由和 source-specific retry 决策；
- 原始响应的完整性、数值、连续性、重复和冲突校验；
- provider 类型到合同 DTO 的转换；
- 采集侧防篡改审计。

provider 失败必须返回 typed failure，不得以 fake、默认值、静默截断或无资格空集合报告成功。

### 6.3 `MarketEvidencePort`

该接口表达应用视角下的证据获取。调用方不需要了解 tonic channel、provider client、feature flag 或 provider-specific error。

生产 `GrpcMarketAdapter` 负责 mTLS、deadline、合同协商和 transport-to-domain 失败转换。

`InMemoryTestAdapter` 只能在测试编译中存在，只接受 `TEST_CODE` identity，生产二进制无法构造它。

### 6.4 `stock_analysis::data_gateway`

该模块继续拥有应用准入。只有验证以下条件后，才能从合同 DTO 构造私有字段的 `Admitted<T>`：

- canonical instrument 和 trading session identity；
- 完整性和 provider evidence；
- source time 与 local acquisition time，二者不得互相替代；
- 2.4 对应的新鲜度窗口；
- 价格大于零且数值有限；
- 时间连续且无重复；
- 适用时的拆股、分红一致性；
- 需要下单时的 source-backed 价格区间证据。

交易、决策、监控和推送模块只能消费已准入的领域值，不能消费原始 gRPC DTO。

## 7. 证据归属与审计链

provider 仓库记录采集事实，`stock_analysis` 记录准入与决策事实。两侧保留相同的不可变 `batch_id`，适用时同时保留 event identity。

完整证据链为：

```text
provider request
  → raw provider response
  → normalized contract batch
  → stock_analysis admission
  → decision/order/push outcome
```

provider 审计包含 provider identity、provider timestamp（若来源提供）、local acquisition timestamp、batch identity、完整性、校验结果和 retryability。

应用审计包含相同 batch identity、新鲜度判定、拒绝或准入依据，以及下游决策 rule ID。

两侧审计不得暴露凭据、账户标识、真实持仓清单或其他受保护信息。关键审计至少保留五年；append、hash 或 sync 失败必须 fail closed。

## 8. 失败语义

合同至少区分：

- `TransportUnavailable`：gRPC、mTLS、DNS 或连接失败；
- `ProviderUnavailable`：上游失败或有效路由耗尽；
- `ContractMismatch`：版本、descriptor hash 或 catalog 不匹配；
- `EvidenceRejected`：缺失、过期、不完整、非法、不连续、重复或冲突；
- `OperationUnsupported`：provider-host 不支持请求的 operation/capability。

所有状态必须显式且 fail closed：

- 错误不得转换为空成功；
- 缺失时间不得由数据库时间或进程时间代替；
- transport/provider 失败不得回退本地 `magic-*`；
- 合同不匹配时拒绝启动生产数据通道，并输出明确错误 banner；
- `Unavailable` 价格限制证据必须拒单；
- `NoLimit` 只能来自同标的、同交易时段的 provider 证据。

禁止用零、无穷、代码前缀、板块名、ST 名称或默认百分比推断 `NoLimit`。

## 9. 迁移顺序

每个阶段都必须是独立、可复审的 PR/change set。前一阶段没有完整验收证据时，不得进入后一阶段。

### 9.1 Phase 1：抽取合同

建立 `magic-market-provider` 仓库和 `market-contract` 包。迁移 canonical proto、DTO 生成规则、operation catalog、闭合错误、版本和 hash，不改变 wire 行为。

验收条件：

- 新旧 descriptor/catalog hash 按字节一致；
- 双向序列化测试通过；
- `stock_analysis` 运行行为不变；
- 两个仓库固定到同一个合同 revision。

### 9.2 Phase 2：迁移 provider 实现

在可行范围内保留 Git 历史，迁移：

- `src/grpc_server/**`；
- `src/bin/grpc_market_server.rs`；
- `src/data_gateway/**` 中 provider-specific 的实现；
- TDX、Tencent 和其他 provider probes；
- provider 真实数据与集成测试；
- 所有 `magic-*` 依赖声明。

旧路径暂时只用于比较和回滚，不再增加新业务行为。

验收条件：

- provider-host 能提供所有已接桥 operation；
- provider 实时数据和协议检查通过；
- provider 采集审计持久化成功；
- 同一请求与证据窗口下，新旧 server 返回相同合同 hash。

### 9.3 Phase 3：关闭最后一个 operation 缺口

完成 `strong_stock_reasons` 的完整 gRPC 路径，将它从 `KEEP_LOCAL_OPS` 移入 `HOOKED_OPS`，然后要求 `KEEP_LOCAL_OPS.is_empty()`。

若迁移触及 filter、sort、limit、dedup 或 mutex，必须先在 `docs/business_rules.md` 注册或更新规则，并引用对应 BR ID（2.10）。

保真和失败行为未验证前，不得删除本地 fallback。

### 9.4 Phase 4：shadow 与权威切换

先部署向后兼容的 provider-host。旧路径保持权威，新路径以 non-authoritative shadow 身份运行至少一个完整交易日。

比较内容包括完整性、provider/source identity、时间戳、数值、顺序和确定性结果 hash。

shadow 使用真实数据，但不得参与订单、决策或推送。每个差异都必须留下显式阻断记录。

观察窗口无差异后，部署 gRPC-only `stock_analysis`，将 provider-host 设为唯一权威来源。切换后发生故障必须 fail closed，不能重新启用旧路径。

### 9.5 Phase 5：从 `stock_analysis` 删除 provider 引用

删除：

- `src/magic_compat/**`；
- `src/grpc_server/**` 和 `grpc_market_server` target；
- provider-specific gateway 实现和 provider-only probes/tests；
- 所有 `#[cfg(feature = "magic-gateway")]` 分支；
- `magic-gateway` feature 及其默认启用；
- 14 个直接 `magic-*` 依赖和剩余传递 lockfile 包；
- `DATA_GATEWAY_GRPC_DISABLED`、library fallback 和其他本地 provider 选择开关。

保留并加深：

- `src/grpc_client/**`：生产 gRPC 适配器实现；
- `src/data_gateway/**` 中 provider-neutral 的准入逻辑；
- canonical identity、evidence 和 freshness 校验；
- 订单、决策与审计模块；
- 进程级 gRPC 合同测试。

### 9.6 Phase 6：清理与性能证据

更新仓库架构和指令文档，删除失效 M5 声明，运行两个仓库的完整 Gate，并在与 `357.24s` 基线相同的机器和 toolchain 上重新测量 focused build。

## 10. 测试策略

### 10.1 合同包

- deterministic descriptor/catalog hash；
- 请求、成功和每个闭合失败的 round trip；
- 与上一受支持合同版本的兼容测试；
- 源码检查：公共接口不存在 `magic_*` 类型；
- capability negotiation mismatch 测试。

### 10.2 Provider 仓库

- adapter-level 解析与校验；
- missing、bad、stale、duplicate、gap、conflict 显式失败；
- provider timeout、rate limit、unavailable；
- 每个 operation 的进程级 gRPC 测试；
- 已注册 provider 的真实数据 canary；
- 核心采集、校验和审计链覆盖率至少 95%。

测试 fixture 和测试适配器必须使用 `TEST_CODE` namespace，且不能进入生产二进制。生产验证只使用真实 provider 证据。

### 10.3 `stock_analysis`

- 测试穿过 `MarketEvidencePort`，不穿透 provider 实现；
- 成功和拒绝证据的 gRPC 进程测试；
- transport、mTLS、version、unsupported operation 和 provider error；
- freshness 与 canonical identity 准入；
- 每个拒绝输入都断言 decision/order/push 下游调用次数为零；
- 测试与实盘账户、标的、数据库、日志和审计隔离。

迁出浅层 provider 实现后，旧测试应迁移或在新接口上替换，不得在两个仓库复制同一批测试。

### 10.4 跨仓生产证据

- 所有已登记 operation 都通过 gRPC 到达 provider-host；
- 切换前 `KEEP_LOCAL_OPS` 为空，删除阶段后该符号不存在；
- provider 与应用审计通过 `batch_id` 精确 join；
- 实时行情不超过 5 秒，持仓/现金不超过 30 秒，净值为同一交易日，日线/历史不超过一个交易日；
- shadow 差异为零，否则必须阻断切换；
- 生产日志证明 provider → admission → consumer 完整链路。

## 11. 验收标准

Phase 5 完成后，`stock_analysis` 必须通过：

```bash
cargo metadata --format-version 1 | jq -e '
  [.. | objects | .name? // empty | select(startswith("magic-"))]
  | length == 0'

! rg -n \
  'magic_(tdx|market_|eastmoney|sina|tencent|ths|cninfo|cls|jin10|thepaper|exchange|baidu)' \
  src tests build.rs Cargo.toml

! rg -n 'magic-gateway|KEEP_LOCAL_OPS|DATA_GATEWAY_GRPC_DISABLED' \
  src tests build.rs Cargo.toml

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

focused build benchmark 必须在一次真实的局部源码变更使应用库 target 失效后，重复原始命令：

```bash
/usr/bin/time -p cargo test --lib \
  block_on_async_with_timeout_panics_with_flavor_error_in_current_thread
```

结果必须与 `357.24s` 基线并列记录。在相同机器、toolchain、feature set 和 dependency-cache 状态下，架构目标为至少降低 50%。

若结果未达标，则阻断“构建性能目标”完成，但不得因此跳过任何数据或资金安全检查。

`magic-market-provider` 也必须通过等价的格式、严格 lint、全量测试、合规、覆盖率和真实 provider 数据检查，才能发布。

## 12. 失败模式与回滚

| 失败 | 必须行为 | 回滚 |
| --- | --- | --- |
| 合同 hash/version 不匹配 | 拒绝启动或建立 channel；输出明确 banner | 部署上一兼容合同/provider artifact |
| provider-host 不可用 | 显式 retryable transport failure；零本地 fallback | 仅在旧 provider 路径仍有效时回滚上一验证应用二进制 |
| provider 数据不完整或过期 | 拒绝 batch；decision/order/push 消费次数为零 | 修复 provider 适配器或路由，不得伪造证据 |
| shadow 出现差异 | 阻断权威切换 | 保持旧路径权威，返回 Phase 2/3 修复 |
| audit append/hash/sync 失败 | fail closed | 修复审计存储，不得关闭审计 |
| Phase 5 编译或测试回归 | 返回 Gate B，并复核 Gate A 接缝 | `git revert` 对应 Phase 5 commit/PR |
| 生产切换回归 | 停止新 monitor，恢复上一验证二进制；provider-host 保持兼容 | artifact rollback，并留下审计事件 |

回滚以 release artifact 或 commit 为单位。运行时环境变量无法恢复已删除的编译期实现。

切换前必须保留已签名或已验证的上一版二进制；provider-host 至少在一个回滚窗口内支持上一合同版本。

## 13. 旧模块处置

| 现有模块/路径 | 决策 | 原因 |
| --- | --- | --- |
| `src/grpc_client/**` | 采用并加深 | gRPC 接缝的生产适配器 |
| `src/grpc_contract/**`、`grpc/market.proto` | 迁移/桥接到 `market-contract` | 形成唯一跨仓接口 |
| `src/grpc_server/**` | 迁移 | provider-host 实现属于 provider 仓库 |
| `src/bin/grpc_market_server.rs` | 迁移 | provider-host 可执行文件不应是本仓 target |
| provider-specific `src/data_gateway/**` | 迁移 | provider 适配器不得泄漏到应用准入 |
| provider-neutral `src/data_gateway/**` | 采用并加深 | 拥有 canonical evidence admission 与 `Admitted<T>` |
| `src/magic_compat/**` | 切换后删除 | 仅为过渡镜像，保留会延续 provider 耦合 |
| `magic-gateway` feature | 切换后删除 | feature 隔离不等于仓库隔离 |
| provider probes/replays | 迁移或明确退役 | 其依赖所有者是 provider-host |
| `KEEP_LOCAL_OPS` / library fallback | 最后 operation 切换后删除 | 第二生产路径违反单一权威与 fail-closed |

## 14. 数据红线映射

- **2.1：** provider 失败不得变成 mock、默认值或空数据；生产只有 gRPC 适配器。
- **2.2：** 缺失合同/provider 字段保持缺失或拒绝准入。
- **2.3：** provider 与应用准入分别校验数值、连续性、重复、公司行动和 source-backed 价格限制。
- **2.4：** 应用准入继续执行现有新鲜度窗口，shadow 和切换不得绕过 freshness gate。
- **2.5：** 测试适配器、标的、账户、数据库、日志和审计使用 `TEST_CODE` 并保持物理隔离。
- **2.6：** 订单只消费同交易时段的 `Bounded`/`NoLimit` 证据，`Unavailable` 必须拒单。
- **2.7：** provider 采集审计与应用决策审计通过 batch/event identity 关联，并至少保留五年。
- **2.8：** 迁移后的 save、sync、reconcile 必须真实操作目标数据源，禁止 logging-only 实现。
- **2.10：** 迁移或调整 dedup、mutex、filter、sort、limit 前先登记业务规则并引用 BR ID。

## 15. PR 证据要求

每个迁移 PR 必须包含：

- `Refs:` 本设计对应章节和相关合同/provider spec；
- `Data-Redlines:` 适用的 2.x 规则；
- `OldModules:` 每个旧模块的采用、迁移或删除决定；
- `Threshold-Proof:` 若没有阈值或配置变化，明确写 `N/A`；
- `Business-Rules:` 受影响 BR ID，特别是 `strong_stock_reasons` 和 routing/filter；
- `Validation:` focused check 和该阶段所需 Gate；
- `Rollback:` 精确 commit/artifact 回滚步骤。

只要仍存在阻断级复审发现、新鲜度失败、合同不匹配、shadow 差异、审计 join 缺失、覆盖率缺口或生产证据缺口，该阶段就不能宣称 release-ready。
