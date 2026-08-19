# Findings & Decisions

## Requirements
- Deadline: production opening readiness for 2026-08-18.
- User authorized autonomous progress and skill installation; no per-step confirmations.
- Success means every production push due in the pre-open/opening windows can acquire real admissible data and complete delivery; partial success is not enough.
- All production data must be real and preserve source/provider/time/batch evidence.
- Do not expose or commit any file content from `client-bundle` credentials.

## Research Findings
- Existing production monitor PID 89949 started 2026-08-17 00:45:03 and writes `logs/monitor.log`.
- Existing local `grpc_market_server` PID 73473 listens on 127.0.0.1:18082.
- Bare monitor startup: 7/7 position-chain refreshes failed; five responses lost `source`, two were cancelled.
- The local server had real board membership evidence, so the five `source empty` failures are bridge serialization defects.
- Off-session quote checks reject stale quotes correctly but retry all seven holdings about every 30 seconds; provider breakers exceeded 120 failures and the monitor repeatedly falls back to daily close for paper valuation.
- Account snapshot is about 12.4 days stale versus the 30-second rule; AccountMode is conservative and DataMode remains Unsafe.
- selection-v2 due/recovery fails every 60 seconds with `production_database_connection_unverified`, and the audit session is dropped without explicit finish.
- News polling runs every 120 seconds. SecurityIdentity has no gRPC path in the no-default-features monitor, and L2 concept loading calls blocking board memberships which bypass the existing async bridge.
- `client-bundle` is a private remote gRPC contract bundle for 10.211.55.3:50051 with mTLS, Bearer auth, protocol v1, 55 upstream unary operations, events, and capability/health RPCs.
- Current client supports Bearer env injection but only plaintext `Channel::from_shared`; it has no client-bundle mTLS/file loader.
- Local build extends upstream proto with operations 56-61 and `QueryResponse.source=11`; remote upstream responses do not have the local `source` field.
- Current production monitor is intentionally built `--no-default-features`, so any capability without a gRPC bridge is unavailable even though the startup banner lists only two local capabilities.
- Remote mTLS handshake succeeds with TLS 1.3 and a verified `magic-market.local` peer.
- Remote health is `live=true`, `ready=true`, `state=ready`.
- Remote capabilities return 98 provider-operation rows: 81 admitted and runtime-available. This is not 98 operations; reflection confirms operations 0..60.
- Opening-critical admitted remote capabilities include realtime quotes, order books, security metadata, global news, announcements, board constituents/memberships, popularity, concept hits, and instrument news.
- `MoneyFlows`, `MarketRankings`, and `MarketBreadth` remain repository-unadmitted diagnostics and cannot support production pushes under redlines 2.1/2.3.
- The bundle proto is stale versus the live server: its enum ends at operation 55, while reflection exposes operations 56..60. The docs already describe 60 operations.
- Remote `QueryResponse` has no `source` field; the local build adds field 11. A direct remote client therefore receives an empty local-extension field and current converters reject it.
- The three `.superpowers/{task_plan,findings,progress}.md` files were genuine but stale historical planning copies. `task_plan.md` and `findings.md` matched the deleted HEAD root blobs exactly; `progress.md` mixed the same old plan with a new 2026-08-17 append. Their old Gate-A prohibition could block Claude from following the current opening-recovery task.
- The stale `.superpowers` files are preserved under `.superpowers/archive/2026-08-17-stale-production-closure/`; the live paths now point to this plan's files.
- Gate-A design and BR-238 exist. Self-review confirmed the referenced production symbols exist and BR-238 is unique.
- The business-rule checker is red on two pre-existing duplicate IDs, BR-224 and BR-225. BR-238 itself produces only expected pre-implementation citation warnings; the duplicate IDs must be repaired before Gate C.
- The delivered bundle is contract-incomplete: `grpc-external-api.md` claims 60 RPCs, while `market.proto` ends at operation 55 and the referenced `grpc-derived-products.md` is absent.
- Only `SecurityMetadata` and `InstrumentNews` have sufficiently frozen external request contracts in the delivered material. The other opening-critical RPC methods are admitted at runtime, but their external payload schema labels are not delivered and must not be guessed.
- Remote `SecurityMetadata` may truthfully return `complete=false`; missing listing date/price-limit evidence must remain missing. It is safe only for a narrow identity projection, never as complete security metadata.
- External capability evidence confirms admitted/runtime providers for quotes, order books, security metadata, global news, announcements, market announcements, board constituents/memberships, limit pools, instrument news, and upper-limit review. EmQuant quote/book rows are unadmitted diagnostics.
- A single external replacement connection is unsafe with this bundle. The selected integration is now dual: the existing local normalized bridge remains authoritative for contract-incomplete operations, while the authenticated bundle is an additional closed client for fixture-proven operations only. There is no silent fallback after an external TLS/auth/schema/evidence failure.
- The BR duplicate audit established the minimal semantic mapping: announcement dedup remains BR-224, P-01 identity remains BR-225, SignalTracker becomes BR-232, and review/settled-close becomes BR-233. The mechanical re-numbering makes the §2.10 checker pass without business logic changes.
- New files under `docs/` are ignored by the repository's `/docs` rule even though existing documentation is tracked. The new Gate-A design and implementation plan must be force-added intentionally before the required PR; they are not absent or lost.
- The live monitor also fails its v26 dry-run report on `data/dispatcher_log/2026-07-17.jsonl` line 215 (`invalid escape`). The original writer manually replaced only double quotes, so backslash-apostrophe and other arbitrary error text can create invalid JSON. BR-132 correctly rejects the entire corrupted report; the append-only historical evidence must not be edited in place. The writer needs real JSON serialization, and recovery of the old row must remain separately auditable.
- The local converter previously discarded `diagnostic_blocker`, did not reject non-ADMITTED or incomplete responses in its common evidence path, and still accepted the invented compatibility provider `tdx-dev`. These are independent redline gaps exposed by the ExternalV1 review; the response model/converter tests now cover preservation and rejection, with `complete=false` allowed only for the narrow identity projection.
- The live authenticated probe verified the actual ExternalV1 record labels and fields: SecurityMetadata emits `magic.market.security_metadata@1` with `complete=false`, while InstrumentNews emits complete `magic.market.news_item@1`. Both carry production-admitted evidence and the narrow identity converter accepts the live metadata response.
- The historical BR-132 malformed JSONL row is isolated to `data/dispatcher_log/2026-07-17.jsonl:215`. It blocks only the strict dry-run history report; `--review` does not read that path and the reporter catches the error without terminating producers. The original evidence remains untouched.
- The new dispatcher writer uses structured Serde JSON and prevents recurrence; recovery of the historical row must use a separate append-only supersession/quarantine design, never in-place rewriting.
- The existing production binary cannot exercise the new identity path until cutover; its repeated `library transport disabled` errors are expected evidence that the old release is stale, not evidence against the new real canary.
- An independent review found request-set and freshness admission missing from the first identity converter implementation. Those checks are being added before Gate B closes; startup readiness and remote retry evidence were also promoted from lazy/flattened behavior to explicit gates.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Remote capabilities must be probed before selecting a topology | The bundle docs say RPC presence does not imply runtime admission. |
| Contract compatibility must be explicit | The local proto adds operations and a response field not present remotely. |
| Existing local server remains rollback path until remote canaries pass | It currently supplies usable historical bars despite other defects. |
| Readiness is measured by due production push families, not only process health | The user explicitly requires all opening pushes to work. |
| Selected topology: authenticated remote upstream plus a local normalization/evidence boundary | It limits blast radius, keeps current monitor consumers stable, and prevents the remote contract's missing local `source` extension from leaking into decisions. |
| Standing project authorization satisfies the consolidated design decision | The user explicitly requested autonomous execution and no per-step confirmations. |
| Parallelize isolated implementation and contract-audit work | The user explicitly requested parallel execution; file ownership is split to avoid overlapping edits. |
| Never infer missing ExternalV1 schemas from local types | The bundle/version split is objective contract evidence; an explicit blocker is safer than parsing guessed production data. |
| Use a dual local-normalized plus authenticated-external client topology | It permits the proven identity/news contracts to advance without routing quote/board/review traffic through guessed ExternalV1 schemas. |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Planning catchup path under `.codex` was absent | Used the complete `.agents` skill installation. |
| `client-bundle` contains secrets | Read only docs/proto and redacted connection metadata; never print key/token. |
| Capabilities projection failed twice on mixed enum JSON representation | Bound/converted the enum safely; final query succeeded. |
| Permission prompts were perceived as repeated business confirmations | Use already-approved command prefixes and request system escalation only when the platform enforces it. |
| Stale `.superpowers` authority blocked Claude | Archived it without deletion and linked the live paths to the current attested plan. |
| External contract bundle omits schemas/docs for several live RPCs | Limit direct adapters to frozen contracts; keep normalized local bridge or explicitly block until fixtures/docs arrive. |

## Resources
- `client-bundle/grpc-external-api.md`
- `client-bundle/market.proto`
- `src/data_gateway/grpc_source.rs`
- `src/grpc_client/client.rs`
- `src/grpc_server/delegate.rs`
- `src/grpc_server/handlers.rs`
- `docs/superpowers/plans/2026-08-15-p4-migration.md`
