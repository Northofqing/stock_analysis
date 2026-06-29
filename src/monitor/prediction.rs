//! 预测追踪闭环（Phase 5）。
//!
//! 核心：
//! - save: 记录今日预测（主线方向 + 评分）
//! - verify: 次日收盘后回填实际结果
//! - hit_rate: 查询近期命中率

use crate::database::DatabaseManager;
use chrono::Local;

/// 保存一条预测
pub fn save_prediction(
    theme: Option<&str>,
    stock: Option<&str>,
    direction: &str,
    score: f64,
    detail: Option<&str>,
) {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let tomorrow = (Local::now() + chrono::Duration::days(1)).format("%Y-%m-%d").to_string();

    let db = match std::panic::catch_unwind(DatabaseManager::get) {
        Ok(db) => db,
        Err(_) => { log::warn!("[Prediction] DB 不可用"); return; }
    };

    if let Err(e) = db.save_prediction(&today, &tomorrow, theme, stock, direction, score, detail) {
        log::warn!("[Prediction] 保存失败: {}", e);
    } else {
        log::info!("[Prediction] ✓ {} {} {}分", direction, stock.unwrap_or(theme.unwrap_or("?")), score);
    }
}

/// 回填实际结果（次日收盘后调用）
///
/// 修复 R-1: 必须真实拉取 stock_daily 的次日实际收盘价，计算 actual_change，
/// 根据 pred_direction 判定 hit。不再硬编码 0.0, false。
///
/// 实现要点：
/// 1. 查昨天所有未 verify 的 prediction（`hit IS NULL`）
/// 2. 对每条 prediction，从本地 stock_daily 表读 pred_date 与 target_date 的 close
///    （不再调网络：本地库是单次 verify 的 source of truth，避免远程 fetch 失败污染 verify）
/// 3. 计算 actual_change (%)，按 pred_direction 判定 hit（阈值 ±0.5%）
/// 4. 写回 actual_change / hit / actual_result
///
/// 异步函数（`pub async fn`），供 monitor 主循环 `.await` 调用。
pub async fn verify_predictions() {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let yesterday = (Local::now() - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
    let db = match std::panic::catch_unwind(DatabaseManager::get) {
        Ok(db) => db,
        Err(_) => { log::warn!("[Prediction] DB 不可用"); return; }
    };

    // 1. 查昨天所有未 verify 的 prediction
    let pending = match db.get_pending_predictions(&yesterday) {
        Ok(v) => v,
        Err(e) => { log::warn!("[Prediction] 查 pending 失败: {}", e); return; }
    };

    let mut verified = 0usize;
    for pred in pending {
        let Some(code) = pred.stock_code.as_deref() else { continue; };
        if code.is_empty() { continue; }
        let direction = pred.pred_direction.as_str();

        // 2-4. 共享 verify 逻辑: 读 close + 算 actual_change + 判定 hit
        let outcome = match verify_one(&db, code, &yesterday, &today, direction).await {
            Some(o) => o,
            None => continue, // read 失败 / 缺数据 / prev_close <= 0
        };

        // 5. 写回
        if let Err(e) = db.update_prediction_result(&yesterday, Some(code), outcome.actual_change, outcome.hit) {
            log::warn!("[Prediction] {} 写回失败: {}", code, e);
            continue;
        }
        verified += 1;
    }
    log::info!("[Prediction] 已 verify {} 条 prediction ({} → {})", verified, yesterday, today);
}

/// 单条 prediction 的 verify 结果。
/// actual_change 单位为 %; hit 为方向匹配 + |actual| > 0.5%。
#[derive(Debug, Clone, Copy)]
pub struct VerifyOutcome {
    pub actual_change: f64,
    pub hit: bool,
}

/// 共享 verify 逻辑 — 读 (pred_date, target_date) 两个本地 close, 算 actual_change, 判定 hit。
///
/// 被 `verify_predictions` (生产盘后回填) 和 `backfill_predictions` (历史回填) 复用, 避免逐字复制。
/// 返回 `None` 表示无法判定 (缺 close / prev_close <= 0), 调用方自行决定 warn-and-continue 还是 `?` 报错。
///
/// 注意: 此函数**不**写回数据库, 调用方负责 `update_prediction_result` — 这样 verify_predictions
/// 的"warn + 继续"语义和 backfill_predictions 的"`?` 失败"语义都能保留。
pub async fn verify_one(
    db: &DatabaseManager,
    code: &str,
    pred_date: &str,
    target_date: &str,
    direction: &str,
) -> Option<VerifyOutcome> {
    let prev_close = read_stock_daily_close(db, code, pred_date)?;
    let target_close = read_stock_daily_close(db, code, target_date)?;
    if prev_close <= 0.0 { return None; }

    let actual_change = (target_close - prev_close) / prev_close * 100.0;
    let hit_threshold = 0.5_f64;
    let direction_match = match direction {
        "看多" => actual_change > hit_threshold,
        "看空" => actual_change < -hit_threshold,
        _ => false,
    };
    let hit = direction_match && actual_change.abs() > hit_threshold;
    Some(VerifyOutcome { actual_change, hit })
}

/// 从本地 stock_daily 表读取某 code+date 的收盘价 (verify 的 source of truth)
fn read_stock_daily_close(
    db: &DatabaseManager,
    code: &str,
    date: &str,
) -> Option<f64> {
    use diesel::RunQueryDsl;
    #[derive(diesel::QueryableByName, Debug)]
    struct CloseRow {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
        close: Option<f64>,
    }
    let mut conn = db.get_conn().ok()?;
    diesel::sql_query(format!(
        "SELECT close FROM stock_daily WHERE code = '{}' AND date = '{}' LIMIT 1",
        code, date
    ))
    .get_result::<CloseRow>(&mut *conn)
    .ok()
    .and_then(|r| r.close)
}

/// 获取近期命中率
pub fn recent_hit_rate(days: i32) -> f64 {
    let db = match std::panic::catch_unwind(DatabaseManager::get) {
        Ok(db) => db,
        Err(_) => return 0.0,
    };
    db.get_prediction_hit_rate(days).unwrap_or(0.0)
}

/// 命中率摘要（用于报告）
pub fn hit_rate_summary(days: i32) -> String {
    let rate = recent_hit_rate(days);
    format!("近{}天预测命中率: {:.0}%", days, rate * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hit_rate_format() {
        let s = hit_rate_summary(7);
        assert!(s.contains("命中率"));
    }

    #[test]
    fn test_save_prediction_no_panic() {
        // 在没有 DB 的环境下也不应 panic
        save_prediction(None, Some("000001"), "看多", 75.0, None);
    }
}
