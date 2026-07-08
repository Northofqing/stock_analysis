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