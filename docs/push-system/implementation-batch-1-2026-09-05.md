# 推送可靠性首批修复实施计划

> 执行要求：使用subagent-driven-development或executing-plans逐任务实施、复核，勾选框追踪完成状态。完整结果与执行偏差见同目录首批开发结果文档，不把勾选状态当生产验收。

**目标：** 修复R-08永久错误被重试、告警测试污染G5b输入两条已经有真实样本的链路，交付可回归的隔离开发候选。

**架构：** 延用当前Rust单体、Gateway、ReviewScheduleState和alert_log边界。错误类型由Gateway向调度/审计传递；告警归档显式区分生产与测试，G5b在读取、模型调用及落盘前检查来源。保留现有物理投递owner，本批不是完整Foundation。

**技术栈：** Rust 2021 / Tokio / Serde，现有本地归档及测试框架，无新增服务。

构建执行注记：本批仅四组筛选测试，实际测试命令增加 `--profile dev` 复用首次构建依赖（不运行全量工具二进制）；测试过滤器、默认并行和验收语义不变。最终报告记录完整实际命令。.gitignore仅对本批plan/results解除忽略，不纳管其他ignored文档或运行文件。

最终验收补充两组既有邻接回归：`risk::env_guard::tests`与`monitor::alert::tests`，分别覆盖所复用的进程/环境隔离和未改变的告警格式；均通过`--offline --profile dev --lib`执行，不调用生产main。

## 已批准范围与基线

Spec：原工作区 `docs/push-system/grill-decisions-2026-09-02.md` 的Q9/Q17/Q22/Q49/Q51/Q101，以及 `comprehensive-reanalysis-2026-09-05.md` §9的R-08、TEST_CODE验收条目。用户已批准“解决”及从a673043隔离开发；测试沿用已确认的Gateway→调度与告警→G5b边界，不另行改变产品范围。

开发区：`.worktrees/push-reliability-20260905`；分支：`codex/push-reliability-20260905`；起点：`a673043acb9390605d2a43fc3ee2ad01488f633e`。

原工作区160项unmerged索引不动。R-07、PaperBuy、Watchdog和其他未提交修改不是本批修复的依赖，暂不移入；后续单元逐项核对。独立开发区补齐唯一编译输入 `client-bundle/market.proto`，SHA-256 `8730bce3c20e170cf8f58047336ae06d3a5e9080d81568dee71e7b0882063332`，保持原字节、ignored，不作为本批源码提交。

## Global Constraints

- 产品代码只修改独立worktree；原目录仅维护docs入口和planning记录。不修改原目录的产品代码/索引，不发送消息、不调用真实LLM/provider、不修改生产数据库、不部署。
- 不激活INACTIVE/STARVED/OPT-IN，不更换physical owner，不把bool、NoData、Disabled或Uncertain伪装成Accepted；所有Uncertain禁止盲目重发。
- 保留R-08 CFFEX必需证据约束、verified-empty有效语义、可选组件降级及终态preflight；不修改成功模板字节。
- 新测试默认并行、无进程级cwd/env切换，用独立临时目录或内存。旧审计必须可读取；禁止删除/改写历史污染记录。
- docs/push-system保存计划、状态、证据；开发候选不等于Foundation Ready、Production Verified。

## Task 1: R-08保留类型化错误到调度及审计

**Files:** 修改 `src/bin/monitor/push_templates.rs` 和 `src/bin/monitor/review_batch.rs`，回归测试放在各自既有tests模块。

边界：使用既有 `dispatch_r08_event_calendar_outcome_with_loader` 注入外部批次/终态preflight，再通过 `ReviewScheduleState::apply` / `is_due` 和返回的 `ReviewTaskTransition` 验证；不触发网络或sink。

- [x] 用2026-07-21业务日的现有批次fixture注入CFFEX永久失败，其诊断文字故意含http/request。断言失败不能在稍后再次due，且机器分类不是source_transport_failed。先运行失败测试。

```rust
let error = stock_analysis::data_gateway::GatewayError::unavailable(
    "futures_delivery", None, false, "http request rejected by permanent contract",
);
// 将error作为loader第二项Err，其他项使用既有announcement_batch / indices_batch / fx_batch。
// 对outcome调用ReviewScheduleState::apply；2026-07-21 23:00不能再次due。
```

- [x] 增加单一可序列化的Gateway失败快照，业务与审计复用同一类型，不通过String反推retryable/category。

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewGatewayFailure {
    pub capability: String,
    pub provider: Option<String>,
    pub audit_outcome: String,
    pub reason_code: String,
    pub retryable: bool,
    pub reason: String,
}
```

`ReviewTaskFailure` 与 `ReviewTransitionFailure` 新增 `GatewaySource(ReviewGatewayFailure)` variant。保留ExistingSourceFailure和AccountDependency原格式。`ReviewTaskOutcome::gateway_failed(&GatewayError)` 构造字段，provider用当前ProviderId既有文本表示；reason仅诊断。所有match穷尽新variant，retryable来自快照；transition的reason_code使用稳定 `gateway_<capability>_<reason_code>`（已有sanitize_reason_code分量处理），不含诊断文字hash，source保留可追踪能力/provider，success=false。旧路径分类不变。

- [x] 在R-08 loader返回后、任何GatewayError字符串化前处理必需CFFEX失败并返回typed outcome；非CFFEX失败仍沿用可选降级。完整记录dispatcher失败原因；不绕过preflight。

```rust
if let Err(error) = &futures_delivery_batch {
    let reason = format!("r08_cffex_component_unavailable: {error}");
    log::error!("[R-08][BR-140] {reason}");
    log_dispatcher_attempt("R-08", false, 0, &reason);
    return ReviewTaskOutcome::gateway_failed(error);
}
```

- [x] 运行永久失败测试变绿。再逐个增加：retryable=true继续按既有退避重试；typed failure审计JSON roundtrip保留每个字段；现有旧failure JSON仍可读取；不同诊断文字不改变reason_code；终态preflight不加载provider；verified-empty及可选降级保持原测例通过。
- [x] 命令：`cargo test --offline --bin monitor r08`、`cargo test --offline --bin monitor review_batch::tests`；保留真实退出码、通过数及任何已有失败，不能以编译失败当作行为RED。
- [x] 仅提交本任务的两个源文件到隔离分支：`git add src/bin/monitor/push_templates.rs src/bin/monitor/review_batch.rs`；`git commit -m "fix(push): preserve R08 gateway failure semantics"`。报告写到本计划SDD workspace指定路径。

## Task 2: 告警归档/G5b测试来源隔离

**Files:** 修改 `src/monitor/alert_log.rs`、`src/monitor/attribution_deep.rs`；若需要tempfile，仅在 `Cargo.toml` dev-dependencies增加已缓存版本并核对Cargo.lock最小变化。

边界：现有告警append/read接口、`top_events_for_deep`、`DeepAttributionAnalyzer::assess`、`append_deep_attribution_row`。生产默认目录仍为reports/alerts、data/g5b，不依赖全局env或cwd切换，不扩大为整个应用的存储框架。

- [x] 增加回归：现有sample_record改成code=TEST_CODE_000001后传入 `top_events_for_deep(vec![record], 3)` 必须为空；先执行证明当前行为失败。历史样本仅构造fixture，不读删原日志。

```rust
let mut record = sample_record();
record.code = "TEST_CODE_000001".into();
assert!(top_events_for_deep(vec![record], 3).is_empty());
```

- [x] 在AlertRecord增加带serde默认的来源枚举，所有本地构造处显式适配，旧JSON缺字段可读且标LegacyUnknown。

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertRecordOrigin {
    #[default]
    LegacyUnknown,
    Production,
    Test,
}
// AlertRecord:
// #[serde(default)]
// pub origin: AlertRecordOrigin,
```

提供统一 `AlertRecord::is_production_eligible()`：Test一律false，Production/LegacyUnknown中的code以 `TEST_CODE` 开头也为false（复用 `risk::env_guard::is_test_code` 隔离已证实的历史污染，不靠股票名称/新闻正文模糊匹配）。未知origin反序列化失败不可默认为Production。不改变正常股票事件优先级或数量。

执行前补充：上述生产默认I/O保护不仅检查cfg(test)，还必须复用 `risk::env_guard::runtime_is_test_process()` 以及 `current_env() == TradingEnv::Test`，前者可识别integration-test进程（library未带cfg(test)）。默认生产读取也必须被此保护拦截，不能让Test进程读取原归档再调用G5b。显式临时归档实例不受生产默认I/O保护限制；不可通过全局env/cwd切换来测试。这是测试污染修复的必要覆盖，不是开启新的运行模式。

- [x] 在alert_log建立显式归档对象 `AlertLog`，持有目录与origin；生产构造固定reports/alerts，测试构造显式临时目录且标Test。提供append_jsonl、append_md、append_batch、read_today、today_stats、read_today_records，与既有自由函数兼容适配；自由函数默认生产接口在cfg(test)拒绝持久化，测试必须使用显式test实例。生产写入拒绝TEST_CODE，生产结构化读取跳过不可生产记录并记录warning；测试归档可往返Test记录。测试构造不能默认回退生产目录，不能用set_current_dir/全局环境变量。

- [x] 将原来三个写reports/alerts的测试移到每测试独立临时目录，断言实际写入/读取一致以及两个实例互不污染；模拟写入失败仍返回Err。不要仅把断言改成不为空或调用私有函数规避行为测试。
- [x] G5b三道防线使用同一eligibility判断：top_events筛选时排除并出声；assess在调用LLM前返回新 `IneligibleRecord` 错误；append_deep_attribution_row在创建目录/写文件前拒绝。使用边界fake LlmProvider验证污染记录直接拒绝，正常记录的原receipt测试继续通过。默认生产落盘函数在cfg(test)不得触碰生产路径。
- [x] 增加normal-code但origin=Test的案例；旧JSON缺origin的正常股票仍准入，旧TEST_CODE仍拒绝；max=0和原Emergency/Important排序保持。生产读取防线用临时目录保存混合legacy/test/production记录，测试生产读取策略，不在真实目录注入fixture。
- [x] 命令：`cargo test --offline --lib monitor::alert_log::tests`、`cargo test --offline --lib monitor::attribution_deep::tests`；一轮一例RED→GREEN，最后组合回归。运行前后确认原目录归档未被测试写入。
- [x] 仅提交本任务源文件及必要dev依赖：`git commit -m "fix(push): isolate alert fixtures from G5b production inputs"`。报告包含测试目录策略和明确未覆盖的全应用Test namespace边界。

## 整批验收与交付

两任务分别实施与独立复核后，运行fresh monitor构建和上述四组测试；再检查类型/JSON相关邻接测试及 `git diff --check a673043..HEAD`。全项目默认并行测试只有在基线可编译、测试副作用已核查后才能作为发布门禁；不得以局部测试冒充全项目通过。

中文交付记录写入 `docs/push-system/implementation-batch-1-results-2026-09-05.md`，列出实际commit、RED/GREEN命令、结果、原工作区移入清单、未完成Foundation/NewsAI/Paper/Watchdog/Completion/outbox门禁。根目录docs入口保留跳转，避免文档只存在深层开发区。

本批代码不自动merge master、不push、不部署。Full Foundation、正式RFC/目录/manifest、全量108决策实现和交易观察另有后续任务，不借本批验收改成已完成。
