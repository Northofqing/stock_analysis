//! BR-162 evidence-preserving R-04 whole-market dragon-tiger Gateway.

use super::review::{acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result};
use super::{BatchEvidence, GatewayBatch, GatewayError};
use chrono::NaiveDate;
#[cfg(feature = "magic-gateway")]
use magic_eastmoney_rs::{EastmoneyClient, EastmoneyError};
use crate::magic_compat::{AssetClass, Exchange, IsoDate, PositiveU32, ProviderId};
#[cfg(feature = "magic-gateway")]
use magic_market_core::{DragonTigerDisclosure, DragonTigerSide, MarketDragonTigerData, MarketDragonTigerRequest};
use magic_market_router::{
    AcceptancePolicy, AttemptStatus, FailureKind, MarketDragonTigerRouter, RouterError, SourceFn,
};
use std::collections::{HashMap, HashSet};

const CAPABILITY: &str = "R-04";

/// One exact source seat from a complete buy-five/sell-five disclosure.
#[derive(Debug, Clone, PartialEq)]
pub struct DragonTigerSeatReview {
    pub side: DragonTigerSide,
    pub rank: u32,
    pub seat_name: String,
    pub amount_yuan: f64,
    pub buy_amount_yuan: Option<f64>,
    pub sell_amount_yuan: Option<f64>,
    pub net_amount_yuan: Option<f64>,
}

/// One source `TRADE_ID` disclosure. Distinct reasons remain distinct records.
#[derive(Debug, Clone, PartialEq)]
pub struct DragonTigerSourceDisclosure {
    pub entry_id: String,
    pub trade_id: String,
    pub reason: Option<String>,
    pub buy_amount_yuan: Option<f64>,
    pub sell_amount_yuan: Option<f64>,
    pub net_amount_yuan: Option<f64>,
    pub turnover_rate_pct: Option<f64>,
    pub seats: Vec<DragonTigerSeatReview>,
}

/// Report-ready aggregation for one stock, retaining all source disclosures.
#[derive(Debug, Clone, PartialEq)]
pub struct DragonTigerStockReview {
    pub exchange: Exchange,
    pub code: String,
    pub ranking_net_amount_yuan: f64,
    pub disclosures: Vec<DragonTigerSourceDisclosure>,
}

/// Evidence-preserving R-04 acquisition seam.
#[derive(Debug, Clone, Copy, Default)]
pub struct DragonTigerGateway;

impl DragonTigerGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn market_review(
        &self,
        trading_date: NaiveDate,
        disclosure_limit: u32,
        stock_limit: usize,
    ) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
        let request_hash = acquisition_request_hash(
            CAPABILITY,
            &format!("{trading_date}:{disclosure_limit}:{stock_limit}"),
        );
        // P4 M3: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("DragonTiger") {
            Ok(Some(bridge)) => {
                let result = bridge
                    .dragon_tiger_async(trading_date, disclosure_limit, stock_limit)
                    .await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Eastmoney);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
            Err(error) => {
                return audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Eastmoney,
                    &request_hash,
                    Err(error),
                );
            }
        }
        let worker_request_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = build_request(trading_date, disclosure_limit).and_then(|request| {
                if stock_limit == 0 {
                    return Err(GatewayError::invalid_request(
                        CAPABILITY,
                        "stock limit must be greater than zero",
                    ));
                }
                fetch_market_review(request, trading_date, stock_limit)
            });
            audit_gateway_result(
                CAPABILITY,
                ProviderId::Eastmoney,
                &worker_request_hash,
                result,
            )
        })
        .await;

        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    CAPABILITY,
                    ProviderId::Eastmoney,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }
}

fn build_request(
    trading_date: NaiveDate,
    disclosure_limit: u32,
) -> Result<MarketDragonTigerRequest, GatewayError> {
    let trading_date = IsoDate::new(trading_date.format("%Y-%m-%d").to_string())
        .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    let limit = PositiveU32::new(disclosure_limit)
        .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    MarketDragonTigerRequest::new(trading_date, limit)
        .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))
}

fn fetch_market_review(
    request: MarketDragonTigerRequest,
    expected_date: NaiveDate,
    stock_limit: usize,
) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
    let provider = EastmoneyClient::new().map_err(eastmoney_gateway_error)?;
    let real_batch = provider
        .market_dragon_tiger(&request)
        .map_err(eastmoney_gateway_error)?;
    let evidence = BatchEvidence::from_provenance(ProviderId::Eastmoney, real_batch.provenance())?;
    if real_batch.records().is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }

    // Route the already acquired real batch so canonical Router admission is
    // applied without issuing a second provider request.
    let prefetched = real_batch.clone();
    let source = SourceFn::new(
        ProviderId::Eastmoney,
        move |_request: &MarketDragonTigerRequest| Ok(prefetched.clone()),
    );
    let mut router = MarketDragonTigerRouter::new(
        AcceptancePolicy::new()
            .with_require_complete(true)
            .with_require_source_at(true),
    );
    router
        .register(source)
        .map_err(|error| router_gateway_error(Some(ProviderId::Eastmoney), error))?;
    let routed = router
        .route(&request)
        .map_err(|error| router_gateway_error(Some(ProviderId::Eastmoney), error))?;
    let batch = routed.into_batch();
    let routed_evidence =
        BatchEvidence::from_provenance(ProviderId::Eastmoney, batch.provenance())?;
    if routed_evidence != evidence {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "routed dragon-tiger provenance changed after acquisition",
        ));
    }

    let records = aggregate_disclosures(batch.records(), expected_date, &evidence, stock_limit)?;
    if records.is_empty() {
        Ok(GatewayBatch::VerifiedEmpty(evidence))
    } else {
        Ok(GatewayBatch::Available { records, evidence })
    }
}

fn aggregate_disclosures(
    records: &[DragonTigerDisclosure],
    expected_date: NaiveDate,
    evidence: &BatchEvidence,
    stock_limit: usize,
) -> Result<Vec<DragonTigerStockReview>, GatewayError> {
    if stock_limit == 0 {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            "stock limit must be greater than zero",
        ));
    }

    let expected_date = expected_date.format("%Y-%m-%d").to_string();
    let mut stocks = Vec::<DragonTigerStockReview>::new();
    let mut positions = HashMap::<(u8, String), usize>::new();
    let mut entry_ids = HashSet::with_capacity(records.len());

    for record in records {
        let entry = record.entry();
        let instrument = entry.instrument();
        validate_instrument(instrument.asset_class(), instrument.code())?;
        if entry.trading_date().as_str() != expected_date {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Eastmoney),
                format!(
                    "dragon-tiger entry date {} differs from requested {}",
                    entry.trading_date().as_str(),
                    expected_date
                ),
            ));
        }
        validate_record_evidence(entry.evidence(), evidence)?;

        let entry_id = entry.entry_id().as_str().to_string();
        if !entry_ids.insert(entry_id.clone()) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Eastmoney),
                format!("duplicate dragon-tiger entry ID {entry_id}"),
            ));
        }
        let trade_id = source_trade_id(&entry_id)?;
        let mut seats = record
            .seats()
            .iter()
            .map(|seat| {
                validate_record_evidence(seat.evidence(), evidence)?;
                if seat.entry_id() != entry.entry_id()
                    || seat.instrument() != instrument
                    || seat.trading_date() != entry.trading_date()
                {
                    return Err(GatewayError::invalid_evidence(
                        CAPABILITY,
                        Some(ProviderId::Eastmoney),
                        format!("seat identity differs from entry {entry_id}"),
                    ));
                }
                Ok(DragonTigerSeatReview {
                    side: seat.side(),
                    rank: seat.rank().get(),
                    seat_name: seat.seat_name().as_str().to_string(),
                    amount_yuan: seat.amount().get(),
                    buy_amount_yuan: seat.buy_amount().map(|value| value.get()),
                    sell_amount_yuan: seat.sell_amount().map(|value| value.get()),
                    net_amount_yuan: seat.net_amount().map(|value| value.get()),
                })
            })
            .collect::<Result<Vec<_>, GatewayError>>()?;
        validate_and_sort_seats(&mut seats, &entry_id)?;

        let disclosure = DragonTigerSourceDisclosure {
            entry_id,
            trade_id,
            reason: entry.reason().map(|value| value.as_str().to_string()),
            buy_amount_yuan: entry.buy_amount().map(|value| value.get()),
            sell_amount_yuan: entry.sell_amount().map(|value| value.get()),
            net_amount_yuan: entry.net_amount().map(|value| value.get()),
            turnover_rate_pct: entry.turnover_rate().map(|value| value.get()),
            seats,
        };
        let key = (
            exchange_order(instrument.exchange()),
            instrument.code().to_string(),
        );
        let position = if let Some(position) = positions.get(&key).copied() {
            position
        } else {
            let position = stocks.len();
            positions.insert(key, position);
            stocks.push(DragonTigerStockReview {
                exchange: instrument.exchange(),
                code: instrument.code().to_string(),
                ranking_net_amount_yuan: f64::NEG_INFINITY,
                disclosures: Vec::new(),
            });
            position
        };
        if let Some(net_amount) = disclosure
            .net_amount_yuan
            .filter(|net_amount| *net_amount > 0.0)
        {
            stocks[position].ranking_net_amount_yuan =
                stocks[position].ranking_net_amount_yuan.max(net_amount);
        }
        stocks[position].disclosures.push(disclosure);
    }

    stocks.retain(|stock| stock.ranking_net_amount_yuan.is_finite());
    for stock in &mut stocks {
        stock.disclosures.sort_by(disclosure_order);
    }
    stocks.sort_by(|left, right| {
        right
            .ranking_net_amount_yuan
            .total_cmp(&left.ranking_net_amount_yuan)
            .then_with(|| exchange_order(left.exchange).cmp(&exchange_order(right.exchange)))
            .then_with(|| left.code.cmp(&right.code))
    });
    stocks.truncate(stock_limit);
    Ok(stocks)
}

fn validate_instrument(asset_class: AssetClass, code: &str) -> Result<(), GatewayError> {
    let source_code = code.strip_prefix("TEST_CODE_").unwrap_or(code);
    if asset_class != AssetClass::Equity
        || source_code.len() != 6
        || !source_code.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!("invalid A-share dragon-tiger instrument {code:?}"),
        ));
    }
    Ok(())
}

fn validate_record_evidence(
    record: &crate::magic_compat::SourceEvidence,
    batch: &BatchEvidence,
) -> Result<(), GatewayError> {
    if record.provider() != ProviderId::Eastmoney
        || batch.provider != ProviderId::Eastmoney
        || record.source_at() != batch.source_at.as_deref()
        || record.observed_at() != batch.observed_at
        || record.batch_id() != batch.batch_id
    {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "dragon-tiger record evidence differs from batch provenance",
        ));
    }
    Ok(())
}

fn source_trade_id(entry_id: &str) -> Result<String, GatewayError> {
    let (_, trade_id) = entry_id.rsplit_once(':').ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!("dragon-tiger entry ID {entry_id:?} has no TRADE_ID segment"),
        )
    })?;
    if trade_id.is_empty() || !trade_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!("dragon-tiger entry ID {entry_id:?} has an invalid TRADE_ID segment"),
        ));
    }
    Ok(trade_id.to_string())
}

fn validate_and_sort_seats(
    seats: &mut [DragonTigerSeatReview],
    entry_id: &str,
) -> Result<(), GatewayError> {
    if seats.len() != 10 {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!(
                "dragon-tiger entry {entry_id} must have exactly 10 seats, got {}",
                seats.len()
            ),
        ));
    }
    let mut identities = HashSet::with_capacity(seats.len());
    for seat in seats.iter() {
        if !(1..=5).contains(&seat.rank) || !identities.insert((seat.side, seat.rank)) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Eastmoney),
                format!("dragon-tiger entry {entry_id} has invalid/duplicate seat rank"),
            ));
        }
    }
    for side in [DragonTigerSide::Buy, DragonTigerSide::Sell] {
        for rank in 1..=5 {
            if !identities.contains(&(side, rank)) {
                return Err(GatewayError::invalid_evidence(
                    CAPABILITY,
                    Some(ProviderId::Eastmoney),
                    format!("dragon-tiger entry {entry_id} has incomplete five-seat side"),
                ));
            }
        }
    }
    seats.sort_by(|left, right| {
        seat_side_order(left.side)
            .cmp(&seat_side_order(right.side))
            .then_with(|| left.rank.cmp(&right.rank))
    });
    Ok(())
}

fn disclosure_order(
    left: &DragonTigerSourceDisclosure,
    right: &DragonTigerSourceDisclosure,
) -> std::cmp::Ordering {
    match (left.net_amount_yuan, right.net_amount_yuan) {
        (Some(left), Some(right)) => right.total_cmp(&left),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
    .then_with(|| left.entry_id.cmp(&right.entry_id))
}

const fn exchange_order(exchange: Exchange) -> u8 {
    match exchange {
        Exchange::Shanghai => 0,
        Exchange::Shenzhen => 1,
        Exchange::Beijing => 2,
    }
}

const fn seat_side_order(side: DragonTigerSide) -> u8 {
    match side {
        DragonTigerSide::Buy => 0,
        DragonTigerSide::Sell => 1,
    }
}

fn eastmoney_gateway_error(error: EastmoneyError) -> GatewayError {
    let reason_code = error.category();
    let message = error.to_string();
    match error {
        EastmoneyError::InvalidRequest(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "invalid_request",
            reason_code,
            false,
            message,
        ),
        EastmoneyError::Transport(_) | EastmoneyError::ResponseTooLarge { .. } => {
            GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Eastmoney),
                "unavailable",
                reason_code,
                true,
                message,
            )
        }
        EastmoneyError::Unsupported(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unsupported",
            reason_code,
            false,
            message,
        ),
        EastmoneyError::VerifiedEmpty(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "verified_empty",
            reason_code,
            false,
            message,
        ),
        EastmoneyError::Decode(_) | EastmoneyError::Protocol(_) | EastmoneyError::Core(_) => {
            GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Eastmoney),
                "partial",
                reason_code,
                false,
                message,
            )
        }
    }
}

fn router_gateway_error(provider: Option<ProviderId>, error: RouterError) -> GatewayError {
    let terminal_kind = error
        .attempts()
        .iter()
        .rev()
        .find_map(|attempt| match attempt.status() {
            AttemptStatus::Failed { kind, .. } | AttemptStatus::Rejected { kind, .. } => {
                Some(*kind)
            }
            AttemptStatus::Selected => None,
        });
    let (audit_outcome, reason_code, retryable) = match terminal_kind {
        Some(FailureKind::InvalidRequest) | None => {
            ("invalid_request", "router_invalid_request", false)
        }
        Some(FailureKind::Unsupported) => ("unsupported", "router_unsupported", false),
        Some(
            FailureKind::Transport
            | FailureKind::Timeout
            | FailureKind::RateLimited
            | FailureKind::Provider
            | FailureKind::NoData,
        ) => ("unavailable", "router_unavailable", true),
        Some(FailureKind::Protocol | FailureKind::Quality | FailureKind::Evidence) => {
            ("partial", "router_batch_rejected", false)
        }
    };
    GatewayError::classified(
        CAPABILITY,
        provider,
        audit_outcome,
        reason_code,
        retryable,
        error.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::magic_compat::{InstrumentId, Money, NonEmptyText, SourceEvidence};
#[cfg(feature = "magic-gateway")]
use magic_market_core::{DragonTigerEntry, DragonTigerSeat};

    fn disclosure(
        code: &str,
        trade_id: &str,
        reason: &str,
        buy: f64,
        sell: f64,
    ) -> DragonTigerDisclosure {
        let date = IsoDate::new("2099-01-02").expect("date");
        let instrument =
            InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).expect("instrument");
        let evidence = SourceEvidence::new(
            ProviderId::Eastmoney,
            "2099-01-02T21:00:00+08:00",
            "TEST_CODE_batch_r04",
        )
        .expect("evidence")
        .with_source_at("2099-01-02")
        .expect("source at");
        let entry_id =
            NonEmptyText::new(format!("{code}:2099-01-02:{trade_id}")).expect("entry ID");
        let entry = DragonTigerEntry::new(
            entry_id.clone(),
            instrument.clone(),
            date.clone(),
            Some(NonEmptyText::new(reason).expect("reason")),
            Some(Money::new(buy).expect("buy")),
            Some(Money::new(sell).expect("sell")),
            Some(Money::new(buy - sell).expect("net")),
            None,
            evidence.clone(),
        )
        .expect("entry");
        let mut seats = Vec::with_capacity(10);
        for side in [DragonTigerSide::Buy, DragonTigerSide::Sell] {
            for rank in 1..=5 {
                let amount = f64::from(rank) * 1_000_000.0;
                let (seat_buy, seat_sell) = match side {
                    DragonTigerSide::Buy => (Some(Money::new(amount).unwrap()), None),
                    DragonTigerSide::Sell => (None, Some(Money::new(amount).unwrap())),
                };
                seats.push(
                    DragonTigerSeat::new(
                        entry_id.clone(),
                        instrument.clone(),
                        date.clone(),
                        side,
                        PositiveU32::new(rank).unwrap(),
                        NonEmptyText::new(format!("TEST_CODE_{side:?}_{rank}")).unwrap(),
                        Money::new(amount).unwrap(),
                        seat_buy,
                        seat_sell,
                        None,
                        evidence.clone(),
                    )
                    .expect("seat"),
                );
            }
        }
        DragonTigerDisclosure::new(entry, seats).expect("disclosure")
    }

    fn evidence() -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: "TEST_CODE_eastmoney-market-dragon-tiger".to_string(),
            source_at: Some("2099-01-02".to_string()),
            observed_at: "2099-01-02T21:00:00+08:00".to_string(),
            batch_id: "TEST_CODE_batch_r04".to_string(),
        }
    }

    #[test]
    fn br162_groups_by_stock_without_summing_distinct_trade_ids() {
        let rows = vec![
            disclosure(
                "TEST_CODE_600396",
                "100380472",
                "TEST_CODE_reason_a",
                500_000_000.0,
                120_000_000.0,
            ),
            disclosure(
                "TEST_CODE_002396",
                "100379754",
                "TEST_CODE_reason_b",
                400_000_000.0,
                100_000_000.0,
            ),
            disclosure(
                "TEST_CODE_600396",
                "100380465",
                "TEST_CODE_reason_c",
                350_000_000.0,
                70_000_000.0,
            ),
        ];

        let stocks = aggregate_disclosures(
            &rows,
            NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
            &evidence(),
            5,
        )
        .expect("aggregate");

        assert_eq!(stocks.len(), 2);
        assert_eq!(stocks[0].code, "TEST_CODE_600396");
        assert_eq!(stocks[0].ranking_net_amount_yuan, 380_000_000.0);
        assert_eq!(stocks[0].disclosures.len(), 2);
        assert_eq!(stocks[0].disclosures[0].trade_id, "100380472");
        assert_eq!(stocks[0].disclosures[1].trade_id, "100380465");
        assert!(stocks[0]
            .disclosures
            .iter()
            .all(|disclosure| disclosure.seats.len() == 10));
    }

    #[test]
    fn br162_filters_nonpositive_stocks_then_limits_after_grouping() {
        let rows = vec![
            disclosure("TEST_CODE_600001", "1001", "TEST_CODE_negative", 10.0, 20.0),
            disclosure(
                "TEST_CODE_600002",
                "1002",
                "TEST_CODE_positive_low",
                30.0,
                20.0,
            ),
            disclosure(
                "TEST_CODE_600003",
                "1003",
                "TEST_CODE_positive_high",
                50.0,
                20.0,
            ),
        ];

        let stocks = aggregate_disclosures(
            &rows,
            NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
            &evidence(),
            1,
        )
        .expect("aggregate");

        assert_eq!(stocks.len(), 1);
        assert_eq!(stocks[0].code, "TEST_CODE_600003");
        assert_eq!(stocks[0].ranking_net_amount_yuan, 30.0);
    }

    fn seat(side: DragonTigerSide, rank: u32) -> DragonTigerSeatReview {
        DragonTigerSeatReview {
            side,
            rank,
            seat_name: format!("TEST_CODE_{side:?}_{rank}"),
            amount_yuan: f64::from(rank),
            buy_amount_yuan: (side == DragonTigerSide::Buy).then_some(f64::from(rank)),
            sell_amount_yuan: (side == DragonTigerSide::Sell).then_some(f64::from(rank)),
            net_amount_yuan: None,
        }
    }

    #[test]
    fn br162_request_identity_and_trade_id_validation_are_explicit() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        assert!(build_request(date, 0).is_err());
        assert!(build_request(date, 1).is_ok());
        assert!(validate_instrument(AssetClass::Index, "TEST_CODE_600001").is_err());
        assert!(validate_instrument(AssetClass::Equity, "TEST_CODE_bad").is_err());
        assert!(validate_instrument(AssetClass::Equity, "TEST_CODE_600001").is_ok());
        assert!(source_trade_id("TEST_CODE_no_separator").is_err());
        assert!(source_trade_id("TEST_CODE:").is_err());
        assert!(source_trade_id("TEST_CODE:not-digits").is_err());
        assert_eq!(source_trade_id("TEST_CODE:12345").unwrap(), "12345");
        assert!(aggregate_disclosures(&[], date, &evidence(), 0).is_err());
    }

    #[test]
    fn br162_seat_contract_requires_exact_buy_and_sell_five() {
        let mut valid = Vec::new();
        for side in [DragonTigerSide::Sell, DragonTigerSide::Buy] {
            for rank in (1..=5).rev() {
                valid.push(seat(side, rank));
            }
        }
        validate_and_sort_seats(&mut valid, "TEST_CODE_entry").unwrap();
        assert_eq!(valid.first().unwrap().side, DragonTigerSide::Buy);
        assert_eq!(valid.first().unwrap().rank, 1);
        assert_eq!(valid.last().unwrap().side, DragonTigerSide::Sell);
        assert_eq!(valid.last().unwrap().rank, 5);

        assert!(validate_and_sort_seats(&mut valid[..9], "TEST_CODE_short").is_err());
        let mut duplicate = valid.clone();
        duplicate[1].rank = 1;
        assert!(validate_and_sort_seats(&mut duplicate, "TEST_CODE_duplicate").is_err());
        let mut out_of_range = valid.clone();
        out_of_range[0].rank = 6;
        assert!(validate_and_sort_seats(&mut out_of_range, "TEST_CODE_range").is_err());
    }

    #[test]
    fn br162_disclosure_sorting_and_exchange_order_are_deterministic() {
        let disclosure = |id: &str, net: Option<f64>| DragonTigerSourceDisclosure {
            entry_id: id.to_string(),
            trade_id: "1".to_string(),
            reason: None,
            buy_amount_yuan: None,
            sell_amount_yuan: None,
            net_amount_yuan: net,
            turnover_rate_pct: None,
            seats: Vec::new(),
        };
        let mut rows = [
            disclosure("TEST_CODE_none", None),
            disclosure("TEST_CODE_low", Some(1.0)),
            disclosure("TEST_CODE_high", Some(2.0)),
        ];
        rows.sort_by(disclosure_order);
        assert_eq!(rows[0].entry_id, "TEST_CODE_high");
        assert_eq!(rows[1].entry_id, "TEST_CODE_low");
        assert_eq!(rows[2].entry_id, "TEST_CODE_none");
        assert_eq!(exchange_order(Exchange::Shanghai), 0);
        assert_eq!(exchange_order(Exchange::Shenzhen), 1);
        assert_eq!(exchange_order(Exchange::Beijing), 2);
        assert_eq!(seat_side_order(DragonTigerSide::Buy), 0);
        assert_eq!(seat_side_order(DragonTigerSide::Sell), 1);
    }

    #[test]
    fn br162_eastmoney_errors_keep_retry_and_audit_semantics() {
        let cases = [
            eastmoney_gateway_error(EastmoneyError::InvalidRequest("TEST_CODE".into())),
            eastmoney_gateway_error(EastmoneyError::Transport("TEST_CODE".into())),
            eastmoney_gateway_error(EastmoneyError::ResponseTooLarge { limit: 1 }),
            eastmoney_gateway_error(EastmoneyError::Unsupported("TEST_CODE".into())),
            eastmoney_gateway_error(EastmoneyError::Decode("TEST_CODE".into())),
            eastmoney_gateway_error(EastmoneyError::Protocol("TEST_CODE".into())),
        ];
        assert_eq!(cases[0].audit_outcome(), "invalid_request");
        for error in &cases[1..3] {
            assert_eq!(error.audit_outcome(), "unavailable");
            assert!(error.retryable());
        }
        assert_eq!(cases[3].audit_outcome(), "unsupported");
        for error in &cases[4..] {
            assert_eq!(error.audit_outcome(), "partial");
            assert!(!error.retryable());
        }
    }
}
