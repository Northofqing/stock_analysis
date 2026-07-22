# Monitor Observation and Remaining Work Handoff

**Updated:** 2026-07-22 Asia/Shanghai

**Repository state:** `master` at `3565bf7`

**Status:** monitoring stopped by explicit user instruction; remaining work is not complete

## Executive status

The release monitor is no longer running. The LaunchAgent was removed and exact process queries
confirmed that neither the monitor nor its `caffeinate` wrapper remained.

Eight defensible active-runtime segments total **28:01:10** (`100,870` seconds). The original
48-hour target was cancelled by the user and must not be reported as complete. Downtime and the
unsupervised interval were excluded rather than inferred from wall-clock time.

The latest `master` includes PRs #2 through #12. The repository safety/coverage closure, README
architecture rewrite, intraday source isolation, degraded-state notifications, persistent Unsafe
reminders, isolated `monitor --test`, source-fact news delivery, announcement relevance filtering,
and terminal/event-audit lifecycle work are merged. The current blocking runtime finding is a
legacy delivery-audit compatibility failure described below.

## What is already merged

Do not duplicate the implementation history; use these authoritative artifacts:

- [Project architecture and operating contract](../../README.md)
- [Business rules BR-135 through BR-142](../business_rules.md)
- [Terminal monitor lifecycle design](../superpowers/specs/2026-07-21-terminal-monitor-lifecycle-design.md)
- [Terminal monitor lifecycle plan and final Gate evidence](../superpowers/plans/2026-07-21-terminal-monitor-lifecycle.md)
- [v16.x completion audit](../v16.x/v16.x-completion-audit-2026-07-19.md)
- [PR #4 — README architecture](https://github.com/Northofqing/stock_analysis/pull/4)
- [PR #5 — intraday source isolation](https://github.com/Northofqing/stock_analysis/pull/5)
- [PR #6 — degraded monitor alerts](https://github.com/Northofqing/stock_analysis/pull/6)
- [PR #7 — persistent Unsafe reminders](https://github.com/Northofqing/stock_analysis/pull/7)
- [PR #8 — isolated monitor test CLI](https://github.com/Northofqing/stock_analysis/pull/8)
- [PR #9 — source-fact news delivery](https://github.com/Northofqing/stock_analysis/pull/9)
- [PR #10 — low-value announcement filtering](https://github.com/Northofqing/stock_analysis/pull/10)
- [PR #11 — terminal and audit lifecycle](https://github.com/Northofqing/stock_analysis/pull/11)
- [PR #12 — merge evidence](https://github.com/Northofqing/stock_analysis/pull/12)

PR #11's final serial Gate D evidence records 80.60% global line coverage and 95.08% registered-core
coverage, with format, strict Clippy, full tests, compliance, release build and isolated canary
passing. Those figures describe commit `3565bf7`; every later behavior change must rerun the gates.

## Sanitized observation evidence

Only aggregates are retained here. Raw log messages, account values, securities, credentials,
notification targets, platform identities and message bodies are intentionally excluded.

| Evidence | Result |
| --- | --- |
| Active segments | 8 |
| Cumulative active runtime | 28:01:10 |
| Final segment | 2026-07-22 09:46:33–10:04:41 +08:00 |
| Final-segment records | 4,513 |
| Final-segment WARN / ERROR | 2,923 / 323 |
| `push.delivery.audit` errors | 90 |
| Panic/fatal markers | 0 |
| Database-lock markers | 0 |
| `banner unavailable` markers | 0 |
| Immutable yearly delivery-audit rows | 568, unchanged during the final segment |

The process remained alive, but a live process is not sufficient acceptance. The unchanged
authority file and repeated audit errors mean governed delivery could not complete its durable
2.7 evidence path.

## P0 — repair the legacy delivery-audit prefix

### Verified failure

The yearly authority file contains 568 pre-BR-142 legacy records and no v2 record. Thirty legacy
rows encode an absent subject as byte-exact empty strings (`""`) in both `payload.code` and
`envelope.entity_key`. The current legacy parser rejects the empty `code` before it can validate
that the paired identity fields agree. No evidence supports treating whitespace-only strings as
the same historical representation.

The first append therefore fails full-prefix validation. `AuditDispatcher` then poisons its
in-memory state, so later governed deliveries keep returning audit failure until restart. The sink
is currently invoked before the authority append, which can produce an external transport receipt
followed by application-level `SinkError` and dedup rollback. Do not equate transport acceptance
with a successfully governed push.

Relevant code:

- `src/event/push_record.rs`
- `src/event/dispatcher.rs`
- `src/bin/monitor/notify.rs`
- `docs/business_rules.md` (BR-142)

### Required repair boundary

Use a persisted-legacy-only compatibility parser. It may interpret **paired byte-exact empty**
legacy `code/entity_key` values as an absent subject after the existing record hash, parent link,
closed schema and complete legacy payload checks pass. Keep the general parser strict, keep all v2
fields strict, and reject one-sided empty values, unequal identities and every whitespace-only
identity (including equal pairs).

Do **not** delete, truncate, rewrite or migrate the existing audit file. Do **not** weaken v2
domain separation or allow a legacy row after the first v2 row.

Minimum regression evidence:

1. The ordinary parser still rejects empty and whitespace-only identity strings.
2. The persisted-legacy parser accepts only an equal `""`/`""` pair and normalizes it to absence.
3. Null/empty, empty/nonempty, unequal and paired-whitespace values are rejected.
4. A valid legacy chain containing a paired-empty row accepts its first v2 append.
5. The byte prefix of the legacy fixture is unchanged after append.
6. A bounded post-merge canary advances the authority row count without an audit error and without
   exposing a payload. Do not restart continuous monitoring unless the user requests it again.

Update BR-142 and the design before implementation, then use one small PR with complete Gate
evidence. This is the first task for the next session.

## Remaining verified or unclosed work

### P1 — paper portfolio must be independent of the real account

The persisted paper ledger exists, but intraday paper decisions and exits still derive risk context
from the real-account banner and same-day real account ledger/position projection. When real-account
evidence is incomplete, the virtual portfolio can be present yet unable to operate.

Create a paper-account projection from immutable `Filled` paper trades plus an explicit, real
configured starting-capital fact. Never borrow real-account cash, clear filled history, infer broker
freshness from a local update timestamp, or invent missing capital.

### P1 — verify operational alerts remain account-independent

Source-fact news delivery is intentionally independent of account/banner availability, while live
trade authorization stays fail-closed. The earlier diagnosis also found DataMode operational alerts
coupled to banner storage. No post-merge evidence proves every operational alert has been decoupled.
Add a regression showing that an unavailable account snapshot cannot suppress a DataMode/system
health alert, without relaxing any trade or price-action gate.

### P1 — make strict review useful under partial real evidence

`monitor --review` correctly fails closed when all strict report tasks lack usable evidence, but
observed failures still include unavailable public providers, incomplete industry-chain evidence,
missing market fields and explicitly disabled outcome sources. Preserve per-task isolation from
BR-140: one bad symbol/source must not erase other complete reports, and zero delivery must remain
an explicit non-zero result when nothing is supportable.

Re-evaluate this path after the P0 audit repair; otherwise delivery failures and source failures are
confounded.

### P2 — market-rule-aware K-line validation

The generic adjacent-change guard cannot be relaxed globally. Board-specific limits, IPO/listing
windows, corporate actions and suspension/resumption need explicit market metadata and tests.
科创板/创业板 limits and new-listing behavior must not be inferred from a single percentage field.
Provider failures must stay isolated per symbol, and a source without the mandatory real `amount`
field must not silently fill it with zero or an estimate.

### P2 — strategy and product backlog requested by the user

These items were discussed but are not complete and need separate Gate-A designs:

- configurable self-managed recovery/T strategy: prioritize evidence-backed intraday T
  opportunities for losing holdings; consider liquidation/switching only with an explicitly
  stronger alternative and no automatic hard-coded stop-loss;
- four session cadences: auction fast decisions, high-frequency intraday T alerts, medium-frequency
  market-direction guidance, and after-close research persisted for next-auction confirmation;
- one-day advance notice and operation guidance for relevant futures delivery dates;
- stronger backtest/review truth: realistic fill model, T+1, board/listing limits, fees/slippage,
  corporate-action continuity, decision/outcome linkage and reproducible data snapshots;
- brokerage integration for continuously fresh position/cash/NAV evidence. The existing local
  account snapshot is historical evidence, not a broker connection and not a 30-second live gate.

Do not combine these into the P0 audit hotfix.

## Safe continuation sequence

1. Read `AGENTS.md`, `docs/ENGINEERING_RULES_V2.md`, `.github/copilot-instructions.md` and
   `CLAUDE.md`; output the mandatory pre-flight.
2. Start from the latest remote `master` and preserve unrelated/untracked user files.
3. Update BR-142 plus the terminal lifecycle design before code.
4. Add the audit regressions first, prove RED for the legacy-prefix fixture, then implement the
   persisted-legacy-only compatibility seam.
5. Run focused tests, format, strict Clippy, serial full tests, compliance, serial coverage threshold
   enforcement, release build and `git diff --check`.
6. Obtain an independent zero-blocker review and merge through a PR with every required evidence
   field.
7. Run only a bounded, payload-free canary unless the user separately reauthorizes continuous
   monitoring.
8. Address P1/P2 items as separate vertical PRs in the order above.

## Suggested skills

- `planning-with-files` — retain state across the multi-step audit repair.
- `systematic-debugging` or `diagnosing-bugs` — preserve the symptom-to-root-cause evidence chain.
- `brainstorming` — mandatory before changing the legacy/v2 compatibility behavior.
- `tdd` — lock the immutable-prefix behavior with a failing fixture before implementation.
- `requesting-code-review` — obtain the required independent Gate review before merge.
- `handoff` — refresh this document if the work changes sessions again.

## Privacy and rollback

- Never commit the raw monitor log, local account evidence, database, audit payloads or notification
  configuration.
- Treat the existing authority file as immutable evidence; read-only aggregate validation is
  allowed, mutation is not.
- Roll back any merged repair with `git revert <merge-commit>` and rebuild. Do not use history
  rewrites or database/audit truncation as rollback.
- The stopped LaunchAgent must remain absent unless the user explicitly asks to resume monitoring.
