use super::{ClientAuthorization, ContractProfile, GrpcMarketClient};
use crate::data_gateway::grpc_source::{
    BoardContinuation, BoardTrailerMaterial, GrpcSource, RestoredDragonTigerRequest,
};
use crate::data_gateway::GatewayBatch;
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_server::{MarketDataService, MarketDataServiceServer},
    AdmissionState, CanonicalPayload, ErrorDetail, Operation, QueryRequest, QueryResponse,
};
use crate::grpc_client::retry::RetryDecision;
use crate::market_domain::{DragonTigerSide, Exchange, ProviderId};
use chrono::NaiveDate;
use prost::Message as _;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tonic::{Code, Request, Response, Status};
use zeroize::Zeroizing;

const DRAGON_TIGER_RECORDS: &str = r#"[
  {
    "exchange": "Shanghai",
    "code": "600001",
    "ranking_net_amount_yuan": 125000,
    "disclosures": [
      {
        "entry_id": "TEST_CODE_LHB_ENTRY_一",
        "trade_id": "TEST_CODE_LHB_TRADE_001",
        "reason": "连续三个交易日内涨幅偏离值累计达到20%  ",
        "buy_amount_yuan": 200000.5,
        "sell_amount_yuan": 75000.25,
        "net_amount_yuan": 125000.25,
        "turnover_rate_pct": 8.75,
        "seats": [
          {
            "side": "Buy",
            "rank": 1,
            "seat_name": "沪股通专用席位 α",
            "amount_yuan": 200000.5,
            "buy_amount_yuan": 200000.5,
            "sell_amount_yuan": null,
            "net_amount_yuan": 200000.5
          },
          {
            "side": "Sell",
            "rank": 2,
            "seat_name": "机构专用席位 β",
            "amount_yuan": 75000.25,
            "buy_amount_yuan": null,
            "sell_amount_yuan": 75000.25,
            "net_amount_yuan": -75000.25
          }
        ]
      }
    ]
  }
]"#;

const REQUEST_PAYLOAD_BYTES: &[u8] =
    br#"{"date":"2026-07-22","disclosure_limit":100,"stock_limit":5000}"#;

#[derive(Clone, Debug, Eq, PartialEq)]
struct DragonTigerLoopbackRequest {
    request_id: String,
    request_bytes: Vec<u8>,
    authorized: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct DragonTigerLoopbackObservation {
    requests: Vec<DragonTigerLoopbackRequest>,
    retry_error_details: Vec<Vec<u8>>,
    success_responses: Vec<Vec<u8>>,
    other_rpc_calls: usize,
}

#[derive(Default)]
struct DragonTigerLoopbackState {
    observation: DragonTigerLoopbackObservation,
    calls_by_request_id: HashMap<String, usize>,
}

#[derive(Clone)]
struct DragonTigerService {
    state: Arc<Mutex<DragonTigerLoopbackState>>,
}

impl DragonTigerService {
    fn success_response(request_id: String) -> QueryResponse {
        QueryResponse {
            request_id,
            operation: Operation::DragonTiger as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "Eastmoney".to_owned(),
            batch_id: "TEST_CODE_LHB_BATCH_20260722".to_owned(),
            complete: true,
            observed_at: "2026-07-22T15:31:02+08:00".to_owned(),
            source_at: "2026-07-22T15:30:00+08:00".to_owned(),
            source: "TEST_CODE_LHB_EASTMONEY_LOOPBACK".to_owned(),
            diagnostic_blocker: String::new(),
            records: vec![CanonicalPayload {
                schema: "market.dragon_tiger".to_owned(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_owned(),
                data: DRAGON_TIGER_RECORDS.as_bytes().to_vec(),
            }],
        }
    }
}

macro_rules! impl_dragon_tiger_service {
    ($($stub:ident),* $(,)?) => {
        #[tonic::async_trait]
        impl MarketDataService for DragonTigerService {
            async fn dragon_tiger(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                let authorized = request
                    .metadata()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer TEST_CODE_LHB_TOKEN");
                let inner = request.into_inner();
                let request_bytes = inner.encode_to_vec();
                let request_id = inner
                    .context
                    .as_ref()
                    .expect("TEST_CODE_LHB request context")
                    .request_id
                    .clone();
                let first_call_for_id = {
                    let mut state = self.state.lock().expect("TEST_CODE_LHB loopback state");
                    state.observation.requests.push(DragonTigerLoopbackRequest {
                        request_id: request_id.clone(),
                        request_bytes,
                        authorized,
                    });
                    let calls = state
                        .calls_by_request_id
                        .entry(request_id.clone())
                        .or_default();
                    let first = *calls == 0;
                    *calls += 1;
                    first
                };

                if first_call_for_id {
                    let detail = ErrorDetail {
                        request_id: request_id.clone(),
                        operation: Operation::DragonTiger as i32,
                        provider: "Eastmoney".to_owned(),
                        reason_code: "no_verified_batch".to_owned(),
                        retryable: true,
                        ..ErrorDetail::default()
                    };
                    let encoded = detail.encode_to_vec();
                    self.state
                        .lock()
                        .expect("TEST_CODE_LHB loopback retry detail")
                        .observation
                        .retry_error_details
                        .push(encoded.clone());
                    let mut status = Status::with_details(
                        Code::Unavailable,
                        "TEST_CODE_LHB synthetic retryable failure",
                        encoded.clone().into(),
                    );
                    status.metadata_mut().insert_bin(
                        "magic-error-detail-bin",
                        tonic::metadata::MetadataValue::from_bytes(&encoded),
                    );
                    return Err(status);
                }

                let response = Self::success_response(request_id);
                self.state
                    .lock()
                    .expect("TEST_CODE_LHB loopback response")
                    .observation
                    .success_responses
                    .push(response.encode_to_vec());
                Ok(Response::new(response))
            }

            $(
                async fn $stub(
                    &self,
                    _request: Request<QueryRequest>,
                ) -> Result<Response<QueryResponse>, Status> {
                    self.state
                        .lock()
                        .expect("TEST_CODE_LHB loopback other RPC")
                        .observation
                        .other_rpc_calls += 1;
                    Err(Status::unimplemented(stringify!($stub)))
                }
            )*
        }
    };
}

impl_dragon_tiger_service!(
    realtime_quotes,
    historical_bars,
    minute_data,
    money_flows,
    order_books,
    auctions,
    trades,
    security_metadata,
    global_indices,
    foreign_exchange,
    economic_calendar,
    futures_delivery,
    reference_rates,
    official_fx_fixings,
    economic_series,
    company_filings,
    global_news,
    announcements,
    market_announcements,
    investor_questions,
    policy_documents,
    security_profiles,
    financial_statements,
    market_statistics,
    technical_bars,
    corporate_actions,
    board_directory,
    board_constituents,
    board_memberships,
    research_reports,
    research_documents,
    consensus,
    target_prices,
    semantic_search,
    fund_flow_series,
    board_flows,
    margin_data,
    block_trades,
    holder_counts,
    lockup_events,
    dividend_plans,
    post_close_flows,
    northbound_daily,
    limit_pools,
    strong_stock_reasons,
    market_dragon_tiger,
    dragon_tiger_discovery,
    market_rankings,
    market_breadth,
    popularity,
    concept_hits,
    option_data,
    provider_top_n_rankings,
    index_quotes,
    instrument_news,
    intraday_shape,
    t0_evidence,
    outcome_daily_bars,
    upper_limit_pool_review,
    chain_batch,
    benchmark_bars,
);

struct DragonTigerLoopbackServer {
    state: Arc<Mutex<DragonTigerLoopbackState>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl DragonTigerLoopbackServer {
    fn snapshot(&self) -> DragonTigerLoopbackObservation {
        self.state
            .lock()
            .expect("TEST_CODE_LHB loopback snapshot")
            .observation
            .clone()
    }

    async fn finish(mut self) -> DragonTigerLoopbackObservation {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                Ok(result) => result.expect("TEST_CODE_LHB loopback server task"),
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    panic!("TEST_CODE_LHB loopback shutdown timeout");
                }
            }
        }
        self.snapshot()
    }
}

impl Drop for DragonTigerLoopbackServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn spawn_dragon_tiger_loopback() -> (GrpcMarketClient, DragonTigerLoopbackServer) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("TEST_CODE_LHB bind loopback");
        let address = listener
            .local_addr()
            .expect("TEST_CODE_LHB loopback address");
        let state = Arc::new(Mutex::new(DragonTigerLoopbackState::default()));
        let service = DragonTigerService {
            state: Arc::clone(&state),
        };
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(MarketDataServiceServer::new(service))
                .serve_with_incoming_shutdown(
                    tokio_stream::wrappers::TcpListenerStream::new(listener),
                    async move {
                        let _ = shutdown_rx.await;
                    },
                )
                .await
                .expect("TEST_CODE_LHB loopback server");
        });
        let server = DragonTigerLoopbackServer {
            state,
            shutdown: Some(shutdown),
            task: Some(task),
        };
        let endpoint = format!("http://{address}");
        let channel = tonic::transport::Channel::from_shared(endpoint)
            .expect("TEST_CODE_LHB loopback endpoint")
            .timeout(Duration::from_secs(35))
            .connect()
            .await
            .expect("TEST_CODE_LHB loopback connect");
        let client = GrpcMarketClient::from_channel(
            channel,
            ContractProfile::LocalBridgeV1,
            ClientAuthorization::InstanceBearer(Zeroizing::new("TEST_CODE_LHB_TOKEN".to_owned())),
            None,
        );
        (client, server)
    })
    .await
    .expect("TEST_CODE_LHB listener/connect timeout")
}

fn assert_complete_dragon_tiger_batch(
    batch: &GatewayBatch<crate::data_gateway::DragonTigerStockReview>,
) {
    assert!(!batch.is_verified_empty());
    assert_eq!(batch.records().len(), 1);
    let stock = &batch.records()[0];
    assert_eq!(stock.exchange, Exchange::Shanghai);
    assert_eq!(stock.code, "600001");
    assert_eq!(stock.ranking_net_amount_yuan, 125000.0);
    assert_eq!(stock.disclosures.len(), 1);

    let disclosure = &stock.disclosures[0];
    assert_eq!(disclosure.entry_id, "TEST_CODE_LHB_ENTRY_一");
    assert_eq!(disclosure.trade_id, "TEST_CODE_LHB_TRADE_001");
    assert_eq!(
        disclosure.reason.as_deref(),
        Some("连续三个交易日内涨幅偏离值累计达到20%  ")
    );
    assert_eq!(disclosure.buy_amount_yuan, Some(200000.5));
    assert_eq!(disclosure.sell_amount_yuan, Some(75000.25));
    assert_eq!(disclosure.net_amount_yuan, Some(125000.25));
    assert_eq!(disclosure.turnover_rate_pct, Some(8.75));
    assert_eq!(disclosure.seats.len(), 2);

    let buy = &disclosure.seats[0];
    assert_eq!(buy.side, DragonTigerSide::Buy);
    assert_eq!(buy.rank, 1);
    assert_eq!(buy.seat_name, "沪股通专用席位 α");
    assert_eq!(buy.amount_yuan, 200000.5);
    assert_eq!(buy.buy_amount_yuan, Some(200000.5));
    assert_eq!(buy.sell_amount_yuan, None);
    assert_eq!(buy.net_amount_yuan, Some(200000.5));

    let sell = &disclosure.seats[1];
    assert_eq!(sell.side, DragonTigerSide::Sell);
    assert_eq!(sell.rank, 2);
    assert_eq!(sell.seat_name, "机构专用席位 β");
    assert_eq!(sell.amount_yuan, 75000.25);
    assert_eq!(sell.buy_amount_yuan, None);
    assert_eq!(sell.sell_amount_yuan, Some(75000.25));
    assert_eq!(sell.net_amount_yuan, Some(-75000.25));

    let evidence = batch.evidence();
    assert_eq!(evidence.provider, ProviderId::Eastmoney);
    assert_eq!(evidence.source, "TEST_CODE_LHB_EASTMONEY_LOOPBACK");
    assert_eq!(
        evidence.source_at.as_deref(),
        Some("2026-07-22T15:30:00+08:00")
    );
    assert_eq!(evidence.observed_at, "2026-07-22T15:31:02+08:00");
    assert_eq!(evidence.batch_id, "TEST_CODE_LHB_BATCH_20260722");
}

#[tokio::test]
async fn dragon_tiger_attempt_resume_preserves_request_and_original_response() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let date = NaiveDate::from_ymd_opt(2026, 7, 22).expect("TEST_CODE_LHB date");
        let disclosure_limit = 100;
        let stock_limit = 5_000;
        let (client, server) = spawn_dragon_tiger_loopback().await;
        let source = GrpcSource::from_board_loopback_test_client(client.clone());
        let resumed_source = GrpcSource::from_board_loopback_test_client(client.clone());
        let legacy_source = GrpcSource::from_board_loopback_test_client(client);

        let mut first_session = source
            .dragon_tiger_query_session(date, disclosure_limit, stock_limit)
            .await
            .expect("TEST_CODE_LHB new attempt session");
        let first_authorized = first_session
            .authorize_next()
            .expect("TEST_CODE_LHB authorize first attempt");
        assert!(server.snapshot().requests.is_empty());

        let request_bytes = first_authorized.request_bytes();
        let request_id = first_authorized.request_id().to_owned();
        let profile = first_authorized.profile();
        let acquisition_authority = first_authorized.acquisition_authority().map(str::to_owned);
        let retry_policy = first_authorized.retry_policy();
        assert!(!request_id.is_empty());
        assert_eq!(profile, "LocalBridgeV1");
        assert_eq!(acquisition_authority, None);
        assert_eq!(retry_policy, (4, 1_000, 60_000, 200));
        assert_eq!(first_authorized.attempt_ordinal(), 1);

        let request = QueryRequest::decode(request_bytes.as_slice())
            .expect("TEST_CODE_LHB decode preserved request");
        let context = request
            .context
            .as_ref()
            .expect("TEST_CODE_LHB preserved request context");
        assert_eq!(context.protocol_version, 1);
        assert_eq!(context.request_id, request_id);
        assert!(request.preferred_provider.is_empty());
        assert!(!request.allow_unadmitted);
        let payload = request
            .payload
            .as_ref()
            .expect("TEST_CODE_LHB preserved request payload");
        assert_eq!(payload.schema, "market.dragon_tiger");
        assert_eq!(payload.schema_version, 1);
        assert_eq!(payload.content_type, "application/json; charset=utf-8");
        assert_eq!(payload.data, REQUEST_PAYLOAD_BYTES);

        let first_completion = first_authorized.execute().await;
        let first_observation = server.snapshot();
        assert_eq!(first_observation.requests.len(), 1);
        assert_eq!(first_observation.other_rpc_calls, 0);
        assert_eq!(first_observation.requests[0].request_id, request_id);
        assert_eq!(first_observation.requests[0].request_bytes, request_bytes);
        assert!(first_observation.requests[0].authorized);
        assert_eq!(first_completion.response_bytes, None);
        assert_eq!(first_completion.status_code, Some(14));
        assert_eq!(
            first_completion.status_details.as_deref(),
            Some(first_observation.retry_error_details[0].as_slice())
        );
        assert_eq!(
            first_completion.status_error_detail_trailer,
            BoardTrailerMaterial::Bytes(first_observation.retry_error_details[0].clone())
        );
        assert!(first_completion.processed.is_err());
        assert_eq!(first_completion.retry_decision, RetryDecision::RetryBackoff);
        assert_eq!(
            first_completion.continuation,
            BoardContinuation::Retry { backoff_ms: 1_000 }
        );
        assert_eq!(server.snapshot().requests.len(), 1);
        drop(first_session);

        tokio::time::sleep(Duration::from_millis(1_000)).await;
        let restored = RestoredDragonTigerRequest::new(
            date,
            disclosure_limit,
            stock_limit,
            request_id.clone(),
            request,
            ContractProfile::LocalBridgeV1,
            acquisition_authority.clone(),
            retry_policy,
            2,
        );
        let mut resumed_session = resumed_source
            .resume_dragon_tiger_query_session(restored)
            .await
            .expect("TEST_CODE_LHB resume attempt session");
        let second_authorized = resumed_session
            .authorize_next()
            .expect("TEST_CODE_LHB authorize resumed attempt");
        assert_eq!(second_authorized.request_bytes(), request_bytes);
        assert_eq!(second_authorized.request_id(), request_id);
        assert_eq!(second_authorized.profile(), profile);
        assert_eq!(
            second_authorized.acquisition_authority(),
            acquisition_authority.as_deref()
        );
        assert_eq!(second_authorized.retry_policy(), retry_policy);
        assert_eq!(second_authorized.attempt_ordinal(), 2);

        let second_completion = second_authorized.execute().await;
        let raw_response_bytes = second_completion
            .response_bytes
            .clone()
            .expect("TEST_CODE_LHB original response bytes");
        let raw_response = QueryResponse::decode(raw_response_bytes.as_slice())
            .expect("TEST_CODE_LHB decode original response");
        let second_observation = server.snapshot();
        assert_eq!(second_observation.requests.len(), 2);
        assert_eq!(second_observation.other_rpc_calls, 0);
        assert_eq!(second_observation.requests[1].request_id, request_id);
        assert_eq!(second_observation.requests[1].request_bytes, request_bytes);
        assert!(second_observation.requests[1].authorized);
        assert_eq!(
            second_observation.success_responses,
            [raw_response_bytes.clone()]
        );
        assert_eq!(raw_response.records.len(), 1);
        assert_eq!(
            raw_response.records[0].data,
            DRAGON_TIGER_RECORDS.as_bytes()
        );
        assert_eq!(second_completion.status_code, None);
        assert_eq!(second_completion.status_details, None);
        assert_eq!(
            second_completion.status_error_detail_trailer,
            BoardTrailerMaterial::Absent
        );
        assert!(second_completion.processed.is_ok());
        assert_eq!(second_completion.retry_decision, RetryDecision::NoRetry);
        assert_eq!(second_completion.continuation, BoardContinuation::Terminal);
        let live_batch = GrpcSource::dragon_tiger_completion(second_completion)
            .expect("TEST_CODE_LHB live completion conversion");
        assert_complete_dragon_tiger_batch(&live_batch);

        drop(resumed_session);
        let restored_batch = GrpcSource::restore_dragon_tiger_response(
            ContractProfile::LocalBridgeV1,
            None,
            &request_id,
            raw_response.clone(),
        )
        .expect("TEST_CODE_LHB pure response restoration");
        assert_complete_dragon_tiger_batch(&restored_batch);
        assert_eq!(
            raw_response.records[0].data,
            DRAGON_TIGER_RECORDS.as_bytes()
        );
        assert_eq!(server.snapshot().requests.len(), 2);
        assert_eq!(server.snapshot().other_rpc_calls, 0);

        let legacy_batch = legacy_source
            .dragon_tiger_async(date, disclosure_limit, stock_limit)
            .await
            .expect("TEST_CODE_LHB legacy public query");
        assert_complete_dragon_tiger_batch(&legacy_batch);
        let final_observation = server.finish().await;
        assert_eq!(final_observation.requests.len(), 4);
        assert_eq!(final_observation.retry_error_details.len(), 2);
        assert_eq!(final_observation.success_responses.len(), 2);
        assert_eq!(final_observation.other_rpc_calls, 0);
        assert_ne!(final_observation.requests[2].request_id, request_id);
        assert_eq!(
            final_observation.requests[2].request_id,
            final_observation.requests[3].request_id
        );
        assert_eq!(
            final_observation.requests[2].request_bytes,
            final_observation.requests[3].request_bytes
        );
        assert!(final_observation.requests[2].authorized);
        assert!(final_observation.requests[3].authorized);
        let legacy_request =
            QueryRequest::decode(final_observation.requests[2].request_bytes.as_slice())
                .expect("TEST_CODE_LHB decode legacy request");
        assert_eq!(
            legacy_request
                .payload
                .expect("TEST_CODE_LHB legacy payload")
                .data,
            REQUEST_PAYLOAD_BYTES
        );
    })
    .await
    .expect("TEST_CODE_LHB attempt test timeout");
}
