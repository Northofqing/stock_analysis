# 冻结设计输入恢复与后继说明（2026-10-07）

原 RFC 输入和设计来源目录按精确字节冻结。后继把现行实现说明写入这两份原件，造成检查失败。本次从真实 Git 对象恢复匹配原 manifest 的字节；后继内容及其提交继续保留在本页与 Git 历史，不改变源码或重写原 manifest。

这些后继描述各自属于其当时的实现记录，不是当前全部生产能力验收。当前模块状态见完整平台交接和本轮上线记录。

## docs/Project_Architecture_Blueprint.md

- 原冻结 SHA-256：`a1acf98ec960880934285d1a71ecf6fba068d809b08174ab51871a91645f2f75`。
- 后继写入 SHA-256：`1eb6f5596ac410f86ce3221338047268d29bc0ea2c71b712c53be9be4b955e98`。
- 恢复来源：`402a7da74870a81fc58975628203923da4dc35f8^`。

保留的后继改动：

```diff
diff --git a/docs/Project_Architecture_Blueprint.md b/docs/Project_Architecture_Blueprint.md
index 53e6fa527..0a1fe3eaa 100644
--- a/docs/Project_Architecture_Blueprint.md
+++ b/docs/Project_Architecture_Blueprint.md
@@ -1753,7 +1753,7 @@ Data Contract Gate / DataHealthSnapshot
 | PR-2 BannerSnapshot | 单一结构化 health truth | 当前仍是 `Mutex<Option<BannerCtx>>`，缺失时 caller 记录字符串并跳过（`src/bin/monitor/main.rs:1666-1695`） | `PROPOSED`；不能把现有 BannerCtx 重命名后算完成 |
 | PR-3 ErrorCode | typed code/retryable/severity/retry_after | exact type 0 命中；当前大量 reason_code 仍分散 | `PROPOSED`；应与 §24 `ReasonCode`、readiness reason schema 合并设计，避免第二套错误 taxonomy |
 | PR-4 Log rotation | 日切、50MB、30 日、warn 分流 | 无 `src/log/rotate.rs`/tracing rotation 实现 | `PROPOSED`；保留期与 secrets/redaction policy 要先定 |
-| PR-5 `--health` | 1 秒内人读/JSON health | 无 health command/module；现有 opening readiness 通过日志暴露 | `PROPOSED`；应读取同一 snapshot，不触发 provider 或业务 sink |
+| PR-5 `--health` | 1 秒内人读/JSON health | `src/bin/monitor/health_cmd.rs` 已提供 `--health [--json] [--test]`：只读固定根目录下由 live banner owner 原子写入的脱敏 AccountMode/DataMode 快照，并核对新鲜度、独占实例锁和启动身份；缺失、过期、旧进程或模式不健康返回非零 | `PARTIAL`；当前 `coverage=banner_account_data_only`，尚无统一 BannerSnapshot、错误聚合、per-source 状态或真实运行时 1 秒时延验收；CLI 不触发 provider 或业务 sink |
 | PR-6 25+ metrics | operational metrics catalog | 当前 `MonitorMetrics` 明确只有 6 项（`src/bin/monitor/metrics.rs:1-67`） | `PARTIAL`；标签基数、单位和 authoritative/BestEffort 结果须与 §24 对齐 |
 | PR-7 per-source breaker | Closed/Open/HalfOpen + recovery | `BackoffState` 已有失败升级、CircuitBreak 和 half-open 检查（`src/monitor/rate_budget.rs:94-230`） | `PARTIAL`；尚未证明每个真实 source 都统一接线或进入 banner；原设计 threshold=5/10 自相矛盾，须先冻结 |
 | PR-8 recovery fields | 每源 last successful pull/失败时长 | 当前 BannerCtx 没有统一 per-source map | `PROPOSED`；source identity 应复用 Gateway evidence catalog |
```

## docs/v19.x/v19.0-operational-clarity-design.md

- 原冻结 SHA-256：`da8f141e2c5aee942ea29539e80ff267dac339ca678c4fe7b764df133dc69284`。
- 后继写入 SHA-256：`10a7825962b92cfffcfcafb7d10771092458cb499b2eb82474d2a24b7fc2c594`。
- 恢复来源：`8792c319311638e6bdd22239e63a99690da379f9^`。

保留的后继改动：

```diff
diff --git a/docs/v19.x/v19.0-operational-clarity-design.md b/docs/v19.x/v19.0-operational-clarity-design.md
index 798579586..22cb08508 100644
--- a/docs/v19.x/v19.0-operational-clarity-design.md
+++ b/docs/v19.x/v19.0-operational-clarity-design.md
@@ -113,7 +113,7 @@ CircuitBreaker 必备熔断 + 半开探测 + 恢复路径。"系统能不能自

 | PR | 文件改动 | 反向测试 | 痛点 |
 | --- | --- | --- | --- |
-| **PR-7 Per-source circuit breaker** | 新 `src/breaker/mod.rs`：`struct Breaker { state: AtomicU8, consecutive_failures: AtomicU32, opened_at_ms: AtomicI64, cooldown_ms: u64 }`；`try_acquire() -> Result<(), ErrorCode>`；应用点到 `data_provider/fallback.rs:208`、每个 news aggregator feed | 测试"5 次失败后 Open"，测试"Open 后 cooldown 内 acquire 返回 CircuitOpen"，测试"cooldown 过后转 HalfOpen 允许一次尝试"，测试"HalfOpen 成功转 Closed 重置" | #4 数据源无熔断 |
+| **PR-7 Per-source circuit breaker** | 新 `src/breaker/mod.rs`：`struct Breaker { state: AtomicU8, consecutive_failures: AtomicU32, opened_at_ms: AtomicI64, cooldown_ms: u64 }`；`try_acquire() -> Result<(), ErrorCode>`；应用点到 `data_provider/fallback.rs:208`、每个 news aggregator feed | 测试"连续 10 次可重试采集失败后 Open"，测试"Open 后 cooldown 内 acquire 返回 CircuitOpen"，测试"cooldown 过后转 HalfOpen 只允许一次尝试"，测试"HalfOpen 成功转 Closed 重置" | #4 数据源无熔断 |
 | **PR-8 Recovery field in banner** | `BannerSnapshot.last_successful_pull: BTreeMap<DataSource, DateTime<Utc>>`；每个 `data_provider` 调用成功时更新；`--health` 显示"consensus 已失败 6h23m" | 测试"fetch 成功后 last_successful_pull 更新"，测试"fetch 失败后 last_successful_pull 不变"，测试"`--health` 显示失败时长" | #4 恢复 / #10 banner 不说"已死多久" |
 | **PR-9 多层通知链** | 三层通知：(1) 本地 heartbeat 文件 `data/health/heartbeat.json` 永远写；(2) 本地 HTTP `/health` 端口可选；(3) webhook 配了就走；health check 失败永远走 (1) | 测试"webhook 未配置时 heartbeat 文件仍写入"，测试"heartbeat 失败不影响 monitor 主循环"，测试"webhook 配错时不挂 monitor" | #9 健康失败也静默 |

@@ -122,7 +122,7 @@ CircuitBreaker 必备熔断 + 半开探测 + 恢复路径。"系统能不能自
 | PR | 文件改动 | 反向测试 | 痛点 |
 | --- | --- | --- | --- |
 | **PR-10 `--test` mode isolation** | `BannerSnapshot.test_mode: bool`；`--test` 启动时 banner 显示 `[TEST]` 前缀 + `test_mode=true`；PushKind 在测试模式强制 dry_run；推送日志写 `data/test/push_log/` 不写 `data/push_log/` | 测试"`--test` 启动 banner 有 [TEST]"，测试"`--test` 不写生产 push_log"，测试"`--test` 时 PushKind 进 dry_run 路径" | #5 测试生产边界 |
-| **PR-11 Failure-mode test coverage** | 每个 ErrorCode 一个反向测试：`BannerUnavailable` 触发 → banner 显示 unavailable；`CircuitOpen` 触发 → 下次 pull 跳过；`DataSourceFailed` 触发 → 第 5 次后 Open；`QuietModeActive` 触发 → 不 fetch；`Halted` 触发 → 任何 fetch 拒绝 | 测试覆盖每个 ErrorCode 至少 1 个反向测试 | #4 失败路径 0 测试 / #5 边界 |
+| **PR-11 Failure-mode test coverage** | 每个 ErrorCode 一个反向测试：`BannerUnavailable` 触发 → banner 显示 unavailable；`CircuitOpen` 触发 → 下次 pull 跳过；`DataSourceFailed` 触发 → 连续第 10 次可重试采集失败后 Open；`QuietModeActive` 触发 → 不 fetch；`Halted` 触发 → 任何 fetch 拒绝 | 测试覆盖每个 ErrorCode 至少 1 个反向测试 | #4 失败路径 0 测试 / #5 边界 |

 ## 4. 核心模块设计

@@ -432,4 +432,4 @@ v18 的 `AuditJournal` / `DataEnvelope` 不落地，**v19.x 不补这个**。v19
 | v18 active §6.3 WORM ≥ 5 年 | 边界约束（v19.x 不补这个） |
 | v18 active §16 Gate P 量化 | 推迟到 v20+ |

-本文不替代 v18 active；它是 v19.x 的独立设计。后续修改必须遵守 spec 证据规则（每条代码事实 grep 验证）。
\ No newline at end of file
+本文不替代 v18 active；它是 v19.x 的独立设计。后续修改必须遵守 spec 证据规则（每条代码事实 grep 验证）。
```
