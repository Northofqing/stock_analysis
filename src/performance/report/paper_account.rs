//! Observational daily presentation. Accounting stays in the existing ledger;
//! this reader must match the immutable attribution financial source exactly.
use crate::database::DatabaseManager;
use crate::performance::attribution_replay::{
    EffectiveAttributionReport, ReplayError, ReplayErrorClass,
};
use crate::trading::paper_ledger::{
    AccountBinding, EffectiveFillScope, EffectiveProjectionReceipt, PaperLedger, Projection,
    VerifiedEffectiveFillSet,
};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use std::collections::BTreeMap;

struct Holding {
    code: String,
    name: String,
    quantity: u64,
    basis_notional: f64,
    reference_cost_notional: Option<f64>,
    mark_price: f64,
    value: f64,
    net_unrealized: f64,
    observed_at: DateTime<Utc>,
    source: String,
}
struct Observation {
    epoch: String,
    cutover: DateTime<Utc>,
    cash: f64,
    value: f64,
    equity: f64,
    since_cutover: f64,
    realized: f64,
    unrealized: f64,
    fees: f64,
    daily_pnl: Option<f64>,
    daily_unavailable: &'static str,
    buy_fills: usize,
    sell_fills: usize,
    buy_quantity: i64,
    sell_quantity: i64,
    holdings: Vec<Holding>,
}

/// No initialization, migrations, model calls, delivery or trading. A failed
/// observation leaves the committed realized report intact and marks only the
/// account presentation Unavailable, never substituting legacy account rows.
pub fn render_effective_daily_details(
    database: &DatabaseManager,
    report: &EffectiveAttributionReport,
    observed_at: DateTime<FixedOffset>,
) -> Result<(String, String), String> {
    let account = observe(database, report, observed_at.with_timezone(&Utc));
    let account_summary = match &account {
        Ok(account) => render_account_summary(account, report.projection().request.as_of),
        Err(_) => format!("🧾 虚拟盘归因 {}\n重点：当前账户明细不可用，账户与本次归因的同版/时间验证未通过。\n账户权益、浮盈及每日盈亏：Unavailable；不能拼入旧账本或券商历史亏损。", report.projection().request.as_of),
    };
    let mut summary = account_summary.clone();
    summary.push_str("\n━━━━━━━━━━━━━━━━━━━━\n策略结果（与期初持仓分列）：\n");
    summary.push_str(
        &report
            .render_summary()
            .lines()
            .skip(1)
            .filter(|line| !line.starts_with("有效投影 "))
            .map(|line| line.replace("Scenario 净盈亏", "模型净盈亏"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let mut cycles = report.cycles().iter().collect::<Vec<_>>();
    cycles.sort_by(|a, b| {
        b.scenario_net_pnl
            .abs()
            .partial_cmp(&a.scenario_net_pnl.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for cycle in cycles.iter().take(3) {
        let family = cycle
            .entry_composition
            .iter()
            .map(|entry| entry.family.as_str())
            .collect::<Vec<_>>()
            .join("+");
        summary.push_str(&format!(
            "\n平仓重点：{}｜净盈亏 {:+.2} 元｜{}",
            inline(&cycle.code),
            cycle.scenario_net_pnl,
            family
        ));
    }
    let mut markdown = format!(
        "# 虚拟盘归因 {}\n\n{}\n",
        report.projection().request.as_of,
        account_summary
    );
    if let Ok(account) = &account {
        append_holdings_markdown(&mut markdown, account);
    }
    markdown.push_str("\n## 策略已实现结果与审计依据\n\n");
    markdown.push_str(&report.render_markdown()?);
    if !cycles.is_empty() {
        markdown.push_str("\n## 完整平仓明细\n\n| 代码 | 入场时间 | 平仓时间 | 入场族 | 价差收益 | 模型净收益 | 模型成本 |\n|---|---|---|---|---:|---:|---:|\n");
        for cycle in cycles {
            let family = cycle
                .entry_composition
                .iter()
                .map(|entry| format!("{} {}股", entry.family.as_str(), entry.quantity))
                .collect::<Vec<_>>()
                .join("+");
            markdown.push_str(&format!(
                "| {} | {} | {} | {} | {:+.2} | {:+.2} | {:.2} |\n",
                inline(&cycle.code),
                cycle.economic_entry_at,
                cycle.economic_exit_at,
                family,
                cycle.gross_pnl,
                cycle.scenario_net_pnl,
                cycle.gross_pnl - cycle.scenario_net_pnl
            ));
        }
    }
    Ok((summary, markdown))
}

fn observe(
    database: &DatabaseManager,
    report: &EffectiveAttributionReport,
    observed_at: DateTime<Utc>,
) -> Result<Observation, String> {
    let receipt = report.projection();
    let EffectiveFillScope::Epoch(binding) = &receipt.request.scope else {
        return Err("native account observation requires exact epoch scope".into());
    };
    let cutover = receipt.cutover_at.ok_or("missing exact paper cutover")?;
    let clock = || observed_at;
    let (view, set) = PaperLedger::open(database, &clock)
        .read_account_with_effective(binding, receipt.request.as_of)
        .map_err(|e| e.to_string())?;
    validate_frontier(
        (view.version, &view.event_hash),
        &view,
        receipt.ledger_head.as_ref(),
        set.receipt().projection_hash.as_str(),
        receipt.projection_hash.as_str(),
        cutover,
        receipt.request.as_of,
        observed_at,
    )?;
    account_from_set(&view, binding, &set, cutover, observed_at)
}

/// This is the only read-only substitution for a missing strategy-sample join.
/// Storage and integrity errors retain the normal fail-closed preparation path.
pub fn native_daily_observation_is_permitted(error: &ReplayError) -> bool {
    permits_observation(error.class(), error.code())
}
fn permits_observation(class: ReplayErrorClass, code: &str) -> bool {
    class == ReplayErrorClass::Unavailable && code == "paper_attribution_cutover_not_aligned"
}

/// An account observation, never a successful strategy attribution. The caller
/// freezes these bytes through the existing once-per-day presentation owner.
/// The read uses the existing ledger/effective owner in one transaction; no
/// legacy attribution epoch or financial report is activated or committed.
pub fn render_native_daily_account_observation(
    database: &DatabaseManager,
    binding: &AccountBinding,
    date: NaiveDate,
    observed_at: DateTime<FixedOffset>,
) -> Result<(String, String), String> {
    let at = observed_at.with_timezone(&Utc);
    let clock = || at;
    let (view, set) = PaperLedger::open(database, &clock)
        .read_account_with_effective(binding, date)
        .map_err(|error| error.to_string())?;
    let receipt = set.receipt();
    let EffectiveFillScope::Epoch(source_binding) = &receipt.request.scope else {
        return Err("native observation requires exact epoch source".into());
    };
    if source_binding != binding || receipt.request.as_of != date {
        return Err("native observation scope does not match the explicit binding/date".into());
    }
    let cutover = receipt
        .cutover_at
        .ok_or("native observation lacks paper cutover")?;
    validate_read_head(
        (view.version, &view.event_hash),
        receipt.ledger_head.as_ref(),
    )?;
    // The effective owner and account head were read in the same transaction;
    // account_from_set additionally checks its owner-issued opening sample and
    // projection receipt before any amount is rendered.
    let account = account_from_set(&view, binding, &set, cutover, at)?;
    render_observation(&account, binding, receipt, date, observed_at)
}

fn account_from_set(
    view: &Projection,
    binding: &AccountBinding,
    set: &VerifiedEffectiveFillSet,
    cutover: DateTime<Utc>,
    observed_at: DateTime<Utc>,
) -> Result<Observation, String> {
    // This owner-issued sample also enforces unresolved price-dispute exclusion;
    // inspecting raw rows alone is insufficient authority for account amounts.
    let sample = set.opening_inventory_sample().map_err(|e| e.to_string())?;
    if sample.seed_binding.as_ref() != Some(binding)
        || sample.projection_hash != set.receipt().projection_hash
    {
        return Err("native opening inventory is not bound to this exact source".into());
    }
    let mut account = observation_from_projection(
        view,
        binding.epoch_id.clone(),
        cutover,
        set.receipt().request.as_of,
        observed_at,
    )?;
    for row in set.rows().map_err(|e| e.to_string())? {
        let at =
            crate::trading::paper_lot_ledger::parse_paper_fill_timestamp(row.id, &row.occurred_at)?;
        if at.date() != set.receipt().request.as_of {
            continue;
        }
        let (count, quantity) = match row.direction.as_str() {
            "buy" => (&mut account.buy_fills, &mut account.buy_quantity),
            "sell" => (&mut account.sell_fills, &mut account.sell_quantity),
            _ => return Err("unsupported effective direction".into()),
        };
        *count = count.checked_add(1).ok_or("fill count overflow")?;
        *quantity = quantity
            .checked_add(row.quantity)
            .ok_or("fill quantity overflow")?;
    }
    Ok(account)
}

fn render_observation(
    account: &Observation,
    binding: &AccountBinding,
    receipt: &EffectiveProjectionReceipt,
    date: NaiveDate,
    observed_at: DateTime<FixedOffset>,
) -> Result<(String, String), String> {
    let account_summary =
        render_account_summary(account, date).replacen("🧾 虚拟盘归因", "🧾 虚拟盘账户日报", 1);
    let (title, body) = account_summary
        .split_once('\n')
        .ok_or("account summary missing title")?;
    let summary = format!("{title}\n策略归因不可用：新持仓起点与旧策略样本不一致。\n{body}\n策略收益、胜率与信号贡献：Unavailable；以上是当前账户账本观察。");
    let mut markdown = format!("# 虚拟盘账户账本观察 {date}\n\n{summary}\n");
    append_holdings_markdown(&mut markdown, account);
    markdown.push_str("\n## 策略归因边界\n\n策略归因状态：`Unavailable`。原因：`paper_attribution_cutover_not_aligned`。本次仅展示同版 native 账户账本观察，没有成功提交策略财务归因，没有重置旧 BR-255 epoch，没有纳入旧账损益或券商切换前亏损。期初持仓不能当作策略入场。\n");
    let evidence = serde_json::json!({
        "schema":"native-paper-daily-account-observation/v1",
        "scope":"native_account_since_cutover_observation_not_strategy_attribution",
        "observed_at":observed_at,"business_date":date,"binding":binding,"source":receipt,
        "strategy_attribution":{"state":"Unavailable","reason_code":"paper_attribution_cutover_not_aligned"},
        "fees_scope":"modeled_paper_fees_not_observed_brokerage_settlement"
    });
    markdown.push_str(&format!(
        "\n## 已验证只读来源\n\n```json\n{}\n```\n",
        serde_json::to_string_pretty(&evidence).map_err(|e| e.to_string())?
    ));
    Ok((summary, markdown))
}

fn append_holdings_markdown(markdown: &mut String, account: &Observation) {
    markdown.push_str("\n## 持仓明细\n\n| 股票 | 股数 | 切换/入场基准均价 | 券商成本参考 | 原始估值价格 | 市值 | 切换后净浮盈 | 价格原始时间 | 价格来源 |\n|---|---:|---:|---:|---:|---:|---:|---|---|\n");
    for h in &account.holdings {
        markdown.push_str(&format!(
            "| {} {} | {} | {:.3} | {} | {:.3} | {:.2} | {:+.2} | {} | {} |\n",
            inline(&h.name),
            inline(&h.code),
            h.quantity,
            h.basis_notional / h.quantity as f64,
            h.reference_cost_notional
                .map(|v| format!("{:.3}（不计新账户收益）", v / h.quantity as f64))
                .unwrap_or_else(|| "不可用".into()),
            h.mark_price,
            h.value,
            h.net_unrealized,
            local(h.observed_at),
            inline(&h.source)
        ));
    }
    markdown.push_str(&format!(
        "\n账户 epoch：`{}`。手续费累计 {:.2} 元，口径为现有模拟费用模型 `{}`；不是券商实扣。\n",
        inline(&account.epoch),
        account.fees,
        crate::trading::paper_ledger::FEE_MODEL
    ));
}

fn validate_frontier(
    actual_head: (i64, &str),
    view: &Projection,
    expected_head: Option<&(i64, String)>,
    actual_projection_hash: &str,
    expected_projection_hash: &str,
    cutover: DateTime<Utc>,
    date: NaiveDate,
    observed_at: DateTime<Utc>,
) -> Result<(), String> {
    validate_read_head(actual_head, expected_head)?;
    if actual_projection_hash != expected_projection_hash {
        return Err("paper account and attribution financial source differ".into());
    }
    validate_time(view, cutover, date, observed_at)
}
fn validate_read_head(
    actual_head: (i64, &str),
    expected_head: Option<&(i64, String)>,
) -> Result<(), String> {
    if expected_head
        .is_none_or(|(version, hash)| *version != actual_head.0 || hash != actual_head.1)
    {
        return Err("paper account and effective financial read head differ".into());
    }
    Ok(())
}

fn validate_time(
    view: &Projection,
    cutover: DateTime<Utc>,
    date: NaiveDate,
    observed_at: DateTime<Utc>,
) -> Result<(), String> {
    if cutover > observed_at
        || view.as_of > observed_at
        || day(cutover) > date
        || day(view.as_of) > date
        || date > day(observed_at)
        || view
            .marks
            .values()
            .any(|mark| mark.observed_at > observed_at || day(mark.observed_at) > date)
    {
        return Err("paper account contains later financial or valuation facts".into());
    }
    Ok(())
}

fn observation_from_projection(
    view: &Projection,
    epoch: String,
    cutover: DateTime<Utc>,
    date: NaiveDate,
    observed_at: DateTime<Utc>,
) -> Result<Observation, String> {
    validate_time(view, cutover, date, observed_at)?;
    let equity = view.equity().map_err(|e| e.to_string())?.cny();
    let unrealized = view.unrealized_pnl().map_err(|e| e.to_string())?.cny();
    let mut holdings = BTreeMap::<String, Holding>::new();
    for lot in &view.lots {
        let mark = view.marks.get(&lot.code).ok_or("missing held mark")?;
        let h = holdings.entry(lot.code.clone()).or_insert_with(|| Holding {
            code: lot.code.clone(),
            name: lot.name.clone(),
            quantity: 0,
            basis_notional: 0.0,
            reference_cost_notional: Some(0.0),
            mark_price: mark.price.cny(),
            value: 0.0,
            net_unrealized: 0.0,
            observed_at: mark.observed_at,
            source: mark.source.clone(),
        });
        h.quantity = h
            .quantity
            .checked_add(u64::from(lot.quantity))
            .ok_or("holding quantity overflow")?;
        h.basis_notional += lot.basis_price.cny() * f64::from(lot.quantity);
        h.reference_cost_notional = h
            .reference_cost_notional
            .zip(lot.reported_cost)
            .map(|(sum, price)| sum + price.cny() * f64::from(lot.quantity));
        h.value += mark.price.cny() * f64::from(lot.quantity);
        h.net_unrealized += (mark.price.cny() - lot.basis_price.cny()) * f64::from(lot.quantity)
            - lot.buy_fee_remaining.cny();
    }
    let mut holdings = holdings.into_values().collect::<Vec<_>>();
    holdings.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.code.cmp(&b.code))
    });
    let current_prices = holdings.iter().all(|h| day(h.observed_at) == date);
    let (daily_pnl, daily_unavailable) = if !current_prices {
        (None, "持仓价格不是报告日估值，不能计算当日盈亏")
    } else if day(cutover) == date {
        (None, "切换首日没有上一交易日模拟收盘基线")
    } else if let Ok(previous) = crate::calendar::verified_prev_a_share_trading_day(date) {
        if let Some(baseline) = view.closes.get(&previous) {
            (Some(equity - baseline.cny()), "")
        } else {
            (None, "缺少准确上一交易日模拟收盘基线")
        }
    } else {
        (None, "上一交易日基线不可核验")
    };
    Ok(Observation {
        epoch,
        cutover,
        cash: view.cash.cny(),
        value: equity - view.cash.cny(),
        equity,
        since_cutover: equity - view.seed_equity.cny(),
        realized: view.realized_pnl.cny(),
        unrealized,
        fees: view.fees.cny(),
        daily_pnl,
        daily_unavailable,
        buy_fills: 0,
        sell_fills: 0,
        buy_quantity: 0,
        sell_quantity: 0,
        holdings,
    })
}
fn render_account_summary(account: &Observation, date: NaiveDate) -> String {
    let mut lines = vec![
        format!("🧾 虚拟盘归因 {date}"),
        "━━━━━━━━━━━━━━━━━━━━".into(),
        if account.buy_fills == 0 && account.sell_fills == 0 {
            format!(
                "重点：今天没有新成交；当前 {} 只持仓，切换后净盈亏 {:+.2} 元。",
                account.holdings.len(),
                account.since_cutover
            )
        } else {
            format!(
                "重点：今日买入 {} 笔、卖出 {} 笔；切换后净盈亏 {:+.2} 元。",
                account.buy_fills, account.sell_fills, account.since_cutover
            )
        },
        format!(
            "权益 {:.2} 元｜现金 {:.2}｜持仓市值 {:.2}",
            account.equity, account.cash, account.value
        ),
        format!(
            "切换后：已实现 {:+.2}｜净浮盈 {:+.2}｜累计模型费用 {:.2} 元",
            account.realized, account.unrealized, account.fees
        ),
        account
            .daily_pnl
            .map(|pnl| format!("今日净盈亏 {pnl:+.2} 元（上一交易日收盘基线）"))
            .unwrap_or_else(|| format!("今日净盈亏不可用：{}。", account.daily_unavailable)),
        format!(
            "收益起点 {}；券商原始成本与切换前亏损不计入新账户。",
            local(account.cutover)
        ),
        format!(
            "今日成交：买入 {} 笔/{} 股；卖出 {} 笔/{} 股",
            account.buy_fills, account.buy_quantity, account.sell_fills, account.sell_quantity
        ),
        "持仓重点（按市值）：".into(),
    ];
    for h in account.holdings.iter().take(6) {
        lines.push(format!(
            "• {} {}｜{} 股｜市值 {:.2} 元",
            inline(&h.name),
            inline(&h.code),
            h.quantity,
            h.value
        ));
        lines.push(format!(
            "  基准 {:.3} / 估值 {:.3}｜净浮盈 {:+.2} 元",
            h.basis_notional / h.quantity as f64,
            h.mark_price,
            h.net_unrealized
        ));
        lines.push(format!(
            "  价格时间 {}｜{}",
            local(h.observed_at),
            inline(&h.source)
        ));
    }
    if account.holdings.len() > 6 {
        lines.push(format!(
            "其余 {} 只见完整日报。",
            account.holdings.len() - 6
        ));
    }
    if account.holdings.is_empty() {
        lines.push("当前无持仓。".into());
    }
    lines.push("当前是账本复盘；估值时间保留原记录，不能据此推定实时行情或签发买卖指令。".into());
    lines.join("\n")
}
fn day(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
        .date_naive()
}
fn local(at: DateTime<Utc>) -> String {
    at.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
        .format("%m-%d %H:%M:%S")
        .to_string()
}
fn inline(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() && *ch != '|')
        .take(80)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading::paper_ledger::{
        seed_projection, Mark, Money, RiskPolicyV1, SeedLot, SeedManifest,
    };
    use chrono::TimeZone;

    fn clock() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 9, 13, 13, 0).unwrap()
    }
    fn manifest() -> SeedManifest {
        let at = clock();
        SeedManifest {
            account_id: "TEST_CODE_report_account".into(),
            epoch_id: "TEST_CODE_report_epoch".into(),
            command_id: "TEST_CODE_seed".into(),
            cutover_at: at,
            account_effective_at: at,
            positions_effective_at: at,
            source_reference: "TEST_CODE_user_snapshot".into(),
            source_hash: "a".repeat(64),
            approved_by: "TEST_CODE_user".into(),
            cash: Money::from_cny(7254.94).unwrap(),
            original_total: Money::from_cny(11554.94).unwrap(),
            excluded_residual: None,
            lots: vec![SeedLot {
                code: "TEST_CODE_002131".into(),
                name: "fixture".into(),
                quantity: 1000,
                reported_cost: Some(Money::from_cny(10.086).unwrap()),
                sellable_from: None,
                sellability_evidence: None,
            }],
            marks: vec![Mark {
                code: "TEST_CODE_002131".into(),
                price: Money::from_cny(4.3).unwrap(),
                observed_at: at,
                source: "TEST_CODE_original_screenshot".into(),
            }],
            policy: RiskPolicyV1::default(),
        }
    }
    fn projection() -> Projection {
        seed_projection(&manifest()).unwrap()
    }
    #[test]
    fn native_seed_reference_cost_is_not_the_new_accounts_loss_or_daily_return() {
        let view = projection();
        let a = observation_from_projection(
            &view,
            "TEST_CODE_report_epoch".into(),
            clock(),
            day(clock()),
            clock(),
        )
        .unwrap();
        assert_eq!(a.equity, 11554.94);
        assert_eq!(a.since_cutover, 0.0);
        assert_eq!(a.unrealized, 0.0);
        assert_eq!(a.fees, 0.0);
        assert!(a.daily_pnl.is_none());
        assert_eq!(a.holdings[0].reference_cost_notional, Some(10086.0));
        assert_eq!(a.holdings[0].basis_notional, 4300.0);
        let summary = render_account_summary(&a, day(clock()));
        assert!(summary.contains("今天没有新成交"));
        assert!(summary.contains("切换后净盈亏 +0.00"));
        assert!(summary.contains("1000 股｜市值 4300.00"));
        assert!(!summary.contains("-5786"));
        assert!(summary.contains("切换首日没有上一交易日"));
    }
    #[test]
    fn native_observation_does_not_refresh_original_mark_times_or_make_daily_return() {
        let view = projection();
        let later = clock() + chrono::Duration::days(3);
        let a = observation_from_projection(
            &view,
            "TEST_CODE_report_epoch".into(),
            clock(),
            day(later),
            later,
        )
        .unwrap();
        assert!(a.daily_pnl.is_none());
        assert_eq!(a.holdings[0].observed_at, clock());
        assert!(a.daily_unavailable.contains("不是报告日"));
        assert!(render_account_summary(&a, day(later)).contains("10-09 21:13:00"));
    }
    #[test]
    fn native_observation_rejects_future_prices_ledger_and_pre_cutover_report() {
        let mut view = projection();
        assert!(observation_from_projection(
            &view,
            "TEST_CODE".into(),
            clock(),
            day(clock() - chrono::Duration::days(1)),
            clock()
        )
        .is_err());
        view.marks.values_mut().next().unwrap().observed_at =
            clock() + chrono::Duration::seconds(1);
        assert!(observation_from_projection(
            &view,
            "TEST_CODE".into(),
            clock(),
            day(clock()),
            clock()
        )
        .is_err());
        let mut view = projection();
        view.as_of = clock() + chrono::Duration::seconds(1);
        assert!(observation_from_projection(
            &view,
            "TEST_CODE".into(),
            clock(),
            day(clock()),
            clock()
        )
        .is_err());
    }
    #[test]
    fn native_detail_requires_same_observed_head_and_effective_projection() {
        let view = projection();
        let head = (2, "TEST_CODE_head".to_string());
        let newer_head = (3, "TEST_CODE_head".to_string());
        let different_head = (2, "TEST_CODE_other_head".to_string());
        let args = |head_ref, actual_hash| {
            validate_frontier(
                (2, "TEST_CODE_head"),
                &view,
                head_ref,
                actual_hash,
                "TEST_CODE_projection",
                clock(),
                day(clock()),
                clock(),
            )
        };
        assert!(args(Some(&head), "TEST_CODE_projection").is_ok());
        assert!(args(None, "TEST_CODE_projection").is_err());
        assert!(args(Some(&newer_head), "TEST_CODE_projection").is_err());
        assert!(args(Some(&different_head), "TEST_CODE_projection").is_err());
        assert!(args(Some(&head), "TEST_CODE_old_projection").is_err());
    }
    #[test]
    fn native_observation_only_handles_known_join_unavailability_not_integrity_or_storage() {
        let code = "paper_attribution_cutover_not_aligned";
        assert!(permits_observation(ReplayErrorClass::Unavailable, code));
        assert!(!permits_observation(
            ReplayErrorClass::FailedIntegrity,
            code
        ));
        assert!(!permits_observation(ReplayErrorClass::Storage, code));
        assert!(!permits_observation(
            ReplayErrorClass::Unavailable,
            "effective_paper_unavailable"
        ));
        assert!(!permits_observation(
            ReplayErrorClass::Unavailable,
            "attribution_epoch_unavailable"
        ));
    }
    #[test]
    fn old_attribution_epoch_with_new_seed_renders_verified_account_without_strategy_fallback() {
        use crate::database::attribution_epochs::{AttributionEpochStore, EpochActivationRequest};
        use crate::performance::attribution_epoch::EpochActivationSource;
        use crate::performance::attribution_replay::commit_effective_window;
        use crate::trading::paper_ledger::PaperCommand;
        use diesel::connection::SimpleConnection;
        let directory = tempfile::tempdir().unwrap();
        let db = DatabaseManager::open_isolated_for_test(
            directory.path().join("TEST_CODE_daily_observation.db"),
        )
        .unwrap();
        let offset = FixedOffset::east_opt(8 * 3600).unwrap();
        let old_epoch = AttributionEpochStore::new(&db)
            .activate_once(EpochActivationRequest {
                source: EpochActivationSource::Cli,
                invoked_at: offset.with_ymd_and_hms(2026, 9, 1, 15, 40, 0).unwrap(),
            })
            .unwrap();
        // This ordinary seeded fixture has no NativeSnapshotPaperV1 sealed
        // activation proof. Declare the existing CatalogV2 test contract;
        // 0/0 with a nonempty unsealed paper namespace must remain rejected.
        db.get_conn()
            .unwrap()
            .batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=2")
            .unwrap();
        let seed = manifest();
        let binding = seed.binding().unwrap();
        let ledger = PaperLedger::open(&db, &clock);
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let before = ledger.read(&binding).unwrap();
        ledger
            .read_account_with_effective(&binding, day(clock()))
            .unwrap_or_else(|error| panic!("TEST_CODE invalid effective fixture: {error}"));
        let at = clock().with_timezone(&offset);
        let error =
            commit_effective_window(&db, binding.clone(), day(clock()), 30, at).unwrap_err();
        assert_eq!(
            error.class(),
            ReplayErrorClass::Unavailable,
            "actual error: {error:?}"
        );
        assert_eq!(error.code(), "paper_attribution_cutover_not_aligned");
        assert!(native_daily_observation_is_permitted(&error));
        let (summary, md) =
            render_native_daily_account_observation(&db, &binding, day(clock()), at).unwrap();
        assert!(summary.contains("权益 11554.94"));
        assert!(summary.contains("切换后净盈亏 +0.00"));
        assert!(summary.contains("策略归因不可用"));
        assert!(summary.contains("策略收益、胜率与信号贡献：Unavailable"));
        assert!(!summary.contains("-13,996"));
        assert!(!summary.contains("NewsCatalyst"));
        assert!(md.contains("没有成功提交策略财务归因"));
        assert!(md.contains("native-paper-daily-account-observation/v1"));
        assert!(md.contains("TEST_CODE_report_epoch"));
        assert_eq!(ledger.read(&binding).unwrap(), before);
        assert_eq!(
            AttributionEpochStore::new(&db).verify_active().unwrap(),
            old_epoch
        );
        let mut wrong = binding.clone();
        wrong.manifest_hash = "b".repeat(64);
        assert!(render_native_daily_account_observation(&db, &wrong, day(clock()), at).is_err());
        assert!(render_native_daily_account_observation(
            &db,
            &binding,
            day(clock()),
            (clock() - chrono::Duration::seconds(1)).with_timezone(&offset)
        )
        .is_err());
    }
}
