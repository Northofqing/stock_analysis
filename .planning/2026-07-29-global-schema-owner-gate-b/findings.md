# Findings

- Frozen identity constants are `STSA` / `1398035265` and global generation `1`.
- `rusqlite` and `fs2` already exist as dependencies.
- Existing offline migration code contains useful Unix no-follow patterns but is out of scope and
  remains unchanged.
- The production database path is already fixed elsewhere as `data/stock_analysis.db`.
- BR-180 already registers the exact global identity and maintenance-lock constraints used by this
  slice.
