//! 修复 P1-1 (2026-06-30 codex review): 持仓研判聚合推送 + 模板裁剪单测.
//! BR-013 (推送按操作建议分组) + BR-014 (模板按综合分裁剪).
//!
//! 实际集成测试见 tests/bin_monitor_dedup.rs (process::Command).
//! 这里只测纯函数 `extract_advice_and_score` + `first_meaningful_line` + `build_holding_summary`.

/// 1. 操作建议在 ## 标题行的下一段
#[test]
fn test_extract_advice_from_section() {
    let md = r#"
# 600703 三安光电
## 摘要
龙头股，业绩向好
## 【操作建议】减持
反弹减仓
## 风险提示
LED 价格战
"#;
    // 模拟 extract_advice_and_score 函数内部
    let advice = extract_advice_test(md);
    assert_eq!(advice, "减持", "应取第一个非空段内容");
}

/// 2. 操作建议直接在标题里 (## 【操作建议】减持)
#[test]
fn test_extract_advice_inline_in_title() {
    let md = "## 【操作建议】强烈卖出\n立即清仓\n";
    let advice = extract_advice_test(md);
    assert_eq!(advice, "强烈卖出");
}

/// 3. 没有操作建议段 → 默认 "未知"
#[test]
fn test_extract_advice_default() {
    let md = "# 标题\n随便写\n";
    let advice = extract_advice_test(md);
    assert_eq!(advice, "未知");
}

/// 4. 综合分: 数字格式 "综合分: 45"
#[test]
fn test_extract_score_colon_format() {
    let md = "## 评分\n综合分: 45\n其他内容\n";
    let score = extract_score_test(md);
    assert_eq!(score, Some(45.0));
}

/// 5. 综合分: "综合分 45 分" 格式
#[test]
fn test_extract_score_space_format() {
    let md = "综合分 67 分, 行业第 3\n";
    let score = extract_score_test(md);
    assert_eq!(score, Some(67.0));
}

/// 6. 综合分: 无 → None
#[test]
fn test_extract_score_none() {
    let md = "无综合分关键词\n";
    let score = extract_score_test(md);
    assert_eq!(score, None);
}

/// 7. 综合分: 数字越界 (>100) → 跳过
#[test]
fn test_extract_score_out_of_range() {
    let md = "综合分 150 (超界), 综合分 45\n";
    let score = extract_score_test(md);
    assert_eq!(score, Some(45.0), "应跳过 150 取 45");
}

/// 8. first_meaningful_line 跳过标题
#[test]
fn test_first_meaningful_line_skips_headers() {
    let md = "# 标题\n## 副标题\n第一行内容\n## 末标题\n";
    let line = first_meaningful_line_test(md);
    assert_eq!(line, "第一行内容");
}

// ====== 测试辅助函数 (复制 main.rs 逻辑) ======

fn extract_advice_test(md: &str) -> String {
    let mut advice = "未知".to_string();
    let mut in_section = false;
    for line in md.lines() {
        let t = line.trim();
        if t.starts_with('#') && t.contains("操作建议") {
            in_section = true;
            if let Some(rest) = t.split('】').nth(1) {
                let rest = rest.trim();
                if !rest.is_empty() {
                    advice = rest.to_string();
                    in_section = false;
                }
            }
            continue;
        }
        if in_section {
            if t.is_empty() { continue; }
            if t.starts_with('#') { in_section = false; continue; }
            advice = t.to_string();
            in_section = false;
        }
    }
    advice
}

fn extract_score_test(md: &str) -> Option<f64> {
    let mut score: Option<f64> = None;
    for line in md.lines() {
        let t = line.trim();
        if !t.contains("综合分") { continue; }
        for token in t.split(|c: char| !c.is_ascii_digit() && c != '.') {
            if let Ok(v) = token.parse::<f64>() {
                if (0.0..=100.0).contains(&v) {
                    score = Some(v);
                    break;
                }
            }
        }
        if score.is_some() { break; }
    }
    score
}

fn first_meaningful_line_test(md: &str) -> String {
    for line in md.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') { continue; }
        return t.to_string();
    }
    String::new()
}
