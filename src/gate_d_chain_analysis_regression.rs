use super::*;

fn stock(code: &str, name: &str) -> TopStock {
    TopStock {
        code: code.to_string(),
        name: name.to_string(),
        change_pct: 10.0,
        price: 10.0,
        ..TopStock::default()
    }
}

fn stocks(base: usize, count: usize) -> Vec<TopStock> {
    (0..count)
        .map(|offset| {
            stock(
                &format!("TEST_CODE_GATE_D_{:06}", base + offset),
                &format!("失败协议股{offset}"),
            )
        })
        .collect()
}

#[tokio::test]
async fn model_commit_failures_remain_missing_sections_instead_of_fake_analysis() {
    let deep = stocks(100, TIER1_MIN);
    let simple = stocks(200, TIER2_MIN);
    let mut concepts: HashMap<String, Vec<String>> = deep
        .iter()
        .map(|stock| (stock.code.clone(), vec!["TEST_CODE_深度失败".into()]))
        .collect();
    concepts.extend(
        simple
            .iter()
            .map(|stock| (stock.code.clone(), vec!["TEST_CODE_简化失败".into()])),
    );
    concepts.insert(
        "TEST_CODE_协议持仓".into(),
        vec!["TEST_CODE_深度失败".into()],
    );
    let limit_ups = deep.into_iter().chain(simple).collect();
    let server = crate::data_provider::TestHttpServer::new(vec![
        crate::data_provider::TestHttpResponse {
            status: 503,
            body: "deep unavailable".into(),
        },
        crate::data_provider::TestHttpResponse {
            status: 503,
            body: "simple unavailable".into(),
        },
        crate::data_provider::TestHttpResponse {
            status: 503,
            body: "overview unavailable".into(),
        },
    ]);
    let analyzer = GeminiAnalyzer::with_loopback_client(crate::analyzer::GeminiConfig {
        doubao_api_key: Some("TEST_CODE_LOCAL_PROTOCOL_KEY".into()),
        doubao_base_url: Some(server.base_url().to_string()),
        doubao_model: "TEST_CODE_MODEL".into(),
        max_retries: 1,
        retry_delay: 0.0,
        request_delay: 0.0,
        agent_pipeline: false,
        ..crate::analyzer::GeminiConfig::default()
    });
    let mut io = preparation_tests::ProtocolIo {
        analyzer: Some(analyzer),
        scripted: None,
        concepts,
        search_enabled: false,
        queries: vec![],
    };

    let prepared = preparation::prepare_chain_analysis_with_io(
        chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        limit_ups,
        Some("TEST_CODE_显式宏观输入".into()),
        &mut io,
    )
    .await
    .expect("model failures must still produce a truthful cluster-only report");

    let report = prepared.report();
    assert!(report.contains("深度分析 0 条 + 简化分析 0 条"));
    assert!(!report.contains("deep unavailable"));
    assert!(!report.contains("simple unavailable"));
    let failed = prepared
        .model_calls()
        .iter()
        .filter(|call| call.failure().is_some())
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 3);
    for (call, stage) in failed.iter().zip([
        preparation::ModelStage::Deep,
        preparation::ModelStage::Simple,
        preparation::ModelStage::Overview,
    ]) {
        assert_eq!(call.stage(), stage);
        assert_eq!(call.response(), None);
        assert!(call.prompt().is_some());
        assert!(!format!("{call:?}").contains("TEST_CODE"));
    }
    let requests = server.finish();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|path| path == "/chat/completions"));
}
