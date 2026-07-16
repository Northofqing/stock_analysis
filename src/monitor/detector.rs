//! 异动检测规则引擎。
//!
//! 职责：消费经过 DQ Gate 校验的 Tick/快讯，按规则判定是否触发告警。
//! 结果发往 SignalStateMachine 做去重/幂等处理，而非直接推送。

use chrono::{DateTime, Local};

// ============================================================================
// 告警事件
// ============================================================================

#[derive(Debug, Clone)]
pub struct AlertEvent {
    pub level: AlertLevel,
    pub category: AlertCategory,
    pub code: String,
    pub name: String,
    pub message: String,
    pub detail: AlertDetail,
    pub triggered_at: DateTime<Local>,
    /// Source external ID if this event was routed through v17_sources::push_normalized_events.
    /// Used by news_monitor_loop to skip duplicate legacy push.
    pub routed_external_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlertLevel {
    Emergency = 0, // 🔴 紧急
    Important = 1, // 🟠 重要
    Info = 2,      // 🟡 参考
}

impl AlertLevel {
    pub fn label(&self) -> &'static str {
        match self {
            AlertLevel::Emergency => "紧急",
            AlertLevel::Important => "重要",
            AlertLevel::Info => "参考",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            AlertLevel::Emergency => "🔴",
            AlertLevel::Important => "🟠",
            AlertLevel::Info => "🟡",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertCategory {
    LimitUp,         // 涨停突破
    LimitDown,       // 跌停扫雷
    MainInflow,      // 主力突袭
    MainOutflow,     // 主力出逃
    VolBurst,        // 量比爆发
    BoardBreak,      // 炸板
    IndexPlunge,     // 指数跳水
    FlashNews,       // 快讯催化
    SectorResonance, // 板块共振
    TechnicalSignal, // 技术形态
    AuctionGap,      // 竞价异动
    ChainRisk,       // T+1 / 产业链风险
}

impl AlertCategory {
    /// 用于状态机的去重 Key
    pub fn key(&self) -> &'static str {
        match self {
            AlertCategory::LimitUp => "limit_up",
            AlertCategory::LimitDown => "limit_down",
            AlertCategory::MainInflow => "main_inflow",
            AlertCategory::MainOutflow => "main_outflow",
            AlertCategory::VolBurst => "vol_burst",
            AlertCategory::BoardBreak => "board_break",
            AlertCategory::IndexPlunge => "index_plunge",
            AlertCategory::FlashNews => "flash_news",
            AlertCategory::SectorResonance => "sector_resonance",
            AlertCategory::TechnicalSignal => "technical",
            AlertCategory::AuctionGap => "auction_gap",
            AlertCategory::ChainRisk => "chain_risk",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            AlertCategory::LimitUp => "涨停突破",
            AlertCategory::LimitDown => "跌停扫雷",
            AlertCategory::MainInflow => "主力突袭",
            AlertCategory::MainOutflow => "主力出逃",
            AlertCategory::VolBurst => "量比爆发",
            AlertCategory::BoardBreak => "炸板",
            AlertCategory::IndexPlunge => "指数跳水",
            AlertCategory::FlashNews => "快讯催化",
            AlertCategory::SectorResonance => "板块共振",
            AlertCategory::TechnicalSignal => "技术形态",
            AlertCategory::AuctionGap => "竞价异动",
            AlertCategory::ChainRisk => "风控告警",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AlertDetail {
    pub price: Option<f64>,
    pub change_pct: Option<f64>,
    pub volume_ratio: Option<f64>,
    pub main_flow_yi: Option<f64>,
    pub threshold: Option<f64>,
    pub news_title: Option<String>,
    pub news_summary: Option<String>,
    pub ai_decision: Option<String>,
    pub t1_locked: bool,
    pub extra: Option<String>,
}

// ============================================================================
// 检测输入
// ============================================================================

#[derive(Debug, Clone)]
pub struct StockSnapshot {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub change_pct: f64,
    pub volume_ratio: f64,
    pub main_net_yi: f64, // 主力净流入（亿）
    pub limit_up_price: Option<f64>,
    pub was_limit_up: bool, // 上一 tick 是否涨停
    pub t1_locked: bool,    // 是否 T+1 锁仓
}

#[derive(Debug, Clone)]
pub struct IndexSnapshot {
    pub name: String,
    pub change_pct: f64,
    pub change_5min_pct: f64, // 最近5分钟涨跌幅
    /// 修复 P3.5: 20 日 ATR 百分比 (绝对值, 例如 0.015 表示 1.5%)
    /// 用于 check_index_plunge 自适应阈值
    /// 量化分析师角度: 牛市 ATR ~0.8% (阈值 -1.6% 都不触发), 熊市 ATR ~2% (阈值 -4% 也不触发)
    ///               真正崩盘的标志是 5min 跌幅 >> 2σ × ATR
    pub atr_pct: f64,
}

#[derive(Debug, Clone)]
pub struct NewsItem {
    pub title: String,
    pub source: String,
    pub importance: u8, // 1-5
}

// ============================================================================
// 规则配置
// ============================================================================

#[derive(Debug, Clone)]
pub struct DetectorConfig {
    pub limit_up_pct: f64,
    pub limit_down_pct: f64,
    pub main_inflow_yi: f64,
    pub main_outflow_yi: f64,
    pub vol_ratio_threshold: f64,
    pub vol_ratio_price_pct: f64,
    pub index_plunge_5min_pct: f64,
    pub index_plunge_daily_pct: f64,
    pub board_break_rate_pct: f64,
    pub auction_gap_pct: f64,
    pub auction_vol_ratio: f64,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            limit_up_pct: 9.5,
            limit_down_pct: -9.5,
            main_inflow_yi: 0.5,
            main_outflow_yi: 0.3,
            vol_ratio_threshold: 3.0,
            vol_ratio_price_pct: 5.0,
            index_plunge_5min_pct: -1.0,
            index_plunge_daily_pct: -3.0,
            board_break_rate_pct: 40.0,
            auction_gap_pct: 3.0,
            auction_vol_ratio: 5.0,
        }
    }
}

// ============================================================================
// 规则引擎
// ============================================================================

pub struct Detector {
    config: DetectorConfig,
}

impl Detector {
    pub fn new(config: DetectorConfig) -> Self {
        Self { config }
    }

    // ── 持仓股检测 ──

    pub fn check_limit_up(&self, s: &StockSnapshot) -> Option<AlertEvent> {
        if s.change_pct >= self.config.limit_up_pct {
            Some(self.build(
                s,
                AlertCategory::LimitUp,
                AlertLevel::Important,
                format!("{} 涨幅 {:.1}%，接近涨停", s.name, s.change_pct),
            ))
        } else {
            None
        }
    }

    pub fn check_limit_down(&self, s: &StockSnapshot) -> Option<AlertEvent> {
        if s.change_pct <= self.config.limit_down_pct {
            // 修复 P1.9: 去掉死分支 (两分支都是 Emergency)
            // 跌停是严重的, T+1锁仓时尤其严重, 永远 Emergency
            let mut msg = format!("{} 跌幅 {:.1}%，跌停", s.name, s.change_pct);
            if s.t1_locked {
                msg.push_str(" ⚠️ T+1锁仓，不可当日卖出");
            }
            Some(self.build(s, AlertCategory::LimitDown, AlertLevel::Emergency, msg))
        } else {
            None
        }
    }

    pub fn check_main_outflow(&self, s: &StockSnapshot) -> Option<AlertEvent> {
        if s.main_net_yi <= -self.config.main_outflow_yi {
            // 修复 P1.9: 去掉死分支 (两分支都是 Important)
            // 量化分析师要求: 死代码删除, 真实分级 (未来可按 magnitude 分 Important/Warning)
            let mut msg = format!("{} 主力净流出 {:.2}亿", s.name, -s.main_net_yi);
            if s.t1_locked {
                msg.push_str("（T+1锁仓中，观察收盘是否回补）");
            } else {
                msg.push_str("，建议关注是否减仓");
            }
            Some(self.build(s, AlertCategory::MainOutflow, AlertLevel::Important, msg))
        } else {
            None
        }
    }

    pub fn check_main_inflow(&self, s: &StockSnapshot) -> Option<AlertEvent> {
        if s.main_net_yi >= self.config.main_inflow_yi {
            Some(self.build(
                s,
                AlertCategory::MainInflow,
                AlertLevel::Important,
                format!("{} 主力净流入 {:.2}亿", s.name, s.main_net_yi),
            ))
        } else {
            None
        }
    }

    pub fn check_vol_burst(&self, s: &StockSnapshot) -> Option<AlertEvent> {
        if s.volume_ratio >= self.config.vol_ratio_threshold
            && s.change_pct >= self.config.vol_ratio_price_pct
        {
            Some(self.build(
                s,
                AlertCategory::VolBurst,
                AlertLevel::Important,
                format!(
                    "{} 量比 {:.1} 涨幅 {:.1}%",
                    s.name, s.volume_ratio, s.change_pct
                ),
            ))
        } else {
            None
        }
    }

    pub fn check_board_break(&self, s: &StockSnapshot) -> Option<AlertEvent> {
        if s.was_limit_up && s.change_pct < 9.0 {
            Some(self.build(
                s,
                AlertCategory::BoardBreak,
                AlertLevel::Emergency,
                format!("{} 涨停打开！现涨幅 {:.1}%", s.name, s.change_pct),
            ))
        } else {
            None
        }
    }

    // ── 大盘检测 ──

    pub fn check_index_plunge(&self, idx: &IndexSnapshot) -> Option<AlertEvent> {
        // 修复 P3.5: 阈值从 -1.0 写死 → ATR 自适应
        // 之前: 5 分钟跌 ≥ 1% 告警 (无视大盘波动率, 牛市震荡市都触发)
        // 现在: 阈值 = -ATR(20) × 2.0 (默认配置, P3.1 集中 risk.toml)
        // 牛市: ATR 小 → 阈值小 (-0.6% 即可)
        // 熊市: ATR 大 → 阈值大 (-1.5% 才算崩盘)
        // 量化分析师角度: 静态 -1% 在不同波动率时期信号质量差异巨大
        let atr_threshold = if idx.atr_pct.abs() > 0.0 {
            // idx.atr_pct 是 20 日 ATR 百分比, 阈值 = -2 × ATR
            -idx.atr_pct * 2.0
        } else {
            // ATR 缺失时退回写死 -1% (兼容旧数据)
            self.config.index_plunge_5min_pct
        };
        if idx.change_5min_pct <= atr_threshold {
            Some(AlertEvent {
                level: AlertLevel::Emergency,
                category: AlertCategory::IndexPlunge,
                code: "INDEX".into(),
                name: idx.name.clone(),
                message: format!(
                    "{} 5分钟跌 {:.1}% (ATR阈值 {:.2}%, 静态回退 {:.1}%)",
                    idx.name, idx.change_5min_pct, atr_threshold, self.config.index_plunge_5min_pct
                ),
                detail: AlertDetail {
                    price: None,
                    change_pct: Some(idx.change_5min_pct),
                    volume_ratio: None,
                    main_flow_yi: None,
                    threshold: Some(atr_threshold),
                    news_title: None,
                    news_summary: None,
                    ai_decision: None,
                    t1_locked: false,
                    extra: Some(format!(
                        "ATR自适应当前阈值={:.3}%, 静态回退={:.1}%",
                        atr_threshold, self.config.index_plunge_5min_pct
                    )),
                },
                triggered_at: Local::now(),
                routed_external_id: None,
            })
        } else {
            None
        }
    }

    // ── 竞价检测 ──

    pub fn check_auction_gap(
        &self,
        s: &StockSnapshot,
        auction_vol_ratio: f64,
    ) -> Option<AlertEvent> {
        if s.change_pct.abs() >= self.config.auction_gap_pct
            && auction_vol_ratio >= self.config.auction_vol_ratio
        {
            let direction = if s.change_pct > 0.0 {
                "高开"
            } else {
                "低开"
            };
            let lvl = if s.change_pct < -5.0 && s.t1_locked {
                AlertLevel::Emergency
            } else {
                AlertLevel::Important
            };
            Some(self.build(
                s,
                AlertCategory::AuctionGap,
                lvl,
                format!(
                    "{} 竞价{} {:.1}%，量比 {:.1}",
                    s.name, direction, s.change_pct, auction_vol_ratio
                ),
            ))
        } else {
            None
        }
    }

    // ── 快讯检测 ──

    pub fn check_flash_news(&self, news: &NewsItem, hit_codes: &[&str]) -> Option<AlertEvent> {
        if news.importance >= 3 && !hit_codes.is_empty() {
            let code_list = hit_codes.join("、");
            Some(AlertEvent {
                level: if news.importance >= 4 {
                    AlertLevel::Important
                } else {
                    AlertLevel::Info
                },
                category: AlertCategory::FlashNews,
                code: hit_codes[0].to_string(),
                name: code_list,
                message: format!("快讯命中: {}", news.title),
                detail: AlertDetail {
                    price: None,
                    change_pct: None,
                    volume_ratio: None,
                    main_flow_yi: None,
                    threshold: None,
                    news_title: Some(news.title.clone()),
                    news_summary: None,
                    ai_decision: None,
                    t1_locked: false,
                    extra: Some(format!("来源: {}", news.source)),
                },
                triggered_at: Local::now(),
                routed_external_id: None,
            })
        } else {
            None
        }
    }

    // ── 风控检测 ──

    pub fn check_chain_concentration(
        &self,
        chain: &str,
        pct: f64,
        threshold: f64,
    ) -> Option<AlertEvent> {
        if pct >= threshold {
            Some(AlertEvent {
                level: AlertLevel::Important,
                category: AlertCategory::ChainRisk,
                code: "RISK".into(),
                name: chain.to_string(),
                message: format!("{} 产业链集中度 {:.0}% ≥ {:.0}%", chain, pct, threshold),
                detail: AlertDetail {
                    price: None,
                    change_pct: Some(pct),
                    volume_ratio: None,
                    main_flow_yi: None,
                    threshold: Some(threshold),
                    news_title: None,
                    news_summary: None,
                    ai_decision: None,
                    t1_locked: false,
                    extra: None,
                },
                triggered_at: Local::now(),
                routed_external_id: None,
            })
        } else {
            None
        }
    }

    // ── 辅助 ──

    fn build(
        &self,
        s: &StockSnapshot,
        cat: AlertCategory,
        lvl: AlertLevel,
        msg: String,
    ) -> AlertEvent {
        AlertEvent {
            level: lvl,
            category: cat,
            code: s.code.clone(),
            name: s.name.clone(),
            message: msg,
            detail: AlertDetail {
                price: Some(s.price),
                change_pct: Some(s.change_pct),
                volume_ratio: Some(s.volume_ratio),
                main_flow_yi: Some(s.main_net_yi),
                threshold: None,
                news_title: None,
                news_summary: None,
                ai_decision: None,
                t1_locked: s.t1_locked,
                extra: None,
            },
            triggered_at: Local::now(),
            routed_external_id: None,
        }
    }

    /// 对单只股票执行全部规则
    pub fn scan_stock(&self, s: &StockSnapshot) -> Vec<AlertEvent> {
        let mut events = Vec::new();
        if let Some(e) = self.check_limit_down(s) {
            events.push(e);
        }
        if let Some(e) = self.check_board_break(s) {
            events.push(e);
        }
        if let Some(e) = self.check_limit_up(s) {
            events.push(e);
        }
        if let Some(e) = self.check_main_outflow(s) {
            events.push(e);
        }
        if let Some(e) = self.check_main_inflow(s) {
            events.push(e);
        }
        if let Some(e) = self.check_vol_burst(s) {
            events.push(e);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stock(code: &str, name: &str, change: f64, vol: f64, flow: f64) -> StockSnapshot {
        StockSnapshot {
            code: code.into(),
            name: name.into(),
            price: 10.0 * (1.0 + change / 100.0),
            change_pct: change,
            volume_ratio: vol,
            main_net_yi: flow,
            limit_up_price: Some(11.0),
            was_limit_up: false,
            t1_locked: false,
        }
    }

    #[test]
    fn test_limit_up_triggers() {
        let d = Detector::new(DetectorConfig::default());
        let s = stock("000001", "测试", 9.8, 1.0, 0.0);
        assert!(d.check_limit_up(&s).is_some());
    }

    #[test]
    fn test_normal_stock_no_alert() {
        let d = Detector::new(DetectorConfig::default());
        let s = stock("000001", "测试", 2.0, 1.0, 0.0);
        assert!(d.scan_stock(&s).is_empty());
    }

    #[test]
    fn test_limit_down_emergency() {
        let d = Detector::new(DetectorConfig::default());
        let s = stock("000002", "跌停股", -10.0, 2.0, 0.0);
        let events = d.scan_stock(&s);
        assert!(!events.is_empty());
        assert_eq!(events[0].level, AlertLevel::Emergency);
    }

    #[test]
    fn test_t1_locked_limit_down_msg() {
        let d = Detector::new(DetectorConfig::default());
        let mut s = stock("000003", "锁仓股", -10.0, 2.0, 0.0);
        s.t1_locked = true;
        let e = d.check_limit_down(&s).unwrap();
        assert!(e.message.contains("T+1"));
        assert!(e.detail.t1_locked);
    }

    #[test]
    fn test_main_outflow_triggers() {
        let d = Detector::new(DetectorConfig::default());
        let s = stock("000004", "出逃股", -2.0, 1.0, -0.5);
        assert!(d.check_main_outflow(&s).is_some());
    }

    #[test]
    fn test_main_inflow_triggers() {
        let d = Detector::new(DetectorConfig::default());
        let s = stock("000005", "流入股", 3.0, 1.5, 0.6);
        assert!(d.check_main_inflow(&s).is_some());
    }

    #[test]
    fn test_vol_burst_triggers() {
        let d = Detector::new(DetectorConfig::default());
        let s = stock("000006", "放量股", 6.0, 4.0, 0.0);
        assert!(d.check_vol_burst(&s).is_some());
    }

    #[test]
    fn test_vol_burst_no_price_triggers_no_alert() {
        let d = Detector::new(DetectorConfig::default());
        let s = stock("000007", "放量不涨", 1.0, 5.0, 0.0); // high vol but low price
        assert!(d.check_vol_burst(&s).is_none());
    }

    #[test]
    fn test_board_break() {
        let d = Detector::new(DetectorConfig::default());
        let mut s = stock("000008", "炸板股", 7.0, 2.0, 0.0);
        s.was_limit_up = true;
        let e = d.check_board_break(&s).unwrap();
        assert_eq!(e.level, AlertLevel::Emergency);
    }

    #[test]
    fn test_index_plunge() {
        let d = Detector::new(DetectorConfig::default());
        let idx = IndexSnapshot {
            name: "沪指".into(),
            change_pct: -1.5,
            change_5min_pct: -1.5,
            atr_pct: 0.0,
        };
        assert!(d.check_index_plunge(&idx).is_some());
    }

    #[test]
    fn test_index_normal() {
        let d = Detector::new(DetectorConfig::default());
        let idx = IndexSnapshot {
            name: "沪指".into(),
            change_pct: -0.3,
            change_5min_pct: -0.3,
            atr_pct: 0.0,
        };
        assert!(d.check_index_plunge(&idx).is_none());
    }

    #[test]
    fn test_auction_gap_up() {
        let d = Detector::new(DetectorConfig::default());
        let s = stock("000009", "竞价股", 5.0, 1.0, 0.0);
        assert!(d.check_auction_gap(&s, 6.0).is_some());
    }

    #[test]
    fn test_auction_normal() {
        let d = Detector::new(DetectorConfig::default());
        let s = stock("000010", "竞价正常", 1.0, 1.0, 0.0);
        assert!(d.check_auction_gap(&s, 2.0).is_none());
    }

    #[test]
    fn test_flash_news_hit() {
        let d = Detector::new(DetectorConfig::default());
        let news = NewsItem {
            title: "重大利好".into(),
            source: "金十".into(),
            importance: 4,
        };
        assert!(d.check_flash_news(&news, &["000001"]).is_some());
    }

    #[test]
    fn test_flash_news_low_importance_skip() {
        let d = Detector::new(DetectorConfig::default());
        let news = NewsItem {
            title: "普通消息".into(),
            source: "金十".into(),
            importance: 1,
        };
        assert!(d.check_flash_news(&news, &["000001"]).is_none());
    }
}
