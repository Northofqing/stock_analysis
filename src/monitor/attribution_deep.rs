//! G5b 深链归因 — AI 深链版异动归因 (盘后批量, 2026-08-22 落地)。
//!
//! Registered business rules: BR-045, BR-181.
//! 设计: monitor/attribution.rs (G5a) 注释「AI 深链归 G5b (盘后/手动)」。
//!
//! G5a = 盘中规则快归因 (P95 ≤ 2s, 不用 LLM); G5b = 盘后 LLM 深链归因:
//! - 输入: 当日 alert_log JSONL 的 AlertRecord (与 G5a 同源事件)
//! - LLM: 复用 LlmRegistry 模型通道 (DeepSeek 优先), 45s 超时 + receipt 保真
//! - 输出: strict JSON {main_reason, catalyst_chain, capital_logic, confidence, risk_note}
//! - 落库: data/g5b/{date}.jsonl (含 receipt), 失败出声不静默
//! - 消费: 15:05 归因闭环追加深链段 + PushKind::G5bAttribution 独立推送
//!
//! 与 G5a 的关系: G5a 的 attribution_decision 作为输入上下文喂给 LLM,
//! 深链验证/深化规则结论, 不是取代。

use crate::llm::{LlmError, LlmProvider, ModelCallReceipt, ReceiptBearingJson};
use crate::monitor::alert_log::AlertRecord;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

/// 深链归因模型调用超时 (与 news_ai 同档, 45s)。
const MODEL_CALL_TIMEOUT_SECONDS: u64 = 45;
/// 单日深链事件上限 (模型调用成本护栏, 优先级取前 N)。
pub const DEEP_ATTRIBUTION_MAX_EVENTS: usize = 3;

/// G5b 系统 prompt v1: 深链归因角色 + strict JSON schema。
pub const G5B_SYSTEM_PROMPT_V1: &str = r#"你是 A 股异动深链归因分析师 (虚拟盘研究, 非投资建议)。
输入: 当日异动告警记录 (含 G5a 规则快归因结论)。
任务: 用你的市场知识深链分析异动根因, 输出 strict JSON (无 markdown 围栏, 无多余文字):

{
  "main_reason": "一句话主因, ≤40 字",
  "catalyst_chain": ["链上证据 1", "链上证据 2", "链上证据 3"],  // 1-3 条, 每条 ≤30 字
  "capital_logic": "资金逻辑, ≤60 字",
  "confidence": "high 或 medium 或 low",
  "risk_note": "风险提示, ≤40 字"
}

约束: 证据不足时 confidence=low 并明示; 不得编造新闻/公告/数据。"#;

/// G5b 分析请求 (输入 = 当日告警记录 + 观测时刻)。
#[derive(Debug, Clone)]
pub struct DeepAttributionRequest {
    pub record: AlertRecord,
    pub as_of: DateTime<Utc>,
}

/// strict JSON 解析产物。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeepAttributionResult {
    pub main_reason: String,
    pub catalyst_chain: Vec<String>,
    pub capital_logic: String,
    pub confidence: String,
    pub risk_note: String,
}

/// assess() 产物: 解析结果 + 模型回执 (落库保真)。
#[derive(Debug, Clone)]
pub struct DeepAttributionOutcome {
    pub result: DeepAttributionResult,
    pub receipt: ModelCallReceipt,
    pub elapsed_ms: u64,
}

/// 落库行: 请求 + 结果 + 模型 receipt (与 news_ai 审计同档保真)。
#[derive(Debug, Clone, Serialize)]
pub struct DeepAttributionRow {
    pub record: AlertRecord,
    pub result: DeepAttributionResult,
    pub analyzed_at: String,
    pub provider: String,
    pub model: String,
    pub upstream_request_id: Option<String>,
    pub upstream_response_id: Option<String>,
    pub elapsed_ms: u64,
}

/// G5b 分析器 (side-effect-free, 仿 NewsAIAnalyzer)。
#[derive(Clone)]
pub struct DeepAttributionAnalyzer {
    provider: Arc<dyn LlmProvider>,
}

/// G5b 错误 (出声语义, 不静默折叠)。
#[derive(Debug)]
pub enum DeepAttributionError {
    ModelUnavailable(String),
    InvalidModelSchema(String),
    Io(String),
}

impl std::fmt::Display for DeepAttributionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelUnavailable(m) => write!(f, "model unavailable: {m}"),
            Self::InvalidModelSchema(m) => write!(f, "invalid model schema: {m}"),
            Self::Io(m) => write!(f, "io: {m}"),
        }
    }
}

impl std::error::Error for DeepAttributionError {}

impl DeepAttributionAnalyzer {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// 深链归因: 45s 超时 + receipt 保真。
    /// provider 无回执能力 (ReceiptUnavailable) → 显式失败 (fail-closed, 与 news_ai 同策)。
    pub async fn assess(
        &self,
        request: &DeepAttributionRequest,
    ) -> Result<DeepAttributionOutcome, DeepAttributionError> {
        let user_prompt = deep_attribution_prompt(request);
        let started = std::time::Instant::now();
        let completed: ReceiptBearingJson = tokio::time::timeout(
            std::time::Duration::from_secs(MODEL_CALL_TIMEOUT_SECONDS),
            self.provider
                .chat_json_with_receipt(G5B_SYSTEM_PROMPT_V1, &user_prompt),
        )
        .await
        .map_err(|_| {
            DeepAttributionError::ModelUnavailable(format!(
                "model call exceeded {MODEL_CALL_TIMEOUT_SECONDS}s"
            ))
        })?
        .map_err(deep_model_call_error)?;
        let (_, raw_response, receipt) = completed.into_parts();
        let result = parse_deep_attribution_output(&raw_response).map_err(|e| {
            DeepAttributionError::InvalidModelSchema(format!(
                "G5b 深链归因输出无法按 v1 schema 解析: {e}"
            ))
        })?;
        Ok(DeepAttributionOutcome {
            result,
            receipt,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

/// user prompt 构建: 告警记录全字段 → 模型输入 (缺字段明示 absent, 不编造)。
pub fn deep_attribution_prompt(request: &DeepAttributionRequest) -> String {
    let r = &request.record;
    format!(
        "告警时间: {triggered}\n\
         代码: {code} {name}\n\
         级别: {level} | 类别: {category}\n\
         消息: {message}\n\
         价格: {price}\n\
         涨跌幅: {change_pct}%\n\
         主力净流入(亿): {main_flow}\n\
         关联新闻: {news_title}\n\
         新闻重要度: {news_importance}\n\
         G5a 规则快归因: {attribution_decision}\n\
         T1 锁定: {t1_locked}",
        triggered = r.triggered_at,
        code = r.code,
        name = r.name,
        level = r.level,
        category = r.category,
        message = r.message,
        price = r
            .price
            .map(|v| v.to_string())
            .unwrap_or_else(|| "absent".to_string()),
        change_pct = r
            .change_pct
            .map(|v| v.to_string())
            .unwrap_or_else(|| "absent".to_string()),
        main_flow = r
            .main_flow_yi
            .map(|v| v.to_string())
            .unwrap_or_else(|| "absent".to_string()),
        news_title = r.news_title.as_deref().unwrap_or("absent"),
        news_importance = r
            .news_importance
            .map(|v| v.to_string())
            .unwrap_or_else(|| "absent".to_string()),
        attribution_decision = r.attribution_decision.as_deref().unwrap_or("absent"),
        t1_locked = r.t1_locked,
    )
}

/// strict JSON 解析: 只接受完整 5 字段; 缺失/类型错 → 显式错误。
pub fn parse_deep_attribution_output(response: &str) -> Result<DeepAttributionResult, String> {
    let trimmed = response.trim();
    // 兼容模型可能输出的 markdown 围栏 (实际要求无, 但剥离后仍按 strict 校验)。
    let json_body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.strip_suffix("```").unwrap_or(s))
        .unwrap_or(trimmed);
    let value: serde_json::Value = serde_json::from_str(json_body).map_err(|e| {
        format!(
            "JSON 解析失败: {e} (原文: {})",
            truncated_for_error(trimmed)
        )
    })?;
    let obj = value.as_object().ok_or("顶层必须是 JSON 对象")?;
    let main_reason = required_string(obj, "main_reason")?;
    let catalyst_chain = match obj.get("catalyst_chain") {
        Some(serde_json::Value::Array(items)) => {
            let chains: Result<Vec<String>, String> = items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "catalyst_chain 元素必须是字符串".to_string())
                })
                .collect();
            chains?
        }
        Some(_) => return Err("catalyst_chain 必须是数组".to_string()),
        None => return Err("缺少 catalyst_chain 字段".to_string()),
    };
    let capital_logic = required_string(obj, "capital_logic")?;
    let confidence = required_string(obj, "confidence")?;
    if !matches!(confidence.as_str(), "high" | "medium" | "low") {
        return Err(format!(
            "confidence 必须是 high/medium/low, 收到: {confidence}"
        ));
    }
    let risk_note = required_string(obj, "risk_note")?;
    Ok(DeepAttributionResult {
        main_reason,
        catalyst_chain,
        capital_logic,
        confidence,
        risk_note,
    })
}

fn required_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("缺少或非字符串字段: {key}"))
}

fn truncated_for_error(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(160).collect();
    if chars.next().is_some() {
        format!("{head}…(截断)")
    } else {
        head
    }
}

fn deep_model_call_error(error: LlmError) -> DeepAttributionError {
    DeepAttributionError::ModelUnavailable(error.to_string())
}

/// 当日事件优先级筛选: Emergency > Important > Info, 最多 max 个。
/// 同级别按告警时间先后 (sort_by_key 稳定), 不足 max 时全取。
pub fn top_events_for_deep(records: Vec<AlertRecord>, max: usize) -> Vec<AlertRecord> {
    let mut events = records;
    let priority = |level: &str| match level {
        "紧急" => 0usize,
        "重要" => 1,
        _ => 2,
    };
    events.sort_by_key(|r| priority(&r.level));
    events.truncate(max);
    events
}

/// 落库: data/g5b/{date}.jsonl (行追加, 失败显式返回)。
pub fn append_deep_attribution_row(row: &DeepAttributionRow) -> Result<(), DeepAttributionError> {
    let dir = PathBuf::from("data/g5b");
    fs::create_dir_all(&dir).map_err(|e| DeepAttributionError::Io(e.to_string()))?;
    let path = dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y-%m-%d")));
    let mut line =
        serde_json::to_string(row).map_err(|e| DeepAttributionError::Io(e.to_string()))?;
    line.push('\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(line.as_bytes())
        })
        .map_err(|e| DeepAttributionError::Io(format!("写入 {path:?}: {e}")))
}

/// 渲染单条深链归因 markdown 段 (15:05 报告追加 + 推送文本复用)。
pub fn render_deep_attribution(row: &DeepAttributionRow) -> String {
    let r = &row.record;
    let chains = if row.result.catalyst_chain.is_empty() {
        "  - （无链上证据）".to_string()
    } else {
        row.result
            .catalyst_chain
            .iter()
            .map(|c| format!("  - {c}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "## G5b 深链归因: {code} {name} ({category} / {level})\n\
         主因: {main_reason}\n\
         催化剂链:\n{chains}\n\
         资金逻辑: {capital_logic}\n\
         置信度: {confidence} | 风险: {risk_note}\n\
         (模型 {provider}/{model}, {elapsed_ms}ms, 告警 {triggered})",
        code = r.code,
        name = r.name,
        category = r.category,
        level = r.level,
        main_reason = row.result.main_reason,
        capital_logic = row.result.capital_logic,
        confidence = row.result.confidence,
        risk_note = row.result.risk_note,
        provider = row.provider,
        model = row.model,
        elapsed_ms = row.elapsed_ms,
        triggered = r.triggered_at,
    )
}

/// 推送摘要 (单条, 供 PushKind::G5bAttribution 独立推送)。
pub fn render_deep_attribution_summary(row: &DeepAttributionRow) -> String {
    let r = &row.record;
    format!(
        "🔍 {code} {name} ({category}) 深链归因\n\
         主因: {main_reason}\n\
         逻辑: {capital_logic}\n\
         置信度: {confidence} | 风险: {risk_note}",
        code = r.code,
        name = r.name,
        category = r.category,
        main_reason = row.result.main_reason,
        capital_logic = row.result.capital_logic,
        confidence = row.result.confidence,
        risk_note = row.result.risk_note,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmError;
    use serde_json::Value;
    use std::sync::Arc;

    fn sample_record() -> AlertRecord {
        AlertRecord {
            triggered_at: "2026-08-22T14:00:00+08:00".to_string(),
            code: "600396".to_string(),
            name: "金山股份".to_string(),
            level: "重要".to_string(),
            category: "主力突袭".to_string(),
            message: "盘中 14:00 主力资金突袭".to_string(),
            price: Some(14.28),
            change_pct: Some(5.2),
            main_flow_yi: Some(0.8),
            news_title: Some("公司中标新能源项目".to_string()),
            news_importance: Some(5),
            attribution_decision: Some("NewsCatalyst".to_string()),
            routed_external_id: None,
            t1_locked: false,
        }
    }

    fn sample_request() -> DeepAttributionRequest {
        DeepAttributionRequest {
            record: sample_record(),
            as_of: Utc::now(),
        }
    }

    #[test]
    fn prompt_contains_all_record_fields() {
        let prompt = deep_attribution_prompt(&sample_request());
        for needle in [
            "600396",
            "金山股份",
            "重要",
            "主力突袭",
            "14.28",
            "5.2",
            "0.8",
            "公司中标新能源项目",
            "5",
            "NewsCatalyst",
            "false",
        ] {
            assert!(
                prompt.contains(needle),
                "prompt 缺少字段: {needle}\n{prompt}"
            );
        }
    }

    #[test]
    fn prompt_absent_fields_are_explicit() {
        let mut record = sample_record();
        record.price = None;
        record.news_title = None;
        record.attribution_decision = None;
        let request = DeepAttributionRequest {
            record,
            as_of: Utc::now(),
        };
        let prompt = deep_attribution_prompt(&request);
        assert!(
            prompt.contains("absent"),
            "缺失字段必须明示 absent:\n{prompt}"
        );
    }

    #[test]
    fn parse_valid_output() {
        let out = parse_deep_attribution_output(
            r#"{"main_reason":"中标新能源项目催化","catalyst_chain":["公告催化","板块共振"],"capital_logic":"主力借题材吸筹","confidence":"medium","risk_note":"谨防冲高回落"}"#,
        )
        .expect("合法输出应解析");
        assert_eq!(out.main_reason, "中标新能源项目催化");
        assert_eq!(out.catalyst_chain.len(), 2);
        assert_eq!(out.confidence, "medium");
    }

    #[test]
    fn parse_accepts_markdown_fence() {
        let out = parse_deep_attribution_output(
            "```json\n{\"main_reason\":\"a\",\"catalyst_chain\":[\"b\"],\"capital_logic\":\"c\",\"confidence\":\"high\",\"risk_note\":\"d\"}\n```",
        )
        .expect("围栏包裹应剥离后解析");
        assert_eq!(out.main_reason, "a");
    }

    #[test]
    fn parse_rejects_missing_fields() {
        let err =
            parse_deep_attribution_output(r#"{"main_reason":"a"}"#).expect_err("缺字段必须报错");
        assert!(err.contains("catalyst_chain"), "错误应点名缺字段: {err}");
    }

    #[test]
    fn parse_rejects_invalid_confidence() {
        let err = parse_deep_attribution_output(
            r#"{"main_reason":"a","catalyst_chain":[],"capital_logic":"c","confidence":"extreme","risk_note":"d"}"#,
        )
        .expect_err("confidence 非法必须报错");
        assert!(err.contains("confidence"), "{err}");
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(parse_deep_attribution_output("这不是 JSON").is_err());
    }

    #[test]
    fn top_events_priority_then_cap() {
        let mk = |level: &str| AlertRecord {
            level: level.to_string(),
            ..sample_record()
        };
        let mut records = vec![mk("参考"), mk("紧急"), mk("重要"), mk("参考")];
        records[0].triggered_at = "t1".to_string();
        records[1].triggered_at = "t2".to_string();
        records[2].triggered_at = "t3".to_string();
        records[3].triggered_at = "t4".to_string();
        let picked = top_events_for_deep(records, 3);
        assert_eq!(picked.len(), 3);
        assert_eq!(picked[0].level, "紧急");
        assert_eq!(picked[1].level, "重要");
        assert_eq!(picked[2].level, "参考");
    }

    #[test]
    fn top_events_empty_input() {
        assert!(top_events_for_deep(vec![], 3).is_empty());
    }

    #[test]
    fn render_includes_core_fields() {
        let row = DeepAttributionRow {
            record: sample_record(),
            result: parse_deep_attribution_output(
                r#"{"main_reason":"中标催化","catalyst_chain":["公告"],"capital_logic":"吸筹","confidence":"medium","risk_note":"回落"}"#,
            )
            .unwrap(),
            analyzed_at: "2026-08-22T15:06:00Z".to_string(),
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            upstream_request_id: None,
            upstream_response_id: Some("resp-1".to_string()),
            elapsed_ms: 12_345,
        };
        let md = render_deep_attribution(&row);
        for needle in [
            "600396",
            "金山股份",
            "中标催化",
            "公告",
            "吸筹",
            "medium",
            "回落",
            "deepseek",
        ] {
            assert!(md.contains(needle), "渲染缺少: {needle}\n{md}");
        }
        let summary = render_deep_attribution_summary(&row);
        assert!(summary.contains("600396") && summary.contains("中标催化"));
    }

    /// mock provider: 合法回执 + 固定响应文本 (LlmError 不可 Clone, 用开关构造)。
    struct MockDeepProvider {
        raw_response: String,
        fail_with_api: bool,
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockDeepProvider {
        fn name(&self) -> &'static str {
            "mock-deep"
        }
        fn model(&self) -> &str {
            "mock-model"
        }
        async fn chat_json(&self, _system: &str, _user: &str) -> Result<Value, LlmError> {
            Ok(serde_json::from_str(&self.raw_response).unwrap_or(Value::Null))
        }
        async fn chat_json_with_receipt(
            &self,
            system: &str,
            user: &str,
        ) -> Result<ReceiptBearingJson, LlmError> {
            if self.fail_with_api {
                return Err(LlmError::Api {
                    status: 500,
                    body: "mock server boom".to_string(),
                });
            }
            Ok(ReceiptBearingJson::test_fixture(
                "mock-deep",
                "mock-model",
                None,
                "mock-response-id",
                system,
                user,
                &self.raw_response,
                Utc::now() - chrono::Duration::seconds(1),
                Utc::now(),
            ))
        }
    }

    #[tokio::test]
    async fn assess_with_mock_provider_succeeds() {
        let provider = Arc::new(MockDeepProvider {
            raw_response: r#"{"main_reason":"中标催化","catalyst_chain":["公告"],"capital_logic":"吸筹","confidence":"high","risk_note":"回落"}"#.to_string(),
            fail_with_api: false,
        });
        let analyzer = DeepAttributionAnalyzer::new(provider);
        let outcome = analyzer
            .assess(&sample_request())
            .await
            .expect("mock 应成功");
        assert_eq!(outcome.result.main_reason, "中标催化");
        assert_eq!(outcome.result.confidence, "high");
        assert_eq!(outcome.receipt.provider(), "mock-deep");
        assert_eq!(
            outcome.receipt.upstream_response_id(),
            Some("mock-response-id")
        );
    }

    #[tokio::test]
    async fn assess_mock_bad_schema_fails_loudly() {
        let provider = Arc::new(MockDeepProvider {
            raw_response: r#"{"main_reason":"a"}"#.to_string(),
            fail_with_api: false,
        });
        let analyzer = DeepAttributionAnalyzer::new(provider);
        let err = analyzer
            .assess(&sample_request())
            .await
            .expect_err("schema 缺失必须失败");
        assert!(
            matches!(err, DeepAttributionError::InvalidModelSchema(_)),
            "非法输出必须是 InvalidModelSchema: {err:?}"
        );
    }

    #[tokio::test]
    async fn assess_mock_call_error_fails_loudly() {
        let provider = Arc::new(MockDeepProvider {
            raw_response: String::new(),
            fail_with_api: true,
        });
        let analyzer = DeepAttributionAnalyzer::new(provider);
        let err = analyzer
            .assess(&sample_request())
            .await
            .expect_err("API 错误必须失败");
        assert!(
            matches!(err, DeepAttributionError::ModelUnavailable(_)),
            "调用失败必须是 ModelUnavailable: {err:?}"
        );
    }
}
