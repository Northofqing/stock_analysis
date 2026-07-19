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

fn cluster(concept: &str, base: usize, count: usize) -> ChainCluster {
    ChainCluster {
        concept: concept.to_string(),
        aliases: Vec::new(),
        stocks: (0..count)
            .map(|offset| {
                stock(
                    &format!("TEST_CODE_GATE_D_{:06}", base + offset),
                    &format!("失败协议股{offset}"),
                )
            })
            .collect(),
        continuation_count: 0,
        streak_days: 1,
        candidates: Vec::new(),
        score: None,
        scenario: None,
    }
}

#[tokio::test]
async fn model_commit_failures_remain_missing_sections_instead_of_fake_analysis() {
    let deep = cluster("TEST_CODE_深度失败", 100, TIER1_MIN);
    let simple = cluster("TEST_CODE_简化失败", 200, TIER2_MIN);
    let limit_ups: Vec<TopStock> = deep
        .stocks
        .iter()
        .chain(simple.stocks.iter())
        .cloned()
        .collect();
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
    let evidence = ResolvedChainEvidence {
        cluster_news: HashMap::new(),
        after_market: String::new(),
    };

    let report = render_resolved_chain_analysis(
        &analyzer,
        "2026-07-19",
        &limit_ups,
        &HashMap::new(),
        &[deep, simple],
        &[],
        &[],
        &HashMap::new(),
        "",
        ChainEvidenceSource::Resolved(evidence),
    )
    .await
    .expect("model failures must still produce a truthful cluster-only report");

    assert!(report.contains("深度分析 0 条 + 简化分析 0 条"));
    assert!(!report.contains("deep unavailable"));
    assert!(!report.contains("simple unavailable"));
    assert_eq!(server.finish().len(), 3);
}
