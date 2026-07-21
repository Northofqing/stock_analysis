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
  -> existing title classification
  -> lifecycle-value filter
  -> real holding/watch-universe membership
  -> normalized BR-137 evidence
  -> LaunchGate / quiet hours / L4 / daily limit / real sink / L7 / hash-chain
```

Every provider item with a complete external identity remains owned by the normalized route once it
is classified. A relevance-filtered item is recorded as skipped and its identity is returned in the
handled-identity set, so the legacy state machine cannot send it as a fallback.

## 4. Required configuration

`config/chain.toml` must contain a dedicated `[announce_keywords]` table. The combined loader parses a
typed wrapper and publishes the section only after all three keyword lists are present. The shared
production `fetch_announcements` boundary checks that the section is available before transport, so
the news loop, R-08, and future production callers all fail closed. Missing or malformed configuration
is an explicit error and blocks the announcement batch; it must not silently select a broader
vocabulary.

No numeric threshold changes in this work.

## 5. Immediate-notification relevance

An announcement is eligible for immediate BR-137 delivery only when all of the following hold:

1. the provider external identity, security code, publication date, source, title, and classified
   level are valid under existing BR-137 rules;
2. the security code is present in the real universe loaded by `news_monitor_loop` from portfolio
   codes plus explicitly registered watch codes;
3. the title is not a procedural creditor notice caused by cancellation of repurchased shares or a
   registered-capital reduction;
4. the title is not a reduction-plan completion, expiry, or completed-implementation notice.

The two lifecycle exclusions change notification urgency only. The real provider item remains in
local processing/audit statistics. New reduction plans, controlling-shareholder risk, regulatory
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
- `route_announcements(&[Announcement], &HashSet<String>)` receives the real eligible universe
  explicitly; it does not query account state or Banner internally.
- `AnnouncementSourceRouteReport.handled_external_ids` prevents legacy bypass for both delivered and
  intentionally filtered normalized events.

## 7. Failure modes

- Missing/malformed announcement configuration: explicit error; announcement batch does not run.
- Real universe unavailable: retain the existing retry loop; never replace it with an empty or fake
  universe.
- Incomplete provider identity/date/code: existing explicit BR-137 rejection.
- Relevance-filtered event: `skipped += 1`, reason-only log without message body or account values,
  and legacy suppression via handled provider identity.
- Governance/sink/audit failure after relevance approval: existing BR-137 retry semantics remain.

## 8. Validation

- Parsing the repository `chain.toml` yields the exact configured announcement lists.
- A procedural capital-reduction creditor notice is not immediately actionable.
- A reduction-plan expiry/completion notice is not immediately actionable.
- A valid important announcement outside the eligible universe is skipped and cannot fall through to
  legacy delivery.
- A valid important or emergency announcement inside the universe still reaches BR-137 governance.
- Policy and critical flash behavior is unchanged.
- Full formatting, strict Clippy, workspace tests, compliance, coverage, release build, and an
  independent review pass before merge.

## 9. Rollback

Revert the dedicated announcement-relevance commit/PR and rebuild `monitor`. Rollback changes only
notification selection; it does not mutate positions, account snapshots, orders, historical market
data, or provider records.
