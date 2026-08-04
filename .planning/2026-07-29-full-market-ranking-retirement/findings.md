# Findings

- The pinned `magic-market-data-rs` revision exposes a typed full-market ranking
  contract, but its provider capability is not live-admitted.
- Bounded live evidence records `f10` absent for volume-ratio ranking and `f62`
  absent for main-net-inflow ranking. The provider capability remains `false`.
- `stock_analysis` still had two local facade functions which could only return
  `Err`, while review, periodic, post-close, and non-isolated test paths kept
  calling them.
- A provider capability failure is neither a verified empty ranking nor a
  transient retry condition.
- Dragon-tiger rows, board rankings, and explicit-code company statistics have
  different universes and meanings; none can substitute for a full-market stock
  ranking.

