# 盘后复盘自动调度设计

**日期：** 2026-07-21  
**规则：** AGENTS 2.1、2.2、2.4、2.7、2.8、2.10；BR-049、BR-108、BR-110、BR-116、BR-139

## 1. 问题与根因

`dispatch_post_session_review()` 已实现 R-02～R-08、A-10、A-01 的严格批量派发，BR-049 也声明常驻 monitor 应在 19:00 后调用它；但生产常驻入口只启动新闻回溯与纸面交易整盘扫。`run_review_only()` 仅由 `--review`、`--test --review` 和隔离夹具调用，因此不运行额外命令时，R 系列不会自动开始。

这是生产入口缺接线，不是“今日无复盘数据”。当前时间窗口、banner 和数据源失败还会形成后续拒绝，但它们不能解释常驻入口根本没有调度调用。

## 2. 选择

采用进程内独立调度器：常驻 monitor 启动一个 60 秒 ticker；真实交易日 19:00 后，若当日未完成且没有批次在途，则调用严格内部复盘批次。选择该方案是因为它复用现有数据质量、治理、sink 和审计链，不引入 launchd/cron 的第二套运行状态，也不扩大为本次无关的逐报告 capability 重构。

不采用：

- 外部定时执行 `--review`：环境、日志与进程生命周期分裂，且 CLI wrapper 会直接退出进程。
- 立即拆分全部 R 报告闸门：长期可降低共享失败面，但改动和验证范围明显超过本次“恢复自动可达性”。

## 3. 数据流与状态

```text
常驻 main
  -> post_session_review_scheduler (60s, MissedTickBehavior::Skip)
  -> due gate (交易日、>=19:00、当日未完成、无在途)
  -> evaluate_account_mode_hook(true)
  -> run_strict_review_only_inner() + 顶层 timeout
  -> dispatch_post_session_review()
  -> 每个 dispatcher 的治理、真实 sink、投递审计
  -> 至少一份确认投递：提交 completed_date
     全部失败：不提交，下一 tick 可重试
```

调度完成状态只存在于进程内。进程在 19:00 后重启时会立即重新评估；L4/业务去重负责阻止同一真实消息重复外发。调度器不创建账户快照、不修改持仓、不补齐缺失字段。

## 4. 失败模式

- 非交易日或 19:00 前：正常等待，不调用复盘。
- AccountMode/banner 未建立：记录 BR-108/BR-139 错误，保留当日重试资格。
- 严格批次超时：记录耗时和超时，释放在途状态，保留重试资格。
- 数据源或全部 dispatcher 失败：沿用 BR-110 逐项 audit，不提交完成日期。
- 至少一份确认投递：视为本日自动复盘已产生用户可见结果并停止重复调度；未成功子报告仍保留逐项失败审计，不伪装成功。
- 调度任务 panic：JoinHandle 失败显式记录；不得使常驻主循环退出。

## 5. 测试与验收

先用纯函数覆盖四个 RED/GREEN 边界：19:00 前、非交易日、已完成/在途、交易日 19:00 后。再用暂停 Tokio 时间的调度测试证明失败不提交、成功才提交；若异步依赖不易注入，至少以可返回 `Result` 的单次 runner 测试该状态转换。

最终必须通过：

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets --all-features -- --test-threads=1`
- `bash tools/compliance/check.sh`
- `cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json`
- `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json`
- `cargo build --release --bin monitor`
- 部署后检查调度启动日志，并以显式 `--review` canary 验证相同严格批次；不得展示账户标的或通知正文。

## 6. 旧模块关系与回滚

采用 `dispatch_post_session_review`、BR-108 banner 评估、BR-110 逐项结果与现有通知治理；拒绝调用遗留 `run_review_only_inner(false)`，因为其生产可达性已由既有设计禁止。

整体回退 BR-139 文档、调度器、注册调用和测试即可。回滚不删除数据库、日志、投递审计或持仓数据。
