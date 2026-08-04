# Findings

## Reproducible facts

- `sector_history::append_today*` has no production caller.
- The JSONL record has no immutable provider, source time, observation time,
  batch identity, or content hash.
- Its whole-file continuity check is incompatible with changing top-N board
  membership and lets one absent board/date poison the entire file.
- `news_ranker::shadow_rank_hits` has no production caller; monitor only logs
  that it is disabled because it would build a zero/default market context.
- `news_audit` has no production caller and its local warn-only JSONL is not a
  BR-007 durable audit.
- `HeatStage` is the only `news_ranker` type still consumed elsewhere, by
  review rendering/aggregation modules.
- Selection-v2 is the formal event-scoped candidate chain and does not depend
  on these modules.

## Data disposition

Existing `data/sector_history.jsonl` and `data/news_rank_audit*.jsonl` files are
historical local artifacts. They remain untouched, are not authoritative
inputs, and are not eligible for replay into selection-v2.

