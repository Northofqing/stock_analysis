use crate::{
    DataBatch, FiniteNumber, InstrumentId, IsoDate, Money, NonEmptyText, Price, Quantity, Ratio,
    SourceEvidence, SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};

/// Exchange contract month in canonical `YYYY-MM` form.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ContractMonth(String);

impl ContractMonth {
    pub fn new(value: impl Into<String>) -> Result<Self, crate::CoreError> {
        let value = value.into();
        if value.len() != 7
            || value.as_bytes().get(4) != Some(&b'-')
            || !value
                .bytes()
                .enumerate()
                .all(|(index, byte)| index == 4 || byte.is_ascii_digit())
        {
            return Err(crate::CoreError::InvalidValue {
                field: "contract_month",
                value,
                reason: "must use YYYY-MM",
            });
        }
        IsoDate::new(format!("{value}-01"))?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ContractMonth {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptionKind {
    Call,
    Put,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "OptionContractWire")]
pub struct OptionContract {
    pub contract_code: NonEmptyText,
    pub underlying: InstrumentId,
    pub expiry_month: ContractMonth,
    pub expiry: Option<IsoDate>,
    pub kind: OptionKind,
    pub strike: Option<Price>,
    pub evidence: SourceEvidence,
}

impl OptionContract {
    fn validate(self) -> Result<Self, crate::CoreError> {
        if let Some(expiry) = self.expiry.as_ref() {
            if &expiry.as_str()[..7] != self.expiry_month.as_str() {
                return Err(crate::CoreError::InvalidRequest(format!(
                    "option expiry {} does not match contract month {}",
                    expiry,
                    self.expiry_month.as_str()
                )));
            }
        }
        Ok(self)
    }
}

#[derive(Deserialize)]
struct OptionContractWire {
    contract_code: NonEmptyText,
    underlying: InstrumentId,
    expiry_month: ContractMonth,
    expiry: Option<IsoDate>,
    kind: OptionKind,
    strike: Option<Price>,
    evidence: SourceEvidence,
}

impl TryFrom<OptionContractWire> for OptionContract {
    type Error = crate::CoreError;

    fn try_from(value: OptionContractWire) -> Result<Self, Self::Error> {
        Self {
            contract_code: value.contract_code,
            underlying: value.underlying,
            expiry_month: value.expiry_month,
            expiry: value.expiry,
            kind: value.kind,
            strike: value.strike,
            evidence: value.evidence,
        }
        .validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "OptionQuoteWire")]
pub struct OptionQuote {
    pub contract_code: NonEmptyText,
    pub name: Option<NonEmptyText>,
    pub bid: Option<Price>,
    pub bid_quantity: Option<Quantity>,
    pub ask: Option<Price>,
    pub ask_quantity: Option<Quantity>,
    pub last: Option<Price>,
    pub previous_close: Option<Price>,
    pub open: Option<Price>,
    pub high: Option<Price>,
    pub low: Option<Price>,
    pub upper_limit: Option<Price>,
    pub lower_limit: Option<Price>,
    pub strike: Option<Price>,
    pub volume: Option<Quantity>,
    pub open_interest: Option<Quantity>,
    pub amount: Option<Money>,
    pub change: Option<Ratio>,
    pub amplitude: Option<Ratio>,
    pub quote_at: Option<NonEmptyText>,
    pub evidence: SourceEvidence,
}

fn validate_level(
    label: &str,
    price: Option<Price>,
    quantity: Option<Quantity>,
) -> Result<(), crate::CoreError> {
    if price.is_some() != quantity.is_some() {
        return Err(crate::CoreError::InvalidRequest(format!(
            "option {label} price and quantity must be present together"
        )));
    }
    Ok(())
}

fn valid_clock(value: &str) -> bool {
    value.len() == 8
        && value.as_bytes()[2] == b':'
        && value.as_bytes()[5] == b':'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 2 || index == 5 || byte.is_ascii_digit())
        && value[0..2].parse::<u8>().is_ok_and(|hour| hour < 24)
        && value[3..5].parse::<u8>().is_ok_and(|minute| minute < 60)
        && value[6..8].parse::<u8>().is_ok_and(|second| second < 60)
}

fn valid_offset(value: &str) -> bool {
    value == "Z"
        || (value.len() == 6
            && matches!(value.as_bytes()[0], b'+' | b'-')
            && value.as_bytes()[3] == b':'
            && value[1..3].parse::<u8>().is_ok_and(|hour| hour <= 23)
            && value[4..6].parse::<u8>().is_ok_and(|minute| minute < 60))
}

fn valid_quote_timestamp(value: &str) -> bool {
    value.is_ascii()
        && value.len() >= 20
        && value.as_bytes()[10] == b'T'
        && IsoDate::new(&value[..10]).is_ok()
        && valid_clock(&value[11..19])
        && valid_offset(&value[19..])
}

impl OptionQuote {
    fn validate(self) -> Result<Self, crate::CoreError> {
        validate_level("bid", self.bid, self.bid_quantity)?;
        validate_level("ask", self.ask, self.ask_quantity)?;
        for (left_name, left, right_name, right) in [
            ("bid", self.bid, "ask", self.ask),
            ("low", self.low, "high", self.high),
            (
                "lower limit",
                self.lower_limit,
                "upper limit",
                self.upper_limit,
            ),
        ] {
            if let (Some(left), Some(right)) = (left, right) {
                if left.get() > right.get() {
                    return Err(crate::CoreError::InvalidRequest(format!(
                        "option {left_name} must not exceed {right_name}"
                    )));
                }
            }
        }
        if self.amount.is_some_and(|amount| amount.get() < 0.0) {
            return Err(crate::CoreError::InvalidRequest(
                "option amount must be non-negative".into(),
            ));
        }
        if self
            .amplitude
            .is_some_and(|amplitude| amplitude.get() < 0.0)
        {
            return Err(crate::CoreError::InvalidRequest(
                "option amplitude must be non-negative".into(),
            ));
        }
        if self
            .quote_at
            .as_ref()
            .is_some_and(|value| !valid_quote_timestamp(value.as_str()))
        {
            return Err(crate::CoreError::InvalidRequest(
                "option quote_at must use a valid ISO-8601 second timestamp with offset".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Deserialize)]
struct OptionQuoteWire {
    contract_code: NonEmptyText,
    name: Option<NonEmptyText>,
    bid: Option<Price>,
    bid_quantity: Option<Quantity>,
    ask: Option<Price>,
    ask_quantity: Option<Quantity>,
    last: Option<Price>,
    previous_close: Option<Price>,
    open: Option<Price>,
    high: Option<Price>,
    low: Option<Price>,
    upper_limit: Option<Price>,
    lower_limit: Option<Price>,
    strike: Option<Price>,
    volume: Option<Quantity>,
    open_interest: Option<Quantity>,
    amount: Option<Money>,
    change: Option<Ratio>,
    amplitude: Option<Ratio>,
    quote_at: Option<NonEmptyText>,
    evidence: SourceEvidence,
}

impl TryFrom<OptionQuoteWire> for OptionQuote {
    type Error = crate::CoreError;

    fn try_from(value: OptionQuoteWire) -> Result<Self, Self::Error> {
        Self {
            contract_code: value.contract_code,
            name: value.name,
            bid: value.bid,
            bid_quantity: value.bid_quantity,
            ask: value.ask,
            ask_quantity: value.ask_quantity,
            last: value.last,
            previous_close: value.previous_close,
            open: value.open,
            high: value.high,
            low: value.low,
            upper_limit: value.upper_limit,
            lower_limit: value.lower_limit,
            strike: value.strike,
            volume: value.volume,
            open_interest: value.open_interest,
            amount: value.amount,
            change: value.change,
            amplitude: value.amplitude,
            quote_at: value.quote_at,
            evidence: value.evidence,
        }
        .validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "OptionGreeksWire")]
pub struct OptionGreeks {
    pub contract_code: NonEmptyText,
    pub name: Option<NonEmptyText>,
    pub volume: Option<Quantity>,
    pub delta: Option<FiniteNumber>,
    pub gamma: Option<FiniteNumber>,
    pub theta: Option<FiniteNumber>,
    pub vega: Option<FiniteNumber>,
    pub rho: Option<FiniteNumber>,
    pub implied_volatility: Option<FiniteNumber>,
    pub high: Option<Price>,
    pub low: Option<Price>,
    pub trade_code: Option<NonEmptyText>,
    pub strike: Option<Price>,
    pub last: Option<Price>,
    pub theoretical_price: Option<Price>,
    pub evidence: SourceEvidence,
}

impl OptionGreeks {
    fn validate(self) -> Result<Self, crate::CoreError> {
        if self
            .delta
            .is_some_and(|value| !(-1.0..=1.0).contains(&value.get()))
        {
            return Err(crate::CoreError::InvalidRequest(
                "option delta must be between -1 and 1".into(),
            ));
        }
        for (name, value) in [
            ("gamma", self.gamma),
            ("vega", self.vega),
            ("implied volatility", self.implied_volatility),
        ] {
            if value.is_some_and(|value| value.get() < 0.0) {
                return Err(crate::CoreError::InvalidRequest(format!(
                    "option {name} must be non-negative"
                )));
            }
        }
        if let (Some(low), Some(high)) = (self.low, self.high) {
            if low.get() > high.get() {
                return Err(crate::CoreError::InvalidRequest(
                    "option Greek low must not exceed high".into(),
                ));
            }
        }
        Ok(self)
    }
}

#[derive(Deserialize)]
struct OptionGreeksWire {
    contract_code: NonEmptyText,
    name: Option<NonEmptyText>,
    volume: Option<Quantity>,
    delta: Option<FiniteNumber>,
    gamma: Option<FiniteNumber>,
    theta: Option<FiniteNumber>,
    vega: Option<FiniteNumber>,
    rho: Option<FiniteNumber>,
    implied_volatility: Option<FiniteNumber>,
    high: Option<Price>,
    low: Option<Price>,
    trade_code: Option<NonEmptyText>,
    strike: Option<Price>,
    last: Option<Price>,
    theoretical_price: Option<Price>,
    evidence: SourceEvidence,
}

impl TryFrom<OptionGreeksWire> for OptionGreeks {
    type Error = crate::CoreError;

    fn try_from(value: OptionGreeksWire) -> Result<Self, Self::Error> {
        Self {
            contract_code: value.contract_code,
            name: value.name,
            volume: value.volume,
            delta: value.delta,
            gamma: value.gamma,
            theta: value.theta,
            vega: value.vega,
            rho: value.rho,
            implied_volatility: value.implied_volatility,
            high: value.high,
            low: value.low,
            trade_code: value.trade_code,
            strike: value.strike,
            last: value.last,
            theoretical_price: value.theoretical_price,
            evidence: value.evidence,
        }
        .validate()
    }
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

impl_sourced!(OptionContract, OptionQuote, OptionGreeks);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OptionCapabilities {
    pub contract_discovery: bool,
    pub quotes: bool,
    pub greeks: bool,
}

pub trait OptionData {
    type Error: std::error::Error + Send + Sync + 'static;
    fn option_contracts(
        &self,
        underlying: &InstrumentId,
        expiry: Option<&ContractMonth>,
    ) -> Result<DataBatch<OptionContract>, Self::Error>;
    fn option_quotes(
        &self,
        contracts: &[NonEmptyText],
    ) -> Result<DataBatch<OptionQuote>, Self::Error>;
    fn option_greeks(
        &self,
        contracts: &[NonEmptyText],
    ) -> Result<DataBatch<OptionGreeks>, Self::Error>;
}
