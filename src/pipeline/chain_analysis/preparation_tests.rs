use super::preparation::{
    prepare_chain_analysis_with_io, ChainPreparationIo, PositionInput, SourceObservation,
    SourceStatus,
};
use crate::data_gateway::{BatchEvidence, GatewayBatch};
use crate::market_data::TopStock;
use std::collections::HashMap;

pub(super) struct ProtocolIo {
    pub(super) analyzer: Option<crate::analyzer::GeminiAnalyzer>,
    pub(super) scripted:
        Option<std::cell::RefCell<std::collections::VecDeque<Result<String, String>>>>,
    pub(super) concepts: HashMap<String, Vec<String>>,
    pub(super) search_enabled: bool,
    pub(super) queries: Vec<String>,
}

#[async_trait::async_trait(?Send)]
impl ChainPreparationIo for ProtocolIo {
    async fn concepts(&mut self, codes: &[String]) -> anyhow::Result<HashMap<String, Vec<String>>> {
        codes
            .iter()
            .map(|code| {
                Ok((
                    code.clone(),
                    self.concepts
                        .get(code)
                        .expect("complete controlled concept input")
                        .clone(),
                ))
            })
            .collect()
    }
    fn min_cluster_size(&mut self) -> usize {
        3
    }
    async fn persist_clusters(
        &mut self,
        date: chrono::NaiveDate,
        rows: &[(String, Vec<String>, i32)],
    ) -> anyhow::Result<HashMap<String, i64>> {
        assert_eq!(date.to_string(), "2026-07-21");
        Ok(rows
            .iter()
            .map(|(concept, _, _)| (concept.clone(), 2))
            .collect())
    }
    async fn board_codes(
        &mut self,
    ) -> anyhow::Result<(HashMap<String, String>, Vec<BatchEvidence>)> {
        anyhow::bail!("TEST_CODE_协议目录缺失")
    }
    async fn positions(&mut self) -> anyhow::Result<Vec<PositionInput>> {
        Ok(vec![PositionInput::new(
            "TEST_CODE_协议持仓".into(),
            "TEST_CODE_持仓敏感正文".into(),
            Some(1.5),
        )])
    }
    async fn lhb(&mut self) -> anyhow::Result<(HashMap<String, f64>, SourceObservation)> {
        Ok((
            HashMap::from([("TEST_CODE_DEEP_0".into(), 321.0)]),
            SourceObservation::unknown(),
        ))
    }
    fn model_available(&mut self) -> bool {
        self.scripted.is_some()
            || self
                .analyzer
                .as_ref()
                .is_some_and(|analyzer| analyzer.is_available())
    }
    async fn model(
        &self,
        prompt: &str,
        system: &str,
        mode: crate::analyzer::AgentMode,
    ) -> anyhow::Result<String> {
        if let Some(scripted) = &self.scripted {
            return scripted
                .borrow_mut()
                .pop_front()
                .expect("registered synthetic model response")
                .map_err(anyhow::Error::msg);
        }
        self.analyzer
            .as_ref()
            .expect("explicit model adapter")
            .call_api_mode(prompt, system, mode)
            .await
    }
    fn search_available(&mut self) -> bool {
        self.search_enabled
    }
    async fn search_topic(
        &mut self,
        query: &str,
        _limit: usize,
    ) -> anyhow::Result<Vec<crate::search_service::SearchResult>> {
        self.queries.push(query.to_string());
        Ok(vec![crate::search_service::SearchResult {
            title: if query.contains("最新 突发 催化") {
                "TEST_CODE_盘后新闻".into()
            } else {
                "TEST_CODE_定向新闻".into()
            },
            snippet: " TEST_CODE_搜索原片段\n".into(),
            url: "https://example.invalid/chain-news".into(),
            source: "TEST_CODE_合成搜索来源".into(),
            published_date: None,
            news_type: crate::search_service::NewsType::Industry,
            sentiment: crate::search_service::Sentiment::Neutral,
            importance: 5,
            relevance: 1.0,
            keywords: Vec::new(),
            evidence: Default::default(),
        }])
    }
    fn local_now(&mut self) -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339("2026-07-22T00:30:00+08:00").unwrap()
    }
}

pub(super) fn protocol_inputs() -> (Vec<TopStock>, HashMap<String, Vec<String>>) {
    let mut stocks = Vec::new();
    let mut concepts = HashMap::new();
    for (prefix, count, concept) in [
        ("DEEP", 8, "TEST_CODE_深度主线"),
        ("SIMPLE", 4, "TEST_CODE_简化主线"),
    ] {
        for index in 0..count {
            let code = format!("TEST_CODE_{prefix}_{index}");
            stocks.push(TopStock {
                code: code.clone(),
                name: format!("TEST_CODE_名称{prefix}{index}"),
                change_pct: 10.0 - index as f64 / 10.0,
                price: 10.0,
                ..TopStock::default()
            });
            concepts.insert(code, vec![concept.into()]);
        }
    }
    concepts.insert(
        "TEST_CODE_协议持仓".into(),
        vec!["TEST_CODE_深度主线".into()],
    );
    (stocks, concepts)
}

#[tokio::test]
async fn model_preparation_records_real_prompts_search_and_original_responses() {
    let texts = [
        "  钨出口限制价格上涨\n化工供给变化  \n",
        "【结论】阶段=启动｜参与=可关注｜候选=无\n【评分】产业逻辑=80/100｜情绪位置=70/100｜资金共识=60/100｜筹码健康=50/100｜证伪概率=20/100\n TEST_CODE_深度原响应 Ω\t \n",
        "【简评】阶段=发酵｜参与=谨慎｜候选=无\n TEST_CODE_简化原响应\t \n",
        "### 核心矛盾与主线优先级\n TEST_CODE_总览原响应 Ω\t \n",
    ];
    let server = crate::data_provider::TestHttpServer::new(
        texts
            .iter()
            .map(|text| {
                crate::data_provider::TestHttpResponse::json(
                    &serde_json::json!({"choices":[{"message":{"content":text}}]}).to_string(),
                )
            })
            .collect(),
    );
    let analyzer =
        crate::analyzer::GeminiAnalyzer::with_loopback_client(crate::analyzer::GeminiConfig {
            doubao_api_key: Some("TEST_CODE_LOCAL_PROTOCOL_KEY".into()),
            doubao_base_url: Some(server.base_url().to_string()),
            doubao_model: "TEST_CODE_MODEL".into(),
            max_retries: 1,
            retry_delay: 0.0,
            request_delay: 0.0,
            agent_pipeline: false,
            ..crate::analyzer::GeminiConfig::default()
        });
    let (stocks, concepts) = protocol_inputs();
    let mut io = ProtocolIo {
        analyzer: Some(analyzer),
        scripted: None,
        concepts,
        search_enabled: true,
        queries: Vec::new(),
    };
    let prepared = prepare_chain_analysis_with_io(
        chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks,
        Some(" TEST_CODE_优先宏观输入\n".into()),
        &mut io,
    )
    .await
    .expect("real model protocol preparation");

    let calls = prepared.model_calls();
    assert_eq!(calls.len(), 4);
    for (call, expected) in calls.iter().zip(texts) {
        assert_eq!(call.response(), Some(expected));
        assert_eq!(call.provider_identity(), None);
        assert_eq!(call.model_identity(), None);
    }
    assert!(calls[0].prompt().unwrap().contains("TEST_CODE_名称DEEP0"));
    assert_eq!(
        calls[0].system(),
        Some("你是A股题材挖掘专家，只输出新闻搜索词，每行一条。")
    );
    assert_eq!(calls[0].mode(), Some(crate::analyzer::AgentMode::Quick));
    assert_eq!(calls[1].mode(), Some(crate::analyzer::AgentMode::Deep));
    for marker in [
        "2026-07-21",
        "TEST_CODE_DEEP_0",
        "321",
        "TEST_CODE_定向新闻",
        "TEST_CODE_优先宏观输入",
        "TEST_CODE_协议目录缺失",
    ] {
        assert!(
            calls[1].prompt().unwrap().contains(marker),
            "missing independent deep-prompt marker {marker}"
        );
    }
    assert!(calls[2].prompt().unwrap().contains("TEST_CODE_SIMPLE_0"));
    for marker in [
        "TEST_CODE_持仓敏感正文",
        "TEST_CODE_深度原响应",
        "TEST_CODE_简化原响应",
        "TEST_CODE_盘后新闻",
    ] {
        assert!(calls[3].prompt().unwrap().contains(marker));
    }
    assert_eq!(io.queries.len(), 5);
    assert!(io.queries.iter().any(|query| query == "钨出口限制价格上涨"));
    assert!(prepared.cluster_news()["TEST_CODE_深度主线"].contains("TEST_CODE_定向新闻"));
    assert!(prepared.after_market_context().contains("07月22日 盘中"));
    assert_eq!(prepared.business_date().to_string(), "2026-07-21");
    assert!(prepared.report().contains("深度分析 1 条 + 简化分析 1 条"));
    assert!(prepared.report().contains("TEST_CODE_深度原响应 Ω\n\n"));
    assert!(prepared
        .report()
        .contains("【简评】阶段=发酵｜参与=谨慎｜候选=无\n TEST_CODE_简化原响应\n\n"));
    assert!(prepared
        .report()
        .contains("### 核心矛盾与主线优先级\n TEST_CODE_总览原响应 Ω\n\n"));
    assert_eq!(prepared.search_observations().len(), 5);
    assert!(prepared
        .search_observations()
        .iter()
        .all(|search| search.results()[0].published_date.is_none()));
    let paths = server.finish();
    assert_eq!(paths.len(), 4);
    assert!(paths.iter().all(|path| path == "/chat/completions"));
}

#[tokio::test]
async fn artifact_round_trip_preserves_owned_bytes_and_rejects_unsupported_or_lossy_input() {
    use super::preparation::PreparedChainAnalysis;
    const RESPONSES: [&str; 4] = [
        " 钨出口价格变化\n",
        " TEST_CODE_深度 Ω\t \n",
        " TEST_CODE_简化\r\n",
        " TEST_CODE_总览 Ω\t \n",
    ];
    fn scripted(concepts: HashMap<String, Vec<String>>) -> ProtocolIo {
        ProtocolIo {
            analyzer: None,
            scripted: Some(std::cell::RefCell::new(
                RESPONSES.iter().map(|text| Ok((*text).into())).collect(),
            )),
            concepts,
            search_enabled: true,
            queries: Vec::new(),
        }
    }
    let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
    let (mut stocks, concepts) = protocol_inputs();
    stocks[0].volume_ratio = Some(1.25);
    let mut reverse_entries: Vec<_> = concepts
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    reverse_entries.sort_by(|a, b| b.0.cmp(&a.0));
    let mut io = scripted(concepts);
    let prepared = prepare_chain_analysis_with_io(
        date,
        stocks.clone(),
        Some(" \r\nTEST_CODE_宏观 Ω\t \n".into()),
        &mut io,
    )
    .await
    .unwrap();
    let bytes = prepared
        .to_artifact_bytes()
        .expect("ordinary finite prepared values must encode");
    let effects_before_decode = io.queries.clone();
    let restored = PreparedChainAnalysis::from_artifact_bytes(&bytes).unwrap();
    assert_eq!(restored.report().as_bytes(), prepared.report().as_bytes());
    assert_eq!(restored.macro_input(), Some(" \r\nTEST_CODE_宏观 Ω\t \n"));
    assert_eq!(
        restored.limit_ups()[0].volume_ratio.unwrap().to_bits(),
        1.25_f64.to_bits()
    );
    for (call, expected) in restored.model_calls().iter().zip(RESPONSES) {
        assert_eq!(call.response(), Some(expected));
    }
    assert_eq!(restored.model_calls().len(), 4);
    assert_eq!(restored.min_cluster_size(), Some(3));
    for source in [
        restored.concept_source(),
        restored.positions_source(),
        restored.position_concept_source(),
    ] {
        assert_eq!(source.status(), &SourceStatus::Unknown);
        assert_eq!(source.source_at(), None);
        assert_eq!(source.batch_id(), None);
    }
    assert_eq!(
        restored.search_observations()[0].results()[0].snippet,
        " TEST_CODE_搜索原片段\n"
    );
    assert_eq!(io.queries, effects_before_decode);
    assert!(io.scripted.as_ref().unwrap().borrow().is_empty());
    assert!(!format!(
        "{restored:?} {:?} {:?}",
        restored.model_calls(),
        restored.search_observations()
    )
    .contains("TEST_CODE"));
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert!(value["data"]["model_calls"][0]
        .get("provider_identity")
        .expect("explicit missing identity")
        .is_null());
    assert!(value["data"]["model_calls"][0]
        .get("model_identity")
        .expect("explicit missing identity")
        .is_null());
    let second = prepare_chain_analysis_with_io(
        date,
        stocks.clone(),
        Some(" \r\nTEST_CODE_宏观 Ω\t \n".into()),
        &mut scripted(reverse_entries.into_iter().collect()),
    )
    .await
    .unwrap();
    assert_eq!(
        bytes,
        second.to_artifact_bytes().unwrap(),
        "owned maps encode deterministically without reordering report selections"
    );

    let encoded = String::from_utf8(bytes.clone()).unwrap();
    for (needle, replacement) in [
        ("\"schema_version\":1", "\"schema_version\":99"),
        (
            "\"schema_version\":1",
            "\"schema_version\":1,\"schema_version\":1",
        ),
        ("\"source_at\":null,", ""),
        ("\"volume_ratio\":1.25,", ""),
        ("\"price\":10.0", "\"price\":10.0,\"price\":10.0"),
        ("\"TEST_CODE_DEEP_0\":[\"TEST_CODE_深度主线\"]", "\"TEST_CODE_DEEP_0\":[\"TEST_CODE_深度主线\"],\"TEST_CODE_DEEP_0\":[\"TEST_CODE_深度主线\"]"),
        (
            "\"schema_version\":1",
            "\"schema_version\":1,\"TEST_CODE_unknown\":null",
        ),
    ] {
        assert!(
            encoded.contains(needle),
            "fixture must actually contain the edited field"
        );
        let invalid = encoded.replacen(needle, replacement, 1);
        let error = PreparedChainAnalysis::from_artifact_bytes(invalid.as_bytes())
            .expect_err("bad version, duplicate, omitted or unknown field must reject");
        assert!(!format!("{error:?}").contains("TEST_CODE"));
    }
    for invalid in [
        b"{".as_slice(),
        &bytes[..bytes.len() - 1],
        b"null".as_slice(),
    ] {
        assert!(PreparedChainAnalysis::from_artifact_bytes(invalid).is_err());
    }
    stocks[0].volume_ratio = Some(f64::NAN);
    let nonfinite = prepare_chain_analysis_with_io(
        date,
        stocks.clone(),
        Some("TEST_CODE_非有限数".into()),
        &mut scripted(protocol_inputs().1),
    )
    .await
    .unwrap();
    let error = nonfinite
        .to_artifact_bytes()
        .expect_err("Option NaN must not silently become null");
    assert!(!format!("{error:?}").contains("TEST_CODE"));
    stocks[0].volume_ratio = Some(1.25);
    stocks[0].price = 1.2345678901234567;
    let precise = prepare_chain_analysis_with_io(
        date,
        stocks,
        Some("TEST_CODE_有限精度".into()),
        &mut scripted(protocol_inputs().1),
    )
    .await
    .unwrap();
    match precise.to_artifact_bytes() {
        Ok(bytes) => assert_eq!(
            PreparedChainAnalysis::from_artifact_bytes(&bytes)
                .unwrap()
                .limit_ups()[0]
                .price
                .to_bits(),
            1.2345678901234567_f64.to_bits()
        ),
        Err(error) => assert!(!format!("{error:?}").contains("TEST_CODE")),
    }
}

struct RejectExternalIo;

// Only external effects vary here; clustering, selection, prompts and rendering stay real.
struct CoverageIo {
    protocol: ProtocolIo,
    macro_result: Result<String, String>,
    macro_calls: usize,
    search_failure: bool,
    candidate_boards: Vec<String>,
}

#[async_trait::async_trait(?Send)]
impl ChainPreparationIo for CoverageIo {
    async fn concepts(&mut self, codes: &[String]) -> anyhow::Result<HashMap<String, Vec<String>>> {
        self.protocol.concepts(codes).await
    }
    fn min_cluster_size(&mut self) -> usize {
        3
    }
    async fn persist_clusters(
        &mut self,
        date: chrono::NaiveDate,
        rows: &[(String, Vec<String>, i32)],
    ) -> anyhow::Result<HashMap<String, i64>> {
        self.protocol.persist_clusters(date, rows).await
    }
    async fn board_codes(
        &mut self,
    ) -> anyhow::Result<(HashMap<String, String>, Vec<BatchEvidence>)> {
        Ok((
            self.protocol
                .concepts
                .values()
                .flatten()
                .map(|tag| (tag.clone(), format!("BOARD_{tag}")))
                .collect(),
            vec![],
        ))
    }
    async fn candidates(
        &mut self,
        board: &str,
        _excluded: &std::collections::HashSet<String>,
    ) -> anyhow::Result<GatewayBatch<TopStock>> {
        self.candidate_boards.push(board.into());
        Ok(GatewayBatch::VerifiedEmpty(candidate_evidence(
            "TEST_CODE_限额来源",
            board,
            None,
        )))
    }
    async fn positions(&mut self) -> anyhow::Result<Vec<PositionInput>> {
        Ok(vec![])
    }
    async fn lhb(&mut self) -> anyhow::Result<(HashMap<String, f64>, SourceObservation)> {
        // The old provider failure policy returns empty facts plus unavailable evidence.
        Ok((
            HashMap::new(),
            SourceObservation::unavailable("TEST_CODE_龙虎榜上游失败".into()),
        ))
    }
    async fn macro_search(&mut self) -> anyhow::Result<String> {
        self.macro_calls += 1;
        self.macro_result.clone().map_err(anyhow::Error::msg)
    }
    fn model_available(&mut self) -> bool {
        self.protocol.model_available()
    }
    async fn model(
        &self,
        prompt: &str,
        system: &str,
        mode: crate::analyzer::AgentMode,
    ) -> anyhow::Result<String> {
        self.protocol.model(prompt, system, mode).await
    }
    fn search_available(&mut self) -> bool {
        self.protocol.search_enabled
    }
    async fn search_topic(
        &mut self,
        query: &str,
        _limit: usize,
    ) -> anyhow::Result<Vec<crate::search_service::SearchResult>> {
        self.protocol.queries.push(query.into());
        if self.search_failure {
            anyhow::bail!("TEST_CODE_搜索上游失败");
        }
        Ok(vec![])
    }
    fn local_now(&mut self) -> chrono::DateTime<chrono::FixedOffset> {
        self.protocol.local_now()
    }
}

fn coverage_io(
    concepts: HashMap<String, Vec<String>>,
    responses: Option<Vec<Result<String, String>>>,
) -> CoverageIo {
    CoverageIo {
        protocol: ProtocolIo {
            analyzer: None,
            scripted: responses.map(|r| std::cell::RefCell::new(r.into())),
            concepts,
            search_enabled: false,
            queries: vec![],
        },
        macro_result: Ok(String::new()),
        macro_calls: 0,
        search_failure: false,
        candidate_boards: vec![],
    }
}

#[tokio::test]
async fn macro_preparation_preserves_input_preference_and_observed_fallback_status() {
    for (input, fallback, expected, calls, status) in [
        (
            Some(" TEST_CODE_调用输入 Ω\r\n"),
            Err("must not run"),
            " TEST_CODE_调用输入 Ω\r\n",
            0,
            SourceStatus::Unknown,
        ),
        (
            Some(" \t\n"),
            Ok(" TEST_CODE_宏观回退 Ω\r\n"),
            " TEST_CODE_宏观回退 Ω\r\n",
            1,
            SourceStatus::Unknown,
        ),
        (None, Ok(""), "", 1, SourceStatus::Unknown),
        (
            None,
            Err("TEST_CODE_宏观上游失败"),
            "",
            1,
            SourceStatus::Unavailable,
        ),
    ] {
        let (stocks, concepts) = protocol_inputs();
        let mut io = coverage_io(concepts, None);
        io.macro_result = fallback.map(str::to_owned).map_err(str::to_owned);
        let prepared = prepare_chain_analysis_with_io(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            input.map(str::to_owned),
            &mut io,
        )
        .await
        .unwrap();
        assert_eq!(prepared.macro_input(), input);
        assert_eq!(prepared.macro_context(), expected);
        assert_eq!(prepared.macro_used_input(), calls == 0);
        assert_eq!(io.macro_calls, calls);
        assert_eq!(prepared.macro_source().status(), &status);
        assert_eq!(prepared.macro_source().source_at(), None);
        assert_eq!(prepared.macro_source().batch_id(), None);
        assert_eq!(
            prepared.macro_source().reason(),
            if status == SourceStatus::Unavailable {
                Some("TEST_CODE_宏观上游失败")
            } else {
                None
            }
        );
        assert!(prepared.lhb_map().is_empty());
        assert_eq!(prepared.lhb_source().status(), &SourceStatus::Unavailable);
        assert_eq!(
            prepared.lhb_source().reason(),
            Some("TEST_CODE_龙虎榜上游失败")
        );
        assert!(prepared
            .model_calls()
            .iter()
            .all(|call| call.not_called_reason() == Some("AI 模型未配置")));
        assert!(prepared.search_observations().is_empty());
        assert_eq!(
            prepared.after_market_source().status(),
            &SourceStatus::NotRequested
        );
        assert!(prepared.report().contains("深度分析 0 条 + 简化分析 0 条"));
    }
}

#[tokio::test]
async fn optional_failures_preserve_missing_analysis_and_uncertified_empty_searches() {
    use super::preparation::{ModelStage, SearchStage};
    for search_failure in [true, false] {
        let (stocks, concepts) = protocol_inputs();
        let mut io = coverage_io(
            concepts,
            Some(vec![
                Err("TEST_CODE_检索词失败".into()),
                Err("TEST_CODE_深度失败".into()),
                Err("TEST_CODE_简化失败".into()),
                Err("TEST_CODE_总览失败".into()),
            ]),
        );
        io.protocol.search_enabled = true;
        io.search_failure = search_failure;
        let prepared = prepare_chain_analysis_with_io(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            Some("TEST_CODE_宏观".into()),
            &mut io,
        )
        .await
        .unwrap();
        assert_eq!(prepared.model_calls().len(), 4);
        for (call, (stage, reason)) in prepared.model_calls().iter().zip([
            (ModelStage::SearchTerms, "TEST_CODE_检索词失败"),
            (ModelStage::Deep, "TEST_CODE_深度失败"),
            (ModelStage::Simple, "TEST_CODE_简化失败"),
            (ModelStage::Overview, "TEST_CODE_总览失败"),
        ]) {
            assert_eq!(call.stage(), stage);
            assert_eq!(call.failure(), Some(reason));
            assert_eq!(call.response(), None);
            assert!(call.prompt().is_some());
            assert!(!prepared.report().contains(reason));
        }
        assert!(io.protocol.scripted.as_ref().unwrap().borrow().is_empty());
        assert_eq!(prepared.search_observations().len(), 3);
        assert_eq!(
            prepared.search_observations()[0].stage(),
            SearchStage::Cluster
        );
        assert_eq!(prepared.search_observations()[0].limit(), 4);
        for search in &prepared.search_observations()[1..] {
            assert_eq!(search.stage(), SearchStage::AfterMarket);
            assert_eq!(search.limit(), 2);
            assert!(search.query().starts_with("07月22日 "));
        }
        let expected = if search_failure {
            SourceStatus::Unavailable
        } else {
            SourceStatus::Unknown
        };
        for search in prepared.search_observations() {
            assert!(search.results().is_empty());
            assert_eq!(search.source().status(), &expected);
            assert_eq!(
                search.source().reason(),
                if search_failure {
                    Some("TEST_CODE_搜索上游失败")
                } else {
                    None
                }
            );
            assert_eq!(search.source().source_at(), None);
        }
        assert_eq!(
            prepared.cluster_news_sources()["TEST_CODE_深度主线"].status(),
            &expected
        );
        assert_eq!(prepared.after_market_source().status(), &expected);
        assert_eq!(
            prepared.after_market_observed_at(),
            Some("2026-07-22T00:30:00+08:00")
        );
        assert_eq!(prepared.business_date().to_string(), "2026-07-21");
        assert_eq!(prepared.cluster_news()["TEST_CODE_深度主线"], "");
        assert_eq!(prepared.after_market_context(), "");
        assert!(prepared.report().contains("深度分析 0 条 + 简化分析 0 条"));
    }
}

#[tokio::test]
async fn preparation_enforces_deep_simple_and_candidate_limits_without_losing_clusters() {
    use super::preparation::ModelStage;
    let mut stocks = vec![];
    let mut concepts = HashMap::new();
    // Nine deep-eligible clusters: the ninth must consume one of twelve simple slots.
    // Twelve ordinary simple clusters plus one small cluster exercise both unselected paths.
    for group in 0..22 {
        let count = if group < 9 {
            8
        } else if group < 21 {
            4
        } else {
            3
        };
        for item in 0..count {
            let code = format!("TEST_CODE_LIMIT_{group}_{item}");
            stocks.push(TopStock {
                code: code.clone(),
                name: code.clone(),
                change_pct: 10.0,
                price: 10.0,
                ..TopStock::default()
            });
            concepts.insert(code, vec![format!("TEST_CODE_限额主线{group:02}")]);
        }
    }
    let mut responses =
        vec![
            Ok("【结论】阶段=启动｜参与=可关注｜候选=无\n TEST_CODE_深度正文 Ω\n".into());
            8
        ];
    responses.extend(vec![
        Ok(
            "【简评】阶段=发酵｜参与=谨慎｜候选=无\n TEST_CODE_简化正文 Ω\n".into()
        );
        12
    ]);
    responses.push(Ok(" TEST_CODE_总览正文 Ω\n".into()));
    let mut io = coverage_io(concepts, Some(responses));
    let prepared = prepare_chain_analysis_with_io(
        chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks,
        Some("TEST_CODE_宏观".into()),
        &mut io,
    )
    .await
    .unwrap();
    assert_eq!(prepared.clusters().len(), 22);
    assert!(prepared.isolated().is_empty());
    assert_eq!(io.candidate_boards.len(), 20);
    assert_eq!(
        io.candidate_boards
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        20
    );
    assert_eq!(
        prepared
            .candidate_sources()
            .values()
            .filter(|s| s.status() == &SourceStatus::VerifiedEmpty)
            .count(),
        20
    );
    assert_eq!(
        prepared
            .candidate_sources()
            .values()
            .filter(|s| s.status() == &SourceStatus::NotRequested)
            .count(),
        2
    );
    for (stage, count) in [
        (ModelStage::Deep, 8),
        (ModelStage::Simple, 12),
        (ModelStage::Overview, 1),
        (ModelStage::UnselectedCluster, 2),
        (ModelStage::SearchTerms, 8),
    ] {
        assert_eq!(
            prepared
                .model_calls()
                .iter()
                .filter(|c| c.stage() == stage)
                .count(),
            count
        );
    }
    let simple_calls = prepared
        .model_calls()
        .iter()
        .filter(|c| c.stage() == ModelStage::Simple)
        .collect::<Vec<_>>();
    assert_eq!(
        prepared
            .clusters()
            .iter()
            .find(|c| Some(c.concept.as_str()) == simple_calls[0].concept())
            .unwrap()
            .stocks
            .len(),
        8
    );
    assert!(prepared
        .model_calls()
        .iter()
        .filter(|c| c.stage() == ModelStage::SearchTerms)
        .all(|c| c.not_called_reason() == Some("新闻搜索未配置，未生成检索词")));
    assert!(io.protocol.scripted.as_ref().unwrap().borrow().is_empty());
    assert!(prepared.search_observations().is_empty());
    assert!(prepared.report().contains("深度分析 8 条 + 简化分析 12 条"));
}

#[async_trait::async_trait(?Send)]
impl ChainPreparationIo for RejectExternalIo {
    async fn concepts(
        &mut self,
        _codes: &[String],
    ) -> anyhow::Result<HashMap<String, Vec<String>>> {
        panic!("empty preparation must not touch external I/O");
    }
}

struct SyntheticIo {
    events: Vec<&'static str>,
    fail_at: Option<&'static str>,
    candidate_batch: Option<GatewayBatch<TopStock>>,
}

#[async_trait::async_trait(?Send)]
impl ChainPreparationIo for SyntheticIo {
    async fn concepts(&mut self, codes: &[String]) -> anyhow::Result<HashMap<String, Vec<String>>> {
        self.events.push("concepts");
        if self.fail_at == Some("concepts") {
            anyhow::bail!("TEST_CODE_核心概念失败");
        }
        Ok(codes
            .iter()
            .map(|code| {
                let tags = match code.as_str() {
                    "TEST_CODE_A" => vec!["TEST_CODE_产业", "昨日涨停"],
                    "TEST_CODE_B" | "TEST_CODE_C" | "TEST_CODE_持仓" => vec!["TEST_CODE_产业"],
                    "TEST_CODE_D" => vec!["TEST_CODE_独立"],
                    _ => panic!("unregistered synthetic code"),
                };
                (code.clone(), tags.into_iter().map(str::to_owned).collect())
            })
            .collect())
    }

    fn min_cluster_size(&mut self) -> usize {
        3
    }

    async fn persist_clusters(
        &mut self,
        business_date: chrono::NaiveDate,
        rows: &[(String, Vec<String>, i32)],
    ) -> anyhow::Result<HashMap<String, i64>> {
        self.events.push("chain_daily");
        if self.fail_at == Some("lifecycle") {
            anyhow::bail!("TEST_CODE_生命周期读取失败（写入后）");
        }
        assert_eq!(business_date.to_string(), "2026-07-21");
        assert_eq!(
            rows,
            &[(
                "TEST_CODE_产业".to_string(),
                vec![
                    "TEST_CODE_A".to_string(),
                    "TEST_CODE_B".to_string(),
                    "TEST_CODE_C".to_string()
                ],
                1
            )]
        );
        Ok(HashMap::from([("TEST_CODE_产业".into(), 2)]))
    }

    async fn board_codes(
        &mut self,
    ) -> anyhow::Result<(HashMap<String, String>, Vec<BatchEvidence>)> {
        self.events.push("board_codes");
        if self.candidate_batch.is_some() {
            return Ok((
                HashMap::from([("TEST_CODE_产业".into(), "TEST_CODE_BK".into())]),
                vec![
                    candidate_evidence(
                        "TEST_CODE_行业目录",
                        "TEST_CODE_BOARD_1",
                        Some("2026-07-21T08:00:00+08:00"),
                    ),
                    candidate_evidence("TEST_CODE_概念目录", "TEST_CODE_BOARD_2", None),
                ],
            ));
        }
        anyhow::bail!("TEST_CODE_目录不可用原因")
    }

    async fn candidates(
        &mut self,
        board: &str,
        excluded: &std::collections::HashSet<String>,
    ) -> anyhow::Result<GatewayBatch<TopStock>> {
        self.events.push("candidates");
        assert_eq!(board, "TEST_CODE_BK");
        assert_eq!(excluded.len(), 4);
        assert!(excluded.contains("TEST_CODE_A"));
        Ok(self
            .candidate_batch
            .clone()
            .expect("unavailable directory must not request candidates"))
    }

    async fn positions(&mut self) -> anyhow::Result<Vec<PositionInput>> {
        self.events.push("positions");
        if self.fail_at == Some("positions") {
            anyhow::bail!("TEST_CODE_核心持仓失败");
        }
        Ok(vec![PositionInput::new(
            "TEST_CODE_持仓".into(),
            "TEST_CODE_敏感持仓名".into(),
            Some(1.25),
        )])
    }

    async fn lhb(&mut self) -> anyhow::Result<(HashMap<String, f64>, SourceObservation)> {
        self.events.push("lhb");
        Ok((
            HashMap::from([("TEST_CODE_A".into(), 123.0)]),
            SourceObservation::unknown(),
        ))
    }

    async fn macro_search(&mut self) -> anyhow::Result<String> {
        panic!("nonblank caller macro input must win")
    }

    fn model_available(&mut self) -> bool {
        self.events.push("model_availability");
        false
    }
}

fn synthetic_input() -> Vec<TopStock> {
    ["A", "B", "C", "D"]
        .iter()
        .enumerate()
        .map(|(index, suffix)| TopStock {
            code: format!("TEST_CODE_{suffix}"),
            name: format!("TEST_CODE_标的{suffix}"),
            change_pct: 10.0 - index as f64 / 10.0,
            price: 12.0,
            ..TopStock::default()
        })
        .collect()
}

#[tokio::test]
async fn nonempty_preparation_retains_consumed_facts_and_one_effect_sequence() {
    let input = synthetic_input();
    let mut io = SyntheticIo {
        events: Vec::new(),
        fail_at: None,
        candidate_batch: None,
    };
    let prepared = prepare_chain_analysis_with_io(
        chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        input,
        Some(" TEST_CODE_原始宏观\n".into()),
        &mut io,
    )
    .await
    .expect("complete synthetic preparation");

    assert_eq!(prepared.clusters().len(), 1);
    assert_eq!(prepared.clusters()[0].concept, "TEST_CODE_产业");
    assert_eq!(prepared.clusters()[0].continuation_count, 1);
    assert_eq!(prepared.clusters()[0].streak_days, 2);
    assert_eq!(prepared.isolated()[0].code, "TEST_CODE_D");
    assert_eq!(
        prepared.concepts()["TEST_CODE_A"],
        vec!["TEST_CODE_产业", "昨日涨停"]
    );
    assert_eq!(
        prepared.position_diags()[0].mainline,
        Some(("TEST_CODE_产业".into(), 2))
    );
    assert!(!prepared.position_diags()[0].in_limit_pool);
    assert_eq!(
        prepared.position_concepts()["TEST_CODE_持仓"],
        vec!["TEST_CODE_产业"]
    );
    assert_eq!(
        prepared.candidate_sources()["TEST_CODE_产业"].status(),
        &SourceStatus::Unavailable
    );
    assert!(prepared.candidate_sources()["TEST_CODE_产业"]
        .reason()
        .unwrap()
        .contains("TEST_CODE_目录不可用原因"));
    assert_eq!(prepared.lhb_map()["TEST_CODE_A"], 123.0);
    assert_eq!(prepared.macro_context(), " TEST_CODE_原始宏观\n");
    assert!(prepared.report().contains("TEST_CODE_敏感持仓名"));
    assert!(prepared.report().contains("TEST_CODE_产业"));
    assert_eq!(
        io.events,
        vec![
            "concepts",
            "chain_daily",
            "board_codes",
            "positions",
            "concepts",
            "lhb",
            "model_availability"
        ]
    );
}

#[tokio::test]
async fn core_failure_retains_stage_and_prior_observations_without_continuing_effects() {
    use super::preparation::{PreparationFailure, PreparationStage};
    let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
    for (fail_at, stage, completed, events, reason) in [
        (
            "concepts",
            PreparationStage::Concepts,
            vec![],
            vec!["concepts"],
            "TEST_CODE_核心概念失败",
        ),
        (
            "lifecycle",
            PreparationStage::ClusterWritesAndLifecycle,
            vec![PreparationStage::Concepts],
            vec!["concepts", "chain_daily"],
            "TEST_CODE_生命周期读取失败（写入后）",
        ),
        (
            "positions",
            PreparationStage::Positions,
            vec![
                PreparationStage::Concepts,
                PreparationStage::ClusterWritesAndLifecycle,
                PreparationStage::Candidates,
            ],
            vec!["concepts", "chain_daily", "board_codes", "positions"],
            "TEST_CODE_核心持仓失败",
        ),
    ] {
        let mut io = SyntheticIo {
            events: Vec::new(),
            fail_at: Some(fail_at),
            candidate_batch: None,
        };
        let error = prepare_chain_analysis_with_io(
            date,
            synthetic_input(),
            Some("TEST_CODE_核心失败宏观".into()),
            &mut io,
        )
        .await
        .expect_err("core failure must block preparation");
        let failure = error
            .downcast_ref::<PreparationFailure>()
            .expect("typed stage observations survive failure");
        assert_eq!(failure.business_date(), date);
        assert_eq!(failure.stage(), stage);
        assert_eq!(failure.completed_stages(), completed);
        assert_eq!(failure.reason(), reason);
        assert_eq!(failure.limit_ups()[0].code, "TEST_CODE_A");
        assert_eq!(failure.macro_input(), Some("TEST_CODE_核心失败宏观"));
        if fail_at == "concepts" {
            assert!(failure.concepts().is_empty());
            assert!(failure.clusters().is_empty());
        } else {
            assert_eq!(
                failure.concepts()["TEST_CODE_A"],
                vec!["TEST_CODE_产业", "昨日涨停"]
            );
        }
        if fail_at == "positions" {
            assert_eq!(failure.clusters()[0].concept, "TEST_CODE_产业");
            assert_eq!(failure.clusters()[0].continuation_count, 1);
            assert_eq!(failure.clusters()[0].streak_days, 2);
            assert_eq!(
                failure.candidate_sources()["TEST_CODE_产业"].status(),
                &SourceStatus::Unavailable
            );
            assert!(failure.candidate_sources()["TEST_CODE_产业"]
                .reason()
                .unwrap()
                .contains("TEST_CODE_目录不可用原因"));
        }
        assert!(failure.failed_stage_may_have_effects());
        assert_eq!(io.events, events);
        assert!(!format!("{failure:?}").contains("TEST_CODE"));
        assert!(!error.to_string().contains("TEST_CODE"));
    }
}

fn candidate_evidence(source: &str, batch_id: &str, source_at: Option<&str>) -> BatchEvidence {
    BatchEvidence {
        provider: crate::market_domain::ProviderId::Custom,
        source: source.into(),
        source_at: source_at.map(str::to_owned),
        observed_at: "2026-07-21T16:00:00+08:00".into(),
        batch_id: batch_id.into(),
    }
}

#[tokio::test]
async fn candidate_preparation_retains_full_evidence_before_optional_projection() {
    for (batch, expected_status, count) in [
        (
            GatewayBatch::Available {
                records: vec![TopStock {
                    code: "TEST_CODE_候选".into(),
                    name: "TEST_CODE_候选名".into(),
                    change_pct: 3.0,
                    price: 12.0,
                    ..TopStock::default()
                }],
                evidence: candidate_evidence(
                    "TEST_CODE_候选来源",
                    "TEST_CODE_CANDIDATE",
                    Some("2026-07-21T15:00:00+08:00"),
                ),
            },
            SourceStatus::Available,
            1,
        ),
        (
            GatewayBatch::VerifiedEmpty(candidate_evidence(
                "TEST_CODE_候选来源",
                "TEST_CODE_CANDIDATE",
                Some("2026-07-21T15:00:00+08:00"),
            )),
            SourceStatus::VerifiedEmpty,
            0,
        ),
        (
            GatewayBatch::Available {
                records: vec![],
                evidence: candidate_evidence(
                    "TEST_CODE_候选来源",
                    "TEST_CODE_CANDIDATE",
                    Some("2026-07-21T15:00:00+08:00"),
                ),
            },
            SourceStatus::Unavailable,
            0,
        ),
    ] {
        let mut io = SyntheticIo {
            events: Vec::new(),
            fail_at: None,
            candidate_batch: Some(batch),
        };
        let prepared = prepare_chain_analysis_with_io(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            synthetic_input(),
            Some("TEST_CODE_候选宏观".into()),
            &mut io,
        )
        .await
        .unwrap();
        let bytes = prepared.to_artifact_bytes().unwrap();
        let restored =
            super::preparation::PreparedChainAnalysis::from_artifact_bytes(&bytes).unwrap();
        assert_eq!(restored.report().as_bytes(), prepared.report().as_bytes());
        let prepared = restored;
        let source = &prepared.candidate_sources()["TEST_CODE_产业"];
        assert_eq!(source.status(), &expected_status);
        assert_eq!(
            source.provider(),
            Some(crate::market_domain::ProviderId::Custom)
        );
        assert_eq!(source.source(), Some("TEST_CODE_候选来源"));
        assert_eq!(source.source_at(), Some("2026-07-21T15:00:00+08:00"));
        assert_eq!(source.observed_at(), Some("2026-07-21T16:00:00+08:00"));
        assert_eq!(source.batch_id(), Some("TEST_CODE_CANDIDATE"));
        assert_eq!(prepared.clusters()[0].candidates.len(), count);
        let boards = prepared.board_evidence();
        assert_eq!(boards.len(), 2);
        assert_eq!(
            boards[0].provider(),
            Some(crate::market_domain::ProviderId::Custom)
        );
        assert_eq!(boards[0].source(), Some("TEST_CODE_行业目录"));
        assert_eq!(boards[0].source_at(), Some("2026-07-21T08:00:00+08:00"));
        assert_eq!(boards[0].observed_at(), Some("2026-07-21T16:00:00+08:00"));
        assert_eq!(boards[0].batch_id(), Some("TEST_CODE_BOARD_1"));
        assert_eq!(
            boards[1].provider(),
            Some(crate::market_domain::ProviderId::Custom)
        );
        assert_eq!(boards[1].source(), Some("TEST_CODE_概念目录"));
        assert_eq!(boards[1].source_at(), None);
        assert_eq!(boards[1].observed_at(), Some("2026-07-21T16:00:00+08:00"));
        assert_eq!(boards[1].batch_id(), Some("TEST_CODE_BOARD_2"));
        assert_eq!(prepared.board_directory()["TEST_CODE_产业"], "TEST_CODE_BK");
        assert_eq!(
            prepared.candidate_board_codes()["TEST_CODE_产业"],
            "TEST_CODE_BK"
        );
        if expected_status == SourceStatus::Unavailable {
            assert!(source.reason().unwrap().contains("Available 但记录为空"));
            assert!(prepared.report().contains("数据不可用"));
        }
        assert_eq!(
            io.events
                .iter()
                .filter(|event| **event == "candidates")
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn empty_preparation_fixes_business_date_and_original_report_without_external_effects() {
    let business_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
    let mut io = RejectExternalIo;
    let prepared = prepare_chain_analysis_with_io(
        business_date,
        Vec::new(),
        Some(" TEST_CODE_调用方宏观上下文\n".into()),
        &mut io,
    )
    .await
    .expect("empty preparation is valid without any external source");

    assert_eq!(prepared.business_date(), business_date);
    assert_eq!(
        prepared.report().as_bytes(),
        "# 产业链联动分析报告 2026-07-21\n\n涨停池批次成功返回 0 只，无可分析内容。\n".as_bytes()
    );
    assert!(prepared.limit_ups().is_empty());
    assert_eq!(prepared.limit_up_source().status(), &SourceStatus::Unknown);
    assert_eq!(prepared.limit_up_source().source_at(), None);
    assert_eq!(prepared.limit_up_source().batch_id(), None);
    assert_eq!(
        prepared.macro_input(),
        Some(" TEST_CODE_调用方宏观上下文\n")
    );
}
