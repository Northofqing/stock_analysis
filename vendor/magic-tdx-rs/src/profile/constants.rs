//! F10 模块常量

// ============================================================
// 协议命令码
// ============================================================

/// 获取公司信息分类
pub const CMD_COMPANY_INFO_CATEGORY: u16 = 0x9b0f;

/// 获取公司信息内容
pub const CMD_COMPANY_INFO_CONTENT: u16 = 0x9c07;

// ============================================================
// 请求包头
// ============================================================

/// get_company_info_category 请求包头
pub const CATEGORY_REQUEST_HEADER: [u8; 12] = [
    0x0c, 0x0f, 0x10, 0x9b, 0x00, 0x01, 0x0e, 0x00, 0x0e, 0x00, 0xcf, 0x02,
];

/// get_company_info_content 请求包头
pub const CONTENT_REQUEST_HEADER: [u8; 12] = [
    0x0c, 0x07, 0x10, 0x9c, 0x00, 0x01, 0x68, 0x00, 0x68, 0x00, 0xd0, 0x02,
];

// ============================================================
// 数据大小
// ============================================================

/// 分类条目大小 (字节)
pub const CATEGORY_ENTRY_SIZE: usize = 152;

/// 分类名称字段大小 (字节)
pub const CATEGORY_NAME_SIZE: usize = 64;

/// 文件名字段大小 (字节)
pub const CATEGORY_FILENAME_SIZE: usize = 80;

/// 响应头大小 (字节)
pub const CONTENT_HEADER_SIZE: usize = 12;

// ============================================================
// F10 分类名称
// ============================================================

/// F10 分类名称常量
pub mod category {
    /// 最新提示
    pub const LATEST_HINT: &str = "最新提示";

    /// 公司概况
    pub const COMPANY_PROFILE: &str = "公司概况";

    /// 财务分析
    pub const FINANCIAL_ANALYSIS: &str = "财务分析";

    /// 股东研究
    pub const SHAREHOLDER_RESEARCH: &str = "股东研究";

    /// 股本结构
    pub const EQUITY_STRUCTURE: &str = "股本结构";

    /// 资本运作
    pub const CAPITAL_OPERATION: &str = "资本运作";

    /// 业内点评
    pub const INDUSTRY_COMMENT: &str = "业内点评";

    /// 行业分析
    pub const INDUSTRY_ANALYSIS: &str = "行业分析";

    /// 公司大事
    pub const COMPANY_EVENTS: &str = "公司大事";

    /// 研究报告
    pub const RESEARCH_REPORT: &str = "研究报告";

    /// 经营分析
    pub const BUSINESS_ANALYSIS: &str = "经营分析";

    /// 主力追踪
    pub const MAINFORCE_TRACKING: &str = "主力追踪";

    /// 分红扩股
    pub const DIVIDEND_EXPANSION: &str = "分红扩股";

    /// 高层治理
    pub const EXECUTIVE_GOVERNANCE: &str = "高层治理";

    /// 龙虎榜单
    pub const TOP_LIST: &str = "龙虎榜单";

    /// 关联个股
    pub const RELATED_STOCKS: &str = "关联个股";
}

// 市场代码: 复用 crate::protocol::constants::{MARKET_SZ, MARKET_SH}
