# 盘后复盘逐任务依赖门设计（BR-194）

**状态**：独立 RED 后的 Gate A 纠正文档，等待重新独立审查；不得据此声明
Gate A Green、实现完成或可发布。
**日期**：2026-07-30
**业务规则**：BR-049、BR-139、BR-194。
**范围**：只修复 `--review` 与常驻盘后复盘被 caller-wide 账户
snapshot/banner 前置门整体阻断的问题。首批只放行已经证明不消费账户事实的
R-04（21:00 龙虎榜）和 R-09（15:35 provider TopN）。

## 1. 目标与非目标

目标：

1. 把复盘依赖从“整个批次必须有 banner”改成每个 `ReviewTask` 的闭集依赖声明。
2. 静态禁用和时间窗判断先于依赖获取；不该运行的任务不得访问账户或 provider。
3. R-04、R-09 不得因账户 snapshot、AccountMode、账户状态变更通知或 banner
   不可用而失去运行资格。
4. 本首版在仓库尚无“真实账户快照 + 同批 broker trade-sync watermark”producer
   时，所有保守账户门任务固定返回 structured typed、可重试的
   `AccountMetricsIncomplete`；禁止实现不可达的成功分支，更禁止用默认 banner、
   空账户、持仓推算或 stale 数据放行。
5. 为 R-04 增加闭合、kind allowlist 的 counted SourceOnly 治理入口：它只把
   `AccountMode/is_frozen` 从 combined banner 中移除，仍读取真实 DataMode capability、
   执行静默期和日限额治理，并继续进入唯一 durable budget/dedup/fence/sink/audit
   owner；generic counted 入口和其他 PushKind 不获得该豁免。
6. 维持 BR-140 的任务级调度，并在本设计内独立验证当前
   `durable_delivery_runtime` 投递/恢复代码；不把尚未通过 Gate C 的其他规格当作
   权威前置条件。
7. `--test --review` 在 preflight 同时禁用 R-04 和 R-09 的真实外部 provider；
   两任务的 provider 与 sink 调用数都必须为 0。
8. Gate D 不再从最终 hydration 状态推断“发生过第二次 scheduler attempt”。发布验证
   必须显式执行一次受控 terminal replay，由 durable owner 保存不可变 typed attempt
   evidence，并用同一 decision 的前后 sink-result / delivery-audit-ref 水位证明零重发。

非目标：

- 不放宽 BR-108、BR-136 的账户真实性和测试隔离要求。
- R-03/R-08 当前分别读取 portfolio projection 与本地 user-confirmed position
  snapshot；这些是真实代码依赖，但都不是带同批 broker trade-sync watermark 的
  verified account evidence，因此仍标为 `LegacyAccountGate`。A-10/A-01 在独立
  数据流审计前标为 `UnclassifiedConservative`；分类不得伪称本地持仓投影已经满足
  真实账户门。
- 不修改 R-04/R-09 provider 合同、排序、Top N、时间阈值、PushKind、模板 ID、
  冷却、预算、sink 或 `push.delivery.audit` 格式；BR-140 review transition 为保存
  typed failure 只追加向后兼容的可空字段。
- 不把受控 terminal replay 称为第二次常驻 scheduler attempt；它是独立的发布验证
  mode，只消费已持久化且已 hydration 的 terminal envelope，禁止重新调用 provider
  或真实 sink。
- 不用 R-04/R-09 代替其他缺失复盘任务。
- 本 Gate A 不修改 Rust、配置、数据库或生产数据。

### 1.1 触发的数据红线

- **2.1 / 2.2**：账户能力缺失显式失败；不以本地投影、默认值或测试 seed 补齐。
- **2.3**：R-04 继续校验日期/来源 provenance 一致、ranking net 与 seat gross
  amount 有限且为正、可空 buy/sell/net/turnover 数值若存在则有限、披露结构、
  买卖双边各五席及唯一 rank；R-09 继续校验有限值、非空双批次、provider/source、
  metric/unit/date/source order 和批次结构。拆门不得绕过这些实际适用的坏数据拒绝。
  两者不是价格序列，故 `price > 0`、相邻涨跌、时间序列 gaps/duplicates 与
  split/dividend consistency 子项在本数据类型上 **N/A**；不得伪称代码已执行这些
  不适用的 price-series 校验。固定账户失败不进入行情/账户计算。
- **2.4**：R-04/R-09 保持各自真实 provider 新鲜度合同；本地 acquisition time
  不得冒充账户 provider source time。
- **2.5**：`--test --review` 使用测试 namespace，且 R-04/R-09 的真实 provider
  和任何 sink 均为 0 调用；生产 source-only 任务只接受真实证券身份。
- **2.7**：逐任务 transition 原样保存 typed failure、来源时间和脱敏证据身份；
  terminal replay 的 start/completion、稳定 attempt identity、前后 authority 水位和
  零 provider/sink 计数进入同一五年不可变 durable audit。
- **2.8**：新增静态 checker 与生产 join verifier 必须真实检查目标 authority；
  logging-only、固定 PASS 或跳过坏记录均为阻塞失败。
- **2.10**：任务分区、过滤、稳定排序、重复拒绝均由 BR-194 登记。

**不适用红线**：

- **2.6 N/A**：本变更不创建订单，不改变数量、价格、资金校验或二次确认。
- **2.9 N/A**：本变更不修改 `config/*.toml`、阈值、clamp 或规格—配置映射。

## 2. 现状证据与根因

执行的 code-fact 命令：

```bash
nl -ba src/bin/monitor/main.rs | sed -n '3997,4100p'
nl -ba src/bin/monitor/review_batch.rs | sed -n '290,347p'
nl -ba src/bin/monitor/review_batch.rs | sed -n '863,934p'
nl -ba src/bin/monitor/push_templates.rs | sed -n '6902,7022p'
nl -ba src/bin/monitor/push_templates.rs | sed -n '6535,6544p'
nl -ba src/bin/monitor/push_templates.rs | sed -n '9115,9159p'
```

与本缺陷直接相关的可重放原始输出（2026-07-30，在仓库根执行）：

```text
$ rg -n 'evaluate_account_mode_hook\(true\)|let banner = current_banner\(\)\?|dispatch_post_session_review\(context' src/bin/monitor/main.rs
1574:    let banner = current_banner()?;
3875:        if !evaluate_account_mode_hook(true).await {
4002:    if !evaluate_account_mode_hook(true).await {
4075:    let banner = current_banner()?;
4076:    Ok(push_templates::dispatch_post_session_review(context, &banner, due).await)
4088:    if !evaluate_account_mode_hook(true).await {
8576:        assert!(runner.contains("dispatch_post_session_review(context, &banner, due)"));

$ rg -n 'pub struct AccountSnapshotInput|broker trade-sync watermark|complete account metrics unavailable' src/database/account_snapshot.rs src/bin/monitor/main.rs
src/database/account_snapshot.rs:51:pub struct AccountSnapshotInput {
src/bin/monitor/main.rs:1976:        "BR-103 complete account metrics unavailable: real broker trade-sync watermark is not connected"

$ rg -n 'let lhb_ready|if is_test|runnable.remove\(&ReviewTask::R09\)' src/bin/monitor/review_batch.rs
142:        Some(base) => base.join(if is_test { "test" } else { "prod" }),
143:        None if is_test => std::path::PathBuf::from("data/test/review_audit"),
894:    let lhb_ready = chrono::NaiveTime::from_hms_opt(21, 0, 0)
907:        let outcome = if is_test {
928:            runnable.remove(&ReviewTask::R09);

$ rg -n 'pub async fn dispatch_post_session_review|dispatch_r04_lhb_outcome\(|provider_top_n_pair\(' src/bin/monitor/push_templates.rs
6540:            .provider_top_n_pair(date)
6898:        assert!(!dispatcher.contains("provider_top_n_pair("));
6906:pub async fn dispatch_post_session_review(
6940:                    dispatch_r04_lhb_outcome(&date, now, banner).await,
9116:pub async fn dispatch_r04_lhb_outcome(
9252:        dispatch_r04_lhb_outcome(date, chrono::Local::now().time(), banner).await,
9289:            .find("pub async fn dispatch_r04_lhb_outcome(")

$ nl -ba src/bin/monitor/review_batch.rs | sed -n '894,901p;903,911p;919,923p;927,931p'
   894	    let lhb_ready = chrono::NaiveTime::from_hms_opt(21, 0, 0)
   895	        .expect("BR-140 LHB publication time must be valid");
   896	    if context.eligibility_time() < lhb_ready && runnable.remove(&ReviewTask::R04) {
   897	        outcomes.push((
   898	            ReviewTask::R04,
   899	            ReviewTaskOutcome::expected_wait(lhb_ready, "LHB source not published before 21:00"),
   900	        ));
   901	    }
   903	    if runnable.contains(&ReviewTask::R09) {
   904	        let current_date = context.observed_at().date();
   905	        let provider_ready = chrono::NaiveTime::from_hms_opt(15, 35, 0)
   906	            .expect("BR-192 provider publication time must be valid");
   907	        let outcome = if is_test {
   908	            Some(ReviewTaskOutcome::disabled(
   909	                "test_environment_external_provider_blocked",
   910	                "test_environment_external_provider_blocked; provider_calls=0",
   911	            ))
   919	        } else if context.eligibility_time() < provider_ready {
   920	            Some(ReviewTaskOutcome::expected_wait(
   921	                provider_ready,
   922	                "Eastmoney provider Top-N is not eligible before 15:35",
   923	            ))
   927	        if let Some(outcome) = outcome {
   928	            runnable.remove(&ReviewTask::R09);
   929	            outcomes.push((ReviewTask::R09, outcome));
   930	        }
   931	    }

$ nl -ba src/bin/monitor/push_templates.rs | sed -n '6906,6909p;6923,6925p;6936,6941p;6956,6961p'
  6906	pub async fn dispatch_post_session_review(
  6907	    context: crate::review_batch::ReviewRunContext,
  6908	    banner: &BannerCtx,
  6909	    due: &std::collections::BTreeSet<crate::review_batch::ReviewTask>,
  6923	    let preflight = review_preflight(context, due, is_test);
  6924	    let runnable = preflight.runnable;
  6925	    let (r03, r04, r08, r09, a10, a01) = tokio::join!(
  6936	        async {
  6937	            if runnable.contains(&ReviewTask::R04) {
  6938	                Some((
  6939	                    ReviewTask::R04,
  6940	                    dispatch_r04_lhb_outcome(&date, now, banner).await,
  6941	                ))
  6956	        async {
  6957	            if runnable.contains(&ReviewTask::R09) {
  6958	                Some((
  6959	                    ReviewTask::R09,
  6960	                    dispatch_r09_provider_top_n_outcome(review_date).await,
  6961	                ))

$ nl -ba src/bin/monitor/push_templates.rs | sed -n '9116,9119p;9123,9124p;9128p;9133,9136p'
  9116	pub async fn dispatch_r04_lhb_outcome(
  9117	    date: &str,
  9118	    now_time: chrono::NaiveTime,
  9119	    banner: &BannerCtx,
  9123	    let gateway = DragonTigerGateway::new();
  9124	    dispatch_r04_lhb_outcome_with_loader(date, now_time, banner, move |trading_date| async move {
  9128	        gateway.market_review(trading_date, 5, 5).await
  9133	async fn dispatch_r04_lhb_outcome_with_loader<F, Fut>(
  9134	    date: &str,
  9135	    now_time: chrono::NaiveTime,
  9136	    _banner: &BannerCtx,
```

R-09 的 producer→prepare→binding→durable 以及“无账户读取”证据：

```text
$ nl -ba src/bin/monitor/push_templates.rs | sed -n '6247,6544p' | rg 'prepare_r09_provider_top_n_report|TaskBinding::new|DeliveryEnvelope::new|with_provider_evidence|build_r09_delivery_envelope|deliver_envelope\(|dispatch_r09_provider_top_n_outcome|provider_top_n_pair'
  6247 fn prepare_r09_provider_top_n_report(
  6352 fn build_r09_delivery_envelope(
  6370     let task_binding = TaskBinding::new(
  6386     let envelope = DeliveryEnvelope::new(
  6400         envelope.with_provider_evidence(
  6463 async fn dispatch_r09_provider_top_n_outcome_with_loader<Loader, Future>(
  6490     let prepared = match prepare_r09_provider_top_n_report(review_date, pair) {
  6517     let envelope = match build_r09_delivery_envelope(&prepared) {
  6526     match crate::durable_delivery_runtime::deliver_envelope(envelope).await {
  6535 async fn dispatch_r09_provider_top_n_outcome(
  6538     dispatch_r09_provider_top_n_outcome_with_loader(review_date, |date| async move {
  6540             .provider_top_n_pair(date)

$ if nl -ba src/bin/monitor/push_templates.rs | sed -n '6247,6544p' | rg 'BannerCtx|current_banner|evaluate_account_mode_hook|account_snapshot|stock_position'; then exit 1; else echo 'R09_ACCOUNT_READS=0'; fi
R09_ACCOUNT_READS=0
```

R-04 的 producer→prepare→binding→durable→authoritative sink 证据：

```text
$ nl -ba src/bin/monitor/push_templates.rs | sed -n '8935,9248p' | rg 'prepare_review_lhb_delivery|DragonTigerGateway|market_review\(|TaskBinding::new|CountedDeliveryBinding::new|CountedDeliveryOrigin::Provider|ordered_batch_ids|push_counted_with_binding|dispatch_r04_lhb_outcome'
  8935 fn prepare_review_lhb_delivery(
  9116 pub async fn dispatch_r04_lhb_outcome(
  9121     use stock_analysis::data_gateway::DragonTigerGateway;
  9123     let gateway = DragonTigerGateway::new();
  9124     dispatch_r04_lhb_outcome_with_loader(date, now_time, banner, move |trading_date| async move {
  9128         gateway.market_review(trading_date, 5, 5).await
  9133 async fn dispatch_r04_lhb_outcome_with_loader<F, Fut>(
  9198     let prepared = match prepare_review_lhb_delivery(today, &entries, &evidence) {
  9206     let task_binding = match stock_analysis::durable_delivery::TaskBinding::new(
  9218     let counted_binding = match crate::durable_delivery_runtime::CountedDeliveryBinding::new(
  9224         crate::durable_delivery_runtime::CountedDeliveryOrigin::Provider {
  9227             ordered_batch_ids: vec![prepared.batch_id],
  9239     let push_result = crate::notify::push_counted_with_binding(

$ nl -ba src/bin/monitor/notify.rs | sed -n '2344,2385p' | rg 'push_counted_with_binding|deliver_counted_binding'
  2344 pub async fn push_counted_with_binding(
  2378     crate::durable_delivery_runtime::deliver_counted_binding(

$ nl -ba src/bin/monitor/durable_delivery_runtime.rs | sed -n '393,422p;607,650p;258,280p' | rg 'deliver_counted_binding|deliver_envelope\(|deliver_envelope_blocking|prepare\(|resume_deliverable|state.sink|MagiclawAuthoritativeSink|deliver_authoritative_blocking'
   258 impl AuthoritativeSinkPort for MagiclawAuthoritativeSink {
   275         crate::notify::deliver_authoritative_blocking(
   393 pub async fn deliver_counted_binding(
   408     match deliver_envelope(envelope).await {
   414 pub async fn deliver_envelope(
   419     tokio::task::spawn_blocking(move || deliver_envelope_blocking(state.as_ref(), envelope))
   607 fn deliver_envelope_blocking(
   614         .prepare(&envelope, 1, Utc::now())
   626             .resume_deliverable(
   628                 std::slice::from_ref(&state.sink),

$ nl -ba src/bin/monitor/push_templates.rs | sed -n '9116,9248p' | rg 'banner|BannerCtx|current_banner|evaluate_account_mode_hook|account_snapshot|stock_position'
  9119     banner: &BannerCtx,
  9124     dispatch_r04_lhb_outcome_with_loader(date, now_time, banner, move |trading_date| async move {
  9136     _banner: &BannerCtx,

$ nl -ba src/bin/monitor/notify.rs | sed -n '2344,2384p'
  2344	pub async fn push_counted_with_binding(
  2359	    let gate = crate::v14_adapter::v14_gate_counted_binding(
  2378	    crate::durable_delivery_runtime::deliver_counted_binding(

$ nl -ba src/bin/monitor/v14_adapter.rs | sed -n '302,342p;399,409p;941,960p'
   302	pub fn v14_gate_counted_binding(
   333	    v14_gate_prepared(V14PreparedGate {
   399	    // L5 governance 先判 (data_mode/frozen/quiet_hour/daily_limit)
   403	        current_governance_ctx()
   408	            return V14Gate::Denied("governance_context_unavailable".to_string());
   941	fn current_governance_ctx() -> Result<GovernanceContext, String> {
   945	    let banner = crate::LATEST_BANNER
   949	        .ok_or_else(|| "governance banner unavailable".to_string())?;
   959	            banner.account_mode,
```

R-04 producer/loader 的 banner 形参确实未使用，但其 counted notification 下游仍经
`v14_gate_counted_binding → v14_gate_prepared → current_governance_ctx →
LATEST_BANNER.account_mode`。因此只删除 outcome/loader 形参不能修复本缺陷；Gate B
必须同时实现 §4.1 的窄化 counted SourceOnly 治理入口。

R-03/R-08 的 banner 形参虽未使用，函数体却具有真实的本地持仓读取：

```text
$ nl -ba src/bin/monitor/push_templates.rs | sed -n '7925,7975p;8148,8181p;8695,8721p'
  7925	/// Load the local user-confirmed position snapshot for the review date.
  7927	/// This evidence is deliberately labelled as local/user-confirmed. It is not a
  7928	/// broker position batch and never authorizes holding/non-holding audience
  7939	        stock_analysis::database::user_position_snapshot::latest_user_position_snapshot()?
  7941	    if snapshot.effective_at.date_naive() != review_date {
  7948	    if snapshot.confirmed_at > chrono::Utc::now().fixed_offset() {
  7951	    if snapshot.confirm_empty {
  7955	    snapshot
  7956	        .items
  8148	    let positions = load_user_confirmed_r08_positions(date);
  8153	    let user_confirmed_holdings = positions.map(|positions| {
  8179	    let broker_holdings = Err(
  8180	        "VerifiedBrokerPositions unsupported: 未接入带 30 秒新鲜度和批次证据的券商源".to_string(),
  8695	/// R-03 涨停题材联动：基于完整、精确日期的已选涨停池批次与实盘持仓/自选交集（BR-106/BR-110/BR-140/BR-159）。
  8704	    let positions =
  8705	        match tokio::task::spawn_blocking(stock_analysis::portfolio::get_positions).await {
  8720	    let batch = match super::load_review_limit_chain_stocks(&positions, date).await {

$ nl -ba src/bin/monitor/push_templates.rs | sed -n '6923,6961p' | rg 'review_preflight|dispatch_r03_industry_chain_outcome|dispatch_r08_event_calendar_outcome|dispatch_r04_lhb_outcome|dispatch_r09_provider_top_n_outcome'
  6923     let preflight = review_preflight(context, due, is_test);
  6930                     dispatch_r03_industry_chain_outcome(&date, banner).await,
  6940                     dispatch_r04_lhb_outcome(&date, now, banner).await,
  6950                     dispatch_r08_event_calendar_outcome(&date, banner).await,
  6960                     dispatch_r09_provider_top_n_outcome(review_date).await,
```

R-03 的 `portfolio::get_positions` 是本地 portfolio projection；R-08 的
`latest_user_position_snapshot` 明确只是 local/user-confirmed，且代码显式声明
`VerifiedBrokerPositions unsupported`。它们都是当前业务输入依赖，却都缺少真实
账户快照的 provider、不可变 batch identity、30 秒 source freshness 与同批 broker
trade-sync watermark，因此不能满足 verified account gate。首版把它们留在
`LegacyAccountGate` 并固定保守失败；后续只能通过独立 Gate A 和真实 producer
证据重新分类。

其中 `main.rs:3875` 是常驻 monitor 启动治理的合法调用，不属于复盘 caller，静态
检查不得误删；`4002`、`4075-4076`、`4088` 才是本设计移除的复盘全局前置门。

观察：

- `main.rs:4002-4004`：手动 `--review` 在创建上下文和逐任务 dispatcher 前调用
  `evaluate_account_mode_hook(true)`，失败即整批返回。
- `main.rs:4075-4076`：严格 runner 无条件调用 `current_banner()?`，随后把同一
  banner 传给整个 batch。
- `main.rs:4088-4089`：常驻 scheduler 的 attempt 也在逐任务 dispatcher 前用同一
  account gate 整批返回。
- `review_batch.rs:871-930` 已有静态 preflight：R-02/R-05/R-06 Disabled，R-04
  在 21:00 前 ExpectedWait，R-09 在 15:35 前 ExpectedWait；R-09 的历史/休市/
  测试隔离也在 provider 前关闭。
- `push_templates.rs:6906-6961` 的整批函数要求 `&BannerCtx`；R-09 直接调用无
  banner 的 dispatcher。
- `push_templates.rs:9116-9137` 的 R-04 loader 参数是 `_banner` 且未读取，真实
  数据来自 `DragonTigerGateway::market_review`；但 `notify.rs:2359-2384` 后续
  counted gate 仍通过 `v14_adapter.rs:399-409,941-960` 读取 combined banner。
- `push_templates.rs:6535-6542` 的 R-09 只调用
  `CapitalDataGateway::provider_top_n_pair`。

根因是旧 caller-wide 前置门把“部分任务需要账户”错误提升为“所有任务必须有
账户”，并把账户状态通知是否确认与 source task 是否可运行耦合，扩大了故障域。

### 2.1 上游债务

| 债务 | 影响 | 本设计处理 |
|---|---|---|
| BR-108 banner 是跨 consumer 的共享可变状态 | 任一健康计算失败可使 banner 不可用 | 首版 review 路径零读取；未来另立 Gate A |
| BR-136 的真实账户要求被实现成整批门 | R-04/R-09 被连带阻断 | 只收窄作用域，不降低证据要求 |
| `evaluate_account_mode_hook` 同时做 metrics、DB、banner 和通知 | 任一步失败都阻断整批 | source-only 阶段不得调用 |
| R-04 producer 保留未使用 banner 参数，counted gate 下游仍读 combined banner | 删除形参仍会在 `current_governance_ctx` 失败 | Gate B 删除 outcome/loader 参数，并新增 allowlist counted SourceOnly gate |
| R-03/R-08 分别读取 portfolio projection / user-confirmed snapshot | 真实输入不是 verified broker account evidence | 标为 `LegacyAccountGate`，首版固定失败 |
| A-10/A-01 没有 banner 形参但受 caller-wide 门影响 | 现状也不能证明无账户依赖 | 标为 `UnclassifiedConservative` |
| `--test --review` 只关闭 R-09 provider | 测试可触发 R-04 真实龙虎榜网络 | preflight 同时关闭 R-04/R-09 |

后续若要把其他任务改成 `SourceOnly`，必须提供独立数据流证据并更新 BR-194。

## 3. 固定领域模型

新增闭集：

```rust
pub enum ReviewTaskDependency {
    SourceOnly,
    LegacyAccountGate,
    UnclassifiedConservative,
}
```

`ReviewTask::dependency()` 是唯一分类权威：

| 任务 | 依赖 | 理由 |
|---|---|---|
| R-04 | `SourceOnly` | producer 只消费完整当日 DragonTiger Gateway 批次；counted 下游须改走 allowlist SourceOnly governance，禁止 combined banner |
| R-09 | `SourceOnly` | 只消费完整当日 provider TopN 双批次 |
| R-03/R-08 | `LegacyAccountGate` | 当前分别读取 portfolio projection / local user-confirmed snapshot；两者均无 verified broker batch + 同批 trade-sync watermark |
| R-02/R-05/R-06 | `UnclassifiedConservative` | 未审计；static preflight 必须先 Disabled |
| A-10/A-01 | `UnclassifiedConservative` | 没有 banner 参数，尚未完成独立依赖审计 |

`LegacyAccountGate` 和 `UnclassifiedConservative` 都不是“已证明需要 banner”的
同义词。它们只表达迁移状态：本批不得放宽旧安全门。任何任务改成
`SourceOnly` 前都要独立追踪其 provider、账户/持仓读取、sink 和审计调用链。

本首版**冻结保守账户门为不可运行**。仓库现有 `AccountSnapshotInput` 没有与账户
快照同批的 broker trade-sync watermark，`compute_account_mode_metrics_blocking`
也明确返回“real broker trade-sync watermark is not connected”。因此 Gate B：

1. 不新增 `VerifiedReviewAccountBanner`、账户 provider DTO、成功 constructor 或
   success branch；
2. `LegacyAccountGate` / `UnclassifiedConservative` 在 preflight 后仍 runnable
   时，一律不读取账户 provider、不调用任务 provider/sink，逐任务返回
   `AccountMetricsIncomplete`；
3. `current_banner()`、本地 `stock_position`、banner cache、TEST_CODE seed、
   acquisition time 或持仓/成本推算均不得满足账户依赖；
4. 首版严格复盘路径不调用 AccountMode hook；常驻 monitor 的独立启动/周期治理
   仍可在复盘之外运行，但其成败不能改写 review task outcome，也不能让保守任务
   进入 provider/sink。

未来只有在另一个已登记业务规则与独立 Gate A 明确以下全部内容后，才可新增
账户成功分支：真实输入 DTO 的 owner/producer、同一 broker batch 的账户快照和
trade-sync watermark、不可变 batch identity、provider `source_captured_at`、
本地 `acquired_at`、30 秒 freshness、业务日和完整 metrics 校验，以及
producer→task→audit 的测试。该未来切片不属于 BR-194 首版。

账户失败使用结构化类型，不允许只把原因拼成自由文本：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAccountFailureReasonCode {
    AccountMetricsIncomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAccountDependencyStage {
    AcquireBatch,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewAccountDependencyFailure {
    stage: ReviewAccountDependencyStage,
    reason_code: ReviewAccountFailureReasonCode,
    retryable: bool,
    source_provider: Option<String>,
    source_time: Option<chrono::DateTime<chrono::FixedOffset>>,
    observed_at: chrono::DateTime<chrono::FixedOffset>,
    evidence_identity_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "failure_class", rename_all = "snake_case")]
pub enum ReviewTaskFailure {
    ExistingSourceFailure { retryable: bool, reason: String },
    AccountDependency(ReviewAccountDependencyFailure),
}
```

`ReviewTaskOutcome::Failed` 的精确新形状是
`Failed { failure: ReviewTaskFailure }`。首版账户门只有一个合法映射：

| stage | reason_code | retryable | source_provider | source_time | evidence_identity_hash |
|---|---|---:|---|---|---|
| `acquire_batch` | `account_metrics_incomplete` | `true` | `None` | `None` | `None` |

`observed_at` 是本次真实本地判定时刻，不得写入 `source_time`。只有未来真实账户
批次存在时，`source_provider`、`source_time` 与 `evidence_identity_hash` 才能为
`Some`；hash 必须是 domain-separated lowercase 64-hex SHA-256，且不得包含账户
明细。首版 constructor 必须把上述六列固定生成，调用者不能传入或覆写。

BR-140 的 `ReviewTaskTransition` 只在结构体**末尾**追加一个嵌套字段：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub failure: Option<ReviewTransitionFailure>,

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "failure_class",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ReviewTransitionFailure {
    ExistingSourceFailure {
        retryable: bool,
        reason: String,
    },
    AccountDependency {
        stage: ReviewAccountDependencyStage,
        reason_code: ReviewAccountFailureReasonCode,
        retryable: bool,
        source_provider: Option<String>,
        source_time: Option<chrono::DateTime<chrono::FixedOffset>>,
        observed_at: chrono::DateTime<chrono::FixedOffset>,
        evidence_identity_hash: Option<String>,
    },
}
```

上述 inline struct variants 是唯一 JSON 形状，不允许 newtype wrapper、flatten
别名或 catch-all variant。`AccountDependency` payload 原样携带
`stage/reason_code/retryable/source_provider/source_time/observed_at/
evidence_identity_hash`；顶层既有 `reason_code/retryable/source_time/observed_at`
也从 typed failure 复制，供旧 reader 保持兼容。
账户失败 transition 的既有 `source` 精确写
`account_dependency_unavailable`，`rule_ids` 增加 `BR-194`，且 `reason_code` 不再
追加自由文本 fingerprint；任务 identity、status=`failed` 和退避 1/5/15 分钟合同
保持不变。

writer/parser 的 `Some`/`None` 矩阵固定如下：

| transition 状态 | 新 writer | reader |
|---|---|---|
| `Failed(ExistingSourceFailure)` | `Some(ExistingSourceFailure{...})` | 只接受精确 variant/字段 |
| `Failed(AccountDependency)` | `Some(AccountDependency{...})` | 只接受精确 variant/字段 |
| 非 `Failed` | `None`，字段完全省略 | `Some` 一律拒绝 |
| 历史 `Failed` 且 JSON 没有 `failure` | 不产生新记录 | 永久接受为 `None` |

因为 `Option` 的默认 Serde 行为会把显式 `null` 当成 `None`，review audit reader
必须先读 `serde_json::Value`：存在 `failure` key 时值必须是 object，再进入 typed
反序列化；`failure:null`、未知 `failure_class`、未知字段和非 Failed 的 `Some`
都返回显式错误。只有 key **不存在**才是 legacy `None`。旧 transition 缺少末尾
`failure` 时反序列化为 `None`，重新序列化因 `skip_serializing_if` 不产生
`failure:null`，所以现有字段次序和 hash preimage byte-identical；不得迁移或重写
历史链。

兼容测试必须使用固定字面 fixture，不能在测试运行时调用新 writer 生成预期值：

1. 一条修改前的完整 `ReviewAuditRecord`（原始 JSON bytes、`prev_hash`、
   `record_hash` 均固定）read→serialize 后 payload bytes 与
   `review_audit_hash` byte-for-byte 相同；
2. 新 `ExistingSourceFailure` 的固定 JSON、`prev_hash`、`record_hash` 精确匹配；
3. 新 `AccountDependency` 的固定 JSON 精确包含三个 `null`，且固定 hash-chain
   精确匹配；
4. 分别拒绝 `failure:null`、未知 variant、未知字段、非 Failed 的 `Some`；
5. legacy 缺字段读为 `None`，再次输出时该 key 仍不存在。

`review_reason_category` 的关键词猜测只保留给
`ExistingSourceFailure { retryable, reason }`，`AccountDependency` 必须 pattern
match 后逐字段原样落 transition，禁止先转成 `String` 再猜分类。序列化回读测试
必须断言嵌套 JSON 中恰为 `failure_class=account_dependency`、
`stage=acquire_batch`、`reason_code=account_metrics_incomplete`、
`retryable=true`，三个可空来源/证据字段为 `null`，且 hash-chain 校验通过。

## 4. 数据流与固定执行顺序

```text
ReviewRunContext + due
  → review_preflight（Disabled / test / 日期 / 时间窗）
      └─ --test --review：R-04/R-09 Disabled，provider=0，sink=0
  → runnable 按 ReviewTaskDependency 分区
  → SourceOnly batch（不触碰账户）
      ├─ R-09（>=15:35）→ 当前 durable runtime（本设计独立复核）
      └─ R-04（>=21:00）→ 当前 durable runtime（本设计独立复核）
  → 仅当仍有 LegacyAccountGate / UnclassifiedConservative task 时
      └─ 每任务 Failed(AccountDependency{
           stage=acquire_batch,
           reason_code=account_metrics_incomplete,
           retryable=true,
           source_provider/source_time/evidence_identity_hash=None
         })
         └─ 账户 provider=0，task provider=0，sink=0
  → 合并 batch，按 ReviewTask 稳定排序，重复 task 拒绝
  → 当前 durable hydration + BR-140 transition audit
```

静态 preflight 必须最先运行，因此 Disabled/ExpectedWait 任务不会触发账户或
provider。source-only batch 必须先运行并完成持久投递尝试，再为仍 runnable 的
保守任务构造固定 typed failure；不存在账户 acquisition、账户 timeout 或账户成功
分支。

`evaluate_account_mode_hook(true)` 不再由 `run_review_only`、
`attempt_post_session_review` 或 `run_strict_review_only_inner` 调用。首版没有
可验证账户 producer，因此保守账户门任务形成上述固定 typed failure；已经完成的
R-04/R-09 outcome 必须原样进入 merge。`main.rs:3875` 的常驻 monitor 独立
AccountMode 治理不属于 review task data flow，静态检查必须保留它；其成功或失败
都不放行、覆盖或重新调度任何 review task。

preflight 的优先级固定为：test/live 隔离 → 静态 Disabled → business-date /
交易日合同 → 发布时间窗。因而 `--test --review` 的 R-04/R-09 无论墙钟是否已经
到 21:00/15:35 都是 `Disabled(test_environment_external_provider_blocked)`，不能
先落入 ExpectedWait，更不能调用真实 provider。

### 4.1 R-04 counted SourceOnly 治理缝

Gate B 新增两个**窄化、不可由 caller 选择任意 profile** 的入口（TO BE BUILT）：

```text
push_templates::dispatch_r04_lhb_outcome
  → notify::push_counted_source_only_with_binding
  → v14_adapter::v14_gate_counted_source_only_binding
  → v14_gate_prepared(context_source=CountedSourceOnly)
  → current_counted_source_only_governance_ctx
  → durable_delivery_runtime::deliver_counted_binding
```

该入口的 allowlist 首版只有 `(ReviewTask::R04, PushKind::ReviewLhb,
template_id=review_lhb_v1)`。caller 不得传一个“绕过 account”Boolean；入口必须从
private-field `CountedDeliveryBinding` 和其 canonical bytes 校验：

1. `is_counted_kind(kind)` 且 kind 恰为 `ReviewLhb`；
2. `schedule_occurrence_identity`、`TaskBinding.task_identity` 与 canonical
   `ReviewLhbSourceBinding.review_task_identity` 都是同一 R-04 identity；
3. origin 恰为 Provider，`as_of == business_date`，`observed_at` 存在，
   `ordered_batch_ids` 恰有一个非空 ID，且与 canonical source binding/evidence
   中的 batch ID 相同；
4. `source_evidence_fingerprint == SHA256(source_binding_canonical)`，
   `TaskBinding.transition_basis_sha256 ==
   SHA256(transition_basis_canonical)`，delivery subject 是 lowercase 64-hex；
5. canonical source binding 的 business date、template ID、provider provenance、
   正值/有限金额、披露和十席结构已经由
   `prepare_review_lhb_delivery` 校验，治理入口必须重新核对关键身份，不能以
   “constructor 已跑过”为由接受 canonical/typed 投影冲突。

任何 mismatch 都在 durable prepare 前以稳定 reason code
`counted_source_only_binding_invalid` 或
`counted_source_only_kind_not_allowed` 拒绝，provider 不重取、sink=0、durable
admission=0；不得回退 `push_counted_with_binding`。

`v14_gate_prepared` 的两个 Boolean context 必须收敛成闭集 context source：
`CombinedAccount`、`SourceFact`、`CountedSourceOnly`。只有
`CountedSourceOnly` 调用新的 `current_counted_source_only_governance_ctx`：

- 从 `monitor::data_mode::current_data_health_input/evaluate` 取得真实 process-local
  DataMode capability；读取失败即 `governance_context_unavailable`，不得填 Full；
- 使用同一真实本地时钟执行 quiet-hour，并继续从 analytics store 取得日限额；
- `is_frozen=false` 只表示此 SourceOnly profile 的账户冻结维度 **N/A**，不表示或
  推断账户为 Normal，也不写入 AccountMode evidence；
- 继续执行原 `default_profile_for_kind(ReviewLhb)`、L5 DataMode、quiet-hour、
  daily-limit、LaunchGate 与拒绝审计；不得复用 BR-137 的 source-fact allowlist 或
  获得其 DataMode Down 豁免；
- Approved 后仍只调用现有 `deliver_counted_binding`，由
  `DurableDeliveryCoordinator` 唯一执行预算、冷却/去重、事务 admission、fence、
  retry、authoritative sink、push-log、delivery audit 和 hydration。

generic `push_counted_with_binding/v14_gate_counted_binding` 保持
`CombinedAccount → current_governance_ctx → LATEST_BANNER`，其他 counted producer
不会因 BR-194 获得账户豁免。R-09 已构造完整 `DeliveryEnvelope` 并直接进入同一
durable owner，不经过这一 R-04 专用 adapter；本设计不把 R-09 改到 counted wrapper。

### 4.2 Producer → task → durable → sink → audit

以下是对当前代码的 old-module adoption，不引用 BR-192 规格作为规范权威。BR-194
Gate B/C/D 自行证明：canonical immutable binding、单一 durable owner、terminal
sink receipt、append-only审计、hydration 零重发和 test/prod authority 隔离；任一
证明失败即阻塞 BR-194，不得以 BR-192 的文档状态代替证据。

R-09：

```text
CapitalDataGateway::provider_top_n_pair(review_date)
  → dispatch_r09_provider_top_n_outcome
  → ReviewTask::R09 / review_provider_top_n_v1
  → canonical DeliveryEnvelope + BR-140 task binding
  → durable_delivery_runtime::deliver_envelope
  → DurableDeliveryCoordinator prepare/resume/reconcile
  → MagiclawAuthoritativeSink::deliver
  → notify::deliver_authoritative_blocking
  → pinned push log + push.delivery.audit hash chain
  → durable BR-140 hydration
```

R-04：

```text
DragonTigerGateway::market_review(review_date, 5, 5)
  → dispatch_r04_lhb_outcome
  → ReviewTask::R04 / ReviewLhb / review_lhb_v1
  → CountedDeliveryBinding
  → notify::push_counted_source_only_with_binding
  → v14_adapter::v14_gate_counted_source_only_binding
  → real DataMode + quiet-hour + daily-limit L5（AccountMode N/A）
  → durable_delivery_runtime::deliver_counted_binding
  → DurableDeliveryCoordinator prepare/resume/reconcile
  → MagiclawAuthoritativeSink::deliver
  → notify::deliver_authoritative_blocking
  → pinned push log + push.delivery.audit hash chain
  → durable BR-140 hydration
```

账户门拆分不得旁路上述链。SourceOnly 只从 R-04 的 pre-durable L5 context 中移除
combined AccountMode/banner 读取，不移除其他治理或 durable owner。只有真实 sink
接受并完成权威审计后才是
`Delivered`；dedup、拒绝、sink 失败和 uncertain 保持原 typed outcome。

### 4.3 合并与恢复

新增唯一 batch 合并入口：

- 输入是 preflight、source-only、account-required 三组 outcome。
- 同一任务出现两次显式失败，禁止后写覆盖或静默去重。
- 输出按 `ReviewTask` 稳定升序，不改变 outcome 类型。

若 source-only 已物理投递，而后续固定 failure 合并或外层 timeout 失败，当前
durable state 仍是权威；下一轮用既有 hydration 恢复 R-04/R-09 transition，
禁止重新调用 sink 或追加 legacy duplicate transition。

### 4.4 受控 terminal replay 与不可变 attempt evidence

当前常驻 scheduler 在成功 dispatcher 调用后会在**同一轮**立即执行 durable
hydration；后续 tick 对 terminal task 得到空 due set，不构成第二次 task dispatch。
因此最终 `hydration_state=Applied`、一个 sink result 和一个 delivery audit 只能证明
最终状态，不能证明发生过“第二次 scheduler attempt”。Gate D 删除该不可验证表述，
改由以下 **TO BE BUILT** 的显式发布验证 seam 证明同一 terminal decision 不重发：

```text
monitor --br194-audited-terminal-replay --business-date DATE --task R-04|R-09
  → review_task_identity(DATE, task)
  → DurableDeliveryCoordinator::load_exact_terminal_replay_input
  → begin_review_terminal_replay（持久化 Started + 前水位）
  → durable_delivery_runtime::replay_terminal_envelope
       → 复用 production envelope validate/prepare/terminal classification
       → terminal/hydration Applied：返回 ExistingTerminalHydrated
       → 任何 resume/sink eligibility：在 sink callable 取得前 fail closed
  → finish_review_terminal_replay（持久化 Passed/Failed + 后水位 + counters）
  → immutable durable audit append/ack
```

该 mode 必须在 monitor 的数据库、provider、普通 sink 和常驻任务初始化前作为独立
early-return command 解析；只接受 production namespace、真实交易日、R-04/R-09，
拒绝 TEST_CODE、未来日期及 0 条或多条匹配 decision。0/多 decision 在 attempt begin
前显式 exit 1、provider/sink=0；唯一 decision 的 terminal/hydration 状态由 begin 后
的 replay classification 检查，使该失败可形成 Failed completion。
输入 envelope、task binding、transition 和 decision identity 全部从固定 production
durable authority 读取并重新验证，不访问 DragonTiger/TopN provider，不重建正文，
不接受路径/env override。`replay_terminal_envelope` 的函数签名不接收真实 sink；
它复用 production validate/prepare/terminal classification，但若分类结果要求
`resume_deliverable` 或 sink ownership，立即返回
`terminal_replay_would_require_sink`，在任何外部 sink 对象可取得前停止。

#### 4.4.1 唯一 owner、schema 与 identity

`DurableDeliveryCoordinator` 是 replay attempt 的唯一 persistence owner。为了不以
原地 update 冒充“不可变 evidence”，schema 迁移新增一张 immutable start 表和一张
append-once completion 表；两表都禁止 UPDATE/DELETE trigger，禁止写 dispatcher
JSONL 代替：

```text
# review_terminal_replay_attempts（immutable start）
attempt_identity TEXT PRIMARY KEY
business_date TEXT NOT NULL
review_task TEXT NOT NULL CHECK(review_task IN ('R-04','R-09'))
task_identity TEXT NOT NULL
decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity)
replay_ordinal INTEGER NOT NULL CHECK(replay_ordinal > 0)
started_at TEXT NOT NULL
pre_sink_count INTEGER NOT NULL
pre_sink_set_sha256 TEXT NOT NULL
pre_delivery_audit_count INTEGER NOT NULL
pre_delivery_audit_set_sha256 TEXT NOT NULL
provider_calls INTEGER NOT NULL CHECK(provider_calls = 0)
start_canonical BLOB NOT NULL
start_sha256 TEXT NOT NULL
start_audit_identity TEXT NOT NULL UNIQUE
  REFERENCES immutable_audit_outbox(audit_identity)
UNIQUE(attempt_identity,decision_identity)
UNIQUE(business_date,review_task,task_identity,decision_identity,replay_ordinal)

# review_terminal_replay_completions（append once；0 或 1 行/attempt）
attempt_identity TEXT PRIMARY KEY
decision_identity TEXT NOT NULL
state TEXT NOT NULL CHECK(state IN ('Passed','Failed'))
completed_at TEXT NOT NULL
post_sink_count INTEGER NOT NULL
post_sink_set_sha256 TEXT NOT NULL
post_delivery_audit_count INTEGER NOT NULL
post_delivery_audit_set_sha256 TEXT NOT NULL
provider_calls INTEGER NOT NULL CHECK(provider_calls = 0)
resume_calls INTEGER NOT NULL CHECK(resume_calls >= 0)
sink_calls INTEGER NOT NULL CHECK(sink_calls >= 0)
delivery_audit_appends INTEGER NOT NULL CHECK(delivery_audit_appends >= 0)
reason_code TEXT NOT NULL
completion_canonical BLOB NOT NULL
completion_sha256 TEXT NOT NULL
completion_audit_identity TEXT NOT NULL UNIQUE
  REFERENCES immutable_audit_outbox(audit_identity)
CHECK(state != 'Passed' OR
      (resume_calls=0 AND sink_calls=0 AND delivery_audit_appends=0))
FOREIGN KEY(attempt_identity,decision_identity)
  REFERENCES review_terminal_replay_attempts(attempt_identity,decision_identity)
```

`attempt_identity` 精确为
`stable_identity("BR-194-terminal-replay-attempt-v1",
[business_date, review_task, task_identity, decision_identity,
replay_ordinal_decimal])`。ordinal 在 coordinator 的 `BEGIN IMMEDIATE` 内按同一
`(business_date, task_identity, decision_identity)` 现存最大值加一分配，不能由 CLI
传入、不能用 wall clock/rowid/随机数。start/completion canonical 使用
`serde_json::to_vec` 的 typed、`deny_unknown_fields` 结构；普通 SHA-256 必须等于
各自 `start_sha256/completion_sha256`。attempt start 行和 completion 行写入后都
不得 UPDATE/DELETE；是否完成只由 completion 行是否存在及其 state 判定。
迁移必须显式创建四个不可变 trigger：
`immutable_review_terminal_replay_attempt_update`、
`immutable_review_terminal_replay_attempt_delete`、
`immutable_review_terminal_replay_completion_update` 和
`immutable_review_terminal_replay_completion_delete`，分别对两表的 UPDATE/DELETE
执行 `RAISE(ABORT, ...)`。completion 的 composite FK 是
`attempt_identity + decision_identity` 的不可分割 authority binding；禁止仅凭两个
彼此独立的 FK 接受“attempt 属于 decision A、completion 声称 decision B”。

canonical 字段与声明顺序固定如下，不把 audit identity 放进其自身 preimage：

```text
ReviewTerminalReplayStartCanonical {
  schema_version=1, attempt_identity, business_date, review_task, task_identity,
  decision_identity, replay_ordinal, started_at, pre_sink_watermark,
  pre_delivery_audit_watermark, provider_calls
}
ReviewTerminalReplayCompletionCanonical {
  schema_version=1, attempt_identity, decision_identity, state, completed_at,
  post_sink_watermark, post_delivery_audit_watermark, provider_calls,
  resume_calls, sink_calls, delivery_audit_appends, reason_code
}
AuthorityWatermark { count, ordered_identity_set_sha256 }
```

`start_audit_identity` / `completion_audit_identity` 分别由现有 `enqueue_audit` 对上述
已冻结 canonical payload 生成并另列保存。两个调用的
`attempt_identity` 参数都必须精确传 `None`：当前
`immutable_audit_outbox.attempt_identity` 的 FK authority 只允许
`delivery_attempts.attempt_identity`，replay attempt 不得伪装成 delivery attempt。
因此两条审计 identity 的冻结 preimage 分别为：

```text
stable_identity("delivery-critical-audit-v1", [
  decision_identity, "NONE", "ReviewTerminalReplayStarted",
  SHA256(start_canonical)
])
stable_identity("delivery-critical-audit-v1", [
  decision_identity, "NONE", "ReviewTerminalReplayCompleted",
  SHA256(completion_canonical)
])
```

`AUDIT_KINDS` 与 outbox schema 的闭集必须同时登记这两个 kind。start/completion
插入前，coordinator 先在同一 transaction 调用 `enqueue_audit(..., None, ...)`；
随后 replay 行以 FK 引用返回的 audit identity。两张 replay 表各自的 INSERT
authority trigger
`validate_review_terminal_replay_attempt_audit_insert` /
`validate_review_terminal_replay_completion_audit_insert` 必须 exact join
`immutable_audit_outbox` 并验证：

- `audit_identity` 等于 replay 行保存值；
- `decision_identity` 与 replay 行相同；
- `attempt_identity IS NULL`；
- kind 分别精确为 `ReviewTerminalReplayStarted` /
  `ReviewTerminalReplayCompleted`；
- `audit_canonical` 与 replay canonical byte-exact 相等；
- `audit_sha256` 与 replay SHA-256 相等，且由 canonical 重算一致。

任一不等立即 `RAISE(ABORT, ...)`。canonical 内嵌 audit identity、以 replay
attempt 填 outbox delivery-attempt FK、由 audit identity 反推 canonical，或只靠
应用层比较而没有 DB authority trigger，都必须拒绝。

水位不是不可验证的日志计数。coordinator 在同一 decision 范围内构造：

- `sink watermark`：按 `result_event_identity ASC` 排序的
  `(result_event_identity, attempt_identity, result_sha256)` canonical 数组的
  `count + SHA256(bytes)`；
- `delivery-audit watermark`：按 `result_event_identity ASC` 排序的非空
  `(result_event_identity, delivery_audit_ref, frozen_delivery_audit_sha256)` canonical
  数组的 `count + SHA256(bytes)`。

Started 事务冻结两项前水位、append-only start 行，并向
`immutable_audit_outbox` 原子加入 `ReviewTerminalReplayStarted`；start outbox 必须
先 append/ack 到 fixed point，之后才允许执行 replay。Passed/Failed 事务 append-once
写 completion 行，冻结后水位、真实
`resume_calls/sink_calls/delivery_audit_appends` 和稳定 reason code，并加入
`ReviewTerminalReplayCompleted`。两类 outbox 必须继续由现有 append/ack owner 写入
`data/durable_delivery_audit/`。schema migration、BEGIN、append、ack 或 completion
失败均 exit 1；没有 completion 的 start 不得覆盖或伪装 Passed，重试分配下一
ordinal，历史 start/failed completion 全部保留。重复 completion INSERT、completion
指向其他 decision、审计 kind/canonical/hash/decision 不匹配、或 start/completion
audit 尚未 `Appended` 均不是 Passed evidence。

Passed 的充要条件是：同一 stored envelope 经 replay 返回
`ExistingTerminalHydrated`，`provider_calls=resume_calls=sink_calls=
delivery_audit_appends=0`，pre sink/delivery-audit count 各恰为 1，四个前后
count/hash 完全相等，并且 start/completion outbox 都为 Appended，
`reason_code=existing_terminal_hydrated`。任何不等 append
Failed completion，reason code 只能取
`terminal_replay_identity_invalid/terminal_replay_not_delivered/
terminal_replay_hydration_not_applied/
terminal_replay_would_require_sink/terminal_replay_watermark_changed/
terminal_replay_evidence_unavailable`；不能返回 review Delivered、不能修改原 task
transition、不能调用 provider/sink，也不能追加 `push.delivery.audit`。

#### 4.4.2 固定 authority manifest 与 verifier join

Gate B 新增编译期常量 `BR194_REPLAY_AUTHORITY_MANIFEST_V1`，同时由 CLI 与 verifier
的 fixture test 锁定以下内容，不接受运行时替换：

```text
database=data/durable_delivery.sqlite3
durable_audit_dir=data/durable_delivery_audit/
push_log_dir=data/push_log/
delivery_audit_dir=data/event_audit/
attempt_table=review_terminal_replay_attempts
completion_table=review_terminal_replay_completions
start_audit_kind=ReviewTerminalReplayStarted
completion_audit_kind=ReviewTerminalReplayCompleted
attempt_identity_domain=BR-194-terminal-replay-attempt-v1
audit_identity_domain=delivery-critical-audit-v1
audit_attempt_binding=NONE
```

只读 verifier 先完成原 producer→task→durable→sink→push-log→delivery-audit join，
再要求同一 `(business_date, task_identity, decision_identity)` 的最新 ordinal 恰有
一个 immutable start + 一个 Passed completion；它重算 attempt identity、两份
canonical/hash 和两项
前后水位，并以
`start_audit_identity/completion_audit_identity` 精确 join 两条已 Appended durable
audit；verifier 必须用
`stable_identity("delivery-critical-audit-v1",
[decision_identity,"NONE",audit_kind,audit_sha256])` 重算两条 audit identity，并
验证 outbox 的 `attempt_identity IS NULL`、decision/kind/canonical/hash 逐项精确
相等。它还必须核对 schema catalog 中 composite FK、两条 audit FK、两条 authority
INSERT trigger 和四条 UPDATE/DELETE immutable trigger 均存在且定义未弱化。
Failed completion/仅有 start 的 attempt 可保留供审计，但指定的最新 ordinal
没有 Passed completion、存在字段缺失、authority 变化或任一计数/hash 不等都
exit 1。最终一条
`push.delivery.audit` 还必须与 replay 前后唯一 `delivery_audit_ref` 相同；不得把
replay completion audit 误计为 delivery audit。

## 5. 失败模型

| 故障 | 结果 | 禁止 |
|---|---|---|
| R-04 早于 21:00 | ExpectedWait；零 provider、零账户 | 冒充 NoData |
| R-09 早于 15:35 | ExpectedWait；零 provider、零账户 | 提前抓取 |
| `--test --review` 的 R-04/R-09 | preflight Disabled；provider=0、sink=0 | DragonTiger/TopN 真实网络或 dry-run sink |
| R-09 历史/休市隔离 | 既有 Failed；零网络 | 旧缓存回退 |
| 首版没有同批账户+trade watermark producer | 保守账户门任务固定 typed `account_metrics_incomplete`；全部 provider/sink=0 | 不可达 success branch、裸/默认/stale banner、阻断 R-04/R-09 |
| 独立 AccountMode 状态持久化或通知失败 | 由既有治理单独记录，不进入 review outcome | 恢复 review 全局前置门或把通知失败猜成任务失败 |
| R-04/R-09 provider 不完整 | 对应任务既有 typed Failed | 跨源补齐、补零 |
| R-04 SourceOnly kind/task/template/origin/batch/hash 不匹配 | durable 前 Denied；sink/admission=0 | 回退 generic counted gate、重取 provider、补字段 |
| R-04 DataMode capability/analytics store 不可用 | L5 显式 Denied；sink/admission=0 | 读取 combined banner、填 Full/Normal、跳过拒绝审计 |
| R-04 LaunchGate 拒绝 | 专用 entry 在 v14/durable 前 Denied；durable admission=0、sink=0 | SourceOnly 绕过、后置或删除 LaunchGate |
| R-04 L5 DataMode/quiet-hour/daily-limit 拒绝 | 保留原 Denied 与 analytics audit | 因 SourceOnly 跳过非账户治理 |
| durable/sink/audit 失败 | 当前代码的 typed disposition；由本设计 Gate B/D 独立复核 | 走旧 push |
| terminal replay 为 0/多 decision | begin 前 exit 1；provider/sink=0、无 attempt | 选最新猜测、重建 envelope |
| terminal replay 唯一 decision 不是 Delivered + Applied | append Failed completion/exit 1；provider/sink=0 | 真实重发、覆盖 start |
| replay 前后水位不同或 resume/sink/audit append 非零 | attempt 以稳定 reason Failed 并留审计；Gate D fail | 输出零增量、覆盖 Started/Failed |
| replay start outbox append/ack 失败 | exit 1；classification/provider/resume/sink 均为 0；dangling start 保留 | Pending 审计后继续分类、覆盖原 start |
| replay completion write/outbox append/ack 失败 | exit 1；verifier 拒绝 Pending/缺失 authority；下次用下一 ordinal | 把内存结果或 logging-only 当 Passed |
| replay UPDATE/DELETE、重复/mismatched completion、audit binding 不等 | DB trigger/FK 原子拒绝；exit 1 | 应用层静默修正、last-row-wins |
| replay ordinal 双连接竞争 | `BEGIN IMMEDIATE` 串行分配唯一连续 ordinal | caller ordinal、clock/rowid/random、把 `SQLITE_BUSY` 当成功 |
| replay evidence begin/completion/outbox 失败 | exit 1；历史与 Started 原样保留 | logging-only、用最终 hydration 推断 replay |
| 合并重复 task | 显式编排错误 | 覆盖或静默去重 |
| source delivered、保守任务固定 failure/merge 后续失败 | hydration 收敛 source；保守任务重试 | 重发 source |

稳定、脱敏日志：

```text
[复盘依赖][BR-194] task=R-09 dependency=source_only status=ready
[复盘依赖][BR-194] task=R-04 dependency=source_only status=expected_wait
[复盘依赖][BR-194] task=R-04 status=disabled reason_code=test_environment_external_provider_blocked provider_calls=0 sink_calls=0
[复盘依赖][BR-194] task=R-09 status=disabled reason_code=test_environment_external_provider_blocked provider_calls=0 sink_calls=0
[复盘依赖][BR-194] dependency=legacy_account_gate status=unavailable affected_count=N stage=acquire_batch reason_code=account_metrics_incomplete retryable=true source_provider=none source_time=none
```

不得记录账户明细、现金、持仓原文或 provider payload。

## 6. 旧模块与改名影响

| 模块 | 决定 | 理由 |
|---|---|---|
| `ReviewTask` | adopt + dependency | 稳定任务身份和排序权威 |
| `review_preflight` | adopt，提升到依赖前 | 已有正确时间/测试/Disabled 语义 |
| outcomes/schedule | adopt | 已有 typed 状态和 1/5/15 退避 |
| 当前 durable R-04/R-09 代码 | adopt + 本设计独立验证 | 作为旧模块审计，不依赖其未过 Gate C 的规格 |
| `notify::push_counted_with_binding` | retain for CombinedAccount callers | 当前必经 combined banner，不可供 R-04 SourceOnly 继续调用 |
| `v14_adapter::v14_gate_prepared` | adopt + closed context-source refactor | 保留 Launch/L5/analytics；新增独立 `CountedSourceOnly` context |
| `current_source_fact_governance_ctx` | reject for R-04 reuse | BR-137 allowlist/DataMode 语义不同，复用会扩大豁免 |
| `durable_delivery_runtime` prepare/terminal classification | adopt + shared replay seam | 受控 replay 必须复用相同 envelope validation 与 terminal classification，但函数签名不取得真实 sink |
| `DurableDeliveryCoordinator` | adopt + replay evidence owner | 唯一保存 Started/Passed/Failed、前后水位和不可变 outbox；dispatcher log 不是 authority |
| 普通 scheduler 的同轮 hydration | retain；不作“两次 attempt”证据 | 当前成功 dispatch 后立即 hydration，后续空 due tick 不能冒充第二次 task dispatch |
| `current_banner()` | reject as evidence | 裸 banner 无 provider/batch/time/date/completeness |
| `VerifiedReviewAccountBanner` | reject/defer | 当前无同批 broker trade watermark；首版禁止虚构 producer 或不可达成功分支 |
| caller-wide account pre-block | reject | 扩大故障域 |
| default/stale banner fallback | reject | 违反 2.1/2.2/2.4 |
| legacy direct push/transition | reject | 会产生双 authority |

无业务 identifier 改名：R-04/R-09、PushKind、`review_lhb_v1`、
`review_provider_top_n_v1`、BR-140 task identity 均保持不变。

API 收敛包括删除 R-04 outcome/loader 未使用的 `&BannerCtx` 参数，以及把 R-04
从 generic `push_counted_with_binding` 切到仅 allowlist ReviewLhb 的
`push_counted_source_only_with_binding`。调用点：

- `dispatch_post_session_review`
- `dispatch_r04_lhb_real` 兼容 wrapper
- R-04 loader 单测
- `notify.rs` counted entry 单测
- `v14_adapter.rs` context-source/L5 单测
- 静态源码合同测试

兼容 wrapper 若仍被非复盘路径使用，只能在 wrapper 边界保留 banner 形参；生产
outcome 路径不得接收它。不得创建 dummy banner 满足旧签名。

## 7. Gate B 文件清单

- `src/bin/monitor/review_batch.rs`：依赖闭集、mapping、唯一 merge、单测。
- `src/bin/monitor/main.rs`：`run_review_only`、`attempt_post_session_review`、
  `run_strict_review_only_inner` 三个生产 caller 去掉 review-wide AccountMode/banner
  前置；保留独立常驻 AccountMode 治理；日志。
- `src/bin/monitor/push_templates.rs`：唯一中央 dispatcher 显式分区；R-04 移除未使用
  banner，并改走窄化 counted SourceOnly entry；计数测试。
- `src/bin/monitor/notify.rs`：新增仅允许 ReviewLhb/R-04 binding 的
  `push_counted_source_only_with_binding`；通过 L5 后仍调用唯一
  `deliver_counted_binding`，generic counted entry 不变；专用 entry 必须在
  v14/durable 前继续调用 `launch_gate_check(kind)`。
- `src/bin/monitor/v14_adapter.rs`：closed context-source、R-04 binding allowlist、
  real DataMode/quiet-hour/daily-limit SourceOnly context 和拒绝审计；不得读取 banner
  或复用 BR-137 DataMode 豁免。
- `src/durable_delivery/model.rs`、`schema.rs`、`coordinator.rs`：**TO BE BUILT**
  typed replay attempt、schema migration、稳定 identity、前后水位、Started/Completed
  immutable outbox 与 exact query；唯一 persistence owner。
- `src/bin/monitor/durable_delivery_runtime.rs`：**TO BE BUILT**
  `replay_terminal_envelope`，复用 production validate/prepare/terminal
  classification，且函数签名不得取得真实 sink。
- `src/bin/monitor/main.rs`：在普通初始化前增加
  `--br194-audited-terminal-replay` early-return mode；只接受固定 production
  authority、真实日期、R-04/R-09，不启动 provider/普通 sink/scheduler。
- `tests/monitor_help_isolation.rs`：**必须修改**既有
  `review_process_initializes_account_governance_before_dispatch` 预期，并新增
  `--test --review` R-04/R-09 provider/sink 零调用进程测试。
- `tools/compliance/lib/check_br194_review_dependency.sh`：**TO BE BUILT**，只检查
  三个复盘 production caller、唯一中央 dispatcher、R-04 签名/专用 counted
  SourceOnly call chain 和 test preflight；不得把常驻 monitor 启动时合法的
  AccountMode 治理调用判成违规。
- `tools/compliance/check.sh`：调用上述 BR-194 静态合同，使其成为 Gate C
  blocking check。
- `tools/release/verify_br194_review_join.py`：**TO BE BUILT**，只读验证固定生产
  authorities 的 provider binding→task→durable→sink→push log→audit join 与
  hydration 最终状态，再 join typed terminal replay attempt 的 start/completion
  audit 与前后水位。

不改 `config/*.toml`、阈值、provider 合同、模板正文和 `Cargo.toml`。schema 只允许
追加 4.4 精确定义的 replay evidence table/outbox kinds；不得改既有 decision、
attempt、sink、transition 或 delivery-audit 语义。

## 8. Machine-checkable 验收

### 8.1 Gate B

```bash
set -euo pipefail
run_exact_test() {
  local target="$1" name="$2" listing paths count full_path
  local -a cargo_target
  case "$target" in
    monitor-bin) cargo_target=(--bin monitor) ;;
    monitor-help-isolation) cargo_target=(--test monitor_help_isolation) ;;
    *) return 2 ;;
  esac
  listing="$(cargo test "${cargo_target[@]}" -- --list --format terse)"
  paths="$(printf '%s\n' "$listing" | sed -n 's/: test$//p' |
    awk -F'::' -v name="$name" '$NF == name { print }')"
  count="$(printf '%s\n' "$paths" | sed '/^$/d' | wc -l | tr -d ' ')"
  test "$count" -eq 1
  full_path="$(printf '%s\n' "$paths" | sed -n '1p')"
  cargo test "${cargo_target[@]}" "$full_path" -- \
    --exact --include-ignored --test-threads=1
}
run_exact_test monitor-bin br194_review_task_dependency_mapping
run_exact_test monitor-bin br194_preflight_precedes_dependency_acquisition
run_exact_test monitor-bin br194_source_only_runs_before_frozen_account_tasks
run_exact_test monitor-bin br194_account_tasks_are_frozen_without_real_batch_watermark
run_exact_test monitor-bin br194_account_failure_serializes_exact_transition_audit
run_exact_test monitor-bin br194_legacy_transition_fixture_remains_byte_identical_and_hash_valid
run_exact_test monitor-bin br194_review_batch_merge_rejects_duplicate_task
run_exact_test monitor-bin br194_time_boundaries_1535_and_2100
run_exact_test monitor-bin br194_r04_source_only_gate_never_reads_banner
run_exact_test monitor-bin br194_r04_source_only_preserves_l5_and_durable_entry
run_exact_test monitor-bin br194_r04_source_only_denied_launch_has_zero_durable_and_sink
run_exact_test monitor-bin br194_terminal_replay_passes_with_equal_authority_watermarks
run_exact_test monitor-bin br194_terminal_replay_sink_eligibility_fails_before_sink
run_exact_test monitor-bin br194_terminal_replay_started_or_failed_cannot_verify
run_exact_test monitor-bin br194_terminal_replay_identity_and_audit_join_are_exact
run_exact_test monitor-bin br194_terminal_replay_audit_uses_none_delivery_attempt_binding
run_exact_test monitor-bin br194_terminal_replay_tables_reject_update_delete_and_second_completion
run_exact_test monitor-bin br194_terminal_replay_rejects_mismatched_completion_decision_and_audit
run_exact_test monitor-bin br194_terminal_replay_start_audit_ack_failure_blocks_classification
run_exact_test monitor-bin br194_terminal_replay_completion_write_or_ack_failure_never_passes
run_exact_test monitor-bin br194_terminal_replay_ordinals_advance_after_dangling_or_failed_attempts
run_exact_test monitor-bin br194_terminal_replay_cross_connection_contention_allocates_unique_ordinals
run_exact_test monitor-help-isolation \
  br194_terminal_replay_cli_rejects_ordinal_override_before_database_open
run_exact_test monitor-help-isolation \
  br194_test_review_blocks_r04_r09_provider_and_sink_before_account_gate
```

`run_exact_test` 先解析 test-harness list 输出，以完整 Rust test path 的末段精确
匹配目标名并断言 count=1，再把唯一完整 path 交给 `--exact` 执行；因此不存在
“0 tests / exit 0”、ignored 未执行或同名多条误命中可满足上述 Gate B 的路径。

测试必须证明：

- 生产模式 15:34:59 的 R-09、20:59:59 的 R-04 为 ExpectedWait，账户/provider
  调用均为 0。
- 15:35:00 的生产 R-09 在账户 evidence 不可用时只调用其 provider 一次。
- 21:00:00 的生产 R-04 在账户 evidence 不可用时只调用其 provider 一次。
- R-04 在 `LATEST_BANNER=None` 时仍可执行专用 SourceOnly L5；DataMode capability、
  quiet-hour、日限额或 analytics store 任何一项拒绝/失败时 durable admission 与
  sink 均为 0。
- R-04 专用 entry 的 LaunchGate 拒绝发生在 v14 与 durable admission 之前，
  `launch_gate_check` 恰调用一次，durable admission=0、sink=0；SourceOnly 不获得
  launch-stage 豁免。
- 专用 SourceOnly entry 对非 ReviewLhb、非 R-04 task/template、非 Provider origin、
  batch/date/hash/canonical 投影不一致逐项 fail closed；每个 mutant 都不得落入
  generic counted gate。
- R-04 专用 gate Approved 后恰调用现有 `deliver_counted_binding` 一次；预算、去重、
  fence、sink、push-log、delivery audit 和 hydration 的现有测试仍通过。
- terminal replay 只从唯一 Delivered + Applied durable decision 读取 stored
  envelope；Passed 必须复用 production validate/prepare/terminal classification，
  且 provider/resume/sink/delivery-audit append 都为 0、前后两项水位 byte-exact
  相等。让 replay 获得 sink eligibility、修改 envelope/hash、制造 0/多 decision、
  留在 Started/Failed、破坏 attempt/audit join 均须非零失败且不触达真实 sink；
  0/多 decision 必须在 begin 前零 attempt，唯一但非 terminal/未 hydration 的
  decision 必须保留 start + Failed completion。
- replay Started/Completed 必须逐项证明调用
  `enqueue_audit(decision_identity, None, exact_kind, exact_canonical, ...)`；测试按
  `delivery-critical-audit-v1 + [decision_identity,"NONE",kind,sha256]` 独立重算
  identity，并拒绝把 replay attempt 填入 outbox `attempt_identity`、改变 domain、
  改变字段顺序、删掉 `review_task`、从 wall clock/rowid/random/调用方取得 ordinal。
- 真实 SQLite 测试必须分别对 start/completion 表执行 UPDATE 与 DELETE，四项都
  `SQLITE_CONSTRAINT_TRIGGER`；同一 attempt 第二次 completion INSERT、completion
  使用其他 decision、start/completion audit 的 decision/kind/canonical/hash 或
  `attempt_identity IS NULL` 任一不匹配都失败且不产生 Passed。测试还必须临时删除
  或放宽每一条 immutable/authority trigger，证明 schema/verifier/mutation harness
  会失败，不能只测试 Rust API。
- start audit append/ack fault injection 必须证明 classification counter 保持 0、
  provider/resume/sink/delivery-audit append 均为 0、进程非零退出且只留下可审计
  dangling start；completion INSERT 故障或 completion audit append/ack 故障必须
  非零退出，verifier 不得把 Pending/缺失 completion audit 当 Passed。后续重试不得
  覆盖原行。
- ordinal 必须覆盖单连接顺序与两个独立 SQLite connection 的竞争：已有 dangling
  Started 或 Failed ordinal=N 后下一次必须是 N+1；两个 connection 在相同 key 上
  经 `BEGIN IMMEDIATE` 竞争后必须得到唯一、连续的 ordinal，不得用
  `SQLITE_BUSY`、last-row-wins 或覆盖旧行冒充通过。
- CLI process test 必须在打开/迁移数据库前拒绝 `--replay-ordinal`、同义别名、
  重复/未知 ordinal override，exit 非零且零 DB mutation；唯一合法 ordinal source
  是 coordinator transaction 中的 `MAX(replay_ordinal)+1`。
- `--test --review` 的 R-04/R-09 都由 preflight Disabled；DragonTiger/TopN provider
  与 sink 分别为 0 次。
- `LegacyAccountGate` / `UnclassifiedConservative` 首版始终 account provider /
  task provider / sink 为 0，并返回精确 `acquire_batch /
  account_metrics_incomplete / retryable=true`；transition JSON 原样保存 typed 字段，
  不能经过 `review_reason_category` 关键词猜测。
- static Disabled task 在依赖获取前移除。
- source-only Delivered 后账户失败，合并仍保留 Delivered。
- 既有 process test 不再断言 AccountMode 通知先于 dispatcher；它必须断言
  即使启动治理报告账户 metrics 不完整，`--test --review` 仍进入 preflight 并把
  R-04/R-09 双禁用且零 provider/sink；三个生产复盘函数体都不再用 hook/banner
  作为整批前置门。

静态合同：

```bash
bash tools/compliance/lib/check_br194_review_dependency.sh
```

预期：exit 0，并精确输出
`BR-194 review dependency static contract: PASS`。脚本以 brace-aware 方式至少抽取
`run_review_only`、`attempt_post_session_review` 和
`run_strict_review_only_inner` 三个 production caller **以及唯一中央
`dispatch_post_session_review` dispatcher**，并抽取
`push_counted_source_only_with_binding`、`v14_gate_counted_source_only_binding`、
`v14_gate_prepared` 和 `current_counted_source_only_governance_ctx` 的函数体；
不能只检查外围 caller。
具体必须全部断言：

1. 三个 caller 和中央 dispatcher 的签名/函数体都不把 `BannerCtx`、
   `current_banner()` 或 `evaluate_account_mode_hook(true)` 作为 review 前置门；
2. dispatcher 先执行 `review_preflight`，再按闭集 dependency mapping 分区；
3. `--test --review` 的 R-04/R-09 在 provider 前 Disabled；
4. source-only phase 只含 R-04/R-09，先完成；首版随后只构造 R-03/R-08/A-10/A-01
   的固定 typed account failure，不得调用这些任务 dispatcher/provider/sink；
5. R-02/R-05/R-06 保持静态 Disabled；不得存在一个把 source-only 与保守任务混在
   一起的 `tokio::join!`；
6. outcome 只能走唯一 merge，按 `ReviewTask` 稳定排序并显式拒绝重复；
7. R-04 outcome/loader 已无 banner 参数，且只调用专用
   `push_counted_source_only_with_binding → v14_gate_counted_source_only_binding →
   deliver_counted_binding`；专用 gate 不得调用 `current_governance_ctx`、
   `current_source_fact_governance_ctx` 或读取 `LATEST_BANNER`；
8. generic `push_counted_with_binding/v14_gate_counted_binding` 仍保留 CombinedAccount
   语义；专用 entry 必须按
   `is_counted_kind → launch_gate_check(kind) → v14_gate_counted_source_only_binding →
   deliver_counted_binding` 排序；专用 gate 的 allowlist、immutable binding 校验、
   real DataMode、quiet-hour、daily-limit、LaunchGate、analytics audit 与唯一 durable
   entry 不得被移除或后置；
9. replay early-return mode 在普通 database/provider/sink/scheduler 初始化前；其
   runtime 函数签名无真实 sink，复用 production validate/prepare/terminal
   classification；coordinator 是唯一 attempt evidence owner，精确 schema、identity、
   水位、Started/Completed outbox、Passed 条件与固定 authority manifest 均存在；
   checker 必须锁定 replay 两表 composite attempt+decision FK、两条 audit FK、
   两条 named authority INSERT trigger、四条 named UPDATE/DELETE immutable trigger，
   并锁定两个 `enqueue_audit` 调用的 attempt 参数都是字面量 `None`；
10. process 测试使用新的顺序断言，所有列出的 exact test 由 nonzero wrapper 证明
   存在且只执行 1 条。

checker 自己必须有 mutation harness：把真实源码复制到临时 fixture 后，分别注入
`current_banner()`、`evaluate_account_mode_hook(true)`、在中央 dispatcher 的
source-only phase 前调用 `dispatch_r03_industry_chain_outcome`、把 R-04/R-09 与
保守任务放入同一 `tokio::join!`、让 R-04 恢复 generic counted gate、让专用 gate
读取 `LATEST_BANNER`/复用 source-fact context、删除 allowlist/binding 校验/L5/
`deliver_counted_binding` 任一环、删除/恒真化/后置 `launch_gate_check(kind)`、
删除 replay early-return 顺序/固定 manifest/唯一 coordinator persistence/
production terminal classification/Started 或 Completed audit/任一前后水位字段、
把任一 replay audit 的 `None` 改为 replay attempt/删改 `"NONE"`、改变 audit 或
attempt identity domain/字段顺序/删掉 `review_task`/改用 clock、rowid、random 或
CLI ordinal、移除 `BEGIN IMMEDIATE` 或 `MAX+1`、删除/弱化 composite FK 或任一
audit FK/authority INSERT trigger、删除/弱化四个 immutable trigger 任一项、
允许重复或 mismatched completion、把 classification 移到 start audit ack 前、
忽略 completion write/append/ack 失败、删除 test 双禁用、删除重复拒绝；
每个 mutant 都必须令 checker exit 1，未变异
fixture 才 exit 0。它不得全文件禁止常驻 monitor 启动治理所需的合法
`evaluate_account_mode_hook(true)` 调用。

全仓 gate：

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

Gate D 另需总覆盖率 ≥80%、本链 ≥95% 和独立审查 0 blocking。
`tools/compliance/check.sh` 必须实际调用
`tools/compliance/lib/check_br194_review_dependency.sh`，不能只把脚本留成手工命令。

### 8.2 真实交易日时间窗证据

已有历史日志只证明 R-04 曾在真实交易日 21:00 后完成：

```text
$ rg -n '"kind":"R-04".*"success":true' data/dispatcher_log/2026-07-23.jsonl data/dispatcher_log/2026-07-24.jsonl
data/dispatcher_log/2026-07-24.jsonl:372:{"ts":"2026-07-24T22:33:28.336","kind":"R-04","success":true,"snapshot_size":4,"error":""}
data/dispatcher_log/2026-07-24.jsonl:377:{"ts":"2026-07-24T22:59:07.759","kind":"R-04","success":true,"snapshot_size":4,"error":""}
data/dispatcher_log/2026-07-24.jsonl:382:{"ts":"2026-07-24T23:26:14.147","kind":"R-04","success":true,"snapshot_size":4,"error":""}
data/dispatcher_log/2026-07-24.jsonl:387:{"ts":"2026-07-24T23:57:55.361","kind":"R-04","success":true,"snapshot_size":4,"error":""}
data/dispatcher_log/2026-07-23.jsonl:1073:{"ts":"2026-07-23T21:00:27.905","kind":"R-04","success":true,"snapshot_size":5,"error":""}
```

它不是 BR-194 新链发布证据。当前无 R-09 真实生产完成日志，禁止声称已有。

Gate D 必须在同一真实 A 股交易日采集：

1. 15:35 前 R-09 ExpectedWait 且 provider/sink=0。
2. 15:35 后在首版账户能力固定不可用时，R-09 source-only ready、真实双批次、durable
   terminal、`review_provider_top_n_v1` push log 和同 identity
   `push.delivery.audit`。
3. 21:00 前 R-04 ExpectedWait 且 provider/sink=0。
4. 21:00 后在首版账户能力固定不可用时，R-04 source-only ready、完整 Gateway batch、
   durable terminal、`review_lhb_v1` push log 和同 identity audit。
5. 保守账户门任务精确产生 `acquire_batch/account_metrics_incomplete/retryable=true`，
   `source_provider/source_time/evidence_identity_hash=null`，且 account provider /
   task provider / sink 均为 0。
6. 同轮 durable hydration 已把 R-04/R-09 收敛为 terminal；随后对每个任务显式执行
   一次 4.4 的 audited terminal replay，真实 provider/sink 为 0，attempt evidence
   的前后 sink-result 与 delivery-audit-ref 水位完全相等。

证据必须 join business date、task identity、durable occurrence/decision、push-log hash
和 delivery-audit identity；只看“推送成功”日志不算完成。

### 8.3 Gate D 精确 join、最终 hydration 与 audited terminal replay

Gate B 必须实现独立 replay command 和只读 verifier。普通 scheduler 的最终状态只
用于证明 terminal/hydration，不再被描述或计数为“两次 scheduler attempt”。真实
交易日先完成正常 R-04/R-09 delivery 与同轮 hydration，再分别执行一次显式 replay：

```bash
DATE=2026-07-30
cargo run --locked --bin monitor -- \
  --br194-audited-terminal-replay --business-date "$DATE" --task R-09
cargo run --locked --bin monitor -- \
  --br194-audited-terminal-replay --business-date "$DATE" --task R-04
python3 tools/release/verify_br194_review_join.py \
  --business-date "$DATE" --task R-09 --require-passed-replay 1
python3 tools/release/verify_br194_review_join.py \
  --business-date "$DATE" --task R-04 --require-passed-replay 1
```

每个 replay command 只有在 Started/Completed durable audit 都 Appended 且 Passed
充要条件成立时 exit 0，并精确输出：

```text
BR194_REPLAY task=<R-04|R-09> state=Passed attempts=1 provider_calls=0 resume_calls=0 sink_calls=0 delivery_audit_appends=0 sink_watermark_equal=true delivery_audit_watermark_equal=true
```

R-09 verifier 必须精确输出并 exit 0：

```text
BR194_JOIN task=R-09 producer_batches=2 task_bindings=1 durable_terminal=1 sink_receipts=1 push_logs=1 delivery_audits=1 joined_identities=1 hydration_state=Applied replay_passed=1 replay_provider_calls=0 replay_resume_calls=0 replay_sink_delta=0 replay_delivery_audit_delta=0 replay_audits=2
```

R-04 verifier 必须精确输出并 exit 0：

```text
BR194_JOIN task=R-04 producer_batches=1 task_bindings=1 durable_terminal=1 sink_receipts=1 push_logs=1 delivery_audits=1 joined_identities=1 hydration_state=Applied replay_passed=1 replay_provider_calls=0 replay_resume_calls=0 replay_sink_delta=0 replay_delivery_audit_delta=0 replay_audits=2
```

当前 hash/receipt/join 实现的可重放 code-fact 原始输出：

```text
$ nl -ba src/bin/monitor/notify.rs | sed -n '2960,3030p;3044,3107p;3353,3360p' | rg 'stock_analysis\.counted_(sink_result|receipt|decision_identity|attempt_identity|push_log_artifact)\.v1|let decision_identity_hash|let attempt_identity_hash|let pending_artifact_sha256|let event_id =|decision_identity_hash: decision_identity_hash|attempt_identity_hash: attempt_identity_hash|pending_artifact_sha256: pending_artifact_sha256|delivery_audit_event_id: event_id|counted_join_hash: event|fn sha256_domain|hasher\.update'
  2964         sha256_domain("stock_analysis.counted_sink_result.v1", &result_canonical);
  2967             Ok(value) => sha256_domain("stock_analysis.counted_receipt.v1", &value),
  2982     let decision_identity_hash = sha256_domain(
  2983         "stock_analysis.counted_decision_identity.v1",
  2986     let attempt_identity_hash = sha256_domain(
  2987         "stock_analysis.counted_attempt_identity.v1",
  2997         decision_identity_hash: decision_identity_hash.clone(),
  2998         attempt_identity_hash: attempt_identity_hash.clone(),
  3018     let pending_artifact_sha256 = sha256_domain(
  3019         "stock_analysis.counted_push_log_artifact.v1",
  3058     let event_id = event
  3099         decision_identity_hash: decision_identity_hash.clone(),
  3100         attempt_identity_hash: attempt_identity_hash.clone(),
  3101         pending_artifact_sha256: pending_artifact_sha256.clone(),
  3102         delivery_audit_event_id: event_id.clone(),
  3103         counted_join_hash: event
  3353 fn sha256_domain(domain: &str, payload: &[u8]) -> String {
  3356     hasher.update(domain.as_bytes());
  3357     hasher.update([0]);
  3358     hasher.update(payload);

$ nl -ba src/durable_delivery/model.rs | sed -n '457,459p;473p;491p;507p;562,564p;577p;666,667p'
   457     pub task_identity: String,
   459     pub transition_basis_sha256: String,
   473         let transition_basis_sha256 = sha256_hex(&transition_basis_canonical);
   491     pub schedule_occurrence_identity: String,
   507 struct DecisionIdentityMaterial<'a> {
   562         let source_binding_sha256 = sha256_hex(&source_binding_canonical);
   563         let rendered_content_sha256 = sha256_hex(&rendered_content);
   564         let material = DecisionIdentityMaterial {
   577         let decision_identity = sha256_hex(&serde_json::to_vec(&material)?);
   666                 || sha256_hex(&binding.transition_basis_canonical)
   667                     != binding.transition_basis_sha256

$ nl -ba src/bin/monitor/push_templates.rs | sed -n '6028,6034p;8910,8916p'
  6028	struct ProviderTopNReportBinding {
  6032	    review_task_identity: String,
  8911	struct ReviewLhbSourceBinding {
  8915	    review_task_identity: String,

$ nl -ba src/event/envelope.rs | sed -n '16p;488,513p'
    16	pub const COUNTED_DELIVERY_JOIN_HASH_DOMAIN: &str = "stock_analysis.counted_delivery_join.v1";
   488	pub(crate) fn counted_delivery_join_hash(
   498	    let mut hasher = Sha256::new();
   499	    hasher.update(COUNTED_DELIVERY_JOIN_HASH_DOMAIN.as_bytes());
   500	    for value in [
   501	        kind,
   502	        outcome,
   503	        channel,
   504	        decision_identity_hash,
   505	        attempt_identity_hash,
   506	        artifact_sha256,
   507	        sink_result_sha256,
   508	        receipt_sha256,
   509	    ] {
   510	        hasher.update([0]);
   511	        hasher.update(value.as_bytes());
   512	    }

$ nl -ba src/bin/monitor/notify.rs | sed -n '2951,2968p;3287,3301p'
  2951	    let result_value = authoritative_sink_result_value(&raw_result);
  2952	    let result_canonical = match serde_json::to_vec(&result_value) {
  2963	    let sink_result_sha256 =
  2964	        sha256_domain("stock_analysis.counted_sink_result.v1", &result_canonical);
  2965	    let receipt_sha256 = match &raw_result {
  2966	        AuthoritativeSinkResult::Accepted(receipt) => match serde_json::to_vec(receipt) {
  2967	            Ok(value) => sha256_domain("stock_analysis.counted_receipt.v1", &value),
  3287	fn authoritative_sink_result_value(
  3292	        AuthoritativeSinkResult::Accepted(receipt) => {
  3293	            serde_json::json!({"kind": "Accepted", "receipt": receipt})
  3295	        AuthoritativeSinkResult::Rejected(rejection) => {
  3296	            serde_json::json!({"kind": "Rejected", "rejection": rejection})
  3298	        AuthoritativeSinkResult::Uncertain(uncertainty) => {
  3299	            serde_json::json!({"kind": "Uncertain", "uncertainty": uncertainty})

$ nl -ba src/bin/monitor/notify.rs | sed -n '3194p;3208,3212p;3226,3227p;3233,3234p;3240,3241p;3247,3248p;3254,3255p'
  3194     let audit_record = delivery_audit.verify_exact_counted_event(expected_audit)?;
  3208     if expected_commit.delivery_audit_event_id != expected_audit.id
  3209         || expected_commit.counted_join_hash
  3210             != expected_audit
  3211                 .payload
  3212                 .get("counted_join_hash")
  3226     if audit_record.decision_identity_hash.as_deref()
  3227         != Some(expected_pending.decision_identity_hash.as_str())
  3233     if audit_record.attempt_identity_hash.as_deref()
  3234         != Some(expected_pending.attempt_identity_hash.as_str())
  3240     if audit_record.artifact_sha256.as_deref()
  3241         != Some(expected_commit.pending_artifact_sha256.as_str())
  3247     if audit_record.sink_result_sha256.as_deref()
  3248         != Some(expected_pending.sink_result_sha256.as_str())
  3254     if audit_record.receipt_sha256.as_deref() != Some(expected_pending.receipt_sha256.as_str()) {
  3255         return Err("schema-v3 audit receipt_sha256 does not match pending artifact".to_owned());

$ nl -ba src/durable_delivery/coordinator.rs | sed -n '5321p;5323,5325p;5332,5337p;5931p;5933,5936p;5942,5946p'
  5321         "BR-140-disposition-v1",
  5323             &binding.task_identity,
  5324             &stored.decision_identity,
  5325             source_identity,
  5332         && payload.task_identity == binding.task_identity
  5333         && payload.decision_identity == stored.decision_identity
  5334         && payload.source_identity == source_identity
  5335         && payload.task_disposition == "Accepted"
  5336         && payload.task_binding_sha256.as_str() == task_binding_sha256.as_str()
  5337         && payload.task_binding_sha256 == binding.transition_basis_sha256
  5931             "BR-140-disposition-v1",
  5933                 &binding.task_identity,
  5934                 &envelope.decision_identity,
  5935                 source_identity,
  5936                 task_disposition,
  5942             "task_identity": binding.task_identity,
  5943             "decision_identity": envelope.decision_identity,
  5944             "source_identity": source_identity,
  5945             "task_disposition": task_disposition,
  5946             "task_binding_sha256": binding.transition_basis_sha256,
```

verifier 以 `(business_date, ReviewTask, task_identity)` 为根键；不同领域的 hash
绝不要求“全部相等”。它按当前 canonical/domain 合同做以下**精确映射**：

1. R-04 `ReviewLhbSourceBinding.review_task_identity`、R-09
   `ProviderTopNReportBinding.review_task_identity`、
   `DeliveryEnvelope.schedule_occurrence_identity`、`TaskBinding.task_identity` 与
   BR-140 transition `task_identity` 都与根 `task_identity` 相等；R-04 的
   `ordered_batch_ids` 恰有 1 个，R-09 恰有 2 个且顺序与 source binding 相同。
   batch ID 本身不与 task/decision/audit identity 相等。
2. `source_binding_sha256` 是 source canonical bytes 的普通 SHA-256；
   `rendered_content_sha256` 是 rendered bytes 的普通 SHA-256；
   `transition_basis_sha256` 是 task-binding canonical bytes 的普通 SHA-256。
   verifier 必须分别重算，不允许彼此替代。
3. durable `decision_identity` 按当前 `DecisionIdentityMaterial` canonical bytes
   的普通 SHA-256 重算；counted pending 的 `decision_identity_hash` 与
   `attempt_identity_hash` 分别按
   `stock_analysis.counted_decision_identity.v1` /
   `stock_analysis.counted_attempt_identity.v1` domain 重算。
4. `sink_results` 必须恰有一个 terminal Accepted。sink-result canonical bytes
   精确为 `serde_json::to_vec(json!({"kind":"Accepted","receipt":receipt}))`，以
   `stock_analysis.counted_sink_result.v1` 重算；receipt canonical bytes 则单独为
   `serde_json::to_vec(&TypedReceipt)`，以 `stock_analysis.counted_receipt.v1`
   重算。两者是不同 preimage/hash，分别与 pending/commit/audit 对应字段相等。
5. push-log artifact 读取精确 bytes，以
   `stock_analysis.counted_push_log_artifact.v1` 重算；commit artifact hash 必须
   与 pending 相等。不得把文件路径、mtime 或裸 content SHA 当成该 domain hash。
6. `push.delivery.audit` schema-v3 的 decision/attempt/artifact/sink-result/receipt
   字段逐一等于对应 pending/commit 字段。`counted_join_hash` 的精确 preimage 是
   domain bytes `stock_analysis.counted_delivery_join.v1`，随后依次追加一个 NUL byte
   和八个 UTF-8 字段：`kind`、`outcome`、`channel`、
   `decision_identity_hash`、`attempt_identity_hash`、`artifact_sha256`、
   `sink_result_sha256`、`receipt_sha256`；无尾部额外字段、无 JSON wrapper。
   只有这个 join hash 必须等于 audit event ID 和 commit 的 counted event/join ID；
   audit subject hash 只等于 decision identity hash。
7. BR-140 task-transition identity 另按当前 `BR-140-disposition-v1` stable identity
   重算，并核对 task/decision/source identity、accepted、task-binding hash 和
   disposition；它不与 counted join hash强制相等。
8. 上述每条 R-04/R-09 正常 delivery 记录计数均须精确为输出所列数值；最终
   hydration 必须是 Applied，但它本身不证明第二次执行。verifier 还必须读取
   `review_terminal_replay_attempts` 与
   `review_terminal_replay_completions` 的最新 Passed ordinal，重算
   `BR-194-terminal-replay-attempt-v1` identity、canonical/hash、前后两项水位及
   provider/resume/sink/delivery-audit append 零计数，并与
   `ReviewTerminalReplayStarted/Completed` 两条 Appended durable audit 精确 join。
   replay 前后 sink result 和 delivery-audit-ref count/hash 必须逐项相等；只看到
   最终一个 receipt/audit、Started、Failed 或日志文字均不能满足。

脚本位置固定为
`tools/release/verify_br194_review_join.py`；仓库根必须从脚本真实路径计算：

```python
REPO_ROOT = Path(__file__).resolve().parents[2]
```

这是**运行时 script-location root**，不是 Rust 编译期 manifest root。脚本拒绝
authority 路径 CLI 参数、相关环境覆盖、逃出 root 的 symlink 和非 production
namespace，只读取以下固定 authority：

```text
data/durable_delivery.sqlite3
data/durable_delivery_audit/
data/push_log/
data/event_audit/
```

固定 manifest 还必须把 `review_terminal_replay_attempts` /
`review_terminal_replay_completions` 两张 table、两种 replay audit kind 和 attempt
identity domain 锁定为 4.4.2 的字节值；verifier 不接受表名、audit kind、domain
或 replay authority override。

为避免 SQLite 的只读查询仍创建 `-shm`，脚本先用 read-only/no-follow descriptor
记录生产 DB 与既有 WAL/SHM（若存在）的 `(device,inode,size,sha256)`，只复制
DB+WAL 到 `TemporaryDirectory`，仅对临时副本执行 SQLite 查询；查询需要的 SHM
只能在临时目录生成，生产路径不得以 writable 模式打开。目录 authority 中每个
消费文件同样以 read-only/no-follow 打开，并在运行前后核对身份与内容 hash。生产
authority 在复制或验证期间变化、出现/消失、symlink、无法稳定读取时 exit 1；
脚本不得修复、迁移、checkpoint 或清理生产 SQLite/WAL/SHM、push log 或 audit。

## 9. 回滚

实现回滚：

```bash
git revert <BR-194-implementation-commit>
```

只恢复 caller-wide gate 和旧 R-04 签名，并禁用 replay CLI；新增 replay evidence
table 与两类 immutable audit 作为向前兼容历史保留，不降 schema、不删除行。不得删除
durable 数据、push log、dispatcher log 或 hash-chain audit。回滚后 R-04/R-09 再次
受账户门阻断，是已知安全降级；禁止伪造 banner 恢复。

## 10. Gate A 审查

- [x] 数据流、失败模式、旧模块、回滚明确。
- [x] 2.1/2.2/2.3/2.4/2.5/2.7/2.8/2.10 已映射；2.6/2.9 已说明 N/A。
- [x] 两条 producer→task→durable→sink→audit 链完整。
- [x] 时间窗、真实证据、改名/调用点已列出。
- [x] 未放宽账户任务和 `--test --review`。
- [x] LaunchGate 有命名拒绝测试、静态顺序合同与 mutation。
- [x] Gate D 以 typed audited terminal replay 证明零重发，不从最终 hydration
      推断第二次 scheduler attempt。
- [x] 独立 Gate A 审查完成，blocking objections = 0。

独立 Gate A 复审针对 Git blob
`18c4c04d0967acc26fb8546f460779479f272734` 完整重跑设计证据，结论为
`C0 / I0 / M0`。本勾选只确认 Gate A 设计可实施；Gate B 独立代码复审已发现
schema migration、DB canonical hash authority 等阻塞项，Gate B/C/D 与真实生产
push/audit join 仍保持未完成，不得据此宣告 release-ready。

## 11. 2026-08-01 独立依赖审计修订

本节是 BR-194 的后续窄化修订；与首版任务分类冲突时，以本节为准。审计生产
outcome 的实际调用图后，分类闭集修订为：

| 任务 | 修订后依赖 | 代码事实 |
|---|---|---|
| R-04 | `SourceOnly` | 保持首版合同 |
| R-08 | `SourceOnly` | BR-199 公共事件日历合同，不读取持仓 |
| R-09 | `SourceOnly` | 保持首版合同 |
| A-10 | `SourceOnly` | 只读取复盘业务日的 immutable `ChainIntelligenceBatch` |
| A-01 | `SourceOnly` | 只读取复盘业务日的 virtual observation 与 HistoricalBars Gateway |
| R-03 | `LegacyAccountGate` | 仍调用 `portfolio::get_positions`，本地 projection 不是 verified broker batch |
| R-02/R-05/R-06 | `UnclassifiedConservative` | 保持静态 Disabled，不访问 provider |

中央 dispatcher 必须先执行既有 static preflight，再并行运行
R-04/R-08/R-09/A-10/A-01 的各自 source-only producer。每条路径继续执行自己的
Launch/L5/durable/sink/audit 合同；本修订只移除无关账户失败，不提供数据质量或
投递治理豁免。source phase 完成后，只能为仍 runnable 的 R-03 生成 typed
`account_metrics_incomplete`。任一 source task 的失败不得改写其他任务 outcome。
`--test --review` 必须在 provider 前禁用全部五个 source-only task，确保测试环境
不读取生产来源、不进入真实 sink；测试隔离不得再依赖账户失败间接阻断 provider。

复盘业务日期由 `ReviewRunContext::business_date()` 唯一提供；该方法与历史
`review_date()` 返回同一冻结值，后者仅作为兼容 accessor 保留。A-10/A-01 的
provider request、任务 identity、报告日期必须使用此值，禁止重新读取墙钟。

### 11.1 失败模式与回滚

- A-10/A-01 来源不可用、部分、冲突或过期：返回各自 typed source failure，保留
  重试资格；不得降级成账户失败、空批次或默认值。
- R-03 缺 verified broker batch：保持 typed account dependency failure；不得用
  本地 projection 补证。
- 回滚仅恢复本节涉及的 dependency mapping、dispatcher 分支和测试；不得删除或
  重写 durable、复盘、持仓、行情或投递审计数据，也不得触碰 R-04 runtime 或交割日
  Gateway。

## 12. 2026-08-26 测试环境旁路闭合修订

### 12.1 问题与归因

BR-194 的 `review_preflight(..., is_test=true)` 已在 provider 前禁用全部
`SourceOnly` ReviewTask；但后续新增的 BR-232 预测样本回填、BR-223 大宗交易扫描和
BR-223 A-11 IPO 催化不属于 `ReviewTask`，仍在合并任务 outcome 之前无条件运行。
因此 `monitor --test --review` 虽打印 `provider_calls=0`，仍可能实际访问真实公告、
板块或大宗交易来源并尝试非 counted 推送。该行为违反 AGENTS 2.1/2.5，也使原进程
测试的“全部来源已阻断”结论不完整。

同一批进程测试还把 selection-v2 禁用原因固定为
`board_artifact_unverified`。审计 artifact 重新生成或 activation 材料变化时，合法的
BR-193 reason token 会改变；数据库与测试隔离不应绑定某一个可变的诊断原因。

### 12.2 数据流与接口

`dispatch_post_session_review` 继续只计算一次既有 `is_test` 权威，并把它同时传给
`review_preflight` 与 ReviewTask 外旁路门：

```text
runtime test authority
  -> review_preflight (ReviewTask provider/sink 全禁用)
  -> post-session side-route gate
       test       -> zero-call audit summary -> merge outcomes
       production -> BR-232 backfill -> BR-223 block trade -> BR-223 IPO -> merge outcomes
```

测试分支必须在调用 `backfill_pending_predictions`、读取持仓/自选代码、构造
BlockTradesGateway、调用 `dispatch_ipo_catalyst` 或任何 renderer/durable/sink 之前
返回。它不得伪造 backfill 数量、`pushed=false` 结果或成功 receipt。唯一允许输出是
包含 prediction/provider/renderer/persistence/durable/sink 全部为零的结构化隔离摘要。
生产分支保留现有调用顺序、输入、数据验证、审计和投递语义。

selection-v2 的进程断言只验证禁用日志的稳定结构、非空 reason token 及
`providers=0 database_operations=0 sinks=0 schedulers=0`；具体 reason 枚举与顺序仍由
BR-193 activation 专用测试负责，禁止在无关的数据库隔离测试中冻结墙钟相关原因。

### 12.3 失败模式、验证与回滚

- test authority 无法确认：沿用既有 fail-closed 环境门，不进入生产旁路；不得根据
  `STOCK_LIST`、数据库路径或网络可达性推断测试模式。
- 测试进程出现 `[DataGateway]`、`[BoardDataGateway]`、BR-232 verified 或 BR-223
  pushed 标记：进程隔离测试失败，Gate B 阻塞。
- 生产来源失败：保持原 typed failure、重试与审计语义，本修订不新增 fallback。
- 回滚仅撤销本节、同一 dispatcher 的 test 分支和断言 helper；不修改配置、数据库
  schema、历史行、真实持仓、订单或生产 provider 注册。

验证至少包含精确 `monitor_help_isolation`、`selection_process_bootstrap_isolation`、
monitor 单元测试、全工作区 Clippy 和全量测试。Gate C/D、覆盖率与真实生产证据仍按
仓库总门执行，本修订本身不宣称 Release Ready。
