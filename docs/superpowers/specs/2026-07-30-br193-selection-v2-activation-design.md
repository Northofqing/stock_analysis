# BR-193 Selection-v2 Generation Activation — Gate A Design

**Status:** Corrective Gate A draft after C0/I6/M0 independent RED; fresh re-review required
**Date:** 2026-07-30
**Scope:** one production-capable `source ingress -> relation -> market evidence ->
terminal generation` vertical slice
**Explicitly out of scope:** outcome acquisition/settlement, notification sinks,
orders, paper trading, legacy selection restoration

This document is not a release approval. It defines the smallest auditable
capability that can move selection-v2 from the BR-183 fail-closed state to
generation-only operation. Gate B, Gate C, Gate D, independent review and a PR
remain blocking.

### 0.1 Normative authority

This document is the complete normative contract for BR-193. It does not
inherit acceptance criteria, limits, identities, retry semantics, migration
semantics or release claims from BR-174 or BR-178; both earlier batches remain
unreleased and are historical implementation context only. Every contract
needed by this slice is repeated and frozen below. Existing code types may be
adopted only when their current HEAD behavior satisfies this document and the
machine-checkable acceptance criteria in section 10.

No unreleased BR is a normative prerequisite. BR-193 does not inherit a lease,
bootstrap or isolation promise from any earlier bootstrap specification.
Section 5.1 freezes the complete lease-before-pool contract needed here and
its acceptance tests must pass against current HEAD before this slice can
activate.

This corrective document replaces, rather than layers on, every earlier
unaccepted BR-193 Gate A/B draft. In particular, an implementation copied from
an earlier draft has no authority when it conflicts with the durable
acquisition, fair paging, migration recovery, calendar or Selected-proof
contracts below. Fresh independent review must evaluate these exact bytes.

## 1. Decision

Implement a capability with exactly two independently visible states:

```text
SelectionV2Capability
├── Disabled(SelectionDisabledReason)
└── GenerationActive(opaque owner)
    └── OutcomeDisabled(outcome_activation_not_released)
```

`GenerationActive` means only:

1. current config, board proposal/artifact, immutable official trading
   calendar and activation are exact and receipted;
2. the fixed production schema has passed the offline migration and exact
   catalog verification;
3. receipted global-news source facts can be recovered, queried and evaluated;
4. direct mentions and activated industry-chain relations produce canonical
   candidates;
5. Magic TDX can attach real market evidence to those candidates;
6. every evaluated candidate becomes a receipted terminal
   `TerminalDecisionKind::Admitted` or
   `TerminalDecisionKind::HardRejected`.

The fixed production scheduling constants are:

```text
NEWS_FETCH_PERIOD_SECS=120
NEWS_PER_FEED_LIMIT=20
PENDING_GENERATION_LIMIT=200
SELECTED_GENERATION_PAGE_LIMIT=200
```

They are compile-time business constants for this slice, have no environment,
CLI or caller override, and are registered by BR-193.
The release-only Gate-D verifier additionally freezes
`GATE_D_OFFICIAL_HTTP_TIMEOUT_SECS=30`; it is not a runtime provider timeout
and likewise has no environment, CLI or caller override.

It does **not** mean that outcomes, pushes, orders or virtual/real trading are
active.

The business label **Selected** maps to the existing persisted token
`TerminalDecisionKind::Admitted`. The Rust enum token, database token, logical
identity and hashes remain `admitted`; this slice must not rename or rewrite
persisted history. `HardRejected` remains the terminal rejected identity.

## 2. Rules and invariants

Triggered repository rules:

- **2.1:** production consumes only real global-news, BoardDataGateway and
  Magic TDX evidence; no mock or local fabricated candidate;
- **2.2:** missing source time, relation, quote, bars or evidence remains
  absent or becomes a typed failure; no zero/default filling;
- **2.3:** price, continuity, duplicate, corporate-action and adjacent
  valid-value checks remain enforced, including the repository's required
  `> +/-20%` manual-confirmation path;
- **2.4:** each datum is checked against its own freshness contract before
  feature calculation;
- **2.5:** production and TEST_CODE symbols, databases, audit roots, locks,
  calendar artifacts and providers are physically separated before any
  selection owner is constructed;
- **2.7:** config activation, ingress and generation retain source, provider,
  time, batch, hashes, decision basis and immutable audit closure;
- **2.8:** activation, save, verify and recovery APIs must perform the named
  operation;
- **2.9:** this design changes no runtime TOML threshold and authorizes no
  threshold override;
- **2.10 / BR-193:** pending filters, stable sort, limit, terminal
  deduplication and cross-process ownership are registered before code.

No `config/*.toml` threshold changes are authorized by this design. Rule 2.9
therefore has no TOML/config bidirectional edit in this slice. The four
compile-time business limits, including the 120-second period and per-feed 20,
still require and receive the explicit Threshold-Proof in section 12.

The following are hard invariants:

- Missing release prerequisites produce one typed Disabled state before any
  selection provider or selection scheduler is constructed.
- Corruption, conflicting receipts, catalog drift, audit-chain failure,
  identity replacement after pinning or mixed partial migration are fatal
  integrity errors. They must not be mislabeled Disabled.
- A verified empty provider response is distinct from provider unavailable.
- Ordinary runtime/generation source/provider work occurs outside SQLite and
  selection-audit critical sections. Durable commit occurs under the existing
  lock order. The sole exception is the fixed-root, offline Gate-D official
  calendar revalidation in section 9: it runs under the exclusive global
  maintenance lease and the one retained audit session, performs no selection
  provider/sink/order construction and may only append the terminal Gate-D
  audit record after every bounded official read succeeds.
- Every external read has a receipted pre-I/O acquisition intent and a
  separate immutable post-response evidence seal. An intent is never inferred
  from a response and a response is never inferred from an intent.
- A generation terminal decision is immutable. A bad stock is not merely
  logged: it is persisted as `HardRejected` with exact rejection evidence and
  excluded from the selected view.
- Retryable acquisition/dependency failure is not `HardRejected`; it is a
  receipted `pending_dependency` generation attempt and remains eligible only
  inside the stored prospective generation date.
- No downstream consumer may infer Selected from an unreceipted sample row.
- `chain_id` and `matched_keyword` always come from a real, exact activated
  chain-keyword match in the receipted source fact. A security mention alone
  cannot invent either field.
- A generation attempt may use market values only after exact security
  lifecycle, corporate-action and BR-171 manual-change-confirmation evidence
  has been joined and persisted.
- A bounded page is a scheduling boundary, not a priority privilege: a
  retryable/busy/invalid fact in the first page cannot prevent later eligible
  facts from eventually receiving an issue slot.

## 3. Current evidence and exact gaps

### 3.1 Process bootstrap is hard-disabled

Reproducible repository search:

```text
$ rg -n -U -C 4 "bootstrap_selection_process\s*\(|selection_v2_disabled_reason_code\s*\(" src/bin/monitor src/selection --glob "*.rs"
src/bin/monitor/main.rs:3136:    let selection_cli = match stock_analysis::selection::bootstrap_selection_process() {
--
src/bin/monitor/main.rs:3158:    let selection_v2_enabled = match selection_cli.selection_v2_disabled_reason_code() {
--
src/selection/process_bootstrap.rs:104:    pub fn selection_v2_disabled_reason_code(&self) -> Option<&'static str> {
--
src/selection/process_bootstrap.rs:193:pub fn bootstrap_selection_process(
```

`src/selection/process_bootstrap.rs::classify_parsed_invocation` currently
constructs `SelectionCapabilityState::Disabled` for every operational
invocation. That enum has no active variant. Monitor only converts the result
to a Boolean and logs BR-183 disabled state.

### 3.2 Production migration is intentionally unavailable

```text
$ rg -n "SELECTION_V2_APPLY_BLOCKER|apply && !test_rehearsal|SelectionCapabilityState" src/database src/selection
src/selection/process_bootstrap.rs:316:enum SelectionCapabilityState {
src/database/global_schema_v1.rs:392:    if apply && !test_rehearsal {
src/database/global_schema_v1.rs:393:        return Err(GlobalSchemaError::SelectionV2Migration(
src/database/selection_v2.rs:208:pub const SELECTION_V2_APPLY_BLOCKER: &str =
```

The blocker correctly states that final schema/parser/receipt ownership,
maintenance locks, full-file backup, fsync and atomic exchange are not all
implemented. BR-193 must implement and verify them; it must not delete or
bypass the blocker first.

### 3.3 Generation has no executable owner

```text
$ rg -n "PreparedGeneration|GenerationOwner|pending.*source|source.*pending|load.*pending" src/selection src/database/selection_v2_read_model.rs src/database/selection_v2_repository.rs
src/selection/persistence_v2.rs:113:    /// entry point until an independent opaque `PreparedGeneration` owner
src/selection/schema_v2.rs:9955:    let mut pending = valid_empty_source_ingress_stage();
```

The existing persistence owner can publicly commit opaque
`PreparedSourceIngress`, while generation still exposes only a crate-private
raw `GenerationStageRequest` path. `VerifiedSelectionReadModel` has recovery
and outcome queries but no receipted pending-source-fact capability.

### 3.4 Market acquisition derives candidates from raw text

```text
$ rg -n -U -C 4 "fetch_selection_market_batch\s*\(|SelectionMarketRequest\s*\{" src --glob "*.rs"
src/data_gateway/magic_tdx_selection.rs:83:pub struct SelectionMarketRequest {
--
src/data_gateway/magic_tdx_selection.rs:179:pub async fn fetch_selection_market_batch(
--
src/bin/selection_live_probe.rs:58:    let batch = fetch_selection_market_batch(SelectionMarketRequest {
```

Inside the gateway, each `SelectionEventReference.text` is passed to
`direct_mentions`; only mentioned securities are fetched. There is no monitor
producer call and no activated industry-chain candidate input. The gateway is
therefore a probe/enrichment draft, not the generation boundary.

### 3.5 Checked-in activation inputs are incomplete

```text
$ find config/selection -maxdepth 1 -type f -print | sort
config/selection/provider_board_bindings.v1.json
```

The checked-in artifact currently has `state=direct_only_unverified` and an
empty binding list. The fixed proposal and activation files required by the
existing activation design are absent. This state must continue to produce a
typed Disabled result.

### 3.6 Raw current-HEAD evidence for the reviewed seams

These commands were run from the repository root. Output is pasted verbatim;
the absent `selected_generation_page` hit is itself the current gap.

```text
$ rg -n 'TRADING_HOLIDAYS|pub fn add_holidays|pub fn is_trading_day' src/calendar.rs
8://! 节假日列表从环境变量 `TRADING_HOLIDAYS` 读取（逗号分隔的 YYYYMMDD），
102:    if let Ok(raw) = std::env::var("TRADING_HOLIDAYS") {
117:pub fn add_holidays(dates: &[NaiveDate]) {
129:pub fn is_trading_day(date: NaiveDate) -> bool {

$ rg -n 'pub enum SelectionAuditPhase|pub struct SelectionAuditRecord|serde\(deny_unknown_fields\)|from_slice::<SelectionAuditRecord>' src/selection/audit.rs
93:pub enum SelectionAuditPhase {
117:#[serde(deny_unknown_fields)]
132:#[serde(deny_unknown_fields)]
133:pub struct SelectionAuditRecord {
1599:        let record = serde_json::from_slice::<SelectionAuditRecord>(line).map_err(|error| {
1907:        let parsed = serde_json::from_slice::<SelectionAuditRecord>(
2227:        let persisted = serde_json::from_slice::<SelectionAuditRecord>(&bytes[..bytes.len() - 1])

$ rg -n 'SELECTION_V2_APPLY_BLOCKER|if apply && !test_rehearsal' src/database/selection_v2.rs src/database/global_schema_v1.rs
src/database/global_schema_v1.rs:392:    if apply && !test_rehearsal {
src/database/global_schema_v1.rs:393:        return Err(super::selection_v2::SELECTION_V2_APPLY_BLOCKER.to_owned());
src/database/global_schema_v1.rs:3783:            super::super::selection_v2::SELECTION_V2_APPLY_BLOCKER
src/database/selection_v2.rs:208:pub const SELECTION_V2_APPLY_BLOCKER: &str =
src/database/selection_v2.rs:610:        apply_blocker: SELECTION_V2_APPLY_BLOCKER,
src/database/selection_v2.rs:6335:            assert_eq!(preflight.apply_blocker, SELECTION_V2_APPLY_BLOCKER);
src/database/selection_v2.rs:6679:        assert_eq!(preflight.apply_blocker, SELECTION_V2_APPLY_BLOCKER);

$ rg -n 'pub struct StoredOutcomeSchedule|pub fn derive_outcome_schedule|const MAX_TICK_LIMIT|selected_generation_page' src/selection/outcome_v2.rs src/database/selection_v2_read_model.rs
src/selection/outcome_v2.rs:142:pub struct StoredOutcomeSchedule {
src/selection/outcome_v2.rs:198:pub fn derive_outcome_schedule(
src/database/selection_v2_read_model.rs:56:const MAX_TICK_LIMIT: i64 = 200;

$ rg -n 'pub const FEATURE_VERSION|pub fn compute_daily_features|pub const ADMISSION_VERSION|pub fn evaluate_admission|pub fn validate_daily\(' src/selection/features.rs src/selection/admission.rs src/selection/quality.rs
src/selection/admission.rs:6:pub const ADMISSION_VERSION: &str = "admission-v1";
src/selection/admission.rs:53:pub fn evaluate_admission(
src/selection/quality.rs:142:pub fn validate_daily(bars: &[SelectionBar]) -> Result<ValidatedDailyBars<'_>, QualityError> {
src/selection/features.rs:4:pub const FEATURE_VERSION: &str = "raw-selection-v1";
src/selection/features.rs:86:pub fn compute_daily_features(bars: &[SelectionBar]) -> Result<RawSelectionFeatures, FeatureError> {
```

The C4/I6 review also requires multiline source context rather than
single-line symbol hits. These commands were run against the same current
HEAD; output is pasted exactly:

```text
$ rg -n -U -C 6 --max-count 1 'pub\(crate\) fn commit_generation\s*\(|pub struct GenerationStageRequest\s*\{' src/selection/persistence_v2.rs src/database/selection_v2_repository.rs
src/selection/persistence_v2.rs-113-    /// entry point until an independent opaque `PreparedGeneration` owner
src/selection/persistence_v2.rs-114-    /// capability exists.
src/selection/persistence_v2.rs-115-    #[allow(
src/selection/persistence_v2.rs-116-        dead_code,
src/selection/persistence_v2.rs-117-        reason = "BR-183 keeps selection-v2 generation persistence disabled until release evidence closes"
src/selection/persistence_v2.rs-118-    )]
src/selection/persistence_v2.rs:119:    pub(crate) fn commit_generation(
src/selection/persistence_v2.rs-120-        request: GenerationStageRequest,
src/selection/persistence_v2.rs-121-    ) -> SelectionV2PersistenceResult<CommitReceipt> {
src/selection/persistence_v2.rs-122-        commit_production(DurableStageRequest::Generation(Box::new(request)))
src/selection/persistence_v2.rs-123-    }
src/selection/persistence_v2.rs-124-
src/selection/persistence_v2.rs-125-    pub(crate) fn commit_outcome_claim(
--
src/database/selection_v2_repository.rs-201-    pub(crate) fn stage_run_id(&self) -> &str {
src/database/selection_v2_repository.rs-202-        &self.stage_input.stage_run_id
src/database/selection_v2_repository.rs-203-    }
src/database/selection_v2_repository.rs-204-}
src/database/selection_v2_repository.rs-205-
src/database/selection_v2_repository.rs-206-#[derive(Debug, Clone, PartialEq, Eq)]
src/database/selection_v2_repository.rs:207:pub struct GenerationStageRequest {
src/database/selection_v2_repository.rs-208-    stage_input: GenerationStageInputPreimage,
src/database/selection_v2_repository.rs-209-    run_payload: RunPayloadPreimage,
src/database/selection_v2_repository.rs-210-    recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
src/database/selection_v2_repository.rs-211-}
src/database/selection_v2_repository.rs-212-
src/database/selection_v2_repository.rs-213-impl GenerationStageRequest {

$ rg -n -U -C 6 'pub fn has_exact_daily_change_confirmation\s*\(|pub\(crate\) fn has_exact_daily_change_confirmation_on_conn\s*\(' src/database/daily_change_confirmation.rs
755-) -> DailyChangeConfirmationResult<DailyChangeConfirmationReceipt> {
756-    conn.immediate_transaction::<_, DailyChangeConfirmationError, _>(|conn| {
757-        insert_confirmation_in_transaction(conn, input)
758-    })
759-}
760-
761:pub(crate) fn has_exact_daily_change_confirmation_on_conn(
762-    conn: &mut SqliteConnection,
763-    query: &DailyChangeConfirmationQuery,
764-) -> DailyChangeConfirmationResult<bool> {
765-    validate_daily_change_confirmation_chain(conn)?;
766-    let canonical = canonical_query(query)?;
767-    let identity_hash = query_identity_hash(&canonical)?;
--
787-            .get_conn()
788-            .map_err(|error| format!("BR-171 confirmation DB connection: {error}"))?;
789-        append_daily_change_confirmation_on_conn(&mut conn, input)
790-            .map_err(|error| error.to_string())
791-    }
792-
793:    pub fn has_exact_daily_change_confirmation(
794-        &self,
795-        query: &DailyChangeConfirmationQuery,
796-    ) -> Result<bool, String> {
797-        let mut conn = self
798-            .get_conn()
799-            .map_err(|error| format!("BR-171 confirmation DB connection: {error}"))?;

$ rg -n -U -C 6 'pub async fn fetch_selection_market_batch\s*\(|fetch_selection_market_batch\s*\(\s*SelectionMarketRequest' src/data_gateway/magic_tdx_selection.rs src/bin/selection_live_probe.rs
src/data_gateway/magic_tdx_selection.rs-173-{
src/data_gateway/magic_tdx_selection.rs-174-    tokio::task::spawn_blocking(operation)
src/data_gateway/magic_tdx_selection.rs-175-        .await
src/data_gateway/magic_tdx_selection.rs-176-        .map_err(SelectionSourceError::join)?
src/data_gateway/magic_tdx_selection.rs-177-}
src/data_gateway/magic_tdx_selection.rs-178-
src/data_gateway/magic_tdx_selection.rs:179:pub async fn fetch_selection_market_batch(
src/data_gateway/magic_tdx_selection.rs-180-    request: SelectionMarketRequest,
src/data_gateway/magic_tdx_selection.rs-181-) -> Result<SelectionMarketBatch, SelectionSourceError> {
src/data_gateway/magic_tdx_selection.rs-182-    validate_request(&request)?;
src/data_gateway/magic_tdx_selection.rs-183-    run_magic_tdx_blocking(move || fetch_selection_market_batch_blocking(request)).await
src/data_gateway/magic_tdx_selection.rs-184-}
src/data_gateway/magic_tdx_selection.rs-185-
--
src/bin/selection_live_probe.rs-52-        .enumerate()
src/bin/selection_live_probe.rs-53-        .map(|(index, code)| SelectionEventReference {
src/bin/selection_live_probe.rs-54-            event_id: format!("selection_live_probe_{index:04}"),
src/bin/selection_live_probe.rs-55-            text: code.clone(),
src/bin/selection_live_probe.rs-56-        })
src/bin/selection_live_probe.rs-57-        .collect();
src/bin/selection_live_probe.rs:58:    let batch = fetch_selection_market_batch(SelectionMarketRequest {
src/bin/selection_live_probe.rs-59-        event_references,
src/bin/selection_live_probe.rs-60-        window: market_window(),
src/bin/selection_live_probe.rs-61-        evaluation_at,
src/bin/selection_live_probe.rs-62-        expected_latest_settled_date,
src/bin/selection_live_probe.rs-63-    })
src/bin/selection_live_probe.rs-64-    .await?;

$ rg -n -U -C 6 'pub fn add_holidays\s*\(|pub fn is_trading_day\s*\(' src/calendar.rs
111-    }
112-    RwLock::new(set)
113-});
114-
115-/// 添加节假日（运行时注入，用于测试或动态更新）
116-/// review #14: poison 时 log error 而非静默丢弃, 让调用方知道 add 失败.
117:pub fn add_holidays(dates: &[NaiveDate]) {
118-    match HOLIDAYS.write() {
119-        Ok(mut guard) => {
120-            for d in dates {
121-                guard.insert(*d);
122-            }
123-        }
124-        Err(e) => log::error!("[calendar] HOLIDAYS RwLock poisoned, add 失败: {}", e),
125-    }
126-}
127-
128-/// 判断指定日期是否为交易日
129:pub fn is_trading_day(date: NaiveDate) -> bool {
130-    // 周末
131-    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
132-        return false;
133-    }
134-    // 节假日
135-    // review #14 修复: RwLock poison 时 .read() 返回 Err, 原 `if let Ok(guard)` 静默
```

The Boolean BR-171 lookup and raw generation request above are current gaps,
not authorities to preserve. Sections 5.4.3 and 5.5 replace them with a closed
receipt and durable intent/seal contract.

## 4. Adopted and rejected modules

| Existing module | Decision | Reason |
| --- | --- | --- |
| `selection::process_bootstrap` | adopt and deepen | preserves sole zero-argument real-argv authority and mode isolation |
| `database::global_schema_v1` | adopt and complete | preserves exact global catalog classification and offline command boundary |
| `database::selection_v2_repository` | adopt | remains the only stage transaction/receipt choreography |
| `selection::persistence_v2` | adopt and deepen | remains the sole production resource owner; receives opaque generation input |
| `selection::config_activation_v2` | adopt | canonical config/activation preparation already exists but is dormant |
| `selection::ingress_v2` | adopt | already creates opaque source-ingress capability |
| `database::selection_v2_read_model` | adopt and deepen | verified snapshot becomes sole pending-fact issuer |
| `data_gateway::board` | adopt | sole strict proposal/artifact loader and BoardDataGateway binding authority |
| `data_gateway::magic_tdx_selection` | adopt as market enrichment only | remove raw-news candidate derivation from its request |
| `data_gateway::security_lifecycle` | adopt as mandatory evidence owner | exact listing/corporate-action request and verified-empty/available evidence for rule 2.3 |
| `database::daily_change_confirmation` | adopt as mandatory receipt owner | exact BR-171 confirmation lookup; absence remains pending dependency |
| `selection::quality` | deepen; reject as direct production authority | current raw-bar API does not own lifecycle/action or >20% confirmation evidence; only its pure price/continuity checks may run behind the opaque confirmed-series owner |
| `selection::features` | deepen; reject raw `&[SelectionBar]` production entry | keep formulas/version, but production must consume an opaque complete-and-confirmed market capability so missing data cannot become a feature |
| `selection::admission` | deepen; reject optional-feature production entry | keep the frozen admission-v1 predicates; production receives non-optional finite features and may emit only the closed hard-rejection taxonomy, never a missing-data rejection |
| `calendar` mutable process helpers | reject as selection schedule authority | checked-in holidays plus environment/runtime injection have no immutable provider/version/hash and cannot own T0..D5 |
| `selection::outcome_v2::derive_outcome_schedule` | adopt algorithm only | exact ascending-vector validation is reusable only behind the receipted immutable calendar owner in section 5.4.2 |
| `selection::schema_v2` | adopt unchanged identity | terminal `Admitted`/`HardRejected`, hashes and manifests remain authoritative |
| legacy opportunity/news-ranker/sector-history candidate chains | reject | unreceipted, incomplete or retired authorities cannot seed v2 |
| chain-intelligence A-10 visible batch | reject as selection authority | a different business projection; it may not be reinterpreted as receipted news-to-stock lineage |
| outcome owner/scheduler | retain disabled | outside minimum generation slice |
| push/sink/order/paper modules | reject from dependency graph | expressly outside scope |

BR-183 remains the fallback behavior when this slice is not provably active.
No unverified earlier selection design is a prerequisite or authority for
BR-193.

BR-193 necessarily supersedes one BR-183 implementation detail: an operational
invocation may perform the bounded, fixed-root, read-only schema/activation
binding described below before deciding Active versus Disabled. Otherwise an
activation receipt could never be verified. Help, version, invalid argv and a
service-disabled invocation remain completely storage-free. A Disabled result
still permits no selection provider, write, scheduler, sink or subsequent
selection read. This is not permission for a caller-selected database probe.

## 5. Target interfaces

Every item in this section is **TO BE BUILT** unless marked EXISTING. Every
type, variant, serialized token, logical field, test, static counter, fixed
path, provider and helper identifier explicitly named by this document is
immutable through Gate B. Gate B may refine only private local implementation
identifiers that do not appear in this document; it may not rename or alias a
named contract identifier.

### 5.1 Capability binding

```rust
// TO BE BUILT; fields private, not Clone/Serialize/Default.
pub enum SelectionRuntimeCapability {
    Disabled(SelectionDisabledReason),
    GenerationActive(GenerationRuntimeOwner),
}

// TO BE BUILT; stable tokens only, no caller string.
pub enum SelectionDisabledReason {
    SchemaNotAmended,
    ProposalMissing,
    BoardArtifactUnverified,
    BoardArtifactExpired,
    ActivationMissing,
    ActivationNotEffective,
    ActivationExpired,
    ActivationUnreceipted,
    ActivationRevoked,
    TradingCalendarMissing,
    TradingCalendarUnverified,
    TradingCalendarCoverageIncomplete,
    IngressContractUnavailable,
}

// TO BE BUILT; always nested under GenerationActive for this release.
pub enum OutcomeCapability {
    Disabled(OutcomeDisabledReason),
}

// TO BE BUILT; closed stable token, no caller string.
pub enum OutcomeDisabledReason {
    OutcomeActivationNotReleased,
}
```

`OutcomeDisabledReason::OutcomeActivationNotReleased` serializes exactly as
`outcome_activation_not_released`. It is the only permitted outcome-disabled
variant for this slice; unknown, missing, differently cased or caller-supplied
tokens fail strict decoding.

The existing zero-argument `bootstrap_selection_process()` parses real argv
exactly once and returns its opaque mode proof. It does not claim that
storage-backed activation is known during storage-free parsing.

The following two-phase lease-before-owner-before-pool rule is self-contained
and mandatory for this slice:

1. the process-proof owner resolves the fixed Production or invocation-unique
   TEST_CODE namespace into a private, non-Clone
   `SelectionNamespaceBootstrap`; this bootstrap can pin only the namespace
   root and the one fixed maintenance-lock leaf and cannot mint a generic
   resource capability;
2. `SelectionNamespaceBootstrap::acquire_shared_maintenance_lease(self)`
   consumes the bootstrap and acquires the mode-bound, process-lifetime shared
   `GlobalSchemaMaintenanceLease` **before** constructing a Diesel/r2d2 pool,
   `SelectionNamespaceOwner`, `DatabaseManager`, catalog reader, audit writer,
   provider or sink; it returns one private
   `AcquiredSelectionNamespace { root_descriptor, mode_proof,
   maintenance_lock_identity, lease }`;
3. `SelectionNamespaceOwner::install(acquired)` consumes that whole value,
   moves the already-held lease and exact lock identity into the single
   private owner, and only then permits the one closed resource-capability
   split described below;
4. while holding that lease, a no-pool descriptor-pinned connection verifies
   `application_id=1398035265`, `user_version=1`, the complete exact global
   catalog and selection audit/database identity;
5. only an exact Amended catalog may construct `DatabaseManager` and a
   `VerifiedSelectionReadModel`; a recognized recoverable partial is routed to
   the offline owner and ordinary startup exits fatal without a pool;
6. the owner reconstructs the exact checked-in proposal/artifact/activation,
   official A-share calendar artifact, and joins exactly one current
   activation receipt by computed config hash; it verifies
   proposal/artifact validity window, calendar provider/version/hash and
   coverage, executable revision, mode and receipt closure;
7. it returns either the opaque active owner or a typed disabled value.

There is no “core database initialized first” exception. Any code path that
opens a pool or catalog before the shared lease is a Gate B failure.

Caller-selected paths, mode enums, database handles, config hashes, receipt
hashes, clocks, provider implementations or environment overrides are not
parameters.

#### 5.1.1 Physical TEST_CODE/production isolation

The real-argv proof selects exactly one namespace before a descriptor, lock,
pool, audit writer, calendar reader or provider can exist:

- production uses only the manifest-root production database, production
  selection-audit root, production subject-lock root, checked-in production
  activation/calendar artifacts and real provider constructors;
- test/rehearsal creates one invocation-unique `TEST_CODE_<uuidv7>` root under
  the test root owner. Database, audit, maintenance lock, subject locks,
  activation/calendar fixtures and provider spies are children of that
  retained descriptor. They may not alias or fall back to a production leaf;
- production rejects any source fact, chain, board member, security or
  candidate whose canonical identity starts with `TEST_CODE` before a provider
  call or write;
- test/rehearsal rejects any non-`TEST_CODE` source fact, chain, board member,
  security or candidate before a provider call or write;
- test mode constructs zero real network provider and zero real sink. A
  verified-empty test fixture is test evidence only and can never activate or
  populate a production read model.

The private, non-Clone `SelectionNamespaceOwner` retains the namespace root
descriptor, mode proof, exact maintenance-lock identity and already-held
process-lifetime shared lease. The owner is never moved into the first
resource and then recreated. After installation, one call to
`split_resource_capabilities(&mut self)` returns one private, non-Clone
`NamespaceResourceCapabilities` bundle containing exactly one linear
`NamespaceResourceCapability<K>` for each closed resource kind:
`database`, `selection_audit`, `maintenance_lock`, `subject_lock`,
`calendar` and `provider`. The mutable borrow prevents a second split.
Each child borrows the retained owner lifetime and binds its exact
descriptor-relative child identity.

The `maintenance_lock` child is deliberately **not** a constructor authority.
It is consumed only by
`bind_retained_maintenance_lease(existing_lease, maintenance_child)`, which
proves the already-held lease's pinned lock identity equals the child identity
and returns a non-Clone borrow proof for registered lock-order checks. It
cannot open, acquire, release, replace or reacquire a lock. Every other child
may be consumed at most once by its one matching resource constructor, which
also requires that borrow proof. A child not required by the classified
capability is dropped unconsumed and constructs no resource; in particular a
Disabled result drops the provider child before provider construction. No
`sink` resource capability is minted, split or consumed in this slice, and no
selection sink constructor exists. Thus the lease exists before the owner, the
owner exists before any child, and the maintenance child attests rather than
constructs the lease; no circular consumption or second lock acquisition is
possible.
Duplicate split/mint, wrong-kind consumption, cross-owner use, use after owner
drop or construction from a raw path is fatal
`namespace_identity_conflict` before open/I/O. Read-only verification borrows
the retained owner; it does not mint a second capability. This
bootstrap/acquire/install/linear-child sequence is the only allowed way to
construct the multiple resources needed by one process.

Filesystem identity tests must prove production/test database, audit,
calendar and lock device/inode pairs are distinct and that a symlink,
hard-link, path replacement or shared override is rejected. They also prove
the bootstrap opens the maintenance-lock leaf exactly once before owner
installation, the post-install maintenance child only binds that retained
lease, all six closed resource kinds are split once, no sink capability is
minted, unused children create zero resources, a second split fails before open
and no child outlives its owner. There is no environment fallback after
namespace binding.

A calendar-prerequisite absence explicitly classified by exactly one reviewed
marker below is Disabled. Other prerequisite absences retain their exact typed
classifications below. Evidence that claims to exist but is conflicting,
malformed, replaced, partially committed or unreceipted in a nonrecoverable
shape is fatal.

Calendar classification uses this exact, exhaustive authority-presence
matrix before capability construction:

| Calendar authority state | Classification |
| --- | --- |
| calendar manifest absent, notice manifest absent, raw root absent, proposal/activation/receipt contain no calendar/notice/parser hash or path claim, and exactly one checked-in release-prerequisite marker says the reviewed calendar release is missing | Disabled `TradingCalendarMissing` |
| the same wholly-unclaimed absence plus exactly one checked-in release-prerequisite marker saying reviewed verification is not yet released | Disabled `TradingCalendarUnverified` |
| the same wholly-unclaimed absence plus exactly one checked-in release-prerequisite marker saying the reviewed activation window cannot yet be covered through D5 | Disabled `TradingCalendarCoverageIncomplete` |
| the same wholly-unclaimed absence without exactly one valid reviewed release-prerequisite marker | fatal `calendar_release_integrity_conflict` |
| any of the three fixed authorities exists, or proposal/activation/receipt contains any calendar/notice/parser path/hash/version/coverage claim | claimed authority; the entire fixed set, every referenced leaf, canonical payload, exact coverage and all hashes must verify, otherwise fatal |

The three mutually exclusive release-prerequisite marker variants use the one
fixed compile-time manifest-root path
`config/selection/a_share_trading_calendar_release_prerequisite.v1.json`.
Its closed canonical payload contains only
`domain,schema_version,reason_code,reviewed_at,executable_revision`, and
`reason_code` is exactly `trading_calendar_missing`,
`trading_calendar_unverified` or
`trading_calendar_coverage_incomplete`. The marker cannot coexist with any
authority file or activation claim. Unknown/missing fields, no marker, an
unregistered second marker/path alias or a conflicting marker is fatal
`calendar_release_integrity_conflict`. Once authority is claimed, a partial
set, missing referenced leaf, wrong file type/path identity, insufficient
claimed T0..D5 coverage, raw-byte/hash mismatch or publication conflict is
fatal `calendar_release_integrity_conflict`; parser ambiguity/disagreement,
session conflict or T0..D5 mismatch is fatal
`calendar_parser_or_session_conflict`. Neither fatal token may be translated
to any Disabled reason.

Startup summaries are exact and emitted once:

```text
selection_v2 disabled=<typed_reason>
```

or:

```text
selection_v2 generation=active activation_run_id=<canonical-uuidv7> activation_receipt_hash=<64-lower-hex> outcome=disabled reason_code=outcome_activation_not_released
```

Disabled mode constructs zero selection providers, performs zero selection-v2
database operations after classification, and starts zero selection
schedulers/sinks. Core monitor business remains available per BR-183.

The active owner revalidates both `effective_from <= now < expires_at` and
`artifact_captured_at <= now < artifact_valid_until` at scheduler wake-up,
after taking a subject lock and immediately before provider construction, and
again immediately before `PreparedGeneration`. Failure before provider
construction returns `ActivationExpired` or `BoardArtifactExpired`, performs
zero provider calls and stops the generation scheduler. Expiry after provider
acquisition writes a receipted `pending_dependency` attempt with the exact
acquisition evidence, creates no sample, then transitions the process
capability to Disabled. A startup-only validity check is insufficient.

### 5.2 Receipted pending-source capability

```rust
// TO BE BUILT; cannot be constructed outside the active owner.
pub struct ReceiptedPendingGenerationPage {
    page_run_id: OpaquePageRunId,
    snapshot_identity: OpaqueSnapshotIdentity,
    activation: ReceiptedConfigActivation,
    fairness_round_id: OpaqueFairnessRoundId,
    fairness_round_receipt_high_water: i64,
    fairness_round_checkpoint_before: Option<PendingGenerationKey>,
    fairness_round_phase: FairnessRoundPhase,
    fairness_round_cursor_before: Option<PendingGenerationKey>,
    page_snapshot_receipt_high_water: i64,
    page_snapshot_audit_record_count: u64,
    page_snapshot_audit_tail_hash: String,
    wrapped_once: bool,
    facts: Vec<ReceiptedPendingSourceFact>,
    page_content_hash: String,
}

// TO BE BUILT; every field is verified by the read model and private.
pub struct ReceiptedPendingSourceFact {
    page_row_id: OpaquePageRowId,
    source_fact: ReceiptedSourceFact,
    scheduling_key: PendingGenerationKey,
    logical_subject_key: String,
    next_attempt_ordinal: u32,
    prior_attempt_receipt_hashes: Vec<String>,
    same_subject_latest_receipt_hash: Option<String>,
}

// TO BE BUILT; no caller cursor, sort, filter or limit.
impl GenerationRuntimeOwner {
    fn issue_pending_generation_page(
        &self,
    ) -> Result<Option<ReceiptedPendingGenerationPage>, GenerationRuntimeError>;
}
```

`PendingGenerationKey` has one closed representation:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingGenerationKey {
    // Exact canonical RFC3339 nanosecond UTC string, ending in `Z`.
    first_ingress_committed_at: CanonicalNanosUtc,
    // Nonblank, trim-stable UTF-8, globally unique within one activation.
    source_fact_key: NonBlankCanonicalId,
}
```

Both fields are persisted columns, not caller input. Rust tuple comparison is
exactly `(first_ingress_committed_at, source_fact_key)` ascending after the
timestamp has passed canonical parse and has been bound as an instant.
SQLite binds the timestamp as that canonical fixed-width UTC string and the
key as UTF-8 bytes under `COLLATE BINARY`; alternate offsets, missing
nanoseconds, Unicode normalization, locale collation, `NULL`, empty strings
and sentinels are rejected before the query.

The owner enforces `PENDING_GENERATION_LIMIT=200`; the constant is **TO BE
BUILT** and no caller limit is accepted. Its private read-model query returns
only ingress-`Admitted` source facts joined to:

- their first source fact and source batch attempt;
- exact config-activation receipt;
- exact ingress receipt and committed audit pair;
- immutable `config_hash` and `generation_market_date`;
- exact source provider/source, provider item identity, provider published
  time, observed time, batch ID and content hashes.

It excludes a logical subject already closed by a receipted `completed`,
`verified_no_relation` or `failed_non_retryable` generation for that fact's
immutable config. A receipted `pending_dependency` remains eligible only
within its stored prospective date. The next attempt ordinal must equal the
count of prior receipted attempts plus one; prior receipt hashes are sorted by
`committed_at, stage_run_id, receipt_hash`, all ascending. An envelope or
manifest without its closing receipt is recovery work and is excluded from
the pending issue set.

The stable order is first ingress receipt `committed_at`, then
`source_fact_key`, both ascending; this two-column tuple is the complete
`PendingGenerationKey` and `source_fact_key` is the final unique tie-break.
Filtering happens before ordering. The filter is exactly: current immutable
activation/config, ingress `Admitted` plus closed ingress receipt/audit pair,
no receipted closed generation disposition, and either no prior attempt or a
receipted `pending_dependency` whose prospective date remains open. No
provider, observed-time, headline, candidate count or caller priority may
alter this order.

Fairness is a durable round-robin keyset, not a repeated “first 200” query.
For each activation/config pair the repository stores exactly one
`GenerationFairnessCheckpoint(last_issued_committed_at,
last_issued_source_fact_key, version)` and at most one open
`GenerationFairnessRound(round_id,round_receipt_high_water,
checkpoint_before,phase,round_cursor,version)`. A new round captures its
`round_receipt_high_water` and validated selection-audit prefix exactly once
under a pinned read snapshot. Every page in that round sets
`page_snapshot_receipt_high_water=round_receipt_high_water`; a later receipt
cannot enter the round.

The `AboveCheckpoint` phase pages eligible keys strictly greater than
`checkpoint_before`, ordered by the complete `PendingGenerationKey`, until a
verified empty query proves the phase exhausted at the fixed round high-water.
One receipted CAS then changes the same round to `WrappedAtOrBelowCheckpoint`
with a null wrap cursor. That phase pages eligible keys less than or equal to
`checkpoint_before` under the same high-water until a verified empty query
proves the wrap exhausted. Each phase advances only its private
`round_cursor`; it never recaptures a high-water. No key appears twice within a
round.

The first fairness round has the only permitted
`fairness_round_checkpoint_before=None`. `None` means mathematical negative
infinity, not SQL `NULL` and not a fabricated minimum key. Its canonical Rust
branch omits the checkpoint lower-bound predicate entirely. Its canonical SQL
shape is:

```sql
-- checkpoint_before = None, AboveCheckpoint
WHERE receipt_id <= :round_receipt_high_water
  AND <closed eligibility predicates>
ORDER BY first_ingress_committed_at COLLATE BINARY ASC,
         source_fact_key COLLATE BINARY ASC
LIMIT 200
```

For a later `Some(k)`, the complete keyset predicate is:

```sql
AND (
  first_ingress_committed_at > :checkpoint_committed_at
  OR (
    first_ingress_committed_at = :checkpoint_committed_at
    AND source_fact_key COLLATE BINARY > :checkpoint_source_fact_key
  )
)
```

When the first round's AboveCheckpoint phase is verified exhausted, its
`WrappedAtOrBelowCheckpoint` phase is canonically empty: Rust closes that phase
without issuing a provider call or a SQL page query (`WHERE 0` is permitted
only in the isolated repository proof test). For `Some(k)`, the wrapped
predicate is the same fixed high-water and eligibility predicate plus:

```sql
AND (
  first_ingress_committed_at < :checkpoint_committed_at
  OR (
    first_ingress_committed_at = :checkpoint_committed_at
    AND source_fact_key COLLATE BINARY <= :checkpoint_source_fact_key
  )
)
```

and, when `round_cursor=Some(c)`, the ordinary strict `key > c` predicate is
also present. `> NULL`, `<= NULL`, `COALESCE`, zero timestamps, empty keys and
any other sentinel implementation are forbidden. The first-round fixture
must prove every eligible row is issued once in AboveCheckpoint, the wrapped
branch executes zero query/provider work, and the closing CAS stores the last
real emitted key; an empty first round leaves the global checkpoint `None`.

Before any subject lock or provider I/O, the exact ordered key list, snapshot
identity/high-water, round ID/high-water/phase/cursor-before,
checkpoint-before, wrap bit and page hash are committed as a receipted
`generation_page_run`. Each row has a durable processing state.
Retryable/provider/data failures close that row with their actual receipted
disposition and processing continues to later rows; a subject-lock loser
closes only the scheduling row as `deferred_lock_busy` and performs zero
provider calls. One failing or busy subject therefore cannot abort the
remaining independent subjects in the page.

Only after every page row is in a durable safe state does one CAS close the
page and advance that phase's `round_cursor` to the page's last scheduling
key. The global fairness checkpoint changes only when the fixed-high-water
round has exhausted both phases; the round-closing CAS sets it to the final
issued key, or leaves it byte-identical when the round emitted no key. A crash
leaves the page/round open; startup recovers its remaining stored keys and
resumes the same phase, cursor and high-water before issuing another page. It
never re-queries a different first 200 or starts a new round early. An
integrity failure keeps the page/round open and terminates the process. An
empty full-wrap round yields a receipted verified-empty scheduling observation
and no provider call. Facts committed after the frozen high-water wait for the
next round, so sustained higher-key arrivals cannot postpone the wrap and every
continuously eligible fact is reached after finitely many closed pages.

The capability binds the verified pinned snapshot, database receipt
high-water and selection-audit tail hash. Waiting for a lock or provider
response cannot turn a stale capability into a write authority.

For each fact, the owner derives the subject-lock filename only as
`sha256(canonical logical_subject_key bytes) + ".lock"` under the
mode-bound fixed `selection-generation-subjects` lock root. It acquires the OS
lock non-blocking. The loser records one bounded busy observation and performs
zero provider calls and only the page-row `deferred_lock_busy` scheduling
write described above. The winner keeps the lock through:

1. fresh receipt/audit-prefix validation;
2. same-subject terminal and latest-attempt requery;
3. durable pre-I/O acquisition-intent commit/read-back;
4. provider acquisition outside SQLite/audit critical sections;
5. durable response-evidence seal;
6. durable `PreparedGeneration` construction from sealed bytes;
7. stage/audit/receipt commit; and
8. exact receipt and page-row safe-state read-back.

The winner then releases the lock. A process crash releases only the OS lock;
the next owner must recover the exact durable envelope/manifest before issuing
new work. An unrelated global receipt may advance the database high-water, so
fresh validation compares the old audit/receipt prefix and exact
same-subject high-water rather than requiring global equality. Any prefix
replacement or same-subject conflict is fatal. There is no retry-count limit
or wall-clock aging rule: only the immutable prospective market date and the
runtime activation/artifact validity windows can close eligibility.

### 5.3 Canonical relation candidate

```rust
// TO BE BUILT; created only by SelectionRelationOwner.
struct CanonicalSelectionCandidate {
    candidate_key: String,
    source_fact_key: String,
    event_id: String,
    config_activation_run_id: String,
    config_hash: String,
    generation_market_date: NaiveDate,
    chain_id: String,
    matched_keyword: String,
    security: SecurityIdentity,
    relation_entries_in_relation_order: Vec<CanonicalRelationEvidence>,
    relation_evidence_set_hash: String,
}
```

`SelectionRelationOwner` receives only `ReceiptedPendingSourceFact` and its
immutable activation snapshot.

The chain match is evaluated first. Its authority is only the immutable
activation snapshot's ordered chain rules. A match records the real
`chain_id`, the exact configured keyword bytes, normalized comparison form,
source field, byte offsets and rule hash. When multiple keywords match one
chain, `matched_keyword` is the first by configured keyword ordinal, then
lowest byte offset; the remaining matches stay in ordered relation evidence.
When multiple chains match, each real chain is a separate cohort in activation
priority-descending/chain-ID order.

For `DirectMention`, the owner resolves exact code/name mentions against the
complete Magic TDX security-master batch and records offsets and master
evidence. A direct mention is attached only to an independently matched real
chain from the same receipted source fact. A mention with no activated chain
match produces no candidate and cannot use `direct`, `unknown`, a stock code,
an evidence hash or a locally derived value as `chain_id` or
`matched_keyword`. If every configured relation is complete, that case
contributes to the receipted `verified_no_relation` result.

For `ProviderBoardConstituent`, it:

1. applies the immutable activated chain keyword rules to the receipted source
   text;
2. resolves the exact chain's reviewed provider-board binding;
3. requests the full bound board membership through `BoardDataGateway`;
4. preserves provider order and complete board request/response evidence;
5. emits one candidate per canonical A-share member.

Direct mention and board evidence flow into one candidate for the logical
`(event_id, chain_id, canonical_stock_code)` cohort frozen by this document.
Evidence entries
are ordered DirectMention first (exact code before exact name), then board
constituent by provider/kind/canonical board code. The evidence-set hash is
terminal sample content, never `candidate_key`/`sample_key`; late evidence
cannot manufacture a second cohort. The sample identity is exactly
`(event_id, chain_id, stock_code, relation_schema_version, feature_version,
evaluation_market_date)`.

Exact duplicate evidence is idempotent and conflicting evidence is rejected;
the owner never silently map-overwrites it. Candidates sort by provider
publication time, event ID, chain priority descending, chain ID and canonical
stock code. The relation entries retain their separate stable order.

An activated chain with a required missing/mismatched binding is a typed
relation dependency failure, not an empty board and not a direct-only
downgrade. An immutable chain config that explicitly has no board binding uses
the existing typed `not_configured` binding state. Within a generation run,
all configured relation identities reach a barrier before terminalization: a
retryable board/direct dependency leaves zero terminal samples even if another
relation is complete. A complete, verified zero-member response may contribute
to `VerifiedNoRelation` only after every configured relation reaches the
barrier.

### 5.4 Magic TDX market evidence

The production market request changes from raw event text to canonical
candidates:

```rust
// TO BE BUILT
pub struct SelectionMarketRequest {
    candidates: CanonicalCandidateBatch,
    window: SelectionMarketWindow,
    evaluation_at: DateTime<Local>,
    expected_latest_settled_date: NaiveDate,
    validation: OpaqueMarketValidationRequest,
}
```

The gateway must not parse news text or decide causal relation. It validates
each candidate identity against the same complete Magic TDX security master,
then requests daily bars, the quote and intraday five-minute bars required by
the selected window.

`OpaqueMarketValidationRequest` is owner-built and always requires all three
of the following; callers cannot turn them off:

1. an exact `SecurityLifecycleGateway::acquire` request for the canonical
   security and complete daily-bar interval, returning lifecycle evidence with
   provider/source, request hash, listing date, board/market, observed time and
   batch/content hash;
2. the same lifecycle response's exact corporate-action evidence covering the
   full requested interval, including an explicit verified-empty batch when
   there was no split/dividend action; and
3. lookup of a BR-171 `DailyChangeConfirmationReceipt` for every adjacent
   valid close change whose absolute value is greater than 20%.

The BR-171 lookup query must bind the exact code, previous/current dates and
closes, calculated percentage, daily provider/source/batch, lifecycle
provider/batch, listing date and corporate-action identity. A lifecycle or
corporate-action response that is missing, partial, stale, contradictory or
not identity-equal to the requested security is `pending_dependency`.
Corporate-action or new-listing context explains the jump but does not waive
the repository's manual-confirmation requirement. A missing, conflicting or
nonmatching confirmation receipt is
`pending_dependency/manual_confirmation_required`; no features, admission or
sample are produced.

Evidence is component-specific and cannot be collapsed into a locally invented
top-level batch identity:

| Component | Required evidence |
| --- | --- |
| security master | provider=`tdx`, source capability, request hash, provider order/count, observed time, batch/content hash |
| board relation | provider/source, provider source/observed time, request hash, batch ID, complete membership hash |
| settled daily bars | code/market, requested category/count, provider market date per bar, observed time, response content/batch hash |
| quote | code/market, provider `servertime` as source time, observed time, request/response batch hash |
| five-minute bars | code/market/category/count, provider bar end times, observed time, response content/batch hash |
| security lifecycle | code/market, listing date, lifecycle state, request hash, provider/source/observed time, batch/content hash |
| corporate actions | code/market/date interval, complete event list or verified-empty marker, request hash, provider/source/observed time, batch/content hash |
| manual confirmation | exact BR-171 query identity hash, confirmation ID, immutable record hash, all bound dates/closes/providers/batches/lifecycle/action identity |

If the upstream protocol has no independent intraday source timestamp for a
settled daily bar, `source_at` remains absent and the provider trading date is
stored separately. `Local::now()` must never be presented as provider source
time. The existing locally derived aggregate `batch_id` may bind the composite
preimage, but it may not replace component provider evidence.

The gateway returns a complete candidate-result matrix: every input candidate
has exactly one complete market record or one typed rejection. It must not
silently omit a candidate or return a partial “success” as a complete batch.

#### 5.4.1 Canonical persistence mapping

The complete market result is not merely retained in memory. The generation
preimage maps it into the current schema-v2 canonical columns exactly as
follows:

| Source evidence | Canonical persisted projection |
| --- | --- |
| immutable request capability + parameters | `SelectionEvaluationAttemptRowContentPreimage.market_request_hash`, `request_evidence_json`, `request_evidence_hash` using `RequestKind::T0MarketEvidence` |
| selected provider attempt sequence | `available_evidence_json/hash`, including ordered transport attempts and selected result hash |
| security master, daily, quote, five-minute, lifecycle, corporate-action and confirmation evidence | an ordered component array inside the T0 available-evidence preimage; order is `security_master,daily_bars,quote,five_minute_bars,security_lifecycle,corporate_actions,manual_confirmations`, with absent quote/5m allowed only for `PostClose` |
| complete aggregate projection | evaluation-attempt `provider/source/source_at/observed_at/batch_id/batch_content_hash`; every scalar must equal its typed evidence projection |
| feature inputs | `SelectionSampleRowContentPreimage.t0_feature_json/hash`; it references the exact component hashes and confirmation record hashes, not copied unbound values |
| terminal sample market fields | `market_provider`, `market_source`, `market_source_at`, `market_observed_at`, `market_batch_id`, `market_batch_content_hash`, equal to the complete aggregate evidence |
| invalid/partial provider result | evaluation-attempt `error_detail_json/hash/fingerprint` plus all available component evidence; no sample row |

The component array, lifecycle/action evidence and confirmation hashes are
part of both `PreparedGeneration` canonical bytes and the generation receipt
closure. The repository recomputes them from canonical JSON before commit.
Dropping a component, changing order, projecting a scalar from a different
batch or using the locally derived aggregate ID in place of component evidence
is an integrity error.

#### 5.4.2 Immutable A-share calendar and T0..D5 ownership

The mutable `crate::calendar` holiday set, `TRADING_HOLIDAYS`,
`add_holidays`, weekdays inferred at runtime and caller-supplied `&[NaiveDate]`
are forbidden as selection schedule evidence. Activation requires the
strictly parsed checked-in calendar **and** its immutable raw official-notice
bundle at these fixed manifest-root locations:

```text
config/selection/a_share_trading_calendar.v1.json
config/selection/a_share_trading_calendar_notices.v1.json
config/selection/a_share_trading_calendar_notices.v1/sse/<notice_id_sha256>.raw
config/selection/a_share_trading_calendar_notices.v1/szse/<notice_id_sha256>.raw
```

These are three non-aliasing authorities whose spelling is frozen:

```text
calendar_manifest_path=config/selection/a_share_trading_calendar.v1.json
notice_manifest_path=config/selection/a_share_trading_calendar_notices.v1.json
raw_notice_root=config/selection/a_share_trading_calendar_notices.v1/
```

The `.json` notice manifest is one regular file. The raw root is the distinct
directory without the `.json` suffix; SSE/SZSE leaves exist only below that
directory. The manifest file and raw root must have different basenames and
different descriptor identities. Any implementation that treats
`a_share_trading_calendar_notices.v1.json` as both a file and a directory,
derives either path from the other, or accepts an alternate path fails before
activation.

`<notice_id_sha256>` is exactly the lowercase SHA-256 of the canonical
nonblank official `notice_id` bytes recorded in the notice manifest; it is not
a caller path. The manifest entries are strictly sorted by
`provider,published_at,notice_id`, provider is only `sse` or `szse`, and the
stored relative path must exactly equal the formula above. Symlinks,
hard-linked aliases, path traversal, missing/extra raw leaves and any file
outside this root reject activation. The raw file is the exact decoded HTTP
response body bytes captured from the canonical official URL; no
pretty-printing, DOM rewrite or text extraction occurs before hashing.

The calendar is a real-data artifact with this closed canonical payload:

```text
domain=stock_analysis.a_share_trading_calendar.v1
schema_version=1
provider=sse_szse_official_trading_calendar
provider_version=<nonblank immutable official release/revision token>
coverage_start=<YYYY-MM-DD>
coverage_end=<YYYY-MM-DD>
session_dates=<strictly ascending unique YYYY-MM-DD array>
source_notices=<provider,notice_id,notice_id_sha256,canonical_url,published_at,
                raw_artifact_path,raw_content_sha256,parser_id,parser_version>
captured_at=<RFC3339-nanos with offset>
executable_revision=<40-lower-hex>
```

Unknown/missing fields, weekends in `session_dates`, duplicate/out-of-order
sessions, source notices without provider publication time, coverage that
does not include every activation-effective T0 plus five following sessions,
or a noncanonical file are fatal claimed-authority failures. Insufficient
claimed coverage is exactly `calendar_release_integrity_conflict`; it is not
`TradingCalendarCoverageIncomplete` Disabled. The artifact hash is exactly:

```text
calendar_hash =
  sha256("stock_analysis.a_share_trading_calendar_artifact.v1\0"
         || RFC-8785 canonical payload bytes)
calendar_version =
  "sse_szse_official_trading_calendar/"
  || provider_version
```

Both strings, the provider token, source-notice hashes and raw artifact hash
are included in the activation manifest and activation receipt together with:

```text
notice_manifest_content_hash =
  sha256("stock_analysis.a_share_calendar_notice_manifest.v1\0"
         || RFC-8785 canonical NoticeManifestPayload bytes)
calendar_raw_notice_set_hash =
  sha256("stock_analysis.a_share_calendar_raw_notice_set.v1\0"
         || RFC-8785 canonical raw-notice-set payload bytes)
calendar_parser_equality_hash =
  sha256("stock_analysis.a_share_calendar_parser_equality.v1\0"
         || RFC-8785 canonical parser-equality payload bytes)
```

`NoticeManifestPayload` is a closed object whose exact logical field order is
`domain,schema_version,entries`. `domain` is
`stock_analysis.a_share_calendar_notice_manifest.v1`, `schema_version` is `1`
and `entries` is the manifest-ordered array of closed objects whose exact
logical field order is
`provider,published_at,notice_id,notice_id_sha256,canonical_url,
raw_artifact_path,raw_content_sha256,parser_id,parser_version`. Every entry
must be in the already frozen `provider,published_at,notice_id` ascending
order, and every value must equal the matching `source_notices` entry and raw
descriptor evidence. The payload and every entry deny unknown or missing
fields. The checked-in file bytes must equal the RFC-8785 canonical
serialization of that decoded closed payload; semantic JSON with noncanonical
object-key encoding, a reordered entry array, an extra field or a single
changed field rejects activation.

The raw-notice-set payload is a closed object with exactly
`domain,schema_version,entries`. `domain` is
`stock_analysis.a_share_calendar_raw_notice_set.v1`, `schema_version` is `1`
and `entries` is the manifest-ordered array of closed objects with exactly
`provider,published_at,notice_id,raw_artifact_path,raw_content_sha256`. The
parser-equality payload is a closed object with exactly
`domain,schema_version,coverage_start,coverage_end,parser_descriptors,
session_dates,t0_d5_vectors`. Its domain is
`stock_analysis.a_share_calendar_parser_equality.v1`, schema is `1`,
`parser_descriptors` is the manifest-ordered array of closed
`provider,notice_id,parser_id,parser_version,executable_revision,
raw_content_sha256` objects, `session_dates` is the checked strictly ordered
array and `t0_d5_vectors` is a T0-ascending array of closed
`t0,d1,d2,d3,d4,d5` objects. Unknown/missing fields, a different array order or
non-RFC-8785 bytes reject activation. The raw-notice-set and parser-equality
root/entry objects also deny unknown or missing fields. No implementation may
hash delimiter-free field concatenation, debug output or provider JSON for
any of these three identities.

For every notice entry the owner opens the exact descriptor-relative raw path,
hashes the complete bytes and invokes the closed deterministic parser selected
by `(provider,parser_id,parser_version,executable_revision)`. The parser has no
clock, environment, network, locale or caller override. It emits explicit
closed dates and exceptional open dates for the declared coverage. The owner
derives a session vector as coverage weekdays minus parser-proven closed dates
plus parser-proven exceptional opens. The independently parsed SSE and SZSE
session vectors must be byte-equal to each other and byte-equal to the
calendar artifact's `session_dates`; a parse ambiguity or exchange mismatch is
fatal `calendar_parser_or_session_conflict`. `TradingCalendarUnverified`
denotes only an absent reviewed verification prerequisite before any artifact
claims activation authority.

The equality check then enumerates every activation-effective T0 and derives
its exact next five sessions independently from (a) the checked-in
`session_dates`, (b) the SSE raw-notice parse and (c) the SZSE raw-notice
parse. All three `T0,D1,D2,D3,D4,D5` vectors must be byte-equal. Their ordered
preimages form `calendar_parser_equality_hash`; the hash is bound into the
activation manifest/receipt, every generation date binding and the Gate-D
join.

File mtime, the current process holiday set and a fresh local clock never
replace these bytes. The artifact is a reviewed release input, not
runtime-discovered configuration: the raw official bytes, notice manifest and
calendar are committed in the same PR and reviewed before activation. Gate D
re-fetches or independently validates every canonical official notice URL,
publication time and raw-content hash against the checked-in bytes. An
unpublished session, locally inferred runtime weekday, provider HTML without
a stable notice identity, a hash-only notice with no checked-in raw bytes, or
an operator-edited date list is not admissible calendar evidence.

Only a TO-BE-BUILT `SelectionTradingCalendarOwner`, constructed from the
receipted activation, may create the private, non-Clone
`ReceiptedTradingCalendarSnapshot`. For each generation subject it finds the
exact `generation_market_date` in `session_dates`, takes that date as T0 and
the next five stored sessions as D1..D5, and creates the existing canonical
`OutcomeTradingDateVectorPreimage`. It recomputes
`trading_date_vector_hash=sha256_json(vector)` and returns a private
`FrozenGenerationDateVector` binding:

```text
activation_run_id
activation_receipt_hash
calendar_provider
calendar_version
calendar_hash
calendar_artifact_content_hash
notice_manifest_content_hash
calendar_raw_notice_set_hash
calendar_parser_equality_hash
T0,D1,D2,D3,D4,D5
trading_date_vector_hash
```

`PreparedGeneration`, every sample row and the generation receipt must carry
that exact binding. The sample's `calendar_version`, `calendar_hash`,
T0..D5 columns, `trading_date_vector_json` and
`trading_date_vector_hash` must equal it byte-for-byte. The existing
`derive_outcome_schedule` arithmetic may be reused privately only after it
accepts this opaque snapshot; its current public raw-slice form is not a
production authority. After a valid activation, a receipted fact whose
prospective generation date lies outside that already verified activation
window is a subject dependency rather than a malformed release artifact; only
that case is
`pending_dependency/trading_calendar_coverage_incomplete`, never a weekday
fallback or a terminal stock rejection.

The owner pins the validated calendar, notice-manifest and every raw-notice
descriptor and canonical byte sequence for its full lifetime. Before every
provider request and generation commit it recomputes all five calendar/notice
hashes, reruns the deterministic equality proof and verifies the activation
binding; pathname replacement, in-process holiday mutation or a newer
calendar/notice file cannot change an already prepared subject.

#### 5.4.3 Self-contained BR-171 closed receipt lookup

BR-193 does not treat the current
`has_exact_daily_change_confirmation(...) -> Result<bool, _>` as evidence.
Gate B replaces that production read seam with this self-contained contract:

```rust
// TO BE BUILT; fields private, not Default, not caller-constructible.
pub struct ClosedDailyChangeConfirmationReceipt {
    confirmation_id: String,
    query_identity_hash: String,
    record_hash: String,
    ledger_tail_record_hash: String,
    ledger_row_high_water: i64,
}

impl DailyChangeConfirmationOwner {
    fn lookup_exact_closed_receipt(
        &self,
        query: &DailyChangeConfirmationQuery,
    ) -> Result<Option<ClosedDailyChangeConfirmationReceipt>,
               DailyChangeConfirmationError>;
}
```

The owner is constructed only from the same descriptor-pinned selection
database snapshot used by the generation subject. It canonicalizes the entire
query listed in section 5.4, validates the complete retained BR-171 hash chain,
loads exactly one row by `query_identity_hash`, recomputes the canonical query,
confirmation content identity and chain `record_hash`, and returns the
immutable `confirmation_id` plus that independently recomputed
`record_hash`. It also binds the ledger row high-water and validated tail so a
later row cannot be mistaken for the proof observed by this subject.

No row returns `Ok(None)`. A byte-equal closed row returns
`Some(ClosedDailyChangeConfirmationReceipt)`. Duplicate/conflicting rows,
query-hash collision, missing chain link, changed record bytes or tail
replacement are integrity errors, never `false`. Production validation may
continue only from `Some`; it persists `confirmation_id`,
`query_identity_hash`, `record_hash`, row high-water and ledger tail in the
market evidence seal, `PreparedGeneration`, sample proof and generation
receipt. A Boolean, count, locally recomputed ID without chain lookup, or
append API's `inserted` flag is not admissible confirmation evidence.

#### 5.4.4 Quality, feature and admission seam

Production may not call the current public chain
`validate_daily(&[SelectionBar]) -> compute_daily_features(&[SelectionBar]) ->
evaluate_admission(&RawSelectionFeatures)` directly. Gate B makes the
production entry crate-private and owned:

```text
OpaqueMarketValidationRequest
  -> CompleteMagicTdxCandidateEvidence
  -> LifecycleAndActionValidatedSeries
  -> DailyChangeConfirmationOwner
  -> ConfirmedSelectionMarketSeries
  -> CompleteSelectionFeatures
  -> AdmissionV1Decision
```

`ConfirmedSelectionMarketSeries` can be constructed only after positive
prices, exact immutable-calendar continuity, no gaps/duplicates, explicit
unadjusted status, complete lifecycle/corporate-action coverage and an exact
BR-171 receipt for **every** adjacent valid close change whose absolute value
is greater than 20%. The current `quality::validate_daily` formula is adopted
only after being deepened to accept that opaque confirmation capability; it
must not reclassify a confirmed jump as invalid and must not expose a
production raw-slice bypass. New listings and real fast-rising securities are
not rejected merely because the move exceeds 20%; they remain
`manual_confirmation_required` until the required independent receipt exists,
then continue through the ordinary quality and strategy predicates.

`features` retains `FEATURE_VERSION=raw-selection-v1` and its formulas, but its
production function consumes `ConfirmedSelectionMarketSeries` and returns
non-optional finite `CompleteSelectionFeatures`. `admission` retains
`ADMISSION_VERSION=admission-v1` and the constants in current HEAD, but its
production function accepts only `CompleteSelectionFeatures`. The current
optional-feature behavior that turns `*_missing`, `*_nonfinite` or
`moving_average_nonpositive` into `AdmissionDecision::Rejected` is rejected
for production; those values cannot reach admission.

The result mapping is exhaustive:

| Stage | Closed result codes | Persisted disposition |
| --- | --- | --- |
| provider/dependency | `daily_bars_unavailable`, `daily_feature_history_insufficient`, `volume_baseline_missing`, `intraday_volume_baseline_missing`, `trading_calendar_coverage_incomplete`, `manual_confirmation_required` | receipted `pending_dependency`, no sample |
| invalid real provider data | `security_code_empty`, `mixed_security_batch`, `duplicate_bar`, `bar_out_of_order`, `bar_gap`, `bar_non_trading_day`, `bar_not_settled`, `adjustment_not_unadjusted`, `split_continuity_unverified`, `ohlc_inconsistent`, `price_non_positive`, `volume_nonfinite`, `amount_nonfinite`, `volume_negative`, `amount_negative`, `daily_future`, `intraday_volume_invalid` | receipted `failed_non_retryable`, no sample |
| stale real provider data | `quote_stale`, `daily_stale` | receipted `pending_dependency`, no sample |
| valid complete strategy failure | `trend_alignment_failed`, `price_below_ma5`, `price_ma20_distance_out_of_range`, `five_day_return_out_of_range`, `settled_volume_confirmation_failed`, `intraday_volume_confirmation_failed` | terminal sample `HardRejected` plus one or more non-empty rejection rows |
| valid complete strategy success | no rejection code | terminal sample `Admitted` |

No broad `invalid_provider_data` token may replace a known code in that table.
Unknown quality/feature/admission codes are integrity failures until this
closed taxonomy and BR-193 are amended first.

### 5.5 Opaque generation preparation and commit

```rust
// TO BE BUILT; fields accessible only to persistence owner.
pub struct PreparedGeneration { /* canonical stage preimages */ }

// TO BE BUILT; only active capability owns this.
impl GenerationRuntimeOwner {
    pub async fn acquire_and_process_news_tick(
        &self,
    ) -> Result<GenerationTickReceipt, GenerationRuntimeError>;

    pub async fn recover_then_process_pending(
        &self,
    ) -> Result<GenerationTickReceipt, GenerationRuntimeError>;
}
```

The owner sequence is:

```text
recover manifested-unreceipted then envelope-only non-outcome stages
  -> recover/resume fixed-high-water fairness round and acquisition state
  -> wait for and commit/read back restart-aware cadence receipt
  -> commit/read back ingress_tick_plan intent with the ordered feed plan
  -> for each registered feed in order:
       commit/read back its ingress_feed intent
       perform exactly that feed read with NEWS_PER_FEED_LIMIT=20
       seal/sync/read back its complete response/error before the next feed
  -> compose/read back the provider-free GenerationGlobalNewsBatchSeal
  -> on success/verified-empty only, commit the exact RawNewsAggregationBatch
       through PreparedSourceIngress
  -> append/sync/read back GenerationIngressCycleTerminalReceipt
  -> recover any open receipted generation page
  -> issue durable ReceiptedPendingGenerationPage (max 200)
  -> acquire one non-blocking subject lock
  -> revalidate snapshot/high-water/logical subject
  -> derive pure activated-chain match/request plan
  -> for every external step: commit acquisition intent before I/O
  -> perform that one read and durably seal its exact response/error evidence
  -> build canonical direct/chain relation candidates only from sealed evidence
  -> acquire and seal Magic TDX market evidence
  -> join closed BR-171 confirmation receipts, never Boolean presence
  -> bind immutable official-calendar T0..D5 vector
  -> validate freshness and bad-data rules
  -> calculate frozen features/admission
  -> durably persist opaque PreparedGeneration canonical bytes/hash
  -> SelectionV2PersistenceOwner::commit_generation(prepared)
  -> close page row; after all rows CAS page/round cursor
  -> after both round phases exhaust CAS round/global checkpoint
  -> return only receipted counts/hashes
```

`SelectionV2PersistenceOwner::commit_generation` changes from accepting a raw
`GenerationStageRequest` to accepting `PreparedGeneration`. The repository
still recomputes all canonical bytes/hashes and commits through the existing
Prepared -> stage manifest -> Committed -> receipt choreography.

Generation terminal semantics:

- at least one valid candidate may yield one or more immutable samples;
- accepted criteria produce persisted `Admitted` (`Selected` in business
  rendering);
- failed admission criteria produce persisted `HardRejected` with non-empty
  typed rejection rows;
- a complete event with no causal relation produces a receipted
  `verified_no_relation` manifest;
- provider/dependency transport failure produces `pending_dependency` and full
  attempt evidence, no sample;
- immutable lineage/hash/receipt/audit conflict is fatal and writes no closing
  business manifest; the exact closed source/domain conditions listed as
  `SelectionFailedNonRetryableCode` write `failed_non_retryable`; missing rule
  2.3 lifecycle/action/confirmation evidence writes `pending_dependency`;
- no bad candidate is left as “recorded only.”

#### 5.5.1 Durable acquisition and PreparedGeneration recovery

An in-memory “envelope before request” is insufficient. BR-193 adds the
following distinct receipted artifacts under the existing selection database
and the sole `SelectionAuditRecord` chain:

```text
GenerationAcquisitionCadenceReceipt
  -> ingress plan GenerationAcquisitionIntent
  -> per-feed GenerationAcquisitionIntent
  -> per-feed FeedAcquisitionResolution =
       Sealed{intent_hash,seal_hash} |
       Uncertain{intent_hash,uncertainty_record_hash}
  -> GenerationGlobalNewsBatchSeal
  -> PreparedSourceIngress + source-ingress receipt, when success/verified-empty
  -> GenerationIngressCycleTerminalReceipt

generation-subject GenerationAcquisitionIntent
  -> GenerationResponseEvidenceSeal | GenerationAcquisitionUncertain
  -> PersistedPreparedGeneration
  -> existing generation Prepared -> Committed -> receipt
```

Before a new ingress tick, the owner must CAS-insert, append, sync and read back
one `GenerationAcquisitionCadenceReceipt`. Its exact field order is
`domain,schema_version,cadence_receipt_id,mode_namespace,activation_run_id,
activation_receipt_hash,scheduler_cycle_id,acquisition_started_at,
next_acquisition_eligible_at,prior_cadence_receipt_hash,boot_instance_id,
committed_at`; `next_acquisition_eligible_at` is exactly
`acquisition_started_at + NEWS_FETCH_PERIOD_SECS`. The current receipt is the
single authority across process restarts. A new scheduler cycle cannot be
allocated while `now < next_acquisition_eligible_at`, while the prior cycle is
not closed by an exact synced/read-back
`GenerationIngressCycleTerminalReceipt`, or when the clock is earlier than the
prior committed observation. It waits without constructing a provider and
never replaces the receipt. A crash after cadence-receipt closure but before
cycle terminalization resumes that same `scheduler_cycle_id`; it does not
allocate a second tick. Receipt/hash conflict or clock regression is fatal.

`GenerationIngressCycleTerminalReceipt` has exact field order:

```text
domain,schema_version,cycle_terminal_receipt_id,mode_namespace,
activation_run_id,activation_receipt_hash,scheduler_cycle_id,
cadence_receipt_hash,ingress_plan_intent_hash,
ordered_feed_resolutions,total_feed_count,resolved_feed_count,
uncontacted_suffix_count,stopped_after_feed_ordinal_or_null,
global_news_batch_seal_hash,terminal_kind,
source_ingress_stage_run_id_or_null,source_ingress_receipt_hash_or_null,
failure_code_or_null,verified_empty_feed_count,
total_response_record_count,closed_at,prior_cycle_terminal_receipt_hash
```

`ordered_feed_resolutions` is an array of the exact closed
`FeedAcquisitionResolution` union frozen below. Its zero-based array position
is the matching frozen registration-plan feed ordinal; an ordinal is never
stored a second time or caller supplied:

```text
FeedAcquisitionResolution =
  Sealed { intent_hash, seal_hash }
| Uncertain { intent_hash, uncertainty_record_hash }
```

The RFC-8785 representation is an internally tagged closed object. `Sealed`
has exact logical field order `kind,intent_hash,seal_hash` and serializes
`kind="sealed"`. `Uncertain` has exact logical field order
`kind,intent_hash,uncertainty_record_hash` and serializes
`kind="uncertain"`. Every hash is exactly 64 lowercase hexadecimal and must
join the immutable record named by that variant. Unknown/missing fields,
wrong-case kinds, a seal hash in `Uncertain`, an uncertainty hash in `Sealed`
or one intent referenced by both variants is fatal
`generation_state_ambiguous`.

`total_feed_count` equals the frozen registration plan length and is an
I-JSON-safe integer `>=1`;
`resolved_feed_count == ordered_feed_resolutions.len()` and
`resolved_feed_count + uncontacted_suffix_count == total_feed_count`.
Success/verified-empty requires `resolved_feed_count == total_feed_count`,
`uncontacted_suffix_count=0`,
`stopped_after_feed_ordinal_or_null=null` and every resolution `Sealed`.
A pending/nonretryable cycle is one contiguous resolved prefix: all entries
before the last are successful/verified-empty `Sealed`; the last is either the
typed non-success `Sealed` that stopped the cycle or exactly one `Uncertain`.
It requires `resolved_feed_count>=1`,
`stopped_after_feed_ordinal_or_null=resolved_feed_count-1` and
`uncontacted_suffix_count=total_feed_count-resolved_feed_count`. No
`ingress_feed` intent, seal, uncertainty record or provider future may exist
for any ordinal in that suffix. A gap, suffix intent, nonterminal failure
before the last resolution, more than one `Uncertain`, or an `Uncertain` that
is not last is fatal. These equations apply even when the failure occurs on
the final planned feed and the suffix count is zero.

`terminal_kind` is exactly `source_ingress_committed`, `verified_empty`,
`pending_dependency` or `failed_non_retryable`. The first two require a
byte-identical source-ingress stage/receipt; the latter two require both
source-ingress fields to be null and create zero source facts.
`failure_code_or_null` is null for the first two, one closed
`SelectionPendingDependencyCode` for `pending_dependency`, and one closed
`SelectionFailedNonRetryableCode` for `failed_non_retryable`. The receipt
inner-joins every ordered resolution to its exact intent plus response seal or
uncertainty record, proves the suffix has no intent, joins the pure aggregate
seal and cadence receipt, is appended/synced/read back before the cycle is
closed, and is the only durable authority allowing allocation of the next
cadence cycle.
Conflicting or duplicate-different terminal receipts are fatal.

The aggregate-seal/terminal-receipt mapping is closed and byte-exact:

| Final ordered resolution state | Exact aggregate outcome | Exact terminal kind | Exact failure code |
| --- | --- | --- | --- |
| full nonempty plan, every resolution `Sealed` as normal success/empty, every feed empty | `verified_empty` | `verified_empty` | JSON null |
| full nonempty plan, every resolution `Sealed` as normal success/empty, at least one `success_nonempty` | `success_nonempty` | `source_ingress_committed` | JSON null |
| stopped prefix ending in `transport_failure` | `pending_dependency` | `pending_dependency` | `feed_unavailable` |
| stopped prefix ending in `provider_cancelled` | `pending_dependency` | `pending_dependency` | `provider_cancelled` |
| stopped prefix ending in exactly one final `Uncertain` | `pending_dependency` | `pending_dependency` | `acquisition_outcome_uncertain` |
| stopped prefix ending in `feed_response_limit_exceeded` | `failed_non_retryable` | `failed_non_retryable` | `feed_response_limit_exceeded` |

No other aggregate outcome, terminal kind or failure-code cross-product is
valid. `source_ingress_stage_run_id_or_null` and
`source_ingress_receipt_hash_or_null` are both non-null only in the first two
rows and must join the exact aggregate bytes; they are both null in the final
four rows.

The closed acquisition-step order is:

```text
global_news_feed (registered feed ordinal asc),
global_news_batch_seal (pure composition, zero I/O),
board_membership (activation chain priority desc, chain_id asc),
security_master,
daily_bars (canonical_stock_code asc),
quote (canonical_stock_code asc, Intraday only),
five_minute_bars (canonical_stock_code asc, Intraday only),
security_lifecycle_and_corporate_actions (canonical_stock_code asc),
br171_closed_receipt_lookup (canonical_stock_code,previous_date,current_date asc)
```

Optional-by-window steps are omitted only by the rule shown above; their
remaining ordinal never changes. Provider order inside a step is provider
evidence, not a scheduling tie-break.

Before **each** external operation, one immediate transaction persists the
full `GenerationAcquisitionIntent` canonical bytes and domain-separated hash,
then the existing audit envelope/manifest/receipt choreography must append,
sync and read back closure. The intent exact field order is:

```text
domain,schema_version,intent_id,intent_scope,scheduler_cycle_id,
page_run_id,logical_subject_key,
attempt_ordinal,step_ordinal,step_kind,mode_namespace,boot_instance_id,
activation_run_id,activation_receipt_hash,config_hash,
page_snapshot_receipt_high_water,page_snapshot_audit_tail_hash,
request_canonical_bytes,request_sha256,expected_provider_capability,
prospective_market_date,intent_observed_at
```

`request_canonical_bytes` is a BLOB containing the complete typed request
preimage; `request_sha256` is recomputed from those bytes. The same
`(logical_subject_key,attempt_ordinal,step_ordinal)` can have exactly one
byte-identical intent. No network future is constructed or polled until the
intent receipt is read back.

`intent_scope` is closed to `ingress_tick_plan`, `ingress_feed` or
`generation_subject`. The `ingress_tick_plan` intent has non-null
`scheduler_cycle_id`, canonical-null page/subject/attempt/page-snapshot/
prospective fields, `step_ordinal=0`, and request bytes binding the exact
ordered registered feed descriptors plus `NEWS_PER_FEED_LIMIT=20`; it is a
durable plan and performs zero I/O. The frozen registration plan must contain
at least one descriptor. Active capability construction rejects a zero-feed
plan as typed Disabled `ingress_contract_unavailable` before scheduler,
cadence receipt, provider or selection write construction. Immediately before
allocating every cadence receipt, the retained active owner revalidates the
same plan and exact nonzero count; zero, descriptor drift or count drift after
activation is fatal `config_snapshot_conflict` before a cadence write or
provider construction. A zero-feed plan can never produce a cadence receipt,
batch seal, verified-empty result or `PreparedSourceIngress`.

Every registered feed then receives one `ingress_feed` child intent in
registration order with the same null matrix,
`step_ordinal=1+feed_ordinal`, and request bytes binding exactly that feed,
provider capability and limit 20. No feed future is constructed until its
child intent is read back. A feed response/error/cancel seal is persisted,
synced and read back before the next feed intent or future is created. This
serial sealed sequence is mandatory; concurrent unsealed feed reads and one
aggregate intent standing in for multiple provider reads are forbidden.

For `generation_subject`, `scheduler_cycle_id` is null and every
page/subject/attempt/snapshot field is non-null. Any other null matrix is
invalid. Ingress plan/feed intents use
`(scheduler_cycle_id,step_ordinal)` as their unique key and must join the exact
cadence receipt for that cycle.

Immediately after a provider returns success, verified-empty, typed transport
failure or cancellation, and before relation parsing, feature computation or
another provider step, the owner persists a separate
`GenerationResponseEvidenceSeal`. Its exact field order is:

```text
domain,schema_version,seal_id,intent_id,intent_sha256,step_kind,
typed_outcome_kind,provider,source,provider_source_at,market_date,
observed_at,batch_id,response_canonical_bytes,response_sha256,
ordered_attempt_evidence_bytes,ordered_attempt_evidence_sha256,
typed_error_bytes,typed_error_sha256,sealed_at
```

For `intent_scope=ingress_feed`, `typed_outcome_kind` is the exact closed
`FeedAcquisitionOutcomeKind` enum with these serialized tokens and no others:

```text
success_nonempty
verified_empty
transport_failure
provider_cancelled
feed_response_limit_exceeded
```

The strict `GenerationFeedTypedErrorV1` carrier has exact field order
`domain,schema_version,code,redacted_detail_sha256_or_null,retryable`; its
domain is `stock_analysis.selection_v2_generation_feed_error.v1`, schema is
integer `1`, and `code` is exactly `feed_unavailable`,
`provider_cancelled` or `feed_response_limit_exceeded`. Detail is null or a
lowercase SHA-256 of redacted evidence, never raw provider text.

The ingress-feed response/error matrix is exhaustive:

| `typed_outcome_kind` | Decoded response count | response bytes/hash | typed-error bytes/hash | Error code / retryable | Resolution role |
| --- | ---: | --- | --- | --- | --- |
| `success_nonempty` | `1..=20` | both non-null; exact complete canonical vector | both JSON null | none | nonterminal `Sealed` |
| `verified_empty` | `0` | both non-null; exact canonical empty vector | both JSON null | none | nonterminal `Sealed` |
| `transport_failure` | unavailable | both JSON null | both non-null | `feed_unavailable` / `true` | terminal pending `Sealed` |
| `provider_cancelled` | unavailable | both JSON null | both non-null | `provider_cancelled` / `true` | terminal pending `Sealed` |
| `feed_response_limit_exceeded` | `>=21` | both non-null; exact complete untruncated canonical vector | both non-null | `feed_response_limit_exceeded` / `false` | terminal nonretryable `Sealed` |

The response count is recomputed from the sealed canonical response bytes; it
is not caller supplied. `ordered_attempt_evidence_bytes` and its hash are
always both non-null and bind the complete nonempty attempt vector. The
remaining provider fields (`provider_source_at`, `market_date`, `batch_id`)
carry exactly what the provider protocol supplied and are JSON null only when
that protocol supplied no value; they never discriminate an outcome or
substitute for response/error evidence. Any other null combination, token,
count range, error code/retryability pair, bytes/hash asymmetry, response
truncation or noncanonical carrier is fatal `generation_state_ambiguous`.

For one resolution, `sealed_response_record_count` is the decoded complete
response-vector length for `success_nonempty`, `verified_empty` and
`feed_response_limit_exceeded`; it is exactly zero for
`transport_failure`, `provider_cancelled` and `Uncertain`.
`verified_empty_feed_count` is exactly the number of ordered `Sealed`
resolutions whose decoded outcome is `verified_empty`.
`total_response_record_count` is exactly the checked sum of
`sealed_response_record_count` over `ordered_feed_resolutions`, including the
complete untruncated over-limit final response and excluding the uncontacted
suffix. Both counters are I-JSON-safe integers recomputed independently in the
aggregate seal and terminal receipt. Overflow, a caller-supplied value, a
counter mismatch between the two artifacts, or a value inconsistent with any
joined resolution is fatal `generation_state_ambiguous`.

The seal transaction verifies the exact intent receipt, strict enum and whole
matrix, inserts one immutable seal, prepares its audit outbox/manifest and
closes it through append, `sync_data`, receipt read-back before any next step.
A second byte-identical seal is idempotent; a different seal for one intent is
fatal. Prefix/aggregate/terminal validation reopens each sealed carrier and
uses this enum and matrix, never a caller string, to decide whether a `Sealed`
entry is nonterminal success/empty or the final typed non-success.

A prior-boot receipted intent without a seal instead persists the strict
`GenerationAcquisitionUncertainPreimageV1` with exact logical field order:

```text
domain,schema_version,uncertainty_id,intent_id,intent_sha256,
prior_boot_instance_id,detection_boot_instance_id,detected_at,reason_code
```

Its domain is
`stock_analysis.selection_v2_generation_acquisition_uncertain.v1`, schema is
`1`, IDs/times/hashes use the same validated canonical newtypes as other
BR-193 evidence, and `reason_code` is exactly
`acquisition_outcome_uncertain`. The output hash is absent from the preimage:

```text
uncertainty_record_hash =
  sha256("stock_analysis.selection_v2_generation_acquisition_uncertain.v1\0"
         || RFC-8785 canonical preimage bytes)
```

The strict immutable carrier has exact logical field order
`preimage,uncertainty_record_hash`, denies unknown/missing fields and is
appended/synced/read back before it may appear in an `Uncertain` resolution.
A byte-identical duplicate is idempotent; the same intent with different
uncertainty bytes/hash, any later seal for that intent or more than one
uncertainty row is fatal.

For every `global_news_feed`, the complete returned record vector is counted
before another feed is contacted. `records.len() <= NEWS_PER_FEED_LIMIT` is
mandatory. A vector of 21 or more records is sealed in full as the typed
non-success outcome `feed_response_limit_exceeded`; it creates zero source
facts and is never truncated, sampled or treated as verified empty. Exactly 20
is valid. The same limit is present in each feed's pre-I/O request bytes, so
request and response sides are independently verifiable.

After either the complete success/verified-empty feed plan is sealed or one
typed non-success/uncertain resolution closes a contiguous stopped prefix, a
provider-free `GenerationGlobalNewsBatchSeal` is built with exact field order:

```text
domain,schema_version,global_news_batch_seal_id,scheduler_cycle_id,
cadence_receipt_hash,ingress_plan_intent_hash,ordered_feed_resolutions,
aggregate_outcome_kind,total_feed_count,resolved_feed_count,
uncontacted_suffix_count,stopped_after_feed_ordinal_or_null,
verified_empty_feed_count,total_response_record_count,
raw_news_aggregation_batch_bytes_or_null,
raw_news_aggregation_batch_sha256_or_null,composed_at
```

`aggregate_outcome_kind` is exactly `success_nonempty`, `verified_empty`,
`pending_dependency` or `failed_non_retryable`. Success/verified-empty carry
the complete canonical `RawNewsAggregationBatch` bytes/hash; dependency or
nonretryable failure require those fields null and retain the complete
individual seal/error or uncertainty bytes by the exact resolution hash. The
aggregate enforces the same total/resolved/suffix/stop equations and exact
variant joins as the terminal receipt. Any unavailable/cancelled/uncertain
feed closes the whole cycle as `pending_dependency`; any over-limit feed
closes it as `failed_non_retryable/feed_response_limit_exceeded`. No intent is
created for the uncontacted suffix and there is no partial fact admission from
other feeds in that cycle.

For a full normal plan, `verified_empty` is valid only when
`verified_empty_feed_count == total_feed_count` and
`total_response_record_count == 0`; otherwise at least one joined
`success_nonempty` seal is required and the aggregate is
`success_nonempty`. Pending, uncertain, cancelled and over-limit aggregates
must use the exact mapping table above. The terminal receipt recomputes these
same predicates from the joined resolution carriers and rejects any aggregate
or counter copied without recomputation.

Only `success_nonempty` or `verified_empty` may construct and commit the exact
`PreparedSourceIngress`; after its receipt reads back, the owner commits the
matching cycle terminal receipt. Dependency, cancellation, uncertainty,
transport error and over-limit paths never construct
`PreparedSourceIngress`; they commit their cycle terminal receipt directly
from the sealed errors. Thus an empty response remains verified data while a
failed response never becomes an empty or fabricated source batch.

After all required seals and exact closed BR-171 receipts exist, the owner
constructs `PreparedGeneration` only from their persisted canonical bytes. It
stores the **complete** PreparedGeneration canonical BLOB, its
domain-separated SHA-256, the ordered intent/seal hash list, calendar/notice
equality hashes, page snapshot/high-water and every closed confirmation
`confirmation_id,record_hash` as `PersistedPreparedGeneration`. Only the
private persistence owner may deserialize it, and it must
decode -> canonical-reserialize -> rehash before using the existing generation
commit choreography.

Startup recovery is deterministic. Every already sealed step is provider-free;
only a required ordered step for which no intent has ever been issued may make
a new provider call:

| Durable state | Mandatory recovery |
| --- | --- |
| new ingress scheduler cycle with no plan intent | persist/read back its `ingress_tick_plan` intent; derive the first registered feed intent; zero provider call until that child intent reads back |
| valid all-success/verified-empty feed-seal prefix with later registered feed never intended | validate/reuse the prefix, persist/read back only the next feed intent, perform only that next feed read and seal it |
| complete ordered success/verified-empty feed-seal set, or a stopped prefix whose final seal is typed non-success, with no aggregate seal | provider-free construct the exact `Sealed` resolution vector/cardinalities and compose/read back the exact `GenerationGlobalNewsBatchSeal`; a stopped prefix creates no suffix intent |
| success/verified-empty aggregate seal with no source-ingress stage/receipt | validate the full sealed `RawNewsAggregationBatch`, enforce every per-feed limit and commit/recover only its exact `PreparedSourceIngress`; zero provider call |
| dependency/error/cancel/uncertain/over-limit aggregate seal with no cycle terminal receipt | append/sync/read back the exact pending/nonretryable `GenerationIngressCycleTerminalReceipt`; never construct `PreparedSourceIngress`; zero provider call |
| source-ingress receipt with no cycle terminal receipt | verify its aggregate/feed/cadence join, then append/sync/read back the exact success/verified-empty cycle terminal receipt; zero provider call |
| page row but no intent | revalidate and create the first subject intent; zero inference about provider I/O |
| prior-boot receipted ingress-feed intent, no seal | never repeat that feed call; append/sync/read back its exact `GenerationAcquisitionUncertain`, place its intent/record hash in the final `Uncertain` resolution after the sealed prefix, prove there are no suffix intents, compose the pending-dependency aggregate with the frozen cardinalities, terminalize the entire cadence cycle, and create zero source facts |
| prior-boot receipted generation-subject intent, no seal | never repeat that intent's provider call; persist `GenerationAcquisitionUncertain`, close this attempt as `pending_dependency/acquisition_outcome_uncertain`, then allow only a new attempt ordinal on a later fair page |
| one or more seals form a valid proper prefix of the required ordered step set, with no later intent | validate and reuse every sealed byte/hash, deterministically derive the next required step from that prefix, persist/read back a new intent only for that missing step, perform only that step's I/O and seal it; never refetch a sealed step |
| complete required ordered intent/seal set, no persisted PreparedGeneration | validate the full no-gap set and every closed BR-171 receipt, run pure relation/validation/preparation and persist PreparedGeneration; zero provider call |
| persisted PreparedGeneration, no generation receipt | reload only DB bytes, canonical-reserialize/rehash, commit or recover existing manifest; zero provider call |
| generation receipt present, page row open | verify exact join and close that page row idempotently |
| missing/corrupt/conflicting bytes, hash, receipt or page binding | fatal `generation_state_ambiguous`; no provider call, no sample and no fabricated failure evidence |

The prefix is complete only when it contains every mandatory step and every
candidate-expanded substep from the closed plan in section 5.5.1 with
contiguous ordinals and no duplicate. A gap, a later intent after a gap or an
unexpected optional step is ambiguous, not a shorter complete plan.
`GenerationAcquisitionUncertain` is not a response seal and carries no
provider result. Its immutable carrier binds the intent, prior boot, detection
boot/time and closed reason; its hash is mandatory in the final `Uncertain`
resolution. For an ingress feed it permanently closes that cadence cycle
through the pending-dependency aggregate/terminal receipts; remaining feeds
have no intent and are not contacted. For a generation subject, a new read may
happen only under a distinct attempt ordinal and a new receipted intent after
the uncertain attempt and page row are closed. This separation makes “intent
persisted” and “response durably known” non-interchangeable.

### 5.6 Formal Selected read projection

```rust
// TO BE BUILT; no raw connection/path/limit/cursor parameters.
pub struct ReceiptedTerminalDecisionProof {
    preimage: TerminalProofPreimage,
    terminal_proof_hash: OpaqueSha256,
}
pub struct ReceiptedSelectedRowProof {
    preimage: SelectedRowProofPreimage,
    row_proof_hash: OpaqueSha256,
}
pub struct ReceiptedSelectedGenerationPage {
    preimage: SelectedPageProofPreimage,
    page_content_hash: OpaqueSha256,
    rows: Vec<ReceiptedSelectedRowProof>,
}
pub struct ReceiptedSelectedGenerationPager<'snapshot> {
    /* private pinned read transaction, activation/audit high-water and
       last emitted keyset tuple */
}

impl VerifiedSelectionReadModel {
    pub fn selected_generation_pager(
        &mut self,
    ) -> Result<ReceiptedSelectedGenerationPager<'_>, SelectionReadError>;
}

impl ReceiptedSelectedGenerationPager<'_> {
    pub fn next_page(
        &mut self,
    ) -> Result<Option<ReceiptedSelectedGenerationPage>, SelectionReadError>;
}
```

All proof JSON is I-JSON plus RFC-8785. Proof scalars are real validated
newtypes, never aliases to `String` or `u64`:

```rust
// TO BE BUILT. Each wrapper has a private field, derives Serialize only and
// implements Deserialize through an explicit Visitor/TryFrom validator.
#[serde(transparent)] struct CanonicalDate(String);
#[serde(transparent)] struct CanonicalUuidV7(String);
#[serde(transparent)] struct CanonicalNanosUtc(String);
#[serde(transparent)] struct NonBlankCanonicalId(String);
#[serde(transparent)] struct CanonicalStockCode(String);
#[serde(transparent)] struct LowerHexSha256(String);
#[serde(transparent)] struct CanonicalSafeU64(u64);
#[serde(transparent)] struct SchemaVersionOne(u8);
#[serde(transparent)] struct TerminalProofDomainV1(String);
#[serde(transparent)] struct SelectedRowProofDomainV1(String);
#[serde(transparent)] struct SelectedPageProofDomainV1(String);
```

The custom deserializers accept exactly:

| Newtype | Accepted JSON scalar/value |
| --- | --- |
| `CanonicalDate` | JSON string, byte-exact `YYYY-MM-DD`, valid proleptic-Gregorian date, reformat byte-equal |
| `CanonicalUuidV7` | JSON string, lowercase canonical hyphenated UUID, version exactly 7, parse/reformat byte-equal |
| `CanonicalNanosUtc` | JSON string, RFC3339 with exactly nine fractional digits and terminal `Z`, parse/reformat byte-equal |
| `NonBlankCanonicalId` | JSON string, nonempty, `value.trim()==value`, no NUL/control bytes and no normalization |
| `CanonicalStockCode` | JSON string accepted by the canonical A-share identity parser and re-rendered byte-equal |
| `LowerHexSha256` | JSON string of exactly 64 lowercase ASCII hexadecimal characters |
| `CanonicalSafeU64` | JSON unsigned integer only, `0..=9_007_199_254_740_991`; signed, negative, float, exponent, string and larger integer reject |
| `SchemaVersionOne` | JSON unsigned integer exactly `1`; every other numeric or scalar form rejects |
| `TerminalProofDomainV1` | JSON string exactly `stock_analysis.selection_v2_terminal_proof_preimage.v1` |
| `SelectedRowProofDomainV1` | JSON string exactly `stock_analysis.selection_v2_selected_row_proof_preimage.v1` |
| `SelectedPageProofDomainV1` | JSON string exactly `stock_analysis.selection_v2_selected_page_proof_preimage.v1` |

Every `Deserialize` implementation consumes the original JSON scalar directly;
it may not deserialize to `serde_json::Value`, stringify/coerce, clamp or
accept an alternate representation. `Serialize` emits the exact primitive
string/number held by the validated wrapper. Constructors are private and
repeat the same validation, so locally built and parsed proofs share one
contract.

```rust

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectedKeyPreimageV1 {
    evaluation_market_date: CanonicalDate,
    event_id: NonBlankCanonicalId,
    chain_id: NonBlankCanonicalId,
    canonical_stock_code: CanonicalStockCode,
    sample_key: NonBlankCanonicalId,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalProofPreimageV1 {
    domain: TerminalProofDomainV1,
    schema_version: SchemaVersionOne,
    evaluation_market_date: CanonicalDate,
    event_id: NonBlankCanonicalId,
    chain_id: NonBlankCanonicalId,
    canonical_stock_code: CanonicalStockCode,
    sample_key: NonBlankCanonicalId,
    source_fact_key: NonBlankCanonicalId,
    generation_stage_run_id: CanonicalUuidV7,
    generation_receipt_hash: LowerHexSha256,
    evaluation_attempt_content_hash: LowerHexSha256,
    sample_content_hash: LowerHexSha256,
    decision_kind: TerminalDecisionKindToken,
    ordered_rejection_codes: Vec<SelectionHardRejectionCode>,
    terminal_decision_hash: LowerHexSha256,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectedRowProofPreimageV1 {
    domain: SelectedRowProofDomainV1,
    schema_version: SchemaVersionOne,
    evaluation_market_date: CanonicalDate,
    event_id: NonBlankCanonicalId,
    chain_id: NonBlankCanonicalId,
    canonical_stock_code: CanonicalStockCode,
    sample_key: NonBlankCanonicalId,
    source_fact_key: NonBlankCanonicalId,
    source_fact_content_hash: LowerHexSha256,
    ingress_receipt_hash: LowerHexSha256,
    ingress_prepared_audit_hash: LowerHexSha256,
    ingress_committed_audit_hash: LowerHexSha256,
    generation_stage_run_id: CanonicalUuidV7,
    generation_receipt_hash: LowerHexSha256,
    generation_prepared_audit_hash: LowerHexSha256,
    generation_committed_audit_hash: LowerHexSha256,
    evaluation_attempt_content_hash: LowerHexSha256,
    sample_content_hash: LowerHexSha256,
    relation_evidence_set_hash: LowerHexSha256,
    market_evidence_seal_set_hash: LowerHexSha256,
    lifecycle_action_evidence_hash: LowerHexSha256,
    br171_confirmation_receipt_set_hash: LowerHexSha256,
    calendar_hash: LowerHexSha256,
    calendar_raw_notice_set_hash: LowerHexSha256,
    calendar_parser_equality_hash: LowerHexSha256,
    trading_date_vector_hash: LowerHexSha256,
    terminal_decision_hash: LowerHexSha256,
    terminal_proof_hash: LowerHexSha256,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectedPageProofPreimageV1 {
    domain: SelectedPageProofDomainV1,
    schema_version: SchemaVersionOne,
    snapshot_identity: LowerHexSha256,
    activation_run_id: CanonicalUuidV7,
    activation_receipt_hash: LowerHexSha256,
    database_receipt_high_water: CanonicalSafeU64,
    selection_audit_record_count: CanonicalSafeU64,
    selection_audit_tail_hash: LowerHexSha256,
    page_index: CanonicalSafeU64,
    // The field is always present. JSON null means first page; it is never
    // omitted and is otherwise the exact closed five-field object above.
    query_after_key_or_null: Option<SelectedKeyPreimageV1>,
    first_key: SelectedKeyPreimageV1,
    last_key: SelectedKeyPreimageV1,
    row_count: CanonicalSafeU64,
    ordered_row_proof_hashes: Vec<LowerHexSha256>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptedTerminalDecisionProofV1 {
    preimage: TerminalProofPreimageV1,
    terminal_proof_hash: LowerHexSha256,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptedSelectedRowProofV1 {
    preimage: SelectedRowProofPreimageV1,
    row_proof_hash: LowerHexSha256,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptedSelectedGenerationPageV1 {
    preimage: SelectedPageProofPreimageV1,
    page_content_hash: LowerHexSha256,
    rows: Vec<ReceiptedSelectedRowProofV1>,
}
```

`TerminalDecisionKindToken` serializes only the lowercase JSON strings
`"admitted"` or `"hard_rejected"`. Counts, page indexes and high-waters are
JSON numbers, never strings or floats, and must remain inside the exact
I-JSON integer range above. Every other scalar is a JSON string after its
validation type accepts it. No field is optional except the value of the
always-present `query_after_key_or_null`. `first_key` and `last_key` are
non-null because a returned page has `1..=200` rows; `row_count` equals both
`rows.len()` and `ordered_row_proof_hashes.len()`. The first/last keys equal
the first/last row's five key fields. RFC-8785 determines serialized object-key
order; the logical field order written below is the construction and golden
fixture order, not permission to bypass RFC-8785 lexical key ordering.

Every terminal generation row first produces one immutable
`ReceiptedTerminalDecisionProof`, regardless of whether its decision is
`Admitted` or `HardRejected`. `TerminalProofPreimage` is a closed RFC-8785
object whose exact logical field order is:

```text
domain,schema_version,
evaluation_market_date,event_id,chain_id,canonical_stock_code,sample_key,
source_fact_key,generation_stage_run_id,generation_receipt_hash,
evaluation_attempt_content_hash,sample_content_hash,decision_kind,
ordered_rejection_codes,terminal_decision_hash
```

Its `domain` is
`stock_analysis.selection_v2_terminal_proof_preimage.v1` and
`schema_version` is `1`. The hash output is deliberately absent from the
preimage:

```text
terminal_proof_hash =
  sha256("stock_analysis.selection_v2_terminal_proof.v1\0"
         || RFC-8785 canonical TerminalProofPreimage bytes)
```

The strict outer `ReceiptedTerminalDecisionProof` carrier has exactly
`preimage,terminal_proof_hash` in that logical order. It recomputes the hash
before construction and contains no alternative caller-supplied hash path. A
`HardRejected` terminal proof requires a non-empty closed rejection-code
vector and proves only terminal rejection; it is never a Selected proof. An
`Admitted` terminal proof requires an empty rejection vector and may be joined
into Selected only by the proof closure below.

The pager is the only formal production “Selected” interface. It inner-joins the
exact current activation receipt, ingress receipt, generation receipt,
Prepared/Committed audit pair, sample row and evaluation attempt, then filters
the persisted token `decision_kind='admitted'`. It additionally verifies
non-empty relation evidence, complete market/lifecycle/corporate-action/manual
confirmation evidence and terminal-decision hash closure.

The fixed page limit is `SELECTED_GENERATION_PAGE_LIMIT=200`; order is
`evaluation_market_date,event_id,chain_id,canonical_stock_code,sample_key`, all
ascending. The pager keeps a private keyset cursor equal to the last emitted
five-column tuple and queries only rows lexicographically greater than that
tuple. It never uses `OFFSET`. Repeated `next_page()` calls therefore reach
every Selected row even when the verified set exceeds 200; `Ok(None)` is the
only end marker. A page may not be returned empty.

The pager pins one SQLite read transaction, activation receipt identity,
database receipt high-water and validated selection-audit prefix for its
entire lifetime. The private cursor cannot be serialized, supplied or reset by
a caller. Snapshot/audit replacement, activation drift, non-strict next keys
or a row repeated across pages is an integrity error and requires a new pager
after the old snapshot is closed. No caller sort/filter/limit, raw row
constructor or connection is accepted in this slice.

The proof is split at the correct ownership boundary.
`SelectedPageProofPreimage` is a closed RFC-8785 object whose exact logical
field order is:

```text
domain,schema_version,
snapshot_identity,activation_run_id,activation_receipt_hash,
database_receipt_high_water,selection_audit_record_count,
selection_audit_tail_hash,page_index,query_after_key_or_null,
first_key,last_key,row_count,ordered_row_proof_hashes
```

Its `domain` is
`stock_analysis.selection_v2_selected_page_proof_preimage.v1` and
`schema_version` is `1`. Every `SelectedRowProofPreimage` is independently a
closed RFC-8785 object whose exact logical field order is:

```text
domain,schema_version,
evaluation_market_date,event_id,chain_id,canonical_stock_code,sample_key,
source_fact_key,source_fact_content_hash,ingress_receipt_hash,
ingress_prepared_audit_hash,ingress_committed_audit_hash,
generation_stage_run_id,generation_receipt_hash,
generation_prepared_audit_hash,generation_committed_audit_hash,
evaluation_attempt_content_hash,sample_content_hash,
relation_evidence_set_hash,market_evidence_seal_set_hash,
lifecycle_action_evidence_hash,br171_confirmation_receipt_set_hash,
calendar_hash,calendar_raw_notice_set_hash,calendar_parser_equality_hash,
trading_date_vector_hash,terminal_decision_hash,terminal_proof_hash
```

Its `domain` is
`stock_analysis.selection_v2_selected_row_proof_preimage.v1` and
`schema_version` is `1`. Hash outputs are absent from both preimages:

```text
row_proof_hash =
  sha256("stock_analysis.selection_v2_selected_row_proof.v1\0"
         || RFC-8785 canonical SelectedRowProofPreimage bytes)
page_content_hash =
  sha256("stock_analysis.selection_v2_selected_page_proof.v1\0"
         || RFC-8785 canonical SelectedPageProofPreimage bytes)
```

The strict outer `ReceiptedSelectedRowProof` carrier has exactly
`preimage,row_proof_hash`; the strict outer
`ReceiptedSelectedGenerationPage` carrier has exactly
`preimage,page_content_hash,rows`. The ordered row carriers in `rows` must
have hashes byte-equal to `preimage.ordered_row_proof_hashes` in the same
keyset order. All three preimages and all three outer carriers deny unknown or
missing fields. Their retained canonical bytes must equal a decode ->
RFC-8785 canonical-reserialize round trip; noncanonical object-key encoding,
reordered rejection codes/row-proof hashes/row carriers, a single mutated
field or a changed computed hash is an integrity error.

The reader recomputes each joined canonical preimage and its outer hash before
constructing a carrier. Page construction rejects the whole page when even one
row lacks or mismatches a proof; it never moves a row proof to a page-level
singular “representative receipt.” Thus a page containing rows from different
ingress/generation receipts remains exact. `HardRejected`,
`pending_dependency`, unreceipted and legacy rows cannot appear. A verified
pager with no rows returns `Ok(None)` and is distinct from unavailable or
integrity failure.

For every Selected row the reader recomputes the corresponding
`ReceiptedTerminalDecisionProof`, requires `decision_kind=admitted` and binds
its `terminal_proof_hash` into the row proof. Terminal proof counts/hashes and
Selected proof counts/hashes are separate fields in `GenerationTickReceipt`.
The same receipt separately binds the cadence receipt hash, ordered
`FeedAcquisitionResolution` objects and their seal/uncertainty record hashes,
global aggregate-seal hash and
`GenerationIngressCycleTerminalReceipt` hash; none may be inferred from a
source-ingress receipt or omitted on a verified-empty cycle.
No code path may satisfy a Selected-proof requirement with a HardRejected
terminal proof or report a terminal decision as Selected merely because it is
receipted.

The sole production consumer in this release is exactly
`src/bin/monitor/main.rs::selection_v2_generation_scheduler_loop`, through its
private call
`GenerationRuntimeOwner::verify_committed_selected_projection_for_tick`.
Immediately after each generation receipt, it drains the pager for that
tick's pinned snapshot, verifies the per-row proofs and uses only receipted
counts plus page/row-proof hashes in `GenerationTickReceipt` and the bounded
summary. It creates no sink, outcome, order or paper edge. No other production
module may call `selected_generation_pager`; the fixed-root Gate-D join helper
is an offline verifier, not a production consumer, and reaches the same
private proof builder only while holding its exclusive verification lease.

## 6. Offline migration and activation

Ordinary monitor startup is read-only with respect to global schema migration.
It never repairs or installs selection-v2. Production mutation remains owned
by the dedicated offline migration command.

### 6.1 Required fixed inputs

Before production apply, all of these must exist at fixed manifest-root paths
and pass strict canonical parsing:

```text
config/selection/provider_board_binding_proposal.v1.json
config/selection/provider_board_bindings.v1.json
config/selection/a_share_trading_calendar.v1.json
config/selection/a_share_trading_calendar_notices.v1.json
config/selection/a_share_trading_calendar_notices.v1/sse/<notice_id_sha256>.raw
config/selection/a_share_trading_calendar_notices.v1/szse/<notice_id_sha256>.raw
config/selection/selection_activation.v1.json
data/stock_analysis.db
data/audit/production/selection-audit.jsonl
```

The proposal is the human-reviewed PR input. The artifact is produced only by
the existing two-category live BoardDataGateway audit capture and must bind
every proposal entry one-to-one with complete provider evidence. The
activation file binds the computed exact config hash and prospective effective
window. `direct_only_unverified`, expired evidence, empty release evidence at
Gate D, unknown fields, noncanonical bytes, mismatched revisions/hashes or a
missing file cannot activate generation.

The canonical board artifact must expose `captured_at` and `valid_until`; the
activation must expose `effective_from` and `expires_at`; the calendar must
carry section 5.4.2's provider/version/coverage/source-notice evidence. These
complete RFC3339-nanosecond times, every raw-file content hash, notice manifest
hash, raw-notice-set hash, parser-equality hash, calendar hash and executable
revision hash are included in the activation manifest and receipt. Runtime
revalidation uses these persisted values and checked-in raw bytes, never file
mtime, the mutable process calendar or a newly parsed wall-clock default.

The migration command accepts no database path, audit root, provider endpoint,
mode, clock, config hash, schema identity or receipt hash. Test injection
exists only behind `cfg(test)` and TEST_CODE isolation.

### 6.2 Transaction, backup and atomic exchange owner

A new private `SelectionV2MigrationOwner` completes the currently blocked
production apply:

#### 6.2.1 Journal authority and canonical hash

The sole migration journal authority is the append-only, five-year-retained,
SHA-256-chained production selection audit at the fixed manifest-relative
`data/audit/production/selection-audit.jsonl`. A second JSONL wire format is
forbidden: the current `SelectionAuditRecord` strict parser is the only line
parser and the current outer
`stock_analysis.selection_audit_record.v1\0` hash chain remains authoritative.
A candidate database, backup filename, console line or SQLite `user_version`
is not a journal.

Gate B adds the following permanent variants to the closed
`SelectionAuditPhase` enum:

```text
v2_migration_planned
v2_migration_leaves_allocated
v2_migration_prepared
v2_migration_candidate_verified
v2_migration_committed
v2_migration_activation_receipted
v2_migration_aborted_pre_exchange
v2_migration_quarantined
v2_restore_planned
v2_restore_candidate_allocated
v2_restore_prepared
v2_restore_committed
v2_restore_closed
v2_restore_aborted_pre_exchange
v2_restore_quarantined
v2_generation_deactivation_prepared
v2_generation_deactivation_committed
v2_generation_deactivation_quarantined
v2_gate_d_join_verified
```

It also adds exactly one optional
`SelectionAuditContext.operation_evidence` field. The field uses
`#[serde(default, skip_serializing_if = "Option::is_none")]`; therefore
historical records that lack it deserialize and reserialize to the exact old
hash preimage. When present it is a strict, internally tagged closed enum:
`{"kind":"selection_migration_v1","payload":...}`,
`{"kind":"selection_restore_v1","payload":...}`,
`{"kind":"selection_deactivation_v1","payload":...}` or
`{"kind":"selection_gate_d_v1","payload":...}`. The deactivation
payload's exact order is `domain,schema_version,operation_kind,phase,
deactivation_id,activation_run_id,activation_receipt_hash,
approval_content_sha256,reason_code,approved_at,expires_at,
previous_phase_record_hash`. The Gate-D payload is the exact closed
`GateDJoinEvidencePayloadV1` frozen in section 9. Every context and payload
denies unknown fields.
Golden vectors must prove every historical phase still parses and retains its
byte-identical expected record hash before a new phase may be appended.

For every new line:

```text
SelectionAuditRecord.schema_version = 1
SelectionAuditRecord.domain = stock_analysis.selection_audit.v1
SelectionAuditRecord.phase = one closed phase above
SelectionAuditRecord.subject_id = canonical migration/restore/deactivation/verification UUIDv7
SelectionAuditRecord.content_hash =
  sha256("stock_analysis.selection_v2_operation_payload.v1\0"
         || RFC-8785 canonical operation payload bytes)
SelectionAuditRecord.context.operation_evidence = exact typed payload
SelectionAuditRecord.previous_hash = writer-owned global audit tail
SelectionAuditRecord.recorded_at = owner-issued UTC RFC3339 nanos
SelectionAuditRecord.record_hash =
  existing SelectionAuditRecord outer hash algorithm
```

For these operation lines the legacy context fields
`event_identity_hash,chain_identity_hash,security_identity_hash,provider,
provider_published_at,observed_at,magic_tdx_batch_id` are JSON null and
`rule_ids` is exactly `["BR-193","2.7"]`. Normal forward phases have
`reason_codes=[]` and `retryable=null`; aborted-pre-exchange phases have
`reason_codes=["operator_recovery_pre_exchange"]` and `retryable=false`;
quarantined phases have `reason_codes=["migration_state_ambiguous"]` or
`["restore_state_ambiguous"]` or `["deactivation_state_ambiguous"]` and
`retryable=false`; these are respectively the exact
`SelectionIntegrityCode` variants frozen in section 8.1 and are serialized
from that enum, not caller strings. Successful deactivation phases use
`reason_codes=["activation_revoked"]` and `retryable=false`. This null/vector
matrix is validated by phase and cannot be caller supplied. The
`v2_gate_d_join_verified` phase uses `reason_codes=[]`, `retryable=null`,
`subject_id=verification_run_id`, and exact
`operation_evidence.kind=selection_gate_d_v1`; it is appended only after the
final descriptor rewalk succeeds.

The migration payload has this exact canonical field order:

```text
domain, schema_version, operation_kind, phase, migration_run_id,
source_parent_device, source_parent_inode, source_name, source_device,
source_inode, source_link_count, source_size, source_sha256,
source_catalog_sha256,
source_checkpoint_kind, source_quiescence_evidence_sha256, backup_name,
backup_device, backup_inode, backup_link_count, backup_size, backup_sha256,
candidate_name, candidate_device, candidate_inode, candidate_link_count,
candidate_size, candidate_sha256,
target_catalog_sha256, proposal_content_sha256, artifact_content_sha256,
calendar_content_sha256, notice_manifest_content_sha256,
calendar_raw_notice_set_sha256, calendar_parser_equality_sha256,
activation_content_sha256, executable_sha256, activation_run_id,
activation_manifest_sha256, activation_receipt_hash, observed_layout_sha256,
previous_phase_record_hash
```

The restore payload has this independent exact field order:

```text
domain, schema_version, operation_kind, phase, restore_run_id,
source_parent_device, source_parent_inode, source_name, source_device,
source_inode, source_link_count, source_size, source_sha256,
source_catalog_sha256,
approval_content_sha256, approval_expires_at, backup_name, backup_device,
backup_inode, backup_link_count, backup_size, backup_sha256,
candidate_name, candidate_device, candidate_inode, candidate_link_count,
candidate_size, candidate_sha256,
retained_pre_restore_name, retained_pre_restore_device,
retained_pre_restore_inode, retained_pre_restore_link_count,
retained_pre_restore_sha256,
restored_catalog_sha256, observed_layout_sha256,
previous_phase_record_hash
```

Strings are trim-stable UTF-8, SHA fields are 64 lowercase hexadecimal,
identities and sizes are unsigned canonical decimal strings, names are single
descriptor-relative path components and IDs are canonical UUIDv7. Optional
fields are present as JSON `null` inside the operation payload, never omitted.
`previous_phase_record_hash` is null only for Planned and otherwise equals the
immediately preceding **outer** `SelectionAuditRecord.record_hash` for the same
run even when unrelated audit lines are interleaved. Duplicate byte-identical
phase append is idempotent; the same run/phase with different bytes, a broken
per-run phase link or a broken outer global link is fatal.

Planned is appended only **after** SQLite is quiesced/checkpointed and the
pinned source descriptor's complete size/hash/catalog plus checkpoint
evidence are known. It binds those immutable source/input identities and the
unique descriptor-relative backup/candidate names but has null file identities
for leaves that do not exist yet. `LeavesAllocated` requires both newly
created leaves' device/inode/link-count, `link_count=1`, `size=0` and the
SHA-256 of empty bytes. **Prepared must require the backup and candidate
device, inode, link-count, size and SHA-256.** At Prepared the candidate
bytes/hash equal the quiesced source bytes/hash; there is no null candidate
identity/hash.
CandidateVerified requires the changed candidate identity/hash, target
catalog and activation manifest hash. Committed repeats the exact
CandidateVerified values and records the swapped layout hash.
ActivationReceipted additionally requires the verified activation receipt
hash. `RestoreCandidateAllocated` requires the newly created candidate's exact
device/inode/link-count, `link_count=1`, size `0` and empty SHA-256;
`RestorePrepared` requires the same identity with bytes/hash exactly equal to
the approved backup. Fields established by an earlier phase may never change except
the backup/candidate size+hash transition from empty `LeavesAllocated` leaves
to the exact source copy at Prepared and the candidate size+hash transition
from Prepared source-copy to CandidateVerified target bytes.
Device/inode/name and link-count never change. Any other null/value or
transition matrix is fatal.

Every phase is appended through `LockedSelectionAuditSession`, followed by
read-back, `sync_data` and required parent `fsync`; no custom appender or
parallel migration journal exists.

#### 6.2.2 Apply sequence

The owner performs exactly:

1. Acquire the fixed global exclusive maintenance lock and fixed selection
   audit lock in registered order. Refuse a running monitor, unknown SQLite
   sidecar, symlink, hard-link alias, namespace replacement or unsupported
   filesystem.
2. Pin the production database/audit parent descriptors and identities.
   Classify the exact source catalog. Anything except a recognized
   pre-amendment catalog or exact target is rejected before mutation. Verify
   the platform/filesystem exact exchange primitive now, before any journal
   line or leaf exists.
3. Quiesce and checkpoint SQLite under the exclusive owner, close every
   mutable connection and prove no unmerged WAL/SHM/journal remains. Keep the
   exclusive maintenance authority for the remainder of the run. Through the
   already pinned source descriptor, compute and re-read the complete
   device/inode/link-count/size/SHA-256/catalog plus canonical checkpoint
   result; hash those facts as `source_quiescence_evidence_sha256`. A failure
   here is a pre-Planned, nonmutating rejection.
4. Allocate the run ID and unique single-component backup/candidate names and
   append/read-back/sync `V2MigrationPlanned` with the complete quiesced source
   identity/hash and null leaf identities. No run-owned leaf exists before
   Planned.
5. Create both leaves descriptor-relative with `O_CREAT|O_EXCL`, immediately
   verify device/inode/link-count=1/size=0/empty SHA-256, `fsync` leaves and
   parent, then append/read-back/sync `V2MigrationLeavesAllocated`. If a crash
   occurs after a create but before LeavesAllocated, recovery has no durable
   leaf identity and must retain/quarantine it under section 6.3; it may not
   infer ownership from the planned filename.
6. Populate the two already allocated, identity-pinned leaves with a full
   byte-preserving backup and candidate in the same pinned filesystem by
   copying from the already pinned source descriptor, never by reopening a
   pathname or creating/replacing a new leaf. `fsync` and hash both; the
   backup bytes/hash must equal the Planned quiesced source bytes/hash before
   the candidate is changed.
   Revalidate all fixed calendar/notice raw bytes and equality hashes.
7. Append/read-back/sync `V2MigrationPrepared` with complete source, backup
   and candidate device/inode/link-count/size/hash. The candidate must still
   equal the source. No candidate mutation begins unless this record is
   durable.
8. Apply the frozen schema plus config-activation envelope and manifest to the
   candidate under one `BEGIN IMMEDIATE` transaction with
   `foreign_keys=ON`, `synchronous=FULL`, exact catalog checks and
   row-preservation counts. The authoritative activation receipt is
   intentionally not written yet.
9. Close/checkpoint the candidate, run integrity/FK/catalog/trigger/index,
   config-envelope/manifest and row-preservation verification; `fsync` the
   candidate and parent. Append/read-back/sync
   `V2MigrationCandidateVerified` with the candidate inode/content hash and
   activation manifest hash.
10. Revalidate both directory descriptors, every inode/hash, audit tail and the
   CandidateVerified outer record. Atomically exchange the descriptor-relative
   source and candidate names on the same pinned directory descriptor:
   `renameat2(dirfd, source_name, dirfd, candidate_name, RENAME_EXCHANGE)` on
   audited Linux, or
   `renameatx_np(dirfd, source_name, dirfd, candidate_name, RENAME_SWAP)` on
   audited macOS. A platform/filesystem without this exact primitive fails
   before Planned; a pathname reopen or two-rename approximation is forbidden.
11. `fsync` the pinned parent; descriptor-relative reopen verifies that the
   production name now has the CandidateVerified inode/hash/catalog and that
   the candidate name now has the exact old source inode/hash equal to the
   backup. Append/read-back/sync `V2MigrationCommitted`. The exchange is the
   irreversible roll-forward boundary.
12. Through the exchanged production descriptor, commit the authoritative
   config-activation receipt that binds exact Prepared, CandidateVerified and
   Committed **outer** record hashes, then append/read-back/sync
   `V2MigrationActivationReceipted`. Reopen database and audit through their
   pinned authorities and verify the exact receipt/audit/catalog closure
   before releasing locks.

The owner allocates migration/run IDs and timestamps. Callers cannot supply
them. The backup remains retained under a versioned fixed backup directory;
this slice adds no cleanup command.

### 6.3 Migration/restore crash closure

Recovery is available only through the offline owner under the same exclusive
maintenance and audit locks. Every recognized run reaches one permanently
parseable terminal phase; removing a candidate without a terminal audit line
is forbidden:

- Planned only: the source must remain exact and match the complete quiesced
  identity/hash in Planned. If both planned leaf names are absent, append
  `V2MigrationAbortedPreExchange`. If either name exists, no durable leaf
  device/inode exists and ownership is unknowable: retain every observed
  leaf, append `V2MigrationQuarantined` with the observed layout hash and
  fail closed. **Never unlink, truncate or overwrite a Planned-only leaf by
  filename, run-name pattern or O_EXCL assumption.**
- LeavesAllocated only: verify each exact persisted device/inode/link-count
  through the pinned parent. Exact run-owned leaves may be retained or
  descriptor-relative unlinked only after their observed state is included in
  `V2MigrationAbortedPreExchange`; a missing/replaced/aliased identity appends
  `V2MigrationQuarantined` and retains all leaves.
- Prepared only: verify complete source/backup/candidate identities from the
  Prepared record. Freeze the candidate's final observed descriptor identity,
  link-count, size and hash into `V2MigrationAbortedPreExchange`; append,
  `sync_data`, parent-`fsync` and read back that terminal record **before**
  descriptor-relative unlink of the exact identity-equal candidate. Retain the
  backup. A crash after terminal read-back but before unlink re-enters the
  same idempotent cleanup using only the terminal record's identity.
- CandidateVerified: compare descriptor-relative source/candidate identities
  and hashes. For an exact pre-exchange layout, freeze the candidate's final
  observed identity/hash in `V2MigrationAbortedPreExchange`; append,
  `sync_data`, parent-`fsync` and read back that terminal record **before**
  descriptor-relative unlink of only the matched candidate. A crash after
  terminal read-back resumes that idempotent cleanup; no pre-terminal unlink
  is permitted.
  Exact swapped layout must roll forward through Committed and
  ActivationReceipted.
- Committed: verify exchanged target/old-source copy, then write/read back the
  exact activation receipt and append ActivationReceipted.
- ActivationReceipted: verify receipt, full catalog, coherent database/audit
  high-water and all phase hashes, then report idempotent closure.
- Any phase with an ambiguous identity/hash/layout/catalog that still leaves
  the outer audit append authority intact appends
  `V2MigrationQuarantined` with `observed_layout_sha256`, then fails closed.
  If the outer audit itself is invalid, no append is possible and the
  original chain error is the permanent evidence; no data file is mutated.

Restore uses the fixed approval described in section 11 and the same
descriptor/lock discipline. It creates a fresh backup-derived restore
candidate, then follows:

```text
V2RestorePlanned
  -> V2RestoreCandidateAllocated
  -> V2RestorePrepared
  -> V2RestoreAbortedPreExchange
  |  V2RestoreCommitted -> V2RestoreClosed
  |  V2RestoreQuarantined
```

`V2RestorePlanned` binds the complete approved backup/source identities and a
unique descriptor-relative candidate name, but requires the candidate identity
fields to be null and the candidate leaf to be absent. The owner then creates
the leaf descriptor-relative with `O_CREAT|O_EXCL`, verifies and fsyncs its
device/inode/link-count=1/size=0/empty hash plus parent, and appends/read-backs
`V2RestoreCandidateAllocated` before copying one byte. RestorePrepared requires
that same complete candidate identity/hash and proves its populated bytes equal
the approved retained backup. RestoreCommitted is appended only after
descriptor-relative atomic exchange and parent `fsync`; it retains the exact
displaced post-migration file. RestoreClosed verifies the restored catalog,
production `GenerationActive` is unavailable, the displaced file is retained,
and the complete outer/per-run audit chains read back.

Restore recovery is phase-exact:

- Planned only with no candidate leaf appends
  `V2RestoreAbortedPreExchange`. If the planned name exists, no durable leaf
  identity proves ownership: retain it, append `V2RestoreQuarantined` with the
  observed layout and fail closed. **Never delete a restore Planned-only leaf
  by name, run-name pattern or O_EXCL assumption.**
- CandidateAllocated verifies the persisted device/inode/link-count through the
  pinned parent. An exact run-owned leaf may be removed only after its state is
  bound into `V2RestoreAbortedPreExchange`; a missing, replaced or aliased
  identity is retained and quarantined.
- RestorePrepared may remove only the exact identity-equal candidate after
  binding its final observed state into `V2RestoreAbortedPreExchange`.
- Post-exchange RestoreCommitted always closes forward to RestoreClosed.
  Every other ambiguous layout appends RestoreQuarantined and mutates nothing
  further.

Audited generation deactivation is also crash closed. Prepared binds the exact
current activation receipt, approval bytes/hash and the current absence of a
prior matching deactivation receipt. Recovery under the same locks may only
roll forward: it persists/read-backs the exact deactivation receipt, appends
Committed and verifies `Disabled(ActivationRevoked)`. A crash after receipt
commit but before Committed reuses that same receipt; it never allocates a new
deactivation ID. A conflicting receipt, activation replacement or ambiguous
approval binding appends `V2GenerationDeactivationQuarantined` when the outer
audit authority remains valid and mutates nothing further.

Aborted, ActivationReceipted, RestoreClosed, DeactivationCommitted and every
Quarantined phase are terminal for their run; later work requires a new
UUIDv7 run linked only through the outer global audit chain. Monitor treats
every nonterminal or quarantined recognized run as a fatal integrity error,
not `SchemaNotAmended`.

## 7. Runtime data flow

```text
global NewsAggregator real provider batch
  -> GenerationRuntimeOwner
       -> receipted ingress acquisition intent
       -> sealed exact news response/error evidence
  -> PreparedSourceIngress
  -> SelectionV2PersistenceOwner ingress receipt
  -> durable fair-keyset generation page + snapshot/high-water
  -> SelectionRelationOwner
       -> direct mention against TDX master
       -> activated chain keyword
       -> reviewed board binding
       -> BoardDataGateway complete constituents
       -> each external step intent -> I/O -> evidence seal
  -> CanonicalCandidateBatch
  -> Magic TDX market evidence gateway
       -> security lifecycle evidence
       -> corporate-action interval evidence
       -> BR-171 exact closed receipt ID+record-hash join
  -> receipted official-calendar owner
       -> immutable raw notices/provider/version/parser equality hashes
       -> exact T0,D1,D2,D3,D4,D5 vector/hash
  -> freshness / continuity / bad-data validation
  -> frozen feature + admission versions
  -> persisted full PreparedGeneration bytes/hash
  -> generation receipt
  -> per-row ReceiptedTerminalDecisionProof for Admitted or HardRejected
  -> Admitted-only ReceiptedSelectedRowProof projection
  -> page safe-state + fair round-cursor/round-closing checkpoint CAS
```

There is no sink, outcome, order or paper-trade edge in this graph.

### 7.1 Exact production scheduler

The only production caller is a TO-BE-BUILT
`src/bin/monitor/main.rs::selection_v2_generation_scheduler_loop`, spawned once
by `main` after successful `GenerationActive` binding and before the ordinary
monitor task join. It owns the `GenerationRuntimeOwner`; no other task may
clone or call it.

The loop has an immediate recovery-only wake-up, then a
`tokio::time::interval(Duration::from_secs(NEWS_FETCH_PERIOD_SECS))` with
`MissedTickBehavior::Skip`. Only after recovery returns a verified fixed point
may a wake-up request a new cadence slot. The owner first reads the latest
`GenerationAcquisitionCadenceReceipt`; it waits until the stored
`next_acquisition_eligible_at`, then CAS-commits/read-backs the new receipt
before calling the zero-argument
`GenerationRuntimeOwner::acquire_and_process_news_tick()`. Inside that method,
the owner reads back the receipted `ingress_tick_plan`, then drives the
NewsAggregator's TO-BE-BUILT sealed-feed interface. That interface exposes the
immutable registered feed descriptors in registration order but performs one
feed read only after the owner supplies its read-back `ingress_feed` intent;
it returns that one complete feed result for immediate sealing before the next
descriptor can be requested. It accepts `NEWS_PER_FEED_LIMIT=20` only. The
provider-free aggregate seal is constructed only after the full sequence
closes. The scheduler never receives, splits, clones, refetches or
independently projects any batch. Before successful ingress, the owner
validates every registered feed vector has at most 20 records; an over-limit
vector is sealed as `feed_response_limit_exceeded`, terminalizes the cadence
cycle and produces no fact.

Single-flight is structural: the loop awaits the current tick future inline
and has no per-tick `spawn`. A second loop installation is rejected by the
process bootstrap owner. A tick that is still running when the interval fires
causes the missed tick to be skipped; no queue is accumulated.

The loop receives the monitor's process cancellation token. Cancellation:

1. stops new acquisition immediately;
2. lets an already durable envelope finish recovery/commit without another
   provider request;
3. if provider I/O is in flight, awaits/cancels only through the gateway's
   typed cancellation boundary and persists the actual available evidence as
   `pending_dependency/provider_cancelled`;
4. awaits the scheduler join before database/audit shutdown.

An unexpected scheduler return, panic, recovery failure, receipt read-back
failure or integrity error is supervised by `main` and terminates monitor with
nonzero status. It is never a detached log-only task.

At startup and before each new cadence receipt, the owner assigns every
incomplete item to the first matching class below and drains the lowest class
to exhaustion. After every closure it restarts enumeration at class 1, because
that closure can expose an earlier class:

| Rank | Recovery class | Complete ascending tuple; final field is unique tie-break |
| --- | --- | --- |
| 1 | manifested-unreceipted non-outcome stage | `(stage_kind_rank,manifest_committed_at,stage_run_id)` |
| 2 | envelope-only non-outcome stage | `(stage_kind_rank,envelope_created_at,stage_run_id)` |
| 3 | complete ingress feed-resolution prefix/aggregate/source-ingress closure without cycle terminal receipt | `(cadence_committed_at,scheduler_cycle_id,ingress_recovery_phase_rank,final_feed_ordinal_or_zero,final_resolution_or_receipt_identity)` |
| 4 | prior-boot ingress-feed intent without seal | `(cadence_committed_at,scheduler_cycle_id,feed_ordinal,intent_id)` |
| 5 | open ingress cadence cycle lacking the next plan/feed intent | `(cadence_committed_at,scheduler_cycle_id,next_feed_ordinal_or_zero,cadence_receipt_id)` |
| 6 | generation receipt with open page row | `(page_committed_at,page_run_id,row_ordinal,logical_subject_key,page_row_id)` |
| 7 | persisted PreparedGeneration without generation stage/receipt | `(page_committed_at,page_run_id,row_ordinal,logical_subject_key,attempt_ordinal,prepared_id)` |
| 8 | complete seal set without PersistedPreparedGeneration | `(page_committed_at,page_run_id,row_ordinal,logical_subject_key,attempt_ordinal,final_step_ordinal,final_seal_id)` |
| 9 | valid sealed prefix with later required steps never intended | `(page_committed_at,page_run_id,row_ordinal,logical_subject_key,attempt_ordinal,next_step_ordinal,last_seal_id)` |
| 10 | prior-boot generation intent without seal | `(page_committed_at,page_run_id,row_ordinal,logical_subject_key,attempt_ordinal,step_ordinal,intent_id)` |
| 11 | open page row without an intent | `(page_committed_at,page_run_id,row_ordinal,logical_subject_key,page_row_id)` |
| 12 | open fixed-high-water fairness round without a page | `(round_committed_at,round_phase_rank,round_cursor_or_null,fairness_round_id)` |

`stage_kind_rank` is exactly `source_ingress=0,generation=1`;
`ingress_recovery_phase_rank` is exactly
`aggregate_seal_missing=0,source_ingress_missing=1,cycle_terminal_missing=2`;
`round_phase_rank` is exactly `above_checkpoint=0,wrapped_at_or_below=1`.
All timestamps are canonical receipt timestamps, `row_ordinal` is unique within
its page, `page_row_id` is a persisted canonical UUIDv7 bound into the page
receipt and the final field of every tuple is an immutable unique ID.
`round_cursor_or_null` is null only for a new phase and is ordered before every
non-null key. Two byte-different items with the same complete tuple, an unknown
class/stage/phase or a tuple containing any other unexpected null is fatal
`generation_state_ambiguous`; no fallback tie, filesystem order or hash-map
iteration is allowed. Classes 1–4, 6–8, 10 and 12
are provider-free. Class 3 composes/validates only closed `Sealed`/`Uncertain`
resolution bytes, commits `PreparedSourceIngress` only for a full
success/verified-empty resolution vector and always finishes the exact
cycle-terminal receipt; typed failure/uncertainty bytes never enter source
ingress. Class 4 closes the exact feed intent uncertain, freezes it as the last
resolution, proves the suffix has no intent and terminalizes the cadence cycle
without another feed call; class 10 closes the exact generation-subject intent
uncertain. Class 9 reuses the generation sealed prefix and calls only the next
never-intended step; class 5 may create and execute only the next registered
feed intent, while class 11 may create and execute only its first subject
intent. Class 12
resumes the stored round high-water/phase/cursor. A sealed response is
never refetched; a prior-boot intent without a seal is closed uncertain and the
same intent is never reissued. New acquisition is forbidden until this exact
queue reaches a verified fixed point.

## 8. Failure matrix

### 8.1 Closed typed taxonomy

Gate B must expose exhaustive enums whose serialized tokens are exactly the
following closed sets. Production code may attach redacted detail/hash, but
may not create caller strings or map an unknown variant to a listed token.

```text
SelectionDisabledReason:
  schema_not_amended
  proposal_missing
  board_artifact_unverified
  board_artifact_expired
  activation_missing
  activation_not_effective
  activation_expired
  activation_unreceipted
  activation_revoked
  trading_calendar_missing
  trading_calendar_unverified
  trading_calendar_coverage_incomplete
  ingress_contract_unavailable

OutcomeDisabledReason:
  outcome_activation_not_released

IngressCycleTerminalKind:
  source_ingress_committed
  verified_empty
  pending_dependency
  failed_non_retryable

GenerationAggregateOutcomeKind:
  success_nonempty
  verified_empty
  pending_dependency
  failed_non_retryable

FeedAcquisitionOutcomeKind:
  success_nonempty
  verified_empty
  transport_failure
  provider_cancelled
  feed_response_limit_exceeded

SelectionIntegrityCode:
  mode_authority_mismatch
  lease_order_violation
  production_test_identity
  test_real_identity
  namespace_identity_conflict
  catalog_drift
  audit_chain_invalid
  receipt_conflict
  migration_state_ambiguous
  restore_state_ambiguous
  deactivation_state_ambiguous
  config_snapshot_conflict
  subject_high_water_conflict
  page_snapshot_conflict
  generation_state_ambiguous
  canonical_hash_mismatch
  evidence_projection_mismatch
  provider_identity_conflict
  calendar_release_integrity_conflict
  calendar_parser_or_session_conflict
  br171_receipt_closure_conflict
  migration_unowned_leaf

SelectionPendingDependencyCode:
  feed_unavailable
  board_unavailable
  board_partial
  security_master_unavailable
  daily_bars_unavailable
  quote_unavailable
  five_minute_bars_unavailable
  security_lifecycle_unavailable
  corporate_actions_unavailable
  manual_confirmation_required
  daily_feature_history_insufficient
  volume_baseline_missing
  intraday_volume_baseline_missing
  trading_calendar_coverage_incomplete
  quote_stale
  daily_stale
  provider_cancelled
  acquisition_outcome_uncertain
  activation_expired_after_acquisition
  board_artifact_expired_after_acquisition

SelectionFailedNonRetryableCode:
  feed_response_limit_exceeded
  source_time_missing
  source_stale
  source_future
  unsupported_security
  security_code_empty
  mixed_security_batch
  duplicate_bar
  bar_out_of_order
  bar_gap
  bar_non_trading_day
  bar_not_settled
  adjustment_not_unadjusted
  split_continuity_unverified
  ohlc_inconsistent
  price_non_positive
  volume_nonfinite
  amount_nonfinite
  volume_negative
  amount_negative
  daily_future
  intraday_volume_invalid
  prospective_window_closed

SelectionHardRejectionCode:
  trend_alignment_failed
  price_below_ma5
  price_ma20_distance_out_of_range
  five_day_return_out_of_range
  settled_volume_confirmation_failed
  intraday_volume_confirmation_failed
```

`HardRejected` is permitted only after complete valid evidence and only for
these admission-v1 predicates: `MA5>=MA10>=MA20`, price `>=MA5`, price relative
to MA20 in `[0,15%]`, five-day return in `[0,20%]`, five-day and twenty-day
volume ratios each `>=1.0`, and intraday same-slot volume ratio `>=1.0` when
the window is Intraday. Multiple failures use the same closed codes in
predicate order. Missing/nonfinite inputs are dependency/data failures, not
strategy rejections. A `>20%` adjacent close change is not a strategy failure:
without its exact BR-171 receipt it is pending dependency; with the receipt it
continues and may be Admitted or HardRejected solely by the listed predicates.
Integrity codes abort the tick/process and must never be persisted as
`failed_non_retryable`.
The three operation-quarantine phases map one-to-one to the closed integrity
tokens `migration_state_ambiguous`, `restore_state_ambiguous` and
`deactivation_state_ambiguous`; the same token is placed in the audit
`reason_codes` singleton and no operation-specific caller string exists.

| Condition | Result | Provider/write behavior |
| --- | --- | --- |
| proposal/artifact/activation absent | Disabled typed reason | zero selection provider, zero selection DB operation after bind, zero scheduler |
| all fixed calendar release inputs wholly absent, no proposal/activation/receipt calendar claim, and exactly one valid reviewed prerequisite marker identifies missing, unverified or coverage-incomplete state | Disabled `TradingCalendarMissing`, `TradingCalendarUnverified` or `TradingCalendarCoverageIncomplete`, exactly matching that marker | zero selection provider, zero scheduler |
| the same wholly-unclaimed absence has no valid reviewed prerequisite marker, more than one marker or a conflicting marker | fatal `calendar_release_integrity_conflict` | zero selection provider, zero scheduler |
| any calendar/manifest/raw authority is present but the set is partial; referenced raw leaf absent; path/file/descriptor identity conflicts; raw bytes/hash, canonical parse, parser-descriptor identity, publication or claimed coverage conflicts | fatal `calendar_release_integrity_conflict` before or after activation | zero new provider work; no local date inference and never Disabled |
| the complete identity-valid authority set reaches parser ambiguity/disagreement, parser-output mismatch, exchange session-vector conflict or T0..D5 mismatch | fatal `calendar_parser_or_session_conflict` before or after activation | zero new provider work; no local date inference and never Disabled |
| exact target schema absent, no partial evidence | `SchemaNotAmended` Disabled | zero selection provider |
| partial schema/migration or conflicting receipt | fatal integrity error | zero new provider work |
| TEST_CODE identity in production or real identity in test | fatal isolation error | zero provider/write; namespace remains unchanged |
| provider feed unavailable | receipted ingress unavailable attempt / generation `pending_dependency` when a receipted fact already exists | no fabricated facts |
| one registered feed returns more than 20 records | seal full response as `feed_response_limit_exceeded`; `failed_non_retryable` cycle terminal receipt | zero source facts and zero `PreparedSourceIngress`; no truncation/sample/default |
| all registered feeds verified empty | receipted verified-empty ingress | no generation candidates |
| source published time missing/stale/future | typed ingress rejection | no relation/market call |
| chain match but configured binding unavailable/partial | `pending_dependency` with closed reason | no direct-only downgrade |
| chain board relation explicitly `not_configured` | relation branch is complete with no board members; exact DirectMention may still proceed | no board call; market call only for a real direct candidate |
| complete board response has zero members | verified no relation if no direct mention | zero market calls for absent candidates |
| board/master/market partial response | `pending_dependency` typed attempt | no partial generation success |
| quote source time missing where realtime is required | typed market rejection | no feature calculation |
| daily gap/duplicate/nonpositive/uncleared corporate action | typed invalid/pending result from closed taxonomy | no admission |
| adjacent absolute close change >20% without exact BR-171 receipt | `pending_dependency/manual_confirmation_required` | no admission |
| adjacent absolute close change >20% with exact BR-171 receipt | continue ordinary quality/features/admission | never reject solely for magnitude |
| admission rejects valid evidenced candidate | `HardRejected` plus rejection rows | excluded from Selected view |
| subject lock busy | skip this subject for tick | no provider call by loser |
| prior-boot acquisition intent has no evidence seal | receipted `pending_dependency/acquisition_outcome_uncertain` for that attempt | never reissue same intent; later attempt requires new ordinal/intent |
| complete required evidence-seal set or persisted PreparedGeneration exists | provider-free recovery from exact stored bytes | zero repeated provider call |
| valid evidence-seal prefix lacks a later never-intended required step | reuse prefix, receipt a new intent for only the next fixed-order step | zero repeated call for every sealed step |
| prospective date closed | receipted `prospective_window_closed` | zero provider call |
| activation/artifact expires after acquisition | receipted `pending_dependency` with actual evidence, then capability Disabled | no sample/new acquisition |
| outcome capability requested | stable `outcome_activation_not_released` | zero outcome provider/scheduler |

Warnings must be aggregated by batch/reason. One line per stale event or per
candidate is forbidden when a bounded summary can carry counts and hashes.
Raw news content, account data and full stock lists are not written to startup
logs.

## 9. Observability and production evidence

Active generation emits one bounded summary per tick:

```text
[selection-v2][BR-193] producer=global_news activation_run_id=<canonical-uuidv7> activation_receipt_hash=<64-lower-hex> ingress_receipt_hash=<64-lower-hex> pending=<n> pending_dependency=<n> relation_candidates=<n> admitted=<n> hard_rejected=<n> generation_receipts=<n> provider=tdx source=<capability> source_time=<provider-time-or-market-date-or-null> observed_at=<time> batch_id=<hash>
```

`activation_run_hash` is forbidden because it hides the join key. Logs and
audit summaries carry the canonical activation run ID and its independently
verified receipt hash as separate fields.

If components have different source identities, the summary contains a
component-count map and audit references rather than claiming one source for
all. The detailed immutable evidence lives in the database/audit, not logs.

Gate D production evidence must join:

1. exact current config activation receipt;
2. one real global-news cadence receipt, pre-I/O intent,
   response-evidence seal and ingress receipt, including its exact per-feed-20
   request, response record-count validation and full provider response;
3. its source provider/published/observed/batch evidence;
4. one receipted fair generation page with its exact key list, snapshot
   identity, fixed round database receipt high-water, validated audit prefix,
   round ID/phase/cursor, checkpoint before/after and wrap bit;
5. one generation subject's complete ordered pre-I/O intents, independent
   response/error seals and persisted-Prepared canonical bytes/hash;
6. one generation receipt;
7. relation evidence (direct or activated board constituent);
8. Magic TDX component provider time/date, observed time and batch hashes;
9. every required closed BR-171 confirmation ID plus immutable record hash;
10. fixed raw calendar-notice paths/hashes, the closed canonical
    `NoticeManifestPayload`, and parser/session/T0..D5 equality hashes;
11. one terminal `Admitted` or `HardRejected` decision and that row's complete
    `TerminalProofPreimage` bytes, outer hash and
    `ReceiptedTerminalDecisionProof`;
12. independently, at least one `Admitted` decision and its complete
    `SelectedRowProofPreimage`/outer hash and
    `ReceiptedSelectedRowProof`, bound to the same terminal proof and its
    page's `SelectedPageProofPreimage`/outer hash/snapshot/high-water;
13. exact Prepared/Committed audit hashes and closed page/round/checkpoint CAS.

The Gate-D helper compiles this fixed authority manifest; it is not loaded
from JSON, argv, environment, CWD or the database:

```text
manifest_root=env!("CARGO_MANIFEST_DIR")
database_path=data/stock_analysis.db
selection_audit_path=data/audit/production/selection-audit.jsonl
calendar_manifest_path=config/selection/a_share_trading_calendar.v1.json
notice_manifest_path=config/selection/a_share_trading_calendar_notices.v1.json
raw_notice_root=config/selection/a_share_trading_calendar_notices.v1/
raw_notice_providers=sse,szse
calendar_domain=stock_analysis.a_share_trading_calendar.v1
notice_manifest_domain=stock_analysis.a_share_calendar_notice_manifest.v1
raw_notice_set_domain=stock_analysis.a_share_calendar_raw_notice_set.v1
parser_equality_domain=stock_analysis.a_share_calendar_parser_equality.v1
```

After taking the exclusive maintenance lease, the helper walks every path
component from a pinned manifest-root descriptor with no-follow opens. It
requires the two manifest leaves to be regular single-link files and the raw
root/provider leaves to be directories; records device, inode, type,
link-count, size and change-time; rejects aliases and any raw-root entry not
listed by the manifest; reads from those pinned descriptors; and performs a
second descriptor-relative traversal plus `fstat` identity check before
release. It re-hashes the exact calendar, notice-manifest and every raw notice
byte sequence; strict-decodes each closed object; requires stored bytes to
equal RFC-8785 canonical reserialization; reruns the selected deterministic
parsers; and recomputes notice-manifest, raw-notice-set, parser-equality,
session and every T0..D5 identity. Database values must exact-join those
recomputed values. Merely trusting activation-row hashes is forbidden.

This official revalidation is the sole exception to the section 3
runtime-provider/outside-audit invariant. The exception is closed to the
release-only `selection_v2_verify_join` module, the two fixed manifest
providers and their already-pinned canonical URLs. It performs exactly one
sequential request per manifest entry, follows no redirect, retries zero times
while the audit session is held and applies the compile-time
`GATE_D_OFFICIAL_HTTP_TIMEOUT_SECS=30` per request. It constructs no ordinary
selection provider capability, runtime scheduler, sink, order, paper engine or
mutable SQLite connection.

Before the first official request the database snapshot and audit prefix are
read-only. Timeout, cancellation, transport failure, non-200 status,
redirect/host drift, parse failure, identity/publication mismatch, response
byte mismatch or any later descriptor drift aborts the SQLite snapshot and
consumes the retained audit session through typed
`abort_without_append()`. That failure path verifies the audit tail is still
the captured prefix, releases the locks, appends no
`v2_gate_d_join_verified`, emits no success JSON and performs no in-session
retry. A later retry is a new offline verifier invocation that reacquires and
revalidates every authority from the beginning. `abort_without_append()` is
unavailable after a terminal append and cannot satisfy the success-path
`finish()` requirement.

While the same pinned inputs and lease are retained, Gate D performs a fresh
real read of every manifest `canonical_url`. It accepts only the HTTPS
canonical URL already stored for the matching `sse` or `szse` entry, follows
no cross-host redirect, supplies no caller credentials and records the actual
HTTP observation time/status. The decoded response body must be byte-identical
to the pinned raw leaf, the official notice identity/publication time obtained
by the provider-specific parser must equal the manifest, and a second fetch or
local cache must not silently substitute evidence. Transport failure,
redirect/host drift, unavailable official publication identity, body mismatch
or publication mismatch fails Gate D explicitly; it is not a Disabled result
and no “last known good” local-only validation may claim live revalidation.

Every official read becomes a closed
`GateDOfficialRevalidationEntryV1`, in manifest order, with exact field order:

```text
provider,notice_id_sha256,canonical_url_sha256,observed_at,http_status,
response_sha256,parsed_notice_id_sha256,parsed_published_at
```

`provider` is exactly `sse` or `szse`; both identity hashes and the URL hash
are lowercase SHA-256, `observed_at` and `parsed_published_at` are canonical
RFC3339 nanosecond UTC, and `http_status` is the actual I-JSON-safe integer
status and must be exactly `200`. The entry and its containing
`GateDOfficialRevalidationEvidenceV1` deny unknown/missing fields. The
container's exact order is `domain,schema_version,entries`, its domain is
`stock_analysis.selection_v2_gate_d_official_revalidation.v1`, and
`official_revalidation_evidence_hash` is SHA-256 over that domain, NUL and its
RFC-8785 bytes. Raw URLs and response bodies are never printed.

The terminal and admitted proof samples are deterministic, not
operator-selected. The helper chooses the first real terminal subject by the
closed terminal key order and independently the first admitted Selected row by
the section 5.6 keyset order. Their identity hashes use respectively
`stock_analysis.selection_v2_gate_d_terminal_subject.v1\0` and
`stock_analysis.selection_v2_gate_d_admitted_subject.v1\0` plus the canonical
logical-subject bytes. It then recomputes the exact terminal/row/page proofs;
a HardRejected terminal can satisfy only the first sample.

The helper opens exactly one `LockedSelectionAuditSession` after taking the
registered audit lock and uses that same non-Clone session to validate and pin
the selection-audit prefix. It must not call `finish()`, release/reacquire the
audit lock or create a second session while the database snapshot, official
revalidation and final descriptor-relative identity rewalk are in progress.
After those operations succeed, while the original session and prefix remain
live, the helper creates the closed `GateDJoinEvidencePreimageV1` with exact
field order:

```text
domain,schema_version,verification_run_id,verification_started_at,
verification_completed_at,activation_run_id,activation_receipt_hash,
database_receipt_high_water,selection_audit_prefix_record_count,
selection_audit_prefix_tail_hash,calendar_hash,
calendar_artifact_content_hash,notice_manifest_content_hash,
calendar_raw_notice_set_hash,calendar_parser_equality_hash,
calendar_descriptor_attestation_hash,
official_revalidation_evidence_hash,
ingress_cycle_terminal_receipt_hash,terminal_subject_identity_hash,
terminal_proof_hash,admitted_subject_identity_hash,
admitted_terminal_proof_hash,selected_row_proof_hash,
selected_page_content_hash,selected_page_snapshot_identity
```

Its domain is `stock_analysis.selection_v2_gate_d_join_evidence.v1`.
`gate_d_evidence_hash` is SHA-256 over that domain, NUL and the RFC-8785
preimage bytes and is absent from its own preimage. The strict
`GateDJoinEvidencePayloadV1` has exact order
`preimage,gate_d_evidence_hash`. While still holding the exclusive maintenance
lease and the same `LockedSelectionAuditSession`, the helper appends a
`v2_gate_d_join_verified` `SelectionAuditRecord` containing
`operation_evidence.kind=selection_gate_d_v1`, calls `sync_data`, fsyncs the
audit parent, and reads back and revalidates that exact record. It then calls
`finish()` exactly once on that same session and requires the returned
validation receipt's record count/tail to equal the appended record. Only
after that single successful `finish()` may it emit output. Its
`previous_hash` is the captured
`selection_audit_prefix_tail_hash`; the final output record count is prefix
count plus one and the final tail is this record's hash. Failure to
retain the original session, append, sync, parent-fsync, read back, or finish
produces no success output. Calling `finish()` before the terminal append,
reopening an audit session for the append or appending against a reloaded tail
is a Gate-D integrity failure.

The release helper emits exactly one closed JSON object with these fields and
no raw content:

```text
domain="stock_analysis.selection_v2_gate_d_join.v1"
schema_version=1
verification_run_id=<canonical-uuidv7>
verification_started_at=<canonical-rfc3339-nanos-utc>
verification_completed_at=<canonical-rfc3339-nanos-utc>
activation_run_id=<canonical-uuidv7>
activation_receipt_hash=<64-lower-hex>
writer_freeze="exclusive_lease"
database_path="data/stock_analysis.db"
selection_audit_path="data/audit/production/selection-audit.jsonl"
database_receipt_high_water=<I-JSON-safe integer>
selection_audit_prefix_record_count=<I-JSON-safe integer>
selection_audit_prefix_tail_hash=<64-lower-hex>
selection_audit_record_count=<I-JSON-safe integer>
selection_audit_tail_hash=<64-lower-hex>
calendar_manifest_path="config/selection/a_share_trading_calendar.v1.json"
notice_manifest_path="config/selection/a_share_trading_calendar_notices.v1.json"
raw_notice_root="config/selection/a_share_trading_calendar_notices.v1/"
calendar_hash=<64-lower-hex>
calendar_artifact_content_hash=<64-lower-hex>
notice_manifest_content_hash=<64-lower-hex>
calendar_raw_notice_set_hash=<64-lower-hex>
calendar_parser_equality_hash=<64-lower-hex>
calendar_descriptor_attestation_hash=<64-lower-hex>
calendar_manifest_descriptor_attested=1
notice_manifest_descriptor_attested=1
raw_notice_root_descriptor_attested=1
raw_notice_leaf_count=<safe JSON integer >=2>
raw_notice_descriptor_attested_count=<same integer>
calendar_manifest_canonical=1
notice_manifest_canonical=1
raw_notice_set_canonical=1
calendar_raw_notice_hash_mismatches=0
calendar_notice_parser_equality=1
calendar_session_vector_mismatches=0
calendar_t0_d5_vector_mismatches=0
calendar_official_url_revalidated_count=<same integer>
calendar_official_http_success_count=<same integer>
calendar_official_notice_identity_mismatches=0
calendar_official_publication_mismatches=0
calendar_official_raw_byte_mismatches=0
official_revalidation_entries=<closed ordered GateDOfficialRevalidationEntryV1 array>
official_revalidation_evidence_hash=<64-lower-hex>
activation_receipts=1
ingress_receipts=<safe JSON integer >=1>
ingress_intents=<safe JSON integer >=1>
response_evidence_seals=<safe JSON integer >=1>
ingress_cycle_terminal_receipts=<safe JSON integer >=1>
ingress_cycle_terminal_receipt_hash=<64-lower-hex>
fair_generation_pages=<safe JSON integer >=1>
persisted_prepared_generations=<safe JSON integer >=1>
generation_receipts=<safe JSON integer >=1>
terminal_samples=<safe JSON integer >=1>
terminal_decision_proofs=<safe JSON integer >=1>
admitted_samples=<safe JSON integer >=1>
admitted_terminal_proofs=<safe JSON integer >=1>
selected_row_proofs=<safe JSON integer >=1>
invalid_selected_rows=0
unreceipted_selected_rows=0
coherent_db_receipt_high_water=1
coherent_audit_prefix=1
terminal_proof_preimage_mismatches=0
selected_row_proof_preimage_mismatches=0
selected_page_proof_preimage_mismatches=0
br171_closed_receipt_mismatches=0
terminal_subject_identity_hash=<64-lower-hex>
terminal_proof_hash=<64-lower-hex>
admitted_subject_identity_hash=<64-lower-hex>
admitted_terminal_proof_hash=<64-lower-hex>
selected_row_proof_hash=<64-lower-hex>
selected_page_content_hash=<64-lower-hex>
selected_page_snapshot_identity=<64-lower-hex>
gate_d_evidence_preimage=<exact closed GateDJoinEvidencePreimageV1 object>
gate_d_evidence_hash=<64-lower-hex>
gate_d_audit_record_hash=<64-lower-hex>
```

The Python validator owns an identical closed schema: unknown/missing fields,
wrong JSON scalar types, unsafe integers, path/domain drift, unequal leaf
counts, an unrecognized official provider/status, noncanonical time/hash,
sample mismatch, evidence-hash mismatch, final audit count/tail mismatch or
any nonzero mismatch count fails. The helper retains each authority descriptor
through DB/audit snapshot close, official revalidation, the final identity
rewalk, terminal audit append/sync/readback and the one final audit-session
`finish()`.

`gate_d_evidence_preimage` is the complete strict object frozen above, not a
hash alias or caller-supplied projection. Every one of its fields that also
appears at top level must be exactly equal. The Python validator strict-decodes
that nested object, RFC-8785 canonicalizes it, independently computes
`sha256("stock_analysis.selection_v2_gate_d_join_evidence.v1\0" || bytes)` and
requires equality with `gate_d_evidence_hash`. It also requires
`selection_audit_prefix_record_count + 1 ==
selection_audit_record_count`,
`selection_audit_prefix_tail_hash` to equal the nested preimage value, and
`selection_audit_tail_hash == gate_d_audit_record_hash`. The Rust helper still
proves that the appended Gate-D audit record's `previous_hash` equals the
prefix tail and its record hash equals the final tail. Omitting or mutating
the nested preimage or either prefix field therefore fails independently even
though Python never opens SQLite or JSONL.

An empty day can prove VerifiedEmpty ingress but cannot alone prove generation
activation. Live proof must contain at least one real terminal generation
subject and at least one real `Admitted` subject with a Selected row proof. A
HardRejected proof remains valid terminal-rejection evidence but cannot satisfy
the separate Selected proof requirement.

## 10. Machine-checkable acceptance criteria

### AC-1 — source shape and business identity

- `TerminalDecisionKind` still contains exactly persisted `Admitted` and
  `HardRejected`; no migration renames `admitted`.
- rendering tests prove business label Selected maps only to receipted
  `Admitted`.
- `rg` proves no active gateway request contains raw `event_references.text`.
- a direct mention without an independently matched activated chain creates
  zero candidates and cannot fabricate `chain_id`/`matched_keyword`.
- the static verifier reports zero renamed or aliased design-named contract
  identifiers; only private locals absent from this document may differ.

### AC-2 — activation fail closed

- missing proposal, artifact or activation yields the exact one-line
  `selection_v2 disabled=<typed_reason>`;
- spy tests prove zero selection provider constructors/calls, zero
  selection-v2 DB operations after binding, and zero selection
  scheduler/sink/order/paper creation;
- malformed/conflicting partial evidence is fatal, never Disabled.
- matrix fixtures prove only complete three-authority absence with no
  activation/proposal/receipt claim and exactly one valid reviewed prerequisite
  marker can map to `TradingCalendarMissing`, `TradingCalendarUnverified` or
  `TradingCalendarCoverageIncomplete`; no marker, multiple/conflicting markers,
  any path, hash, leaf, activation claim or partial authority instead fails exactly
  `calendar_release_integrity_conflict`, while parser/session/T0..D5 conflict
  fails exactly `calendar_parser_or_session_conflict`;
- strict enum fixtures prove `OutcomeDisabledReason` has exactly one variant,
  `OutcomeActivationNotReleased`, serializes only as
  `outcome_activation_not_released`, and rejects unknown/missing,
  differently-cased or caller-supplied tokens;
- tests prove the shared global lease is acquired before any pool/catalog
  constructor and that activation/artifact expiry is rechecked at all three
  runtime boundaries.
- a two-phase namespace fixture proves the bootstrap acquires the maintenance
  lease before owner installation, the post-install six-child split occurs
  exactly once, no sink child is minted/consumed, unused children construct
  zero resources, and the maintenance child can only bind the retained lease;
  any attempt to use it as a constructor or cause a second lock open/acquire
  fails before resource I/O;
- production and TEST_CODE database/audit/calendar/lock identities are
  physically distinct; cross-mode security identities fail before provider or
  write.
- exact checked-in notice manifest and every fixed raw SSE/SZSE notice path
  are required; descriptor/hash/parser/session/T0..D5 equality failure cannot
  activate and is fatal whenever any member claims release authority.
- the regular
  `config/selection/a_share_trading_calendar_notices.v1.json` manifest and
  directory `config/selection/a_share_trading_calendar_notices.v1/` raw root
  are proven distinct fixed descriptor authorities; file-as-directory,
  alias, suffix drift and derived alternate paths reject.
- notice-manifest, raw-notice-set and parser-equality golden vectors use their
  exact closed RFC-8785 payload schemas; every root/entry denies unknown or
  missing fields. A one-field mutation, reordered manifest entry,
  parser-descriptor, session or T0..D5 array, noncanonical object-key encoding
  and delimiter-free concatenation each produce a different/rejected identity.

### AC-3 — offline production migration

- plan mode is byte-for-byte nonmutating;
- recognized pre-amendment fixture applies to a candidate and preserves every
  unaffected row/count;
- production apply uses pinned roots, full backup, file+directory fsync and
  real atomic exchange;
- crash injection at every boundary in section 6.3 appends one permanently
  parseable terminal closure, safely discards only an exact pre-exchange
  candidate, or closes forward after exchange;
- strict audit golden vectors prove historical records retain their existing
  hashes and every new migration/restore phase round-trips through
  `SelectionAuditRecord`; no second JSONL parser exists;
- migration, restore and deactivation quarantine fixtures accept only their
  one-to-one closed `SelectionIntegrityCode` and identical audit
  `reason_codes` singleton; cross-mapping, unknown strings and using
  `migration_state_ambiguous` for either other operation reject;
- Prepared golden vectors require complete candidate
  device/inode/size/hash, exact canonical operation payload order,
  domain-separated content hash and per-run outer-record linkage;
- source checkpoint/quiescence/complete hash precede Planned and remain pinned
  through exchange; crash injection proves no leaf exists before Planned;
- a Planned-only run with no leaves aborts, while a Planned-only run with any
  unbound named leaf retains it and appends Quarantined; no recovery deletion
  is selected by filename;
- `LeavesAllocated` persists exact device/inode/link-count/empty hash before
  copy, and later recovery mutates only an identity-equal run-owned leaf;
- restore Planned creates no candidate; a crash with no leaf aborts, while any
  named leaf before `RestoreCandidateAllocated` is retained and quarantined;
  exact CandidateAllocated/Prepared identities alone authorize removal after a
  terminal aborted audit line, and alias/replacement is retained/quarantined;
- both migration Prepared and CandidateVerified abort fixtures freeze the
  final candidate descriptor identity/hash, append, `sync_data`, parent-fsync
  and read back `V2MigrationAbortedPreExchange` before descriptor-relative
  unlink; crash after readback resumes only identity-equal idempotent cleanup,
  and every unlink-before-terminal mutant rejects;
- macOS adapter calls descriptor-relative `renameatx_np(..., RENAME_SWAP)`;
  Linux calls descriptor-relative `renameat2(..., RENAME_EXCHANGE)`;
- unsupported filesystem/exchange, alias, sidecar, lock or identity drift
  rejects before mutation;
- exact target is idempotent; mixed/extra/weakened catalog is rejected.

### AC-4 — pending query and concurrency

- query returns only activation+ingress-receipted admitted facts;
- fixed ordering is `committed_at, source_fact_key`; fixed maximum is 200;
- closed terminal subjects are excluded, retryable subjects obey prospective
  date, and later rows remain pending;
- a fairness round freezes one receipt high-water until both above-checkpoint
  and wrapped phases exhaust; every page binds that same high-water, exact key
  list, phase/cursor/checkpoint and audit prefix before subject work;
- the initial `checkpoint_before=None` branch has no SQL lower-bound
  predicate and means negative infinity; after AboveCheckpoint exhausts, its
  wrap is closed without a page query. Tests reject `> NULL`, `<= NULL`,
  `COALESCE`, fabricated timestamps/keys and locale collation;
- page closure advances only the private round cursor after every row is safe;
  the global checkpoint changes only in the round-closing CAS;
- a fixture whose first 200 keys always close `pending_dependency` still
  issues keys 201..451 on following pages; restart with a half-processed page
  recovers its remaining stored keys before any new page and produces no
  duplicate provider intent;
- a sustained-arrival fixture inserts at least 200 higher keys between pages;
  those keys remain above the frozen round high-water, the existing round still
  wraps and reaches every older eligible key, then the next round reaches the
  inserted keys;
- two processes reading the same snapshot yield at most one provider call and
  one terminal receipt for the logical subject;
- recovery fixtures prove ranks 1–12, every complete ascending tuple and unique
  tie-break from section 7.1; equal tuple/different bytes, unknown class or
  unexpected null fails closed, and the queue restarts at rank 1 after closure;
- every provider spy proves its full intent bytes/hash and receipt exist
  before I/O; success/error/cancel produces a distinct full evidence seal;
  every success, verified-empty, transport-error, over-limit and cancellation
  cycle appends/syncs/reads back exactly one closed
  `GenerationIngressCycleTerminalReceipt`; failure cycles create no
  `PreparedSourceIngress`;
- a multi-feed fixture proves only one feed future exists at a time, every
  feed seal reads back before the next intent, aggregate order equals the
  frozen registration plan, recovery reuses the exact sealed prefix, and an
  error/cancel/uncertain feed terminalizes the cadence without contacting or
  creating intents for remaining feeds. Success/empty proves a full all-Sealed
  vector; stopped cycles prove the exact contiguous resolution prefix,
  `resolved+suffix=total`, stop ordinal and absent suffix intents in both the
  aggregate and cycle terminal receipt;
- prior-boot feed intent recovery appends/syncs/reads back one strict
  uncertainty carrier, binds its hash in exactly one final `Uncertain`
  resolution and rejects omitted/substituted uncertainty hashes, a later seal,
  duplicate uncertainty, gaps, suffix intents and any non-final `Uncertain`;
- strict enum fixtures accept only the four frozen ingress terminal kinds,
  four aggregate outcome kinds, five feed-acquisition outcome kinds and closed
  failure/integrity codes; every feed response/error/null/count matrix row is
  accepted only with its exact typed error/retryability pair. Unknown,
  wrong-case, bytes/hash asymmetry, wrong nulls, wrong count range, legacy
  `response_limit_exceeded` and reordered-token substitutes reject;
- the exact
  `br193_feed_outcome_enum_and_response_error_matrix_are_closed` fixture
  recomputes `verified_empty_feed_count` and `total_response_record_count` for
  all-empty, mixed nonempty/empty, transport, cancel and over-limit prefixes,
  then mutates each counter, aggregate outcome, terminal kind and failure code
  independently and requires rejection; the exact
  `br193_ingress_uncertain_resolution_closes_stopped_prefix_and_suffix`
  fixture performs the same mutation matrix for the final `Uncertain` case
  and proves the uncontacted suffix contributes zero to both counters;
- activation and pre-cadence fixtures require the frozen registration plan
  length and `total_feed_count` to be at least one. An empty activation plan is
  Disabled before scheduler/provider/write construction; an empty or changed
  plan at tick revalidation is fatal before cadence persistence, and neither
  may claim verified-empty or create `PreparedSourceIngress`;
- prior-boot intent-only recovery emits uncertainty and never repeats that
  intent; a sealed-prefix fixture reuses steps 0..k without calls and issues an
  intent/call only for missing step k+1, while a complete no-gap seal set and
  persisted-Prepared fixture reach the generation receipt with zero calls;
- a restart before the next 120-second eligibility instant resumes the same
  cadence cycle or waits and makes zero new acquisition-tick calls; only a
  synced/read-back cadence receipt can start the next cycle;
- each feed request is exactly 20; responses of 0..20 follow their typed normal
  path, while a 21-record fixture is sealed as
  `feed_response_limit_exceeded`, creates zero source facts and is never
  truncated;
- attempt ordinal/prior receipt hashes and same-subject high-water form an
  exact closed chain; unrelated global high-water advancement does not
  invalidate the subject, while prefix replacement does.
- Selected paging uses a pinned keyset pager, not OFFSET; a fixture with 451
  valid Selected rows returns pages `200,200,51,None` with no duplicate,
  omission or caller cursor/limit.

### AC-5 — canonical industry-chain candidates

- direct mention and board constituent both produce the canonical candidate
  type;
- a chain keyword match uses the immutable activation snapshot and exact
  reviewed binding;
- complete membership evidence is retained in provider order;
- direct and board evidence for the same event/chain/code merge into one
  ordered evidence set without changing the existing sample key;
- missing required binding, partial membership, cross-batch evidence,
  evidence-hash-as-identity and stock-only dedup reject;
- complete verified zero members is not provider unavailable.
- direct mention chain ownership follows section 5.3's real keyword rule;
  mention-only source text yields no fabricated chain.

### AC-6 — Magic TDX evidence

- market gateway accepts canonical candidates and does not parse news text;
- every input candidate has one complete result or typed rejection;
- component provider/source, source time or market date, observed time,
  request hash and batch/content hash survive into generation rows/receipt;
- locally observed time never fills provider source time;
- realtime 5-second and daily one-trading-day freshness plus all rule 2.3
  checks execute before features.
- lifecycle, corporate-action and every required BR-171 confirmation receipt
  are part of request, complete evidence, persisted mapping and receipt hash;
  missing confirmation yields `pending_dependency`, never a sample.
- production BR-171 lookup returns a closed object containing exact
  `confirmation_id` and independently recomputed chain `record_hash`, ledger
  high-water/tail; Boolean presence has zero production callers.
- the official calendar artifact provider/version/hash and exact T0..D5
  vector/hash survive into activation, sample and generation receipt; mutable
  process holidays cannot affect a bound run.
- fixed raw-notice paths and bytes/hash, notice manifest hash, raw-notice-set
  hash and deterministic parser-equality hash survive the same join; fixtures
  prove either exchange/parser/session/T0..D5 mismatch rejects.

### AC-7 — terminal rejection is real

- admission failure persists `HardRejected` with at least one typed rejection;
- every Admitted and HardRejected row has an independent
  `ReceiptedTerminalDecisionProof`; a HardRejected proof cannot construct or
  satisfy a Selected row proof;
- golden vectors independently freeze the exact domain, schema, logical field
  order, RFC-8785 bytes and domain-separated output hash for
  `TerminalProofPreimage`, `SelectedRowProofPreimage` and
  `SelectedPageProofPreimage`; output hashes occur only in their strict outer
  carriers and never in their own preimages;
- compile-time and parse fixtures freeze the closed typed structs in section
  5.6: exact date/UUID/hash/ID/stock-code strings, I-JSON-safe integer
  counts/high-waters, lowercase decision tokens, always-present
  `query_after_key_or_null` encoded as JSON null or the exact five-field key,
  non-null first/last keys and all three exact outer carriers;
- per-newtype rejection fixtures cover invalid date, non-v7/uppercase UUID,
  non-nanosecond/non-UTC time, blank/trim/control ID, noncanonical stock code,
  uppercase/wrong-length hash, signed/float/string/exponent/over-I-JSON integer,
  schema other than integer `1`, wrong domain and wrong JSON scalar type;
- single-field mutation tests cover every preimage field and output hash;
  reorder tests cover multi-code HardRejected vectors, page
  `ordered_row_proof_hashes` and outer `rows`; unknown/missing fields,
  noncanonical input bytes and any row-hash/order mismatch fail closed;
- Selected projection excludes it;
- retryable acquisition failure creates no sample and remains eligible only
  inside the stored window;
- replay is idempotent by logical subject/content and conflict fails closed.
- the formal Selected pager returns every fully receipted `admitted` row in
  fixed keyset order and excludes every HardRejected/pending/legacy row.
- each Selected row contains its own ingress/generation/audit/evidence/calendar/
  terminal proof hash and row hash; it must join an Admitted terminal proof.
  The page contains one pinned snapshot/high-water plus an ordered row-proof
  hash list, never singular representative receipts.
- multiline static evidence proves exactly one production consumer:
  `selection_v2_generation_scheduler_loop`; the offline join verifier is not
  counted as a production consumer.
- confirmed >20% changes continue to admission, while an absent/mismatched
  confirmation remains pending; neither case is a magnitude-only hard reject.

### AC-8 — forbidden dependencies

Static dependency tests prove this slice does not import or call notification
sinks, durable delivery, order submission, broker mutation, paper engine or
outcome provider/scheduler APIs. Startup always reports outcome disabled.

### AC-9 — operational evidence

With complete release artifacts and real providers:

```text
cargo run --bin monitor
```

must show active generation and one bounded producer-to-receipt summary.
With an artifact intentionally absent in an isolated test fixture, it must
show the exact disabled summary and zero selection side effects.

The authoritative database check must return at least one source fact joined
through activation, ingress and generation receipts to a terminal decision and
at least one Admitted subject joined to its separate Selected row proof; the
audit check must validate their exact Prepared/Committed hashes. Queries print
only IDs, hashes, provider tokens, times and counts.

The startup and tick lines must carry separate
`activation_run_id=<canonical-uuidv7>` and
`activation_receipt_hash=<64-lower-hex>` fields. Any
`activation_run_hash=` field fails.

The Gate-D audit fixture proves one `LockedSelectionAuditSession` owns the
captured prefix through the final descriptor rewalk and terminal
append/sync/readback, then receives exactly one `finish()` call. Early finish,
lock/session reacquisition, append against a refreshed tail, a second finish
or success output before the final validation receipt each fails.
The controlled official-I/O fixture proves the only provider-under-audit
operation is the fixed Gate-D canonical-URL loop, each request is sequential,
single-attempt and timeout-bounded, and every timeout/cancel/transport/status/
parse/identity/byte/descriptor failure takes `abort_without_append()` with an
unchanged prefix and no output. The Python fixture independently canonicalizes
the emitted complete `GateDJoinEvidencePreimageV1`, recomputes the
domain-separated hash and rejects a mutated preimage, prefix count/tail or
top-level duplicate even when the helper-supplied hash string is unchanged.

### AC-10 — repository gates

Gate B adds exactly these isolated suites and verifier; their expected terminal
counts are frozen:

```bash
cargo test --test br193_selection_activation -- --test-threads=1
# expected: 22 passed; 0 failed

cargo test --test br193_selection_migration -- --test-threads=1
# expected: 26 passed; 0 failed

cargo test --test br193_selection_scheduler -- --test-threads=1
# expected: 19 passed; 0 failed

cargo test --test br193_selection_projection -- --test-threads=1
# expected: 11 passed; 0 failed

run_exact_named_test() {
  suite="$1"
  exact_name="$2"
  listing="$(cargo test --test "$suite" -- --list --format terse)"
  count="$(printf '%s\n' "$listing" |
    awk -v wanted="$exact_name: test" '$0 == wanted { n += 1 } END { print n + 0 }')"
  test "$count" -eq 1
  cargo test --test "$suite" "$exact_name" -- \
    --exact --include-ignored --test-threads=1
}

# These tests are included in, not additional to, the frozen suite totals.
run_exact_named_test br193_selection_activation \
  br193_calendar_paths_manifest_and_raw_root_are_distinct
run_exact_named_test br193_selection_activation \
  br193_calendar_notice_manifest_rfc8785_golden
run_exact_named_test br193_selection_activation \
  br193_calendar_auxiliary_payloads_reject_mutation_and_reorder
run_exact_named_test br193_selection_activation \
  br193_calendar_conflicts_are_fatal_reviewed_absence_only_is_disabled
run_exact_named_test br193_selection_activation \
  br193_outcome_disabled_reason_enum_and_token_are_closed
run_exact_named_test br193_selection_scheduler \
  br193_fairness_first_round_none_is_negative_infinity
run_exact_named_test br193_selection_scheduler \
  br193_fairness_first_round_sql_has_no_null_or_sentinel_comparison
run_exact_named_test br193_selection_projection \
  br193_terminal_proof_rfc8785_golden_and_mutations
run_exact_named_test br193_selection_projection \
  br193_selected_row_proof_rfc8785_golden_and_mutations
run_exact_named_test br193_selection_projection \
  br193_selected_page_proof_rfc8785_golden_mutations_and_reorder
run_exact_named_test br193_selection_projection \
  br193_proof_outer_carriers_reject_self_reference_and_order_drift
run_exact_named_test br193_selection_activation \
  br193_gate_d_notice_authority_descriptor_join_is_exact
run_exact_named_test br193_selection_scheduler \
  br193_ingress_uncertain_resolution_closes_stopped_prefix_and_suffix
run_exact_named_test br193_selection_activation \
  br193_gate_d_retains_one_audit_session_through_verified_append
run_exact_named_test br193_selection_scheduler \
  br193_feed_outcome_enum_and_response_error_matrix_are_closed
run_exact_named_test br193_selection_scheduler \
  br193_nonempty_feed_plan_is_required_at_activation_and_tick
run_exact_named_test br193_selection_migration \
  br193_quarantine_reasons_use_closed_integrity_codes
run_exact_named_test br193_selection_activation \
  br193_namespace_bootstrap_binds_existing_lease_without_reacquire
run_exact_named_test br193_selection_activation \
  br193_gate_d_official_io_exception_is_bounded_and_fail_closed
run_exact_named_test br193_selection_activation \
  br193_gate_d_python_recomputes_emitted_evidence_preimage_hash

python3 tools/release/verify_br193_selection_activation.py
# expected exit 0 and:
# provider_constructor_callers=1
# scheduler_install_callers=1
# selected_projection_public_callers=1
# selected_projection_named_production_consumer=selection_v2_generation_scheduler_loop
# selected_projection_offset_queries=0
# pending_generation_keyset_queries=1
# pending_generation_offset_queries=0
# fairness_round_fixed_high_water_paths=1
# durable_pre_io_intent_paths=1
# post_response_evidence_seal_paths=1
# restart_aware_cadence_receipt_paths=1
# ingress_tick_plan_intent_paths=1
# ingress_feed_intent_paths=1
# ingress_feed_evidence_seal_paths=1
# ingress_global_batch_seal_paths=1
# ingress_cycle_terminal_receipt_paths=1
# ingress_feed_resolution_union_variants=2
# ingress_feed_plan_min_count=1
# ingress_feed_outcome_kind_variants=5
# ingress_response_error_null_matrix_rows=5
# ingress_uncertainty_record_hash_paths=1
# ingress_stopped_prefix_cardinality_paths=1
# ingress_uncontacted_suffix_intent_paths=0
# ingress_failure_prepared_source_paths=0
# response_record_limit_validation_paths=1
# recovery_order_registrations=1
# br171_boolean_production_callers=0
# calendar_raw_notice_fixed_path_violations=0
# calendar_release_prerequisite_marker_fixed_paths=1
# calendar_release_prerequisite_marker_variants=3
# calendar_artifact_rfc8785_payload_paths=1
# calendar_auxiliary_rfc8785_evidence_hash_payloads=3
# calendar_notice_manifest_closed_payload_paths=1
# calendar_notice_manifest_raw_root_distinct=1
# fairness_initial_none_rust_branches=1
# fairness_initial_none_sql_branches=1
# proof_typed_closed_preimage_structs=3
# proof_typed_closed_outer_structs=3
# proof_validated_newtype_count=11
# proof_unvalidated_string_aliases=0
# proof_named_exact_test_wrappers=20
# outcome_disabled_reason_variants=1
# outcome_disabled_reason_tokens=1
# br193_frozen_contract_identifier_renames=0
# proof_mutation_harness_cases=25
# operation_quarantine_closed_integrity_mappings=3
# operation_quarantine_caller_string_paths=0
# namespace_bootstrap_types=1
# namespace_owner_types=1
# namespace_resource_capability_kinds=6
# namespace_sink_capability_mint_paths=0
# namespace_sink_capability_consume_paths=0
# namespace_duplicate_capability_mint_paths=0
# namespace_maintenance_acquire_before_owner_paths=1
# namespace_maintenance_child_lock_constructor_paths=0
# namespace_maintenance_reacquire_paths=0
# gate_d_locked_audit_session_types=1
# gate_d_verified_append_before_finish_paths=1
# gate_d_finish_before_verified_append_paths=0
# gate_d_locked_audit_session_finish_calls=1
# gate_d_official_io_exception_modules=1
# gate_d_official_io_retry_paths=0
# gate_d_emitted_evidence_preimage_paths=1
# gate_d_python_evidence_hash_recomputations=1
# br193_mutation_manifest_sha256=639a588a3a0a47555a2791dbcbf3cca95cd5b1814e94dff0133906b37175f1a9
# br193_mutation_manifest_total=54
# br193_mutation_manifest_family_counts=calendar:12,fairness:4,typed_proof:25,gate_d:13
# br193_mutation_registered_code_paths=54
# br193_mutation_executed_code_paths=54
# planned_only_name_delete_paths=0
# restore_planned_only_name_delete_paths=0
# terminal_selected_proof_conflations=0
# terminal_proof_rfc8785_preimage_paths=1
# selected_row_proof_rfc8785_preimage_paths=1
# selected_page_proof_rfc8785_preimage_paths=1
# proof_output_hash_self_reference_paths=0
# migration_audit_line_parsers=1
# historical_audit_golden_vectors=1
# test_prod_namespace_aliases=0
# raw_text_market_request_fields=0
# sink_order_paper_outcome_edges=0
# activation_run_hash_log_fields=0
# activation_run_id_log_fields=2
# activation_receipt_hash_log_fields=2

bash tools/compliance/lib/check_br193_selection_activation_mutations.sh
# expected exit 0 and:
# unchanged_fixture_passes=1
# mutation_manifest_sha256=639a588a3a0a47555a2791dbcbf3cca95cd5b1814e94dff0133906b37175f1a9
# registered_mutants=54
# executed_mutants=54
# calendar_mutants_rejected=12
# fairness_mutants_rejected=4
# proof_mutants_rejected=25
# gate_d_authority_mutants_rejected=13
# accepted_mutants=0
```

`run_exact_named_test` is mandatory; aggregate suite success never substitutes
for it. A missing test, duplicate terminal name or zero selected tests fails.
The mutation script copies a checked-in minimal BR-193 fixture to a fresh
temporary directory per case, applies exactly one mutation, executes
`verify_br193_selection_activation.py --fixture-root <absolute-temp-root>`,
requires exit nonzero and deletes no source/repository file. The verifier's
only non-production argument is this checker-only fixture root, guarded by
`TEST_CODE_BR193_MUTATION`; release binaries and the production join helper
still accept no path override.

The mutation registry is the checked-in fixed file
`tools/compliance/fixtures/br193/mutation_manifest.v1.json`. Its bytes must be
exactly the following RFC-8785 canonical object, with no BOM, whitespace or
trailing newline:

```json
{"domain":"stock_analysis.br193_mutation_manifest.v1","families":[{"family":"calendar","ids":["cal_manifest_equals_raw_root","cal_raw_root_add_json","cal_leaf_symlink","cal_leaf_hardlink_alias","cal_unlisted_leaf","cal_raw_byte_changed","cal_manifest_hash_changed","cal_manifest_entries_reordered","cal_parser_descriptors_reordered","cal_sessions_reordered","cal_t0_d5_reordered","cal_referenced_leaf_deleted"]},{"family":"fairness","ids":["fair_none_gt_null","fair_none_coalesce_sentinel","fair_none_wrap_query","fair_empty_round_fabricated_checkpoint"]},{"family":"typed_proof","ids":["proof_terminal_preimage_allow_unknown","proof_row_preimage_allow_unknown","proof_page_preimage_allow_unknown","proof_terminal_outer_allow_unknown","proof_row_outer_allow_unknown","proof_page_outer_allow_unknown","proof_safe_integer_string","proof_safe_integer_float","proof_safe_integer_over_max","proof_query_after_omitted","proof_terminal_self_hash","proof_row_self_hash","proof_page_self_hash","proof_terminal_golden_field","proof_row_golden_field","proof_page_golden_field","proof_rejection_codes_reordered","proof_page_hashes_reordered","proof_outer_rows_reordered","proof_first_key_mismatch","proof_last_key_mismatch","proof_outer_missing_field","proof_validated_string_wrong_type","proof_domain_or_schema_wrong","proof_empty_feed_plan"]},{"family":"gate_d","ids":["gated_trust_db_calendar_hash","gated_skip_final_descriptor_rewalk","gated_alias_manifest_raw_root","gated_skip_canonical_reparse","gated_skip_official_url_fetch","gated_allow_redirect_host_drift","gated_mutate_official_publication","gated_mutate_official_body","gated_remove_output_field","gated_add_output_field","gated_finish_before_evidence_audit_append","gated_mutate_http_status_or_observed_at","gated_mutate_emitted_evidence_preimage"]}],"schema_version":1}
```

The exact SHA-256 of those bytes is
`639a588a3a0a47555a2791dbcbf3cca95cd5b1814e94dff0133906b37175f1a9`.
The registry has exactly 54 unique IDs in this exact family order and exact
counts: `calendar=12`, `fairness=4`, `typed_proof=25`, `gate_d=13`. The
checker rejects any missing, additional, duplicate or reordered family/ID,
any count drift, noncanonical bytes or manifest hash drift before executing a
mutant.

The exact `proof_empty_feed_plan` mutant removes the sole descriptor from the
otherwise valid frozen feed plan and reaches both activation classification
and pre-cadence revalidation in the isolated harness; the checker must reject
before any cadence/provider/write path and must reject any fabricated
verified-empty result. The exact
`gated_mutate_emitted_evidence_preimage` mutant changes one
`selection_audit_prefix_tail_hash` nibble inside the emitted nested preimage
while retaining the original helper-supplied evidence hash and top-level
fields; the Python checker must independently recompute and reject it.

Every ID must execute exactly once and report its ID, family, changed-byte
count, checker invocation count and rejected result. Filesystem artifact
mutants operate on a fresh isolated fixture copy. Source/checker-path mutants
must copy the checked-in minimal compilable BR-193 checker/helper harness (or
an isolated temporary worktree), apply the exact registered source patch,
compile it and execute the mutated code path; changing only a JSON fixture
cannot satisfy a source mutation. Gate-D official-read mutations use a
physically isolated `TEST_CODE` local HTTPS provider spy with fixed
certificates and never production URLs, credentials or files.
The exact `gated_finish_before_evidence_audit_append` source mutant consumes
the original `LockedSelectionAuditSession` with `finish()` immediately after
prefix validation, reacquires a second session and reaches the
terminal-append branch through that invalid replacement. The isolated mutant
must compile and execute that branch exactly once; the checker must reject the
early-finish/reacquisition path before success output.

The script itself rejects an empty registry, a mutation that changes zero
bytes, a registered mutation whose code path did not execute exactly once, a
mutant that exits zero, a family count other than `12/4/25/13`, a total other
than 54 or an accepted mutant. `tools/compliance/check.sh` invokes both the
verifier and mutation script.

Gate C/D commands and expected results are:

```bash
cargo fmt --all -- --check
# expected exit 0
cargo clippy --workspace --all-targets --all-features -- -D warnings
# expected exit 0, 0 warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
# expected exit 0, 0 failed
bash tools/compliance/check.sh
# expected exit 0, every sub-check PASS
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
# expected exit 0
python3 tools/coverage/check_thresholds.py \
  target/coverage/coverage.json --global-min 80 --core-min 95
# expected exit 0; global >= 80.00%, registered core >= 95.00%
cargo build --release --bin monitor
# expected exit 0
cargo build --release --bin selection_v2_verify_join
# expected exit 0
python3 tools/release/verify_br193_production_join.py
# expected exit 0; verification_run_id=<canonical-uuidv7>,
# verification_started_at/verification_completed_at=<canonical nanos UTC>,
# activation_run_id=<canonical-uuidv7>,
# activation_receipt_hash=<64-lower-hex>,
# database_receipt_high_water=<safe integer>,
# selection_audit_prefix_record_count=<safe integer>,
# selection_audit_prefix_tail_hash=<64-lower-hex>,
# selection_audit_record_count=<safe integer>,
# selection_audit_record_count=selection_audit_prefix_record_count+1,
# selection_audit_tail_hash=gate_d_audit_record_hash,
# activation_receipts=1, ingress_receipts>=1,
# ingress_intents>=1, response_evidence_seals>=1,
# ingress_cycle_terminal_receipts>=1,
# ingress_cycle_terminal_receipt_hash=<64-lower-hex>,
# fair_generation_pages>=1, persisted_prepared_generations>=1,
# generation_receipts>=1, terminal_samples>=1, terminal_decision_proofs>=1,
# admitted_samples>=1, admitted_terminal_proofs>=1, selected_row_proofs>=1,
# invalid_selected_rows=0,
# unreceipted_selected_rows=0, coherent_db_receipt_high_water=1,
# coherent_audit_prefix=1,
# calendar_manifest_path=config/selection/a_share_trading_calendar.v1.json,
# notice_manifest_path=config/selection/a_share_trading_calendar_notices.v1.json,
# raw_notice_root=config/selection/a_share_trading_calendar_notices.v1/,
# calendar_manifest_descriptor_attested=1,
# notice_manifest_descriptor_attested=1,
# raw_notice_root_descriptor_attested=1,
# raw_notice_leaf_count>=2,
# raw_notice_descriptor_attested_count=raw_notice_leaf_count,
# calendar_manifest_canonical=1, notice_manifest_canonical=1,
# raw_notice_set_canonical=1, calendar_raw_notice_hash_mismatches=0,
# calendar_notice_parser_equality=1,
# calendar_session_vector_mismatches=0,
# calendar_t0_d5_vector_mismatches=0,
# calendar_official_url_revalidated_count=raw_notice_leaf_count,
# calendar_official_http_success_count=raw_notice_leaf_count,
# calendar_official_notice_identity_mismatches=0,
# calendar_official_publication_mismatches=0,
# calendar_official_raw_byte_mismatches=0,
# official_revalidation_entries=<closed nonempty array with status/observed_at>,
# official_revalidation_evidence_hash=<64-lower-hex>,
# calendar_hash/calendar_artifact_content_hash=<64-lower-hex>,
# notice_manifest_content_hash/calendar_raw_notice_set_hash=<64-lower-hex>,
# calendar_parser_equality_hash/calendar_descriptor_attestation_hash=<64-lower-hex>,
# terminal_proof_preimage_mismatches=0,
# selected_row_proof_preimage_mismatches=0,
# selected_page_proof_preimage_mismatches=0,
# br171_closed_receipt_mismatches=0,
# terminal_subject_identity_hash/terminal_proof_hash=<64-lower-hex>,
# admitted_subject_identity_hash/admitted_terminal_proof_hash=<64-lower-hex>,
# selected_row_proof_hash/selected_page_content_hash=<64-lower-hex>,
# selected_page_snapshot_identity=<64-lower-hex>,
# gate_d_evidence_preimage=<strict closed object matching top-level fields>,
# gate_d_evidence_hash/gate_d_audit_record_hash=<64-lower-hex>,
# writer_freeze=exclusive_lease
```

The Python verifier never opens SQLite or JSONL itself. It executes only the
fixed-root `selection_v2_verify_join` release helper, validates its closed JSON
output, independently canonicalizes the emitted
`gate_d_evidence_preimage`, recomputes its domain-separated hash, validates
prefix-to-final count/tail equations and rejects extra fields. The helper first
acquires the same fixed
global **exclusive** maintenance lease used by migration, so a running monitor,
pool or migration writer prevents verification. While that writer freeze is
held it pins the database/audit parents and all three fixed calendar authority
descriptors from the compile-time manifest, takes the selection-audit lock in
registered order, captures one validated outer audit snapshot
(`record_count`, tail hash and every record hash), opens the production
database descriptor read-only and begins one SQLite snapshot transaction.

The helper captures the database commit-receipt high-water inside that
transaction and proves every activation/ingress/generation receipt in the join
references Prepared/Committed audit hashes present at or below the captured
audit high-water. It also proves the joined activation receipt/calendar hash
and terminal decision live in that same SQLite snapshot. From the pinned
calendar descriptors it independently re-hashes/reparses every fixed config
and raw notice, then revalidates every official URL/publication/raw byte as
specified in section 9. The lease remains held through transaction close,
official revalidation, descriptor identity revalidation, append/sync/readback
of the single `v2_gate_d_join_verified` terminal audit record and the one
final `LockedSelectionAuditSession::finish()`. This makes the
database/audit/calendar report coherent rather than joining moving
observations. The helper proves the final validation receipt and output
tail/count against that record before printing. It accepts no database path,
audit path, calendar path, raw-root path, identity, high-water, clock, URL or
lease argument.

Independent reviewer objections must be zero before Gate A is described as
approved.

## 11. Implementation order and rollback

Implementation order:

1. register BR-193 and independently approve this Gate A design;
2. complete exact offline migration/receipt owner and crash tests;
3. add the reviewed official-calendar artifact, closed RFC-8785
   `NoticeManifestPayload`, fixed raw SSE/SZSE notice bytes, deterministic
   equality parser and trustworthy proposal/artifact/activation through PR;
4. add fixed-high-water fair rounds, restart-aware cadence receipts, fair
   pending pages, pre-I/O intents, response seals, sealed-prefix recovery and
   opaque persisted-Prepared generation owner;
5. split relation candidate generation from Magic TDX market enrichment;
6. add closed BR-171 receipt lookup, the three non-self-referential proof
   preimages and strict hash carriers, separate per-row terminal-decision proof
   and Admitted-only Selected proof, then bind the sole production consumer to
   `GenerationActive`;
7. add the fixed-root `selection_v2_verify_join` helper and closed-output
   Python validator;
8. run Gate B/C/D and live evidence checks.

Before production exchange, rollback is executable as:

```bash
cargo run --release --bin migrate_selection_v2 --
# expected: fixed-root diagnostic identifies Prepared/CandidateVerified and
# reports exchange_performed=false
cargo run --release --bin migrate_selection_v2 -- --recover-pre-exchange
# expected: exit 0; only the journal-identity-matched candidate is removed,
# backup and all journal/audit evidence remain; the run ends in
# V2MigrationAbortedPreExchange
```

After production exchange, code or database `git revert` is forbidden because
it can remove the only parser for permanent migration/restore audit phases.
The mandatory first action is forward closure:

```bash
cargo run --release --bin migrate_selection_v2 -- --recover-forward
# expected: exit 0; Committed + authoritative activation receipt +
# ActivationReceipted are verified
```

For a normal behavior rollback after forward closure, an approver checks in
the strict fixed
`config/selection/selection_generation_deactivation.v1.json`. It binds the
current activation run/receipt, stable reason code, approver identity,
approved-at, expiry (at most 24 hours) and its canonical content hash. Then:

```bash
cargo run --release --bin migrate_selection_v2 -- --deactivate-generation
# expected: exit 0; appends/read-backs/syncs
# V2GenerationDeactivationPrepared/Committed, persists the matching
# deactivation receipt and verifies:
# selection_v2 disabled=activation_revoked
```

This operation leaves the amended schema, migration/restore parser, receipts,
audit, backups and every terminal selection row intact. A compatibility
rollback binary may disable producer/scheduler construction, but it **must**
retain `SelectionAuditRecord`, every phase/context payload parser, the
migration/restore recovery owner and read-only verification. Removing those
components is never a rollback option.

If forward closure is proven impossible, restore is a Controlled Exception
Path, not an automatic fallback. An approver must check in the canonical fixed
file `config/selection/selection_restore_approval.v1.json`; it binds the
migration run ID, exact retained backup inode/hash, current post-exchange
inode/hash, source catalog hash, audit high-water, approver identity, risk
statement, approval time and expiry (at most 24 hours). Then:

```bash
cargo run --release --bin migrate_selection_v2 -- --restore-approved
# expected: fixed-root owner verifies the approval, constructs/fsyncs a
# backup-derived restore candidate, descriptor-relative atomically exchanges
# it with the live file, retains the displaced post-migration file, appends
# and syncs RestorePrepared/RestoreCommitted/RestoreClosed audit records, and
# verifies selection_v2 disabled=schema_not_amended
```

`--recover-pre-exchange`, `--recover-forward`,
`--deactivate-generation` and `--restore-approved` accept no
path/run/hash/clock arguments. A missing/expired/mismatched approval is a
nonmutating refusal. Never copy rows back piecemeal, delete receipts/audit,
unlink a backup, revive the legacy selector, remove the audit parser or route
around data-quality gates.

## 12. PR evidence fields

The implementation PR must include:

```text
Refs: spec §1, §5.1, §5.2, §5.4, §5.5, §5.6, §6, §7.1, §9, §10
Data-Redlines: [2.1, 2.2, 2.3, 2.4, 2.5, 2.7, 2.8, 2.9, 2.10]
OldModules: table from section 4 with implementation disposition
Threshold-Proof: no config threshold changed;
NEWS_FETCH_PERIOD_SECS=120, NEWS_PER_FEED_LIMIT=20,
PENDING_GENERATION_LIMIT=200 and SELECTED_GENERATION_PAGE_LIMIT=200 are frozen
by this document and implemented without environment/CLI/caller override; the
synced/read-back cadence receipt persists `next_acquisition_eligible_at` across
restart, while Skip/single-flight prevents overlap, so a second new acquisition
tick cannot start inside the prior 120-second eligibility window; per-feed 20
is bound independently in the request and enforced before ingress by rejecting
and fully sealing any response with `records.len()>20`; both 200-row limits
bound database work without OFFSET, and the pending limit uses one
fixed-high-water two-phase round so continuous arrivals cannot starve the wrap;
this slice introduces no separate clamp_max field
Business-Rules: [BR-193]
Rollback: section 11 forward-deactivation/recovery commands plus migration
receipt/backup identity; audit parser/phase support remains deployed
```

Until every acceptance criterion and repository Done criterion passes, report
the work as **In Progress / Blocked**, never Done or release-ready.
