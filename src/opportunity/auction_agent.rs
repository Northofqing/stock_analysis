//! v10 P0.2 — 09:25 竞价 Agent
//!
//! 设计 (v10 §9 P0.2 + §10.3):
//! - 跑 auction.rs 扫描, 命中规则 → 进虚拟仓 (写 prediction_tracker)
//! - P0.2 范围: 仅"建仓" 环节, **不含** 异动归因 (归因测算移 Phase 6 G5a)
//! - 虚拟仓理由固定枚举 (BR-016), 写入 reason 字段
//! - 样本 < 阈值警告 (Q4=C 动态阈值, 避免 3/3=100% 假胜率)
//!
//! 实施路径:
//! 1. 调用方在 09:25 提供来自真实竞价源的 AuctionResult
//! 2. 严格校验 AuctionResult 后判定是否异常
//! 3. 异常的票调 save_prediction 写盘口, reason = AuctionAnomaly
//! 4. 写完调 check_sample_sufficiency 检查 reason 样本, 不足时 log warn
//!
//! 替代 v9 路径: monitor::prediction::save_prediction 仍走 _legacy (reason=None)

use crate::calendar::{is_auction_now, next_trading_day};
use crate::database::DatabaseManager;
use crate::monitor::auction::AuctionResult;
use crate::opportunity::virtual_reason::{is_sample_sufficient, VirtualReason};
use chrono::Local;
use log::{info, warn};

/// 竞价 Agent 配置 (env 覆盖)
#[derive(Debug, Clone)]
pub struct AuctionAgentConfig {
    /// 涨跌幅阈值 (%), 默认 3.0 (与 auction.rs 现有 classify_auction 一致)
    pub gap_pct_threshold: f64,
    /// 量比阈值, 默认 5.0
    pub vol_ratio_threshold: f64,
    /// 是否 dry-run (仅 log, 不写 DB), 默认 false
    pub dry_run: bool,
    /// 是否强制绕过时段检查 (env `V10_AUCTION_FORCE=1`)
    pub force: bool,
}

impl Default for AuctionAgentConfig {
    fn default() -> Self {
        Self {
            gap_pct_threshold: 3.0,
            vol_ratio_threshold: 5.0,
            dry_run: false,
            force: false,
        }
    }
}

fn validate_config(config: &AuctionAgentConfig) -> Result<(), String> {
    if !config.gap_pct_threshold.is_finite()
        || config.gap_pct_threshold <= 0.0
        || config.gap_pct_threshold > 20.0
    {
        return Err(format!(
            "gap_pct_threshold 必须在 (0, 20]，实际 {}",
            config.gap_pct_threshold
        ));
    }
    if !config.vol_ratio_threshold.is_finite() || config.vol_ratio_threshold <= 0.0 {
        return Err(format!(
            "vol_ratio_threshold 必须为正有限值，实际 {}",
            config.vol_ratio_threshold
        ));
    }
    Ok(())
}

fn validate_auction_result(result: &AuctionResult) -> Result<(), String> {
    if !valid_source_stock_code(&result.code) {
        return Err(format!("股票代码无效: {}", result.code));
    }
    if result.name.trim().is_empty() {
        return Err(format!("{} 股票名称缺失", result.code));
    }
    if !result.gap_pct.is_finite() || result.gap_pct.abs() > 20.0 {
        return Err(format!(
            "{} 竞价涨跌幅无效: {}",
            result.code, result.gap_pct
        ));
    }
    if !result.vol_ratio.is_finite() || result.vol_ratio <= 0.0 {
        return Err(format!(
            "{} 竞价量比无效: {}",
            result.code, result.vol_ratio
        ));
    }
    if !result.match_ratio.is_finite() || !(0.0..=100.0).contains(&result.match_ratio) {
        return Err(format!(
            "{} 匹配量占比无效: {}",
            result.code, result.match_ratio
        ));
    }
    if result.suspected_fake {
        return Err(format!("{} 疑似虚假申报，拒绝写入影子预测", result.code));
    }
    Ok(())
}

fn valid_source_stock_code(code: &str) -> bool {
    if code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()) {
        return true;
    }
    #[cfg(test)]
    {
        code.strip_prefix("TEST_CODE_")
            .is_some_and(|raw| raw.len() == 6 && raw.bytes().all(|byte| byte.is_ascii_digit()))
    }
    #[cfg(not(test))]
    {
        false
    }
}

/// 竞价 Agent 执行结果
#[derive(Debug, Clone, Default)]
pub struct AuctionAgentReport {
    /// 扫描到的总票数
    pub scanned: usize,
    /// 触发的异常票数 (is_abnormal)
    pub abnormal: usize,
    /// 写入虚拟仓的票数 (调 save_prediction 成功)
    pub written: usize,
    /// 样本 < 阈值警告数 (Q4=C)
    pub sample_warnings: usize,
    /// 失败数 (DB 写失败 / 异常)
    pub errors: usize,
    /// 详细记录 (每条 auction result + 写盘结果)
    pub records: Vec<AuctionAgentRecord>,
}

/// 单条 auction result 的处理记录
#[derive(Debug, Clone)]
pub struct AuctionAgentRecord {
    pub code: String,
    pub name: String,
    pub gap_pct: f64,
    pub vol_ratio: f64,
    pub abnormal: bool,
    pub written: bool,
    pub reason: Option<String>,
    pub error: Option<String>,
}

/// 跑竞价 Agent (P0.2 入口)
/// 输入: 扫描结果 (从外部 fetch 传入, 不在本函数内做 HTTP 请求)
/// 输出: 写入盘口 + 报告
///
/// 替代 v9 save_prediction 路径: 同时写 reason 字段 (AuctionAnomaly)
pub fn run_auction_agent(
    results: &[AuctionResult],
    config: &AuctionAgentConfig,
) -> AuctionAgentReport {
    run_auction_agent_for_session(results, config, is_auction_now())
}

fn run_auction_agent_for_session(
    results: &[AuctionResult],
    config: &AuctionAgentConfig,
    auction_now: bool,
) -> AuctionAgentReport {
    let mut report = AuctionAgentReport {
        scanned: results.len(),
        ..Default::default()
    };

    if let Err(error) = validate_config(config) {
        report.errors = 1;
        warn!("[AuctionAgent] 配置无效，拒绝处理: {}", error);
        return report;
    }

    // 时段检查: 不在 09:15-09:25 → 跳过 (除非 cfg.force = true)
    // cfg.force 由调用方 (main 入口) 从 env V10_AUCTION_FORCE 读取, 测试可显式传
    if !auction_now && !config.force {
        warn!("[AuctionAgent] 当前不在竞价时段 (09:15-09:25), 跳过 (设 cfg.force=true 强制)");
        return report;
    }

    info!(
        "[AuctionAgent] 启动: scanned={}, gap≥{}%, vol≥{}, dry_run={}",
        results.len(),
        config.gap_pct_threshold,
        config.vol_ratio_threshold,
        config.dry_run
    );

    for r in results {
        let abnormal = r.is_abnormal(config.gap_pct_threshold, config.vol_ratio_threshold);
        let mut record = AuctionAgentRecord {
            code: r.code.clone(),
            name: r.name.clone(),
            gap_pct: r.gap_pct,
            vol_ratio: r.vol_ratio,
            abnormal,
            written: false,
            reason: None,
            error: None,
        };

        if let Err(error) = validate_auction_result(r) {
            warn!("[AuctionAgent] 无效竞价快照: {}", error);
            record.error = Some(error);
            report.errors += 1;
            report.records.push(record);
            continue;
        }

        if !abnormal {
            report.records.push(record);
            continue;
        }
        report.abnormal += 1;

        // 2. 写盘口 (dry-run 跳过)
        if config.dry_run {
            record.reason = Some(VirtualReason::AuctionAnomaly.to_string());
            record.written = false;
            report.records.push(record);
            continue;
        }

        // 3. 调 save_prediction, reason = AuctionAnomaly
        let today = Local::now().format("%Y-%m-%d").to_string();
        let target_date = next_trading_day(Local::now().date_naive())
            .format("%Y-%m-%d")
            .to_string();
        let direction = if r.gap_pct > 0.0 { "看多" } else { "看空" };
        // 简单 score: 高开 + 量比 加权 (与 v9 类似)
        let score = (r.gap_pct.abs() * 10.0 + r.vol_ratio * 5.0).min(100.0);

        // 调 db.save_prediction, 用 v10 新签名 (带 reason)
        let db_result = DatabaseManager::try_get().map(|db| {
            db.save_prediction(
                &today,
                &target_date,
                Some("auction"), // theme_name
                Some(&r.code),   // stock_code
                direction,
                score,
                Some(&format!(
                    "auction gap={:.2}% vol={:.2}",
                    r.gap_pct, r.vol_ratio
                )),
                Some(VirtualReason::AuctionAnomaly.as_str()), // reason (主)
                None,                                         // reason_secondary
            )
        });

        match db_result {
            Some(Ok(())) => {
                record.written = true;
                record.reason = Some(VirtualReason::AuctionAnomaly.to_string());
                report.written += 1;
            }
            Some(Err(e)) => {
                record.error = Some(format!("DB 写失败: {}", e));
                report.errors += 1;
                warn!("[AuctionAgent] 写盘口失败: {} ({})", r.code, e);
            }
            None => {
                record.error = Some("DB 未初始化".to_string());
                report.errors += 1;
                warn!("[AuctionAgent] DB 未初始化, 跳过写盘口");
            }
        }
        report.records.push(record);
    }

    // 4. 样本 < 阈值检查 (Q4=C, 警告而非阻断)
    // review #14: try_get() 不 panic, None 路径走 DB 未初始化分支
    let total = report.written;
    if total > 0 {
        let sample_check = DatabaseManager::try_get().map(|db| {
            let reason_count = db
                .count_predictions_by_reason("AuctionAnomaly")
                .map_err(|error| format!("统计 AuctionAnomaly 样本失败: {error}"))?
                as usize;
            let total_pred = db
                .count_predictions()
                .map_err(|error| format!("统计 prediction_tracker 总样本失败: {error}"))?
                as usize;
            Ok::<_, String>((reason_count, total_pred))
        });
        match sample_check {
            Some(Ok((reason_count, total_pred))) => {
                if !is_sample_sufficient(reason_count, total_pred) {
                    report.sample_warnings += 1;
                    warn!(
                        "[AuctionAgent] 样本不足警告: AuctionAnomaly 当前 {} 条, 总推送 {} 条, 阈值需 ≥ {}. 不进胜率表, 等数据增长",
                        reason_count,
                        total_pred,
                        crate::opportunity::virtual_reason::compute_sample_threshold(total_pred)
                    );
                }
            }
            Some(Err(error)) => {
                report.errors += 1;
                warn!("[AuctionAgent] {}", error);
            }
            None => {
                report.errors += 1;
                warn!("[AuctionAgent] DB 未初始化, 跳过样本检查");
            }
        }
    }

    info!(
        "[AuctionAgent] 完成: scanned={}, abnormal={}, written={}, warnings={}, errors={}",
        report.scanned, report.abnormal, report.written, report.sample_warnings, report.errors
    );

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::auction::AuctionResult;

    fn make_result(code: &str, gap: f64, vol: f64, fake: bool) -> AuctionResult {
        AuctionResult {
            code: code.into(),
            name: format!("测试{}", code),
            gap_pct: gap,
            vol_ratio: vol,
            match_ratio: 80.0,
            suspected_fake: fake,
        }
    }

    #[test]
    fn test_config_default_values() {
        let c = AuctionAgentConfig::default();
        assert_eq!(c.gap_pct_threshold, 3.0);
        assert_eq!(c.vol_ratio_threshold, 5.0);
        assert!(!c.dry_run);
        assert!(!c.force);
    }

    #[test]
    fn test_run_auction_agent_filters_normal() {
        // 显式注入时段外状态，避免测试结果依赖真实墙钟。
        let results = vec![
            make_result("TEST_CODE_000001", 5.0, 6.0, false), // 异常
            make_result("TEST_CODE_000002", 1.0, 2.0, false), // 正常
        ];
        let cfg = AuctionAgentConfig {
            gap_pct_threshold: 3.0,
            vol_ratio_threshold: 5.0,
            dry_run: false,
            force: false,
        };
        let report = run_auction_agent_for_session(&results, &cfg, false);
        assert_eq!(report.scanned, 2);
        assert_eq!(report.abnormal, 0);
        assert_eq!(report.written, 0);
    }

    #[test]
    fn test_run_auction_agent_force_mode_filters_correctly() {
        // cfg.force = true 强制跑 (绕时段检查) + dry_run=true 不写 DB
        let results = vec![
            make_result("TEST_CODE_000001", 5.0, 6.0, false), // 异常 (高开 + 量比足)
            make_result("TEST_CODE_000002", 1.0, 2.0, false), // 正常
            make_result("TEST_CODE_000003", -5.0, 6.0, false), // 异常 (低开 + 量比足)
        ];
        let cfg = AuctionAgentConfig {
            gap_pct_threshold: 3.0,
            vol_ratio_threshold: 5.0,
            dry_run: true,
            force: true,
        };
        let report = run_auction_agent(&results, &cfg);
        assert_eq!(report.scanned, 3);
        assert_eq!(
            report.abnormal, 2,
            "应识别 2 个异常 (000001 高开 + 000003 低开)"
        );
        assert_eq!(report.written, 0, "dry_run 不写 DB");
        let abnormal_records: Vec<&AuctionAgentRecord> =
            report.records.iter().filter(|r| r.abnormal).collect();
        assert_eq!(abnormal_records.len(), 2);
        for r in &abnormal_records {
            assert_eq!(r.reason.as_deref(), Some("AuctionAnomaly"));
            assert!(!r.written);
        }
    }

    #[test]
    fn test_run_auction_agent_empty_results() {
        let cfg = AuctionAgentConfig {
            gap_pct_threshold: 3.0,
            vol_ratio_threshold: 5.0,
            dry_run: false,
            force: true,
        };
        let report = run_auction_agent(&[], &cfg);
        assert_eq!(report.scanned, 0);
        assert_eq!(report.abnormal, 0);
        assert_eq!(report.written, 0);
    }

    #[test]
    fn test_suspected_fake_snapshot_is_rejected_before_prediction() {
        let results = vec![make_result("TEST_CODE_000001", 5.0, 6.0, true)];
        let cfg = AuctionAgentConfig {
            dry_run: true,
            force: true,
            ..Default::default()
        };
        let report = run_auction_agent(&results, &cfg);
        assert_eq!(report.written, 0);
        assert_eq!(report.errors, 1);
        assert!(report.records[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("虚假申报")));
    }

    #[test]
    fn test_invalid_snapshot_value_is_rejected() {
        let results = vec![make_result("TEST_CODE_000001", f64::NAN, 6.0, false)];
        let cfg = AuctionAgentConfig {
            dry_run: true,
            force: true,
            ..Default::default()
        };
        let report = run_auction_agent(&results, &cfg);
        assert_eq!(report.written, 0);
        assert_eq!(report.errors, 1);
        assert!(report.records[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("涨跌幅无效")));
    }

    #[test]
    fn test_invalid_threshold_rejects_whole_batch() {
        let results = vec![make_result("TEST_CODE_000001", 5.0, 6.0, false)];
        let cfg = AuctionAgentConfig {
            gap_pct_threshold: 25.0,
            dry_run: true,
            force: true,
            ..Default::default()
        };
        let report = run_auction_agent(&results, &cfg);
        assert_eq!(report.written, 0);
        assert_eq!(report.errors, 1);
        assert!(report.records.is_empty());
    }
}
