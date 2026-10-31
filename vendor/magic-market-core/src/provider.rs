use crate::{provenance::checked_text, DataBatch, InstrumentId};
use serde::{de, Deserialize, Deserializer, Serialize};

fn ensure_status_consistent(
    status: DataStatus,
    complete: bool,
    record: &'static str,
) -> Result<(), crate::CoreError> {
    if status == DataStatus::Available && !complete {
        return Err(crate::CoreError::InvalidRequest(format!(
            "available {record} is missing required normalized fields"
        )));
    }
    Ok(())
}

fn ensure_nonnegative_money(
    field: &'static str,
    value: Option<crate::Money>,
) -> Result<(), crate::CoreError> {
    if let Some(money) = value {
        if money.get() < 0.0 {
            return Err(crate::CoreError::InvalidValue {
                field,
                value: money.get().to_string(),
                reason: "must be non-negative",
            });
        }
    }
    Ok(())
}

/// Provider identity used in provenance and capability reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderId {
    Tdx,
    Tencent,
    Eastmoney,
    Sina,
    Baostock,
    Baidu,
    Tonghuashun,
    HithinkFinance,
    Iwencai,
    Cninfo,
    Cailianpress,
    Jin10,
    ThePaper,
    Yonhap,
    WallstreetCn,
    Sse,
    Szse,
    Hkex,
    Cffex,
    StateCouncil,
    Nbs,
    Pbc,
    Cfets,
    Fred,
    Imf,
    WorldBank,
    SecEdgar,
    XinhuaFinance,
    Yicai,
    SecuritiesTimes,
    LocalAnalysis,
    /// Read-only data exposed by an authorized local terminal/SDK.
    LocalTerminal,
    Custom,
}

/// Common source evidence exposed by every normalized market-data record.
pub trait SourcedRecord {
    fn provider_id(&self) -> ProviderId;
    fn evidence_batch_id(&self) -> &str;

    /// Provider-supplied record time when this record family exposes one.
    fn evidence_source_at(&self) -> Option<&str> {
        None
    }

    /// Local observation time corresponding to this record when exposed.
    fn evidence_observed_at(&self) -> Option<&str> {
        None
    }
}

/// Normalized realtime quote contract for cross-provider consumers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "QuoteWire")]
pub struct Quote {
    instrument: InstrumentId,
    name: Option<String>,
    price: crate::Price,
    previous_close: Option<crate::Price>,
    open: Option<crate::Price>,
    high: Option<crate::Price>,
    low: Option<crate::Price>,
    change_percent: Option<crate::Ratio>,
    volume: crate::Quantity,
    amount: Option<crate::Money>,
    /// Overall completeness of the normalized quote fields.
    status: DataStatus,
    /// Timestamp supplied by the source, when the packet proves one.
    source_at: Option<String>,
    /// Local observation timestamp, kept separate from `source_at`.
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl Quote {
    /// Creates a quote with explicit observation evidence.
    pub fn new(
        instrument: InstrumentId,
        price: crate::Price,
        volume: crate::Quantity,
        amount: Option<crate::Money>,
        observed_at: impl Into<String>,
        provider: ProviderId,
        batch_id: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        Self::from_parts(
            instrument,
            None,
            price,
            None,
            None,
            None,
            None,
            None,
            volume,
            amount,
            DataStatus::Unavailable,
            None,
            observed_at,
            provider,
            batch_id,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        instrument: InstrumentId,
        name: Option<String>,
        price: crate::Price,
        previous_close: Option<crate::Price>,
        open: Option<crate::Price>,
        high: Option<crate::Price>,
        low: Option<crate::Price>,
        change_percent: Option<crate::Ratio>,
        volume: crate::Quantity,
        amount: Option<crate::Money>,
        status: DataStatus,
        source_at: Option<String>,
        observed_at: impl Into<String>,
        provider: ProviderId,
        batch_id: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        let name = name
            .map(|value| checked_text("quote_name", value))
            .transpose()?;
        let source_at = source_at
            .map(|value| checked_text("source_at", value))
            .transpose()?;
        ensure_nonnegative_money("quote_amount", amount)?;
        let complete = name.is_some()
            && previous_close.is_some()
            && open.is_some()
            && high.is_some()
            && low.is_some()
            && change_percent.is_some()
            && amount.is_some()
            && source_at.is_some();
        ensure_status_consistent(status, complete, "quote")?;
        Ok(Self {
            instrument,
            name,
            price,
            previous_close,
            open,
            high,
            low,
            change_percent,
            volume,
            amount,
            status,
            source_at,
            observed_at: checked_text("observed_at", observed_at)?,
            provider,
            batch_id: checked_text("batch_id", batch_id)?,
        })
    }
    pub fn with_source_at(
        mut self,
        source_at: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        self.source_at = Some(checked_text("source_at", source_at)?);
        Ok(self)
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    pub fn price(&self) -> crate::Price {
        self.price
    }
    pub fn previous_close(&self) -> Option<crate::Price> {
        self.previous_close
    }
    pub fn open(&self) -> Option<crate::Price> {
        self.open
    }
    pub fn high(&self) -> Option<crate::Price> {
        self.high
    }
    pub fn low(&self) -> Option<crate::Price> {
        self.low
    }
    pub fn change_percent(&self) -> Option<crate::Ratio> {
        self.change_percent
    }
    pub fn volume(&self) -> crate::Quantity {
        self.volume
    }
    pub fn amount(&self) -> Option<crate::Money> {
        self.amount
    }
    pub fn status(&self) -> DataStatus {
        self.status
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }
}

#[derive(Deserialize)]
struct QuoteWire {
    instrument: InstrumentId,
    name: Option<String>,
    price: crate::Price,
    previous_close: Option<crate::Price>,
    open: Option<crate::Price>,
    high: Option<crate::Price>,
    low: Option<crate::Price>,
    change_percent: Option<crate::Ratio>,
    volume: crate::Quantity,
    amount: Option<crate::Money>,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl TryFrom<QuoteWire> for Quote {
    type Error = crate::CoreError;
    fn try_from(value: QuoteWire) -> Result<Self, Self::Error> {
        Self::from_parts(
            value.instrument,
            value.name,
            value.price,
            value.previous_close,
            value.open,
            value.high,
            value.low,
            value.change_percent,
            value.volume,
            value.amount,
            value.status,
            value.source_at,
            value.observed_at,
            value.provider,
            value.batch_id,
        )
    }
}

/// Availability state for optional source fields; absence is never encoded as zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataStatus {
    Available,
    Unavailable,
    Stale,
    Conflicted,
    Unsupported,
}

/// Normalized money-flow snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "MoneyFlowWire")]
pub struct MoneyFlow {
    instrument: InstrumentId,
    main_net: Option<crate::Money>,
    super_large_net: Option<crate::Money>,
    large_net: Option<crate::Money>,
    medium_net: Option<crate::Money>,
    small_net: Option<crate::Money>,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl MoneyFlow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        main_net: Option<crate::Money>,
        super_large_net: Option<crate::Money>,
        large_net: Option<crate::Money>,
        medium_net: Option<crate::Money>,
        small_net: Option<crate::Money>,
        status: DataStatus,
        source_at: Option<String>,
        observed_at: impl Into<String>,
        provider: ProviderId,
        batch_id: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        let source_at = source_at
            .map(|value| checked_text("source_at", value))
            .transpose()?;
        let complete = [main_net, super_large_net, large_net, medium_net, small_net]
            .iter()
            .all(Option::is_some)
            && source_at.is_some();
        ensure_status_consistent(status, complete, "money flow")?;
        Ok(Self {
            instrument,
            main_net,
            super_large_net,
            large_net,
            medium_net,
            small_net,
            status,
            source_at,
            observed_at: checked_text("observed_at", observed_at)?,
            provider,
            batch_id: checked_text("batch_id", batch_id)?,
        })
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn main_net(&self) -> Option<crate::Money> {
        self.main_net
    }
    pub fn super_large_net(&self) -> Option<crate::Money> {
        self.super_large_net
    }
    pub fn large_net(&self) -> Option<crate::Money> {
        self.large_net
    }
    pub fn medium_net(&self) -> Option<crate::Money> {
        self.medium_net
    }
    pub fn small_net(&self) -> Option<crate::Money> {
        self.small_net
    }
    pub fn status(&self) -> DataStatus {
        self.status
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }
}

#[derive(Deserialize)]
struct MoneyFlowWire {
    instrument: InstrumentId,
    main_net: Option<crate::Money>,
    super_large_net: Option<crate::Money>,
    large_net: Option<crate::Money>,
    medium_net: Option<crate::Money>,
    small_net: Option<crate::Money>,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl TryFrom<MoneyFlowWire> for MoneyFlow {
    type Error = crate::CoreError;
    fn try_from(value: MoneyFlowWire) -> Result<Self, Self::Error> {
        Self::new(
            value.instrument,
            value.main_net,
            value.super_large_net,
            value.large_net,
            value.medium_net,
            value.small_net,
            value.status,
            value.source_at,
            value.observed_at,
            value.provider,
            value.batch_id,
        )
    }
}

/// One level of a normalized five-level order book.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "BookLevelWire")]
pub struct BookLevel {
    price: Option<crate::Price>,
    quantity: Option<crate::Quantity>,
}
impl BookLevel {
    pub fn new(
        price: Option<crate::Price>,
        quantity: Option<crate::Quantity>,
    ) -> Result<Self, crate::CoreError> {
        if price.is_some() != quantity.is_some() {
            return Err(crate::CoreError::InvalidRequest(
                "order-book price and quantity must be present together".into(),
            ));
        }
        Ok(Self { price, quantity })
    }
    pub const fn unavailable() -> Self {
        Self {
            price: None,
            quantity: None,
        }
    }
    pub fn price(self) -> Option<crate::Price> {
        self.price
    }
    pub fn quantity(self) -> Option<crate::Quantity> {
        self.quantity
    }
}

#[derive(Deserialize)]
struct BookLevelWire {
    price: Option<crate::Price>,
    quantity: Option<crate::Quantity>,
}
impl TryFrom<BookLevelWire> for BookLevel {
    type Error = crate::CoreError;
    fn try_from(value: BookLevelWire) -> Result<Self, Self::Error> {
        Self::new(value.price, value.quantity)
    }
}

/// Normalized order-book snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "OrderBookWire")]
pub struct OrderBook {
    instrument: InstrumentId,
    bids: [BookLevel; 5],
    asks: [BookLevel; 5],
    /// Sum of the quantities exposed by the returned bid levels.
    total_bid_quantity: Option<crate::Quantity>,
    /// Sum of the quantities exposed by the returned ask levels.
    total_ask_quantity: Option<crate::Quantity>,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl OrderBook {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        bids: [BookLevel; 5],
        asks: [BookLevel; 5],
        total_bid_quantity: Option<crate::Quantity>,
        total_ask_quantity: Option<crate::Quantity>,
        status: DataStatus,
        source_at: Option<String>,
        observed_at: impl Into<String>,
        provider: ProviderId,
        batch_id: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        validate_book_total("total_bid_quantity", &bids, total_bid_quantity)?;
        validate_book_total("total_ask_quantity", &asks, total_ask_quantity)?;
        let source_at = source_at
            .map(|value| checked_text("source_at", value))
            .transpose()?;
        let complete = bids.iter().chain(&asks).all(|level| level.price.is_some())
            && total_bid_quantity.is_some()
            && total_ask_quantity.is_some()
            && source_at.is_some();
        ensure_status_consistent(status, complete, "order book")?;
        Ok(Self {
            instrument,
            bids,
            asks,
            total_bid_quantity,
            total_ask_quantity,
            status,
            source_at,
            observed_at: checked_text("observed_at", observed_at)?,
            provider,
            batch_id: checked_text("batch_id", batch_id)?,
        })
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn bids(&self) -> &[BookLevel; 5] {
        &self.bids
    }
    pub fn asks(&self) -> &[BookLevel; 5] {
        &self.asks
    }
    pub fn total_bid_quantity(&self) -> Option<crate::Quantity> {
        self.total_bid_quantity
    }
    pub fn total_ask_quantity(&self) -> Option<crate::Quantity> {
        self.total_ask_quantity
    }
    pub fn status(&self) -> DataStatus {
        self.status
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }
}

fn validate_book_total(
    field: &'static str,
    levels: &[BookLevel; 5],
    total: Option<crate::Quantity>,
) -> Result<(), crate::CoreError> {
    let quantities: Vec<_> = levels.iter().filter_map(|level| level.quantity).collect();
    if quantities.is_empty() != total.is_none() {
        return Err(crate::CoreError::InvalidRequest(format!(
            "{field} presence contradicts order-book levels"
        )));
    }
    if let Some(total) = total {
        let sum: f64 = quantities.iter().copied().map(crate::Quantity::get).sum();
        let tolerance = crate::NumericTolerance::new(f64::EPSILON * sum.abs().max(1.0) * 8.0, 0.0)?;
        if !tolerance.matches(sum, total.get()) {
            return Err(crate::CoreError::InvalidValue {
                field,
                value: total.get().to_string(),
                reason: "must equal the sum of returned levels",
            });
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct OrderBookWire {
    instrument: InstrumentId,
    bids: [BookLevel; 5],
    asks: [BookLevel; 5],
    total_bid_quantity: Option<crate::Quantity>,
    total_ask_quantity: Option<crate::Quantity>,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl TryFrom<OrderBookWire> for OrderBook {
    type Error = crate::CoreError;
    fn try_from(value: OrderBookWire) -> Result<Self, Self::Error> {
        Self::new(
            value.instrument,
            value.bids,
            value.asks,
            value.total_bid_quantity,
            value.total_ask_quantity,
            value.status,
            value.source_at,
            value.observed_at,
            value.provider,
            value.batch_id,
        )
    }
}

/// Normalized call-auction snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "AuctionSnapshotWire")]
pub struct AuctionSnapshot {
    instrument: InstrumentId,
    name: Option<String>,
    matched_price: Option<crate::Price>,
    previous_close: Option<crate::Price>,
    change_percent: Option<crate::Ratio>,
    matched_quantity: Option<crate::Quantity>,
    matched_amount: Option<crate::Money>,
    unmatched_bid_quantity: Option<crate::Quantity>,
    unmatched_ask_quantity: Option<crate::Quantity>,
    volume_ratio: Option<crate::Ratio>,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl AuctionSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        name: Option<String>,
        matched_price: Option<crate::Price>,
        previous_close: Option<crate::Price>,
        change_percent: Option<crate::Ratio>,
        matched_quantity: Option<crate::Quantity>,
        matched_amount: Option<crate::Money>,
        unmatched_bid_quantity: Option<crate::Quantity>,
        unmatched_ask_quantity: Option<crate::Quantity>,
        volume_ratio: Option<crate::Ratio>,
        status: DataStatus,
        source_at: Option<String>,
        observed_at: impl Into<String>,
        provider: ProviderId,
        batch_id: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        let name = name
            .map(|value| checked_text("auction_name", value))
            .transpose()?;
        let source_at = source_at
            .map(|value| checked_text("source_at", value))
            .transpose()?;
        ensure_nonnegative_money("auction_matched_amount", matched_amount)?;
        let complete = name.is_some()
            && matched_price.is_some()
            && previous_close.is_some()
            && change_percent.is_some()
            && matched_quantity.is_some()
            && matched_amount.is_some()
            && unmatched_bid_quantity.is_some()
            && unmatched_ask_quantity.is_some()
            && volume_ratio.is_some()
            && source_at.is_some();
        ensure_status_consistent(status, complete, "auction snapshot")?;
        Ok(Self {
            instrument,
            name,
            matched_price,
            previous_close,
            change_percent,
            matched_quantity,
            matched_amount,
            unmatched_bid_quantity,
            unmatched_ask_quantity,
            volume_ratio,
            status,
            source_at,
            observed_at: checked_text("observed_at", observed_at)?,
            provider,
            batch_id: checked_text("batch_id", batch_id)?,
        })
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    pub fn matched_price(&self) -> Option<crate::Price> {
        self.matched_price
    }
    pub fn previous_close(&self) -> Option<crate::Price> {
        self.previous_close
    }
    pub fn change_percent(&self) -> Option<crate::Ratio> {
        self.change_percent
    }
    pub fn matched_quantity(&self) -> Option<crate::Quantity> {
        self.matched_quantity
    }
    pub fn matched_amount(&self) -> Option<crate::Money> {
        self.matched_amount
    }
    pub fn unmatched_bid_quantity(&self) -> Option<crate::Quantity> {
        self.unmatched_bid_quantity
    }
    pub fn unmatched_ask_quantity(&self) -> Option<crate::Quantity> {
        self.unmatched_ask_quantity
    }
    pub fn volume_ratio(&self) -> Option<crate::Ratio> {
        self.volume_ratio
    }
    pub fn status(&self) -> DataStatus {
        self.status
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }
}

#[derive(Deserialize)]
struct AuctionSnapshotWire {
    instrument: InstrumentId,
    name: Option<String>,
    matched_price: Option<crate::Price>,
    previous_close: Option<crate::Price>,
    change_percent: Option<crate::Ratio>,
    matched_quantity: Option<crate::Quantity>,
    matched_amount: Option<crate::Money>,
    unmatched_bid_quantity: Option<crate::Quantity>,
    unmatched_ask_quantity: Option<crate::Quantity>,
    volume_ratio: Option<crate::Ratio>,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl TryFrom<AuctionSnapshotWire> for AuctionSnapshot {
    type Error = crate::CoreError;
    fn try_from(value: AuctionSnapshotWire) -> Result<Self, Self::Error> {
        Self::new(
            value.instrument,
            value.name,
            value.matched_price,
            value.previous_close,
            value.change_percent,
            value.matched_quantity,
            value.matched_amount,
            value.unmatched_bid_quantity,
            value.unmatched_ask_quantity,
            value.volume_ratio,
            value.status,
            value.source_at,
            value.observed_at,
            value.provider,
            value.batch_id,
        )
    }
}

/// Aggressor side reported for an executed trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeSide {
    Buy,
    Sell,
    Neutral,
    /// A provider-specific value that is preserved rather than guessed.
    Unknown(u32),
}

/// Provider-neutral executed-trade record with source evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "TradeWire")]
pub struct Trade {
    instrument: InstrumentId,
    /// Source trade time. Historical requests include the requested ISO date.
    trade_at: String,
    price: crate::Price,
    quantity: crate::Quantity,
    /// Number of source executions aggregated into this row, when supplied.
    trade_count: Option<u32>,
    side: TradeSide,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl Trade {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        trade_at: impl Into<String>,
        price: crate::Price,
        quantity: crate::Quantity,
        trade_count: Option<u32>,
        side: TradeSide,
        status: DataStatus,
        source_at: Option<String>,
        observed_at: impl Into<String>,
        provider: ProviderId,
        batch_id: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        if trade_count == Some(0) {
            return Err(crate::CoreError::InvalidValue {
                field: "trade_count",
                value: "0".into(),
                reason: "must be positive when present",
            });
        }
        let trade_at = checked_text("trade_at", trade_at)?;
        let source_at = source_at
            .map(|value| checked_text("source_at", value))
            .transpose()?;
        let complete = !matches!(side, TradeSide::Unknown(_)) && source_at.is_some();
        ensure_status_consistent(status, complete, "trade")?;
        Ok(Self {
            instrument,
            trade_at,
            price,
            quantity,
            trade_count,
            side,
            status,
            source_at,
            observed_at: checked_text("observed_at", observed_at)?,
            provider,
            batch_id: checked_text("batch_id", batch_id)?,
        })
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn trade_at(&self) -> &str {
        &self.trade_at
    }
    pub fn price(&self) -> crate::Price {
        self.price
    }
    pub fn quantity(&self) -> crate::Quantity {
        self.quantity
    }
    pub fn trade_count(&self) -> Option<u32> {
        self.trade_count
    }
    pub fn side(&self) -> TradeSide {
        self.side
    }
    pub fn status(&self) -> DataStatus {
        self.status
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }
}

#[derive(Deserialize)]
struct TradeWire {
    instrument: InstrumentId,
    trade_at: String,
    price: crate::Price,
    quantity: crate::Quantity,
    trade_count: Option<u32>,
    side: TradeSide,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl TryFrom<TradeWire> for Trade {
    type Error = crate::CoreError;
    fn try_from(value: TradeWire) -> Result<Self, Self::Error> {
        Self::new(
            value.instrument,
            value.trade_at,
            value.price,
            value.quantity,
            value.trade_count,
            value.side,
            value.status,
            value.source_at,
            value.observed_at,
            value.provider,
            value.batch_id,
        )
    }
}

/// Normalized listing board. `Unknown` preserves a source value that cannot be
/// mapped without guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Board {
    Main,
    ChiNext,
    Star,
    Beijing,
    Unknown,
}

/// Source-backed daily price-limit rule. Missing values remain `None`; callers
/// must not infer a current rule from an instrument code alone.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PriceLimitRule {
    percent: Option<crate::Ratio>,
    version: Option<String>,
}
impl PriceLimitRule {
    pub fn new(
        percent: Option<crate::Ratio>,
        version: Option<String>,
    ) -> Result<Self, crate::CoreError> {
        if let Some(percent) = percent {
            if percent.unit() != crate::RatioUnit::Percent || percent.get() <= 0.0 {
                return Err(crate::CoreError::InvalidValue {
                    field: "price_limit_percent",
                    value: percent.get().to_string(),
                    reason: "must be a positive percentage",
                });
            }
        }
        let version = version
            .map(|value| checked_text("price_limit_version", value))
            .transpose()?;
        Ok(Self { percent, version })
    }
    pub fn percent(&self) -> Option<crate::Ratio> {
        self.percent
    }
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}
impl<'de> Deserialize<'de> for PriceLimitRule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            percent: Option<crate::Ratio>,
            version: Option<String>,
        }
        let repr = Repr::deserialize(deserializer)?;
        Self::new(repr.percent, repr.version).map_err(de::Error::custom)
    }
}

/// Provider-neutral security master record with field-level availability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SecurityMetadataWire")]
pub struct SecurityMetadata {
    instrument: InstrumentId,
    name: Option<String>,
    board: Option<Board>,
    is_st: Option<bool>,
    /// Source listing date in `YYYY-MM-DD` form, when supplied.
    listed_on: Option<String>,
    price_limit: PriceLimitRule,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl SecurityMetadata {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        name: Option<String>,
        board: Option<Board>,
        is_st: Option<bool>,
        listed_on: Option<String>,
        price_limit: PriceLimitRule,
        status: DataStatus,
        source_at: Option<String>,
        observed_at: impl Into<String>,
        provider: ProviderId,
        batch_id: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        let name = name
            .map(|value| checked_text("security_name", value))
            .transpose()?;
        if listed_on
            .as_deref()
            .is_some_and(|date| !valid_iso_date(date))
        {
            return Err(crate::CoreError::InvalidRequest(
                "invalid security listing date".into(),
            ));
        }
        let source_at = source_at
            .map(|value| checked_text("source_at", value))
            .transpose()?;
        let complete = name.is_some()
            && board.is_some()
            && is_st.is_some()
            && listed_on.is_some()
            && price_limit.percent.is_some()
            && price_limit.version.is_some()
            && source_at.is_some();
        ensure_status_consistent(status, complete, "security metadata")?;
        Ok(Self {
            instrument,
            name,
            board,
            is_st,
            listed_on,
            price_limit,
            status,
            source_at,
            observed_at: checked_text("observed_at", observed_at)?,
            provider,
            batch_id: checked_text("batch_id", batch_id)?,
        })
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    pub fn board(&self) -> Option<Board> {
        self.board
    }
    pub fn is_st(&self) -> Option<bool> {
        self.is_st
    }
    pub fn listed_on(&self) -> Option<&str> {
        self.listed_on.as_deref()
    }
    pub fn price_limit(&self) -> &PriceLimitRule {
        &self.price_limit
    }
    pub fn status(&self) -> DataStatus {
        self.status
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }
}

#[derive(Deserialize)]
struct SecurityMetadataWire {
    instrument: InstrumentId,
    name: Option<String>,
    board: Option<Board>,
    is_st: Option<bool>,
    listed_on: Option<String>,
    price_limit: PriceLimitRule,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}
impl TryFrom<SecurityMetadataWire> for SecurityMetadata {
    type Error = crate::CoreError;
    fn try_from(value: SecurityMetadataWire) -> Result<Self, Self::Error> {
        Self::new(
            value.instrument,
            value.name,
            value.board,
            value.is_st,
            value.listed_on,
            value.price_limit,
            value.status,
            value.source_at,
            value.observed_at,
            value.provider,
            value.batch_id,
        )
    }
}

/// Declares which data families a provider implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub quotes: bool,
    pub bars: bool,
    pub minute: bool,
    pub trades: bool,
    pub fundamentals: bool,
    pub corporate_actions: bool,
    pub blocks: bool,
    pub money_flow: bool,
    pub order_book: bool,
    pub auction: bool,
    pub security_metadata: bool,
}
impl Capabilities {
    pub const fn new() -> Self {
        Self {
            quotes: false,
            bars: false,
            minute: false,
            trades: false,
            fundamentals: false,
            corporate_actions: false,
            blocks: false,
            money_flow: false,
            order_book: false,
            auction: false,
            security_metadata: false,
        }
    }
}
impl Default for Capabilities {
    fn default() -> Self {
        Self::new()
    }
}

/// Standard bar interval shared by market-data providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BarInterval {
    Minute1,
    Minute5,
    Minute15,
    Minute30,
    Hour1,
    Day,
    Week,
    Month,
    Year,
}

/// Price adjustment applied by the source to a historical bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Adjustment {
    Unadjusted,
    Forward,
    Backward,
}

fn valid_iso_date(value: &str) -> bool {
    if value.len() != 10
        || value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = value[0..4].parse().unwrap_or(0);
    let month: u32 = value[5..7].parse().unwrap_or(0);
    let day: u32 = value[8..10].parse().unwrap_or(0);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    year >= 1900 && max_day != 0 && day >= 1 && day <= max_day
}

fn valid_clock_time(value: &str) -> bool {
    if value.len() != 8
        || value.as_bytes()[2] != b':'
        || value.as_bytes()[5] != b':'
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 2 || index == 5 || byte.is_ascii_digit())
    {
        return false;
    }
    let hour: u8 = value[0..2].parse().unwrap_or(24);
    let minute: u8 = value[3..5].parse().unwrap_or(60);
    let second: u8 = value[6..8].parse().unwrap_or(60);
    hour < 24 && minute < 60 && second < 60
}

fn valid_minute_timestamp(value: &str) -> bool {
    if value.len() != 16
        || !matches!(value.as_bytes()[10], b' ' | b'T')
        || !valid_iso_date(&value[..10])
    {
        return false;
    }
    let clock = &value[11..];
    clock.len() == 5
        && clock.as_bytes()[2] == b':'
        && clock
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 2 || byte.is_ascii_digit())
        && clock[0..2].parse::<u8>().is_ok_and(|hour| hour < 24)
        && clock[3..5].parse::<u8>().is_ok_and(|minute| minute < 60)
}

fn valid_bar_time(value: &str, interval: BarInterval) -> bool {
    match interval {
        BarInterval::Minute1
        | BarInterval::Minute5
        | BarInterval::Minute15
        | BarInterval::Minute30
        | BarInterval::Hour1 => {
            value.len() == 19
                && matches!(value.as_bytes()[10], b' ' | b'T')
                && valid_iso_date(&value[..10])
                && valid_clock_time(&value[11..])
        }
        BarInterval::Day | BarInterval::Week | BarInterval::Month | BarInterval::Year => {
            valid_iso_date(value)
        }
    }
}

/// One source minute with cumulative session volume and amount.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "MinutePointWire")]
pub struct MinutePoint {
    instrument: InstrumentId,
    minute_at: String,
    price: crate::Price,
    cumulative_quantity: crate::Quantity,
    cumulative_amount: Option<crate::Money>,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}

impl MinutePoint {
    /// Builds one checked minute point.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        minute_at: impl Into<String>,
        price: crate::Price,
        cumulative_quantity: crate::Quantity,
        cumulative_amount: Option<crate::Money>,
        status: DataStatus,
        source_at: Option<String>,
        observed_at: impl Into<String>,
        provider: ProviderId,
        batch_id: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        let minute_at = checked_text("minute_at", minute_at)?;
        if !valid_minute_timestamp(&minute_at) {
            return Err(crate::CoreError::InvalidRequest(
                "invalid minute timestamp".into(),
            ));
        }
        ensure_nonnegative_money("minute_cumulative_amount", cumulative_amount)?;
        let source_at = source_at
            .map(|value| checked_text("source_at", value))
            .transpose()?;
        ensure_status_consistent(status, source_at.is_some(), "minute point")?;
        Ok(Self {
            instrument,
            minute_at,
            price,
            cumulative_quantity,
            cumulative_amount,
            status,
            source_at,
            observed_at: checked_text("observed_at", observed_at)?,
            provider,
            batch_id: checked_text("batch_id", batch_id)?,
        })
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn minute_at(&self) -> &str {
        &self.minute_at
    }
    pub fn price(&self) -> crate::Price {
        self.price
    }
    pub fn cumulative_quantity(&self) -> crate::Quantity {
        self.cumulative_quantity
    }
    pub fn cumulative_amount(&self) -> Option<crate::Money> {
        self.cumulative_amount
    }
    pub fn status(&self) -> DataStatus {
        self.status
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }
}

#[derive(Deserialize)]
struct MinutePointWire {
    instrument: InstrumentId,
    minute_at: String,
    price: crate::Price,
    cumulative_quantity: crate::Quantity,
    cumulative_amount: Option<crate::Money>,
    status: DataStatus,
    source_at: Option<String>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}

impl TryFrom<MinutePointWire> for MinutePoint {
    type Error = crate::CoreError;

    fn try_from(value: MinutePointWire) -> Result<Self, Self::Error> {
        Self::new(
            value.instrument,
            value.minute_at,
            value.price,
            value.cumulative_quantity,
            value.cumulative_amount,
            value.status,
            value.source_at,
            value.observed_at,
            value.provider,
            value.batch_id,
        )
    }
}

/// Provider-neutral OHLCV bar with record-level source evidence.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bar {
    instrument: InstrumentId,
    interval: BarInterval,
    bar_start: String,
    bar_end: String,
    open: crate::Price,
    high: crate::Price,
    low: crate::Price,
    close: crate::Price,
    volume: crate::Quantity,
    amount: Option<crate::Money>,
    adjustment: Adjustment,
    source_at: Option<String>,
    provider: ProviderId,
    batch_id: String,
}
impl Bar {
    /// Builds a bar and rejects inconsistent OHLC ranges.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        interval: BarInterval,
        bar_start: impl Into<String>,
        bar_end: impl Into<String>,
        open: crate::Price,
        high: crate::Price,
        low: crate::Price,
        close: crate::Price,
        volume: crate::Quantity,
        amount: Option<crate::Money>,
        adjustment: Adjustment,
        provider: ProviderId,
        batch_id: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        let bar_start = checked_text("bar_start", bar_start)?;
        let bar_end = checked_text("bar_end", bar_end)?;
        if !valid_bar_time(&bar_start, interval)
            || !valid_bar_time(&bar_end, interval)
            || bar_start.as_bytes().get(10) != bar_end.as_bytes().get(10)
            || bar_start > bar_end
        {
            return Err(crate::CoreError::InvalidRequest(
                "invalid bar time range".into(),
            ));
        }
        if low.get() > open.get().min(close.get())
            || high.get() < open.get().max(close.get())
            || low.get() > high.get()
        {
            return Err(crate::CoreError::InvalidRequest(
                "inconsistent OHLC range".into(),
            ));
        }
        ensure_nonnegative_money("bar_amount", amount)?;
        Ok(Self {
            instrument,
            interval,
            bar_start,
            bar_end,
            open,
            high,
            low,
            close,
            volume,
            amount,
            adjustment,
            source_at: None,
            provider,
            batch_id: checked_text("batch_id", batch_id)?,
        })
    }

    pub fn with_source_at(
        mut self,
        source_at: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        self.source_at = Some(checked_text("source_at", source_at)?);
        Ok(self)
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn interval(&self) -> BarInterval {
        self.interval
    }
    pub fn bar_start(&self) -> &str {
        &self.bar_start
    }
    pub fn bar_end(&self) -> &str {
        &self.bar_end
    }
    pub fn open(&self) -> crate::Price {
        self.open
    }
    pub fn high(&self) -> crate::Price {
        self.high
    }
    pub fn low(&self) -> crate::Price {
        self.low
    }
    pub fn close(&self) -> crate::Price {
        self.close
    }
    pub fn volume(&self) -> crate::Quantity {
        self.volume
    }
    pub fn amount(&self) -> Option<crate::Money> {
        self.amount
    }
    pub fn adjustment(&self) -> Adjustment {
        self.adjustment
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }
}

impl<'de> Deserialize<'de> for Bar {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            instrument: InstrumentId,
            interval: BarInterval,
            bar_start: String,
            bar_end: String,
            open: crate::Price,
            high: crate::Price,
            low: crate::Price,
            close: crate::Price,
            volume: crate::Quantity,
            amount: Option<crate::Money>,
            adjustment: Adjustment,
            source_at: Option<String>,
            provider: ProviderId,
            batch_id: String,
        }
        let repr = Repr::deserialize(deserializer)?;
        let mut bar = Self::new(
            repr.instrument,
            repr.interval,
            repr.bar_start,
            repr.bar_end,
            repr.open,
            repr.high,
            repr.low,
            repr.close,
            repr.volume,
            repr.amount,
            repr.adjustment,
            repr.provider,
            repr.batch_id,
        )
        .map_err(de::Error::custom)?;
        if let Some(source_at) = repr.source_at {
            bar = bar.with_source_at(source_at).map_err(de::Error::custom)?;
        }
        Ok(bar)
    }
}

/// Validated historical-bar request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BarsRequest {
    instrument: InstrumentId,
    interval: BarInterval,
    start: Option<String>,
    end: Option<String>,
    limit: u16,
}
impl BarsRequest {
    /// Creates a bounded request.
    pub fn new(
        instrument: InstrumentId,
        interval: BarInterval,
        limit: u16,
    ) -> Result<Self, crate::CoreError> {
        if limit == 0 {
            return Err(crate::CoreError::InvalidValue {
                field: "limit",
                value: limit.to_string(),
                reason: "must be positive",
            });
        }
        Ok(Self {
            instrument,
            interval,
            start: None,
            end: None,
            limit,
        })
    }
    /// Adds an inclusive ISO date range after validating ordering and format.
    pub fn with_range(
        mut self,
        start: impl Into<String>,
        end: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        let start = start.into();
        let end = end.into();
        if !valid_iso_date(&start) || !valid_iso_date(&end) || start > end {
            return Err(crate::CoreError::InvalidRequest(
                "invalid date range".into(),
            ));
        }
        self.start = Some(start);
        self.end = Some(end);
        Ok(self)
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn interval(&self) -> BarInterval {
        self.interval
    }
    pub fn start(&self) -> Option<&str> {
        self.start.as_deref()
    }
    pub fn end(&self) -> Option<&str> {
        self.end.as_deref()
    }
    pub fn limit(&self) -> u16 {
        self.limit
    }
}

impl<'de> Deserialize<'de> for BarsRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            instrument: InstrumentId,
            interval: BarInterval,
            start: Option<String>,
            end: Option<String>,
            limit: u16,
        }
        let repr = Repr::deserialize(deserializer)?;
        let request =
            Self::new(repr.instrument, repr.interval, repr.limit).map_err(de::Error::custom)?;
        match (repr.start, repr.end) {
            (None, None) => Ok(request),
            (Some(start), Some(end)) => request.with_range(start, end).map_err(de::Error::custom),
            _ => Err(de::Error::custom(
                "bar date range requires both start and end",
            )),
        }
    }
}

/// Validated current or single-day minute-data request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MinuteDataRequest {
    instrument: InstrumentId,
    date: Option<String>,
}

impl MinuteDataRequest {
    pub fn new(instrument: InstrumentId) -> Self {
        Self {
            instrument,
            date: None,
        }
    }

    /// Selects one historical trading date in `YYYY-MM-DD` form.
    pub fn with_date(mut self, date: impl Into<String>) -> Result<Self, crate::CoreError> {
        let date = date.into();
        if !valid_iso_date(&date) {
            return Err(crate::CoreError::InvalidRequest(
                "invalid minute-data date".into(),
            ));
        }
        self.date = Some(date);
        Ok(self)
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn date(&self) -> Option<&str> {
        self.date.as_deref()
    }
}

impl<'de> Deserialize<'de> for MinuteDataRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            instrument: InstrumentId,
            date: Option<String>,
        }
        let repr = Repr::deserialize(deserializer)?;
        let request = Self::new(repr.instrument);
        match repr.date {
            Some(date) => request.with_date(date).map_err(de::Error::custom),
            None => Ok(request),
        }
    }
}

/// Validated current or single-day historical trade request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TradesRequest {
    instrument: InstrumentId,
    /// `None` requests the current session; `Some` requests one historical day.
    date: Option<String>,
    /// Maximum number of records. Providers automatically paginate up to it.
    limit: u16,
}
impl TradesRequest {
    pub fn new(instrument: InstrumentId, limit: u16) -> Result<Self, crate::CoreError> {
        if limit == 0 {
            return Err(crate::CoreError::InvalidValue {
                field: "limit",
                value: limit.to_string(),
                reason: "must be positive",
            });
        }
        Ok(Self {
            instrument,
            date: None,
            limit,
        })
    }

    /// Selects one historical trading date in `YYYY-MM-DD` form.
    pub fn with_date(mut self, date: impl Into<String>) -> Result<Self, crate::CoreError> {
        let date = date.into();
        if !valid_iso_date(&date) {
            return Err(crate::CoreError::InvalidRequest(
                "invalid trade date".into(),
            ));
        }
        self.date = Some(date);
        Ok(self)
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn date(&self) -> Option<&str> {
        self.date.as_deref()
    }
    pub fn limit(&self) -> u16 {
        self.limit
    }
}

impl<'de> Deserialize<'de> for TradesRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            instrument: InstrumentId,
            date: Option<String>,
            limit: u16,
        }
        let repr = Repr::deserialize(deserializer)?;
        let request = Self::new(repr.instrument, repr.limit).map_err(de::Error::custom)?;
        match repr.date {
            Some(date) => request.with_date(date).map_err(de::Error::custom),
            None => Ok(request),
        }
    }
}

/// Provider capability for historical bars.
pub trait HistoricalBars {
    type Bar;
    type Error: std::error::Error + Send + Sync + 'static;
    fn historical_bars(&self, request: &BarsRequest) -> Result<DataBatch<Self::Bar>, Self::Error>;
}

/// Provider capability for current and historical minute data.
pub trait MinuteData {
    type Error: std::error::Error + Send + Sync + 'static;
    fn minute_data(
        &self,
        request: &MinuteDataRequest,
    ) -> Result<DataBatch<MinutePoint>, Self::Error>;
}

/// Provider capability for realtime quotes.
pub trait RealtimeQuotes {
    type Quote;
    type Error: std::error::Error + Send + Sync + 'static;
    fn realtime_quotes(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<Self::Quote>, Self::Error>;
}

/// Provider capability for money-flow snapshots.
pub trait MoneyFlows {
    type Error: std::error::Error + Send + Sync + 'static;
    fn money_flows(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<MoneyFlow>, Self::Error>;
}

/// Provider capability for order books.
pub trait OrderBooks {
    type Error: std::error::Error + Send + Sync + 'static;
    fn order_books(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<OrderBook>, Self::Error>;
}

/// Provider capability for call-auction snapshots.
pub trait Auctions {
    type Error: std::error::Error + Send + Sync + 'static;
    fn auction_snapshots(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<AuctionSnapshot>, Self::Error>;
}

/// Provider capability for current and historical executed trades.
pub trait Trades {
    type Error: std::error::Error + Send + Sync + 'static;
    fn trades(&self, request: &TradesRequest) -> Result<DataBatch<Trade>, Self::Error>;
}

/// Provider capability for source-backed security master data.
pub trait SecurityMetadataProvider {
    type Error: std::error::Error + Send + Sync + 'static;
    fn security_metadata(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<SecurityMetadata>, Self::Error>;
}

/// Async provider capability for historical bars.
#[allow(async_fn_in_trait)]
pub trait AsyncHistoricalBars {
    type Bar;
    type Error: std::error::Error + Send + Sync + 'static;
    async fn historical_bars_async(
        &self,
        request: &BarsRequest,
    ) -> Result<DataBatch<Self::Bar>, Self::Error>;
}

/// Async provider capability for current and historical minute data.
#[allow(async_fn_in_trait)]
pub trait AsyncMinuteData {
    type Error: std::error::Error + Send + Sync + 'static;
    async fn minute_data_async(
        &self,
        request: &MinuteDataRequest,
    ) -> Result<DataBatch<MinutePoint>, Self::Error>;
}

/// Async provider capability for realtime quotes.
#[allow(async_fn_in_trait)]
pub trait AsyncRealtimeQuotes {
    type Quote;
    type Error: std::error::Error + Send + Sync + 'static;
    async fn realtime_quotes_async(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<Self::Quote>, Self::Error>;
}

/// Async provider capability for current and historical executed trades.
#[allow(async_fn_in_trait)]
pub trait AsyncTrades {
    type Error: std::error::Error + Send + Sync + 'static;
    async fn trades_async(&self, request: &TradesRequest) -> Result<DataBatch<Trade>, Self::Error>;
}

impl SourcedRecord for Quote {
    fn provider_id(&self) -> ProviderId {
        self.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.batch_id()
    }

    fn evidence_source_at(&self) -> Option<&str> {
        self.source_at()
    }

    fn evidence_observed_at(&self) -> Option<&str> {
        Some(self.observed_at())
    }
}

macro_rules! impl_sourced_record {
    ($($record:ty),+ $(,)?) => {
        $(
            impl SourcedRecord for $record {
                fn provider_id(&self) -> ProviderId {
                    self.provider()
                }

                fn evidence_batch_id(&self) -> &str {
                    self.batch_id()
                }
            }
        )+
    };
}

impl_sourced_record!(
    Bar,
    MinutePoint,
    Trade,
    MoneyFlow,
    OrderBook,
    AuctionSnapshot,
    SecurityMetadata,
);
