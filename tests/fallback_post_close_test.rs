//! Task 7: 验证 fetch_kline_post_close (盘后专用路径).
//!
//! 盘后窗口 (15:00-次日 9:30) Baostock 优先 — 日终权威、无限流.
//! 失败 fallthrough 到 review #15 5-way join.
//!
//! 集成测试 (网络依赖): 任一源能返数据即可, source 必须在链范围内.
//! 直连测试: BaostockProvider::fetch_kline_async 单独验证 provider 本身能工作.

use stock_analysis::data_provider::baostock_provider::BaostockProvider;
use stock_analysis::data_provider::fallback::fetch_kline_post_close;

/// Task 7 主断言: fetch_kline_post_close 至少能从一条链拿到数据.
/// 期望 Baostock 胜出 (盘后窗口), 但允许 fallthrough 到 5-way 任一源.
#[tokio::test]
async fn post_close_prefers_baostock() {
    let (data, src) = fetch_kline_post_close("600000", 30)
        .await
        .expect("fetch_kline_post_close should return Ok from baostock or 5-way fallthrough");
    assert!(
        !data.is_empty(),
        "盘后专用路径至少应返 1 条 K 线, 实际 {} 条",
        data.len()
    );
    assert!(
        matches!(
            src,
            "baostock" | "sina_hq" | "tencent_qfq" | "eastmoney_qfq" | "rustdx_none"
        ),
        "source 必须在盘后链或 5-way 链中, 实际={}",
        src
    );
    eprintln!("[task-7] post_close src = {src}, 条数 = {}", data.len());
}

/// 回归保护: BaostockProvider 单独能拉数据 (Task 6 已测, 这里测 round-trip).
/// 若此测试 PASS 但上条 FAIL, 说明 fallback 链未集成 Baostock.
/// 网络依赖, 默认跳过; 手动跑: cargo test --test fallback_post_close_test -- --ignored
#[tokio::test]
#[ignore]
async fn baostock_provider_direct_fetch_works() {
    let data = BaostockProvider::new()
        .fetch_kline_async("600000", 5)
        .await
        .expect("BaostockProvider::fetch_kline_async should succeed for 600000");
    assert!(!data.is_empty(), "Baostock 直连应至少返 1 条 K 线");
}
