# BR-195 Chain Streak Business-Date Binding

**Status:** Gate A ready after independent C1/I4/M1 findings resolved; local recheck C0/I0/M0
**Date:** 2026-08-01
**Data red lines:** 2.2, 2.3, 2.4, 2.7, 2.10
**Business rule:** BR-195

## 1. Outcome

Live clustering and enrichment must calculate a chain's recent appearance-day
count relative to the evidence business date, never relative to the machine
wall clock. The compatibility `chain_daily` reader therefore requires an
explicit `as_of: NaiveDate` and applies a closed natural-day window:

```text
[as_of - (days - 1), as_of]
```

This is a correctness repair for the legacy compatibility reader while BR-160's
immutable Chain Intelligence migration remains incomplete. It does not promote
`chain_daily` to a new factual authority or alter any source acquisition.

## 2. Reproduced defect and code facts

The full deterministic library run on 2026-08-01 produced one failure:

```text
test result: FAILED. 2309 passed; 1 failed; 7 ignored
pipeline::chain_analysis::tests::resolved_chain_facts_persist_match_and_render_without_external_sources
left: 0
right: 1
```

The exact caller inventory was reproduced with:

```bash
rg -n -C 5 "get_chain_streak_days|chain_streak" \
  src/pipeline/extra_context.rs src --glob '*.rs'
```

Relevant output:

```text
src/pipeline/chain_analysis/mod.rs:412: .get_chain_streak_days_strict(&cluster.concept, 10)
src/pipeline/extra_context.rs:102: let streak = db.get_chain_streak_days_strict(&row.concept, 10)?;
src/database/concepts.rs:240: pub fn get_chain_streak_days_strict(...)
```

The DAO derives its lower bound from `Local::now()` and has no upper bound.
Consequently, deterministic replay/tests drift as wall time advances, and a row
later than the clustering or enrichment evidence date can leak into the count.

## 3. Data flow and ownership

```text
live compatibility chain clustering
  -> parse the clustering business date
  -> persist exact chain_daily date
  -> query BR-195 window using the same clustering date as as_of
  -> render natural-day appearance count

latest-chain enrichment
  -> load latest committed chain_daily rows
  -> parse the matched row.date
  -> query BR-195 window using that row date as as_of
  -> render natural-day appearance count
```

`DatabaseManager` owns the bounded query. Callers own selection of the business
date from existing evidence. No caller may substitute `Local::now()`, acquisition
time, database write time, or a default date.

## 4. Closed semantics

1. `concept.trim()` must be non-empty.
2. `days` must be positive.
3. `as_of` is a typed `NaiveDate`; string parsing remains at the evidence boundary.
4. Compute the offset without panic: `days.checked_sub(1)`, then
   `TimeDelta::try_days(offset)`, then `as_of.checked_sub_signed(delta)`.
5. Any subtraction, duration conversion, or date overflow is an explicit error.
6. SQL binds both `date >= lower_bound` and `date <= as_of`.
7. The result counts distinct stored dates; no missing day is fabricated.
8. The window uses natural dates. Every active prompt, report and notification
   must say “近 N 个自然日”; this patch does not infer a trading calendar.
9. The implicit wall-clock convenience methods are removed so production cannot
   silently regress to time-dependent behavior.
10. The compatibility field name `streak_days` remains temporarily to constrain
    the repair, but comments and interfaces call it an appearance-day count; it
    is not a consecutive streak.
11. Consumers render the exact verified count. `max(1)`, defaults and other
    count repair are forbidden because they would fabricate missing evidence.

## 5. Failure modes

| Failure | Result |
| --- | --- |
| blank concept or non-positive window | typed `Err`; no query result |
| invalid persisted/request date at caller | explicit caller error; no rendering |
| date arithmetic overflow | typed `Err`; no query result |
| database acquisition/query failure | existing strict error propagation |
| no matching date in the closed window | verified count `0` |
| row after `as_of` | excluded by the SQL upper bound |

No path converts parsing or database failure into a successful zero. Missing
evidence remains explicit under rules 2.2 and 2.4.

## 6. Old-module disposition

| Module | Adopt/reject | Reason |
| --- | --- | --- |
| `DatabaseManager::get_chain_streak_days_strict` | replace | implicit wall clock violates historical determinism |
| `DatabaseManager::get_chain_streak_days` | delete | non-strict zero fallback hides missing data |
| `chain_daily` persistence and its two existing consumers | temporarily adopt | BR-160 migration is not complete; no new caller or schema rewrite is allowed |
| BR-160 immutable Chain Intelligence tables | no change | authoritative replacement remains the target architecture; BR-195 is not release evidence for BR-160 |

## 7. Validation and acceptance

Gate B:

```bash
cargo test --lib pipeline::chain_analysis::tests::resolved_chain_facts_persist_match_and_render_without_external_sources -- --exact --test-threads=1
cargo test --lib database::concepts::tests:: -- --test-threads=1
cargo test --lib pipeline::chain_analysis::tests:: -- --test-threads=1
cargo test --lib pipeline::extra_context::tests:: -- --test-threads=1
```

The DAO tests must prove `row == as_of` and `row == lower_bound` are included,
while `row < lower_bound` and `row > as_of` are excluded. They also cover blank
concept, `days == 0`, `days == i64::MAX`, and explicit rejection of an invalid
persisted chain date at the enrichment boundary. The integration fixture uses a
real weekday business date rather than a weekend.

Gate C/D continue to require the repository-wide format, strict Clippy, full
workspace tests, compliance, coverage and live-binary checks from
`docs/ENGINEERING_RULES_V2.md`. This repair has no new producer or PushKind, so
production push-log evidence is N/A. `monitor --review` does not call this seam;
the exact DAO and live-clustering integration tests are the business-date proof,
and a bounded normal monitor run remains part of the repository release smoke.

## 8. BR-160 compatibility boundary

BR-195 permits exactly the two callers recorded in §2 and forbids new callers.
It neither closes the evidence gaps in `chain_daily` nor satisfies any BR-160
Gate. When BR-160 cuts over to its immutable Chain Intelligence reader, this API,
the compatibility field and both callers must be deleted in the same release.

## 9. Rollback

Revert the scoped BR-195 commit/PR. Rollback must not delete or rewrite existing
`chain_daily`, audit, delivery, position, trade, or market-data evidence.

## 10. 2026-08-01 Entry-Date Amendment

The public `run_chain_analysis` entry accepts a typed caller-owned
`business_date: NaiveDate`. It must not call `Local::now()` internally. The
same value is used for report text, `cluster_and_persist`, and every bounded
appearance-day query.

Monitor review code obtains the value from
`ReviewRunContext::business_date()`. Non-review live callers capture one
`observed_at` at their acquisition boundary and derive the latest completed
trading date with `calendar::latest_completed_trading_day_at(observed_at)`.
They must not independently recalculate the date after provider acquisition.

Acceptance adds a historical-date empty-batch test proving that report output
uses the supplied business date rather than the machine date. Rollback restores
the former function signature only together with its callers; it must not
restore an internal wall-clock read.
