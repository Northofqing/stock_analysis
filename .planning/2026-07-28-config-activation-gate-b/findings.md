# Findings & Decisions

## Requirements
- Implement skill is mandatory; use TDD-compatible seams where possible and request review after implementation.
- Only one public zero-argument bootstrap may read real `args_os`; parser, owner and process
  `OnceLock` stay in one library-private module.
- The opaque binding selects and constructs production/test database, audit, lock, sink and
  coordinator identities; callers cannot pass mode, argv, path, manager or nonce.
- New provider work accepts only `VerifiedCurrentConfigActivation`; exact recovery accepts only
  `VerifiedRecoveryConfigActivation` sealed from a receipt-verified historical registry.
- Config activation and registry failures are fatal before providers; no empty/default/current
  fallback.
- UUID/time allocation for new activation occurs only inside `BEGIN IMMEDIATE` after exact rechecks.
- This lane must not run Cargo; use targeted rustfmt and static/diff/BR checks.

## Research Findings
- Approved design is `docs/superpowers/specs/2026-07-28-config-activation-owner-design.md`.
- BR-179 is registered in `docs/business_rules.md`.
- Existing code inventory is in progress locally and via one read-only parallel reviewer.
- The target files are large and active: config activation 1,616 lines, persistence owner 374,
  selection-v2 repository 7,716, selection-v2 schema 4,610, database manager 2,863 and monitor
  10,176 lines. Scope must stay narrow and preserve concurrent edits.
- Monitor currently reads argv repeatedly (`std::env::args`) around startup and initializes
  `DatabaseManager` from a caller-selected path near `main.rs:3175-3438`, violating the new
  process-binding seam.
- `DatabaseManager::run_migrations` still calls legacy `selection::create_schema` at
  `src/database/mod.rs:588`, which mutates the legacy trigger set before the activation owner.
- `SelectionV2PersistenceOwner::commit_config_activation` is public and obtains the global
  database itself; it already has envelope → Prepared → stage → Committed → receipt choreography
  suitable for a locked owner-internal seam.
- `selection_v2` already persists recovery envelopes, manifests and receipts with
  config-activation linkage. Historical registry work should verify/derive from these rows rather
  than add a mutable current-config table.
- `config_activation_v2` is deterministic and storage-free but its trusted boundary is inverted:
  `ConfigActivationPreparationContext::checked_in` accepts caller stage ID, activation/envelope
  times and arbitrary legacy snapshot, while `prepare_config_activation` accepts arbitrary root.
- `PreparedConfigActivation` already hides stage/run/envelope preimages and is the correct payload
  to deepen, but it lacks process/database binding hashes required by BR-179.
- Existing preparation validates activation chronology, exact activation-file hash, board artifact,
  executable revision and canonical recovery envelope. These validators should be reused under an
  owner-only fixed-root material loader rather than reimplemented.
- Existing `validate_legacy_cutover_snapshot` only requires domain/hash/sorted arbitrary table
  names; it does not enforce the exact seven-table contract, matching the Gate A blocker.
- Current tests create arbitrary fixture roots and caller-owned contexts. They remain useful for
  pure material validation but cannot prove the public process/bootstrap boundary; new process
  integration tests must be separate-child tests.
- `SelectionV2PersistenceOwner` currently owns production DB/audit acquisition and all stage
  timestamps. Its internal `commit_with_owned_resources` already holds the audit session through
  receipt readback and separates envelope, Prepared, stage, Committed and receipt operations.
- Config activation enters persistence through public `commit_config_activation`; the owner slice
  can first make this crate-private and route it only from `ConfigActivationOwner`, without
  redesigning ingress/outcome persistence in the same patch.
- `DatabaseManager` has one global `OnceCell`, but public `init(Option<PathBuf>)` permits caller path
  selection and special `cfg(test)` behavior ignores caller paths. The BR-179 process binding must
  own the only selection bootstrap call into DB initialization while preserving unrelated generic
  DB consumers until a later narrowing.
- Monitor currently captures env before dotenv, parses strings itself, sets mode env variables,
  constructs event/push resources and providers before database activation, and chooses test/
  production paths from env/caller state. Full monitor cutover is materially larger than adding the
  capability types.
- `run_migrations` creates legacy selection schema before activation; removing that call
  immediately would break all existing consumers unless the new owner initializes/cuts over the
  schema before any such consumer. This change must be atomic with monitor wiring, not standalone.
- The checked-in `config/selection/selection_activation.v1.json` is absent and
  `provider_board_bindings.v1.json` is `direct_only_unverified`. A production owner must fail
  closed; it cannot manufacture a current activation from this tree.
- Monitor now has no direct argv read and no direct `DatabaseManager::init` call. The only
  monitor/selection `args_os` read is in `selection::process_bootstrap`.
- Existing child-process tests encoded the removed `DATABASE_PATH` authority. They now verify that
  hostile/caller values, including `:memory:` and blocked parents, are ignored by test binding.
- The pre-existing untracked `process_bootstrap.rs` is not Gate A compliant: it calls
  `DatabaseManager::init_selection_bound`, exports database paths/mode through environment
  variables, constructs filesystem namespaces, and installs operational success without the
  required global lease/catalog/config/global-receipt classification.
- Gate A explicitly requires help/version to install storage-free terminal state, invalid argv to
  install storage-free rejected state, operational startup to install only exact Amended or exact
  recovery-only state, and all repeat calls to fail through the one outer process `OnceLock`.
- The assigned first slice therefore must return typed operational `Unavailable` until the private
  `GlobalSchemaVersionOwner` factory exists; doing so is a real fail-closed boundary, not a fake
  provider/database implementation.
- Existing parser unit tests are useful but insufficient. Exact executable behavior must be tested
  in child processes using real argv; pure argv-slice tests remain module-private.
- `event::cli::parse_args` recognizes help but not version and permits help to be combined with
  operational flags. The process owner must recognize exact `--help`/`-h` and
  `--version`/`-V` before delegation and reject mixed terminal/operational argv.
- The legacy `risk::env_guard` infers mode from `STOCK_ENV_MODE`, `cfg(test)` and executable
  location and accepts any `TEST_CODE*` prefix. It cannot serve as BR-179 proof. The bootstrap
  binding needs a private exact contract: production accepts only six ASCII digits and test accepts
  only `TEST_CODE_` plus six ASCII digits.
- `monitor` currently handles only `is_help()` before continuing. The bootstrap capability may
  expose a distinct version predicate, but monitor integration belongs to its separate owner; the
  first slice must at least keep version storage-free and must not construct any operational
  resource.
- Independent scoped review reported no Critical issues and two Important integration gaps:
  monitor must consume `is_version()` before audit/storage, and executable tests must prove the
  fixed manifest-root production DB/audit/lock identities remain unchanged rather than checking
  only hostile CWD paths.
- Process tests now fingerprint the manifest-root production database, WAL/SHM, Magiclaw database
  and production selection-audit/lock files before and after every child process. Version has its
  own terminal child-process test; main-owner dispatch remains pending outside this lane.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Prefer deepening existing v2 envelope/manifest/receipt code | Avoid a second authority and preserve audited crash choreography |
| Keep tests process-isolated for argv/mode binding | A process-global `OnceLock` cannot be safely reset or caller-seeded |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Root planning files belong to the broader migration | Created isolated plan `.planning/2026-07-28-config-activation-gate-b/` |

## Resources
- `AGENTS.md`
- `docs/ENGINEERING_RULES_V2.md`
- `.github/copilot-instructions.md`
- `CLAUDE.md`
- `docs/superpowers/specs/2026-07-28-config-activation-owner-design.md`
