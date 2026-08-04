# Full-market ranking retirement task plan

- [x] Gate A: record the pinned-provider capability evidence and selected retirement design.
- [x] Register BR-190 before changing filter/scheduler behavior.
- [x] Gate B: remove the two review volume-ratio calls.
- [x] Gate B: retire the periodic I-10 and BR-073 scheduler call paths.
- [x] Gate B: make non-isolated `--test` record capability-unavailable instead of calling providers.
- [x] Gate B: delete the two permanent-error facades and orphaned projection/template code.
- [x] Validate with targeted `rustfmt`, multiline static searches, and `git diff --check`.

## Constraints

- Do not edit `src/market_analyzer/sector_history.rs`.
- Do not edit `src/opportunity/news_ranker.rs`.
- Do not run Cargo in this slice.
- Preserve unrelated dirty-worktree changes.
