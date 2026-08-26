# NewsAI Evidence-Preserving Restoration Design

## 1. Scope and safety boundary

This design restores only security-level analysis for:

1. a `GlobalNewsRecord` whose source explicitly names the target instrument; or
2. a `SinaInstrumentNewsRecord` returned by the exact-instrument Gateway.

The producer is a derived signal, not a source fact. It therefore requires
admitted market evidence and does not receive the BR-137 DataMode-Down
exception. Until the complete path below is implemented, the long-running
monitor keeps the producer explicitly disabled under BR-112.

The following claims remain out of scope:

- “real holding” analysis without a fresh verified broker-position batch;
- instrument inference from a company-name or title substring;
- chip distribution, ROE or hard-stop values without equivalent typed facts;
- replay of legacy `news_items` rows that lack batch and record evidence;
- keyword output presented as an AI result.

## 2. Existing-module disposition

| Module | Decision | Reason |
| --- | --- | --- |
| `GlobalNewsGateway` | adopt | It preserves complete source record and batch evidence. |
| `SinaInstrumentNewsGateway` | adopt | It validates exact request/response instrument identity. |
| `HistoricalBarsGateway` | adopt | It provides admitted settled daily bars and BR-171 handling. |
| realtime/company Gateways | adopt field by field | Only fresh, present, evidence-bound fields enter the prompt. |
| `NewsAIAnalyzer` scalar interface | reject | Bare strings and numbers cannot prove source, freshness or missingness. |
| `keyword_decision` fallback | reject for AI | An LLM failure must remain a model failure. |
| aggregator `MarketEvent` projection | reject for analysis | It drops batch, item, content, instrument and topic evidence. |
| legacy `news_items` replay | reject | The table cannot reconstruct authoritative input evidence. |
| L4 reserve/commit/rollback | adopt | It already models delivery settlement correctly. |

## 3. Deep interface

`NewsAiAnalyzer::assess` accepts one `NewsAiRequest` and has no persistence,
push or dedup side effect.

The request contains:

- `AdmittedNewsFact`: original record, original `BatchEvidence`, exact target;
- `NewsMarketSnapshot`:
  - intraday: quote not older than five seconds plus admitted daily bars; or
  - post-close: settled admitted daily bars;
- optional evidence-bearing statistics, represented as
  `EvidenceStatus<T>`, never numeric sentinels;
- an analysis version.

Daily bars must be fresh within one trading day and contain enough settled
history to compute MA5/MA10/MA20, five-day return, BIAS5 and volume structure.
The admitted daily-bar capability keeps its requested canonical instrument
identity private and inseparable from records and batch evidence; NewsAI
rejects a target-code mismatch instead of accepting a caller-supplied label.
Missing optional statistics are omitted from the prompt together with a
machine-readable unavailability reason.

The result is a `NewsAiAssessment` containing:

- deterministic assessment identity;
- strict impact enum;
- confidence in `0..=100`;
- non-empty uncertainty and core logic;
- hashes of the complete input evidence and normalized prompt;
- `ModelCallReceipt` with the adapter that performed the HTTP request, the
  model returned by the upstream response, optional upstream request ID,
  mandatory upstream response ID, exact system/user prompt hashes, exact raw
  response hash and start/completion times.

The model adapter must return the receipt with the exact raw response. The
NewsAI boundary verifies that the receipt's user hash equals the normalized
request prompt, that its system hash equals the versioned NewsAI system
instruction, and that its response hash equals the bytes parsed by the strict
schema. A configuration label is not proof of the model actually called.

## 4. Strict output and failure semantics

The model response is parsed from the receipt-bearing raw content as one strict
schema. Missing fields, duplicate fields, unknown impact values, confidence
outside `0..=100`, empty required text, trailing content, timeout, transport
failure, provider fallback, missing upstream response ID or any
prompt/response hash mismatch returns a typed error.

There are no default values and no automatic keyword fallback. If a separate
heuristic classifier is retained later, it uses a distinct type, rule version
and audit identity; no match returns `NoDecision`.

Important failure states include:

- `NewsEvidenceMismatch`
- `InstrumentNotSourceBound`
- `InsufficientContent`
- `MarketEvidenceMismatch`
- `StaleQuote`
- `StaleDailyBars`
- `InsufficientDailyHistory`
- `ModelUnavailable`
- `ModelReceiptMissing`
- `InvalidModelSchema`
- `AnalysisAuditFailed`
- `PredictionCommitFailed`
- `DeliveryDenied`
- `DeliverySinkFailed`

No failure is converted to an empty or neutral assessment.

## 5. Audit, delivery and dedup state machine

The producer follows this sequence:

1. acquire and validate news and market batches;
2. call the model and validate its receipt/schema;
3. append an immutable assessment record containing all evidence hashes;
4. reserve delivery identity
   `(news provider, source batch ID, item ID, code, analysis version)`;
5. run normal L4-L7 governance and the sink;
6. on confirmed physical delivery and authoritative delivery audit, commit
   the reservation and append the prediction settlement link;
7. on Denied, SinkError or pre-delivery audit failure, rollback the
   reservation and preserve retry eligibility;
8. on Deduped, do not create another prediction;
9. on a post-sink audit failure, follow BR-145 incident recovery and never
   call the sink a second time for the same receipt.

Neutral or `NoDecision` assessments may be retained in the immutable analysis
audit but do not reserve a delivery identity.

The assessment and prediction stores must reject UPDATE/DELETE, validate the
existing SHA-256 chain before append, bind test/live identity, and propagate
every database error. Retention is at least five years.

## 6. News pipeline relationship

The four-source aggregator may continue emitting `MarketEvent` for
source-fact notifications. In parallel, the exact admitted
`GlobalNewsRecord + BatchEvidence` outcome is passed to evidence-requiring
consumers. The two views share one source identity but the lossy event is never
promoted back into an analysis fact.

Session `seen`, daily critical budget and aggregate-window state become
reservations. Entity mismatch, classification failure, analysis failure,
governance denial, sink failure and audit failure do not permanently advance
those states.

## 7. Rollout

1. Introduce types, strict parser and pure evidence validation.
2. Add model receipts without enabling the monitor producer.
3. Add immutable assessment/prediction linkage and failure tests.
4. Add two-phase dedup/budget/window settlement.
5. Enable only exact source-bound targets in the bounded governed producer.
6. Validate real provider/model receipts and real sink audit.
7. Enable governed production output; retain explicit unsupported states for
   every excluded claim.

The governed producer is enabled from the admitted aggregator tick and permits
at most one in-flight batch per process. A busy worker skips the new attempt
without recording completion.
On each aggregator tick it forms exact
source-bound candidates, orders and deduplicates them by
`(provider, batch, item, canonical code, analysis version)`, and evaluates at
most five. A matching immutable assessment identity is loaded as an audited
delivery capability before model selection, so an existing assessment can
retry an incomplete governed delivery without another model call. A new model
result must be appended to the immutable assessment chain before delivery.
Non-neutral assessments then use the exact BR-172 identity ledger:
`reserve -> sink_started -> delivered -> prediction_linked`; governance denial
or a definite pre-sink failure rolls back the reservation, while an attempted
sink or uncertain post-sink audit writes recovery state and is not
automatically resent. Neutral assessments remain audit-only. The producer has
no order capability.
Each selected provider HTTP call has a 45-second timeout and is never retried
against a different provider.

## 8. Validation and release gates

Focused tests must cover every item listed under BR-172, including evidence
mismatch, missing content, freshness, MA20 history, optional-field omission,
strict schema, actual model receipt, no heuristic fallback, test/live
isolation, immutable audit, prediction error propagation and delivery
reserve/sink-start/commit/rollback/uncertain-recovery. Gate tests must prove
that the typed NewsAI path consumes combined-account context, requires
`DataMode::Full`, bypasses the weaker legacy `(kind, code, cooldown)` dedup
owner and never gains a generic DataMode-Down exemption.

The final release additionally requires repository formatting, strict Clippy,
all tests, compliance, coverage thresholds, bounded real-data validation,
`monitor --review`, isolated `monitor --test` and a bounded normal-monitor
smoke run.

## 9. Rollback

Disable the producer and revert its scheduler/consumer wiring. Keep the
evidence-preserving Gateways, immutable audit rows and normal delivery
governance. Rollback must not restore the scalar interface, default values,
keyword-as-AI fallback, early dedup advancement or legacy acquisition.
