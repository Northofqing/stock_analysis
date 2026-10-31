use crate::{
    DataBatch, FiniteNumber, InstrumentId, IsoDate, Money, NonEmptyText, PositiveU32, Price, Ratio,
    RatioUnit, SourceEvidence, SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::collections::HashSet;

/// Classification supplied for a board membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardCategory {
    Industry,
    Concept,
    Region,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardMembership {
    pub instrument: InstrumentId,
    pub board_code: NonEmptyText,
    pub board_name: NonEmptyText,
    pub category: BoardCategory,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrongStockReason {
    pub instrument: InstrumentId,
    pub trading_date: IsoDate,
    pub reason: NonEmptyText,
    pub subjects: Vec<NonEmptyText>,
    pub limit_state: Option<NonEmptyText>,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DragonTigerEntry {
    entry_id: NonEmptyText,
    instrument: InstrumentId,
    instrument_name: Option<NonEmptyText>,
    trading_date: IsoDate,
    reason: Option<NonEmptyText>,
    buy_amount: Option<Money>,
    sell_amount: Option<Money>,
    net_amount: Option<Money>,
    turnover_rate: Option<Ratio>,
    evidence: SourceEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DragonTigerSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DragonTigerSeat {
    entry_id: NonEmptyText,
    instrument: InstrumentId,
    trading_date: IsoDate,
    side: DragonTigerSide,
    rank: PositiveU32,
    seat_name: NonEmptyText,
    amount: Money,
    buy_amount: Option<Money>,
    sell_amount: Option<Money>,
    net_amount: Option<Money>,
    evidence: SourceEvidence,
}

impl DragonTigerEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        entry_id: NonEmptyText,
        instrument: InstrumentId,
        trading_date: IsoDate,
        reason: Option<NonEmptyText>,
        buy_amount: Option<Money>,
        sell_amount: Option<Money>,
        net_amount: Option<Money>,
        turnover_rate: Option<Ratio>,
        evidence: SourceEvidence,
    ) -> Result<Self, crate::CoreError> {
        validate_dragon_tiger_evidence(&trading_date, &evidence)?;
        validate_non_negative_money("dragon-tiger buy amount", buy_amount)?;
        validate_non_negative_money("dragon-tiger sell amount", sell_amount)?;
        validate_net_amount(buy_amount, sell_amount, net_amount)?;
        if turnover_rate.is_some_and(|ratio| {
            ratio.unit() != RatioUnit::Percent || ratio.get().is_sign_negative()
        }) {
            return Err(crate::CoreError::InvalidRequest(
                "dragon-tiger turnover rate must be a non-negative percentage".into(),
            ));
        }
        Ok(Self {
            entry_id,
            instrument,
            instrument_name: None,
            trading_date,
            reason,
            buy_amount,
            sell_amount,
            net_amount,
            turnover_rate,
            evidence,
        })
    }

    pub fn entry_id(&self) -> &NonEmptyText {
        &self.entry_id
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn instrument_name(&self) -> Option<&NonEmptyText> {
        self.instrument_name.as_ref()
    }

    pub fn with_instrument_name(mut self, instrument_name: NonEmptyText) -> Self {
        self.instrument_name = Some(instrument_name);
        self
    }

    pub fn trading_date(&self) -> &IsoDate {
        &self.trading_date
    }

    pub fn reason(&self) -> Option<&NonEmptyText> {
        self.reason.as_ref()
    }

    pub fn buy_amount(&self) -> Option<Money> {
        self.buy_amount
    }

    pub fn sell_amount(&self) -> Option<Money> {
        self.sell_amount
    }

    pub fn net_amount(&self) -> Option<Money> {
        self.net_amount
    }

    pub fn turnover_rate(&self) -> Option<Ratio> {
        self.turnover_rate
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

impl DragonTigerSeat {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        entry_id: NonEmptyText,
        instrument: InstrumentId,
        trading_date: IsoDate,
        side: DragonTigerSide,
        rank: PositiveU32,
        seat_name: NonEmptyText,
        amount: Money,
        buy_amount: Option<Money>,
        sell_amount: Option<Money>,
        net_amount: Option<Money>,
        evidence: SourceEvidence,
    ) -> Result<Self, crate::CoreError> {
        validate_dragon_tiger_evidence(&trading_date, &evidence)?;
        if rank.get() > 5 {
            return Err(crate::CoreError::InvalidRequest(
                "dragon-tiger seat rank must be between 1 and 5".into(),
            ));
        }
        validate_non_negative_money("dragon-tiger seat amount", Some(amount))?;
        validate_non_negative_money("dragon-tiger seat buy amount", buy_amount)?;
        validate_non_negative_money("dragon-tiger seat sell amount", sell_amount)?;
        validate_net_amount(buy_amount, sell_amount, net_amount)?;
        let side_amount = match side {
            DragonTigerSide::Buy => buy_amount,
            DragonTigerSide::Sell => sell_amount,
        };
        let side_amount_matches = match side_amount {
            Some(value) => money_values_match(value, amount)?,
            None => false,
        };
        if !side_amount_matches {
            return Err(crate::CoreError::InvalidRequest(
                "dragon-tiger seat amount must match its side amount".into(),
            ));
        }
        Ok(Self {
            entry_id,
            instrument,
            trading_date,
            side,
            rank,
            seat_name,
            amount,
            buy_amount,
            sell_amount,
            net_amount,
            evidence,
        })
    }

    pub fn entry_id(&self) -> &NonEmptyText {
        &self.entry_id
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn trading_date(&self) -> &IsoDate {
        &self.trading_date
    }

    pub fn side(&self) -> DragonTigerSide {
        self.side
    }

    pub fn rank(&self) -> PositiveU32 {
        self.rank
    }

    pub fn seat_name(&self) -> &NonEmptyText {
        &self.seat_name
    }

    pub fn amount(&self) -> Money {
        self.amount
    }

    pub fn buy_amount(&self) -> Option<Money> {
        self.buy_amount
    }

    pub fn sell_amount(&self) -> Option<Money> {
        self.sell_amount
    }

    pub fn net_amount(&self) -> Option<Money> {
        self.net_amount
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

/// One source entry plus its complete buy-five and sell-five disclosure.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DragonTigerDisclosure {
    entry: DragonTigerEntry,
    seats: Vec<DragonTigerSeat>,
}

impl DragonTigerDisclosure {
    pub fn new(
        entry: DragonTigerEntry,
        seats: Vec<DragonTigerSeat>,
    ) -> Result<Self, crate::CoreError> {
        if seats.len() != 10 {
            return Err(crate::CoreError::InvalidRequest(format!(
                "dragon-tiger disclosure must contain exactly 10 seats, got {}",
                seats.len()
            )));
        }
        let mut identities = HashSet::with_capacity(seats.len());
        for seat in &seats {
            if seat.entry_id() != entry.entry_id()
                || seat.instrument() != entry.instrument()
                || seat.trading_date() != entry.trading_date()
            {
                return Err(crate::CoreError::InvalidRequest(
                    "dragon-tiger disclosure seat identity must match its entry".into(),
                ));
            }
            if seat.evidence().provider() != entry.evidence().provider()
                || seat.evidence().batch_id() != entry.evidence().batch_id()
                || seat.evidence().source_at() != entry.evidence().source_at()
                || seat.evidence().observed_at() != entry.evidence().observed_at()
            {
                return Err(crate::CoreError::InvalidRequest(
                    "dragon-tiger disclosure entry and seats must share source evidence".into(),
                ));
            }
            if !identities.insert((seat.side(), seat.rank().get())) {
                return Err(crate::CoreError::InvalidRequest(
                    "dragon-tiger disclosure contains a duplicate side/rank".into(),
                ));
            }
        }
        for side in [DragonTigerSide::Buy, DragonTigerSide::Sell] {
            for rank in 1..=5 {
                if !identities.contains(&(side, rank)) {
                    return Err(crate::CoreError::InvalidRequest(
                        "dragon-tiger disclosure requires ranks 1 through 5 on both sides".into(),
                    ));
                }
            }
        }
        Ok(Self { entry, seats })
    }

    pub fn entry(&self) -> &DragonTigerEntry {
        &self.entry
    }

    pub fn seats(&self) -> &[DragonTigerSeat] {
        &self.seats
    }
}

#[derive(Deserialize)]
struct DragonTigerDisclosureWire {
    entry: DragonTigerEntry,
    seats: Vec<DragonTigerSeat>,
}

impl<'de> Deserialize<'de> for DragonTigerDisclosure {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DragonTigerDisclosureWire::deserialize(deserializer)?;
        Self::new(wire.entry, wire.seats).map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
struct DragonTigerEntryWire {
    entry_id: NonEmptyText,
    instrument: InstrumentId,
    #[serde(default)]
    instrument_name: Option<NonEmptyText>,
    trading_date: IsoDate,
    reason: Option<NonEmptyText>,
    buy_amount: Option<Money>,
    sell_amount: Option<Money>,
    net_amount: Option<Money>,
    turnover_rate: Option<Ratio>,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for DragonTigerEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DragonTigerEntryWire::deserialize(deserializer)?;
        Self::new(
            wire.entry_id,
            wire.instrument,
            wire.trading_date,
            wire.reason,
            wire.buy_amount,
            wire.sell_amount,
            wire.net_amount,
            wire.turnover_rate,
            wire.evidence,
        )
        .map(|mut record| {
            record.instrument_name = wire.instrument_name;
            record
        })
        .map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
struct DragonTigerSeatWire {
    entry_id: NonEmptyText,
    instrument: InstrumentId,
    trading_date: IsoDate,
    side: DragonTigerSide,
    rank: PositiveU32,
    seat_name: NonEmptyText,
    amount: Money,
    buy_amount: Option<Money>,
    sell_amount: Option<Money>,
    net_amount: Option<Money>,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for DragonTigerSeat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DragonTigerSeatWire::deserialize(deserializer)?;
        Self::new(
            wire.entry_id,
            wire.instrument,
            wire.trading_date,
            wire.side,
            wire.rank,
            wire.seat_name,
            wire.amount,
            wire.buy_amount,
            wire.sell_amount,
            wire.net_amount,
            wire.evidence,
        )
        .map_err(de::Error::custom)
    }
}

fn validate_dragon_tiger_evidence(
    trading_date: &IsoDate,
    evidence: &SourceEvidence,
) -> Result<(), crate::CoreError> {
    let source_at = evidence.source_at().ok_or_else(|| {
        crate::CoreError::InvalidRequest(
            "dragon-tiger record evidence must include source_at".into(),
        )
    })?;
    let remainder = source_at
        .strip_prefix(trading_date.as_str())
        .ok_or_else(|| {
            crate::CoreError::InvalidRequest(
                "dragon-tiger record evidence date must match trading date".into(),
            )
        })?;
    if !remainder.is_empty() && !remainder.starts_with('T') && !remainder.starts_with(' ') {
        return Err(crate::CoreError::InvalidRequest(
            "dragon-tiger record evidence date must match trading date".into(),
        ));
    }
    Ok(())
}

fn validate_non_negative_money(name: &str, value: Option<Money>) -> Result<(), crate::CoreError> {
    if value.is_some_and(|money| money.get().is_sign_negative()) {
        return Err(crate::CoreError::InvalidRequest(format!(
            "{name} must be non-negative"
        )));
    }
    Ok(())
}

fn validate_net_amount(
    buy_amount: Option<Money>,
    sell_amount: Option<Money>,
    net_amount: Option<Money>,
) -> Result<(), crate::CoreError> {
    if let (Some(buy), Some(sell), Some(net)) = (buy_amount, sell_amount, net_amount) {
        let expected = Money::new(buy.get() - sell.get())?;
        if !money_values_match(net, expected)? {
            return Err(crate::CoreError::InvalidRequest(
                "dragon-tiger net amount must equal buy amount minus sell amount".into(),
            ));
        }
    }
    Ok(())
}

fn money_values_match(left: Money, right: Money) -> Result<bool, crate::CoreError> {
    Ok(crate::NumericTolerance::new(0.01, 0.0)?.matches(left.get(), right.get()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketRankingKind {
    VolumeRatio,
    MainNetInflow,
    Industry,
    Concept,
    Region,
    Popularity,
    Custom(NonEmptyText),
}

/// Unit carried by a ranking metric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketRankingUnit {
    /// Turnover volume divided by the comparable recent average.
    Multiple,
    /// Chinese yuan.
    Yuan,
    /// Percentage points.
    Percent,
    /// Source-specific dimensionless score.
    Score,
    /// Explicit unit for a custom metric.
    Custom(NonEmptyText),
}

/// Independently admitted production ranking metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MarketRankingCapabilities {
    pub volume_ratio: bool,
    pub main_net_inflow: bool,
}

impl MarketRankingCapabilities {
    pub const fn all_admitted(self) -> bool {
        self.volume_ratio && self.main_net_inflow
    }

    pub fn supports(self, kind: &MarketRankingKind) -> bool {
        match kind {
            MarketRankingKind::VolumeRatio => self.volume_ratio,
            MarketRankingKind::MainNetInflow => self.main_net_inflow,
            _ => false,
        }
    }
}

/// Market session proved by the source observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketSession {
    PreOpen,
    OpeningAuction,
    Continuous,
    LunchBreak,
    Close,
    PostClose,
}

/// One checked source ranking entry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarketRankingEntry {
    kind: MarketRankingKind,
    rank: PositiveU32,
    instrument: Option<InstrumentId>,
    label: NonEmptyText,
    value: FiniteNumber,
    unit: MarketRankingUnit,
    source_date: IsoDate,
    source_session: MarketSession,
    universe: NonEmptyText,
    universe_size: PositiveU32,
    covered_count: PositiveU32,
    max_source_skew_millis: u64,
    evidence: SourceEvidence,
}

impl MarketRankingEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: MarketRankingKind,
        rank: PositiveU32,
        instrument: Option<InstrumentId>,
        label: NonEmptyText,
        value: FiniteNumber,
        unit: MarketRankingUnit,
        source_date: IsoDate,
        source_session: MarketSession,
        universe: NonEmptyText,
        universe_size: PositiveU32,
        covered_count: PositiveU32,
        max_source_skew_millis: u64,
        evidence: SourceEvidence,
    ) -> Result<Self, crate::CoreError> {
        validate_ranking_unit(&kind, &unit)?;
        if matches!(
            kind,
            MarketRankingKind::VolumeRatio
                | MarketRankingKind::MainNetInflow
                | MarketRankingKind::Popularity
        ) && instrument.is_none()
        {
            return Err(crate::CoreError::InvalidRequest(
                "instrument ranking requires both a security code and name".into(),
            ));
        }
        if matches!(kind, MarketRankingKind::VolumeRatio) && value.get().is_sign_negative() {
            return Err(crate::CoreError::InvalidRequest(
                "volume ratio must be non-negative".into(),
            ));
        }
        if covered_count.get() != universe_size.get() {
            return Err(crate::CoreError::InvalidRequest(
                "full-market ranking covered count must equal universe size".into(),
            ));
        }
        if rank.get() > covered_count.get() {
            return Err(crate::CoreError::InvalidRequest(
                "ranking rank must not exceed covered count".into(),
            ));
        }
        validate_evidence_date(&source_date, &evidence, "market ranking")?;
        Ok(Self {
            kind,
            rank,
            instrument,
            label,
            value,
            unit,
            source_date,
            source_session,
            universe,
            universe_size,
            covered_count,
            max_source_skew_millis,
            evidence,
        })
    }

    pub fn kind(&self) -> &MarketRankingKind {
        &self.kind
    }

    pub fn rank(&self) -> PositiveU32 {
        self.rank
    }

    pub fn instrument(&self) -> Option<&InstrumentId> {
        self.instrument.as_ref()
    }

    pub fn label(&self) -> &NonEmptyText {
        &self.label
    }

    pub fn value(&self) -> FiniteNumber {
        self.value
    }

    pub fn unit(&self) -> &MarketRankingUnit {
        &self.unit
    }

    pub fn source_date(&self) -> &IsoDate {
        &self.source_date
    }

    pub fn source_session(&self) -> MarketSession {
        self.source_session
    }

    pub fn universe(&self) -> &NonEmptyText {
        &self.universe
    }

    pub fn universe_size(&self) -> PositiveU32 {
        self.universe_size
    }

    pub fn covered_count(&self) -> PositiveU32 {
        self.covered_count
    }

    pub fn coverage_ratio(&self) -> Ratio {
        Ratio::decimal(self.covered_count.get() as f64 / self.universe_size.get() as f64)
            .expect("positive integer coverage is finite")
    }

    pub fn max_source_skew_millis(&self) -> u64 {
        self.max_source_skew_millis
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

#[derive(Deserialize)]
struct MarketRankingEntryWire {
    kind: MarketRankingKind,
    rank: PositiveU32,
    instrument: Option<InstrumentId>,
    label: NonEmptyText,
    value: FiniteNumber,
    unit: MarketRankingUnit,
    source_date: IsoDate,
    source_session: MarketSession,
    universe: NonEmptyText,
    universe_size: PositiveU32,
    covered_count: PositiveU32,
    max_source_skew_millis: u64,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for MarketRankingEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MarketRankingEntryWire::deserialize(deserializer)?;
        Self::new(
            wire.kind,
            wire.rank,
            wire.instrument,
            wire.label,
            wire.value,
            wire.unit,
            wire.source_date,
            wire.source_session,
            wire.universe,
            wire.universe_size,
            wire.covered_count,
            wire.max_source_skew_millis,
            wire.evidence,
        )
        .map_err(de::Error::custom)
    }
}

fn validate_ranking_unit(
    kind: &MarketRankingKind,
    unit: &MarketRankingUnit,
) -> Result<(), crate::CoreError> {
    let valid = matches!(
        (kind, unit),
        (MarketRankingKind::VolumeRatio, MarketRankingUnit::Multiple)
            | (MarketRankingKind::MainNetInflow, MarketRankingUnit::Yuan)
            | (
                MarketRankingKind::Industry
                    | MarketRankingKind::Concept
                    | MarketRankingKind::Region,
                MarketRankingUnit::Percent
            )
            | (MarketRankingKind::Popularity, MarketRankingUnit::Score)
            | (MarketRankingKind::Custom(_), MarketRankingUnit::Custom(_))
    );
    if !valid {
        return Err(crate::CoreError::InvalidRequest(
            "market ranking metric and unit are inconsistent".into(),
        ));
    }
    Ok(())
}

fn validate_evidence_date(
    source_date: &IsoDate,
    evidence: &SourceEvidence,
    context: &str,
) -> Result<(), crate::CoreError> {
    let source_at = evidence.source_at().ok_or_else(|| {
        crate::CoreError::InvalidRequest(format!("{context} evidence must include source_at"))
    })?;
    if source_at.get(..10) != Some(source_date.as_str())
        || !matches!(source_at.as_bytes().get(10), None | Some(b'T') | Some(b' '))
    {
        return Err(crate::CoreError::InvalidRequest(format!(
            "{context} source date does not match evidence source_at"
        )));
    }
    Ok(())
}

/// Checks batch-level full-market ranking invariants that cannot be established
/// by validating one record in isolation.
pub fn validate_market_ranking_batch(
    records: &[MarketRankingEntry],
    kind: &MarketRankingKind,
    limit: PositiveU32,
) -> Result<(), crate::CoreError> {
    let first = records.first().ok_or_else(|| {
        crate::CoreError::InvalidRequest("market ranking must contain ranked records".into())
    })?;
    if first.kind() != kind {
        return Err(crate::CoreError::InvalidRequest(
            "market ranking kind does not match the request".into(),
        ));
    }
    if first.covered_count() != first.universe_size() {
        return Err(crate::CoreError::InvalidRequest(
            "market ranking does not cover the complete declared universe".into(),
        ));
    }
    let expected_len = usize::try_from(limit.get().min(first.universe_size().get()))
        .map_err(|_| crate::CoreError::InvalidRequest("market ranking limit overflow".into()))?;
    if records.len() != expected_len {
        return Err(crate::CoreError::InvalidRequest(format!(
            "market ranking returned {} records but exactly {expected_len} are required",
            records.len()
        )));
    }
    let mut instruments = HashSet::with_capacity(records.len());
    let mut previous = None;
    for (index, record) in records.iter().enumerate() {
        let expected_rank = u32::try_from(index + 1)
            .map_err(|_| crate::CoreError::InvalidRequest("market rank overflow".into()))?;
        if record.kind() != kind
            || record.rank().get() != expected_rank
            || record.source_date() != first.source_date()
            || record.source_session() != first.source_session()
            || record.universe() != first.universe()
            || record.universe_size() != first.universe_size()
            || record.covered_count() != first.covered_count()
            || record.max_source_skew_millis() != first.max_source_skew_millis()
            || record.evidence.provider() != first.evidence.provider()
            || record.evidence.batch_id() != first.evidence.batch_id()
        {
            return Err(crate::CoreError::InvalidRequest(
                "market ranking records do not share one continuous atomic ranking context".into(),
            ));
        }
        if let Some(instrument) = record.instrument() {
            if !instruments.insert(instrument.clone()) {
                return Err(crate::CoreError::InvalidRequest(
                    "market ranking contains duplicate instruments".into(),
                ));
            }
        }
        if previous.is_some_and(|value| value < record.value().get()) {
            return Err(crate::CoreError::InvalidRequest(
                "market ranking values must be in descending source order".into(),
            ));
        }
        previous = Some(record.value().get());
    }
    Ok(())
}

/// Explicit market-breadth request over a named universe and source window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarketBreadthRequest {
    universe: NonEmptyText,
    source_date: IsoDate,
    source_session: MarketSession,
    minimum_coverage: Ratio,
    maximum_source_skew_millis: u64,
}

impl MarketBreadthRequest {
    pub fn new(
        universe: NonEmptyText,
        source_date: IsoDate,
        source_session: MarketSession,
        minimum_coverage: Ratio,
        maximum_source_skew_millis: u64,
    ) -> Result<Self, crate::CoreError> {
        if minimum_coverage.unit() != RatioUnit::Decimal
            || !(0.0..=1.0).contains(&minimum_coverage.get())
        {
            return Err(crate::CoreError::InvalidRequest(
                "breadth minimum coverage must be a decimal ratio in 0..=1".into(),
            ));
        }
        Ok(Self {
            universe,
            source_date,
            source_session,
            minimum_coverage,
            maximum_source_skew_millis,
        })
    }

    pub fn universe(&self) -> &NonEmptyText {
        &self.universe
    }

    pub fn source_date(&self) -> &IsoDate {
        &self.source_date
    }

    pub fn source_session(&self) -> MarketSession {
        self.source_session
    }

    pub fn minimum_coverage(&self) -> Ratio {
        self.minimum_coverage
    }

    pub fn maximum_source_skew_millis(&self) -> u64 {
        self.maximum_source_skew_millis
    }
}

#[derive(Deserialize)]
struct MarketBreadthRequestWire {
    universe: NonEmptyText,
    source_date: IsoDate,
    source_session: MarketSession,
    minimum_coverage: Ratio,
    maximum_source_skew_millis: u64,
}

impl<'de> Deserialize<'de> for MarketBreadthRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MarketBreadthRequestWire::deserialize(deserializer)?;
        Self::new(
            wire.universe,
            wire.source_date,
            wire.source_session,
            wire.minimum_coverage,
            wire.maximum_source_skew_millis,
        )
        .map_err(de::Error::custom)
    }
}

/// Breadth is a checked aggregate, not a synthetic ranked security.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarketBreadthSnapshot {
    universe: NonEmptyText,
    source_date: IsoDate,
    source_session: MarketSession,
    total: u32,
    valid: u32,
    up: u32,
    down: u32,
    flat: u32,
    limit_up: u32,
    limit_down: u32,
    coverage: Ratio,
    max_source_skew_millis: u64,
    input_evidence: Vec<SourceEvidence>,
    evidence: SourceEvidence,
}

impl MarketBreadthSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        universe: NonEmptyText,
        source_date: IsoDate,
        source_session: MarketSession,
        total: u32,
        valid: u32,
        up: u32,
        down: u32,
        flat: u32,
        limit_up: u32,
        limit_down: u32,
        max_source_skew_millis: u64,
        input_evidence: Vec<SourceEvidence>,
        evidence: SourceEvidence,
    ) -> Result<Self, crate::CoreError> {
        if total == 0 || valid > total {
            return Err(crate::CoreError::InvalidRequest(
                "breadth total must be positive and valid must not exceed total".into(),
            ));
        }
        let partition_total = up
            .checked_add(down)
            .and_then(|value| value.checked_add(flat))
            .ok_or_else(|| {
                crate::CoreError::InvalidRequest("breadth directional count overflow".into())
            })?;
        if valid != partition_total {
            return Err(crate::CoreError::InvalidRequest(
                "breadth valid count must equal up + down + flat".into(),
            ));
        }
        if limit_up > up || limit_down > down {
            return Err(crate::CoreError::InvalidRequest(
                "breadth limit counts must be subsets of directional counts".into(),
            ));
        }
        if input_evidence.is_empty() {
            return Err(crate::CoreError::InvalidRequest(
                "breadth must retain at least one input evidence record".into(),
            ));
        }
        let mut evidence_ids = HashSet::with_capacity(input_evidence.len());
        for input in &input_evidence {
            if !evidence_ids.insert((input.provider(), input.batch_id().to_owned())) {
                return Err(crate::CoreError::InvalidRequest(
                    "breadth input evidence contains a duplicate provider/batch pair".into(),
                ));
            }
            validate_evidence_date(&source_date, input, "breadth input")?;
        }
        validate_evidence_date(&source_date, &evidence, "breadth")?;
        let coverage = Ratio::decimal(valid as f64 / total as f64)?;
        Ok(Self {
            universe,
            source_date,
            source_session,
            total,
            valid,
            up,
            down,
            flat,
            limit_up,
            limit_down,
            coverage,
            max_source_skew_millis,
            input_evidence,
            evidence,
        })
    }

    pub fn universe(&self) -> &NonEmptyText {
        &self.universe
    }
    pub fn source_date(&self) -> &IsoDate {
        &self.source_date
    }
    pub fn source_session(&self) -> MarketSession {
        self.source_session
    }
    pub fn total(&self) -> u32 {
        self.total
    }
    pub fn valid(&self) -> u32 {
        self.valid
    }
    pub fn up(&self) -> u32 {
        self.up
    }
    pub fn down(&self) -> u32 {
        self.down
    }
    pub fn flat(&self) -> u32 {
        self.flat
    }
    pub fn limit_up(&self) -> u32 {
        self.limit_up
    }
    pub fn limit_down(&self) -> u32 {
        self.limit_down
    }
    pub fn coverage(&self) -> Ratio {
        self.coverage
    }
    pub fn max_source_skew_millis(&self) -> u64 {
        self.max_source_skew_millis
    }
    pub fn input_evidence(&self) -> &[SourceEvidence] {
        &self.input_evidence
    }
    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

#[derive(Deserialize)]
struct MarketBreadthSnapshotWire {
    universe: NonEmptyText,
    source_date: IsoDate,
    source_session: MarketSession,
    total: u32,
    valid: u32,
    up: u32,
    down: u32,
    flat: u32,
    limit_up: u32,
    limit_down: u32,
    coverage: Ratio,
    max_source_skew_millis: u64,
    input_evidence: Vec<SourceEvidence>,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for MarketBreadthSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MarketBreadthSnapshotWire::deserialize(deserializer)?;
        let snapshot = Self::new(
            wire.universe,
            wire.source_date,
            wire.source_session,
            wire.total,
            wire.valid,
            wire.up,
            wire.down,
            wire.flat,
            wire.limit_up,
            wire.limit_down,
            wire.max_source_skew_millis,
            wire.input_evidence,
            wire.evidence,
        )
        .map_err(de::Error::custom)?;
        if snapshot.coverage != wire.coverage {
            return Err(de::Error::custom(
                "breadth serialized coverage contradicts valid / total",
            ));
        }
        Ok(snapshot)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PopularityRank {
    pub instrument: InstrumentId,
    pub rank: PositiveU32,
    pub price: Option<Price>,
    pub name: Option<NonEmptyText>,
    pub rank_change: Option<FiniteNumber>,
    pub return_ratio: Option<Ratio>,
    pub heat: Option<FiniteNumber>,
    #[serde(default)]
    pub concepts: Vec<NonEmptyText>,
    pub tag: Option<NonEmptyText>,
    /// Evidence for an optional second-source quote join.
    pub quote_evidence: Option<SourceEvidence>,
    /// Evidence for the ranking response itself.
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConceptHit {
    pub instrument: InstrumentId,
    pub concept: NonEmptyText,
    pub detail: Option<NonEmptyText>,
    pub evidence: SourceEvidence,
}

macro_rules! impl_sourced {
    ($($record:ty),+ $(,)?) => {
        $(
            impl SourcedRecord for $record {
                fn provider_id(&self) -> crate::ProviderId {
                    self.evidence.provider()
                }

                fn evidence_batch_id(&self) -> &str {
                    self.evidence.batch_id()
                }
            }
        )+
    };
}

impl_sourced!(
    BoardMembership,
    StrongStockReason,
    DragonTigerEntry,
    DragonTigerSeat,
    MarketRankingEntry,
    MarketBreadthSnapshot,
    PopularityRank,
    ConceptHit,
);

impl SourcedRecord for DragonTigerDisclosure {
    fn provider_id(&self) -> crate::ProviderId {
        self.entry.evidence().provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.entry.evidence().batch_id()
    }
}

/// Bounded whole-market dragon-tiger request for one trading date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarketDragonTigerRequest {
    trading_date: IsoDate,
    limit: PositiveU32,
}

impl MarketDragonTigerRequest {
    pub fn new(trading_date: IsoDate, limit: PositiveU32) -> Result<Self, crate::CoreError> {
        if limit.get() > 100 {
            return Err(crate::CoreError::InvalidRequest(
                "market dragon-tiger limit must be at most 100".into(),
            ));
        }
        Ok(Self {
            trading_date,
            limit,
        })
    }

    pub fn trading_date(&self) -> &IsoDate {
        &self.trading_date
    }

    pub fn limit(&self) -> PositiveU32 {
        self.limit
    }
}

#[derive(Deserialize)]
struct MarketDragonTigerRequestWire {
    trading_date: IsoDate,
    limit: PositiveU32,
}

impl<'de> Deserialize<'de> for MarketDragonTigerRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MarketDragonTigerRequestWire::deserialize(deserializer)?;
        Self::new(wire.trading_date, wire.limit).map_err(de::Error::custom)
    }
}

/// Bounded request for an instrument signal family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstrumentSignalRequest {
    instrument: InstrumentId,
    trading_date: Option<IsoDate>,
    limit: PositiveU32,
}

impl InstrumentSignalRequest {
    pub fn new(instrument: InstrumentId, limit: PositiveU32) -> Result<Self, crate::CoreError> {
        if limit.get() > 10_000 {
            return Err(crate::CoreError::InvalidRequest(
                "signal limit must be at most 10000".into(),
            ));
        }
        Ok(Self {
            instrument,
            trading_date: None,
            limit,
        })
    }

    pub fn with_trading_date(mut self, trading_date: IsoDate) -> Self {
        self.trading_date = Some(trading_date);
        self
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn trading_date(&self) -> Option<&IsoDate> {
        self.trading_date.as_ref()
    }

    pub fn limit(&self) -> PositiveU32 {
        self.limit
    }
}

#[derive(Deserialize)]
struct InstrumentSignalRequestWire {
    instrument: InstrumentId,
    trading_date: Option<IsoDate>,
    limit: PositiveU32,
}

impl<'de> Deserialize<'de> for InstrumentSignalRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = InstrumentSignalRequestWire::deserialize(deserializer)?;
        let mut request = Self::new(wire.instrument, wire.limit).map_err(de::Error::custom)?;
        if let Some(trading_date) = wire.trading_date {
            request = request.with_trading_date(trading_date);
        }
        Ok(request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SignalCapabilities {
    pub board_memberships: bool,
    pub strong_stock_reasons: bool,
    pub dragon_tiger: bool,
    pub market_rankings: bool,
    pub popularity: bool,
    pub concept_hits: bool,
}

pub trait BoardMembershipProvider {
    type Error: std::error::Error + Send + Sync + 'static;
    fn board_memberships(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<BoardMembership>, Self::Error>;
}

pub trait StrongStockReasons {
    type Error: std::error::Error + Send + Sync + 'static;
    fn strong_stock_reasons(
        &self,
        request: &InstrumentSignalRequest,
    ) -> Result<DataBatch<StrongStockReason>, Self::Error>;
}

pub trait DragonTigerData {
    type Error: std::error::Error + Send + Sync + 'static;
    fn dragon_tiger_entries(
        &self,
        request: &InstrumentSignalRequest,
    ) -> Result<DataBatch<DragonTigerEntry>, Self::Error>;
    fn dragon_tiger_seats(
        &self,
        request: &InstrumentSignalRequest,
    ) -> Result<DataBatch<DragonTigerSeat>, Self::Error>;
}

pub trait MarketDragonTigerData {
    type Error: std::error::Error + Send + Sync + 'static;
    fn market_dragon_tiger(
        &self,
        request: &MarketDragonTigerRequest,
    ) -> Result<DataBatch<DragonTigerDisclosure>, Self::Error>;
}

pub trait MarketRankings {
    type Error: std::error::Error + Send + Sync + 'static;
    fn market_rankings(
        &self,
        kind: &MarketRankingKind,
        limit: PositiveU32,
    ) -> Result<DataBatch<MarketRankingEntry>, Self::Error>;
}

pub trait MarketBreadth {
    type Error: std::error::Error + Send + Sync + 'static;

    fn market_breadth(
        &self,
        request: &MarketBreadthRequest,
    ) -> Result<DataBatch<MarketBreadthSnapshot>, Self::Error>;
}

pub trait PopularityData {
    type Error: std::error::Error + Send + Sync + 'static;
    fn popularity(&self, limit: PositiveU32) -> Result<DataBatch<PopularityRank>, Self::Error>;
}

pub trait ConceptHits {
    type Error: std::error::Error + Send + Sync + 'static;
    fn concept_hits(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<ConceptHit>, Self::Error>;
}
