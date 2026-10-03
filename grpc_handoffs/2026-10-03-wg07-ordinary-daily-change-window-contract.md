# WG07 explicit ordinary daily-change window, local contract v1

This is a Mac caller contract and synthetic test protocol. **No production WG07 source profile or SDK dispatcher is admitted.** The production profile registry is empty, so a live caller receives `UnsupportedProfileOrVersion` before contacting an existing service. Do not advertise or send this schema on a real SDK until that exact source/profile/version, native interpretation, capability scope and public delivery have been reviewed. HithinkFinance in the test fixture is the existing transport provider label; `TEST_CODE_SYNTHETIC` describes a synthetic protocol and is not evidence about the vendor.

The public entry points are `data_gateway::ordinary_daily_change_window::{prepare_window, consume_window}`. Both require a single Equity instrument, explicit inclusive from/to, UTC as_of, source profile `id@version`, client bundle and an existing database. Prepare returns an actually persisted receipt; consume performs a fresh acquisition and admits every anomalous pair through the same exact review-state owner. Both are separate from legacy code/days/outcome routes. No SQL tables, DDL, proto methods, descriptors or current input pins are added.

CLI:

```text
confirm_daily_change --prepare-window --exchange Shanghai --code 600519 \
  --from 2026-09-11 --to 2026-09-15 --as-of 2026-09-16T08:00:00Z \
  --source-profile delivered-id@1 --client-bundle /existing/client-bundle \
  --database /existing/review.sqlite
```

The displayed profile is a placeholder for a future reviewed delivery and is not currently accepted. Explicit `--days` and candidate/confirm/reject/renew arguments conflict with prepare. Omitted legacy days still means 60. Databases use SQLite mode=rw/ro and are never created or migrated by either entry point.

## Wire family and profile binding

Use the existing `HistoricalBars(QueryRequest)` method and operation 1. The new payload families are `magic.market.ordinary_daily_change_window.request` and `.result`, both version 1, content type `application/json; charset=utf-8`. One result record is required. Neither historical v1 nor observation v2 grants window qualification.

The request fields are shown in [synthetic-request-v1.json](fixtures/wg07/synthetic-request-v1.json). The owner fixes Day/Unadjusted, Asia/Shanghai, the actual checked-in SSE authority hash and metadata, and the exact ordered expected sessions. It freezes invocation time once. The checked civil span is at most 366 days; at most 260 sessions are inserted; an empty session vector fails. as_of must not exceed invocation and must be at/after the last requested session's Shanghai 15:00 close. Publication is proved separately. This implementation does not claim SSE applicability to Shenzhen.

The request ID lives in the existing protobuf context. Only after the complete QueryRequest is assembled are its exact bytes hashed. `request_binding.issued_query_sha256` is that SHA-256, not a self-referential field in the request. A native request must separately prove native query identity/range/selection.

The test capability exact_scope is the literal:

```text
TEST_CODE_SYNTHETIC@1;magic.market.ordinary_daily_change_window.request@1;magic.market.ordinary_daily_change_window.result@1;Equity/Day/Unadjusted/Shanghai
```

The caller requires one matching provider/operation, repository admission, runtime availability, no blocker and that exact scope before calling data RPC. Actual mTLS connection qualification, Health build identity and Capabilities remain mandatory. The old Hithink reader keeps its original selector. Failed control qualification retains an actually decoded response's bytes; a status retains code/details/trailer. Lower-level malformed control frames that the existing decoder does not deliver are not invented.

## Result and native evidence

[synthetic-result-v1.json](fixtures/wg07/synthetic-result-v1.json) contains the full strict DTO. Unknown fields, escaped object keys, unsupported variants, duplicate IDs and unclosed refs fail. `native_hex` carries exact lowercase hex bytes; its SHA is checked, but a hash alone does not grant qualification.

The fixture separately includes [native request](fixtures/wg07/synthetic-native-request-v1.json), [native response](fixtures/wg07/synthetic-native-response-v1.json) and [canonical QueryRequest protobuf hex](fixtures/wg07/synthetic-query-request.hex). Its request binding and native SHA values are deterministic. The interpreter reconstructs the native query, returned instrument/venue, adjustment, exhaustive page manifest, source version manifests, sessions and lifecycle, and requires the outer DTO to match them. Evidence refs use native response ID plus an interpreted fact ID, not arbitrary pointers. Native request and response IDs are unique; the page manifest names the actual native responses. Session/manifest/prior-revision IDs cannot collide with each other or reserved identity/range/lifecycle facts.

Each expected session has exactly one ordered terminal: Bar, Suspended, NotYetListed or Delisted. Missing, duplicate, extra, prefix and source-error windows fail. Suspended sessions only bridge when that native protocol proves the bridge; an unproved interior suspension bridge fails. Listing and delisting terminals reset adjacency. The first bar has no silently fetched predecessor.

A version availability manifest may bind many records. Its exact selection includes every session plus listing and actions. Availability, UTC precision/timezone, selected_as_of and revision are checked. Initial has native initial evidence; Replaces resolves an actual interpreted predecessor manifest with identity, availability and selected records. All selected versions must be available by as_of. Opaque revision/snapshot IDs are acquisition provenance; they do not alone change stable pair facts.

Lifecycle uses the same admitted provider/source as bars. It proves listing interval and either Complete implemented actions or explicit None. Relevant action matching follows effective/record/ex/payable dates; category must be an existing CorporateActionCategory, status Implemented, and terms remain exact native material. No Hithink-bar/TDX-lifecycle join is permitted.

Decimals are source lexemes with explicit unit and scale <=8. Prices are CNY/share, quantity share, amount CNY; parsing uses checked i128 and never guesses f64 units. OHLC and nonnegative volume/amount are checked. BR171 uses strict `abs(current - previous) * 100 > previous * 20` on aligned exact integers. The operator query's percentage text truncates toward zero to twelve fractional digits; it does not decide anomalies or schema2 stable identity. Full canonical bars, relevant action dates/terms and material suspension semantics are included in the stable fact.

## Local authority, identities and ledger

The qualified window and candidate capabilities have private construction and are non-Clone/non-Deserialize. Stored proof replay only returns ordinary validated facts; it cannot create a live capability. The library's consumer capability also has private construction and holds lossless bars, explicit bounds and proof identity.

Local snapshot branch is exactly schema 2 / `ordinary-full-window-v1`. The SQL chain remains schema 1. Scope retains the old instrument+adjacent dates formula. Domain-separated fact/request/proof/acquisition identities are specified in the implementation, with [literal golden vectors](fixtures/wg07/identity-goldens.json) and a [literal material pair](fixtures/wg07/synthetic-pair-fact-v1.json). Material facts exclude batch/request/time/outer-range and opaque evidence IDs. All acquisition provenance remains in immutable complete proof.

One IMMEDIATE transaction prepares all candidates and a `WindowObservation` event in the existing review_event table. SQL kind is Observation; candidate_id is an independent `br171_window_<acquisition>` namespace, revision 1, command `window:<acquisition>`, scope `window:<request>`. No fake candidate represents NoChanges. Loader checks row fields, chain, proof, exact receipt and every candidate/observation's window closure. Renewals keep the original proof and reach its receipt through renewal ancestry.

An exact acquisition retry returns its original immutable receipt without appending. A fresh acquisition of the same fact appends observations without refreshing expiry. Changed material facts revise/supersede the candidate. Failure on a later append rolls back the whole window. Database errors without a proven commit receipt are conservatively reported with commit_outcome Unknown and never automatically retried.

Existing confirm/reject/renew operations work with the new branch. Schema2 Confirm writes only the original review Decision event with confirmation=None. It does not write a weaker legacy confirmation or use a legacy fallback. New consumer admission requires the exact latest fact to be Confirmed. Schema1 formulas, events, old confirmation tables and old consumer routes remain unchanged. Local namespace qualification explicitly supports inherited generations 3–7, including 6/7; unknown generations or changed main/temp objects fail. This is not whole-application production pool admission.

## Bounds and failure contract

Limits apply before owned JSON/native decode or the next collection insertion: request 1 MiB; one native entry 1 MiB decoded; native cumulative 8 MiB; protobuf response and complete proof each 8 MiB; short scalar 16 KiB; native entries total 1024; sessions 260; actions 1024; candidate snapshots plus receipt 16 MiB; one ledger event 8 MiB. Proof's hex representation counts toward its 8 MiB bound. Checked bounded writers prevent serialize-then-check allocation. The WG07 codec preflights protobuf scalars and JSON before Prost builds owned response fields. Its closed 8 MiB class reuses existing actual-bound fields; the old 4 MiB class and its serialized evidence remain unchanged.

A failure is locally classified with a nonempty list, known instrument/date/range, retryability and refs; remote JSON cannot deserialize a capability or local failure authority. Independent date failures are aggregated. Actual raw transport/status material is retained after data calls, and successful acquisition proof is retained if later persistence/admission fails. Failure kinds are InvalidRequest, UnsupportedProfileOrVersion, CalendarUnavailableOrInapplicable, IncompleteSession, ConnectionQualification, CapabilityUnavailable, TransportFailure, EnvelopeRejected, RequestBindingMismatch, NativeIdentityMissingOrMismatch, AdjustmentMissingOrMismatch, UnknownSourceTerminal, IncompleteRange, DuplicateOrUnexpectedSession, SourceRejected, PublicationMissingOrAfterAsOf, RevisionMissingOrConflict, CorrectionProofMissing, LifecycleCoverageMissing, InvalidBar, AuditFailure and PersistenceFailure.

The supplied fixtures and tests establish a local synthetic contract only. Production profile admission, real native provider evidence, matching SDK delivery and natural same-version live RPC acceptance remain separate requirements.
