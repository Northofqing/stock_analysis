# Announcement Relevance Gate Design

Date: 2026-07-21
Status: approved (the operator previously authorized all confirmation gates)
Scope: real-provider announcement notification quality only

## 1. Problem

The BR-137 source-fact path restored real announcement delivery, but it currently treats every
classified item in the provider's market-wide batch as eligible for an immediate user notification.
That makes structurally valid but non-actionable events noisy. Two representative failures are:

- a procedural creditor notice following cancellation of repurchased shares and a capital reduction;
- a shareholder reduction-plan completion/expiry notice that introduces no new action window.

The combined `config/chain.toml` also places announcement keyword keys after the final `[[rules]]`
entry without opening a dedicated table. TOML therefore attaches those keys to the final rule and the
independent `AnnounceKeywordsFile` parser cannot load them. Production can consequently retain the
broader compile-time fallback rather than the intended configured vocabulary.

## 2. Considered approaches

### A. Audience filtering only

Only immediately notify for codes in the real holding/watch universe. This removes most market-wide
noise but still pushes procedural events for a held security.

### B. Keyword filtering only

Add exclusions for the two observed lifecycle phrases. This fixes the examples but leaves every other
market-wide classified announcement eligible for immediate delivery.

### C. Layered relevance gate (selected)

Validate the required keyword configuration, apply a narrow lifecycle-value filter, and then require
membership in the real holding/watch universe before BR-137 delivery. This closes both root causes
without weakening provenance, freshness, governance, audit, or sink requirements.

## 3. Data flow

```text
Eastmoney announcement batch
  -> complete provider-field and publication-date validation
  -> existing title classification OR explicit lifecycle-only local retention
  -> lifecycle-value filter
  -> fresh real holding / explicit watch-universe membership
  -> normalized BR-137 evidence
  -> LaunchGate / quiet hours / L4 / daily limit / real sink / L7 / hash-chain
```

Every provider item with a complete external identity remains owned by the normalized route once it
is classified. The route returns a typed disposition for each owned identity. A relevance-filtered
item is recorded as skipped/handled so the legacy state machine cannot send it as a fallback, but it
is not eligible to trigger D-01, I-02, or another downstream notification. Only a `Pushed`
disposition can feed those downstream triggers.

## 4. Required configuration

`config/chain.toml` must contain a dedicated `[announce_keywords]` table. The combined loader parses a
typed wrapper and publishes the section only after all three keyword lists are present. The shared
production `fetch_announcements` boundary loads one exact configuration snapshot before transport
and holds it through detail selection and final classification, so a concurrent reload cannot create
a check/use race. The news loop, R-08, and future production callers all fail closed. Missing or
malformed configuration is an explicit error and blocks the announcement batch; it must not silently
select a broader vocabulary.

No numeric threshold changes in this work.

## 5. Immediate-notification relevance

An announcement is eligible for immediate BR-137 delivery only when all of the following hold:

1. the provider external identity, security code, publication date, source, title, and classified
   level are valid under existing BR-137 rules;
2. the security code is present in the audience rebuilt immediately before routing from a real
   broker position batch no older than 30 seconds, carrying immutable provider/batch provenance,
   plus the independently loaded explicit watch codes;
3. the title is not a procedural creditor notice caused by cancellation of repurchased shares or a
   registered-capital reduction;
4. the title is not a reduction-plan completion, expiry, or completed-implementation notice.

The two lifecycle exclusions change notification urgency only. The real provider item remains in
local processing/audit statistics even when it does not match the configured notification keyword
vocabulary; provider assembly retains it as non-pushable `Skip` evidence until the route assigns
`FilteredLifecycle`. New reduction plans, controlling-shareholder risk, regulatory
actions, earnings events, and emergency announcements for an eligible code retain their existing
classification and governance.

Policy facts and critical market flash facts are global by nature and are not subject to the security
universe gate.

## 6. Interfaces

- `CombinedChainConfig` owns `rules`, `announce_keywords`, and `boards` at their actual TOML paths.
- `fetch_announcements` is the production fail-closed boundary; lower-level injected transports remain
  available only for isolated protocol tests.
- `announcement_title_is_immediately_actionable(&str) -> bool` contains only the registered
  lifecycle exclusions and is deterministic.
- `announcement_is_immediate_notification_candidate(&Announcement) -> bool` is the shared consumer
  gate. NewsMonitor, NewsAggregator, R-08 summaries/holding events, and future renderers must apply it;
  only the normalized route consumes `LocalOnly/Skip` rows for `FilteredLifecycle` accounting.
- Provider assembly retains a lifecycle-only row even when ordinary keyword classification is
  `Skip`; it does not fetch risk detail for that row and never upgrades it to notification-eligible.
- `route_announcements(&[Announcement], &HashSet<String>)` receives the real eligible universe
  explicitly; it does not query account state or Banner internally.
- A verified position-audience component requires immutable broker provider/batch identity and a
  source timestamp no older than 30 seconds. The local simulated `stock_position` table and its
  mutable `updated_at` are never accepted as that evidence. Until a broker position batch is
  connected, production explicitly excludes the position component and continues with independent
  watch codes.
- `AnnouncementSourceRouteReport.dispositions` maps every owned external ID to a typed `Pushed`,
  `FilteredLifecycle`, `FilteredAudience`, or `Failed` result. All four prevent legacy fallback;
  only `Pushed` participates in downstream notification triggers.

## 7. Failure modes

- Missing/malformed announcement configuration: explicit error; announcement batch does not run.
- Verified broker position snapshot unavailable, missing immutable batch provenance, untimestamped,
  or older than 30 seconds: exclude the position component explicitly while retaining the
  independently validated explicit-watch audience; never infer freshness from a local database
  write time or replace either component with a fake universe.
- Incomplete provider identity/date/code: existing explicit BR-137 rejection.
- Relevance-filtered event: `skipped += 1`, a typed filtered disposition, reason-only log without
  message body or account values, legacy suppression via owned provider identity, and no D-01/I-02
  downstream trigger.
- Governance/sink/audit failure after relevance approval: existing BR-137 retry semantics remain.
- Configuration, transport, audience, or name-resolution failure is isolated to the announcement
  sub-chain. The outer news loop must still run its unrelated scheduler, daily reset, state flush,
  banner refresh, and common sleep.
- Explicit-watch loading is attempted by one outer-loop-owned background task at a time. Policy and
  critical flash run before its readiness is inspected; an unfinished task is never awaited. A
  missing/failed watch result closes only the announcement audience and optional watch increment,
  while the independently loaded holding code pool still drives earnings/analyst and L2 work.
  Its first failure is logged during startup/first tick.
- Each route aggregate reports counts for Pushed, FilteredLifecycle, FilteredAudience, and Failed so
  a live canary can verify selection without logging bodies, account values, or security identities.

## 8. Validation

- Parsing the repository `chain.toml` yields the exact configured announcement lists.
- A procedural capital-reduction creditor notice is not immediately actionable.
- The same creditor notice survives provider assembly as local-only evidence and reaches the route's
  `FilteredLifecycle` disposition without requiring a risk-detail request.
- NewsMonitor, NewsAggregator, both R-08 paths, and the event-calendar summary cannot render or push
  that local-only row.
- A reduction-plan expiry/completion notice is not immediately actionable.
- A valid important announcement outside the eligible universe is skipped and cannot fall through to
  legacy delivery.
- A valid important or emergency announcement inside the universe still reaches BR-137 governance.
- A mutable local `stock_position.updated_at` cannot authorize audience membership; absent verified
  broker batch evidence, only explicit watch codes form the audience.
- Typed filtered dispositions suppress both legacy fallback and D-01/I-02 downstream triggers;
  only `Pushed` remains downstream-eligible.
- A pending or failed explicit-watch background load returns immediately, leaves policy/critical
  flash/holding earnings/L2/reset/flush scheduling runnable, and retries only the announcement
  audience/watch increment on a later outer-loop tick.
- Policy and critical flash behavior is unchanged.
- Full formatting, strict Clippy, workspace tests, compliance, coverage, release build, and an
  independent review pass before merge.

## 9. Rollback

Revert the dedicated announcement-relevance commit/PR and rebuild `monitor`. Rollback changes only
notification selection; it does not mutate positions, account snapshots, orders, historical market
data, or provider records.
