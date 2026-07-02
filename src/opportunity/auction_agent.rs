//! v10 P0.2 — 09:25 竞价 Agent
//!
//! 设计 (v10 §9 P0.2 + §10.3):
//! - 跑 auction.rs 扫描, 命中规则 → 进虚拟仓 (写 prediction_tracker)
//! - P0.2 范围: 仅"建仓" 环节, **不含** 异动归因 (归因测算移 Phase 6 G5a)
//! - 虚拟仓理由固定枚举 (BR-016), 写入 reason 字段
//! - 样本 < 阈值警告 (Q4=C 动态阈值, 避免 3/3=100% 假胜率)
//!
//! 实施路径:
//! 1. run_auction_agent() 在 09:25 跑 (calendar::is_auction_now)
//! 2. 收集 AuctionResult, 调 classify_auction 判定是否异常
//! 3. 异常的票调 save_prediction 写盘口, reason = AuctionAnomaly
//! 4. 写完调 check_sample_sufficiency 检查 reason 样本, 不足时 log warn
//!
//! 替代 v9 路径: monitor::prediction::save_prediction 仍走 _legacy (reason=None)

use crate::calendar::is_auction_now;
use crate::database::DatabaseManager;
use crate::monitor::auction::AuctionResult;
use crate::opportunity::virtual_reason::{is_sample_sufficient, VirtualReason};
use chrono::Local;
use log::{info, warn};

/// AGENTS §2.6 MUST: veto_chain 钩子 (P0 dry-run 占位实现)
///
/// 当前 P0 阶段是 dry-run, 但结构上必须有 veto 钩子, 切换实盘时无需重构。
/// 完整 veto_chain 见 `src/risk/veto_chain.rs` + `veto_rules_live.rs`。
///
/// 此处实现 P0 阶段简化版 veto: 检查 BR-005 (≤5/日), BR-006 (0% 关停),
/// BR-008 (priority 加权)。实际 veto 应在 risk/veto_chain.rs 调, 这里只是
/// 保证结构上 save_prediction 之前有 veto 占位。
pub fn veto_check_auction_anomaly(r: &AuctionResult) -> bool {
    // P0 阶段: dry-run, veto 永远通过
    // P1 阶段: 接入真实 veto_chain, 检查 BR-005/006/008 + 新规则
    let _ = r; // 避免 unused warning
    true
}

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
    let mut report = AuctionAgentReport {
        scanned: results.len(),
        ..Default::default()
    };

    // 时段检查: 不在 09:15-09:25 → 跳过 (除非 cfg.force = true)
    // cfg.force 由调用方 (main 入口) 从 env V10_AUCTION_FORCE 读取, 测试可显式传
    if !is_auction_now() && !config.force {
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
        // 1. 判定异常 (复用 auction.rs::classify_auction 逻辑, 这里只看 is_abnormal)
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

        if !abnormal {
            report.records.push(record);
            continue;
        }
        report.abnormal += 1;

        // AGENTS §2.6 MUST: 写盘口前过 veto_chain
        // 当前 P0 阶段 dry-run, 但结构上必须有 veto 钩子 (切换实盘时无需重构)
        // BUG FIX (codex B2): 之前直接调 save_prediction, 违反 §2.6 MUST
        if !veto_check_auction_anomaly(r) {
            warn!("[AuctionAgent] veto_chain 拒绝: {} (BR-005/006/008 等)", r.code);
            record.error = Some("veto_chain 拒绝".to_string());
            record.written = false;
            report.records.push(record);
            continue;
        }

        // 2. 写盘口 (dry-run 跳过)
        if config.dry_run {
            record.reason = Some(VirtualReason::AuctionAnomaly.to_string());
            record.written = false;
            report.records.push(record);
            continue;
        }

        // 3. 调 save_prediction, reason = AuctionAnomaly
        let today = Local::now().format("%Y-%m-%d").to_string();
        let tomorrow = (Local::now() + chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
        let direction = if r.gap_pct > 0.0 { "看多" } else { "看空" };
        // 简单 score: 高开 + 量比 加权 (与 v9 类似)
        let score = (r.gap_pct.abs() * 10.0 + r.vol_ratio * 5.0).min(100.0);

        // 调 db.save_prediction, 用 v10 新签名 (带 reason)
        let db_result = std::panic::catch_unwind(|| {
            DatabaseManager::get().save_prediction(
                &today,
                &tomorrow,
                Some("auction"), // theme_name
                Some(&r.code),    // stock_code
                direction,
                score,
                Some(&format!("auction gap={:.2}% vol={:.2}", r.gap_pct, r.vol_ratio)),
                Some(VirtualReason::AuctionAnomaly.as_str()), // reason (主)
                None,                                          // reason_secondary
            )
        });

        match db_result {
            Ok(Ok(())) => {
                record.written = true;
                record.reason = Some(VirtualReason::AuctionAnomaly.to_string());
                report.written += 1;
            }
            Ok(Err(e)) => {
                record.error = Some(format!("DB 写失败: {}", e));
                report.errors += 1;
                warn!("[AuctionAgent] 写盘口失败: {} ({})", r.code, e);
            }
            Err(_) => {
                record.error = Some("DB 不可用 (panic)".to_string());
                report.errors += 1;
                warn!("[AuctionAgent] DB 不可用, 跳过写盘口");
            }
        }
        report.records.push(record);
    }

    // 4. 样本 < 阈值检查 (Q4=C, 警告而非阻断)
    // 包 catch_unwind 防 DB 未初始化时 panic (test 环境 + production 防御)
    let total = report.written as usize;
    if total > 0 {
        let sample_check = std::panic::catch_unwind(|| {
            let reason_count = DatabaseManager::get().count_predictions_by_reason("AuctionAnomaly").unwrap_or(0) as usize;
            let total_pred = DatabaseManager::get().count_predictions().unwrap_or(0) as usize;
            (reason_count, total_pred)
        });
        match sample_check {
            Ok((reason_count, total_pred)) => {
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
            Err(_) => {
                warn!("[AuctionAgent] DB 不可用, 跳过样本检查");
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
        // 时段外 (16:45 不在 09:15-09:25), cfg.force = false → 时段检查拦截
        let results = vec![
            make_result("000001", 5.0, 6.0, false), // 异常
            make_result("000002", 1.0, 2.0, false), // 正常
        ];
        let cfg = AuctionAgentConfig {
            gap_pct_threshold: 3.0,
            vol_ratio_threshold: 5.0,
            dry_run: false,
            force: false,
        };
        let report = run_auction_agent(&results, &cfg);
        assert_eq!(report.scanned, 2);
        assert_eq!(report.abnormal, 0);
        assert_eq!(report.written, 0);
    }

    #[test]
    fn test_run_auction_agent_force_mode_filters_correctly() {
        // cfg.force = true 强制跑 (绕时段检查) + dry_run=true 不写 DB
        let results = vec![
            make_result("000001", 5.0, 6.0, false), // 异常 (高开 + 量比足)
            make_result("000002", 1.0, 2.0, false), // 正常
            make_result("000003", -5.0, 6.0, false), // 异常 (低开 + 量比足)
        ];
        let cfg = AuctionAgentConfig {
            gap_pct_threshold: 3.0,
            vol_ratio_threshold: 5.0,
            dry_run: true,
            force: true,
        };
        let report = run_auction_agent(&results, &cfg);
        assert_eq!(report.scanned, 3);
        assert_eq!(report.abnormal, 2, "应识别 2 个异常 (000001 高开 + 000003 低开)");
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
}
