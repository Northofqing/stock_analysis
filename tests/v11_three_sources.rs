//! v11 P0-1+P0-2 commit 4: 端到端验收 — 三源价格重合 + adjust 标注一致性
//!
//! ⚠️ 网络依赖 — `#[ignore]` 跳过 CI, 手动跑:
//!   cargo test --test v11_three_sources -- --ignored
//!
//! 验收内容 (v11-p0-1-p0-2-设计定稿v2-2026-07-02 §五):
//! 1. 同一只股票走 fallback 链 (腾讯 → 东财 → RustDX) 拉到数据
//! 2. 返回的 source 字符串符合规范 (tencent_qfq / eastmoney_qfq / rustdx_none)
//! 3. 每根 K 线的 `adjust` 字段与 source 一致:
//!    - tencent_qfq / eastmoney_qfq → Qfq
//!    - rustdx_none → None
//! 4. 数据 sanity: 非空 + 价格为正 + 跳空不超 max_gap_for(code)

use stock_analysis::data_provider::fallback::fetch_kline_with_fallback;
use stock_analysis::data_provider::AdjustType;

/// 主板 (600519) + 深市主板 (000001) + 创业板 (300750) 各一只
const TEST_CODES: &[&str] = &["600519", "000001", "300750"];

/// 验收 1+2+3: fallback 返回正确 source 字符串 + adjust 标注一致
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn fallback_returns_consistent_source_and_adjust() {
    for &code in TEST_CODES {
        let (data, source) = match fetch_kline_with_fallback(code, 30).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[{}] fallback failed (网络问题, 跳过): {}", code, e);
                continue;
            }
        };

        eprintln!("[{}] source={} bars={}", code, source, data.len());
        assert!(!data.is_empty(), "[{}] 应返回非空数据", code);

        // 关键断言: source 字符串必须是约定之一
        assert!(
            source == "tencent_qfq" || source == "eastmoney_qfq" || source == "rustdx_none",
            "[{}] 未知的 source: {}",
            code,
            source
        );

        // 关键断言: adjust 字段必须与 source 严格一致
        let expected_adjust = match source {
            "tencent_qfq" | "eastmoney_qfq" => AdjustType::Qfq,
            "rustdx_none" => AdjustType::None,
            _ => unreachable!(),
        };

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
async fn fallback_returns_sane_prices() {
    for &code in TEST_CODES {
        let (data, _) = match fetch_kline_with_fallback(code, 30).await {
            Ok(v) => v,
            Err(_) => continue,
        };

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

/// 验收 5: 创业板 (300750) 用 25% 阈值不误杀正常跳空
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn fallback_skips_data_with_extreme_gap() {
    // 验证: 如果三源数据都包含超过 max_gap_for(code) 的跳空, fallback 应 reject 触发下个源
    // (实际不可能构造, 这里只验证能拉到不超阈值的数据)
    let (data, _) = match fetch_kline_with_fallback("300750", 30).await {
        Ok(v) => v,
        Err(_) => return,
    };

    let mut sorted = data.clone();
    sorted.sort_by_key(|b| b.date);
    for w in sorted.windows(2) {
        let gap = ((w[1].open - w[0].close) / w[0].close * 100.0).abs();
        // 创业板阈值 25%, 真实数据不应超过 (除权除息日豁免, 但生产路径没喂 mark_ex_rights)
        assert!(
            gap <= 25.0,
            "创业板 300750 跳空 {:.2}% 超过 25% 阈值 (prev_close={:.3} cur_open={:.3})",
            gap,
            w[0].close,
            w[1].open
        );
    }
}
