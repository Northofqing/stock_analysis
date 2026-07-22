# Engineering Rules V2

This document is the required engineering companion to repository-root `AGENTS.md`.
`AGENTS.md` remains authoritative. If this file, a design document, or an implementation
conflicts with it, use the precedence defined in `AGENTS.md`: data safety, fund safety,
process compliance, then development efficiency.

## 1. Evidence before decisions

Production decisions may consume only typed evidence that retains:

- provider/source identity;
- provider publication or observation time when the provider supplies it;
- local acquisition time, kept distinct from provider time;
- immutable batch or event identity;
- completeness, freshness, and validation status.

Missing provider time stays absent. Database write time, process time, a locally refreshed
projection, cost price, zero, an empty collection, or a placeholder must not replace missing
evidence. A verified empty batch is different from an unavailable source.

## 2. Failure boundaries

Every external acquisition and persistence boundary returns an explicit result. A failure may
be isolated only when independent records or components have their own complete evidence.
Partial reports must declare excluded components. If no independent complete component remains,
the operation fails and does not render or push.

Production code must not catch an error and return mock data, defaults, or an unqualified empty
collection. Retryability is recorded separately from the reason category.

## 3. Test and production isolation

Tests use `TEST_CODE` identities and test-only databases, audit directories, logs, and sinks.
Runtime path overrides are namespace roots: the program appends a `test` or `prod` namespace
where a shared override could otherwise collapse the boundary. Production rejects test orders;
tests reject real-symbol orders.

## 4. Audit contract

Critical data flow, governance, push delivery, review decisions, and every order attempt leave a
durable audit record. Audit records include source, real observation/as-of time when known,
decision status, retryability, rule IDs, and a non-reversible identity hash.

Append-only JSONL audit writers must:

1. take a cross-process lock;
2. validate the complete existing chain and trailing-record boundary;
3. serialize each new batch before opening the append handle;
4. append while the lock is held;
5. flush and `sync_data` before reporting success;
6. fail closed on a partial tail, hash mismatch, lock, write, or sync failure;
7. keep test and production paths physically separate;
8. have no retention policy shorter than five years.

Audit output and PR evidence must not expose account identifiers, credentials, webhook values,
real holding lists, or announcement identities. Use domain-separated SHA-256 identity hashes and
structured reason codes.

## 5. Change flow and gates

Every change follows Gate A through Gate D:

- Gate A: pre-flight plus a reviewable design in `docs/`, including data flow, failures,
  rollback, and old-module disposition.
- Gate B: implementation with explicit failure paths and tests.
- Gate C: formatting, Clippy with warnings denied, full tests, and
  `bash tools/compliance/check.sh`.
- Gate D: global line coverage at least 80%, core trading/data paths at least 95%, live-data or
  isolated protocol evidence as appropriate, independent review, and complete PR evidence.

The required baseline commands are:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Do not mark a task complete or merge its PR while any mandatory gate, independent Critical or
Important review finding, data-freshness check, or required PR field remains open.

## 6. Business-rule changes

Register deduplication, mutex/locking, filtering, sorting, and limiting behavior in
`docs/business_rules.md` before implementation. Threshold changes require bidirectional spec and
configuration references plus proof that the threshold does not exceed its domain clamp.

## 7. Rollback

Rollback is by root cause. Architecture or data-flow failures return to Gate A; implementation
defects return to Gate B; red-line violations require both an implementation fix and a repeat of
the Gate A failure-mode review. Prefer a Git revert of the scoped PR. Never delete audit, account,
holding, trade, or market-data evidence as a rollback mechanism.
