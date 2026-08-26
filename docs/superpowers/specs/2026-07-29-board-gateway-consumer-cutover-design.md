# BR-188 Board Gateway Consumer Cutover

Status: Gate B in progress — implementation present, parent-owned Gate C/D evidence pending.
Business rules: BR-071, BR-188.

## 1. Scope and intent

This slice removes three live/dormant consumers from the constant-Unsupported
compatibility facades in `market_analyzer::sector_monitor`:

1. the active intraday board summary in `src/bin/monitor/main.rs`;
2. NewsMonitor's tracked-symbol concept reverse index;
3. `decision::exclusion`'s target-symbol membership scan.

It does not claim that the released upstream can provide the old resonance,
leader-selection, turnover-ranking or ignition contracts.

## 2. Reproducible code facts

Pinned release identity:

```text
$ rg -n 'magic-(tdx|market-core|eastmoney)-rs = .*rev' Cargo.toml
51:magic-tdx-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "660902ff93a07f18367dc16879cf67732accd25a", version = "=0.2.0" }
52:magic-market-core = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "660902ff93a07f18367dc16879cf67732accd25a", version = "=0.2.0" }
54:magic-eastmoney-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "660902ff93a07f18367dc16879cf67732accd25a", version = "=0.2.0" }
```

Affected call sites, using a multiline-capable search:

```text
$ rg -n -U 'fetch_board_(ranking|components)\s*\(' \
    src/bin/monitor/main.rs src/monitor/news_monitor.rs src/decision/exclusion.rs
src/monitor/news_monitor.rs:473:    let boards = match sector_monitor::fetch_board_ranking("f3", 15) {
src/monitor/news_monitor.rs:489:        let stocks = match sector_monitor::fetch_board_components(&board.code, 30) {
src/decision/exclusion.rs:140:        match crate::market_analyzer::sector_monitor::fetch_board_ranking("f3", 100) {
src/decision/exclusion.rs:159:                crate::market_analyzer::sector_monitor::fetch_board_components(board_code, 50)
src/bin/monitor/main.rs:8367:                                    let boards = sector_monitor::fetch_board_ranking("f3", 10)
src/bin/monitor/main.rs:8813:    let boards = sector_monitor::fetch_board_ranking("f3", 30)
src/bin/monitor/main.rs:8837:        let comps = match sector_monitor::fetch_board_components(&b.code, 30) {
```

The final three `main.rs` lines are inside `run_stock_screener`. A second
multiline-capable call search found one timer-owned caller:

```text
$ rg -n -U 'run_stock_screener\s*\(' src/bin/monitor/main.rs src -g '*.rs'
src/bin/monitor/main.rs:8245:match tokio::task::spawn_blocking(run_stock_screener).await {
src/bin/monitor/main.rs:8802:fn run_stock_screener() -> Result<Vec<(String, String)>, String> {
```

The timer and function form one closed legacy path. Because its only candidate
discovery requires the unreleased constituent market shape, the path cannot
produce an admitted recommendation after the old facades became constant
Unsupported. This slice removes both timer and function together. It does not
remove the independently governed selection-v2/Magic TDX selection pipeline.

The released Eastmoney provider fixes `fid=f62`, orders by main net flow, caps
the request at 200, and includes `f3` only as a record field:

```text
$ sed -n '10,55p' \
  ~/.cargo/git/checkouts/magic-market-data-rs-b56d463f5db752be/660902f/crates/magic-eastmoney-rs/src/board_flow.rs
impl BoardFlows for EastmoneyClient {
    ...
    if limit.get() > 200 { ... }
    ...
    FlowInterval::Day1 => (
        "f62",
        "f12,f14,f3,f62,...",
    ),
```

Therefore a consumer may report provider-ranked main-flow rows and their
attached return field. It may not call them a full-market return ranking.

`BoardDataGateway` exposes TDX `memberships(code)` and Eastmoney
`day1_flows(kind, limit)`. The TDX board-constituent contract contains identity
only and the selection-facing route additionally requires a verified binding;
it has no same-batch quote, amount, volume-ratio or turnover fields.

## 3. Data flow

### 3.1 Intraday board summary

```text
monitor timer
  -> BoardDataGateway::day1_flows_blocking(Concept, 10)
  -> complete GatewayBatch + BatchEvidence
  -> require finite return_pct and main_net_yuan for every row
  -> render "concept-board main-net-inflow sample"
  -> existing ReviewSignal governance / delivery / audit
```

Provider rank is preserved. The consumer does not sort this sample into a
different market-wide claim.

### 3.2 NewsMonitor L2 index

```text
complete tracked-code set
  -> stable code sort
  -> BoardDataGateway::memberships_blocking(code), once per code
  -> require every request Available or VerifiedEmpty
  -> retain Concept memberships only
  -> stable dedup/sort board -> tracked codes
  -> atomically replace the old index
```

One Unavailable/Partial/Invalid result rejects the whole refresh. No partially
rebuilt index is installed. A complete set of VerifiedEmpty responses commits
an empty index, clearing stale relationships.

### 3.3 Exclusion scan

```text
holdings + watchlist
  -> stable code dedup/sort
  -> BoardDataGateway::memberships_blocking(code), once per code
  -> require every request Available or VerifiedEmpty
  -> retain Industry/Concept, reject Region as exclusion evidence
  -> configured-order substring match against real provider board names
  -> cache only the complete successful map for (local date, exact code set)
  -> scan positions
```

The public scan returns `Result`; source failure cannot become a clean
zero-hit result.

## 4. Failure modes

| Failure | Disposition |
| --- | --- |
| Gateway invalid request, transport, partial or evidence failure | Return/record explicit error; do not install/cache partial output |
| VerifiedEmpty for one TDX symbol | Complete zero memberships for that symbol |
| VerifiedEmpty board-flow batch | Explicit complete empty summary; no push |
| Missing/non-finite return or main net | Reject entire board-flow consumer batch |
| Missing same-batch constituent market fields | Keep old resonance/leader/turnover capability Unsupported |
| TDX membership category Region | Exclude from concept/exclusion classification |
| Worker panic/join failure | Explicit error; retain prior NewsMonitor index |

## 5. Old-module disposition

| Module/path | Decision | Reason |
| --- | --- | --- |
| `BoardDataGateway::{day1_flows,memberships}` | Adopt | Released typed real-provider contracts with evidence and acquisition audit |
| `sector_monitor::fetch_board_ranking` | Reject for these consumers | Claims unsupported f3/full legacy shape |
| `sector_monitor::search_board_code_by_keyword` | Reject | No released authoritative name-search contract; BR-174 forbids upgrading fuzzy names |
| `sector_monitor::fetch_board_components` | Reject for these consumers | Released membership shape lacks quote/volume/turnover fields |
| `main::run_stock_screener` timer + function | Delete | Closed legacy path is permanently blocked by the unreleased constituent market shape; selection-v2/Magic TDX remains the formal selector |
| legacy SectorTop/turnover/I-01 timers | Delete | Their only source is a constant-Unsupported incomplete board shape; the admitted BR-188 flow summary remains active |
| `app::bootstrap` resonance/unexplained append | Delete | Candidate append cannot be admitted without complete ranks, constituents and leading-signal fields |
| old direct Eastmoney/TDX URL or wire parameters | Reject | Public-data ownership belongs to unified Gateway |

### 5.1 Explicit residual capability inventory

These consumers remain fail-closed because the pinned upstream does not
publish their complete business shape. BR-188 must not relabel a provider
`f62` sample or cross-source join fields merely to make them appear active:

| Consumer | Missing released contract |
| --- | --- |
| `push_templates::load_turnover_top_real` | Board constituents with same-batch price, return and turnover |
| `push_templates::load_sector_snapshot_real` | Full-market return ranking plus complete `ConceptBoard` scoring fields |
| `push_templates::dispatch_sector_top_daily_result` | Full-market return-ranked SectorTop contract |
| `push_templates::dispatch_sector_anomaly_daily` | Full return rank, volume-ratio and flow-acceleration fields |

Their current constant-Unsupported dependencies remain visible in the final
`rg` inventory and are not counted as successful Gateway migrations.

## 6. Tests

Tests use `TEST_CODE_` identities and exercise public/pure consumer behavior:

- Gateway blocking membership request validation and evidence outcome;
- concept-index all-or-nothing admission, Concept-only filtering,
  deterministic dedup/sort and VerifiedEmpty semantics;
- exclusion Industry/Concept matching, Region rejection, full-code-set cache
  binding and no cache update on error;
- static multiline-aware regression proving the three target consumers no
  longer call constant-Unsupported facades;
- static ownership regression proving the migrated call sites enter only
  `BoardDataGateway` and the unserviceable legacy screener is absent.

No test calls a production network endpoint.

## 7. Acceptance and evidence

Local implementation checks delegated to the parent Cargo owner:

```bash
rg -n -U 'fetch_board_(ranking|components)\s*\(' \
  src/bin/monitor/main.rs src/monitor/news_monitor.rs src/decision/exclusion.rs
# expected: zero

rg -n -U 'BoardDataGateway::|memberships_blocking|day1_flows_blocking' \
  src/bin/monitor/main.rs src/monitor/news_monitor.rs src/decision/exclusion.rs
# expected: target consumers have real Gateway call sites

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

Gate D additionally requires a bounded live monitor run showing:

```text
[盘中盘面][BR-188] status=available provider=Eastmoney ...
[NewsMonitor][BR-188] L2 membership index status=available ...
```

The dormant exclusion API is not represented as a live producer. It must not
be described as production-active until a separately designed caller is
wired. This slice nevertheless removes its unavailable data dependency and
tests its complete typed behavior.

## 8. Rollback

Rollback the scoped PR with `git revert <commit-sha>`. Do not delete provider,
delivery, audit, position or market-data evidence. A data-flow defect returns
the work to Gate A; an implementation defect returns it to Gate B.
