# Task Plan: magic-market-data-rs 统一数据迁移

## Goal
让 stock_analysis 的行情、金融和新闻数据通过 magic-market-data-rs 的真实 provider/统一合同访问；未实现能力保持显式 Unsupported/Unavailable，并满足两个仓库的数据红线与 PR 门禁。

## Current Phase
Phase 4 — downstream production caller cutover and legacy acquisition deletion

## Phases

### Phase 1: Requirements & Discovery
- [x] Understand user intent
- [x] Identify repository/process constraints
- [x] Audit upstream provider capabilities and downstream call sites
- [x] Confirm first migration slice with user
- **Status:** completed

### Phase 2: Architecture & Written Spec
- [x] Compare 2-3 migration approaches
- [x] Present recommended design and obtain user approval
- [x] Write, self-review and commit design spec
- [x] Obtain user review of written spec
- [x] Create implementation plan with writing-plans
- **Status:** completed

### Phase 3: Upstream Provider Slices
- [x] Implement and unit-verify the selected real Provider crates
- [x] Align the public workspace crates on version `0.2.0`
- [x] Pass upstream fmt, strict clippy, full tests, compliance and docs checks
- [x] Reject cached “current session” minute/trade payloads outside a valid session
- [x] Publish one immutable `0.2.0` dependency baseline and pin every downstream
      Magic crate to revision `b2b68df78156df1d67824e5c44c0cb01b752f55a`
- [ ] Re-run upstream coverage and bounded real live probes
- **Status:** in_progress

### Phase 4: stock_analysis Adapter Migration
- [x] Introduce one deep data-access module and Provider composition root
- [x] Migrate A-01 and R-03 through the Gateway, then delete their old loaders
- [x] Add upstream all-market discovery/board-membership capabilities required by R-04/A-10
- [x] Add upstream full-market announcement discovery and global-market snapshot capabilities required by R-08
- [x] Migrate R-04, A-10 and R-08 through the Gateway
- [x] Migrate finance/research/news/announcement production callers through the Gateway
- [x] Restore NewsAI against verified content and market-evidence batches
- [x] Complete BR-173 canonical SH/SZ/BSE equity identity and migrate every
      Gateway away from prefix-only exchange guessing
- [x] Complete BR-171 security-lifecycle evidence and an explicit operator
      entry point for exact adjacent-change admission; no automatic
      confirmation or static IPO/ex-right cache is permitted
- [x] After each migrated domain passes focused validation, delete its superseded direct
      provider code, dependencies, configuration and fallback paths
- [x] Prove production financial/news acquisition has no legacy direct HTTP
      path after the final cutover
- [x] Add a regression gate that rejects legacy `data_provider` acquisition
      calls, not only direct hosts/Magic imports
- [x] Remove the remaining announcement, financial, money-flow, intraday and
      north-flow acquisition entry points after each caller is migrated or
      made explicitly unsupported at the Gateway boundary
- [x] Fix test namespace leaks and make repeated `monitor --test` fixtures idempotent
- [x] Run a real strict review with truthful A-01 target-day filtering and
      confirmed R-03/A-10/R-08 delivery
- [x] Remove stale startup text, historical source-selection comments and
      unused provider configuration left behind by the cutover
- [x] Replace or retire the production static industry-chain registry; it must
      not silently masquerade as current Magic TDX board evidence
- [x] Reconcile deliberately unsupported review/report families (R-02/R-05/R-06
      and CFFEX delivery) with their real upstream capability state; each
      remains an explicit typed Disabled/Unsupported outcome until its complete
      source contract passes admission
- [x] Split R-08 overnight indices and USD/CNY into independently admitted
      Magic Sina Gateway components; reject the current index packet because
      it lacks provider `source_at` while preserving any fresh verified FX batch
- [x] Accept the declared Magic TDX provenance timestamp forms (RFC3339 and
      exact Unix seconds/fractional seconds) without weakening freshness checks
- [ ] Append explicit operator confirmations for the two currently pending
      BR-171 batches (688548 2026-06-26→2026-06-29 and 688690
      2026-04-09→2026-04-10), then complete their daily backfill
- **Status:** in_progress

### Phase 5: Verification, PR & Merge
- [ ] Run fmt/clippy/test/compliance and coverage gates
- [ ] Run isolated live probes and both monitor commands
- [ ] Record evidence, independent review, PR and merge
- **Status:** pending

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Do not map contract-only upstream capabilities as available | AGENTS 2.1/2.2/2.8 require real operations and explicit missing data |
| Use a domain data-access seam in stock_analysis | Avoid leaking provider crate details into dozens of callers |
| Preserve the in-progress event-selection branch until migration scope is approved | New request is materially broader and must not silently overwrite current work |
| Recommend upstream-first, domain-by-domain cutover | New upstream provider crates are not yet committed; atomic per-domain verification keeps production rollback possible |
| Preserve unique high-evidence news sources by moving them upstream; delete duplicate/stale sources | User selected option 1; final stock_analysis production paths must not retain local HTTP acquisition |
| Write the overall architecture on an isolated branch and plan only Slice 0 next | Current root is a separate dirty feature branch; later slices require their own Gate A after the previous slice reaches Gate C |
| Register upstream BR-009 through BR-011 before adopting Provider code | Slice 0 introduces capability admission, request bounds/pacing and duplicate identity rules; rule registration is a 2.10 merge gate |
| Keep downstream BR-158/BR-159 for Slice 1 | Slice 0 changes only the Magic repository; Gateway routing and log governance do not exist downstream yet |
| Delete legacy acquisition code after verified cutover | User explicitly rejected long-term compatibility retention; deletion follows per-domain parity/shadow so production is never left without a real source |
| Parallelize only dependency-independent work | The user explicitly requested proactive parallelism. Before each phase, classify data dependencies and write sets; read-only audits and disjoint file sets may run concurrently, while shared modules, schema migrations, final gates and Git integration remain serialized. |

## Errors Encountered
| Error | Resolution |
|-------|------------|
| Root tests blocked by unrelated uncommitted magic_tdx_t0 compile error | Validate current selection repair in clean tree; do not alter unrelated draft |
| R-03 awaited async loader from an existing blocking closure | Diagnosed during current work; keep as open adjacent repair until migration design determines owner |
| Upstream live probe admitted Friday cached trades on Saturday and Beijing current-minute returned explicit empty | Return to Gate A for a current-session admission rule; raw protocol diagnostics may observe cache, but normalized Provider/Gateway data must fail closed |
| Bare `monitor --test` originally reused production dotenv paths and accumulated repeat fixtures | Database/audit namespace fixes landed locally; remaining recommendation-log and trade-fixture leaks stay open in Phase 4 |
| Existing R-04 identity `code:date` collapses distinct same-day Dragon-Tiger reasons | Upstream slice now keys discovery records by source `TRADE_ID` and retains distinct reasons; exact duplicates alone may be folded |
