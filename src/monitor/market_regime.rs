//! 市场情绪周期（2026-10-24，取自「逻辑—情绪」框架里可测的那部分）。
//!
//! 只做三件事：
//! 1. 每交易日 15:35 落一份**涨停池快照**（`limit_up_pool_daily`，含连板高度/封单/炸板次数/首封时间）；
//! 2. 由快照 + 当日日线算一份**市场情绪快照**（`market_regime_daily`）：涨停家数、最高连板、
//!    炸板数、跌停家数、昨日涨停今日溢价；
//! 3. 给出五档阶段（冰点/修复/发酵/高潮/退潮）——只用于两件事：
//!    高潮期不给高位追价、冰点期放宽门槛。**不参与选股排序。**
//!
//! 阈值是**预注册**的（写下就不再按周调）：
//! - 冰点：涨停 < 40
//! - 退潮：昨日涨停今日溢价 ≤ −1.5%（且昨日池 ≥ 40 只），或涨停数 < 昨日池 × 0.6
//! - 高潮：涨停 ≥ 100 且最高连板 ≥ 6
//! - 发酵：涨停 ≥ 60
//! - 其余：修复
//!
//! 边界：跌停家数用 `pct_chg <= -9.5` 近似（20cm 票会被低估，只作为情绪温度，不当作精确计数）。

use crate::data_gateway::grpc_source::bridge_for;
use crate::data_gateway::GatewayBatch;
use crate::database::DatabaseManager;
use diesel::sql_types::{BigInt, Double, Integer, Nullable, Text};
use diesel::Connection as _;
use diesel::RunQueryDsl as _;

/// 预注册阈值。
pub const ICE_LIMIT_UP: usize = 40;
pub const FERMENT_LIMIT_UP: usize = 60;
pub const CLIMAX_LIMIT_UP: usize = 100;
pub const CLIMAX_STREAK: u32 = 6;
pub const EBB_PREMIUM: f64 = -1.5;
pub const EBB_SHRINK: f64 = 0.6;
pub const EBB_MIN_POOL: usize = 40;
/// 连板高度达到这个值算「情绪段」（不给回踩价、高潮期不追价）。
pub const EMOTION_STREAK: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketPhase {
    Ice,
    Repair,
    Ferment,
    Climax,
    Ebb,
}

impl MarketPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ice => "冰点",
            Self::Repair => "修复",
            Self::Ferment => "发酵",
            Self::Climax => "高潮",
            Self::Ebb => "退潮",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Ice => "ice",
            Self::Repair => "repair",
            Self::Ferment => "ferment",
            Self::Climax => "climax",
            Self::Ebb => "ebb",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ice" => Some(Self::Ice),
            "repair" => Some(Self::Repair),
            "ferment" => Some(Self::Ferment),
            "climax" => Some(Self::Climax),
            "ebb" => Some(Self::Ebb),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MarketSnapshot {
    pub trade_date: String,
    pub limit_up_count: usize,
    pub max_streak: u32,
    pub broken_count: usize,
    pub limit_down_count: usize,
    pub prev_pool_count: usize,
    pub prev_premium: Option<f64>,
    pub phase: MarketPhase,
}

impl MarketSnapshot {
    pub fn summary_line(&self) -> String {
        format!(
            "[情绪周期] date={} phase={} 涨停={} 最高连板={} 炸板={} 跌停={} 昨涨停溢价={} 昨池={}",
            self.trade_date,
            self.phase.label(),
            self.limit_up_count,
            self.max_streak,
            self.broken_count,
            self.limit_down_count,
            self.prev_premium
                .map(|value| format!("{value:+.2}%"))
                .unwrap_or_else(|| "无".to_string()),
            self.prev_pool_count
        )
    }
}

pub fn ensure_schema(conn: &mut diesel::SqliteConnection) -> Result<(), String> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS limit_up_pool_daily (\
         trade_date TEXT NOT NULL,\
         code TEXT NOT NULL,\
         streak INTEGER,\
         sealed_amount REAL,\
         break_count INTEGER,\
         first_seal_at TEXT,\
         board_name TEXT,\
         PRIMARY KEY (trade_date, code))",
    )
    .execute(conn)
    .map_err(|error| format!("limit_up_pool_daily 建表失败: {error}"))?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS market_regime_daily (\
         trade_date TEXT PRIMARY KEY,\
         limit_up_count INTEGER NOT NULL,\
         max_streak INTEGER NOT NULL,\
         broken_count INTEGER NOT NULL,\
         limit_down_count INTEGER NOT NULL,\
         prev_pool_count INTEGER NOT NULL,\
         prev_premium REAL,\
         phase TEXT NOT NULL,\
         captured_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)",
    )
    .execute(conn)
    .map_err(|error| format!("market_regime_daily 建表失败: {error}"))?;
    Ok(())
}

/// 预注册的五档判定（纯函数，单测覆盖）。
pub fn classify(
    limit_up_count: usize,
    max_streak: u32,
    prev_premium: Option<f64>,
    prev_pool_count: usize,
) -> MarketPhase {
    if prev_pool_count >= EBB_MIN_POOL {
        if let Some(premium) = prev_premium {
            if premium <= EBB_PREMIUM {
                return MarketPhase::Ebb;
            }
        }
        if (limit_up_count as f64) < prev_pool_count as f64 * EBB_SHRINK {
            return MarketPhase::Ebb;
        }
    }
    if limit_up_count < ICE_LIMIT_UP {
        return MarketPhase::Ice;
    }
    if limit_up_count >= CLIMAX_LIMIT_UP && max_streak >= CLIMAX_STREAK {
        return MarketPhase::Climax;
    }
    if limit_up_count >= FERMENT_LIMIT_UP {
        return MarketPhase::Ferment;
    }
    MarketPhase::Repair
}

/// 当日快照（15:35 调度用）。涨停池拉取失败返回 Err，调用方保留重试。
pub async fn capture(db: &DatabaseManager, date: &str) -> Result<MarketSnapshot, String> {
    let day = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|error| format!("日期非法 {date}: {error}"))?;
    let bridge = bridge_for("LimitPools").map_err(|error| error.to_string())?;
    let entries = match bridge.limit_pools_async(day).await {
        Ok(GatewayBatch::Available { records, .. }) => records,
        Ok(GatewayBatch::VerifiedEmpty(_)) => Vec::new(),
        Err(error) => return Err(format!("涨停池拉取失败: {error}")),
    };

    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    ensure_schema(&mut conn)?;
    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        diesel::sql_query("DELETE FROM limit_up_pool_daily WHERE trade_date = ?1")
            .bind::<Text, _>(date.to_owned())
            .execute(conn)?;
        Ok(())
    })
    .map_err(|error| format!("清旧快照失败: {error}"))?;

    let mut max_streak = 0u32;
    let mut broken = 0usize;
    for entry in &entries {
        let streak = entry.streak.map(|value| value.get()).unwrap_or(0);
        max_streak = max_streak.max(streak);
        if entry.break_count.unwrap_or(0) > 0 {
            broken += 1;
        }
        diesel::sql_query(
            "INSERT OR REPLACE INTO limit_up_pool_daily \
             (trade_date, code, streak, sealed_amount, break_count, first_seal_at, board_name) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind::<Text, _>(date.to_owned())
        .bind::<Text, _>(entry.instrument.code().to_owned())
        .bind::<Nullable<Integer>, _>(entry.streak.map(|value| value.get() as i32))
        .bind::<Nullable<Double>, _>(entry.sealed_amount.map(|value| value.get()))
        .bind::<Nullable<Integer>, _>(entry.break_count.map(|value| value as i32))
        .bind::<Nullable<Text>, _>(
            entry
                .first_seal_at
                .as_ref()
                .map(|value| value.as_str().to_owned()),
        )
        .bind::<Nullable<Text>, _>(
            entry
                .board_name
                .as_ref()
                .map(|value| value.as_str().to_owned()),
        )
        .execute(&mut conn)
        .map_err(|error| format!("{} 涨停快照写失败: {error}", entry.instrument.code()))?;
    }

    // 昨日池 → 今日溢价。
    #[derive(diesel::QueryableByName)]
    struct PrevRow {
        #[diesel(sql_type = Text)]
        trade_date: String,
    }
    let prev_date: Option<String> = diesel::sql_query(
        "SELECT trade_date FROM limit_up_pool_daily WHERE trade_date < ?1 ORDER BY trade_date DESC LIMIT 1",
    )
    .bind::<Text, _>(date.to_owned())
    .get_result::<PrevRow>(&mut conn)
    .ok()
    .map(|row| row.trade_date);

    let (prev_pool_count, prev_premium) = match prev_date.as_deref() {
        Some(prev) => {
            #[derive(diesel::QueryableByName)]
            struct CountRow {
                #[diesel(sql_type = BigInt)]
                n: i64,
            }
            let count: i64 = diesel::sql_query(
                "SELECT COUNT(*) AS n FROM limit_up_pool_daily WHERE trade_date = ?1",
            )
            .bind::<Text, _>(prev.to_owned())
            .get_result::<CountRow>(&mut conn)
            .map(|row| row.n)
            .unwrap_or(0);
            #[derive(diesel::QueryableByName)]
            struct AvgRow {
                #[diesel(sql_type = Nullable<Double>)]
                avg: Option<f64>,
            }
            let avg: Option<f64> = diesel::sql_query(
                "SELECT AVG(d.pct_chg) AS avg FROM limit_up_pool_daily p \
                  JOIN stock_daily d ON d.code = p.code AND d.date = ?1 \
                 WHERE p.trade_date = ?2",
            )
            .bind::<Text, _>(date.to_owned())
            .bind::<Text, _>(prev.to_owned())
            .get_result::<AvgRow>(&mut conn)
            .ok()
            .and_then(|row| row.avg);
            (count as usize, avg)
        }
        None => (0, None),
    };

    #[derive(diesel::QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    let limit_down_count: i64 = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM stock_daily WHERE date = ?1 AND pct_chg <= -9.5",
    )
    .bind::<Text, _>(date.to_owned())
    .get_result::<CountRow>(&mut conn)
    .map(|row| row.n)
    .unwrap_or(0);

    let phase = classify(entries.len(), max_streak, prev_premium, prev_pool_count);
    diesel::sql_query(
        "INSERT INTO market_regime_daily \
         (trade_date, limit_up_count, max_streak, broken_count, limit_down_count, prev_pool_count, prev_premium, phase) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(trade_date) DO UPDATE SET \
           limit_up_count=excluded.limit_up_count, max_streak=excluded.max_streak, \
           broken_count=excluded.broken_count, limit_down_count=excluded.limit_down_count, \
           prev_pool_count=excluded.prev_pool_count, prev_premium=excluded.prev_premium, \
           phase=excluded.phase, captured_at=CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(date.to_owned())
    .bind::<Integer, _>(entries.len() as i32)
    .bind::<Integer, _>(max_streak as i32)
    .bind::<Integer, _>(broken as i32)
    .bind::<Integer, _>(limit_down_count as i32)
    .bind::<Integer, _>(prev_pool_count as i32)
    .bind::<Nullable<Double>, _>(prev_premium)
    .bind::<Text, _>(phase.code().to_owned())
    .execute(&mut conn)
    .map_err(|error| format!("情绪快照落库失败: {error}"))?;

    Ok(MarketSnapshot {
        trade_date: date.to_string(),
        limit_up_count: entries.len(),
        max_streak,
        broken_count: broken,
        limit_down_count: limit_down_count as usize,
        prev_pool_count,
        prev_premium,
        phase,
    })
}

/// 查询某日阶段（读不到返回 None，不猜）。
pub fn phase_on(conn: &mut diesel::SqliteConnection, date: &str) -> Option<MarketPhase> {
    #[derive(diesel::QueryableByName)]
    struct PhaseRow {
        #[diesel(sql_type = Text)]
        phase: String,
    }
    let row: Option<PhaseRow> = diesel::sql_query(
        "SELECT phase FROM market_regime_daily WHERE trade_date <= ?1 ORDER BY trade_date DESC LIMIT 1",
    )
    .bind::<Text, _>(date.to_owned())
    .get_result(conn)
    .ok();
    row.and_then(|row| MarketPhase::parse(&row.phase))
}

/// 某票在某日的连板高度（读不到按 0 处理——不把未知当高位）。
pub fn streak_on(conn: &mut diesel::SqliteConnection, date: &str, code: &str) -> u32 {
    #[derive(diesel::QueryableByName)]
    struct StreakRow {
        #[diesel(sql_type = Nullable<Integer>)]
        streak: Option<i32>,
    }
    let row: Option<StreakRow> = diesel::sql_query(
        "SELECT streak FROM limit_up_pool_daily WHERE trade_date = ?1 AND code = ?2",
    )
    .bind::<Text, _>(date.to_owned())
    .bind::<Text, _>(code.to_owned())
    .get_result(conn)
    .ok();
    row.and_then(|row| row.streak)
        .map(|value| value.max(0) as u32)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ice_when_limit_up_thin() {
        assert_eq!(classify(20, 2, None, 0), MarketPhase::Ice);
        assert_eq!(classify(39, 5, Some(1.0), 30), MarketPhase::Ice);
    }

    #[test]
    fn ebb_wins_over_ice_when_yesterday_pool_bled() {
        // 昨日 60 只涨停，今天只剩 30 只 → 退潮（即便绝对值低于 40 的冰点线）
        assert_eq!(classify(30, 3, Some(0.5), 60), MarketPhase::Ebb);
        // 昨日涨停今天平均 -2.1% → 退潮
        assert_eq!(classify(70, 4, Some(-2.1), 50), MarketPhase::Ebb);
    }

    #[test]
    fn climax_needs_both_breadth_and_height() {
        assert_eq!(classify(120, 6, Some(1.0), 60), MarketPhase::Climax);
        // 家数够但高度不够 → 发酵
        assert_eq!(classify(120, 3, Some(1.0), 60), MarketPhase::Ferment);
        assert_eq!(classify(80, 5, None, 0), MarketPhase::Ferment);
        assert_eq!(classify(50, 3, None, 0), MarketPhase::Repair);
    }

    #[test]
    fn small_prev_pool_does_not_trigger_ebb() {
        // 昨日只有 10 只涨停，今天 5 只：不按「缩量 60%」判退潮（小样本不判）
        assert_eq!(classify(5, 1, Some(-3.0), 10), MarketPhase::Ice);
    }
}
