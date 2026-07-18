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
use crate::strategy::v16_4::{
    AuctionAnomalyStrategy, BreakoutStrategy, LLMSelectStrategy, MainNetInflowStrategy,
    MomentumStrategy, NewsCatalystStrategy, SectorLeaderStrategy, Strategy, StrategyInput,
    StrategyOutput, VolumeSurgeStrategy,
};
use crate::trading::paper_trade::{self, Direction, PaperSignal};
use chrono::{DateTime, Duration, Local, NaiveDate};
use diesel::prelude::*;

/// 4 步过滤阈值常量
const PUSH_AGE_MAX_HOURS: i64 = 1;
const DECISION_SCORE_THRESHOLD: f64 = 6.0;
const VOLUME_SURGE_THRESHOLD: f64 = 5.0;
const MAX_CANDIDATES: i64 = 50;
/// 盘后整盘扫候选上限 (review fix: 消除魔数, 与 MAX_CANDIDATES 区分)
const MAX_EVENING_CANDIDATES: i64 = 100;

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
}

impl Candidate {
    fn push_time_parsed(&self) -> Result<DateTime<Local>, String> {
        // 字符串无时区, 用 NaiveDateTime 解析 + assume_local (避免 UTC→+08:00 偏移 8h)
        chrono::NaiveDateTime::parse_from_str(&self.push_time, "%Y-%m-%d %H:%M:%S%.3f")
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(&self.push_time, "%Y-%m-%d %H:%M:%S")
            })
            .map_err(|error| format!("{} push_time 解析失败: {error}", self.code))?
            .and_local_timezone(Local)
            .single()
            .ok_or_else(|| format!("{} push_time 本地时区不唯一: {}", self.code, self.push_time))
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

        // 1. 扫推送票池 (review fix Issue #4: 参数化绑定替代 format! 拼接)
        let candidates: Vec<Candidate> = diesel::sql_query(
            "SELECT id, push_time, push_kind, code, name, push_price, metric_json \
             FROM pushed_stocks \
             WHERE consumed_at IS NULL AND push_time < ? \
             AND push_time > ? \
             ORDER BY push_time DESC LIMIT ?",
        )
        .bind::<diesel::sql_types::Text, _>(now.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .bind::<diesel::sql_types::Text, _>(cutoff.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .bind::<diesel::sql_types::BigInt, _>(MAX_CANDIDATES)
        .load::<Candidate>(&mut conn)
        .map_err(|e| format!("query pushed_stocks: {}", e))?;

        let mut emitted = 0;
        let candidate_count = candidates.len();

        // 2. 4 步过滤
        for cand in candidates {
            let signal = match self.evaluate_candidate(&cand, now) {
                Ok(signal) => signal,
                Err(error) => {
                    log::error!(
                        "[intraday_monitor][BR-098] 拒绝坏候选 {}({}): {}",
                        cand.name,
                        cand.code,
                        error
                    );
                    continue;
                }
            };
            if let Some(signal) = signal {
                if signal.score < DECISION_SCORE_THRESHOLD {
                    log::debug!(
                        "[intraday_monitor] 跳过 {}({}): 综合分 {:.1} < 阈值 {:.1}",
                        cand.name,
                        cand.code,
                        signal.score,
                        DECISION_SCORE_THRESHOLD
                    );
                    continue;
                }

                // 3. risk_adapter 4 项检查 (复用 Commit 1)
                let execution_quote = match crate::broker::execution_quote(&cand.code) {
                    Ok(quote) => quote,
                    Err(error) => {
                        log::warn!(
                            "[intraday_monitor] 跳过 {}({}): 实时报价不可用: {}",
                            cand.name,
                            cand.code,
                            error
                        );
                        continue;
                    }
                };
                let paper_signal = PaperSignal {
                    plan_id: format!("intraday-{}-{}", cand.code, now.format("%Y%m%d%H%M%S%3f")),
                    code: cand.code.clone(),
                    name: cand.name.clone(),
                    direction: Direction::Buy,
                    price: execution_quote.price,
                    quantity: 100,
                    virtual_reason: signal.source.to_string(),
                    is_limit_up: false,
                    is_limit_down: false,
                    is_suspended: false,
                    limit_up_price: Some(execution_quote.limit_up_price),
                    limit_down_price: Some(execution_quote.limit_down_price),
                    secondary_confirmed: false,
                    quote_observed_at: execution_quote.observed_at,
                    account_mode: "Normal".to_string(),
                    data_mode: "Full".to_string(),
                };
                let (cash, total, pos_pct) =
                    match paper_trade::portfolio_state(&cand.code, execution_quote.price) {
                        Ok(state) => state,
                        Err(error) => {
                            log::warn!(
                                "[intraday_monitor] 跳过 {}({}): 账户快照不可用: {}",
                                cand.name,
                                cand.code,
                                error
                            );
                            continue;
                        }
                    };
                match paper_trade::simulate(
                    &paper_signal,
                    execution_quote.price,
                    cash,
                    total,
                    pos_pct,
                ) {
                    Ok(_) => {
                        // 4. 标记 consumed (review fix Issue #4: 参数化绑定)
                        let outcome = signal.source;
                        diesel::sql_query(
                            "UPDATE pushed_stocks SET consumed_at = ?, consumed_by = 'intraday_monitor', outcome = ? \
                             WHERE id = ?",
                        )
                        .bind::<diesel::sql_types::Text, _>(now.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
                        .bind::<diesel::sql_types::Text, _>(outcome)
                        .bind::<diesel::sql_types::Integer, _>(cand.id)
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
                            cand.name,
                            cand.code,
                            e
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
    fn evaluate_candidate(
        &self,
        cand: &Candidate,
        now: DateTime<Local>,
    ) -> Result<Option<Signal>, String> {
        let push_time = cand.push_time_parsed()?;
        if now < push_time {
            return Err(format!(
                "push_time 位于未来: push_time={push_time} now={now}"
            ));
        }
        let push_age = now - push_time;
        if push_age > Duration::hours(PUSH_AGE_MAX_HOURS) {
            return Ok(None);
        }

        let label = cand.push_kind_label();
        let Some(output) = Self::score_candidate(cand)? else {
            return Ok(None);
        };

        Ok(Some(Signal {
            source: label,
            code: cand.code.clone(),
            score: output.score,
        }))
    }

    fn score_candidate(cand: &Candidate) -> Result<Option<StrategyOutput>, String> {
        let label = cand.push_kind_label();
        if label == "Unknown" {
            return Ok(None);
        }

        let metrics = crate::strategy::v16_4::_helpers::parse(
            &cand.metric_json,
            &cand.code,
            cand.push_price,
        )?;
        if metrics.push_subkind.as_deref() == Some("AuctionVolume")
            && metrics
                .vol_ratio
                .is_some_and(|value| value < VOLUME_SURGE_THRESHOLD)
        {
            return Ok(None);
        }
        let require_number = |value: Option<f64>, field: &str| {
            value.ok_or_else(|| format!("{} 缺少必需指标 {field}", cand.code))
        };
        match label {
            "VolumeSurge" | "AuctionAnomaly" => {
                require_number(metrics.vol_ratio, "vol_ratio")?;
            }
            "Breakout" | "Momentum" => {
                require_number(metrics.vol_ratio, "vol_ratio")?;
                require_number(metrics.price_chg_pct, "price_chg_pct")?;
            }
            "MainNetInflow" => {
                require_number(metrics.main_net_yi, "main_net_yi")?;
                require_number(metrics.price_chg_pct, "price_chg_pct")?;
            }
            "SectorLeader" => {
                require_number(metrics.price_chg_pct, "price_chg_pct")?;
                if metrics.sector.is_none() {
                    return Err(format!("{} 缺少必需指标 sector", cand.code));
                }
            }
            "LLMSelect" => {
                let value: serde_json::Value = serde_json::from_str(&cand.metric_json)
                    .map_err(|error| format!("{} metric_json 解析失败: {error}", cand.code))?;
                let confidence = value
                    .get("llm_confidence")
                    .and_then(serde_json::Value::as_f64)
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| format!("{} 缺少有效 llm_confidence", cand.code))?;
                if !(0.0..=1.0).contains(&confidence) {
                    return Err(format!("{} llm_confidence 越界: {confidence}", cand.code));
                }
                value
                    .get("llm_verdict")
                    .and_then(serde_json::Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| format!("{} 缺少有效 llm_verdict", cand.code))?;
            }
            "NewsCatalyst" => {}
            _ => return Ok(None),
        }

        let input = StrategyInput {
            code: cand.code.clone(),
            push_price: cand.push_price,
            metric_json: cand.metric_json.clone(),
            push_kind: cand.push_kind.clone(),
            now: Local::now(),
        };
        let output = match label {
            "SectorLeader" => SectorLeaderStrategy.score(&input),
            "Breakout" => BreakoutStrategy.score(&input),
            "VolumeSurge" => VolumeSurgeStrategy.score(&input),
            "AuctionAnomaly" => AuctionAnomalyStrategy.score(&input),
            "MainNetInflow" => MainNetInflowStrategy.score(&input),
            "NewsCatalyst" => NewsCatalystStrategy.score(&input),
            "LLMSelect" => LLMSelectStrategy.score(&input),
            "Momentum" => MomentumStrategy.score(&input),
            _ => None,
        };
        Ok(output)
    }
}

/// review fix Issue #7: evening_review 当日防重入 (30s tick 在 15:30 分钟内会触发 2 次)
/// Fix review (MEDIUM): 失败路径 5 分钟 debounce, 避免 DB 锁时 30s tick × N 次失败
static EVENING_LAST_RUN: std::sync::Mutex<Option<NaiveDate>> = std::sync::Mutex::new(None);
static EVENING_LAST_FAIL: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>> =
    std::sync::Mutex::new(None);

/// 盘后 15:30 整盘扫 (R5) — 复用 evaluate_candidate 评分, 跑 Momentum 整合
pub fn evening_review(today: NaiveDate) -> Result<usize, String> {
    {
        let last = EVENING_LAST_RUN.lock().unwrap_or_else(|e| e.into_inner());
        if *last == Some(today) {
            log::info!("[evening_review] {} 已跑过, 跳过 (当日防重入)", today);
            return Ok(0);
        }
    }
    {
        // Fix MEDIUM: 失败 5 分钟内不重试 (DB 锁时 30s tick 反复失败)
        let now = chrono::Utc::now();
        let last_fail = EVENING_LAST_FAIL.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = *last_fail {
            if now - t < chrono::Duration::minutes(5) {
                log::warn!("[evening_review] 上次失败 5 分钟内, 跳过 (防重试风暴)");
                return Err("evening_review 失败 5 分钟 debounce 中".to_string());
            }
        }
    }
    let now = Local::now();
    let cutoff = today
        .and_hms_opt(15, 30, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;

    // review fix Issue #4: 参数化绑定
    let candidates: Vec<Candidate> = diesel::sql_query(
        "SELECT id, push_time, push_kind, code, name, push_price, metric_json \
         FROM pushed_stocks \
         WHERE consumed_at IS NULL AND push_time <= ? \
         AND date(push_time) = ? \
         ORDER BY push_time DESC LIMIT ?",
    )
    .bind::<diesel::sql_types::Text, _>(cutoff.format("%Y-%m-%d %H:%M:%S").to_string())
    .bind::<diesel::sql_types::Text, _>(today.to_string())
    .bind::<diesel::sql_types::BigInt, _>(MAX_EVENING_CANDIDATES)
    .load::<Candidate>(&mut conn)
    .map_err(|e| format!("evening query: {}", e))?;

    let mut emitted = 0;

    for cand in candidates {
        // Fix 5: R5 收紧: 仅 Momentum + score ≥ 8, 不限 1h 时间窗 (全天整盘扫)
        if cand.push_kind != "Momentum" {
            continue;
        }
        let scored = match IntradayMonitor::score_candidate(&cand) {
            Ok(Some(scored)) => scored,
            Ok(None) => continue,
            Err(error) => {
                log::error!(
                    "[evening_review][BR-098] 拒绝坏候选 {}({}): {}",
                    cand.name,
                    cand.code,
                    error
                );
                continue;
            }
        };
        if scored.score < 8.0 {
            continue;
        }

        let execution_quote = match crate::broker::execution_quote(&cand.code) {
            Ok(quote) => quote,
            Err(error) => {
                log::warn!(
                    "[evening_review] 跳过 {}({}): 实时报价不可用: {}",
                    cand.name,
                    cand.code,
                    error
                );
                continue;
            }
        };
        let paper_signal = PaperSignal {
            plan_id: format!("evening-{}-{}", cand.code, now.format("%Y%m%d%H%M%S%3f")),
            code: cand.code.clone(),
            name: cand.name.clone(),
            direction: Direction::Buy,
            price: execution_quote.price,
            quantity: 100,
            virtual_reason: "Momentum".to_string(),
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            limit_up_price: Some(execution_quote.limit_up_price),
            limit_down_price: Some(execution_quote.limit_down_price),
            secondary_confirmed: false,
            quote_observed_at: execution_quote.observed_at,
            account_mode: "Normal".to_string(),
            data_mode: "Full".to_string(),
        };
        let (cash, total, pos_pct) =
            match paper_trade::portfolio_state(&cand.code, execution_quote.price) {
                Ok(state) => state,
                Err(error) => {
                    log::warn!(
                        "[evening_review] 跳过 {}({}): 账户快照不可用: {}",
                        cand.name,
                        cand.code,
                        error
                    );
                    continue;
                }
            };
        match paper_trade::simulate(&paper_signal, execution_quote.price, cash, total, pos_pct) {
            Ok(_) => {
                // review fix Issue #4: 参数化绑定
                diesel::sql_query(
                    "UPDATE pushed_stocks SET consumed_at = ?, consumed_by = 'evening_review', outcome = 'Momentum' \
                     WHERE id = ?",
                )
                .bind::<diesel::sql_types::Text, _>(now.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
                .bind::<diesel::sql_types::Integer, _>(cand.id)
                .execute(&mut conn)
                .map_err(|e| format!("evening update: {}", e))?;
                emitted += 1;
                log::info!(
                    "[evening_review] Momentum 命中 {}({}) → 推 paper_trade",
                    cand.name,
                    cand.code
                );
            }
            Err(e) => {
                log::warn!("[evening_review] 拒 {}({}): {}", cand.name, cand.code, e);
            }
        }
    }

    log::info!(
        "[evening_review] {} 盘后整盘: 消费 {} 条 Momentum 推送",
        today,
        emitted
    );
    // 成功跑完才记 date (失败可重试)
    *EVENING_LAST_RUN.lock().unwrap_or_else(|e| e.into_inner()) = Some(today);
    Ok(emitted)
}

// ============ Unit tests (≥ 5) ============

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(kind: &str, subkind: &str, vol_ratio: f64) -> Candidate {
        let json = serde_json::json!({
            "vol_ratio": vol_ratio,
            "price_chg_pct": 0.1,
            "main_net_yi": 1.0,
            "sector": "测试板块",
            "push_subkind": subkind,
        })
        .to_string();
        Candidate {
            id: 1,
            push_time: Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            push_kind: kind.to_string(),
            code: "TEST_CODE_000001".to_string(),
            name: "测试".to_string(),
            push_price: 10.0,
            metric_json: json,
        }
    }

    #[test]
    fn skips_auction_volume_with_low_vol_ratio() {
        let monitor = IntradayMonitor;
        let c = candidate("P-02", "AuctionVolume", 2.0);
        let s = monitor
            .evaluate_candidate(&c, Local::now())
            .expect("valid candidate");
        assert!(s.is_none(), "vol_ratio 2.0 < 5.0 应跳过");
    }

    #[test]
    fn accepts_auction_volume_with_high_vol_ratio() {
        let monitor = IntradayMonitor;
        let c = candidate("P-02", "AuctionVolume", 8.0);
        // 传 push_time 自身作为 now, 避开精度差
        let now = c.push_time_parsed().expect("valid time");
        let s = monitor
            .evaluate_candidate(&c, now)
            .expect("valid candidate");
        assert!(s.is_some());
        assert_eq!(s.expect("signal").source, "VolumeSurge");
    }

    #[test]
    fn skips_old_pushes_after_1h() {
        let monitor = IntradayMonitor;
        let mut c = candidate("D-01", "NewsCatalyst", 0.0);
        c.push_time = (Local::now() - Duration::hours(2))
            .format("%Y-%m-%d %H:%M:%S%.3f")
            .to_string();
        let s = monitor
            .evaluate_candidate(&c, Local::now())
            .expect("valid candidate");
        assert!(s.is_none(), "推送 2h 前应跳过");
    }

    #[test]
    fn accepts_pushes_within_1h() {
        let monitor = IntradayMonitor;
        let c = candidate("D-01", "NewsCatalyst", 0.0);
        // 传 push_time 自身作为 now, 避开精度差
        let now = c.push_time_parsed().expect("valid time");
        let s = monitor
            .evaluate_candidate(&c, now)
            .expect("valid candidate");
        assert!(s.is_some());
    }

    #[test]
    fn momentum_gets_highest_score() {
        let monitor = IntradayMonitor;
        let c = candidate("Momentum", "Momentum", 6.0);
        let now = c.push_time_parsed().expect("valid time");
        let s = monitor
            .evaluate_candidate(&c, now)
            .expect("valid candidate")
            .expect("signal");
        assert!((s.score - 8.02).abs() < 1e-9, "Momentum 应使用真实指标评分");
    }

    #[test]
    fn breakout_score_is_7_5() {
        let monitor = IntradayMonitor;
        let mut c = candidate("I-03", "Breakout", 4.0);
        c.metric_json = serde_json::json!({
            "vol_ratio": 4.0,
            "price_chg_pct": 5.0,
            "push_subkind": "Breakout"
        })
        .to_string();
        let now = c.push_time_parsed().expect("valid time");
        let s = monitor
            .evaluate_candidate(&c, now)
            .expect("valid candidate")
            .expect("signal");
        assert_eq!(s.score, 7.5);
    }

    #[test]
    fn unknown_kind_returns_none() {
        let monitor = IntradayMonitor;
        let c = candidate("UnknownKind", "UnknownSubkind", 0.0);
        let s = monitor
            .evaluate_candidate(&c, Local::now())
            .expect("valid candidate");
        assert!(s.is_none());
    }

    #[test]
    fn rejects_invalid_metric_json_explicitly() {
        let monitor = IntradayMonitor;
        let mut c = candidate("P-02", "AuctionVolume", 8.0);
        c.metric_json = "not-json".to_string();
        let error = monitor
            .evaluate_candidate(&c, Local::now())
            .expect_err("invalid JSON must fail");
        assert!(error.contains("metric_json 解析失败"));
    }

    #[test]
    fn rejects_future_push_time_explicitly() {
        let monitor = IntradayMonitor;
        let mut c = candidate("D-01", "NewsCatalyst", 1.0);
        c.push_time = (Local::now() + Duration::minutes(5))
            .format("%Y-%m-%d %H:%M:%S%.3f")
            .to_string();
        let error = monitor
            .evaluate_candidate(&c, Local::now())
            .expect_err("future row must fail");
        assert!(error.contains("位于未来"));
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
