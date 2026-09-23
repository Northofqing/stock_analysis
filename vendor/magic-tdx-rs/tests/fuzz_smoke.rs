use magic_tdx_rs::protocol::parsers::{
    parse_finance_info, parse_history_minute_time_data, parse_security_bars, parse_security_quotes,
    parse_transaction_data, parse_xdxr_info,
};
use magic_tdx_rs::reader::{
    block::{parse_block, parse_block_group},
    daily_bar::parse_daily_bar,
    financial::parse_financial,
    min_bar::{parse_lc_min_bar, parse_min_bar},
};
use std::panic::{catch_unwind, AssertUnwindSafe};

#[test]
fn truncated_inputs_never_panic() {
    for len in 0..=64 {
        let bytes: Vec<u8> = (0..len).map(|v| (v as u8).wrapping_mul(37)).collect();
        let bars = catch_unwind(AssertUnwindSafe(|| parse_security_bars(&bytes, 4)));
        let quotes = catch_unwind(AssertUnwindSafe(|| parse_security_quotes(&bytes)));
        let minute = catch_unwind(AssertUnwindSafe(|| {
            parse_history_minute_time_data(&bytes, 1, "600519")
        }));
        let trades = catch_unwind(AssertUnwindSafe(|| parse_transaction_data(&bytes)));
        let finance = catch_unwind(AssertUnwindSafe(|| parse_finance_info(&bytes, 1, "600519")));
        let xdxr = catch_unwind(AssertUnwindSafe(|| parse_xdxr_info(&bytes)));
        let daily = catch_unwind(AssertUnwindSafe(|| parse_daily_bar(&bytes, 1.0)));
        let min = catch_unwind(AssertUnwindSafe(|| parse_min_bar(&bytes)));
        let lcmin = catch_unwind(AssertUnwindSafe(|| parse_lc_min_bar(&bytes)));
        let financial = catch_unwind(AssertUnwindSafe(|| parse_financial(&bytes)));
        let block = catch_unwind(AssertUnwindSafe(|| parse_block(&bytes)));
        let block_group = catch_unwind(AssertUnwindSafe(|| parse_block_group(&bytes)));
        if let Ok(Ok(records)) = &bars {
            assert_eq!(
                records.len(),
                u16::from_le_bytes([bytes[0], bytes[1]]) as usize,
                "bar parser returned a partial declared batch at length {len}"
            );
        }
        if let Ok(Ok(records)) = &quotes {
            assert_eq!(
                records.len(),
                u16::from_le_bytes([bytes[2], bytes[3]]) as usize,
                "quote parser returned a partial declared batch at length {len}"
            );
        }
        assert!(bars.is_ok(), "bars panic at length {len}");
        assert!(quotes.is_ok(), "quotes panic at length {len}");
        assert!(minute.is_ok(), "minute panic at length {len}");
        assert!(trades.is_ok(), "trades panic at length {len}");
        assert!(finance.is_ok(), "finance panic at length {len}");
        assert!(xdxr.is_ok(), "xdxr panic at length {len}");
        assert!(daily.is_ok(), "daily reader panic at length {len}");
        assert!(min.is_ok(), "minute reader panic at length {len}");
        assert!(lcmin.is_ok(), "lc minute reader panic at length {len}");
        assert!(financial.is_ok(), "financial reader panic at length {len}");
        assert!(block.is_ok(), "block reader panic at length {len}");
        assert!(
            block_group.is_ok(),
            "block group reader panic at length {len}"
        );
    }
}
