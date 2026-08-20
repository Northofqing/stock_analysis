# 常驻监控与数据能力降级设计

状态：Gate A 设计待用户书面复核。
业务规则：BR-246。
关联规则：BR-135、BR-148、BR-170、BR-238、AGENTS 2.1/2.2/2.3/2.4/2.7/2.8/2.10。

## 1. 问题与目标

当前生产 monitor 在取得进程唯一租约、数据库和不可变审计 authority 后，仍在所有
resident producer 之前同步循环 `external_static_opening_readiness()`。九路静态检查中
任一路长期失败，都会让 `p01_scheduler_loop`、`monitor_loop`、`news_monitor_loop` 和
`data_mode_monitor_loop` 全部不可达。2026-08-20 的真实结果是静态检查 8/9，唯一
`InstrumentNews` 失败；monitor 因此未常驻，数据异常通知本身也无法继续发送。

目标是把“进程可持续运行”与“某类业务数据可用于决策”分开：

1. authority 基础设施成立后，monitor 必须进入 resident loops；
2. 静态、实时、账户、持仓产业链等业务数据能力在后台持续重试并显式审计；
3. 数据异常时，DataMode/风险状态消息继续通过既有真实治理和 sink 投递；
4. 具有独立完整证据的业务消息可继续，其既有 combined banner 明确展示缺失能力；
5. 缺少自身必要证据的消息、价格建议、承接判断、订单和纸面成交继续 fail closed；
6. 数据恢复后同一进程自动恢复对应 producer，无需人工重启。

本设计不把 8/9 改称 9/9，不放宽 ExternalV1/LocalBridge、freshness、完整性、来源身份、
审计或订单安全合同，也不允许缓存、默认值、空批次或成本价成为生产证据。

## 2. 启动条件分类

### 2.1 仍然阻断进程的 authority 基础设施

- 参数、环境与 TEST_CODE/production 隔离失败；
- production monitor 唯一租约失败；
- 核心数据库身份/初始化失败；
- durable delivery authority、不可变审计链或 JSONL writer 失败；
- sink/目标配置无法建立可审计投递能力；
- 配置值本身非法，例如未支持的 `BROKER_SOURCE`；
- resident task 意外退出、panic 或 shutdown/drain 失败。

这些失败使进程无法安全审计、去重或投递，不能降级成“继续运行”。

### 2.2 不再阻断进程的业务数据能力

- BR-238 九路 static/auth/contract readiness 的任一路失败；
- BR-238 三路 live-session readiness 失败或尚未进入实时窗口；
- 真实账户、持仓、净值、行情、OrderBook、MoneyFlow、News 等 capability 不可用或过期；
- 启动期持仓产业链刷新传输、解析或证据失败。

这些状态必须保留真实错误、provider、reason、retryable 与时间，进入后台重试和消息
降级；不得使整个 resident scheduler 不可达。

## 3. 方案比较

### A. 常驻进程 + producer 级资格门（采用）

把 BR-238 static 检查改为与 live 检查相同的后台 supervisor。supervisor 只生产
诊断/审计状态，不生产可转移的“数据许可”。每个 producer 在实际消费时继续执行自身
完整性和 freshness 门。

优点：最符合故障隔离；DataMode 能持续出声；独立数据源互不短路；恢复无需重启。
风险：必须用测试锁定 authority 硬门仍位于 producer 前，避免误把审计失败也降级。

### B. 保留全局 static 启动门，只增加等待消息（拒绝）

进程仍卡在 producer 前，唯一可能发送的是旁路通知；这会复制治理/sink 并继续让其他
独立 producer 停摆，不能满足持续运行。

### C. 所有模板照常渲染，缺失字段留空并加警告（拒绝）

当一条消息没有任何完整独立事实时仍渲染，会把 unavailable 混同为 partial report，
违反 AGENTS 2.2/2.3。价格建议和交易也可能被误读为有效结论。

### D. 新增独立 watchdog 进程（暂不采用）

可在 monitor 退出时告警，但增加第二套租约、投递去重和审计 authority，不能解决
monitor 内部各 producer 的故障隔离。本轮不引入。

## 4. 选定接口与数据流

monitor authority 初始化完成后启动一个小接口的深模块：

```text
resident_readiness_loop()
  ├─ 每 30 秒执行 BR-238 static diagnostics
  ├─ 当前实时窗口执行 live readiness
  ├─ 追加既有 acquisition/readiness audit
  ├─ 状态变化时输出稳定 banner；相同状态只按既有提醒规则节流
  └─ 永不向 producer 返回可替代消费证据的 Boolean permit

main resident loops
  ├─ p01_scheduler_loop
  ├─ monitor_loop
  ├─ news_monitor_loop
  └─ data_mode_monitor_loop

position_chain_refresh_loop()
  ├─ resident 启动后立即执行一次 BR-170 exact-position refresh
  ├─ 失败或部分失败保持 typed outcome
  └─ 每 300 秒重试，积压 tick 使用 Skip

每个 producer
  ├─ 获取自己的真实 batch/evidence
  ├─ 完整性/freshness/identity 成功 -> 仅消费完整独立组件
  └─ 失败 -> typed rejection/audit；不渲染依赖该证据的结论
```

`external_static_opening_readiness()` 继续作为 release probe、`grpc_bundle_probe` 和严格
生产切换证据，要求 9/9；它不再是 resident monitor 的进程启动许可。后台 supervisor
优先使用能保留全部独立 route 结果的 diagnostics 接口，并按既有 BR-238 quorum 计算
状态。诊断结果只用于可见性、审计和恢复检测。

## 5. 消息语义

| 场景 | 允许 | 禁止 |
|---|---|---|
| 任一数据能力异常 | DataMode/风险状态消息，列出真实缺失能力和恢复条件 | 把失败改成空/0/缓存成功 |
| 一条业务消息仍有完整独立事实 | 推送这些事实；已有 combined banner 附数据缺失提示 | 暗示缺失组件参与了结论 |
| 一条消息的必要事实全部不可用 | 只写 typed rejection/dispatcher/readiness audit | 渲染看似完整的业务卡片 |
| Quote/OrderBook/MoneyFlow 不完整 | 风险提示、非价格事实 | 价格目标、承接判断、做 T 或下单建议 |
| 账户/持仓超过 30 秒或跨日 | 公共 SourceOnly 事实、账户异常提示 | 仓位/现金/盈亏推断和账户型建议 |
| 数据恢复 | 同进程恢复对应 producer，发送受既有去重/冷却约束的恢复消息 | 重启后绕过去重或补发过期建议 |

DataMode 卡是全局数据异常的唯一通用通知。SourceOnly 卡不强行拼入与其无关的账户
banner；combined-account 卡继续使用既有 `data_missing_note`。这样既“出声”，又不让
一张警告文字成为缺失证据的替代品。

## 6. 失败模式与审计

- static supervisor 的一次调用失败：追加 OpeningStatic failure audit，保持上次状态，
  30 秒后重试；非 retryable 表示该 route 的失败分类，不再终止 resident process。
- diagnostics 自身无法建立连接：记录 typed capability failure；resident loops 继续，
  各 consumer 按自己的调用结果失败关闭。
- readiness 审计失败：不得宣称 ready；由于全局不可变投递审计已在启动期验证，单次
  acquisition audit 写失败记录为 data/audit capability 异常并持续重试。若写入失败
  表明全局 authority 已失效，则由既有 lifecycle supervisor 终止，不在本设计猜测放行。
- 持仓产业链刷新失败：记录 BR-170 unavailable；独立后台任务每 300 秒重试。恢复前
  不以旧链接、未知链或空链授权候选/交易，也不阻断其他 resident producer。
- DataMode 消息 sink 失败：保留未确认状态，按 BR-116/BR-135 重试；不写成已通知。
- resident task 提前返回或 panic：仍由 BR-141 终止进程，防止“活着但不工作”。

所有日志和 PR 证据只记录 capability/provider/reason/hash，不包含账户身份、持仓列表、
凭据、新闻标题或 webhook。

## 7. 旧模块关系

| 模块 | 决策 | 原因 |
|---|---|---|
| `external_static_opening_readiness` | 保留为严格 probe/release gate | 9/9 仍是生产发布与完整能力证据 |
| `external_static_opening_diagnostics` | 采用为 resident supervisor 输入 | 保留全部独立 route 结果和 quorum |
| `opening_live_readiness_loop` | 合并/复用 | static 与 live 采用同一生命周期和可见性语义 |
| `data_mode_monitor_loop` | 采用 | 已有独立 scheduler、Unsafe reminder 和 confirmed-delivery 状态 |
| consumer 自身 gateway/freshness gate | 保留 | readiness 状态不可替代消费时证据 |
| 旧同步 startup readiness loop | 删除 | 单路数据失败错误阻断所有 resident producer |
| `refresh_startup_position_chains` | 纳入独立后台刷新任务 | 首次立即执行，之后每 300 秒重试；失败路径仍显式，成功项仍可提交 |
| generic push/旧通知旁路 | 不新增 | 继续使用现有治理、sink 和审计 authority |

## 8. 测试与验收

TDD 首个回归必须证明：static readiness 连续失败时，resident supervisor 会重试，且
`data_mode_monitor_loop` 和一个独立 producer 已启动；原代码应 RED。随后覆盖：

1. 8/9、0/9、9/9 和连接失败；
2. static 失败不阻断四个 resident loops；
3. position-chain 第一次失败不阻断 loops，300 秒后重试；积压 tick 不补跑；
4. authority/租约/审计初始化失败仍在 provider 和 sink 前退出；
5. 一个 producer 数据失败不阻断另一独立 producer；
6. DataMode Unsafe/reminder 必须取得真实 confirmed sink 回执后才提交状态；
7. combined 卡展示缺失能力，SourceOnly 卡不伪造账户状态；
8. Quote/账户不新鲜时价格建议、订单和 paper execution 调用次数均为 0；
9. readiness 从 false 恢复 true 时无需重启且不重复发送已去重业务消息；
10. production source 不含等待 static success 才注册/启动 producer 的循环。

Gate C/D 命令：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

生产验收分两段：

- 故障段：保持已知 8/9 或隔离协议故障，monitor 持有唯一 production lease 并持续
  运行；DataMode/风险消息出现真实 typed Accepted 回执；InstrumentNews 依赖消息为
  explicit rejection；价格建议和订单为 0。
- 恢复段：gRPC 修复后不重启 monitor，后台状态自动变为 9/9；对应 producer 下一合法
  时窗恢复，并形成 provider evidence、durable decision、typed remote receipt 和不可变
  delivery audit 的精确 join。

README 和 PR 必须明确：resident liveness 不等于 data readiness；8/9 不能标记为完整
数据健康或 Gate D production-ready。

## 9. 回滚

回滚本 PR 后恢复旧的同步 static 启动门：

```bash
git revert <merge-commit>
cargo build --release --bin monitor
```

回滚不得删除任何 readiness、DataMode、delivery、账户或市场数据审计。若回滚发生在
数据仍异常期间，应先停止唯一 monitor，再部署旧 binary，避免两个 delivery owner。
