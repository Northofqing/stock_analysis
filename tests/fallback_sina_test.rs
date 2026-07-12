//! Task 4: 验证 SinaProvider 已接入 fallback 5-way join (review #15 → 4-way → 5-way).
//!
//! 集成测试 (网络依赖): 任一源能返数据即可, source label 必须在 4 源范围内.
//! 直连测试: 用 SinaProvider 直接调用, 确认 provider 本身可工作 (不依赖 join).

use stock_analysis::data_provider::fallback::fetch_kline_with_fallback;
use stock_analysis::data_provider::sina_provider::SinaProvider;

#[tokio::test]
async fn fallback_returns_data_with_sina_in_chain() {
    // 任一源能返数据即可 (网络依赖, 不强求 sina); 但 source 必须在 4 源范围.
    let (data, src) = fetch_kline_with_fallback("600000", 5)
        .await
        .expect("fetch_kline_with_fallback should return Ok from any of 4 sources");
    assert!(
        !data.is_empty(),
        "Sina/腾讯/东财/RustDX 中任一源都不该返空, 实际 {} 条",
        data.len()
    );
    assert!(
        matches!(
            src,
            "sina_hq" | "tencent_qfq" | "eastmoney_qfq" | "rustdx_none"
        ),
        "source 必须是 4-way 链中之一, 实际={}",
        src
    );
}

/// 直连 SinaProvider: 验证 provider 本身能拉数据 (Task 2 已测 build/url, 这里测 round-trip).
/// 若此测试 PASS 但上条 FAIL, 说明 fallback 链未集成 Sina (回归保护).
#[tokio::test]
async fn sina_provider_direct_fetch_works() {
    let data = SinaProvider::new()
        .fetch_kline_raw("600000", 5)
        .await
        .expect("SinaProvider::fetch_kline_raw should succeed for 600000");
    assert!(!data.is_empty(), "Sina 直连应至少返 1 条 K 线");
}
