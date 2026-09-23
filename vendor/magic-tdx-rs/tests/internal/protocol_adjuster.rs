use super::*;

#[test]
fn test_calc_qfq_factor_cash_div() {
    // 简单分红: 前收盘 46.90, 分红 1.0/股
    let parts = FactorParts {
        div_per_share: 1.0,
        bonus_ratio: 0.0,
        rights_ratio: 0.0,
        rights_price: 0.0,
    };
    let factor = calc_qfq_factor(46.90, &parts);
    let expected = (46.90 - 1.0) / 46.90;
    assert!((factor - expected).abs() < 1e-10);
}

#[test]
fn test_calc_qfq_factor_bonus() {
    // 10送10: songzhuangu=10.0(每10股送10股)
    let parts = FactorParts {
        div_per_share: 0.0,
        bonus_ratio: 1.0,
        rights_ratio: 0.0,
        rights_price: 0.0,
    };
    let factor = calc_qfq_factor(20.0, &parts);
    // factor = 20 / (20 * 2) = 0.5
    assert!((factor - 0.5).abs() < 1e-10);
}

#[test]
fn test_adjust_no_event() {
    let mut bars = vec![SecurityBar {
        open: 10.0,
        close: 11.0,
        high: 12.0,
        low: 9.0,
        vol: 100.0,
        amount: 1000.0,
        year: 2025,
        month: 6,
        day: 15,
        hour: 0,
        minute: 0,
        datetime: "2025-06-15".into(),
    }];
    let orig = bars[0].open;
    adjust_security_bars(&mut bars, &[], &[], FqType::Qfq);
    assert!((bars[0].open - orig).abs() < 1e-10);
}

#[test]
fn test_find_close_before_fallback_to_context() {
    // bars 中无日期早于事件的数据, 应回退到 context
    let bars = vec![SecurityBar {
        open: 20.0,
        close: 21.0,
        high: 22.0,
        low: 19.0,
        vol: 100.0,
        amount: 1000.0,
        year: 2025,
        month: 6,
        day: 15,
        hour: 0,
        minute: 0,
        datetime: "".into(),
    }];
    let context = vec![SecurityBar {
        open: 10.0,
        close: 11.0,
        high: 12.0,
        low: 9.0,
        vol: 100.0,
        amount: 1000.0,
        year: 2024,
        month: 6,
        day: 15,
        hour: 0,
        minute: 0,
        datetime: "".into(),
    }];
    // event at 2025-01-01 — bars 中所有数据 > 2025-01-01, 应回退到 context
    let result = find_close_before_event(&bars, &context, 20250101);
    assert!(result.is_some());
    assert!((result.unwrap() - 11.0).abs() < 1e-10);
}

#[test]
fn test_adjust_context_hfq() {
    // 后复权场景: events 在 bars 之前, close_before 从 context 获取
    let context = vec![SecurityBar {
        open: 100.0,
        close: 100.0,
        high: 101.0,
        low: 99.0,
        vol: 1000.0,
        amount: 100000.0,
        year: 2024,
        month: 5,
        day: 1,
        hour: 0,
        minute: 0,
        datetime: "".into(),
    }];
    let mut bars = vec![
        SecurityBar {
            open: 103.0,
            close: 105.0,
            high: 106.0,
            low: 102.0,
            vol: 1000.0,
            amount: 100000.0,
            year: 2024,
            month: 9,
            day: 1,
            hour: 0,
            minute: 0,
            datetime: "".into(),
        },
        SecurityBar {
            open: 110.0,
            close: 112.0,
            high: 113.0,
            low: 109.0,
            vol: 1000.0,
            amount: 100000.0,
            year: 2025,
            month: 3,
            day: 1,
            hour: 0,
            minute: 0,
            datetime: "".into(),
        },
    ];
    // Event at 2024-06-15: fenhong=5.0 元/10股 = 0.5 元/股
    // close_before from context: 2024-05-01 close=100.0
    let xdxr = vec![XdXrInfo {
        category: 1,
        year: 2024,
        month: 6,
        day: 15,
        name: String::new(),
        fenhong: Some(5.0),
        songzhuangu: Some(0.0),
        peigu: Some(0.0),
        peigujia: Some(0.0),
        suogu: Some(0.0),
        panqianliutong: None,
        panhouliutong: None,
        qianzongguben: None,
        houzongguben: None,
        fenshu: None,
        xingquanjia: None,
    }];
    // factor = (100.0 - 0.5) / 100.0 = 0.995
    // HFQ: bars after the event get cum *= 1/factor = 1.005025...
    // Both bars are after the event, so both get adjusted
    let expected_cum = 1.0 / 0.995;
    adjust_security_bars(&mut bars, &context, &xdxr, FqType::Hfq);
    assert!(
        (bars[0].close - 105.0 * expected_cum).abs() < 0.01,
        "expected {}, got {}",
        105.0 * expected_cum,
        bars[0].close
    );
    assert!(
        (bars[1].close - 112.0 * expected_cum).abs() < 0.01,
        "expected {}, got {}",
        112.0 * expected_cum,
        bars[1].close
    );
}
