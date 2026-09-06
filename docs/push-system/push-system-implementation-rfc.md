# Push system implementation RFC

Status: **PROVISIONAL**. Version: `push-system-rfc-v1`. This document specifies
PROPOSED application contracts, not deployed runtime behavior or remote receipt.
Task2 delivers domain types, application outcomes, source mappings and ReasonCode.
DDL/recovery, operational gates and WBS are separate Task3/4/5 deliverables;
this document makes no completion claim for them. [Q:56] [Q:61] [Q:69] [Q:108]

## Metadata

```json
{
  "schema_version": 1,
  "status": "PROVISIONAL",
  "version": "push-system-rfc-v1",
  "source_baseline": "07781bf386aafdf202851ae928efee8920387058",
  "input_manifest_sha256": "6a74428f1cc18cc1b0800ab86be19e3d8afdaafd0107d7656a2d3a5857f18aab",
  "catalog_sha256": "0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3",
  "evidence_manifest_sha256": "54dc705961da7a6deb458009d2125ee612257d82bad3c14b65d25642e09b64fa",
  "decisions_sha256": "55354916a4b03401afa771e2f4e149bc1189222fc3c76d89aeb5ad79c086e794",
  "counts": {"kinds": 65, "producers": 102, "units": 52, "evidence": 195, "mapped": 26, "durable_kinds": 23, "unmapped": 39, "states": 14},
  "status_counts": {"ACTIVE": 36, "INACTIVE": 22, "STARVED": 5, "OPT-IN": 2}
}
```

## Scope and authority

Normative priority: approved Q1–Q108 > frozen 65-kind/102-producer/52-Unit catalog
and 195 symbol evidence > frozen Rust mapping/state > blueprint proposals/recent
samples. CURRENT means code wiring at the metadata baseline, not live activation.
PROPOSED means this RFC's future contract. Historical 37/24/2/2, 35 Units and old
estimates are not this baseline. PaperBuy/Watchdog remain excluded worktree
additions; enum-external CLI producers remain in the catalog. [Q:21] [Q:30]
[Q:59] [Q:65] [Q:93] [evidence:push-kind] [unit:MU-cli-single]

Implementation topology is Foundation → atomic Unit slices → tail cleanup.
Four phases are Epics, never physical-owner boundaries. No Unit activation,
HTML/CI release, runtime deployment or user receipt is proved here. [Q:16]
[Q:17] [Q:23] [Q:26] [Q:42] [Q:74]

## Navigation and canonical rules (PROPOSED)

The sections below specify field contracts, JobDecision, DeliveryResult, completion
branches, CURRENT mapping, CURRENT durable states, ReasonCode and adapter conformance.
References use square-bracketed `Q:number`, `unit:ID`, `producer:ID` and `evidence:ID`; IDs
are parsed against the frozen documents, not accepted by keyword presence. [Q:59] [Q:66]

`prepare(RunContext) -> PreparedFacts -> project() -> Ready(PreparedPush) ->
business intent -> DeliveryCoordinator -> VerifiedTerminalRef -> Finalizer(CompletionPolicy)`.
Non-send JobDecision proposals use the separate completion branch rules below and
never manufacture a terminal ref. Callers do not interpret bool/Ok, send directly
after a refusal, or advance completion themselves. [Q:27] [Q:34] [Q:78] [Q:86]

Canonical v1 uses a domain label (the type name plus schema/version), then UTF-8
JSON with lexicographically sorted object keys, no insignificant whitespace or
trailing newline, unique keys, integers in minimal decimal, no floats/nonfinite
values, and JSON string escaping. Strings are not normalized after capture.
Dates are validated YYYY-MM-DD; UtcMicros is signed i64 UTC microseconds. Sha256 is
64 lowercase hex; GitSha40 is 40 lowercase hex. ExactBytes hashes the original
bytes directly; canonical envelopes represent bytes as their SHA and byte length.
Ordered arrays preserve captured order. Option is explicit null, never omitted.
NonEmptyText/IDs are bounded validated text in their registered source contract.
OccurrenceId is the typed tuple (business_date, registered occurrence family,
family_key); its namespace and Unit are bound by the enclosing identity. Reusing
a display label across business dates cannot collide. A SourceRef carries the
provider identity, external item identity, contract identity and immutable content
hash, not permission to send. These value types are constructor-validated, not
unrestricted String aliases. [Q:16] [Q:59] [Q:89]
Every table field is mandatory, including explicit Option values. [Q:10] [Q:19]
[Q:33] [Q:40] [Q:72] [Q:89]

The canonical column describes inclusion in that type's envelope: include = the
typed field; external exact bytes = length + SHA in envelope, original bytes in
immutable payload; derived self-excluded = derived material excluded from its own
hash, while downstream bindings include its digest. Every field inherits its
table's creator/consumer and prohibition: no mutation after construction, no
unvalidated deserialization, no caller-forged authority. A constructor rejects
inconsistent cross-field bindings; it never repairs them silently. A business
retry reloads the original RunContext/PreparedFacts/PreparedPush, rather than
capturing a new run ID, clock, provider response or template for the same intent.
[Q:10] [Q:33] [Q:72] [Q:78] [Q:89]

## Type: RunContext (PROPOSED)

Creator: RunContextFactory (scheduler/event/manual adapter). Consumer: prepare, project, coordinator, finalizer.
Field lifecycle and canonical rules above apply to **every row**.
[Q:16] [Q:33] [Q:40] [Q:74] [Q:79] [unit:MU-p01] [producer:p01-scheduled]

| field | type | invariant | canonical |
| --- | --- | --- | --- |
| schema_version | u32 | Exactly 1; unknown schema rejected | include |
| run_id | RunId | First preparation identity; reload the original on replay/restart | include |
| unit_id | UnitId | Existing catalog Unit; producer belongs to this Unit | include |
| namespace | Namespace | Production or Test(run_id); never mix namespaces | include |
| business_date | Date | Calendar authority decision captured once | include |
| calendar_date | Date | Captured local calendar day; not a substitute for business_date | include |
| phase | PhaseEpic | Preopen, Auction, Intraday or Postclose; not owner identity | include |
| trigger | Trigger | Scheduled(schedule_id), Event(producer_id,source_ref) or Manual(command_id,authenticated_operator_ref) | include |
| occurrence | OccurrenceId | Canonical business occurrence from registered producer family | include |
| captured_business_time | UtcMicros | One captured clock value, not shadow wall time | include |
| activation_generation | u64 | Generation of the approved Unit owner fence | include |
| build_commit | GitSha40 | Exactly the build identity checked by activation | include |
| catalog_sha256 | Sha256 | Exactly the catalog used to resolve Unit/owner | include |
| source_contract_version | NonEmptyText | Approved version, not inferred from payload | include |
| template_version | NonEmptyText | Frozen first-render version for this occurrence | include |

## Type: PreparedFacts (PROPOSED)

Creator: prepare: admitted source adapters + immutable constructor. Consumer: project in both active and shadow.
Field lifecycle and canonical rules above apply to **every row**.
[Q:33] [Q:40] [Q:72] [Q:83] [unit:MU-news-ai] [producer:news-ai-same-tick]

| field | type | invariant | canonical |
| --- | --- | --- | --- |
| run_context_sha256 | Sha256 | Binds the captured RunContext canonical bytes | include |
| source_contract_id | NonEmptyText | Names the producer source contract | include |
| source_contract_version | NonEmptyText | Equals RunContext source contract version | include |
| source_refs | Vec<SourceRef> | Ordered unique refs with provider, external_id, source_contract, content_sha256; order frozen before capture | include |
| canonical_facts | ExactBytes | Validated source facts with stable field schema; not renderer text | external exact bytes |
| facts_sha256 | Sha256 | SHA256(canonical_facts), never recomputed from a later provider batch | derived self-excluded |
| provider_observed_at | Vec<SourceTime> | One source_ref_id and observed_at/as_of UtcMicros pair per source; unknown time is explicit None, not now | include |
| verified_empty | bool | True only after successful scoped source verification; failures never become empty | include |
| model_output_refs | Vec<ModelOutputRef> | Ordered model/version/input_sha256/output_sha256/protected_ref; captured once, empty when unused | include |

## Type: SemanticProjection (PROPOSED)

Creator: project(PreparedFacts) pure function. Consumer: shadow comparator and PreparedPush constructor.
Field lifecycle and canonical rules above apply to **every row**.
[Q:19] [Q:40] [Q:72] [Q:83] [unit:MU-p01] [producer:p01-compensation]

| field | type | invariant | canonical |
| --- | --- | --- | --- |
| audience | AudienceId | Explicit immutable routing audience; never guessed from logs | include |
| monitor_kind | Option<MonitorKind> | One of frozen 65 monitor variants, or None only for enum-external catalog producer | include |
| sub_kind | SubKind | None or approved kind-specific value; not another physical owner | include |
| occurrence | OccurrenceId | Equals RunContext occurrence | include |
| business_subject | SubjectId | Typed Global or canonical code/subject; no display-text parsing | include |
| severity | Severity | Emergency, Important, Info or Research; cannot authorize uncertain retry | include |
| suppression | Suppression | Eligible or Suppressed(reason,eligible_after) | include |
| completion_policy_id | NonEmptyText | Registered catalog-bound policy identity | include |
| completion_policy_version | NonEmptyText | Version used by both shadow and active | include |
| evidence_fingerprint | Sha256 | SHA256(ordered source refs and captured model refs) | include |
| template_id | NonEmptyText | Registered renderer identity | include |
| template_version | NonEmptyText | Equals RunContext template_version | include |
| canonical_bytes | ExactBytes | Encoding of preceding semantic fields only | derived self-excluded |
| sha256 | Sha256 | SHA256(canonical_bytes) | derived self-excluded |

## Type: PreparedPush (PROPOSED)

Creator: Ready constructor after pure project + first render. Consumer: business intent store and DeliveryCoordinator.
Field lifecycle and canonical rules above apply to **every row**.
[Q:10] [Q:72] [Q:76] [Q:89] [unit:MU-p01] [evidence:counted-envelope]

| field | type | invariant | canonical |
| --- | --- | --- | --- |
| intent_id | IntentId | SHA256(namespace,Unit,registered owner,source-contract ID,occurrence,subject,audience); never contains payload hash | include |
| decision_id | DecisionId | Stable authority-scoped derivative of intent_id; same across attempt/restart | include |
| unit_id | UnitId | Equals captured RunContext and registered completion owner | include |
| occurrence | OccurrenceId | Equals RunContext and SemanticProjection | include |
| subject | SubjectId | Equals SemanticProjection business_subject | include |
| run_context_sha256 | Sha256 | Original RunContext canonical binding | include |
| prepared_facts_sha256 | Sha256 | Canonical PreparedFacts envelope binding, including facts_sha256 | include |
| semantic_projection_sha256 | Sha256 | Equals SemanticProjection sha256 | include |
| source_binding | SourceBinding | Source-contract ID/version plus ordered source refs and evidence fingerprint | include |
| rendered_bytes | ExactBytes | Original rendered UTF-8 bytes, including intentional whitespace; never render again on replay | external exact bytes |
| rendered_sha256 | Sha256 | SHA256(rendered_bytes); drift on same intent requires ResolutionRequired | include |

## Type: VerifiedTerminalRef (PROPOSED)

Creator: private authority adapter query + exact-binding verification. Consumer: DeliveryResult and finalizer re-query/reverify.
Field lifecycle and canonical rules above apply to **every row**.
[Q:7] [Q:46] [Q:55] [Q:78] [Q:87] [evidence:startup-kind-map] [unit:MU-p01] [unit:MU-news-flash-aggregate]

| field | type | invariant | canonical |
| --- | --- | --- | --- |
| ref_id | TerminalRefId | Stable authority disposition identity, not a log ID | include |
| authority_class | AuthorityClass | GenericCounted, P01Dedicated or N02Dedicated; adapter must pass conformance | include |
| namespace | Namespace | Exact original intent namespace and audience scope | include |
| decision_id | DecisionId | Exact persisted immutable decision identity | include |
| attempt_id | Option<AttemptId> | Binds outcome to attempt; None only for a verified pre-attempt denial or manual disposition | include |
| intent_id | IntentId | Exact requested business intent | include |
| unit_id | UnitId | Same Unit and owner binding as requested intent | include |
| occurrence | OccurrenceId | Exact original business occurrence, not current tick | include |
| business_date | Date | Exact captured business date | include |
| subject | SubjectId | Exact business subject | include |
| audience | AudienceId | Exact target audience | include |
| template_id | NonEmptyText | Exact persisted template identity, not only a coincident version label | include |
| template_version | NonEmptyText | Exact persisted template version | include |
| rendered_sha256 | Sha256 | Exact persisted rendered bytes hash | include |
| terminal_disposition | TerminalDisposition | Accepted, Rejected, Uncertain, ManualConfirmedAccepted or ManualConfirmedNotDelivered; manual is never TransportAccepted | include |
| evidence_sha256 | Sha256 | Verified receipt/disposition OR authenticated manual-resolution evidence hash; no copied receipt payload | include |
| durable_schema_version | NonEmptyText | Authority's compatible persisted schema version | include |
| verified_at | UtcMicros | Fresh re-query time; audit only, excluded from stable binding digest and shadow semantics | derived self-excluded |
| binding_sha256 | Sha256 | Canonical hash of preceding stable binding fields excluding verified_at and this digest; verify again at finalization | derived self-excluded |

## Type: CompatibilityEvidenceRef (PROPOSED)

Creator: NotificationService compatibility adapter. Consumer: CLI/report observer only; never authoritative finalizer.
Field lifecycle and canonical rules above apply to **every row**.
[Q:4] [Q:7] [Q:44] [unit:MU-cli-single] [producer:cli-single-default]

| field | type | invariant | canonical |
| --- | --- | --- | --- |
| compat_id | CompatId | Local invocation/result identity, not durable receipt | include |
| intent_id | IntentId | Report invocation intent; does not open production durable DB | include |
| unit_id | UnitId | Existing CLI/compatibility Unit | include |
| occurrence | OccurrenceId | Same report invocation/occurrence | include |
| configured_channels | Vec<ChannelId> | Ordered unique configured channels at invocation | include |
| attempted_channels | Vec<ChannelId> | Subset of configured channels actually attempted | include |
| weak_outcomes | Vec<WeakOutcome> | Per attempted channel: Accepted, Rejected or Unknown with local evidence ref; no authority assertion | include |
| local_evidence_sha256 | Sha256 | Hash of local per-channel facts; not a remote receipt hash | include |
| observed_at | UtcMicros | Local observation time, not remote accepted_at | include |
| not_authoritative | TrueLiteral | Must be true; conversion to VerifiedTerminalRef is forbidden | include |

## Type: CompletionPolicy (PROPOSED)

Creator: versioned catalog policy registry; no caller-defined policy. Consumer: project proposal and registered finalizer.
Field lifecycle and canonical rules above apply to **every row**.
[Q:2] [Q:9] [Q:73] [Q:85] [Q:86] [Q:87] [Q:88] [unit:MU-p01] [unit:MU-cli-single]

| field | type | invariant | canonical |
| --- | --- | --- | --- |
| id | NonEmptyText | Stable registered policy ID | include |
| version | NonEmptyText | Frozen version, changes require new approved contract | include |
| completion_owner | CatalogOwnerRef | Unit + catalog SHA + exact completion-owner identity; not PushKind or transport quota | include |
| advance_event | AdvanceEvent | AcceptedBound or AcceptedOrManualBound; never bool/Ok/local audit | include |
| schedule_close_policy | ScheduleClosePolicy | OnAccepted, VerifiedNoData, ExplicitDisabled or SuppressedOccurrence; selected branches only | include |
| notification_cursor_policy | CursorPolicy | AcceptedBoundOnly or Never; schedule closure does not imply cursor movement | include |
| no_data_policy | NoDataPolicy | KeepOpen or CloseVerifiedOccurrence; requires verified_empty and facts binding | include |
| disabled_policy | DisabledPolicy | KeepOpen or CloseDisabledOccurrence; never activates disabled producer | include |
| retry_policy | RetryPolicy | Never, InputBackoff(not_before) or AuthorizedRejected(not_before,max_attempts); bounded by disposition/fence | include |
| uncertain_manual_policy | UncertainPolicy | QuarantineThenVerifiedManual; no blind retransmission at any severity | include |
| already_terminal_policy | AlreadyTerminalPolicy | ReverifyExactBinding; independently distinguish manual acceptance and transport acceptance | include |
| allowed_authority | Vec<AuthorityClass> | Explicit conformance-approved adapters; COMPAT is never an authority class | include |
| finalizer_kind | FinalizerKind | BoundCursor, ScheduleOnly or CompatibilityObservation; only registered owner may act | include |
| retention_class | RetentionClass | Strictest applicable migration/regulatory/model/trade class; nonterminal never auto-cleaned | include |

## Type: JobDecision (PROPOSED)

Creator: pure project(PreparedFacts). Consumer: application dispatcher and
CompletionPolicy proposal builder. Canonical material is the variant tag and all
typed payload fields (nested objects use the bindings above); diagnostic prose is
excluded. ReasonCode fields below must be registry members. Option<UtcMicros>
means a captured eligibility bound, not permission to schedule a retry blindly.
No variant can invoke provider/LLM/send or advance a cursor. [Q:33] [Q:34]
[Q:72] [Q:85] [Q:86] [unit:MU-news-ai] [producer:news-ai-same-tick]

| variant | payload | allowed_input | proposal | forbidden | refs |
| --- | --- | --- | --- | --- | --- |
| Ready | PreparedPush | Verified captured facts and eligible policy | Persist immutable intent; dispatch via coordinator | Direct send or completion | [Q:72] |
| NoData | {reason:ReasonCode,evidence_sha256:Sha256} | Successful verified-empty source bound to PreparedFacts | Policy-selected schedule closure only | Turn source failure into empty; advance notification cursor | [Q:85] [Q:86] |
| Disabled | {reason:ReasonCode} | Explicit disabled policy/activation snapshot | Keep open or close disabled occurrence by policy | Activate producer or claim accepted | [Q:21] [Q:30] |
| BlockedOnInput | {reason:ReasonCode,retry_after:Option<UtcMicros>} | Missing/unready or invalid source evidence | Keep occurrence pending; isolate producer | Fabricate facts or business completion | [Q:12] |
| Suppressed | {reason:ReasonCode,eligible_after:Option<UtcMicros>} | Captured policy suppression/cooldown | Keep open or explicit suppressed schedule closure | Infer delivery from suppression | [Q:19] [Q:86] |
| RetryableFailure | {reason:ReasonCode,retry_after:Option<UtcMicros>} | Classified pre-send recoverable failure | Bounded preparation retry under policy | Reclassify uncertain send as pre-send retry | [Q:9] [Q:85] |
| PermanentFailure | {reason:ReasonCode} | Classified nonrecoverable preparation/contract failure | Stop and expose typed failure | Silent drop marked Completed | [Q:12] [Q:101] |

## Type: DeliveryResult (PROPOSED)

Creator: coordinator's verified authority adapter or isolated compatibility
adapter. Consumer: application caller for observation, then the registered
CompletionPolicy/finalizer. Canonical material is variant + typed reference binding
(or ReasonCode); it never embeds a copied receipt. Only a private authority query
creates VerifiedTerminalRef, and finalizer queries/revalidates it again. A
CompatibilityEvidenceRef has no conversion API to that type. [Q:7] [Q:27]
[Q:46] [Q:78] [Q:87] [unit:MU-cli-single] [producer:cli-single-default]

| variant | payload | authority | authoritative_completion | condition | refs |
| --- | --- | --- | --- | --- | --- |
| TransportAccepted | VerifiedTerminalRef | strong | policy_bound | Reverified disposition Accepted with exact bound receipt; not manual or local audit | [Q:2] [Q:7] |
| TransportRejected | VerifiedTerminalRef | strong | never | Reverified Rejected; only explicit current retry authorization permits a new attempt | [Q:9] [Q:85] |
| TransportUncertain | VerifiedTerminalRef | strong | never | Reverified Uncertain; quarantine/manual resolution, never blind resend | [Q:9] |
| AlreadyTerminal | VerifiedTerminalRef | strong | policy_bound | Fresh exact-binding query; inspect actual disposition, never infer Accepted from this variant | [Q:46] [Q:87] |
| BestEffortAccepted | CompatibilityEvidenceRef | compat | never | All configured attempted channels weakly accepted; no remote authority claim | [Q:4] [Q:7] |
| PartiallyAccepted | CompatibilityEvidenceRef | compat | never | At least one weak acceptance and at least one configured channel not accepted | [Q:4] [Q:7] |
| NoChannelConfigured | ReasonCode | compat | never | Empty configured channels; transport.no_channel_configured | [Q:7] [Q:101] |
| AllChannelsFailed | CompatibilityEvidenceRef | compat | never | No channel weakly accepted, nonempty configured set; Unknown remains explicitly unknown | [Q:7] |
| Blocked | ReasonCode | none | never | Authority query, policy, lease or binding failed; not a transport attempt receipt | [Q:78] [Q:101] |

COMPAT branches are not_authoritative: they MUST NOT construct VerifiedTerminalRef,
project to TransportAccepted, open production durable SQLite, or advance
authoritative completion. A weak Unknown may not be retried as definite rejection.
Business Completed/NoData/Disabled are not DeliveryResult variants. ManualConfirmedAccepted
is preserved as a distinct terminal disposition in AlreadyTerminal; it has separate
evidence and metrics and never becomes TransportAccepted. [Q:4] [Q:7] [Q:46]
[Q:87] [unit:MU-cli-summary] [producer:cli-summary-default]

## Completion branches (PROPOSED)

Only finalizer applies a proposal to its registered completion owner. Application
callers cannot commit it. These are policy choices, not an implemented SQL state
machine. ScheduleOnly/CompatibilityObservation can never advance a notification
cursor. BoundCursor requires AcceptedBoundOnly plus an allowed authority and exact
identity, namespace, occurrence, business date, subject, audience, template and
render hash binding. Manual acceptance additionally requires AcceptedOrManualBound;
manual non-delivery never counts as acceptance. [Q:2] [Q:73] [Q:78] [Q:85]
[Q:86] [Q:87] [unit:MU-p01] [unit:MU-cli-single]

| input | allowed_policy | schedule_proposal | cursor_proposal | forbidden | refs |
| --- | --- | --- | --- | --- | --- |
| Ready | Any registered policy | KeepOpen until delivery result | None | Early completion on intent persistence | [Q:2] |
| NoData | KeepOpen or CloseVerifiedOccurrence | KeepOpen or Close only with verified_empty evidence | None | Closing unverified empty source | [Q:85] [Q:86] |
| Disabled | KeepOpen or CloseDisabledOccurrence | KeepOpen or explicit disabled closure | None | Activating INACTIVE/STARVED/OPT-IN | [Q:21] [Q:30] |
| BlockedOnInput | InputBackoff or Never | KeepOpen | None | Source failure hidden as NoData | [Q:12] |
| Suppressed | SuppressedOccurrence or KeepOpen | Explicit suppressed closure or KeepOpen | None | Suppressed equals delivered | [Q:19] [Q:86] |
| RetryableFailure | InputBackoff or Never | KeepOpen; respect not_before | None | Retry post-send Uncertain here | [Q:9] |
| PermanentFailure | Never | KeepOpen with typed stop reason | None | Silent success | [Q:101] |
| TransportAccepted | AcceptedBound or AcceptedOrManualBound | OnAccepted may close | BoundCursor may AdvanceAccepted; Never remains None | Wrong owner or binding | [Q:2] [Q:78] |
| TransportRejected | AuthorizedRejected or Never | KeepOpen; honor explicit durable retry permission and max_attempts | None | Retry without authorization | [Q:85] |
| TransportUncertain | QuarantineThenVerifiedManual | KeepOpen; retain evidence | None | Any blind resend | [Q:9] [Q:88] |
| AlreadyTerminal | ReverifyExactBinding | Inspect disposition; only accepted branches permit OnAccepted | AcceptedBoundOnly plus Accepted or permitted ManualConfirmedAccepted; else None | ManualConfirmedNotDelivered or Rejected treated as Accepted | [Q:46] [Q:87] |
| BestEffortAccepted | CompatibilityObservation | Local observation only | None | VerifiedTerminalRef conversion | [Q:7] |
| PartiallyAccepted | CompatibilityObservation | Local per-channel observation only | None | Hide failed/unknown channel | [Q:7] |
| NoChannelConfigured | CompatibilityObservation | Local no-channel observation only | None | Count as attempted or delivered | [Q:7] |
| AllChannelsFailed | CompatibilityObservation | Local failure observation only | None | Retrying Unknown as definite failure | [Q:7] [Q:9] |
| Blocked | Never or bounded pre-send InputBackoff | KeepOpen; preserve reason | None | Bypass coordinator or binding gate | [Q:78] |

Retain nonterminal facts indefinitely until resolved; terminal migration evidence
at least 90 days, with stricter regulatory/model/trade retention taking precedence.
This is a policy field contract, not a cleanup implementation. [Q:48] [Q:88]

## Monitor to durable mapping (CURRENT)

Source: `src/bin/monitor/durable_delivery_runtime.rs::durable_kind_and_sub_kind_with_override`
and `src/durable_delivery/model.rs::PushKind` at the frozen baseline.
26 monitor variants map to 23 durable variants; DailyReport sub-kind preserves
FactorIC/SectorTier/CapitalVerify. Mapping presence does not create a producer,
activity, receipt or Unit. Existing stored-decision recovery does not activate
normal INACTIVE loaders. [Q:21] [Q:30] [Q:75] [evidence:startup-kind-map]
[evidence:startup-resume] [evidence:push-kind]

| monitor_kind | durable_kind | sub_kind | refs |
| --- | --- | --- | --- |
| HoldingPlan | HoldingPlan | None | [evidence:startup-kind-map] |
| HoldingEvent | HoldingEvent | None | [evidence:startup-kind-map] |
| T0Advice | T0Advice | None | [evidence:startup-kind-map] |
| CandidateTriggered | CandidateTriggered | None | [evidence:startup-kind-map] |
| PreopenNewsHot | PreopenNewsHot | None | [evidence:startup-kind-map] |
| CloseCall | CloseCall | None | [evidence:startup-kind-map] |
| ForbiddenOps | ForbiddenOps | None | [evidence:startup-kind-map] |
| PaperTrade | PaperTrade | None | [evidence:startup-kind-map] |
| ReviewMarket | ReviewMarket | None | [evidence:startup-kind-map] |
| ReviewLhb | ReviewLhb | None | [evidence:startup-kind-map] |
| ReviewSignal | ReviewSignal | None | [evidence:startup-kind-map] |
| ReviewFailure | ReviewFailure | None | [evidence:startup-kind-map] |
| TomorrowWatch | TomorrowWatch | None | [evidence:startup-kind-map] |
| EventCalendar | EventCalendar | None | [evidence:startup-kind-map] |
| ReviewProviderTopN | ReviewProviderTopN | None | [evidence:startup-kind-map] |
| SectorTop | SectorTop | None | [evidence:startup-kind-map] |
| SectorAnomaly | SectorAnomaly | None | [evidence:startup-kind-map] |
| IndustryChain | IndustryChain | None | [evidence:startup-kind-map] |
| PositionReview | PositionReview | None | [evidence:startup-kind-map] |
| ReviewBacktest | ReviewBacktest | None | [evidence:startup-kind-map] |
| WatchlistTracking | WatchlistTracking | None | [evidence:startup-kind-map] |
| CatalystReview | CatalystReview | None | [evidence:startup-kind-map] |
| FactorIC | DailyReport | FactorIC | [evidence:startup-kind-map] |
| SectorTier | DailyReport | SectorTier | [evidence:startup-kind-map] |
| CapitalVerify | DailyReport | CapitalVerify | [evidence:startup-kind-map] |
| DailyReport | DailyReport | requested FactorIC/SectorTier/CapitalVerify or None | [evidence:startup-kind-map] |

## Unmapped monitor kinds (CURRENT status; PROPOSED treatment)

These 39 kinds have no direct generic counted mapping. adapt_or_conform means
preserve an existing stronger dedicated protocol where present, then supply the
shared application/intent/finalizer boundary; it does not mean rewrite every
authority. retain_starved/retain_opt_in never restore or enable input by this RFC.
INACTIVE creates no new scheduler or producer. Enum-external CLI paths are already
part of the 102-producer/52-Unit catalog, not part of this 65-kind subtraction.
[Q:21] [Q:27] [Q:28] [Q:30] [Q:93] [unit:MU-news-flash-aggregate]
[producer:news-flash-aggregate] [unit:MU-cli-single]

| monitor_kind | status | treatment | refs |
| --- | --- | --- | --- |
| Announcement | ACTIVE | adapt_or_conform | [evidence:news-loop] [producer:news-announcement] |
| AuctionVolume | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:auction-volume] |
| VirtualWatch | STARVED | retain_starved | [evidence:monitor-loop] [producer:virtual-watch-pilot] |
| LimitBoards | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:limit-boards-first] |
| FundInflow | INACTIVE | keep_inactive | [evidence:push-kind] |
| AuctionRepush | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:auction-repush] |
| WeeklySOP | INACTIVE | keep_inactive | [evidence:push-kind] |
| StockPick | INACTIVE | keep_inactive | [evidence:push-kind] |
| TurnoverTop | INACTIVE | keep_inactive | [evidence:push-kind] |
| CandidateBoard | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:candidate-board] |
| NewsRanked | INACTIVE | keep_inactive | [evidence:dispatch-disabled] |
| AccountMode | ACTIVE | adapt_or_conform | [evidence:monitor-main] [producer:account-mode-main] |
| DataMode | ACTIVE | adapt_or_conform | [evidence:monitor-main] [producer:data-mode] |
| PaperSell | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:paper-sell-intraday] |
| SnapshotStale | ACTIVE | adapt_or_conform | [evidence:monitor-main] [producer:snapshot-stale-startup] |
| AttributionDaily | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:attribution-daily] |
| G5bAttribution | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:g5b-attribution] |
| IntradayMarket | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:market-view-periodic] |
| NewsCatalyst | ACTIVE | adapt_or_conform | [evidence:news-loop] [producer:catalyst-announcement] |
| NewsToIdea | ACTIVE | adapt_or_conform | [evidence:news-loop] [producer:d01-announcement] |
| IndustryChainIntraday | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:industry-chain-periodic] |
| PostFixedPriceOrder | STARVED | retain_starved | [evidence:monitor-loop] [producer:post-fixed-order] |
| PostFixedPriceFill | STARVED | retain_starved | [evidence:monitor-loop] [producer:post-fixed-fill] |
| StPriceLimitChanged | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:st-price-limit-batch] |
| EtfClosingCallAuction | INACTIVE | keep_inactive | [evidence:etf-unused] |
| BlockTradeIntradayConfirm | ACTIVE | adapt_or_conform | [evidence:review-batch] [producer:block-confirm-side-route] |
| BlockTradePriceRange | INACTIVE | keep_inactive | [evidence:block-review] |
| PaperReview | STARVED | retain_starved | [evidence:monitor-loop] [producer:paper-review-noon] |
| CandidateInvalidated | ACTIVE | adapt_or_conform | [evidence:candidate-board] [producer:candidate-invalidated] |
| IpoListingApproval | INACTIVE | keep_inactive | [evidence:review-manual] |
| IpoProspectus | INACTIVE | keep_inactive | [evidence:review-manual] |
| IpoCatalyst | ACTIVE | adapt_or_conform | [evidence:review-batch] [producer:ipo-catalyst-side-route] |
| PolicyHit | INACTIVE | keep_inactive | [evidence:policy-classify] |
| EarningsBeat | OPT-IN | retain_opt_in | [evidence:news-loop] [producer:earnings-beat] |
| EarningsMiss | OPT-IN | retain_opt_in | [evidence:news-loop] [producer:earnings-miss] |
| AnalystUpgrade | ACTIVE | adapt_or_conform | [evidence:news-loop] [producer:analyst-upgrade] |
| MarketActionAlert | ACTIVE | adapt_or_conform | [evidence:account-push] [producer:account-frozen-side] |
| NewsFlashCritical | INACTIVE | keep_inactive | [evidence:flash-reserve] |
| NewsFlashAggregated | ACTIVE | adapt_or_conform | [evidence:news-loop] [producer:news-flash-aggregate] |

## Durable states (CURRENT); application projection (PROPOSED)

The state names are the exact 14 variants of
`src/durable_delivery/model.rs::DecisionState` (baseline lines 800–814).
Projection below is the new application contract, not a claim that Rust currently
returns these RFC types. terminal means a sealed transport-disposition checkpoint,
not accepted business completion; Uncertain remains unresolved at business level.
Pending audit/task-transition states return Blocked until authority verification
can produce the terminal ref. This deliberately cannot infer authority from a
state label or Ok/log line. Business intent state is separate, not a copy of these
14 states. [Q:7] [Q:75] [Q:78] [evidence:startup-reconcile]
[evidence:startup-list-deliverable] [evidence:startup-begin-attempt]

| state | application_result | terminal | automatic_send_retry | business_finalizer | refs |
| --- | --- | --- | --- | --- | --- |
| Reserved | Blocked | no | lease_fenced_first_attempt | no | [Q:90] |
| AttemptInFlight | Blocked | no | never_until_reconciled | no | [Q:9] |
| AcceptedAuditPending | Blocked | no | never | after_authority_sealed | [Q:78] |
| AcceptedTaskTransitionPending | Blocked | no | never | after_authority_sealed | [Q:78] |
| Delivered | TransportAccepted/AlreadyTerminal | yes | never | accepted_binding_only | [Q:2] [Q:87] |
| RejectedAuditPending | Blocked | no | never_until_reconciled | no | [Q:78] |
| RejectedTaskTransitionPending | Blocked | no | never_until_reconciled | no | [Q:78] |
| RejectedDurable | TransportRejected/AlreadyTerminal | yes | explicit_authorization_only | rejection_proposal_no_cursor | [Q:85] |
| UncertainAuditPending | Blocked | no | never | no | [Q:9] |
| UncertainTaskTransitionPending | Blocked | no | never | no | [Q:9] |
| UncertainManualReview | TransportUncertain/AlreadyTerminal | yes | never | quarantine_no_cursor | [Q:9] [Q:87] |
| ManualRejectedAuditPending | Blocked | no | never | no | [Q:46] |
| ManualRejectedTaskTransitionPending | Blocked | no | never | no | [Q:46] |
| ManualResolvedRejected | AlreadyTerminal | yes | never | manual_not_delivered_no_cursor | [Q:46] [Q:87] |

## Type: ReasonCode (PROPOSED)

Creator: the typed boundary detecting the condition. Consumer: project,
coordinator, finalizer, operator/alert adapters and tests. Canonical material is
the exact ASCII code, not explanatory text. The closed minimum registry below
uses stable lowercase namespace.suffix; additions need an approved cited reason
and schema review. Explanation edits cannot change control flow. Retry below is
eligibility only: fences and explicit authority constraints still apply.
[Q:9] [Q:85] [Q:101] [unit:MU-p01] [producer:p01-scheduled]

| code | condition | handling | refs |
| --- | --- | --- | --- |
| schedule.not_trading_day | Calendar authority says non-trading business day | NoData/Disabled by policy, never send | [Q:101] [Q:28] |
| schedule.window_not_open | Captured business time precedes valid window | Keep occurrence pending until eligible | [Q:101] [Q:28] |
| schedule.window_expired | Captured business time exceeds catch-up window | Policy schedule proposal, no implied delivery | [Q:101] [Q:28] |
| schedule.occurrence_closed | Exact schedule occurrence already closed | No new dispatch; notification status remains separate | [Q:101] [Q:86] |
| input.source_unavailable | Provider read failed | BlockedOnInput; bounded pre-send retry | [Q:101] [Q:12] |
| input.source_unready | Required producer capability not ready | BlockedOnInput and producer isolation | [Q:101] [Q:12] |
| input.evidence_invalid | Source ref/hash/time binding invalid | BlockedOnInput; do not synthesize facts | [Q:101] [Q:59] |
| input.no_verified_batch | No admitted same-tick batch | BlockedOnInput; no cross-batch NewsAI facts | [Q:101] [Q:33] |
| input.account_snapshot_missing | Required account snapshot absent | BlockedOnInput; no substitute portfolio | [Q:101] [Q:12] |
| input.namespace_violation | Source or intent namespace differs | PermanentFailure; no cross-namespace use | [Q:101] [Q:49] |
| policy.disabled | Explicit disabled producer or activation | Disabled; preserve registered policy | [Q:101] [Q:21] |
| policy.starved | Catalog source remains STARVED | BlockedOnInput; no product activation | [Q:101] [Q:30] |
| policy.opt_in_disabled | Required opt-in not granted | Disabled; no product activation | [Q:101] [Q:30] |
| policy.cooldown_active | Captured cooldown excludes this occurrence | Suppressed until eligible; not delivered | [Q:101] [Q:19] |
| policy.daily_budget_full | Shared applicable budget exhausted | Suppressed; quota is not completion owner | [Q:101] [Q:16] |
| policy.suppressed | Explicit semantic suppression rule | Suppressed proposal; no cursor advance | [Q:101] [Q:19] |
| intent.payload_conflict | Same identity has different immutable payload/evidence hash | ResolutionRequired; no overwrite or resend | [Q:101] [Q:89] |
| intent.expected_version_conflict | Business CAS version mismatch | ResolutionRequired; block promotion | [Q:101] [Q:77] |
| intent.lease_held | Unexpired foreign owner lease | Blocked; no competing attempt | [Q:101] [Q:90] |
| intent.transition_conflict | Intent/journal transition inconsistent | ResolutionRequired; no fabricated completion | [Q:101] [Q:97] |
| transport.rejected | Verified authority rejects attempt | TransportRejected; retry only if explicitly authorized | [Q:101] [Q:85] |
| transport.uncertain | Verified authority records unknown outcome | TransportUncertain; quarantine, no blind retry | [Q:101] [Q:9] |
| transport.no_channel_configured | No compatibility channel configured | NoChannelConfigured; no attempt/receipt | [Q:101] [Q:7] |
| transport.all_channels_failed | No weak channel accepted | AllChannelsFailed; preserve Unknown and no authority | [Q:101] [Q:7] |
| transport.partially_accepted | Only a subset weakly accepted | PartiallyAccepted; no authority | [Q:101] [Q:7] |
| finalizer.terminal_ref_invalid | Authority re-query cannot verify reference | Blocked; no business mutation | [Q:101] [Q:78] |
| finalizer.binding_mismatch | Verified ref does not bind requested intent | Blocked; no completion advance | [Q:101] [Q:87] |
| finalizer.cas_conflict | Completion owner version changed | ResolutionRequired; no overwrite | [Q:101] [Q:77] |
| finalizer.deadline_exceeded | Accepted-to-finalized exceeds two cycles or five-minute hard bound | Expose lag and block promotion at hard deadline | [Q:101] [Q:38] |
| finalizer.transition_append_failed | Business transition record cannot be appended | Fail finalization atomically; no completion claim | [Q:101] [Q:97] |
| activation.manifest_mismatch | Build/catalog/schema/template/source binding differs | CoreUnready or Unit blocked; no promotion | [Q:101] [Q:79] |
| activation.generation_conflict | Generation CAS fails | Blocked; no owner change | [Q:101] [Q:80] |
| activation.owner_conflict | Competing physical owners for one occurrence | Blocked; no second physical send | [Q:101] [Q:13] |
| activation.core_unready | Shared authority or store unavailable | CoreUnready; block production readiness | [Q:101] [Q:12] |
| activation.producer_unready | A registered producer dependency is unavailable | Isolate producer and fail deployment readiness | [Q:101] [Q:12] |
| shadow.semantic_diff | Typed decision/hash/reason/proposal differs | Block promotion; retain comparison evidence | [Q:101] [Q:83] |
| shadow.side_effect_attempted | Shadow tries provider reread/LLM/write/send/order | Reject action; block promotion | [Q:101] [Q:83] |
| operator.unauthorized | Authenticated principal lacks approved permission | Reject request; audit denial | [Q:101] [Q:47] |
| operator.evidence_invalid | Manual evidence missing/invalid/excessive disclosure | Reject manual resolution | [Q:101] [Q:55] |
| operator.resolution_conflict | Manual expected version or binding conflicts | ResolutionRequired; no blind override | [Q:101] [Q:87] |

## Adapter conformance (PROPOSED)

P01 scheduled, compensation and startup restoration share the registered business
occurrence/owner; different callers are not separate notifications. N02 keeps its
dedicated accepted-window/reservation/attempt settlement authority. Both must
implement the same preparation, identity, result and finalizer boundary as the
generic coordinator; no duplicated receipts or third completion truth.
[Q:16] [Q:27] [Q:78] [unit:MU-p01] [producer:p01-scheduled]
[producer:p01-compensation] [producer:startup-resume-preopen-news-hot]
[unit:MU-news-flash-aggregate] [producer:news-flash-aggregate]

Conformance assertions: one immutable PreparedFacts instance (captured model output
included) for active/shadow; project is pure; exact semantic/rendered hashes,
ReasonCode and completion proposals match; attempt ID, latency and diagnostic
timestamp are the only excluded shadow differences. No second provider/LLM/query,
DB write, cursor movement, order or send is allowed in shadow. The active path
alone may persist intent and call the authority adapter. [Q:19] [Q:33] [Q:34]
[Q:40] [Q:72] [Q:83]

Same intent + changed rendered/evidence/source binding enters ResolutionRequired.
AlreadyTerminal and authenticated manual acceptance require exact fresh binding;
neither retry nor compensation can loosen it. Source failure cannot be relabeled
NoData; local persistence, sink attempt, Ok/log text and compatibility success
cannot become TransportAccepted. P01/N02 adapter conformance does not assert
that these new RFC types or a common finalizer already exist in Rust.
[Q:7] [Q:27] [Q:46] [Q:78] [Q:85] [Q:87] [Q:89]

## Task2 verification boundary

`check-rfc.rb --root ROOT --draft` enforces the same content/hash/reference
checks as `--check`. Strict currently additionally rejects PROVISIONAL; the full
publication/HTML/CI gate is a separate Task5/6 deliverable. RFC validation is not
deployment verification, runtime migration completion, a receipt, or a WBS estimate.
This checker validates frozen catalog/evidence bytes and reference membership;
actual Rust symbol-byte freshness is independently checked by
`check-catalog.rb --root ROOT --draft` and the source gate. Neither gate substitutes
for the other or re-queries production state.
[Q:5] [Q:42] [Q:69] [Q:92] [Q:105]
