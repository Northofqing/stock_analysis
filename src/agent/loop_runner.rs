use crate::agent::context::ContextManager;
use crate::agent::toolbelt::Toolbelt;
use crate::agent::validation::ValidationEngine;
use crate::database::agent_logs::AgentLogDao;
use async_openai::types::{
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequestArgs,
};
use async_openai::{config::OpenAIConfig, Client};
use log::{info, warn};
use serde_json::Value;

pub struct AgentRunner {
    client: Client<OpenAIConfig>,
    /// 备选 (client, model) 三级 fallback：主模型失败时依次尝试
    fallbacks: Vec<(Client<OpenAIConfig>, String)>,
    toolbelt: Toolbelt,
    validation_engine: ValidationEngine,
    context: ContextManager,
    system_prompt: String,
    pub model_name: String,
    pub session_id: String,
}

impl AgentRunner {
    pub fn new(
        client: Client<OpenAIConfig>,
        toolbelt: Toolbelt,
        validation_engine: ValidationEngine,
        system_prompt: String,
        model_name: String,
    ) -> Self {
        Self {
            client,
            fallbacks: Vec::new(),
            toolbelt,
            validation_engine,
            context: ContextManager::new(),
            system_prompt,
            model_name,
            session_id: chrono::Utc::now().timestamp_millis().to_string(),
        }
    }

    /// 设置备选 (client, model) 列表，主模型失败时依次尝试
    pub fn with_fallbacks(mut self, fallbacks: Vec<(Client<OpenAIConfig>, String)>) -> Self {
        self.fallbacks = fallbacks;
        self
    }

    /// 统一的 chat 调用入口：主模型跳转到备选（处理 transport / API 错误）
    async fn chat_create(
        &self,
        request: async_openai::types::CreateChatCompletionRequest,
    ) -> Result<async_openai::types::CreateChatCompletionResponse, async_openai::error::OpenAIError>
    {
        match self.client.chat().create(request.clone()).await {
            Ok(resp) => Ok(resp),
            Err(primary_err) => {
                warn!(
                    "主模型 {} 调用失败: {} - 尝试备选模型",
                    self.model_name, primary_err
                );
                let mut last_err = primary_err;
                for (idx, (client, model)) in self.fallbacks.iter().enumerate() {
                    let mut req = request.clone();
                    req.model = model.clone();
                    match client.chat().create(req).await {
                        Ok(resp) => {
                            info!("备选模型生效（序号 {} -> {}）", idx + 1, model);
                            return Ok(resp);
                        }
                        Err(e) => {
                            warn!("备选模型 {} 调用失败: {}", model, e);
                            last_err = e;
                        }
                    }
                }
                Err(last_err)
            }
        }
    }

    /// 核心 ReAct 循环
    pub async fn run(&mut self, user_query: &str, max_iterations: usize) -> anyhow::Result<String> {
        let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();
        // 增加循环检测哈希表
        let mut tool_call_history: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        // 保留最新一次未通过 Critic 的草稿 + 最近一次 Critic 评分/反馈，用于兜底落盘
        let mut last_draft: Option<String> = None;
        let mut last_critic_score: Option<i64> = None;
        let mut last_critic_feedback: Option<String> = None;

        // 注入 System Prompt
        messages.push(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(&self.system_prompt)
                .build()?
                .into(),
        );

        // 注入 User Query
        messages.push(
            ChatCompletionRequestUserMessageArgs::default()
                .content(user_query)
                .build()?
                .into(),
        );

        for step in 0..max_iterations {
            info!("Agent Iteration {}/{}", step + 1, max_iterations);

            // 准备调用模型
            let request = CreateChatCompletionRequestArgs::default()
                .model(&self.model_name) // 根据配置选择
                .messages(messages.clone())
                .tools(self.toolbelt.as_openai_tools())
                .build()?;

            let response = self.chat_create(request).await?;
            let choice = &response.choices[0];
            let message = &choice.message;

            // 将 AI 的回复存回历史
            if let Some(ref content) = message.content {
                self.context.log_event(&format!("AI Thought: {}", content));
                AgentLogDao::insert_log(&self.session_id, step as i32, "thought", content)?;
            }

            // 为规避 Deepseek/Doubao 等模型强制校验推理内容（reasoning_content）的问题，
            // 且 async-openai 0.19 无法序列化该专有字段，
            // 我们不把带有 tool_calls 的 assistant 消息送回 API。
            // 只保留文本 Thought：
            if let Some(ref content) = message.content {
                messages.push(
                    ChatCompletionRequestAssistantMessageArgs::default()
                        .content(content.clone())
                        .build()?
                        .into(),
                );
            }

            // 判断是否有 Tool Call (Acting 阶段)
            if let Some(tool_calls) = &message.tool_calls {
                for tool_call in tool_calls {
                    let tool_name = &tool_call.function.name;
                    let arguments: serde_json::Value =
                        serde_json::from_str(&tool_call.function.arguments)
                            .unwrap_or(serde_json::Value::Null);

                    self.context
                        .log_event(&format!("Tool Call: {} args: {}", tool_name, arguments));
                    AgentLogDao::insert_log(
                        &self.session_id,
                        step as i32,
                        "tool_call",
                        &format!("{} - {}", tool_name, arguments),
                    )?;

                    info!(
                        "[执行智能体 Action Agent] 决定调用工具：{}，参数：{}",
                        tool_name, arguments
                    );

                    // ================= 安全层：死循环检测 (Loop Detection) =================
                    // v17.4 (P2 fix): 用 sorted JSON keys 生成签名, 避免字段顺序不同导致不同签名
                    let canonical_args = canonicalize_json_value(&arguments);
                    let call_sig = format!("{}-{}", tool_name, canonical_args);
                    let count = tool_call_history.entry(call_sig.clone()).or_insert(0);
                    *count += 1;

                    let tool_result;
                    if *count >= 3 {
                        let err_msg =
                            format!("Loop Detection Triggered: Repeated call {} times", count);
                        warn!("{}", err_msg);
                        self.context.remove_fact(tool_name);
                        tool_result = format!("【安全拦截】系统检测到你连续 {} 次使用完全相同的参数调用了该工具！为了防止死循环，本次调用被阻断。请立即停止重复毫无意义的操作！如无法获取数据，请改用其他工具，或直接根据现有上下文汇总结论。", count);
                        self.context.log_event(&err_msg);
                        AgentLogDao::insert_log(
                            &self.session_id,
                            step as i32,
                            "loop_detection",
                            &err_msg,
                        )?;
                    } else {
                        // 执行 Tool
                        match self.toolbelt.execute(tool_name, arguments.clone()).await {
                            Ok(result) => {
                                tool_result = result;
                                // 只有成功结果可以成为事实；纯文本以 String 保存。
                                if let Ok(json_val) =
                                    serde_json::from_str::<serde_json::Value>(&tool_result)
                                {
                                    self.context.insert_fact(tool_name, json_val);
                                } else {
                                    self.context.insert_fact(
                                        tool_name,
                                        serde_json::Value::String(tool_result.clone()),
                                    );
                                }
                            }
                            Err(error) => {
                                self.context.remove_fact(tool_name);
                                tool_result = format!(
                                    "【数据不可用】工具 `{tool_name}` 执行失败: {error:#}。不得把缺失字段补成事实。"
                                );
                                self.context
                                    .log_event(&format!("Tool Error: {tool_name}: {error:#}"));
                                AgentLogDao::insert_log(
                                    &self.session_id,
                                    step as i32,
                                    "tool_error",
                                    &format!("{tool_name}: {error:#}"),
                                )?;
                            }
                        }
                    }

                    self.context
                        .log_event(&format!("Tool Result: {}", tool_result));
                    AgentLogDao::insert_log(
                        &self.session_id,
                        step as i32,
                        "tool_result",
                        &tool_result,
                    )?;

                    // 将工具调用和结果以 User 视角压入历史（替代原有的 ToolMessage）
                    let observation = format!(
                        "【系统通知：工具执行结果】
你刚才决定调用工具 `{}` (参数: `{}`)。
该工具返回的数据如下：
{}

请基于上述数据继续分析，如果还需要数据请继续调用工具；如果数据已充分，请直接输出最终结论。",
                        tool_name,
                        arguments.clone(),
                        tool_result
                    );

                    messages.push(
                        ChatCompletionRequestUserMessageArgs::default()
                            .content(observation)
                            .build()?
                            .into(),
                    );
                }

                // Observing 阶段：运行自我校验引擎 (Validation Agent)
                info!("[验证智能体 Validation Agent] 正在对新抓取的数据执行自洽性检查...");
                let validation_errors = self.validation_engine.run_all(&self.context);
                if !validation_errors.is_empty() {
                    let error_msgs: Vec<String> =
                        validation_errors.iter().map(|e| e.to_string()).collect();
                    let feedback = format!(
                        "【系统级自检失败 Validation Agent 拦截】系统检测到你的数据或推理存在以下不一致问题，请立即纠正并重新思考/搜索：\n{}", 
                        error_msgs.join("\n")
                    );

                    warn!("Validation failed: {}", feedback);
                    self.context
                        .log_event("Validation Failed, triggering self-correction");
                    AgentLogDao::insert_log(
                        &self.session_id,
                        step as i32,
                        "validation_error",
                        &feedback,
                    )?;

                    messages.push(
                        ChatCompletionRequestUserMessageArgs::default()
                            .content(feedback)
                            .build()?
                            .into(),
                    );
                }

                // 继续下一次循环，让 LLM 根据工具返回或校验失败的结果再次决定
                continue;
            }

            // 如果没有 Tool Call，说明 AI 认为任务已完成，给出了最终答案 (Reporting 阶段)
            if let Some(final_content) = &message.content {
                info!("[审查智能体 Critic Agent] 正在对初稿进行盲审和逻辑对抗校验...");

                let fact_sheet = serde_json::to_string(&self.context.facts)?;
                let available_tools = self
                    .toolbelt
                    .as_openai_tools()
                    .into_iter()
                    .map(|t| t.function.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");

                let critic_prompt = format!(
                    "你现在是顶级的金融投资总监（审查官）。下面是研究员写的一份《研报初稿》以及他在调研过程中收集到的《原始数据事实清单 (Fact Sheet)》。\n\n\
                    【你的任务】\n\
                    1. 数据溯源：校验初稿中的关键数字和事实是否在 Fact Sheet 中有明确支撑，严打凭空捏造（幻觉）。\n\
                    2. 逻辑漏洞：寻找初稿中的逻辑硬伤或片面分析（例如光说营收好不说利润，或缺乏风险提示）。\n\
                    3. 策略输出：强制要求研究员在财报的最后，**给出明确的操作策略（买入、持有、卖出建议）并设定具体的应对价格阈值！**\n\
                    4. 工具边界：研究员目前可用的工具只有：[{}]。你提出的补充查证要求必须是不超纲的。\n\n\
                    【Fact Sheet】\n{}\n\n\
                    【研报初稿】\n{}\n\n\
                    请你给出客观的评分（0-100分）。如果不满85分，请给出极其明确的修改指令。\n\
                    务必只返回合法的 JSON 格式，不要有任何 Markdown 修饰符，如下所示：\n\
                    {{\n  \"score\": 85,\n  \"feedback\": \"你的反馈意见...\"\n}}",
                    available_tools, fact_sheet, final_content
                );

                let critic_req = CreateChatCompletionRequestArgs::default()
                    .model(&self.model_name)
                    .messages(vec![ChatCompletionRequestUserMessageArgs::default()
                        .content(critic_prompt)
                        .build()?
                        .into()])
                    .build()?;

                let critic_res = self.chat_create(critic_req).await?;
                let critic_text = critic_res.choices[0]
                    .message
                    .content
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Critic returned no content"))?;

                let cleaned_json = critic_text
                    .trim_start_matches("```json")
                    .trim_end_matches("```")
                    .trim();
                let eval: serde_json::Value =
                    serde_json::from_str(cleaned_json).unwrap_or(serde_json::Value::Null);
                // 修复 P2.3 silent fail: 解析失败应得 0 分, 不是 100 分
                // 之前: unwrap_or(100) → JSON 解析失败反而给满分, 完全反语义
                let score = eval
                    .get("score")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| {
                        log::warn!(
                            "[P2.3] critic JSON 解析失败, 默认 score=0, raw={:?}",
                            cleaned_json
                        );
                        0
                    });
                let feedback = eval
                    .get("feedback")
                    .and_then(|v| v.as_str())
                    .unwrap_or("没有有效反馈")
                    .to_string();

                match score {
                    s if s >= 85 => {
                        info!("[汇总智能体 Answer Agent] 报告已通过审查，评分 {}/100，输出最终定稿...", s);
                        self.context.log_event(&format!(
                            "Agent finished tasks with Critic Approval (Score: {}).",
                            s
                        ));
                        AgentLogDao::insert_log(
                            &self.session_id,
                            step as i32,
                            "final_answer",
                            final_content,
                        )?;
                        return Ok(final_content.to_string());
                    }
                    s => {
                        warn!(
                            "[审查智能体 Critic Agent] 报告被打回！评分: {}/100，意见：{}",
                            s, feedback
                        );
                        // 记录最近一次草稿和 Critic 反馈，便于迭代用尽时兜底返回
                        last_draft = Some(final_content.to_string());
                        last_critic_score = Some(s);
                        last_critic_feedback = Some(feedback.clone());

                        let revision_prompt = format!(
                            "【审查官打回修改指令】你的《初稿》未通过审核（仅得 {} 分）。审查官意见如下：\n{}\n\n请不要仅仅修改文本文字！请务必使用相应的工具（如 {}）去获取缺失的数据事实，修正你的幻觉依据，然后再生成下一版报告！",
                            s, feedback, available_tools
                        );

                        self.context.log_event(&format!(
                            "Critic Rejected: Score {}, Feedback: {}",
                            s, feedback
                        ));
                        AgentLogDao::insert_log(
                            &self.session_id,
                            step as i32,
                            "critic_feedback",
                            &feedback,
                        )?;

                        // 动态清理 Context (Memory Compaction), 防止 Token 爆炸
                        if messages.len() > 40 {
                            info!("上下文消息过长，正在触发记忆浓缩机制以防止 Token 超载...");
                            let sys_and_user = messages[0..2].to_vec();
                            let mut compacted = sys_and_user;
                            // 保留最近四轮的关键操作上下文
                            let drop_start = messages.len().saturating_sub(8);
                            compacted.extend_from_slice(&messages[drop_start..]);
                            messages = compacted;
                        }

                        messages.push(
                            ChatCompletionRequestUserMessageArgs::default()
                                .content(revision_prompt)
                                .build()?
                                .into(),
                        );
                        continue;
                    }
                }
            }
        }

        // 迭代用尽：若有最近一次草稿，则附加 Critic 反馈作为风险提示返回，避免上层只能拿到 Err 导致空文件
        if let Some(draft) = last_draft {
            warn!(
                "Agent 已达最大迭代次数 {}，返回最近一次未通过 Critic 的草稿作为兜底报告",
                max_iterations
            );
            let score = last_critic_score.unwrap_or(0);
            let feedback = last_critic_feedback.unwrap_or_else(|| "无 Critic 反馈".to_string());
            let warning_block = format!(
                "\n\n---\n\n> ⚠️ **本报告未通过审查官最终复核（最后一次评分 {}/100），仅作为参考草稿。**\n>\n> **审查官保留意见**：{}\n",
                score, feedback
            );
            AgentLogDao::insert_log(
                &self.session_id,
                max_iterations as i32,
                "final_answer_fallback",
                &draft,
            )?;
            return Ok(format!("{}{}", draft, warning_block));
        }

        Err(anyhow::anyhow!(
            "Agent failed to complete task within max iterations"
        ))
    }

    pub fn get_context(&self) -> &ContextManager {
        &self.context
    }
}

/// v17.4 (P2 fix): 递归规范化 JSON Value, 按 key 排序 (BTreeMap), 保证字段顺序无关
fn canonicalize_json_value(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut sorted: std::collections::BTreeMap<String, &Value> =
                std::collections::BTreeMap::new();
            for (k, v) in map {
                sorted.insert(k.clone(), v);
            }
            let parts: Vec<String> = sorted
                .iter()
                .map(|(k, v)| format!("\"{}\":{}", k, canonicalize_json_value(v)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(canonicalize_json_value).collect();
            format!("[{}]", parts.join(","))
        }
        Value::String(s) => {
            // 转义双引号 + 反斜杠, 避免破坏 JSON 字符串边界
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{}\"", escaped)
        }
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
    }
}

#[cfg(test)]
mod gate_d_tests {
    use super::*;
    use crate::agent::tool::Tool;
    use async_trait::async_trait;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn local_client() -> Client<OpenAIConfig> {
        Client::with_config(
            OpenAIConfig::new()
                .with_api_key("TEST_CODE_local_only")
                .with_api_base("http://127.0.0.1:9/v1"),
        )
        .with_http_client(reqwest_011::Client::builder().no_proxy().build().unwrap())
    }

    fn client_for(base: &str) -> Client<OpenAIConfig> {
        Client::with_config(
            OpenAIConfig::new()
                .with_api_key("TEST_CODE_local_only")
                .with_api_base(format!("{base}/v1")),
        )
        .with_http_client(reqwest_011::Client::builder().no_proxy().build().unwrap())
    }

    fn response(content: Option<&str>, tool_call: bool) -> String {
        let mut message = serde_json::json!({
            "role": "assistant",
            "content": content,
        });
        if tool_call {
            message["tool_calls"] = serde_json::json!([{
                "id": "TEST_CODE_call_1",
                "type": "function",
                "function": {
                    "name": "TEST_CODE_fixture_tool",
                    "arguments": "{\"value\":1}"
                }
            }]);
        }
        serde_json::json!({
            "id": "TEST_CODE_chat_completion",
            "object": "chat.completion",
            "created": 1,
            "model": "TEST_CODE_model",
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": if tool_call { "tool_calls" } else { "stop" }
            }],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 1,
                "total_tokens": 2
            }
        })
        .to_string()
    }

    fn spawn_chat_fixture(responses: Vec<String>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local chat fixture");
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for body in responses {
                let (mut stream, _) = listener.accept().expect("accept chat request");
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buf = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut buf).expect("read chat request");
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..read]);
                    if let Some(header_end) = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|position| position + 4)
                    {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = headers
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length: ")
                                    .or_else(|| line.strip_prefix("Content-Length: "))
                            })
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if request.len() >= header_end + content_length {
                            break;
                        }
                    }
                }
                assert!(String::from_utf8_lossy(&request).starts_with("POST /v1/chat/completions"));
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(reply.as_bytes()).expect("reply to chat");
                stream.flush().expect("flush chat response");
                stream
                    .shutdown(std::net::Shutdown::Write)
                    .expect("close chat response body");
            }
        });
        (format!("http://{address}"), handle)
    }

    struct FixtureTool;

    #[async_trait]
    impl Tool for FixtureTool {
        fn name(&self) -> &str {
            "TEST_CODE_fixture_tool"
        }

        fn description(&self) -> &str {
            "TEST_CODE local deterministic tool"
        }

        fn parameters(&self) -> Value {
            serde_json::json!({"type":"object"})
        }

        async fn call(&self, input: Value) -> anyhow::Result<String> {
            Ok(serde_json::json!({"observed": input["value"]}).to_string())
        }
    }

    #[tokio::test]
    async fn zero_iteration_boundary_never_calls_the_model() {
        let mut runner = AgentRunner::new(
            local_client(),
            Toolbelt::new(),
            ValidationEngine::new_with_defaults(),
            "TEST_CODE system".to_string(),
            "TEST_CODE model".to_string(),
        )
        .with_fallbacks(vec![(local_client(), "TEST_CODE fallback".to_string())]);

        assert!(runner.session_id.parse::<i64>().is_ok());
        assert!(runner.get_context().facts.is_empty());
        let err = runner
            .run("TEST_CODE query", 0)
            .await
            .expect_err("zero iterations must fail explicitly");
        assert!(err.to_string().contains("max iterations"));
    }

    #[test]
    fn canonical_json_is_recursive_stable_and_escaped() {
        let left = serde_json::json!({
            "z": [true, null, {"quote": "a\\\"b"}],
            "a": 1
        });
        let right = serde_json::json!({
            "a": 1,
            "z": [true, null, {"quote": "a\\\"b"}]
        });
        let canonical = canonicalize_json_value(&left);
        assert_eq!(canonical, canonicalize_json_value(&right));
        assert_eq!(canonical, r#"{"a":1,"z":[true,null,{"quote":"a\\\"b"}]}"#);
    }

    #[tokio::test]
    async fn local_protocol_covers_tool_fallback_critic_and_loop_detection() {
        crate::database::DatabaseManager::init(None).expect("agent audit database");

        let (approval_base, approval_fixture) = spawn_chat_fixture(vec![
            response(Some("TEST_CODE approved draft"), false),
            response(Some(r#"{"score":90,"feedback":"TEST_CODE ok"}"#), false),
        ]);
        let mut approved = AgentRunner::new(
            client_for(&approval_base),
            Toolbelt::new(),
            ValidationEngine::new_with_defaults(),
            "TEST_CODE system".to_string(),
            "TEST_CODE model".to_string(),
        );
        assert_eq!(
            approved.run("TEST_CODE query", 1).await.unwrap(),
            "TEST_CODE approved draft"
        );
        approval_fixture.join().unwrap();

        let (tool_base, tool_fixture) = spawn_chat_fixture(vec![
            response(None, true),
            response(Some("TEST_CODE tool-backed draft"), false),
            response(Some(r#"{"score":92,"feedback":"TEST_CODE ok"}"#), false),
        ]);
        let mut belt = Toolbelt::new();
        belt.register(FixtureTool);
        let mut tool_runner = AgentRunner::new(
            local_client(),
            belt,
            ValidationEngine::new_with_defaults(),
            "TEST_CODE system".to_string(),
            "TEST_CODE model".to_string(),
        )
        .with_fallbacks(vec![(
            client_for(&tool_base),
            "TEST_CODE fallback".to_string(),
        )]);
        assert_eq!(
            tool_runner.run("TEST_CODE tool query", 2).await.unwrap(),
            "TEST_CODE tool-backed draft"
        );
        assert_eq!(
            tool_runner.get_context().get_fact("TEST_CODE_fixture_tool"),
            Some(&serde_json::json!({"observed": 1}))
        );
        tool_fixture.join().unwrap();

        let (loop_base, loop_fixture) = spawn_chat_fixture(vec![
            response(None, true),
            response(None, true),
            response(None, true),
        ]);
        let mut loop_belt = Toolbelt::new();
        loop_belt.register(FixtureTool);
        let mut loop_runner = AgentRunner::new(
            client_for(&loop_base),
            loop_belt,
            ValidationEngine::new_with_defaults(),
            "TEST_CODE system".to_string(),
            "TEST_CODE model".to_string(),
        );
        assert!(loop_runner.run("TEST_CODE loop", 3).await.is_err());
        assert!(loop_runner
            .get_context()
            .get_fact("TEST_CODE_fixture_tool")
            .is_none());
        loop_fixture.join().unwrap();
    }
}
