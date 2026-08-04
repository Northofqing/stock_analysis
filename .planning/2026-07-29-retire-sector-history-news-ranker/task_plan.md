# Retire sector-history JSONL and shadow NewsRanker

## Goal

Remove the unowned `sector_history` JSONL and shadow `news_ranker` /
`news_audit` production surfaces without changing the selection-v2 candidate
pipeline or deleting historical user data.

## Scope

- Gate A design and BR-191 registration.
- Remove module exports and the isolated monitor log residue.
- Move the still-used `HeatStage` vocabulary into the review domain.
- Remove dead stage-scoring code and tests that depended on the retired
  history/ranker.
- Correct active comments that still describe the retired chain.
- Preserve `data/sector_history.jsonl` and `data/news_rank_audit*.jsonl` as
  isolated, non-authoritative historical files.

## Phases

1. [completed] Register Gate A design and BR-191 before source edits.
2. [completed] Retire dead source modules and relocate the review-owned enum.
3. [completed] Remove active stale references and configuration documentation.
4. [completed] Run rustfmt/static searches and hand full Cargo gates to root.

## Definition of Done

- No active Rust module exports or production call/log path reference
  `sector_history`, `news_ranker`, `news_audit`, or their environment switches.
- `HeatStage` consumers compile conceptually against the review module.
- Selection-v2 source paths are unchanged.
- Existing user `data/` files are not modified or deleted.
- Scoped rustfmt, `git diff --check`, and targeted `rg` checks pass.
