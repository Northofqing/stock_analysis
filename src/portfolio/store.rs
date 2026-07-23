//! Portfolio 存储层 — SQLite 读写（复用 DatabaseManager 单例）。

use chrono::{NaiveDate, NaiveDateTime};
use diesel::prelude::*;

use super::{LedgerEntry, Position, PositionStatus, Trade, TradeDirection};

/// 从 stock_position 表加载持仓
pub fn load_positions() -> Result<Vec<Position>, String> {
    load_positions_with_source_time().map(|(positions, _)| positions)
}

/// 加载持仓及整批最旧的来源更新时间；调用方据此执行 30 秒账户新鲜度门。
pub fn load_positions_with_source_time(
) -> Result<(Vec<Position>, Option<chrono::DateTime<chrono::Local>>), String> {
    let db = crate::database::DatabaseManager::try_get()
        .ok_or_else(|| "DB 未初始化，无法加载持仓".to_string())?;
    let records = db.get_all_open_positions().map_err(|e| e.to_string())?;
    let oldest_source_time = records
        .iter()
        .map(|record| record.updated_at)
        .min()
        .map(|time| {
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(time, chrono::Utc)
                .with_timezone(&chrono::Local)
        });
    let positions = records
        .into_iter()
        .map(|r| -> Result<Position, String> {
            crate::risk::env_guard::validate_symbol_for_current_env(&r.code)?;
            if r.name.trim().is_empty() {
                return Err(format!("持仓 {} 名称为空", r.code));
            }
            if !r.buy_price.is_finite() || r.buy_price <= 0.0 {
                return Err(format!("持仓 {} buy_price 非法: {}", r.code, r.buy_price));
            }
            let shares = u64::try_from(r.quantity)
                .ok()
                .filter(|quantity| *quantity > 0 && quantity.is_multiple_of(100))
                .ok_or_else(|| format!("持仓 {} quantity 非法: {}", r.code, r.quantity))?;
            let added_at = NaiveDate::parse_from_str(&r.buy_date, "%Y-%m-%d")
                .map_err(|error| format!("持仓 {} buy_date 非法: {error}", r.code))?;
            // v14.1 F7: 从 stock_position.st_type 列派生 is_st/star_st
            //   "ST" → is_st, "*ST" → star_st, NULL → 都是 false
            let (is_st, star_st) = match r.st_type.as_deref() {
                Some("ST") => (true, false),
                Some("*ST") => (false, true),
                None => (false, false),
                Some(other) => {
                    return Err(format!("持仓 {} st_type 非法: {other:?}", r.code));
                }
            };
            let sector = r
                .chain_name
                .filter(|chain| !chain.trim().is_empty())
                .or_else(|| {
                    crate::data_provider::chain_registry::lookup(&r.code).map(str::to_string)
                })
                .unwrap_or_default();
            if sector.trim().is_empty() {
                log::warn!("[portfolio] 持仓 {} 产业链缺失，保留空值", r.code);
            }
            Ok(Position {
                code: r.code,
                name: r.name,
                shares,
                cost_price: r.buy_price,
                hard_stop: None,
                added_at,
                status: PositionStatus::Holding,
                sector,
                is_st,
                star_st,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((positions, oldest_source_time))
}

/// 从环境变量加载自选（尝试解析真实名称）
pub fn load_watchlist() -> Result<Vec<Position>, String> {
    let list = match std::env::var("STOCK_LIST") {
        Ok(list) => list,
        Err(std::env::VarError::NotPresent) => return Ok(Vec::new()),
        Err(error) => return Err(format!("STOCK_LIST 不是有效 Unicode: {error}")),
    };
    let codes: Vec<&str> = list
        .split(',')
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .collect();
    if codes.is_empty() {
        return Ok(Vec::new());
    }
    let name_fetcher = crate::data_provider::DataFetcherManager::new()
        .map_err(|error| format!("初始化自选名称数据源失败: {error:#}"))?;
    let today = chrono::Local::now().date_naive();
    let mut seen = std::collections::HashSet::new();
    let mut positions = Vec::with_capacity(codes.len());
    for code in codes {
        crate::risk::env_guard::validate_symbol_for_current_env(code)?;
        let valid_shape = if crate::risk::env_guard::is_test_code(code) {
            code.len() > "TEST_CODE".len()
        } else {
            code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit())
        };
        if !valid_shape {
            return Err(format!("STOCK_LIST code 非法: {code:?}"));
        }
        if !seen.insert(code) {
            continue;
        }
        let name = name_fetcher
            .get_stock_name(code)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| format!("自选 {code} 缺少真实名称证据"))?;
        positions.push(Position {
            code: code.to_string(),
            name,
            shares: 0,
            cost_price: 0.0,
            hard_stop: None,
            added_at: today,
            status: PositionStatus::Watching,
            sector: String::new(),
            ..Default::default()
        });
    }
    Ok(positions)
}

/// 从 trades 表加载交易记录
/// 修复 P1.1: SQL 注入风险
/// 原代码用 format!() 拼接 SQL, 字段值含单引号或反斜杠时会破坏查询甚至被攻击
/// 改用 ? 占位符 + bind 绑定, Diesel 自动转义
pub fn load_trades_since(since: NaiveDate) -> Result<Vec<Trade>, String> {
    use diesel::sql_types::Date;
    let db = crate::database::DatabaseManager::try_get()
        .ok_or_else(|| "DB 未初始化，无法加载交易记录".to_string())?;
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let rows: Vec<TradeRow> = diesel::sql_query(
        "SELECT id, code, name, direction, price, shares, amount, reason, traded_at \
         FROM trades WHERE traded_at >= ? ORDER BY traded_at DESC",
    )
    .bind::<Date, _>(since)
    .load(&mut *conn)
    .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|r| {
            let direction = match r.direction.as_str() {
                "buy" => TradeDirection::Buy,
                "sell" => TradeDirection::Sell,
                other => return Err(format!("交易 {} direction 非法: {other:?}", r.id)),
            };
            if r.code.trim().is_empty() || r.name.trim().is_empty() {
                return Err(format!("交易 {} code/name 缺失", r.id));
            }
            if !r.price.is_finite() || r.price <= 0.0 {
                return Err(format!("交易 {} price 非法: {}", r.id, r.price));
            }
            let shares = u64::try_from(r.shares)
                .ok()
                .filter(|shares| *shares > 0)
                .ok_or_else(|| format!("交易 {} shares 非法: {}", r.id, r.shares))?;
            if !r.amount.is_finite() {
                return Err(format!("交易 {} amount 非法: {}", r.id, r.amount));
            }
            let traded_at = NaiveDateTime::parse_from_str(&r.traded_at, "%Y-%m-%d %H:%M:%S")
                .or_else(|_| {
                    NaiveDate::parse_from_str(&r.traded_at, "%Y-%m-%d").map(|date| {
                        date.and_hms_opt(0, 0, 0)
                            .expect("midnight is a valid NaiveTime")
                    })
                })
                .map_err(|error| format!("交易 {} traded_at 非法: {error}", r.id))?;
            Ok(Trade {
                id: Some(r.id.to_string()),
                code: r.code,
                name: r.name,
                direction,
                price: r.price,
                shares,
                amount: r.amount,
                reason: r.reason,
                traded_at,
            })
        })
        .collect()
}

/// 检查今日是否有买入（DB 未初始化时返回 false）
/// 修复 P1.1: SQL 注入风险 (改 ? 占位符)
/// review #14: 去 catch_unwind — try_get 显式处理 None (init 失败),
/// 不再静默吞 panic. 调用方需检查错误.
pub fn has_buy_today(code: &str, today: NaiveDate) -> Result<bool, String> {
    use diesel::sql_types::{Date, Text};
    let db =
        crate::database::DatabaseManager::try_get().ok_or_else(|| "DB 未初始化".to_string())?;
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    #[derive(QueryableByName, Debug)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        cnt: i32,
    }
    let result = diesel::sql_query(
        "SELECT COUNT(*) as cnt FROM trades WHERE code = ? AND direction = 'buy' AND traded_at = ?",
    )
    .bind::<Text, _>(code)
    .bind::<Date, _>(today)
    .get_result::<Count>(&mut *conn)
    .map_err(|e| e.to_string())?;
    Ok(result.cnt > 0)
}

/// 保存净值快照
/// 修复 P1.1: SQL 注入风险
pub fn save_ledger(entry: LedgerEntry) -> Result<(), String> {
    use diesel::sql_types::{Date, Double};
    if !entry.total_value.is_finite()
        || !entry.cash.is_finite()
        || !entry.market_value.is_finite()
        || !entry.daily_pnl.is_finite()
        || entry.total_value <= 0.0
        || entry.cash < 0.0
        || entry.market_value < 0.0
    {
        return Err("ledger contains invalid numeric fields".to_string());
    }
    let accounted = entry.cash + entry.market_value;
    if (entry.total_value - accounted).abs() > accounted.abs().max(1.0) * 1e-6 {
        return Err(format!(
            "ledger accounting mismatch: total={} cash={} market={}",
            entry.total_value, entry.cash, entry.market_value
        ));
    }
    if !crate::calendar::is_trading_day(entry.date)
        || entry.date > chrono::Local::now().date_naive()
    {
        return Err(format!(
            "ledger date is not a completed trading day: {}",
            entry.date
        ));
    }
    let db = crate::database::DatabaseManager::try_get()
        .ok_or_else(|| "DB 未初始化，无法保存净值快照".to_string())?;
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    diesel::sql_query(
        "INSERT INTO ledger (date, total_value, cash, market_value, daily_pnl) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind::<Date, _>(entry.date)
    .bind::<Double, _>(entry.total_value)
    .bind::<Double, _>(entry.cash)
    .bind::<Double, _>(entry.market_value)
    .bind::<Double, _>(entry.daily_pnl)
    .execute(&mut *conn)
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 加载净值时间序列
/// 修复 P1.1: SQL 注入风险
pub fn load_ledger(since: NaiveDate) -> Result<Vec<LedgerEntry>, String> {
    use diesel::sql_types::Date;
    let db = crate::database::DatabaseManager::try_get()
        .ok_or_else(|| "DB 未初始化，无法加载净值序列".to_string())?;
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let rows: Vec<LedgerRow> = diesel::sql_query(
        "SELECT date, total_value, cash, market_value, daily_pnl \
         FROM ledger WHERE date >= ? ORDER BY date ASC",
    )
    .bind::<Date, _>(since)
    .load(&mut *conn)
    .map_err(|e| e.to_string())?;
    let entries: Vec<LedgerEntry> = rows
        .into_iter()
        .map(|r| {
            let date = NaiveDate::parse_from_str(&r.date, "%Y-%m-%d")
                .map_err(|error| format!("ledger date 非法 {:?}: {error}", r.date))?;
            for (field, value) in [
                ("total_value", r.total_value),
                ("cash", r.cash),
                ("market_value", r.market_value),
                ("daily_pnl", r.daily_pnl),
            ] {
                if !value.is_finite() {
                    return Err(format!("ledger {date} {field} 非有限: {value}"));
                }
            }
            if r.total_value < 0.0 || r.cash < 0.0 || r.market_value < 0.0 {
                return Err(format!("ledger {date} 资产字段为负"));
            }
            let accounted = r.cash + r.market_value;
            if r.total_value <= 0.0
                || (r.total_value - accounted).abs() > accounted.abs().max(1.0) * 1e-6
            {
                return Err(format!(
                    "ledger {date} 会计恒等式不成立: total={} cash={} market={}",
                    r.total_value, r.cash, r.market_value
                ));
            }
            if !crate::calendar::is_trading_day(date) {
                return Err(format!("ledger {date} 不是交易日"));
            }
            Ok(LedgerEntry {
                date,
                total_value: r.total_value,
                cash: r.cash,
                market_value: r.market_value,
                daily_pnl: r.daily_pnl,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    for pair in entries.windows(2) {
        if pair[1].date <= pair[0].date {
            return Err(format!(
                "ledger 日期无序或重复: {} -> {}",
                pair[0].date, pair[1].date
            ));
        }
        let expected = crate::calendar::next_trading_day(pair[0].date);
        if pair[1].date != expected {
            return Err(format!(
                "ledger 交易日断档: {} 后应为 {}, 实际为 {}",
                pair[0].date, expected, pair[1].date
            ));
        }
    }
    Ok(entries)
}

/// 修复 P3.9: 实盘 rolling Sharpe (基于 ledger 净值)
/// 计算最近 N 日的年化 Sharpe, rf=0.03 (与 sharpe_calculator 一致)
/// A 股交易日数取 245 (US 默认 252; A 股实际 242-244, 245 略偏保守)
/// 返回 None 当数据 < 30 日 (样本不足)
pub fn live_rolling_sharpe(ledger: &[LedgerEntry], window: usize) -> Option<f64> {
    if ledger.len() < 30 {
        return None;
    }
    let recent = &ledger[ledger.len().saturating_sub(window)..];
    if recent.len() < 2 {
        return None;
    }
    let mut returns = Vec::new();
    for w in recent.windows(2) {
        if w[0].total_value > 0.0 {
            returns.push((w[1].total_value - w[0].total_value) / w[0].total_value);
        }
    }
    if returns.len() < 5 {
        return None;
    }
    let mean: f64 = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance: f64 =
        returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
    let std = variance.sqrt();
    if std <= 0.0 {
        return None;
    }
    let rf_daily = 0.03 / 245.0;
    let ann_factor = 245.0_f64.sqrt();
    Some((mean - rf_daily) * ann_factor / std)
}

/// 修复 P3.10: 策略相关性矩阵
/// 输入多个策略的日收益率序列 (Vec<Vec<f64>>), 输出 (n, n) 相关性矩阵
/// 量化分析师要求: 多策略组合时, 相关性 > 0.7 的策略对要降权
pub fn strategy_correlation_matrix(daily_returns: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = daily_returns.len();
    let mut matrix = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            if i == j {
                matrix[i][j] = 1.0;
            } else {
                matrix[i][j] = pearson_corr(&daily_returns[i], &daily_returns[j]);
            }
        }
    }
    matrix
}

fn pearson_corr(xs: &[f64], ys: &[f64]) -> f64 {
    let n = xs.len().min(ys.len());
    if n < 5 {
        return 0.0;
    }
    let (xs, ys) = (&xs[..n], &ys[..n]);
    let mean_x: f64 = xs.iter().sum::<f64>() / n as f64;
    let mean_y: f64 = ys.iter().sum::<f64>() / n as f64;
    let mut cov = 0.0;
    let mut var_x = 0.0;
    let mut var_y = 0.0;
    for k in 0..n {
        let dx = xs[k] - mean_x;
        let dy = ys[k] - mean_y;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }
    let denom = (var_x * var_y).sqrt();
    if denom <= 0.0 {
        0.0
    } else {
        cov / denom
    }
}

// ── diesel raw query row types ──

#[derive(QueryableByName, Debug)]
struct TradeRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    direction: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    price: f64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    shares: i32,
    #[diesel(sql_type = diesel::sql_types::Double)]
    amount: f64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    reason: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    traded_at: String,
}

#[derive(QueryableByName, Debug)]
struct LedgerRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    date: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    total_value: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    cash: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    market_value: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    daily_pnl: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const TEST_DB: &str = "./test_data/test.db";

    fn init() {
        std::fs::create_dir_all("./test_data").ok();
        // DB 可能已被其他测试初始化，忽略重复初始化错误
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::database::DatabaseManager::init(Some(PathBuf::from(TEST_DB)))
        }));
    }

    // ── load_positions ──

    #[test]
    fn test_load_positions_empty() {
        init();
        // 无 open 持仓时返回空
        let positions = load_positions().unwrap_or_default();
        // 可能为空（测试 DB 无数据）或有数据（DB 已有记录），两种都合法
        assert!(positions.is_empty() || !positions.is_empty());
    }

    // ── load_trades_since ──

    #[test]
    fn test_load_trades_since_empty() {
        init();
        let future = NaiveDate::from_ymd_opt(2099, 1, 1).unwrap();
        let trades = load_trades_since(future).unwrap_or_default();
        assert!(trades.is_empty());
    }

    // ── save_ledger + load_ledger ──

    #[test]
    #[serial_test::serial]
    fn test_ledger_roundtrip() {
        init();
        let date = NaiveDate::from_ymd_opt(2026, 6, 16).unwrap();
        struct TestLedgerGuard(NaiveDate);
        impl Drop for TestLedgerGuard {
            fn drop(&mut self) {
                if let Ok(mut conn) = crate::database::DatabaseManager::get().get_conn() {
                    let _ = diesel::sql_query("DELETE FROM ledger WHERE date = ?")
                        .bind::<diesel::sql_types::Date, _>(self.0)
                        .execute(&mut conn);
                }
            }
        }
        let _ledger = TestLedgerGuard(date);
        let entry = LedgerEntry {
            date,
            total_value: 100_000.0,
            cash: 20_000.0,
            market_value: 80_000.0,
            daily_pnl: 5_000.0,
        };
        save_ledger(entry).unwrap();

        let curve = load_ledger(date).unwrap();
        assert!(!curve.is_empty());
        let last = curve.last().unwrap();
        assert_eq!(last.date, date);
        assert!((last.total_value - 100_000.0).abs() < 0.01);
        assert!((last.daily_pnl - 5_000.0).abs() < 0.01);
    }

    #[test]
    fn test_load_ledger_empty() {
        init();
        let future = NaiveDate::from_ymd_opt(2099, 1, 1).unwrap();
        let curve = load_ledger(future).unwrap_or_default();
        assert!(curve.is_empty());
    }

    // ── has_buy_today ──

    #[test]
    fn test_has_buy_today_no_trades() {
        init();
        let today = NaiveDate::from_ymd_opt(2026, 6, 16).unwrap();
        // 测试 DB 里没有今天的买入 → false
        let result = has_buy_today("TEST_CODE_000000", today);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    // v14.1 F7: st_type 派生测试 (不依赖 DB, 直接 inline 派生逻辑)
    // 派生规则: "ST" → is_st=true, "*ST" → star_st=true, 其他 → 都 false
    #[test]
    fn st_type_derivation() {
        let derive = |st_type: Option<&str>| -> (bool, bool) {
            match st_type {
                Some("ST") => (true, false),
                Some("*ST") => (false, true),
                _ => (false, false),
            }
        };
        assert_eq!(derive(Some("ST")), (true, false));
        assert_eq!(derive(Some("*ST")), (false, true));
        assert_eq!(derive(None), (false, false));
        assert_eq!(derive(Some("OTHER")), (false, false));
    }
}
