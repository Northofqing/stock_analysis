/// F10 模块数据类型
use std::fmt;

/// F10 分类信息
///
/// 表示一个 F10 数据分类，包含名称、文件名、起始位置和长度。
/// 通过 `get_company_info_category` API 获取。
#[derive(Debug, Clone, PartialEq)]
pub struct F10Category {
    /// 分类名称 (如 "公司概况", "财务分析")
    pub name: String,

    /// 文件名
    pub filename: String,

    /// 文件名原始 GBK 字节 (用于精确回传服务器)
    pub(crate) filename_raw: Vec<u8>,

    /// 数据起始位置 (字节偏移)
    pub start: u32,

    /// 数据长度 (字节)
    pub length: u32,
}

impl F10Category {
    /// 创建新的 F10Category
    pub fn new(name: String, filename: String, start: u32, length: u32) -> Self {
        Self {
            name,
            filename,
            filename_raw: Vec::new(),
            start,
            length,
        }
    }

    /// 创建新的 F10Category (带原始字节)
    pub fn new_with_raw(
        name: String,
        filename: String,
        filename_raw: Vec<u8>,
        start: u32,
        length: u32,
    ) -> Self {
        Self {
            name,
            filename,
            filename_raw,
            start,
            length,
        }
    }

    /// 获取数据大小 (KB)
    pub fn size_kb(&self) -> f64 {
        self.length as f64 / 1024.0
    }

    /// 获取数据大小 (MB)
    pub fn size_mb(&self) -> f64 {
        self.length as f64 / (1024.0 * 1024.0)
    }
}

impl fmt::Display for F10Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({} bytes, {} KB)",
            self.name,
            self.length,
            self.size_kb()
        )
    }
}

/// F10 内容
///
/// 表示一个分类的文本内容，通过 `get_company_info_content` API 获取。
#[derive(Debug, Clone)]
pub struct F10Content {
    /// 分类名称
    pub category: String,

    /// 文本内容 (GBK 解码后的 UTF-8 字符串)
    pub content: String,

    /// 内容长度 (字节)
    pub byte_length: usize,
}

impl F10Content {
    /// 创建新的 F10Content
    pub fn new(category: String, content: String) -> Self {
        let byte_length = content.len();
        Self {
            category,
            content,
            byte_length,
        }
    }

    /// 获取字符数
    pub fn char_count(&self) -> usize {
        self.content.chars().count()
    }

    /// 获取内容摘要 (前 N 个字符)
    pub fn summary(&self, max_chars: usize) -> &str {
        if self.content.chars().count() <= max_chars {
            &self.content
        } else {
            &self.content[..self
                .content
                .char_indices()
                .nth(max_chars)
                .map_or(0, |(i, _)| i)]
        }
    }

    /// 按行分割内容
    pub fn lines(&self) -> Vec<&str> {
        self.content.lines().collect()
    }

    /// 搜索关键词
    pub fn contains(&self, keyword: &str) -> bool {
        self.content.contains(keyword)
    }
}

impl fmt::Display for F10Content {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} chars, {} bytes",
            self.category,
            self.char_count(),
            self.byte_length
        )
    }
}

/// F10 数据包
///
/// 包含分类信息和对应的内容，便于批量获取和处理。
#[derive(Debug, Clone)]
pub struct F10Data {
    /// 股票代码
    pub code: String,

    /// 市场代码 (0=SZ, 1=SH)
    pub market: u8,

    /// 所有分类的内容
    pub contents: Vec<F10Content>,
}

impl F10Data {
    /// 创建新的 F10Data
    pub fn new(code: String, market: u8) -> Self {
        Self {
            code,
            market,
            contents: Vec::new(),
        }
    }

    /// 添加内容
    pub fn add_content(&mut self, content: F10Content) {
        self.contents.push(content);
    }

    /// 获取指定分类的内容
    pub fn get(&self, category_name: &str) -> Option<&F10Content> {
        self.contents.iter().find(|c| c.category == category_name)
    }

    /// 获取分类数量
    pub fn category_count(&self) -> usize {
        self.contents.len()
    }

    /// 获取总字符数
    pub fn total_chars(&self) -> usize {
        self.contents.iter().map(|c| c.char_count()).sum()
    }

    /// 获取总字节数
    pub fn total_bytes(&self) -> usize {
        self.contents.iter().map(|c| c.byte_length).sum()
    }

    /// 列出所有分类名称
    pub fn category_names(&self) -> Vec<&str> {
        self.contents.iter().map(|c| c.category.as_str()).collect()
    }
}

impl fmt::Display for F10Data {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "F10Data({} {}): {} categories, {} chars",
            self.market,
            self.code,
            self.category_count(),
            self.total_chars()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_f10_category_new() {
        let cat = F10Category::new(
            "公司概况".to_string(),
            "company_profile.dat".to_string(),
            0,
            36830,
        );

        assert_eq!(cat.name, "公司概况");
        assert_eq!(cat.filename, "company_profile.dat");
        assert_eq!(cat.start, 0);
        assert_eq!(cat.length, 36830);
        assert!((cat.size_kb() - 35.97).abs() < 0.1);
        assert!((cat.size_mb() - (36830.0 / 1_048_576.0)).abs() < f64::EPSILON);
        assert!(cat.to_string().contains("公司概况"));

        let raw = F10Category::new_with_raw(
            "股东研究".into(),
            "holder.dat".into(),
            vec![1, 2, 3],
            10,
            2048,
        );
        assert_eq!(raw.filename_raw, vec![1, 2, 3]);
        assert_eq!(raw.size_kb(), 2.0);
    }

    #[test]
    fn test_f10_content_new() {
        let content = F10Content::new("公司概况".to_string(), "hello world".to_string());

        assert_eq!(content.category, "公司概况");
        assert_eq!(content.char_count(), 11);
        assert!(content.contains("hello"));
        assert!(!content.contains("不存在"));
        assert_eq!(content.summary(100), "hello world");
        assert_eq!(content.lines(), vec!["hello world"]);
        assert!(content.to_string().contains("11 chars"));
    }

    #[test]
    fn test_f10_content_summary() {
        let content = F10Content::new(
            "测试".to_string(),
            "这是一段很长的文本内容，用于测试摘要功能".to_string(),
        );

        let summary = content.summary(5);
        assert!(summary.len() <= 15); // 5 个中文字符 ≈ 15 字节
    }

    #[test]
    fn test_f10_data() {
        let mut data = F10Data::new("600519".to_string(), 1);

        let content1 = F10Content::new("公司概况".to_string(), "内容1".to_string());
        let content2 = F10Content::new("财务分析".to_string(), "内容2".to_string());

        data.add_content(content1);
        data.add_content(content2);

        assert_eq!(data.category_count(), 2);
        assert!(data.get("公司概况").is_some());
        assert!(data.get("财务分析").is_some());
        assert!(data.get("不存在").is_none());

        let names = data.category_names();
        assert_eq!(names, vec!["公司概况", "财务分析"]);
        assert_eq!(data.total_chars(), 6);
        assert_eq!(data.total_bytes(), 14);
        assert!(data.to_string().contains("2 categories"));
    }
}
