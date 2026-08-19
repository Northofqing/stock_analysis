//! R-12 盘后回测 — 用 15 分钟 K线数据回测策略信号。
//!
//! 背景: 盘后复盘 (R-01~R-11) 全为当日回顾 + 明日计划, 无策略回测。
//! 用户要求补上, 明确「用 15 分钟 K线数据进行回测」, 三个信号全回测:
//!   1. 虚拟仓买卖信号 (paper_trades 历史成交, 8 类 virtual_reason)
//!   2. boll_macd 信号 (15min 滑动窗口 detect_boll_macd_signal)
//!   3. T0 做T 信号 — 依赖实时五档盘口 + 分时均价 (t0_advisor::evaluate_structured
//!      需要 MagicTdxT0Evidence 实时数据), 历史回放不可得 → 标注不可回测。
//!
//! 数据: TDX 15 分钟 K线 (get_security_bars KLINE_15MIN=1, 升序 旧→新,
//! 单次 ≤800 根 ≈ 50 交易日, 覆盖虚拟仓 7/14 起全部信号)。KlineData 只有
//! NaiveDate 无时分 → 本模块直接操作原始 SecurityBar 做时间对齐, 转
//! KlineData 仅用于喂 detect_boll_macd_signal (该函数只用数组顺序)。
//!
//! 纯计算函数与网络/DB 薄壳分离: 单测不依赖网络。

use crate::magic_compat::SecurityBar;
use chrono::{Duration, NaiveDate, NaiveDateTime};

use crate::data_gateway::historical_bars::HistoricalBarsGateway;
use crate::data_provider::{AdjustType, KlineData};
use crate::strategy::boll_macd::{detect_boll_macd_signal, BollMacdAction, BollMacdSignal};

/// 一组信号的统计结果 (单窗口)。reason 为信号来源, window_bars 为
/// forward return 窗口 (4 根 = 1h, 16 根 = 1 交易日)。
#[derive(Debug, Clone, PartialEq)]
pub struct SignalGroup {
    pub reason: String,
    pub window_bars: usize,
    pub count: usize,
    pub win_rate: Option<f64>,
    pub avg_ret: Option<f64>,
    pub mfe: Option<f64>,
    pub mae: Option<f64>,
}

impl SignalGroup {
    fn is_under_sampled(&self) -> bool {
        self.count < 10
    }
}

/// R-12 回测汇总结果。
#[derive(Debug, Clone, Default)]
pub struct R12BacktestResult {
    /// 虚拟仓买入信号按 virtual_reason 分组 (每 reason × 窗口一个组)
    pub virtual_buy: Vec<SignalGroup>,
    /// 虚拟仓卖出信号 (BR-234 四大铁律卖出) 分组
    pub virtual_sell: Vec<SignalGroup>,
    /// 破损成交 (price < 1.0, 7/14-16 Breakout 价格 feed 异常期) 排除数
    pub broken_excluded: usize,
    /// boll_macd 15min 信号统计
    pub boll_macd: Vec<SignalGroup>,
    /// 取数失败跳过的 code (出声)
    pub skipped_codes: Vec<String>,
    /// 无 15min bar 可对齐的信号数 (集合竞价/收盘后信号, 出声)
    pub unaligned_signals: usize,
}

/// 虚拟仓信号条目 (paper_trades Filled)。
#[derive(Debug, Clone)]
pub struct SignalEntry {
    pub code: String,
    pub name: String,
    pub direction: String, // "buy" | "sell"
    pub price: f64,
    pub virtual_reason: String,
    pub ts_utc: NaiveDateTime,
}

/// 虚拟仓买入信号分组前缀: BR-234 卖出归 sell 组。
const SELL_REASON_PREFIX: &str = "BR-234四大铁律卖出";
/// 破损成交价阈值: 真实 A 股价格不可能低于 1 元 (virtual_pnl 同款判定)
const BROKEN_PRICE_THRESHOLD: f64 = 1.0;
/// 价格 feed 异常日窗口 (7/14-16): 该窗口 190 笔买入全为破损成交 —
/// 002463 沪电股份真实价 121 元当日记 0.07-10 元, Momentum 反向放大到 1680 元。
/// 按日期整体排除 (virtual_pnl 同款先例), 卖出自 8/10 BR-234 起不受影响。
pub const BROKEN_FILL_DAYS: [&str; 3] = ["2026-07-14", "2026-07-15", "2026-07-16"];

/// 破损成交判定: 坏日期窗口内 或 价格低于阈值 (兜底未来异常)。
pub fn is_broken_fill(ts_utc: &NaiveDateTime, price: f64) -> bool {
    if price < BROKEN_PRICE_THRESHOLD {
        return true;
    }
    BROKEN_FILL_DAYS
        .iter()
        .any(|day| ts_utc.format("%Y-%m-%d").to_string().as_str() == *day)
}
/// forward return 窗口 (根 15min bar)
pub const WINDOWS_BARS: [usize; 2] = [4, 16];
/// boll_macd 买入动作
const BUY_ACTIONS: [BollMacdAction; 2] = [BollMacdAction::BottomBuy, BollMacdAction::UptrendStart];

// ============================================================
// 纯计算 (可测, 不依赖网络/DB)
// ============================================================

/// 在升序 15min bars 里定位信号时刻所在的 bar: 返回最后一个
/// (date, hour, minute) <= 信号北京时间的 bar index。无匹配返回 None
/// (集合竞价 9:25 / 收盘后 15:00 之后信号, 无 bar 承载 → 对齐失败)。
pub fn locate_signal_bar(bars: &[SecurityBar], signal_utc: NaiveDateTime) -> Option<usize> {
    let bj = signal_utc + Duration::hours(8);
    bars.iter()
        .position(|bar| {
            let date = NaiveDate::from_ymd_opt(bar.year as i32, bar.month, bar.day);
            let Some(date) = date else {
                return false;
            };
            let bar_time = date
                .and_hms_opt(bar.hour, bar.minute, 0)
                .unwrap_or_else(|| {
                    NaiveDate::from_ymd_opt(2020, 1, 1)
                        .expect("valid fallback date")
                        .and_hms_opt(0, 0, 0)
                        .expect("valid fallback time")
                });
            bar_time > bj
        })
        .map(|idx| idx.saturating_sub(1))
}

/// 计算 forward return: (bars[idx+n].close - base_price) / base_price。
/// idx+n 越界或 base_price <= 0 返回 None。
pub fn forward_return(bars: &[SecurityBar], idx: usize, n: usize, base_price: f64) -> Option<f64> {
    if base_price <= 0.0 {
        return None;
    }
    let target = bars.get(idx + n)?;
    Some((target.close - base_price) / base_price)
}

/// 从收益序列聚合 SignalGroup (胜率 = ret > 0 占比, MFE/MAE = 极值)。
pub fn aggregate_group(reason: &str, window_bars: usize, rets: &[f64]) -> SignalGroup {
    let count = rets.len();
    let win_rate = if count > 0 {
        Some(rets.iter().filter(|r| **r > 0.0).count() as f64 / count as f64)
    } else {
        None
    };
    let avg_ret = if count > 0 {
        Some(rets.iter().sum::<f64>() / count as f64)
    } else {
        None
    };
    let mfe = if count > 0 {
        rets.iter().cloned().fold(f64::MIN, f64::max)
    } else {
        f64::MIN
    };
    let mae = if count > 0 {
        rets.iter().cloned().fold(f64::MAX, f64::min)
    } else {
        f64::MAX
    };
    SignalGroup {
        reason: reason.to_string(),
        window_bars,
        count,
        win_rate,
        avg_ret,
        mfe: if count > 0 { Some(mfe) } else { None },
        mae: if count > 0 { Some(mae) } else { None },
    }
}

/// 升序 SecurityBar → 降序 KlineData (data[0]=最新, 喂 detect_boll_macd_signal)。
/// date 填 bar 日期 (时分丢失不影响 detect — 只用数组顺序)。
pub fn bars_to_desc_kline(bars: &[SecurityBar]) -> Vec<KlineData> {
    let mut out: Vec<KlineData> = bars
        .iter()
        .rev()
        .map(|bar| {
            let date = NaiveDate::from_ymd_opt(bar.year as i32, bar.month, bar.day)
                .unwrap_or_else(|| NaiveDate::from_ymd_opt(2020, 1, 1).expect("valid fallback"));
            KlineData {
                date,
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                volume: bar.vol,
                amount: bar.amount,
                pct_chg: 0.0,
                intraday_price: None,
                settled: true,
                pe_ratio: None,
                pb_ratio: None,
                turnover_rate: None,
                market_cap: None,
                circulating_cap: None,
                eps: None,
                roe: None,
                revenue_yoy: None,
                net_profit_yoy: None,
                gross_margin: None,
                net_margin: None,
                sharpe_ratio: None,
                financials_history: None,
                valuation_history: None,
                consensus: None,
                industry: None,
                is_limit_up: false,
                is_limit_down: false,
                is_suspended: false,
                adjust: AdjustType::None,
            }
        })
        .collect();
    out.sort_by_key(|bar| std::cmp::Reverse(bar.date));
    out
}

/// 在降序 KlineData 上滑动窗口跑 detect_boll_macd_signal:
/// 对每个位置 j (0..=len-35, data[j] 作为该时刻最新 bar, 窗口 data[j..]
/// 含 ≥35 根), 若 action ∈ 买入信号则返回 (j, action)。
/// 调用方用 j-n 取未来第 n 根 (j 越小越新, j-n 是 j 之后第 n 根)。
pub fn scan_boll_macd_buys(desc: &[KlineData]) -> Vec<(usize, BollMacdAction)> {
    let mut signals = Vec::new();
    let max_j = desc.len().saturating_sub(35);
    for j in 0..=max_j {
        let sig: BollMacdSignal = detect_boll_macd_signal(&desc[j..]);
        if BUY_ACTIONS.contains(&sig.action) {
            signals.push((j, sig.action));
        }
    }
    signals
}

/// 降序数组上, 信号在位置 j, 未来第 n 根 = j - n。计算 forward return。
pub fn forward_return_desc(desc: &[KlineData], j: usize, n: usize) -> Option<f64> {
    let base = desc.get(j)?.close;
    let target = desc.get(j.checked_sub(n)?)?.close;
    if base <= 0.0 {
        return None;
    }
    Some((target - base) / base)
}

// ============================================================
// 网络/DB 薄壳 (spawn_blocking 内调用)
// ============================================================

/// 读 paper_trades 近 days 天 Filled 信号 (buy + sell), 破损价排除计数。
pub fn read_paper_signal_entries(days: usize) -> Result<(Vec<SignalEntry>, usize), String> {
    use crate::database::DatabaseManager;
    use diesel::query_dsl::RunQueryDsl;

    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::Text)]
        code: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        name: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        direction: String,
        #[diesel(sql_type = diesel::sql_types::Double)]
        price: f64,
        #[diesel(sql_type = diesel::sql_types::Text)]
        virtual_reason: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        ts: String,
    }

    let db = DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| format!("db conn: {e}"))?;
    let sql = format!(
        "SELECT code, name, direction, price, virtual_reason, ts \
         FROM paper_trades \
         WHERE status='Filled' AND ts >= datetime('now', '-{days} days')"
    );
    let rows: Vec<Row> = diesel::sql_query(sql)
        .load::<Row>(&mut conn)
        .map_err(|e| format!("paper_trades read: {e}"))?;

    let mut entries = Vec::with_capacity(rows.len());
    let mut broken = 0_usize;
    for row in rows {
        let ts_utc = match NaiveDateTime::parse_from_str(&row.ts, "%Y-%m-%d %H:%M:%S") {
            Ok(ts) => ts,
            Err(_) => {
                log::warn!("[r12-backtest] unparseable ts {} code={}", row.ts, row.code);
                continue;
            }
        };
        // 破损成交排除: 7/14-16 价格 feed 异常期 (virtual_pnl 同款先例) — 该窗口
        // 全部 190 笔买入价格偏低/放大 (沪电股份真实 121 元, 当日记 0.07-10 元;
        // Momentum 反向放大到 1680 元), price<1.0 阈值只能覆盖部分 → 按日期整体排除。
        // sell 不受影响 (卖出自 BR-234 8/10 起, 价格正常)。
        if is_broken_fill(&ts_utc, row.price) {
            broken += 1;
            continue;
        }
        let ts_utc = match NaiveDateTime::parse_from_str(&row.ts, "%Y-%m-%d %H:%M:%S") {
            Ok(ts) => ts,
            Err(_) => {
                log::warn!("[r12-backtest] unparseable ts {} code={}", row.ts, row.code);
                continue;
            }
        };
        entries.push(SignalEntry {
            code: row.code,
            name: row.name,
            direction: row.direction,
            price: row.price,
            virtual_reason: row.virtual_reason,
            ts_utc,
        });
    }
    Ok((entries, broken))
}

/// 虚拟仓信号回测: 每笔信号 → 15min bars 对齐 → forward return 分组。
type TechnicalBarsCache = std::collections::HashMap<String, Result<Vec<SecurityBar>, String>>;

fn load_technical_bars_cached<'a, Loader>(
    cache: &'a mut TechnicalBarsCache,
    code: &str,
    loader: Loader,
) -> Result<&'a [SecurityBar], &'a str>
where
    Loader: FnOnce(&str) -> Result<Vec<SecurityBar>, String>,
{
    cache
        .entry(code.to_owned())
        .or_insert_with(|| loader(code))
        .as_ref()
        .map(Vec::as_slice)
        .map_err(String::as_str)
}

pub fn backtest_virtual_signals(days: usize) -> Result<R12BacktestResult, String> {
    let (entries, broken) = read_paper_signal_entries(days)?;
    let gateway = HistoricalBarsGateway::new();
    let mut cache = TechnicalBarsCache::new();
    let mut loader = |code: &str| {
        gateway
            .fifteen_min_bars(code, 800)
            .map_err(|error| error.to_string())
    };
    backtest_virtual_signals_with_entries_and_cache(&entries, broken, &mut cache, &mut loader)
}

fn backtest_virtual_signals_with_entries_and_cache<Loader>(
    entries: &[SignalEntry],
    broken: usize,
    bars_by_code: &mut TechnicalBarsCache,
    loader: &mut Loader,
) -> Result<R12BacktestResult, String>
where
    Loader: FnMut(&str) -> Result<Vec<SecurityBar>, String>,
{
    if entries.is_empty() {
        return Ok(R12BacktestResult {
            broken_excluded: broken,
            ..Default::default()
        });
    }

    let mut skipped_codes = std::collections::BTreeSet::new();
    let mut unaligned = 0_usize;

    // buy 分组累积: reason → window → rets
    let mut buy_rets: std::collections::BTreeMap<String, Vec<Vec<f64>>> =
        std::collections::BTreeMap::new();
    let mut sell_rets: Vec<Vec<f64>> = vec![Vec::new(), Vec::new()];

    for entry in entries {
        // BR-239: success and failure are both cached for the whole run. A
        // failed code must not be reacquired once per duplicate signal row.
        let bars = match load_technical_bars_cached(bars_by_code, &entry.code, |code| loader(code))
        {
            Ok(bars) => bars,
            Err(error) => {
                log::warn!(
                    "[r12-backtest] 15min bars failed {} ({}): {error}",
                    entry.code,
                    entry.name
                );
                skipped_codes.insert(entry.code.clone());
                continue;
            }
        };
        let Some(idx) = locate_signal_bar(bars, entry.ts_utc) else {
            unaligned += 1;
            continue;
        };
        let is_sell =
            entry.virtual_reason.starts_with(SELL_REASON_PREFIX) || entry.direction == "sell";
        for (wi, window) in WINDOWS_BARS.iter().enumerate() {
            if let Some(ret) = forward_return(bars, idx, *window, entry.price) {
                if is_sell {
                    sell_rets[wi].push(ret);
                } else {
                    let group = buy_rets
                        .entry(entry.virtual_reason.clone())
                        .or_insert_with(|| vec![Vec::new(), Vec::new()]);
                    group[wi].push(ret);
                }
            }
        }
    }

    let mut result = R12BacktestResult {
        broken_excluded: broken,
        skipped_codes: skipped_codes.into_iter().collect(),
        unaligned_signals: unaligned,
        ..Default::default()
    };
    for (reason, windows) in &buy_rets {
        for (wi, window) in WINDOWS_BARS.iter().enumerate() {
            if windows[wi].is_empty() {
                continue;
            }
            result
                .virtual_buy
                .push(aggregate_group(reason, *window, &windows[wi]));
        }
    }
    for (wi, window) in WINDOWS_BARS.iter().enumerate() {
        if sell_rets[wi].is_empty() {
            continue;
        }
        result.virtual_sell.push(aggregate_group(
            "虚拟仓卖出(四大铁律)",
            *window,
            &sell_rets[wi],
        ));
    }
    Ok(result)
}

/// boll_macd 15min 回测: 每只票 800 根 → 滑动窗口信号 → forward return。
pub fn backtest_boll_macd_15min(codes: &[String]) -> Result<Vec<SignalGroup>, String> {
    let gateway = HistoricalBarsGateway::new();
    let mut cache = TechnicalBarsCache::new();
    let mut loader = |code: &str| {
        gateway
            .fifteen_min_bars(code, 800)
            .map_err(|error| error.to_string())
    };
    backtest_boll_macd_15min_with_cache(codes, &mut cache, &mut loader)
}

fn backtest_boll_macd_15min_with_cache<Loader>(
    codes: &[String],
    bars_by_code: &mut TechnicalBarsCache,
    loader: &mut Loader,
) -> Result<Vec<SignalGroup>, String>
where
    Loader: FnMut(&str) -> Result<Vec<SecurityBar>, String>,
{
    let mut rets_by_action: std::collections::BTreeMap<String, Vec<Vec<f64>>> =
        std::collections::BTreeMap::new();
    let mut skipped = Vec::new();
    for code in codes {
        let bars = match load_technical_bars_cached(bars_by_code, code, |code| loader(code)) {
            Ok(bars) => bars,
            Err(error) => {
                log::warn!("[r12-backtest] boll_macd 15min bars failed {code}: {error}");
                skipped.push(code.clone());
                continue;
            }
        };
        let desc = bars_to_desc_kline(bars);
        let signals = scan_boll_macd_buys(&desc);
        for (j, action) in signals {
            let label = format!("{action:?}");
            let group = rets_by_action
                .entry(label)
                .or_insert_with(|| vec![Vec::new(), Vec::new()]);
            for (wi, window) in WINDOWS_BARS.iter().enumerate() {
                if let Some(ret) = forward_return_desc(&desc, j, *window) {
                    group[wi].push(ret);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (label, windows) in &rets_by_action {
        for (wi, window) in WINDOWS_BARS.iter().enumerate() {
            if windows[wi].is_empty() {
                continue;
            }
            out.push(aggregate_group(label, *window, &windows[wi]));
        }
    }
    if !skipped.is_empty() {
        log::warn!(
            "[r12-backtest] boll_macd skipped {} codes: {}",
            skipped.len(),
            skipped.join(",")
        );
    }
    Ok(out)
}

/// 渲染 R-12 段文本 (三段式: 虚拟仓信号 / boll_macd / T0 标注)。
pub fn render_r12(result: &R12BacktestResult) -> String {
    let mut lines = vec![
        "📈 R-12 盘后回测 (15min K线)".to_string(),
        "━━━━━━━━━━━━━━━━━━━━".to_string(),
    ];

    lines.push("【虚拟仓信号回测】".to_string());
    if result.virtual_buy.is_empty() && result.virtual_sell.is_empty() {
        lines.push("· 近 30 天无 Filled 信号或全部对齐失败".to_string());
    } else {
        lines.push("买入信号 (按来源):".to_string());
        for group in &result.virtual_buy {
            lines.push(format!(
                "· {} ({}根) 样本{} 胜率{} 均收益{:+.2}% MFE {:+.1}% MAE {:+.1}%{}",
                group.reason,
                group.window_bars,
                group.count,
                pct(group.win_rate),
                group.avg_ret.unwrap_or(0.0) * 100.0,
                group.mfe.unwrap_or(0.0) * 100.0,
                group.mae.unwrap_or(0.0) * 100.0,
                if group.is_under_sampled() {
                    " (样本不足)"
                } else {
                    ""
                }
            ));
        }
        lines.push("卖出信号 (四大铁律, 跌=卖对):".to_string());
        for group in &result.virtual_sell {
            lines.push(format!(
                "· {} ({}根) 样本{} 胜率{} 均收益{:+.2}% MFE {:+.1}% MAE {:+.1}%{}",
                group.reason,
                group.window_bars,
                group.count,
                pct(group.win_rate),
                group.avg_ret.unwrap_or(0.0) * 100.0,
                group.mfe.unwrap_or(0.0) * 100.0,
                group.mae.unwrap_or(0.0) * 100.0,
                if group.is_under_sampled() {
                    " (样本不足)"
                } else {
                    ""
                }
            ));
        }
    }
    if result.broken_excluded > 0 {
        lines.push(format!(
            "⚠️ 排除破损成交 {} 笔 (7/14-16 价格 feed 异常期, 按日期整体排除)",
            result.broken_excluded
        ));
    }
    if result.unaligned_signals > 0 {
        lines.push(format!(
            "⚠️ 无 15min bar 对齐信号 {} 笔 (集合竞价/收盘后)",
            result.unaligned_signals
        ));
    }
    if !result.skipped_codes.is_empty() {
        lines.push(format!(
            "⚠️ 取数失败跳过 {} 只: {}",
            result.skipped_codes.len(),
            result.skipped_codes.join(",")
        ));
    }

    lines.push(String::new());
    lines.push("【boll_macd 15min 回测】".to_string());
    if result.boll_macd.is_empty() {
        lines.push("· 无买入信号 (BottomBuy/UptrendStart)".to_string());
    } else {
        for group in &result.boll_macd {
            lines.push(format!(
                "· {} ({}根) 信号{} 胜率{} 均收益{:+.2}% MFE {:+.1}% MAE {:+.1}%{}",
                group.reason,
                group.window_bars,
                group.count,
                pct(group.win_rate),
                group.avg_ret.unwrap_or(0.0) * 100.0,
                group.mfe.unwrap_or(0.0) * 100.0,
                group.mae.unwrap_or(0.0) * 100.0,
                if group.is_under_sampled() {
                    " (样本不足)"
                } else {
                    ""
                }
            ));
        }
    }

    lines.push(String::new());
    lines.push("【T0 做T 信号】".to_string());
    lines.push("· 依赖实时五档盘口 + 分时均价, 历史不可回测 (数据不可得)".to_string());

    lines.join("\n")
}

fn pct(rate: Option<f64>) -> String {
    match rate {
        Some(r) => format!("{:.0}%", r * 100.0),
        None => "-".to_string(),
    }
}

/// 供 dispatcher 用的组合入口: 虚拟仓回测 + boll_macd 回测。
pub fn run_full_backtest(days: usize) -> Result<R12BacktestResult, String> {
    let (entries, broken) = read_paper_signal_entries(days)?;
    let gateway = HistoricalBarsGateway::new();
    run_full_backtest_with_entries_and_loader(&entries, broken, |code| {
        gateway
            .fifteen_min_bars(code, 800)
            .map_err(|error| error.to_string())
    })
}

fn run_full_backtest_with_entries_and_loader<Loader>(
    entries: &[SignalEntry],
    broken: usize,
    mut loader: Loader,
) -> Result<R12BacktestResult, String>
where
    Loader: FnMut(&str) -> Result<Vec<SecurityBar>, String>,
{
    let mut cache = TechnicalBarsCache::new();
    let mut result =
        backtest_virtual_signals_with_entries_and_cache(entries, broken, &mut cache, &mut loader)?;
    let mut seen = std::collections::BTreeSet::new();
    for e in entries {
        seen.insert(e.code.clone());
    }
    let codes: Vec<String> = seen.into_iter().collect();
    if codes.is_empty() {
        return Ok(result);
    }
    result.boll_macd = backtest_boll_macd_15min_with_cache(&codes, &mut cache, &mut loader)?;
    Ok(result)
}

// ============================================================
// 测试
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn br239_failed_technical_bars_code_is_negative_cached() {
        let mut cache = std::collections::HashMap::new();
        let calls = std::cell::Cell::new(0_usize);
        for _ in 0..3 {
            let result = load_technical_bars_cached(&mut cache, "TEST_CODE_000001", |_| {
                calls.set(calls.get() + 1);
                Err("TEST_CODE no_verified_batch".to_owned())
            });
            assert_eq!(result.unwrap_err(), "TEST_CODE no_verified_batch");
        }
        assert_eq!(
            calls.get(),
            1,
            "failed codes must not be reacquired in one run"
        );
    }

    #[test]
    fn br239_full_backtest_loads_each_code_once_across_all_analyses() {
        let entries = vec![SignalEntry {
            code: "TEST_CODE_000001".to_owned(),
            name: "TEST_CODE sample".to_owned(),
            direction: "buy".to_owned(),
            price: 10.0,
            virtual_reason: "TEST_CODE reason".to_owned(),
            ts_utc: NaiveDate::from_ymd_opt(2026, 8, 10)
                .unwrap()
                .and_hms_opt(1, 30, 0)
                .unwrap(),
        }];
        let calls = std::cell::Cell::new(0_usize);

        run_full_backtest_with_entries_and_loader(&entries, 0, |_| {
            calls.set(calls.get() + 1);
            Ok(sample_bars())
        })
        .expect("TEST_CODE full backtest succeeds");

        assert_eq!(
            calls.get(),
            1,
            "virtual-signal and boll/macd analysis must share one run cache"
        );
    }

    fn bar(date: (i32, u32, u32), hhmm: (u32, u32), close: f64) -> SecurityBar {
        SecurityBar {
            open: close,
            close,
            high: close,
            low: close,
            vol: 1_000.0,
            amount: close * 1_000.0,
            year: date.0 as u32,
            month: date.1,
            day: date.2,
            hour: hhmm.0,
            minute: hhmm.1,
            datetime: String::new(),
        }
    }

    fn sample_bars() -> Vec<SecurityBar> {
        // 2026-08-10 全天 16 根 15min (9:30-11:30, 13:00-15:00)
        let mut bars = Vec::new();
        let d = (2026, 8, 10);
        for (h, m) in [
            (9, 30),
            (9, 45),
            (10, 0),
            (10, 15),
            (10, 30),
            (10, 45),
            (11, 0),
            (11, 15),
            (11, 30),
            (13, 0),
            (13, 15),
            (13, 30),
            (13, 45),
            (14, 0),
            (14, 15),
            (14, 30),
        ] {
            bars.push(bar(d, (h, m), 10.0));
        }
        bars
    }

    #[test]
    fn locate_signal_bar_finds_bracketing_bar() {
        let bars = sample_bars();
        // 北京时间 10:23 → 最后 bar <= 10:23 是 10:15 (idx 3)
        let utc = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(2, 23, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc), Some(3));
        // 北京时间 9:31 → 9:30 (idx 0)
        let utc = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(1, 31, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc), Some(0));
        // 北京时间 14:25 → 14:15 (idx 14)
        let utc = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(6, 25, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc), Some(14));
        // 北京时间 15:05 (盘后, 无 bar 承载) → None — 收盘后信号对齐失败
        let utc = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(7, 5, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc), None);
        // 数据早于信号 → None
        let utc = NaiveDate::from_ymd_opt(2026, 8, 11)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc), None);
    }

    #[test]
    fn forward_return_uses_close_after_n_bars() {
        let mut bars = sample_bars();
        bars[5].close = 11.0; // 10:45 bar
        bars[9].close = 12.5; // 13:00 bar
                              // idx 5, n=4 → bars[9].close = 12.5, base 11.0 → +13.64%
        let ret = forward_return(&bars, 5, 4, 11.0).expect("ret");
        assert!((ret - 0.13636).abs() < 0.001);
        // 越界
        assert_eq!(forward_return(&bars, 15, 4, 10.0), None);
        // base <= 0
        assert_eq!(forward_return(&bars, 5, 1, 0.0), None);
    }

    #[test]
    fn aggregate_group_computes_stats() {
        let rets = vec![0.05, -0.02, 0.03, 0.01, -0.01];
        let g = aggregate_group("NewsCatalyst", 4, &rets);
        assert_eq!(g.count, 5);
        assert_eq!(g.win_rate, Some(0.6));
        assert!((g.avg_ret.unwrap() - 0.012).abs() < 1e-9);
        assert_eq!(g.mfe, Some(0.05));
        assert_eq!(g.mae, Some(-0.02));
        assert!(g.is_under_sampled());
        let g2 = aggregate_group("Breakout", 16, &[0.1; 10]);
        assert!(!g2.is_under_sampled());
        let empty = aggregate_group("x", 4, &[]);
        assert_eq!(empty.count, 0);
        assert_eq!(empty.win_rate, None);
    }

    #[test]
    fn bars_to_desc_kline_reverses_order() {
        let mut bars = sample_bars();
        bars[0].close = 10.0;
        bars[15].close = 9.0;
        let desc = bars_to_desc_kline(&bars);
        assert_eq!(desc.len(), 16);
        // data[0] = 最新 (14:30)
        assert_eq!(desc[0].close, 9.0);
        assert_eq!(desc[15].close, 10.0);
        // date 单调降序
        assert!(desc[0].date >= desc[15].date);
    }

    #[test]
    fn scan_boll_macd_buys_detects_signal_and_forward_return_desc() {
        // 合成 80 根序列: 缓涨 50 根 (MACD 转正) 后急跌 30 根 (触下轨 +
        // MACD 绿柱缩短) → BottomBuy (实测 variant=1 触发 7 个信号)。
        // 注意 desc[0]=最新 (急跌末端), desc[79]=最旧 (缓涨起点) —
        // 构造 raw 时 i=0 最旧, i 越大越新。
        let mut raw: Vec<KlineData> = Vec::new();
        for i in 0..80 {
            // 0..50 缓涨 10→12; 50..80 急跌 12→9
            let close = if i < 50 {
                10.0 + (i as f64) * 0.04
            } else {
                12.0 - (i as f64 - 50.0) * 0.10
            };
            let volume = 1_000.0;
            raw.push(KlineData {
                date: NaiveDate::from_ymd_opt(2026, 8, 1)
                    .unwrap()
                    .checked_sub_days(chrono::Days::new(i as u64))
                    .expect("date"),
                open: close,
                high: close,
                low: close,
                close,
                volume,
                amount: close * volume,
                pct_chg: 0.0,
                intraday_price: None,
                settled: true,
                pe_ratio: None,
                pb_ratio: None,
                turnover_rate: None,
                market_cap: None,
                circulating_cap: None,
                eps: None,
                roe: None,
                revenue_yoy: None,
                net_profit_yoy: None,
                gross_margin: None,
                net_margin: None,
                sharpe_ratio: None,
                financials_history: None,
                valuation_history: None,
                consensus: None,
                industry: None,
                is_limit_up: false,
                is_limit_down: false,
                is_suspended: false,
                adjust: AdjustType::None,
            });
        }
        // raw[0]=最旧 → 反转使 desc[0]=最新
        let desc: Vec<KlineData> = raw.into_iter().rev().collect();
        let signals = scan_boll_macd_buys(&desc);
        assert!(!signals.is_empty(), "expected at least one buy signal");
        // forward return 在信号后可用
        let (j, _) = signals[0];
        let r4 = forward_return_desc(&desc, j, 4);
        let r16 = forward_return_desc(&desc, j, 16);
        assert!(r4.is_some() || r16.is_some());
    }

    #[test]
    fn sell_reason_prefix_matches_br234() {
        assert!("BR-234四大铁律卖出:ATR动态止损".starts_with(SELL_REASON_PREFIX));
        assert!(!("NewsCatalyst".starts_with(SELL_REASON_PREFIX)));
    }

    #[test]
    fn broken_fill_excludes_bad_window_and_low_price() {
        // 7/14-16 窗口内: 低价 + 正常价 + 放大价全部排除 (190 笔全破损)
        let d15 = NaiveDate::from_ymd_opt(2026, 7, 15)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        assert!(is_broken_fill(&d15, 0.07), "窗口内低价必须排除");
        assert!(
            is_broken_fill(&d15, 10.0),
            "窗口内正常价也须排除 (沪电真实 121 元)"
        );
        assert!(
            is_broken_fill(&d15, 1680.0),
            "窗口内放大价也须排除 (Momentum)"
        );
        // 窗口外: 仅 <1.0 排除, 低价股 (如 3 元) 保留
        let d20 = NaiveDate::from_ymd_opt(2026, 7, 20)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        assert!(is_broken_fill(&d20, 0.5), "窗口外 <1.0 兜底排除");
        assert!(!is_broken_fill(&d20, 3.5), "窗口外正常低价股保留");
        // sell 不受影响 (8/10 起)
        let d10 = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(15, 0, 0)
            .unwrap();
        assert!(!is_broken_fill(&d10, 45.0));
        // 8/11 今日成交 (38-121 元) 保留
        let d11 = NaiveDate::from_ymd_opt(2026, 8, 11)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        assert!(!is_broken_fill(&d11, 121.35));
    }
}
