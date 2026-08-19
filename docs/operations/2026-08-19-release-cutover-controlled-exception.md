# 2026-08-19 Release Cutover — Controlled Exception

> **Status: ACTIVE / corrective cutover complete / live receipt pending / do
> not merge.** The corrected source has passed fresh Gate C, authenticated
> exact-route probes, and the paired production cutover. The remaining
> non-waived acceptance item is a real provider-backed business delivery from
> the corrected monitor with an authoritative typed Feishu receipt and exact
> durable/audit joins.

- Approved by: `Northofqing`, GitHub repository owner/operator, in the active
  2026-08-19 session
- Approved at: 2026-08-19 09:05:02 +0800
- Expires at: 2026-08-20 09:05:02 +0800
- Scope: paired production cutover of `grpc_market_server` and `monitor`, plus
  the PR merge to `master` and remote push of the exact verified source used by
  that cutover
- Original reason: the session record treated its Gate-C run and authenticated
  opening probe as acceptance evidence, while the repository still lacked the
  independently accepted
  BR-202 coverage-authority entrypoint. The deployed pair predates the P-01,
  NewsFlash, NewsAI, gRPC timeout, data-admission, and durable-delivery fixes.
  Later review invalidated the probe and made the old Gate-C result inapplicable
  to the corrective source; this paragraph is historical rationale only.

## Risk and non-waived requirements

This exception temporarily waives only the BR-202 Gate-D coverage-authority
artifact for the paired cutover and its exact-source PR merge. It does not
waive AGENTS data red lines 2.1, 2.2, 2.3, 2.4, 2.5,
2.6, 2.7, or 2.8. Missing, stale, mixed, partial, unaudited, or fake data must
continue to fail closed. A listener, startup banner, transport handshake, or
generic push log is not acceptance evidence.

The original source revision recorded zero exits for formatting, strict
all-feature Clippy, workspace tests, daily-data freshness, and compliance. The
updated private client bundle completed mTLS/Bearer health, and the release
candidate on port 18083 reported a route set and GlobalNews quorum. Those facts
are retained as historical observations only: the probe's required non-news
route was the prohibited compatibility view rather than exact `LimitPools`, and
the source has since changed. Neither the old Gate-C run nor that probe is
current acceptance evidence.

## Corrective validation status

- Complete: BR-238 uses exact `LimitPools`; authenticated probes on isolated
  18083 and production 18082 both exited zero with all five mandatory non-news
  routes and a two-of-four independent GlobalNews quorum.
- Complete: GRPC-20260818-003 uses a closed diagnostic vocabulary, hashes the
  request correlation value, and redacts unclassified provider/reason/detail.
- Complete: BR-240 classification, append-before-open ordering, P-01 failure
  evidence, NewsFlash authority, and NewsAI physical-boundary findings passed
  focused regressions and independent review.
- Complete: fresh `cargo fmt`, strict all-feature Clippy, all workspace targets,
  data freshness, and the complete compliance entrypoint exited zero.
- Pending: one corrected-process provider-backed business delivery with typed
  Feishu receipt and exact durable/audit joins. A generic `Pushed` event without
  remote receipt fields is explicitly insufficient.

## Cutover and acceptance

1. Preserve the currently deployed executable pair and SHA-256 identities.
2. Stop the old monitor before the old server with SIGINT and prove the lease
   and port are released.
3. Start the fresh default-feature server on 127.0.0.1:18082.
4. Run the authenticated release opening probe; failure triggers rollback
   before monitor startup.
5. Start one no-default-feature monitor and prove it alone owns the production
   delivery lease.
6. Acceptance requires at least one real provider-backed business delivery
   with an authoritative typed Feishu receipt and exact durable/audit joins.

## Rollback

Stop candidate processes with SIGINT, verify the listener and lease are free,
then restart the preserved pair from:

`/private/tmp/stock-analysis-cutover.lvCl30`

The preserved SHA-256 values are:

- server: `53ed54cfd357440241ce613f3b73db11d022d6fc86ddaef8482654a1a1d8d85e`
- monitor: `8ae6aa803fb24afaeac8593678b2030fe4fa91b0fa9b6b8f504d361d873ed208`
- probe: `1e651712b17d6cf3bf0dedc5325978fae8d9680d6222fe6e592b7bf155dd1fb4`

Rollback must not delete or rewrite SQLite, WAL/SHM, JSONL, event audit,
acquisition audit, durable decisions, sink results, or lease evidence.

## Required follow-up

Within 24 hours, append the exact cutover and live-receipt evidence, document
all degraded provider routes, and record a postmortem. BR-202 implementation
and independent acceptance remain required before ordinary Gate-D completion.

## Historical cutover evidence (not current acceptance)

- The stale monitor could not complete SIGINT because its main thread was
  blocked in the pre-BR-243 synchronous gRPC bridge join. A process sample was
  retained at `/tmp/monitor_2026-08-19_090617_IDFY.sample.txt`; the controlled
  cutover used SIGTERM only after the sample identified that root cause. The
  delivery lease and port were then proved free before candidate startup.
- The fresh release server SHA-256 is
  `b1edcfd58c00c4a3ceffb2f449a1759748154ef1f935786044baa5df6afe6857`;
  the fresh no-default-feature monitor SHA-256 is
  `327a11b112322157b9c49e23819f17d74984fe734801dfc8db52c5188f8012d6`.
- The production server started as PID 96550 and became the sole listener on
  `127.0.0.1:18082`. The authenticated opening probe exited zero and printed
  `opening_static_ready=true`, but later inspection proved that output was a
  false-positive classification because it used `UpperLimitPoolReview` rather
  than exact `LimitPools`. The provider observations remain useful for route
  diagnosis, but the probe result is not BR-238 or release acceptance.
- The fresh monitor started as PID 97167 and was the sole holder of
  `data/locks/production/monitor-delivery.lock`. Its startup durable fixed
  point completed with zero resumed sink calls before resident producers ran.

## Historical P-01 delivery evidence (scoped, not release acceptance)

At 2026-08-19 09:12:53 +0800 the P-01 provider-backed pre-open news delivery
was accepted by Feishu. The durable decision
`7c6698844939d6bfff935814923092ae23662150d1731bb497683da73bc701e7`
reached `Delivered`; its current attempt reached `Accepted`; the authoritative
sink result is `Accepted`, channel `feishu`, provider `magiclaw-cli`, and both
remote message identifiers are present. The accepted receipt SHA-256 is
`467dc9686446d98907d88ecce25fe3c58fbc12147df7517da7dc2a87cac10ff2`.

The same decision has one policy-v5 BusinessDateOnce claim. Both its claim
audit and sink-authority audit are `Appended`. The immutable
`DeliveryAcceptedAudit` canonical SHA-256 is
`486ac259adcc3a4acf2829cad6652b40cef5943f64f6316020f926d5e57a18b9`;
its retained record hash is
`1276a19c04e74b130d3d93659b86ecfac59d469499278a8ee61ddfa704168a37`.
The matching schema-v3 `push.delivery.audit` event has ID
`6757d91ccde27a1bdfa46dd1f1351df791a774f5d95bc59d632711a884164961`
in both the retained event audit and observation bus. This satisfies only the
P-01 occurrence's real-provider, typed-receipt, and durable/audit-join
requirement. It does not repair the false-positive opening probe, validate the
corrective source, reactivate this exception, or authorize merge/cutover.

## Corrective cutover evidence (current source)

- Candidate identities: server
  `55b4110920ef6f2ce74fb1d3f665817dc882d17a765b62811e858323fd7fc275`,
  no-default monitor
  `1fdc93df6dad03d110f6dfd7991fd6ebf8c1ac49a3618058fe35bdf7df89082d`,
  and probe
  `b71b8304513b57193fec573182e3a2ff56c8db781328fb2f6f1649f66b46a5fe`.
- The isolated 18083 probe and the post-start production 18082 probe both
  exited zero. Both reports named exact `LimitPools`, retained the explicit
  degraded Eastmoney/ThePaper routes, admitted independent CLS/Jin10 routes,
  and printed `opening_static_ready=true routes=7/9 global_news=2/4`.
- The old monitor released the production lease after SIGINT, then the old
  server released 18082. The corrected server is the sole 18082 listener and
  the corrected no-default monitor is the sole holder of
  `data/locks/production/monitor-delivery.lock`.
- Startup durable reconciliation reached a fixed point with zero resumed sink
  calls. The monitor's own static gate independently repeated the exact route
  result before starting resident producers.
- A current-process DataMode delivery reached generic `Pushed`, proving the
  resident loop and physical channel are active, but that audit has no typed
  remote IDs or authoritative receipt hash. It is operational evidence only,
  not step 6 acceptance.

## Postmortem and residual risks

The old pair was fail-closed but operationally stale: its monitor could remain
stuck inside an unbounded synchronous bridge join and therefore could not
service SIGINT. The replacement contains the BR-243 bounded bridge path and
completed startup without that hang. No database, WAL/SHM file, JSONL chain,
receipt, claim, or acquisition audit was deleted or rewritten during cutover.

Residual fail-closed conditions are visible rather than hidden: startup health
reported `perf_recent=false`; realtime position/cash authority remains stale
against the 30-second rule until a new real-account snapshot arrives; Jin10 was
unavailable; and the NewsFlash SourceOnly projection rejected current provider
batches whose evidence did not match its registered contract. These conditions
do not invalidate the scoped historical P-01 receipt, but they must not be
described as healthy producers until fresh authority is admitted. BR-202 formal
coverage authority is the only artifact this exception is permitted to waive;
it is not the only remaining blocker. The status remains `SUSPENDED` until all
non-waived blockers above have fresh evidence appended.
