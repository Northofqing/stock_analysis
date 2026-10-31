use regex::Regex;
/// F10 文本解析器
///
/// 解析通达信 F10 原始文本，提取结构化数据。
/// 基于港澳资讯格式，兼容不同公司的 F10 数据差异。
use std::collections::HashMap;

/// F10 解析结果
#[derive(Debug, Clone, Default)]
pub struct F10Parsed {
    /// 基本资料
    pub basic_info: HashMap<String, String>,

    /// 发行上市信息
    pub listing_info: HashMap<String, String>,

    /// 章节内容 (标题 -> 内容)
    pub sections: HashMap<String, String>,
}

impl F10Parsed {
    /// 获取指定字段
    pub fn get(&self, key: &str) -> Option<&str> {
        self.basic_info
            .get(key)
            .or_else(|| self.listing_info.get(key))
            .map(|s| s.as_str())
    }

    /// 获取所有字段
    pub fn all_fields(&self) -> HashMap<&str, &str> {
        let mut result = HashMap::new();
        for (k, v) in &self.basic_info {
            result.insert(k.as_str(), v.as_str());
        }
        for (k, v) in &self.listing_info {
            result.insert(k.as_str(), v.as_str());
        }
        result
    }
}

/// F10 文本解析器
pub struct F10TextParser {
    raw: String,
}

impl F10TextParser {
    /// 创建新的解析器
    pub fn new(text: &str) -> Self {
        Self {
            raw: text.to_string(),
        }
    }

    /// 解析 F10 文本
    pub fn parse(&self) -> F10Parsed {
        let sections = self.split_sections();
        let mut result = F10Parsed::default();

        // 使用索引来判断章节类型
        let mut basic_found = false;
        let mut listing_found = false;

        for (title, content) in &sections {
            // 检查章节标题是否包含数字前缀
            let is_first = title.starts_with("1.");
            let is_second = title.starts_with("2.");

            if is_first && !basic_found {
                let basic = self.parse_basic_info(content);
                result.basic_info = basic;
                basic_found = true;
            } else if is_second && !listing_found {
                let listing = self.parse_listing_info(content);
                result.listing_info = listing;
                listing_found = true;
            }
            result.sections.insert(title.clone(), content.clone());
        }

        result
    }

    /// 按章节分割文本
    fn split_sections(&self) -> HashMap<String, String> {
        let mut sections = HashMap::new();
        // 匹配 【数字.标题】 格式
        let pattern = Regex::new(r"【(\d+\.[^】]+)】").unwrap();

        let matches: Vec<_> = pattern.find_iter(&self.raw).collect();
        for (i, m) in matches.iter().enumerate() {
            let title = m
                .as_str()
                .trim_matches(|c| c == '【' || c == '】')
                .to_string();
            let start = m.end();
            let end = if i + 1 < matches.len() {
                matches[i + 1].start()
            } else {
                self.raw.len()
            };
            let content = self.raw[start..end].trim().to_string();
            sections.insert(title, content);
        }

        sections
    }

    /// 解析基本资料
    fn parse_basic_info(&self, content: &str) -> HashMap<String, String> {
        let mut result = HashMap::new();

        let key_fields = [
            ("公司名称", vec!["公司名称"]),
            ("英文名称", vec!["英文名称", "英文全称"]),
            ("证券代码", vec!["证券代码"]),
            ("证券简称", vec!["证券简称"]),
            ("行业类别", vec!["行业类别", "所属行业", "通达信研究行业"]),
            ("上市日期", vec!["上市日期"]),
            ("注册资本", vec!["注册资本"]),
            ("法人代表", vec!["法定代表人", "法人代表"]),
            ("注册地址", vec!["注册地址"]),
            ("办公地址", vec!["办公地址"]),
            ("主营业务", vec!["主营业务"]),
            ("经营范围", vec!["经营范围"]),
            ("公司简介", vec!["公司简介"]),
            ("联系电话", vec!["联系电话"]),
            ("电子邮箱", vec!["电子邮箱"]),
            ("公司网址", vec!["公司网址"]),
        ];

        for (display_name, search_keys) in &key_fields {
            for sk in search_keys {
                if let Some(value) = self.extract_table_field(content, sk) {
                    result.insert(display_name.to_string(), value);
                    break;
                }
            }
        }

        result
    }

    /// 解析发行上市信息
    ///
    /// 注意: "上市日期" 已在 basic_info 中提取，此处不再重复。
    fn parse_listing_info(&self, content: &str) -> HashMap<String, String> {
        let mut result = HashMap::new();

        let key_fields = [
            ("网上发行日期", vec!["网上发行日期"]),
            ("发行方式", vec!["发行方式"]),
            ("发行量(股)", vec!["发行量(股)", "发行量(万股)"]),
            ("每股发行价(元)", vec!["每股发行价(元)", "发行价格(元)"]),
            (
                "募集资金净额(元)",
                vec!["募集资金净额(元)", "募集资金净额(万)"],
            ),
            (
                "上市首日收盘价(元)",
                vec!["上市首日收盘价(元)", "上市首日收盘价"],
            ),
            ("主承销商", vec!["主承销商"]),
            ("保荐人", vec!["保荐人"]),
        ];

        for (display_name, search_keys) in &key_fields {
            for sk in search_keys {
                if let Some(value) = self.extract_table_field(content, sk) {
                    result.insert(display_name.to_string(), value);
                    break;
                }
            }
        }

        result
    }

    /// 从表格格式提取字段值
    ///
    /// 支持格式:
    /// - │字段名│值│
    /// - │字段名      │值│
    /// - ｜字段名｜值｜
    /// - |字段名|值|
    fn extract_table_field(&self, text: &str, field_name: &str) -> Option<String> {
        // 在文本中搜索字段名
        if let Some(pos) = text.find(field_name) {
            let after_field = &text[pos + field_name.len()..];

            // 查找紧随其后的竖线字符
            for (i, ch) in after_field.char_indices() {
                if i > 20 {
                    // 字段名和竖线之间不应有太远的距离
                    break;
                }
                if ch == '|' || ch == '\u{FF5C}' || ch == '\u{2502}' {
                    // 找到竖线，提取后面的内容直到下一个竖线
                    let value_part = &after_field[i + ch.len_utf8()..];
                    let value = value_part
                        .split(|c: char| {
                            c == '|' || c == '\u{FF5C}' || c == '\u{2502}' || c == '\n'
                        })
                        .next()
                        .unwrap_or("")
                        .trim();
                    if !value.is_empty() && value != "---" {
                        return Some(value.to_string());
                    }
                    break;
                }
            }
        }

        None
    }
}

/// 便捷函数：解析 F10 文本
pub fn parse_f10_text(text: &str) -> F10Parsed {
    let parser = F10TextParser::new(text);
    parser.parse()
}

/// 便捷函数：提取基本资料
pub fn extract_basic_info(text: &str) -> HashMap<String, String> {
    let parser = F10TextParser::new(text);
    let sections = parser.split_sections();

    for (title, content) in &sections {
        if title.starts_with("1.基本资料") || title.starts_with("1.公司资料") {
            return parser.parse_basic_info(content);
        }
    }

    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_info() {
        let text = r#"
【1.基本资料】
┌───────┬───────────────────────────────┐
│公司名称      │贵州茅台酒股份有限公司                                        │
├───────┼───────────────────────────────┤
│证券代码      │600519                │
├───────┼───────────────────────────────┤
│主营业务      │茅台酒及系列酒的生产与销售                                    │
└───────┴───────────────────────────────┘
"#;

        let result = extract_basic_info(text);
        assert_eq!(result.get("公司名称").unwrap(), "贵州茅台酒股份有限公司");
        assert_eq!(result.get("证券代码").unwrap(), "600519");
        assert_eq!(
            result.get("主营业务").unwrap(),
            "茅台酒及系列酒的生产与销售"
        );
    }

    #[test]
    fn test_parse_sections() {
        let text = r#"
【1.基本资料】
内容1
【2.发行上市】
内容2
"#;

        let parser = F10TextParser::new(text);
        let sections = parser.split_sections();
        assert!(sections.contains_key("1.基本资料"));
        assert!(sections.contains_key("2.发行上市"));
    }

    #[test]
    fn complete_parser_maps_basic_listing_aliases_and_field_boundaries() {
        let text = r#"
【1.公司资料】
|公司名称|示例股份|
｜英文全称｜Example Corp｜
│证券代码│600001│
│证券简称│示例│
│所属行业│制造业│
│上市日期│2020-01-02│
│注册资本│1000万元│
│法定代表人│张三│
│注册地址│上海│
│办公地址│深圳│
│主营业务│制造│
│经营范围│研发│
│公司简介│简介│
│联系电话│123│
│电子邮箱│a@example.com│
│公司网址│https://example.com│
【2.发行上市】
│网上发行日期│2019-12-01│
│发行方式│网下│
│发行量(万股)│100│
│发行价格(元)│10│
│募集资金净额(万)│900│
│上市首日收盘价│12│
│主承销商│承销商│
│保荐人│保荐机构│
【3.其他】
│公司名称│不应覆盖│
"#;

        let parsed = parse_f10_text(text);
        assert_eq!(parsed.get("公司名称"), Some("示例股份"));
        assert_eq!(parsed.get("英文名称"), Some("Example Corp"));
        assert_eq!(parsed.get("行业类别"), Some("制造业"));
        assert_eq!(parsed.get("法人代表"), Some("张三"));
        assert_eq!(parsed.get("发行量(股)"), Some("100"));
        assert_eq!(parsed.get("每股发行价(元)"), Some("10"));
        assert_eq!(parsed.get("募集资金净额(元)"), Some("900"));
        assert_eq!(parsed.get("上市首日收盘价(元)"), Some("12"));
        assert_eq!(parsed.get("不存在"), None);
        assert_eq!(parsed.all_fields().len(), 24);
        assert_eq!(parsed.sections.len(), 3);

        assert_eq!(
            extract_basic_info(text).get("证券简称").map(String::as_str),
            Some("示例")
        );
        assert!(extract_basic_info("【2.发行上市】无").is_empty());

        let parser = F10TextParser::new(
            "【1.基本资料】\n│公司名称│---│\n公司名称                      │太远│",
        );
        assert!(parser.parse().get("公司名称").is_none());
    }
}
