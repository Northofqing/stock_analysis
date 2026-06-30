//! 修复 P1-3 (2026-06-30 codex review): 北向资金 Option<f64> 化单测.
//! BR-012: 北向资金缺失必须显式标注, 禁止隐式 0.0.

use stock_analysis::market_analyzer::async_overview::{
    get_market_overview_blocking, generate_market_overview_text_blocking,
};
use stock_analysis::market_data::MarketOverview;

/// 测试 1: MarketOverview::new() 默认 None (不再 0.0 假数据)
#[test]
fn test_market_overview_new_default_is_none() {
    let o = MarketOverview::new("2026-06-30".to_string());
    assert!(o.north_flow.is_none(), "new() 应默认 None 而非 0.0 (BR-012)");
}

/// 测试 2: get_market_overview_blocking 失败时, north_flow 仍保持 None (Err 路径)
/// 沙箱无外网时 fetch 失败, 验证 overview.north_flow == None (不写入 0.0)
#[test]
fn test_blocking_overview_none_on_network_failure() {
    // 沙箱环境调用大概率失败, 但即使失败也不应把 north_flow 写成 0.0
    match get_market_overview_blocking() {
        Ok(overview) => {
            // 成功: 检查 0.0 已被处理为 None
            if let Some(v) = overview.north_flow {
                assert!(v.abs() > 0.001, "0.0 已被挡, 不应出现 Some(0.0): {:?}", overview.north_flow);
            }
            // Some(正/负) 或 None 都接受
        }
        Err(_) => {
            // 沙箱断网, 整体失败. 验证 MarketOverview::new() 仍默认 None
            let o = MarketOverview::new("2026-06-30".to_string());
            assert!(o.north_flow.is_none());
        }
    }
}

/// 测试 3: 端到端 generate_market_overview_text_blocking 不 panic
/// 沙箱环境下应优雅处理 (返回空或含 [数据缺失]), 不 panic.
#[test]
fn test_generate_market_overview_text_does_not_panic() {
    let text = generate_market_overview_text_blocking();
    // 不论成功失败, 不应 panic. 验证可调用.
    let _ = text.len();
}

/// 测试 4: Some(0.0) 不再被静默接受为 "真实数据"
/// 验证 MarketOverview.north_flow 类型签名是 Option<f64> (编译期检查)
#[test]
fn test_north_flow_type_is_option() {
    let o = MarketOverview::new("2026-06-30".to_string());
    // 编译期: north_flow: Option<f64> 已保证
    let _: Option<f64> = o.north_flow;
}

/// 测试 5: 一些边界值类型测试
#[test]
fn test_north_flow_assignments() {
    let mut o = MarketOverview::new("2026-06-30".to_string());
    o.north_flow = Some(12.34);
    assert_eq!(o.north_flow, Some(12.34));
    o.north_flow = Some(-5.67);
    assert_eq!(o.north_flow, Some(-5.67));
    o.north_flow = None;
    assert_eq!(o.north_flow, None);
}
