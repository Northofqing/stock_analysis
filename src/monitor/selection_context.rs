//! 选股时点上下文（2026-10-29）。
//!
//! 目的：记录“这只票被选出时，之前已经走了多远”，供 shadow 观察和后续
//! 独立评估使用。它不改变提案、触发或用户可见推送；阈值是观察口径，
//! 不是交易闸门。
//!
//! 计算只用 `stock_daily` 中 `date <= as_of` 的日线，避免把未来数据带进来。
//! 任一历史窗口不足时返回 `None`，由 `ExtensionPolicy` 统一标记 `Unknown`，
//! 不补 0、不缩短窗口冒充 60 日。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};
use serde::{Deserialize, Serialize};

/// 观察口径版本；修改阈值时同步升级版本，避免旧 context 被新规则改写。
pub const RULE_VERSION: &str = "selection_v1";

/// 20 日涨幅超过 20% 视为“已经走过一段”。
pub const PRE20_EXTENDED: f64 = 0.20;
/// 距 60 日最高不足 5% 视为“贴近区间上沿”。
pub const DIST_HI60_NEAR: f64 = -0.05;
/// 60 日区间位置高于 70% 视为“处于区间上部”。
pub const POS60_HIGH: f64 = 0.70;
/// 连续 2 个及以上涨停视为短线已充分发酵。
pub const LIMIT_UP_STREAK_EXTENDED: u32 = 2;

const LOOKBACK_BARS: i64 = 60;

/// 选股时的历史价格位置。全部字段都是“选股日收盘”口径。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SelectionContext {
    pub pre5_return: Option<f64>,
    pub pre10_return: Option<f64>,
    pub pre20_return: Option<f64>,
    /// 收盘价相对 60 日最高价的折价，例如 -0.08 = 距高点 8%。
    pub dist_hi60: Option<f64>,
    /// 收盘价在 60 日高低区间中的位置，0=最低，1=最高。
    pub position_60d: Option<f64>,
    /// 截至选股日的连续涨停天数；0 表示没有连续涨停。
    pub limit_up_streak: Option<u32>,
    /// 实际参与计算的日线根数（最多 60）。
    pub bars_used: usize,
}

/// 派生状态只用于 shadow 观察，不影响 `EntryPolicy` 的触发结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionState {
    #[default]
    Unknown,
    Normal,
    Extended,
}

impl ExtensionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Normal => "normal",
            Self::Extended => "extended",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionAssessment {
    pub state: ExtensionState,
    /// 命中的观察原因；保留全部原因，不只留第一个。
    pub reasons: Vec<String>,
    /// 生成该评估的观察阈值版本。
    #[serde(default)]
    pub rule_version: String,
}

/// 纯函数：同一份 SelectionContext 必须得到同一评估结果。
pub struct ExtensionPolicy;

impl ExtensionPolicy {
    pub fn evaluate(selection: &SelectionContext) -> ExtensionAssessment {
        let mut reasons = Vec::new();

        if let Some(value) = selection.pre20_return {
            if value > PRE20_EXTENDED {
                reasons.push(format!("pre20>{:.0}%", PRE20_EXTENDED * 100.0));
            }
        }
        if let Some(value) = selection.dist_hi60 {
            if value > DIST_HI60_NEAR {
                reasons.push(format!("dist_hi60>{:.0}%", DIST_HI60_NEAR * 100.0));
            }
        }
        if let Some(value) = selection.position_60d {
            if value > POS60_HIGH {
                reasons.push(format!("pos60>{:.0}%", POS60_HIGH * 100.0));
            }
        }
        if let Some(value) = selection.limit_up_streak {
            if value >= LIMIT_UP_STREAK_EXTENDED {
                reasons.push(format!("limit_up_streak>={LIMIT_UP_STREAK_EXTENDED}"));
            }
        }

        let has_all_windows = selection.pre20_return.is_some()
            && selection.dist_hi60.is_some()
            && selection.position_60d.is_some();
        let state = if !reasons.is_empty() {
            ExtensionState::Extended
        } else if has_all_windows {
            ExtensionState::Normal
        } else {
            ExtensionState::Unknown
        };

        ExtensionAssessment {
            state,
            reasons,
            rule_version: RULE_VERSION.to_owned(),
        }
    }
}

#[derive(QueryableByName)]
struct BarRow {
    #[diesel(sql_type = Text)]
    _date: String,
    #[diesel(sql_type = Nullable<Double>)]
    close: Option<f64>,
    #[diesel(sql_type = Nullable<Double>)]
    high: Option<f64>,
    #[diesel(sql_type = Nullable<Double>)]
    low: Option<f64>,
    #[diesel(sql_type = Nullable<Double>)]
    pct_chg: Option<f64>,
}

/// 从 `stock_daily` 读取截至 `as_of` 的最近 60 根日线。
///
/// 窗口不足时对应字段为 `None`；调用方不得把它当成 0 或 Normal。
pub fn load(
    conn: &mut SqliteConnection,
    code: &str,
    as_of: &str,
) -> Result<SelectionContext, String> {
    let bars: Vec<BarRow> = diesel::sql_query(
        "SELECT date AS _date, close, high, low, pct_chg \
         FROM stock_daily WHERE code = ? AND date <= ? \
         ORDER BY date DESC LIMIT ?",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(as_of)
    .bind::<BigInt, _>(LOOKBACK_BARS)
    .load(conn)
    .map_err(|error| format!("{code} 读取选股日线失败: {error}"))?;

    let latest = bars
        .first()
        .ok_or_else(|| format!("{code} 没有 <= {as_of} 的日线"))?;
    let close = latest
        .close
        .filter(|value| *value > 0.0)
        .ok_or_else(|| format!("{code} 最新收盘价不可用"))?;

    let return_since = |index: usize| -> Option<f64> {
        let base = bars.get(index)?.close?;
        if base > 0.0 {
            Some(close / base - 1.0)
        } else {
            None
        }
    };

    let mut context = SelectionContext {
        pre5_return: return_since(5),
        pre10_return: return_since(10),
        pre20_return: return_since(20),
        bars_used: bars.len(),
        ..Default::default()
    };

    // 60 日窗口必须完整，否则不计算区间位置和距高点，避免用不完整窗口冒充。
    if bars.len() >= LOOKBACK_BARS as usize {
        let window = &bars[..LOOKBACK_BARS as usize];
        if window
            .iter()
            .all(|row| row.high.is_some() && row.low.is_some())
        {
            let high60 = window
                .iter()
                .filter_map(|row| row.high)
                .fold(f64::NEG_INFINITY, f64::max);
            let low60 = window
                .iter()
                .filter_map(|row| row.low)
                .fold(f64::INFINITY, f64::min);
            if high60.is_finite() && high60 > 0.0 {
                context.dist_hi60 = Some(close / high60 - 1.0);
            }
            if high60.is_finite() && low60.is_finite() && high60 > low60 {
                context.position_60d = Some((close - low60) / (high60 - low60));
            }
        }
    }

    let threshold = crate::monitor::ticket_regime::limit_up_pct(code);
    context.limit_up_streak = latest.pct_chg.map(|_| {
        bars.iter()
            .take_while(|row| row.pct_chg.map(|value| value >= threshold).unwrap_or(false))
            .count() as u32
    });

    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::connection::SimpleConnection;

    fn base() -> SelectionContext {
        SelectionContext {
            pre5_return: Some(0.02),
            pre10_return: Some(0.03),
            pre20_return: Some(0.05),
            dist_hi60: Some(-0.15),
            position_60d: Some(0.35),
            limit_up_streak: Some(0),
            bars_used: 60,
        }
    }

    #[test]
    fn normal_base_is_not_extended() {
        let assessment = ExtensionPolicy::evaluate(&base());
        assert_eq!(assessment.state, ExtensionState::Normal);
        assert!(assessment.reasons.is_empty());
        assert_eq!(assessment.rule_version, RULE_VERSION);
    }

    #[test]
    fn long_run_and_near_high_are_reported_together() {
        let mut selection = base();
        selection.pre20_return = Some(0.30);
        selection.dist_hi60 = Some(-0.02);
        selection.position_60d = Some(0.82);
        selection.limit_up_streak = Some(2);

        let assessment = ExtensionPolicy::evaluate(&selection);
        assert_eq!(assessment.state, ExtensionState::Extended);
        assert_eq!(assessment.reasons.len(), 4);
        assert!(assessment
            .reasons
            .iter()
            .any(|reason| reason.contains("pre20")));
        assert!(assessment
            .reasons
            .iter()
            .any(|reason| reason.contains("dist_hi60")));
        assert!(assessment
            .reasons
            .iter()
            .any(|reason| reason.contains("pos60")));
        assert!(assessment
            .reasons
            .iter()
            .any(|reason| reason.contains("limit_up")));
    }

    #[test]
    fn missing_60d_window_is_unknown_not_normal() {
        let mut selection = base();
        selection.dist_hi60 = None;
        selection.position_60d = None;

        assert_eq!(
            ExtensionPolicy::evaluate(&selection).state,
            ExtensionState::Unknown
        );
    }

    #[test]
    fn load_computes_pre_returns_and_60d_position() {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        conn.batch_execute(
            "CREATE TABLE stock_daily (\
             code TEXT NOT NULL, date TEXT NOT NULL, close REAL, high REAL, low REAL, pct_chg REAL);",
        )
        .unwrap();

        for index in 0..60 {
            let date = format!("2026-08-{:02}", index + 1);
            let close = 10.0 + index as f64 * 0.1;
            diesel::sql_query(
                "INSERT INTO stock_daily (code, date, close, high, low, pct_chg) \
                 VALUES ('603626', ?1, ?2, ?3, ?4, 0.0)",
            )
            .bind::<Text, _>(&date)
            .bind::<Double, _>(close)
            .bind::<Double, _>(close + 0.2)
            .bind::<Double, _>(close - 0.2)
            .execute(&mut conn)
            .unwrap();
        }

        let selection = load(&mut conn, "603626", "2026-09-30").unwrap();
        assert_eq!(selection.bars_used, 60);
        assert!((selection.pre20_return.unwrap() - (15.9 / 13.9 - 1.0)).abs() < 1e-9);
        assert!((selection.dist_hi60.unwrap() - (15.9 / 16.1 - 1.0)).abs() < 1e-9);
        assert!(selection.position_60d.unwrap() > 0.96);
        assert_eq!(selection.limit_up_streak, Some(0));
    }
}
