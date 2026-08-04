# Findings

- `monitor --review` completes and shuts down normally; R-04/R-09 delivered state can be reused.
- `monitor --test --push-dry-run` renders all 48 isolated TEST_CODE templates with no external process.
- Bare `monitor --test` intentionally refuses live acceptance without an independently configured non-production Feishu target; the production target cannot be reused under rule 2.5.
- BR-171 stable confirmation v2 survives source reacquisition batch rotation, and admitted gateway authority is preserved through K-line persistence.
- Real daily backfill for 688548/688690 reaches 2026-08-03 and the data-freshness check passes.
- The earlier focused fixture mismatch was a stale assertion, not a production renderer defect;
  the provider-shaped fact identity remains canonical and the zero-I/O test now reflects it.
- Full workspace tests, release build, Clippy, formatting, compliance, `--review`, and the
  complete isolated template dry-run all pass.
- Bare `--test` remains externally blocked until an independent non-production Feishu target
  is configured and allowlisted; automatically reusing production would violate rule 2.5.
- Repository Gate D remains incomplete because measured global/core coverage is 78.66%/78.17%
  against required 80%/95%.
- BR-196 local authority audit found no explicit `BR196_LIVE_FEISHU_ACCEPTANCE`, tenant,
  app, app-secret, or conversation variables in the process environment or repository `.env`.
  The release-pinned non-production allowlist is intentionally empty, while the single known
  generic target identity is listed in `production_deny`. Bare `--test` therefore has no safe
  identity it can authorize; this is external target provisioning, not a renderer failure.
- The exact bare command reproduces deterministically in about six seconds: without opt-in it
  exits 2 before target resolution; with opt-in only, it resolves the existing generic target
  and exits 2 as `production_feishu_target_rejected`. Both paths attempt zero external sends.
- The stock project pins every Magic crate to revision `5f1ce936...`; the adjacent upstream
  repository is at `06b4d0f6...`. Both the pinned and adjacent current CFFEX implementation keep
  `calendar_capabilities().futures_delivery=false`: the official notice parser/probe exists, but
  the production trait intentionally returns Unsupported until bounded HTTPS live admission.
  R-08 cannot safely treat that diagnostic parser as a production delivery calendar.
- R-03 is intentionally pinned to `LegacyAccountGate`; the central review join has no R-03
  source branch. BR-204's replacement is still Gate-A-only because this branch lacks BR-203
  P1/P3/P4 authority, four required P4 verifier files, and a fresh exact-packet C0/I0/M0 review.
- The database does contain seven append-only user-confirmed historical snapshots through
  2026-08-03 with seven children each. They can later support a historical, non-transactional
  monitored universe, but do not prove broker identity, cash, NAV, or 30-second freshness.
  The current reader lacks exact-date selection/hash revalidation, and current watchlist loading
  drops evidence and conflates NotConfigured with VerifiedEmpty; using them directly would violate
  2.1/2.2/2.4 and BR-204.
- The isolated `global_market.rs` coverage slice adds ten passing source-admission tests without
  changing production semantics. It raises that module from 42.57% (192/451) to 83.76% (691/825)
  and reduces its uncovered lines by 125. Repository coverage nevertheless remains below Gate D:
  global 151803/192724 = 78.77%; core 123672/157937 = 78.30%.
- The post-integration `monitor --review` run exits 0. It acquires real Eastmoney, Magic TDX,
  CNInfo and Sina gateway evidence; R-04 and R-09 resolve as durable delivered results. R-03,
  R-08 and A-10 remain explicit fail-closed outcomes rather than fabricated review content.
- The post-integration isolated template run exits 0 with 48/48 families, three batches, 6/6
  smoke checks, zero failed families and zero external process attempts. The exact bare `--test`
  still exits 2 at `live_acceptance_not_opted_in`, before target resolution or any sink call.
- BR-206 closes the intermittent durable-delivery startup failure caused by SQLite's Unix VFS
  unused-fd pool. A serialized `Connection::open` can reuse an existing exact-inode descriptor and
  create no process-fd delta; retaining unproved connections and retrying up to the exact evidence
  bound makes descriptor proof observable without allowing SQL/PRAGMA before attestation.
- BR-206 evidence is green: 100/100 focused process runs, the exact full workspace test command,
  strict Clippy, formatting, compliance and release build all pass.
- The final real `--review` command exits 0 and terminates normally. Its remaining R-03/R-08/A-10
  task failures are source/policy capability gaps, not a monitor process failure, and no substitute
  data is synthesized.
- The final exact bare `--test` run proves the only remaining test-command blocker is external:
  no independent non-production tenant/app/conversation identity is configured or allowlisted.
  The full 48-template TEST_CODE path is locally green through `--push-dry-run`.
- BR-207 fixes a real review lifecycle defect without weakening quiet-hour governance: an A-10
  `quiet_hour` denial is now retryable. The provider data remains real and audited; the task is
  deferred instead of falsely becoming terminal.
- R-03 cannot legally be switched on yet: the real historical position rows exist, but BR-204
  explicitly blocks Gate B until BR-203 P4 and a fresh independent C0/I0/M0 review are complete.
- R-08 is correctly wired to the formal Magic calendar trait. The pinned and adjacent upstream
  versions both advertise `futures_delivery=false`; using the diagnostic CFFEX probe in production
  would violate the production-source admission rules.
- Magic Tencent's canonical realtime provenance uses unsigned fractional Unix seconds, while the
  local realtime gateway had drifted to RFC3339-only reparsing. BR-208 restores parity with the
  upstream `EvidenceTimestamp` contract and retains exact integer/nanosecond conversion.
- The BR-208 live closed-session proof changed the same observation from `invalid_evidence` to the
  truthful `quote_stale` outcome. This is expected outside the five-second market-data window and
  confirms that source freshness remains enforced.
- The latest exact full-workspace test command passes after BR-208, as do strict Clippy and the
  complete compliance suite. Release status is still In Progress because Gate D coverage and an
  independent open-market live-session proof are not complete.
- The requested production review entry is operational and exits 0. The requested 48-template
  TEST_CODE catalog is operational in explicit dry-run mode. Live test delivery remains blocked
  before transport because no independent non-production Feishu identity has been provisioned;
  the production identity cannot be reused under red line 2.5.
- Current A-10 sequencing confirms BR-207 is only a terminal-classification repair:
  `dispatch_catalyst_review_daily_outcome` remains the acquisition owner, while `quiet_hour` is
  first observed later in L5. The 2026-08-04 review therefore admitted Eastmoney/Magic TDX batches
  before returning a retryable failed outcome. Phase 7 must prove and register an absolute
  provider-free defer contract before changing this order.
- `ExpectedWait` cannot safely represent the A-10 manual-review quiet defer when
  `observed_at.date() > review_date`: schedule audit derives `next_attempt` from the business date,
  producing a past timestamp. A correct future contract needs an absolute `DeferredUntil`, must
  state `automatic_retry=false/manual_reinvoke_required=true`, and must attribute the decision to
  review preflight rather than a provider that was never called.
- Fresh normal-monitor evidence is operationally healthy: TDX board membership resolves all seven
  actual positions, DataMode reaches validated Feishu delivery/audit, NewsMonitor initializes, and
  shutdown is graceful. Paper evaluation outside the market remains unavailable only because the
  admitted Tencent quote is stale by hours; no runtime panic or fabricated fallback occurs.
- The configured generic Feishu identity exactly matches the BR-196 production deny hash. Bare
  live test delivery therefore cannot be made green by setting the opt-in alone; a separate test
  conversation is an external prerequisite.
- Current R-08 failure is upstream capability authority, not local wiring: official-notice parsing
  exists in Magic only as a diagnostic path, while the formal production calendar trait returns
  `Unsupported` in both pinned and adjacent revisions.
- Gate-D coverage is presently unavailable as authority, not merely below threshold. The latest
  JSON predates changed sources, has no commit/tree binding, and its checker accepts caller-lowered
  thresholds. The next release slice must first repair coverage authority before adding the
  selection-v2 read/recovery E2E coverage proposed by the independent audit.
- Phase 8 is explicitly parallel: exact test CLI, R-03 and R-08 are independent investigation
  seams. The existing dirty worktree is user-owned shared state, so each seam must avoid broad
  cleanup and preserve unrelated changes while producing scoped evidence.
- BR-196 §6/§7 is authoritative for the current bare-test behavior: without an exact opt-in and
  release-pinned independent non-production Feishu identity, bare `--test` must exit 2 and cannot
  fall back to dry-run or the production target. The complete no-transport acceptance command is
  therefore the explicit `--test --push-dry-run` path, which is green on the current tree.
- Current normal monitor operation is healthy enough for business execution: the startup and
  scheduler path runs, real Magic TDX/Tencent evidence is audited, DataMode reaches Feishu, and
  SIGINT is graceful. Off-hours stale quotes and unavailable broker freshness remain explicit
  degraded evidence rather than process failure.
- Phase-9 coverage inspection confirms the authority defect is concrete: the current checker
  accepts caller-supplied `--global-min`/`--core-min`, classifies core through an incomplete fixed
  prefix tuple, and the repository regression helper intentionally invokes it with core=75. The
  current BR-202 design labels itself an unaccepted sixth-remediation Gate-A candidate and requires
  a fresh independent C0/I0 review plus a separate implementation plan before Gate B. Therefore a
  quick checker edit would violate the repository's sequential gate rather than close Gate D.
- The dirty worktree spans the data migration, selection, monitor and release surfaces. Any Phase-9
  implementation must use exact path allowlists and must not stage, reset or normalize unrelated
  user changes.
- BR-203 P4 is not merely an unchecked command: all four specified P4 artifacts are absent
  (`check_br194_recovery_focused.sh`, `check_br194_bounded_startup.sh`, its structured Python
  verifier, and `tests/br203_recovery_verifiers.rs`). The active BR-203 row still says its docs-only
  P0-A3 correction requires independent acceptance and a commit before Gate B. R-03 cannot lawfully
  consume BR-204 until that sequential authority chain is closed.
- Current-day production evidence is present and structurally readable: 2026-08-04 has nine push-log
  artifacts, nine `push.delivery.audit` event-bus rows, and 48 dispatcher JSONL rows; the complete
  dispatcher file parses with `jq` without an invalid line. This supports normal-monitor delivery
  health independently of the BR-196 test-target blocker.
- Phase-9 live recovery found no running project monitor and no LaunchAgent/crontab supervisor.
  Starting the current release artifact under the real network boundary succeeds: all seven actual
  positions receive admitted TDX board memberships, Tencent admits the watchlist identity batch,
  and DataMode receives a validated Feishu receipt. The same artifact under the restricted command
  sandbox fails DNS for every provider and Feishu, which is an execution-boundary failure rather
  than a provider or monitor regression.
- An initial direct `backfill_daily` invocation without `STOCK_DB` exposed an existing operator
  hazard: the binary defaults to legacy `data/stock.db`, while monitor and the formal one-shot script
  use `data/stock_analysis.db`. That first run truthfully failed two BR-171 decisions and did not
  repair the monitor database. Re-running through `tools/one_shot/backfill_daily.sh` bound the
  intended database and admitted 39/39 symbols using its existing immutable confirmations. The
  seven actual positions now each have 90 real rows through 2026-08-03 in the monitor database.
- The fresh coverage diagnostic completed successfully with every test target green and wrote
  `target/coverage/coverage-phase9.json`. It remains diagnostic-only because the independent
  BR-202 Gate-A review is RED and the current checker is not release authority.
- The post-backfill production review exits 0 in nine seconds, reuses durable R-04/R-09 decisions,
  acquires fresh Eastmoney/TDX/CNInfo evidence and commits another validated A-10 Feishu receipt.
  R-03 and R-08 remain explicit capability failures rather than preventing the review command.
- The post-backfill isolated template command exits 0 with manifest `BR196_V2`, all 48 active
  families, 6/6 governance smoke checks and zero external transport. Bare live `--test` remains
  intentionally unavailable without an independently provisioned non-production Feishu target.
- Continued live-monitor evidence exposed a new production consumer drift: CNInfo acquisition
  admits 100 announcements with Magic observation evidence `1785799426.000037000`, but
  `v17_sources::route_announcement_batch` reparses the field with RFC3339-only chrono and marks
  all 100 inputs failed. The upstream batch is real and complete; the local consumer rejects an
  instant encoding already accepted by Magic Core. This is analogous to BR-208 but outside its
  realtime-quote-only scope and requires a shared evidence-time authority rather than duplicated
  parsing or evidence normalization.
- BR-210 follow-up audit found the same RFC3339-only reparsing after otherwise valid Magic
  admission in four active seams: monitor candidate quote projection, Sina financial projection,
  Eastmoney consensus/source-event construction, and latest mixed-batch observation selection.
  R-04/R-08/R-09 provider-specific parsers match their current provider contracts and are excluded.
  The BR-210 Gate-A scope now explicitly covers these four seams; all reuse the one exact parser.
- BR-138 needed two independent handled states: lifecycle suppression and ordinary classification
  rejection. Conflating them made valid keyword-unmatched provider rows look previously processed
  and allowed invalid Skip-shaped rows to bypass source validation. The route now validates first
  and reports the two states separately without changing freshness or missing-data red lines.
- The current monitor binary is operational, but several data capabilities still degrade honestly:
  Magic TDX realtime transport can fall back slowly, post-close/premarket quotes can exceed the
  five-second gate, TopStock lacks same-batch volume-ratio/main-flow evidence, holding batches can
  lack source time, and FullMarketRankings is not yet available. These do not crash startup or the
  review/test catalog, but they can suppress individual intraday recommendations.
