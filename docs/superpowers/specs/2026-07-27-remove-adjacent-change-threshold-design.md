# Remove Adjacent Daily-Change Threshold Design

## 1. Goal

Remove percentage-based rejection of adjacent historical daily values. A legitimate
listing-day, board-rule, corporate-action, or source-defined price series must not
be rejected solely because the open or close moved by more than a fixed percentage
from the previous settled close.

This change does not weaken structural data validation.

## 2. Scope

The removal applies to every production admission path that validates adjacent
historical daily bars:

- generic daily K-line admission and database persistence;
- event-scoped selection daily bars;
- post-session review daily bars.

The following checks remain blocking:

- finite, positive prices;
- valid OHLC relationships;
- finite, valid volume and amount;
- unique and continuous trading dates;
- provider-reported change consistency when that field is present;
- provider previous-close consistency used to detect unverified adjustment or
  split/dividend discontinuity;
- exact instrument identity, batch completeness, freshness, and source evidence.

The following are explicitly outside this change:

- realtime quote freshness and abnormal tick-to-tick jump checks;
- exchange daily price-limit checks used for order safety;
- index or quote payload field-range checks that do not compare adjacent
  historical values.

## 3. Interface Change

`validate_daily_kline_quality` no longer accepts a percentage threshold. The
board-prefix helper `max_gap_for` is removed because no caller may select a
historical batch by an adjacent percentage limit.

Selection and review validators remove their fixed `20%` adjacent-close branches.
They continue to validate reference previous close when the provider supplies it.

## 4. Data Flow

```text
provider batch
  -> identity/completeness/evidence admission
  -> finite/positive/OHLC/volume/amount validation
  -> trading-date continuity and duplicate validation
  -> provider change/reference-previous-close consistency
  -> accepted historical batch
```

No adjacent open/close percentage is calculated for rejection or manual
confirmation.

## 5. Failure Modes

- A large but structurally consistent price move is accepted.
- A large move with a provider `pct_chg` that contradicts the actual prices is
  rejected as a source-consistency failure.
- A series with an explicit reference previous close that contradicts the prior
  settled close is rejected as unverified split/dividend continuity.
- Missing or unavailable lifecycle evidence is not fabricated.
- All existing source, freshness, identity, cardinality, and audit failures remain
  explicit.

## 6. Business Rules

- BR-092, BR-147, BR-156, and BR-159 are updated to remove adjacent-percentage
  admission language while preserving their other gates.
- The repository data-redline policy no longer contains an adjacent `±20%`
  requirement. The user made that policy change explicitly on 2026-07-27.

## 7. Validation

Required focused tests:

- main-board and STAR/ChiNext daily batches with an adjacent change above `20%`
  pass when all structural fields are consistent;
- the same batches still fail for invalid OHLC, gaps, duplicates, invalid amount,
  inconsistent provider change, or reference-previous-close mismatch;
- no production historical validator contains `max_gap_for`,
  `MAX_ADJACENT_CHANGE`, or an adjacent `20%` rejection.

Release gates remain:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash tools/compliance/check.sh
```

## 8. Rollback

Revert the dedicated implementation commit and the dedicated design/rule commit.
No database migration or destructive data rewrite is required.
