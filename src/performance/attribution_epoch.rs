//! BR-255 attribution epoch carry isolation.
//!
//! This module is deliberately a pure quantity overlay: it never infers a
//! cost basis, splits a fill, or relaxes the BR-248 FIFO/T+1 validation that
//! applies after a code has returned to flat.

use std::collections::{BTreeMap, HashSet};

use chrono::{NaiveDate, NaiveDateTime};
use sha2::{Digest, Sha256};

use super::economic_position::EconomicFillRow;
use crate::trading::paper_lot_ledger::parse_paper_fill_timestamp;

const CARRY_MANIFEST_DOMAIN: &[u8] = b"BR255_ATTRIBUTION_CARRY_V1\0";
const EXCLUSION_MANIFEST_DOMAIN: &[u8] = b"BR255_ATTRIBUTION_EXCLUSION_V1\0";
const SCOPED_FILL_MANIFEST_DOMAIN: &[u8] = b"BR255_ATTRIBUTION_SCOPED_FILL_V1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributionEpochSelector {
    Active,
    Legacy,
    Exact(String),
}

impl AttributionEpochSelector {
    pub fn canonical_value(&self) -> String {
        match self {
            Self::Active => "active".to_owned(),
            Self::Legacy => "legacy".to_owned(),
            Self::Exact(epoch_id) => format!("exact:{epoch_id}"),
        }
    }
}

impl serde::Serialize for AttributionEpochSelector {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.canonical_value())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LegacyCarryPosition {
    pub code: String,
    pub quantity: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpochExclusionReason {
    LegacyCarryOverlap,
    MixedLegacyCarryExit,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EpochExclusion {
    pub fill_id: i64,
    pub code: String,
    pub direction: String,
    pub quantity: u64,
    pub reason: EpochExclusionReason,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EpochScopedFills {
    pub attributable: Vec<EconomicFillRow>,
    pub exclusions: Vec<EpochExclusion>,
    pub remaining_quarantine: Vec<LegacyCarryPosition>,
    pub released_codes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochActivationSource {
    Monitor,
    Cli,
}

struct ValidatedEpochFill<'a> {
    row: &'a EconomicFillRow,
    occurred_at: NaiveDateTime,
    quantity: u64,
}

#[derive(Debug)]
struct QuarantineState {
    legacy_remaining: u64,
    total_quantity: u64,
    quarantined: bool,
}

/// Projects completed pre-boundary fills into the remaining per-code quantity.
/// This validates all source facts, but intentionally does not enforce T+1:
/// the output is an isolation quantity, not an economic position report.
pub fn build_legacy_carry(
    rows: &[EconomicFillRow],
    completed_session: NaiveDate,
) -> Result<Vec<LegacyCarryPosition>, String> {
    let validated = validate_rows(rows, None)?;
    let mut positions = BTreeMap::<String, u64>::new();

    for fill in validated {
        if fill.occurred_at.date() > completed_session {
            continue;
        }
        let position = positions.entry(fill.row.code.clone()).or_default();
        match fill.row.direction.as_str() {
            "buy" => {
                *position = position
                    .checked_add(fill.quantity)
                    .ok_or_else(|| "attribution_epoch_quantity_overflow".to_owned())?;
            }
            "sell" => {
                if fill.quantity > *position {
                    return Err("attribution_epoch_cumulative_oversell".to_owned());
                }
                *position -= fill.quantity;
            }
            _ => return Err("attribution_epoch_direction_invalid".to_owned()),
        }
    }

    Ok(positions
        .into_iter()
        .filter_map(|(code, quantity)| {
            (quantity > 0).then_some(LegacyCarryPosition { code, quantity })
        })
        .collect())
}

/// Keeps only complete post-boundary fills which are independent of a legacy
/// position. A quarantined code is released only by the sell that reaches
/// exact flat; that terminal fill itself remains excluded.
pub fn scope_epoch_fills(
    rows: &[EconomicFillRow],
    effective_date: NaiveDate,
    carry: &[LegacyCarryPosition],
) -> Result<EpochScopedFills, String> {
    let validated = validate_rows(rows, Some(effective_date))?;
    let mut states = validate_carry(carry)?;
    let mut attributable = Vec::new();
    let mut exclusions = Vec::new();
    let mut released_codes = 0;

    for fill in validated {
        let state = states
            .entry(fill.row.code.clone())
            .or_insert_with(|| QuarantineState {
                legacy_remaining: 0,
                total_quantity: 0,
                quarantined: false,
            });
        match (state.quarantined, fill.row.direction.as_str()) {
            (false, "buy") => {
                state.total_quantity = state
                    .total_quantity
                    .checked_add(fill.quantity)
                    .ok_or_else(|| "attribution_epoch_quantity_overflow".to_owned())?;
                attributable.push(fill.row.clone());
            }
            (false, "sell") => {
                if fill.quantity > state.total_quantity {
                    return Err("attribution_epoch_cumulative_oversell".to_owned());
                }
                state.total_quantity -= fill.quantity;
                attributable.push(fill.row.clone());
            }
            (true, "buy") => {
                state.total_quantity = state
                    .total_quantity
                    .checked_add(fill.quantity)
                    .ok_or_else(|| "attribution_epoch_quantity_overflow".to_owned())?;
                exclusions.push(overlap(fill.row, fill.quantity));
            }
            (true, "sell") => {
                if fill.quantity > state.total_quantity {
                    return Err("attribution_epoch_cumulative_oversell".to_owned());
                }
                let mixed = fill.quantity > state.legacy_remaining && state.legacy_remaining > 0;
                state.legacy_remaining = state.legacy_remaining.saturating_sub(fill.quantity);
                state.total_quantity -= fill.quantity;
                exclusions.push(overlap(fill.row, fill.quantity));
                if mixed {
                    exclusions.push(mixed_exit(fill.row, fill.quantity));
                }
                if state.total_quantity == 0 {
                    state.quarantined = false;
                    released_codes += 1;
                }
            }
            _ => return Err("attribution_epoch_direction_invalid".to_owned()),
        }
    }

    let remaining_quarantine = states
        .into_iter()
        .filter_map(|(code, state)| {
            state.quarantined.then_some(LegacyCarryPosition {
                code,
                quantity: state.total_quantity,
            })
        })
        .collect();
    Ok(EpochScopedFills {
        attributable,
        exclusions,
        remaining_quarantine,
        released_codes,
    })
}

pub fn canonical_legacy_carry_manifest_hash(carry: &[LegacyCarryPosition]) -> String {
    let mut sorted = carry.to_vec();
    sorted.sort_by(|left, right| left.code.cmp(&right.code));
    let mut hasher = Sha256::new();
    hasher.update(CARRY_MANIFEST_DOMAIN);
    hasher.update((sorted.len() as u64).to_be_bytes());
    for position in sorted {
        update_len_prefixed(&mut hasher, position.code.as_bytes());
        hasher.update(position.quantity.to_be_bytes());
    }
    hex::encode(hasher.finalize())
}

pub fn canonical_exclusion_manifest_hash(
    exclusions: &[EpochExclusion],
    source_rows: &[EconomicFillRow],
) -> Result<String, String> {
    let sources = canonical_source_fill_index(source_rows)?;
    let mut sorted = Vec::with_capacity(exclusions.len());
    for exclusion in exclusions {
        let (occurred_at, source) = sources
            .get(&exclusion.fill_id)
            .ok_or_else(|| "attribution_epoch_exclusion_source_missing".to_owned())?;
        if source.code != exclusion.code
            || source.direction != exclusion.direction
            || u64::try_from(source.quantity).ok() != Some(exclusion.quantity)
        {
            return Err("attribution_epoch_exclusion_source_mismatch".to_owned());
        }
        sorted.push((*occurred_at, exclusion));
    }
    sorted.sort_by(|(left_time, left), (right_time, right)| {
        (
            *left_time,
            left.fill_id,
            exclusion_reason_tag(left.reason),
            left.code.as_str(),
            left.direction.as_str(),
            left.quantity,
        )
            .cmp(&(
                *right_time,
                right.fill_id,
                exclusion_reason_tag(right.reason),
                right.code.as_str(),
                right.direction.as_str(),
                right.quantity,
            ))
    });
    let mut hasher = Sha256::new();
    hasher.update(EXCLUSION_MANIFEST_DOMAIN);
    hasher.update((sorted.len() as u64).to_be_bytes());
    for (occurred_at, exclusion) in sorted {
        update_len_prefixed(
            &mut hasher,
            occurred_at
                .format("%Y-%m-%d %H:%M:%S%.9f")
                .to_string()
                .as_bytes(),
        );
        hasher.update(exclusion.fill_id.to_be_bytes());
        update_len_prefixed(&mut hasher, exclusion.code.as_bytes());
        update_len_prefixed(&mut hasher, exclusion.direction.as_bytes());
        hasher.update(exclusion.quantity.to_be_bytes());
        hasher.update([exclusion_reason_tag(exclusion.reason)]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Hashes only the canonical ordered fill IDs, because the complete fills are
/// retained independently as the attribution evidence set.
pub fn canonical_scoped_fill_manifest_hash(rows: &[EconomicFillRow]) -> Result<String, String> {
    let mut seen_ids = HashSet::new();
    let mut seen_plan_ids = HashSet::new();
    let mut sorted = Vec::with_capacity(rows.len());
    for row in rows {
        let occurred_at = validate_fill_facts(row, &mut seen_ids, &mut seen_plan_ids)?;
        sorted.push((occurred_at, row.id));
    }
    sorted.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(SCOPED_FILL_MANIFEST_DOMAIN);
    hasher.update((sorted.len() as u64).to_be_bytes());
    for (_, id) in sorted {
        hasher.update(id.to_be_bytes());
    }
    Ok(hex::encode(hasher.finalize()))
}

fn validate_rows<'a>(
    rows: &'a [EconomicFillRow],
    effective_date: Option<NaiveDate>,
) -> Result<Vec<ValidatedEpochFill<'a>>, String> {
    let mut seen_ids = HashSet::new();
    let mut seen_plan_ids = HashSet::new();
    let mut previous_order = None;
    let mut validated = Vec::with_capacity(rows.len());
    for row in rows {
        let occurred_at = validate_fill_facts(row, &mut seen_ids, &mut seen_plan_ids)?;
        let current_order = (occurred_at, row.id);
        if previous_order.is_some_and(|previous| previous >= current_order) {
            return Err("attribution_epoch_order_invalid".to_owned());
        }
        previous_order = Some(current_order);
        if effective_date.is_some_and(|date| occurred_at.date() < date) {
            return Err("attribution_epoch_fill_before_effective_date".to_owned());
        }
        let quantity = validate_quantity(row)?;
        validated.push(ValidatedEpochFill {
            row,
            occurred_at,
            quantity,
        });
    }
    Ok(validated)
}

fn canonical_source_fill_index(
    source_rows: &[EconomicFillRow],
) -> Result<BTreeMap<i64, (NaiveDateTime, &EconomicFillRow)>, String> {
    let mut seen_ids = HashSet::new();
    let mut seen_plan_ids = HashSet::new();
    let mut sources = BTreeMap::new();
    for row in source_rows {
        let occurred_at = validate_fill_facts(row, &mut seen_ids, &mut seen_plan_ids)?;
        validate_quantity(row)?;
        sources.insert(row.id, (occurred_at, row));
    }
    Ok(sources)
}

fn validate_quantity(row: &EconomicFillRow) -> Result<u64, String> {
    u64::try_from(row.quantity)
        .ok()
        .filter(|quantity| *quantity > 0 && quantity.is_multiple_of(100))
        .ok_or_else(|| "attribution_epoch_quantity_invalid".to_owned())
}

fn validate_fill_facts<'a>(
    row: &'a EconomicFillRow,
    seen_ids: &mut HashSet<i64>,
    seen_plan_ids: &mut HashSet<&'a str>,
) -> Result<NaiveDateTime, String> {
    if row.id <= 0
        || row.plan_id.trim().is_empty()
        || row.code.trim().is_empty()
        || row.name.trim().is_empty()
        || row.virtual_reason.trim().is_empty()
    {
        return Err("attribution_epoch_identity_invalid".to_owned());
    }
    if !seen_ids.insert(row.id) {
        return Err("attribution_epoch_duplicate_fill_id".to_owned());
    }
    if !seen_plan_ids.insert(row.plan_id.as_str()) {
        return Err("attribution_epoch_duplicate_plan_id".to_owned());
    }
    let occurred_at = parse_paper_fill_timestamp(row.id, &row.occurred_at)
        .map_err(|_| "attribution_epoch_timestamp_invalid".to_owned())?;
    if !row
        .fill_price
        .is_some_and(|value| value.is_finite() && value > 0.0)
    {
        return Err("attribution_epoch_price_invalid".to_owned());
    }
    if !matches!(row.direction.as_str(), "buy" | "sell") {
        return Err("attribution_epoch_direction_invalid".to_owned());
    }
    Ok(occurred_at)
}

fn validate_carry(
    carry: &[LegacyCarryPosition],
) -> Result<BTreeMap<String, QuarantineState>, String> {
    let mut states = BTreeMap::new();
    for position in carry {
        if position.code.trim().is_empty()
            || position.quantity == 0
            || !position.quantity.is_multiple_of(100)
        {
            return Err("attribution_epoch_carry_invalid".to_owned());
        }
        if states
            .insert(
                position.code.clone(),
                QuarantineState {
                    legacy_remaining: position.quantity,
                    total_quantity: position.quantity,
                    quarantined: true,
                },
            )
            .is_some()
        {
            return Err("attribution_epoch_duplicate_carry_code".to_owned());
        }
    }
    Ok(states)
}

fn overlap(row: &EconomicFillRow, quantity: u64) -> EpochExclusion {
    EpochExclusion {
        fill_id: row.id,
        code: row.code.clone(),
        direction: row.direction.clone(),
        quantity,
        reason: EpochExclusionReason::LegacyCarryOverlap,
    }
}

fn mixed_exit(row: &EconomicFillRow, quantity: u64) -> EpochExclusion {
    EpochExclusion {
        fill_id: row.id,
        code: row.code.clone(),
        direction: row.direction.clone(),
        quantity,
        reason: EpochExclusionReason::MixedLegacyCarryExit,
    }
}

fn exclusion_reason_tag(reason: EpochExclusionReason) -> u8 {
    match reason {
        EpochExclusionReason::LegacyCarryOverlap => 1,
        EpochExclusionReason::MixedLegacyCarryExit => 2,
    }
}

fn update_len_prefixed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::{
        build_legacy_carry, canonical_exclusion_manifest_hash,
        canonical_legacy_carry_manifest_hash, canonical_scoped_fill_manifest_hash,
        scope_epoch_fills, EpochExclusion, EpochExclusionReason, LegacyCarryPosition,
    };
    use crate::performance::economic_position::{rebuild_economic_positions, EconomicFillRow};

    const TEST_CODE_600001: &str = "TEST_CODE_600001";
    const TEST_CODE_600002: &str = "TEST_CODE_600002";

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").expect("TEST_CODE date")
    }

    fn fill(
        id: i64,
        code: &str,
        direction: &str,
        price: f64,
        quantity: i64,
        occurred_at: &str,
    ) -> EconomicFillRow {
        EconomicFillRow {
            id,
            plan_id: format!("TEST_CODE_PLAN_{id}"),
            code: code.to_owned(),
            name: format!("TEST_CODE_NAME_{code}"),
            direction: direction.to_owned(),
            fill_price: Some(price),
            quantity,
            occurred_at: occurred_at.to_owned(),
            virtual_reason: "NewsCatalyst".to_owned(),
        }
    }

    fn legacy_and_epoch_rows() -> Vec<EconomicFillRow> {
        vec![
            fill(1, TEST_CODE_600001, "buy", 10.0, 400, "2026-07-31 10:00:00"),
            fill(3, TEST_CODE_600001, "buy", 11.0, 300, "2026-08-03 09:30:00"),
            fill(7, TEST_CODE_600002, "buy", 20.0, 100, "2026-08-03 10:00:00"),
            fill(
                4,
                TEST_CODE_600001,
                "sell",
                12.0,
                200,
                "2026-08-04 09:30:00",
            ),
            fill(
                8,
                TEST_CODE_600002,
                "sell",
                21.0,
                100,
                "2026-08-04 10:00:00",
            ),
            fill(
                5,
                TEST_CODE_600001,
                "sell",
                13.0,
                300,
                "2026-08-05 09:30:00",
            ),
            fill(
                6,
                TEST_CODE_600001,
                "sell",
                14.0,
                200,
                "2026-08-06 09:30:00",
            ),
        ]
    }

    #[test]
    fn carry_is_code_sorted_and_zero_positions_are_omitted() {
        let rows = vec![
            fill(1, TEST_CODE_600002, "buy", 20.0, 100, "2026-07-30 09:30:00"),
            fill(
                2,
                TEST_CODE_600002,
                "sell",
                21.0,
                100,
                "2026-07-31 09:30:00",
            ),
            fill(3, TEST_CODE_600001, "buy", 10.0, 200, "2026-07-31 10:00:00"),
        ];

        assert_eq!(
            build_legacy_carry(&rows, date("2026-07-31")).expect("TEST_CODE carry"),
            vec![LegacyCarryPosition {
                code: TEST_CODE_600001.to_owned(),
                quantity: 200,
            }]
        );
    }

    #[test]
    fn carry_quarantines_complete_fills_until_first_flat_then_releases() {
        let rows = legacy_and_epoch_rows();
        let carry = build_legacy_carry(&rows[..1], date("2026-07-31")).expect("TEST_CODE carry");

        let scoped = scope_epoch_fills(&rows[1..], date("2026-08-03"), &carry)
            .expect("TEST_CODE scoped fills");

        assert_eq!(
            scoped
                .attributable
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            vec![7, 8]
        );
        assert_eq!(
            scoped
                .exclusions
                .iter()
                .map(|row| row.fill_id)
                .collect::<Vec<_>>(),
            vec![3, 4, 5, 5, 6]
        );
        assert_eq!(
            scoped
                .exclusions
                .iter()
                .map(|row| row.reason)
                .collect::<Vec<_>>(),
            vec![
                EpochExclusionReason::LegacyCarryOverlap,
                EpochExclusionReason::LegacyCarryOverlap,
                EpochExclusionReason::LegacyCarryOverlap,
                EpochExclusionReason::MixedLegacyCarryExit,
                EpochExclusionReason::LegacyCarryOverlap,
            ]
        );
        assert!(scoped.remaining_quarantine.is_empty());
        assert_eq!(scoped.released_codes, 1);
    }

    #[test]
    fn terminal_sell_remains_excluded_and_only_following_complete_cycle_is_attributable() {
        let mut rows = legacy_and_epoch_rows();
        rows.extend([
            fill(9, TEST_CODE_600001, "buy", 15.0, 100, "2026-08-07 09:30:00"),
            fill(
                10,
                TEST_CODE_600001,
                "sell",
                16.0,
                100,
                "2026-08-10 09:30:00",
            ),
        ]);
        let carry = build_legacy_carry(&rows[..1], date("2026-07-31")).expect("TEST_CODE carry");

        let scoped = scope_epoch_fills(&rows[1..], date("2026-08-03"), &carry)
            .expect("TEST_CODE scoped fills");

        assert_eq!(
            scoped
                .attributable
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            vec![7, 8, 9, 10]
        );
        assert_eq!(
            scoped
                .exclusions
                .iter()
                .map(|row| row.fill_id)
                .collect::<Vec<_>>(),
            vec![3, 4, 5, 5, 6]
        );
    }

    #[test]
    fn legacy_carry_waives_t_plus_one_but_attributable_rows_remain_subject_to_br248() {
        let legacy_rows = vec![
            fill(1, TEST_CODE_600001, "buy", 10.0, 100, "2026-07-31 09:30:00"),
            fill(
                2,
                TEST_CODE_600001,
                "sell",
                11.0,
                100,
                "2026-07-31 10:00:00",
            ),
        ];
        assert!(build_legacy_carry(&legacy_rows, date("2026-07-31"))
            .expect("TEST_CODE legacy T+1 is waived")
            .is_empty());

        let attributable_rows = vec![
            fill(3, TEST_CODE_600002, "buy", 20.0, 100, "2026-08-03 09:30:00"),
            fill(
                4,
                TEST_CODE_600002,
                "sell",
                21.0,
                100,
                "2026-08-03 10:00:00",
            ),
        ];
        let scoped = scope_epoch_fills(&attributable_rows, date("2026-08-03"), &[])
            .expect("TEST_CODE no carry");
        assert!(
            rebuild_economic_positions(&scoped.attributable, date("2026-08-03"), None)
                .expect_err("TEST_CODE BR-248 T+1")
                .contains("T+1")
        );
    }

    #[test]
    fn invalid_source_facts_fail_explicitly() {
        let valid = fill(1, TEST_CODE_600001, "buy", 10.0, 100, "2026-08-03 09:30:00");

        let mut bad_identity = valid.clone();
        bad_identity.code.clear();
        assert_eq!(
            scope_epoch_fills(&[bad_identity], date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_identity_invalid"
        );

        let mut bad_price = valid.clone();
        bad_price.fill_price = Some(f64::NAN);
        assert_eq!(
            scope_epoch_fills(&[bad_price], date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_price_invalid"
        );

        let mut zero_price = valid.clone();
        zero_price.fill_price = Some(0.0);
        assert_eq!(
            scope_epoch_fills(&[zero_price], date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_price_invalid"
        );

        let mut bad_direction = valid.clone();
        bad_direction.direction = "hold".to_owned();
        assert_eq!(
            scope_epoch_fills(&[bad_direction], date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_direction_invalid"
        );

        let mut bad_quantity = valid.clone();
        bad_quantity.quantity = 50;
        assert_eq!(
            scope_epoch_fills(&[bad_quantity], date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_quantity_invalid"
        );

        let mut bad_timestamp = valid.clone();
        bad_timestamp.occurred_at = "2026-08-03".to_owned();
        assert_eq!(
            scope_epoch_fills(&[bad_timestamp], date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_timestamp_invalid"
        );

        let duplicate = vec![
            valid.clone(),
            fill(1, TEST_CODE_600002, "buy", 20.0, 100, "2026-08-03 10:00:00"),
        ];
        assert_eq!(
            scope_epoch_fills(&duplicate, date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_duplicate_fill_id"
        );

        let unordered = vec![
            fill(2, TEST_CODE_600001, "buy", 10.0, 100, "2026-08-03 10:00:00"),
            fill(1, TEST_CODE_600002, "buy", 20.0, 100, "2026-08-03 09:30:00"),
        ];
        assert_eq!(
            scope_epoch_fills(&unordered, date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_order_invalid"
        );

        let equal_timestamp_reversed_ids = vec![
            fill(2, TEST_CODE_600001, "buy", 10.0, 100, "2026-08-03 09:30:00"),
            fill(1, TEST_CODE_600002, "buy", 20.0, 100, "2026-08-03 09:30:00"),
        ];
        assert_eq!(
            scope_epoch_fills(&equal_timestamp_reversed_ids, date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_order_invalid"
        );
    }

    #[test]
    fn oversell_overflow_and_pre_effective_rows_fail_explicitly() {
        let oversell = [fill(
            1,
            TEST_CODE_600001,
            "sell",
            10.0,
            100,
            "2026-08-03 09:30:00",
        )];
        assert_eq!(
            scope_epoch_fills(
                &oversell,
                date("2026-08-03"),
                &[LegacyCarryPosition {
                    code: TEST_CODE_600001.to_owned(),
                    quantity: 0,
                }],
            )
            .unwrap_err(),
            "attribution_epoch_carry_invalid"
        );

        assert_eq!(
            scope_epoch_fills(
                &[fill(
                    1,
                    TEST_CODE_600001,
                    "sell",
                    10.0,
                    200,
                    "2026-08-03 09:30:00"
                )],
                date("2026-08-03"),
                &[LegacyCarryPosition {
                    code: TEST_CODE_600001.to_owned(),
                    quantity: 100,
                }],
            )
            .unwrap_err(),
            "attribution_epoch_cumulative_oversell"
        );

        assert_eq!(
            scope_epoch_fills(&oversell, date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_cumulative_oversell"
        );

        let post_release_oversell = vec![
            fill(
                2,
                TEST_CODE_600001,
                "sell",
                10.0,
                100,
                "2026-08-03 09:30:00",
            ),
            fill(
                3,
                TEST_CODE_600001,
                "sell",
                10.0,
                100,
                "2026-08-04 09:30:00",
            ),
        ];
        assert_eq!(
            scope_epoch_fills(
                &post_release_oversell,
                date("2026-08-03"),
                &[LegacyCarryPosition {
                    code: TEST_CODE_600001.to_owned(),
                    quantity: 100,
                }],
            )
            .unwrap_err(),
            "attribution_epoch_cumulative_oversell"
        );

        let zero_quantity = [fill(
            4,
            TEST_CODE_600002,
            "buy",
            20.0,
            0,
            "2026-08-03 09:30:00",
        )];
        assert_eq!(
            scope_epoch_fills(&zero_quantity, date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_quantity_invalid"
        );

        let max_hundred = u64::MAX / 100 * 100;
        let overflow = [fill(
            2,
            TEST_CODE_600001,
            "buy",
            10.0,
            100,
            "2026-08-03 09:30:00",
        )];
        assert_eq!(
            scope_epoch_fills(
                &overflow,
                date("2026-08-03"),
                &[LegacyCarryPosition {
                    code: TEST_CODE_600001.to_owned(),
                    quantity: max_hundred,
                }],
            )
            .unwrap_err(),
            "attribution_epoch_quantity_overflow"
        );

        let before_effective = [fill(
            3,
            TEST_CODE_600001,
            "buy",
            10.0,
            100,
            "2026-08-02 09:30:00",
        )];
        assert_eq!(
            scope_epoch_fills(&before_effective, date("2026-08-03"), &[]).unwrap_err(),
            "attribution_epoch_fill_before_effective_date"
        );
    }

    #[test]
    fn legacy_carry_rejects_bad_facts_and_cumulative_oversell_without_t_plus_one() {
        let invalid = [fill(
            1,
            TEST_CODE_600001,
            "sell",
            10.0,
            100,
            "2026-07-31 09:30:00",
        )];
        assert_eq!(
            build_legacy_carry(&invalid, date("2026-07-31")).unwrap_err(),
            "attribution_epoch_cumulative_oversell"
        );

        let huge = i64::MAX / 100 * 100;
        let overflow = vec![
            fill(
                1,
                TEST_CODE_600001,
                "buy",
                10.0,
                huge,
                "2026-07-29 09:30:00",
            ),
            fill(
                2,
                TEST_CODE_600001,
                "buy",
                10.0,
                huge,
                "2026-07-30 09:30:00",
            ),
            fill(
                3,
                TEST_CODE_600001,
                "buy",
                10.0,
                huge,
                "2026-07-31 09:30:00",
            ),
        ];
        assert_eq!(
            build_legacy_carry(&overflow, date("2026-07-31")).unwrap_err(),
            "attribution_epoch_quantity_overflow"
        );
    }

    #[test]
    fn canonical_manifests_are_order_independent_and_domain_separated() {
        let carry = vec![
            LegacyCarryPosition {
                code: TEST_CODE_600002.to_owned(),
                quantity: 100,
            },
            LegacyCarryPosition {
                code: TEST_CODE_600001.to_owned(),
                quantity: 200,
            },
        ];
        let mut reordered_carry = carry.clone();
        reordered_carry.reverse();
        assert_eq!(
            canonical_legacy_carry_manifest_hash(&carry),
            canonical_legacy_carry_manifest_hash(&reordered_carry)
        );

        let exclusions = vec![
            EpochExclusion {
                fill_id: 4,
                code: TEST_CODE_600002.to_owned(),
                direction: "sell".to_owned(),
                quantity: 100,
                reason: EpochExclusionReason::LegacyCarryOverlap,
            },
            EpochExclusion {
                fill_id: 3,
                code: TEST_CODE_600001.to_owned(),
                direction: "buy".to_owned(),
                quantity: 100,
                reason: EpochExclusionReason::MixedLegacyCarryExit,
            },
        ];
        let exclusion_sources = vec![
            fill(3, TEST_CODE_600001, "buy", 10.0, 100, "2026-08-03 10:00:00"),
            fill(
                4,
                TEST_CODE_600002,
                "sell",
                20.0,
                100,
                "2026-08-03 09:30:00",
            ),
        ];
        let mut reordered_exclusions = exclusions.clone();
        reordered_exclusions.reverse();
        assert_eq!(
            canonical_exclusion_manifest_hash(&exclusions, &exclusion_sources)
                .expect("TEST_CODE exclusion hash"),
            canonical_exclusion_manifest_hash(&reordered_exclusions, &exclusion_sources)
                .expect("TEST_CODE exclusion hash")
        );

        let rebound_sources = vec![
            fill(3, TEST_CODE_600001, "buy", 10.0, 100, "2026-08-03 09:00:00"),
            fill(
                4,
                TEST_CODE_600002,
                "sell",
                20.0,
                100,
                "2026-08-03 09:30:00",
            ),
        ];
        assert_ne!(
            canonical_exclusion_manifest_hash(&exclusions, &exclusion_sources)
                .expect("TEST_CODE exclusion hash"),
            canonical_exclusion_manifest_hash(&exclusions, &rebound_sources)
                .expect("TEST_CODE timestamp rebinding must change hash")
        );

        let fills = vec![
            fill(2, TEST_CODE_600002, "buy", 20.0, 100, "2026-08-03 10:00:00"),
            fill(1, TEST_CODE_600001, "buy", 10.0, 100, "2026-08-03 09:30:00"),
        ];
        let mut reordered_fills = fills.clone();
        reordered_fills.reverse();
        let carry_hash = canonical_legacy_carry_manifest_hash(&carry);
        let exclusion_hash = canonical_exclusion_manifest_hash(&exclusions, &exclusion_sources)
            .expect("TEST_CODE exclusion hash");
        let fills_hash = canonical_scoped_fill_manifest_hash(&fills).expect("TEST_CODE hash");
        assert_eq!(
            fills_hash,
            canonical_scoped_fill_manifest_hash(&reordered_fills).expect("TEST_CODE hash")
        );
        assert_eq!(carry_hash.len(), 64);
        assert!(carry_hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(carry_hash, exclusion_hash);
        assert_ne!(exclusion_hash, fills_hash);
    }
}
