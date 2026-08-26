# Full-market ranking unavailable-capability retirement design

**Status:** Gate A approved; Gate B implemented; Gate C/D pending parent validation
**Business rules:** BR-150, BR-190
**Pinned upstream revision:** `660902ffdbfa4d3972548f41381d1ff8d33fb42f`

## 1. Intent

Keep the monitor operational without misrepresenting an unavailable
full-market ranking as an empty result, and without retrying a capability which
the pinned provider has not live-admitted.

This slice retires:

- the two `--review` all-market volume-ratio candidate calls;
- the five-minute I-10 main-net-inflow loop;
- the 15:35 BR-073 virtual-buy loop;
- the corresponding non-isolated `--test` calls; and
- the local permanent-error facades and their orphaned render/dispatch code.

It does not change sector history or the news ranker.

## 2. Reproducible current-state evidence

The local dead-call graph is reproducible with:

```bash
rg -n -U 'fetch_market_(main_inflow_top|volume_ratio_leaders)|last_fund_top_push|post_close_fund_inflow_scheduler|dispatch_(fund_inflow_top|post_close_fund_inflow)' \
  src/bin/monitor/main.rs src/bin/monitor/market_data.rs src/bin/monitor/push_templates.rs
```

Before Gate B, this identifies:

- two `market_data.rs` facades which always return `Err`;
- review call sites in both review implementations;
- a five-minute periodic loop;
- a 15:35 daily retry loop; and
- two non-isolated test dispatches.

The pinned upstream evidence is retained at:

`magic-market-data-rs/docs/evidence/2026-07-27-rankings-consensus-target-price.md`

Its bounded post-close probes report:

- volume-ratio ranking unavailable because provider field `f10` is absent;
- main-net-inflow ranking unavailable because provider field `f62` is absent;
- incomplete page coverage and mixed source timestamps; and
- `SignalCapabilities.market_rankings == false`.

The fixed revision also keeps the provider capability disabled in
`crates/magic-eastmoney-rs/src/lib.rs` and labels its market-ranking integration
test as waiting for live admission.

## 3. Decision

Use explicit static retirement until a separately reviewed upstream release
live-admits the capability.

At startup, review, and non-isolated test entry points, emit one stable state:

```text
status=unavailable
reason_code=provider_capability_not_live_admitted
metrics=volume_ratio,main_net_inflow
retryable=false
```

`--test` additionally writes the dispatcher audit disposition
`capability_unavailable:provider_capability_not_live_admitted`. It must not
count the result as no-data.

### Rejected alternatives

- **Permanent `Err` facade:** retains dead public API and permits accidental
  retry loops.
- **Return an empty vector:** violates missing-data semantics by converting
  unavailable into verified empty.
- **Dragon-tiger substitution:** ranked disclosure events are not a full-market
  stock ranking.
- **Board-ranking substitution:** board membership/ranking is a different
  entity and universe.
- **Explicit-code company statistics:** cannot establish full-market discovery
  or ranking completeness.

## 4. Runtime data flow

### Normal monitor

Startup records the unavailable capability once. No I-10 timer, no 15:35
fund-inflow scheduler, and no provider request exist.

### Review

Holding and explicit-watch analysis continue. The full-market candidate section
is omitted after recording capability unavailable. The review does not fail
because a non-admitted optional capability is absent.

### Test

Isolated fixtures remain isolated. Non-isolated test mode records the two
retired task dispositions as capability unavailable and performs no request.

## 5. Failure modes and audit

- Provider capability false is a non-retryable disabled state, not an empty
  result.
- The state is visible in logs with owner/task identity.
- Non-isolated test audit uses a stable capability-unavailable reason.
- No synthetic rank, zero fill, cross-source merge, or old HTTP acquisition is
  allowed.
- Re-enablement requires a new Gate A decision, an admitted typed Gateway, and
  fresh bounded-live evidence for both metrics.

## 6. Old module disposition

| Module | Decision | Reason |
|---|---|---|
| `fetch_market_main_inflow_top` | delete | permanent-error facade |
| `fetch_market_volume_ratio_leaders` | delete | permanent-error facade |
| I-10 renderer/dispatchers | delete | only consumed unavailable facade |
| BR-073 post-close virtual-buy dispatcher | delete | ranking identity unavailable |
| `TopStock` quote projection | retain minimal boundary | still used by real quote consumers |
| `PushKind::FundInflow` | retain | persisted/audit compatibility is outside this slice |

## 7. Validation and rollback

Validation for this no-Cargo slice:

```bash
rustfmt --edition 2021 src/bin/monitor/main.rs src/bin/monitor/market_data.rs src/bin/monitor/push_templates.rs
rg -n -U 'fetch_market_(main_inflow_top|volume_ratio_leaders)|last_fund_top_push|post_close_fund_inflow_scheduler|dispatch_(fund_inflow_top|post_close_fund_inflow)' \
  src/bin/monitor/main.rs src/bin/monitor/market_data.rs src/bin/monitor/push_templates.rs
rg -n 'BR-190|provider_capability_not_live_admitted|capability_unavailable' \
  docs/business_rules.md src/bin/monitor
git diff --check
```

Rollback is a Git revert of the implementing commit. Do not delete audit
records or substitute a different source during rollback.
