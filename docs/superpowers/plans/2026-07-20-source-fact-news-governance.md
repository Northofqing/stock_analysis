# Source-fact news governance implementation plan

## Task 1: RED evidence contract

- Add `SourceFactEvidence` constructor tests for the whitelist, kind binding, identity/code rules,
  non-empty provenance, bounds, stale flag, provider publication date, and future timestamp. Extend `NormalizedSourceEvent`
  with the real adapter observation and provider publication date; derive stale at construction and
  revalidation instead of accepting `now`/`false` placeholders.
- Add red tests proving financial `REPORT_DATE` cannot authorize freshness, provider `NOTICE_DATE`
  is preserved, and every registered flash provider retains a full timestamp rather than `HH:MM`.
- Prove the current generic path denies an earnings fact at DataMode Down.
- Require the new typed path to approve it with a `NewsCatalyst` payload while a generic mixed-news
  kind remains denied.

## Task 2: Prepared-event gate

- Refactor `v14_gate_with_sub_kind` into a default wrapper over one private prepared-event gate.
- Add the source-fact constructor and source profile inside `v14_adapter`.
- Hash the provider governance identity into `SignalEvent.event_id`; add the narrow L4 dispatcher
  API that uses that event ID for reserve/commit while leaving `SignalEvent.code` as the real
  optional security code consumed by delivery audit.
- Keep every generic profile and caller unchanged.

## Task 3: Sole delivery entry and source adapters

- Add `notify::push_source_fact_v3`, delegating to the existing common governor and delivery tail.
- Route the five normalized source-fact kinds through it; keep MarketAction on generic governance.
- Extend critical FlashDecision to preserve event identity, headline, source, timestamp, and stale
  state through the typed entry. Derive flash freshness from the provider date/timestamp and reject
  malformed/stale/future events before both critical and aggregate buffers. Preserve the real
  `SearchResult.source` as provenance. Keep aggregated flash generic and fail closed.
- Carry the financial provider `NOTICE_DATE` independently from the accounting `REPORT_DATE`, and
  carry the real financial/report provider name rather than classifier/tracker labels.

## Task 4: Focused validation

```bash
cargo test --bin monitor source_fact -- --nocapture
cargo test --bin monitor v17_sources::tests -- --nocapture
cargo test --bin monitor news_aggregator_init::tests -- --nocapture
cargo test --lib news::aggregator::source_event::tests -- --nocapture
```

## Task 5: Repository gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

## Task 6: PR and production acceptance

- Complete all required PR evidence fields and obtain independent zero-blocker review.
- Merge, fast-forward local master, rebuild, and restart only the verified process.
- Compare aggregate source-fact L7, event-bus, and immutable-audit counts; require explicit source
  failures and zero banner/sink/audit/panic/fatal regressions.
