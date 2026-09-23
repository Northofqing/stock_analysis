use super::*;

// --- parse_security_count ---

#[test]
fn test_security_count_empty() {
    assert!(parse_security_count(&[]).is_err());
}

#[test]
fn test_security_count_one_byte() {
    assert!(parse_security_count(&[0x01]).is_err());
}

#[test]
fn test_security_count_zero() {
    let result = parse_security_count(&[0x00, 0x00]).unwrap();
    assert_eq!(result, 0);
}

#[test]
fn test_security_count_normal() {
    let result = parse_security_count(&[0xE8, 0x03]).unwrap(); // 1000
    assert_eq!(result, 1000);
}

// --- parse_security_list ---

#[test]
fn test_security_list_empty_body() {
    assert!(parse_security_list(&[]).is_err());
}

#[test]
fn test_security_list_one_byte() {
    assert!(parse_security_list(&[0x01]).is_err());
}

#[test]
fn test_security_list_zero_count() {
    let result = parse_security_list(&[0x00, 0x00]).unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_security_list_truncated_record() {
    // count=1 but only 10 bytes (need 29)
    let mut data = vec![0x01, 0x00];
    data.extend_from_slice(&[0u8; 10]);
    let error = parse_security_list(&data).unwrap_err();
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
    );
    assert!(error.to_string().contains("record 0"));
}

#[test]
fn test_security_list_one_record() {
    // count=1, record_size=29
    let mut data = vec![0x01, 0x00];
    let mut record = vec![0u8; 29];
    // code: "600519\0"
    record[..6].copy_from_slice(b"600519");
    // name: GBK encoded "贵州茅台\0\0\0\0"
    let (gbk_bytes, _, _) = GBK.encode("贵州茅台");
    record[8..8 + gbk_bytes.len()].copy_from_slice(&gbk_bytes);
    data.extend_from_slice(&record);
    let result = parse_security_list(&data).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].code, "600519");
    assert_eq!(result[0].name, "贵州茅台");
}

#[test]
fn security_list_rejects_a_missing_later_record_atomically() {
    let mut data = vec![0x02, 0x00];
    let mut record = vec![0u8; 29];
    record[..6].copy_from_slice(b"600519");
    data.extend_from_slice(&record);

    let error = parse_security_list(&data).unwrap_err();
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
    );
    assert!(error.to_string().contains("record 1"));
}

#[test]
fn security_list_rejects_undeclared_trailing_bytes() {
    let error = parse_security_list(&[0, 0, 0]).unwrap_err();
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
    );
    assert!(error.to_string().contains("trailing bytes"));
}

// --- parse_security_bars ---

#[test]
fn test_security_bars_empty() {
    assert!(parse_security_bars(&[], 4).is_err());
}

#[test]
fn test_security_bars_zero_count() {
    let result = parse_security_bars(&[0x00, 0x00], 4).unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_security_bars_truncated() {
    // count=1 but only 5 bytes (need at least 16)
    let mut data = vec![0x01, 0x00];
    data.extend_from_slice(&[0u8; 5]);
    let error = parse_security_bars(&data, 4).unwrap_err();
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
    );
    assert!(error.to_string().contains("row 0"));
}

#[test]
fn test_security_bars_daily_format() {
    // category=4 (daily): u32 date + 4*price(var) + vol(4) + amount(4)
    // Build minimal: date=20260429, then 4 zero prices (0x00), vol=0, amount=0
    let mut data = vec![0x01, 0x00]; // count=1
    data.extend_from_slice(&20260429u32.to_le_bytes()); // date
    data.extend_from_slice(&[0x00; 16]); // 4 prices(var=1B each) + vol(4) + amount(4)
    let result = parse_security_bars(&data, 4).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].year, 2026);
    assert_eq!(result[0].month, 4);
    assert_eq!(result[0].day, 29);
}

#[test]
fn bar_parsers_accept_only_the_protocol_authorized_tail() {
    for trailing in [1_usize, 2, 3, 5] {
        let security = [vec![0, 0], vec![0; trailing]].concat();
        let error = parse_security_bars(&security, 4).unwrap_err();
        assert!(error.to_string().contains("unsupported trailing bytes"));

        let index = [vec![0, 0], vec![0; trailing]].concat();
        let error = parse_index_bars(&index, 4).unwrap_err();
        assert!(error.to_string().contains("unsupported trailing bytes"));
    }
    assert!(parse_security_bars(&[0, 0, 0, 0, 0, 0], 4)
        .unwrap()
        .is_empty());
    assert!(parse_index_bars(&[0, 0, 0, 0, 0, 0], 4).unwrap().is_empty());
}

// --- parse_index_bars ---

#[test]
fn test_index_bars_empty() {
    assert!(parse_index_bars(&[], 4).is_err());
}

#[test]
fn test_index_bars_zero_count() {
    let result = parse_index_bars(&[0x00, 0x00], 4).unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_index_bars_truncated() {
    let mut data = vec![0x01, 0x00];
    data.extend_from_slice(&[0u8; 10]);
    let error = parse_index_bars(&data, 4).unwrap_err();
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
    );
    assert!(error.to_string().contains("row 0"));
}

// --- parse_minute_time_data ---

#[test]
fn test_minute_time_empty() {
    assert!(parse_minute_time_data(&[], 1, "600519").is_err());
}

#[test]
fn test_minute_time_too_short() {
    assert!(parse_minute_time_data(&[0x00], 1, "600519").is_err());
}

#[test]
fn test_minute_time_zero_count() {
    // 头部: 2(count) + 2(padding) + 1(indicator) + 6(stock_code) + 2(unknown) = 13 bytes
    let body = [
        0x00, 0x00, 0x00, 0x00, 0x01, 0x36, 0x30, 0x30, 0x35, 0x31, 0x39, 0x00, 0x00,
    ];
    let result = parse_minute_time_data(&body, 1, "600519").unwrap();
    assert!(result.is_empty());

    let mut unsupported_tail = body.to_vec();
    unsupported_tail.push(0);
    assert!(parse_minute_time_data(&unsupported_tail, 1, "600519").is_err());
}

#[test]
fn minute_parsers_reject_price_overflow_and_negative_volume() {
    let maximum = [0xbf, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];
    let above_u32 = [0x80, 0x80, 0x80, 0x80, 0x20];

    let mut realtime_overflow = vec![3, 0];
    realtime_overflow.extend_from_slice(&[0; 11]);
    realtime_overflow.extend_from_slice(&maximum);
    realtime_overflow.extend_from_slice(&[0, 1]);
    realtime_overflow.extend_from_slice(&maximum);
    realtime_overflow.extend_from_slice(&[0, 1, 2, 0, 1]);
    assert!(parse_minute_time_data(&realtime_overflow, 1, "600519")
        .unwrap_err()
        .to_string()
        .contains("cumulative price overflow"));

    let mut realtime_negative_volume = vec![1, 0];
    realtime_negative_volume.extend_from_slice(&[0; 11]);
    realtime_negative_volume.extend_from_slice(&[10, 0, 0x41]);
    assert!(
        parse_minute_time_data(&realtime_negative_volume, 1, "600519")
            .unwrap_err()
            .to_string()
            .contains("volume is negative")
    );

    let mut realtime_negative_price = vec![1, 0];
    realtime_negative_price.extend_from_slice(&[0; 11]);
    realtime_negative_price.extend_from_slice(&[0x41, 0, 1]);
    assert!(
        parse_minute_time_data(&realtime_negative_price, 1, "600519")
            .unwrap_err()
            .to_string()
            .contains("cumulative price is negative")
    );

    let mut realtime_oversized_volume = vec![1, 0];
    realtime_oversized_volume.extend_from_slice(&[0; 11]);
    realtime_oversized_volume.extend_from_slice(&[10, 0]);
    realtime_oversized_volume.extend_from_slice(&above_u32);
    assert!(
        parse_minute_time_data(&realtime_oversized_volume, 1, "600519")
            .unwrap_err()
            .to_string()
            .contains("unsigned 32-bit domain")
    );

    let mut history_overflow = vec![0; 6];
    history_overflow.extend_from_slice(&maximum);
    history_overflow.extend_from_slice(&[0, 1]);
    history_overflow.extend_from_slice(&maximum);
    history_overflow.extend_from_slice(&[0, 1, 2, 0, 1]);
    assert!(
        parse_history_minute_time_data(&history_overflow, 1, "600519")
            .unwrap_err()
            .to_string()
            .contains("cumulative price overflow")
    );

    let history_negative_price = [vec![0; 6], vec![0x41, 0, 1]].concat();
    assert!(
        parse_history_minute_time_data(&history_negative_price, 1, "600519")
            .unwrap_err()
            .to_string()
            .contains("cumulative price is negative")
    );

    let history_negative_volume = [vec![0; 6], vec![10, 0, 0x41]].concat();
    assert!(
        parse_history_minute_time_data(&history_negative_volume, 1, "600519")
            .unwrap_err()
            .to_string()
            .contains("volume is negative")
    );

    let history_oversized_volume = [vec![0; 6], vec![10, 0], above_u32.to_vec()].concat();
    assert!(
        parse_history_minute_time_data(&history_oversized_volume, 1, "600519")
            .unwrap_err()
            .to_string()
            .contains("unsigned 32-bit domain")
    );
}

#[test]
fn minute_parsers_preserve_zero_volume_without_division() {
    let mut realtime = vec![1, 0];
    realtime.extend_from_slice(&[0; 11]);
    realtime.extend_from_slice(&[10, 0, 0]);
    let rows = parse_minute_time_data(&realtime, 1, "600519").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].price, rows[0].avg_price);
    assert_eq!(rows[0].vol, 0.0);

    let history = [vec![0; 6], vec![10, 0, 0]].concat();
    let rows = parse_history_minute_time_data(&history, 1, "600519").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].price, rows[0].avg_price);
    assert_eq!(rows[0].vol, 0.0);
}

// --- parse_history_minute_time_data ---

#[test]
fn test_history_minute_time_empty() {
    assert!(parse_history_minute_time_data(&[], 1, "600519").is_err());
}

#[test]
fn test_history_minute_time_short_header() {
    // Less than 6 bytes header
    assert!(parse_history_minute_time_data(&[0u8; 5], 1, "600519").is_err());
}

// --- parse_transaction_data ---

#[test]
fn test_transaction_empty() {
    assert!(parse_transaction_data(&[]).is_err());
}

#[test]
fn test_transaction_one_byte() {
    assert!(parse_transaction_data(&[0x01]).is_err());
}

#[test]
fn test_transaction_zero_count() {
    let result = parse_transaction_data(&[0x00, 0x00]).unwrap();
    assert!(result.is_empty());
}

fn current_transaction_row() -> [u8; 7] {
    [0x5a, 0x02, 10, 2, 3, 1, 0]
}

fn assert_transaction_length_mismatch(body: &[u8], message: &str) {
    let error = parse_transaction_data(body).unwrap_err();
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
    );
    assert!(error.to_string().contains(message), "{error}");
}

fn assert_transaction_type_mismatch(body: &[u8], message: &str) {
    let error = parse_transaction_data(body).unwrap_err();
    assert_eq!(error.error_code(), Some(ErrorCode::TYPE_MISMATCH));
    assert!(error.to_string().contains(message), "{error}");
}

#[test]
fn current_transaction_rejects_declared_short_record() {
    assert_transaction_length_mismatch(&[1, 0], "row 0 is truncated");

    let mut missing_reserved = vec![1, 0];
    missing_reserved.extend_from_slice(&current_transaction_row()[..6]);
    assert_transaction_length_mismatch(&missing_reserved, "row 0 is truncated");
}

#[test]
fn current_transaction_rejects_unterminated_variable_integer() {
    for (offset, field) in [
        (2, "price"),
        (3, "volume"),
        (4, "trade count"),
        (5, "trade side"),
        (6, "reserved"),
    ] {
        let mut packet = vec![1, 0];
        let mut row = current_transaction_row();
        row[offset..].fill(0x80);
        packet.extend_from_slice(&row);

        assert_transaction_length_mismatch(
            &packet,
            &format!("{field} has invalid variable-length framing"),
        );
    }

    let mut overlong = vec![1, 0, 0x5a, 0x02];
    overlong.extend_from_slice(&[0x80; 9]);
    overlong.push(0);
    overlong.extend_from_slice(&[0; 4]);
    assert_transaction_length_mismatch(&overlong, "price has invalid variable-length framing");
}

#[test]
fn current_transaction_rejects_second_truncated_record_atomically() {
    let mut packet = vec![2, 0];
    packet.extend_from_slice(&current_transaction_row());
    packet.extend_from_slice(&current_transaction_row()[..6]);

    assert_transaction_length_mismatch(&packet, "row 1 is truncated");
}

#[test]
fn current_transaction_rejects_zero_and_nonzero_count_trailing_bytes() {
    assert_transaction_length_mismatch(&[0, 0, 0], "1 trailing bytes");

    let mut packet = vec![1, 0];
    packet.extend_from_slice(&current_transaction_row());
    packet.push(0);
    assert_transaction_length_mismatch(&packet, "1 trailing bytes");
}

#[test]
fn current_transaction_accepts_exact_single_and_multiple_record_framing() {
    let mut one = vec![1, 0];
    one.extend_from_slice(&current_transaction_row());
    let records = parse_transaction_data(&one).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].time, "10:02");
    assert_eq!(records[0].price, 0.1);
    assert_eq!(records[0].num, 3);

    let mut two = vec![2, 0];
    two.extend_from_slice(&current_transaction_row());
    two.extend_from_slice(&current_transaction_row());
    let records = parse_transaction_data(&two).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].price, 0.2);

    let valid_multibyte = [1, 0, 0x5a, 0x02, 0x80, 0x01, 2, 3, 1, 0];
    let records = parse_transaction_data(&valid_multibyte).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].price, 0.64);
}

#[test]
fn current_transaction_rejects_negative_or_oversized_unsigned_fields() {
    for (offset, field) in [
        (3, "volume"),
        (4, "trade count"),
        (5, "trade side"),
        (6, "reserved"),
    ] {
        let mut packet = vec![1, 0];
        let mut row = current_transaction_row();
        row[offset] = 0x41;
        packet.extend_from_slice(&row);
        assert_transaction_type_mismatch(
            &packet,
            &format!("{field} is outside the unsigned 32-bit domain"),
        );
    }

    let mut oversized_count = vec![1, 0, 0x5a, 0x02, 10, 2];
    oversized_count.extend_from_slice(&[0xbf, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]);
    oversized_count.extend_from_slice(&[1, 0]);
    assert_transaction_type_mismatch(
        &oversized_count,
        "trade count is outside the unsigned 32-bit domain",
    );

    let mut invalid_side = vec![1, 0];
    let mut row = current_transaction_row();
    row[5] = 3;
    invalid_side.extend_from_slice(&row);
    assert_transaction_type_mismatch(&invalid_side, "trade side 3 is outside 0..=2");
}

#[test]
fn current_transaction_rejects_negative_and_overflowed_cumulative_prices() {
    let negative_price = [1, 0, 0x5a, 0x02, 0x41, 2, 3, 1, 0];
    assert_transaction_type_mismatch(&negative_price, "cumulative price is negative");

    let max_positive = [0xbf, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];
    let mut overflow = vec![3, 0];
    for price in [&max_positive[..], &max_positive[..], &[2][..]] {
        overflow.extend_from_slice(&[0x5a, 0x02]);
        overflow.extend_from_slice(price);
        overflow.extend_from_slice(&[2, 3, 1, 0]);
    }
    assert_transaction_type_mismatch(&overflow, "cumulative price overflow");
}

// --- parse_history_transaction_data ---

#[test]
fn test_history_transaction_short() {
    assert!(parse_history_transaction_data(&[0u8; 5]).is_err());
}

#[test]
fn test_history_transaction_empty_body() {
    let result = parse_history_transaction_data(&[0u8; 6]).unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_history_transaction_keeps_minimum_size_last_record() {
    let body = [1, 0, 0, 0, 0, 0, 0x5a, 0x02, 10, 1, 0, 0];
    let result = parse_history_transaction_data(&body).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].time, "10:02");
    assert_eq!(result[0].price, 0.1);
    assert_eq!(result[0].vol, 1.0);
    assert_eq!(result[0].buyorsell, 0);
}

#[test]
fn test_history_transaction_rejects_truncated_record() {
    let body = [1, 0, 0, 0, 0, 0, 0x5a, 0x02, 10, 1, 0];
    assert!(parse_history_transaction_data(&body).is_err());
}

#[test]
fn history_transaction_rejects_invalid_domains_and_cumulative_overflow() {
    for (offset, message) in [
        (9, "volume is outside the unsigned 32-bit domain"),
        (10, "trade side is outside the unsigned 32-bit domain"),
        (11, "reserved is outside the unsigned 32-bit domain"),
    ] {
        let mut body = [1, 0, 0, 0, 0, 0, 0x5a, 0x02, 10, 1, 0, 0];
        body[offset] = 0x41;
        assert!(parse_history_transaction_data(&body)
            .unwrap_err()
            .to_string()
            .contains(message));
    }

    let invalid_side = [1, 0, 0, 0, 0, 0, 0x5a, 0x02, 10, 1, 3, 0];
    assert!(parse_history_transaction_data(&invalid_side)
        .unwrap_err()
        .to_string()
        .contains("trade side 3 is outside 0..=2"));

    let maximum = [0xbf, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];
    let mut overflow = vec![3, 0, 0, 0, 0, 0];
    for price in [&maximum[..], &maximum[..], &[2][..]] {
        overflow.extend_from_slice(&[0x5a, 0x02]);
        overflow.extend_from_slice(price);
        overflow.extend_from_slice(&[1, 0, 0]);
    }
    assert!(parse_history_transaction_data(&overflow)
        .unwrap_err()
        .to_string()
        .contains("cumulative price overflow"));
}

#[test]
fn history_transaction_rejects_negative_price_and_trailing_bytes() {
    let negative_price = [1, 0, 0, 0, 0, 0, 0x5a, 0x02, 0x41, 1, 0, 0];
    let error = parse_history_transaction_data(&negative_price).unwrap_err();
    assert!(error.to_string().contains("cumulative price is negative"));

    let mut trailing = [1, 0, 0, 0, 0, 0, 0x5a, 0x02, 10, 1, 0, 0].to_vec();
    trailing.push(0);
    let error = parse_history_transaction_data(&trailing).unwrap_err();
    assert!(error.to_string().contains("trailing bytes"));
}

// --- parse_security_quotes ---

#[test]
fn test_quotes_empty() {
    assert!(parse_security_quotes(&[]).is_err());
}

#[test]
fn test_quotes_too_short() {
    assert!(parse_security_quotes(&[0x00, 0x00, 0x00]).is_err());
}

#[test]
fn test_quotes_zero_count() {
    // b1 cb (2 bytes) + count=0 (2 bytes)
    let result = parse_security_quotes(&[0x00, 0x00, 0x00, 0x00]).unwrap();
    assert!(result.is_empty());
}

#[test]
fn zero_quote_count_rejects_undeclared_trailing_bytes() {
    let error = parse_security_quotes(&[0, 0, 0, 0, 0]).unwrap_err();
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
    );
    assert!(error.to_string().contains("trailing bytes"));
}

// --- format_servertime (reversed_bytes0 解码) ---
// 参考: pytdx _format_time (rainx/pytdx issue #187)。
// 样本来源: 2026-08-06 对 218.75.126.9:7709 的 live wire 抓包 (午休冻结 ~11:29)
// + issue #187 官方样本。

#[test]
fn test_format_servertime_live_wire_captures() {
    // 2026-08-06 live wire: 605178 → 11:29:21.810 (整秒向下取整)
    assert_eq!(format_servertime(11293635), "11:29:21");
    // 002415 → 11:29:37.644
    assert_eq!(format_servertime(11296274), "11:29:37");
    // 000001 → 11:29:53.304
    assert_eq!(format_servertime(11298884), "11:29:53");
}

#[test]
fn test_format_servertime_issue187_samples() {
    // issue #187 官方样本: 14:59:57.368 (分钟位 ≥ 60 → else 分支)
    assert_eq!(format_servertime(14999269), "14:59:57");
    // 10:07:20.292
    assert_eq!(format_servertime(10073382), "10:07:20");
    // 10:05:09.378
    assert_eq!(format_servertime(10051563), "10:05:09");
    // 10:05:16.800
    assert_eq!(format_servertime(10052800), "10:05:16");
}

#[test]
fn test_format_servertime_leading_zero_hour() {
    // 09:15:00.000 → 数字 9150000 (前导 0 丢失), 需补齐后解码
    assert_eq!(format_servertime(9150000), "09:15:00");
    // 09:30:00.000
    assert_eq!(format_servertime(9300000), "09:30:00");
}

#[test]
fn test_format_servertime_max_fraction_stays_in_second_bound() {
    // last4 = 9999 → 59.994 s: 向下取整 → 59, 绝不产生非法的 ":60"
    assert_eq!(format_servertime(10059999), "10:05:59");
}

#[test]
fn test_format_servertime_invalid_returns_empty() {
    // 非正 → 空串, 不伪造时间证据
    assert_eq!(format_servertime(0), "");
    assert_eq!(format_servertime(-1), "");
    // 小时超 23
    assert_eq!(format_servertime(24000000), "");
}

#[test]
fn test_parse_security_quotes_servertime_from_live_wire() {
    // 2026-08-06 live wire 抓包 (218.75.126.9:7709, 605178, market=1):
    // body 78 字节 = [01 06] + count=1 + 完整 74 字节 quote 行。
    // 同一份字节里, 除 servertime 外全部字段与已知成交一致
    // (price=63.66, last_close=64.24, open=64.37, high=66.78, low=62.80)。
    let body: Vec<u8> = vec![
        0x01, 0x06, 0x01, 0x00, //
        0x01, 0x36, 0x30, 0x35, 0x31, 0x37, 0x38, 0x11, 0x09, 0x9e, 0x63, 0x3a, //
        0x87, 0x01, 0xb8, 0x04, 0xd6, 0x01, 0x83, 0xcf, 0xe2, 0x0a, 0xde, 0x63, //
        0xbe, 0x87, 0x07, 0x0c, 0x85, 0x9c, 0xb2, 0x4d, 0xb6, 0xbf, 0x03, 0x88, //
        0xc8, 0x03, 0x55, 0x8a, 0xae, 0x08, 0x00, 0x15, 0x01, 0x12, 0x41, 0x16, //
        0x02, 0x01, 0x45, 0x1f, 0x0d, 0x02, 0x46, 0x22, 0xbc, 0x02, 0x02, 0x47, //
        0x84, 0x01, 0x0a, 0x06, 0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc6, 0xff, //
        0x11, 0x09,
    ];
    let quotes = parse_security_quotes(&body).unwrap();
    assert_eq!(quotes.len(), 1);
    let quote = &quotes[0];
    assert_eq!(quote.code, "605178");
    assert!((quote.price - 63.66).abs() < 0.01);
    assert!((quote.last_close - 64.24).abs() < 0.01);
    assert!((quote.open - 64.37).abs() < 0.01);
    assert!((quote.high - 66.78).abs() < 0.01);
    assert!((quote.low - 62.80).abs() < 0.01);
    // reversed_bytes0 = 11293635 → 11:29:21.810 → 整秒 21
    assert_eq!(quote.servertime, "11:29:21");
}

// --- parse_finance_info ---

fn finance_record(market: u8, code: &str, ipo_date: u32) -> Vec<u8> {
    let mut data = vec![0u8; 143];
    data[0] = market;
    data[1..7].copy_from_slice(code.as_bytes());
    data[19..23].copy_from_slice(&ipo_date.to_le_bytes());
    data
}

fn finance_packet(market: u8, code: &str, ipo_date: u32) -> Vec<u8> {
    let mut data = 1u16.to_le_bytes().to_vec();
    data.extend_from_slice(&finance_record(market, code, ipo_date));
    data
}

#[test]
fn test_finance_empty() {
    assert!(parse_finance_info(&[], 1, "600519").is_err());
}

#[test]
fn test_finance_short() {
    assert!(parse_finance_info(&[0u8; 50], 1, "600519").is_err());
}

#[test]
fn test_finance_valid() {
    // 9 bytes header + 136 bytes struct = 145 bytes minimum
    let mut data = finance_packet(1, "600519", 20010827);
    // Set liutongguben (first f32 at pos 9)
    let liutongguben: f32 = 10.0;
    data[9..13].copy_from_slice(&liutongguben.to_le_bytes());
    let result = parse_finance_info(&data, 1, "600519").unwrap();
    assert_eq!(result.code, "600519");
    assert_eq!(result.market, 1);
    // raw value, no unit conversion
    assert!((result.liutongguben - 10.0).abs() < 0.1);
    assert_eq!(result.ipo_date, 20010827);
}

#[test]
fn finance_rejects_response_identity_mismatch() {
    assert!(parse_finance_info(&finance_packet(0, "600519", 20010827), 1, "600519").is_err());
    assert!(parse_finance_info(&finance_packet(1, "000001", 20010827), 1, "600519").is_err());
}

#[test]
fn finance_rejects_malformed_and_future_ipo_dates() {
    assert!(parse_finance_info(&finance_packet(1, "600519", 20260231), 1, "600519").is_err());
    assert!(parse_finance_info(&finance_packet(1, "600519", 99991231), 1, "600519").is_err());
    assert!(parse_finance_info(&finance_packet(1, "600519", 0), 1, "600519").is_ok());
}

#[test]
fn finance_requires_one_declared_record_and_exact_framing() {
    assert!(parse_finance_info(&0u16.to_le_bytes(), 1, "600519").is_err());

    let mut declared_two = 2u16.to_le_bytes().to_vec();
    declared_two.extend_from_slice(&finance_record(1, "600519", 20010827));
    declared_two.extend_from_slice(&finance_record(1, "600519", 20010827));
    assert!(parse_finance_info(&declared_two, 1, "600519").is_err());

    let mut undeclared_second = finance_packet(1, "600519", 20010827);
    undeclared_second.extend_from_slice(&finance_record(1, "600519", 20010827));
    assert!(parse_finance_info(&undeclared_second, 1, "600519").is_err());

    let mut trailing = finance_packet(1, "600519", 20010827);
    trailing.push(0);
    assert!(parse_finance_info(&trailing, 1, "600519").is_err());
}

#[test]
fn finance_rejects_non_ascii_identity_and_non_finite_fields() {
    let mut non_ascii = finance_packet(1, "600519", 20010827);
    non_ascii[3] = 0xff;
    assert!(parse_finance_info(&non_ascii, 1, "600519")
        .unwrap_err()
        .to_string()
        .contains("not valid ASCII"));

    let mut non_finite_circulating_shares = finance_packet(1, "600519", 20010827);
    non_finite_circulating_shares[9..13].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(
        parse_finance_info(&non_finite_circulating_shares, 1, "600519")
            .unwrap_err()
            .to_string()
            .contains("non-finite circulating-share")
    );

    let mut non_finite_finance_field = finance_packet(1, "600519", 20010827);
    non_finite_finance_field[25..29].copy_from_slice(&f32::INFINITY.to_le_bytes());
    assert!(parse_finance_info(&non_finite_finance_field, 1, "600519")
        .unwrap_err()
        .to_string()
        .contains("non-finite numeric field"));
}

#[test]
fn finance_rejects_non_eight_digit_ipo_date() {
    let error =
        parse_finance_info(&finance_packet(1, "600519", 100_000_000), 1, "600519").unwrap_err();
    assert!(error
        .to_string()
        .contains("must contain exactly eight digits"));
}

// --- parse_xdxr_info ---

fn xdxr_packet_with_rows(
    outer_market: u8,
    outer_code: &str,
    rows: &[(u8, &str, u32, u8, f32)],
) -> Vec<u8> {
    let mut data = vec![0u8; 11];
    data[2] = outer_market;
    data[3..9].copy_from_slice(outer_code.as_bytes());
    data[9..11].copy_from_slice(&(rows.len() as u16).to_le_bytes());
    for (market, code, date, category, value) in rows {
        data.push(*market);
        data.extend_from_slice(code.as_bytes());
        data.push(0);
        data.extend_from_slice(&date.to_le_bytes());
        data.push(*category);
        let mut terms = [0u8; 16];
        terms[8..12].copy_from_slice(&value.to_le_bytes());
        data.extend_from_slice(&terms);
    }
    data
}

fn xdxr_packet(date: u32, category: u8, value: f32) -> Vec<u8> {
    xdxr_packet_with_rows(1, "600519", &[(1, "600519", date, category, value)])
}

fn xdxr_packet_with_terms(date: u32, category: u8, terms: [u8; 16]) -> Vec<u8> {
    let mut packet = xdxr_packet(date, category, 0.0);
    packet[24..40].copy_from_slice(&terms);
    packet
}

#[test]
fn test_xdxr_empty() {
    assert!(parse_xdxr_info(&[]).is_err());
}

#[test]
fn test_xdxr_short() {
    assert!(parse_xdxr_info(&[0u8; 10]).is_err());
}

#[test]
fn test_xdxr_zero_count() {
    /* 9 bytes header + 2 bytes count=0 */
    let data = vec![0u8; 11];
    let result = parse_xdxr_info(&data).unwrap();
    assert!(result.is_empty());
}

#[test]
fn xdxr_rejects_declared_count_truncation_and_trailing_bytes() {
    let mut truncated = xdxr_packet(20260101, 11, 2.0);
    truncated.pop();
    assert!(parse_xdxr_info(&truncated).is_err());

    let mut trailing = xdxr_packet(20260101, 11, 2.0);
    trailing.push(0);
    assert!(parse_xdxr_info(&trailing).is_err());
}

#[test]
fn xdxr_rejects_invalid_dates_and_non_finite_values() {
    assert!(parse_xdxr_info(&xdxr_packet(20260231, 11, 2.0)).is_err());
    assert!(parse_xdxr_info(&xdxr_packet(99991231, 11, 2.0)).is_err());
    assert!(parse_xdxr_info(&xdxr_packet(20260101, 11, f32::NAN)).is_err());
    assert!(parse_xdxr_info(&xdxr_packet(20260101, 11, f32::INFINITY)).is_err());
}

#[test]
fn xdxr_request_parser_rejects_response_identity_mismatch() {
    let packet = xdxr_packet(20260101, 11, 2.0);
    assert!(parse_xdxr_info_for(&packet, 1, "600519").is_ok());
    assert!(parse_xdxr_info_for(&packet, 0, "600519").is_err());
    assert!(parse_xdxr_info_for(&packet, 1, "000001").is_err());
}

#[test]
fn xdxr_request_parser_rejects_each_mismatched_row_identity_atomically() {
    let wrong_market = xdxr_packet_with_rows(1, "600519", &[(0, "600519", 20260101, 11, 2.0)]);
    assert!(parse_xdxr_info_for(&wrong_market, 1, "600519").is_err());

    let wrong_code = xdxr_packet_with_rows(1, "600519", &[(1, "000001", 20260101, 11, 2.0)]);
    assert!(parse_xdxr_info_for(&wrong_code, 1, "600519").is_err());

    let mixed = xdxr_packet_with_rows(
        1,
        "600519",
        &[
            (1, "600519", 20250101, 11, 2.0),
            (1, "000001", 20260101, 11, 2.0),
        ],
    );
    assert!(parse_xdxr_info_for(&mixed, 1, "600519").is_err());
}

#[test]
fn xdxr_empty_response_still_requires_matching_outer_identity() {
    let empty = xdxr_packet_with_rows(1, "600519", &[]);
    assert!(parse_xdxr_info_for(&empty, 1, "600519").unwrap().is_empty());
    assert!(parse_xdxr_info_for(&empty, 0, "600519").is_err());
    assert!(parse_xdxr_info_for(&empty, 1, "000001").is_err());
}

#[test]
fn xdxr_request_parser_rejects_short_and_non_ascii_outer_identity() {
    assert!(parse_xdxr_info_for(&[0u8; 8], 1, "600519")
        .unwrap_err()
        .to_string()
        .contains("too short for XDXR identity"));

    let mut non_ascii = xdxr_packet(20260101, 11, 2.0);
    non_ascii[3] = 0xff;
    assert!(parse_xdxr_info_for(&non_ascii, 1, "600519")
        .unwrap_err()
        .to_string()
        .contains("not valid ASCII"));
}

#[test]
fn xdxr_decodes_distribution_capital_and_warrant_term_layouts() {
    let mut distribution_terms = [0u8; 16];
    for (index, value) in [1.0f32, 2.0, 3.0, 4.0].into_iter().enumerate() {
        let start = index * 4;
        distribution_terms[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }
    let distribution =
        parse_xdxr_info(&xdxr_packet_with_terms(20260101, 1, distribution_terms)).unwrap();
    assert_eq!(distribution[0].name, "除权除息");
    assert_eq!(distribution[0].fenhong, Some(1.0));
    assert_eq!(distribution[0].peigujia, Some(2.0));
    assert_eq!(distribution[0].songzhuangu, Some(3.0));
    assert_eq!(distribution[0].peigu, Some(4.0));

    let mut capital_terms = [0u8; 16];
    for (index, value) in [1u32, 2, 3, 4].into_iter().enumerate() {
        let start = index * 4;
        capital_terms[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }
    let capital = parse_xdxr_info(&xdxr_packet_with_terms(20260101, 2, capital_terms)).unwrap();
    assert_eq!(capital[0].name, "送配股上市");
    assert!(capital[0].panqianliutong.is_some_and(|value| value > 0.0));
    assert!(capital[0].panhouliutong.is_some_and(|value| value > 0.0));
    assert!(capital[0].qianzongguben.is_some_and(|value| value > 0.0));
    assert!(capital[0].houzongguben.is_some_and(|value| value > 0.0));

    let mut warrant_terms = [0u8; 16];
    warrant_terms[0..4].copy_from_slice(&30.5f32.to_le_bytes());
    warrant_terms[8..12].copy_from_slice(&2.5f32.to_le_bytes());
    let warrant = parse_xdxr_info(&xdxr_packet_with_terms(20260101, 13, warrant_terms)).unwrap();
    assert_eq!(warrant[0].name, "送认购权证");
    assert_eq!(warrant[0].xingquanjia, Some(30.5));
    assert_eq!(warrant[0].fenshu, Some(2.5));
}

#[test]
fn xdxr_rejects_non_finite_distribution_and_warrant_terms() {
    let mut distribution_terms = [0u8; 16];
    distribution_terms[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(
        parse_xdxr_info(&xdxr_packet_with_terms(20260101, 1, distribution_terms))
            .unwrap_err()
            .to_string()
            .contains("non-finite numeric field")
    );

    let mut warrant_terms = [0u8; 16];
    warrant_terms[0..4].copy_from_slice(&f32::INFINITY.to_le_bytes());
    assert!(
        parse_xdxr_info(&xdxr_packet_with_terms(20260101, 13, warrant_terms))
            .unwrap_err()
            .to_string()
            .contains("non-finite numeric field")
    );
}

// --- parse_block_info_meta ---

#[test]
fn test_block_meta_empty() {
    assert!(parse_block_info_meta(&[]).is_err());
}

#[test]
fn test_block_meta_short() {
    assert!(parse_block_info_meta(&[0u8; 30]).is_err());
}

#[test]
fn test_block_meta_valid() {
    let mut data = vec![0u8; 38];
    data[0..4].copy_from_slice(&1000u32.to_le_bytes()); // size
    data[5..37].copy_from_slice(&[0xAB; 32]); // hash
    let result = parse_block_info_meta(&data).unwrap();
    assert_eq!(result.size, 1000);
    assert_eq!(result.hash_value.len(), 64); // 32 bytes * 2 hex chars
}

// --- parse_block_info ---

#[test]
fn test_block_info_empty() {
    assert!(parse_block_info(&[]).is_err());
}

#[test]
fn test_block_info_short_header() {
    assert!(parse_block_info(&[0u8; 3]).is_err());
}

#[test]
fn test_block_info_complete_empty_payload() {
    assert!(parse_block_info(&[0u8; 4]).unwrap().is_empty());
}

#[test]
fn test_block_info_valid() {
    let mut data = vec![0u8; 4]; // 4 byte header
    data.extend_from_slice(&[0x42; 10]); // 10 bytes payload
    let result = parse_block_info(&data).unwrap();
    assert_eq!(result.len(), 10);
    assert!(result.iter().all(|&b| b == 0x42));
}
