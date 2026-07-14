//! v16.3 Commit 3 — 盘中监控 + 盘后整盘扫描 (R4 + R5 业务核心).
//!
//! 业务流:
//!   1. intraday_monitor.tick() 每 30s 跑一次
//!      - 扫 pushed_stocks (consumed_at IS NULL, push_time < now, 1h 内)
//!      - 4 步过滤: (a) metric_json 早盘量能 (b) 时间窗 (c) VirtualReason 命中 (d) 综合分 ≥ 6.0
//!      - 命中 → 调 risk_adapter 4 项检查 → 调 paper_trade::simulate Buy → 标记 consumed
//!   2. evening_review() 15:30 跑一次 (R5)
//!      - 整盘扫全天未消费推送 + Momentum 整合
//!      - 命中 → 调 paper_trade::simulate Buy → 标记 consumed (outcome = "Momentum")
//!
//! 8 Source trait 推迟到 v16.4; Commit 3 直接用 v16.2 现有函数调
//! (按 plan review 修复: signal_sources 改 None, 4 步过滤用 v16.2 6 函数直接调)

use crate::database::DatabaseManager;
use crate::trading::paper_trade::{self, Direction, PaperSignal};
use chrono::{DateTime, Duration, Local, NaiveDate};
use diesel::prelude::*;

/// 4 步过滤阈值常量
const PUSH_AGE_MAX_HOURS: i64 = 1;
const DECISION_SCORE_THRESHOLD: f64 = 6.0;
const VOLUME_SURGE_THRESHOLD: f64 = 5.0;
const MAX_CANDIDATES: i64 = 50;

/// 综合分评分
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub source: &'static str,
    pub code: String,
    pub score: f64,
}

#[derive(diesel::QueryableByName, Debug, Clone)]
struct Candidate {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    push_time: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    push_kind: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    push_price: f64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    metric_json: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source: String,
}

impl Candidate {
    fn push_time_parsed(&self) -> DateTime<Local> {
        // 字符串无时区, 用 NaiveDateTime 解析 + assume_local (避免 UTC→+08:00 偏移 8h)
        chrono::NaiveDateTime::parse_from_str(&self.push_time, "%Y-%m-%d %H:%M:%S%.3f")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(&self.push_time, "%Y-%m-%d %H:%M:%S"))
            .map(|ndt| ndt.and_local_timezone(Local).unwrap())
            .unwrap_or_else(|_| Local::now())
    }

    fn push_kind_label(&self) -> &'static str {
        match self.push_kind.as_str() {
            "D-01" => "NewsCatalyst",
            "盘后资金" => "MainNetInflow",
            "I-01" => "SectorLeader",
            "I-03" => "Breakout",
            "P-02" => "VolumeSurge",
            "AuctionAnomaly" => "AuctionAnomaly",
            "LLMSelect" => "LLMSelect",
            "Momentum" => "Momentum",
            _ => "Unknown",
        }
    }
}

/// intraday_monitor 主入口
pub struct IntradayMonitor;

impl IntradayMonitor {
    /// 每 30s 跑一次 (从 main_loop 调, 推送消费核心)
    pub fn tick(&self) -> Result<usize, String> {
        let now = Local::now();
        let cutoff = now - Duration::hours(PUSH_AGE_MAX_HOURS);
        let mut conn = DatabaseManager::get()
            .get_conn()
            .map_err(|e| format!("DB 连接失败: {}", e))?;

        // 1. 扫推送票池
        let candidates: Vec<Candidate> = diesel::sql_query(format!(
            "SELECT id, push_time, push_kind, code, name, push_price, metric_json, source \
             FROM pushed_stocks \
             WHERE consumed_at IS NULL AND push_time < '{}' \
             AND push_time > '{}' \
             ORDER BY push_time DESC LIMIT {}",
            now.format("%Y-%m-%d %H:%M:%S%.3f"),
            cutoff.format("%Y-%m-%d %H:%M:%S%.3f"),
            MAX_CANDIDATES
        ))
        .load::<Candidate>(&mut conn)
        .map_err(|e| format!("query pushed_stocks: {}", e))?;

        let mut emitted = 0;
        let candidate_count = candidates.len();

        // 2. 4 步过滤
        for cand in candidates {
            let cand_time = cand.push_time_parsed();
            if let Some(signal) = self.evaluate_candidate(&cand, cand_time) {
                if signal.score < DECISION_SCORE_THRESHOLD {
                    log::debug!(
                        "[intraday_monitor] 跳过 {}({}): 综合分 {:.1} < 阈值 {:.1}",
                        cand.name, cand.code, signal.score, DECISION_SCORE_THRESHOLD
                    );
                    continue;
                }

                // 3. risk_adapter 4 项检查 (复用 Commit 1)
                let paper_signal = PaperSignal {
                    plan_id: format!(
                        "intraday-{}-{}",
                        cand.code,
                        now.format("%Y%m%d%H%M%S%3f")
                    ),
                    code: cand.code.clone(),
                    name: cand.name.clone(),
                    direction: Direction::Buy,
                    price: cand.push_price,
                    quantity: 100,
                    virtual_reason: signal.source.to_string(),
                    is_limit_up: false,
                    is_limit_down: false,
                    is_suspended: false,
                    account_mode: "Normal".to_string(),
                    data_mode: "Full".to_string(),
                };
                match paper_trade::simulate(&paper_signal, cand.push_price, 0.0, 0.0, 0.0) {
                    Ok(_) => {
                        // 4. 标记 consumed
                        let outcome = signal.source;
                        diesel::sql_query(format!(
                            "UPDATE pushed_stocks SET consumed_at = '{}', consumed_by = 'intraday_monitor', outcome = '{}' \
                             WHERE id = {}",
                            now.format("%Y-%m-%d %H:%M:%S%.3f"),
                            outcome.replace('\'', "''"),
                            cand.id
                        ))
                        .execute(&mut conn)
                        .map_err(|e| format!("update consumed_at: {}", e))?;
                        emitted += 1;
                        log::info!(
                            "[intraday_monitor] 入候选 {}({}) source={} score={:.1} → 推 paper_trade",
                            cand.name, cand.code, signal.source, signal.score
                        );
                    }
                    Err(e) => {
                        log::warn!(
                            "[intraday_monitor] risk_adapter 拒 {}({}): {}",
                            cand.name, cand.code, e
                        );
                    }
                }
            }
        }

        if emitted > 0 {
            log::info!("[intraday_monitor] tick 完成: 消费 {} 条推送", emitted);
        } else {
            log::info!(
                "[intraday_monitor] tick: 扫到 {} 候选 (now={}, cutoff={})",
                candidate_count,
                now.format("%H:%M:%S"),
                cutoff.format("%H:%M:%S")
            );
        }
        Ok(emitted)
    }

    /// 4 步过滤: (a) metric_json (b) 时间窗 (c) VirtualReason 命中 (d) 计算综合分
    fn evaluate_candidate(&self, cand: &Candidate, now: DateTime<Local>) -> Option<Signal> {
        // (a) 解析 metric_json
        let metrics: serde_json::Value =
            serde_json::from_str(&cand.metric_json).unwrap_or_default();
        let vol_ratio = metrics
            .get("vol_ratio")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let push_subkind = metrics
            .get("push_subkind")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // 早盘量能: AuctionVolume 子类需 vol_ratio ≥ 5
        if push_subkind == "AuctionVolume" && vol_ratio < VOLUME_SURGE_THRESHOLD {
            return None;
        }

        // (b) 时间窗
        let push_age = now - cand.push_time_parsed();
        if push_age > Duration::hours(PUSH_AGE_MAX_HOURS) {
            return None;
        }

        // (c) VirtualReason 命中 (按 push_kind_label 分, v16.4 再走 trait)
        let label = cand.push_kind_label();
        let score = match label {
            "SectorLeader" => 7.0,
            "Breakout" => 7.5,
            "VolumeSurge" => 6.5,
            "AuctionAnomaly" => 6.5,
            "MainNetInflow" => 6.0,
            "NewsCatalyst" => 7.0,
            "LLMSelect" => 6.5,
            "Momentum" => 8.0,
            _ => 0.0,
        };

        if score == 0.0 {
            return None;
        }

        Some(Signal {
            source: cand.push_kind_label(),
            code: cand.code.clone(),
            score,
        })
    }
}

/// 盘后 15:30 整盘扫 (R5) — 复用 evaluate_candidate 评分, 跑 Momentum 整合
pub fn evening_review(today: NaiveDate) -> Result<usize, String> {
    let now = Local::now();
    let cutoff = today
        .and_hms_opt(15, 30, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;

    let candidates: Vec<Candidate> = diesel::sql_query(format!(
        "SELECT id, push_time, push_kind, code, name, push_price, metric_json, source \
         FROM pushed_stocks \
         WHERE consumed_at IS NULL AND push_time <= '{}' \
         AND date(push_time) = '{}' \
         ORDER BY push_time DESC LIMIT 100",
        cutoff.format("%Y-%m-%d %H:%M:%S"),
        today
    ))
    .load::<Candidate>(&mut conn)
    .map_err(|e| format!("evening query: {}", e))?;

    let mut emitted = 0;

    for cand in candidates {
        // Fix 5: R5 收紧: 仅 Momentum + score ≥ 8, 不限 1h 时间窗 (全天整盘扫)
        if cand.push_kind != "Momentum" {
            continue;
        }
        let m: serde_json::Value =
            serde_json::from_str(&cand.metric_json).unwrap_or_default();
        let vol = m.get("vol_ratio").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sub = m.get("push_subkind").and_then(|v| v.as_str()).unwrap_or("");
        // 早盘量能仍过滤 (防早盘低量能票入盘后)
        if sub == "AuctionVolume" && vol < 5.0 {
            continue;
        }
        // Momentum 评分 = 8.0, 但需 vol_ratio ≥ 阈值
        if vol < 5.0 {
            continue;
        }

        let paper_signal = PaperSignal {
            plan_id: format!(
                "evening-{}-{}",
                cand.code,
                now.format("%Y%m%d%H%M%S%3f")
            ),
            code: cand.code.clone(),
            name: cand.name.clone(),
            direction: Direction::Buy,
            price: cand.push_price,
            quantity: 100,
            virtual_reason: "Momentum".to_string(),
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            account_mode: "Normal".to_string(),
            data_mode: "Full".to_string(),
        };
        match paper_trade::simulate(&paper_signal, cand.push_price, 0.0, 0.0, 0.0) {
            Ok(_) => {
                diesel::sql_query(format!(
                    "UPDATE pushed_stocks SET consumed_at = '{}', consumed_by = 'evening_review', outcome = 'Momentum' \
                     WHERE id = {}",
                    now.format("%Y-%m-%d %H:%M:%S%.3f"),
                    cand.id
                ))
                .execute(&mut conn)
                .map_err(|e| format!("evening update: {}", e))?;
                emitted += 1;
                log::info!(
                    "[evening_review] Momentum 命中 {}({}) → 推 paper_trade",
                    cand.name, cand.code
                );
            }
            Err(e) => {
                log::warn!(
                    "[evening_review] 拒 {}({}): {}",
                    cand.name, cand.code, e
                );
            }
        }
    }

    log::info!(
        "[evening_review] {} 盘后整盘: 消费 {} 条 Momentum 推送",
        today,
        emitted
    );
    Ok(emitted)
}

// ============ Unit tests (≥ 5) ============

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(kind: &str, subkind: &str, vol_ratio: f64) -> Candidate {
        let json = serde_json::json!({
            "vol_ratio": vol_ratio,
            "push_subkind": subkind,
        })
        .to_string();
        Candidate {
            id: 1,
            push_time: Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            push_kind: kind.to_string(),
            code: "000001".to_string(),
            name: "测试".to_string(),
            push_price: 10.0,
            metric_json: json,
            source: "intraday".to_string(),
        }
    }

    #[test]
    fn skips_auction_volume_with_low_vol_ratio() {
        let monitor = IntradayMonitor;
        let c = candidate("P-02", "AuctionVolume", 2.0);
        let s = monitor.evaluate_candidate(&c, Local::now());
        assert!(s.is_none(), "vol_ratio 2.0 < 5.0 应跳过");
    }

    #[test]
    fn accepts_auction_volume_with_high_vol_ratio() {
        let monitor = IntradayMonitor;
        let c = candidate("P-02", "AuctionVolume", 8.0);
        // 传 push_time 自身作为 now, 避开精度差
        let now = c.push_time_parsed();
        let s = monitor.evaluate_candidate(&c, now);
        assert!(s.is_some());
        assert_eq!(s.unwrap().source, "VolumeSurge");
    }

    #[test]
    fn skips_old_pushes_after_1h() {
        let monitor = IntradayMonitor;
        let mut c = candidate("D-01", "NewsCatalyst", 0.0);
        c.push_time = (Local::now() - Duration::hours(2))
            .format("%Y-%m-%d %H:%M:%S%.3f")
            .to_string();
        let s = monitor.evaluate_candidate(&c, Local::now());
        assert!(s.is_none(), "推送 2h 前应跳过");
    }

    #[test]
    fn accepts_pushes_within_1h() {
        let monitor = IntradayMonitor;
        let c = candidate("D-01", "NewsCatalyst", 0.0);
        // 传 push_time 自身作为 now, 避开精度差
        let now = c.push_time_parsed();
        let s = monitor.evaluate_candidate(&c, now);
        assert!(s.is_some());
    }

    #[test]
    fn momentum_gets_highest_score() {
        let monitor = IntradayMonitor;
        let c = candidate("Momentum", "Momentum", 0.0);
        let now = c.push_time_parsed();
        let s = monitor.evaluate_candidate(&c, now).unwrap();
        assert_eq!(s.score, 8.0, "Momentum 应得 8.0 最高分");
    }

    #[test]
    fn breakout_score_is_7_5() {
        let monitor = IntradayMonitor;
        let c = candidate("I-03", "Breakout", 0.0);
        let now = c.push_time_parsed();
        let s = monitor.evaluate_candidate(&c, now).unwrap();
        assert_eq!(s.score, 7.5);
    }

    #[test]
    fn unknown_kind_returns_none() {
        let monitor = IntradayMonitor;
        let c = candidate("UnknownKind", "UnknownSubkind", 0.0);
        let s = monitor.evaluate_candidate(&c, Local::now());
        assert!(s.is_none());
    }

    #[test]
    fn push_kind_label_maps_d01() {
        let c = candidate("D-01", "NewsCatalyst", 0.0);
        assert_eq!(c.push_kind_label(), "NewsCatalyst");
    }

    #[test]
    fn push_kind_label_maps_postclose() {
        let c = candidate("盘后资金", "MainNetInflow", 0.0);
        assert_eq!(c.push_kind_label(), "MainNetInflow");
    }
}
