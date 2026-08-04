//! v11 P0-1+P0-2 commit 4: 端到端验收 — 统一日 K 路由证据 + adjust 标注一致性
//!
//! ⚠️ 网络依赖 — `#[ignore]` 跳过 CI, 手动跑:
//!   cargo test --test v11_three_sources -- --ignored
//!
//! 验收内容 (v11-p0-1-p0-2-设计定稿v2-2026-07-02 §五):
//! 1. 同一只股票走 BR-164 Magic BarsRouter 拉到数据
//! 2. 返回的 provider/source evidence 符合规范
//! 3. 每根 K 线的 `adjust` 字段与 source 一致:
//!    - 当前固定 revision 的四个 provider 都只提供未复权日线 → None
//! 4. 数据 sanity: 非空 + 价格为正 + OHLC/量额完整 + 日期连续

use stock_analysis::data_gateway::{daily_bar_provider_label, HistoricalBarsGateway};
use stock_analysis::data_provider::AdjustType;

/// 主板 (600519) + 深市主板 (000001) + 创业板 (300750) 各一只
const TEST_CODES: &[&str] = &["600519", "000001", "300750"];

async fn fetch_gateway_records(
    code: &str,
    days: usize,
) -> anyhow::Result<(Vec<stock_analysis::data_provider::KlineData>, &'static str)> {
    let batch = HistoricalBarsGateway::new()
        .required_daily_bars_async(code, days)
        .await?;
    let (records, evidence) = batch.into_parts();
    Ok((records, daily_bar_provider_label(evidence.provider)))
}

/// 验收 1+2+3: Gateway 返回正确 source 字符串 + adjust 标注一致
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn gateway_returns_consistent_source_and_adjust() {
    for &code in TEST_CODES {
        let (data, source) = fetch_gateway_records(code, 30)
            .await
            .unwrap_or_else(|error| panic!("[{code}] Gateway live admission failed: {error}"));

        eprintln!("[{}] source={} bars={}", code, source, data.len());
        assert!(!data.is_empty(), "[{}] 应返回非空数据", code);

        // 关键断言: source 字符串必须是约定之一
        assert!(
            matches!(
                source,
                "magic_tdx" | "magic_tencent" | "magic_sina" | "magic_baidu"
            ),
            "[{}] 未知的 source: {}",
            code,
            source
        );

        // 当前固定 revision 的四个 Provider 都把日线声明为未复权。
        let expected_adjust = AdjustType::None;

        let wrong_count = data.iter().filter(|b| b.adjust != expected_adjust).count();
        assert_eq!(
            wrong_count, 0,
            "[{}] source={} 但有 {} 根 K 线 adjust 不一致 (expected={:?})",
            code, source, wrong_count, expected_adjust
        );
    }
}

/// 验收 4: 数据 sanity (价格非负 + 非 NaN)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn gateway_returns_sane_prices() {
    for &code in TEST_CODES {
        let (data, _) = fetch_gateway_records(code, 30)
            .await
            .unwrap_or_else(|error| panic!("[{code}] Gateway live admission failed: {error}"));

        for bar in &data {
            assert!(
                bar.open.is_finite()
                    && bar.high.is_finite()
                    && bar.low.is_finite()
                    && bar.close.is_finite(),
                "[{}] {} 存在 NaN/Inf",
                code,
                bar.date
            );
            assert!(
                bar.open > 0.0 && bar.high > 0.0 && bar.low > 0.0 && bar.close > 0.0,
                "[{}] {} 存在非正价格",
                code,
                bar.date
            );
        }
    }
}

/// 验收 5: Gateway 接纳的真实日线必须日期唯一并严格递增。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn gateway_returns_unique_ascending_dates() {
    let (data, _) = fetch_gateway_records("300750", 30)
        .await
        .unwrap_or_else(|error| panic!("[300750] Gateway live admission failed: {error}"));

    for pair in data.windows(2) {
        assert!(
            pair[0].date < pair[1].date,
            "300750 日线日期必须唯一且严格递增: {} -> {}",
            pair[0].date,
            pair[1].date
        );
    }
}
