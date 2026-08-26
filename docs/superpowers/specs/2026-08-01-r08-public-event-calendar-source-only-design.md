# R-08 Public Event Calendar SourceOnly Amendment

**Status:** Gate A/B complete; Gate C/D pending
**Rule:** BR-199
**Supersedes:** the account-coupled R-08 execution clauses in BR-140, BR-161 and BR-194
**Data red lines:** 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10

## 1. Outcome

R-08 becomes a public-data review task. It reports the next trading session's
public event calendar from four independently evidenced gateway components:

1. CNInfo full-market announcements;
2. official CFFEX delivery notices;
3. Sina global-index facts;
4. Sina USD/CNY facts.

The SourceOnly report must not read or render broker positions,
user-confirmed position snapshots, local portfolio projections or virtual
observation records. Those records remain available to account-specific
features, but cannot be used to classify R-08 as a public task or to infer
holding relevance.

This is a dependency correction, not an AccountMode or DataMode bypass. The
report retains Launch, L5, quiet-hour, daily-limit, analytics, durable budget,
deduplication, fence, sink, push-log, audit and hydration governance.

## 2. Alternatives considered

### A. Dedicated public R-08 SourceOnly route — selected

Keep `ReviewTask::R08` and `PushKind::EventCalendar`, remove all account inputs,
use a dedicated exact binding validator and a closed SourceOnly allowlist
entry. This produces the requested public delivery reminder with the smallest
new authority surface.

### B. Split public and account event calendars into two tasks

This gives independent retry and cooldown semantics, but requires a new task,
PushKind, scheduler state, audit schema and user-facing template. It is larger
than the current need and is deferred until a verified broker batch exists.

### C. Keep account placeholders inside R-08

Rejected. Rendering "position unavailable" or local/virtual holdings in a
SourceOnly envelope preserves the contradictory account dependency and risks
presenting local projections as broker evidence.

## 3. Data flow

```text
ReviewRunContext.business_date
  -> calendar::next_trading_day(business_date)
  -> join public gateways
       EventCalendarGateway::market_announcements(business_date)
       FuturesDeliveryGateway::cffex_contract_month(reminder year/month)
       GlobalMarketGateway::us_indices()
       GlobalMarketGateway::usd_cny()
  -> validate each complete batch before projection
  -> require the official CFFEX component to be Available or VerifiedEmpty
  -> render a public-only event calendar
  -> canonical R08PublicSourceBinding
  -> dedicated R-08 SourceOnly validator and L5 gate
  -> existing durable delivery coordinator
  -> sink receipt, push log, delivery audit and task hydration
```

`business_date.succ_opt()` is not a trading-calendar contract and is removed
from R-08. Friday, holiday-eve and closed-day review use the repository trading
calendar's next session. Wall-clock time never selects the reminder date.

## 4. Component contract

### 4.1 Mandatory CFFEX component

The CFFEX batch is mandatory because the user-visible purpose includes advance
delivery-day warning. `Available` and `VerifiedEmpty` are complete states.
Unavailable, stale, partial, conflict or unsupported CFFEX evidence makes R-08
retryable `Failed`; it must not consume the daily delivery terminal.

Every available exact-reminder projection row must retain contract code,
product code, last trading date when supplied, delivery date, canonical notice
URL, provider/source identity, provider source time when supplied, local
observation time and immutable batch identity. Missing optional
last-trading-date remains absent. A complete admitted provider batch may carry
other same-month delivery sessions; those rows remain covered by the immutable
batch evidence but are excluded from both rendered text and the durable
`futures_delivery` projection unless `delivery_date == reminder_date`.

### 4.2 Optional independent components

Announcements, global indices and USD/CNY may be rendered independently when
their own batch is complete. An unavailable optional component is shown as
unavailable and recorded by stable component name and acquisition audit; it is
not converted into `VerifiedEmpty` and does not erase another complete batch.

The canonical delivery binding lists every admitted provider batch in fixed
component order:

1. `market_announcements`;
2. `cffex_delivery`;
3. `overnight_indices`;
4. `overnight_fx`.

Unavailable optional components are listed separately and never receive a
fabricated provider, source time, batch ID, record or zero value.

## 5. Rendering and limits

The production R-08 renderer has no holding section. It uses these labelled
sections only:

- announcements;
- CFFEX delivery;
- overnight indices;
- USD/CNY;
- an explicit degraded-components line when applicable.

Announcement validation precedes stable identity deduplication and display
limiting. The existing maximum of six announcements is retained. Rendering and
durable binding consume the same canonical CFFEX projection: first filter to
`delivery_date == reminder_date`, then order by contract code with product code,
notice URL and optional last-trading-date tie-breakers. All matching rows are
shown and receive zero-based canonical projection ordinals. A complete source
batch with no exact-date match renders an explicit verified no-reminder result.
Global index ordering is the request order; FX ordering is the request order.
No public component is re-ranked by a locally inferred score.

The text must not contain "持仓", "用户确认", "虚拟观察" or an assertion that
the account has no positions.

## 6. Immutable binding and governance

`R08PublicSourceBinding` is a deny-unknown-fields canonical JSON structure. It
binds the business date, reminder trading date, template ID, task identity,
delivery subject, ordered admitted provider batches, exact public projections,
unavailable optional component names, rendered-content SHA-256 and transition
basis.

A dedicated validator reconstructs the canonical bytes and checks:

- exact tuple `(R-08, EventCalendar, event_calendar_v1)`;
- task and schedule identities for the same business date;
- exact reminder trading date;
- mandatory CFFEX completeness;
- provider/source/component allowlists and fixed ordering;
- non-empty batch identities and valid observation timestamps;
- exact reminder-date projection validation, deterministic CFFEX order and
  rendered-text hash;
- absence of all account/local/virtual fields;
- identical ordered batch IDs and UTC observation instant in the durable
  origin.

The existing R-04 validator remains R-04-specific. R-08 receives its own public
entry and cannot be selected by arbitrary counted PushKinds.

## 7. Failure modes

- invalid review/reminder date: non-retryable failure before provider I/O;
- mandatory CFFEX unavailable/stale/partial/conflict/unsupported: retryable
  failure, zero durable admission and zero sink;
- optional component unavailable: explicit degraded public report, with no
  invented evidence;
- invalid provider/source/date/batch/order/projection/hash/text binding:
  non-retryable failure before Launch/L5/durable/sink;
- all public components unavailable: retryable failure;
- Launch/L5/durable/sink rejection: preserve the typed push outcome and log a
  non-empty dispatcher reason;
- deduplicated or previously delivered terminal: hydrate from the durable
  authority without a second sink attempt.

## 8. Old-module disposition

| Existing module/path | Decision | Reason |
| --- | --- | --- |
| public gateway calls already in `dispatch_r08_event_calendar_outcome` | adopt | typed provider evidence is already present |
| `load_user_confirmed_r08_positions` in R-08 | reject/remove from R-08 | local confirmation is not a broker batch |
| `event_calendar_virtual_holdings` in R-08 | reject/remove from R-08 | account/observation data is outside public SourceOnly scope |
| hard-coded unsupported broker placeholder | reject/remove from public renderer | absence is not a public fact to render |
| account-coupled `push_counted_with_binding` R-08 call | replace | incorrectly requires complete account metrics |
| generic R-04 SourceOnly validator | retain unchanged | R-04 and R-08 have distinct binding contracts |
| old holdings-aware renderer | retain only for non-production compatibility until caller audit proves zero use | it must not be called by production R-08 |

## 9. Validation and acceptance evidence

Gate B focused checks:

```bash
cargo test --bin monitor br199_r08 -- --test-threads=1
cargo test --lib data_gateway::futures_delivery -- --test-threads=1
bash tools/compliance/lib/check_br194_review_dependency.sh
```

Required source assertions:

```bash
rg -n -A180 'dispatch_r08_event_calendar_outcome' src/bin/monitor/push_templates.rs
rg -n 'load_user_confirmed_r08_positions|event_calendar_virtual_holdings' src/bin/monitor/push_templates.rs
rg -n 'BR-199|R08PublicSourceBinding' docs/business_rules.md src/bin/monitor tools/compliance
```

Gate C/D checks follow the repository baseline exactly:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
cargo run --release --bin monitor -- --review
```

Production evidence requires an `EventCalendar` push log, a delivery-audit
record, R-08 task hydration, and a canonical binding containing no account,
user-confirmed or virtual-observation fields.

## 10. Rollback

Revert the scoped BR-199 PR and redeploy the previous release. The rollback
must not delete or rewrite gateway acquisition audits, durable decisions,
account snapshots, push logs or delivery audits. R-08 then returns to the
conservative account gate until another approved design replaces it.
