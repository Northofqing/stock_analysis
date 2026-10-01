# M4 historical bars: exact persisted read gap

Read-only audit at `68781c4c`, 2026-10-01. This records why a
`VerifiedHistoricalBarsSnapshot` cannot yet be constructed honestly. It does
not admit a backtest run, certify a point-in-time data set, or change legacy
acquisition and storage.

## 2026-10-02 handoff update

The Windows public bundle 2026-10-01.3 documents explicit inclusive start/end
for HithinkFinance HistoricalBars (and EmQuant, whose bridge is currently
unavailable). Mac's general acquisition still sends codes/days. The next task
is to map the existing exact-date request, verify caller-limit truncation and
per-symbol trading-date coverage, and retain an immutable request/evidence
capture. A date-range RPC alone does not establish complete coverage or PIT.
Do not continue describing the upstream as having no date-range interface.
See the [handoff task plan](../superpowers/plans/2026-10-01-windows-grpc-development-plan.md),
WG06, for the current dependency and acceptance criteria.

## Current boundaries

| Boundary | What it proves | What it does not prove |
| --- | --- | --- |
| `HistoricalBarsGateway::daily_bars{,_async}` in `src/data_gateway/historical_bars.rs` | A nonempty `AdmittedDailyBars` binds target code, records and `BatchEvidence` after Gateway admission. | Its private fields do not retain requested `days`, a requested trading-date vector, a persisted manifest hash, or a per-symbol included/skipped disposition. Admission is for the live batch, not an exact persisted replay. |
| `DatabaseManager::save_admitted_kline_data` and `get_data_range` in `src/database/kline.rs` | The former accepts admitted records and checks evidence presence; the latter returns matching `stock_daily` rows in date order. | Storage calls the ordinary UPSERT with only `evidence.source`. The table is unique by `(code, date)` and later writes replace OHLCV and source. Neither method persists or verifies the batch ID, provider, observation time, request, ordered content hash, exact coverage, or immutable version. A range query can return fewer dates than requested without error. |
| `BenchmarkReader::read_verified_exact` in `src/data_gateway/benchmark.rs` | A separate benchmark store has a retained manifest, exact expected request and complete payload verification. | Its manifest and capability cover benchmark bars, not equity `stock_daily` rows. |
| `AdmittedOutcomeDailyBars` in `src/data_gateway/outcome_daily_bars.rs` | A qualified outcome sample can bind its own exact due-date window. | This request is tied to an outcome sample and phase; it cannot stand in for a general historical backtest request. |
| `get_backtest_daily_data` and callers in `src/pipeline/backtest_runner.rs` | The live Gateway response arrives with evidence. | The return is split into `(Vec<KlineData>, BatchEvidence)`; callers take `data` and discard evidence. Failures and short histories are skipped. The full requested universe and omissions are not retained. |

The existing `stock_daily` schema in `src/database/mod.rs` has no historical
bar manifest or evidence table. A hash of rows returned by `get_data_range`
would identify those rows at read time, but would not establish the originally
requested dates, accepted provider batch, omitted symbols, or that the rows
were not overwritten. Putting a private-field wrapper around that query would
therefore falsely imply exact persisted provenance.

## Next executable slice

1. At the admitted Gateway boundary, retain the actual `code` and `days`
   request inside `AdmittedDailyBars`; no later caller may supply a replacement
   count. Add a sealed **observed capture** with that request, ordered finite
   OHLCV records, their observed per-bar adjustment semantics, a canonical
   domain-separated content hash and complete `BatchEvidence`. Name it as
   observation, not verification. Keep legacy callers working. Test a
   `TEST_CODE` admitted fixture: changing a record, request or evidence changes
   the capture identity; nonfinite values fail. This is not yet a full-window
   or persisted-read proof because `days` alone does not name exact dates.
2. Define the exact trading-date request and its authoritative calendar/source
   before a v2 backtest capture is admitted. For the requested universe, retain
   an explicit included or skipped result for every symbol. Do not infer a
   complete window from a count or silently omit failed symbols. A mismatch
   between requested and returned dates must fail admission (or remain a typed
   skip), including provider and calendar unavailability.
3. Persist immutable per-symbol captures and the complete universe disposition
   atomically in a separate content-addressed store. Bind the exact request,
   ordered bars, evidence, calendar identity and hash algorithm/schema version
   in a manifest. Then implement `read_exact(manifest_hash, expected_request)`
   that verifies hashes and exact date coverage from retained rows. Only this
   successful read may construct a private-field
   `VerifiedHistoricalBarsSnapshot`. Add a persisted SQLite fixture that
   proves equal request succeeds and tampered/missing/extra bars, request
   mismatch, evidence mismatch and skipped-symbol omission fail closed.

The first code slice should be the observed capture in step 1, with a targeted
Gateway unit test and `git diff --check`. It must not be passed to a v2 run
descriptor as a verified persisted snapshot. Steps 2 and 3 are required before
that capability and its persisted fixture can be added. Point-in-time
membership, adjustment and factor provenance remain separate M4 gates.

## Step 2 follow-up: request contract gap (2026-10-01)

Step 1's observed capture landed in `42ffb84d`; it retains the actual `days`
request and a bounded ordered row projection. A read-only Step 2 audit found
that `VerifiedReplayCalendar` in `src/calendar.rs` can return an authoritative
closed trading-date vector and authority hash from checked-in SSE data, but
that data covers only 2025 and 2026. Unsupported years fail admission.

The general `HistoricalBarsGateway` still sends only `{codes:[code],days}`
through `GrpcSource` to `market.historical_bars` v1. The response converter
parses rows without a request-bound date window. Even if a Mac-side helper
compares returned dates to a calendar vector, it can prove only that the
observed rows match that vector; it cannot prove that the provider was asked
for those exact dates. A safe local observation API may retain a per-symbol
`ObservedIncluded` or typed `Skipped` result and reject missing, duplicate,
or extra symbols. It cannot construct `VerifiedHistoricalBarsSnapshot` or a
v2 run identity.

The next admission contract requires a versioned VM/bridge HistoricalBars
request carrying `from/to` or the exact date vector, plus a response receipt
bound to its request ID/hash and calendar authority, complete batch/time
evidence, and an explicit per-symbol coverage or failure result. The provider
must implement the window semantics; a latest-N-only provider must reject an
exact-window request. Adding fields to the v1 JSON locally would not establish
that behavior. Once this contract has real same-version RPC evidence, Step 2
can compare every returned date against the verified calendar before the
immutable persisted-read work in Step 3.
