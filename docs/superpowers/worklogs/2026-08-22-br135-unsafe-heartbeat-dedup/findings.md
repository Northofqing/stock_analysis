# Findings

- Root cause: BR-135 explicitly routes unchanged Unsafe through `T-02-data-mode-reminder` every 30 minutes.
- Correct documentation locations: authoritative rule in `docs/business_rules.md`; new design and plan under dated `docs/superpowers/specs` and `docs/superpowers/plans`; preserve the 2026-07-20 design as history.
- Existing `event::publish_delivery` provides the required synchronized immutable audit path.
- Baseline before BR-135 edits: workspace tests have two pre-existing monitor source-text count failures caused by BR-246 test literals inside the counted source prefix.
- Baseline `cargo fmt --all -- --check` is blocked by unrelated, pre-existing formatting drift in recent 15:05/G5b and other files; the BR-135 files and changed `main.rs` region are formatted.
- Baseline strict Clippy is blocked by four pre-existing lints in `src/performance/attribution.rs` and `src/performance/report.rs`; `monitor` passes after diagnostic-only command-line allowances for exactly those baseline lint classes, with no source allowance added.
- The worktree uses only the real upstream `client-bundle/market.proto` contract symlink and an empty ignored `data/` namespace; no credential or production database was copied.
