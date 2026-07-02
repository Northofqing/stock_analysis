//! v10 P0.2 (BR-016) — VirtualReason 枚举 + 主理由优先级
//! BR-020: 样本阈值动态 (Q4=C, max(20, total_pushes*0.05))
//!
//! 设计 (v10 §10.3 + BC-6):
//! - 6 个固定枚举值, **禁止自由文本** 避免统计口径漂移
//! - 多理由命中时按固定优先级取主理由:
//!   NewsCatalyst > AuctionAnomaly > MainNetInflow > SectorLeader > Breakout > VolumeSurge
//! - 结算时按主理由分组统计胜率, reason 样本 < 阈值 → "样本不足"
//! - 新增理由走 enum 变更 + business_rules.md 登记 (AGENTS §2.10 MUST)
//!
//! 归属 (BC-6 DDD): VirtualReason 作 Opportunity context 值对象,
//! Portfolio 存 reason_name snapshot 字符串 (非 FK), Opportunity 自读自分组.

use std::fmt;

/// v10 P0.2 虚拟仓理由枚举 (v10 §10.3)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VirtualReason {
    /// 突破 (平台/新高/形态)
    Breakout,
    /// 放量 (量比异动)
    VolumeSurge,
    /// 主力净流入
    MainNetInflow,
    /// 行业/板块第一 (龙头)
    SectorLeader,
    /// 新闻/公告催化
    NewsCatalyst,
    /// 竞价量能异动 (P0.2 竞价 Agent 专用)
    AuctionAnomaly,
}

impl VirtualReason {
    /// 枚举值数量 (用于硬约束测试)
    pub const COUNT: usize = 6;

    /// 转字符串 (snapshot 用, 禁止自由文本)
    pub fn as_str(&self) -> &'static str {
        match self {
            VirtualReason::Breakout => "Breakout",
            VirtualReason::VolumeSurge => "VolumeSurge",
            VirtualReason::MainNetInflow => "MainNetInflow",
            VirtualReason::SectorLeader => "SectorLeader",
            VirtualReason::NewsCatalyst => "NewsCatalyst",
            VirtualReason::AuctionAnomaly => "AuctionAnomaly",
        }
    }

    /// 从字符串解析 (反向), 用于读 DB 记录
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "Breakout" => Some(VirtualReason::Breakout),
            "VolumeSurge" => Some(VirtualReason::VolumeSurge),
            "MainNetInflow" => Some(VirtualReason::MainNetInflow),
            "SectorLeader" => Some(VirtualReason::SectorLeader),
            "NewsCatalyst" => Some(VirtualReason::NewsCatalyst),
            "AuctionAnomaly" => Some(VirtualReason::AuctionAnomaly),
            _ => None,
        }
    }

    /// 优先级 (BC-6 优先级表, 数字越小优先级越高, 用于 pick_primary)
    /// v10 §10.3 + BC-6: NewsCatalyst > AuctionAnomaly > MainNetInflow > SectorLeader > Breakout > VolumeSurge
    pub fn priority(&self) -> u8 {
        match self {
            VirtualReason::NewsCatalyst => 1,    // 最高优先
            VirtualReason::AuctionAnomaly => 2,
            VirtualReason::MainNetInflow => 3,
            VirtualReason::SectorLeader => 4,
            VirtualReason::Breakout => 5,
            VirtualReason::VolumeSurge => 6,     // 最低
        }
    }

    /// 全部枚举值 (迭代用)
    pub fn all() -> [VirtualReason; Self::COUNT] {
        [
            VirtualReason::Breakout,
            VirtualReason::VolumeSurge,
            VirtualReason::MainNetInflow,
            VirtualReason::SectorLeader,
            VirtualReason::NewsCatalyst,
            VirtualReason::AuctionAnomaly,
        ]
    }
}

impl fmt::Display for VirtualReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 从多个同时命中的理由中选主理由 (BC-6)
/// 优先级最低数字胜出 (1 > 2 > ... > 6)
/// 输入: 命中的理由集合 (Vec 或 slice), 输出: 主理由
/// 空输入 → 返回 None (理论上不应发生, 防御性)
pub fn pick_primary(reasons: &[VirtualReason]) -> Option<VirtualReason> {
    reasons.iter().min_by_key(|r| r.priority()).copied()
}

/// 主/副理由拆分: 选主, 剩余作副
/// 返回 (primary, Option<secondary>)
/// 单一理由时 secondary = None
pub fn split_primary_secondary(reasons: &[VirtualReason]) -> (Option<VirtualReason>, Option<VirtualReason>) {
    let primary = pick_primary(reasons);
    let secondary = reasons
        .iter()
        .filter(|r| Some(*r) != primary.as_ref())
        .min_by_key(|r| r.priority())
        .copied();
    (primary, secondary)
}

/// 样本阈值动态计算 (Q4=C 决策, v10 §5 + §10.3 边缘规则)
/// 公式: `sample_threshold = max(20, total_pushes * 0.05)`
/// 配置: `V10_SAMPLE_THRESHOLD_PCT` env 覆盖 (默认 5%)
///
/// 边界:
/// - total_pushes < 400: 阈值 = 20 (floor)
/// - total_pushes >= 400: 阈值 = total_pushes * 5%
/// - 阈值上限: 200 (避免无限大)
pub fn compute_sample_threshold(total_pushes: usize) -> usize {
    let pct = std::env::var("V10_SAMPLE_THRESHOLD_PCT")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(5.0); // 默认 5%
    let pct_factor = pct / 100.0;

    let by_pct = ((total_pushes as f64) * pct_factor).ceil() as usize;
    let threshold = std::cmp::max(20, by_pct); // floor 20
    std::cmp::min(threshold, 200) // cap 200
}

/// 判断某 reason 样本是否足够 (避免 3/3=100% 假胜率, §10.3)
pub fn is_sample_sufficient(reason_count: usize, total_pushes: usize) -> bool {
    reason_count >= compute_sample_threshold(total_pushes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== VirtualReason enum 测试 =====

    #[test]
    fn test_count_is_six() {
        assert_eq!(VirtualReason::COUNT, 6, "v10 §10.3 规定 6 个枚举值");
    }

    #[test]
    fn test_as_str_all_unique() {
        let mut seen = std::collections::HashSet::new();
        for r in VirtualReason::all().iter() {
            let s = r.as_str();
            assert!(seen.insert(s), "枚举字符串重复: {}", s);
        }
    }

    #[test]
    fn test_parse_round_trip() {
        for r in VirtualReason::all().iter() {
            let parsed = VirtualReason::parse(r.as_str()).unwrap();
            assert_eq!(*r, parsed);
        }
    }

    #[test]
    fn test_parse_unknown_returns_none() {
        assert!(VirtualReason::parse("UnknownReason").is_none());
        assert!(VirtualReason::parse("").is_none());
        assert!(VirtualReason::parse("breakout").is_none()); // 大小写敏感
    }

    // ===== 优先级表 (BC-6) =====

    #[test]
    fn test_priority_table_matches_v10_spec() {
        // v10 §10.3 BC-6 优先级: NewsCatalyst > AuctionAnomaly > MainNetInflow > SectorLeader > Breakout > VolumeSurge
        assert_eq!(VirtualReason::NewsCatalyst.priority(), 1);
        assert_eq!(VirtualReason::AuctionAnomaly.priority(), 2);
        assert_eq!(VirtualReason::MainNetInflow.priority(), 3);
        assert_eq!(VirtualReason::SectorLeader.priority(), 4);
        assert_eq!(VirtualReason::Breakout.priority(), 5);
        assert_eq!(VirtualReason::VolumeSurge.priority(), 6);
    }

    #[test]
    fn test_priorities_all_unique() {
        let mut seen = std::collections::HashSet::new();
        for r in VirtualReason::all().iter() {
            assert!(seen.insert(r.priority()), "优先级重复: {:?}", r);
        }
    }

    // ===== pick_primary (BC-6) =====

    #[test]
    fn test_pick_primary_single() {
        let r = vec![VirtualReason::Breakout];
        assert_eq!(pick_primary(&r), Some(VirtualReason::Breakout));
    }

    #[test]
    fn test_pick_primary_multiple_picks_highest_priority() {
        // 命中: Breakout (5) + NewsCatalyst (1) → NewsCatalyst (1 最高)
        let r = vec![VirtualReason::Breakout, VirtualReason::NewsCatalyst];
        assert_eq!(pick_primary(&r), Some(VirtualReason::NewsCatalyst));

        // 命中: VolumeSurge (6) + MainNetInflow (3) → MainNetInflow (3 最高)
        let r = vec![VirtualReason::VolumeSurge, VirtualReason::MainNetInflow];
        assert_eq!(pick_primary(&r), Some(VirtualReason::MainNetInflow));
    }

    #[test]
    fn test_pick_primary_empty_returns_none() {
        let r: Vec<VirtualReason> = vec![];
        assert_eq!(pick_primary(&r), None);
    }

    // ===== split_primary_secondary =====

    #[test]
    fn test_split_single_returns_none_secondary() {
        let r = vec![VirtualReason::NewsCatalyst];
        assert_eq!(
            split_primary_secondary(&r),
            (Some(VirtualReason::NewsCatalyst), None)
        );
    }

    #[test]
    fn test_split_multiple_picks_secondary_by_priority() {
        // 命中: Breakout (5) + NewsCatalyst (1) + MainNetInflow (3)
        // primary = NewsCatalyst (1), secondary = MainNetInflow (3 除去 primary)
        let r = vec![
            VirtualReason::Breakout,
            VirtualReason::NewsCatalyst,
            VirtualReason::MainNetInflow,
        ];
        assert_eq!(
            split_primary_secondary(&r),
            (Some(VirtualReason::NewsCatalyst), Some(VirtualReason::MainNetInflow))
        );
    }

    // ===== compute_sample_threshold (Q4=C 决策) =====

    #[test]
    fn test_sample_threshold_floor_20() {
        // total < 400 时, 阈值 floor = 20
        assert_eq!(compute_sample_threshold(0), 20);
        assert_eq!(compute_sample_threshold(100), 20);
        assert_eq!(compute_sample_threshold(399), 20);
    }

    #[test]
    fn test_sample_threshold_5pct() {
        // total >= 400 时, 阈值 = 5%
        assert_eq!(compute_sample_threshold(400), 20); // 400 * 0.05 = 20
        assert_eq!(compute_sample_threshold(500), 25); // 500 * 0.05 = 25
        assert_eq!(compute_sample_threshold(1000), 50); // 1000 * 0.05 = 50
        assert_eq!(compute_sample_threshold(2000), 100);
    }

    #[test]
    fn test_sample_threshold_cap_200() {
        // 极端大数: 阈值 cap = 200
        assert_eq!(compute_sample_threshold(10_000), 200);
        assert_eq!(compute_sample_threshold(100_000), 200);
    }

    // ===== is_sample_sufficient =====

    #[test]
    fn test_sample_sufficient_basic() {
        // 10 推送, 1 reason → 1 < 20 (floor) → 不够
        assert!(!is_sample_sufficient(1, 10));
        // 10 推送, 20 reason → 20 >= 20 → 够
        assert!(is_sample_sufficient(20, 10));
        // 10 推送, 19 reason → 19 < 20 → 不够
        assert!(!is_sample_sufficient(19, 10));
    }
}
