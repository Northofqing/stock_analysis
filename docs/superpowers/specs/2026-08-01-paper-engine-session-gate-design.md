# BR-201 Paper-engine executable-session gate

Status: Gate A remediation for the seventh fresh independent RED C1/I7/M1 review is documented
below; another independent C0/I0 acceptance is required before implementation. Gate B has not
started. The BR-201 registry row remains strictly spec-only until that review succeeds.

## Problem

The always-running intraday loop invokes the four-rule paper exit engine every 30 seconds even
when A-share orders cannot execute. Before the opening session, during the lunch break, after
close and on closed days the quote source may correctly expose no fresh five-second quote.
Treating that expected market state as an immediately retryable paper-engine failure produces
repeated BR-134 warnings and needless database/provider work. It also permits an unguarded direct
caller, or a call which crosses 11:30/15:00 after its initial check, to persist a paper order
outside continuous trading.

The legacy boolean `calendar::can_trade_now()` is not sufficient authority for this boundary. It
cannot distinguish a verified closed session from an unavailable calendar, reads the host-local
timezone and is backed by the mutable legacy holiday set whose poisoned-lock path fails open. A
paper-order safety gate must keep those states separate and fail closed.

### Reproducible current-code evidence

The problem statement above is a claim about the current working tree, not a claim inherited from
another specification. The exact commands and outputs used for this Gate-A snapshot are below.
`nl` plus explicit ranges is intentionally multiline-aware and shows the caller and callee together.

The current loop constructs the account context eagerly, explicitly admits the BR-151 fallback,
calls the unguarded paper engine, advances debounce only on `Ok`, and sleeps for 30 seconds:

```text
$ rg --no-line-number -A5 -B2 'let risk_context = current_banner_for\("v16\.3 paper decision"\)' src/bin/monitor/main.rs
        let monitor = IntradayMonitor;
        loop {
            let risk_context = current_banner_for("v16.3 paper decision").and_then(|banner| {
                match push_templates::paper_risk_context_from_banner(&banner) {
                    Ok(context) => Some(context),
                    Err(error) => match push_templates::snapshot_paper_risk_context_from_banner(&banner) {
                        Ok(context) => {
                            log::info!("[BR-151] SnapshotPaper 使用用户确认持仓进入虚拟盘引擎");
$ rg --no-line-number -A12 -B4 'and_then\(paper_engine::run_once\)' src/bin/monitor/main.rs
            };
            if should_run_4_iron {
                let result = risk_context
                    .ok_or_else(|| "latest evaluated paper risk context unavailable".to_string())
                    .and_then(paper_engine::run_once);
                match result {
                    Ok(count) => {
                        log::debug!("[paper_engine] 4 铁律批次成功: {} 个退出决定", count);
                        *PAPER_ENGINE_LAST_RUN
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
                    }
                    Err(e) => {
                        log::warn!("[paper_engine][BR-134] 本轮失败，保留立即重试资格: {}", e)
                    }
                }
            } else {
$ rg --no-line-number -m1 'tokio::time::sleep\(tokio::time::Duration::from_secs\(30\)\)\.await' src/bin/monitor/main.rs
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
```

The engine entry is public and accepts only a caller-provided risk context, with no session permit:

```text
$ nl -ba src/trading/paper_engine.rs | sed -n '388,395p'
   388  /// One complete four-iron-rule attempt. The caller may advance its success
   389  /// debounce only when this function returns `Ok`.
   390  pub fn run_once(risk_context: PaperRiskContext) -> Result<usize, String> {
   391      let checks = load_open_positions()?;
   392      let decisions = check_4_iron_rules(&checks)?;
   393      let count = decisions.len();
   394      let mut failures = Vec::new();
   395      for decision in &decisions {
```

The complete bounded multiline-aware API/caller inventory at this snapshot is reproduced below.
It searches production, tests and the repository bench target rather than inferring counts from a
single-line grep. The old entry has exactly one production caller and one public definition;
`emit_sell_signal` has one public definition, one internal production call and four direct test
calls. There is no import/re-export alias. The four actual production
`paper_trade::simulate` calls are the four-rule sell plus the three unrelated BR-134 callers.
Guarded/private/account-provider symbols do not yet exist:

```text
$ rg -n -U '\bpaper_engine\s*::\s*run_once\b|\band_then\s*\(\s*paper_engine\s*::\s*run_once\s*\)' src tests benches --glob '*.rs'
src/bin/monitor/main.rs:6136:                    .and_then(paper_engine::run_once);
$ rg -n '^\s*pub\s+fn\s+run_once\b|^\s*fn\s+run_once\b' src tests benches --glob '*.rs'
src/trading/paper_engine.rs:390:pub fn run_once(risk_context: PaperRiskContext) -> Result<usize, String> {
$ rg -n -U '\bemit_sell_signal\s*\(' src tests benches --glob '*.rs'
src/trading/paper_engine.rs:285:pub fn emit_sell_signal(
src/trading/paper_engine.rs:396:        if let Err(error) = emit_sell_signal(decision, risk_context) {
src/trading/paper_engine.rs:820:        emit_sell_signal(&decisions[0], risk_context).expect("audited paper sell");
src/trading/paper_engine.rs:832:        assert!(emit_sell_signal(&invalid, risk_context).is_err());
src/trading/paper_engine.rs:834:        assert!(emit_sell_signal(&invalid, risk_context).is_err());
src/trading/paper_engine.rs:902:        let error = emit_sell_signal(&decision, context)
$ rg -n -U '^\s*(?:match\s+)?paper_trade\s*::\s*simulate\s*\(' src tests benches --glob '*.rs'
src/trading/paper_engine.rs:327:    match paper_trade::simulate(&signal, effective_price, cash, total, pos_pct) {
src/decision/intraday_monitor.rs:209:                match paper_trade::simulate(
src/decision/intraday_monitor.rs:501:        match paper_trade::simulate(&paper_signal, execution_quote.price, cash, total, pos_pct) {
src/bin/monitor/push_templates.rs:3919:    match paper_trade::simulate(&signal, quote.price, cash, total, pos_pct) {
$ rg -n -U '\b(?:pub\s+use|use)\s+[^;]*(?:run_once|emit_sell_signal|run_once_guarded_v1)[^;]*;' src tests benches --glob '*.rs' || true
# no matches
$ rg -n 'run_once_guarded_v1|Br201PrivateExecutionAuthorityV1|Br201ExitFinalOwnerV1|Br134AccountEvaluationProviderV1|Br134AccountEvaluationBatchV1' src tests benches --glob '*.rs' || true
# no matches
```

These outputs are current-code evidence, not the Gate-B acceptance inventory; Gate B must rerun the
same bounded searches across every Rust target and prove the exact post-migration caller counts below
against the implementation commit.

The current successful paper path publishes directly from the engine result instead of a committed
BR-201 outbox fact:

```text
$ nl -ba src/trading/paper_engine.rs | sed -n '329,342p'
   329              log::info!(
   330                  "[paper_engine] 4 铁律卖出 {}({}) status={} reason={}",
   331                  decision.name,
   332                  decision.code,
   333                  outcome.result.status.as_str(),
   334                  decision.reason
   335              );
   336              for event in paper_trading_events(decision, &outcome, decision_id, order_id, exec_id)? {
   337                  crate::bus::TradingBus::global().publish(event);
   338              }
   339              Ok(())
   340          }
   341          Err(e) => {
   342              log::warn!(
```

The legacy calendar uses host-local time, accepts mutable environment additions and returns
`true` when its holiday lock is poisoned:

```text
$ nl -ba src/calendar.rs | sed -n '8,14p;82,84p;101,113p;219,243p;246,249p;280,283p'
     8  //! 节假日列表从环境变量 `TRADING_HOLIDAYS` 读取（逗号分隔的 YYYYMMDD），
     9  //! 也可通过 `add_holidays` 运行时注入。
    10
    11  use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
    12  use once_cell::sync::Lazy;
    13  use std::collections::{BTreeSet, HashSet};
    14  use std::sync::RwLock;
    82  static HOLIDAYS: Lazy<RwLock<HashSet<NaiveDate>>> = Lazy::new(|| {
    83      let mut set = HashSet::new();
    84      // 仓库内经交易所公告核对的休市日是默认事实源；环境变量只用于追加临时调整。
   101      // 从环境变量加载
   102      if let Ok(raw) = std::env::var("TRADING_HOLIDAYS") {
   103          for s in raw.split(',') {
   104              let s = s.trim();
   105              if s.len() == 8 {
   106                  if let Ok(d) = NaiveDate::parse_from_str(s, "%Y%m%d") {
   107                      set.insert(d);
   108                  }
   109              }
   110          }
   111      }
   112      RwLock::new(set)
   113  });
   219  /// 判断指定日期是否为交易日
   220  pub fn is_trading_day(date: NaiveDate) -> bool {
   221      // 周末
   222      if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
   223          return false;
   224      }
   225      // 节假日
   226      // review #14 修复: RwLock poison 时 .read() 返回 Err, 原 `if let Ok(guard)` 静默
   227      // fall through → 节假日当交易日. 改为显式处理: poison 时按"非节假日"处理
   228      // (保守, 让周末检查继续生效) + log::error 提醒 operator 排查.
   229      match HOLIDAYS.read() {
   230          Ok(guard) => !guard.contains(&date),
   231          Err(e) => {
   232              log::error!(
   233                  "[calendar] HOLIDAYS RwLock poisoned: {} — 当作非节假日处理, 请排查",
   234                  e
   235              );
   236              true
   237          }
   238      }
   239  }
   240
   241  /// 判断今天是否为交易日
   242  pub fn today_is_trading_day() -> bool {
   243      is_trading_day(Local::now().date_naive())
   246  /// 获取当前市场时段
   247  pub fn current_session() -> MarketSession {
   248      let now = Local::now();
   249      let today = now.date_naive();
   280  /// 现在是否可以交易（连续竞价时段）
   281  pub fn can_trade_now() -> bool {
   282      current_session().can_trade()
   283  }
```

The old-module dispositions below also depend on current-code facts outside the defect statement.
Those facts have their own literal evidence rather than relying on prose assertions.

The same eager context currently feeds `IntradayMonitor`, the four-rule engine and the 15:30
evening review; the review is time-gated, but its context acquisition is not lazy today:

```text
$ rg --no-line-number 'match monitor\.tick\(risk_context\)|and_then\(paper_engine::run_once\)|if now\.hour\(\) == 15 && now\.minute\(\) == 30|evening_review\(today, risk_context\)' src/bin/monitor/main.rs
                match monitor.tick(risk_context) {
                    .and_then(paper_engine::run_once);
            if now.hour() == 15 && now.minute() == 30 {
                        if let Err(e) = evening_review(today, risk_context) {
```

The checked-in verified trading-day API is already immutable, fail-closed outside coverage and
independent of legacy runtime holiday overrides:

```text
$ nl -ba src/calendar.rs | sed -n '186,204p;393,414p'
   186  /// Fail-closed, immutable A-share trading-day authority for audited replay.
   187  ///
   188  /// Unlike [`is_trading_day`], this API never reads runtime environment
   189  /// overrides and rejects dates outside the checked-in exchange-calendar year.
   190  pub fn verified_a_share_trading_day(date: NaiveDate) -> Result<bool, String> {
   191      let calendar = VERIFIED_TRADING_CALENDAR
   192          .as_ref()
   193          .map_err(std::clone::Clone::clone)?;
   194      if date.year() != calendar.coverage_year {
   195          return Err(format!(
   196              "checked-in A-share trading-calendar coverage unavailable for {}",
   197              date.year()
   198          ));
   199      }
   200      Ok(
   201          !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
   202              && !calendar.closures.contains(&date),
   203      )
   204  }
   393      #[test]
   394      fn br194_verified_calendar_is_immutable_fail_closed_and_coverage_bounded() {
   395          let trading_day = NaiveDate::from_ymd_opt(2026, 7, 30).unwrap();
   396          let exchange_holiday = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
   397          let weekend = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
   398          assert_eq!(verified_a_share_trading_day(trading_day), Ok(true));
   399          assert_eq!(verified_a_share_trading_day(exchange_holiday), Ok(false));
   400          assert_eq!(verified_a_share_trading_day(weekend), Ok(false));
   401          assert!(
   402              verified_a_share_trading_day(NaiveDate::from_ymd_opt(2027, 1, 4).unwrap()).is_err()
   403          );
   404
   405          add_holidays(&[trading_day]);
   406          assert!(
   407              !is_trading_day(trading_day),
   408              "legacy runtime calendar accepts dynamic overrides"
   409          );
   410          assert_eq!(
   411              verified_a_share_trading_day(trading_day),
   412              Ok(true),
   413              "audited replay authority must ignore runtime overrides"
   414          );
```

The actual artifact and its SSE provenance are not the placeholder `repo://calendar/*.json`
previously used by this proposal:

```text
$ nl -ba src/calendar.rs | sed -n '121,126p'; sed -n '1,4p' config/a_share_market_holidays.csv; shasum -a 256 config/a_share_market_holidays.csv
   121  const VERIFIED_TRADING_CALENDAR_AUTHORITY_ORIGIN: &str =
   122      crate::data_gateway::OFFICIAL_SSE_AUTHORITY_ROOT;
   124  static VERIFIED_TRADING_CALENDAR: Lazy<Result<VerifiedTradingCalendar, String>> = Lazy::new(|| {
   125      parse_verified_trading_calendar(include_str!("../config/a_share_market_holidays.csv"))
   126  });
# A-share exchange-wide weekday closures used by the freshness gate.
# year=2026
# source=https://www.sse.com.cn/disclosure/announcement/general/c/c_20251222_10802507.shtml
2026-01-01
ef9044635e9fc7475efcc1972961fd5306a9cbb28e052e91997f132e6da413d5  config/a_share_market_holidays.csv
```

The existing paper-trade write and BR-086 order-attempt audit already share one SQLite transaction
and return the immutable audit receipt:

```text
$ nl -ba src/trading/paper_trade.rs | sed -n '641,649p;661,684p'
   641  fn persist_paper_trade_with_audit(
   642      conn: &mut diesel::sqlite::SqliteConnection,
   643      sql: &str,
   644      signal: &PaperSignal,
   645      result: &PaperResult,
   646      observed_at: &str,
   647  ) -> diesel::QueryResult<(usize, Option<PaperTradePersistenceReceipt>)> {
   648      conn.transaction::<_, diesel::result::Error, _>(|conn| {
   649          let rows = diesel::sql_query(sql).execute(conn)?;
   661          let audit = crate::database::order_audit::OrderAuditRecord {
   662              business_order_id: &signal.plan_id,
   663              source: "PaperTrade",
   664              decision_basis: &signal.virtual_reason,
   665              side: signal.direction.as_str(),
   666              code: &signal.code,
   667              requested_price: signal.price,
   668              execution_price: if rows > 0 { result.fill_price } else { None },
   669              quantity: i64::from(signal.quantity),
   670              quote_observed_at: Some(observed_at),
   671              outcome,
   672              failure_reason,
   673          };
   674          let receipt =
   675              crate::database::order_audit::insert_order_audit_with_receipt_query(conn, &audit)?;
   676          let terminal_receipt = (rows > 0).then(|| PaperTradePersistenceReceipt {
   677              plan_id: signal.plan_id.clone(),
   678              order_audit_id: receipt.order_audit_id,
   679              audit_previous_hash: receipt.previous_hash,
   680              audit_record_hash: receipt.record_hash,
   681              terminal_at: receipt.created_at,
   682          });
   683          Ok((rows, terminal_receipt))
   684      })
```

The current runtime explicitly separates synchronous durable delivery audit from the
observation-only bus/JSONL consumer:

```text
$ rg --no-line-number 'delivery audit mode=synchronous_durable|JsonlWriter::spawn|event_bus\.jsonl.*mode=enabled' src/bin/monitor/main.rs
    log::info!("[event_bus] delivery audit mode=synchronous_durable; bus=observation_only");
        match stock_analysis::event::JsonlWriter::spawn(
        "[event_bus.jsonl] mode=enabled retention_days=1827 isolated_test={}",
```

The existing immutable audit dispatcher uses a kernel lock across validation, append, flush and
`sync_all`; BR-201 may reuse its projection mechanics but not claim that filesystem append is part
of the new SQLite authority transaction:

```text
$ nl -ba src/event/dispatcher.rs | sed -n '431,443p;455,468p'
   431              FileExt::lock_exclusive(&lock_file)
   432                  .map_err(|error| format!("lock audit {lock_name}: {error}"))?;
   433              capability.validate_complete_chain()?;
   434              revalidate_audit_leaf(capability, OsStr::new(&lock_name), &lock_identity)?;
   435
   436              // The kernel lock spans full-chain validation, append and fsync.
   437              // Revalidate on every append because another monitor process may
   438              // have extended the chain since this dispatcher last wrote.
   439              let json_name = format!("{year}.jsonl");
   440              let path = self.base_dir.join(&json_name);
   441              let (mut file, json_identity) =
   442                  open_or_create_audit_file(capability, OsStr::new(&json_name), true)?;
   443              let previous_hash = validate_existing_chain_file(&file, &path)?
   455              let mut line = serde_json::to_vec(&record)
   456                  .map_err(|error| format!("serialize audit line: {error}"))?;
   457              line.push(b'\n');
   458
   459              file.write_all(&line)
   460                  .map_err(|error| format!("append {}: {error}", path.display()))?;
   461              file.flush()
   462                  .map_err(|error| format!("flush {}: {error}", path.display()))?;
   463              file.sync_all()
   464                  .map_err(|error| format!("sync {}: {error}", path.display()))?;
   465              capability
   466                  .root()
   467                  .sync_all()
   468                  .map_err(|error| format!("sync audit root {}: {error}", self.base_dir.display()))?;
```

The BR-154 branch is currently inside paper-engine position loading and isolates stale realtime
quotes after local hour 15 rather than authorizing a substitute exit price:

```text
$ nl -ba src/trading/paper_engine.rs | sed -n '179,196p'
   179          let avg_cost = total_cost / f64::from(quantity);
   180          if !avg_cost.is_finite() || avg_cost <= 0.0 {
   181              return Err(format!(
   182                  "paper position {code} average cost invalid: {avg_cost}"
   183              ));
   184          }
   185          let quote = match crate::broker::execution_quote(&code) {
   186              Ok(quote) => quote,
   187              Err(realtime_error) if chrono::Local::now().hour() >= 15 => {
   188                  log::warn!(
   189                      "[BR-154] paper position {code} isolated after-close: realtime={realtime_error}; SettledDaily capability_unavailable: settled daily PaperTrade capability_unavailable"
   190                  );
   191                  continue;
   192              }
   193              Err(error) => {
   194                  return Err(format!("paper position {code} quote unavailable: {error}"));
   195              }
   196          };
```

The separate post-session review scheduler currently retains its own 60-second/19:00 gate and calls
closing valuation only after its review window and after-close eligibility both pass:

```text
$ rg --no-line-number -m1 'async fn post_session_review_scheduler' src/bin/monitor/main.rs
async fn post_session_review_scheduler(selection_v2_enabled: bool) {
$ rg --no-line-number -m1 'threshold=19:00 interval=60s' src/bin/monitor/main.rs
    log::info!("[复盘调度][BR-139] started threshold=19:00 interval=60s");
$ rg --no-line-number -m1 '        if !post_session_review_window_open\(' src/bin/monitor/main.rs
        if !post_session_review_window_open(
$ rg --no-line-number -m1 '            && closing_valuation_runtime::eligible_after_close' src/bin/monitor/main.rs
            && closing_valuation_runtime::eligible_after_close(now.fixed_offset())
$ rg --no-line-number -m1 '            match closing_valuation_runtime::run_closing_valuation_once' src/bin/monitor/main.rs
            match closing_valuation_runtime::run_closing_valuation_once(now.date_naive()).await {
```

The current paper path does contain the shared BR-084 validator and the v16.3 risk adapter, but it
reserves the 60-second business-order identity on a separate connection before either validator.
The shared validator also supplies `available_cash` only for buys today. These are current-code
facts to adopt or replace explicitly; their presence is not proof that the future BR-201
transaction owner enforces AGENTS 2.5/2.6:

```text
$ nl -ba src/trading/paper_trade.rs | sed -n '708,744p'
   708      validate_realtime_quote_freshness(signal.quote_observed_at, chrono::Utc::now())?;
   710      let db = DatabaseManager::try_get()
   711          .ok_or_else(|| "BR-086 paper-order audit database is not initialized".to_string())?;
   712      if !db
   713          .reserve_business_order_id(&signal.plan_id)
   714          .map_err(|error| format!("BR-086 paper-order idempotency reservation: {error}"))?
   715      {
   736      // v16.3 R1+R2: pre-trade gate 4 项硬检查 (拒 → 不入 paper_trades, 不调 evaluate)
   737      if let Err(reason) = crate::trading::risk_adapter::pre_trade_check(
   738          signal,
   739          quote_price,
   740          current_cash,
   741          total_value,
   742          current_position_pct,
   743      ) {
$ nl -ba src/trading/risk_adapter.rs | sed -n '76,90p'
    76      use crate::trading::order_safety::{OrderSafetyInput, SafetySide};
    78      crate::trading::order_safety::validate(&OrderSafetyInput {
    79          code: &signal.code,
    80          side: match signal.direction {
    81              Direction::Buy => SafetySide::Buy,
    82              Direction::Sell => SafetySide::Sell,
    83          },
    84          order_price: signal.price,
    85          quantity: signal.quantity as u64,
    86          available_cash: (signal.direction == Direction::Buy).then_some(current_cash),
    87          limit_down_price: signal.limit_down_price,
    88          limit_up_price: signal.limit_up_price,
    89          secondary_confirmed: signal.secondary_confirmed,
    90      })?;
$ nl -ba src/trading/order_safety.rs | sed -n '24,40p;54,85p'
    24  pub fn validate(input: &OrderSafetyInput<'_>) -> Result<(), String> {
    25      crate::risk::env_guard::validate_symbol_for_current_env(input.code)?;
    33      if input.quantity == 0 || !input.quantity.is_multiple_of(100) {
    40      let lower = input
    54      if input.order_price < lower || input.order_price > upper {
    61      let notional = input.order_price * input.quantity as f64;
    62      if !notional.is_finite() || notional > MAX_SINGLE_ORDER_RMB {
    68      if input.side == SafetySide::Buy {
    80      if notional >= SECONDARY_CONFIRM_RMB && !input.secondary_confirmed {
$ nl -ba src/database/order_audit.rs | sed -n '303,330p'
   303  impl DatabaseManager {
   304      /// Atomically reserve a business order ID in shared persistence.
   308      pub fn reserve_business_order_id(&self, business_order_id: &str) -> Result<bool, String> {
   312          let mut conn = self
   315          let rows = diesel::sql_query(
   316              "INSERT INTO order_idempotency (business_order_id, reserved_at)
   317               VALUES (?, CURRENT_TIMESTAMP)
   318               ON CONFLICT(business_order_id) DO UPDATE SET reserved_at = CURRENT_TIMESTAMP
   319               WHERE order_idempotency.reserved_at <= datetime('now', '-60 seconds')",
```

The current `portfolio_state` is a local cross-read plus caller-price calculation and returns only
a tuple; it does not carry one provider batch identity or source capture time for cash, totals and
the selected position:

```text
$ nl -ba src/trading/paper_trade.rs | sed -n '590,617p'
   590  pub fn portfolio_state(code: &str, quote_price: f64) -> Result<(f64, f64, f64), String> {
   591      if !quote_price.is_finite() || quote_price <= 0.0 {
   597      let db = DatabaseManager::try_get().ok_or_else(|| "DB 未初始化".to_string())?;
   601      let ledger = diesel::sql_query(
   602          "SELECT date, total_value, cash, market_value, created_at FROM ledger ORDER BY date DESC LIMIT 1",
   608      let (positions, position_source_time) = crate::portfolio::get_positions_with_source_time()?;
   616      let pos_pct = position_pct(&positions, code, quote_price, ledger.total_value);
   617      Ok((ledger.cash, ledger.total_value, pos_pct))
```

The repository has no production account adapter which can emit the proposed complete same-response
account batch. This is an audited capability gap, not an invitation to adapt a local projection:

```text
$ rg -n 'Br134AccountEvaluationBatchV1|Br134AccountEvaluationProviderV1' src --glob '*.rs' || true
# no matches
$ nl -ba src/broker.rs | sed -n '1,6p;58,69p'
     1  //! Realtime quote integration used by decision and paper-trading paths.
     2  //!
     3  //! The previous module registered logging-only broker implementations and a
     4  //! `MockQuoteProvider` returning zero. That made an unavailable data source look
     5  //! healthy and encouraged callers to substitute cost/push prices. This module
     6  //! keeps one fail-closed, evidence-preserving Magic provider Gateway seam.
    58  /// Synchronous quote boundary. Failures and missing data are explicit.
    59  pub trait QuoteProvider: Send + Sync {
    60      fn get_execution_quote(&self, code: &str) -> Result<ExecutionQuote, String>;
    61  }
$ nl -ba src/bin/monitor/main.rs | sed -n '1645,1652p'
  1645      // User-confirmed snapshots are display-only account facts until a real
  1646      // broker is connected. Keep `account_metrics_complete` false so risk
  1647      // gates remain conservative, but do not label known values as missing.
$ nl -ba src/bin/import_real_account_snapshot.rs | sed -n '1,3p;23,29p'
     1  //! One-shot BR-103 importer for an ignored, user-attested evidence manifest.
    23      }
    24      let json = std::fs::read_to_string(&args.evidence)?;
    25      let input = account_snapshot_input_from_json(&json)?;
    26      DatabaseManager::init(Some(args.database))?;
    27      let receipt = save_account_snapshot(&input)?;
```

`broker::QuoteProvider` owns public execution quotes only. `real_account_snapshot` is a one-shot
user-attested image manifest and has no per-position response, provider batch identity or live
broker acquisition seam; `user_account_summary`, `user_position_snapshot`, `SnapshotPaper` and
ledger/portfolio rows are explicitly non-authoritative. Therefore no existing module is named as
the production owner of `Br134AccountEvaluationBatchV1` in this design.

These outputs establish only the current defect and old-module relation. Every API and table named
`TO BE BUILT` below remains a proposal until its Gate-B command and expected output pass.

## Decision

### Typed Asia/Shanghai session authority

A new calendar API (TO BE BUILT) is the sole authority for a four-rule paper exit:

```rust
fn paper_executable_session_at(
    observed_at: DateTime<FixedOffset>,
) -> Result<PaperSessionEvidence, PaperSessionAuthorityError>;
```

Production obtains `observed_at` from UTC and converts it to the exact `+08:00`
Asia/Shanghai offset before the call; the host timezone must not affect classification. The API
accepts only `+08:00`, validates the checked-in official exchange calendar and rejects an invalid
authority, poisoned state or date outside its declared coverage. The authority binding is not one
ambiguous URI. `PaperSessionEvidence` carries the exact provider notice URI
`source_authority_uri=https://www.sse.com.cn/disclosure/announcement/general/c/c_20251222_10802507.shtml`,
the checked-in artifact URI `artifact_uri=repo://config/a_share_market_holidays.csv`, declared
coverage version, and SHA-256 of the artifact's exact raw bytes before parsing. The current
checked-in bytes have
`artifact_raw_bytes_sha256=ef9044635e9fc7475efcc1972961fd5306a9cbb28e052e91997f132e6da413d5`.
The evidence also carries the Shanghai observation time, market date, classified `MarketSession`
and exact half-open continuous-session window. Gate B recomputes the raw-byte hash over
`include_bytes!("../config/a_share_market_holidays.csv")`, verifies that the CSV's one `# source=`
line equals `source_authority_uri`, validates it through the existing canonical SSE URL validator,
and only then parses dates. A source URL cannot masquerade as a repository artifact, a parsed-date
hash cannot replace the raw-byte hash, and missing fields are not defaulted.

Only `Morning [09:30:00, 11:30:00)` and `Afternoon [13:00:00, 15:00:00)` evidence can be converted
inside `calendar` into a `ContinuousTradingPermit` with private fields. `Closed`, `Auction`, the
09:25-09:30 gap, `LunchBreak` and `AfterHours` are verified non-executable results. Calendar
unavailable/corrupt/out-of-coverage is a distinct typed error. Both cases retain paper-engine
eligibility and write exactly one append-only session-decision audit through the dedicated
`Br201PaperExitStore` audit façade. They perform zero account, paper-ledger/open-position database,
quote, decision, idempotency-reservation, order/order-audit, outbox, bus/sink or debounce calls.
Thus `session_audit_db_calls=1`, `paper_ledger_db_calls=0` and `order_db_calls=0` are distinct
observable facts, not the contradictory shorthand “zero paper DB”. The authority-error case
remains operationally visible and is never converted to `Closed` or `false`.

### Gate order and lazy context acquisition

Each 30-second loop captures one Shanghai observation instant and applies this deterministic
read/observe-then-decide order:

1. observe the typed session authority in memory; this observation is not a decision record and
   has no separately persisted trace schema;
2. if the observation is executable, read and acknowledge the persistent success-only five-minute
   debounce from the same pinned BR-201 SQLite authority; non-executable and authority-error
   observations do not read debounce;
3. only after all reads needed by the branch are complete, select exactly one initial scheduler
   decision: `SkippedNonExecutable` or `RejectedAuthority` from step 1,
   `DeferredDebounce` from an executable/not-due step 2, or `Admission/Admitted` from an
   executable/due step 2;
4. atomically append and acknowledge that one initial decision record. A tick may not append a
   speculative pre-debounce record and later relabel or append a second decision. A later
   Admission attempt `Terminal` is an attempt outcome record, not another scheduler decision;
5. invoke exactly one public production entry, `paper_engine::run_once_guarded_v1`, which accepts
   only that tick's typed `observed_at_shanghai`; it immediately delegates to the child-private
   `execute_paper_exit_tick_v1` and exposes no permit, provider handle, store or Admission seam;
6. that one private tick operation owns session observation, the executable-only debounce read,
   selection and durable acknowledgement of exactly one scheduler decision, and the return for
   skip/reject/defer. For Admission it atomically inserts and reads back Admission/open/fence/
   projection-intent and mints a non-`Clone`, child-private
   `Br201AdmittedAttemptAuthorityV1` bound to those exact bytes;
7. the Admission transaction is already durable before any paper ledger, account-context,
   provider or order work. Only the same still-live private tick operation may consume its admitted
   authority, resolve the process-wide register-once provider and call it lazily after revalidating
   the permit;
8. load the paper ledger/open positions, acquire real quotes, make exit decisions and attempt
   paper orders under the existing BR-134 rules;
9. for every proposed paper order, enforce AGENTS 2.5/2.6 plus BR-084/BR-086 at the sealed
   transaction owner as specified below; and
10. advance the persistent debounce exactly once in the successful Terminal transaction,
    including a verified empty batch.

The initial-decision uniqueness key is `(namespace,process_boot_identity_sha256,
scheduler_tick_ordinal)` and is UNIQUE. A transaction retry reuses the same key and canonical
bytes. A conflicting second value is corruption and fails startup. `DeferredDebounce` and
`Admission/Admitted` are invalid unless the same transaction proves the acknowledged debounce row
version read in step 2; `SkippedNonExecutable` and `RejectedAuthority` require a null debounce
version. This makes the forbidden order—decision append followed by debounce read—machine
detectable.

The private tick call graph is frozen as
`monitor -> run_once_guarded_v1 -> execute_paper_exit_tick_v1 ->
Br201ExitFinalOwnerV1::authorize_order_v1`. There is no second scheduler, session-audit,
debounce, Admission, provider-resolution or final-owner entry. A transaction retry stays inside the
same private tick and reuses the exact decision key and canonical bytes. A process restart may
reconcile an acknowledged open attempt through the separately fenced reconciler, but it may not
reconstruct `Br201AdmittedAttemptAuthorityV1` or resume account/provider/order work.

`Br134AccountEvaluationBatchV1` is the only account input accepted by this path. Its exact fields
are `schema_version,batch_identity_sha256,provider_source_id,source_authority_uri,
source_captured_at_shanghai,locally_observed_at_shanghai,available_cash_fen,total_assets_fen,
total_position_market_value_fen,total_position_bps,daily_pnl_fen,
consecutive_stop_loss_count,positions`. Every position has exact fields
`instrument_identity_sha256,source_position_identity_sha256,quantity,available_quantity,
market_value_fen,position_bps`. Monetary values are integer fen, ratios are integer basis points,
counts are nonnegative integers, hashes are lowercase 64-hex, and the position array is strictly
BINARY sorted by `instrument_identity_sha256`; a unique but noncanonical/out-of-order array is a
separate typed failure from a duplicate identity. All aggregate and per-position fields must come from one
provider response carrying the same immutable `batch_identity_sha256`; joining independently
captured cash, totals or positions is forbidden. `source_captured_at_shanghai` is provider source
time; both timestamps use exact `YYYY-MM-DDTHH:MM:SS.nnnnnnnnn+08:00`, must be present,
parseable and offset-exact. Source time must not be after `locally_observed_at_shanghai` or the owner
sample; local observation must not be after the owner sample. Source time must be no more than
30 seconds old at both context admission and the final order transaction. `available_quantity <=
quantity`, finite/equivalent scaled values are positive where used, total position value and basis
points must exactly reconcile to the per-position set under the declared integer-rounding rule,
and every proposed exit must join exactly one position identity from this same batch. Zero matches
and multiple matches are distinct typed failures. Unsupported `schema_version`, a malformed local
observation timestamp, a local observation after the owner sample, and a negative
`consecutive_stop_loss_count` are also distinct typed failures after the already acknowledged
initial `Admission/Admitted`, but before paper-ledger access, proposal creation or any order
authorization. They produce the attempt's sole `FailedAccountContext` Terminal joined to that
Admission; none may be reclassified as a pre-Admission scheduler decision. The Terminal
audit retains `account_batch_identity_sha256`, `account_provider_source_id`,
`account_source_authority_uri` and `account_source_captured_at_shanghai`; it never records raw
account or instrument identity.

There is currently no admissible production provider for that type. Gate B may introduce only the
minimal seam `Br134AccountEvaluationProviderV1::capture_account_evaluation() ->
Result<Br134AccountEvaluationBatchV1, Br134AccountEvaluationErrorV1>` and one process-wide,
register-once production adapter owner. The adapter must call one real broker/account endpoint (or
one upstream response explicitly documented by that provider as atomic) and convert that single
response without secondary reads. It must preserve provider/source authority, source capture time,
local observation time, immutable batch identity, cash, assets, position aggregate, daily PnL,
consecutive stop-loss count and every position. Registration of zero or more than one production
owner, an adapter assembled from multiple responses, or any local/snapshot/test adapter in the
production namespace is `CapabilityDisabled`. The provider registration object and trait object
remain child-private; neither monitor nor `run_once_guarded_v1` accepts or returns them. Startup
preflight resolves exactly one owner and installs it in the child-private register once. The private
tick merely obtains a borrow after its acknowledged Admission; obtaining the borrow is not a
provider call, and `capture_account_evaluation` is the first account/provider boundary call.

Until a separately reviewed upstream change supplies and production-bootstrap-registers that
owner, BR-201 is disabled before session construction even when the enable switch is exactly `1`.
The exact one-time startup banner is:

```text
[BR-201] paper_exit mode=disabled reason=br134_account_evaluation_provider_unavailable required_provider=Br134AccountEvaluationProviderV1 session_calls=0 account_calls=0 paper_ledger_db_calls=0 provider_calls=0 order_db_calls=0 outbox_calls=0 push_log_calls=0
```

No local projection, `SnapshotPaper`, fixture, simulated account, `real_account_snapshot`,
`user_account_summary`, `user_position_snapshot` or manually imported file may suppress this
banner or make the positive canary executable.

#### Exact integer fen and basis-point reconciliation

`Br134AccountEvaluationBatchV1` never accepts binary floating point. Each provider money input is
a canonical signed base-10 CNY string matching
`-?(0|[1-9][0-9]*)(\.[0-9]{1,4})?`; plus signs, exponent notation, commas, leading zeroes, negative
zero and more than four fractional CNY digits are rejected. Conversion first parses an `i128`
integer at scale 10,000 sub-CNY units, then rounds to integer fen (100 fen/CNY) by
round-half-to-even. Ties choose an even absolute fen for both signs: `1.0050 -> 100`,
`1.0150 -> 102`, `-1.0050 -> -100`, `-1.0150 -> -102`. Cash, assets and market values must be
nonnegative; daily PnL is signed. Every stored fen value must fit `i64`; all products and sums use
checked `i128`, and parse, multiply, add, cast or absolute-value overflow is a typed rejection.

`total_assets_fen` must be positive and exactly equal
`available_cash_fen + total_position_market_value_fen`; the latter must exactly equal the checked
sum of BINARY-unique position `market_value_fen`. Empty positions require both position totals to
be zero. For a non-empty set, every quantity is positive, every available quantity is nonnegative
with `available_quantity <= quantity`, and every market value is positive.

Basis points use denominator `total_assets_fen` and scale 10,000. Compute aggregate
`B = round_half_to_even(total_position_market_value_fen * 10_000 / total_assets_fen)` with exact
integer quotient/remainder comparison; require `0 <= B <= 10_000`. For each BINARY-sorted position
compute `q_i=floor(market_value_fen_i * 10_000 / total_assets_fen)` and nonnegative remainder
`r_i`. Let `delta=B-sum(q_i)`. Checked arithmetic must prove `0 <= delta <= positions.len()`;
otherwise reject. Add one basis point to exactly the first `delta` positions ordered by
`r_i DESC, instrument_identity_sha256 BINARY ASC`; all others retain `q_i`. The stored
`position_bps` values must equal that allocation exactly and sum to `total_position_bps=B`.
Because all position numerators are nonnegative and floor is used, a negative residual is
impossible and is rejected rather than redistributed.

Golden arithmetic vectors are frozen: assets/cash/position values `(300,100,[100,100])` yield
aggregate `6667` and, for equal remainders with identities `aa.. < bb..`, `[3334,3333]`;
`(600,400,[100,100])` yields aggregate `3333` and `[1667,1666]`; `(100,100,[])` yields zero/empty.
Mutation tests reject opposite tie direction, half-away-from-zero, signed asymmetry, float input,
wrong scale, a one-fen aggregate mismatch, duplicate identity, alternate residual recipient,
negative/oversized residual and every `i64`/`i128` overflow edge.

The legacy `paper_trade::portfolio_state(code, quote_price)` result is explicitly rejected for
BR-201. It derives cash/total/position state from local projections plus a caller price and has no
complete provider batch/capture-time contract. `BannerCtx`, `user_account_summary`,
`user_position_snapshot`, BR-151 SnapshotPaper and any independently refreshed per-position row
also cannot be promoted into `Br134AccountEvaluationBatchV1`. Missing source time, an unknown
provider, a zero/negative total, an absent position, a 30-second freshness violation or any
cross-batch/reconciliation mismatch produces `FailedAccountContext` before paper-ledger/provider/
order access. Every such rejection uses the one-to-one closed reason mapping in the status registry
below; a parser, arithmetic or reconciliation error may not be collapsed into
`account_context_partial` or a free-form diagnostic.

The debounce authority is the singleton `paper_exit_success_debounce` row in the same physical
namespace, with exact fields `namespace,version,last_success_terminal_sha256,
success_committed_at_shanghai,next_due_at_shanghai`. For an executable observation, a due check
reads and acknowledges this row before selecting `DeferredDebounce` or `Admission`; the selected
initial decision binds that exact row version. Only a `Succeeded` or `SucceededEmpty`
Terminal transaction may CAS its version and set `next_due_at_shanghai =
success_committed_at_shanghai + 300 seconds`; failure, quarantine, skip and rejected authority never
write it. Missing/corrupt state, a backwards wall clock, version conflict or commit ambiguity fails
closed. Restart and a second process therefore observe the same five-minute boundary;
process-local `PAPER_ENGINE_LAST_RUN`/`Instant` is diagnostic only and cannot authorize a due tick.

The debounce genesis is deterministic and migration-owned. A fresh physical namespace is valid
only when the BR-201 migration creates exactly one row in the same `BEGIN IMMEDIATE` transaction as
the table and singleton constraints, with exact values `namespace=<attested namespace>`,
`version=1`, `last_success_terminal_sha256=NULL`, `success_committed_at_shanghai=NULL` and
`next_due_at_shanghai=NULL`. The three nullable fields are all-null exactly at genesis and all-
non-null after the first successful Terminal; every partial combination is corruption. Genesis is
immediately due on the first executable tick, and that Admission binds observed version `1`; no
wall-clock value, epoch sentinel or fabricated Terminal hash is seeded. The first successful
Terminal CASes the exact genesis tuple to version `2` and fills all three fields atomically.

Migration is idempotent but not repairing: under the pinned descriptor and process-owner lock it
requires either an entirely absent table in a namespace with zero BR-201 v1 audit/open/order facts,
or the byte-exact valid singleton above/already-advanced singleton. It may create the former once;
it must read back the row and schema before acknowledgement. A missing row after table creation,
multiple rows, version zero/overflow, a non-null genesis field, an all-null row at version other than
one, existing BR-201 facts without a debounce row, or a row from another namespace fails startup.
Restart never reseeds or rewinds it. Containment and rollback retain this row and its decoder, so a
deep revert cannot manufacture a new first-due state.

The current eager, loop-wide `risk_context` must not be reused by the four-rule exit. BR-151
`SnapshotPaper`, `closing_valuation`, `user_account_summary` and `user_position_snapshot` are
absolutely forbidden from authorizing or constructing this exit context, including before open,
after close and when the complete BR-134 batch is missing. A missing, stale, partial, cross-batch
or unidentifiable account evaluation returns a typed `AccountContextUnavailable` before the paper
ledger, open positions, quote provider or order owner is called. It is never replaced by
`AccountMode::Normal`, `DataMode::Full`, a snapshot, a valuation or a local database timestamp.

Other consumers retain their registered business semantics, but the current code evidence shows
that `IntradayMonitor`, the four-rule engine and 15:30 `evening_review` share one eager context.
Gate B must split their acquisition paths: the evening-review trigger remains exactly 15:30 and
acquires its own context only after that branch is due; `IntradayMonitor` retains its own tick
contract; the separate 19:00 post-session/closing-valuation scheduler retains its existing gates.
A closed-session four-rule tick therefore cannot read any account/valuation projection, the paper
ledger or a quote, and cannot emit the repeated BR-134 context-unavailable/error pair.

This is a narrow BR-201 supersession of two BR-134 clauses for the four-rule exit only. The
four-rule scheduler passes the permit and lazy provider handle instead of eagerly constructing and
passing `PaperRiskContext`; every other intraday, post-session and paper-trade caller retains
BR-134's explicit context contract. Likewise, BR-134's attempt-all rule means that every exit
decision proposed while the permit is valid receives one ordered BR-086 outcome audit. If the
permit expires before a later order's atomic authorization, fund/session safety supersedes further
side effects: the owner still writes that proposal's exact
`Rejected/permit_expired_before_atomic_authorization` BR-086 audit, with reservation/order/outbox/
delivery affected counts all zero, then audits all remaining already-proposed ordinals the same way
unless an audit write itself fails. It must not acquire a new quote or create a new proposal after
expiry. Expiry before decisions exist has no fabricated order audit. This scoped exception does not
weaken BR-134 for any other caller and does not permit a proposed exit to disappear unaudited.

### Deep enforcement and time-of-check/time-of-use closure

Gate B adds the frozen versioned production entry (TO BE BUILT)
`paper_engine::run_once_guarded_v1(observed_at_shanghai: DateTime<FixedOffset>) ->
Result<PaperExitAttemptResultV1, PaperExitAttemptErrorV1>`. It rejects any offset other than exact
`+08:00` and delegates immediately to the one child-private
`execute_paper_exit_tick_v1(observed_at_shanghai)` operation. That operation owns the complete
session -> debounce -> initial decision -> durable Admission -> lazy provider -> validation ->
final-owner order flow above. No caller can construct a permit, select a provider, name an
Admission receipt or reach a BR-201 store/order operation. `paper_engine` declares the private child
module `mod br201_exit_owner;` at
`src/trading/paper_engine/br201_exit_owner.rs` (TO BE BUILT). That single child module owns
`Br201AdmittedAttemptAuthorityV1`, `Br201PrivateExecutionAuthorityV1`,
`Br201ExitFinalOwnerV1`, `Br201PaperExitStore`, all BR-201
schema migrations and the final `BEGIN IMMEDIATE` alias/reservation/BR-086/order/outbox/delivery
transaction. The authority is non-`Clone` and non-serializable; its fields and constructor are
private even to the parent. The child exposes to its parent only one `pub(super)` high-level
`execute_paper_exit_tick_v1` operation. It exposes no session/debounce/Admission sub-operation,
provider registration/trait object, connection, transaction callback, insert,
reserve, commit, authority constructor or side-effect method to a sibling module or crate caller.

Only the acknowledged Admission transaction may construct
`Br201AdmittedAttemptAuthorityV1`; only `Br201ExitFinalOwnerV1` may consume a borrow of it and
construct the private execution authority, after revalidating the private
`ContinuousTradingPermit`, account batch, quote and the exact Admission/open attempt inside the
pinned transaction. Selection by "latest", attempt identity supplied by a caller, or lookup without
the admitted record hash/open-row/fence tuple is forbidden. The
child-private side-effect function requires `&Br201PrivateExecutionAuthorityV1`; neither
`PaperSignal`, a reason string, a public enum, a trait object nor any public constructor can satisfy
that parameter. The authority is consumed by one accepted/rejected transaction outcome and cannot
escape, be cached or cross a process boundary. `src/database/mod.rs` may continue to provide its
generic descriptor-attested connection primitives, but owns no BR-201 migration, low-level write or
final transaction API. This module allocation is part of the guarded-v1 migration and is not
described as unchanged.

The existing public
`paper_engine::run_once(PaperRiskContext) -> Result<usize,String>` remains source-compatible and is
marked deprecated, but its body becomes a fail-closed shim returning exact error
`BR-201 legacy paper_engine::run_once disabled; use run_once_guarded_v1` before clock, session,
account, paper-ledger, quote, database, order, outbox, bus or sink access. It can never delegate to
the guarded entry. The current public `emit_sell_signal(&SellDecision,PaperRiskContext)` receives
the same source-compatible deprecated zero-call shim treatment; its former side-effecting body
moves behind the private permit/store owner. No public re-export, feature flag, test hook, trait
object or legacy `simulate` overload may reach that body.

The complete current production caller inventory is frozen as four actual
`paper_trade::simulate` calls: the four-rule sell at `src/trading/paper_engine.rs:327`, intraday buy
at `src/decision/intraday_monitor.rs:209`, 15:30 evening-review buy at
`src/decision/intraday_monitor.rs:501`, and D-01 virtual buy at
`src/bin/monitor/push_templates.rs:3919`. Gate B removes the first call completely when its former
body moves behind the private authority, while preserving the other three unrelated BR-134 callers
and their existing semantics. It also freezes multiline-aware searches for
`paper_engine::run_once`, imported/aliased `run_once`, `emit_sell_signal`, `paper_trade::simulate`,
`PaperSignal` construction, imports/re-exports, traits and function pointers across `src/`, tests and
binaries. The production loop migrates to `run_once_guarded_v1`; internal BR-201 tests use
TEST_CODE-only guarded fixtures; deliberate old-API compatibility tests are the only old calls.
Static acceptance requires exactly one production guarded caller, exactly three unrelated
production `paper_trade::simulate` callers, zero BR-201 production `simulate` callers, zero
production calls or aliases to either legacy shim, no public/re-exported private authority, and zero
callable BR-201 side-effecting symbol lacking that private parameter. Static visibility checks must
also prove that only `paper_engine` names the child module's sole `pub(super)` high-level operation
and that no sibling can name `Br201ExitFinalOwnerV1`, `Br201PrivateExecutionAuthorityV1`, its store,
connection or mutation methods. Public `PaperSignal` remains a generic BR-134 input and is
explicitly never BR-201 execution authority.

The permit binds one exact half-open session window. A caller-side check is diagnostic only and
cannot authorize a write. Permit validation is passed down to the transaction owner that
atomically commits a paper order and its durable TradingBus outbox/event fact. The transaction
owner obtains a fresh Asia/Shanghai instant at the conditional insert use-site, after all
preparation. The paper engine therefore revalidates:

- before any paper-ledger/open-position or quote-provider acquisition; and
- inside the paper-order transaction owner at the final conditional order-plus-outbox insert.

The transaction owner must not expose a public method that accepts a caller-computed boolean or a
prevalidated timestamp. It may perform a preliminary check before preparation, but its final
mutation is one conditional use-site operation that binds the owner-sampled Shanghai instant, the
permit window, the paper-order row and a `PaperExitEventOutbox` row in the same database
transaction. The same instant is stored as both the order fact's `occurred_at` and the outbox
fact's `authorized_at`; the outbox also stores the immutable order-fact identity and BR-201 attempt
identity. A condition miss, an affected-row count other than exactly one for either insert, a
duplicate identity, or commit ambiguity rejects/quarantines the attempt. There is no public
“validated=true”, caller timestamp, direct TradingBus publish or check-then-write seam.

External TradingBus/sink workers are projection-only owners. They may deliver after 11:30/15:00
only from an already committed, hash-verified outbox fact whose `authorized_at` is inside its bound
permit window and whose order-fact join is exact. They cannot accept a permit, resample a trading
session, synthesize a new paper decision, or publish from an in-memory engine result. Delivery time
is separately audited and never replaces `authorized_at`; a post-boundary external send therefore
projects a pre-boundary durable fact instead of creating a post-boundary paper fact.

Failure injection must advance the clock after caller/engine validation but before the transaction
owner, and again after the owner's preliminary revalidation/transaction preparation but before the
conditional order-plus-outbox mutation. The mutation's final use-site sample must catch both and
commit neither row. There is deliberately no injectable gap between that final sample and the
conditional predicate/insert that consumes it. A separate failpoint advances the clock immediately
after the atomic use-site insert but before transaction commit: commit remains valid because both
facts are already bound to the same pre-boundary `authorized_at`. Another test advances the clock
after commit but before external delivery and proves that the worker emits only the committed
outbox projection, without a second paper fact or a new authorization timestamp. This explicitly
covers crossing both before and after the final sample without reintroducing a direct-send race.

Crossing 11:30 or 15:00 before the atomic authorization insert invalidates the permit. The guarded
entry returns the current and remaining already-proposed decisions as explicit typed non-executed
failures. Each receives its ordered rejection-only BR-086 audit transaction, but no post-boundary
reservation, paper order, outbox authorization or delivery fact is created, and debounce is not
advanced. An order-plus-outbox pair atomically authorized before the boundary
remains immutable and may be projected later; it must not be rolled back or disguised as a whole-
batch success. BR-134's attempt-all audit coverage continues for the frozen proposal set; BR-201
safety supersedes side-effecting order attempts and new proposal/provider work after permit expiry.

### Order-safety and namespace enforcement

The permit is necessary but never sufficient to authorize an order. Every BR-201 proposed exit
must preserve the complete AGENTS 2.5/2.6 and BR-084 contract. Before any business-order identity
reservation or order/order-audit write, the sealed owner must:

1. call `risk::env_guard::validate_symbol_for_current_env` through the shared order-safety façade;
   production rejects `TEST_CODE*`, tests reject real symbols, and the database/audit/outbox roots
   have already been attested to the same physical namespace;
2. validate a finite positive order price, positive 100-share lot quantity, finite positive source
   limit-down/limit-up bounds and an order price inside those exact bounds;
3. validate AGENTS 2.3 before using any quote/price series. `PaperExitMarketValidationV1` binds the
   current and immediately preceding valid provider observations, quote batch/source identities,
   source capture times, time-continuity and split/dividend consistency results. A finite positive
   current price is mandatory. An adjacent valid-value change whose absolute value exceeds 20%
   requires an independently audited `AdjacentChangeManualConfirmationV1` capability bound to both
   observation hashes, exact instrument, quote batch, approver identity hash, approval time and
   expiry. This capability is distinct from the RMB 500,000 notional confirmation and neither can
   satisfy the other. `NotApplicable` is valid only with a provider-bound
   `AdjacentChangeNotApplicableProofV1` whose closed reason is `first_provider_observation` or
   `no_prior_trading_session` and whose raw source evidence proves no adjacent valid value exists;
   a missing previous value, new-listing guess or local cache miss is not proof. Gaps, duplicates or
   unresolved split/dividend jumps reject the data. The BR-086 order-attempt audit stores the
   validation record hash, the exact `Within20Percent`/`ManualConfirmed`/`NotApplicable` state and
   the distinct confirmation/proof hash before the order can commit;
4. validate a finite order notional no greater than both the same-batch available-cash evidence
   and RMB 1,000,000. For BR-201 sells the transaction owner must pass `Some(available_cash)` and
   apply this repository red line even though the current `order_safety::validate` implementation
   only checks cash for buys; Gate B must close that shared-validator gap rather than bypass it;
5. require a typed, independently audited secondary-confirmation capability for notional greater
   than or equal to RMB 500,000; the current hard-coded `secondary_confirmed=false` is retained as
   a fail-closed rejection, never promoted by the session permit; and
6. run the existing account-mode, DataMode, single-position and cash-floor checks in
   `risk_adapter::pre_trade_check` against the same immutable BR-134 account batch. A session
   permit cannot override `Frozen`, `Unsafe`, a position limit or the cash floor.

### BR-134 data-admission alignment (AGENTS 2.3)

BR-134 does not classify a stock as good or bad from a fixed percentage. A finite positive
provider observation whose adjacent valid-value change has absolute magnitude greater than 20%
enters the typed `ManualConfirmationRequired` data-admission state; it is not unconditionally
rejected and is not a stock-quality filter. Until confirmation/proof, the current data admission
fails closed with `manual_confirmation_required`; once satisfied, the magnitude alone cannot reject
the stock. It may advance only with the exact `AdjacentChangeManualConfirmationV1` capability above. The only N/A
route is the provider-bound `AdjacentChangeNotApplicableProofV1` above; a guessed new listing,
missing cache row or locally inferred market rule is insufficient. When the two observations are
daily closes, the capability must be the complete BR-171 receipt including its daily-K and
lifecycle evidence; the realtime quote-pair capability here cannot substitute for it. BR-092 still
forbids filling a missing amount/price or other required provider field. This preserves AGENTS 2.3
while removing the legacy fixed-20% whole-batch rejection semantics.

BR-134 quote freshness is a separate two-boundary invariant. `PaperExitQuoteEvidenceV1` is an
immutable typed struct with exact ordered fields
`schema_version,instrument,quote_identity_sha256,provider_source_id,source_authority_uri,
provider_batch_identity_sha256,source_captured_at_shanghai,price`. Its identity binds the exact raw
provider quote bytes and all listed provenance fields. The owner samples Shanghai time twice:

1. **pre-side-effect boundary**: after quote acquisition and before any business-order-ID
   reservation, BR-086 order-attempt audit, paper-order, event-outbox or delivery mutation;
2. **conditional-use boundary**: inside the final `BEGIN IMMEDIATE`, immediately before the
   conditional reservation/BR-086/order/outbox/delivery inserts.

At each boundary, nanosecond arithmetic must prove
`0 <= sampled_at_shanghai - source_captured_at_shanghai <= 5_000_000_000ns`. A future capture
(`age < 0`) or age strictly greater than five seconds rejects before the corresponding mutations;
age exactly five seconds is accepted. The second check must use the byte-identical
`quote_identity_sha256`, instrument, provider source, source authority, provider batch, source
capture and price accepted by the first check; refetching or substituting a newer quote is a new
decision and cannot repair the old one. The transaction stores both sampled instants and binds
`quote_terminal_checked_at_shanghai` to the same owner sample used as the conditional
`authorized_at`/`occurred_at`. Any identity/provenance/capture/terminal-sample mismatch makes every
reservation/BR-086/order/outbox/delivery affected-row count zero. Admission/session-audit records
that precede quote acquisition remain authoritative attempt evidence, not quote authorization.

The 60-second business-order identity is deliberately **not** derived from the boot-scoped BR-201
attempt identity or scheduler ordinal. `PaperExitBusinessIntentKeyV1` has exact ordered fields
`schema_version,namespace,market_date,paper_position_fact_sha256,trigger_rule_id,direction,
quantity`; it excludes boot, attempt, quote, provider batch and wall-clock fields. Its
`v1_business_order_id` is lowercase SHA-256 over
`stock_analysis.br201.paper_exit_business_intent.v1\0` plus the canonical compact JSON bytes.
The immutable paper-position fact changes only when the position ledger changes, so the same
position/rule/direction/quantity decision keeps the same ID across a process restart while a
genuinely changed position becomes a new business intent.

This is an explicit identity migration, not an unchanged identity. Every historical
`business_order_id` byte string already present in `order_idempotency`, `paper_trades` or BR-086 is
opaque authority: migration reads and binds those exact bytes and never regenerates, normalizes or
silently repairs them from code/date/reason. Classification may prove that a byte string belongs to
the four-rule legacy namespace, but a regenerated candidate can never replace the stored authority.

New compatibility aliases use the legacy UTF-8 algorithm
`exit-{code}-{YYYYMMDD}-{reason.replace(' ','_').chars().take(16)}` only through the immutable
`LegacyPaperExitCompatibilityDescriptorV1`. Its exact fields are
`schema_version,source_release_id,source_commit,legacy_algorithm_id,
legacy_host_timezone_iana,legacy_host_timezone_evidence_sha256,legacy_timestamp_grammar,
trigger_reason_projection_sha256,descriptor_sha256`. `legacy_algorithm_id` is exactly
`host_local_yyyymmdd_reason_space_underscore_utf8_chars16_v0`; the timezone must be a canonical IANA
zone proven by the signed deployed-release/runtime evidence that produced the legacy rows. The
owner converts its sampled instant through that persisted zone before formatting `YYYYMMDD`; it
does not use current process `TZ`, host `Local`, Shanghai by assumption, or the V1 market date. If
the old deployment timezone, algorithm, code bytes or reason bytes cannot be proven exactly, the
history is `business_order_identity_history_unresolved`, capability stays Disabled and no alias is
guessed. The descriptor is created/read back under the same locked migration and retained across
restart, timezone changes, containment and rollback.

The legacy alias input is byte authority, never a projection recomputed after restart or upgrade.
`LegacyPaperExitAliasInputV1` has exact ordered fields
`schema_version,namespace,paper_position_fact_sha256,trigger_rule_id,
legacy_code_bytes,legacy_code_bytes_sha256,legacy_reason_bytes,
legacy_reason_bytes_sha256,compatibility_descriptor_sha256`. Both byte fields are SQLite BLOBs;
the code must be the byte-identical code carried by the joined immutable legacy paper-position
fact, and its hash uses domain `stock_analysis.br201.legacy_code_bytes.v1\0`. The trigger-to-reason
projection is the exact closed table below; reason hash uses domain
`stock_analysis.br201.legacy_reason_bytes.v1\0`. The descriptor's
`trigger_reason_projection_sha256` signs the compact ordered array of these exact UTF-8 byte
strings. An unclassified trigger, invalid UTF-8, different code bytes for the same position fact,
or any hash/descriptor mismatch is `business_order_identity_history_unresolved`.

| `trigger_rule_id` | exact `legacy_reason_bytes` UTF-8 string |
| --- | --- |
| `IronRule1StopLoss` | `铁律1:止损(-8%)` |
| `IronRule3FiveDayTakeProfit` | `铁律3:跌破5日线止盈` |
| `IronRule4FourteenDayRotate` | `铁律4:14天不涨换股` |
| `IronRule5BollMacdDivergence` | `铁律5:布林上轨+MACD顶背离` |
| `AtrDynamicStopLoss` | `ATR动态止损` |

The signed projection's exact compact UTF-8 bytes are:

```text
[{"trigger_rule_id":"IronRule1StopLoss","legacy_reason_utf8":"铁律1:止损(-8%)"},{"trigger_rule_id":"IronRule3FiveDayTakeProfit","legacy_reason_utf8":"铁律3:跌破5日线止盈"},{"trigger_rule_id":"IronRule4FourteenDayRotate","legacy_reason_utf8":"铁律4:14天不涨换股"},{"trigger_rule_id":"IronRule5BollMacdDivergence","legacy_reason_utf8":"铁律5:布林上轨+MACD顶背离"},{"trigger_rule_id":"AtrDynamicStopLoss","legacy_reason_utf8":"ATR动态止损"}]
```

There is no trailing newline. `trigger_reason_projection_sha256` is lowercase SHA-256 of exact
domain `stock_analysis.br201.trigger_reason_projection.v1\0` followed by those bytes and is exactly
`1b44379d7a719ad30c915ddf131997e3934a691051fe2faa30a81a97ad05fcd0`.

`paper_exit_legacy_alias_inputs` persists that complete record before alias construction with PK
`(namespace,paper_position_fact_sha256,trigger_rule_id)` and UNIQUE constraints on each complete
record hash and `(namespace,legacy_code_bytes_sha256,legacy_reason_bytes_sha256,
compatibility_descriptor_sha256)`. New aliases insert and read back the input, both hashes and both
exact BLOBs in the same `BEGIN IMMEDIATE` migration/admission transaction; an existing row is read
and used byte-for-byte and is never regenerated from current extractor code. Only then may the
owner apply exact v0 operations: decode both BLOBs as UTF-8, replace ASCII byte `0x20` in reason by
`0x5f`, take the first 16 Unicode scalar values without normalization, convert the owner instant
through the descriptor's IANA zone, and encode
`exit-{code}-{YYYYMMDD}-{transformed_reason}`. The raw code/reason BLOBs and plaintext alias remain
inside the private descriptor-attested SQLite authority; session audit, JSONL, logs and PR evidence
carry only their domain hashes. Mutation tests cover every trigger/reason substitution, code/reason
byte change, Unicode-byte-vs-scalar truncation, normalization, ASCII-space replacement, timezone,
restart/upgrade recomputation, and descriptor/hash mismatch.

Legacy `order_idempotency.reserved_at` accepts only exact SQLite UTC bytes
`YYYY-MM-DD HH:MM:SS`, interprets them as UTC with no offset/fraction/host-local conversion, and
converts that value to an instant before comparison. A legacy reservation is unexpired exactly when
`owner_sampled_instant < parsed_utc_instant + 60 seconds`; equality is expired, subject to all
confirmed/unresolved-history guards. Malformed, future, overflowed or ambiguous bytes are unresolved
and never treated as expired. This freezes the current SQLite `CURRENT_TIMESTAMP` contract while
all new V1 reservation columns remain explicit `+08:00` Shanghai timestamps.

Golden migration vectors include `59.999999999s` rejection and exact `60.000000000s` expiry; a
restart at each boundary; a process `TZ` change before/after restart; a UTC/legacy-zone midnight
where `YYYYMMDD` differs; the same instant before/after rollback; malformed/future legacy
`reserved_at`; and persisted historical IDs whose bytes do not equal a regenerated candidate.
Every vector must retain the stored opaque identity/descriptor and permanent dual guard. Because
truncation can make two V1 intents share one legacy token, such a collision is conservative
contention: neither intent may proceed until the conflicting historical state is resolved through
the Controlled Exception Path. It is never disambiguated by a suffix or clock.

`PaperExitBusinessIdentityBindingV1` has exact ordered fields
`schema_version,namespace,v1_business_order_id,legacy_business_order_id,
paper_position_fact_sha256,trigger_rule_id,direction,quantity,cutover_generation`. Its canonical
hash binds both identities to one logical intent. The same pinned database adds
`paper_exit_business_identity_aliases` with UNIQUE keys `(namespace,identity_scheme,identity_bytes)`
and `(namespace,v1_business_order_id,legacy_business_order_id,cutover_generation)`. The only schemes
are `LegacyPlanIdV0` and `PaperExitBusinessIntentV1`; unknown schemes fail. Every admitted proposal
must claim or read back both alias rows in one `BEGIN IMMEDIATE` transaction before either identity
can authorize a reservation. Two intents mapping to either same token cannot both own it.

The authoritative reservation stores the V1 ID, legacy ID, binding hash, cutover generation,
`reservation_generation`, owner-sampled `reserved_at_shanghai`,
`expires_at_shanghai=reserved+60 seconds` and nullable
`confirmed_order_attempt_record_sha256`. The owner performs a dual read under the same SQLite write
lock: both alias rows; the V1 reservation; the legacy `order_idempotency` row; and every legacy/V1
`paper_trades`, BR-086 and BR-201 order join at their frozen high-water. Either identity having an
unexpired reservation rejects. Either identity having a confirmed BR-086 record, paper order or an
unresolved/ambiguous historical order join rejects regardless of age. A rejection-only audit with
no reservation/order does not become a permanent order, but remains immutable evidence. Missing,
partial or conflicting legacy evidence is `business_order_identity_history_unresolved`, never an
empty history.

A reserved identity is only an idempotency mutex; it is never a BR-086 confirmed attempt record or
proof that an order exists. A confirmed record is the immutable BR-086 row joined by that hash and
may become non-null only in the same transaction as its paper order/outbox/delivery facts. Any
second reservation under either identity before expiry is rejected across crash/restart. After
expiry, reuse requires a versioned CAS of the exact dual reservation/binding tuple and remains
forbidden if either identity joins a confirmed or unresolved order. Backwards clock, generation
overflow or an unknown reservation/alias state fails closed.

Cutover authority is the singleton `paper_exit_business_identity_cutover` with exact fields
`namespace,generation,state,started_at_shanghai,legacy_compatibility_descriptor_sha256,
legacy_reservation_high_water,
legacy_order_high_water,legacy_audit_high_water,alias_backfill_sha256,
drain_not_before_shanghai,completed_at_shanghai`. Its closed states are `LegacyOnly`,
`DualReadV1Write` and `V1PrimaryDualGuard`; state never returns backwards and generation increases by
checked one on every transition. Migration begins in `LegacyOnly`, freezes the three legacy high-
waters plus the verified descriptor, backfills both aliases for every classifiable historical four-rule identity without
rewriting legacy rows, validates the complete order/audit chain, then atomically enters
`DualReadV1Write` with `drain_not_before_shanghai=started_at_shanghai+60 seconds`. Ambiguous,
unclassifiable or colliding history blocks the transition; it is not skipped.

The state eligibility contract is total. `LegacyOnly` is migration-only and never permits a new
BR-201 production order or positive canary. `DualReadV1Write` is canary- and production-eligible
after every ordinary Gate-D prerequisite succeeds; new accepted transactions write the V1
reservation and both immutable aliases while continuing every legacy/V1 read above.
`V1PrimaryDualGuard` is the normal steady state and remains equally production-eligible; it changes
only the primary lookup/write organization and never relaxes the permanent legacy/V1 dual read,
alias write or confirmed/unresolved guard. Transition to
`V1PrimaryDualGuard` requires owner time at or after the drain bound, zero unexpired legacy
reservations at a new locked high-water, complete alias coverage through all three frozen/current
high-waters, zero unresolved historical joins, no open attempt, and an independently recomputed
alias-backfill hash. `V1PrimaryDualGuard` still performs the permanent confirmed/unresolved dual
guard; “V1 primary” never deletes or stops consulting legacy aliases. Thus the 60-second window and
historical order protection survive indefinitely rather than expiring with the migration. The
transition is monotonic `LegacyOnly -> DualReadV1Write -> V1PrimaryDualGuard`; restart and rollback
retain the current state and generation. Successful entry into `V1PrimaryDualGuard` can never itself
disable an otherwise qualified capability.

The identity concurrency and cutover result matrix is closed:

| Locked observation/state | Required result | Permitted writes |
|---|---|---|
| Either legacy or V1 reservation is unexpired | reject `business_order_duplicate_within_60s` | one rejection-only BR-086 audit; no alias/reservation/order/outbox/delivery mutation |
| Either identity joins any confirmed order fact, at any age | reject `business_order_identity_already_confirmed` | one rejection-only BR-086 audit; no order-authorizing mutation |
| Both stored alias rows are complete and internally valid, but either token is already uniquely bound to a different complete V1/legacy pair or binding hash | reject `business_order_identity_alias_conflict`; the conflict is permanent until an append-only Controlled Exception resolution | one rejection-only BR-086 audit; no alias/reservation/order/outbox/delivery mutation and no suffix/clock disambiguation |
| Legacy UTC reservation bytes or compatibility timezone/date descriptor cannot be parsed and proven exactly | reject `business_order_identity_history_unresolved` and keep capability Disabled | diagnostic/rejection evidence only; no inferred expiry/date/alias or order-authorizing mutation |
| Legacy/V1 source history is missing, partial or ambiguous before a complete alias binding can be proven | reject `business_order_identity_history_unresolved` and keep capability closed | diagnostic/rejection evidence only; no guessed alias or order-authorizing mutation |
| No conflicting history in `LegacyOnly` | reject `business_order_identity_cutover_not_ready` | migration/backfill facts only; no new BR-201 order |
| No conflicting history in `DualReadV1Write` | perform the locked dual read, complete every remaining pre-side-effect validation, then let only an accepted transaction claim/read back both aliases and the V1 reservation | zero writes before acceptance; then exact two alias rows plus the frozen six order effects in one transaction |
| No conflicting history in `V1PrimaryDualGuard` | remain production-eligible and perform the same permanent legacy/V1 dual guard before the private owner may authorize | the same atomic alias-plus-six-effect contract; no legacy-only writer or guard removal |
| Two accepted owners race on either token | exactly one UNIQUE/CAS winner; loser reads the winner and rejects deterministically | winner's one atomic alias-plus-six-effect transaction, loser's one rejection-only audit |
| Drain bound or any high-water/coverage/open-attempt proof is incomplete | remain `DualReadV1Write` | no cutover transition |
| Drain bound and every proof are complete under the write lock | checked-one transition to `V1PrimaryDualGuard` | cutover row plus immutable transition audit only |
| Restart, containment or deep rollback | retain current generation/state, compatibility descriptor, aliases, opaque historical identity decoder, legacy UTC decoder and permanent dual guard | no state rewind, alias deletion or legacy-only writer |

These identity reasons are disjoint and precedence is the matrix order. An unexpired reservation is
only `business_order_duplicate_within_60s`; a confirmed fact at any age is only
`business_order_identity_already_confirmed`; a complete-but-differently-owned alias pair is only
`business_order_identity_alias_conflict`; and unparseable, missing, partial or ambiguous source
history that prevents constructing any complete pair is only
`business_order_identity_history_unresolved`. The former unused
`business_order_id_duplicate` byte string is not a v1 enum member; decoding it is an unknown-reason
failure. Mutation tests exercise all four conditions and reject every cross-substitution.

The current standalone `DatabaseManager::reserve_business_order_id` call is rejected for this path
because it opens its own connection and can leave a reservation before validation or without the
atomic order fact. Gate B adds one private on-connection owner for alias, reservation and BR-086
audit. Its immutable `PaperExitOrderAttemptKeyV1` is
`(attempt_identity_sha256,order_ordinal,v1_business_order_id,
business_identity_binding_sha256)` and is UNIQUE; ordinals are assigned once from the BINARY-sorted
proposed-exit set and may not be renumbered after a rejection. Containment, compatibility releases
and deep rollback retain the cutover row, compatibility descriptor, alias table, opaque historical
identity decoder, legacy UTC timestamp decoder and dual reader. Rollback may
move no state backwards and may not restore a legacy-only writer.

The sort is a total, machine-checkable order, not a locale/database/insertion-order promise.
`PaperExitProposedOrderKeyV1` has exact ordered fields
`instrument_identity_sha256,trigger_rule_id,direction,quantity,
paper_position_fact_sha256,v1_business_order_id`. Hash/ID fields are decoded to exactly 32 bytes
and compared unsigned byte-by-byte ascending. `trigger_rule_id` is the closed enum with ordinals
`IronRule1StopLoss=0`, `IronRule3FiveDayTakeProfit=1`, `IronRule4FourteenDayRotate=2`,
`IronRule5BollMacdDivergence=3`, `AtrDynamicStopLoss=4`; any legacy text that cannot prove exactly
one member is `business_order_identity_history_unresolved`, not a new rule. `direction` has the
sole BR-201 value `Sell=0`; quantity is a checked positive `u64` compared numerically ascending.
The comparator applies the six fields in declaration order and never uses collation, Unicode,
host endianness, a database rowid, or sort stability. A byte-equal complete key appearing twice is
Terminal `FailedEngine/proposed_exit_sort_key_duplicate`; otherwise the sorted vector receives
checked one-based nonzero `u32` ordinals once, before validation begins, and rejection cannot
renumber the suffix.

The frozen vector `(11..11,IronRule3FiveDayTakeProfit,Sell,200,aa..aa,bb..bb)`,
`(11..11,IronRule1StopLoss,Sell,900,cc..cc,dd..dd)`,
`(22..22,IronRule1StopLoss,Sell,100,00..00,00..00)` sorts in the exact input-index order
`[1,0,2]` and receives ordinals `[1,2,3]`. Mutation tests independently reject every adjacent field
swap, UTF-8/locale comparison, lexicographic decimal quantity, little-endian quantity, duplicate
complete key, zero/overflow ordinal, and post-rejection renumbering.

Every proposed exit produces exactly one BR-086 record—never “when required.” The owner repeats
symbol/namespace, AGENTS 2.3, the exact BR-134 five-second quote check, BR-084 fields and current
reservation state at the final use site. For any rejection, one `BEGIN IMMEDIATE` transaction
inserts and reads back exactly one `Rejected` BR-086 row with the exact closed reason and all
available non-sensitive validation hashes; it inserts/CASes zero reservation, paper-order, outbox
and delivery rows. This includes duplicate identity, missing confirmation, invalid N/A proof,
stale/cross-batch evidence, future/stale/substituted quote, namespace mismatch and every safety
failure. A field unavailable because the rejection occurred earlier remains JSON `null`; it is
never omitted or fabricated. If the rejection audit cannot commit, the attempt remains open and
no Terminal or later order ordinal may be acknowledged until reconciliation records the exact
failure.

For an accepted order, one `BEGIN IMMEDIATE` transaction requires the exact open attempt owner,
attempt/fence generations and unoccupied attempt key; requires exactly the two byte-equal alias rows,
inserting both together if this is a new intent; inserts or version-CASes exactly one reservation;
constructs the canonical intended `Confirmed` BR-086 preimage in memory, inserts exactly one
`Confirmed` BR-086 record, reads back its immutable hash, updates that reservation's confirmed hash
to the read-back value; and
inserts exactly one paper-order fact, `PaperExitEventOutbox` and initial `Pending` delivery row. A
pre-existing complete alias pair has `alias_insert=0`; a new pair has `alias_insert=2`; any other
alias affected/read-back cardinality is corruption. Required order-effect affected counts are
`reservation=1,br086=1,reservation_confirm=1,order=1,outbox=1,delivery=1`; both aliases and all six
rows are read back and hash-joined before commit acknowledgement. Any zero/multiple order-effect
count, uniqueness conflict, commit ambiguity or read-back mismatch rolls back the aliases and all
six order effects. Thus neither a naked alias/reservation nor a logging-only rejection can survive
as order audit evidence. Before this transaction begins, the intended BR-086 preimage is only a
planned value: it is not an acknowledged audit fact and cannot satisfy any FK/hash predicate. Only
the Confirmed row inserted and read back inside this same accepted transaction may bind the
reservation confirmation and later effects. No legacy `simulate` overload, caller boolean, public `PaperSignal` or
pre-reserved ID can construct `Br201PrivateExecutionAuthorityV1` or skip this owner.

### Durable session-decision audit

Every BR-201 scheduling decision uses one pinned production/test-isolated SQLite authority owned by
the private child-module `Br201PaperExitStore` (TO BE BUILT). The same private child module owns the
final order transaction described above; there is no sibling `database::br201_paper_exit_store`
owner and no exported low-level BR-201 database API. The same database contains
`paper_exit_session_audit`, `paper_exit_open_attempts`, `paper_exit_attempt_fences`, paper-order,
`paper_exit_event_outbox`, `paper_exit_outbox_delivery`, external receipt-audit and
`paper_exit_audit_projection_outbox`, `paper_exit_success_debounce`, cross-boot business-order
reservation, adjacent-change confirmation/proof and BR-086 order-attempt audit tables, so
audit/open-row, order/outbox, delivery/receipt and
audit/projection-intent invariants can use real SQLite transactions. Production and
invocation-unique TEST_CODE roots are physically separate; path/env/CWD override, symlink/hardlink
alias and cross-namespace open are rejected. The connection uses `BEGIN IMMEDIATE`, WAL,
`synchronous=FULL`, foreign keys, pinned descriptor attestation and read-back before
acknowledgement.

#### Canonical SQLite v1 manifest and owner operations

The following is the complete BR-201 SQLite schema authority for Gate B. It is TSV, one logical
row per physical table, and is parsed by AC-14; `NEW` means created only by BR-201 and `REUSED`
means an existing table whose existing columns remain byte-for-byte unchanged. Type tokens are
exact: `I` is SQLite INTEGER; `T` TEXT; `B` BLOB; `NN` NOT NULL; `N` nullable; `SHA` is `T NN`
checked lowercase 64-hex; `TS` is `T NN` checked exact nanosecond Shanghai grammar; and `CANON` is
compact canonical JSON `B NN`. `AO(x)` expands to exactly two `BEFORE UPDATE/DELETE` abort triggers
named `trg_x_no_update` and `trg_x_no_delete`. `ND(x)` expands to exact `BEFORE DELETE` trigger
`trg_x_no_delete`. No table, column, primary/foreign/unique key, index, trigger, default, or owner
operation outside this manifest is part of schema version 1.

<!-- BR201_SQLITE_SCHEMA_V1_BEGIN -->
```text
table	class	columns	primary_key	foreign_keys	unique_keys	indexes	triggers
paper_exit_schema_meta	NEW	singleton I NN CHECK(singleton=1);schema_version I NN CHECK(schema_version=1);schema_manifest_sha256 SHA;installed_at_shanghai TS	(singleton)	-	(schema_manifest_sha256)	-	AO(paper_exit_schema_meta)
paper_exit_process_lock_generation	NEW	namespace T NN;generation I NN CHECK(generation>0);owner_boot_identity_sha256 SHA;lock_descriptor_identity_sha256 SHA;updated_at_shanghai TS	(namespace)	(owner_boot_identity_sha256)->paper_exit_process_boots(process_boot_identity_sha256)	(namespace,generation)	idx_paper_exit_lock_owner(owner_boot_identity_sha256)	ND(paper_exit_process_lock_generation);trg_paper_exit_process_lock_generation_validate_update
paper_exit_process_boots	NEW	process_boot_identity_sha256 SHA;namespace T NN;process_lock_generation I NN CHECK(process_lock_generation>0);started_at_shanghai TS;ended_at_shanghai T N;previous_boot_record_sha256 T N;record_sha256 SHA	(process_boot_identity_sha256)	-	(namespace,process_lock_generation);(record_sha256)	idx_paper_exit_process_boots_namespace_generation(namespace,process_lock_generation)	AO(paper_exit_process_boots)
paper_exit_session_audit	NEW	audit_ordinal I NN CHECK(audit_ordinal>0);namespace T NN;attempt_identity_sha256 SHA;phase T NN;decision_status T NN;reason_code T NN;canonical_record_bytes CANON;record_sha256 SHA;previous_record_sha256 T N;created_at_shanghai TS	(namespace,audit_ordinal)	-	(namespace,record_sha256);(namespace,attempt_identity_sha256,phase)	idx_paper_exit_session_attempt(namespace,attempt_identity_sha256,audit_ordinal);idx_paper_exit_session_tail(namespace,audit_ordinal DESC)	AO(paper_exit_session_audit)
paper_exit_audit_projection_outbox	NEW	projection_identity_sha256 SHA;namespace T NN;audit_ordinal I NN;record_sha256 SHA;canonical_projection_bytes CANON;state T NN;row_version I NN CHECK(row_version>0);claim_owner_boot_identity_sha256 T N;claim_generation I N;claim_process_lock_generation I N;claim_identity_sha256 T N;consumed_proof_identity_sha256 T N;created_at_shanghai TS;updated_at_shanghai TS	(projection_identity_sha256)	(namespace,audit_ordinal)->paper_exit_session_audit(namespace,audit_ordinal);claim_owner_boot_identity_sha256->paper_exit_process_boots(process_boot_identity_sha256)	(namespace,audit_ordinal);(namespace,record_sha256)	idx_paper_exit_projection_state(namespace,state,audit_ordinal)	ND(paper_exit_audit_projection_outbox);trg_paper_exit_audit_projection_validate_update
paper_exit_success_debounce	NEW	namespace T NN;version I NN CHECK(version>0);last_success_terminal_sha256 T N;success_committed_at_shanghai T N;next_due_at_shanghai T N	(namespace)	-	(namespace,version)	-	ND(paper_exit_success_debounce);trg_paper_exit_success_debounce_validate_update
paper_exit_open_attempts	NEW	attempt_identity_sha256 SHA;namespace T NN;admission_audit_ordinal I NN;admitted_record_sha256 SHA;owner_boot_identity_sha256 SHA;owner_process_lock_generation I NN;attempt_generation I NN CHECK(attempt_generation>0);state T NN;terminal_record_sha256 T N;opened_at_shanghai TS;updated_at_shanghai TS	(attempt_identity_sha256)	(namespace,admission_audit_ordinal)->paper_exit_session_audit(namespace,audit_ordinal);owner_boot_identity_sha256->paper_exit_process_boots(process_boot_identity_sha256)	(namespace,admission_audit_ordinal);(namespace,admitted_record_sha256)	idx_paper_exit_open_state(namespace,state,admission_audit_ordinal,attempt_identity_sha256)	ND(paper_exit_open_attempts);trg_paper_exit_open_attempts_validate_update
paper_exit_attempt_fences	NEW	attempt_identity_sha256 SHA;fence_generation I NN CHECK(fence_generation>0);owner_boot_identity_sha256 SHA;owner_process_lock_generation I NN;state T NN;pending_status T N;pending_reason T N;pending_joined_count I N;pending_joined_sha256 T N;pending_reconciliation_evidence_sha256 T N;pending_from_audit_ordinal I N;updated_at_shanghai TS	(attempt_identity_sha256)	(attempt_identity_sha256)->paper_exit_open_attempts(attempt_identity_sha256);owner_boot_identity_sha256->paper_exit_process_boots(process_boot_identity_sha256)	(attempt_identity_sha256,fence_generation)	idx_paper_exit_fence_state(state,attempt_identity_sha256)	ND(paper_exit_attempt_fences);trg_paper_exit_attempt_fences_validate_update
paper_exit_account_acquisition_evidence	NEW	acquisition_evidence_identity_sha256 SHA;namespace T NN;attempt_identity_sha256 SHA;provider_registration_identity_sha256 SHA;provider_call_ordinal I NN CHECK(provider_call_ordinal>0);provider_call_started_at_shanghai TS;provider_call_finished_at_shanghai TS;outcome_kind T NN;response_raw_bytes_sha256 T N;error_raw_bytes_sha256 T N;protected_evidence_locator_sha256 SHA;previous_acquisition_record_sha256 T N;canonical_record_bytes CANON;record_sha256 SHA	(acquisition_evidence_identity_sha256)	(attempt_identity_sha256)->paper_exit_open_attempts(attempt_identity_sha256)	(namespace,attempt_identity_sha256,provider_call_ordinal);(record_sha256)	idx_paper_exit_account_evidence_attempt(attempt_identity_sha256,provider_call_ordinal)	AO(paper_exit_account_acquisition_evidence)
paper_exit_compatibility_descriptors	NEW	namespace T NN;schema_version I NN CHECK(schema_version=1);source_release_id T NN;source_commit T NN;legacy_algorithm_id T NN;legacy_host_timezone_iana T NN;legacy_host_timezone_evidence_sha256 SHA;legacy_timestamp_grammar T NN;trigger_reason_projection_sha256 SHA;canonical_descriptor_bytes CANON;descriptor_sha256 SHA;created_at_shanghai TS	(namespace)	-	(descriptor_sha256)	idx_paper_exit_compatibility_descriptor_hash(descriptor_sha256)	AO(paper_exit_compatibility_descriptors)
paper_exit_legacy_alias_inputs	NEW	namespace T NN;paper_position_fact_sha256 SHA;trigger_rule_id T NN;legacy_code_bytes B NN;legacy_code_bytes_sha256 SHA;legacy_reason_bytes B NN;legacy_reason_bytes_sha256 SHA;compatibility_descriptor_sha256 SHA;canonical_record_sha256 SHA;created_at_shanghai TS	(namespace,paper_position_fact_sha256,trigger_rule_id)	compatibility_descriptor_sha256->paper_exit_compatibility_descriptors(descriptor_sha256)	(canonical_record_sha256);(namespace,legacy_code_bytes_sha256,legacy_reason_bytes_sha256,compatibility_descriptor_sha256)	idx_paper_exit_legacy_alias_input_hashes(namespace,legacy_code_bytes_sha256,legacy_reason_bytes_sha256)	AO(paper_exit_legacy_alias_inputs)
paper_exit_business_identity_cutover	NEW	namespace T NN;generation I NN CHECK(generation>0);state T NN;started_at_shanghai TS;legacy_compatibility_descriptor_sha256 SHA;legacy_reservation_high_water I NN;legacy_order_high_water I NN;legacy_audit_high_water I NN;alias_backfill_sha256 SHA;drain_not_before_shanghai TS;completed_at_shanghai T N	(namespace)	-	(namespace,generation)	idx_paper_exit_cutover_state(state,namespace)	ND(paper_exit_business_identity_cutover);trg_paper_exit_business_identity_cutover_validate_update
paper_exit_business_identity_aliases	NEW	alias_identity_sha256 SHA;namespace T NN;identity_scheme T NN;identity_bytes B NN;v1_business_order_id B NN;legacy_business_order_id B NN;paper_position_fact_sha256 SHA;trigger_rule_id T NN;direction T NN;quantity I NN CHECK(quantity>0);cutover_generation I NN;business_identity_binding_sha256 SHA;legacy_alias_input_record_sha256 SHA;created_at_shanghai TS	(alias_identity_sha256)	(namespace,paper_position_fact_sha256,trigger_rule_id)->paper_exit_legacy_alias_inputs(namespace,paper_position_fact_sha256,trigger_rule_id);namespace->paper_exit_business_identity_cutover(namespace)	(namespace,identity_scheme,identity_bytes);(namespace,v1_business_order_id,legacy_business_order_id,cutover_generation)	idx_paper_exit_alias_v1(namespace,v1_business_order_id);idx_paper_exit_alias_legacy(namespace,legacy_business_order_id)	AO(paper_exit_business_identity_aliases)
paper_exit_business_reservations	NEW	namespace T NN;v1_business_order_id B NN;legacy_business_order_id B NN;business_identity_binding_sha256 SHA;cutover_generation I NN;reservation_generation I NN CHECK(reservation_generation>0);reserved_at_shanghai TS;expires_at_shanghai TS;confirmed_order_attempt_record_sha256 T N;updated_at_shanghai TS	(namespace,v1_business_order_id)	(namespace,v1_business_order_id,legacy_business_order_id,cutover_generation)->paper_exit_business_identity_aliases(namespace,v1_business_order_id,legacy_business_order_id,cutover_generation);confirmed_order_attempt_record_sha256->order_audit_chain(record_hash)	(namespace,legacy_business_order_id);(namespace,business_identity_binding_sha256,reservation_generation)	idx_paper_exit_reservation_expiry(namespace,expires_at_shanghai,v1_business_order_id)	ND(paper_exit_business_reservations);trg_paper_exit_business_reservations_validate_update
paper_exit_event_outbox	NEW	outbox_identity_sha256 SHA;namespace T NN;attempt_identity_sha256 SHA;order_ordinal I NN CHECK(order_ordinal>0);v1_business_order_id B NN;order_fact_identity_sha256 SHA;authorized_at_shanghai TS;permit_window_start TS;permit_window_end TS;canonical_event_bytes CANON;event_record_sha256 SHA;created_at_shanghai TS	(outbox_identity_sha256)	(attempt_identity_sha256)->paper_exit_open_attempts(attempt_identity_sha256);(namespace,v1_business_order_id)->paper_exit_business_reservations(namespace,v1_business_order_id)	(namespace,attempt_identity_sha256,order_ordinal);(order_fact_identity_sha256);(event_record_sha256)	idx_paper_exit_event_attempt(namespace,attempt_identity_sha256,order_ordinal)	AO(paper_exit_event_outbox)
paper_exit_outbox_delivery	NEW	outbox_identity_sha256 SHA;delivery_generation I NN CHECK(delivery_generation>0);state T NN;worker_boot_identity_sha256 T N;fence_generation I NN;scheduled_at_shanghai TS;started_at_shanghai T N;finished_at_shanghai T N;last_receipt_record_sha256 T N;updated_at_shanghai TS	(outbox_identity_sha256)	(outbox_identity_sha256)->paper_exit_event_outbox(outbox_identity_sha256);worker_boot_identity_sha256->paper_exit_process_boots(process_boot_identity_sha256);last_receipt_record_sha256->paper_exit_delivery_receipt_audit(record_sha256)	(outbox_identity_sha256,delivery_generation)	idx_paper_exit_delivery_state(state,scheduled_at_shanghai,outbox_identity_sha256)	ND(paper_exit_outbox_delivery);trg_paper_exit_outbox_delivery_validate_update
paper_exit_delivery_receipt_audit	NEW	receipt_identity_sha256 SHA;outbox_identity_sha256 SHA;delivery_generation I NN;outcome_kind T NN;provider_receipt_raw_bytes_sha256 T N;typed_error_raw_bytes_sha256 T N;protected_evidence_locator_sha256 SHA;canonical_record_bytes CANON;record_sha256 SHA;previous_record_sha256 T N;recorded_at_shanghai TS	(receipt_identity_sha256)	(outbox_identity_sha256)->paper_exit_event_outbox(outbox_identity_sha256)	(outbox_identity_sha256,delivery_generation);(record_sha256)	idx_paper_exit_receipt_outbox(outbox_identity_sha256,delivery_generation)	AO(paper_exit_delivery_receipt_audit)
paper_exit_adjacent_change_confirmations	NEW	confirmation_identity_sha256 SHA;namespace T NN;instrument_identity_sha256 SHA;previous_observation_sha256 SHA;current_observation_sha256 SHA;quote_batch_identity_sha256 SHA;approver_identity_sha256 SHA;approved_at_shanghai TS;expires_at_shanghai TS;canonical_record_sha256 SHA	(confirmation_identity_sha256)	-	(namespace,instrument_identity_sha256,previous_observation_sha256,current_observation_sha256);(canonical_record_sha256)	idx_paper_exit_change_confirmation_expiry(namespace,expires_at_shanghai)	AO(paper_exit_adjacent_change_confirmations)
paper_exit_adjacent_change_not_applicable_proofs	NEW	proof_identity_sha256 SHA;namespace T NN;instrument_identity_sha256 SHA;current_observation_sha256 SHA;quote_batch_identity_sha256 SHA;reason T NN;raw_source_evidence_sha256 SHA;canonical_record_sha256 SHA;recorded_at_shanghai TS	(proof_identity_sha256)	-	(namespace,instrument_identity_sha256,current_observation_sha256);(canonical_record_sha256)	idx_paper_exit_change_na_reason(namespace,reason)	AO(paper_exit_adjacent_change_not_applicable_proofs)
paper_exit_owner_handoffs	NEW	handoff_identity_sha256 SHA;namespace T NN;subject_kind T NN;subject_identity_sha256 SHA;old_owner_boot_identity_sha256 SHA;old_generation I NN;new_owner_boot_identity_sha256 SHA;new_process_lock_generation I NN;canonical_record_sha256 SHA;consumed_at_shanghai T N;created_at_shanghai TS	(handoff_identity_sha256)	old_owner_boot_identity_sha256->paper_exit_process_boots(process_boot_identity_sha256);new_owner_boot_identity_sha256->paper_exit_process_boots(process_boot_identity_sha256)	(namespace,subject_kind,subject_identity_sha256,old_generation);(canonical_record_sha256)	idx_paper_exit_handoff_consume(namespace,subject_kind,subject_identity_sha256,consumed_at_shanghai)	ND(paper_exit_owner_handoffs);trg_paper_exit_owner_handoffs_validate_update
paper_exit_owner_death_proofs	NEW	proof_identity_sha256 SHA;namespace T NN;subject_kind T NN;subject_identity_sha256 SHA;old_owner_boot_identity_sha256 SHA;old_generation I NN;new_owner_boot_identity_sha256 SHA;new_process_lock_generation I NN;audit_tail_record_sha256 SHA;canonical_record_sha256 SHA;consumed_at_shanghai T N;created_at_shanghai TS	(proof_identity_sha256)	old_owner_boot_identity_sha256->paper_exit_process_boots(process_boot_identity_sha256);new_owner_boot_identity_sha256->paper_exit_process_boots(process_boot_identity_sha256)	(namespace,subject_kind,subject_identity_sha256,old_generation);(canonical_record_sha256)	idx_paper_exit_death_proof_consume(namespace,subject_kind,subject_identity_sha256,consumed_at_shanghai)	ND(paper_exit_owner_death_proofs);trg_paper_exit_owner_death_proofs_validate_update
paper_exit_quarantine_resolution_events	NEW	resolution_event_identity_sha256 SHA;namespace T NN;quarantine_terminal_record_sha256 SHA;resolution_identity_sha256 SHA;event_ordinal I NN CHECK(event_ordinal>0);event_kind T NN;canonical_record_bytes CANON;record_sha256 SHA;previous_resolution_record_sha256 T N;created_at_shanghai TS	(resolution_event_identity_sha256)	-	(namespace,resolution_identity_sha256,event_ordinal);(record_sha256)	idx_paper_exit_resolution_sequence(namespace,resolution_identity_sha256,event_ordinal)	AO(paper_exit_quarantine_resolution_events)
order_idempotency	REUSED	business_order_id T NN;reserved_at T NN DEFAULT CURRENT_TIMESTAMP	(business_order_id)	-	-	-	existing-only
order_audit	REUSED	id I NN AUTOINCREMENT;business_order_id T NN;source T NN;decision_basis T NN;side T NN;code T NN;requested_price REAL NN;execution_price REAL N;quantity I NN;quote_observed_at T N;outcome T NN;failure_reason T N;created_at T NN DEFAULT CURRENT_TIMESTAMP	(id)	-	-	-	existing trg_order_audit_no_delete;NEW trg_order_audit_no_update;BR-201 inserts only through Br201ExitFinalOwnerV1
order_audit_chain	REUSED	order_audit_id I NN;previous_hash T NN;record_hash T NN;created_at T NN DEFAULT CURRENT_TIMESTAMP	(order_audit_id)	(order_audit_id)->order_audit(id)	(record_hash)	-	existing trg_order_audit_chain_no_update;trg_order_audit_chain_no_delete
paper_trades	REUSED	id I NN AUTOINCREMENT;plan_id T NN;code T NN;name T NN;direction T NN;price REAL NN;quantity I NN;status T NN;fill_price REAL N;not_fill_reason T N;virtual_reason T NN;account_mode T NN;data_mode T NN;ts T NN DEFAULT CURRENT_TIMESTAMP;updated_at T NN DEFAULT CURRENT_TIMESTAMP	(id)	-	(plan_id)	uniq_paper_trades_plan_id(plan_id);idx_paper_trades_code(code);idx_paper_trades_status(status)	existing-only;BR-201 insert only through Br201ExitFinalOwnerV1
```
<!-- BR201_SQLITE_SCHEMA_V1_END -->

The manifest payload is the 27 lines between the `text` fences joined by LF with no trailing LF.
Its exact domain-separated SHA-256 over
`stock_analysis.br201.sqlite_schema_manifest.v1\0 || payload` is
`d2c86f4da921d684f37264d3f8ec86c861f991964ad4df0e8e0238697e0763c7`.

The exact state triggers reject every update except these checked transitions and require all
unmentioned columns byte-equal: process-lock generation increments by one with a new live boot;
projection follows only the closed projection graph and proof-consuming same-state takeover;
debounce increments by one only from a successful Terminal CAS; open attempt follows
`Open -> Terminalized|Quarantined`; fence follows the closed generation/state graph; cutover follows
only its two monotonic edges; reservation increments generation only after expiry or changes its
confirmation from null to one BR-086 hash inside the accepted transaction; delivery follows
`Pending -> Sending -> Delivered|Rejected|Uncertain`; handoff/death proof changes only
`consumed_at_shanghai:null -> value` in the same owner-takeover transaction. Every trigger also
checks the table-specific null/enumeration/cardinality constraints defined elsewhere in this
design; the owning Rust operation separately requires affected row count one and exact read-back.
The new reused-table `trg_order_audit_no_update` unconditionally aborts every UPDATE, matching the
existing delete prohibition and immutable chain. Trigger SQL text and this TSV's SHA-256 are golden fixtures.

`Br201PaperExitStore::migrate_schema_v1` is the only migration owner. Its exact transactional order
is: (1) descriptor-attest the physical namespace and obtain the exclusive maintenance lock; (2)
validate reused-table SQL, columns, indexes and triggers byte-for-byte; (3) create schema-meta and
process tables; (4) create session/projection/debounce/open/fence tables; (5) create protected
account evidence; (6) create compatibility descriptor, legacy alias input, cutover, alias and reservation tables; (7) create
order outbox/delivery/receipt tables; (8) create adjacent-change, ownership-proof and quarantine
tables; (9) create all named indexes; (10) create all named triggers; (11) install singleton genesis
rows; (12) scan and bind exact legacy high-waters/alias inputs; (13) validate `foreign_key_check`,
`integrity_check`, schema-manifest hash, chain tails, row decoders and read-back; then commit FULL and
parent-directory sync. Any failure rolls back the whole v1 transaction and leaves capability
Disabled. There is no downgrade/drop migration. Containment and deep rollback retain
`decode_br201_sqlite_schema_v1`, every table and every trigger; rollback may stop new writes but
must continue full decoding, reconciliation, permanent alias/confirmed guards and append-only audit
retention.

The only child-private owner operations are `register_account_provider_once_v1`,
`migrate_schema_v1`, `decode_br201_sqlite_schema_v1`, `begin_tick_v1`,
`admit_attempt_v1`, `seal_account_acquisition_v1`, `record_rejection_v1`,
`authorize_order_v1`, `terminalize_attempt_v1`, `reconcile_open_attempts_v1`,
`project_audit_v1`, `deliver_outbox_v1`, and `advance_identity_cutover_v1`. Each is called only from
`execute_paper_exit_tick_v1`; none is `pub`, re-exported, returned as a connection/transaction, or
callable by a sibling module. The parent sees only the final `PaperExitAttemptResultV1` or
`PaperExitAttemptErrorV1`.

`paper_exit_session_audit` is append-only (UPDATE/DELETE denied), hash-chained, retained at least
five years and stores canonical record bytes plus their hashes. Each audit record has exactly one
projection-intent row, keyed by its immutable record hash with a UNIQUE foreign key. Admission
append + open-row insert + initial `ProjectionOpen` attempt fence + Admission projection intent are
one transaction; Terminal append + open-row CAS + terminal attempt-fence CAS + Terminal projection
intent are another transaction, with the persistent debounce CAS as a fifth effect only for
`Succeeded`/`SucceededEmpty`. Admission reads back four effects; Terminal reads back four or five
according to its frozen status before acknowledgement. An intent/fence/debounce insert or CAS,
constraint or read-back failure aborts the whole
authoritative transaction, so there is no commit-then-enqueue crash gap and no committed Admission
or Terminal that a restarted projector cannot discover.

Existing JSONL/audit-dispatcher files are downstream observability projections only. Their worker
claims the already committed `paper_exit_audit_projection_outbox` row and appends the exact
canonical SQLite record; file append/flush/sync success then acknowledges that intent. A file append
is never claimed atomic with SQLite, never authorizes an order and never replaces the SQLite chain.
The projection-intent state set is closed as `Pending`, `Claimed`, `AppendAckPending`, `Acked`,
`Quarantined`; there is no `Uncertain` state or free-form alias. The only transitions are
`Pending -> Claimed`, `Claimed -> AppendAckPending`, `AppendAckPending -> Acked`, and
`Pending|Claimed|AppendAckPending -> Quarantined`, plus the two explicitly legal ownership-only
takeovers `Claimed -> Claimed` and `AppendAckPending -> AppendAckPending`. Either same-state
takeover requires the successor to hold the pinned namespace exclusive lock, consume exactly one
durable handoff or owner-death proof, preserve the state/canonical bytes/projection identity, install
a new owner and checked next claim generation, and bind the successor's strictly newer persisted
process-lock generation. No other same-state rewrite is valid. A surviving claim is recovered in place; it is
never rewritten to `Pending`, and no transition changes the immutable projection identity or
canonical bytes. Any append, flush, sync, state-CAS or read-back failure leaves the last committed
state for reconciliation; it cannot erase or reinterpret the authoritative audit or synthesize a
new intent.

`Claimed` is not anonymous or lease-based. Each intent stores
`claim_owner_boot_identity_sha256`, nonzero monotonic `claim_generation`,
`claim_process_lock_generation` and `claim_identity_sha256`; they are all-null in `Pending`,
all-present in `Claimed`/`AppendAckPending`, and preserved through `Acked`/`Quarantined` for audit.
`Pending -> Claimed` requires the current boot to hold the pinned namespace exclusive owner lock
and CAS the exact prior intent version. A live claimant may create one durable handoff only after
stopping file work. A successor to a crashed claimant must hold that same namespace lock and append
a durable owner-death proof bound to old owner/claim/process generations, intent identity,
canonical bytes/hash and current audit tail; age, PID lookup and lease expiry never authorize it.
The takeover transaction consumes exactly one handoff/death proof, preserves immutable bytes and
state, changes only owner/claim metadata, increments `claim_generation` by checked one and binds the
successor's current process-lock generation. It requires affected counts `proof=1,intent=1`, commits
both effects together and reads back the exact state, new owner, new claim identity/generation,
process-lock generation, immutable bytes and consumed-proof identity before acknowledgement. A
zero/multiple count, overflow, stale owner/generation, changed canonical byte or lost namespace lock
rolls back both effects. Repeated claimant crashes repeat this proof chain; they never reset to
`Pending`.

Before validating or appending JSONL, a claimant takes the existing exclusive file lock and,
while holding it, rereads the SQLite intent plus claim owner/generation/identity. Only a byte-equal
row still owned by that claimant generation may continue. Immediately before each projection CAS
it performs the same lock-internal reread. If a recovery/takeover CAS affects zero rows, the loser
must remain under the file lock, read the committed winner, and converge deterministically:
byte-equal `Claimed`/`AppendAckPending` owned by another generation means zero append and stop;
byte-equal `Acked` means verify the exact durable leaf and return success; `Quarantined` means
return the same quarantine; missing/conflicting bytes or any unknown state fails closed. It may not
retry its stale CAS, append from memory or infer victory from a readable line. Two recovery
processes can therefore produce one owner and at most one append even across repeated crashes.
The decoder and verifier accept the two same-state transitions only when every proof/owner/
generation predicate above is present; an owner change without the matching consumed proof, an
unchanged owner, a non-incremented claim generation or a transition from any other state is corrupt.

Append success followed by loss of the SQLite acknowledgement is not handled by blind re-append,
and a readable line is **not** proof that an earlier process completed `sync_all`. Every JSONL
projection envelope carries the immutable SQLite `record_sha256`, projection-intent identity and
exact canonical-record hash. Recovery holds the existing cross-process file lock continuously,
performs the owner/generation reread above, and:

1. validates the complete JSONL hash chain, framing and tail, then requires either zero or exactly
   one projection line with byte-equal identity, canonical bytes and hashes;
2. if zero lines match, appends exactly once, flushes, calls file `sync_all`, calls `sync_all` on
   the pinned parent-directory descriptor, and validates the complete chain and exact leaf again;
3. if exactly one line already matches, performs no append but still calls file `sync_all`, calls
   parent-directory `sync_all`, and then repeats the same complete-chain and exact-leaf validation;
4. only after that second validation may SQLite CAS `Claimed -> AppendAckPending`; immediately
   before every `AppendAckPending -> Acked` CAS, while still holding the file lock, it repeats file
   and directory `sync_all` and the full chain/leaf validation once more; and
5. commits each CAS with exact affected-row count and byte-identical read-back.

A crash in `Claimed` after any write/flush/file-sync/directory-sync/revalidation step resumes the
same procedure. A crash in `AppendAckPending`, including SQLite acknowledgement loss, repeats the
durability and validation cycle before the Ack CAS; prior readability never shortcuts it. Multiple
matches, identity reuse with different bytes/hash, a partial tail, an unreadable chain, descriptor
drift, a failed durability call or a post-sync revalidation mismatch writes append-only
`Quarantined` evidence where possible and keeps startup closed. Thus exact existing bytes prevent a
second append, while current durability is re-established and proven before acknowledgement.

The authority emits schema-versioned `PaperExitSessionAudit` records; logs are not authority. The
closed record field order is:

1. `schema_version`;
2. `phase` (`Admission` or `Terminal`);
3. fixed ordered
   `rule_ids=["2.1","2.2","2.3","2.4","2.5","2.6","2.7","BR-084","BR-086","BR-134","BR-201"]`;
4. `attempt_identity_sha256` and monotonic `audit_ordinal`;
5. optional calendar `source_authority_uri`, `artifact_uri`, `calendar_version`,
   `artifact_raw_bytes_sha256`;
6. `observed_at_shanghai`, `market_date`, optional `session`, `window_start`, `window_end`;
7. `decision_status`, non-empty structured `reason_code`, typed `retryability`;
8. optional `debounce_version_observed`;
9. optional `account_acquisition_evidence_identity_sha256`,
   `account_acquisition_evidence_record_sha256`, `account_batch_identity_sha256`,
   `account_provider_source_id`, `account_source_authority_uri`,
   `account_source_captured_at_shanghai`;
10. optional `admitted_record_sha256`, optional `joined_order_fact_count`, optional
   `joined_order_facts_sha256`, optional `reconciliation_evidence_sha256`;
11. optional `previous_record_sha256` (serialized `null` only for the first namespace record); and
12. the separately computed `record_sha256` envelope field.

All hashes are lowercase 64-hex. Source-authority URI, artifact URI/version/raw-byte hash are
mandatory for admitted, success and ordinary engine-failure records. The two acquisition-evidence
fields are all-null before a provider boundary is entered and both present after any provider
response or typed provider error is durably sealed. They identify a separate protected acquisition
record and never contain the raw response/error bytes. The four normalized account fields preserve
only independently validated facts according to the reason-specific matrix below; a missing or
invalid fact is `null`, never guessed or copied from a sibling field. `RejectedAuthority` preserves
only fields actually parsed and identifies every absence through its reason code. Executable decisions require session and both
window fields; verified non-executable decisions require a session and null windows; authority
rejection must not invent any of them.

#### Canonical attempt key and full-record hash

The attempt key and full record are two different canonical preimages. They use Rust structs (not
maps) serialized by `serde_json::to_vec`, compact UTF-8, declaration order, JSON integers, no BOM,
whitespace, trailing newline or Unicode normalization. Every `Option::None` is the literal four
bytes `null`; it is never omitted or encoded as an empty string. Shanghai timestamps use exactly
`YYYY-MM-DDTHH:MM:SS.nnnnnnnnn+08:00`; dates use `YYYY-MM-DD`.

`PaperExitAttemptKeyV1` has the exact ordered fields
`schema_version,rule_id,process_boot_identity_sha256,scheduler_tick_ordinal,observed_at_shanghai,
market_date,source_authority_uri,artifact_uri,calendar_version,artifact_raw_bytes_sha256,session,
window_start,window_end`.
Its hash is lowercase SHA-256 over the exact domain bytes
`stock_analysis.br201.paper_exit_attempt_key.v1\0` followed by the canonical JSON bytes. This key
excludes phase, terminal result and previous-record hash so Admission and Terminal for one attempt
must have the same identity.

`PaperExitSessionRecordV1` has exactly 29 ordered fields:
`schema_version,phase,rule_ids,attempt_identity_sha256,audit_ordinal,source_authority_uri,
artifact_uri,calendar_version,artifact_raw_bytes_sha256,observed_at_shanghai,market_date,session,
window_start,window_end,decision_status,reason_code,retryability,debounce_version_observed,
account_acquisition_evidence_identity_sha256,account_acquisition_evidence_record_sha256,
account_batch_identity_sha256,
account_provider_source_id,account_source_authority_uri,account_source_captured_at_shanghai,
admitted_record_sha256,joined_order_fact_count,joined_order_facts_sha256,
reconciliation_evidence_sha256,previous_record_sha256`. This is the full expansion of items 1-11
above; item 12 `record_sha256` is envelope-only and is not part of the record preimage. In particular,
`previous_record_sha256: Option<String>` is mandatory in the Rust struct and always serialized:
literal JSON `null` for the first record in a physical namespace, otherwise the lowercase 64-hex
hash of the immediately preceding record. It is never omitted or replaced by an empty/genesis
sentinel. The record hash is lowercase SHA-256 over the exact domain bytes
`stock_analysis.br201.paper_exit_record.v1\0` followed by those canonical JSON bytes. Admission and
Terminal therefore share their attempt key but necessarily have different record hashes.

The executable-attempt golden bytes are:

```text
{"schema_version":1,"rule_id":"BR-201","process_boot_identity_sha256":"0000000000000000000000000000000000000000000000000000000000000000","scheduler_tick_ordinal":7,"observed_at_shanghai":"2026-08-03T09:30:00.000000000+08:00","market_date":"2026-08-03","source_authority_uri":"https://www.sse.com.cn/disclosure/announcement/general/c/c_20251222_10802507.shtml","artifact_uri":"repo://config/a_share_market_holidays.csv","calendar_version":"a-share-2026-v1","artifact_raw_bytes_sha256":"ef9044635e9fc7475efcc1972961fd5306a9cbb28e052e91997f132e6da413d5","session":"Morning","window_start":"2026-08-03T09:30:00.000000000+08:00","window_end":"2026-08-03T11:30:00.000000000+08:00"}
```

The required attempt hash is
`7c216d3123d2434636d4c04bc096df3160e2ab1ed1aa8df530704a9c63d384d8`.
The terminal-record golden bytes using that attempt identity are:

```text
{"schema_version":1,"phase":"Terminal","rule_ids":["2.1","2.2","2.3","2.4","2.5","2.6","2.7","BR-084","BR-086","BR-134","BR-201"],"attempt_identity_sha256":"7c216d3123d2434636d4c04bc096df3160e2ab1ed1aa8df530704a9c63d384d8","audit_ordinal":9,"source_authority_uri":"https://www.sse.com.cn/disclosure/announcement/general/c/c_20251222_10802507.shtml","artifact_uri":"repo://config/a_share_market_holidays.csv","calendar_version":"a-share-2026-v1","artifact_raw_bytes_sha256":"ef9044635e9fc7475efcc1972961fd5306a9cbb28e052e91997f132e6da413d5","observed_at_shanghai":"2026-08-03T09:30:00.000000000+08:00","market_date":"2026-08-03","session":"Morning","window_start":"2026-08-03T09:30:00.000000000+08:00","window_end":"2026-08-03T11:30:00.000000000+08:00","decision_status":"SucceededEmpty","reason_code":"verified_empty","retryability":"AfterDebounce","debounce_version_observed":4,"account_acquisition_evidence_identity_sha256":"5555555555555555555555555555555555555555555555555555555555555555","account_acquisition_evidence_record_sha256":"6666666666666666666666666666666666666666666666666666666666666666","account_batch_identity_sha256":"4444444444444444444444444444444444444444444444444444444444444444","account_provider_source_id":"TEST_CODE_VERIFIED_BROKER","account_source_authority_uri":"test://TEST_CODE_BR201/account-evaluation","account_source_captured_at_shanghai":"2026-08-03T09:29:59.000000000+08:00","admitted_record_sha256":"2222222222222222222222222222222222222222222222222222222222222222","joined_order_fact_count":0,"joined_order_facts_sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","reconciliation_evidence_sha256":null,"previous_record_sha256":"3333333333333333333333333333333333333333333333333333333333333333"}
```

The required record hash is
`9855f9b9032ebd8fe4bd2f1e457eec74f1ab785e295016e3c1bdf2d93a1798df`.
Golden and mutation tests must independently recompute both hashes and reject domain, NUL, field-
order, time precision/offset, `previous_record_sha256` omission/null-at-nonfirst/empty-or-sentinel,
non-null-at-first, integer/string, extra-field and newline changes.

`debounce_version_observed` is null only for standalone `SkippedNonExecutable` and
`RejectedAuthority`. It is a positive JSON integer for `DeferredDebounce`, `Admission/Admitted`
and every Terminal outcome of an admitted attempt; the Admission and its Terminal must preserve the
same value. A missing, zero, changed or string-encoded version is invalid. This field is the
canonical proof that an executable tick read debounce before selecting defer/admit; there is no
separate pre-decision trace record.

#### Closed status/field matrix

`reason_code` is not a free-form string or prefix family. Gate B introduces the closed
`PaperExitReasonCodeV1` enum and serializes each variant to exactly the following lowercase bytes;
unknown bytes fail decoding and a known code used with another status fails validation:

| Status | Exact allowed serialized `reason_code` values |
| --- | --- |
| `SkippedNonExecutable` | `session_closed`, `session_auction`, `session_preopen_gap`, `session_lunch_break`, `session_after_hours` |
| `RejectedAuthority` | `calendar_unavailable`, `calendar_invalid_bytes`, `calendar_source_authority_mismatch`, `calendar_artifact_uri_mismatch`, `calendar_version_mismatch`, `calendar_artifact_raw_bytes_hash_mismatch`, `calendar_required_field_missing`, `calendar_poisoned`, `calendar_out_of_coverage` |
| `DeferredDebounce` | `success_debounce_not_due` |
| `Admitted` | `attempt_admitted` |
| `FailedAccountContext` | `account_context_missing`, `account_schema_version_unsupported`, `account_context_partial`, `account_context_cross_batch`, `account_provider_authority_invalid`, `account_context_identity_missing`, `account_source_time_invalid`, `account_local_observation_time_invalid`, `account_source_time_future`, `account_local_observation_time_future`, `account_context_stale`, `account_decimal_syntax_invalid`, `account_decimal_negative_zero`, `account_decimal_scale_exceeded`, `account_decimal_parse_overflow`, `account_fen_rounding_overflow`, `account_money_negative`, `account_total_assets_nonpositive`, `account_assets_cash_position_mismatch`, `account_position_market_value_sum_mismatch`, `account_consecutive_stop_loss_count_negative`, `account_position_identity_duplicate`, `account_positions_order_noncanonical`, `account_position_quantity_invalid`, `account_position_market_value_nonpositive`, `account_position_bps_aggregate_out_of_range`, `account_position_bps_residual_invalid`, `account_position_bps_mismatch`, `account_proposed_exit_position_missing`, `account_proposed_exit_position_ambiguous` |
| `FailedPermitExpired` | `permit_expired_before_atomic_authorization` |
| `FailedEngine` | `paper_ledger_unavailable`, `quote_unavailable`, `quote_stale`, `quote_incomplete`, `manual_confirmation_required`, `market_data_validation_rejected`, `decision_evaluation_failed`, `proposed_exit_sort_key_duplicate`, `order_safety_rejected`, `test_live_namespace_rejected`, `business_order_duplicate_within_60s`, `business_order_identity_already_confirmed`, `business_order_identity_alias_conflict`, `business_order_identity_history_unresolved`, `business_order_identity_cutover_not_ready`, `order_attempt_audit_unavailable`, `order_commit_failed`, `reconciled_complete_fact_set`, `reconciled_no_committed_fact`, `reconciliation_frozen_pending_unsent` |
| `SucceededEmpty` | `verified_empty` |
| `Succeeded` | `completed` |
| `QuarantinedUncertain` | `reconciliation_broken_join`, `reconciliation_sending_ambiguous`, `reconciliation_ack_pending_ambiguous`, `reconciliation_send_uncertain`, `reconciliation_stale_generation_ack`, `reconciliation_unknown_delivery_state` |

No `session_*`, `calendar_*`, `account_context_*`, `registered typed engine failure` or
`reconciliation_*` wildcard is an accepted value. Provider/database diagnostics remain a typed
internal error and a non-sensitive operational log; they do not extend this wire enum or enter the
canonical audit preimage.

The `FailedAccountContext` mappings are exhaustive and one-to-one:

| Validation failure | Exact reason |
| --- | --- |
| provider returned no account batch | `account_context_missing` |
| decoded `schema_version` is not exactly `1` | `account_schema_version_unsupported` |
| batch exists but any required non-provider/non-identity top-level or position field is absent | `account_context_partial` |
| more than one response/batch contributes fields | `account_context_cross_batch` |
| provider source ID or authority URI is absent, malformed, unregistered or does not match the register-once owner | `account_provider_authority_invalid` |
| batch/position hash is absent, malformed or unknown | `account_context_identity_missing` |
| source timestamp is unparseable or not the exact Shanghai grammar/offset | `account_source_time_invalid` |
| local-observation timestamp is unparseable or not the exact Shanghai grammar/offset | `account_local_observation_time_invalid` |
| source capture is after local observation or owner sample | `account_source_time_future` |
| local observation is after the owner sample | `account_local_observation_time_future` |
| source age is strictly greater than 30 seconds at admission or final use site | `account_context_stale` |
| decimal violates canonical grammar other than the two cases below | `account_decimal_syntax_invalid` |
| decimal is `-0` or a negative-zero fractional spelling | `account_decimal_negative_zero` |
| decimal has more than four fractional CNY digits | `account_decimal_scale_exceeded` |
| checked `i128` decimal accumulation overflows | `account_decimal_parse_overflow` |
| half-even conversion, absolute value or `i64` fen cast overflows | `account_fen_rounding_overflow` |
| cash/assets/market value is negative | `account_money_negative` |
| `total_assets_fen` is zero | `account_total_assets_nonpositive` |
| assets do not equal cash plus aggregate position value | `account_assets_cash_position_mismatch` |
| aggregate position value, empty-set totals or checked per-position sum disagree | `account_position_market_value_sum_mismatch` |
| `consecutive_stop_loss_count` is negative | `account_consecutive_stop_loss_count_negative` |
| BINARY position identity repeats | `account_position_identity_duplicate` |
| unique position array is not strictly BINARY ascending by instrument identity | `account_positions_order_noncanonical` |
| quantity is nonpositive, available quantity negative or exceeds quantity | `account_position_quantity_invalid` |
| a non-empty position market value is zero/nonpositive | `account_position_market_value_nonpositive` |
| half-even aggregate bps is outside `0..=10_000` | `account_position_bps_aggregate_out_of_range` |
| residual is negative, exceeds position count or arithmetic overflows | `account_position_bps_residual_invalid` |
| stored aggregate/per-position bps differ from the deterministic allocation or sum | `account_position_bps_mismatch` |
| a proposed exit joins zero positions in the same batch | `account_proposed_exit_position_missing` |
| a proposed exit joins more than one position in the same batch | `account_proposed_exit_position_ambiguous` |

`Br201AccountAcquisitionEvidenceV1` is a separate protected record with exact ordered fields
`schema_version,namespace,attempt_identity_sha256,provider_registration_identity_sha256,
provider_call_ordinal,provider_call_started_at_shanghai,provider_call_finished_at_shanghai,
outcome_kind,response_raw_bytes_sha256,error_raw_bytes_sha256,
protected_evidence_locator_sha256,previous_acquisition_record_sha256`. `outcome_kind` is exactly
`Response` or `TypedError`; the corresponding raw-byte hash is a lowercase 64-hex value and the
other is `null`. The two owner-sampled call timestamps are valid canonical Shanghai instants and
ordered start `<=` finish. The locator hash identifies an access-controlled, append-only provider
evidence object retained under the account-provider policy; the object contains the exact raw
response or typed-error bytes. Those sensitive bytes, account IDs, holding codes, and locator
plaintext never enter `PaperExitSessionRecordV1`, JSONL, logs, PR evidence, or general audit. The
acquisition record hash uses exact domain
`stock_analysis.br201.account_acquisition_evidence.v1\0`; its identity uses exact domain
`stock_analysis.br201.account_acquisition_identity.v1\0` over the first seven fields. A missing,
unreadable, hash-mismatched, cross-attempt, or wrong-outcome protected object is itself
`account_context_partial` and cannot authorize normalized facts.

The four normalized provenance fields in `PaperExitSessionRecordV1` use this reason-specific
matrix. `V` means preserve the independently parsed and validated value, `N` means literal JSON
`null`, and `V/N` means validate that individual field without depending on the failed sibling;
invalid/missing is `N`. The acquisition evidence pair is `N/N` only before the provider call and
is `V/V` for every provider response or typed error.

| Selected `FailedAccountContext` reason/class | batch identity | provider source ID | source authority URI | source captured time |
| --- | --- | --- | --- | --- |
| provider typed error or no batch: `account_context_missing` | N | N | N | N |
| unsupported schema, partial batch, decimal/arithmetic/position/order/join failure | V/N | V/N | V/N | V/N |
| `account_context_cross_batch` | N | N | N | N |
| `account_provider_authority_invalid` | V/N | N | N | V/N |
| `account_context_identity_missing` | N | V/N | V/N | V/N |
| `account_source_time_invalid` or `account_source_time_future` | V/N | V/N | V/N | N |
| `account_local_observation_time_invalid` or `account_local_observation_time_future` | V/N | V/N | V/N | V/N |
| `account_context_stale` | V | V | V | V |

For local-observation-time failures, the rejected raw field remains only in the protected response
object; the session record has no local-observation field and therefore cannot imply that it was
valid. All later normalized validation failures preserve every safely validated fact and never
force an all-present fiction. Mutation tests cover every reason row and every `V/N` choice,
including response/error hash swaps, raw locator leakage, a fabricated sibling fact, and a valid
field being unnecessarily erased.

Precedence is the table order after safe decoding: the first failing invariant selects the sole
reason. A byte sequence that cannot be safely parsed stops at its parser reason and is never also
classified by a later arithmetic invariant. Mutation tests cover every row; no account rejection
may use `market_data_validation_rejected`, `order_safety_rejected` or an internal diagnostic.

The table above is the one wire registry for `PaperExitReasonCodeV1`; there is no second string
mapping in the engine, database or projector. Encoding is a total one-to-one match from enum
variant to the exact bytes in the table. Decoding unknown bytes, aliases, case changes, duplicate
serialized values or a reason/status pair not present in the table fails before record insertion
and projection. In particular, an unresolved AGENTS 2.3 adjacent-change gate maps only to
`FailedEngine/manual_confirmation_required`; it may not collapse into
`market_data_validation_rejected`, a free-form diagnostic or an order-safety reason.

No status/field combination outside this matrix is valid:

| Phase/status | Authority/session/window | Admission join | Order-fact join | Retryability | Exact reason class |
| --- | --- | --- | --- | --- | --- |
| Terminal / `SkippedNonExecutable` | authority required; non-executable session; null window | null | null | `Immediate` | exact `SkippedNonExecutable` enum set above |
| Terminal / `RejectedAuthority` | parsed authority fields optional; session/window null | null | null | `Immediate` | exact `RejectedAuthority` enum set above |
| Terminal / `DeferredDebounce` | authority + executable session/window required | null | null | `AfterDebounce` | `success_debounce_not_due` |
| Admission / `Admitted` | authority + executable session/window required | null | null | `NotApplicable` | `attempt_admitted` |
| Terminal / `FailedAccountContext` | authority + executable session/window required | admission hash required | count/hash required, including canonical empty join | `Immediate` | exact `FailedAccountContext` enum set above |
| Terminal / `FailedPermitExpired` | authority + executable session/window required | admission hash required | count/hash required | `Immediate` | `permit_expired_before_atomic_authorization` |
| Terminal / `FailedEngine` | authority + executable session/window required | admission hash required | count/hash required | `Immediate` | exact `FailedEngine` enum set above |
| Terminal / `SucceededEmpty` | authority + executable session/window required | admission hash required | zero + canonical empty hash | `AfterDebounce` | `verified_empty` |
| Terminal / `Succeeded` | authority + executable session/window required | admission hash required | positive count + exact join hash | `AfterDebounce` | `completed` |
| Terminal / `QuarantinedUncertain` | preserve admitted authority/session/window | admission hash required | nullable; never presented as a valid join | `NotApplicable` | exact `QuarantinedUncertain` enum set above; reconciliation evidence hash required |

`SkippedNonExecutable`, `RejectedAuthority`, `DeferredDebounce` and `Admission/Admitted` require the
acquisition-evidence pair and all four normalized account fields null. `FailedAccountContext`
requires the evidence pair present and the exact reason matrix above. A permit expiry before the
provider boundary has both groups null; after the provider boundary it preserves the evidence pair
and only individually validated normalized facts. Every `FailedEngine`, `SucceededEmpty` and
`Succeeded` occurs only after a fully admitted account batch and therefore requires the evidence
pair plus all four normalized facts present. Quarantine preserves the last validated combination
from its frozen source record and may not add or erase one. These are status/field validity rules,
not logging preferences.

Standalone skip/reject/defer decisions produce exactly one Terminal record. A due executable
attempt produces exactly one Admission/Admitted followed by exactly one Terminal with the same
attempt identity. `Admitted` is invalid in Terminal; every other status is invalid in Admission.
The canonical empty join hash is SHA-256 of zero bytes. `reconciliation_evidence_sha256` is null for
every non-quarantine row. For quarantine it hashes a separate typed, non-sensitive description of
the raw conflicting identities and reason; it must not populate `joined_order_facts_sha256` with an
invalid set. No preimage may contain account IDs, holding codes, position lists, webhook values or
credentials.

A due executable attempt must append its `Admission/Admitted` record, insert its open-attempt row,
insert its generation-one `ProjectionOpen` attempt fence and insert its Admission projection intent
in one `BEGIN IMMEDIATE` transaction, commit with FULL durability and read all four back before
BR-134 account context, paper ledger, provider or order acquisition. Every order transaction
starts with only the acknowledged initial Admission/open-attempt/fence authority. It may receive a
canonical intended BR-086 preimage prepared in memory, but that preimage is not acknowledged
evidence. For an accepted order, `Br201ExitFinalOwnerV1` inserts and reads back the Confirmed BR-086
row inside the same alias/reservation/order/outbox/delivery transaction, then binds its exact hash to
the remaining effects; for a rejection, the rejection-only transaction inserts and reads back its
Rejected row. The owner refuses commit if either the prior Admission authority or the in-transaction
BR-086 insertion/read-back is absent or invalid. Thus an authoritative SQLite
audit/constraint/read-back failure is order-preventing and returns a typed fail-closed result without
advancing debounce.

Completing the four-rule evaluation does not append a Terminal while an external projection can
still become uncertain. Instead, the engine seals one immutable terminal candidate and its exact
order/outbox join on the open row. The candidate may later become the normal Terminal only after all
of that attempt's outboxes are durably `DeliveredAcked`, or immediately when its exact outbox count
is zero. An uncertainty supersedes the candidate with `QuarantinedUncertain`; it never appends a
second Terminal. If final terminalization fails after an earlier immutable order fact, that fact is
not deleted; the persistent latch blocks later work across restart until reconciliation. No
subsequent order or debounce success is allowed.

#### Persistent open-attempt latch and reconciliation

The Admission transaction creates a durable `paper_exit_open_attempts` row. It freezes
`attempt_identity_sha256`, admission ordinal/hash, owner process-boot identity, admission
paper-order/order-audit/outbox high-water marks, `engine_phase=Collecting`, a nullable sealed
terminal candidate and latch state `Open`. The closed engine-phase set is `Collecting`, `Sealed`,
`HandoffReady`, `OwnerDeathProven`, `ReconciliationOwned`, `ReconciliationAuditPending`,
`Terminalized`, `Quarantined`; unknown values fail closed. The row freezes the reconciliation-only
fields `reconciliation_owner_boot_identity_sha256,reconciliation_claim_generation,
frozen_snapshot_sha256,pending_terminal_payload_sha256,pending_terminal_status,
pending_reason_code,pending_joined_order_fact_count,pending_joined_order_facts_sha256,
pending_reconciliation_evidence_sha256,pending_from_audit_ordinal`. Owner identity, claim
generation and frozen snapshot hash are non-null exactly in `ReconciliationOwned` and
`ReconciliationAuditPending` and null in every other phase. Every `pending_*` field is null in
`ReconciliationOwned` and outside those two phases. In `ReconciliationAuditPending`, payload,
status, reason, joined count/hash and from-audit ordinal are non-null; reconciliation-evidence is
nullable only where the Terminal status/field matrix permits null. A partial or cross-phase
combination is corruption. The live engine may CAS only
its own `Collecting -> Sealed` and
write the candidate once. Any nonterminal phase, especially `ReconciliationAuditPending`, or a
`Quarantined` phase is a persistent capability latch checked before scheduler construction;
process-local state is not authority. The Terminal event identity is derived from the attempt key plus literal phase
`Terminal`, has a UNIQUE constraint and can never be replaced; two Terminals are a fatal invariant
violation.

Reconciliation is not authorized merely because a row is nonterminal, old, apparently idle or
owned by a different PID. The BR-201 store namespace has one pinned process-owner lock held for the
entire owner boot. Its inode/device/root capability and monotonic `process_lock_generation` are
persisted in `paper_exit_process_boots`. A boot that cannot hold the exclusive lock cannot create an
Admission, order/outbox, handoff, owner-death proof or reconciliation claim. PID lookup, timestamp
age, retry count, wall clock and monotonic elapsed time are not owner-death evidence.

Process-lock generations are allocated from one checked SQLite singleton only after the OS lock is
held: the new boot CASes `next_generation -> next_generation+1`, persists that strictly increasing
value in its boot row and reads both back. Takeover requires `new_process_lock_generation >
old_process_lock_generation`, not exact old-plus-one. Gaps are valid because any number of later
boots may acquire the OS lock and crash before they claim a particular attempt. A death proof binds
the stale row's recorded generation to the current lock-holding boot's larger persisted generation.
Overflow, reuse, decrease, a boot row not matching the held lock capability or two current boot rows
fails closed. This permits the third and later recovery boot to take over safely without allowing
two simultaneous owners.

A live owner explicitly hands off only after stopping new BR-201 scheduler admissions and worker
claims. In one `BEGIN IMMEDIATE` transaction it CASes its own `Collecting` or `Sealed` row to
`HandoffReady`, increments the attempt generation, freezes the exact order/audit/outbox/delivery
high-water, and appends a hash-bound `paper_exit_owner_handoffs` row. Every order/outbox and delivery
mutation requires the phase and pre-handoff generation, so transaction ordering makes a concurrent
mutation either commit before the captured high-water or affect zero rows after handoff. A log,
shutdown signal or in-memory flag is not a handoff.

After a crash, a later boot may prove prior-owner death only by first acquiring and continuously
holding that same validated exclusive process-owner lock. It then uses one `BEGIN IMMEDIATE`
transaction to append a `paper_exit_owner_death_proofs` row bound to the prior and proving boot
identities, prior open-attempt identity, previous and new process-lock generations, pinned lock-file
identity, startup audit ordinal/hash and current audit-chain tail, and CASes that prior-owned
`Collecting` or `Sealed` row to `OwnerDeathProven`. Already-frozen reconciler states use the
same-phase fenced takeover transaction defined below and never pass through `OwnerDeathProven`,
because doing so would discard their frozen snapshot or pending payload. The proof is acknowledged
only after commit and exact read-back. Because a live owner must still hold the exclusive lock, a
competing live boot cannot manufacture this proof. There is deliberately no lease, expiry or
time-based takeover; if the exclusive lock or durable proof is unavailable, reconciliation remains
blocked.

The reconciler's eligibility matrix is closed:

| Durable engine phase | Normal owner may mutate | Reconciler may freeze | Required authority |
| --- | --- | --- | --- |
| `Collecting` | only the exact live owner boot and generation | no | prior owner must first become `OwnerDeathProven` through durable death proof |
| `Sealed` | exact owner may finish delivery/normal terminalization | no | explicit `HandoffReady` or durable prior-owner death proof |
| `HandoffReady` | no order/delivery creation | yes, once | exact unconsumed handoff hash and generation |
| `OwnerDeathProven` | no | yes, once | exact unconsumed owner-death-proof hash and proving boot |
| `ReconciliationOwned` | no | exact owner may prepare; a proven successor may perform same-phase fenced takeover | exact owner/generation/frozen snapshot, or one new durable death proof bound to all old fields |
| `ReconciliationAuditPending` | no | exact owner may finalize; a proven successor may perform same-phase fenced takeover, never refreeze or rejoin | exact owner/generation/frozen snapshot/pending payload, or one new durable death proof bound to all old fields |
| `Terminalized` / `Quarantined` | no | no | terminal/quarantine is permanent |

Takeover of an already-frozen reconciler is executable and has no intermediate phase. The
successor must hold the pinned namespace lock at its persisted
`new_process_lock_generation > old_process_lock_generation`; intervening unused generations are
permitted and are bound into the proof. One `BEGIN IMMEDIATE` transaction inserts exactly one
unconsumed death-proof row and performs the following compare-and-swaps:

- From `ReconciliationOwned`, require exact old owner boot `O`, claim generation `g`, non-null
  frozen snapshot `S`, every `pending_*` field null, attempt fence
  `(ReconciliationFrozen, owner=O, fence_generation=f)`, zero Terminal rows and no consumed proof.
  The new row remains `ReconciliationOwned`, changes only owner `O -> N` and
  `g -> g.checked_add(1)`, preserves `S` byte-for-byte and every pending null, while the fence
  remains `ReconciliationFrozen`, changes owner `O -> N` and
  `f -> f.checked_add(1)`.
- From `ReconciliationAuditPending`, require the same exact owner/generation/fence/zero-Terminal
  predicates plus a complete valid pending-field tuple. The new row remains
  `ReconciliationAuditPending`, changes only owner and checked claim generation, and preserves
  `S`, terminal payload, status, reason, joined count/hash, nullable reconciliation-evidence value
  and from-audit ordinal byte-for-byte; the fence changes only owner and checked generation as
  above. It must not refreeze, re-query, rejoin or regenerate the payload.

The death proof binds old/new process-lock generations, old/new owner identities, old/new claim and
fence generations, attempt identity, phase, `S`, a canonical hash of the complete pending tuple
(canonical all-null tuple for `ReconciliationOwned`) and current audit tail. The transaction marks
that proof consumed by the exact takeover identity, requires affected-row counts
`proof=1,open=1,fence=1,terminal=0`, commits, and reads every field back before acknowledgement.
Overflow, a changed pending byte, changed fence, existing Terminal, reused proof, lost lock or any
zero/multiple affected-row count rolls back all effects. Two successors racing on the same old
tuple can yield only one winner; the loser writes no proof, owner, generation, fence or Terminal.
A crash after any statement but before commit leaves the complete old tuple. A crash after commit
but before read-back leaves the complete new tuple; a later successor needs a new death proof for
`N` and cannot replay the proof for `O`. Neither takeover creates a Terminal, so the unique
Terminal constraint and phase/fence predicates still prevent a second terminalization.

Thus the startup reconciler must skip a current owner's live `Collecting`/`Sealed` attempt, even if
it can read the row. A same-boot reconciler also cannot freeze it until that owner commits the exact
handoff. A prior-boot row cannot be frozen until the new boot commits owner-death proof. Every
ineligible CAS must affect zero rows and leave the attempt, delivery rows and audit chain unchanged.

Attempt freeze ownership and per-outbox delivery progress are deliberately different state
machines:

- `paper_exit_attempt_fences` has one row per attempt with monotonic nonzero `fence_generation`,
  owner boot/claim identities and the closed attempt-level state set `ProjectionOpen`,
  `ReconciliationFrozen`, `Terminalized`, `Quarantined`. It never stores `Sending`, `AckPending` or
  another per-outbox delivery state.
- `paper_exit_outbox_delivery` has exactly one row for each `PaperExitEventOutbox`, keyed by exact
  attempt and outbox identities, with the closed state set `Pending`, `Sending`, `AckPending`,
  `DeliveredAcked`, `SendUncertain`, `FrozenPending`, `Quarantined`. It stores its own claim owner,
  claim identity and the attempt `fence_generation` observed when claimed. An attempt may own zero,
  one or many outboxes; uniqueness is `(attempt_identity_sha256,outbox_identity_sha256)`, and every
  outbox must have exactly one delivery row before engine sealing.

The final conditional paper-order transaction inserts the paper-order fact,
`PaperExitEventOutbox` and its initial `Pending` delivery row together. An affected-row count other
than one for any member aborts all three. Consequently a worker cannot observe an outbox without
its ownership/delivery state, and a delivery row cannot exist without the exact committed outbox.

A worker claim is one `BEGIN IMMEDIATE` transaction: require the attempt fence to be
`ProjectionOpen` at generation `g`, CAS exactly one delivery row `Pending -> Sending` with `g`, and
read it back before launching the external process. A worker never changes the attempt-level state
to `Sending`. After a syntactically valid external response it may persist
`Sending -> AckPending` with the immutable receipt bytes/hash at the same `g`; neither `Sending` nor
`AckPending` is resendable. The receipt-audit insert and `AckPending -> DeliveredAcked` CAS are one
SQLite transaction with an exact fence-generation predicate and read-back. They either both commit
or neither commits. Timeout, cancellation, crash, nonzero/invalid receipt or any possible loss
between send and this atomic acknowledgement yields or is reconciled as `SendUncertain` and is never
resent automatically.

Normal terminalization is permitted only for a sealed candidate with zero outboxes or with every
owned delivery row exactly `DeliveredAcked`. Its one transaction CASes the attempt fence
`ProjectionOpen -> Terminalized` while incrementing the generation, captures the exact
order/outbox/delivery snapshot, appends the single normal Terminal, CASes the open row to
`Terminalized` and inserts the Terminal projection intent; all effects commit and read back
together, including the persistent debounce CAS when the candidate is `Succeeded` or
`SucceededEmpty`. The SQLite write lock plus generation-changing CAS is the terminalization freeze; there
is no separately committed intermediate terminalization state. Because that freeze precedes the
Terminal in the transaction and the precondition excludes `Sending`, `AckPending` and
`SendUncertain`, no worker can start or acknowledge after a normal Terminal and no later delivery
uncertainty can require a second one.

`PaperExitAttemptReconciler` is the only owner allowed to reconcile an eligible nonterminal open
row. It must consume exactly one unconsumed durable handoff or owner-death proof and CAS
`HandoffReady`/`OwnerDeathProven -> ReconciliationOwned` together with attempt fence
`ProjectionOpen -> ReconciliationFrozen`, incrementing both monotonic generations. A crashed
`ReconciliationOwned` claim has no expiry: a later boot must create a new durable owner-death proof
for that reconciler boot and execute the exact same-phase takeover transaction above. In the same
`BEGIN IMMEDIATE` freeze transaction it CASes every still-`Pending` delivery row to `FrozenPending`,
captures the SQLite audit-chain tail/high-water and exact order/outbox/delivery/receipt rows, and
persists the immutable snapshot identity before joining. A worker or former engine owner with an
older generation cannot append receipt audit, acknowledge, create an order/outbox or project
another fact.

SQLite transaction ordering defines the freeze/send/ack races. If worker acknowledgement commits
first, the frozen snapshot sees `DeliveredAcked`. If reconciliation freeze commits first, the stale
worker's combined receipt-audit/ack transaction affects zero rows and appends no receipt audit. A
snapshot containing `Sending`, `AckPending`, `SendUncertain`, a recorded stale-generation late ack,
or any possible external-send-without-ack interval selects the one permanent
`QuarantinedUncertain` Terminal. `FrozenPending` proves no external claim began for that outbox and
is not delivery success. A frozen snapshot containing one or more `FrozenPending` rows and no
`Sending`/`AckPending`/`SendUncertain`/stale-ack ambiguity has one deterministic terminal outcome:
the reconciler preserves the complete positive order/outbox join, performs zero send/reprojection,
and appends `FailedEngine/reconciliation_frozen_pending_unsent`; it then CASes the attempt fence and
open row to `Terminalized`. It never calls that state `Succeeded`, `DeliveredAcked`, verified empty
or quarantine. If any uncertain delivery state coexists, uncertainty dominates and the one outcome
is `QuarantinedUncertain`. The reconciler never waits and guesses that a send did or did not happen.
The same ordering closes the engine/handoff/death races: an order/outbox commit either precedes and
is included in handoff/death high-water, or loses the owner-phase/generation CAS and writes nothing;
exclusive-lock acquisition cannot succeed against a live owner; and a former owner or reconciler
cannot mutate after a newer proof/claim generation commits.

The reconciler processes frozen attempts by
`admission_audit_ordinal ASC, attempt_identity_sha256 ASC`. For each attempt it joins only the fixed
snapshot facts after the frozen admission high-water marks, in this order:

1. acknowledged order-attempt audits by BR-201 attempt identity and order ordinal;
2. paper-order rows by exact order-attempt identity;
3. `PaperExitEventOutbox` rows by exact paper-order identity and the same `authorized_at`;
4. existing external-delivery audit projections by exact outbox identity.

`PaperExitJoinedOrderFactV1.fact_kind` is the closed enum with serialized values and sort ordinals
`OrderAttemptAudit=0`, `PaperOrder=1`, `EventOutbox=2`, `DeliveryAudit=3`; unknown, alias or
case-changed values fail decoding. The struct uses the exact declared field order
`order_ordinal,fact_kind,order_identity_sha256,order_attempt_audit_sha256,
paper_order_fact_sha256,outbox_fact_sha256,delivery_audit_sha256`. The four hash fields after order
identity are `Option<String>` and always serialize as lowercase 64-hex or literal JSON `null` under
this only valid matrix:

| `fact_kind` | attempt audit | paper order | outbox | delivery audit |
| --- | --- | --- | --- | --- |
| `OrderAttemptAudit` | value | null | null | null |
| `PaperOrder` | value | value | null | null |
| `EventOutbox` | value | value | value | null |
| `DeliveryAudit` | value | value | value | value |

For every committed order identity the frozen join contains exactly one row of each of the first
three kinds. It contains exactly one `DeliveryAudit` row only when one immutable external-delivery
audit exists; missing delivery is represented by absence of that fourth row, never an all-null
placeholder. A BR-086 rejection has no committed order identity and therefore appears only in the
attempt's separate rejection-audit set, not this committed-order join. The UNIQUE key is
`(attempt_identity_sha256,order_ordinal,order_identity_sha256,fact_kind)`; all rows sharing an order
must carry the byte-identical cumulative hashes required by the matrix. A missing mandatory kind,
extra/duplicate kind, delivery row without its three predecessors, null/value mismatch,
cross-attempt hash, broken parent link, order-without-outbox, outbox-without-order or ambiguous
commit evidence is invalid rather than guessed.

Canonical order is `order_ordinal ASC`, then `order_identity_sha256 BINARY ASC`, then the enum sort
ordinal above; no database locale or insertion order participates. Each row is compact UTF-8 JSON
from the Rust struct in declaration order, with no BOM/whitespace/newline. The join preimage is the
canonical sequence of an unsigned `u64` big-endian byte length followed by exactly those bytes.
`joined_order_facts_sha256` is lowercase SHA-256 of that whole byte sequence with no domain prefix,
separator or final newline. Thus a non-empty join can be independently rebuilt solely from the
frozen rows, while the zero-record value is exactly
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

The frozen non-empty vector uses order ordinal `1`, order identity `aa..aa`, attempt-audit hash
`bb..bb`, paper-order hash `cc..cc`, outbox hash `dd..dd` and the three mandatory rows (no delivery
row). Their canonical JSON byte lengths plus prefixes total 1,169 bytes and the required join hash
is `7deb3461c3c0e4ac1a8a771031bf25f42c556b301cfb531e4dea0b8133e07731`. Golden tests construct
the structs, not copied JSON text, and must reject a reordered enum, little-endian/decimal length,
null placeholder delivery, omitted null, trailing newline or any changed hash.

Before appending a reconciled Terminal, the exact reconciliation owner performs one preparation
transaction. It CASes
`engine_phase=ReconciliationOwned,reconciliation_claim_generation=g` to
`ReconciliationAuditPending` and freezes the snapshot hash, status/reason, canonical joined-fact
count/hash, nullable reconciliation-evidence hash, current audit ordinal and
`pending_terminal_payload_sha256`. That payload hash covers every Terminal field except the
global-chain-dependent `audit_ordinal`, `previous_record_sha256` and envelope `record_sha256`; it
therefore remains stable if another valid session audit advances the shared chain before recovery.
The transaction requires the attempt fence to remain `ReconciliationFrozen` with the exact same
owner and its independently stored `fence_generation=f`,
affects exactly one row, reads every frozen field back, and performs zero provider/order/outbox/
sink work. A crash before this CAS leaves `ReconciliationOwned`; a crash after it leaves the exact
recoverable `ReconciliationAuditPending` row. Neither state claims a Terminal.

The final reconciliation transaction accepts only the exact pending owner/claim generation and
the same-owner `ReconciliationFrozen` fence generation, revalidates the immutable snapshot and
payload hash, reads the current audit-chain tail, assigns the next
ordinal, constructs the canonical Terminal from the frozen payload, and atomically appends the
Terminal, inserts its projection intent, CASes the open row
`ReconciliationAuditPending -> Terminalized|Quarantined`, and CASes the attempt fence to the same
terminal class, plus the success-only persistent debounce CAS when applicable. Exact read-back
precedes acknowledgement. A constraint/hash/read-back/CAS failure
rolls back all effects and leaves `ReconciliationAuditPending` unchanged. Recovery of that state
does not re-freeze, re-query or rejoin facts and performs zero provider/order/outbox/sink/debounce
calls; the same owner may retry only this final transaction. If that owner dies, a later boot must
hold the namespace lock and append a new durable owner-death proof bound to the pending owner,
generation, snapshot and complete pending tuple and then execute the exact
`ReconciliationAuditPending -> ReconciliationAuditPending` same-phase takeover transaction above.
Age, retry
count or lease expiry never authorizes recovery.

A complete join with a sealed candidate and all deliveries `DeliveredAcked` appends that candidate
as the single normal Terminal. A complete fact set without a trustworthy sealed candidate appends
one conservative `FailedEngine` Terminal with exact reason `reconciled_complete_fact_set`; a
provably empty join appends one `FailedEngine` Terminal with `reconciled_no_committed_fact`. Any
complete join whose delivery states are limited to `{DeliveredAcked,FrozenPending}` and contain at
least one `FrozenPending` appends one
`FailedEngine/reconciliation_frozen_pending_unsent` Terminal as defined above. Any uncertain join
appends the one `QuarantinedUncertain` Terminal and CASes the latch and attempt fence
to permanent quarantine. In every case the Terminal append, open-row CAS, Terminal projection
intent, final fence state and applicable success-only debounce CAS are the final SQLite
transaction after the frozen `ReconciliationAuditPending` preparation. Automatic retry, order creation, external
re-projection and debounce advancement remain disabled after quarantine. Only the Controlled
Exception Path may later supersede that quarantine; it cannot rewrite its Terminal.

If reconciliation preparation fails, `ReconciliationOwned` remains unchanged; if final audit
hash/insert, projection-intent insert, read-back or terminal CAS fails,
`ReconciliationAuditPending` remains unchanged. No Terminal is claimed, the latch remains closed,
zero provider/order/outbox/sink/debounce work occurs, and the unique reconciler retries only the
frozen preparation or final audit step allowed by its exact phase. Startup cannot enable the engine until every
non-quarantined open attempt has an exact terminal join and no permanent quarantine remains.

#### Append-only controlled-exception resolution

A quarantine is never cleared by updating/deleting its Terminal, open row, fence, delivery row or
audit. The only Controlled Exception Path is a separate append-only, hash-chained
`paper_exit_quarantine_resolution_events` table in the same pinned namespace. Its closed event
sequence is `Opened -> Approved -> EvidenceRecorded -> PostmortemRecorded -> Closed`; every event
binds the original quarantine Terminal hash, previous resolution-event hash and resolution
identity. `Approved` must contain a non-reversible explicit-approver identity, non-empty reason and
risk statement, approval start/expiry, exact permitted corrective action and a postmortem deadline
no later than 24 hours. Missing, expired, reordered, duplicated or unknown events fail closed.

The permitted action set is only `MaintainDisabled`, `RecordExternalNoEffect` or
`RecordCompensatingFact`; none authorizes resend, order replay, a new paper fact, debounce advance,
test/live namespace crossing or bypass of AGENTS 2.5/2.6. Evidence and postmortem records bind their
own immutable audit hashes. Only one fully validated sequence ending in `Closed` may satisfy the
startup quarantine latch for future *new* attempts; the original `QuarantinedUncertain` Terminal
and delivery state remain permanent and every read model must display both facts. A later
correction is a new append-only compensating fact, never a rewrite or a claim that the original
delivery succeeded.

Verified non-executable and calendar rejection may write only their dedicated session-decision
audit record; their exact counter contract is `session_audit_db_calls=1`,
`debounce_db_reads=0`, `paper_ledger_db_calls=0`, `order_db_calls=0`, with zero account/provider/
bus/sink work. Debounce deferral also writes only the session-decision audit, but performs exactly
one persistent `debounce_db_read` and zero debounce writes; it performs zero account/ledger/provider/
order/outbox/sink work.

#### Frozen names and migration impact

This repair renames no BR ID, existing database table or existing audit event, but it explicitly
changes API, identity and storage contracts and therefore does not call them “unchanged.” The
side-effecting API moves to versioned `run_once_guarded_v1`; the old `run_once` and
`emit_sell_signal` signatures remain only as deprecated source-compatible fail-closed shims. The
legacy `plan_id` and V1 SHA are distinct business-order identities bound by the permanent dual-
identity migration above.

The following TSV is the exhaustive high-level identifier/wire/executable inventory for Gate B;
AC-13 parses between the markers and requires every proposed BR-201 token to classify exactly once.
`private-child` means no Rust visibility modifier; `pub(super)-child` is visible only to parent
`paper_engine`; `pub(crate)` never leaves this crate. `none` in alias policy forbids type aliases,
`use ... as`, re-exports, trait/function-pointer adapters and compatibility forwarding. All legacy
dispositions are total: `shim-zero-call`, `read-guard-only`, `reused-owner`, or `new`.

<!-- BR201_IDENTIFIER_INVENTORY_V1_BEGIN -->
```text
identifier	kind	visibility_owner	file_or_manifest	alias_policy	legacy_disposition
ContinuousTradingPermit	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
run_once_guarded_v1	function	pub(crate)-paper_engine	src/trading/paper_engine.rs	none	new
execute_paper_exit_tick_v1	function	pub(super)-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
register_account_provider_once_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
migrate_schema_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
decode_br201_sqlite_schema_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
begin_tick_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
admit_attempt_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
seal_account_acquisition_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
record_rejection_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
authorize_order_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
terminalize_attempt_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
reconcile_open_attempts_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
project_audit_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
deliver_outbox_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
advance_identity_cutover_v1	function	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
run_once	function	pub-paper_engine	src/trading/paper_engine.rs	none	shim-zero-call
emit_sell_signal	function	pub-paper_engine	src/trading/paper_engine.rs	none	shim-zero-call
Br134AccountEvaluationProviderV1	trait	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
Br134AccountEvaluationBatchV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
Br134AccountEvaluationErrorV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
Br201AccountAcquisitionEvidenceV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
Br201AdmittedAttemptAuthorityV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
Br201PrivateExecutionAuthorityV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
Br201ExitFinalOwnerV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
Br201PaperExitStore	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitAttemptResultV1	type	pub(crate)-paper_engine	src/trading/paper_engine.rs	none	new
PaperExitAttemptErrorV1	type	pub(crate)-paper_engine	src/trading/paper_engine.rs	none	new
PaperExitAttemptKeyV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitSessionRecordV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitReasonCodeV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitSuccessDebounceV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitMarketValidationV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitQuoteEvidenceV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
AdjacentChangeManualConfirmationV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
AdjacentChangeNotApplicableProofV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitBusinessIntentKeyV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitProposedOrderKeyV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
LegacyPaperExitCompatibilityDescriptorV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
LegacyPaperExitAliasInputV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitBusinessIdentityBindingV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitBusinessIdentityAliasV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitBusinessIdentityCutoverV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitOrderAttemptKeyV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitJoinedOrderFactV1	type	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
PaperExitBusinessIntentV1	wire-identity-scheme	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	new
LegacyPlanIdV0	wire-identity-scheme	private-child	src/trading/paper_engine/br201_exit_owner.rs	none	read-guard-only
paper.exit.session.audit	wire-event	private-child	src/event/envelope.rs	none	new
Br201ReleaseCommitV1	type	private-bin	src/bin/br201_evidence.rs	none	new
Br201OrderedCommitListV1	type	private-bin	src/bin/br201_evidence.rs	none	new
Br201BootstrapDescriptorV1	type	release-host-bootstrap	/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1	none	new
Br201PathAttestationV1	type	release-host-bootstrap	/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1	none	new
Br201CallerWorktreeManifestV1	type	release-host-bootstrap	/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1	none	new
Br201Stage1ContainmentReceiptV1	type	release-host-bootstrap	/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1	none	new
Br201Stage2PreparedRollbackV1	type	release-host-bootstrap	/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1	none	new
Br201DeploymentReceiptV1	type	private-bin	src/bin/br201_evidence.rs	none	new
Br201VerifiedRollbackInputsV1	type	private-bin	src/bin/br201_evidence.rs	none	new
Br201RollbackSourceManifestV1	type	private-bin	src/bin/br201_evidence.rs	none	new
Br201RollbackCommitListV1	type	private-bin	src/bin/br201_evidence.rs	none	new
Br201RollbackCommitV1	type	private-bin	src/bin/br201_evidence.rs	none	new
Br201RollbackAttestationV1	type	private-bin	src/bin/br201_evidence.rs	none	new
br201-evidence	executable	package-bin	Cargo.toml:[[bin]]name=br201-evidence,path=src/bin/br201_evidence.rs	none	new
verify_br201_rollback_manifest.py	secondary-executable	release-tool	tools/release/verify_br201_rollback_manifest.py	none	new
```
<!-- BR201_IDENTIFIER_INVENTORY_V1_END -->

The inventory payload is the 64 lines between the `text` fences joined by LF with no trailing LF.
Its exact SHA-256 over `stock_analysis.br201.identifier_inventory.v1\0 || payload` is
`0cd5307273d7579247b72d10b48fb21812b2d2feffeb6dbaa302c0713ab311c1`.

Inventory row types that decode SQLite rows have no alternate shape:
`PaperExitSuccessDebounceV1` is exactly the five columns of
`paper_exit_success_debounce`; `PaperExitBusinessIdentityAliasV1` is exactly the fourteen columns
of `paper_exit_business_identity_aliases`; and `PaperExitBusinessIdentityCutoverV1` is exactly the
eleven columns of `paper_exit_business_identity_cutover`, in manifest order.
`Br134AccountEvaluationErrorV1` is the child-private closed enum `Unavailable`, `Transport`,
`Cancelled`, `Unauthorized`, `MalformedResponse`, `ProtectedEvidenceStoreUnavailable`; each outcome
must still seal exact typed-error raw-byte evidence before mapping to the sole applicable account
reason. `Br201AdmittedAttemptAuthorityV1` privately owns exactly the admitted record hash, open-row
generation, attempt-fence generation, owner process-lock generation and consumed permit.
`Br201PrivateExecutionAuthorityV1` privately owns that consumed Admission capability plus the exact
account-acquisition/batch, quote, risk-validation and final owner-sample hashes. Debug, Serialize,
Deserialize, Clone, Copy, Default and public field/constructor implementations are forbidden for
both capabilities.

The inventory extractor is bounded to this design and the Gate-B staged slice. It collects
backtick/proposed Rust identifiers matching `[A-Z][A-Za-z0-9_]*(V1|Permit|Store)`, every inventory
function row (including the two guarded names, all private owner operations and legacy shims),
`br201-evidence`, all `paper_exit_*` SQLite identifiers from the
canonical schema manifest, all exact reason bytes from the one reason table, the two identity
schemes, and every `paper.exit.*` wire value. A planned TEST_CODE test
`br201_ac14_identifier_schema_inventory` parses the two marked TSV registries plus the schema and
reason tables, scans `Cargo.toml` and every staged BR-201-cited path, and emits exact counts
`unclassified=0 duplicate_owner=0 undeclared_alias=0 wrong_visibility=0 wrong_path=0`.
Adding/renaming/removing any token without changing this design, its golden inventory hash, its
owner/path/disposition, and the forward decoder is a Gate-A failure; future incompatible shapes
require v2 names.

Gate B migrations may only add the explicitly designed `paper_exit_*` tables/columns, append-only
triggers, UNIQUE/FK/CHECK constraints and forward-compatible decoders. They may not rename/drop an
existing table or audit event, rewrite an existing row, or backfill missing provider facts. Legacy
identity bytes remain immutable and are only bound to V1 by new append-only alias rows. A database
already containing legacy or BR-201 v1 rows must complete the dual-identity scan, debounce genesis/
singleton validation and open-attempt reconciliation before enabling; a partial migration, unknown
enum/reason/schema version or unresolved legacy identity fails closed.

Rule 2.10 ownership is intentionally two-phase. At Gate A the BR-201 row status contains the exact
substring `spec-only`, and its Code cell contains only the two current normative artifacts:
`docs/superpowers/specs/2026-08-01-paper-engine-session-gate-design.md` and
`docs/business_rules.md`. It must not list a future file as though implementation existed.

In this shared-worktree Gate-A repair, the BR-201 design is staged by this repairer, while the
shared `docs/business_rules.md` BR-201/BR-134 rows are intentionally left unstaged because the root
integrator owns the combined BR-196/BR-201/BR-202 business-rules stage. Therefore no intermediate
claim is made that the index copy of `docs/business_rules.md` equals this design or that Rule 2.10
passes on this repairer's partial index. Before requesting the independent Gate-A decision, the
root integrator must stage the exact reviewed business-rule rows in the same final docs-only
candidate, verify the design blob and row text against the worktree, and run Rule 2.10 on that
combined index. A missing/stale/foreign BR-201 row is an integration blocker, not evidence that
this design alone is a complete same-slice gate.

The planned Gate-B paths are the explicit `br201-evidence` bin target in `Cargo.toml`; runtime
orchestration `src/bin/monitor/main.rs`; verifier executable `src/bin/br201_evidence.rs`; session authority
`src/calendar.rs`; guarded evaluation and shims `src/trading/paper_engine.rs`; the sole private
authority, BR-201 migrations/store and final SQLite transaction owner
`src/trading/paper_engine/br201_exit_owner.rs`; generic-call separation only
`src/trading/paper_trade.rs`; safety `src/trading/order_safety.rs` and
`src/trading/risk_adapter.rs`; BR-086 chain ownership `src/database/order_audit.rs`; new SQLite
tables are created only by the child-module migration using generic descriptor-attested connection
primitives from `src/database/mod.rs`, which exposes no BR-201 write owner;
projection envelopes/locking `src/event/envelope.rs` and `src/event/dispatcher.rs`; user projection
`src/bin/monitor/push_templates.rs`; rollback verification
`tools/release/verify_br201_rollback_manifest.py`; and contract/integration tests
`tests/br201_paper_exit_gate.rs`.

The transition order is frozen per path and per staged slice: first create or modify the exact
planned file with an in-file `BR-201` citation; in the same working-tree slice update the BR-201 row
from `spec-only` to the next truthful status and add only that now-existing path to its Code cell;
then stage that file and `docs/business_rules.md` together; then run
`bash tools/compliance/lib/check_business_rules.sh` before any later path is started. A new path may
not be registered before it exists, an existing path may not be registered before its citation is
present, and a path change must update the row in the same staged slice. Gate B may begin only after
a fresh independent C0/I0 Gate-A review.

## Failure modes

- No unique real BR-134 account provider registered: capability Disabled before session
  construction, exact one-time provider-unavailable banner, and every BR-201 boundary counter stays
  zero; no positive canary is permitted.
- Verified non-executable session: provider-free skip, only the mandatory session-decision audit
  may be durably written, eligibility retained and no BR-134 failure diagnostic.
- Calendar authority invalid, unavailable, poisoned or outside coverage: typed fail-closed result,
  explicit warning without account/holding identities, zero paper side effects, eligibility
  retained.
- Executable session but five-minute debounce not due: exactly one session-audit write and one
  persistent debounce read, zero risk-context/account/ledger/provider/order work and no debounce
  write.
- Any executable tick that appends a decision before its debounce read, appends both defer and
  Admission, or changes the bound debounce version is corrupt and fails closed.
- Debounce table/row is absent after migration, genesis nullability/version is wrong, existing facts
  precede genesis, or restart attempts to reseed: startup remains Disabled with zero Admission.
- Account schema, provider authority, either timestamp, consecutive-stop-loss count, canonical
  BINARY order or proposed-exit position cardinality fails: use its sole `FailedAccountContext`
  reason from the closed registry after the acknowledged initial Admission and before any proposal
  or order authorization; never relabel it as pre-Admission or collapse it into
  `account_context_partial`, an engine reason or a free-form diagnostic.
- Legacy/V1 identity alias is missing/conflicting, either reservation is unexpired, either identity
  joins a confirmed/unresolved historical order, cutover high-water drifts, or the 60-second drain is
  incomplete: write the exact rejection audit where an attempt exists and commit zero reservation/
  order/outbox/delivery effects. Never suffix, guess, drop or age away confirmed history.
- Historical identity bytes are regenerated, legacy UTC reservation bytes are interpreted as local
  time, or the signed legacy timezone/date descriptor cannot be proven: capability remains Disabled
  as unresolved history. Restart, current-host timezone and rollback cannot alter that result.
- `LegacyOnly` never executes; qualified `DualReadV1Write` permits canary/production execution;
  qualified `V1PrimaryDualGuard` remains steady-state executable. Any implementation that disables
  solely because the monotonic transition succeeded, or stops the permanent legacy guard, fails.
- A caller invokes legacy `run_once` or `emit_sell_signal`: return the frozen deprecation error with
  every BR-201 boundary counter unchanged; no compatibility shim may call the guarded owner.
- A BR-201 caller constructs public `PaperSignal` or invokes public `paper_trade::simulate`: reject
  at static acceptance and runtime with zero BR-201 side effects. The three unrelated BR-134 public
  simulate callers retain their semantics and cannot construct the private BR-201 authority. Any
  sibling-visible low-level BR-201 migration/store/transaction function, or any placement of the
  private authority outside `paper_engine::br201_exit_owner`, is a static acceptance failure.
- Permit expires before the first provider call: fail closed with zero provider/order work.
- Permit expires after acquisition or between decisions: audit every already-proposed remaining
  ordinal as rejection, reject every post-boundary side effect, retain earlier immutable facts, fail
  the batch and keep immediate retry eligibility.
- Permit is valid at the atomic order-plus-outbox authorization but the wall clock crosses before
  commit or external projection: retain the pre-boundary fact pair and allow only its later
  projection; do not create or reauthorize another paper fact.
- Risk context, paper ledger, quote completeness or an order attempt fails while the permit remains
  valid: preserve the existing explicit BR-134 failure and retry semantics. At either quote
  boundary, future or age `>5s` rejects; exactly `5s` is valid. A second-boundary identity/source/
  authority/batch/capture/price/terminal-sample mismatch commits zero reservation, BR-086, order,
  outbox or delivery mutations and cannot be repaired by quote substitution.
- Every proposed order, including every rejection, requires exactly one immutable BR-086 record.
  Before the accepted transaction, the intended Confirmed BR-086 preimage is non-authoritative and
  unacknowledged; the private owner inserts and reads back that Confirmed row inside the same
  reservation/alias/order/outbox/delivery transaction. Rejection-audit failure leaves the attempt
  open with zero later order work.
- Open-attempt reconciliation has a broken or ambiguous order/audit/outbox join: append the unique
  permanent quarantine Terminal when audit is healthy, keep the capability latch closed and never
  retry/reproject automatically.
- Engine evaluation is sealed while one or more outboxes remain `Pending`: append no Terminal and
  leave only those never-claimed outboxes eligible for their first projection under the same open
  attempt generation.
- Reconciliation freezes an attempt containing any `Sending`, `AckPending` or `SendUncertain`
  delivery: append the one permanent quarantine Terminal; do not overwrite the delivery row, resend
  it or reuse the sealed normal-terminal candidate.
- Reconciler observes the current live owner's `Collecting`/`Sealed` attempt: skip it with zero CAS,
  audit, delivery or provider mutation; age/PID/timeout never upgrades eligibility.
- Clean owner handoff races an order/outbox mutation: the SQLite write order either includes the
  committed mutation in the handoff high-water or makes the stale mutation affect zero rows.
- Prior owner or reconciliation owner crashes: the next boot must hold the validated exclusive
  namespace lock and durably commit a new owner-death proof before takeover; frozen phases use the
  exact same-phase owner/claim/fence CAS and preserve their frozen/pending tuple. Unavailable proof
  keeps the engine latched closed indefinitely rather than using expiry.
- External receipt validation succeeds but receipt-audit/`DeliveredAcked` commit fails: commit
  neither effect, preserve `AckPending`/uncertainty and quarantine on freeze; never claim delivery
  from only one side of the pair.
- Admission/open/fence/projection-intent or Terminal/open-CAS/fence-CAS/projection-intent loses any
  member: the whole SQLite transaction aborts, no authoritative phase is acknowledged and restart
  sees no enqueue gap.
- Projection recovery sees an existing readable line: it still repeats file/directory `sync_all`
  and full-chain/exact-leaf validation under the file lock before `AppendAckPending -> Acked`;
  failure retains the last closed state or quarantines, never invents `Uncertain` or treats past
  readability as durability.
- Reconciliation preparation CAS fails: retain `ReconciliationOwned`; final audit/CAS fails:
  retain the exact frozen `ReconciliationAuditPending` fields. Emit no success claim and keep the
  engine disabled until the exact owner/generation (or death-proof successor) completes only its
  allowed frozen step.
- Projection ownership changes in `Claimed` or `AppendAckPending` without the exact same-state
  transition, exclusive namespace lock, consumed durable handoff/death proof, new owner and checked
  generation: reject as corruption. A CAS=0 loser performs the lock-internal winner reread and zero
  append; it never retries stale ownership.
- Successful empty or completed batch: advance the persistent five-minute debounce exactly once in
  the Terminal transaction.
- The absolute rollback bootstrap, root ownership/mode/immutable/package hash, signed descriptor,
  absolute object-root attestations, cleared environment, race-free no-follow FD open/execute, key,
  receipt, verifier, manifest or monitor hash cannot be proven before repository code runs: abort
  before worktree creation and keep the deployed capability Disabled. No ambient shell/tool
  fallback exists.
- Rollback cannot create and verify a clean detached worktree at the immutable signed source, its
  secondary Python verifier/interpreter does not match descriptor/manifest-bound raw-byte hashes,
  either pre/post canonical caller-worktree manifest differs, or any per-step parent/tree/inverse
  check fails: descriptor-delete only that rollback workspace, leave every caller tracked/untracked
  byte, symlink, mode, tombstone, index stage and HEAD/ref untouched, and keep capability Disabled.

## Old-module disposition

| Existing module/path | Disposition | Reason |
| --- | --- | --- |
| `calendar::can_trade_now()` / mutable `HOLIDAYS` | Reject as BR-201 authority; retain only for legacy non-order callers | Boolean/host-local/fail-open states cannot authorize a paper-order boundary. |
| checked-in official calendar and `verified_a_share_trading_day` | Adopt and deepen | Exact evidence above proves immutable fail-closed coverage and override isolation; add typed Shanghai session evidence and permit construction. |
| `main::PAPER_ENGINE_LAST_RUN` | Reject as authority; retain only as optional diagnostic | Replace the process-local `Instant` with the pinned persistent debounce row so restart/cross-process execution cannot bypass five minutes. |
| `trading::paper_trade::portfolio_state` | Reject for BR-201; retain for separately governed legacy consumers | It mixes local projection with caller price and cannot prove complete real same-batch cash/total/per-position fields or provider capture freshness. |
| `broker::QuoteProvider`, `real_account_snapshot`, `user_account_summary`, `user_position_snapshot` | Reject as BR-134 account provider; retain their existing quote/display/archive semantics | Current-code evidence proves the broker seam is public quote-only and account rows are user-attested/local without one atomic per-position broker response. No production owner exists; BR-201 remains Disabled until the minimal real provider contract lands separately. |
| `trading::order_safety::validate` / `risk::env_guard` | Adopt and deepen | Keep TEST_CODE/live rejection, price/lot/limit/notional/confirmation checks; add AGENTS 2.3 continuity/adjacent-change/split validation, enforce same-batch available cash for BR-201 sells and revalidate at the atomic use site. |
| `trading::risk_adapter::pre_trade_check` | Adopt without bypass | Keep AccountMode/DataMode/position/cash-floor enforcement against the same BR-134 batch; a permit cannot weaken it. |
| legacy `plan_id` plus `DatabaseManager::reserve_business_order_id` standalone call | Adopt opaque stored IDs/UTC rows as permanent read guards; reject as BR-201 writer | Historical ID bytes are never reconstructed. Bind them to V1 through immutable dual aliases; parse exact SQLite UTC reservation bytes; persist the signed legacy timezone/date-algorithm descriptor for new compatibility aliases; check old reservation/order/audit history forever. Only the private same-connection V1 owner writes new BR-201 reservations after cutover. |
| existing durable audit dispatcher and observation-only bus/JSONL writer | Retain only as projection mechanics | Exact evidence above proves their current separation and sync behavior. BR-201 authority is the same pinned SQLite store as open rows/order/outbox; file output mirrors committed records asynchronously and is never claimed atomic with SQLite. Append-success/SQLite-ack-loss recovery matches immutable record identity and never blindly duplicates a line. |
| existing TradingBus/sink direct publication from the engine result | Reject for BR-201 | Replace with an atomically committed `PaperExitEventOutbox` fact; external workers project only that immutable fact and cannot authorize execution. |
| ambient shell/Git/Python/OpenSSL rollback snippets | Reject completely | Replace with the absolute root-owned immutable `br201-rollback-bootstrap-v1`, signed absolute descriptor roots, cleared environment, no-follow FD hashing/execution, sealed inputs and byte-complete pre/post caller manifests. Repository helpers remain hash-pinned secondary validators only. |
| `STOCK_ANALYSIS_PAPER_EXIT_ENABLED` containment switch (TO BE BUILT) | Add as the first implementation commit | Only exact `1` enables a Gate-D-qualified guarded engine; `0`, missing or invalid values fail closed before session/account/provider and print one startup plus per-tick zero-call banner. |
| eager loop-wide `risk_context` | Reject and split | Exact evidence above proves that it currently feeds IntradayMonitor, the four-rule engine and 15:30 evening review. The four-rule path becomes session-gated/lazy; evening review acquires independently only inside its unchanged 15:30 branch; IntradayMonitor keeps its own contract. |
| public `paper_engine::run_once(PaperRiskContext)` and `emit_sell_signal` | Retain only as deprecated source-compatible fail-closed shims | Add `run_once_guarded_v1`; migrate every production caller and move side effects private. Old signatures return the frozen zero-call error so existing downstream source compiles but cannot bypass the permit. |
| public `paper_trade::simulate` and public `PaperSignal` | Retain for exactly three unrelated BR-134 production callers; reject as BR-201 authority | Remove the current four-rule call. The private child module `paper_engine::br201_exit_owner` co-locates `Br201PrivateExecutionAuthorityV1`, migrations/store and `Br201ExitFinalOwnerV1`, exposing only one high-level `pub(super)` operation to its parent; public signal fields, reason strings, aliases, traits, sibling database APIs and re-exports cannot reach a BR-201 commit. |
| BR-154 after-close branch in `paper_engine::load_open_positions` | Supersede for four-rule exit | Exact evidence above proves the current isolation branch. The guarded four-rule path never loads positions after close, and BR-154 cannot authorize an exit or introduce daily/cost/zero fallback. Remove the now-unreachable branch and update BR-154's code pointer unless a separately evidenced production consumer remains. |
| `IntradayMonitor` and 15:30 `evening_review` in the current intraday loop | Adopt calculations/trigger, split context ownership | Exact evidence above proves their present shared eager context and the 15:30 gate. Preserve their business semantics while moving evening-review acquisition inside its due branch and keeping them outside BR-201 authorization. |
| 19:00 `post_session_review_scheduler` and after-close valuation gate | Adopt existing entry semantics | Exact evidence above proves the separate 60-second/19:00 review scheduler and the additional closing-valuation eligibility check; BR-201 does not alter either gate. |

No old module, public re-export or compatibility shim may retain a second production route to the
sealed unguarded paper exit.

## Tests and acceptance evidence

Gate B must add TEST_CODE-only spies/fakes at injected boundaries; production paths remain wired to
the real clock, calendar, databases and providers. Each acceptance criterion has one named test,
one concrete command and one exact marker; emitting a marker before all assertions complete is a
test defect. Success requires command exit 0 and exactly one byte-equal marker from the table.

- **AC-01 Session authority:** assert the exact SSE source URI, actual artifact URI/raw-byte hash,
  parser binding and the eight exact Shanghai boundary seconds plus Auction,
  09:25-09:30, weekend and checked-in holiday rejection; the same UTC instant under three host
  `TZ` values must always classify with `+08:00`.
- **AC-02 Provider-free rejections:** invalid bytes/authority, poison, out-of-coverage and every
  verified non-executable session return their exact typed status/reason and call risk-context,
  paper-ledger DB, order DB, provider, bus/sink and debounce boundaries zero times without a BR-134
  failure, while each decision appends exactly one session-audit DB record.
- **AC-03 Gate order/account evidence:** debounce-not-due is downstream-zero-call; a due tick loads
  account context only after acknowledged Admission. Exact tests cover every cash/total/per-position
  field, provider source/capture identity, <=30-second checks at both use sites and aggregate
  reconciliation. Every schema/provider/parser/sign/scale/overflow/identity/source-time/local-time/
  consecutive-count/BINARY-order/quantity/accounting/bps/proposed-exit-join failure maps to its sole
  exact reason, including separate zero-join and multiple-join reasons; every cross-status or generic
  substitution is rejected. Otherwise-
  valid legacy `portfolio_state`, BR-151 SnapshotPaper, closing valuation,
  user-account and user-position rows still cannot authorize. Failures do not advance debounce.
- **AC-04 Sealed entry and time use-sites:** direct permit construction is impossible, the old
  public `run_once`/`emit_sell_signal` signatures still compile but return their exact zero-call
  deprecation error, have no production caller/re-export, and the only production caller reaches
  `run_once_guarded_v1`. Inventory proves the current four real `paper_trade::simulate` callers,
  removes only the four-rule call, preserves exactly three unrelated BR-134 callers, and proves
  public `PaperSignal`, simulate aliases, traits and re-exports cannot construct or receive
  `Br201PrivateExecutionAuthorityV1`. Visibility tests prove the private child module co-locates
  `Br201PrivateExecutionAuthorityV1`, `Br201ExitFinalOwnerV1`, migrations/store and final SQLite
  transaction, exposes only one high-level `pub(super)` operation to `paper_engine`, and exposes no
  low-level BR-201 database/write seam to a sibling. Expired/stale permits fail before provider. Deterministic
  clocks cross 11:30 and 15:00 at every pre-authorization gap; only a pair atomically committed with
  a pre-boundary owner sample may be projected later. Already-proposed remaining ordinals receive
  rejection-only BR-086 audits after expiry while all side-effect counts stay zero.
- **AC-05 Canonical audit and closed reasons:** assert both golden JSON byte strings/hashes and every
  status/field/reason positive row. Reject domain/NUL, field order, omission/null, integer/string,
  timestamp, rule-order, hash, sensitive-field, unknown reason and every cross-status reason
  mutation before acknowledgement. The exact record struct has items 1-11: first-record
  `previous_record_sha256=null`, every later record requires the exact prior lowercase 64-hex hash,
  and the envelope-only `record_sha256` can never enter its own preimage. Identity-reason tests prove
  the four disjoint conditions for within-60-second duplicate, already-confirmed, complete alias
  conflict and unresolved history; removed `business_order_id_duplicate` bytes fail decoding.
- **AC-06 Audit/projection atomicity:** failpoints before/after Admission audit, open row, initial
  fence, Admission projection intent, Terminal audit, open CAS, fence CAS, Terminal projection
  intent, success-only debounce CAS, commit and read-back prove each four-effect Admission and
  four-or-five-effect Terminal transaction is all-or-nothing. JSONL
  append failure leaves exactly one committed projection intent and no audit-only/row-only/enqueue-
  gap orphan. Append success followed by SQLite ack loss is recovered by exact record/projection
  identity with zero second append. Crash after write, flush, file `sync_all`, directory `sync_all`,
  post-sync revalidation, `AppendAckPending` CAS, Ack CAS and Ack read-back must resume in the exact
  closed state. Existing-line recovery must prove new file-sync + directory-sync + full-chain/leaf
  revalidation before Ack; missing/conflicting/multiple identities or any resync/revalidation
  failure quarantines and closes startup. Both legal same-state ownership takeovers are tested with
  exact lock/proof/new-owner/checked-generation predicates and atomic `proof=1,intent=1`; missing
  proof, unchanged owner/generation, an ineligible source state and CAS=0 loser append all fail.
- **AC-07 Order/outbox authorization:** missing/invalid session or order-attempt audit proof rejects
  inside the transaction owner. Exact tests cover production↔TEST_CODE rejection, positive price,
  continuity/gap/duplicate/split checks, within-20%, independently manually confirmed over-20% and
  provider-proven N/A adjacency states, and prove adjacent-change confirmation cannot substitute
  for RMB 500,000 confirmation or vice versa. They also cover 100-share lot, same-batch sell cash,
  RMB 1,000,000, source limits and the complete legacy/V1 identity cutover matrix. Tests start from
  legacy rows/reservations immediately before deployment, derive both byte-exact IDs, race two
  owners on either alias, cover truncation collisions, crash before/after both alias/reservation
  writes, and prove no same intent bypasses 60 seconds. They also cover confirmed/unresolved legacy
  history at arbitrary age, the locked high-water/backfill hash, generation monotonicity, exact
  60-second drain boundary, `DualReadV1Write -> V1PrimaryDualGuard`, restart and rollback without
  disabling production eligibility or the permanent legacy guard. Historical IDs remain opaque;
  exact SQLite UTC grammar, 59.999999999/exact-60-second boundaries, signed legacy timezone/date
  descriptor, UTC/legacy-zone midnight, restart under a changed `TZ` and rollback vectors all retain
  identical alias/expiry outcomes. Both quote boundaries
  independently reject a one-nanosecond future capture and ages `5_000_000_001ns`, accept
  `4_999_999_999ns` and exactly `5_000_000_000ns`, and bind byte-identical quote identity,
  instrument, provider source/authority/batch, source capture, price and terminal owner sample.
  Mutating any bound field between checks leaves reservation/BR-086/order/outbox/delivery rows all
  zero. Crash/restart before
  expiry must reject the unchanged intent; changed position fact and post-expiry reuse follow their
  exact outcomes. Before the accepted transaction the intended Confirmed BR-086 preimage is proven
  non-authoritative and the durable Confirmed-row count remains zero; only its in-transaction insert
  and read-back hash may bind reservation confirmation and later effects. Only then may reservation, BR-086 audit carrying the 2.3 proof, order, event
  outbox and initial Pending delivery row commit together with one authorization time and no direct
  bus publication.
- **AC-08 Split multi-outbox delivery:** zero, one and three-outbox attempts prove one attempt fence
  plus one delivery row per outbox. Worker claims alter only one delivery row. Receipt audit plus
  `DeliveredAcked` commit atomically; failure leaves neither side falsely complete. `Sending`,
  `AckPending` and `SendUncertain` are never attempt-fence states or resendable states.
- **AC-09 Single Terminal convergence:** zero-outbox sealed candidates and all-acked multi-outbox
  candidates produce one normal Terminal only after convergence. Any sending/ack uncertainty before
  Terminal produces the one quarantine Terminal; after a normal Terminal every send/ack mutation is
  rejected, so no second or replacement Terminal is possible.
- **AC-10 Restart reconciliation:** cover zero, one and multiple attempts in every engine/fence/
  delivery state, fixed order, exact no-fact/complete/broken joins, persistent quarantine and
  every nullable/non-nullable field combination and CAS/crash boundary of
  `ReconciliationAuditPending`. Prove recovery uses only its frozen snapshot/payload and performs
  zero rejoin/provider/order/sink calls. Prove live same-/other-boot `Collecting` and `Sealed` are never frozen;
  prove `FrozenPending` without uncertainty terminates exactly as
  `FailedEngine/reconciliation_frozen_pending_unsent`, while any coexisting uncertainty quarantines;
  only exact handoff or exclusive-lock-backed durable prior-owner-death proof makes an attempt
  eligible; no age/PID/wall-clock/monotonic-time lease can take over. Repeated reconciliation performs
  zero provider/order/sink/reproject, and Terminal/CAS/projection-intent remains one transaction.
  Crash before/after each death-proof/open-row/fence/proof-consumption statement and commit/read-back
  proves an all-old or all-new tuple for same-phase `ReconciliationOwned` and
  `ReconciliationAuditPending` takeover; pending fields are preserved exactly and Terminal remains
  absent until the separately authorized final transaction.
- **AC-11 Cross-process races:** race engine order/handoff, live-owner/death-proof acquisition,
  handoff/reconcile, worker claim/freeze, freeze/send, acceptance/ack, stale-generation late owner or
  ack, snapshot/new outbox and restart in `Pending`, `Sending`, `AckPending`, `DeliveredAcked`,
  `SendUncertain` and `FrozenPending`. Crash a `ReconciliationOwned` boot and prove takeover requires
  a new durable death proof rather than expiry. Race two successor reconcilers against both frozen
  phases: exactly one increments owner/claim/fence generations and consumes its proof, the loser
  writes zero rows, stale generations cannot prepare/finalize, and a crash-after-commit successor
  needs a proof for the newly recorded owner. Prove the transaction-order outcomes above,
  immutable snapshot/pending tuple and at most one Terminal/quarantine when ambiguity exists.
- **AC-12 Rollback/forensics:** a post-deep revert parses every v1 status/reason/optional field,
  rejects unknown future bytes, blocks every latch state, validates append-only quarantine
  resolution chains and validates the exact source manifest, signed post-build attestation and
  deployed binary hashes. It independently derives the trailer-classified Git range, exact typed
  ordered-commit preimage/domain+NUL and golden hash; missing/duplicate/foreign/merge/reordered
  commits, invalid trailers, class projection drift and any source commit whose sole parent is not
  `implementation_tip` all fail before canary authority. It independently reconstructs the ordered
  revert list and its golden domain/hash, requires a no-merge exact first-parent chain from the
  verified deployed source, applies each original deep full-index binary patch in reverse without
  fuzz/three-way/rename inference, and matches every actual tree and inverse-patch hash. Reordered,
  omitted, duplicate or extra revert commits, wrong deep target, parent/tree drift, empty inverse,
  submodule/mode/path drift and any piggyback byte fail before signing or deployment. It creates a
  new clean detached worktree at the verified rollback base, proves exact HEAD/index/worktree state,
  performs and verifies every revert there, cleans it on failure and proves byte-equal pre/post
  `Br201CallerWorktreeManifestV1` across caller HEAD/ref, raw index/stages/tree, every tracked/
  untracked content hash, symlink target, mode, gitlink and tombstone. The
  disabled canary compares real high-water/counters and rejects a forged log-only success. Before
  any dirty-worktree code runs, mutation tests reject a changed root-owned key/hash, active receipt,
  receipt signature/domain, content-addressed manifest/verifier/monitor hash, signed source commit or
  immutable allowed-commit list. A dirty caller script cannot choose the base. The clean detached
  secondary Python verifier must match the signed source-manifest blob/raw-byte hashes and may only
  revalidate the immutable inputs; a changed script or attempt to override base/commits fails.
- **AC-13 Success-only debounce and preserved scheduler gates:** verified empty and non-empty
  successful attempts each advance the persistent debounce exactly once in their Terminal
  transaction. Crash before/after Terminal commit, immediate restart, a second process and
  backwards-clock cases prove there is no process-local bypass or double advance. Fresh migration
  creates the exact version-one all-null genesis once; first executable use is due and binds version
  one; first success atomically creates version two/all-non-null. Partial/multiple/missing genesis,
  existing facts without state, reseed, rollback reset and version overflow all fail closed. The 15:30
  evening-review trigger and separate 19:00 post-session/closing-valuation gates remain exact while
  context acquisition is split as specified.
- **AC-14 Identifier/schema closure:** parse the complete identifier and SQLite manifests, exact
  `Cargo.toml` bin source path, all wire/reason/identity values, owner operations, migration order,
  rollback decoder, aliases, visibility and legacy dispositions. Scan every BR-201-cited Gate-B
  path with the bounded extractor and require no unclassified name, undeclared table/column/key/
  index/trigger, second owner, wrong path/visibility, low-level parent operation, or alias/re-export.
  Golden schema/trigger/inventory hashes and mutation tests make every row and attribute executable.

The seventh-review C1/I7/M1 negative matrix is mandatory in those named ACs: Rule 2.10 first proves the
unique row is truly `spec-only`, lists only the two existing docs and rejects every future path until
the same staged slice creates/cites/registers it; AC-03 rejects decision-before-debounce,
two decisions per tick, wrong debounce version, every decimal/tie/sign/scale/overflow and bps
residual mutation, every newly enumerated schema/count/order/time/join failure, verifies the
one-to-one account reason/provenance matrix, raw response/error evidence separation and every
non-real account fallback; AC-04 inventories the one full-tick operation, transaction-minted
Admission capability, both
legacy public shims plus all four current simulate callers and rejects every authority
alias/re-export/bypass;
AC-05 rejects unknown/aliased/duplicate
reason encodings, the removed `business_order_id_duplicate` bytes and every cross-substitution among
the four exact identity conditions, and requires the sole `manual_confirmation_required` mapping; AC-06 crashes
successive JSONL claim owners, races two recoverers and proves the CAS=0 loser lock-rereads the
winner with zero append, while both same-state takeover forms require their exact proof/new-owner/
generation tuple; AC-07 requires exactly one rejection audit per proposed order, proves an intended
Confirmed BR-086 preimage is not an acknowledged fact before the accepted transaction, the exact
two-alias cardinality plus six-order-effect accepted transaction and the full legacy/V1 migration,
opaque legacy code/reason bytes and closed trigger projection, exact proposed-exit total sort,
UTC/tz/midnight state eligibility, restart and rollback matrix; AC-10 covers skipped process generations across three or more
crashed boots plus every joined-fact enum/null/cardinality/order/hash mutation; AC-12 rejects local
build authority, any non-absolute/mutable/ambient-tool rollback bootstrap, dirty-script trust,
root-key/receipt/object-hash/descriptor-root drift, pre/post raw caller-manifest drift,
dirty/wrong-base/caller-worktree mutation and every rollback list/parent/inverse-
patch/tree/signature/original-attestation/artifact mismatch; AC-13 covers genesis migration and
reseed mutations; AC-14 requires exact manifest classification with `unclassified=0`. A test
marker emitted without every listed negative assertion is invalid.

| AC | Exact command | Exact required output marker |
| --- | --- | --- |
| AC-01 | `cargo test --bin monitor br201_ac01_session_authority -- --nocapture --test-threads=1` | `BR201_AC01 status=PASS authority_artifact_binding=exact raw_hash=ef9044635e9fc7475efcc1972961fd5306a9cbb28e052e91997f132e6da413d5 boundary_matrix=complete host_tz_invariant=true` |
| AC-02 | `cargo test --bin monitor br201_ac02_provider_free_rejections -- --nocapture --test-threads=1` | `BR201_AC02 status=PASS typed_rejections=complete session_audit_db_calls=1 paper_ledger_db_calls=0 order_db_calls=0 downstream_calls=0 br134_failures=0` |
| AC-03 | `cargo test --bin monitor br201_ac03_gate_order_account_evidence -- --nocapture --test-threads=1` | `BR201_AC03 status=PASS read_before_decision=true decisions_per_tick=1 account_schema=complete acquisition_response_error_evidence=separate reason_specific_provenance=exact raw_sensitive_leaks=0 account_reason_mapping=total_one_to_one schema_count_order_time_join_reasons=distinct fen_bps_vectors=complete same_batch=true freshness_le_30s=true production_provider=disabled forbidden_fallbacks=7 downstream_calls=0` |
| AC-04 | `cargo test --bin monitor br201_ac04_sealed_entry_time_use_sites -- --nocapture --test-threads=1` | `BR201_AC04 status=PASS permit_boundary=sealed admitted_authority=transaction_minted_nonclone full_tick_owner=execute_paper_exit_tick_v1 private_execution_authority=unforgeable private_owner_module=co_located parent_high_level_ops=1 sibling_low_level_ops=0 guarded_callers=1 legacy_shims=2 simulate_callers_before=4 br201_simulate_callers_after=0 unrelated_simulate_callers_after=3 legacy_side_effect_calls=0 crossings=complete expired_proposals_audited=all unauthorized_rows=0` |
| AC-05 | `cargo test --bin monitor br201_ac05_canonical_audit_reason_registry -- --nocapture --test-threads=1` | `BR201_AC05 status=PASS golden_hashes=2 record_fields=29 account_acquisition_evidence=separate reason_specific_provenance=exact debounce_version_cardinality=exact previous_record_cardinality=exact manual_confirmation_reason=exact identity_reason_conditions=4 removed_reason_rejected=true reason_registry=closed cross_status_rejections=complete` |
| AC-06 | `cargo test --bin monitor br201_ac06_audit_projection_atomicity -- --nocapture --test-threads=1` | `BR201_AC06 status=PASS admission_effects=4 terminal_effects=4_or_5 debounce_atomic=true enqueue_gaps=0 ack_loss_reappends=0 projection_states=5 same_state_takeovers=2 claim_recovery_winners=1 claim_loser_appends=0 recovery_resync_revalidate=true` |
| AC-07 | `cargo test --bin monitor br201_ac07_order_outbox_authorization -- --nocapture --test-threads=1` | `BR201_AC07 status=PASS rule_2_3_2_5_2_6_br084_br086=complete adjacent_confirmation_distinct=true quote_boundaries=2 quote_5s_inclusive=true quote_future_rejected=true quote_binding=exact proposed_exit_total_sort=exact duplicate_sort_keys=rejected identity_schemes=2 identity_reason_conditions=4 historical_identity=opaque legacy_code_reason_bytes=persisted_exact trigger_reason_projection=closed legacy_timestamp=sqlite_utc_exact legacy_timezone_descriptor=verified dual_alias_unique=true cutover_states=3 executable_states=2 v1_primary_steady_state=true drain_60s=exact historical_guard=permanent cutover_races=complete rejected_orders_audited=all pretransaction_confirmed_audits=0 naked_reservations=0 accepted_effects=6 direct_bus_calls=0` |
| AC-08 | `cargo test --bin monitor br201_ac08_split_multi_outbox_delivery -- --nocapture --test-threads=1` | `BR201_AC08 status=PASS attempt_fence_states=4 delivery_states=7 receipt_ack_atomic=true` |
| AC-09 | `cargo test --bin monitor br201_ac09_single_terminal_convergence -- --nocapture --test-threads=1` | `BR201_AC09 status=PASS normal_after_ack=true quarantine_on_uncertainty=true max_terminals=1` |
| AC-10 | `cargo test --bin monitor br201_ac10_restart_reconciliation -- --nocapture --test-threads=1` | `BR201_AC10 status=PASS eligibility_matrix=complete frozen_takeover_states=2 generation_gaps=accepted takeover_crash_matrix=complete joined_fact_kinds=4 joined_hashes=2 pending_fields_preserved=true collecting_freezes=0 lease_takeovers=0 duplicate_terminals=0 reprojections=0` |
| AC-11 | `cargo test --bin monitor br201_ac11_cross_process_races -- --nocapture --test-threads=1` | `BR201_AC11 status=PASS race_matrix=complete competing_reconciler_winners=1 live_owner_death_proofs=0 stale_owner_writes=0 stale_acks_committed=0 duplicate_terminals=0 uncertain_resends=0` |
| AC-12 | `cargo test --bin monitor br201_ac12_rollback_forensics -- --nocapture --test-threads=1` | `BR201_AC12 status=PASS v1_decoder=compatible ordered_commit_preimage=exact ordered_commit_hash=5b80352e80109d585e8cdce45f308454760f5d62ada9af76192ddce6ed72d595 source_parent=exact deployment_receipt=verified bootstrap_absolute_root_owned_immutable=true bootstrap_env=sanitized_no_ambient_tools object_roots=absolute_descriptor_attested open_execute=race_free_fd immutable_verifier_hash=exact dirty_bootstrap_authority=rejected deployed_before_revert=verified detached_worktree=clean_exact_base secondary_verifier_hash=exact caller_manifest=raw_tracked_untracked_symlink_mode_tombstone caller_worktree_mutations=0 per_step_parent_tree_inverse=exact local_rebuild_authority=rejected rollback_commit_list_hash=1d137a5dc6a36a62bdf37045d29bc215c60515f777b605af6eccb016ab5d5abd rollback_parent_chain=exact inverse_patch_trees=exact piggyback_changes=0 rollback_signature=exact rollback_artifacts=exact forged_banner_rejected=true` |
| AC-13 | `cargo test --bin monitor br201_ac13_debounce_preserved_scheduler_gates -- --nocapture --test-threads=1` | `BR201_AC13 status=PASS debounce_genesis=version_1_all_null first_due=true first_success_version=2 reseeds=0 persistent_debounce_advances=2 restart_bypasses=0 crash_double_advances=0 preserved_scheduler_gates=2 eager_shared_contexts=0` |
| AC-14 | `cargo test --bin monitor br201_ac14_identifier_schema_inventory -- --nocapture --test-threads=1` | `BR201_AC14 status=PASS unclassified=0 duplicate_owner=0 undeclared_schema_items=0 undeclared_alias=0 wrong_visibility=0 wrong_path=0 parent_high_level_ops=1 schema_version=1 migration_order=exact rollback_decoder=retained` |

Required verification includes:

```bash
cargo test --bin monitor br201_ac -- --nocapture --test-threads=1
cargo test --lib paper_engine -- --test-threads=1
cargo test --lib calendar -- --test-threads=1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor --bin br201-evidence
```

The exact command-level acceptance predicates are: the filtered BR-201 command emits each of the
thirteen table markers exactly once and no `status=FAIL`; the paper-engine/calendar/full-workspace
commands exit 0 and every emitted `test result:` has `0 failed`; formatting exits 0 with no diff;
Clippy exits 0 with zero `warning:`/`error:` diagnostics; compliance exits 0 with final line exactly
`[compliance] ALL CHECKS PASSED`; coverage generation exits 0 and the threshold command prints the
two exact-format lines `global line coverage: <covered>/<count> = <pct>% (required 80.00%)` and
`core line coverage: <covered>/<count> = <pct>% (required 95.00%, <files> files)` with both reported
percentages at or above their requirement; the release build exits 0 and produces both executable
files. A broad command cannot substitute for a missing AC marker.

Gate D additionally requires the repository coverage gates, release build, an independent review
with no Critical/Important findings, and bounded production runtime evidence. Production startup
must print exactly one non-sensitive BR-201 mode banner. Because the repository audit above found
no real `Br134AccountEvaluationProviderV1`, current capability is Disabled and Gate D is blocked;
neither a design type, fixture nor local projection may be used to run the positive canary. After a
separate upstream change registers the unique real provider, an executable canary must demonstrate
the guarded entry and a closed-session canary must demonstrate a provider-free skip. No PushKind
or external notification is fabricated solely to prove this gate.

The containment switch is evaluated before scheduler/session construction. When
`STOCK_ANALYSIS_PAPER_EXIT_ENABLED` is missing or is not the exact bytes `1`, production prints
exactly one startup line:

```text
[BR-201] paper_exit mode=disabled reason=enable_switch_not_exact_1 session_calls=0 account_calls=0 paper_ledger_db_calls=0 provider_calls=0 order_db_calls=0 outbox_calls=0 push_log_calls=0
```

Every disabled scheduler tick additionally emits its registered non-sensitive skip warning, but a
log line is never permission or zero-call proof. The switch path must not initialize the session,
account, ledger, provider, order, outbox or push-log façades. Exact `1` prints one enabled startup
line with `source_authority_uri`, `artifact_uri`, `calendar_version` and
`artifact_raw_bytes_sha256` only after register-once provider preflight proves exactly one real
production owner. With the current audited gap, exact `1` instead prints the frozen
`br134_account_evaluation_provider_unavailable` Disabled banner above before session construction.
No account/holding/target identity is printed.

If and only if an already committed `PaperExitEventOutbox` requests a user-visible projection, its
delivery uses `PushKind::PaperTrade` / stable template ID `paper_trade_v1`. Physical acceptance must
produce exactly one non-overwriting artifact matching
`data/push_log/<YYYY-MM-DD>/HHMMSS_<unique-audit-suffix>.md`. Its exact first four lines are
`[PaperTrade]`, `BR201-Attempt: <attempt_identity_sha256>`,
`BR201-Outbox: <outbox_identity_sha256>` and
`BR201-Delivery: <delivery_identity_sha256>`; the artifact hash is stored in the same delivery
receipt audit. A projection that does not produce this exact artifact and its receipt join is not a
confirmed delivery. No-data/failed/quarantined attempts create no PaperTrade push log.

The SQLite session-record projection uses JSONL `event_type="paper.exit.session.audit"`,
`source="br201_paper_exit_store"` and `identity_hash=<record_sha256>` in
`data/event_bus/<YYYY-MM-DD>.jsonl`. The externally accepted notification additionally has one
synchronous durable `event_type="push.delivery.audit"`,
`source="br201.paper_exit_outbox"`, `stable_template_id="paper_trade_v1"`; its immutable
`identity_hash` is SHA-256 over
`stock_analysis.br201.paper_exit_delivery.v1\0` plus the lowercase attempt hash, outbox hash and
validated receipt hash in that order. Gate D joins these exact identities to the SQLite Admission,
order, outbox, delivery row, receipt audit and push-log artifact; a log-only line, JSONL-only line,
unjoined generic PaperTrade push or recomputed local counter does not pass.

The canaries use only counters inside the BR-201 façades and durable store high-water; whole-process
network/log counts cannot substitute. `br201-evidence assert-session` reads the same pinned calendar
bytes as the monitor and refuses the wrong current session, unavailable authority, release mismatch,
open/quarantined preflight or TEST_CODE/production namespace alias. `wait` exits nonzero on timeout
or if a counter exceeds its requested exact delta. These commands are TO BE BUILT in Gate B and are
the exact Gate-D production commands, run against the same tracked source manifest and signed
post-build attestation.

The following positive command is a future Gate-D acceptance command, not a currently runnable
claim. It remains prohibited until the upstream provider contract is implemented, independently
reviewed, production-registered and its Disabled banner is replaced by the enabled preflight
banner. Then, during one verified continuous A-share session, run exactly one admitted guarded
attempt:

```bash
./target/release/br201-evidence assert-session --namespace production \
  --expected continuous --source-manifest docs/releases/br201/source-release.json \
  --attestation-root data/release_attestations/br201
./target/release/br201-evidence snapshot --namespace production \
  --output /tmp/br201-gate-d-executable-before.json
MONITOR_ENABLED=true STOCK_ANALYSIS_PAPER_EXIT_ENABLED=1 ./target/release/monitor \
  > /tmp/br201-gate-d-executable.log 2>&1 &
br201_executable_pid=$!
./target/release/br201-evidence wait --namespace production \
  --before /tmp/br201-gate-d-executable-before.json \
  --exact-guarded-entry-delta 1 --exact-terminal-delta 1 \
  --minimum-account-call-delta 1 --minimum-ledger-call-delta 1 \
  --minimum-provider-call-delta 1 --minimum-paper-order-delta 1 \
  --minimum-outbox-delta 1 --minimum-delivered-ack-delta 1 \
  --minimum-push-log-delta 1 --timeout-secs 120
kill -INT "$br201_executable_pid"
wait "$br201_executable_pid"
./target/release/br201-evidence snapshot --namespace production \
  --output /tmp/br201-gate-d-executable-after.json
./target/release/br201-evidence verify-startup \
  --log /tmp/br201-gate-d-executable.log --expected-enabled 1
./target/release/br201-evidence verify-executable-canary \
  --before /tmp/br201-gate-d-executable-before.json \
  --after /tmp/br201-gate-d-executable-after.json \
  --log /tmp/br201-gate-d-executable.log \
  --push-log-root data/push_log --event-bus-root data/event_bus
```

The three exact stdout lines required from `assert-session`, `verify-startup` and the final verifier
are, respectively:

```text
BR201_SESSION status=PASS expected=continuous authority_binding=verified release_binding=exact
BR201_GATE_D_STARTUP status=PASS banners=1 authority_binding=verified sensitive_fields=0
BR201_GATE_D_EXECUTABLE status=PASS guarded_entries=1 unguarded_entries=0 admissions=1 terminals=1 duplicate_terminals=0 account_path=real account_same_batch=true account_fresh_le_30s=true provider_path=real br084_br086=complete order_outbox_join=positive push_log_join=positive typed_failure_only=false
```

The final verifier additionally rejects a terminal not joined to that single Admission, a direct
bus call, a missing handoff/death proof used by reconciliation or a counter reset. It requires a
truthful `Succeeded` Terminal with a positive exact order→outbox→`DeliveredAcked` join and positive
counter deltas from the real BR-134 account, ledger, quote-provider, order and outbox façades.
It also requires the Terminal account provenance to equal the account batch consumed by every
BR-084/BR-086 order row, source capture age <=30 seconds at both gates, one persisted cross-boot
business-order reservation, the exact 2.3 validation proof, one matching
`paper.exit.session.audit` JSONL identity, one matching durable `push.delivery.audit` identity and
the one hash-joined `[PaperTrade]` push-log artifact.
`SucceededEmpty`, any typed failure, a fixture, synthetic holding/order/provider response or a
log-only counter cannot pass. If no real exit candidate occurs during the bounded window, Gate D
remains blocked rather than fabricating one.

Second, during a verified non-executable session, observe exactly two ordinary scheduler ticks and
prove every BR-201 downstream boundary stayed provider-free:

```bash
./target/release/br201-evidence assert-session --namespace production \
  --expected non-executable --source-manifest docs/releases/br201/source-release.json \
  --attestation-root data/release_attestations/br201
./target/release/br201-evidence snapshot --namespace production \
  --output /tmp/br201-gate-d-closed-before.json
MONITOR_ENABLED=true STOCK_ANALYSIS_PAPER_EXIT_ENABLED=1 ./target/release/monitor \
  > /tmp/br201-gate-d-closed.log 2>&1 &
br201_closed_pid=$!
./target/release/br201-evidence wait --namespace production \
  --before /tmp/br201-gate-d-closed-before.json \
  --exact-scheduler-tick-delta 2 --timeout-secs 90
kill -INT "$br201_closed_pid"
wait "$br201_closed_pid"
./target/release/br201-evidence snapshot --namespace production \
  --output /tmp/br201-gate-d-closed-after.json
./target/release/br201-evidence verify-closed-canary \
  --before /tmp/br201-gate-d-closed-before.json \
  --after /tmp/br201-gate-d-closed-after.json \
  --log /tmp/br201-gate-d-closed.log --exact-observed-ticks 2 \
  --push-log-root data/push_log --event-bus-root data/event_bus
```

The exact required stdout lines are:

```text
BR201_SESSION status=PASS expected=non-executable authority_binding=verified release_binding=exact
BR201_GATE_D_CLOSED status=PASS scheduler_ticks=2 skipped_non_executable=2 session_audit_db_calls=2 admissions=0 account_calls=0 paper_ledger_db_calls=0 provider_calls=0 order_db_calls=0 outbox_calls=0 push_log_calls=0 unguarded_entries=0
```

Any extra tick, Admission, downstream-call delta, BR-134 context-unavailable diagnostic, counter
reset/wrap, namespace drift, unmatched audit ordinal or missing exact marker fails Gate D. Capturing
only startup/log text is insufficient. Both canaries and the independent 0C/0I review must refer to
the same source manifest, signed post-build attestation and deployed binary hashes.

## Required PR evidence

Every Gate-B staged slice and the final release PR must include all AGENTS 3.1 fields. `Refs` names
this design and the exact section changed. `Data-Redlines` includes at least 2.1, 2.2, 2.3, 2.4,
2.5, 2.6, 2.7, 2.8 and 2.10. Its 2.3 evidence states that positive price,
time-continuity/gap/duplicate validation, split/dividend consistency and the distinct
adjacent-change manual-confirmation/provider-proven-N/A capabilities are enforced at both required
quote boundaries without fixed-percentage rejection or missing-data fallback. `OldModules` copies the applicable rows above, including explicit migration
of `run_once_guarded_v1`, both legacy shims, public `paper_trade::simulate`, the private execution
owner/full-tick Admission capability, protected account acquisition evidence, exact proposed-exit
sort, legacy raw-byte alias input, canonical SQLite v1 manifest/rollback decoder and immutable
rollback bootstrap; none may be labelled unchanged. `Threshold-Proof`
states that no configurable trading threshold changed and separately binds the fixed 5-second,
30-second, 60-second and 300-second contracts to their cited rules. `Business-Rules` lists BR-201,
the scoped BR-134 clause, BR-084 and BR-086. `Rollback` names the absolute root-owned
`/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1`, signed descriptor and opaque Stage-1/2
receipts, and identifies the reviewed source/release manifest rather than a local build or shell
snippet. The PR also attaches AC-14 `unclassified=0` output and the final root-staged Rule 2.10
result; this repairer's intentionally partial shared-file index is not merge evidence.

The PR evidence must also state that no production `Br134AccountEvaluationProviderV1`, real BR-201
Disabled banner, 2026-08-02 push/event artifact or fresh `<=30s` real-account batch exists in the
current tree. Tests, golden hashes and this design are not substitutes, so Gate D remains blocked.

## Canonical BR-201 registry text

The following paragraph is the canonical BR-201 description and must remain byte-for-byte equal to
the description cell of the unique BR-201 row in `docs/business_rules.md`:

四铁律 paper exit 仅由 SSE 官方日历、固定 Asia/Shanghai `+08:00`、原始日历字节 SHA-256 与 child-private `ContinuousTradingPermit` 授权，仅 `[09:30,11:30)` 和 `[13:00,15:00)` 可执行；legacy bool、host Local、可变假日、损坏、poison、越界或五分钟 persistent debounce 未到期均失败关闭。唯一 production entry 为 `run_once_guarded_v1`，它只把 typed observation 交给唯一 `pub(super)` `execute_paper_exit_tick_v1`；该 single full-tick owner 独占 session、debounce、initial decision、Admission、lazy provider、validation、reconciliation 与 final order transaction。Admission transaction 成功并 read back 后才 mint 不可 Clone/不可序列化的 `Br201AdmittedAttemptAuthorityV1`，最终 owner 必须消费 exact admitted-record/open-row/fence tuple；旧 `run_once`/`emit_sell_signal` 仅 zero-call deprecated shim，public `PaperSignal`、`paper_trade::simulate`、provider/store/connection/re-export 均无 BR-201 authority。`Br134AccountEvaluationBatchV1` 只接受唯一 register-once 真实 provider 的单一原子 response，Admission 后才可调用且 admission/final use-site 均须 `<=30s`；当前无 production provider，capability 必须 Disabled、输出 frozen zero-call banner，Gate D positive canary 继续 blocked。raw provider response 或 typed error 仅进入 protected append-only `Br201AccountAcquisitionEvidenceV1`，session audit 只存 evidence identity/hash；batch/provider/authority/source-time 四字段按每个 `FailedAccountContext` reason 独立验证并 preserve-or-null，禁止 all-present fiction、补值、敏感原文进入 audit/JSONL/log/PR。账户 schema、provider authority、source/local time、same-batch、decimal/fen、资产/仓位/bps、连续止损、position BINARY order 与 proposed-exit join 的失败一对一映射；proposed exits 按 `PaperExitProposedOrderKeyV1` 六字段的 exact byte/numeric total order 一次分配非零 ordinal，complete duplicate 明确失败且 rejection 后不重编号。AGENTS 2.3/2.5/2.6、BR-084、BR-086 在最终 use-site 重验；每 proposal 恰有一条 BR-086 outcome，accepted transaction 原子写/read-back exact alias pair、V1 reservation/confirmation、Confirmed audit、paper order、outbox、delivery，rejection 仅写 exact audit。legacy ID 保持 opaque；`LegacyPaperExitAliasInputV1` 在私有 SQLite 永久保存 position code raw bytes、由 closed trigger-rule table 投影的 exact reason raw bytes及各自 domain hash，alias 只从签名绑定 IANA timezone/algorithm/projection descriptor 和已持久字节生成，升级/重启不得重算；60 秒 legacy UTC、四个互斥 identity reason、三态 monotonic cutover 与 permanent dual guard 不变。唯一 child migration/owner 按 canonical SQLite v1 TSV 冻结每个 new/reused table、column、PK/FK/UNIQUE/index/trigger、schema version、operation、migration order 与 retained rollback decoder；marked identifier inventory 冻结所有 Rust/wire/schema/bin 名称、visibility、source/Cargo path、alias policy 与 legacy disposition，AC-14 必须 `unclassified=0`。SQLite 是唯一 authority，JSONL 仅 projection；same-state takeover 仍须 exclusive lock、consumed handoff/death proof、new owner 与 checked generations，CAS=0 loser 零 append。rollback 在任何 repository code 前仅运行 absolute root-owned immutable `/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1`：它 clearenv、只用 absolute signed descriptor-attested roots、内部 Ed25519/SHA-256、no-follow `openat2` FD hash/read-back 与 `fexecve`，禁止 ambient shell/openssl/jq/shasum/awk/Git/Python；verified receipt 唯一固定 base/source/deep order。Stage 2 只在 descriptor-root detached worktree 用 hash-pinned Git/Python secondary verifier逐步证明 exact inverse/tree，并以 canonical raw-byte caller manifest 在前后覆盖 tracked/untracked/index stages/content/symlink/mode/gitlink/tombstone，任何失败仅 descriptor-clean rollback workspace、caller bytes/index/HEAD 不变且 capability Disabled。BR-201 仍严格 `spec-only`，Code cell 只含本设计与 `docs/business_rules.md`；本 repairer 只 stage 设计，shared business-rules row 由 root integrator 在最终 docs-only candidate stage 并运行 Rule 2.10，不声称 intermediate index equality。API、identity、schema、migration 与 rollback 均明确迁移。本修复回应第七次独立 RED C1/I7/M1；当前仍无真实 provider、真实 Disabled banner、2026-08-02 push/event 或 fresh real-account evidence，不声称 Gate A passed、Gate B started 或 Gate D ready，必须等待 fresh independent C0/I0 review。

## Rollback

Implementation is split into three release-manifest classes:

1. `containment_commit`: the fail-closed caller switch, added first;
2. `compatibility_commits`: the forward-compatible v1 audit decoder, old-public-entry seal,
   persistent open-attempt preflight/latch/reconciler, independent boundary counters and
   `br201-evidence` verifier; and
3. `deep_commits`: typed session authority, guarded engine, order-plus-outbox authorization and
   caller integration.

Containment and compatibility commits are never reverted while any v1 audit/open-attempt fact may
exist. A tracked file cannot truthfully contain the SHA of the commit which contains that same file,
and binaries cannot be hashed before that source commit exists. BR-201 therefore uses a realizable
two-stage binding rather than a self-referential manifest.

The source-only manifest `docs/releases/br201/source-release.json` is committed alone after the
implementation tip. Its exact ordered fields are
`schema_version,rule_id,release_id,merge_base,implementation_tip,containment_commit,
compatibility_commits,deep_commits,ordered_commit_list_sha256,
rollback_verifier_source_blob_oid,rollback_verifier_source_raw_bytes_sha256,attestation_key_id`.
Compatibility/deep arrays are oldest-first; every Git SHA is lowercase 40-hex.
`rollback_verifier_source_blob_oid` and `rollback_verifier_source_raw_bytes_sha256` bind the exact
`tools/release/verify_br201_rollback_manifest.py` bytes in `source_commit`; they are secondary
worktree-verifier identity only and never bootstrap the deployed trust root.

`ordered_commit_list_sha256` is independently derived from `Br201OrderedCommitListV1`, whose exact
ordered fields are `schema_version,rule_id,merge_base,implementation_tip,commits`; `commits` is an
ordered array of `Br201ReleaseCommitV1` with exact fields `commit_sha,release_class`, where
`release_class` is exactly `containment`, `compatibility` or `deep`. Serialization is
`serde_json::to_vec` compact UTF-8 in declaration order, with no map, BOM, whitespace, newline or
Unicode normalization. The hash is lowercase SHA-256 over domain bytes
`stock_analysis.br201.ordered_commit_list.v1\0` followed by those canonical bytes.

Membership is derived, never trusted from the manifest: run the equivalent of
`git rev-list --reverse --topo-order --no-merges merge_base..implementation_tip`, require that the
range contains no merge commit, and require every returned commit message to contain exactly one
trailer `BR-201-Release-Class: <class>` with one of the three exact values. The derived sequence
must contain exactly one `containment` first, one or more `compatibility` next and one or more
`deep` last; `implementation_tip` must equal the final derived commit. A duplicate SHA, missing or
duplicate/unknown trailer, foreign/unclassified commit, omitted range member, extra manifest
member, class interleaving or array-order difference rejects. The manifest's containment scalar
and compatibility/deep arrays must be exact projections of this derived ordered sequence before
the typed preimage and hash are accepted.

The golden canonical bytes are:

```text
{"schema_version":1,"rule_id":"BR-201","merge_base":"0000000000000000000000000000000000000000","implementation_tip":"4444444444444444444444444444444444444444","commits":[{"commit_sha":"1111111111111111111111111111111111111111","release_class":"containment"},{"commit_sha":"2222222222222222222222222222222222222222","release_class":"compatibility"},{"commit_sha":"3333333333333333333333333333333333333333","release_class":"deep"},{"commit_sha":"4444444444444444444444444444444444444444","release_class":"deep"}]}
```

With the domain and NUL above, its required hash is
`5b80352e80109d585e8cdce45f308454760f5d62ada9af76192ddce6ed72d595`.
The verifier self-test and mutation suite must independently reconstruct the range and reject a
changed domain/NUL/schema/field order, reordered/duplicate/missing/foreign/merge commit, bad trailer,
wrong class projection and any manifest-supplied hash not equal to this recomputation.

The commit containing the manifest is the externally observed `source_commit`; it is absent from
the manifest and ordered list by design. Verification requires
the source commit has exactly one parent, `source_commit^ == implementation_tip`, and its diff
contains exactly this source
manifest, `merge_base` is an ancestor, and the full classification above succeeds. This detached
source-parent sequence is physically constructible and never asks a tracked file to contain its
own commit SHA.

Release CI checks out that exact `source_commit`, performs the release build, hashes the source
manifest raw bytes and the two executable raw bytes, then writes an append-only post-build
attestation outside the source tree at
`data/release_attestations/br201/<release_id>/release-attestation.v1.json`. Its exact ordered fields
are `schema_version,rule_id,release_id,source_commit,source_manifest_blob_oid,
source_manifest_raw_bytes_sha256,monitor_binary_sha256,br201_evidence_binary_sha256,
build_invocation_sha256,built_at_utc,attestation_key_id`. A detached Ed25519 signature over
`stock_analysis.br201.release_attestation.v1\0` plus the compact canonical JSON bytes is stored as
`release-attestation.v1.sig`. The trusted public key and key ID are checked in before
`implementation_tip`; the private key never enters the repository or process environment. The
append-only audit directory is retained for at least five years and rejects replacement, aliasing
or a second attestation for one release ID.

Deployment adds a signed, append-only `Br201DeploymentReceiptV1` outside the source and caller
worktrees at `data/deployments/br201/receipts/<deployment_id>/deployment-receipt.v1.json`. Its exact
ordered fields are `schema_version,rule_id,deployment_id,release_id,source_commit,
source_manifest_blob_oid,source_manifest_raw_bytes_sha256,release_attestation_sha256,
monitor_binary_sha256,br201_evidence_binary_sha256,deployed_at_utc,attestation_key_id`. A detached
signature covers exact domain `stock_analysis.br201.deployment_receipt.v1\0` followed by the compact
canonical receipt bytes. The corresponding manifest and binaries live only at content-addressed,
non-replacing paths derived from those hashes; the receipt never supplies an arbitrary executable
or manifest path.

The initial trust anchor is not code from the caller worktree. Operations pins the Ed25519 public
key and its lowercase raw-byte SHA-256 in the immutable root-owned descriptor consumed only by the
absolute rollback bootstrap frozen below. Before executing `br201-evidence`, that bootstrap uses
its internal Ed25519/SHA-256 implementation and descriptor-relative no-follow FDs to verify the key,
deployment-receipt signature, and exact content-addressed verifier/manifest raw bytes. Only that
FD-held independently verified deployed `br201-evidence` binary may then
verify the release-attestation signature, key ID, source-commit ancestry, manifest blob/raw-byte
binding and deployed monitor bytes and emit the immutable rollback inputs. Its output fixes
`rollback_of_source_commit == rollback_base == receipt.source_commit` and the only allowed revert
set/order to the verified source manifest's exact non-empty `deep_commits` newest-first. Neither an
environment variable, dirty repository script, branch name, caller `HEAD`, locally rebuilt binary
nor later worktree script may select or alter those values.

Before every rollback canary/prepare command, the immutable bootstrap repeats its descriptor and
FD checks, then the authenticated deployed `br201-evidence` repeats receipt, attestation,
trusted-key ID, source ancestry, exact manifest blob/raw-byte and both deployed executable hash
checks. All hashes are lowercase 64-hex. A source manifest without its
signed post-build attestation and deployment receipt, an attestation checked into or generated by
the deployed source tree, a locally rebuilt binary, a self-declared `release_commit`, a dirty caller
script or a branch-wide subject grep is not release authority.

A deep revert is a new release, not permission to deploy the local validation build. After the
revert commits, a source-only `Br201RollbackSourceManifestV1` is committed at
`docs/releases/br201/rollbacks/<rollback_release_id>/source-release.json`. Its exact ordered fields
are `schema_version,rule_id,rollback_release_id,rollback_of_release_id,
rollback_of_source_commit,rollback_base,rollback_implementation_tip,reverted_deep_commits,
ordered_revert_commit_list_sha256,rollback_implementation_tree_oid,
retained_containment_commit,retained_compatibility_commits,
original_deployment_receipt_sha256,original_release_attestation_sha256,attestation_key_id`. The manifest commit is
`rollback_source_commit`, must have exactly one parent, must satisfy
`rollback_source_commit^ == rollback_implementation_tip`, and its diff must contain only that
manifest. `reverted_deep_commits` is the exact original deep list newest-first; retained arrays,
the original deployment-receipt raw-byte hash and the original attestation raw-byte hash must match
the already verified deployed release.

The ordered revert history is independently derived, not trusted from that array.
`Br201RollbackCommitListV1` has exact ordered fields
`schema_version,rule_id,rollback_of_source_commit,rollback_base,
rollback_implementation_tip,entries`. `entries` is an ordered array of
`Br201RollbackCommitV1` with exact fields
`revert_commit_sha,reverts_deep_commit_sha,parent_sha,tree_oid,inverse_patch_sha256`. Serialization
is compact declaration-order UTF-8 JSON with no map, BOM, whitespace, newline or Unicode
normalization. Its hash is lowercase SHA-256 over exact domain
`stock_analysis.br201.rollback_commit_list.v1\0` followed by those bytes.

The golden bytes are:

```text
{"schema_version":1,"rule_id":"BR-201","rollback_of_source_commit":"9999999999999999999999999999999999999999","rollback_base":"9999999999999999999999999999999999999999","rollback_implementation_tip":"6666666666666666666666666666666666666666","entries":[{"revert_commit_sha":"5555555555555555555555555555555555555555","reverts_deep_commit_sha":"4444444444444444444444444444444444444444","parent_sha":"9999999999999999999999999999999999999999","tree_oid":"7777777777777777777777777777777777777777","inverse_patch_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"revert_commit_sha":"6666666666666666666666666666666666666666","reverts_deep_commit_sha":"3333333333333333333333333333333333333333","parent_sha":"5555555555555555555555555555555555555555","tree_oid":"8888888888888888888888888888888888888888","inverse_patch_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]}
```

The required domain-separated hash is
`1d137a5dc6a36a62bdf37045d29bc215c60515f777b605af6eccb016ab5d5abd`.
Mutation tests reject a changed domain/NUL/order/parent/tree/inverse hash, omitted/extra entry or
trailing newline.

The verifier starts at the already verified deployed `rollback_of_source_commit`, requires
`rollback_base` to equal it, and walks first-parent commits through
`rollback_implementation_tip`. The entry list length and `reverts_deep_commit_sha` sequence must
equal the original manifest's non-empty `deep_commits` newest-first. Every revert commit must be
non-merge, have exactly the prior verified SHA as its sole parent, and contain exactly one
`BR-201-Reverts: <deep_sha>` trailer plus exactly one
`BR-201-Rollback-Class: deep-revert` trailer. The first parent is `rollback_base`; each later parent
is the preceding entry's `revert_commit_sha`; the final entry is
`rollback_implementation_tip`. No unlisted commit can occur in that chain.

For each entry the verifier reads the original deep commit and its sole parent, materializes the
original commit's exact full-index binary patch with rename detection disabled, and computes
`inverse_patch_sha256` over domain `stock_analysis.br201.inverse_deep_patch.v1\0` plus those raw
patch bytes. In a temporary Git index rooted at the current revert parent tree, it applies exactly
that patch in reverse with binary support and no three-way/fuzz fallback. The resulting index tree
must equal both the revert commit's actual tree and the entry `tree_oid`; the actual revert commit
must have no worktree/submodule/mode/path change outside that exact reverse application. Application
failure, an original merge commit, context drift, rename heuristic, extra deletion/addition,
piggyback file, empty inverse or tree mismatch rejects. This tree construction, rather than commit
subject or `git revert` exit status, proves the inverse and prevents unrelated changes from riding
inside a signed rollback.

Release CI checks out exactly `rollback_source_commit`, builds in a clean environment and emits
append-only `rollback-attestation.v1.json` plus detached `.sig`. `Br201RollbackAttestationV1` has
exact ordered fields `schema_version,rule_id,rollback_release_id,rollback_of_release_id,
rollback_of_source_commit,rollback_source_commit,rollback_source_parent,
rollback_source_manifest_blob_oid,rollback_source_manifest_raw_bytes_sha256,
ordered_revert_commit_list_sha256,rollback_implementation_tree_oid,
original_deployment_receipt_sha256,original_release_attestation_sha256,
monitor_binary_sha256,br201_evidence_binary_sha256,
build_invocation_sha256,built_at_utc,attestation_key_id`. The signature preimage is exact domain
`stock_analysis.br201.rollback_attestation.v1\0` followed by compact declaration-order UTF-8 JSON
with no trailing newline. Verification requires `rollback_source_parent ==
rollback_implementation_tip`, validates both original-release and rollback signatures, recomputes
the original deployment-receipt binding, both manifest bindings, the complete ordered revert list,
every inverse-patch/tree proof and the
deployed binary hashes, and confirms containment/compatibility are present while every named deep
commit is exactly reverted. Unknown fields/enums, parent/list/tree mismatch, unsigned or locally
generated attestation, wrong original receipt/attestation hash, build-hash mismatch, piggyback change or
missing retained decoder/reconciler fails closed and leaves the supervisor disabled. Only artifacts
fetched from this verified signed rollback attestation may be staged or deployed.

Rollback is executable and two-stage.

The following immutable trust-root bootstrap is a prerequisite to both stages. The only bootstrap
executable is the absolute `/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1`, installed by
the release-host package as UID/GID `0:0`, mode `0555`, link count one, immutable/verity-protected,
on the release-host filesystem. Its package signature and raw-byte SHA-256 are pinned by the OS
package trust database, outside the repository, object store and monitor account. The only
descriptor is absolute `/etc/stock-analysis/br201/bootstrap-descriptor.v1.json`, UID/GID `0:0`,
mode `0444`, link count one, immutable, signed by a public key compiled into that bootstrap.
`Br201BootstrapDescriptorV1` has exact ordered fields
`schema_version,rule_id,descriptor_id,active_receipt_ref_path,
active_receipt_ref_attestation,trust_key_path,trust_key_attestation,
deployment_object_root_path,deployment_object_root_attestation,
release_attestation_root_path,release_attestation_root_attestation,
rollback_workspace_root_path,rollback_workspace_root_attestation,
runtime_evidence_root_path,runtime_evidence_root_attestation,pinned_git_path,
pinned_git_attestation,pinned_git_sha256,pinned_python_path,pinned_python_attestation,
pinned_python_sha256,bootstrap_key_id`. Every path is absolute.
Each nested `Br201PathAttestationV1` has exact ordered fields
`device,inode,mount_id,uid,gid,mode,file_type,nlink,immutable_flag,verity_digest_sha256`;
root directories and executable/files are rechecked against those exact values.
The detached descriptor signature covers exact domain
`stock_analysis.br201.bootstrap_descriptor.v1\0` plus compact declaration-order UTF-8 JSON with no
trailing newline; unknown/duplicate fields, a relative path, path normalization, changed root
attestation or key ID rejects.

On entry the bootstrap calls `clearenv`, sets only `PATH=/usr/bin:/bin`, `LANG=C`, `LC_ALL=C`,
`TZ=UTC`, `IFS=<space-tab-newline>`, and umask `077`; it never performs PATH lookup and never reads
`LD_*`, `DYLD_*`, Git, Python, locale, home, temp, config, credential or repository environment.
It implements Ed25519, SHA-256 and strict canonical JSON internally. It never invokes ambient
`openssl`, `jq`, `shasum`, `awk`, `sed`, `dd`, a shell, or a repository script. On the Linux release
host every root/leaf is opened with `openat2` from the attested root descriptor using
`RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV`, then checked by
`fstat` for exact owner/mode/type/link/device/inode and immutable/verity identity, hashed from that
same FD, re-`fstat`ed, and executed only by `fexecve` from the still-open FD. The pinned Git/Python,
deployed verifier and monitor receive sealed descriptor FDs, never caller paths. A platform without
those race-free primitives is unsupported and aborts before repository code runs.

The active receipt reference is exact absolute
`/etc/stock-analysis/br201/active-deployment-receipt.v1`; it is canonical signed JSON, not a
caller-selected path. The descriptor fixes absolute object root
`/var/lib/stock-analysis/br201/deployment-objects`, attestation root
`/var/lib/stock-analysis/br201/release-attestations`, rollback root
`/var/lib/stock-analysis/br201/rollback-workspaces`, and runtime root
`/var/lib/stock-analysis/br201/runtime-evidence`. Receipt hashes select only a lowercase-hex child
opened below the already attested object-root FD. The bootstrap verifies the receipt/attestation,
manifest and both deployed executable bytes, then FD-executes the deployed verifier; neither the
descriptor nor verifier accepts base, source commit, deep commit, ordering or executable paths from
argv/environment/caller output.

The only permitted initial operation is therefore:

```text
/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1 verify-active-deployment --descriptor /etc/stock-analysis/br201/bootstrap-descriptor.v1.json
```

It atomically writes an immutable root-owned `Br201VerifiedRollbackInputsV1` beneath the attested
runtime root and returns only its descriptor-relative opaque capability ID. Any stdout path is
diagnostic and cannot be supplied back as authority; later subcommands reopen the object by that
ID beneath the runtime-root FD and validate its signed hash and active receipt binding.

Before Stage 2 the bootstrap also emits `Br201CallerWorktreeManifestV1` without modifying the
caller worktree. The caller may identify only the absolute repository root to be *observed*; it
cannot affect rollback base/order/workspace/tool selection. Its raw canonical byte preimage begins
with `stock_analysis.br201.caller_worktree_manifest.v1\0`, followed by unsigned big-endian length-
prefixed fields `schema_version,repository_root_device,repository_root_inode,head_ref_raw_bytes,
head_oid_raw_bytes,index_file_raw_bytes_sha256,index_tree_oid_raw_bytes,entry_count`, then entries
sorted by raw path bytes. Each entry is length-prefixed exact
`path_raw_bytes,source_kind,index_stage,index_mode,index_blob_oid,worktree_kind,worktree_mode,
worktree_content_raw_bytes_sha256,symlink_target_raw_bytes_sha256,gitlink_head_raw_bytes,
tombstone`. `source_kind` is `Tracked`, `Untracked`, or `UntrackedDirectory`; every index stage
`0..3`, executable/mode change, regular-file bytes, symlink target bytes, gitlink, untracked leaf and
tracked deletion (`tombstone=1`) is represented. `.git` administrative objects and the external
rollback/runtime roots are the only exclusions. Raw paths/targets are never UTF-8-normalized. The
bootstrap opens the repository and every descendant no-follow from a pinned root FD, detects
rename/replace races by before/after descriptor walks, records the raw index file itself, and aborts
on a changing entry set. It writes the exact manifest bytes/hash both before and after Stage 2 and
requires byte identity; a dirty caller is allowed as evidence but can neither be cleaned nor select
rollback inputs.

`Br201VerifiedRollbackInputsV1` has exact ordered fields
`schema_version,rule_id,deployment_receipt_sha256,release_attestation_sha256,
source_manifest_raw_bytes_sha256,rollback_of_source_commit,rollback_base,
deep_commits_newest_first,ordered_commit_list_sha256,
rollback_verifier_source_blob_oid,rollback_verifier_source_raw_bytes_sha256`. The authenticated
deployed verifier derives every field from the already signature/hash-verified receipt,
attestation and immutable source manifest; it rejects caller-supplied overrides. A mutation of the
active receipt, key/hash, signature domain, receipt bytes, object path/hash, source commit, allowed
commit list/order or emitted input bytes must fail before either stage starts.

### Stage 1: contain and prove zero calls

Resolve the currently deployed monitor/verifier from the signed deployment inventory and verify
those already deployed raw bytes before running either executable. Do not run `cargo build`, alter
the binaries or accept `target/release` as authority in this stage. The verifier's snapshot reads pinned
production authorities and returns durable high-water marks for BR-201 Admission/Terminal audit,
open-attempt rows, paper-order rows, order-attempt audit and `PaperExitEventOutbox`, plus independent
monotonic counters incremented inside the BR-201 account-context, ledger, provider, order-commit and
outbox-projection façades before each real boundary call. These counters are owned outside the
scheduler and cannot be supplied by its banner.

Capture before/after snapshots around one bounded normal monitor run with the explicit disable
value using only the bootstrap-owned operation:

```text
/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1 stage1-disabled-canary --descriptor /etc/stock-analysis/br201/bootstrap-descriptor.v1.json --minimum-observed-ticks 2 --bounded-seconds 65
```

The numeric bounds are the only caller values and are range-checked to exact policy
`minimum-observed-ticks=2` and `bounded-seconds=65`; any other value rejects. The bootstrap reopens
and revalidates the active receipt and immutable rollback-input object, FD-executes the deployed
verifier for a before snapshot, FD-executes the deployed monitor with its internally constructed
allowlisted environment containing exact `MONITOR_ENABLED=true` and
`STOCK_ANALYSIS_PAPER_EXIT_ENABLED=0`, signals that exact child PID after 65 seconds, FD-executes
the verifier for the after snapshot, and verifies the canary. Snapshot/log/result objects are
root-owned immutable entries beneath the attested runtime root and are bound into one signed
`Br201Stage1ContainmentReceiptV1`; `/tmp`, caller redirection, a caller PID, supervisor environment,
or a caller-selected binary/path cannot participate.

`Br201Stage1ContainmentReceiptV1` has exact ordered fields
`schema_version,rule_id,bootstrap_descriptor_sha256,verified_rollback_inputs_sha256,
deployment_receipt_sha256,before_snapshot_sha256,after_snapshot_sha256,monitor_log_sha256,
minimum_observed_ticks,bounded_seconds,observed_tick_count,boundary_counter_delta_sha256,
authority_high_water_delta_sha256,exact_disabled_startup_banner_count,completed_at_utc,
bootstrap_key_id`; its signature domain is
`stock_analysis.br201.stage1_containment_receipt.v1\0`.

Verification succeeds only when at least two scheduler ticks are independently observed, every
five boundary counters is unchanged, and every Admission/open-attempt/order/order-audit/outbox
high-water is byte-identical. Terminal high-water may change only if the retained reconciler closes
an already-open pre-canary attempt and the verifier proves its exact admission join; a new Admission
is forbidden. Logs are supplemental and cannot satisfy the command. Keep
`STOCK_ANALYSIS_PAPER_EXIT_ENABLED=0` in the process supervisor throughout Stage 2. An absent or
malformed setting must produce the same disabled evidence, never implicit enablement.

### Stage 2: revert the deep implementation while containment remains

Select the tracked source manifest and signed post-build attestation for the deployed release,
verify both against the deployed source commit/merge-base and executable bytes, then create a new
clean detached worktree at that exact verified commit before reverting only the manifest's ordered
`deep_commits`, newest first. The caller worktree is evidence, never the rollback workspace. The
verifier refuses an untracked source manifest, an invalid/missing signature, a SHA not bound to the
release, an unclassified BR-201 implementation commit, a containment/compatibility SHA in the deep
array, a dirty detached rollback worktree or any ancestry/order/hash mismatch. The exact operation
is one bootstrap call; the caller-worktree argument is observation scope only:

```text
/usr/libexec/stock-analysis/br201-rollback-bootstrap-v1 stage2-prepare-deep-revert --descriptor /etc/stock-analysis/br201/bootstrap-descriptor.v1.json --caller-worktree /absolute/path/to/observed/repository
```

The bootstrap revalidates the active receipt and sealed inputs, writes the pre-operation
`Br201CallerWorktreeManifestV1`, creates a root-owned random child of the descriptor-attested
rollback root, and FD-executes only the descriptor-hash-pinned Git. Git is given a cleared,
allowlisted environment, fixed system/global config null devices, disabled hooks, disabled signing,
fixed author/committer identity/time supplied by the sealed inputs, and the sealed repository/worktree
FDs. It detaches at `rollback_base` obtained only from the signed verified-input object. For every
sealed deep commit newest-first it FD-executes the blob/raw-hash-pinned secondary verifier through
the pinned Python FD, materializes the exact full-index inverse with rename/fuzz/three-way disabled,
requires the exact parent/index/tree proof, commits only that inverse with the two frozen trailers,
and read-backs the commit/tree. It then derives the ordered revert list, uses only the same pinned
secondary verifier for structural validation, and records its raw outputs and exit codes. Cargo/build/test/compliance
commands run only later in clean release CI from the prepared source commit; Stage 2 produces no
local executable artifact.

Before success, the bootstrap repeats the descriptor walk of both rollback and caller roots and
emits the post-operation caller manifest. It requires raw-byte equality with the pre-operation
manifest, including HEAD/ref bytes, raw index hash/tree, tracked/untracked paths, stages, modes,
content hashes, symlink targets, gitlinks and tombstones. It writes a signed
`Br201Stage2PreparedRollbackV1` binding both caller manifests, sealed inputs, exact rollback-root
device/inode, ordered revert bytes/hash, final tree, structural-validation results and original deployment
receipt. It returns only an opaque handoff ID. No shell variable, process substitution, `mktemp`,
ambient Git/Python/Cargo, caller `HEAD`/index/status, or repository helper executes in this flow.

`Br201Stage2PreparedRollbackV1` has exact ordered fields
`schema_version,rule_id,bootstrap_descriptor_sha256,verified_rollback_inputs_sha256,
original_deployment_receipt_sha256,caller_manifest_before_sha256,
caller_manifest_after_sha256,rollback_workspace_device,rollback_workspace_inode,rollback_base,
rollback_implementation_tip,ordered_revert_commit_list_sha256,
rollback_implementation_tree_oid,secondary_verifier_raw_bytes_sha256,
structural_validation_results_sha256,prepared_at_utc,bootstrap_key_id`; its signature domain is
`stock_analysis.br201.stage2_prepared_rollback.v1\0`.

Every failure before the signed Stage-2 receipt removes only the root-owned detached rollback
workspace through the bootstrap's retained descriptors; the byte-identical caller manifest must
still validate and the supervisor remains Disabled. Cleanup never follows a caller path or symlink.
The successful detached worktree is retained under the attested rollback root until its source-only
rollback manifest commit is created and handed to release CI, then removed by an exact bootstrap
handoff operation; it contains no locally built deployment artifact. The derived commit-list
bytes/hash and final tree are copied into the
rollback source manifest; then commit that manifest alone, submit its exact source commit to release CI, and require the new
detached `Br201RollbackAttestationV1`. Release CI independently re-derives rather than trusting the
temporary output. The deployment controller
first verifies the new signature/parent/original-release binding and downloaded artifact hashes,
then stages those signed bytes while the switch remains `0`; any verification, fetch, staging,
preflight or zero-call-canary failure leaves the old verified deployment (or already staged signed
rollback) disabled and performs no partial executable replacement.

The signed reverted release must still decode every v1 record, run open-attempt preflight/reconciliation,
honor permanent quarantine, expose the independent counters and keep the old unguarded entry
sealed. Run the signed rollback verifier's `verify-open-attempts` and repeat the Stage-1
snapshots/canary against the signed staged rollback bytes. Any open reconciliation, quarantine or changed boundary/high-water evidence keeps
deployment disabled. Do not revert containment/compatibility commits, delete audit/order/outbox
facts, restore the unguarded entry or use BR-154 as a quote fallback.

### Recovery

Repair the guarded implementation on top of the retained containment and compatibility foundation,
pass Gates B-D, create a new source-only ordered manifest commit, build that exact source commit and
verify a new signed post-build attestation, and deploy
while the switch is still `0`. Only when `br201-evidence verify-open-attempts` reports no open or
quarantined attempt, all Gates B-D pass and independent runtime evidence is accepted may a new
process start explicitly with `STOCK_ANALYSIS_PAPER_EXIT_ENABLED=1`; its first attempt must join a
valid BR-201 Admission audit and atomic order/outbox authority. No database deletion, audit
truncation or configuration migration is part of rollback.

## 2026-08-22 修订：BR-134 T+1/FIFO 批次库存修复（算法切片）

### 1. 状态与边界

本节是对 BR-134 的算法级修订，修复 legacy `paper_sell` 将全部历史买入混合摊薄、并用最早买入日期放行整仓卖出的错误。它只定义批次库存重建、T+1 可卖数量和卖出评估输入，不批准或激活 BR-201，不解除 `paper_sell_paused` 的默认暂停，不修改 monitor 的调用频率、现有卖出触发条件、真实下单路径、配置阈值、数据库 schema 或历史数据。

本切片触发 AGENTS 规则 2.1、2.2、2.3、2.4、2.7 和 2.10。由于不修改配置阈值，不触发 2.9 的阈值证明。实现完成也只能证明批次算法达到 Gate B/C 的候选条件；在 BR-201 的真实账户 provider、session/order authority 和正向 canary 完成前，Gate D 仍然阻塞，生产能力必须保持 Disabled。

### 2. 当前代码证据

以下命令用于固定本修订的当前代码事实，输出必须随实现 PR 一并复核。

legacy `paper_sell` 仍有盘中和盘后两个 monitor 调用点：

```text
$ rg -n -A4 -B3 'scan_and_sell(_post_close)?\s*\(' src --glob '*.rs'
src/trading/paper_sell.rs:231:pub fn scan_and_sell(risk_context: PaperRiskContext) -> Result<Vec<PaperSellResult>, String> {
src/trading/paper_sell.rs:239:pub fn scan_and_sell_post_close(
src/bin/monitor/main.rs:8228:                    match stock_analysis::trading::paper_sell::scan_and_sell(risk_context) {
src/bin/monitor/main.rs:8273:                            match stock_analysis::trading::paper_sell::scan_and_sell_post_close(
```

现有实现把全部买入聚合成一个均价和一个最早日期，并把聚合净数量直接作为卖单数量：

```text
$ rg -n 'aggregate_open_positions|first_buy_date == today|quantity: pos.quantity|avg_buy_price' src/trading/paper_sell.rs
40:    pub avg_buy_price: f64,
134:pub fn aggregate_open_positions() -> Result<Vec<PaperPosition>, String> {
164:            avg_buy_price: row.avg_price,
246:    let positions = aggregate_open_positions()?;
287:        buy_price: pos.avg_buy_price,
302:    if pos.first_buy_date == today {
318:    let gross_pct = (quote.price / pos.avg_buy_price - 1.0) * 100.0;
326:        quantity: pos.quantity as u32,
352:        quantity: pos.quantity,
```

生产 gate 已明确记录该账本问题并默认暂停，但环境变量仍能显式越过暂停，因此算法不能继续保留已知错误：

```text
$ rg -n -A25 -B10 'fn paper_sell_paused' src/bin/monitor/main.rs
7656:/// PaperSell 生产 gate (v19 review 2026-08-12, invalid_position_ledger):
7657:/// 成本为全部历史买入混合摊薄 (Σamt/Σqty), T+1 用 MIN(ts) 最早买入日, 无批次
7658:/// 账本 → 生产 100 笔虚拟卖出含 3 笔收益率 >100% (最高 +22751% 为买价记录错误)、
7659:/// 11 笔当日买入即卖、7 笔买入后 60s 内卖出 (最短 5s)。暂停投递直到批次账本重建。
7660:/// 默认禁用, 仅 `PAPER_SELL_ENABLED=1` 显式启用
7662:fn paper_sell_paused(phase: &str) -> bool {
7663:    if std::env::var("PAPER_SELL_ENABLED")
7664:        .map(|value| value == "1")
7665:        .unwrap_or(false)
7666:    {
7667:        return false;
```

在当前源码中，下面的完整仓库检索没有找到 BR-201 唯一 owner 的实现或调用：

```text
$ rg -n -A6 -B4 'execute_paper_exit_tick_v1\s*\(' src --glob '*.rs'
# no matches
```

因此，本修订不能把“FIFO 算法修复”表述为 BR-201 已就绪或生产卖出可恢复。

### 3. 方案选择

采用执行层共享的纯 FIFO 批次账本模块，并由 `paper_sell` 的数据库适配器调用。核心函数只接收已排序成交事实和显式 `as_of_date`，不访问数据库、行情、时钟、订单或推送；它输出每个代码的总库存、可卖库存、当日锁定库存、可卖批次加权成本和最早可卖日期。

不直接复用 `performance::attribution::fifo_match`。该模块面向报告窗口、信号族和成交片段归因，一笔卖出可能展开为多条报告记录；执行路径依赖它会把报告语义带进资金安全边界。后续归因修复可以改为消费同一个底层批次账本，但本切片不修改归因统计口径。

不把 FIFO 隐藏在 SQL 窗口或聚合语句中。SQL 只负责按 `(ts, id)` 读取事实，Rust 纯函数负责逐行校验和 FIFO 状态转换，使超卖、乱序、重复身份和 T+1 锁定可以通过确定性单元测试验证。

### 4. 组件和数据流

新增执行层纯模块 `src/trading/paper_lot_ledger.rs`，其最小领域结构为：

- `PaperFill`：`id`、证券代码/名称、方向、成交价、数量、发生时间；
- 内部 `OpenPaperLot`：买入身份、买入时间、剩余数量和价格；
- `PaperPositionInventory`：总数量、可卖数量、锁定数量、可卖均价和最早可卖日期；后两项为显式 `Option`，且当且仅当可卖数量为零时为 `None`；
- `rebuild_paper_positions(rows, as_of_date)`：校验全部输入、按代码做 FIFO、返回稳定排序的库存快照或显式错误。

完整数据流如下：

1. `paper_sell` 数据库适配器一次读取全部 `Filled` 纸面成交，SQL 明确 `ORDER BY ts, id`。
2. 在任何 quote/provider/order/push 调用前，将行转换为 `PaperFill` 并交给纯账本模块。
3. 模块验证非空身份和代码、唯一且严格递增的 `(occurred_at, id)`、合法方向、有限正价、正数且为 100 股整数手的数量，以及所有 checked arithmetic；卖出按代码逐批 FIFO 消耗，库存不足即整批失败。
4. 每个剩余买入批次保留自己的日期、价格和数量。`buy_date < as_of_date` 才属于可卖库存；`buy_date == as_of_date` 属于锁定库存。更高层 session gate 负责证明 `as_of_date` 是否是可交易评估日，本模块不猜测交易日。
5. `paper_sell` 只为 `sellable_quantity > 0` 的库存调用既有卖出规则。规则输入中的买入价、最早买入日和收益率只来自可卖批次；触发后数量固定为全部可卖数量。当日锁定批次继续留在账本中。
6. 只有当日批次时不调用卖出规则、不取得行情、不生成订单。既有“一代码一日只卖一次”幂等检查保持不变。
7. 订单成功后不做本地猜测式减仓；下一轮仍从已持久化成交事实完整重建，以账本事实作为唯一库存来源。

证券名称只作为展示字段：同一代码的名称变化不构成库存结构错误，输出使用该代码最近一条非空名称；空名称仍显式失败。代码才是 FIFO 分组身份。

### 5. 失败模式与副作用边界

以下任一情况必须使整轮库存重建失败，而不是跳过坏行或返回部分仓位：数据库读取失败、时间解析失败、重复或非递增顺序、空身份/代码/名称、未知方向、价格非正或非有限、数量非正或不是 100 的倍数、整数/金额计算溢出、卖出超过该代码已有数量。

账本结构失败必须发生在 quote/provider/order/push 之前，因此该轮外部副作用为零。卖出规则自身的仓位级失败语义保持不变：某仓位未满足规则或只有当日锁定库存时不产生订单，不伪装成全局成功卖出。数据库和订单失败继续显式上抛，不回退到 mock 数据、成本价、零值或推测值。

本切片不删除 `paper_sell_paused`，不修改 `PAPER_SELL_ENABLED` 行为，不修改 `src/bin/monitor/main.rs`，也不把 legacy `paper_engine`、`paper_trade::simulate` 或 attribution 模块变成新的生产 authority。

### 6. 旧模块关系

| 模块 | 处理 | 原因 |
| --- | --- | --- |
| `trading::paper_sell` | 采用并收窄 | 保留数据库、规则评估和订单适配，只把错误聚合替换为共享纯账本输出 |
| `pipeline::position_tracker` 卖出规则 | 原样采用 | 本切片不改变策略阈值、规则优先级或触发语义 |
| `performance::attribution` | 拒绝直接依赖 | 它是报告层片段归因，后续可迁移为消费共享账本 |
| `trading::paper_engine` | 本切片不改 | legacy/test-only FIFO 与 BR-201 authority 修复属于独立工作 |
| `trading::paper_trade::simulate` | 本切片不改 | BR-201 已规定四铁律 caller 迁出，不能借本修订扩大 authority |
| `monitor::paper_sell_paused` | 保留 | 默认暂停和 Disabled banner 是当前 containment |

### 7. 测试与验收

纯账本至少覆盖以下确定性例子：

- 隔夜买入 200 股、当日买入 100 股：可卖 200、锁定 100，卖出规则成本只取隔夜 200 股；
- 先前部分卖出：严格消耗最老批次，剩余批次的价格、日期和数量不被重写；
- 可卖批次与当日批次价格不同：收益率和最早持有日排除当日批次；
- 只有当日批次：不取得行情、不产生卖单；
- 超卖、坏价格、非法数量、重复/乱序身份：整批显式失败且外部调用为零；
- 多代码交错成交：各代码独立 FIFO，结果使用稳定代码顺序；
- 既有卖出条件、真实订单适配和一代码一日一次幂等逻辑不变。

Gate B/C 候选验证命令：

```bash
cargo test --lib trading::paper_lot_ledger::tests -- --test-threads=1
cargo test --lib trading::paper_sell::tests -- --test-threads=1
cargo test --lib pipeline::position_tracker::tests -- --test-threads=1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo build --release
```

Gate D 还必须补充覆盖率报告（全局至少 80%，核心交易/数据链路至少 95%）、独立审计签字和真实数据/正向 canary。由于当前 BR-201 provider/authority 尚未实现且生产仍 Disabled，本算法切片不能单独满足 Gate D，也不能声称策略胜率已经验证为正。

### 8. 回滚

设计和实现分别使用独立小提交。失败时先按根因回到 Gate A 或 Gate B，再使用 `git revert` 回滚对应的实际提交 SHA；不得删除或改写成交、订单、审计和归因历史数据。因为默认暂停在本切片中始终保留，回滚不需要切换生产开关，也不得用回滚恢复未受保护的卖出路径。
