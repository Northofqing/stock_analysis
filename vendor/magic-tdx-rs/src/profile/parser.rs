use super::constants::*;
use super::types::*;
/// F10 模块响应解析器
use crate::error::{Result, TdxError};

/// 解析公司信息分类响应
///
/// # 参数
/// * `data` - 响应数据
///
/// # 返回
/// 分类列表
pub fn parse_company_info_category(data: &[u8]) -> Result<Vec<F10Category>> {
    if data.len() < 2 {
        return Err(TdxError::InvalidData("响应数据太短".to_string()));
    }

    // 解析分类数量 (u16, little-endian)
    let count = u16::from_le_bytes([data[0], data[1]]) as usize;

    // 验证数据长度
    let expected_len = 2 + count * CATEGORY_ENTRY_SIZE;
    if data.len() < expected_len {
        return Err(TdxError::InvalidData(format!(
            "响应数据长度不足: 期望 {} 字节, 实际 {} 字节",
            expected_len,
            data.len()
        )));
    }

    let mut categories = Vec::with_capacity(count);
    let mut pos = 2;

    for _ in 0..count {
        // 解析分类名称 (64 字节, GBK)
        let name_bytes = &data[pos..pos + CATEGORY_NAME_SIZE];
        let name = decode_gbk_string(name_bytes)?;
        pos += CATEGORY_NAME_SIZE;

        // 解析文件名 (80 字节, GBK) — 保留原始字节用于精确回传
        let filename_bytes = &data[pos..pos + CATEGORY_FILENAME_SIZE];
        let filename = decode_gbk_string(filename_bytes)?;
        let filename_raw = trim_null_bytes(filename_bytes).to_vec();
        pos += CATEGORY_FILENAME_SIZE;

        // 解析起始位置 (u32, little-endian)
        let start = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;

        // 解析数据长度 (u32, little-endian)
        let length = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;

        categories.push(F10Category::new_with_raw(
            name,
            filename,
            filename_raw,
            start,
            length,
        ));
    }

    Ok(categories)
}

/// 去除末尾的 null 字节
fn trim_null_bytes(bytes: &[u8]) -> &[u8] {
    let mut len = bytes.len();
    while len > 0 && bytes[len - 1] == 0 {
        len -= 1;
    }
    &bytes[..len]
}

/// 解析公司信息内容响应
///
/// # 参数
/// * `data` - 响应数据
///
/// # 返回
/// 文本内容
pub fn parse_company_info_content(data: &[u8]) -> Result<String> {
    if data.len() < CONTENT_HEADER_SIZE {
        return Err(TdxError::InvalidData("响应数据太短".to_string()));
    }

    // 解析内容长度 (u16, little-endian, 偏移 10)
    let length = u16::from_le_bytes([data[10], data[11]]) as usize;

    // 验证数据长度
    let expected_len = CONTENT_HEADER_SIZE + length;
    if data.len() < expected_len {
        return Err(TdxError::InvalidData(format!(
            "响应数据长度不足: 期望 {} 字节, 实际 {} 字节",
            expected_len,
            data.len()
        )));
    }

    // 解析内容 (GBK)
    let content_bytes = &data[CONTENT_HEADER_SIZE..CONTENT_HEADER_SIZE + length];
    let content = decode_gbk_string(content_bytes)?;

    Ok(content)
}

/// 解码 GBK 字节数组为 UTF-8 字符串
///
/// 自动去除末尾的 null 字节和空白字符
fn decode_gbk_string(bytes: &[u8]) -> Result<String> {
    // 去除末尾的 null 字节
    let mut len = bytes.len();
    while len > 0 && bytes[len - 1] == 0 {
        len -= 1;
    }

    if len == 0 {
        return Ok(String::new());
    }

    // 使用 encoding_rs 解码 GBK
    let (decoded, _, _) = encoding_rs::GBK.decode(&bytes[..len]);
    Ok(decoded.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_category_empty() {
        // 空分类列表 (count = 0)
        let data = [0x00, 0x00];
        let result = parse_company_info_category(&data);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_parse_category_invalid_length() {
        // 数据太短
        let data = [0x01, 0x00]; // count = 1, 但没有数据
        let result = parse_company_info_category(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_category_one_entry() {
        // 构造一个分类条目
        let mut data = Vec::new();

        // count = 1
        data.extend_from_slice(&[0x01, 0x00]);

        // name: "test" (简单 ASCII 测试)
        let mut name = [0u8; 64];
        let name_bytes = b"test";
        name[..name_bytes.len()].copy_from_slice(name_bytes);
        data.extend_from_slice(&name);

        // filename: "company.dat"
        let mut filename = [0u8; 80];
        let filename_bytes = b"company.dat";
        filename[..filename_bytes.len()].copy_from_slice(filename_bytes);
        data.extend_from_slice(&filename);

        // start = 100
        data.extend_from_slice(&100u32.to_le_bytes());

        // length = 2048
        data.extend_from_slice(&2048u32.to_le_bytes());

        let result = parse_company_info_category(&data);
        assert!(result.is_ok());

        let categories = result.unwrap();
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].name, "test");
        assert_eq!(categories[0].filename, "company.dat");
        assert_eq!(categories[0].start, 100);
        assert_eq!(categories[0].length, 2048);
    }

    #[test]
    fn test_parse_content_invalid_length() {
        // 数据太短
        let data = [0u8; 8];
        let result = parse_company_info_content(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_content_empty() {
        // 空内容 (length = 0)
        let mut data = [0u8; 12];
        data[10] = 0;
        data[11] = 0;

        let result = parse_company_info_content(&data);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn test_parse_content_with_text() {
        // 构造带文本的响应
        let mut data = Vec::new();

        // 头部 12 字节
        data.extend_from_slice(&[0u8; 10]);

        // length = 20
        data.extend_from_slice(&20u16.to_le_bytes());

        // 内容: "测试内容" (GBK)
        let content_gbk = &[0xB2, 0xE2, 0xCA, 0xD4, 0xC4, 0xDC, 0xC1, 0xEE];
        // 填充到 20 字节
        let mut content = vec![0u8; 20];
        content[..content_gbk.len()].copy_from_slice(content_gbk);
        data.extend_from_slice(&content);

        let result = parse_company_info_content(&data);
        assert!(result.is_ok());

        let text = result.unwrap();
        assert!(text.contains("测试"));
    }

    #[test]
    fn test_decode_gbk_string() {
        // 测试 GBK 解码 - 使用简单的 ASCII
        let gbk_bytes = b"hello";
        let result = decode_gbk_string(gbk_bytes);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn test_decode_gbk_string_with_null() {
        // 测试带 null 结尾的 GBK 字符串
        let gbk_bytes = b"hello\x00\x00\x00";
        let result = decode_gbk_string(gbk_bytes);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn test_decode_gbk_string_empty() {
        // 测试空字符串
        let gbk_bytes: &[u8] = &[0x00, 0x00];
        let result = decode_gbk_string(gbk_bytes);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }
}
