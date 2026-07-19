use super::*;

fn overnight_quote(code: &str, change_pct: f64) -> YahooQuote {
    YahooQuote {
        code: code.to_string(),
        price: None,
        change_pct: Some(change_pct),
        volume: None,
        previous_close: None,
    }
}

#[test]
fn complete_quote_row_executes_every_domain_validator() {
    let response: QuoteResponse = serde_json::from_str(
        r#"{"quoteResponse":{"result":[{"symbol":"TEST_CODE_000001","regularMarketPrice":10.5,"regularMarketChangePercent":-2.5,"regularMarketVolume":1234.0,"regularMarketPreviousClose":10.8}]}}"#,
    )
    .expect("complete local Yahoo fixture");
    let symbol_map = std::collections::HashMap::from([(
        "TEST_CODE_000001".to_string(),
        "TEST_CODE_000001".to_string(),
    )]);

    let quotes = parse_quotes(response, &symbol_map).expect("complete row must validate");

    assert_eq!(quotes[0].price, Some(10.5));
    assert_eq!(quotes[0].change_pct, Some(-2.5));
    assert_eq!(quotes[0].volume, Some(1234.0));
    assert_eq!(quotes[0].previous_close, Some(10.8));
}

#[test]
fn nonflat_overnight_snapshot_preserves_largest_move_and_fx_direction() {
    let negative = vec![
        overnight_quote("^IXIC", -2.0),
        overnight_quote("^DJI", 1.0),
        overnight_quote("^GSPC", -0.5),
        overnight_quote("CNY=X", -0.25),
    ];
    let (us, fx) = format_overnight_data(&negative).expect("complete negative snapshot");
    assert_eq!(us, "-2.0% (纳-2.0% 道+1.0% 标-0.5%)");
    assert_eq!(fx, "-0.25% (USD/CNY)");

    let positive = vec![
        overnight_quote("^IXIC", 0.4),
        overnight_quote("^DJI", 1.3),
        overnight_quote("^GSPC", -0.2),
        overnight_quote("CNY=X", 0.18),
    ];
    let (us, fx) = format_overnight_data(&positive).expect("complete positive snapshot");
    assert_eq!(us, "+1.3% (纳+0.4% 道+1.3% 标-0.2%)");
    assert_eq!(fx, "+0.18% (USD/CNY)");
}
