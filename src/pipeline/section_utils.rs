//! AI 章节格式化辅助函数 — 从 mod.rs 提取以减小文件体积。

/// 规范化 AI 输出的章节标题：统一为 `## 【XX】` 形式。
const AI_SECTIONS: &[&str] = &[
    "宏观影响",
    "消息面",
    "技术面",
    "主力资金",
    "基本面",
    "操作建议",
    "风险提示",
    "逆势布局逻辑",
];

/// 尝试把一行解析为 AI 章节标题行。
///
/// 返回 `(canonical, full_name, content)`。
pub fn parse_ai_section_line(trimmed: &str) -> Option<(&'static str, String, String)> {
    let has_hash = trimmed.starts_with('#');
    let title = trimmed.trim_start_matches('#').trim();

    if let Some(rest) = title.strip_prefix('【') {
        let end = rest.find('】')?;
        let name = rest[..end].trim();
        let content = rest[end + '】'.len_utf8()..].trim();
        for s in AI_SECTIONS {
            if name.contains(s) {
                return Some((s, name.to_string(), content.to_string()));
            }
        }
        return None;
    }

    if has_hash {
        for s in AI_SECTIONS {
            if title.contains(s) {
                return Some((s, title.to_string(), String::new()));
            }
        }
    }

    None
}

/// 规范化 AI 输出的章节结构：
/// - 统一为 `## 【章节】` 标题
/// - 去重连续重复的同一章节标题
pub fn normalize_ai_sections(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    let mut last_section: Option<&'static str> = None;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }

        if let Some((canonical, name, content)) = parse_ai_section_line(trimmed) {
            if last_section != Some(canonical) {
                if !out.is_empty() && !out.ends_with("\n\n") {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push('\n');
                }
                out.push_str("## 【");
                out.push_str(&name);
                out.push_str("】\n");
                last_section = Some(canonical);
            }
            if !content.is_empty() {
                out.push_str(&content);
                out.push('\n');
            }
            continue;
        }

        out.push_str(raw_line);
        out.push('\n');
    }
    out
}

/// 把深度研判 markdown 合并进标准 `analysis_summary`。
pub fn merge_deep_analysis(standard: &str, deep_md: &str) -> String {
    let cut = ["\n# AI分析", "\n# 相关新闻"]
        .iter()
        .filter_map(|m| standard.find(m))
        .min();
    let tech_part = match cut {
        Some(idx) => &standard[..idx],
        None => standard,
    };
    format!(
        "{}\n\n# 🏛️ 机构级深度研判（多智能体）\n\n{}\n",
        tech_part.trim_end(),
        deep_md.trim()
    )
}

/// 深度研判报告落盘备份。
pub fn save_deep_report(code: &str, content: &str) -> std::io::Result<()> {
    let date = chrono::Local::now().format("%Y%m%d").to_string();
    let dir = std::path::PathBuf::from("reports/details");
    save_deep_report_to(&dir, &date, code, content)
}

fn save_deep_report_to(
    dir: &std::path::Path,
    date: &str,
    code: &str,
    content: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(format!("{}_{}.md", date, code)), content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_parser_accepts_bracket_and_hash_forms_only() {
        assert_eq!(
            parse_ai_section_line("## 【技术面分析】 趋势向上"),
            Some(("技术面", "技术面分析".into(), "趋势向上".into()))
        );
        assert_eq!(
            parse_ai_section_line("### 风险提示"),
            Some(("风险提示", "风险提示".into(), String::new()))
        );
        assert_eq!(parse_ai_section_line("风险提示"), None);
        assert_eq!(parse_ai_section_line("## 【未知章节】内容"), None);
        assert_eq!(parse_ai_section_line("## 【技术面"), None);
    }

    #[test]
    fn normalization_canonicalizes_and_deduplicates_adjacent_sections() {
        let input = "开头\n\n## 【技术面】趋势向上\n### 技术面\n正文\n# 风险提示\n注意回撤";

        let normalized = normalize_ai_sections(input);

        assert_eq!(normalized.matches("## 【技术面】").count(), 1);
        assert_eq!(normalized.matches("## 【风险提示】").count(), 1);
        assert!(normalized.starts_with("开头\n\n## 【技术面】\n趋势向上\n正文\n\n"));
        assert!(normalized.ends_with("## 【风险提示】\n注意回撤\n"));
    }

    #[test]
    fn deep_analysis_replaces_only_trailing_ai_and_news_sections() {
        let standard = "# 技术报告\n正文\n# AI分析\n旧内容\n# 相关新闻\n旧新闻";
        let merged = merge_deep_analysis(standard, "\n新深度结论\n");

        assert!(merged.starts_with("# 技术报告\n正文\n\n# 🏛️ 机构级深度研判"));
        assert!(merged.ends_with("新深度结论\n"));
        assert!(!merged.contains("旧内容"));
        assert!(!merged.contains("旧新闻"));

        let without_markers = merge_deep_analysis("仅技术正文", "深度");
        assert!(without_markers.starts_with("仅技术正文\n\n# 🏛️"));
    }

    #[test]
    fn deep_report_backup_creates_directory_and_writes_exact_content() {
        let dir = std::env::temp_dir().join(format!(
            "stock-analysis-section-utils-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        save_deep_report_to(&dir, "20260718", "TEST_CODE_000001", "审计内容")
            .expect("write isolated report");

        let path = dir.join("20260718_TEST_CODE_000001.md");
        assert_eq!(
            std::fs::read_to_string(path).expect("read report"),
            "审计内容"
        );
        std::fs::remove_dir_all(dir).expect("cleanup isolated report directory");
    }
}
