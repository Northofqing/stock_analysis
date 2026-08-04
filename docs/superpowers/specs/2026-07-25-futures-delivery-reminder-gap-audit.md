# Futures Delivery-Day Advance Reminder — Updated Capability Audit

Business rule: BR-165.

**Status:** Gate B integrated; CFFEX production capability remains unadmitted
**Date:** 2026-08-01 (live admission rechecked)
**Scope:** CFFEX implemented; SHFE, DCE, CZCE, INE and GFEX remain explicit gaps
**Data red lines:** 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10

## 1. Corrected outcome

The earlier audit concluded that the unified upstream had no futures-delivery
contract. That conclusion is superseded by the contract retained in the
currently fixed upstream revision
`5f1ce93656a55854c844065390520cd4aecd9a14`.

`magic-market-data-rs` now exposes:

- `magic_market_core::FuturesDeliveryRequest`;
- `magic_market_core::FuturesDeliveryEvent`;
- `magic_market_core::FuturesDeliveryCalendar`;
- `magic_exchange_rs::CffexClient`;
- `ProviderId::Cffex` and a strict `cffex-official-notice` diagnostic batch;
- a production capability flag that remains false until a bounded live probe
  succeeds and its evidence is reviewed.

The diagnostic provider reads only the official HTTPS CFFEX notice path and
validates IF, IH, IC and IM delivery facts. The notice does not independently
prove settlement method or last trading date, so those values remain
`NotProvided` and `None`. It explicitly refuses to infer a date from the common
“third Friday” convention.

This closes the typed-contract and downstream-integration blocker for CFFEX,
but it does not claim that the production capability is currently available.
It also does not prove delivery-calendar coverage for SHFE, DCE, CZCE, INE or
GFEX.

## 2. Current production-admission evidence

The downstream dependency and remote release were checked again on 2026-08-01:

```bash
rg -n 'magic-exchange-rs' Cargo.toml Cargo.lock
git ls-remote https://github.com/Northofqing/magic-market-data-rs.git \
  refs/heads/main
```

The downstream dependency resolves to:

```text
5f1ce93656a55854c844065390520cd4aecd9a14
```

The fixed checkout, local upstream HEAD
`546c59761a9488179d22a9f365f6e11078c6272f`, and remote `main`
`06b4d0f6295f3d138e06733927c1114c7ded146c` were inspected with:

```bash
rg -n 'futures_delivery: false|futures_delivery_calendar' \
  crates/magic-exchange-rs/src/cffex.rs \
  crates/magic-exchange-rs/tests/capabilities.rs
```

Both retain `calendar_capabilities().futures_delivery == false`, and the
formal trait returns `ExchangeError::Unsupported` before provider I/O. The
upstream immutable evidence file
`docs/evidence/2026-07-27-cffex-delivery.md` records:

```text
admission_state=failed_transport
calendar_capabilities.futures_delivery=false
formal_trait=Unsupported
```

The bounded live probe was rerun on 2026-08-01 for the 2026-08 contract month.
Both Rustls and Native TLS failed while initializing the official HTTPS
connection, before an authenticated response. A separate `curl -4` handshake
failed at the same boundary. Therefore this execution environment still has no
successful official HTTPS acceptance evidence, the upstream capability remains
false, and production must continue to return `provider_unsupported`. A
plain-HTTP result, a local formula, or calling the diagnostic probe from the
production Gateway is not an authorized workaround.

## 3. Downstream contract

`stock_analysis` consumes the released client through
`src/data_gateway/futures_delivery.rs`. Business code must not construct the
client or retain the exchange URL/parser.

Production calls only the formal
`FuturesDeliveryCalendar::futures_delivery_calendar` trait. The Provider owns
the capability admission decision; the Gateway must not duplicate its static
capability flag and must never invoke the diagnostic probe. While the fixed
revision remains unadmitted, the formal trait returns typed `Unsupported`,
which the Gateway preserves as `provider_unsupported`.

If a later reviewed upstream release admits the production capability, the
Gateway may accept a batch only when all of the following hold:

1. provenance is complete, sourced from `cffex-official-notice`, and has a
   non-empty immutable batch ID;
2. exactly one IF, IH, IC and IM contract exists for the requested year/month;
3. every record has `ProviderId::Cffex`, the same batch ID and observation
   time, an official HTTPS notice URL, and `NotProvided` delivery-method
   semantics;
4. last trading date is absent unless a future official notice explicitly
   proves it; it is never copied from delivery date;
5. all four records agree on the delivery date and the batch `source_at`
   equals the official notice publication date;
6. duplicate/conflicting contract identities reject the whole batch;
7. every provider outcome is committed to the BR-159 acquisition audit.

Missing, partial, conflicting or unauditable data is not an empty calendar.

## 4. Advance-reminder semantics

The review owner requests the contract month containing the day after the
review date. R-08 renders a reminder only when the accepted official batch
states that its delivery date is exactly the next calendar day. A complete
batch whose delivery date is not the next day is a verified “no reminder”
result.

While the production capability is unadmitted, downstream reports the
component as unsupported and does not run the diagnostic parser. If a future
admitted provider has not yet published the source notice, it returns an
explicit incomplete/unavailable outcome. Downstream must show that component
as waiting/unavailable and must not invent the date. Therefore the system can
provide a one-day-ahead reminder only after both capability admission and an
official notice published before that reminder.

The reminder identity is `(CFFEX, delivery_date, contract_code)`. R-08 derives
one canonical projection used by both rendering and durable binding: only rows
whose `delivery_date` exactly equals the reminder session are included, ordered
by contract code with product code, notice URL and optional last-trading-date
tie-breakers, and bounded to the four admitted stock-index contracts. Other
same-month rows remain covered by their immutable provider batch but are not
reminder facts.

## 5. Remaining exchange gap

No released typed provider currently proves schedules for:

- Shanghai Futures Exchange (SHFE);
- Dalian Commodity Exchange (DCE);
- Zhengzhou Commodity Exchange (CZCE);
- Shanghai International Energy Exchange (INE);
- Guangzhou Futures Exchange (GFEX).

The CFFEX result must never be labelled “all futures exchanges.” Those five
venues require their own official provider contracts and live evidence before
downstream integration.

## 6. Failure and rollback

- An unadmitted production capability is the formal Provider trait's explicit
  non-retryable `provider_unsupported`, not a downstream-invented capability
  result or a diagnostic network attempt.
- TLS, HTTP, rate-limit and anti-bot failures from the explicit diagnostic
  probe remain typed and do not admit production capability.
- Schema, identity, completeness and evidence failures reject the batch.
- HTTP downgrade, a local calendar formula and cross-source field filling are
  prohibited.
- Rollback reverts the CFFEX Gateway and R-08 consumer as one change. It does
  not delete positions, notifications, provider evidence or acquisition
  audit records, and it must not restore formula inference or an unsupported
  capability claim.
