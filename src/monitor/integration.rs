//! 监控全链路集成测试。
//!
//! 验证完整链路：检测器 → 状态机 → 告警格式化 → 聚合

#[cfg(test)]
mod tests {
    use crate::monitor::detector::{
        AlertCategory, AlertEvent, AlertLevel, Detector, DetectorConfig, StockSnapshot,
    };
    use crate::monitor::signal_state::SignalStateMachine;
    use crate::monitor::{alert, auction};

    fn stock(code: &str, name: &str, change: f64) -> StockSnapshot {
        StockSnapshot {
            code: code.into(),
            name: name.into(),
            price: 10.0 * (1.0 + change / 100.0),
            change_pct: change,
            volume_ratio: 1.0,
            main_net_yi: 0.0,
            limit_up_price: Some(11.0),
            was_limit_up: false,
            t1_locked: false,
        }
    }

    #[test]
    fn test_full_pipeline_limit_down() {
        // 1. 检测
        let detector = Detector::new(DetectorConfig::default());
        let s = stock("000001", "测试股", -10.0);
        let events = detector.scan_stock(&s);
        assert!(!events.is_empty());

        // 2. 状态机去重
        let mut sm = SignalStateMachine::default();
        let filtered: Vec<_> = events.into_iter().filter_map(|e| sm.process(e)).collect();
        assert!(!filtered.is_empty());

        // 3. 格式化
        for e in &filtered {
            let text = alert::format_alert(e);
            assert!(text.contains("测试股"));
            assert!(text.contains("000001"));
            // 紧急级别应该有 emoji
            if e.level == AlertLevel::Emergency {
                assert!(text.contains("🔴"));
            }
        }
    }

    #[test]
    fn test_pipeline_dedup() {
        let detector = Detector::new(DetectorConfig::default());
        let s = stock("000002", "重复股", -10.0);
        let events = detector.scan_stock(&s);

        let mut sm = SignalStateMachine::default();
        // 同一事件连续发两次 → 第二次被静默
        let first: Vec<_> = events
            .iter()
            .filter_map(|e| sm.process(e.clone()))
            .collect();
        let second: Vec<_> = events
            .iter()
            .filter_map(|e| sm.process(e.clone()))
            .collect();
        assert!(!first.is_empty());
        assert!(second.is_empty(), "重复告警应被状态机静默");
    }

    #[test]
    fn test_pipeline_different_stocks_pass() {
        let detector = Detector::new(DetectorConfig::default());
        let mut sm = SignalStateMachine::default();

        let e1 = detector.scan_stock(&stock("000003", "A股", -10.0));
        let e2 = detector.scan_stock(&stock("000004", "B股", -10.0));

        let r1: Vec<_> = e1.into_iter().filter_map(|e| sm.process(e)).collect();
        let r2: Vec<_> = e2.into_iter().filter_map(|e| sm.process(e)).collect();

        assert!(!r1.is_empty());
        assert!(!r2.is_empty()); // 不同股票应分别触发
    }

    #[test]
    fn test_auction_classification_flows() {
        let r = auction::AuctionResult {
            code: "000005".into(),
            name: "竞价股".into(),
            gap_pct: -6.0,
            vol_ratio: 8.0,
            match_ratio: 90.0,
            suspected_fake: false,
        };

        let event = auction::classify_auction(&r, true, 3.0, 5.0).unwrap();
        assert_eq!(event.level, AlertLevel::Emergency);
        assert!(event.detail.t1_locked);

        // 格式化
        let text = alert::format_alert(&event);
        assert!(text.contains("T+1锁仓"));
    }

    #[test]
    fn test_alert_aggregation_with_state_machine() {
        let detector = Detector::new(DetectorConfig::default());
        let mut sm = SignalStateMachine::default();

        let stocks = [
            ("000006", "股1", -10.0),
            ("000007", "股2", 9.8),
            ("000008", "股3", -5.0),
        ];

        let mut alerts = Vec::new();
        for (code, name, change) in &stocks {
            for e in detector.scan_stock(&stock(code, name, *change)) {
                if let Some(event) = sm.process(e) {
                    alerts.push(event);
                }
            }
        }

        // 至少应该有 2 条（跌停 + 涨停）
        assert!(alerts.len() >= 2, "应有至少2条告警，实际 {}", alerts.len());

        // 聚合
        let summary = alert::aggregate_alerts(&alerts).unwrap();
        assert!(summary.contains("告警聚合"));
    }
}
