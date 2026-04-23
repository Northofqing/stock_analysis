//! 企业微信渠道实现

use anyhow::{Context, Result};
use log::{error, info};
use serde_json::json;

use super::service::NotificationService;

impl NotificationService {
    /// 发送到企业微信
    pub async fn send_to_wechat(&self, content: &str) -> Result<bool> {
        let url = self
            .config
            .wechat_webhook_url
            .as_ref()
            .context("企业微信 Webhook 未配置")?;

        let max_bytes = self.config.wechat_max_bytes;
        let content_bytes = content.as_bytes().len();

        if content_bytes > max_bytes {
            info!("消息内容超长({}字节)，将分批发送", content_bytes);
            return self.send_wechat_chunked(url, content, max_bytes).await;
        }

        self.send_wechat_message(url, content).await
    }

    /// 发送单条企业微信消息
    pub(super) async fn send_wechat_message(&self, url: &str, content: &str) -> Result<bool> {
        let payload = json!({
            "msgtype": "markdown",
            "markdown": {
                "content": content
            }
        });

        let response = self.client.post(url).json(&payload).send().await?;

        if response.status().is_success() {
            let result: serde_json::Value = response.json().await?;
            if result.get("errcode").and_then(|v| v.as_i64()) == Some(0) {
                info!("企业微信消息发送成功");
                Ok(true)
            } else {
                error!("企业微信返回错误: {:?}", result);
                Ok(false)
            }
        } else {
            error!("企业微信请求失败: {}", response.status());
            Ok(false)
        }
    }

    /// 分批发送长消息到企业微信
    pub(super) async fn send_wechat_chunked(&self, url: &str, content: &str, max_bytes: usize) -> Result<bool> {
        let chunks = self.chunk_by_sections(content, max_bytes);
        let total_chunks = chunks.len();
        let mut success_count = 0;

        info!("企业微信分批发送：共 {} 批", total_chunks);

        for (i, chunk) in chunks.iter().enumerate() {
            let page_marker = if total_chunks > 1 {
                format!("\n\n📄 *({}/{})*", i + 1, total_chunks)
            } else {
                String::new()
            };

            let chunk_with_marker = format!("{}{}", chunk, page_marker);

            if self.send_wechat_message(url, &chunk_with_marker).await? {
                success_count += 1;
                info!("企业微信第 {}/{} 批发送成功", i + 1, total_chunks);
            } else {
                error!("企业微信第 {}/{} 批发送失败", i + 1, total_chunks);
            }

            if i < total_chunks - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }

        Ok(success_count == total_chunks)
    }

    /// 按段落智能分割内容
    pub(super) fn chunk_by_sections(&self, content: &str, max_bytes: usize) -> Vec<String> {
        let sections: Vec<&str> = if content.contains("\n---\n") {
            content.split("\n---\n").collect()
        } else if content.contains("\n### ") {
            let parts: Vec<&str> = content.split("\n### ").collect();
            let mut result = vec![parts[0]];
            let formatted_parts: Vec<String> = parts[1..].iter().map(|p| format!("### {}", p)).collect();
            result.extend(formatted_parts.iter().map(|s| s.as_str()));
            return self.chunk_sections(&result, max_bytes);
        } else {
            vec![content]
        };

        self.chunk_sections(&sections, max_bytes)
    }

    pub(super) fn chunk_sections(&self, sections: &[&str], max_bytes: usize) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut current_chunk = Vec::new();
        let mut current_bytes = 0;

        for section in sections {
            let section_bytes = section.as_bytes().len();

            if section_bytes > max_bytes {
                if !current_chunk.is_empty() {
                    chunks.push(current_chunk.join("\n---\n"));
                    current_chunk.clear();
                    current_bytes = 0;
                }

                let truncated = self.truncate_to_bytes(section, max_bytes - 200);
                chunks.push(format!("{}\n\n...(本段内容过长已截断)", truncated));
                continue;
            }

            if current_bytes + section_bytes > max_bytes {
                if !current_chunk.is_empty() {
                    chunks.push(current_chunk.join("\n---\n"));
                }
                current_chunk = vec![section.to_string()];
                current_bytes = section_bytes;
            } else {
                current_chunk.push(section.to_string());
                current_bytes += section_bytes;
            }
        }

        if !current_chunk.is_empty() {
            chunks.push(current_chunk.join("\n---\n"));
        }

        chunks
    }

    pub(super) fn truncate_to_bytes(&self, text: &str, max_bytes: usize) -> String {
        if text.as_bytes().len() <= max_bytes {
            return text.to_string();
        }

        let mut result = String::new();
        let mut current_bytes = 0;

        for c in text.chars() {
            let char_bytes = c.len_utf8();
            if current_bytes + char_bytes > max_bytes {
                break;
            }
            result.push(c);
            current_bytes += char_bytes;
        }

        result
    }
}
