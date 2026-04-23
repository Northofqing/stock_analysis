//! 飞书渠道实现 + Markdown→HTML 表格增强（供邮件和其他富文本渠道共用）

use anyhow::{Context, Result};
use log::info;
use serde_json::json;

use super::service::NotificationService;

impl NotificationService {
    /// 发送到飞书
    pub async fn send_to_feishu(&self, content: &str) -> Result<bool> {
        let url = self
            .config
            .feishu_webhook_url
            .as_ref()
            .context("飞书 Webhook 未配置")?;

        let formatted = self.format_feishu_markdown(content);
        let max_bytes = self.config.feishu_max_bytes;

        if formatted.as_bytes().len() > max_bytes {
            info!("飞书消息内容超长，将分批发送");
            return self.send_feishu_chunked(url, &formatted, max_bytes).await;
        }

        self.send_feishu_message(url, &formatted).await
    }

    pub(super) async fn send_feishu_message(&self, url: &str, content: &str) -> Result<bool> {
        // 优先使用交互卡片
        let card_payload = json!({
            "msg_type": "interactive",
            "card": {
                "config": {"wide_screen_mode": true},
                "header": {
                    "title": {
                        "tag": "plain_text",
                        "content": "A股智能分析报告"
                    }
                },
                "elements": [{
                    "tag": "div",
                    "text": {
                        "tag": "lark_md",
                        "content": content
                    }
                }]
            }
        });

        let response = self.client.post(url).json(&card_payload).send().await?;

        if response.status().is_success() {
            let result: serde_json::Value = response.json().await?;
            let code = result
                .get("code")
                .or_else(|| result.get("StatusCode"))
                .and_then(|v| v.as_i64())
                .unwrap_or(-1);

            if code == 0 {
                info!("飞书消息发送成功");
                return Ok(true);
            }
        }

        // 回退到普通文本
        let text_payload = json!({
            "msg_type": "text",
            "content": {
                "text": content
            }
        });

        let response = self.client.post(url).json(&text_payload).send().await?;
        Ok(response.status().is_success())
    }

    pub(super) async fn send_feishu_chunked(&self, url: &str, content: &str, max_bytes: usize) -> Result<bool> {
        let chunks = self.chunk_by_sections(content, max_bytes);
        let total = chunks.len();
        let mut success = 0;

        for (i, chunk) in chunks.iter().enumerate() {
            let marker = if total > 1 {
                format!("\n\n📄 ({}/{})", i + 1, total)
            } else {
                String::new()
            };

            if self.send_feishu_message(url, &format!("{}{}", chunk, marker)).await? {
                success += 1;
            }

            if i < total - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }

        Ok(success == total)
    }

    pub(super) fn format_feishu_markdown(&self, content: &str) -> String {
        use regex::Regex;

        let mut result = content.to_string();

        // 标题转加粗
        let re_h = Regex::new(r"^#{1,6}\s+(.+)$").unwrap();
        result = re_h
            .replace_all(&result, |caps: &regex::Captures| format!("**{}**", &caps[1]))
            .to_string();

        // 引用转前缀
        result = result.replace("> ", "💬 ");

        // 分隔线
        result = result.replace("---", "────────");

        // 列表
        result = result.replace("- ", "• ");

        result
    }

    /// 将 Markdown 转换为 HTML（优化邮件客户端兼容性）
    pub(super) fn markdown_to_html(&self, markdown: &str) -> String {
        let mut html = markdown.to_string();
        
        // 清理多余的空行（3个以上连续换行合并为2个）
        let re_multiple_newlines = regex::Regex::new(r"\n{3,}").unwrap();
        html = re_multiple_newlines.replace_all(&html, "\n\n").to_string();
        
        // 先处理表格（最重要的部分）
        html = self.convert_markdown_tables_enhanced(&html);
        
        // 处理引用块
        let re_quote = regex::Regex::new(r"(?m)^> (.+)$").unwrap();
        html = re_quote.replace_all(&html, 
            "<div style='border-left: 4px solid #3498db; padding: 10px 15px; margin: 15px 0; background-color: #f8f9fa; color: #555;'>$1</div>").to_string();
        
        // 处理标题（从小到大避免冲突）
        let re_h4 = regex::Regex::new(r"(?m)^####\s+(.+)$").unwrap();
        html = re_h4.replace_all(&html, 
            "<h4 style='color: #666; margin: 15px 0 10px 0; font-size: 16px;'>$1</h4>").to_string();
        
        let re_h3 = regex::Regex::new(r"(?m)^###\s+(.+)$").unwrap();
        html = re_h3.replace_all(&html, 
            "<h3 style='color: #555; margin: 20px 0 10px 0; font-size: 18px;'>$1</h3>").to_string();
        
        let re_h2 = regex::Regex::new(r"(?m)^##\s+(.+)$").unwrap();
        html = re_h2.replace_all(&html, 
            "<h2 style='color: #34495e; margin: 25px 0 15px 0; padding-left: 10px; border-left: 4px solid #3498db; font-size: 20px;'>$1</h2>").to_string();
        
        let re_h1 = regex::Regex::new(r"(?m)^#\s+(.+)$").unwrap();
        html = re_h1.replace_all(&html, 
            "<h1 style='color: #2c3e50; border-bottom: 3px solid #3498db; padding-bottom: 10px; margin: 30px 0 20px 0; font-size: 24px;'>$1</h1>").to_string();
        
        // 处理粗体
        let re_bold = regex::Regex::new(r"\*\*(.+?)\*\*").unwrap();
        html = re_bold.replace_all(&html, "<strong style='color: #e74c3c; font-weight: bold;'>$1</strong>").to_string();
        
        // 处理分隔线
        html = html.replace("\n---\n", "\n<hr style='border: none; border-top: 2px solid #ecf0f1; margin: 20px 0;'/>\n");
        
        // 处理列表
        html = self.convert_markdown_lists(&html);
        
        // 清理HTML标签周围的多余换行
        // 移除标签前后的空白行
        let re_clean_before_tags = regex::Regex::new(r"\n+(<(?:table|h[1-4]|ul|div|hr))").unwrap();
        html = re_clean_before_tags.replace_all(&html, "\n$1").to_string();
        
        let re_clean_after_tags = regex::Regex::new(r"(</(?:table|h[1-4]|ul|div)>)\n+").unwrap();
        html = re_clean_after_tags.replace_all(&html, "$1\n").to_string();
        
        // 移除表格、列表等块级元素内部的单独换行符（但保留有内容的行）
        // 这一步要在段落处理之前
        let re_empty_lines_in_blocks = regex::Regex::new(r"(<(?:table|thead|tbody|tr|ul)>)\n+").unwrap();
        html = re_empty_lines_in_blocks.replace_all(&html, "$1").to_string();
        
        let re_empty_lines_after_blocks = regex::Regex::new(r"\n+(</(?:table|thead|tbody|tr|ul)>)").unwrap();
        html = re_empty_lines_after_blocks.replace_all(&html, "$1").to_string();
        
        // 最后处理剩余的文本换行
        // 只对纯文本段落添加 <br/>，而不是所有换行
        let lines: Vec<&str> = html.lines().collect();
        let mut final_lines = Vec::new();
        
        for line in lines {
            let trimmed = line.trim();
            // 跳过空行
            if trimmed.is_empty() {
                continue;
            }
            // 如果是HTML标签行，直接保留
            if trimmed.starts_with('<') {
                final_lines.push(line.to_string());
            } else {
                // 普通文本行，如果前一行也是文本，添加<br/>
                if !final_lines.is_empty() {
                    let last_line = final_lines.last().unwrap();
                    if !last_line.trim().starts_with('<') && !last_line.trim().ends_with('>') {
                        final_lines.push("<br/>".to_string());
                    }
                }
                final_lines.push(line.to_string());
            }
        }
        html = final_lines.join("\n");
        
        // 包装完整HTML
        format!(
            "<!DOCTYPE html>
<html>
<head>
    <meta charset='UTF-8'>
    <meta name='viewport' content='width=device-width, initial-scale=1.0'>
</head>
<body style='font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", \"Microsoft YaHei\", Arial, sans-serif; line-height: 1.6; padding: 20px; background-color: #ffffff; color: #333; max-width: 1200px; margin: 0 auto;'>
{}
<div style='text-align: center; margin-top: 40px; padding-top: 20px; border-top: 1px solid #ecf0f1; color: #999; font-size: 12px;'>
    <p>本邮件由A股分析系统自动生成</p>
</div>
</body>
</html>",
            html
        )
    }

    /// 转换Markdown列表为HTML（优化版）
    pub(super) fn convert_markdown_lists(&self, content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::new();
        let mut in_list = false;
        
        for line in lines {
            if line.trim().starts_with("- ") {
                if !in_list {
                    result.push("<ul style='margin: 10px 0; padding-left: 25px;'>".to_string());
                    in_list = true;
                }
                let content = line.trim_start_matches("- ").trim();
                result.push(format!("<li style='margin: 5px 0;'>{}</li>", content));
            } else {
                if in_list {
                    result.push("</ul>".to_string());
                    in_list = false;
                }
                result.push(line.to_string());
            }
        }
        if in_list {
            result.push("</ul>".to_string());
        }
        result.join("\n")
    }

    /// 转换Markdown表格为HTML（增强版，完全内联样式）
    pub(super) fn convert_markdown_tables_enhanced(&self, content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::new();
        let mut i = 0;
        
        while i < lines.len() {
            let line = lines[i];
            
            // 检测表格开始
            if line.contains('|') && line.split('|').filter(|s| !s.trim().is_empty()).count() >= 2 {
                // 检查下一行是否是分隔符
                let is_table_start = if i + 1 < lines.len() {
                    lines[i + 1].contains("---") || lines[i + 1].contains("|-")
                } else {
                    false
                };
                
                if is_table_start {
                    // 表格样式（内联）
                    let table_style = "width: 100%; border-collapse: collapse; margin: 15px 0; background-color: #ffffff; box-shadow: 0 1px 3px rgba(0,0,0,0.1);";
                    let th_style = "background-color: #3498db; color: #ffffff; padding: 12px; text-align: left; font-weight: bold; border: 1px solid #2980b9;";
                    let td_style = "padding: 10px 12px; border: 1px solid #ecf0f1; background-color: #ffffff;";
                    
                    result.push(format!("<table style='{}'>", table_style));
                    
                    // 处理表头
                    let headers: Vec<&str> = line.split('|')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect();
                    
                    result.push("<thead>".to_string());
                    result.push("<tr>".to_string());
                    for header in headers {
                        result.push(format!("<th style='{}'>{}</th>", th_style, header));
                    }
                    result.push("</tr>".to_string());
                    result.push("</thead>".to_string());
                    
                    // 跳过分隔符行
                    i += 2;
                    
                    // 处理表格数据行
                    result.push("<tbody>".to_string());
                    let mut row_index = 0;
                    while i < lines.len() {
                        let data_line = lines[i];
                        if !data_line.contains('|') || data_line.trim().is_empty() {
                            break;
                        }
                        
                        let cells: Vec<&str> = data_line.split('|')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .collect();
                        
                        if !cells.is_empty() {
                            // 交替行背景色
                            let row_bg = if row_index % 2 == 0 { "#ffffff" } else { "#f8f9fa" };
                            result.push(format!("<tr style='background-color: {};'>", row_bg));
                            for cell in cells {
                                // 处理单元格内容中的emoji和颜色标记
                                let cell_content = self.enhance_cell_content(cell);
                                result.push(format!("<td style='{}'>{}</td>", td_style, cell_content));
                            }
                            result.push("</tr>".to_string());
                            row_index += 1;
                        }
                        i += 1;
                    }
                    result.push("</tbody>".to_string());
                    result.push("</table>".to_string());
                    continue;
                }
            }
            
            result.push(line.to_string());
            i += 1;
        }
        
        result.join("\n")
    }

    /// 增强单元格内容显示（处理emoji和特殊标记）
    pub(super) fn enhance_cell_content(&self, content: &str) -> String {
        let mut enhanced = content.to_string();
        
        // 处理emoji - 使用更兼容的方式
        enhanced = enhanced.replace("✅", "<span style='color: #27ae60;'>✅</span>");
        enhanced = enhanced.replace("⚠️", "<span style='color: #f39c12;'>⚠️</span>");
        enhanced = enhanced.replace("🔴", "<span style='color: #e74c3c;'>🔴</span>");
        enhanced = enhanced.replace("🟢", "<span style='color: #27ae60;'>●</span>");
        enhanced = enhanced.replace("🟡", "<span style='color: #f39c12;'>●</span>");
        enhanced = enhanced.replace("📊", "📊");
        enhanced = enhanced.replace("📈", "📈");
        enhanced = enhanced.replace("💰", "💰");
        enhanced = enhanced.replace("🎯", "🎯");
        
        // 处理评估标签的颜色
        if enhanced.contains("合理") || enhanced.contains("正常") || enhanced.contains("低估") {
            enhanced = format!("<span style='color: #27ae60;'>{}</span>", enhanced);
        } else if enhanced.contains("偏高") || enhanced.contains("较高") || enhanced.contains("亏损") {
            enhanced = format!("<span style='color: #e74c3c;'>{}</span>", enhanced);
        } else if enhanced.contains("适中") || enhanced.contains("中性") {
            enhanced = format!("<span style='color: #f39c12;'>{}</span>", enhanced);
        }
        
        enhanced
    }
}
