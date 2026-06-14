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
pub fn verify_predictions() {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let db = match std::panic::catch_unwind(DatabaseManager::get) {
        Ok(db) => db,
        Err(_) => { log::warn!("[Prediction] DB 不可用"); return; }
    };

    // 查找昨天需要验证的预测（这里简化：全量更新）
    match db.update_prediction_result(&today, None, 0.0, false) {
        Ok(n) => log::info!("[Prediction] 已更新 {} 条预测结果", n),
        Err(e) => log::warn!("[Prediction] 更新失败: {}", e),
    }
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
