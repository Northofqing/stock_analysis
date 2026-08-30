//! MarketDataService handler: 校验请求 → fixture 或 data_gateway 委托 → QueryResponse。
//! BR-238 preserves authenticated external batch evidence end-to-end.
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_server::MarketDataService, AdmissionState, CanonicalPayload, Operation,
    QueryRequest, QueryResponse,
};
use crate::grpc_contract::schema::schema_for;
use crate::grpc_server::{delegate, fixture, ServerState};
use prost::Message; // ErrorDetail::encode_to_vec (tonic 0.14 Status::with_details 取 bytes)
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub use crate::grpc_client::pb::magic::market::v1::market_data_service_server;

pub struct DataService {
    fixture_mode: bool,
}

impl DataService {
    pub fn new(_state: Arc<ServerState>, fixture_mode: bool) -> Self {
        Self { fixture_mode }
    }

    /// 统一查询入口: 校验 → 取数 → 包装 QueryResponse。
    async fn serve_query(
        &self,
        op: Operation,
        req: QueryRequest,
    ) -> Result<Response<QueryResponse>, Status> {
        let payload = req
            .payload
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("QueryRequest 缺 payload"))?;
        let request_schema = payload.schema.clone();
        let request_version = payload.schema_version;

        // 合同 §5: 未知 schema/version 必须拒绝。
        let frozen = schema_for(op).ok_or_else(|| {
            Status::unimplemented(format!(
                "{} 未实现",
                crate::grpc_contract::ops::method_name(op)
            ))
        })?;
        if frozen.schema_name != request_schema {
            return Err(Status::invalid_argument(format!(
                "schema 不匹配: op 期望 {} 实际 {request_schema}",
                frozen.schema_name
            )));
        }
        if frozen.schema_version != request_version {
            return Err(Status::invalid_argument(format!(
                "schema 版本不支持: {} v{request_version} (冻结 v{})",
                frozen.schema_name, frozen.schema_version
            )));
        }

        // fixture 模式 (离线确定性测试) 优先。
        if self.fixture_mode {
            if let Some(mut resp) = fixture::fixture_response(op, &request_schema, request_version)
            {
                // fixture 硬编码 request_id ("fixture-xxx"); 客户端 §6 严格匹配 request_id,
                // 必须回显请求的 request_id (与真实路径行为一致)。
                resp.request_id = req
                    .context
                    .as_ref()
                    .map(|c| c.request_id.clone())
                    .unwrap_or_default();
                return Ok(Response::new(resp));
            }
            return Err(Status::unimplemented(format!(
                "{} 无 fixture",
                frozen.schema_name
            )));
        }

        // 真实路径: 委托 data_gateway。gateway 全部是 async fn 且内部自行
        // spawn_blocking (见 delegate.rs 头注释), 直接 await, 不套 spawn_blocking。
        // 请求方向 payload.data = params JSON 对象 ({} = 全默认, 合同 §5 与
        // grpc_contract::params 默认值表); 缺失/非法 → invalid_argument。
        let params: serde_json::Value = match payload.data.as_slice() {
            [] => serde_json::json!({}),
            bytes => serde_json::from_slice(bytes).map_err(|e| {
                Status::invalid_argument(format!("payload.data 不是合法 JSON: {e}"))
            })?,
        };
        let request_id = req
            .context
            .as_ref()
            .map(|c| c.request_id.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let result = delegate::fetch(op, &request_schema, &params)
            .await
            .map_err(|e| match e {
                delegate::DelegateError::Params(pe) => Status::invalid_argument(pe.to_string()),
                // Fetch → internal + ErrorDetail (provider/reason_code/retryable),
                // 客户端桥据此重建 GatewayError 保真 (D2 分类不折叠)。
                delegate::DelegateError::Fetch(failure) => {
                    let detail = crate::grpc_client::pb::magic::market::v1::ErrorDetail {
                        request_id: request_id.clone(),
                        operation: op as i32,
                        provider: failure
                            .provider
                            .map(|p| format!("{:?}", p))
                            .unwrap_or_default(),
                        reason_code: failure.reason_code.to_string(),
                        retryable: failure.retryable,
                        ..Default::default()
                    };
                    Status::with_details(
                        tonic::Code::Internal,
                        format!("取数失败: {}", failure.message),
                        detail.encode_to_vec().into(), // Bytes (tonic 0.14)
                    )
                }
                delegate::DelegateError::BenchmarkFetch {
                    failure,
                    audit_outcome,
                    audit_state,
                } => {
                    let exact_server_failure = failure.provider
                        == Some(crate::market_domain::ProviderId::Tdx)
                        && crate::data_gateway::grpc_source::benchmark_server_failure_is_exact(
                            audit_outcome,
                            failure.reason_code,
                            failure.retryable,
                            audit_state,
                        );
                    let detail = crate::grpc_client::pb::magic::market::v1::BenchmarkErrorDetail {
                        error: Some(crate::grpc_client::pb::magic::market::v1::ErrorDetail {
                            request_id: request_id.clone(),
                            operation: op as i32,
                            // ErrorDetail.provider has no presence bit: empty means absent only.
                            // The benchmark client accepts ServerHandled solely for exact `Tdx`;
                            // empty therefore becomes an unknown transport audit and never invents Tdx.
                            provider: failure
                                .provider
                                .map(|p| format!("{:?}", p))
                                .unwrap_or_default(),
                            reason_code: failure.reason_code.to_string(),
                            retryable: failure.retryable,
                            ..Default::default()
                        }),
                        audit_outcome: audit_outcome.to_string(),
                        audit_state: if exact_server_failure {
                            audit_state.as_proto()
                        } else {
                            crate::grpc_client::pb::magic::market::v1::BenchmarkAuditState::Unspecified
                                as i32
                        },
                    };
                    Status::with_details(
                        tonic::Code::Internal,
                        format!("历史基准取数失败: {}", failure.message),
                        detail.encode_to_vec().into(),
                    )
                }
            })?;
        response_from_fetched(
            request_id,
            op,
            frozen.schema_name,
            frozen.schema_version,
            result,
        )
        .map(Response::new)
    }
}

fn response_from_fetched(
    request_id: String,
    op: Operation,
    schema: &str,
    schema_version: u32,
    result: delegate::Fetched,
) -> Result<QueryResponse, Status> {
    if result.provider.trim().is_empty()
        || result.source.trim().is_empty()
        || result.batch_id.trim().is_empty()
    {
        return Err(Status::internal(
            "delegate 证据身份缺失: provider/source/batch_id 必须完整",
        ));
    }
    match op {
        Operation::SemanticSearch => {
            crate::data_gateway::GeneralWebResearchProvider::from_wire_name(&result.provider)
                .ok_or_else(|| {
                    Status::internal(
                        "delegate 证据身份非法: SemanticSearch provider 未在冻结枚举中",
                    )
                })?;
            chrono::DateTime::parse_from_rfc3339(&result.observed_at).map_err(|_| {
                Status::internal(
                    "delegate 证据时间缺失或非法: SemanticSearch observed_at 必须是 RFC3339 instant",
                )
            })?;
        }
        _ => {
            let provider =
                crate::data_gateway::grpc_source::convert::parse_provider(&result.provider)
                    .map_err(|_| {
                        Status::internal("delegate 证据身份非法: provider 未在冻结枚举中")
                    })?;
            crate::data_gateway::parse_evidence_instant(
                "GrpcBridge",
                provider,
                "observed_at",
                &result.observed_at,
            )
            .map_err(|_| {
                Status::internal(
                    "delegate 证据时间缺失或非法: observed_at 必须是已冻结 Magic instant",
                )
            })?;
        }
    }
    Ok(QueryResponse {
        request_id,
        operation: op as i32,
        admission: AdmissionState::Admitted as i32,
        selected_provider: result.provider,
        batch_id: result.batch_id,
        complete: true,
        observed_at: result.observed_at,
        source_at: result.source_at,
        source: result.source,
        // 上游合同字段 10: 本地 server 无诊断 handler, 永远不产生诊断阻塞。
        diagnostic_blocker: String::new(),
        records: vec![CanonicalPayload {
            schema: schema.to_string(),
            schema_version,
            content_type: "application/json; charset=utf-8".to_string(),
            data: result.data,
        }],
    })
}

// 54 个 RPC 的统一实现 (全部委托 serve_query; 未实现 op 返回 UNIMPLEMENTED)。
// 宏展开必须包含 #[tonic::async_trait] 属性本身: 属性宏在宏展开前运行,
// 若属性在 impl 外面而方法由宏生成, async_trait 看不到生成的方法 → E0195
// (方法与 trait 声明签名不匹配)。客户端测试 MockData 已验证此模式可编译。
macro_rules! impl_market_data_service {
    ($( $method:ident => $op:ident ),* $(,)?) => {
        #[tonic::async_trait]
        impl MarketDataService for DataService {
            $(
                async fn $method(&self, req: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
                    self.serve_query(Operation::$op, req.into_inner()).await
                }
            )*
        }
    };
}

impl_market_data_service!(
    historical_bars => HistoricalBars,
    minute_data => MinuteData,
    realtime_quotes => RealtimeQuotes,
    money_flows => MoneyFlows,
    order_books => OrderBooks,
    auctions => Auctions,
    trades => Trades,
    security_metadata => SecurityMetadata,
    global_indices => GlobalIndices,
    foreign_exchange => ForeignExchange,
    economic_calendar => EconomicCalendar,
    futures_delivery => FuturesDelivery,
    reference_rates => ReferenceRates,
    official_fx_fixings => OfficialFxFixings,
    economic_series => EconomicSeries,
    company_filings => CompanyFilings,
    global_news => GlobalNews,
    announcements => Announcements,
    market_announcements => MarketAnnouncements,
    investor_questions => InvestorQuestions,
    policy_documents => PolicyDocuments,
    security_profiles => SecurityProfiles,
    financial_statements => FinancialStatements,
    market_statistics => MarketStatistics,
    technical_bars => TechnicalBars,
    corporate_actions => CorporateActions,
    board_directory => BoardDirectory,
    board_constituents => BoardConstituents,
    board_memberships => BoardMemberships,
    research_reports => ResearchReports,
    research_documents => ResearchDocuments,
    consensus => Consensus,
    target_prices => TargetPrices,
    semantic_search => SemanticSearch,
    fund_flow_series => FundFlowSeries,
    board_flows => BoardFlows,
    margin_data => MarginData,
    block_trades => BlockTrades,
    holder_counts => HolderCounts,
    lockup_events => LockupEvents,
    dividend_plans => DividendPlans,
    post_close_flows => PostCloseFlows,
    northbound_daily => NorthboundDaily,
    limit_pools => LimitPools,
    strong_stock_reasons => StrongStockReasons,
    dragon_tiger => DragonTiger,
    market_dragon_tiger => MarketDragonTiger,
    dragon_tiger_discovery => DragonTigerDiscovery,
    market_rankings => MarketRankings,
    market_breadth => MarketBreadth,
    popularity => Popularity,
    concept_hits => ConceptHits,
    option_data => OptionData,
    provider_top_n_rankings => ProviderTopNRankings,
    index_quotes => IndexQuotes,
    instrument_news => InstrumentNews,
    intraday_shape => IntradayShape,
    t0_evidence => T0Evidence,
    outcome_daily_bars => OutcomeDailyBars,
    upper_limit_pool_review => UpperLimitPoolReview,
    chain_batch => ChainBatch,
    benchmark_bars => BenchmarkBars,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn br242_semantic_search_handler_local_bridge_roundtrips_every_general_web_provider() {
        use crate::data_gateway::{
            GeneralWebResearchBatch, GeneralWebResearchProvider, ResearchUseScope,
        };

        for (wire_name, serde_name, source, expected_provider) in [
            (
                "Bocha",
                "bocha",
                "bocha-general-web",
                GeneralWebResearchProvider::Bocha,
            ),
            (
                "Tavily",
                "tavily",
                "tavily-general-web",
                GeneralWebResearchProvider::Tavily,
            ),
            (
                "SerpApi",
                "serp_api",
                "serpapi-general-web",
                GeneralWebResearchProvider::SerpApi,
            ),
        ] {
            let request_id = format!("TEST_CODE_BR242_{wire_name}");
            let batch_id = format!("TEST_CODE_BR242_BATCH_{wire_name}");
            let record = serde_json::json!({
                "title": "TEST_CODE semantic result",
                "snippet": "TEST_CODE source-backed context",
                "url": "https://example.com/TEST_CODE_BR242",
                "publisher": "TEST_CODE publisher",
                "published_at_raw": null,
                "published_at": null,
                "evidence": {
                    "provider": serde_name,
                    "observed_at": "2026-08-18T10:00:00+08:00",
                    "batch_id": batch_id,
                    "item_id": format!("TEST_CODE_BR242_ITEM_{wire_name}"),
                    "publication_quality": "missing",
                    "use_scope": "research_only"
                }
            });
            let response = response_from_fetched(
                request_id.clone(),
                Operation::SemanticSearch,
                "market.semantic_search",
                1,
                delegate::Fetched {
                    data: serde_json::to_vec(&vec![record]).expect("TEST_CODE record serializes"),
                    source_at: String::new(),
                    observed_at: "2026-08-18T10:00:00+08:00".to_string(),
                    provider: wire_name.to_string(),
                    source: source.to_string(),
                    batch_id: batch_id.clone(),
                },
            )
            .expect("BR-242 handler must admit every frozen general-web provider");
            let result = crate::grpc_client::envelope::parse_query_response(
                &request_id,
                Operation::SemanticSearch,
                response,
            )
            .expect("LocalBridge envelope must preserve the handler response");
            let batch = crate::data_gateway::grpc_source::convert::semantic_search(
                &result,
                "TEST_CODE semantic query",
                expected_provider,
                1,
            )
            .expect("LocalBridge converter must admit the exact requested provider");

            let (records, evidence) = match batch {
                GeneralWebResearchBatch::Available { records, evidence } => (records, evidence),
                GeneralWebResearchBatch::VerifiedEmpty(_) => {
                    panic!("TEST_CODE fixture contains one source-backed record")
                }
            };
            assert_eq!(records.len(), 1);
            assert_eq!(evidence.provider, expected_provider);
            assert_eq!(evidence.source, source);
            assert_eq!(evidence.batch_id, batch_id);
            assert_eq!(evidence.use_scope, ResearchUseScope::ResearchOnly);
        }
    }

    #[test]
    fn response_preserves_delegate_batch_identity() {
        let response = response_from_fetched(
            "TEST_CODE_REQUEST_1".to_string(),
            Operation::BoardConstituents,
            "board.constituents",
            1,
            delegate::Fetched {
                data: br#"[{"instrument_code":"TEST_CODE_600519"}]"#.to_vec(),
                source_at: "2026-08-17T09:20:00+08:00".to_string(),
                observed_at: "2026-08-17T09:20:01+08:00".to_string(),
                provider: "Tdx".to_string(),
                source: "tdx".to_string(),
                batch_id: "TEST_CODE_MEMBERSHIP_BATCH_1".to_string(),
            },
        )
        .expect("source-backed response");

        assert_eq!(response.selected_provider, "Tdx");
        assert_eq!(response.source, "tdx");
        assert_eq!(response.batch_id, "TEST_CODE_MEMBERSHIP_BATCH_1");
        assert_eq!(response.observed_at, "2026-08-17T09:20:01+08:00");
    }

    #[test]
    fn br238_response_accepts_validated_magic_observation_encodings() {
        for observed_at in ["1786970635.386291000", "unix-ms:1786970635026"] {
            let response = response_from_fetched(
                "TEST_CODE_REQUEST_MAGIC_TIME".to_string(),
                Operation::GlobalNews,
                "market.global_news",
                1,
                delegate::Fetched {
                    data: br#"[{"item_id":"TEST_CODE_GLOBAL_NEWS_001"}]"#.to_vec(),
                    source_at: "2026-08-17T20:43:49+08:00".to_string(),
                    observed_at: observed_at.to_string(),
                    provider: "Cailianpress".to_string(),
                    source: "cls-v1".to_string(),
                    batch_id: "TEST_CODE_GLOBAL_NEWS_BATCH_1".to_string(),
                },
            )
            .expect("validated Magic evidence time must survive the gRPC response boundary");
            assert_eq!(response.observed_at, observed_at);
        }
    }

    #[test]
    fn response_rejects_missing_delegate_batch_identity() {
        for (provider, source, batch_id) in [
            ("", "tdx", "TEST_CODE_MEMBERSHIP_BATCH_1"),
            ("Tdx", "", "TEST_CODE_MEMBERSHIP_BATCH_1"),
            ("Tdx", "tdx", ""),
        ] {
            let error = response_from_fetched(
                "TEST_CODE_REQUEST_2".to_string(),
                Operation::BoardConstituents,
                "board.constituents",
                1,
                delegate::Fetched {
                    data: br#"[{"instrument_code":"TEST_CODE_600519"}]"#.to_vec(),
                    source_at: "2026-08-17T09:20:00+08:00".to_string(),
                    observed_at: "2026-08-17T09:20:01+08:00".to_string(),
                    provider: provider.to_string(),
                    source: source.to_string(),
                    batch_id: batch_id.to_string(),
                },
            )
            .expect_err("missing delegate identity must fail closed");

            assert_eq!(error.code(), tonic::Code::Internal);
            assert!(error.message().contains("证据身份缺失"));
        }
    }

    #[test]
    fn response_rejects_missing_or_invalid_delegate_observation_time() {
        for observed_at in ["", "not-a-time"] {
            let error = response_from_fetched(
                "TEST_CODE_REQUEST_TIME".to_string(),
                Operation::BoardConstituents,
                "board.constituents",
                1,
                delegate::Fetched {
                    data: br#"[{"instrument_code":"TEST_CODE_600519"}]"#.to_vec(),
                    source_at: "2026-08-17T09:20:00+08:00".to_string(),
                    observed_at: observed_at.to_string(),
                    provider: "Tdx".to_string(),
                    source: "tdx".to_string(),
                    batch_id: "TEST_CODE_MEMBERSHIP_BATCH_1".to_string(),
                },
            )
            .expect_err("missing/invalid observed_at must fail closed");
            assert_eq!(error.code(), tonic::Code::Internal);
            assert!(error.message().contains("observed_at"));
        }
    }
}
