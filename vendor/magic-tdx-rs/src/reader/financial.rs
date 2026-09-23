use crate::error::{Result, TdxError};

const HEADER_SIZE: usize = 20; // 2+4+2+12 = 20 bytes for <hI1H3L>
const INDEX_ITEM_SIZE: usize = 11; // 6 code + 1 sep + 4 offset

#[derive(Debug, Clone, serde::Serialize)]
pub struct FinancialRecord {
    pub code: String,
    pub report_date: u32,
    pub fields: Vec<f32>,
}

/// 解析 gpcw*.dat 二进制财务数据
///
/// 格式:
///   Header (20 bytes): <1hI1H3L>
///     - i16: record type
///     - u32: report_date
///     - u16: max_count (stock count)
///     - u32: reserved
///     - u32: report_size (bytes per stock's float fields)
///     - u32: reserved
///
///   Stock Index (11 bytes each × max_count): <6s1c1L>
///     - [u8; 6]: stock code (UTF-8, null-padded)
///     - u8: separator (0x00)
///     - u32: file offset to report data
///
///   Report Data: at each offset, report_size/4 little-endian f32 values
pub fn parse_financial(data: &[u8]) -> Result<Vec<FinancialRecord>> {
    if data.len() < HEADER_SIZE {
        return Err(TdxError::InvalidData(
            "Financial file too small for header".into(),
        ));
    }

    // Parse header
    let _record_type = i16::from_le_bytes([data[0], data[1]]);
    let report_date = u32::from_le_bytes([data[2], data[3], data[4], data[5]]);
    let max_count = u16::from_le_bytes([data[6], data[7]]) as usize;
    // data[8..12] = reserved
    let report_size = usize::try_from(u32::from_le_bytes([data[12], data[13], data[14], data[15]]))
        .map_err(|_| TdxError::InvalidData("Financial report size exceeds usize".into()))?;
    // data[16..20] = reserved

    if report_size % 4 != 0 {
        return Err(TdxError::InvalidData(
            "Financial report size must be a multiple of four bytes".into(),
        ));
    }
    let report_fields_count = report_size / 4;
    let index_table_size = max_count
        .checked_mul(INDEX_ITEM_SIZE)
        .ok_or_else(|| TdxError::InvalidData("Financial index table size overflow".into()))?;
    let index_table_end = HEADER_SIZE
        .checked_add(index_table_size)
        .ok_or_else(|| TdxError::InvalidData("Financial index table end overflow".into()))?;
    if index_table_end > data.len() {
        return Err(TdxError::InvalidData(format!(
            "Financial index table is truncated: declared {max_count} records"
        )));
    }

    let mut result = Vec::with_capacity(max_count);

    for idx in 0..max_count {
        let index_offset = HEADER_SIZE
            .checked_add(idx.checked_mul(INDEX_ITEM_SIZE).ok_or_else(|| {
                TdxError::InvalidData(format!("Financial index {idx} offset overflow"))
            })?)
            .ok_or_else(|| {
                TdxError::InvalidData(format!("Financial index {idx} offset overflow"))
            })?;

        // Read stock code (6 bytes)
        let code_bytes = &data[index_offset..index_offset + 6];
        let code = String::from_utf8_lossy(code_bytes)
            .trim_end_matches('\0')
            .to_string();

        // Skip 1 byte separator, read 4-byte offset
        let foa = usize::try_from(u32::from_le_bytes([
            data[index_offset + 7],
            data[index_offset + 8],
            data[index_offset + 9],
            data[index_offset + 10],
        ]))
        .map_err(|_| {
            TdxError::InvalidData(format!(
                "Financial record {idx} report offset exceeds usize"
            ))
        })?;

        // Read report fields
        let report_end = foa.checked_add(report_size).ok_or_else(|| {
            TdxError::InvalidData(format!("Financial record {idx} report bounds overflow"))
        })?;
        if foa < index_table_end || report_end > data.len() {
            return Err(TdxError::InvalidData(format!(
                "Financial record {idx} report bounds {foa}..{report_end} are outside \
                 {index_table_end}..{}",
                data.len()
            )));
        }

        let mut fields = Vec::with_capacity(report_fields_count);
        for i in 0..report_fields_count {
            let offset = foa + i * 4;
            let val = f32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            fields.push(val);
        }

        result.push(FinancialRecord {
            code,
            report_date,
            fields,
        });
    }

    Ok(result)
}

/// 从 .dat 文件读取
pub fn read_financial_file(filename: &str) -> Result<Vec<FinancialRecord>> {
    let data = std::fs::read(filename)?;
    parse_financial(&data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn build_test_financial() -> Vec<u8> {
        let mut data = Vec::new();

        // Header: type=1, date=20231231, max_count=2, reserved=0, report_size=8 (2 floats), reserved=0
        data.extend_from_slice(&1i16.to_le_bytes());
        data.extend_from_slice(&20231231u32.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&8u32.to_le_bytes()); // 2 floats × 4 bytes
        data.extend_from_slice(&0u32.to_le_bytes());

        // Stock index: 2 items
        // Stock 1: "600519" at offset 38 (after header 16 + index 22)
        let report_offset_1 = HEADER_SIZE + 2 * INDEX_ITEM_SIZE;
        data.extend_from_slice(b"600519");
        data.push(0x00);
        data.extend_from_slice(&(report_offset_1 as u32).to_le_bytes());

        // Stock 2: "000858" at offset 38 + 8 = 46
        let report_offset_2 = report_offset_1 + 8;
        data.extend_from_slice(b"000858");
        data.push(0x00);
        data.extend_from_slice(&(report_offset_2 as u32).to_le_bytes());

        // Report data for stock 1: 2 floats (1.5, 2.5)
        data.extend_from_slice(&1.5f32.to_le_bytes());
        data.extend_from_slice(&2.5f32.to_le_bytes());

        // Report data for stock 2: 2 floats (3.5, 4.5)
        data.extend_from_slice(&3.5f32.to_le_bytes());
        data.extend_from_slice(&4.5f32.to_le_bytes());

        data
    }

    #[test]
    fn test_parse_financial() {
        let data = build_test_financial();
        let records = parse_financial(&data).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].code, "600519");
        assert_eq!(records[0].report_date, 20231231);
        assert_eq!(records[0].fields.len(), 2);
        assert!((records[0].fields[0] - 1.5).abs() < 0.001);
        assert!((records[0].fields[1] - 2.5).abs() < 0.001);
        assert_eq!(records[1].code, "000858");
        assert!((records[1].fields[0] - 3.5).abs() < 0.001);
    }

    #[test]
    fn declared_financial_records_fail_atomically_on_bad_index_or_report_bounds() {
        let complete = build_test_financial();
        let truncated_index = complete[..HEADER_SIZE + INDEX_ITEM_SIZE].to_vec();
        assert!(parse_financial(&truncated_index)
            .unwrap_err()
            .to_string()
            .contains("index table"));

        let mut invalid_offset = complete;
        invalid_offset[HEADER_SIZE + 7..HEADER_SIZE + 11].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_financial(&invalid_offset)
            .unwrap_err()
            .to_string()
            .contains("report bounds"));
    }

    #[test]
    fn financial_report_size_must_be_a_complete_float_width() {
        let mut data = build_test_financial();
        data[12..16].copy_from_slice(&7_u32.to_le_bytes());
        assert!(parse_financial(&data)
            .unwrap_err()
            .to_string()
            .contains("multiple of four"));
    }

    #[test]
    fn test_read_financial_file() {
        let data = build_test_financial();
        let path =
            std::env::temp_dir().join(format!("magic_tdx_finance_{}.dat", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&data).unwrap();
        drop(f);

        let records = read_financial_file(path.to_str().unwrap()).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].code, "600519");
        let _ = std::fs::remove_file(path);
    }
}
