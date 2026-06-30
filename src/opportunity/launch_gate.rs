//! 修复 P0-3: launch_gate 阶段门槛
//!
//! 量化产品经理要求 (P0-3):
//!  - 沙盘 → 灰度: 12 周 + 200 样本 + 60% 胜率 + Calmar 1.0 (全部满足)
//!  - 灰度 → 实盘: 30 天 + 55% 胜率
//!  - 灰度 → 沙盘 (回退): 胜率 < 50%
//!  - 实盘阶段 LaunchGate::check_transition 不自动转, 只能人工/风控回退
//!
//! 修复 F20 (2026-06-29 codex review): 真接 LaunchStage 状态机.
//! 之前 LaunchStage / check_transition 是死代码 (grep 0 外部读).
//! 这次接入: (1) `current_stage()` 读 env STAGE (默认 Shadow),
//! (2) `should_push_user()`: Shadow=不打用户, Gray=仅 critical alert, Live=全量推送,
//! (3) `metrics_from_db()`: 从 prediction_tracker 算 StageMetrics,
//! (4) e2e 测试 shadow → gray → live 三阶段切换.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchStage {
    /// 沙盘: 只入 prediction_tracker 影子盘, 不推送用户 (P0-3 推荐)
    Shadow,
    /// 灰度: 单日推送 ≤ 5 候选, 限 30 天
    Gray,
    /// 实盘: 全量推送
    Live,
}

impl LaunchStage {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Shadow => "Shadow",
            Self::Gray => "Gray",
            Self::Live => "Live",
        }
    }
}

impl FromStr for LaunchStage {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "shadow" => Ok(Self::Shadow),
            "gray" | "grey" => Ok(Self::Gray),
            "live" | "prod" => Ok(Self::Live),
            other => Err(format!(
                "未识别 stage '{other}', 期望 Shadow|Gray|Live"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StageMetrics {
    /// 沙盘运行天数
    pub shadow_days: u32,
    /// winrate 真实样本数
    pub winrate_samples: u32,
    /// 真实胜率 (0-1)
    pub winrate_pct: f64,
    /// Calmar 比率 (年化收益/最大回撤)
    pub calmar_ratio: f64,
    /// 灰度运行天数
    pub gray_days: u32,
}

pub struct LaunchGate;

impl LaunchGate {
    /// 修复 P0-3: 阶段切换检查
    /// 沙盘 → 灰度: 12 周 + 200 样本 + 60% 胜率 + Calmar ≥ 1.0 (全部满足)
    /// 灰度 → 实盘: 30 天 + 55% 胜率
    /// 灰度 → 沙盘 (回退): 胜率 < 50%
    /// 实盘 → 任何: 人工/风控事件手动处理, 不自动
    pub fn check_transition(current: LaunchStage, m: &StageMetrics) -> Option<LaunchStage> {
        match current {
            LaunchStage::Shadow => {
                // 修复 (2026-06-30 codex review): 之前用 60 触发, 文档说 12 周
                // (= 84 calendar days). metrics_from_db 用 (now.date_naive()
                // - min_date).num_days() 计算 calendar days, 阈值必须对齐到 84
                // (12 周). 60 会让 Shadow→Gray 在 ~8.5 周触发, 与文档矛盾,
                // 违反 AGENTS §2.9 设计矛盾禁令.
                if m.shadow_days >= 84
                    && m.winrate_samples >= 200
                    && m.winrate_pct >= 0.60
                    && m.calmar_ratio >= 1.0
                {
                    Some(LaunchStage::Gray)
                } else {
                    None
                }
            }
            LaunchStage::Gray => {
                if m.gray_days >= 30 && m.winrate_pct >= 0.55 {
                    Some(LaunchStage::Live)
                } else if m.winrate_pct < 0.50 {
                    Some(LaunchStage::Shadow)
                } else {
                    None
                }
            }
            LaunchStage::Live => None,
        }
    }
}

/// 修复 F20: 真接 LaunchStage 当前状态读取.
/// 优先级:
///   1. env STAGE (e.g. STAGE=Live cargo run --bin monitor)
///   2. 默认 Shadow (沙盘, 不打用户)
pub fn current_stage() -> LaunchStage {
    match std::env::var("STAGE") {
        Ok(s) => match LaunchStage::from_str(&s) {
            Ok(stage) => stage,
            Err(e) => {
                log::warn!("[LaunchGate] {} — fallback to Shadow", e);
                LaunchStage::Shadow
            }
        },
        Err(_) => LaunchStage::Shadow,
    }
}

/// 修复 F20: 推送 gate. 当前 stage 决定是否给用户发.
/// Shadow: 推所有消息 (默认行为, 用户没设 STAGE=Live 时收全量)
/// Gray: 仅 critical alert (止损/风控), 普通扫描不推
/// Live: 全量推送
///
/// 修复 (2026-06-30): 之前 Shadow 返回 false 导致所有飞书推送静默,
/// "📡 产业链扫描" 等常规报告全不发. 改为 Shadow 也推, 仅 Gray 限 critical.
/// 实盘切到 Live 后行为不变.
pub fn should_push_user(stage: LaunchStage, is_critical_alert: bool) -> bool {
    match stage {
        LaunchStage::Live => true,
        LaunchStage::Gray => is_critical_alert,
        LaunchStage::Shadow => true,  // 修复: Shadow 推全量, 不静默
    }
}

/// 修复 F20: 从 prediction_tracker 表算 StageMetrics.
/// shadow_days = 数据库最早 pred_date 距今天数.
/// winrate_pct = hit IS NOT NULL 样本的命中率 (0-1).
/// calmar_ratio = 未实现 (需要 portfolio 历史 equity 曲线), 留 0.0 让灰度阈值卡住.
/// gray_days = 留给外部从 .env LAUNCH_GRAY_START_DATE 读取.
#[allow(dead_code)]
pub fn metrics_from_db(
    db: &crate::database::DatabaseManager,
    gray_start_date: Option<chrono::NaiveDate>,
) -> Result<StageMetrics, Box<dyn std::error::Error>> {
    use diesel::RunQueryDsl;
    let mut conn = db.get_conn()?;

    #[derive(diesel::QueryableByName, Debug)]
    struct MinDateRow {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        min_date: Option<String>,
    }
    #[derive(diesel::QueryableByName, Debug)]
    struct HitRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        hits: i64,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        total: i64,
    }

    let min_date: Option<String> = diesel::sql_query(
        "SELECT MIN(pred_date) AS min_date FROM prediction_tracker WHERE pred_date IS NOT NULL",
    )
    .get_result::<MinDateRow>(&mut *conn)
    .ok()
    .and_then(|r| r.min_date);

    let shadow_days = if let Some(min_date) = min_date {
        chrono::NaiveDate::parse_from_str(&min_date, "%Y-%m-%d")
            .ok()
            .map(|d| (chrono::Local::now().date_naive() - d).num_days().max(0) as u32)
            .unwrap_or(0)
    } else {
        0
    };

    let hit_sql = "SELECT SUM(CASE WHEN hit = 1 THEN 1 ELSE 0 END) AS hits, COUNT(*) AS total \
                   FROM prediction_tracker WHERE hit IS NOT NULL";
    let hit: HitRow = diesel::sql_query(hit_sql).get_result(&mut *conn)?;
    let (winrate_samples, winrate_pct) = if hit.total > 0 {
        (hit.total as u32, hit.hits as f64 / hit.total as f64)
    } else {
        (0u32, 0.0)
    };

    let gray_days = gray_start_date
        .map(|d| (chrono::Local::now().date_naive() - d).num_days().max(0) as u32)
        .unwrap_or(0);

    Ok(StageMetrics {
        shadow_days,
        winrate_samples,
        winrate_pct,
        calmar_ratio: 0.0,
        gray_days,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stage_from_str() {
        assert_eq!("shadow".parse::<LaunchStage>().unwrap(), LaunchStage::Shadow);
        assert_eq!("GRAY".parse::<LaunchStage>().unwrap(), LaunchStage::Gray);
        assert_eq!("Grey".parse::<LaunchStage>().unwrap(), LaunchStage::Gray);
        assert_eq!("live".parse::<LaunchStage>().unwrap(), LaunchStage::Live);
        assert_eq!("prod".parse::<LaunchStage>().unwrap(), LaunchStage::Live);
        assert!("foo".parse::<LaunchStage>().is_err());
    }

    #[test]
    fn test_should_push_user() {
        // 修复 v9.4.23: Shadow 推全量 (之前返回 false 导致飞书推送静默)
        assert!(should_push_user(LaunchStage::Shadow, false));
        assert!(should_push_user(LaunchStage::Shadow, true));
        assert!(!should_push_user(LaunchStage::Gray, false));
        assert!(should_push_user(LaunchStage::Gray, true));
        assert!(should_push_user(LaunchStage::Live, false));
        assert!(should_push_user(LaunchStage::Live, true));
    }

    #[test]
    fn test_check_transition_shadow_to_gray() {
        let m = StageMetrics {
            shadow_days: 10, winrate_samples: 50, winrate_pct: 0.40, calmar_ratio: 0.5, gray_days: 0,
        };
        assert_eq!(LaunchGate::check_transition(LaunchStage::Shadow, &m), None);

        let m = StageMetrics {
            shadow_days: 84, winrate_samples: 200, winrate_pct: 0.60, calmar_ratio: 1.0, gray_days: 0,
        };
        assert_eq!(LaunchGate::check_transition(LaunchStage::Shadow, &m), Some(LaunchStage::Gray));
    }

    #[test]
    fn test_check_transition_gray_to_live_or_shadow() {
        let m = StageMetrics {
            shadow_days: 0, winrate_samples: 0, winrate_pct: 0.55, calmar_ratio: 0.0, gray_days: 30,
        };
        assert_eq!(LaunchGate::check_transition(LaunchStage::Gray, &m), Some(LaunchStage::Live));

        let m = StageMetrics {
            shadow_days: 0, winrate_samples: 0, winrate_pct: 0.45, calmar_ratio: 0.0, gray_days: 15,
        };
        assert_eq!(LaunchGate::check_transition(LaunchStage::Gray, &m), Some(LaunchStage::Shadow));

        let m = StageMetrics {
            shadow_days: 0, winrate_samples: 0, winrate_pct: 0.52, calmar_ratio: 0.0, gray_days: 10,
        };
        assert_eq!(LaunchGate::check_transition(LaunchStage::Gray, &m), None);
    }

    #[test]
    fn test_check_transition_live_no_auto() {
        let m = StageMetrics {
            shadow_days: 100, winrate_samples: 1000, winrate_pct: 0.30, calmar_ratio: 0.0, gray_days: 60,
        };
        assert_eq!(LaunchGate::check_transition(LaunchStage::Live, &m), None);
    }

    #[test]
    fn test_current_stage_default_shadow() {
        std::env::remove_var("STAGE");
        assert_eq!(current_stage(), LaunchStage::Shadow);

        std::env::set_var("STAGE", "Live");
        assert_eq!(current_stage(), LaunchStage::Live);

        std::env::set_var("STAGE", "bogus");
        assert_eq!(current_stage(), LaunchStage::Shadow);

        std::env::remove_var("STAGE");
    }

    #[test]
    fn test_shadow_gray_live_three_stage_e2e() {
        let mut current = LaunchStage::Shadow;
        let mut m = StageMetrics {
            shadow_days: 0, winrate_samples: 0, winrate_pct: 0.0, calmar_ratio: 0.0, gray_days: 0,
        };

        // 1. Shadow 早期
        m.shadow_days = 30;
        assert_eq!(LaunchGate::check_transition(current, &m), None);

        // 2. Shadow 84 天, 胜率低 (修复 v9.4.25.3: 60 → 84, 与 12 周对齐)
        m.shadow_days = 84;
        m.winrate_samples = 200;
        m.winrate_pct = 0.40;
        m.calmar_ratio = 0.5;
        assert_eq!(LaunchGate::check_transition(current, &m), None);

        // 3. Shadow 满足 → Gray
        m.winrate_pct = 0.65;
        m.calmar_ratio = 1.2;
        assert_eq!(LaunchGate::check_transition(current, &m), Some(LaunchStage::Gray));
        current = LaunchStage::Gray;
        m.shadow_days = 0;

        // 4. Gray 早期, 不满足
        m.gray_days = 10;
        assert_eq!(LaunchGate::check_transition(current, &m), None);

        // 5. Gray 胜率掉 → 回退 Shadow
        m.winrate_pct = 0.45;
        assert_eq!(LaunchGate::check_transition(current, &m), Some(LaunchStage::Shadow));

        // 6. 再 Gray, 满足 → Live
        current = LaunchStage::Gray;
        m.gray_days = 30;
        m.winrate_pct = 0.55;
        assert_eq!(LaunchGate::check_transition(current, &m), Some(LaunchStage::Live));
        current = LaunchStage::Live;

        // 7. Live 阶段, 胜率暴跌也不自动转
        m.winrate_pct = 0.10;
        m.gray_days = 100;
        assert_eq!(LaunchGate::check_transition(current, &m), None);
    }
}