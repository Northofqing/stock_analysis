//! LLM 驱动的 ticker 提取 — 从新闻标题直接出 (code, name, importance, reason)
//!
//! 一劳永逸的关键: 让 LLM 认识所有公司名/产品名/产业链词, 不再维护 32 关键词表.
//! 离线 / 不可用 → 返回空 Vec, 业务降级到 chain_mapper 关键词路径.

use super::{LlmError, LlmProvider};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 提取出的个股信号
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TickerHit {
    /// 6 位 A 股代码 (e.g. "002916")
    pub code: String,
    /// 股票名称 (e.g. "深南电路")
    pub name: String,
    /// 1-10 重要度
    pub importance: u8,
    /// 1-2 句关联原因
    pub reason: String,
    /// 所属产业链 (e.g. "PCB", "AI 算力", "数据中心")
    pub chain: String,
}

const SYSTEM_PROMPT: &str = "你是 A 股产业链映射专家, 负责从新闻标题提取受益个股.\n\
输出严格 JSON, 不要任何解释文字, 不要 markdown 代码块.\n\
\n\
JSON schema:\n\
{\n  \
  \"hits\": [\n    \
    {\"code\": \"002916\", \"name\": \"深南电路\", \"importance\": 8, \"reason\": \"PCB 涨价直接受益\", \"chain\": \"PCB\"}\n  \
  ]\n\
}\n\
\n\
规则:\n\
1. code 必须是 6 位数字 A 股代码 (沪市 6 开头, 深市 0/3 开头, 创业板 300, 科创板 688). **不确定就跳过, 不要编造**\n\
2. importance 1-10: 10=政策级/订单级, 7-9=行业级催化, 4-6=题材级, 1-3=边缘相关\n\
3. 同一新闻可输出多只票 (产业链上下游/竞争对手), 按 importance 降序\n\
4. 公司名/产品名/技术名 (DeepSeek/Claude/IDC) 已知映射到产业链时输出对应票, 不映射到产业链时输出空 hits\n\
5. 与 watchlist sector 无关的票也允许输出 (大盘主线可能影响所有持仓)\n\
6. 重要度 < 4 的票不要输出, 避免噪声";

const USER_TEMPLATE: &str = "从以下 {n} 条新闻标题提取受益个股.\n\
每条标题独立分析, 输出综合的 hits 列表 (去重, 按 importance 降序).\n\
\n\
新闻标题:\n{titles}\n\
\n\
输出 JSON (无 markdown, 无解释):";

/// 调 LLM 提取 ticker. 失败 → 返回空 Vec (业务降级).
///
/// 设计: 单次调用处理所有 titles, 避免 N 次调用.
pub async fn extract_tickers(
    provider: Arc<dyn LlmProvider>,
    titles: Vec<String>,
) -> Result<Vec<TickerHit>, LlmError> {
    if titles.is_empty() {
        return Ok(Vec::new());
    }

    let titles_block = titles
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. {}", i + 1, t))
        .collect::<Vec<_>>()
        .join("\n");
    let user = USER_TEMPLATE
        .replace("{n}", &titles.len().to_string())
        .replace("{titles}", &titles_block);

    let value = provider.chat_json(SYSTEM_PROMPT, &user).await?;

    // 解析: 支持 {"hits": [...]} 或 [...] 两种格式
    let hits: Vec<TickerHit> = if let Some(arr) = value.as_array() {
        serde_json::from_value(serde_json::Value::Array(arr.clone())).unwrap_or_default()
    } else if let Some(arr) = value.get("hits").and_then(|v| v.as_array()) {
        serde_json::from_value(serde_json::Value::Array(arr.clone())).unwrap_or_default()
    } else {
        log::warn!(
            "[LLM ticker] 响应无 hits 字段: {}",
            value.to_string().chars().take(200).collect::<String>()
        );
        return Ok(Vec::new());
    };

    // 二次清洗: 6 位 code 校验, importance clamp, 同 code 取 importance 最高
    let mut by_code: std::collections::HashMap<String, TickerHit> =
        std::collections::HashMap::new();
    for mut h in hits {
        if h.code.len() != 6 || !h.code.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        h.importance = h.importance.clamp(1, 10);
        if h.importance < 4 {
            continue; // 过滤低分
        }
        // 同 code 多次出现, 保留 importance 最高
        match by_code.get(&h.code) {
            Some(existing) if existing.importance >= h.importance => {}
            _ => {
                by_code.insert(h.code.clone(), h);
            }
        }
    }
    let mut cleaned: Vec<TickerHit> = by_code.into_values().collect();
    cleaned.sort_by(|a, b| b.importance.cmp(&a.importance));

    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    /// Mock provider — 业务侧可注入假 LLM 测路径, 不打网络
    struct MockProvider {
        response: serde_json::Value,
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &'static str {
            "mock"
        }
        fn model(&self) -> &str {
            "mock-v1"
        }
        async fn chat_json(
            &self,
            _system: &str,
            _user: &str,
        ) -> Result<serde_json::Value, LlmError> {
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    async fn test_extract_tickers_filters_low_importance_and_validates_code() {
        let mock = MockProvider {
            response: json!({
                "hits": [
                    {"code": "002916", "name": "深南电路", "importance": 9, "reason": "PCB 龙头", "chain": "PCB"},
                    {"code": "002463", "name": "沪电股份", "importance": 7, "reason": "PCB 涨价", "chain": "PCB"},
                    {"code": "12345", "name": "非法代码", "importance": 8, "reason": "5 位", "chain": "?"},
                    {"code": "999999", "name": "低分票", "importance": 2, "reason": "边缘", "chain": "?"},
                    {"code": "002916", "name": "深南电路", "importance": 15, "reason": "超上限", "chain": "PCB"},
                ]
            }),
        };
        let hits = extract_tickers(Arc::new(mock), vec!["PCB 涨价 12%".into()])
            .await
            .unwrap();
        // 期望: 002916 (clamp 15→10), 002463; 12345/999999/duplicate-002916 被过滤
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].code, "002916");
        assert_eq!(hits[0].importance, 10, "importance 应 clamp 到 10");
        assert_eq!(hits[1].code, "002463");
    }

    #[tokio::test]
    async fn test_extract_tickers_handles_empty_input() {
        let mock = MockProvider {
            response: json!({"hits": []}),
        };
        let hits = extract_tickers(Arc::new(mock), vec![]).await.unwrap();
        assert!(hits.is_empty(), "空输入应短路返回");
    }

    #[tokio::test]
    async fn test_extract_tickers_handles_array_response() {
        // 兼容直接返回数组 (无 hits wrapper)
        let mock = MockProvider {
            response: json!([
                {"code": "002916", "name": "深南电路", "importance": 8, "reason": "PCB", "chain": "PCB"}
            ]),
        };
        let hits = extract_tickers(Arc::new(mock), vec!["x".into()])
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn test_extract_tickers_handles_bad_response() {
        // 响应无 hits 字段 → 返回空
        let mock = MockProvider {
            response: json!({"unrelated": "data"}),
        };
        let hits = extract_tickers(Arc::new(mock), vec!["x".into()])
            .await
            .unwrap();
        assert!(hits.is_empty());
    }

    /// 集成测试占位: 真实调 DeepSeek, 默认 ignore 避免 CI 跑
    #[tokio::test]
    #[ignore = "需 DEEPSEEK_API_KEY, 本地手动跑"]
    async fn test_extract_tickers_real_api() {
        if std::env::var("DEEPSEEK_API_KEY").is_err() {
            return;
        }
        let p = super::super::providers::DeepSeekProvider::from_env().unwrap();
        let hits = extract_tickers(Arc::new(p), vec!["国务院印发低空经济发展规划".into()])
            .await
            .unwrap();
        // 至少不 panic, 输出 0~N 条
        assert!(hits.len() <= 20);
    }
}
