# 异步监控中的阻塞 HTTP 生命周期修复设计

日期：2026-07-23

状态：Gate A 已批准（用户于 2026-07-23 指示“继续”）

规则：数据红线 2.1、2.2、2.4、2.7、2.8；BR-116、BR-128、BR-148

## 1. 问题

生产日志在 T0Advice 去重后出现：

```text
Cannot drop a runtime in a context where blocking is not allowed.
This happens when a runtime is dropped from within an asynchronous context.
```

panic 来源是 `reqwest::blocking::Client` 内部 Tokio runtime 在外层 Tokio async context 中销毁。T0Advice 是崩溃前最后一条业务日志，不是直接根因。

## 2. 代码证据

复现审计命令：

```bash
rg -n -C 4 \
  "fetch_(eastmoney_quotes|sina_quotes|market_main_inflow_top|market_volume_ratio_leaders)\\(" \
  src/bin/monitor/main.rs src/bin/monitor/push_templates.rs
```

已确认的危险路径包括：

- `main.rs` 的异步 market loop 直接调用 `fetch_eastmoney_quotes` 为虚拟观察仓补报价。
- `push_templates.rs` 的异步 I-03 dispatcher 直接调用同步 `load_industry_chain_snapshot_real`，其内部创建 blocking HTTP client。
- `push_templates.rs` 的异步候选 dispatcher 直接调用同步 `load_real_candidate_batch`，其内部创建 blocking HTTP client。

已有安全路径（例如 T0 持仓扫描和 I-10 资金榜）已经使用 `tokio::task::spawn_blocking`，证明仓库已有正确模式。

现有 `--test` 入口不能作为本缺陷反馈环：它先被既有 JSONL 审计链不一致阻断，尚未进入目标路径。因此本修复必须新增正确的最小测试 seam。

## 3. 设计

新增一个 monitor 内部异步边界函数，接收 `Send + 'static` 同步闭包并通过 `tokio::task::spawn_blocking` 执行。它必须：

- 区分业务 `Result` 与 `JoinError`；
- 保留原始业务错误；
- 将 panic/cancel 等后台任务失败转换成带调用标签的显式错误；
- 不吞错、不返回空集合或默认行情。

所有从 async context 调用、且传递路径中可能创建 `reqwest::blocking::Client` 的行情函数改走此边界。纯同步调用点保持不变。

```text
async scheduler/dispatcher
  -> run_blocking_market_data(label, closure)
       -> Tokio blocking pool
            -> existing synchronous provider
            -> strict source/quality result
       -> explicit business error or JoinError
```

## 4. 测试反馈环

新增确定性测试：闭包内部断言 `tokio::runtime::Handle::try_current()` 不处于外层 async runtime，并返回带标记的值。旧式直接调用会使断言失败；通过边界执行则通过。

同时测试：

- 业务错误原样返回；
- blocking task panic 不导致测试 runtime 崩溃，而转换为显式后台任务错误；
- 已知危险调用点不再直接调用同步 HTTP 函数。

目标反馈命令：

```bash
cargo test --bin monitor blocking_market_data --offline -- --nocapture
```

## 5. 失败边界与行为保持

- 不改变行情源顺序、数据字段、新鲜度阈值或质量校验。
- 不改变 T0Advice 内容、去重、冷却或投递确认语义。
- blocking 任务失败时，本轮相应消费者显式失败并保留重试资格。
- 不使用本机时间、默认价格、空批次或旧行情掩盖失败。

## 6. 旧模块处置

| 模块 | 处置 | 原因 |
| --- | --- | --- |
| `market_data` 同步函数 | adopt unchanged | 保留现有真实源与数据质量合同 |
| T0 `spawn_blocking` 路径 | adopt | 已符合目标边界 |
| I-10 `spawn_blocking` 路径 | adopt | 已符合目标边界 |
| async market loop 直接同步调用 | replace | runtime drop 根因 |
| async I-03/P-03 直接同步调用 | replace | 同类潜在崩溃路径 |

## 7. 验证与回滚

```bash
cargo fmt --all -- --check
cargo test --bin monitor blocking_market_data --offline -- --nocapture
cargo check --bin monitor --offline
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

运行验证需在隔离审计目录或已修复审计链环境中执行，并确认不再出现该 panic。回滚使用 `git revert <implementation-commit>`；不得删除审计、行情、账户或持仓数据。
