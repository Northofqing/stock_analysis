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

    let Some(db) = DatabaseManager::try_get() else {
        log::warn!("[Prediction] DB 未初始化");
        return;
    };

    if let Err(e) = db.save_prediction_legacy(&today, &tomorrow, theme, stock, direction, score, detail) {
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
/// 1. 查过去 7 个日历日所有未 verify 的 prediction（`hit IS NULL`），覆盖周末 + 7 天小长假
///    — 修复 C-1 (2026-06-29 codex review): 周五推送的 prediction 周一 verify 时
///    "yesterday"=周日 但推送都在周五，原本只查 yesterday 会漏掉整批 pending。
/// 2. 对每条 prediction，从本地 stock_daily 表读 pred_date 与 target_date 的 close
///    （不再调网络：本地库是单次 verify 的 source of truth，避免远程 fetch 失败污染 verify）
/// 3. 计算 actual_change (%)，按 pred_direction 判定 hit（阈值 ±0.5%）
/// 4. 写回 actual_change / hit / actual_result
/// 5. 输出 `pending_count` vs `verified_count`, 差距 > 50 触发 WARN 告警, 让 operator 看到
///    "verify 静默跳过" 这一类数据源问题, 而非误以为一切正常.
///
/// 异步函数（`pub async fn`），供 monitor 主循环 `.await` 调用。
pub async fn verify_predictions() {
    let today_date = Local::now().date_naive();
    let today = today_date.format("%Y-%m-%d").to_string();
    let Some(db) = DatabaseManager::try_get() else {
        log::warn!("[Prediction] DB 未初始化");
        return;
    };

    let mut total_pending = 0usize;
    let mut verified = 0usize;
    let mut skipped = 0usize;
    let mut oldest_pred_date = String::new();

    // 1. 循环过去 7 天 (覆盖周一周二补跑周末遗漏 + 节假日), 每一天都查 pending.
    for offset in 1..=7i64 {
        let pred_date = today_date - chrono::Duration::days(offset);
        let target_date = pred_date + chrono::Duration::days(1);
        let pred_date_s = pred_date.format("%Y-%m-%d").to_string();
        let target_date_s = target_date.format("%Y-%m-%d").to_string();

        let pending = match db.get_pending_predictions(&pred_date_s) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("[Prediction] 查 pending ({} → {}) 失败: {}", pred_date_s, target_date_s, e);
                continue;
            }
        };

        if pending.is_empty() { continue; }
        total_pending += pending.len();
        if oldest_pred_date.is_empty() { oldest_pred_date = pred_date_s.clone(); }

        for pred in pending {
            let Some(code) = pred.stock_code.as_deref() else {
                skipped += 1;
                continue;
            };
            if code.is_empty() {
                skipped += 1;
                continue;
            }
            let direction = pred.pred_direction.as_str();

            // 2-4. 共享 verify 逻辑: 读 close + 算 actual_change + 判定 hit
            // verify_one 内部向前找最近交易日 (修复 C-1: 周末/节假日不静默 skip)
            let outcome = match verify_one(&db, code, &pred_date_s, &target_date_s, direction).await {
                Some(o) => o,
                None => {
                    // 真正缺数据 (7 天内都没 close), 不能算 verify 失败, 计入 skipped
                    log::debug!("[Prediction] {} {} → {} 7 天内无可用 close, skip", code, pred_date_s, target_date_s);
                    skipped += 1;
                    continue;
                }
            };

            // 5. 写回
            if let Err(e) = db.update_prediction_result(&pred_date_s, Some(code), outcome.actual_change, outcome.hit) {
                log::warn!("[Prediction] {} {} 写回失败: {}", code, pred_date_s, e);
                skipped += 1;
                continue;
            }
            verified += 1;
        }
    }

    // 6. 告警: pending_count >> verified_count 说明数据源/DB 有问题
    if total_pending == 0 {
        log::info!("[Prediction] 本轮无 pending prediction (latest: {})", today);
    } else {
        log::info!(
            "[Prediction] 本轮 verify: pending={} verified={} skipped={} (range: {} → {})",
            total_pending, verified, skipped,
            oldest_pred_date, today
        );
        if total_pending > 0 && verified == 0 {
            log::warn!(
                "[Prediction] ⚠ pending={} 但 verified=0 — 可能是 stock_daily 数据断层或 DB 异常, 检查 backfill_daily.sh",
                total_pending
            );
        } else if skipped as f64 / total_pending as f64 > 0.3 {
            log::warn!(
                "[Prediction] ⚠ skipped/total = {:.0}% (>30%), 检查 stock_daily 数据完整性",
                skipped as f64 / total_pending as f64 * 100.0
            );
        }
    }
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
/// 返回 `None` 表示无法判定 (缺 close / prev_close <= 0 / 7 天内都无 close), 调用方自行决定 warn-and-continue 还是 `?` 报错。
///
/// **修复 C-1 (2026-06-29 codex review)**: 周五推送的 prediction, 周一早上 verify 时
/// pred_date=周五有数据, target_date=周六/周日/周一 (周一有数据), 但若 pred_date 在周末
/// (跨节假日 A 股不交易) 或 target_date 跨周末, 原本 `read_stock_daily_close` 会返回 None
/// → verify 静默跳过。现改为向前找最近有数据的交易日, 最多 7 天 offset。
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
    let prev_close = read_stock_daily_close_with_offset(db, code, pred_date)?;
    let target_close = read_stock_daily_close_with_offset(db, code, target_date)?;
    if prev_close <= 0.0 { return None; }

    let actual_change = (target_close - prev_close) / prev_close * 100.0;
    let hit_threshold = 0.5_f64;
    let direction_match = match direction {
        "看多" => actual_change > hit_threshold,
        "看空" => actual_change < -hit_threshold,
        // 修复 I-2 (2026-06-29 codex review): 中性 prediction 不管涨跌都不算 hit,
        // |actual| > 0.5 的子句原本冗余 (direction_match 已经隐含), 现在删掉.
        _ => false,
    };
    Some(VerifyOutcome { actual_change, hit: direction_match })
}

/// 修复 C-1 (2026-06-29 codex review): 向前找最近的有 close 数据的交易日 (最多 7 天 offset)。
/// A 股周末不交易, 国庆/春节等节假日也不交易, 直接查 pred_date / target_date 可能无数据.
/// 返回 None 表示 7 天内都找不到有数据的 close (股票可能停牌或 stock_daily 未回填).
///
/// 语义: 给定 `date` (字符串), 从 date 开始向后查 stock_daily, 直到找到有 close 数据的日期,
/// 最多 7 天 (覆盖春节 7 天假期 + 1 天 buffer). 用于:
/// - pred_date 是周五, 实际推送日 = 周五 (有数据)
/// - target_date 是 "next trading day", 通常是推送后第 1 个交易日, 但 verify 跨周末跑
///   时可能还没数据 (target_date=周一 verify 在周一晚上跑, 有数据; verify 在周日跑, 无数据)
fn read_stock_daily_close_with_offset(
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

    let base_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    for offset in 0..=7i64 {
        let candidate = base_date + chrono::Duration::days(offset);
        let candidate_s = candidate.format("%Y-%m-%d").to_string();
        let result = diesel::sql_query(format!(
            "SELECT close FROM stock_daily WHERE code = '{}' AND date = '{}' LIMIT 1",
            code, candidate_s
        ))
        .get_result::<CloseRow>(&mut *conn)
        .ok()
        .and_then(|r| r.close);
        if let Some(close) = result {
            return Some(close);
        }
    }
    None
}

/// 获取近期命中率
pub fn recent_hit_rate(days: i32) -> f64 {
    let Some(db) = DatabaseManager::try_get() else { return 0.0; };
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
