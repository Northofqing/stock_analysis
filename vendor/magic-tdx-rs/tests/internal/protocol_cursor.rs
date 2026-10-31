use super::*;

#[test]
fn fixed_reads_report_field_and_offset() {
    let mut cursor = PacketCursor::at(&[1], 1).unwrap();
    let error = cursor.read_u16_le("record volume").unwrap_err();
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
    );
    assert!(error.to_string().contains("record volume at offset 1"));
}

#[test]
fn bounded_slice_never_reads_past_packet() {
    let mut cursor = PacketCursor::new(&[1, 2]);
    assert_eq!(cursor.read_slice(2, "payload").unwrap(), &[1, 2]);
    assert!(cursor.is_empty());
    assert!(cursor.read_u8("tail").is_err());
}

#[test]
fn valid_zero_is_not_a_decoder_failure() {
    let mut cursor = PacketCursor::new(&[0]);
    assert_eq!(cursor.read_tdx_varint("price").unwrap(), 0);
    assert_eq!(cursor.position(), 1);
}

#[test]
fn unterminated_and_overlong_varints_are_rejected() {
    for length in 1..=MAX_TDX_VARINT_BYTES {
        let bytes = vec![0x80; length];
        let mut cursor = PacketCursor::new(&bytes);
        let error = cursor.read_tdx_varint("price").unwrap_err();
        assert_eq!(
            error.error_code(),
            Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
        );
    }
}

#[test]
fn cursor_start_overflow_and_record_context_are_explicit() {
    let error = PacketCursor::at(&[1], 2).unwrap_err();
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
    );
    assert!(error.to_string().contains("cursor start at offset 2"));

    let mut cursor = PacketCursor::at(&[1], 1).unwrap();
    cursor.set_record(7);
    let error = cursor.read_slice(usize::MAX, "payload").unwrap_err();
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::RESPONSE_LENGTH_MISMATCH)
    );
    assert!(error.to_string().contains("in record 7"));
}

#[test]
fn every_fixed_width_reader_preserves_wire_bits() {
    let i32_value = -123_456_i32;
    let i64_value = -9_876_543_210_i64;
    let f32_value = 12.5_f32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x4433_2211_u32.to_le_bytes());
    bytes.extend_from_slice(&i32_value.to_le_bytes());
    bytes.extend_from_slice(&i64_value.to_le_bytes());
    bytes.extend_from_slice(&f32_value.to_le_bytes());

    let mut cursor = PacketCursor::new(&bytes);
    assert_eq!(cursor.read_u32_le("u32").unwrap(), 0x4433_2211);
    assert_eq!(cursor.read_i32_le("i32").unwrap(), i32_value);
    assert_eq!(cursor.read_i64_le("i64").unwrap(), i64_value);
    assert_eq!(cursor.read_f32_le("f32").unwrap(), f32_value);
    assert!(cursor.is_empty());
}

#[test]
fn signed_varints_preserve_positive_and_negative_values() {
    let mut cursor = PacketCursor::new(&[0x01, 0x41]);
    assert_eq!(cursor.read_tdx_varint("positive").unwrap(), 1);
    assert_eq!(cursor.read_tdx_varint("negative").unwrap(), -1);
    assert!(cursor.is_empty());
}
