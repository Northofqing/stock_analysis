# Benchmark Request-Bound Attestation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让受信任且锁定 revision 的 Magic TDX HS300 Daily 数据通过精确请求/响应绑定进入 Benchmark segment，并使策略归因不再被永久身份布尔阻断。

**Architecture:** 在现有 `data_gateway::benchmark` 深模块内部引入唯一的 TDX HS300 协议合同能力值。生产 attestation 只有在 canonical instrument 与合同一致时签发能力，分页请求、请求 hash、批次 hash 和来源 revision 均由该能力生成；Minute1 的独立时间语义门保持关闭。

**Tech Stack:** Rust 1.95、Tokio、Magic TDX、SHA-256、Diesel/SQLite、Clap CLI。

**Spec:** `docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md §15.14`

## Global Constraints

- 生产路径只使用真实 Magic TDX 数据，不得使用 mock、空集合或旧日数据 fallback（2.1/2.2）。
- Daily 必须验证正有限 OHLC、严格时间顺序和权威交易日精确覆盖（2.3/2.4）。
- 正式采集必须先形成 BR-159 acquisition receipt，segment/manifest 只追加（2.7/2.8）。
- Raw probe 继续为不可准入的 `Unverified` 诊断；Minute1 继续返回 `benchmark_time_semantics_unavailable`。
- 不修改 config、threshold、订单、推送、monitor scheduler 或旧市场数据表；BR-251 语义不变。
- 既有失败审计和新增市场事实不可删除；回滚只停止 writer 并 `git revert` 代码/文档提交。

---

### Task 1: 冻结 Gate A 修订与 RED 行为

**Files:**
- Modify: `docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md`
- Create: `docs/superpowers/plans/2026-08-27-benchmark-request-bound-attestation.md`
- Test: `src/data_gateway/benchmark.rs`

**Interfaces:**
- Consumes: `BenchmarkRequest`、`BenchmarkAdmissionCoverage::Daily`、测试 `IndexBarsSource`。
- Produces: 行为测试 `production_daily_attestation_binds_exact_hs300_protocol_request`，供 Task 2 的最小实现满足。

- [x] **Step 1: 写入 §15.14 的真实 probe/capture 证据、数据流、失败模式、旧模块关系和回滚。**

- [x] **Step 2: 增加生产 Daily 请求绑定行为测试。**

```rust
#[test]
fn production_daily_attestation_binds_exact_hs300_protocol_request() {
    let day = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
    let source = TestIndexBarsSource::new(vec![Ok(vec![raw_daily(day)])]);
    let prepared = acquire_benchmark_batch_from_source(
        &source,
        daily_request(day, day, HS300_CANONICAL),
        &BenchmarkRegistry::production_default(),
        BenchmarkProviderAttestation::production_default(),
        BenchmarkAdmissionCoverage::Daily {
            authoritative_trading_days: &[day],
        },
        "2099-01-02T10:00:00+08:00",
    )
    .expect("request-bound production Daily identity must admit exact HS300 data");

    assert_eq!(prepared.batch.records().len(), 1);
    assert_eq!(
        prepared.batch.evidence().source,
        format!("magic-tdx-index-bars@{TDX_DEPENDENCY_REVISION}")
    );
    let requests = source.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].market, 1);
    assert_eq!(requests[0].code, "000300");
    assert_eq!(requests[0].category, 4);
    assert_eq!(requests[0].fq_type, 0);
}
```

- [x] **Step 3: 运行 RED 并保存失败原因。**

Run: `cargo test --lib data_gateway::benchmark::tests::production_daily_attestation_binds_exact_hs300_protocol_request --all-features -- --exact --nocapture --test-threads=1`

Expected: FAIL，测试在 source access 前得到 `benchmark_identity_unverified`。

Actual: exit 101；1 failed / 3002 filtered。失败值为
`GatewayError { capability: "BenchmarkBars", provider: Some(Tdx), audit_outcome: "unavailable", reason_code: "benchmark_identity_unverified", retryable: false }`，定位于生产 attestation，符合预期 RED。

- [x] **Step 4: 提交 Gate A + RED。**

```bash
git add docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md \
  docs/superpowers/plans/2026-08-27-benchmark-request-bound-attestation.md \
  src/data_gateway/benchmark.rs
git commit -m "test: freeze benchmark request-bound attestation"
```

Actual: commit `d2dc4dc`。

### Task 2: 实现类型化 TDX HS300 请求绑定

**Files:**
- Modify: `src/data_gateway/benchmark.rs`
- Modify: `src/data_gateway/review.rs`
- Test: 上述两个模块内单元测试

**Interfaces:**
- Consumes: `BenchmarkProviderAttestation::admit(&BenchmarkRequest)` 和锁定的 Magic TDX revision。
- Produces: 私有 `TdxIndexProtocolContract` 能力；`admit` 返回该能力并供 page request/hash/evidence 使用。

- [x] **Step 1: 用类型化 identity mode 替换生产身份布尔。**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BenchmarkIdentityAttestation {
    RequestBoundTdxHs300V1,
    Unverified,
    #[cfg(test)]
    TestOnlyTdxHs300V1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TdxIndexProtocolContract {
    canonical_instrument: &'static str,
    market: u8,
    code: &'static str,
    daily_category: u8,
    minute1_category: u8,
    fq_type: u8,
    dependency_revision: &'static str,
}
```

`production_default()` 固定为 `RequestBoundTdxHs300V1`；`admit` 对生产请求验证
`request.instrument == contract.canonical_instrument`，Minute1 再独立检查时间标签语义，并返回
`TdxIndexProtocolContract`。测试专用 variant 只允许测试 registry 路径，不进入生产构建。

- [x] **Step 2: 让同一合同生成所有 provider 身份字段。**

把 `canonical_base_request_hash`、`fetch_raw_benchmark_pages`、
`canonical_acquisition_bytes`、raw identity anchor 和 `BatchEvidence.source` 中的
market/code/category/fq/revision 改为从 `TdxIndexProtocolContract` 读取；保留既有 hash domain，
避免使已保存 V1 manifest 在无迁移情况下失效。

- [x] **Step 3: 保持 Minute1 和 unsupported identity fail-closed。**

更新既有 `attestation_fails_before_source_access_and_minute_semantics_are_independent`：Daily 改由
Task 1 正向测试覆盖；Minute1 仍断言 `benchmark_time_semantics_unavailable` 且 source offsets 为空。
生产 registry 的非 `sh000300` 请求继续在 provider access 前返回
`benchmark_instrument_unsupported`。`raw_diagnostic_accesses_source_without_minting_admitted_evidence`
只保留 raw DTO/零审计断言，不再把正式 Daily 永久不可达当成诊断隔离条件；
`audit_persists_benchmark_outcome_independently_from_retryability` 使用显式测试专用 `Unverified`
attestation 继续覆盖 `benchmark_identity_unverified` 错误分类。

- [x] **Step 4: 让 ReviewDataGateway 入口测试不依赖真实网络。**

把 `benchmark_entrypoint_delegates_to_library_and_appends_exactly_one_audit_row` 的请求改为
未注册 `sh000905`，断言 `benchmark_instrument_unsupported`、精确 request hash，以及
`BenchmarkBars` 审计行只增加一条；这只验证委派/审计，不把网络可用性写入单元测试。

- [x] **Step 5: 运行 GREEN 和定向回归。**

```bash
cargo test --lib data_gateway::benchmark --all-features -- --test-threads=1
cargo test --lib data_gateway::review::tests::benchmark_entrypoint_delegates_to_library_and_appends_exactly_one_audit_row --all-features -- --exact --nocapture --test-threads=1
cargo test --lib data_gateway::benchmark --no-default-features -- --test-threads=1
cargo test --bin strategy_attribution --all-features -- --test-threads=1
```

Expected: 全部 exit 0；Daily 正向测试访问一次 TEST source，Minute1/unsupported 均零 source access。

Actual: all-features Benchmark 26/26、no-default Benchmark 26/26、Review 单审计入口 1/1、CLI 6/6
均 exit 0；`cargo clippy --lib --all-features -- -D warnings` exit 0。no-default 的本次
`contract` warning 已清零，剩余 62 个 warning 均来自未修改路径。

- [x] **Step 6: 提交最小实现。**

```bash
git add src/data_gateway/benchmark.rs src/data_gateway/review.rs
git commit -m "fix: bind benchmark identity to trusted TDX request"
```

- [x] **Step 7: 关闭独立 review 发现的 revision 双写缺口。**

`build.rs` 从 `Cargo.lock` 中 `magic-tdx-rs` 的 resolved Git source 生成编译期 revision；
Benchmark 合同不再持有手写提交。回归测试同时验证 adapter 使用生成值，且该值等于 lockfile
唯一 source 的 40 位 commit。lockfile 缺失、歧义、仓库不匹配或 commit 非法均在构建时失败。

Actual: RED 精确失败于 adapter 仍为手写常量；实现后 GREEN 1/1。Benchmark 默认/无默认特性
均 27/27、Review 审计入口 1/1、CLI 6/6 通过。独立 review 同时发现并关闭测试真实代码与
不可达 identity mismatch reason，设计证据从“同一连接”校正为“同一次 typed client acquisition”。

### Task 3: Gate 验证、真实采集与归因闭环

**Files:**
- Modify: `docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md`（追加实际证据）
- Modify: Draft PR 描述（不改生产代码）

**Interfaces:**
- Consumes: `strategy_attribution capture` 返回的 exact `manifest哈希`。
- Produces: accepted BR-159 receipt、不可变 Daily segment/manifest、Reader/归因 CLI 的真实结果或下一个准确 typed blocker。

- [x] **Step 1: 执行 Gate B/C。**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh --policy pr
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
cargo llvm-cov report --lcov --output-path target/coverage/lcov.info
python3 tools/coverage/check_thresholds.py --policy pr --report target/coverage/coverage.json \
  --lcov target/coverage/lcov.info --base-ref master
```

Actual: fmt、strict workspace Clippy、隔离全量 tests、PR compliance 和 coverage ratchet 均
exit 0；core patch 94.34%，other production patch N/A（无可执行变更行），global/core
202960/260224 = 77.99% 与 158758/203874 = 77.87%，均高于最新 `master` 的覆盖率比例。
默认与 no-default Benchmark 聚焦测试均 27/27，CLI 6/6。Gate C PASS；Gate D 因固定
80%/95%、freshness、完整 live attribution 与 auditor 未闭合保持 Release Blocked。

- [x] **Step 2: 执行真实 Daily capture。**

```bash
target/release/strategy_attribution capture --db data/stock_analysis.db \
  --instrument sh000300 --granularity daily --from 2026-08-27 --to 2026-08-27 \
  --commit --format json
```

Expected: exit 0，输出一个 manifest hash；`data_acquisition_audit` 新增 accepted
`BenchmarkBars/Tdx` 行，`benchmark_segment_revision` 与 `benchmark_manifest` 各新增或幂等返回一项。

Actual: sandbox 内首次 transport failure 保留 audit `978013`；获准真实网络后 exit 0。
accepted audit `1002704`（1/1/0），source revision `75ee2a2...e7000e`，segment
`f0c8c460...571643`，manifest `80036750...52f3c`，一条 2026-08-27 Daily 记录；关联、segment
chain、manifest chain 和 `PRAGMA quick_check` 均通过。

- [x] **Step 3: 使用 exact manifest 运行归因 CLI。**

从 Step 2 JSON 读取 manifest hash，按 `strategy_attribution scheduled --help` 的现行参数格式传入
同一业务日。Expected: 不再出现 `benchmark_identity_unverified`；成功则核对报告 manifest，其他
输入失败则保存其真实 typed reason，禁止补值或伪称全链成功。

Actual: benchmark 阶段不再出现 identity failure；只读 replay 在 `trade_evidence` 以
`paper_trade_source_failed` 退出。精确阻塞为 `paper_trades.id=520` 卖出 002594 时消耗同日
`id=490` 买入的 100 股，违反 T+1；全量盘点共 9 个同类卖单。历史来源和对应 order audit
一一匹配，禁止改写或跳过；没有纸面空仓确认且仍有 325 个代码/49,500 股净持仓，因此未执行
归因 `--commit`，等待用户提供新纪元空仓确认或权威纸面持仓基线。

- [ ] **Step 4: 追加真实证据并完成独立 review。**

记录命令、退出码、BR-159 audit ID/hash、segment/manifest hash、归因结果以及 Gate B/C/D 的准确
状态。reviewer 必须独立复跑定向测试、diff、生产事实查询和规则检查；Critical/Important 为零才可
接受。

- [ ] **Step 5: 提交证据并更新 Draft PR。**

```bash
git add docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md
git commit -m "docs: record benchmark attestation live evidence"
```

PR 字段固定包含：`Refs: spec §15.14`、`Data-Redlines: [2.1,2.2,2.3,2.4,2.7,2.8]`、
OldModules 表、`Threshold-Proof: N/A`、`Business-Rules: BR-251`、Gate C/Gate D 分列证据和
`Rollback:` 中列出本任务实际提交 SHA，并逐项使用 `git revert`；不得删除任何审计或市场事实。
