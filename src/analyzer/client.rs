//! AI 模型 HTTP 调用层（从 analyzer.rs 拆分）。
//!
//! 封装 Gemini / OpenAI 兼容 / 豆包 三家 API 的重试与故障转移逻辑。

use anyhow::{anyhow, Context, Result};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::GeminiAnalyzer;

impl GeminiAnalyzer {
    /// 调用 API（带重试和故障转移，使用默认系统提示词）
    pub(super) async fn call_api_with_retry(&self, prompt: &str) -> Result<String> {
        self.call_api_with_retry_ex(prompt, Self::SYSTEM_PROMPT).await
    }

    /// 调用 API（带重试和故障转移，自定义系统提示词）
    pub(super) async fn call_api_with_retry_ex(&self, prompt: &str, system_prompt: &str) -> Result<String> {
        if self.use_doubao {
            return self.call_doubao_api(prompt, system_prompt).await;
        }
        
        if self.use_openai {
            return self.call_openai_api(prompt, system_prompt).await;
        }

        let mut last_error = None;

        for attempt in 0..self.config.max_retries {
            if attempt > 0 {
                let delay = self.config.retry_delay * 2_f64.powi(attempt as i32 - 1);
                let delay = delay.min(60.0);
                info!("[Gemini] 第 {} 次重试，等待 {:.1} 秒...", attempt + 1, delay);
                tokio::time::sleep(Duration::from_secs_f64(delay)).await;
            }

            match self.call_gemini_api(prompt, system_prompt).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    last_error = Some(e);
                    let error_str = last_error.as_ref().unwrap().to_string();
                    
                    let is_rate_limit = error_str.contains("429") 
                        || error_str.to_lowercase().contains("quota")
                        || error_str.to_lowercase().contains("rate");

                    if is_rate_limit {
                        warn!(
                            "[Gemini] API 限流 (429)，第 {}/{} 次尝试",
                            attempt + 1,
                            self.config.max_retries
                        );

                        // 切换到备选模型
                        if attempt >= self.config.max_retries / 2 && !*self.using_fallback.borrow() {
                            self.switch_to_fallback();
                        }
                    } else {
                        warn!(
                            "[Gemini] API 调用失败，第 {}/{} 次尝试: {}",
                            attempt + 1,
                            self.config.max_retries,
                            &error_str[..100.min(error_str.len())]
                        );
                    }
                }
            }
        }

        // 尝试豆包作为第一备选
        if self.config.doubao_api_key.is_some() {
            warn!("[Gemini] 所有重试失败，切换到豆包 API");
            match self.call_doubao_api(prompt, system_prompt).await {
                Ok(response) => return Ok(response),
                Err(doubao_error) => {
                    error!("[豆包] 备选 API 也失败: {}", doubao_error);
                }
            }
        }

        // 尝试 OpenAI 作为最后的备选
        if self.config.openai_api_key.is_some() {
            warn!("[Gemini] 切换到 OpenAI 兼容 API");
            match self.call_openai_api(prompt, system_prompt).await {
                Ok(response) => return Ok(response),
                Err(openai_error) => {
                    error!("[OpenAI] 备选 API 也失败: {}", openai_error);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("所有 AI API 调用失败")))
    }

    /// 调用 Gemini API
    async fn call_gemini_api(&self, prompt: &str, system_prompt: &str) -> Result<String> {
        #[derive(Serialize)]
        struct GeminiRequest {
            contents: Vec<Content>,
            #[serde(skip_serializing_if = "Option::is_none")]
            system_instruction: Option<SystemInstruction>,
            generation_config: GenerationConfig,
        }

        #[derive(Serialize)]
        struct Content {
            parts: Vec<Part>,
        }

        #[derive(Serialize)]
        struct Part {
            text: String,
        }

        #[derive(Serialize)]
        struct SystemInstruction {
            parts: Vec<Part>,
        }

        #[derive(Serialize)]
        struct GenerationConfig {
            temperature: f32,
            max_output_tokens: u32,
        }

        #[derive(Deserialize)]
        struct GeminiResponse {
            candidates: Vec<Candidate>,
        }

        #[derive(Deserialize)]
        struct Candidate {
            content: ResponseContent,
        }

        #[derive(Deserialize)]
        struct ResponseContent {
            parts: Vec<ResponsePart>,
        }

        #[derive(Deserialize)]
        struct ResponsePart {
            text: String,
        }

        let api_key = self.config.api_key.as_ref().ok_or_else(|| anyhow!("Gemini API Key 未配置"))?;
        
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.current_model.borrow(), api_key
        );

        let request = GeminiRequest {
            contents: vec![Content {
                parts: vec![Part {
                    text: prompt.to_string(),
                }],
            }],
            system_instruction: Some(SystemInstruction {
                parts: vec![Part {
                    text: system_prompt.to_string(),
                }],
            }),
            generation_config: GenerationConfig {
                temperature: 0.7,
                max_output_tokens: 8192,
            },
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Gemini API 请求失败")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(anyhow!("HTTP {}: {}", status, error_text));
        }

        let gemini_response: GeminiResponse = response.json().await.context("解析 Gemini 响应失败")?;

        gemini_response
            .candidates
            .get(0)
            .and_then(|c| c.content.parts.get(0))
            .map(|p| p.text.clone())
            .ok_or_else(|| anyhow!("Gemini 返回空响应"))
    }

    /// 调用 OpenAI 兼容 API
    async fn call_openai_api(&self, prompt: &str, system_prompt: &str) -> Result<String> {
        #[derive(Serialize)]
        struct OpenAIRequest {
            model: String,
            messages: Vec<Message>,
            temperature: f32,
            max_tokens: u32,
        }

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: String,
        }

        #[derive(Deserialize)]
        struct OpenAIResponse {
            choices: Vec<Choice>,
        }

        #[derive(Deserialize)]
        struct Choice {
            message: ResponseMessage,
        }

        #[derive(Deserialize)]
        struct ResponseMessage {
            content: String,
        }

        let api_key = self.config.openai_api_key.as_ref().ok_or_else(|| anyhow!("OpenAI API Key 未配置"))?;
        
        let base_url = self.config.openai_base_url.as_deref().unwrap_or("https://api.openai.com/v1");
        let url = format!("{}/chat/completions", base_url);

        let request = OpenAIRequest {
            model: self.config.openai_model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: prompt.to_string(),
                },
            ],
            temperature: 0.7,
            max_tokens: 8192,
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await
            .context("OpenAI API 请求失败")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(anyhow!("HTTP {}: {}", status, error_text));
        }

        let openai_response: OpenAIResponse = response.json().await.context("解析 OpenAI 响应失败")?;

        openai_response
            .choices
            .get(0)
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("OpenAI 返回空响应"))
    }

    /// 调用豆包 (Doubao) API
    async fn call_doubao_api(&self, prompt: &str, system_prompt: &str) -> Result<String> {
        #[derive(Serialize)]
        struct DoubaoRequest {
            model: String,
            messages: Vec<Message>,
            temperature: f32,
            max_tokens: u32,
        }

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: String,
        }

        #[derive(Deserialize)]
        struct DoubaoResponse {
            choices: Vec<Choice>,
        }

        #[derive(Deserialize)]
        struct Choice {
            message: ResponseMessage,
        }

        #[derive(Deserialize)]
        struct ResponseMessage {
            content: String,
        }

        let api_key = self.config.doubao_api_key.as_ref().ok_or_else(|| anyhow!("豆包 API Key 未配置"))?;
        
        let base_url = self.config.doubao_base_url.as_deref()
            .unwrap_or("https://ark.cn-beijing.volces.com/api/v3");
        let url = format!("{}/chat/completions", base_url);

        let request = DoubaoRequest {
            model: self.config.doubao_model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: prompt.to_string(),
                },
            ],
            temperature: 0.7,
            max_tokens: 8192,
        };

        info!("[豆包] 调用 API: {} (model: {})", url, self.config.doubao_model);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await
            .context("豆包 API 请求失败")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            error!("[豆包] API 错误: HTTP {}: {}", status, error_text);
            return Err(anyhow!("HTTP {}: {}", status, error_text));
        }

        let doubao_response: DoubaoResponse = response.json().await.context("解析豆包响应失败")?;

        doubao_response
            .choices
            .get(0)
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("豆包返回空响应"))
    }

}
