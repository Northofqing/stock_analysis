//! 统一错误码体系
//!
//! 所有错误码按模块分段，便于用户端识别和处理。
//!
//! ## 错误码分段
//!
//! | 范围 | 模块 | 说明 |
//! |------|------|------|
//! | 1000-1099 | 通用 | 参数校验、输入错误 |
//! | 1100-1199 | 代码分类 | 股票/指数/板块/债券/基金 |
//! | 1200-1299 | 限流 | 请求频率限制 |
//! | 2000-2099 | 连接 | 网络连接错误 |
//! | 2100-2199 | 协议 | TDX 协议错误 |
//! | 3000-3099 | 解析 | 数据解析错误 |
//! | 4000-4099 | 文件 | 本地文件错误 |

use std::fmt;

/// 错误码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorCode(pub u32);

impl ErrorCode {
    // ── 通用 (1000-1099) ──
    /// 参数为空
    pub const EMPTY_ARGUMENT: Self = Self(1001);
    /// 参数长度超限
    pub const ARGUMENT_TOO_LONG: Self = Self(1002);
    /// 参数值超出允许范围
    pub const ARGUMENT_OUT_OF_RANGE: Self = Self(1003);
    /// 参数格式错误
    pub const INVALID_FORMAT: Self = Self(1004);

    // ── 代码分类 (1100-1199) ──
    /// 板块代码 (88xxxx) 在通用客户端中被拒绝
    pub const BLOCK_CODE_IN_GENERAL_CLIENT: Self = Self(1101);
    /// 债券代码在不支持的接口中使用
    pub const BOND_CODE_NOT_SUPPORTED: Self = Self(1102);
    /// 基金代码在不支持的接口中使用
    pub const FUND_CODE_NOT_SUPPORTED: Self = Self(1103);
    /// 未知代码格式
    pub const UNKNOWN_CODE_FORMAT: Self = Self(1104);

    // ── 限流 (1200-1299) ──
    /// 通用请求限流触发
    pub const RATE_LIMIT_EXCEEDED: Self = Self(1201);
    /// 日K级别限流触发
    pub const RATE_LIMIT_DAILY_EXCEEDED: Self = Self(1202);
    /// 分时限流触发 (不可禁用)
    pub const RATE_LIMIT_MINUTE_EXCEEDED: Self = Self(1203);
    /// 板块K线级别不支持
    pub const BLOCK_KLINE_CATEGORY_NOT_ALLOWED: Self = Self(1204);
    /// 板块K线条数超限
    pub const BLOCK_KLINE_COUNT_EXCEEDED: Self = Self(1205);
    /// 板块分时数据禁用
    pub const BLOCK_MINUTE_DISABLED: Self = Self(1206);
    /// 板块逐笔数据禁用
    pub const BLOCK_TRADES_DISABLED: Self = Self(1207);

    // ── 连接 (2000-2099) ──
    /// 连接失败
    pub const CONNECTION_FAILED: Self = Self(2001);
    /// 连接超时
    pub const CONNECTION_TIMEOUT: Self = Self(2002);
    /// 服务器断开
    pub const DISCONNECTED: Self = Self(2003);
    /// 握手失败
    pub const HANDSHAKE_FAILED: Self = Self(2004);
    /// 重试耗尽
    pub const RETRY_EXHAUSTED: Self = Self(2005);
    /// 连接池耗尽
    pub const POOL_EXHAUSTED: Self = Self(2006);

    // ── 协议 (2100-2199) ──
    /// 响应头解析失败
    pub const RESPONSE_HEADER_INVALID: Self = Self(2101);
    /// 响应体解压失败
    pub const DECOMPRESS_FAILED: Self = Self(2102);
    /// 响应数据长度不匹配
    pub const RESPONSE_LENGTH_MISMATCH: Self = Self(2103);

    // ── 解析 (3000-3099) ──
    /// 日期格式无效
    pub const INVALID_DATE: Self = Self(3001);
    /// 日期超出范围 (年份异常)
    pub const DATE_OUT_OF_RANGE: Self = Self(3002);
    /// 股票代码无效
    pub const INVALID_STOCK_CODE: Self = Self(3003);
    /// 数据字段缺失
    pub const MISSING_FIELD: Self = Self(3004);
    /// 数据类型不匹配
    pub const TYPE_MISMATCH: Self = Self(3005);

    // ── 文件 (4000-4099) ──
    /// 文件不存在
    pub const FILE_NOT_FOUND: Self = Self(4001);
    /// 文件格式无效
    pub const INVALID_FILE_FORMAT: Self = Self(4002);
    /// 文件过小
    pub const FILE_TOO_SMALL: Self = Self(4003);
    /// 文件读取失败
    pub const FILE_READ_ERROR: Self = Self(4004);

    /// 获取错误码数值
    pub fn code(&self) -> u32 {
        self.0
    }

    /// 快捷创建 TdxError::Coded
    pub fn err(&self, message: impl Into<String>) -> crate::error::TdxError {
        crate::error::TdxError::coded(*self, message)
    }

    /// 从错误码数值创建
    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            1001..=1004
            | 1101..=1104
            | 1201..=1207
            | 2001..=2006
            | 2101..=2103
            | 3001..=3005
            | 4001..=4004 => Some(Self(code)),
            _ => None,
        }
    }

    /// 获取错误码描述
    pub fn description(&self) -> &'static str {
        match self.0 {
            // 通用
            1001 => "empty argument",
            1002 => "argument too long",
            1003 => "argument out of range",
            1004 => "invalid format",

            // 代码分类
            1101 => "block code (88xxxx) not allowed in general client, use TdxBlockClient",
            1102 => "bond code not supported in this context",
            1103 => "fund code not supported in this context",
            1104 => "unknown code format",

            // 限流
            1201 => "rate limit exceeded (general)",
            1202 => "rate limit exceeded (daily K-line)",
            1203 => "rate limit exceeded (minute, cannot be disabled)",
            1204 => "block K-line category not allowed",
            1205 => "block K-line count exceeded",
            1206 => "block minute data disabled",
            1207 => "block trades data disabled",

            // 连接
            2001 => "connection failed",
            2002 => "connection timeout",
            2003 => "server disconnected",
            2004 => "handshake failed",
            2005 => "retry exhausted",
            2006 => "connection pool exhausted",

            // 协议
            2101 => "invalid response header",
            2102 => "zlib decompress failed",
            2103 => "response length mismatch",

            // 解析
            3001 => "invalid date format",
            3002 => "date out of range",
            3003 => "invalid stock code",
            3004 => "missing required field",
            3005 => "type mismatch",

            // 文件
            4001 => "file not found",
            4002 => "invalid file format",
            4003 => "file too small",
            4004 => "file read error",

            _ => "unknown error",
        }
    }

    /// 获取用户友好的中文描述
    pub fn description_zh(&self) -> &'static str {
        match self.0 {
            // 通用
            1001 => "参数为空",
            1002 => "参数长度超限",
            1003 => "参数值超出允许范围",
            1004 => "参数格式错误",

            // 代码分类
            1101 => "板块代码(88xxxx)不允许在通用客户端中使用，请使用 TdxBlockClient",
            1102 => "债券代码在此接口中不支持",
            1103 => "基金代码在此接口中不支持",
            1104 => "未知代码格式",

            // 限流
            1201 => "请求频率超限(通用)",
            1202 => "请求频率超限(日K级别)",
            1203 => "请求频率超限(分时级别，不可禁用)",
            1204 => "板块K线级别不支持",
            1205 => "板块K线条数超限",
            1206 => "板块分时数据已禁用",
            1207 => "板块逐笔数据已禁用",

            // 连接
            2001 => "连接失败",
            2002 => "连接超时",
            2003 => "服务器断开连接",
            2004 => "握手失败",
            2005 => "重试次数耗尽",
            2006 => "连接池已满",

            // 协议
            2101 => "响应头解析失败",
            2102 => "zlib解压失败",
            2103 => "响应数据长度不匹配",

            // 解析
            3001 => "日期格式无效",
            3002 => "日期超出范围",
            3003 => "股票代码无效",
            3004 => "缺少必要字段",
            3005 => "数据类型不匹配",

            // 文件
            4001 => "文件不存在",
            4002 => "文件格式无效",
            4003 => "文件过小",
            4004 => "文件读取失败",

            _ => "未知错误",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[E{:04}] {}", self.0, self.description())
    }
}

/// 带错误码的错误信息
#[derive(Debug, Clone)]
pub struct CodedError {
    pub code: ErrorCode,
    pub message: String,
}

impl CodedError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// 格式化为用户友好的错误信息
    ///
    /// 格式: `[E1101] 板块代码(88xxxx)不允许在通用客户端中使用，请使用 TdxBlockClient: 具体信息`
    pub fn format(&self) -> String {
        format!(
            "[E{:04}] {}: {}",
            self.code.0,
            self.code.description(),
            self.message
        )
    }

    /// 格式化为中文错误信息
    pub fn format_zh(&self) -> String {
        format!(
            "[E{:04}] {}: {}",
            self.code.0,
            self.code.description_zh(),
            self.message
        )
    }
}

impl fmt::Display for CodedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

impl std::error::Error for CodedError {}

/// 代码类型分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeType {
    /// 普通股票 (000xxx, 300xxx, 600xxx, 688xxx)
    Stock,
    /// 指数 (000001, 399001 等)
    Index,
    /// 板块 (88xxxx)
    Block,
    /// 债券 (10xxxx, 11xxxx, 12xxxx, 13xxxx)
    Bond,
    /// 基金 (50xxxx, 51xxxx, 52xxxx)
    Fund,
    /// 未知
    Unknown,
}

impl CodeType {
    /// 获取代码类型描述
    pub fn description(&self) -> &'static str {
        match self {
            CodeType::Stock => "stock",
            CodeType::Index => "index",
            CodeType::Block => "block",
            CodeType::Bond => "bond",
            CodeType::Fund => "fund",
            CodeType::Unknown => "unknown",
        }
    }

    /// 获取中文描述
    pub fn description_zh(&self) -> &'static str {
        match self {
            CodeType::Stock => "股票",
            CodeType::Index => "指数",
            CodeType::Block => "板块",
            CodeType::Bond => "债券",
            CodeType::Fund => "基金",
            CodeType::Unknown => "未知",
        }
    }
}

/// 分类股票代码
pub fn classify_code(code: &str) -> CodeType {
    if code.len() != 6 {
        return CodeType::Unknown;
    }

    let first_two = &code[..2];
    match first_two {
        // 板块
        "88" => CodeType::Block,
        // 股票
        "00" | "30" | "60" | "68" => CodeType::Stock,
        // 债券
        "10" | "11" | "12" | "13" | "14" => CodeType::Bond,
        // 基金
        "50" | "51" | "52" | "55" | "56" | "58" => CodeType::Fund,
        // 其他
        _ => CodeType::Unknown,
    }
}

/// 检查代码是否为板块代码
pub fn is_block_code(code: &str) -> bool {
    classify_code(code) == CodeType::Block
}

/// 检查代码是否为股票代码
pub fn is_stock_code(code: &str) -> bool {
    classify_code(code) == CodeType::Stock
}

/// 检查代码是否为指数代码 (6位数字，非板块/债券/基金)
pub fn is_index_code(code: &str) -> bool {
    // 指数代码是6位数字，不以88/10-14/50-58开头
    code.len() == 6
        && code.chars().all(|c| c.is_ascii_digit())
        && !is_block_code(code)
        && classify_code(code) != CodeType::Bond
        && classify_code(code) != CodeType::Fund
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_code() {
        assert_eq!(classify_code("600519"), CodeType::Stock);
        assert_eq!(classify_code("000858"), CodeType::Stock);
        assert_eq!(classify_code("300750"), CodeType::Stock);
        assert_eq!(classify_code("688001"), CodeType::Stock);

        assert_eq!(classify_code("880001"), CodeType::Block);
        assert_eq!(classify_code("881001"), CodeType::Block);

        assert_eq!(classify_code("101000"), CodeType::Bond);
        assert_eq!(classify_code("110073"), CodeType::Bond);

        assert_eq!(classify_code("510300"), CodeType::Fund);
        assert_eq!(classify_code("510010"), CodeType::Fund);

        assert_eq!(classify_code("12345"), CodeType::Unknown); // 5位
        assert_eq!(classify_code("1234567"), CodeType::Unknown); // 7位
    }

    #[test]
    fn test_is_block_code() {
        assert!(is_block_code("880001"));
        assert!(is_block_code("881001"));
        assert!(!is_block_code("600519"));
        assert!(!is_block_code("000858"));
    }

    #[test]
    fn test_is_stock_code() {
        assert!(is_stock_code("600519"));
        assert!(is_stock_code("000858"));
        assert!(is_stock_code("300750"));
        assert!(is_stock_code("688001"));
        assert!(!is_stock_code("880001"));
        assert!(!is_stock_code("510300"));
    }

    #[test]
    fn test_is_index_code() {
        assert!(is_index_code("000001")); // 上证指数
        assert!(is_index_code("399001")); // 深证成指
        assert!(!is_index_code("880001")); // 板块
        assert!(!is_index_code("510300")); // 基金
        assert!(!is_index_code("110073")); // 债券
    }

    #[test]
    fn test_error_code_display() {
        let code = ErrorCode::BLOCK_CODE_IN_GENERAL_CLIENT;
        assert_eq!(code.code(), 1101);
        assert!(code.description().contains("block code"));
        assert!(code.description_zh().contains("板块代码"));
    }

    #[test]
    fn test_coded_error_format() {
        let err = CodedError::new(ErrorCode::BLOCK_CODE_IN_GENERAL_CLIENT, "code=880001");
        let msg = err.format();
        assert!(msg.contains("[E1101]"));
        assert!(msg.contains("block code"));
        assert!(msg.contains("code=880001"));
    }

    #[test]
    fn test_error_code_range_no_overlap() {
        // 确保错误码范围不重叠
        let codes = [
            ErrorCode::EMPTY_ARGUMENT,               // 1001
            ErrorCode::BLOCK_CODE_IN_GENERAL_CLIENT, // 1101
            ErrorCode::RATE_LIMIT_EXCEEDED,          // 1201
            ErrorCode::CONNECTION_FAILED,            // 2001
            ErrorCode::RESPONSE_HEADER_INVALID,      // 2101
            ErrorCode::INVALID_DATE,                 // 3001
            ErrorCode::FILE_NOT_FOUND,               // 4001
        ];
        // 所有错误码应该不同
        for i in 0..codes.len() {
            for j in i + 1..codes.len() {
                assert_ne!(codes[i], codes[j], "duplicate error code: {}", codes[i].0);
            }
        }
    }

    #[test]
    fn every_registered_error_code_and_code_type_has_stable_text() {
        let ranges = [
            1001..=1004,
            1101..=1104,
            1201..=1207,
            2001..=2006,
            2101..=2103,
            3001..=3005,
            4001..=4004,
        ];
        for value in ranges.into_iter().flatten() {
            let code = ErrorCode::from_code(value).unwrap();
            assert_eq!(code.code(), value);
            assert_ne!(code.description(), "unknown error");
            assert_ne!(code.description_zh(), "未知错误");
            assert!(code.to_string().starts_with("[E"));
            let coded = CodedError::new(code, "fixture");
            assert!(coded.to_string().contains("fixture"));
            assert!(coded.format_zh().contains("fixture"));
            assert!(code.err("fixture").to_string().contains("fixture"));
        }
        for value in [0, 1000, 1005, 1199, 5000] {
            assert_eq!(ErrorCode::from_code(value), None);
            assert_eq!(ErrorCode(value).description(), "unknown error");
            assert_eq!(ErrorCode(value).description_zh(), "未知错误");
        }

        for code_type in [
            CodeType::Stock,
            CodeType::Index,
            CodeType::Block,
            CodeType::Bond,
            CodeType::Fund,
            CodeType::Unknown,
        ] {
            assert!(!code_type.description().is_empty());
            assert!(!code_type.description_zh().is_empty());
        }
    }
}
