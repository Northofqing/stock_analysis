//! Explicit, modeled Shanghai A-share fee policy for future ledger generations.
//! This module does not attest an exchange classification or a broker receipt.

use crate::trading::paper_replay_financial_work_v1 as fw;
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use sha2::{Digest, Sha256};

use super::fee_evidence::{FillSide, A_SHARE_FEE_SCHEDULE_V2};

const DESCRIPTOR_SCHEMA: &str = "a-share-fee-policy-descriptor/v1";
const FIRST_SUPPORTED_DATE: &str = "2008-09-19";
const DATE_BASIS: &str = "executed_fill_at_asia_shanghai_date";
pub const DEFAULT_FEE_SOURCE_REVISION: &str =
    "adr-0001-2026-09-28+sse-fees-reviewed-2026-09-29+mof-2023-39";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeeMarket {
    Shanghai,
    Shenzhen,
    Beijing,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeeSecurityKind {
    AShareStock,
    BShareStock,
    Fund,
    Bond,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeeListingSegment {
    ShanghaiMainA,
    ShanghaiStarA,
    Other,
}

impl FeeListingSegment {
    fn token(self) -> &'static str {
        match self {
            Self::ShanghaiMainA => "ShanghaiMainA",
            Self::ShanghaiStarA => "ShanghaiStarA",
            Self::Other => "Other",
        }
    }
}

/// An explicit scope assertion supplied by an upstream qualified instrument
/// source. This constructor checks the supported combination; it does not
/// infer eligibility from a ticker or verify the upstream source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualifiedInstrument {
    market: FeeMarket,
    security_kind: FeeSecurityKind,
    segment: FeeListingSegment,
}

impl QualifiedInstrument {
    pub fn new(
        market: FeeMarket,
        security_kind: FeeSecurityKind,
        segment: FeeListingSegment,
    ) -> Result<Self, AShareFeeV2Error> {
        if market != FeeMarket::Shanghai
            || security_kind != FeeSecurityKind::AShareStock
            || !matches!(
                segment,
                FeeListingSegment::ShanghaiMainA | FeeListingSegment::ShanghaiStarA
            )
        {
            return Err(AShareFeeV2Error::UnsupportedInstrument);
        }
        Ok(Self {
            market,
            security_kind,
            segment,
        })
    }

    pub fn segment(self) -> FeeListingSegment {
        self.segment
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeRate {
    numerator: i64,
    denominator: i64,
}

impl FeeRate {
    pub fn new(numerator: i64, denominator: i64) -> Result<Self, AShareFeeV2Error> {
        if numerator < 0 || denominator <= 0 {
            return Err(AShareFeeV2Error::InvalidCommissionRate);
        }
        let divisor = gcd(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcludedFeeReason {
    Unmodeled,
    Unverified,
}

impl ExcludedFeeReason {
    fn token(self) -> &'static str {
        match self {
            Self::Unmodeled => "excluded_unmodeled",
            Self::Unverified => "excluded_unverified",
        }
    }
}

/// Commission and stamp tax are modeled. Transfer and other charges are
/// excluded; differing exclusion evidence gives a differing policy identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeCoverage {
    transfer_fee: ExcludedFeeReason,
    other_charges: ExcludedFeeReason,
}

impl FeeCoverage {
    pub fn new(transfer_fee: ExcludedFeeReason, other_charges: ExcludedFeeReason) -> Self {
        Self {
            transfer_fee,
            other_charges,
        }
    }

    pub fn initial_model() -> Self {
        Self::new(ExcludedFeeReason::Unmodeled, ExcludedFeeReason::Unverified)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeeCoverageRequirement {
    ModeledComponentsOnly,
    CompleteTradingCost,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AShareFeeV2Error {
    #[error("Shanghai A-share stock fee policy does not support this instrument")]
    UnsupportedInstrument,
    #[error("fee instrument scope does not match policy scope")]
    ScopeMismatch,
    #[error("complete trading cost is unavailable: transfer and other charges are excluded")]
    UnsupportedCoverage,
    #[error("A-share fee schedule v2 requires positive micro-CNY notional")]
    InvalidNotional,
    #[error("A-share fee schedule v2 has no authority before 2008-09-19")]
    UnsupportedTradeDate,
    #[error("commission rate must be nonnegative with a positive denominator")]
    InvalidCommissionRate,
    #[error("commission minimum must be nonnegative")]
    InvalidCommissionMinimum,
    #[error("source revision must be 1..128 ASCII token characters")]
    InvalidSourceRevision,
    #[error("A-share fee schedule v2 amount overflow")]
    Overflow,
}

/// Immutable policy descriptor; all fields in the canonical bytes participate
/// in its domain-separated SHA-256 instance identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AShareFeePolicyV2 {
    scope: QualifiedInstrument,
    commission_rate: FeeRate,
    commission_minimum_micro_cny: i64,
    coverage: FeeCoverage,
    source_revision: String,
}

impl AShareFeePolicyV2 {
    pub fn new(
        scope: QualifiedInstrument,
        commission_rate: FeeRate,
        commission_minimum_micro_cny: i64,
        coverage: FeeCoverage,
        source_revision: impl Into<String>,
    ) -> Result<Self, AShareFeeV2Error> {
        Self::validate_minimum(commission_minimum_micro_cny)?;
        Self::from_owned_revision(
            scope,
            commission_rate,
            commission_minimum_micro_cny,
            coverage,
            source_revision.into(),
        )
    }
    fn validate_minimum(value: i64) -> Result<(), AShareFeeV2Error> {
        if value < 0 {
            Err(AShareFeeV2Error::InvalidCommissionMinimum)
        } else {
            Ok(())
        }
    }
    fn from_owned_revision(
        scope: QualifiedInstrument,
        commission_rate: FeeRate,
        commission_minimum_micro_cny: i64,
        coverage: FeeCoverage,
        source_revision: String,
    ) -> Result<Self, AShareFeeV2Error> {
        if source_revision.is_empty()
            || source_revision.len() > 128
            || !source_revision
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
        {
            return Err(AShareFeeV2Error::InvalidSourceRevision);
        }
        Ok(Self {
            scope,
            commission_rate,
            commission_minimum_micro_cny,
            coverage,
            source_revision,
        })
    }

    /// Fixed compatibility assumption. New research and paper callers should
    /// construct and pass their own reviewed descriptor explicitly.
    pub fn fixed_compatibility_assumption() -> Self {
        Self::new(
            QualifiedInstrument::new(
                FeeMarket::Shanghai,
                FeeSecurityKind::AShareStock,
                FeeListingSegment::ShanghaiMainA,
            )
            .expect("fixed Shanghai stock scope"),
            FeeRate::new(3, 10_000).expect("fixed commission rate"),
            5_000_000,
            FeeCoverage::initial_model(),
            DEFAULT_FEE_SOURCE_REVISION,
        )
        .expect("fixed descriptor")
    }

    pub fn scope(&self) -> QualifiedInstrument {
        self.scope
    }

    pub fn coverage(&self) -> FeeCoverage {
        self.coverage
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = fw::FinancialSink::Owned(Vec::new());
        self.write_replay_descriptor(&mut out)
            .expect("Historical descriptor writer");
        match out {
            fw::FinancialSink::Owned(bytes) => bytes,
            _ => unreachable!(),
        }
    }

    pub(crate) fn write_replay_descriptor(
        &self,
        out: &mut fw::FinancialSink<'_>,
    ) -> std::result::Result<(), ()> {
        out.bytes(b"schema=")?;
        out.bytes(DESCRIPTOR_SCHEMA.as_bytes())?;
        out.bytes(b"\nschedule=")?;
        out.bytes(A_SHARE_FEE_SCHEDULE_V2.as_bytes())?;
        out.bytes(b"\nmarket=Shanghai\ninstrument=AShareStock\nsegment=")?;
        out.bytes(self.scope.segment.token().as_bytes())?;
        out.bytes(b"\nfirst_supported_trade_date=")?;
        out.bytes(FIRST_SUPPORTED_DATE.as_bytes())?;
        out.bytes(b"\ndate_basis=")?;
        out.bytes(DATE_BASIS.as_bytes())?;
        out.bytes(b"\ncommission_kind=simulated_rate_with_per_fill_minimum\ncommission_rate_num=")?;
        out.signed(self.commission_rate.numerator)?;
        out.bytes(b"\ncommission_rate_den=")?;
        out.signed(self.commission_rate.denominator)?;
        out.bytes(b"\ncommission_minimum_micro_cny=")?;
        out.signed(self.commission_minimum_micro_cny)?;
        out.bytes(b"\ncommission_applies_to=buy_and_sell\nstamp_tax_buy_num=0\nstamp_tax_buy_den=1\nstamp_tax_sell_bracket_1=2008-09-19..2023-08-27:1/1000\nstamp_tax_sell_bracket_2=2023-08-28..:1/2000\nrounding=integer_half_up_micro_cny_per_component\ncoverage_commission=modeled_assumption\ncoverage_stamp_tax=modeled_policy\ncoverage_transfer_fee=")?;
        out.bytes(self.coverage.transfer_fee.token().as_bytes())?;
        out.bytes(b"\ncoverage_other_charges=")?;
        out.bytes(self.coverage.other_charges.token().as_bytes())?;
        out.bytes(b"\nsource_revision=")?;
        out.bytes(self.source_revision.as_bytes())?;
        out.bytes(b"\n")?;
        Ok(())
    }
    pub(crate) fn new_with_work(
        scope: QualifiedInstrument,
        rate: FeeRate,
        minimum: i64,
        coverage: FeeCoverage,
        revision: &str,
        w: &mut fw::FinancialWork<'_, '_>,
    ) -> fw::Result<Self> {
        w.finish()?;
        Self::validate_minimum(minimum)?;
        let owned = w.fee_source_revision(revision)?;
        Ok(Self::from_owned_revision(
            scope, rate, minimum, coverage, owned,
        )?)
    }
    pub fn descriptor_hash(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"a-share-fee-policy-descriptor/v1\n");
        digest.update(self.canonical_bytes());
        hex::encode(digest.finalize())
    }

    pub fn instance_id(&self) -> String {
        format!(
            "{A_SHARE_FEE_SCHEDULE_V2}:sha256:{}",
            self.descriptor_hash()
        )
    }
}

/// Date bracket is selected even for buys, whose stamp tax remains zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampTaxBracketV2 {
    SellerOnePerThousand,
    SellerHalfPerThousand,
}

impl StampTaxBracketV2 {
    pub fn id(self) -> &'static str {
        match self {
            Self::SellerOnePerThousand => "2008-09-19..2023-08-27:1/1000",
            Self::SellerHalfPerThousand => "2023-08-28..:1/2000",
        }
    }
}

/// Modeled components for one fill; a durable consumer must separately bind
/// its fill ID and retain the policy descriptor. This is not a receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AShareFillFeeV2 {
    pub basis_id: &'static str,
    pub policy_instance_id: String,
    pub scope: QualifiedInstrument,
    pub trade_date: NaiveDate,
    pub stamp_tax_bracket: StampTaxBracketV2,
    pub side: FillSide,
    pub notional_micro_cny: i64,
    pub commission_micro_cny: i64,
    pub stamp_tax_micro_cny: i64,
    pub total_micro_cny: i64,
    pub coverage: FeeCoverage,
}

pub fn shanghai_execution_date(executed_at_utc: DateTime<Utc>) -> NaiveDate {
    let shanghai = FixedOffset::east_opt(8 * 60 * 60).expect("Shanghai UTC offset exists");
    executed_at_utc.with_timezone(&shanghai).date_naive()
}

/// `trade_date` must be the actual fill date in Asia/Shanghai, not signal or
/// report date. The pure model deliberately does not validate exchange days.
pub fn a_share_stock_fill_fee_with_policy_v2(
    policy: &AShareFeePolicyV2,
    instrument: QualifiedInstrument,
    side: FillSide,
    notional_micro_cny: i64,
    trade_date: NaiveDate,
    required_coverage: FeeCoverageRequirement,
) -> Result<AShareFillFeeV2, AShareFeeV2Error> {
    fw::historical_fee(fill_fee_with_work(
        policy,
        instrument,
        side,
        notional_micro_cny,
        trade_date,
        required_coverage,
        &mut fw::FinancialWork::Historical,
    ))
}
pub(crate) fn fill_fee_with_work(
    policy: &AShareFeePolicyV2,
    instrument: QualifiedInstrument,
    side: FillSide,
    notional_micro_cny: i64,
    trade_date: NaiveDate,
    required_coverage: FeeCoverageRequirement,
    w: &mut fw::FinancialWork<'_, '_>,
) -> fw::Result<AShareFillFeeV2> {
    w.finish()?;
    if instrument != policy.scope {
        return Err(AShareFeeV2Error::ScopeMismatch.into());
    }
    if required_coverage == FeeCoverageRequirement::CompleteTradingCost {
        return Err(AShareFeeV2Error::UnsupportedCoverage.into());
    }
    if notional_micro_cny <= 0 {
        return Err(AShareFeeV2Error::InvalidNotional.into());
    }
    if trade_date < NaiveDate::from_ymd_opt(2008, 9, 19).expect("fixed date") {
        return Err(AShareFeeV2Error::UnsupportedTradeDate.into());
    }
    let stamp_tax_bracket =
        if trade_date < NaiveDate::from_ymd_opt(2023, 8, 28).expect("fixed date") {
            StampTaxBracketV2::SellerOnePerThousand
        } else {
            StampTaxBracketV2::SellerHalfPerThousand
        };
    let commission = rounded_rate_micro(notional_micro_cny, policy.commission_rate)?
        .max(policy.commission_minimum_micro_cny);
    let stamp_tax_micro_cny = if side == FillSide::Buy {
        0
    } else {
        let rate = match stamp_tax_bracket {
            StampTaxBracketV2::SellerOnePerThousand => FeeRate {
                numerator: 1,
                denominator: 1_000,
            },
            StampTaxBracketV2::SellerHalfPerThousand => FeeRate {
                numerator: 1,
                denominator: 2_000,
            },
        };
        rounded_rate_micro(notional_micro_cny, rate)?
    };
    Ok(AShareFillFeeV2 {
        basis_id: A_SHARE_FEE_SCHEDULE_V2,
        policy_instance_id: w.fee_instance(policy)?,
        scope: instrument,
        trade_date,
        stamp_tax_bracket,
        side,
        notional_micro_cny,
        commission_micro_cny: commission,
        stamp_tax_micro_cny,
        total_micro_cny: commission
            .checked_add(stamp_tax_micro_cny)
            .ok_or(AShareFeeV2Error::Overflow)?,
        coverage: policy.coverage,
    })
}

fn rounded_rate_micro(amount: i64, rate: FeeRate) -> Result<i64, AShareFeeV2Error> {
    let scaled = i128::from(amount)
        .checked_mul(i128::from(rate.numerator))
        .and_then(|value| value.checked_add(i128::from(rate.denominator / 2)))
        .ok_or(AShareFeeV2Error::Overflow)?;
    let rounded = scaled / i128::from(rate.denominator);
    i64::try_from(rounded).map_err(|_| AShareFeeV2Error::Overflow)
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate, Utc};

    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn scope(segment: FeeListingSegment) -> QualifiedInstrument {
        QualifiedInstrument::new(FeeMarket::Shanghai, FeeSecurityKind::AShareStock, segment)
            .unwrap()
    }

    fn policy(segment: FeeListingSegment) -> AShareFeePolicyV2 {
        AShareFeePolicyV2::new(
            scope(segment),
            FeeRate::new(3, 10_000).unwrap(),
            5_000_000,
            FeeCoverage::initial_model(),
            DEFAULT_FEE_SOURCE_REVISION,
        )
        .unwrap()
    }

    fn fill(
        policy: &AShareFeePolicyV2,
        side: FillSide,
        notional_micro_cny: i64,
        trade_date: NaiveDate,
    ) -> Result<AShareFillFeeV2, AShareFeeV2Error> {
        a_share_stock_fill_fee_with_policy_v2(
            policy,
            policy.scope(),
            side,
            notional_micro_cny,
            trade_date,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
    }

    #[test]
    fn descriptor_canonical_bytes_and_sha256_are_frozen() {
        let descriptor = policy(FeeListingSegment::ShanghaiMainA);
        let literal = concat!(
            "schema=a-share-fee-policy-descriptor/v1\n",
            "schedule=a-share-policy-by-trade-date-v2\n",
            "market=Shanghai\n",
            "instrument=AShareStock\n",
            "segment=ShanghaiMainA\n",
            "first_supported_trade_date=2008-09-19\n",
            "date_basis=executed_fill_at_asia_shanghai_date\n",
            "commission_kind=simulated_rate_with_per_fill_minimum\n",
            "commission_rate_num=3\n",
            "commission_rate_den=10000\n",
            "commission_minimum_micro_cny=5000000\n",
            "commission_applies_to=buy_and_sell\n",
            "stamp_tax_buy_num=0\n",
            "stamp_tax_buy_den=1\n",
            "stamp_tax_sell_bracket_1=2008-09-19..2023-08-27:1/1000\n",
            "stamp_tax_sell_bracket_2=2023-08-28..:1/2000\n",
            "rounding=integer_half_up_micro_cny_per_component\n",
            "coverage_commission=modeled_assumption\n",
            "coverage_stamp_tax=modeled_policy\n",
            "coverage_transfer_fee=excluded_unmodeled\n",
            "coverage_other_charges=excluded_unverified\n",
            "source_revision=adr-0001-2026-09-28+sse-fees-reviewed-2026-09-29+mof-2023-39\n",
        );
        assert_eq!(descriptor.canonical_bytes(), literal.as_bytes());
        assert_eq!(
            descriptor.descriptor_hash(),
            "12b2a0b9978ea7780771c72f64d7f102c1c30384aada3a3702b525db4f98bd70"
        );
        assert_eq!(
            descriptor.instance_id(),
            format!(
                "a-share-policy-by-trade-date-v2:sha256:{}",
                descriptor.descriptor_hash()
            )
        );
    }

    #[test]
    fn commission_scope_coverage_and_revision_change_identity() {
        let initial = policy(FeeListingSegment::ShanghaiMainA);
        let changed_commission = AShareFeePolicyV2::new(
            initial.scope(),
            FeeRate::new(4, 10_000).unwrap(),
            5_000_000,
            initial.coverage(),
            DEFAULT_FEE_SOURCE_REVISION,
        )
        .unwrap();
        let changed_scope = policy(FeeListingSegment::ShanghaiStarA);
        let changed_coverage = AShareFeePolicyV2::new(
            initial.scope(),
            FeeRate::new(3, 10_000).unwrap(),
            5_000_000,
            FeeCoverage::new(ExcludedFeeReason::Unverified, ExcludedFeeReason::Unverified),
            DEFAULT_FEE_SOURCE_REVISION,
        )
        .unwrap();
        let changed_revision = AShareFeePolicyV2::new(
            initial.scope(),
            FeeRate::new(3, 10_000).unwrap(),
            5_000_000,
            initial.coverage(),
            "adr-0001-revision-2",
        )
        .unwrap();
        for changed in [
            changed_commission,
            changed_scope,
            changed_coverage,
            changed_revision,
        ] {
            assert_ne!(changed.instance_id(), initial.instance_id());
        }
        let reduced_same_rate = AShareFeePolicyV2::new(
            initial.scope(),
            FeeRate::new(6, 20_000).unwrap(),
            5_000_000,
            initial.coverage(),
            DEFAULT_FEE_SOURCE_REVISION,
        )
        .unwrap();
        assert_eq!(reduced_same_rate.instance_id(), initial.instance_id());
    }

    #[test]
    fn dated_fills_are_deterministic_and_keep_component_evidence() {
        let descriptor = policy(FeeListingSegment::ShanghaiMainA);
        let buy = fill(&descriptor, FillSide::Buy, 1_000_000_000, date(2026, 9, 28)).unwrap();
        let sell = fill(
            &descriptor,
            FillSide::Sell,
            1_000_000_000,
            date(2026, 9, 28),
        )
        .unwrap();
        assert_eq!(buy.total_micro_cny + sell.total_micro_cny, 10_500_000);
        assert_eq!(buy.stamp_tax_micro_cny, 0);
        assert_eq!(sell.stamp_tax_micro_cny, 500_000);
        assert_eq!(sell.policy_instance_id, descriptor.instance_id());
        assert_eq!(sell.scope, descriptor.scope());
        assert_eq!(sell.side, FillSide::Sell);
        assert_eq!(sell.notional_micro_cny, 1_000_000_000);
        assert_eq!(sell.coverage, descriptor.coverage());
        assert_eq!(
            fill(
                &descriptor,
                FillSide::Sell,
                1_000_000_000,
                date(2026, 9, 28)
            ),
            Ok(sell)
        );
        let large = fill(
            &descriptor,
            FillSide::Sell,
            100_000_000_000,
            date(2026, 9, 28),
        )
        .unwrap();
        assert_eq!(large.commission_micro_cny, 30_000_000);
        assert_eq!(large.stamp_tax_micro_cny, 50_000_000);
    }

    #[test]
    fn boundaries_use_actual_shanghai_execution_date() {
        let descriptor = policy(FeeListingSegment::ShanghaiMainA);
        assert_eq!(
            fill(
                &descriptor,
                FillSide::Sell,
                1_000_000_000,
                date(2008, 9, 18)
            ),
            Err(AShareFeeV2Error::UnsupportedTradeDate)
        );
        for (trade_date, tax, bracket) in [
            (
                date(2008, 9, 19),
                1_000_000,
                StampTaxBracketV2::SellerOnePerThousand,
            ),
            (
                date(2023, 8, 27),
                1_000_000,
                StampTaxBracketV2::SellerOnePerThousand,
            ),
            (
                date(2023, 8, 28),
                500_000,
                StampTaxBracketV2::SellerHalfPerThousand,
            ),
        ] {
            let sell = fill(&descriptor, FillSide::Sell, 1_000_000_000, trade_date).unwrap();
            let buy = fill(&descriptor, FillSide::Buy, 1_000_000_000, trade_date).unwrap();
            assert_eq!(sell.stamp_tax_micro_cny, tax);
            assert_eq!(sell.stamp_tax_bracket, bracket);
            assert_eq!(buy.stamp_tax_micro_cny, 0);
        }
        let before_midnight = DateTime::parse_from_rfc3339("2023-08-27T15:59:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let after_midnight = DateTime::parse_from_rfc3339("2023-08-27T16:01:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(shanghai_execution_date(before_midnight), date(2023, 8, 27));
        assert_eq!(shanghai_execution_date(after_midnight), date(2023, 8, 28));
        let weekend_signal_date = date(2023, 8, 27);
        let executed_date = date(2023, 8, 28);
        assert_ne!(weekend_signal_date, executed_date);
        assert_eq!(
            fill(&descriptor, FillSide::Sell, 1_000_000_000, executed_date)
                .unwrap()
                .stamp_tax_micro_cny,
            500_000
        );
    }

    #[test]
    fn minimum_and_half_up_are_integer_micro_cny() {
        let descriptor = policy(FeeListingSegment::ShanghaiMainA);
        assert_eq!(
            fill(
                &descriptor,
                FillSide::Buy,
                10_000_000_000,
                date(2026, 9, 28)
            )
            .unwrap()
            .commission_micro_cny,
            5_000_000
        );
        assert_eq!(
            fill(
                &descriptor,
                FillSide::Buy,
                20_000_000_000,
                date(2026, 9, 28)
            )
            .unwrap()
            .commission_micro_cny,
            6_000_000
        );
        let half_micro_policy = AShareFeePolicyV2::new(
            descriptor.scope(),
            FeeRate::new(1, 2).unwrap(),
            0,
            descriptor.coverage(),
            DEFAULT_FEE_SOURCE_REVISION,
        )
        .unwrap();
        assert_eq!(
            fill(&half_micro_policy, FillSide::Buy, 1, date(2026, 9, 28))
                .unwrap()
                .commission_micro_cny,
            1
        );
    }

    #[test]
    fn invalid_and_unsupported_inputs_fail_closed() {
        let descriptor = policy(FeeListingSegment::ShanghaiMainA);
        for (market, kind, segment) in [
            (
                FeeMarket::Shenzhen,
                FeeSecurityKind::AShareStock,
                FeeListingSegment::Other,
            ),
            (
                FeeMarket::Unknown,
                FeeSecurityKind::AShareStock,
                FeeListingSegment::Other,
            ),
            (
                FeeMarket::Shanghai,
                FeeSecurityKind::Unknown,
                FeeListingSegment::ShanghaiMainA,
            ),
            (
                FeeMarket::Shanghai,
                FeeSecurityKind::Fund,
                FeeListingSegment::ShanghaiMainA,
            ),
        ] {
            assert_eq!(
                QualifiedInstrument::new(market, kind, segment),
                Err(AShareFeeV2Error::UnsupportedInstrument)
            );
        }
        assert_eq!(
            FeeRate::new(-1, 100),
            Err(AShareFeeV2Error::InvalidCommissionRate)
        );
        assert_eq!(
            FeeRate::new(1, 0),
            Err(AShareFeeV2Error::InvalidCommissionRate)
        );
        assert_eq!(
            AShareFeePolicyV2::new(
                descriptor.scope(),
                FeeRate::new(1, 100).unwrap(),
                -1,
                descriptor.coverage(),
                DEFAULT_FEE_SOURCE_REVISION
            ),
            Err(AShareFeeV2Error::InvalidCommissionMinimum)
        );
        assert_eq!(
            AShareFeePolicyV2::new(
                descriptor.scope(),
                FeeRate::new(1, 100).unwrap(),
                0,
                descriptor.coverage(),
                "bad\nrevision"
            ),
            Err(AShareFeeV2Error::InvalidSourceRevision)
        );
        assert_eq!(
            fill(&descriptor, FillSide::Buy, 0, date(2026, 9, 28)),
            Err(AShareFeeV2Error::InvalidNotional)
        );
        assert_eq!(
            fill(&descriptor, FillSide::Buy, -1, date(2026, 9, 28)),
            Err(AShareFeeV2Error::InvalidNotional)
        );
        assert_eq!(
            a_share_stock_fill_fee_with_policy_v2(
                &descriptor,
                descriptor.scope(),
                FillSide::Buy,
                1_000_000_000,
                date(2026, 9, 28),
                FeeCoverageRequirement::CompleteTradingCost,
            ),
            Err(AShareFeeV2Error::UnsupportedCoverage)
        );
        assert_eq!(
            a_share_stock_fill_fee_with_policy_v2(
                &descriptor,
                scope(FeeListingSegment::ShanghaiStarA),
                FillSide::Buy,
                1_000_000_000,
                date(2026, 9, 28),
                FeeCoverageRequirement::ModeledComponentsOnly,
            ),
            Err(AShareFeeV2Error::ScopeMismatch)
        );
        let huge_rate = AShareFeePolicyV2::new(
            descriptor.scope(),
            FeeRate::new(i64::MAX, 1).unwrap(),
            0,
            descriptor.coverage(),
            DEFAULT_FEE_SOURCE_REVISION,
        )
        .unwrap();
        assert_eq!(
            fill(&huge_rate, FillSide::Buy, i64::MAX, date(2026, 9, 28)),
            Err(AShareFeeV2Error::Overflow)
        );
        let sum_overflow = AShareFeePolicyV2::new(
            descriptor.scope(),
            FeeRate::new(1, 1).unwrap(),
            0,
            descriptor.coverage(),
            DEFAULT_FEE_SOURCE_REVISION,
        )
        .unwrap();
        assert_eq!(
            fill(&sum_overflow, FillSide::Sell, i64::MAX, date(2026, 9, 28)),
            Err(AShareFeeV2Error::Overflow)
        );
    }
}
