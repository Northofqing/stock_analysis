//! 通用工具函数 — 跨模块复用的辅助逻辑。
//!
//! review #15 抽出: 多个模块重复实现的 truncate helper 集中到这里.

/// 按 char 数量截断字符串, 末尾追加 "…" 省略号.
/// review #15: 一次 char_indices().nth() 扫描, 避免 chars().count() 二次扫描
/// + chars().take().collect() 重复分配. 中文字符正确处理 (UTF-8 字节边界).
pub fn truncate_chars(s: &str, max: usize) -> String {
    if let Some((idx, _)) = s.char_indices().nth(max) {
        format!("{}…", &s[..idx])
    } else {
        s.to_string()
    }
}

/// CR-9 (review): 简单 HTML 标签剥离, 复用给 xueqiu/em_industry_news/cninfo/event_extractor.
///   处理 `<em>关键词</em>` `<strong>x</strong>` 等简单高亮, 不处理嵌套 / 自闭合 / HTML 实体.
///   之前 3 个 provider 各有一份私有实现 + event_extractor 又有第 4 份 inline 实现, 现在统一调用此处.
pub fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}
