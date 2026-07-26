# Futures Delivery-Day Advance Reminder — Capability Gap Audit

**Status:** Gate A blocked on unified upstream provider contract
**Date:** 2026-07-25
**Scope:** CFFEX, SHFE, DCE, CZCE, INE, GFEX
**Data red lines:** 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10

## 1. Outcome

The requested advance reminder is not implemented in the production monitor.
The live event-calendar path does not acquire or render futures delivery
schedules, and the unified `magic-market-data-rs` upstream does not expose a
source-backed futures-delivery contract for any of the six requested
exchanges.

No downstream Gateway or reminder is added in this slice. A local calendar
formula, contract-code month inference, option expiry, or process time cannot
replace exchange evidence. Doing so would violate rules 2.1, 2.2, 2.4, 2.7,
and 2.8.

## 2. Reproducible production evidence

The red-capable production check is:

```bash
rg -n -i -U -S \
  '((CFFEX|SHFE|DCE|CZCE|INE|GFEX).{0,160}(delivery[_ -]?(date|day)|交割日))|((delivery[_ -]?(date|day)|交割日).{0,160}(CFFEX|SHFE|DCE|CZCE|INE|GFEX))' \
  src/bin/monitor src/data_gateway
```

Observed output:

```text
exit_code=1
<zero matches>
```

The broad repository check is:

```bash
rg -n -i -S -g '*.rs' -g '*.toml' -g '*.md' \
  '交割日|交割日期|delivery.?day|delivery.?date|last.?trade|CFFEX|SHFE|DCE|CZCE|INE|GFEX|期货' \
  src docs config tests scratchpad
```

The only requested-behavior match is the explicit P2 backlog in
`docs/handoffs/HANDOFF_2026-07-22_MONITOR_AND_REMAINING_WORK.md`. No Rust
producer, scheduler, Gateway, renderer, or delivery call exists.

The current governed event-calendar call chain is:

```text
BR-139 post-session scheduler
  -> dispatch_post_session_review
  -> dispatch_r08_event_calendar_outcome
  -> prepare_r08_event_calendar
  -> dispatch_outcome(PushKind::EventCalendar)
```

Its four inputs are announcements, real holdings, virtual holdings, and Yahoo
overnight market data. None is a futures delivery schedule. Consequently,
`PushKind::EventCalendar` existing in production is not evidence that the
requested reminder exists.

## 3. Reproducible upstream evidence

The upstream capability scan used both the formal checkout and the current
unified worktree:

```bash
rg -n -i -S -g '*.rs' -g '*.md' -g '*.toml' \
  '交割日|交割日期|delivery.?date|delivery.?day|last.?trade.?date|expire.?date|expiry.?date|CFFEX|SHFE|DCE|CZCE|INE|GFEX' \
  ../magic-market-data-rs target/magic_market_unified_work
```

Observed result: no futures-delivery record, request, provider trait, provider
implementation, or six-exchange identity. Incidental matches are K-line names,
test words such as “future source time”, and stock-option endpoints; none
provides a futures delivery schedule.

The typed core currently proves the gap:

```text
magic-market-core::Exchange =
  Shanghai | Shenzhen | Beijing

magic-market-core::AssetClass =
  Equity | Index | Fund | Bond | Option
```

There is no futures asset class or CFFEX/SHFE/DCE/CZCE/INE/GFEX venue. The
upstream `OptionContract { expiry_month, expiry, ... }` is restricted by its
Sina adapter to Shanghai ETF options and must not be reinterpreted as a futures
delivery date.

## 4. Required upstream contract

Downstream implementation is blocked until the unified upstream exposes, at
minimum, a typed contract equivalent to:

```text
FuturesDeliveryCalendarRequest {
  from_date,
  through_date,
  exchanges,
}

FuturesDeliverySchedule {
  exchange,                 // CFFEX/SHFE/DCE/CZCE/INE/GFEX
  contract_code,
  product_code,
  last_trading_date,        // optional only when provider omits it
  delivery_window_start,    // source fact, not formula output
  delivery_window_end,      // supports single-day and multi-day delivery
  settlement_method,        // optional source fact
  source_document_id,
  source_document_url,
  source_document_version,  // publication revision/version, never local code version
  source_published_at,      // optional only when provider omits it
  exchange_calendar_ref,    // official calendar/holiday revision supporting the dates
  special_rule_refs,        // product/contract/holiday/emergency adjustment notices
  supersedes,               // prior source identity when an exchange amends a schedule
  evidence,                 // provider, observed_at, immutable batch_id
}

FuturesDeliveryCalendar::delivery_schedules(request)
  -> DataBatch<FuturesDeliverySchedule>
```

The provider implementation must:

1. acquire official or explicitly approved exchange-source evidence for all
   requested venues;
2. prove pagination/completeness for the requested date interval;
3. preserve exchange publication time separately from local observation time;
4. validate exchange, contract identity, date order, duplicate/conflicting
   identities, and future source time;
5. retain the exchange publication revision, official holiday-calendar
   revision, and any product-specific, contract-specific, holiday-adjustment,
   or emergency-adjustment notice that determines the published dates;
6. treat a newer exchange amendment as a new immutable source version linked
   to the superseded record, never an in-place overwrite;
7. distinguish `Available`, `VerifiedEmpty`, `Partial`, `Unsupported`,
   `Stale`, `Conflict`, and `Unavailable`;
8. retain immutable provider/batch evidence suitable for BR-159 acquisition
   audit.

Missing one exchange cannot be reported as six-exchange coverage. A partial
batch cannot drive a whole-market “no upcoming delivery” conclusion.

The accepted source must publish actual per-contract dates. Neither the
downstream nor the unified provider may turn a generic “nth trading day”
formula into exchange evidence. Holiday calendars and special-rule notices are
retained to prove why the exchange-published date is applicable and which
revision was used, not to authorize local date invention.

## 5. Additional downstream prerequisites

Even after the upstream source contract exists, two consumer decisions remain
explicit:

- **Relevant universe:** there is no typed real futures-position snapshot or
  user-confirmed futures watchlist in `stock_analysis`. The reminder must state
  whether it covers all listed contracts, a configured product set, or verified
  account positions. It must not infer relevance from A-share holdings.
- **Advance window and ownership:** the requested lead time, dispatch time,
  same-event dedup identity, retry rules, and notification owner require a new
  business rule before implementation. The existing 19:00 R-08 scheduler alone
  does not prove a one-day-advance operational alert will be timely.

Operation guidance must be derived only from source-backed settlement method
and verified user exposure. Generic “close or roll” advice without those facts
is not authorized.

## 6. Old-module disposition

| Existing module | Decision |
| --- | --- |
| R-08 renderer, governed delivery, BR-139/BR-140 outcomes | retain; possible future consumer after upstream parity |
| local A-share trading calendar | reject as futures delivery evidence |
| Sina ETF option expiry contract | reject as futures delivery evidence |
| generic contract-month/date formulas | reject |
| downstream direct HTTP to six exchanges | reject; data acquisition belongs in unified upstream |

## 7. Acceptance criteria for a future implementation

Implementation may start only when:

- all six venue identities and `Futures` exist in the unified typed core;
- at least one provider adapter returns complete source-backed schedule batches
  for every requested venue, with an explicit unsupported state where parity is
  not yet reached;
- live probes preserve provider time, observed time, batch ID, contract ID, and
  delivery window, source revision, holiday-calendar reference, and applicable
  contract/product exception notices;
- the relevant universe and lead-time business rule are registered;
- a production test proves one source-backed schedule becomes one governed
  reminder and the immutable acquisition/delivery audits both commit;
- unavailable, partial, stale, conflicting, or unauditable evidence fails
  closed and never becomes a guessed reminder.

## 8. Rollback

This audit changes no production behavior. Rollback removes only this document
and its planning records. It must not delete or rewrite market data, positions,
notifications, or audit evidence.
