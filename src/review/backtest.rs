//! R-12 买入事件研究 — 用 15 分钟 K 线衡量入场后的短期价格路径。
//! Registered business rules: BR-239, BR-247.
//!
//! 本模块不是完整策略回测，不计算买入到卖出的净胜率。它只报告：
//!   1. 虚拟仓已成交买入事件 (paper_trades Filled buy, 9 类入场来源)
//!   2. boll_macd 信号 (15min 滑动窗口 detect_boll_macd_signal)
//!   3. T0 做T 信号 — 依赖实时五档盘口 + 分时均价 (t0_advisor::evaluate_structured
//!      需要 MagicTdxT0Evidence 实时数据), 历史回放不可得 → 标注不可回测。
//!
//! 数据: TDX 15 分钟 K线 (get_security_bars KLINE_15MIN=1, 升序 旧→新,
//! 单次 ≤800 根 ≈ 50 交易日)。本模块只操作原始 SecurityBar；boll_macd
//! 通过仅含真实 close/volume 的窄观察接口消费，不构造证据外字段。
//!
//! 纯计算函数与网络/DB 薄壳分离: 单测不依赖网络。

use crate::magic_compat::SecurityBar;
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use crate::data_gateway::historical_bars::HistoricalBarsGateway;
use crate::performance::attribution::{signal_family_of, SignalFamily};
use crate::strategy::boll_macd::{
    detect_boll_macd_observations, BollMacdAction, BollMacdObservation,
};
use crate::trading::paper_lot_ledger::parse_paper_fill_timestamp;

/// 一组买入事件的统计结果（单窗口）。`up_rate` 只是终点上涨比例，
/// 不是完整经济持仓胜率。
#[derive(Debug, Clone, PartialEq)]
pub struct SignalGroup {
    pub reason: String,
    pub window_bars: usize,
    pub count: usize,
    pub up_rate: Option<f64>,
    pub avg_terminal_ret: Option<f64>,
    pub avg_mfe: Option<f64>,
    pub avg_mae: Option<f64>,
}

impl SignalGroup {
    fn is_under_sampled(&self) -> bool {
        self.count < 200
    }
}

/// R-12 买入事件研究汇总结果。
#[derive(Debug, Clone, Default)]
pub struct R12BacktestResult {
    /// 虚拟仓买入事件按明确入场族分组（每族 × 窗口一个组）。
    pub virtual_buy: Vec<SignalGroup>,
    /// 同窗口内真实卖出事实数量；只披露，不进入入场上涨率。
    pub exit_rows_excluded: usize,
    /// boll_macd 15min 信号统计
    pub boll_macd: Vec<SignalGroup>,
    /// 无 15min bar 可对齐的买入事件数（集合竞价、收盘后或覆盖外）。
    pub unaligned_signals: usize,
    /// 已对齐但未来 4/16 根窗口尚不完整的事件窗口数（右删失）。
    pub censored_windows: usize,
}

/// 已校验的虚拟仓买入事件（paper_trades Filled buy）。
#[derive(Debug, Clone)]
pub struct SignalEntry {
    pub id: i64,
    pub plan_id: String,
    pub code: String,
    pub name: String,
    pub fill_price: f64,
    pub family: SignalFamily,
    pub ts_utc: NaiveDateTime,
}

/// forward return 窗口 (根 15min bar)
pub const WINDOWS_BARS: [usize; 2] = [4, 16];
/// boll_macd 买入动作
const BUY_ACTIONS: [BollMacdAction; 2] = [BollMacdAction::BottomBuy, BollMacdAction::UptrendStart];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventObservation {
    pub terminal_ret: f64,
    pub mfe: f64,
    pub mae: f64,
}

// ============================================================
// 纯计算 (可测, 不依赖网络/DB)
// ============================================================

fn security_bar_time(bar: &SecurityBar, index: usize) -> Result<NaiveDateTime, String> {
    let year = i32::try_from(bar.year)
        .map_err(|_| format!("technical bar index={index} year invalid: {}", bar.year))?;
    NaiveDate::from_ymd_opt(year, bar.month, bar.day)
        .and_then(|date| date.and_hms_opt(bar.hour, bar.minute, 0))
        .ok_or_else(|| {
            format!(
                "technical bar index={index} timestamp invalid: {:04}-{:02}-{:02} {:02}:{:02}",
                bar.year, bar.month, bar.day, bar.hour, bar.minute
            )
        })
}

fn start_stamped_slot(time: NaiveTime) -> Option<u8> {
    let minute = time.hour().checked_mul(60)?.checked_add(time.minute())?;
    let (start, base_slot) = if (570..=675).contains(&minute) {
        (570, 0_u32)
    } else if (780..=885).contains(&minute) {
        (780, 8_u32)
    } else {
        return None;
    };
    let offset = minute.checked_sub(start)?;
    if offset % 15 != 0 {
        return None;
    }
    u8::try_from(base_slot.checked_add(offset / 15)?).ok()
}

fn end_stamped_slot(time: NaiveTime) -> Option<u8> {
    let minute = time.hour().checked_mul(60)?.checked_add(time.minute())?;
    let (start, base_slot) = if (585..=690).contains(&minute) {
        (585, 0_u32)
    } else if (795..=900).contains(&minute) {
        (795, 8_u32)
    } else {
        return None;
    };
    let offset = minute.checked_sub(start)?;
    if offset % 15 != 0 {
        return None;
    }
    u8::try_from(base_slot.checked_add(offset / 15)?).ok()
}

fn sequence_is_continuous_for(
    times: &[NaiveDateTime],
    slot_of: fn(NaiveTime) -> Option<u8>,
) -> Result<bool, String> {
    let slots = times
        .iter()
        .map(|time| slot_of(time.time()))
        .collect::<Option<Vec<_>>>();
    let Some(slots) = slots else {
        return Ok(false);
    };
    for (time_pair, slot_pair) in times.windows(2).zip(slots.windows(2)) {
        let previous_time = time_pair[0];
        let next_time = time_pair[1];
        let previous_slot = slot_pair[0];
        let next_slot = slot_pair[1];
        if previous_time.date() == next_time.date() {
            if previous_slot.checked_add(1) != Some(next_slot) {
                return Ok(false);
            }
            continue;
        }
        if previous_slot != 15 || next_slot != 0 {
            return Ok(false);
        }
        let expected_previous =
            crate::calendar::verified_prev_a_share_trading_day(next_time.date())?;
        if expected_previous != previous_time.date() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// BR-247：生产入口消费前的结构门。来源/新鲜度由未来发布的 TechnicalBars
/// 能力负责；这里仍拒绝空批次、坏 OHLC、负量额、重复或逆序时间。
pub fn validate_technical_bars(bars: &[SecurityBar]) -> Result<(), String> {
    if bars.is_empty() {
        return Err("technical bars are empty".to_owned());
    }
    let mut previous = None;
    let mut times = Vec::with_capacity(bars.len());
    for (index, bar) in bars.iter().enumerate() {
        let time = security_bar_time(bar, index)?;
        if previous.is_some_and(|old| old >= time) {
            return Err(format!(
                "technical bars not strictly ordered at index={index}: {time}"
            ));
        }
        previous = Some(time);
        if !crate::calendar::verified_a_share_trading_day(time.date()).map_err(|error| {
            format!("technical bar index={index} trading calendar unavailable: {error}")
        })? {
            return Err(format!(
                "technical bar index={index} falls on a non-trading date: {}",
                time.date()
            ));
        }
        times.push(time);
        let prices = [bar.open, bar.high, bar.low, bar.close];
        if prices
            .iter()
            .any(|price| !price.is_finite() || *price <= 0.0)
        {
            return Err(format!(
                "technical bar index={index} contains non-positive/non-finite OHLC"
            ));
        }
        if bar.high < bar.open.max(bar.close).max(bar.low)
            || bar.low > bar.open.min(bar.close).min(bar.high)
        {
            return Err(format!("technical bar index={index} OHLC is inconsistent"));
        }
        if !bar.vol.is_finite() || bar.vol < 0.0 || !bar.amount.is_finite() || bar.amount < 0.0 {
            return Err(format!(
                "technical bar index={index} volume/amount is invalid"
            ));
        }
    }
    let start_stamped = sequence_is_continuous_for(&times, start_stamped_slot)?;
    let end_stamped = sequence_is_continuous_for(&times, end_stamped_slot)?;
    if !start_stamped && !end_stamped {
        return Err("technical bars are not continuous on a stable 15-minute grid".to_owned());
    }
    Ok(())
}

/// 在升序 15min bars 里定位与信号北京时间完全相等的来源 bar 边界。
/// raw TechnicalBars 尚未发布区间起止语义，因此任意非边界分钟、午休、
/// 盘前或盘后时间保持 `Ok(None)`，禁止映射到此前 bar。
pub fn locate_signal_bar(
    bars: &[SecurityBar],
    signal_utc: NaiveDateTime,
) -> Result<Option<usize>, String> {
    validate_technical_bars(bars)?;
    let bj = signal_utc
        .checked_add_signed(Duration::hours(8))
        .ok_or_else(|| format!("signal timestamp overflow after UTC+8 conversion: {signal_utc}"))?;
    for (index, bar) in bars.iter().enumerate() {
        let bar_time = security_bar_time(bar, index)?;
        if bar_time == bj {
            return Ok(Some(index));
        }
        if bar_time > bj {
            break;
        }
    }
    Ok(None)
}

/// 计算 forward return: (bars[idx+n].close - base_price) / base_price。
/// idx+n 越界或 base_price <= 0 返回 None。
pub fn forward_return(bars: &[SecurityBar], idx: usize, n: usize, base_price: f64) -> Option<f64> {
    forward_observation(bars, idx, n, base_price).map(|observation| observation.terminal_ret)
}

/// 从信号后的完整路径计算终点收益及逐事件 MFE/MAE。终点使用最后一根
/// `close`，MFE/MAE 分别消费每根真实 `high/low`；二者以 0 为初始界。
pub fn forward_observation(
    bars: &[SecurityBar],
    idx: usize,
    n: usize,
    base_price: f64,
) -> Option<EventObservation> {
    if !base_price.is_finite() || base_price <= 0.0 || n == 0 {
        return None;
    }
    let end = idx.checked_add(n)?;
    let path = bars.get(idx.checked_add(1)?..=end)?;
    let mut mfe = 0.0_f64;
    let mut mae = 0.0_f64;
    for bar in path {
        if [bar.close, bar.high, bar.low]
            .iter()
            .any(|price| !price.is_finite() || *price <= 0.0)
        {
            return None;
        }
        let favorable_ret = (bar.high - base_price) / base_price;
        let adverse_ret = (bar.low - base_price) / base_price;
        mfe = mfe.max(favorable_ret);
        mae = mae.min(adverse_ret);
    }
    let terminal_ret = (path.last()?.close - base_price) / base_price;
    Some(EventObservation {
        terminal_ret,
        mfe,
        mae,
    })
}

/// 聚合逐事件路径；上涨比例不是策略胜率，MFE/MAE 是逐事件路径值的均值。
pub fn aggregate_group(
    reason: &str,
    window_bars: usize,
    observations: &[EventObservation],
) -> SignalGroup {
    let count = observations.len();
    let up_rate = if count > 0 {
        Some(
            observations
                .iter()
                .filter(|observation| observation.terminal_ret > 0.0)
                .count() as f64
                / count as f64,
        )
    } else {
        None
    };
    let average = |project: fn(&EventObservation) -> f64| {
        (count > 0).then(|| observations.iter().map(project).sum::<f64>() / count as f64)
    };
    let avg_terminal_ret = average(|observation| observation.terminal_ret);
    let avg_mfe = average(|observation| observation.mfe);
    let avg_mae = average(|observation| observation.mae);
    SignalGroup {
        reason: reason.to_string(),
        window_bars,
        count,
        up_rate,
        avg_terminal_ret,
        avg_mfe,
        avg_mae,
    }
}

/// 在已验证的升序 SecurityBar 上逐个历史边界运行 boll_macd。
/// 返回的 index 与原始 bars 一致，未来路径统一复用 `forward_observation`。
pub fn scan_boll_macd_buys(bars: &[SecurityBar]) -> Result<Vec<(usize, BollMacdAction)>, String> {
    validate_technical_bars(bars)?;
    let observations = bars
        .iter()
        .map(|bar| BollMacdObservation {
            close: bar.close,
            volume: bar.vol,
        })
        .collect::<Vec<_>>();
    let mut signals = Vec::new();
    if observations.len() < 35 {
        return Ok(signals);
    }
    for index in (34..observations.len()).rev() {
        let signal = detect_boll_macd_observations(&observations[..=index])?;
        if BUY_ACTIONS.contains(&signal.action) {
            signals.push((index, signal.action));
        }
    }
    Ok(signals)
}

// ============================================================
// 网络/DB 薄壳 (spawn_blocking 内调用)
// ============================================================

#[derive(diesel::QueryableByName, Debug, Clone)]
struct PaperFilledReviewRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    plan_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    fill_price: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    occurred_at: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    virtual_reason: String,
}

fn parse_paper_signal_rows(
    rows: Vec<PaperFilledReviewRow>,
    as_of_date: NaiveDate,
    days: usize,
) -> Result<(Vec<SignalEntry>, usize), String> {
    if days == 0 {
        return Err("R-12 review window days must be positive".to_owned());
    }
    let lookback =
        i64::try_from(days - 1).map_err(|_| format!("R-12 review window too large: {days}"))?;
    let start_date = as_of_date
        .checked_sub_signed(Duration::days(lookback))
        .ok_or_else(|| format!("R-12 review window underflow: as_of={as_of_date} days={days}"))?;
    let mut previous_order = None;
    let mut identities = std::collections::HashSet::new();
    let mut entries = Vec::new();
    let mut exit_rows_excluded = 0_usize;

    for row in rows {
        if row.id <= 0 || !identities.insert(row.id) {
            return Err(format!(
                "R-12 paper fill identity invalid/duplicate: id={}",
                row.id
            ));
        }
        if row.plan_id.trim().is_empty() || row.code.trim().is_empty() || row.name.trim().is_empty()
        {
            return Err(format!(
                "R-12 paper fill id={} plan/code/name is blank",
                row.id
            ));
        }
        let ts_utc = parse_paper_fill_timestamp(row.id, &row.occurred_at)?;
        if previous_order.is_some_and(|previous| previous > (ts_utc, row.id)) {
            return Err(format!("R-12 paper fills are not ordered at id={}", row.id));
        }
        previous_order = Some((ts_utc, row.id));
        let fill_price = row
            .fill_price
            .filter(|price| price.is_finite() && *price > 0.0)
            .ok_or_else(|| format!("R-12 paper fill id={} fill_price missing/invalid", row.id))?;
        u32::try_from(row.quantity)
            .ok()
            .filter(|quantity| *quantity > 0 && quantity.is_multiple_of(100))
            .ok_or_else(|| {
                format!(
                    "R-12 paper fill id={} quantity invalid: {}",
                    row.id, row.quantity
                )
            })?;
        let local_date = ts_utc
            .checked_add_signed(Duration::hours(8))
            .ok_or_else(|| format!("R-12 paper fill id={} local time overflow", row.id))?
            .date();
        let direction = row.direction.trim();
        if direction != "buy" && direction != "sell" {
            return Err(format!(
                "R-12 paper fill id={} direction invalid: {:?}",
                row.id, row.direction
            ));
        }
        if local_date < start_date || local_date > as_of_date {
            continue;
        }
        if direction == "sell" {
            exit_rows_excluded += 1;
            continue;
        }
        let family = signal_family_of(&row.virtual_reason);
        if family == SignalFamily::Unknown || family == SignalFamily::ExitByRule {
            return Err(format!(
                "R-12 paper buy id={} entry strategy family unavailable: {:?}",
                row.id, row.virtual_reason
            ));
        }
        entries.push(SignalEntry {
            id: row.id,
            plan_id: row.plan_id,
            code: row.code,
            name: row.name,
            fill_price,
            family,
            ts_utc,
        });
    }
    Ok((entries, exit_rows_excluded))
}

/// 读取并严格校验纸面成交，再按显式评估日投影买入事件。卖出只计数，不评分。
pub fn read_paper_signal_entries(
    as_of_date: NaiveDate,
    days: usize,
) -> Result<(Vec<SignalEntry>, usize), String> {
    use crate::database::DatabaseManager;
    use diesel::query_dsl::RunQueryDsl;

    let db = DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| format!("db conn: {e}"))?;
    let rows = diesel::sql_query(
        "SELECT id, plan_id, code, name, direction, fill_price, quantity, \
                CAST(ts AS TEXT) AS occurred_at, virtual_reason \
         FROM paper_trades \
         WHERE status = 'Filled' \
         ORDER BY ts ASC, id ASC",
    )
    .load::<PaperFilledReviewRow>(&mut conn)
    .map_err(|e| format!("paper_trades read: {e}"))?;
    parse_paper_signal_rows(rows, as_of_date, days)
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

pub fn backtest_virtual_signals(
    as_of_date: NaiveDate,
    days: usize,
) -> Result<R12BacktestResult, String> {
    let (entries, exit_rows_excluded) = read_paper_signal_entries(as_of_date, days)?;
    let gateway = HistoricalBarsGateway::new();
    let mut cache = TechnicalBarsCache::new();
    let mut loader = |code: &str| {
        gateway
            .fifteen_min_bars(code, 800)
            .map_err(|error| error.to_string())
    };
    backtest_virtual_signals_with_entries_and_cache(
        &entries,
        exit_rows_excluded,
        &mut cache,
        &mut loader,
    )
}

fn backtest_virtual_signals_with_entries_and_cache<Loader>(
    entries: &[SignalEntry],
    exit_rows_excluded: usize,
    bars_by_code: &mut TechnicalBarsCache,
    loader: &mut Loader,
) -> Result<R12BacktestResult, String>
where
    Loader: FnMut(&str) -> Result<Vec<SecurityBar>, String>,
{
    if entries.is_empty() {
        return Ok(R12BacktestResult {
            exit_rows_excluded,
            ..Default::default()
        });
    }

    let mut unaligned = 0_usize;
    let mut censored_windows = 0_usize;

    // 入场族 → 窗口 → 逐事件路径观察。
    let mut observations_by_family: std::collections::BTreeMap<String, Vec<Vec<EventObservation>>> =
        std::collections::BTreeMap::new();

    for entry in entries {
        // BR-239: success and failure are both cached for the whole run. A
        // failed code must not be reacquired once per duplicate signal row.
        let bars = load_technical_bars_cached(bars_by_code, &entry.code, |code| loader(code))
            .map_err(|error| {
                format!(
                    "R-12 15min bars unavailable for {} ({}): {error}",
                    entry.code, entry.name
                )
            })?;
        let Some(idx) = locate_signal_bar(bars, entry.ts_utc)
            .map_err(|error| format!("R-12 15min bars invalid for {}: {error}", entry.code))?
        else {
            unaligned += 1;
            continue;
        };
        let group = observations_by_family
            .entry(entry.family.as_str().to_owned())
            .or_insert_with(|| vec![Vec::new(), Vec::new()]);
        for (wi, window) in WINDOWS_BARS.iter().enumerate() {
            if let Some(observation) = forward_observation(bars, idx, *window, entry.fill_price) {
                group[wi].push(observation);
            } else {
                censored_windows += 1;
            }
        }
    }

    let mut result = R12BacktestResult {
        exit_rows_excluded,
        unaligned_signals: unaligned,
        censored_windows,
        ..Default::default()
    };
    for (reason, windows) in &observations_by_family {
        for (wi, window) in WINDOWS_BARS.iter().enumerate() {
            if windows[wi].is_empty() {
                continue;
            }
            result
                .virtual_buy
                .push(aggregate_group(reason, *window, &windows[wi]));
        }
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
        .map(|(groups, _censored_windows)| groups)
}

fn backtest_boll_macd_15min_with_cache<Loader>(
    codes: &[String],
    bars_by_code: &mut TechnicalBarsCache,
    loader: &mut Loader,
) -> Result<(Vec<SignalGroup>, usize), String>
where
    Loader: FnMut(&str) -> Result<Vec<SecurityBar>, String>,
{
    let mut observations_by_action: std::collections::BTreeMap<String, Vec<Vec<EventObservation>>> =
        std::collections::BTreeMap::new();
    let mut censored_windows = 0_usize;
    for code in codes {
        let bars = load_technical_bars_cached(bars_by_code, code, |code| loader(code))
            .map_err(|error| format!("R-12 boll_macd bars unavailable for {code}: {error}"))?;
        let signals = scan_boll_macd_buys(bars)
            .map_err(|error| format!("R-12 boll_macd bars invalid for {code}: {error}"))?;
        for (index, action) in signals {
            let label = format!("{action:?}");
            let group = observations_by_action
                .entry(label)
                .or_insert_with(|| vec![Vec::new(), Vec::new()]);
            for (wi, window) in WINDOWS_BARS.iter().enumerate() {
                if let Some(observation) =
                    forward_observation(bars, index, *window, bars[index].close)
                {
                    group[wi].push(observation);
                } else {
                    censored_windows += 1;
                }
            }
        }
    }
    let mut out = Vec::new();
    for (label, windows) in &observations_by_action {
        for (wi, window) in WINDOWS_BARS.iter().enumerate() {
            if windows[wi].is_empty() {
                continue;
            }
            out.push(aggregate_group(label, *window, &windows[wi]));
        }
    }
    Ok((out, censored_windows))
}

/// 渲染 R-12 事件研究；固定声明该结果不是完整策略胜率。
pub fn render_r12(result: &R12BacktestResult) -> String {
    let mut lines = vec![
        "📈 R-12 买入事件研究（15 分钟 K 线）".to_string(),
        "━━━━━━━━━━━━━━━━━━━━".to_string(),
        "ℹ️ 上涨比例仅描述入场后短期价格路径，不是买入→卖出扣成本策略胜率。".to_string(),
    ];

    lines.push("【虚拟仓买入事件】".to_string());
    if result.virtual_buy.is_empty() {
        lines.push("· 窗口内无可采信的 Filled 买入事件或全部未对齐".to_string());
    } else {
        lines.push("买入事件（按入场来源）:".to_string());
        for group in &result.virtual_buy {
            lines.push(format!(
                "· {}（{}根）样本{} 上涨比例{} 平均终点{} 平均MFE {} 平均MAE {}{}",
                group.reason,
                group.window_bars,
                group.count,
                pct(group.up_rate),
                signed_pct(group.avg_terminal_ret),
                signed_pct(group.avg_mfe),
                signed_pct(group.avg_mae),
                if group.is_under_sampled() {
                    "（样本不足 200，不形成策略结论）"
                } else {
                    ""
                }
            ));
        }
    }
    if result.exit_rows_excluded > 0 {
        lines.push(format!(
            "ℹ️ 同窗口卖出事实 {} 笔，仅作为退出记录披露，未进入入场上涨比例",
            result.exit_rows_excluded
        ));
    }
    if result.unaligned_signals > 0 {
        lines.push(format!(
            "⚠️ 无 15 分钟 K 线对齐的买入事件 {} 笔（覆盖外/集合竞价/收盘后）",
            result.unaligned_signals
        ));
    }
    if result.censored_windows > 0 {
        lines.push(format!(
            "ℹ️ 未来窗口不完整 {} 个，按右删失保留且不进入分母",
            result.censored_windows
        ));
    }

    lines.push(String::new());
    lines.push("【boll_macd 15 分钟买入事件】".to_string());
    if result.boll_macd.is_empty() {
        lines.push("· 无买入信号 (BottomBuy/UptrendStart)".to_string());
    } else {
        for group in &result.boll_macd {
            lines.push(format!(
                "· {}（{}根）信号{} 上涨比例{} 平均终点{} 平均MFE {} 平均MAE {}{}",
                group.reason,
                group.window_bars,
                group.count,
                pct(group.up_rate),
                signed_pct(group.avg_terminal_ret),
                signed_pct(group.avg_mfe),
                signed_pct(group.avg_mae),
                if group.is_under_sampled() {
                    "（样本不足 200，不形成策略结论）"
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

fn signed_pct(rate: Option<f64>) -> String {
    match rate {
        Some(value) => format!("{:+.2}%", value * 100.0),
        None => "-".to_owned(),
    }
}

/// 供 dispatcher 用的组合入口：调用方必须传入同一复盘业务日。
pub fn run_full_backtest(as_of_date: NaiveDate, days: usize) -> Result<R12BacktestResult, String> {
    let (entries, exit_rows_excluded) = read_paper_signal_entries(as_of_date, days)?;
    let gateway = HistoricalBarsGateway::new();
    run_full_backtest_with_entries_and_loader(&entries, exit_rows_excluded, |code| {
        gateway
            .fifteen_min_bars(code, 800)
            .map_err(|error| error.to_string())
    })
}

fn run_full_backtest_with_entries_and_loader<Loader>(
    entries: &[SignalEntry],
    exit_rows_excluded: usize,
    mut loader: Loader,
) -> Result<R12BacktestResult, String>
where
    Loader: FnMut(&str) -> Result<Vec<SecurityBar>, String>,
{
    let mut cache = TechnicalBarsCache::new();
    let mut result = backtest_virtual_signals_with_entries_and_cache(
        entries,
        exit_rows_excluded,
        &mut cache,
        &mut loader,
    )?;
    let mut seen = std::collections::BTreeSet::new();
    for e in entries {
        seen.insert(e.code.clone());
    }
    let codes: Vec<String> = seen.into_iter().collect();
    if codes.is_empty() {
        return Ok(result);
    }
    let (boll_macd, boll_censored_windows) =
        backtest_boll_macd_15min_with_cache(&codes, &mut cache, &mut loader)?;
    result.boll_macd = boll_macd;
    result.censored_windows = result
        .censored_windows
        .checked_add(boll_censored_windows)
        .ok_or_else(|| "R-12 censored window count overflow".to_owned())?;
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
            id: 1,
            plan_id: "TEST_CODE_PLAN_1".to_owned(),
            code: "TEST_CODE_000001".to_owned(),
            name: "TEST_CODE sample".to_owned(),
            fill_price: 10.0,
            family: SignalFamily::NewsCatalyst,
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
        // 起点标记制：2026-08-10 全天 16 根 15min。
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
            (13, 0),
            (13, 15),
            (13, 30),
            (13, 45),
            (14, 0),
            (14, 15),
            (14, 30),
            (14, 45),
        ] {
            bars.push(bar(d, (h, m), 10.0));
        }
        bars
    }

    fn end_stamped_bars() -> Vec<SecurityBar> {
        let d = (2026, 8, 10);
        [
            (9, 45),
            (10, 0),
            (10, 15),
            (10, 30),
            (10, 45),
            (11, 0),
            (11, 15),
            (11, 30),
            (13, 15),
            (13, 30),
            (13, 45),
            (14, 0),
            (14, 15),
            (14, 30),
            (14, 45),
            (15, 0),
        ]
        .into_iter()
        .map(|hhmm| bar(d, hhmm, 10.0))
        .collect()
    }

    #[test]
    fn br247_complete_end_stamped_grid_is_admitted() {
        validate_technical_bars(&end_stamped_bars()).expect("complete end-stamped grid");
    }

    #[test]
    fn locate_signal_bar_finds_exact_source_boundary() {
        let bars = sample_bars();
        // 北京时间 9:25 早于第一根已完成 K 线，不能偷配到 idx 0。
        let utc = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(1, 25, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc).unwrap(), None);
        // 北京时间 10:15 → 与来源 bar 边界精确相等 (idx 3)
        let utc = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(2, 15, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc).unwrap(), Some(3));
        // 北京时间 9:30 → idx 0
        let utc = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc).unwrap(), Some(0));
        // 北京时间 14:15 → idx 13
        let utc = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(6, 15, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc).unwrap(), Some(13));
        // 北京时间 15:05 (盘后, 无 bar 承载) → None — 收盘后信号对齐失败
        let utc = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(7, 5, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc).unwrap(), None);
        // 数据早于信号 → None
        let utc = NaiveDate::from_ymd_opt(2026, 8, 11)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, utc).unwrap(), None);
    }

    #[test]
    fn br247_only_exact_source_bar_boundaries_are_aligned() {
        let bars = sample_bars();
        let exact_boundary = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(2, 15, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, exact_boundary).unwrap(), Some(3));

        let inside_unpublished_interval = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(2, 23, 0)
            .unwrap();
        assert_eq!(
            locate_signal_bar(&bars, inside_unpublished_interval).unwrap(),
            None,
            "raw TechnicalBars has no source-backed interval semantics"
        );

        let lunch_break = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(3, 45, 0)
            .unwrap();
        assert_eq!(locate_signal_bar(&bars, lunch_break).unwrap(), None);
    }

    #[test]
    fn forward_return_uses_close_after_n_bars() {
        let mut bars = sample_bars();
        bars[5] = bar((2026, 8, 10), (10, 45), 11.0);
        bars[9] = bar((2026, 8, 10), (13, 15), 12.5);
        // idx 5, n=4 → bars[9].close = 12.5, base 11.0 → +13.64%
        let ret = forward_return(&bars, 5, 4, 11.0).expect("ret");
        assert!((ret - 0.13636).abs() < 0.001);
        let observation = forward_observation(&bars, 5, 4, 11.0).expect("path observation");
        assert!((observation.mfe - 0.13636).abs() < 0.001);
        assert!((observation.mae + 0.09090).abs() < 0.001);
        // 越界
        assert_eq!(forward_return(&bars, 15, 4, 10.0), None);
        // base <= 0
        assert_eq!(forward_return(&bars, 5, 1, 0.0), None);
    }

    #[test]
    fn br247_path_mfe_uses_high_and_mae_uses_low() {
        let mut bars = sample_bars();
        bars[6].close = 10.0;
        bars[6].high = 12.0;
        bars[6].low = 8.0;
        let observation = forward_observation(&bars, 5, 1, 10.0).expect("complete path");
        assert_eq!(observation.terminal_ret, 0.0);
        assert!((observation.mfe - 0.2).abs() < 1e-9);
        assert!((observation.mae + 0.2).abs() < 1e-9);
    }

    #[test]
    fn aggregate_group_computes_stats() {
        let observations = vec![
            EventObservation {
                terminal_ret: 0.05,
                mfe: 0.08,
                mae: -0.01,
            },
            EventObservation {
                terminal_ret: -0.02,
                mfe: 0.01,
                mae: -0.03,
            },
            EventObservation {
                terminal_ret: 0.03,
                mfe: 0.04,
                mae: 0.0,
            },
            EventObservation {
                terminal_ret: 0.01,
                mfe: 0.02,
                mae: 0.0,
            },
            EventObservation {
                terminal_ret: -0.01,
                mfe: 0.0,
                mae: -0.02,
            },
        ];
        let g = aggregate_group("NewsCatalyst", 4, &observations);
        assert_eq!(g.count, 5);
        assert_eq!(g.up_rate, Some(0.6));
        assert!((g.avg_terminal_ret.unwrap() - 0.012).abs() < 1e-9);
        assert!((g.avg_mfe.unwrap() - 0.03).abs() < 1e-9);
        assert!((g.avg_mae.unwrap() + 0.012).abs() < 1e-9);
        assert!(g.is_under_sampled());
        let positive = EventObservation {
            terminal_ret: 0.1,
            mfe: 0.1,
            mae: 0.0,
        };
        let g199 = aggregate_group("Breakout", 16, &[positive; 199]);
        assert!(g199.is_under_sampled());
        let g2 = aggregate_group("Breakout", 16, &[positive; 200]);
        assert!(!g2.is_under_sampled());
        let empty = aggregate_group("x", 4, &[]);
        assert_eq!(empty.count, 0);
        assert_eq!(empty.up_rate, None);
        assert_eq!(empty.avg_mfe, None);
    }

    #[test]
    fn scan_boll_macd_buys_uses_raw_bars_and_shared_forward_path() {
        // 合成 80 根序列: 缓涨 50 根 (MACD 转正) 后急跌 30 根 (触下轨 +
        // MACD 绿柱缩短) → BottomBuy (实测 variant=1 触发 7 个信号)。
        let slots = [
            (9, 30),
            (9, 45),
            (10, 0),
            (10, 15),
            (10, 30),
            (10, 45),
            (11, 0),
            (11, 15),
            (13, 0),
            (13, 15),
            (13, 30),
            (13, 45),
            (14, 0),
            (14, 15),
            (14, 30),
            (14, 45),
        ];
        let mut bars = Vec::new();
        for date in [
            (2026, 8, 3),
            (2026, 8, 4),
            (2026, 8, 5),
            (2026, 8, 6),
            (2026, 8, 7),
        ] {
            for slot in slots {
                let index = bars.len();
                let close = if index < 50 {
                    10.0 + (index as f64) * 0.04
                } else {
                    12.0 - (index as f64 - 50.0) * 0.10
                };
                bars.push(bar(date, slot, close));
            }
        }
        let signals = scan_boll_macd_buys(&bars).expect("valid raw technical bars");
        assert!(!signals.is_empty(), "expected at least one buy signal");
        let (index, _) = signals[0];
        let r4 = forward_return(&bars, index, 4, bars[index].close);
        let r16 = forward_return(&bars, index, 16, bars[index].close);
        assert!(r4.is_some() || r16.is_some());
    }

    fn raw_fill(
        id: i64,
        direction: &str,
        fill_price: Option<f64>,
        occurred_at: &str,
        virtual_reason: &str,
    ) -> PaperFilledReviewRow {
        PaperFilledReviewRow {
            id,
            plan_id: format!("TEST_CODE_PLAN_{id}"),
            code: "TEST_CODE_000001".to_owned(),
            name: "TEST_CODE sample".to_owned(),
            direction: direction.to_owned(),
            fill_price,
            quantity: 100,
            occurred_at: occurred_at.to_owned(),
            virtual_reason: virtual_reason.to_owned(),
        }
    }

    #[test]
    fn br247_sell_rows_are_not_scored_as_entry_events() {
        let as_of = NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
        let (entries, exits) = parse_paper_signal_rows(
            vec![raw_fill(
                1,
                "sell",
                Some(10.0),
                "2026-08-10 01:30:00",
                "BR-234四大铁律卖出:ATR动态止损",
            )],
            as_of,
            1,
        )
        .expect("sell row is a valid exit fact");
        assert!(entries.is_empty());
        assert_eq!(exits, 1);
        let calls = std::cell::Cell::new(0_usize);

        let result = run_full_backtest_with_entries_and_loader(&entries, exits, |_| {
            calls.set(calls.get() + 1);
            Ok(sample_bars())
        })
        .expect("sell row is a valid exit fact but not an entry event");

        assert!(result.virtual_buy.is_empty());
        assert_eq!(result.exit_rows_excluded, 1);
        assert_eq!(
            calls.get(),
            0,
            "exit-only input must make zero provider calls"
        );
    }

    #[test]
    fn br247_does_not_delete_positive_fill_by_fixed_date_or_one_yuan_threshold() {
        let as_of = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        let (entries, exits) = parse_paper_signal_rows(
            vec![raw_fill(
                1,
                "buy",
                Some(0.5),
                "2026-07-15 02:00:00",
                "Momentum",
            )],
            as_of,
            1,
        )
        .expect("positive source fill must not be rejected by a fixed price/date rule");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].fill_price, 0.5);
        assert_eq!(exits, 0);
    }

    #[test]
    fn br247_invalid_fill_or_unknown_entry_family_fails_the_batch() {
        let as_of = NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
        let missing = raw_fill(1, "buy", None, "2026-08-10 01:30:00", "Momentum");
        assert!(parse_paper_signal_rows(vec![missing], as_of, 1)
            .unwrap_err()
            .contains("fill_price missing/invalid"));

        let bad_time = raw_fill(2, "buy", Some(10.0), "now", "Momentum");
        assert!(parse_paper_signal_rows(vec![bad_time], as_of, 1)
            .unwrap_err()
            .contains("timestamp invalid"));

        let unknown = raw_fill(3, "buy", Some(10.0), "2026-08-10 01:30:00", "mystery");
        assert!(parse_paper_signal_rows(vec![unknown], as_of, 1)
            .unwrap_err()
            .contains("entry strategy family unavailable"));
    }

    #[test]
    fn br247_invalid_or_unordered_technical_bars_fail_closed() {
        let signal_utc = NaiveDate::from_ymd_opt(2026, 8, 10)
            .unwrap()
            .and_hms_opt(2, 23, 0)
            .unwrap();
        let mut invalid = sample_bars();
        invalid[0].close = 0.0;
        assert!(validate_technical_bars(&invalid)
            .unwrap_err()
            .contains("OHLC"));
        assert!(locate_signal_bar(&invalid, signal_utc)
            .unwrap_err()
            .contains("OHLC"));

        let mut unordered = sample_bars();
        unordered.swap(0, 1);
        assert!(validate_technical_bars(&unordered)
            .unwrap_err()
            .contains("not strictly ordered"));
    }

    #[test]
    fn br247_internal_technical_bar_gap_fails_closed() {
        let mut bars = sample_bars();
        bars.remove(4);
        let error = validate_technical_bars(&bars).unwrap_err();
        assert!(error.contains("not continuous"), "{error}");
    }

    #[test]
    fn br247_cross_trading_day_gap_fails_closed() {
        let bars = vec![
            bar((2026, 8, 3), (14, 45), 10.0),
            bar((2026, 8, 5), (9, 30), 10.0),
        ];
        let error = validate_technical_bars(&bars).unwrap_err();
        assert!(error.contains("not continuous"), "{error}");
    }

    #[test]
    fn br247_mixed_timestamp_conventions_fail_closed() {
        let mut bars = sample_bars();
        bars.remove(8);
        bars.push(bar((2026, 8, 10), (15, 0), 10.0));
        let error = validate_technical_bars(&bars).unwrap_err();
        assert!(error.contains("not continuous"), "{error}");
    }

    #[test]
    fn br247_render_discloses_event_semantics_and_censoring() {
        let result = R12BacktestResult {
            virtual_buy: vec![aggregate_group(
                "Momentum",
                4,
                &[EventObservation {
                    terminal_ret: 0.01,
                    mfe: 0.02,
                    mae: -0.01,
                }],
            )],
            exit_rows_excluded: 2,
            censored_windows: 1,
            ..Default::default()
        };
        let rendered = render_r12(&result);
        assert!(rendered.contains("上涨比例"));
        assert!(rendered.contains("不是买入→卖出扣成本策略胜率"));
        assert!(rendered.contains("样本不足 200"));
        assert!(rendered.contains("右删失"));
        assert!(!rendered.contains("卖出信号"));
    }
}
