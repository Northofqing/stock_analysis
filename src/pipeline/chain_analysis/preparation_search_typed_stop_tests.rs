use super::super::preparation::{PreparationFailure, PreparationStage};
use super::*;

const CLUSTER_INTENT: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const AFTER_MARKET_INTENT: &str =
    "2222222222222222222222222222222222222222222222222222222222222222";
const MACRO_INPUT: &str = "TEST_CODE_固定宏观输入";
const FIRST_CLUSTER_QUERY: &str =
    "TEST_CODE_深度主线 板块 集体涨停 原因 TEST_CODE_名称DEEP0 TEST_CODE_名称DEEP1";
const FIRST_AFTER_MARKET_QUERY: &str = "07月21日 TEST_CODE_深度主线 最新 突发 催化";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailurePoint {
    Cluster,
    AfterMarket,
}

impl FailurePoint {
    fn intent(self) -> &'static str {
        match self {
            Self::Cluster => CLUSTER_INTENT,
            Self::AfterMarket => AFTER_MARKET_INTENT,
        }
    }

    fn operation(self) -> &'static str {
        match self {
            Self::Cluster => "cluster search checkpoint",
            Self::AfterMarket => "after-market search checkpoint",
        }
    }
}

struct SearchStopIo {
    base: CoverageIo,
    failure: FailurePoint,
    effects: Vec<&'static str>,
    model_available_calls: usize,
    model_stages: std::cell::RefCell<Vec<&'static str>>,
    search_available_calls: usize,
    clock_calls: usize,
    queries: Vec<(String, usize)>,
    render_calls: std::cell::RefCell<Vec<&'static str>>,
}

#[async_trait::async_trait(?Send)]
impl ChainPreparationIo for SearchStopIo {
    async fn concepts(&mut self, codes: &[String]) -> anyhow::Result<HashMap<String, Vec<String>>> {
        self.effects.push("concepts");
        self.base.concepts(codes).await
    }

    fn min_cluster_size(&mut self) -> usize {
        self.base.min_cluster_size()
    }

    async fn persist_clusters(
        &mut self,
        date: chrono::NaiveDate,
        rows: &[(String, Vec<String>, i32)],
    ) -> anyhow::Result<HashMap<String, i64>> {
        self.effects.push("chain_daily");
        self.base.persist_clusters(date, rows).await
    }

    async fn board_codes(
        &mut self,
    ) -> anyhow::Result<(HashMap<String, String>, Vec<BatchEvidence>)> {
        self.effects.push("board_codes");
        self.base.board_codes().await
    }

    async fn candidates(
        &mut self,
        board: &str,
        excluded: &std::collections::HashSet<String>,
    ) -> anyhow::Result<GatewayBatch<TopStock>> {
        self.effects.push("candidates");
        self.base.candidates(board, excluded).await
    }

    async fn positions(&mut self) -> anyhow::Result<Vec<PositionInput>> {
        self.effects.push("positions");
        Ok(vec![PositionInput::new(
            "TEST_CODE_协议持仓".into(),
            "TEST_CODE_持仓敏感正文".into(),
            Some(1.5),
        )])
    }

    async fn lhb(&mut self) -> anyhow::Result<(HashMap<String, f64>, SourceObservation)> {
        self.effects.push("lhb");
        self.base.protocol.lhb().await
    }

    async fn macro_search(&mut self) -> anyhow::Result<String> {
        self.base.macro_search().await
    }

    fn model_available(&mut self) -> bool {
        self.model_available_calls += 1;
        true
    }

    async fn model(
        &self,
        prompt: &str,
        system: &str,
        mode: crate::analyzer::AgentMode,
    ) -> anyhow::Result<String> {
        let stage = if mode == crate::analyzer::AgentMode::Quick
            && system == "你是A股题材挖掘专家，只输出新闻搜索词，每行一条。"
            && prompt.contains("今日 A 股「TEST_CODE_深度主线」概念 8 只股票集体涨停")
        {
            "SearchTerms"
        } else if mode == crate::analyzer::AgentMode::Deep
            && prompt.contains("概念「TEST_CODE_深度主线」")
            && prompt.contains("聚集了 8 只涨停股")
            && prompt.contains(MACRO_INPUT)
        {
            "Deep"
        } else if mode == crate::analyzer::AgentMode::Quick
            && prompt.contains("概念「TEST_CODE_简化主线」")
            && prompt.contains("聚集了 4 只涨停股")
        {
            "Simple"
        } else if mode == crate::analyzer::AgentMode::Quick
            && prompt.contains("以下是今日各涨停主线的产业链分析摘要")
        {
            "Overview"
        } else {
            "Unexpected"
        };
        self.model_stages.borrow_mut().push(stage);
        self.render_calls.borrow_mut().push(stage);
        Ok(match stage {
            "SearchTerms" => "TEST_CODE_供给变化\nTEST_CODE_终端需求",
            "Deep" => "【结论】阶段=发酵｜参与=谨慎｜候选=无\n【评分】产业逻辑=70/100｜情绪位置=60/100｜资金共识=65/100｜筹码健康=50/100｜证伪概率=40/100\nTEST_CODE_受控深度模型正文",
            "Simple" => "【简评】阶段=发酵｜参与=谨慎｜候选=无\n【评分】产业逻辑=70/100｜情绪位置=60/100｜资金共识=65/100｜筹码健康=50/100｜证伪概率=40/100\nTEST_CODE_受控简化模型正文",
            "Overview" => "TEST_CODE_受控总览模型正文",
            _ => "TEST_CODE_意外模型调用",
        }
        .to_owned())
    }

    fn search_available(&mut self) -> bool {
        self.search_available_calls += 1;
        true
    }

    async fn search_topic(
        &mut self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::search_service::SearchResult>> {
        let stage = if query.contains("最新 突发 催化") {
            "AfterMarketSearch"
        } else {
            "ClusterSearch"
        };
        self.queries.push((query.to_owned(), limit));
        self.render_calls.borrow_mut().push(stage);
        let selected = match self.failure {
            FailurePoint::Cluster => stage == "ClusterSearch" && self.queries.len() == 1,
            FailurePoint::AfterMarket => {
                stage == "AfterMarketSearch"
                    && self
                        .queries
                        .iter()
                        .filter(|(query, _)| query.contains("最新 突发 催化"))
                        .count()
                        == 1
            }
        };
        if selected {
            return Err(anyhow::Error::new(SyntheticStorageCause {
                operation: self.failure.operation(),
            })
            .context(PreparationStop::AuthorityRejected {
                intent_id: self.failure.intent().to_owned(),
            }));
        }
        Ok(Vec::new())
    }

    fn local_now(&mut self) -> chrono::DateTime<chrono::FixedOffset> {
        self.clock_calls += 1;
        self.render_calls.borrow_mut().push("Clock");
        chrono::DateTime::parse_from_rfc3339("2026-07-21T16:00:00+08:00").unwrap()
    }
}

struct Scenario {
    failure: FailurePoint,
    error: Option<anyhow::Error>,
    effects: Vec<&'static str>,
    model_available_calls: usize,
    model_stages: Vec<&'static str>,
    search_available_calls: usize,
    clock_calls: usize,
    queries: Vec<(String, usize)>,
    render_calls: Vec<&'static str>,
    macro_calls: usize,
    candidate_calls: usize,
}

impl Scenario {
    fn result_kind(&self) -> &'static str {
        match self.error.as_ref() {
            None => "Ok",
            Some(error) if error.downcast_ref::<PreparationStop>().is_some() => "TypedStop",
            Some(_) => "OtherError",
        }
    }

    fn is_expected_stop(&self) -> bool {
        self.error.as_ref().is_some_and(|error| {
            matches!(
                error.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::AuthorityRejected { intent_id })
                    if intent_id == self.failure.intent()
            ) && matches!(
                error.downcast_ref::<SyntheticStorageCause>(),
                Some(SyntheticStorageCause { operation })
                    if *operation == self.failure.operation()
            )
        })
    }
}

async fn run_scenario(failure: FailurePoint) -> Scenario {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
    let (stocks, concepts) = protocol_inputs();
    assert_eq!(stocks.len(), 12);
    let mut io = SearchStopIo {
        base: coverage_io(concepts, None),
        failure,
        effects: Vec::new(),
        model_available_calls: 0,
        model_stages: std::cell::RefCell::new(Vec::new()),
        search_available_calls: 0,
        clock_calls: 0,
        queries: Vec::new(),
        render_calls: std::cell::RefCell::new(Vec::new()),
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        prepare_chain_analysis_with_io(date, stocks, Some(MACRO_INPUT.to_owned()), &mut io),
    )
    .await
    .expect("TEST_CODE search typed-stop bounded preparation");
    Scenario {
        failure,
        error: result.err(),
        effects: io.effects,
        model_available_calls: io.model_available_calls,
        model_stages: io.model_stages.into_inner(),
        search_available_calls: io.search_available_calls,
        clock_calls: io.clock_calls,
        queries: io.queries,
        render_calls: io.render_calls.into_inner(),
        macro_calls: io.base.macro_calls,
        candidate_calls: io.base.candidate_boards.len(),
    }
}

fn assert_prior_facts(scenario: &Scenario) {
    let error = scenario.error.as_ref().unwrap();
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("TEST_CODE search typed stop retains PreparationFailure");
    assert_eq!(
        failure.business_date(),
        chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap()
    );
    assert_eq!(failure.stage(), PreparationStage::ModelsSearchAndReport);
    assert!(failure.failed_stage_may_have_effects());
    assert_eq!(
        failure.completed_stages(),
        [
            PreparationStage::Concepts,
            PreparationStage::ClusterWritesAndLifecycle,
            PreparationStage::Candidates,
            PreparationStage::Positions,
            PreparationStage::PositionConcepts,
            PreparationStage::DragonTiger,
            PreparationStage::Macro,
        ]
    );
    assert_eq!(failure.limit_ups().len(), 12);
    assert_eq!(
        failure.concepts()["TEST_CODE_DEEP_0"],
        ["TEST_CODE_深度主线"]
    );
    assert_eq!(failure.clusters().len(), 2);
    assert_eq!(
        failure
            .clusters()
            .iter()
            .find(|cluster| cluster.concept == "TEST_CODE_深度主线")
            .unwrap()
            .stocks
            .len(),
        8
    );
    assert_eq!(
        failure
            .clusters()
            .iter()
            .find(|cluster| cluster.concept == "TEST_CODE_简化主线")
            .unwrap()
            .stocks
            .len(),
        4
    );
    assert_eq!(failure.positions().len(), 1);
    assert_eq!(failure.positions()[0].code(), "TEST_CODE_协议持仓");
    assert_eq!(
        failure.position_concepts()["TEST_CODE_协议持仓"],
        ["TEST_CODE_深度主线"]
    );
    assert_eq!(failure.lhb_map()["TEST_CODE_DEEP_0"], 321.0);
    assert_eq!(failure.lhb_source().status(), &SourceStatus::Unknown);
    assert_eq!(failure.macro_input(), Some(MACRO_INPUT));
    assert_eq!(
        failure
            .candidate_sources()
            .values()
            .filter(|source| source.status() == &SourceStatus::VerifiedEmpty)
            .count(),
        2
    );
}

#[tokio::test]
async fn search_typed_stops_abort_cluster_and_after_market_effects() {
    let cluster = run_scenario(FailurePoint::Cluster).await;
    let after_market = run_scenario(FailurePoint::AfterMarket).await;
    let cluster_stop = cluster.is_expected_stop();
    let after_market_stop = after_market.is_expected_stop();
    if cluster_stop {
        assert_prior_facts(&cluster);
    }
    if after_market_stop {
        assert_prior_facts(&after_market);
    }

    let expected_effects = [
        "concepts",
        "chain_daily",
        "board_codes",
        "candidates",
        "candidates",
        "positions",
        "concepts",
        "lhb",
    ];
    let cluster_ok = cluster_stop
        && cluster.effects == expected_effects
        && cluster.model_available_calls == 1
        && cluster.model_stages == ["SearchTerms"]
        && cluster.search_available_calls == 1
        && cluster.clock_calls == 0
        && cluster.queries == [(FIRST_CLUSTER_QUERY.to_owned(), 4)]
        && cluster.render_calls == ["SearchTerms", "ClusterSearch"]
        && cluster.macro_calls == 0
        && cluster.candidate_calls == 2;
    let after_market_ok = after_market_stop
        && after_market.effects == expected_effects
        && after_market.model_available_calls == 1
        && after_market.model_stages == ["SearchTerms", "Deep", "Simple"]
        && after_market.search_available_calls == 2
        && after_market.clock_calls == 1
        && after_market.queries
            == [
                (FIRST_CLUSTER_QUERY.to_owned(), 4),
                ("TEST_CODE_供给变化".to_owned(), 4),
                ("TEST_CODE_终端需求".to_owned(), 4),
                (FIRST_AFTER_MARKET_QUERY.to_owned(), 2),
            ]
        && after_market.render_calls
            == [
                "SearchTerms",
                "ClusterSearch",
                "ClusterSearch",
                "ClusterSearch",
                "Deep",
                "Simple",
                "Clock",
                "AfterMarketSearch",
            ]
        && after_market.macro_calls == 0
        && after_market.candidate_calls == 2;
    assert!(
        cluster_ok && after_market_ok,
        "TEST_CODE search typed stops must abort: cluster_result={}; cluster_models={:?}; cluster_effects={:?}; cluster_queries={:?}; cluster_render={:?}; cluster_search_available={}; cluster_clock={}; after_result={}; after_models={:?}; after_effects={:?}; after_queries={:?}; after_render={:?}; after_search_available={}; after_clock={}",
        cluster.result_kind(),
        cluster.model_stages,
        cluster.effects,
        cluster.queries,
        cluster.render_calls,
        cluster.search_available_calls,
        cluster.clock_calls,
        after_market.result_kind(),
        after_market.model_stages,
        after_market.effects,
        after_market.queries,
        after_market.render_calls,
        after_market.search_available_calls,
        after_market.clock_calls,
    );
}
