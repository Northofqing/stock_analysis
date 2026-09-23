# TDX E2103 backport for the legacy local provider host

This crate starts from `magic-market-data-rs` commit
`75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e` and carries the
`CMD_SECURITY_BARS` server-fault fix from upstream commit
`98207a4497d2012883b6b1f4b0cfb19679353ace`.

The 2026-09-23 failure was a two-byte response declaring 800 bars (`20 03`)
without any row bytes. The old parser correctly raised E2103, but the caller
returned immediately and never switched servers. This backport classifies that
shape before parsing, blocks and rotates away from the server with a bounded
budget, and uses the seven servers that upstream verified serving complete
K-lines on 2026-09-23. Other Magic providers remain at the old Git revision.

`build.rs` publishes a SHA-256 fingerprint over this crate's `Cargo.toml` and
`src/` as `MAGIC_TDX_DEPENDENCY_REVISION`. The base and upstream fix references
in that value describe ancestry; the fingerprint identifies the exact local
source bytes. This branch is for rebuilding the legacy `grpc_market_server`
that still serves `127.0.0.1:18082`; it does not change the current mainline
provider dependency policy.
