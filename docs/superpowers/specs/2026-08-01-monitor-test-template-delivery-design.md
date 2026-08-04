# `monitor --test` Template Delivery Acceptance Design

Status: Gate A repair proposed; formal independent review required; Gate B/C/D pending

Rule: BR-196

Date: 2026-08-01
Revision: 2026-08-02

This decision changes the BR-196 template-delivery acceptance contract and
records the public monitor push/template API migration already present in the
reviewed source preimage. It does not rename an existing `PushKind`, descriptor
ID, template ID, replay envelope ID, or persisted identity.

Gate B adds `PushKind::EventReplay` and `PushKind::HealthWebhook`, the
`X-02-event-replay` and `X-03-health-webhook` families/shapes, and two
descriptors. These are additive identities. Existing API removals, visibility
reductions, replacements, and compatibility decisions are frozen in §9; the
design treats them as explicit migration scope.

## 1. Decision and safety boundary

A renderer definition, public wrapper, descriptor declaration, catalog row,
preview fixture, test token, or startup self-check is not evidence that a real
producer can reach a card. `Active` requires an independently audited,
non-test source chain from a production entry to the exact assembler and
production gateway. A path that cannot be proved by that audit and has no
production observation is `Disabled`. A catalog-only or historical identity
with no production presentation contract is `SpecOnly`.

On 2026-08-02 both required production evidence paths are absent. The exact
probe and output are:

```bash
test -d data/push_log/2026-08-02 && find data/push_log/2026-08-02 -maxdepth 1 -type f -print || echo 'ABSENT data/push_log/2026-08-02'
test -f data/event_bus/2026-08-02.jsonl && wc -l data/event_bus/2026-08-02.jsonl || echo 'ABSENT data/event_bus/2026-08-02.jsonl'
```

```text
ABSENT data/push_log/2026-08-02
ABSENT data/event_bus/2026-08-02.jsonl
```

The release-pinned `non_production_acceptance.target_sha256` allowlist is also
empty. Test output, fake-process receipts, dry-run output, manifest contents,
and registry contents therefore provide no production-delivery evidence.

```bash
sed -n '/^\[non_production_acceptance\]/,/^\[/p' config/br196_non_production_feishu_targets.toml
```

```text
[non_production_acceptance]
target_sha256 = []

# Production targets are denied explicitly.  Never place raw tenant, app or
# conversation identifiers in this repository.
[production_deny]
```

### 1.1 Canonical BR-196 wording

The following paragraph is the canonical business-rule text. Its bytes are
copied verbatim into the single BR-196 row in `docs/business_rules.md`:

> `monitor --test` 的模板权威是版本化三层闭集；生命周期仅允许 `Active`、`Disabled`、`SpecOnly`。`Active` 必须由独立于 registry/manifest 的非测试生产调用链审计证明，或由生产命名空间的真实运行审计证明；函数定义、wrapper、descriptor/manifest token、preview、自调用测试、测试入口及 explicit-disabled/log-only 分支均为零生产证据。当前 50 个 descriptor 的逐项审计结果为 25 Active、25 Disabled；registry 外另有四个 shape：`P-05-virtual-watch-pilot-snapshot` 因其唯一输入 `post_close=""` 而为 Disabled，`I-01-intraday-market-board-flow`、raw replay 与 raw health webhook 具备非测试生产调用链。Gate B 必须保留 I-01 inline family/PushKind 身份，并以 additive `X-02-event-replay`/`EventReplay` 与 `X-03-health-webhook`/`HealthWebhook` family/shape、PushKind 及 descriptor 登记两个 raw 外发，不得重命名既有身份；目标路由 descriptor 为 28。固定目标矩阵为 Shape `A28/D26/S14/T68`、Family `A27/D25/S14/T66`、PushKind `A24/D24/S12/T60`，必须从本规则的精确集合独立重算，不得从旧计数或 catalog 内容推导。26 个无可达生产链 shape 与 14 个 SpecOnly path 启动时必须各输出一次 `[BR-196] shape=<shape_id> lifecycle=<lifecycle> disabled=<reason> producer=none`，其中 lifecycle 必须为 `Disabled` 或 `SpecOnly`，并先写入隔离、追加式审计；重复、缺字段、banner/audit 写入或同步失败均须在 fixture、provider、sink 和外部进程构造前 exit 2。replay 仅接受显式 `--replay --force`、`push.source` envelope 和 typed replay evidence；Gate B 必须保持 `[REPLAY <date>] <source_text>` 文本、fresh replay ID 与 `replay_of` 身份兼容，将 raw `push_wechat` 迁移至 typed presentation gateway，禁止用证券 code 字段伪装 replay identity。health webhook 只接受 typed startup-health evidence，保持 `health_check_fail` 事件和显式 Disabled/Delivered/Error 语义，并将 raw HTTP POST 迁移至 typed presentation gateway；缺 URL 不得伪装成功。bare `--test` 只有在精确 opt-in、release-pinned 非生产飞书 allowlist、typed target authority、逐批 target/time 复验和真实回执全部成功时才可外发；当前 allowlist 为空，必须 fail closed。`--test --push-dry-run` 只渲染 28 个 Active shape，外部进程、网络发送和 receipt audit 必须为零。所有 fixture 只接受 typed `TEST_CODE` identity；governance smoke 只证明治理行为，不得升级任何 lifecycle 或充当生产可达性/投递证据。HEAD→current 的精确 18 路径 public-surface 清单必须覆盖 `pub`/`pub(crate)`/`pub(super)` item 与 method、公开 struct field、公开 enum 的全部 variant、公开 trait member、type/const/static/use/re-export；当前审计基线固定为 HEAD=1091、current=1143、one-sided delta=278、delta SHA-256 `5219a3521583f1a97ef8716a8457d21673fbe1d9b23668e215bd8c4363a5a69b`，278/278 必须逐项归类且 `unclassified=0`。Gate B 若修改 R-08 dispatcher→public presented wrapper→private source-only helper 合同，必须同步更新 `tools/compliance/lib/check_br194_review_dependency.sh`，同时验证 wrapper token-kind/delegation与 private helper 的 source-only 语义，禁止恢复已废弃 public helper；该 checker、调用方、wrapper/helper 与对应测试必须进入同一 post-Gate-B source manifest 和 Rule-2.10 证据。

The canonical preimage excludes the Markdown quote prefix and line terminator.
The unique BR row's Intent cell is byte-equal:

```bash
canonical=$(sed -n '/^> /s/^> //p' docs/superpowers/specs/2026-08-01-monitor-test-template-delivery-design.md)
row_intent=$(awk -F'|' '/^\| BR-196 \|/{v=$4; sub(/^[[:space:]]/,"",v); sub(/[[:space:]]$/,"",v); print v}' docs/business_rules.md)
printf 'br196_rows=%s\n' "$(rg -c '^\| BR-196 \|' docs/business_rules.md)"
printf 'header_cells=%s row_cells=%s\n' "$(awk -F'|' '/^\| Rule ID \|/{print NF-2; exit}' docs/business_rules.md)" "$(awk -F'|' '/^\| BR-196 \|/{print NF-2; exit}' docs/business_rules.md)"
[[ "$canonical" == "$row_intent" ]] && echo canonical_byte_equal=yes || echo canonical_byte_equal=no
printf %s "$canonical" | shasum -a 256
```

```text
br196_rows=1
header_cells=4 row_cells=4
canonical_byte_equal=yes
591f83b1da974af511b982bea96b773b6b8f8beee194f6d57b0ba32f194450be  -
```

### 1.2 Reviewed source preimage and later commit attestation

This untracked design is not part of `HEAD`. `HEAD` is used only as the API
comparison base. No count, lifecycle, or reachability claim is described as
HEAD-derived.

The reviewed source preimage is the following eighteen-file manifest. It
includes every file used as reachability, external-terminal, or repository-wide
public-API authority below. Paths are sorted with `LC_ALL=C`; each line is the
literal `shasum -a 256` output, including the two spaces before the path and
the trailing newline.

```bash
for p in src/bin/monitor/br196_test_delivery.rs src/bin/monitor/br196_transport.rs src/bin/monitor/health.rs src/bin/monitor/l6_sink.rs src/bin/monitor/main.rs src/bin/monitor/news_aggregator_init.rs src/bin/monitor/notify.rs src/bin/monitor/presentation_registry.rs src/bin/monitor/push_templates.rs src/bin/monitor/v17_sources.rs src/bin/monitor/webhook_alert.rs src/event/history.rs src/event/replay.rs src/monitor/alert.rs src/news/aggregator/classifier.rs src/opportunity/candidate_state.rs src/opportunity/mod.rs src/opportunity/scheduler.rs; do shasum -a 256 "$p"; done | LC_ALL=C sort
```

```text
09c7cdf04883ce2d6b158cc53946e5573eb76c52044072043371f5d35773ae92  src/news/aggregator/classifier.rs
0b503edf2196d9b40696d97bcb676c0e78563b8819b369b8aff1ea8211e5ecf3  src/event/replay.rs
0bda3a4bdc3e295704ba91d0ad6693c25387328488e706c4fcfe8c5e9049586b  src/bin/monitor/l6_sink.rs
14f46cd4830e3706a4d8a45ad4de112ab75376c8ac08a988018fcfdeb7dded45  src/bin/monitor/br196_transport.rs
2fffed295c9e4036397dc39ad25e65c55b5f8d146fe9b5b74413ff46eeb54f25  src/bin/monitor/main.rs
4a4c21b6fd11c2d95a6c090cf631fdcdb7df300812b98a33061ebe63590dacb5  src/bin/monitor/push_templates.rs
634d30f2576f1324375791ef3f941259cdfbdddd1239855b05105a6e00c20194  src/bin/monitor/webhook_alert.rs
6f54d1979e0ab4c57dd93709589079fb802f1121cede318d5cd0195ec6eca8ec  src/opportunity/scheduler.rs
837d9b7e09b02cd33a66eddbfa38c1f01a093287e07f8d2d53039d6682341897  src/bin/monitor/presentation_registry.rs
84bdd9819079082e137ff165feb4a0c98921f1f6f7553058528fc78e6980c58e  src/bin/monitor/health.rs
91801223d04cecd2a2a280678031245237323ed9295721625b3885f14f094359  src/bin/monitor/notify.rs
9898b47eb6543dcd89332bf0c10b410de6b0ef5846181ad41e4b5bab0f190f6b  src/opportunity/candidate_state.rs
a1b4fd3399b8d54c83eac35fbdf6f519a0514034e7ee541d46fade841319848a  src/monitor/alert.rs
aa9bf638cfdca1f12b187e7fb17e6f7d2f98f1de425736d726e7b37df511d230  src/opportunity/mod.rs
cdc36e870dca96effe9516ba414eb3ad6173c71fd7b681c6e94eb5e689bdc2eb  src/event/history.rs
cf6c7fd14c7b8bcf9574b33c5d3711aec74818b3e6dfd4d6c8240c9baa9ca4ec  src/bin/monitor/news_aggregator_init.rs
dae54cfee2654c6d9847f208166d496a7686ef0cd4feaf44763358c336fddd4d  src/bin/monitor/br196_test_delivery.rs
e762ec55da0e8c7034e7660023092a62dfd5bcbe8ff337caa849b120f8ede3d9  src/bin/monitor/v17_sources.rs
```

The same pipeline followed by `shasum -a 256` outputs:

```text
5b939feb0ee03e603fa0939fb82a67da3ef8486e42319af10689915de1700e80  -
```

The API comparison base is commit
`b4aeee68d2c0259cc968914b3d39e3a89a18a496`, tree
`eb5aafaeee62b5bb58395417b25c87ace9cfd062`. It is not the reviewed working
tree and is not the design identity.

The physically possible commit flow is: review this file, the BR row, and the
config-comment-only diff against the manifest root, canonical BR hash, and
config semantic hash; commit those three paths without changing the eighteen
source files; then put the resulting commit ID, source-manifest root, canonical
BR hash, and config semantic hash in the PR review attestation. Gate B starts
from that reviewed commit and records its own post-implementation source
manifest. The design never contains or predicts its own commit ID, so there is
no self-reference.

## 2. Reachability contract

### 2.1 Independent authority

The reachability authority is this independently reviewed 50-row audit, not a
production descriptor registry and not the test manifest. Gate B may check
that routing metadata conforms to the reviewed Active set, but that check is
only drift detection; agreement between two tables cannot activate a path.

There is deliberately no home-grown text scanner that claims to compute a
Rust call graph. Such a scanner would miss macro expansion, multiline calls,
`cfg` boundaries, re-exports, and dynamic dispatch. Static evidence is instead
an exact reviewer-traced sequence of resolved non-test calls with `file:line`
anchors. The formal reviewer must reopen every anchor, verify that the root is
a production binary path, and verify every hop to the exact renderer/assembler
and gateway use-site. A definition, test, preview, or unresolved indirect edge
invalidates the chain.

The executable portion of the contract is conservative:

1. Gate B freezes the exact Active/Disabled/SpecOnly sets and matrices below.
2. Startup validates IDs, ordinals, reasons, matrices, and inactive banners
   before fixture/provider/sink construction.
3. A production gateway observation appends
   `br196.presentation_reachability_observed.v1` only when a non-test process
   actually arrives with real producer evidence. Test, dry-run, preview, and
   non-production audit roots are rejected by the writer.
4. A future Disabled-to-Active change requires a new independent call-chain
   review or a valid production observation. An automatic test may verify
   metadata and source anchors, but must not claim it proved transitive
   reachability.
5. If either proof is absent or ambiguous, the only accepted result is
   `Disabled`.

The runtime observation record contains `event_type`, `schema_version`,
`manifest_version`, `manifest_sha256`, `source_tree_manifest_sha256`, the
actual committed source revision when one exists, `environment`,
`process_invocation_sha256`, `family_id`, `shape_id`, `push_kind`,
`producer_seam_id`, `renderer_or_assembler_seam_id`, provider/source identity,
provider time when supplied, local observation time, immutable source batch or
event identity hash, decision status, retryability, and rule IDs. Missing
provider time stays absent. Append/lock/chain/flush/`sync_data` failure is fatal
and cannot be replaced by a log line.

### 2.2 Full audit of the 50 declared descriptors

The descriptor enumeration command is executable as written:

```bash
awk '/^[[:space:]]*descriptor\($/{getline; gsub(/[",[:space:]]/, ""); print}' \
  src/bin/monitor/presentation_registry.rs
awk '/^[[:space:]]*descriptor\($/{n++} END{print "descriptor_count=" n}' \
  src/bin/monitor/presentation_registry.rs
```

Its exact output for the §1.2 source preimage is:

```text
T-01-account-mode
T-02-data-mode
T-02-data-mode-reminder
T-03-holding-plan
T-04-holding-event
T-05-t0-advice
T-06-t0-forbid
T-07-candidate-triggered
T-08-candidate-invalidated
T-09-forbidden-ops
P-05-virtual-watch
T-10-paper-trade
T-11-auction-volume
T-12-close-call
I-09-sector-top
T-13-turnover-top
R-01-daily-report
R-02-review-market
R-03-industry-chain
R-04-review-lhb-gateway
R-05-review-signal
R-06-review-failure
R-07-tomorrow-watch
R-09-provider-top-n
P-01-preopen-news-hot
I-01-intraday-market
I-02-news-catalyst
I-09-sector-anomaly
D-01-news-to-idea
A-10-catalyst-review
I-03-industry-chain-intraday
T-14-post-fixed-price-order
T-15-post-fixed-price-fill
T-16-st-price-limit-changed
T-17-etf-closing-call-auction
BR-033-block-trade-confirm
BR-034-block-trade-range
A-01-paper-review
R-08-public-event-calendar
L-01-limit-boards-first
L-02-limit-boards-second
L-03-limit-boards-third-plus
S-01-announcement
S-02-policy-hit
S-03-earnings-beat
S-04-earnings-miss
S-05-analyst-upgrade
S-06-market-action-alert
N-01-news-flash-critical
N-02-news-flash-aggregated
descriptor_count=50
```

Lifecycle classification remains a human-reviewed architecture decision; no
text search is represented as a Rust call-graph proof. The following bounded
command instead verifies that the artifact has exactly one row for every
declared descriptor and reproduces the reviewed lifecycle totals:

```bash
design=docs/superpowers/specs/2026-08-01-monitor-test-template-delivery-design.md
declared=$(awk '/^[[:space:]]*descriptor\($/{getline; gsub(/[",[:space:]]/, ""); print}' src/bin/monitor/presentation_registry.rs | LC_ALL=C sort)
audited=$(awk -F'|' '/^\| [0-9]+ \| `/{cell=$3; gsub(/`/, "", cell); split(cell, a, " / "); gsub(/^[[:space:]]+|[[:space:]]+$/, "", a[1]); print a[1]}' "$design" | LC_ALL=C sort)
printf 'descriptor_declared=%s\n' "$(printf '%s\n' "$declared" | wc -l | tr -d ' ')"
printf 'audit_rows=%s\n' "$(printf '%s\n' "$audited" | wc -l | tr -d ' ')"
printf 'descriptor_set_diff_lines=%s\n' "$(comm -3 <(printf '%s\n' "$declared") <(printf '%s\n' "$audited") | wc -l | tr -d ' ')"
awk -F'|' '/^\| [0-9]+ \| `/{v=$4; gsub(/[[:space:]]/, "", v); n[v]++} END{printf "descriptor_lifecycle=Active:%d Disabled:%d\n", n["Active"], n["Disabled"]}' "$design"
```

```text
descriptor_declared=50
audit_rows=50
descriptor_set_diff_lines=0
descriptor_lifecycle=Active:25 Disabled:25
```

The exact classified artifact is the 50-row table below. Each Active chain
begins at a non-test production root, names the exact assembler/renderer, and
ends at the typed gateway use-site. Definitions without such a root remain
Disabled. The §1.2 source hashes bind every cited source byte; the bounded
anchor-digest command following the table verifies that every literal
`file:line` anchor still resolves. Neither check upgrades a human lifecycle
decision into computed reachability.

`Active` rows below show a non-test upstream root followed by the exact
production use-site. `none` means the audit found no qualifying upstream chain;
the cited lines explain the nearest misleading declaration/wrapper or the
explicit failure boundary.

Path aliases in this table are exact: `main.rs`, `push_templates.rs`,
`notify.rs`, `v17_sources.rs`, and `news_aggregator_init.rs` are under
`src/bin/monitor/`; `classifier.rs` is
`src/news/aggregator/classifier.rs`.

| # | Descriptor / PushKind | Lifecycle | Non-test upstream caller evidence | Exact reason when Disabled |
| ---: | --- | --- | --- | --- |
| 1 | `T-01-account-mode` / `AccountMode` | Active | `main.rs:1893` → renderer `push_templates.rs:2000` → tuple `push_templates.rs:2005` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 2 | `T-02-data-mode` / `DataMode` | Active | startup `main.rs:4043` → hook `main.rs:2092,2176` → renderer `push_templates.rs:11529` → tuple `push_templates.rs:11554` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 3 | `T-02-data-mode-reminder` / `DataMode` | Active | scheduler `main.rs:2234` → hook `main.rs:2092,2176` → renderer `push_templates.rs:11541` → tuple `push_templates.rs:11563` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 4 | `T-03-holding-plan` / `HoldingPlan` | Disabled | none; `push_templates.rs:5728` always fails before acquisition | `holding_plan_counted_binding_unavailable` |
| 5 | `T-04-holding-event` / `HoldingEvent` | Disabled | none; token acquisition is test-only at `notify.rs:6144` | `holding_event_counted_binding_unavailable` |
| 6 | `T-05-t0-advice` / `T0Advice` | Active | producer `main.rs:7415` → renderer `main.rs:5993` → tuple `main.rs:7434` → counted gateway `main.rs:7452` / `notify.rs:2345` | — |
| 7 | `T-06-t0-forbid` / `T0Advice` | Disabled | none; renderer definition `push_templates.rs:580`, preview only at `push_templates.rs:12593` | `production_presentation_caller_not_found` |
| 8 | `T-07-candidate-triggered` / `CandidateTriggered` | Disabled | none; `push_templates.rs:11350` returns capability error before token/gateway | `candidate_counted_binding_unavailable` |
| 9 | `T-08-candidate-invalidated` / `CandidateInvalidated` | Disabled | none; public wrapper exists at `push_templates.rs:11373` but has no non-test upstream caller | `production_presentation_caller_not_found` |
| 10 | `T-09-forbidden-ops` / `ForbiddenOps` | Disabled | none; renderer definition `push_templates.rs:718`, preview only at `push_templates.rs:12642` | `production_presentation_caller_not_found` |
| 11 | `P-05-virtual-watch` / `VirtualWatch` | Disabled | none; `post_close` is explicitly empty at `main.rs:6544`, its only population loop starts at `main.rs:6558`, and both the wrapper call at `main.rs:6958` and pilot branch require non-empty `virtual_observation` | `producer_input_explicitly_empty` |
| 12 | `T-10-paper-trade` / `PaperTrade` | Active | `main.rs:6776` → renderer `push_templates.rs:5498` → tuple `push_templates.rs:5555` → counted gateway `push_templates.rs:5568` / `notify.rs:2345` | — |
| 13 | `T-11-auction-volume` / `AuctionVolume` | Active | `main.rs:6512` → renderer `push_templates.rs:5837` → tuple `push_templates.rs:5844` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 14 | `T-12-close-call` / `CloseCall` | Disabled | none; renderer definition `push_templates.rs:1009`, preview only at `push_templates.rs:12712` | `production_presentation_caller_not_found` |
| 15 | `I-09-sector-top` / `SectorTop` | Disabled | none; `dispatch_sector_top_daily/periodic` definitions at `push_templates.rs:13243` and `:13247` have no non-test callers | `production_presentation_caller_not_found` |
| 16 | `T-13-turnover-top` / `TurnoverTop` | Disabled | none; renderer definition `push_templates.rs:1099`, preview only at `push_templates.rs:12720` | `production_presentation_caller_not_found` |
| 17 | `R-01-daily-report` / `DailyReport` | Disabled | none; renderer definition `push_templates.rs:1162`, preview only at `push_templates.rs:12734` | `production_presentation_caller_not_found` |
| 18 | `R-02-review-market` / `ReviewMarket` | Disabled | none; explicit startup failure at `main.rs:3888`; wrapper at `push_templates.rs:7319` is not called | `provider_capability_not_live_admitted` |
| 19 | `R-03-industry-chain` / `IndustryChain` | Disabled | none; implementation at `push_templates.rs:9532` is reached only by wrapper `push_templates.rs:9693` and tests; neither has a non-test upstream caller | `production_presentation_caller_not_found` |
| 20 | `R-04-review-lhb-gateway` / `ReviewLhb` | Active | `main.rs:4234` → task `push_templates.rs:7220` → renderer `push_templates.rs:9991` → tuple `push_templates.rs:10215` → source-only gateway `push_templates.rs:10228` / `notify.rs:2391` | — |
| 21 | `R-05-review-signal` / `ReviewSignal` | Disabled | none; `push_templates.rs:10261` is explicit-disabled | `no_signal_delivery_execution_settlement_outcome_source` |
| 22 | `R-06-review-failure` / `ReviewFailure` | Disabled | none; `push_templates.rs:10270` is explicit-disabled | `no_evidence_bound_classified_failure_outcome_source` |
| 23 | `R-07-tomorrow-watch` / `TomorrowWatch` | Disabled | none; production banner at `main.rs:7738` is log-only | `incomplete_source_contract` |
| 24 | `R-09-provider-top-n` / `ReviewProviderTopN` | Active | `main.rs:4234` → task `push_templates.rs:7239` → renderer `push_templates.rs:6294,6373` → tuple `push_templates.rs:6755` → durable typed gateway `push_templates.rs:6769` | — |
| 25 | `P-01-preopen-news-hot` / `PreopenNewsHot` | Active | `main.rs:6427` → producer `push_templates.rs:2200,2235` → renderer/tuple `push_templates.rs:2075-2076` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 26 | `I-01-intraday-market` / `IntradayMarket` | Active | CLI root `main.rs:3909,1471` → producer `push_templates.rs:2578,2611` → renderer/tuple `push_templates.rs:11203-11204` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 27 | `I-02-news-catalyst` / `NewsCatalyst` | Active | `main.rs:5852` → producer `push_templates.rs:2885` → renderer/tuple `push_templates.rs:11221-11222` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 28 | `I-09-sector-anomaly` / `SectorAnomaly` | Disabled | none; `dispatch_sector_anomaly_daily` at `push_templates.rs:13254` has no non-test caller | `production_presentation_caller_not_found` |
| 29 | `D-01-news-to-idea` / `NewsToIdea` | Active | `main.rs:5820` → producer `push_templates.rs:3981` → renderer/tuple `push_templates.rs:11289-11291` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 30 | `A-10-catalyst-review` / `CatalystReview` | Active | `main.rs:4234` → task `push_templates.rs:7249` → renderer `push_templates.rs:10848` → tuple `push_templates.rs:10878` → source-batch gateway `push_templates.rs:10892` / `notify.rs:2524` | — |
| 31 | `I-03-industry-chain-intraday` / `IndustryChainIntraday` | Active | `main.rs:7553` → producer `push_templates.rs:3241,3328` → renderer/tuple `push_templates.rs:11271-11273` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 32 | `T-14-post-fixed-price-order` / `PostFixedPriceOrder` | Disabled | none; scheduler calls `push_templates.rs:4703`, but no non-test caller registers `TRADE_EVENT_SOURCE` (`push_templates.rs:4599`) | `trade_event_source_not_registered` |
| 33 | `T-15-post-fixed-price-fill` / `PostFixedPriceFill` | Disabled | none; scheduler calls `push_templates.rs:4778`, but no non-test caller registers `TRADE_EVENT_SOURCE` (`push_templates.rs:4599`) | `trade_event_source_not_registered` |
| 34 | `T-16-st-price-limit-changed` / `StPriceLimitChanged` | Active | `main.rs:7661` → batch `main.rs:1583` → renderer `push_templates.rs:4969` → tuple `push_templates.rs:4970` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 35 | `T-17-etf-closing-call-auction` / `EtfClosingCallAuction` | Disabled | none; explicit startup failure at `main.rs:7694`; wrapper at `push_templates.rs:5018` has no caller | `no_etf_auction_producer` |
| 36 | `BR-033-block-trade-confirm` / `BlockTradeIntradayConfirm` | Disabled | none; public wrapper definition only at `push_templates.rs:5033` | `production_presentation_caller_not_found` |
| 37 | `BR-034-block-trade-range` / `BlockTradePriceRange` | Disabled | none; public wrapper definition only at `push_templates.rs:5082` | `production_presentation_caller_not_found` |
| 38 | `A-01-paper-review` / `PaperReview` | Active | `main.rs:4234` → task `push_templates.rs:7259,4504,4522` → renderer/tuple `push_templates.rs:11311-11312` → gateway `push_templates.rs:11713` / `notify.rs:2331` | — |
| 39 | `R-08-public-event-calendar` / `EventCalendar` | Active | `main.rs:4234` → task `push_templates.rs:7229` → renderer `push_templates.rs:7444` → tuple `push_templates.rs:8651` → R-08 gateway `push_templates.rs:8664` / `notify.rs:2441` | — |
| 40 | `L-01-limit-boards-first` / `LimitBoards` | Active | root/renderer `main.rs:7072` → tuple `main.rs:7078` → gateway `main.rs:7085` / `notify.rs:2331` | — |
| 41 | `L-02-limit-boards-second` / `LimitBoards` | Active | root/renderer `main.rs:7097` → tuple `main.rs:7103` → gateway `main.rs:7110` / `notify.rs:2331` | — |
| 42 | `L-03-limit-boards-third-plus` / `LimitBoards` | Active | root/renderer `main.rs:7122` → tuple `main.rs:7128` → gateway `main.rs:7135` / `notify.rs:2331` | — |
| 43 | `S-01-announcement` / `Announcement` | Active | `main.rs:5758` → producer `v17_sources.rs:309,398` → renderer/tuple `v17_sources.rs:626,598` → source-fact gateway `v17_sources.rs:648` / `notify.rs:2511` | — |
| 44 | `S-02-policy-hit` / `PolicyHit` | Disabled | none; `classify_policy` is defined at `classifier.rs:285` but all its callers are tests | `production_presentation_caller_not_found` |
| 45 | `S-03-earnings-beat` / `EarningsBeat` | Active | `main.rs:5603` → producer `v17_sources.rs:839,1037` → renderer/tuple `v17_sources.rs:626,598` → source-fact gateway `v17_sources.rs:648` / `notify.rs:2511` | — |
| 46 | `S-04-earnings-miss` / `EarningsMiss` | Active | `main.rs:5603` → producer `v17_sources.rs:839,1037` → renderer/tuple `v17_sources.rs:626,598` → source-fact gateway `v17_sources.rs:648` / `notify.rs:2511` | — |
| 47 | `S-05-analyst-upgrade` / `AnalystUpgrade` | Active | `main.rs:5603` → producer `v17_sources.rs:839,1037` → renderer/tuple `v17_sources.rs:626,598` → source-fact gateway `v17_sources.rs:648` / `notify.rs:2511` | — |
| 48 | `S-06-market-action-alert` / `MarketActionAlert` | Active | event consumer `main.rs:3969` → producer `v17_sources.rs:200,212` → renderer/tuple `v17_sources.rs:626,598` → presented gateway `v17_sources.rs:657` / `notify.rs:2331` | — |
| 49 | `N-01-news-flash-critical` / `NewsFlashCritical` | Disabled | none; `push_flash_decisions` at `news_aggregator_init.rs:370` has only a test caller | `production_presentation_caller_not_found` |
| 50 | `N-02-news-flash-aggregated` / `NewsFlashAggregated` | Disabled | none; `push_flash_decisions` at `news_aggregator_init.rs:370` has only a test caller | `production_presentation_caller_not_found` |

The exact literal `file:line` anchors in those 50 rows are source-bound by this
bounded command. It prints only a count and digest, not 4,531 search-context
lines. Comma/range continuations in a row remain part of the manual full-hop
review and are also bound by the complete-file hashes in §1.2.

```bash
design_file=docs/superpowers/specs/2026-08-01-monitor-test-template-delivery-design.md
refs=$(awk '/^\| 1 \|/{inside=1} inside{print} /^\| 50 \|/{exit}' "$design_file" | rg -o '(main|push_templates|notify|v17_sources|news_aggregator_init|classifier)\.rs:[0-9]+' | LC_ALL=C sort -u)
printf 'descriptor_anchor_count=%s\n' "$(printf '%s\n' "$refs" | wc -l | tr -d ' ')"
printf '%s\n' "$refs" | while IFS= read -r ref; do
  alias_name=${ref%%:*}; source_line=${ref##*:}
  case "$alias_name" in
    main.rs) source_file=src/bin/monitor/main.rs ;;
    push_templates.rs) source_file=src/bin/monitor/push_templates.rs ;;
    notify.rs) source_file=src/bin/monitor/notify.rs ;;
    v17_sources.rs) source_file=src/bin/monitor/v17_sources.rs ;;
    news_aggregator_init.rs) source_file=src/bin/monitor/news_aggregator_init.rs ;;
    classifier.rs) source_file=src/news/aggregator/classifier.rs ;;
  esac
  printf '%s|' "$ref"; sed -n "${source_line}p" "$source_file"
done | shasum -a 256
```

```text
descriptor_anchor_count=119
0a5140804724e6c1fb9c7df94c5210f9d4bd9e277188edafae17b7b969e107e6  -
```

The following four registry-external shapes exhaust the current monitor
presentation bypass inventory. P-05 pilot is Disabled by its empty upstream;
the other three are Active. Gate B keeps the I-01 family/`PushKind` and adds
stable typed identities for replay and health webhook.

| Family | Shape ID | Current lifecycle / kind | Independent non-test evidence and target terminal |
| --- | --- | --- | --- |
| `P-05-virtual-watch` | `P-05-virtual-watch-pilot-snapshot` | Disabled / `VirtualWatch` | `main.rs:6544` fixes `post_close=""`; the only population is inside `main.rs:6558`, so guards at `main.rs:6601` and persistence/raw gateway at `main.rs:6689,6693` are unreachable; reason `producer_input_explicitly_empty` |
| `I-01-intraday-market` | `I-01-intraday-market-board-flow` | Active / `IntradayMarket` | root/acquisition `main.rs:7489,7497` → renderer `main.rs:7501` → raw gateway `main.rs:7510`; Gate B terminal is `notify::push_presented_v3` |
| `X-02-event-replay` | `X-02-event-replay` | Active / currently untyped; additive `EventReplay` | CLI root `main.rs:3529,3541` → selection/assembler `replay.rs:141,174` → publisher `replay.rs:182` → sink `main.rs:2533` → raw `notify::push_wechat` at `main.rs:2355`; Gate B terminal is `notify::push_presented_replay_v3` |
| `X-03-health-webhook` | `X-03-health-webhook` | Active / currently untyped; additive `HealthWebhook` | startup health root `main.rs:3717` → failure branch `main.rs:3720` → assembler `webhook_alert.rs:42,55` → raw HTTP terminal `webhook_alert.rs:19,30,33`; Gate B terminal is `notify::push_presented_health_webhook_v3` and preserves explicit Disabled/Delivered/Error outcomes |

This bounded command verifies every external-row source anchor against the
§1.2 preimage without admitting test hits or claiming to compute reachability:

```bash
refs=(src/bin/monitor/main.rs:6544 src/bin/monitor/main.rs:6558 src/bin/monitor/main.rs:6601 src/bin/monitor/main.rs:6689 src/bin/monitor/main.rs:6693 src/bin/monitor/main.rs:7489 src/bin/monitor/main.rs:7497 src/bin/monitor/main.rs:7501 src/bin/monitor/main.rs:7510 src/bin/monitor/main.rs:3529 src/bin/monitor/main.rs:3541 src/event/replay.rs:141 src/event/replay.rs:174 src/event/replay.rs:182 src/bin/monitor/main.rs:2533 src/bin/monitor/main.rs:2355 src/bin/monitor/main.rs:3717 src/bin/monitor/main.rs:3720 src/bin/monitor/webhook_alert.rs:42 src/bin/monitor/webhook_alert.rs:55 src/bin/monitor/webhook_alert.rs:19 src/bin/monitor/webhook_alert.rs:30 src/bin/monitor/webhook_alert.rs:33)
printf 'external_anchor_count=%s\n' "${#refs[@]}"
for ref in "${refs[@]}"; do source_path=${ref%:*}; source_line=${ref##*:}; printf '%s|' "$ref"; sed -n "${source_line}p" "$source_path"; done | shasum -a 256
```

```text
external_anchor_count=23
aedcdb85cc33bbcc837466e44cabb0df9ec2257eaedc778048a8a63c0e213ca8  -
```

The command deliberately hashes the exact bounded source lines. The table is
the human-reviewed classification authority; the hash detects anchor or source
drift and cannot turn an unreachable guard into an Active producer.

P-05's specific reachability failure is also reproduced without surrounding
test hits:

```bash
nl -ba src/bin/monitor/main.rs | sed -n '6544p;6558p;6587p;6601p;6602p;6835p;6837p;6958p'
rg -n 'virtual_observation\.(push|extend|append)' src/bin/monitor/main.rs
```

```text
  6544	                            let post_close = String::new();
  6558	                            for line in post_close.lines() {
  6587	                                                    virtual_observation.push((
  6601	                            if entry_mode == AirRefuelEntryMode::Pilot
  6602	                                && !virtual_observation.is_empty()
  6835	                        if entry_mode == AirRefuelEntryMode::Confirm
  6837	                            && !virtual_observation.is_empty()
  6958	                            let _ = push_templates::dispatch_virtual_watch_daily(
6587:                                                    virtual_observation.push((
```

The only population sits inside iteration over the literal empty string;
therefore both pilot and confirm guards are unreachable in the reviewed
preimage.

### 2.3 Frozen lifecycle matrices

There is one matrix. News pipeline registration cannot activate N-01/N-02
because neither has a non-test upstream caller.

| Projection | Active | Disabled | SpecOnly | Total |
| --- | ---: | ---: | ---: | ---: |
| Shape (current audit and target) | 28 | 26 | 14 | 68 |
| Family (current audit and target) | 27 | 25 | 14 | 66 |
| PushKind (current source enum; X-02/X-03 untyped) | 22 | 24 | 12 | 58 |
| PushKind (Gate B target) | 24 | 24 | 12 | 60 |

The exact Active shape set is the following 28 IDs:

```text
T-01-account-mode
T-02-data-mode
T-02-data-mode-reminder
T-05-t0-advice
T-10-paper-trade
T-11-auction-volume
R-04-review-lhb-gateway
R-09-provider-top-n
P-01-preopen-news-hot
I-01-intraday-market
I-02-news-catalyst
D-01-news-to-idea
A-10-catalyst-review
I-03-industry-chain-intraday
T-16-st-price-limit-changed
A-01-paper-review
R-08-public-event-calendar
L-01-limit-boards-first
L-02-limit-boards-second
L-03-limit-boards-third-plus
S-01-announcement
S-03-earnings-beat
S-04-earnings-miss
S-05-analyst-upgrade
S-06-market-action-alert
I-01-intraday-market-board-flow
X-02-event-replay
X-03-health-webhook
```

The exact 26 Disabled shape IDs are the 25 Disabled descriptor rows in §2.2
plus `P-05-virtual-watch-pilot-snapshot`. The 25 Disabled family IDs are the
descriptor IDs; the pilot reuses `P-05-virtual-watch`. The 14 SpecOnly
shape/family IDs are exactly the rows below. No catalog token adds an ID to
either Active set.

```text
P-05-virtual-watch P-05-virtual-watch-pilot-snapshot T-03-holding-plan
T-04-holding-event T-06-t0-forbid T-07-candidate-triggered
T-08-candidate-invalidated T-09-forbidden-ops T-12-close-call
I-09-sector-top T-13-turnover-top R-01-daily-report R-02-review-market
R-03-industry-chain R-05-review-signal R-06-review-failure
R-07-tomorrow-watch I-09-sector-anomaly T-14-post-fixed-price-order
T-15-post-fixed-price-fill T-17-etf-closing-call-auction
BR-033-block-trade-confirm BR-034-block-trade-range S-02-policy-hit
N-01-news-flash-critical N-02-news-flash-aggregated
```

The exact Active family set is the 25 Active descriptor IDs plus
`X-02-event-replay` and `X-03-health-webhook`. The I-01 inline shape reuses
`I-01-intraday-market`; therefore its cardinality is 27:

```text
T-01-account-mode T-02-data-mode T-02-data-mode-reminder T-05-t0-advice T-10-paper-trade
T-11-auction-volume R-04-review-lhb-gateway R-09-provider-top-n
P-01-preopen-news-hot I-01-intraday-market I-02-news-catalyst
D-01-news-to-idea A-10-catalyst-review I-03-industry-chain-intraday
T-16-st-price-limit-changed A-01-paper-review R-08-public-event-calendar
L-01-limit-boards-first L-02-limit-boards-second L-03-limit-boards-third-plus
S-01-announcement S-03-earnings-beat S-04-earnings-miss
S-05-analyst-upgrade S-06-market-action-alert X-02-event-replay
X-03-health-webhook
```

The exact Disabled family set is:

```text
P-05-virtual-watch T-03-holding-plan T-04-holding-event T-06-t0-forbid
T-07-candidate-triggered T-08-candidate-invalidated T-09-forbidden-ops
T-12-close-call I-09-sector-top T-13-turnover-top R-01-daily-report
R-02-review-market R-03-industry-chain R-05-review-signal R-06-review-failure
R-07-tomorrow-watch I-09-sector-anomaly T-14-post-fixed-price-order
T-15-post-fixed-price-fill T-17-etf-closing-call-auction
BR-033-block-trade-confirm BR-034-block-trade-range S-02-policy-hit
N-01-news-flash-critical N-02-news-flash-aggregated
```

The exact current 22 Active PushKinds are:

```text
AccountMode DataMode T0Advice PaperTrade AuctionVolume ReviewLhb
ReviewProviderTopN PreopenNewsHot IntradayMarket NewsCatalyst NewsToIdea
CatalystReview IndustryChainIntraday StPriceLimitChanged PaperReview
EventCalendar LimitBoards Announcement EarningsBeat EarningsMiss
AnalystUpgrade MarketActionAlert
```

The exact Gate B target 24 Active PushKinds are:

```text
AccountMode DataMode T0Advice PaperTrade AuctionVolume ReviewLhb
ReviewProviderTopN PreopenNewsHot IntradayMarket NewsCatalyst NewsToIdea
CatalystReview IndustryChainIntraday StPriceLimitChanged PaperReview
EventCalendar LimitBoards Announcement EarningsBeat EarningsMiss
AnalystUpgrade MarketActionAlert EventReplay HealthWebhook
```

The exact 24 Disabled-only PushKinds are:

```text
HoldingPlan HoldingEvent CandidateTriggered CandidateInvalidated ForbiddenOps VirtualWatch
CloseCall SectorTop TurnoverTop DailyReport ReviewMarket IndustryChain
ReviewSignal ReviewFailure TomorrowWatch SectorAnomaly PostFixedPriceOrder
PostFixedPriceFill EtfClosingCallAuction BlockTradeIntradayConfirm
BlockTradePriceRange PolicyHit NewsFlashCritical NewsFlashAggregated
```

The exact 12 SpecOnly-only PushKinds are:

```text
FundInflow FactorIC SectorTier CapitalVerify WeeklySOP StockPick CandidateBoard
NewsRanked IpoListingApproval IpoProspectus IpoCatalyst AuctionRepush
```

Independent arithmetic from those sets is:

- Shape: `(25 descriptor Active + 3 external Active) + (25 descriptor Disabled + 1 external Disabled) + 14 = 68`.
- Family: `(25 descriptor Active + 2 additive external families) + 25 + 14 = 66`.
- Current PushKind: `22 Active + 24 Disabled + 12 SpecOnly = 58`; replay and health are two Active raw shapes with no current enum identity.
- Gate B PushKind: `(22 current Active + EventReplay + HealthWebhook) + 24 + 12 = 60`.

`DataMode` projects two Active shapes and `LimitBoards` projects three.
`T-06-t0-forbid` shares Active `T0Advice`; historical R-04/R-08 SpecOnly rows
share Active kinds. These overlaps explain every projection reduction.

The 14 SpecOnly rows are exact and retain their existing IDs:

| Shape/family ID | PushKind | Exact reason |
| --- | --- | --- |
| `M-01-fund-inflow` | `FundInflow` | `template_contract_not_live_admitted` |
| `M-02-factor-ic` | `FactorIC` | `template_contract_not_live_admitted` |
| `M-03-sector-tier` | `SectorTier` | `template_contract_not_live_admitted` |
| `M-04-capital-verify` | `CapitalVerify` | `template_contract_not_live_admitted` |
| `M-05-weekly-sop` | `WeeklySOP` | `template_contract_not_live_admitted` |
| `M-06-stock-pick` | `StockPick` | `template_contract_not_live_admitted` |
| `M-07-candidate-board` | `CandidateBoard` | `template_contract_not_live_admitted` |
| `M-08-news-ranked` | `NewsRanked` | `template_contract_not_live_admitted` |
| `M-09-ipo-listing-approval` | `IpoListingApproval` | `template_contract_not_live_admitted` |
| `M-10-ipo-prospectus` | `IpoProspectus` | `template_contract_not_live_admitted` |
| `M-11-ipo-catalyst` | `IpoCatalyst` | `template_contract_not_live_admitted` |
| `R-04-review-lhb-legacy` | `ReviewLhb` | `superseded_by_gateway_renderer` |
| `R-08-event-calendar` | `EventCalendar` | `superseded_by_public_source_only_renderer` |
| `X-01-auction-repush` | `AuctionRepush` | `production_call_deleted_v13_10_1` |

The following bounded artifact command independently counts every frozen set;
its output reproduces the matrices above without reading registry/catalog
cardinality claims:

```bash
design_file=docs/superpowers/specs/2026-08-01-monitor-test-template-delivery-design.md
count_fence_after() {
  marker=$1
  awk -v marker="$marker" 'index($0,marker){found=1; next} found && $0=="```text"{block=1; next} block && $0=="```"{print n+0; exit} block{n+=NF}' "$design_file"
}
printf 'active_shapes=%s\n' "$(count_fence_after 'exact Active shape set')"
printf 'disabled_shapes=%s\n' "$(count_fence_after 'exact 26 Disabled shape IDs')"
printf 'active_families=%s\n' "$(count_fence_after 'exact Active family set')"
printf 'disabled_families=%s\n' "$(count_fence_after 'exact Disabled family set')"
printf 'current_active_kinds=%s\n' "$(count_fence_after 'exact current 22 Active PushKinds')"
printf 'target_active_kinds=%s\n' "$(count_fence_after 'exact Gate B target 24 Active')"
printf 'disabled_kinds=%s\n' "$(count_fence_after 'exact 24 Disabled-only')"
printf 'spec_kinds=%s\n' "$(count_fence_after 'exact 12 SpecOnly-only')"
awk '/^\| `M-01-fund-inflow`/{inside=1} inside && /^\| `/{n++} /^\| `X-01-auction-repush`/{print "spec_rows=" n; exit}' "$design_file"
```

```text
active_shapes=28
disabled_shapes=26
active_families=27
disabled_families=25
current_active_kinds=22
target_active_kinds=24
disabled_kinds=24
spec_kinds=12
spec_rows=14
```

Counts alone are insufficient. This second bounded artifact command derives
sets from the 50-row and four-row audits and compares them with every frozen
set byte-for-byte after sorting:

```bash
design_file=docs/superpowers/specs/2026-08-01-monitor-test-template-delivery-design.md
fence_set_after() {
  marker=$1
  awk -v marker="$marker" 'index($0,marker){found=1;next} found&&$0=="```text"{block=1;next} block&&$0=="```"{exit} block{for(i=1;i<=NF;i++) print $i}' "$design_file" | LC_ALL=C sort -u
}
descriptor_set() {
  lifecycle=$1
  awk -F'|' -v lifecycle="$lifecycle" '/^\| [0-9]+ \| `/{state=$4; gsub(/[[:space:]]/,"",state); if(state==lifecycle){cell=$3; gsub(/`/,"",cell); split(cell,a," / "); gsub(/^[[:space:]]+|[[:space:]]+$/,"",a[1]); print a[1]}}' "$design_file" | LC_ALL=C sort -u
}
descriptor_kind_set() {
  lifecycle=$1
  awk -F'|' -v lifecycle="$lifecycle" '/^\| [0-9]+ \| `/{state=$4; gsub(/[[:space:]]/,"",state); if(state==lifecycle){cell=$3; gsub(/`/,"",cell); split(cell,a," / "); gsub(/^[[:space:]]+|[[:space:]]+$/,"",a[2]); print a[2]}}' "$design_file" | LC_ALL=C sort -u
}
external_set() {
  field_no=$1; lifecycle=$2
  awk -F'|' -v field_no="$field_no" -v lifecycle="$lifecycle" '/following four registry-external/{scope=1} scope&&/^\| `/{state=$4; gsub(/^[[:space:]]+|[[:space:]]+$/,"",state); if(index(state,lifecycle)==1){v=$field_no; gsub(/`/,"",v); gsub(/^[[:space:]]+|[[:space:]]+$/,"",v); print v}} /This bounded command/{exit}' "$design_file" | LC_ALL=C sort -u
}
active_shapes=$( { descriptor_set Active; external_set 3 Active; } | LC_ALL=C sort -u)
disabled_shapes=$( { descriptor_set Disabled; external_set 3 Disabled; } | LC_ALL=C sort -u)
active_families=$( { descriptor_set Active; external_set 2 Active; } | LC_ALL=C sort -u)
disabled_families=$( { descriptor_set Disabled; external_set 2 Disabled; } | LC_ALL=C sort -u)
current_active_kinds=$(descriptor_kind_set Active)
all_disabled_kinds=$(descriptor_kind_set Disabled)
disabled_only_kinds=$(comm -23 <(printf '%s\n' "$all_disabled_kinds") <(printf '%s\n' "$current_active_kinds"))
target_additions=$(awk '/following four registry-external/{scope=1} scope{print} /This bounded command/{exit}' "$design_file" | rg -o 'additive `[^`]+`' | sed -E 's/additive `([^`]+)`/\1/' | LC_ALL=C sort -u)
target_active_kinds=$( { printf '%s\n' "$current_active_kinds"; printf '%s\n' "$target_additions"; } | LC_ALL=C sort -u)
spec_all_kinds=$(awk -F'|' '/^\| `M-01-fund-inflow`/{scope=1} scope&&/^\| `/{v=$3; gsub(/`/,"",v); gsub(/^[[:space:]]+|[[:space:]]+$/,"",v); print v} /^\| `X-01-auction-repush`/{exit}' "$design_file" | LC_ALL=C sort -u)
spec_only_kinds=$(comm -23 <(printf '%s\n' "$spec_all_kinds") <(printf '%s\n' "$target_active_kinds"))
printf 'active_shape_set_diff=%s\n' "$(comm -3 <(printf '%s\n' "$active_shapes") <(fence_set_after 'exact Active shape set') | wc -l | tr -d ' ')"
printf 'disabled_shape_set_diff=%s\n' "$(comm -3 <(printf '%s\n' "$disabled_shapes") <(fence_set_after 'exact 26 Disabled shape IDs') | wc -l | tr -d ' ')"
printf 'active_family_set_diff=%s\n' "$(comm -3 <(printf '%s\n' "$active_families") <(fence_set_after 'exact Active family set') | wc -l | tr -d ' ')"
printf 'disabled_family_set_diff=%s\n' "$(comm -3 <(printf '%s\n' "$disabled_families") <(fence_set_after 'exact Disabled family set') | wc -l | tr -d ' ')"
printf 'current_active_kind_set_diff=%s\n' "$(comm -3 <(printf '%s\n' "$current_active_kinds") <(fence_set_after 'exact current 22 Active PushKinds') | wc -l | tr -d ' ')"
printf 'target_active_kind_set_diff=%s\n' "$(comm -3 <(printf '%s\n' "$target_active_kinds") <(fence_set_after 'exact Gate B target 24 Active') | wc -l | tr -d ' ')"
printf 'disabled_kind_set_diff=%s\n' "$(comm -3 <(printf '%s\n' "$disabled_only_kinds") <(fence_set_after 'exact 24 Disabled-only') | wc -l | tr -d ' ')"
printf 'spec_kind_set_diff=%s\n' "$(comm -3 <(printf '%s\n' "$spec_only_kinds") <(fence_set_after 'exact 12 SpecOnly-only') | wc -l | tr -d ' ')"
```

```text
active_shape_set_diff=0
disabled_shape_set_diff=0
active_family_set_diff=0
disabled_family_set_diff=0
current_active_kind_set_diff=0
target_active_kind_set_diff=0
disabled_kind_set_diff=0
spec_kind_set_diff=0
```

## 3. Inactive startup banner and audit contract

Every one of the 26 Disabled and 14 SpecOnly rows emits exactly this line,
with literal field order and no optional fields:

```text
[BR-196] shape=<shape_id> lifecycle=<lifecycle> disabled=<reason> producer=none
```

`<lifecycle>` becomes the literal `Disabled` or `SpecOnly`. `<reason>` becomes
the exact reason code in §2.2 or §2.3. There is no combined family banner and
no silent omission. Gate B emits one line for every one of the 40 rows.

Before writing a line, startup appends a
`br196.presentation_inactive.v1` record containing `schema_version`,
`manifest_version`, `manifest_sha256`, `source_tree_manifest_sha256`, committed
source revision, environment,
process-invocation identity hash, local timestamp, `family_id`, `shape_id`,
`push_kind`, lifecycle, exact reason, `producer="none"`, rule IDs, decision
status, and `retryable=false`. The raw process nonce and credentials are not
persisted.

Dedup scope is exactly
`(process_invocation_sha256, manifest_version, shape_id)`. The append-only
writer rejects a second record for the same key and rejects missing/extra
inactive rows. After durable append and `sync_data`, a dedicated startup writer
writes and flushes the exact line to stderr. Lock, chain validation, append,
flush, sync, stderr write, stderr flush, duplicate, count, or field mismatch
exits 2 before fixture construction, provider acquisition, sink construction,
target resolution, or external process construction. A generic logger that
cannot report write success is not the banner authority.

## 4. Manifest and routing metadata

Gate B replaces the current family-only lifecycle projection with explicit
shape rows. Each row has stable `family_id`, stable `shape_id`, optional
existing `template_id`, `push_kind`, one lifecycle, exact reason when inactive,
stable ordinal, manifest version, and routing seam IDs only when Active.

The production registry is routing metadata, not reachability authority. Gate
B removes the 25 Disabled descriptors/tokens, adds the I-01 board-flow shape
and additive `X-02-event-replay` plus `X-03-health-webhook`, and targets 28
unique routing descriptors. It may check agreement with the 28 Active
manifest rows for drift only.

Disabled and SpecOnly rows have no production token, fixture builder, render,
provider, or sink call. Registry agreement is never caller or production
observation evidence.

Missing/duplicate ID, ordinal drift, unknown lifecycle, incorrect reason,
matrix drift, inactive routing metadata, or an Active row outside the reviewed
set fails before any side effect.

## 5. Typed fixture and governance-smoke boundary

Every Active fixture uses a private typed identity whose constructor accepts
only canonical `TEST_CODE_...` values and rejects six-digit production codes,
empty values, whitespace, and aliases. Nested identities are validated before
render. Missing fields stay absent and source failures stay explicit.

The existing six governance-smoke identities remain an orthogonal governance
test:

```text
{
  (D-01-news-to-idea, NewsToIdea),
  (I-02-news-catalyst, NewsCatalyst),
  (P-01-preopen-news-hot, PreopenNewsHot),
  (T-11-auction-volume, AuctionVolume),
  (R-03-industry-chain, IndustryChain),
  (A-10-catalyst-review, CatalystReview)
}
```

The multiset must be exact and each outcome must be `Pushed` under the
invocation-scoped test governance context. Because R-03 is Disabled, this
smoke must use a dedicated test-only governance capability and must not acquire
an R-03 production descriptor or count R-03 as a rendered/transported Active
shape. Smoke success means only `governance_smoke_passed`; it cannot change a
lifecycle, prove a caller, prove production data, or prove Feishu receipt.

Any missing/extra/duplicate tuple or non-Pushed outcome exits 2 before live
target authority, transport, or receipt-audit construction.

## 6. CLI and transport behavior

### 6.1 `monitor --test --push-dry-run`

Dry-run validates the exact matrix, 28 Active fixtures/renders, stable ordering,
inactive banners/audits, and governance smoke. It performs zero target
resolution, external notification-process construction, network send, and
receipt-audit append. Every Active shape terminates as `explicit_dry_run`.

### 6.2 Bare `monitor --test`

Bare test is live only when all of the following succeed:

1. exact `BR196_LIVE_FEISHU_ACCEPTANCE=1` operator intent;
2. governance smoke;
3. side-effect-free resolution of a target identity;
4. exact match in the release-pinned
   `config/br196_non_production_feishu_targets.toml`
   `non_production_acceptance` hash allowlist;
5. a non-cloneable invocation-bound permit with exclusive expiry;
6. target/allowlist/time revalidation immediately before every spawn; and
7. one structurally valid real Feishu receipt per bounded batch, durably
   audited in the TEST_CODE namespace.

The allowlist is currently empty, so bare `--test` must return a typed
`non_production_feishu_target_not_allowlisted` failure and exit 2 with zero
external process, network, and receipt-audit calls. It must not fall back to a
default target, alias, first configured target, or production target.

Receipts, raw target identities, credentials, webhook values, account
identities, real holdings, and announcement identities never enter PR evidence.

## 7. Result model and failure behavior

The summary separately reports:

- manifest version/hash, source-tree manifest hash, and committed source revision;
- Shape/Family/PushKind Active, Disabled, SpecOnly, and Total counts;
- inactive audit/banner attempted/appended/emitted counts;
- rendered Active shape total;
- governance smoke attempted/passed;
- opt-in and target-authority status plus redacted hashes;
- external process and batch attempted/pushed counts;
- shapes pushed and receipt audits appended;
- explicit dry-run count; and
- failed status and typed reason.

Success requires the exact §2.3 matrix and exactly one inactive audit/banner
record for each of the 26 Disabled and 14 SpecOnly rows. Dry-run additionally
requires `rendered_shape_total=28`,
`explicit_dry_run_shape_total=28`, and all transport/receipt counters zero.
Bare test additionally requires a valid non-production target authority,
`rendered_shape_total=28`, `shapes_pushed=28`, every batch confirmed, and one
receipt audit per batch. With the current empty allowlist, bare-test success is
not possible.

Any lifecycle, matrix, identity, banner, audit, smoke, target, permit, spawn,
receipt, or namespace failure is explicit and exits 2. Earlier valid TEST_CODE
receipts remain immutable and are not automatically resent. No failure falls
back to mock/default/empty production evidence.

## 8. Gate B implementation debt and validation

This Gate A repair changes only this design, the unique BR-196 row, and the
non-behavioral config citation described below. Gate B must repair, but this
task must not edit production Rust:

1. Gate A adds only the comment
   `# BR-196: release-pinned non-production target authority; metadata only.`
   to `config/br196_non_production_feishu_targets.toml`. Removing comments and
   blank lines before and after yields the identical SHA-256
   `9c047e5fcb2fea54e68c594f66a5f4a21ee7810dff85bc96b12240763ded1aaf`;
   version, empty acceptance allowlist, production-deny hash, targets,
   thresholds, and runtime behavior are byte/semantically unchanged.

   ```bash
   sed '/^[[:space:]]*#/d;/^[[:space:]]*$/d' config/br196_non_production_feishu_targets.toml | shasum -a 256
   sed '/^[[:space:]]*#/d;/^[[:space:]]*$/d' config/br196_non_production_feishu_targets.toml
   ```

   ```text
   9c047e5fcb2fea54e68c594f66a5f4a21ee7810dff85bc96b12240763ded1aaf  -
   version = "BR196_FEISHU_TARGETS_V1"
   [non_production_acceptance]
   target_sha256 = []
   [production_deny]
   target_sha256 = [
     "0f5755c48e678964a6e3fe0179077f5d743b61d166b1357df49399b17d129bb5",
   ]
   ```

   The current checker is still globally RED because three BR-201 paths are
   absent. The former BR-196 config hard error is gone. This bounded output is
   exact; BR-196 source-citation warnings are not rewritten as success and may
   be removed only by later metadata comments in those source owners:

   ```bash
   set +e
   checker_output=$(bash tools/compliance/lib/check_business_rules.sh 2>&1); checker_rc=$?
   set -e
   printf 'business_rule_checker_rc=%s\n' "$checker_rc"
   printf '%s\n' "$checker_output" | rg 'BR-196|business-rule gate failed'
   ```

   ```text
   business_rule_checker_rc=1
   ⚠ §2.10 active path src/bin/monitor/health.rs does not cite BR-196
   ⚠ §2.10 active path src/bin/monitor/l6_sink.rs does not cite BR-196
   ⚠ §2.10 active path src/bin/monitor/webhook_alert.rs does not cite BR-196
   ⚠ §2.10 active path src/event/history.rs does not cite BR-196
   ⚠ §2.10 active path src/event/replay.rs does not cite BR-196
   ⚠ §2.10 active path src/monitor/alert.rs does not cite BR-196
   ⚠ §2.10 active path src/news/aggregator/classifier.rs does not cite BR-196
   ⚠ §2.10 active path src/opportunity/candidate_state.rs does not cite BR-196
   ⚠ §2.10 active path src/opportunity/mod.rs does not cite BR-196
   ⚠ §2.10 active path src/opportunity/scheduler.rs does not cite BR-196
   ✗ §2.10 business-rule gate failed (3 errors, 141 warnings)
   ```
2. Strict Clippy reports `clippy::manual_contains` at
   `src/bin/monitor/br196_test_delivery.rs:278`; Gate B must use the typed
   `.contains(...)` form rather than adding an allow.
3. Current implementation still treats all declared routing metadata as
   reachable and uses the old lifecycle/count projection. It must implement
   the exact sets, banners, audits, and matrix in this design.
4. The inactive startup authority is absent. This exact probe currently emits
   `startup_inactive_banner_or_audit_hits=0`:

   ```bash
   count=$(rg -n '\[BR-196\] shape=.*lifecycle=.*disabled=.*producer=none|br196\.presentation_inactive\.v1' src --glob '*.rs' | wc -l | tr -d ' '); echo "startup_inactive_banner_or_audit_hits=$count"
   ```

   Gate B implements the fatal-before-construction banner/audit contract; a
   function definition or test string is not implementation evidence.
5. Add `PushKind::EventReplay` and `PushKind::HealthWebhook`, descriptors and
   families/shapes `X-02-event-replay` and `X-03-health-webhook`, typed
   `ReplayPresentationEvidence` and `HealthWebhookPresentationEvidence`, plus
   `notify::push_presented_replay_v3` and
   `notify::push_presented_health_webhook_v3`.
   `src/event/replay.rs:141-175` remains replay selector/assembler authority;
   startup health evidence remains owned by `health.rs`.
6. Migrate `main.rs:2533` / `RealReplayNotificationSink` from
   `notify::push_wechat` to the typed replay gateway. Preserve the exact
   `[REPLAY {date_str}] {source_text}` bytes, `fresh_replay_id`, `replay_of`,
   source identity, ordering, rate spacing, and per-envelope result mapping.
7. Replay evidence binds the fresh replay ID and original ID outside the
   optional securities-code field. Missing/mismatched IDs, non-`push.source`,
   missing marker, gateway denial, dedup, sink failure, or audit failure stays
   explicit; none is converted to `published`.
8. Migrate `main.rs:3717-3720` / `webhook_alert.rs:19-55` from direct HTTP to
   the typed health-webhook gateway. Preserve event `health_check_fail`,
   redacted endpoint handling, and explicit Disabled/Delivered/Error results;
   missing URL or transport failure must not become success.
9. Apply every API disposition in §9. No raw-governor compatibility shim may
   reopen a production bypass. Compile and fixed current-callsite searches are
   required for every breaking migration.
10. Bind generated manifest/audit rows to a real committed source revision plus
   the post-Gate-B source-tree manifest hash. Never label an uncommitted
   working tree as `HEAD`.

Gate B/C/D validation commands remain:

```bash
cargo test --bin monitor br196_ -- --test-threads=1
cargo test --test monitor_help_isolation -- --test-threads=1
cargo run --bin monitor -- --test --push-dry-run
rg -n -U -A18 -B4 'EventCommand::Replay|ReplayRunner::new|payload\["text"\]|push_presented_replay_v3\(|notify::push_wechat\(' src/bin/monitor/main.rs src/event/replay.rs src/bin/monitor/notify.rs
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Gate D also reruns the fixed 2026-08-02 evidence probes in §1, then uses the
actual release date for production push-log/event-bus evidence. A zero result
keeps Gate D RED. The empty non-production Feishu allowlist makes bare-test
live acceptance fail closed until a separately reviewed target is pinned.

Gate A is not claimed. A fresh reviewer must independently reproduce every
row in §2.2, confirm the arithmetic and inactive contract, and return
Critical=0/Important=0 before Gate B begins.

## 9. Old-module disposition and rollback

### 9.1 HEAD/current public API audit

The audit is bounded to the exact eighteen paths in §1.2 and compares the
fixed base commit with the reviewed working tree. It lexes Rust tokens and
enumerates every `pub`, `pub(crate)`, and `pub(super)` free item and method,
public struct field, every variant of a public enum, every public trait
member, and public `type`/`const`/`static`/`use`/module declaration. A `use`
row therefore also captures a re-export. It does not search only for
`push_`/`dispatch_` names and does not treat private local syntax as API.
A HEAD-only plus current-only pair with the same path/name is a visibility or
signature change; a single side is a removal or addition.

```bash
python3 - <<'PY'
import hashlib
import pathlib
import re
import subprocess
from collections import Counter

BASE = "b4aeee68d2c0259cc968914b3d39e3a89a18a496"
PATHS = """src/bin/monitor/br196_test_delivery.rs
src/bin/monitor/br196_transport.rs
src/bin/monitor/health.rs
src/bin/monitor/l6_sink.rs
src/bin/monitor/main.rs
src/bin/monitor/news_aggregator_init.rs
src/bin/monitor/notify.rs
src/bin/monitor/presentation_registry.rs
src/bin/monitor/push_templates.rs
src/bin/monitor/v17_sources.rs
src/bin/monitor/webhook_alert.rs
src/event/history.rs
src/event/replay.rs
src/monitor/alert.rs
src/news/aggregator/classifier.rs
src/opportunity/candidate_state.rs
src/opportunity/mod.rs
src/opportunity/scheduler.rs""".splitlines()
RX = re.compile(r'''(?x)
(?:br|rb|b|r)?r\#{0,8}"(?:\\.|[^"\\])*"\#{0,8}
|(?:b|c)?"(?:\\.|[^"\\\n])*"
|'[A-Za-z_][A-Za-z0-9_]*
|(?:b|c)?'(?:\\.|[^'\\\n])'
|//[^\n]*|/\*(?:.|\n)*?\*/
|[A-Za-z_][A-Za-z0-9_]*|[0-9]+(?:\.[0-9]+)?
|::|->|=>|\.\.=|\.\.|==|!=|<=|>=|&&|\|\||<<|>>|\+=|-=|\*=|/=|%=|&=|\|=|\^=
|[^\s]
''')

def toks(src):
    out = []
    for match in RX.finditer(src):
        value = match.group(0)
        if not value.startswith(("//", "/*")):
            out.append(value)
    return out

def canon(tokens):
    return " ".join(tokens)

def pairs(tokens):
    closing = {"{": "}", "(": ")", "[": "]"}
    stack, matched = [], {}
    for index, token in enumerate(tokens):
        if token in closing:
            stack.append((token, index))
        elif token in closing.values() and stack and closing[stack[-1][0]] == token:
            _, opening = stack.pop()
            matched[opening] = index
            matched[index] = opening
    return matched

def attrs_end(tokens, index, matched, end):
    while index + 1 < end and tokens[index:index + 2] == ["#", "["] and index + 1 in matched:
        index = matched[index + 1] + 1
    return index

def top_stop(tokens, index, end, matched, stops):
    while index < end:
        if tokens[index] in ("(", "[", "{") and index in matched:
            if tokens[index] in stops:
                return index
            index = matched[index] + 1
            continue
        if tokens[index] in stops:
            return index
        index += 1
    return end

def split_entries(tokens, start, end, matched):
    entries, entry_start, index = [], start, start
    while index < end:
        if tokens[index] in ("(", "[", "{") and index in matched:
            index = matched[index] + 1
            continue
        if tokens[index] == ",":
            if entry_start < index:
                entries.append(tokens[entry_start:index])
            entry_start = index + 1
        index += 1
    if entry_start < end:
        entries.append(tokens[entry_start:end])
    return entries

def strip_attrs(entry):
    index = 0
    while index + 1 < len(entry) and entry[index:index + 2] == ["#", "["]:
        depth, index = 1, index + 2
        while index < len(entry) and depth:
            depth += (entry[index] == "[") - (entry[index] == "]")
            index += 1
    return entry[index:]

def extract(path, src):
    tokens = toks(src)
    matched = pairs(tokens)
    rows = []

    def emit(kind, owner, name, signature):
        rows.append(f"{path}|{kind}|{owner}|{name}|{canon(signature)}")

    def vis_after_list(entry, index=0):
        cursor = index + 1
        if cursor < len(entry) and entry[cursor] == "(":
            depth, cursor = 1, cursor + 1
            while cursor < len(entry) and depth:
                depth += (entry[cursor] == "(") - (entry[cursor] == ")")
                cursor += 1
        return cursor

    def scan(start, end, owner="module"):
        index = start
        while index < end:
            item = attrs_end(tokens, index, matched, end)
            if item >= end:
                break
            if tokens[item] == "impl":
                body = top_stop(tokens, item, end, matched, {"{"})
                if body < end and body in matched:
                    scan(body + 1, matched[body], "impl " + canon(tokens[item + 1:body]))
                    index = matched[body] + 1
                    continue
            if tokens[item] == "mod" and item + 1 < end:
                body = top_stop(tokens, item, end, matched, {"{", ";"})
                if body < end and tokens[body] == "{" and body in matched:
                    scan(body + 1, matched[body], owner + "::" + tokens[item + 1])
                    index = matched[body] + 1
                    continue
            if tokens[item] != "pub":
                cursor = item
                while cursor < end and tokens[cursor] in ("async", "unsafe", "const", "extern"):
                    cursor += 1
                if cursor < end and tokens[cursor] in ("fn", "struct", "enum", "trait"):
                    body = top_stop(tokens, cursor, end, matched, {"{", ";"})
                    if body < end and tokens[body] == "{" and body in matched:
                        index = matched[body] + 1
                        continue
                index = item + 1
                continue
            visibility_end = item + 1
            if visibility_end < end and tokens[visibility_end] == "(" and visibility_end in matched:
                visibility_end = matched[visibility_end] + 1
            cursor = visibility_end
            while cursor < end and tokens[cursor] in ("async", "unsafe", "const", "extern", "default"):
                cursor += 1
            if cursor >= end:
                break
            kind = tokens[cursor]
            if kind == "fn":
                name = tokens[cursor + 1] if cursor + 1 < end else "?"
                body = top_stop(tokens, cursor + 2, end, matched, {"{", ";"})
                emit("fn", owner, name, tokens[item:body])
                index = matched[body] + 1 if body < end and tokens[body] == "{" and body in matched else body + 1
                continue
            if kind in ("type", "const", "static", "use"):
                stop = top_stop(tokens, cursor + 1, end, matched, {";"})
                name = tokens[cursor + 1] if kind != "use" and cursor + 1 < end else canon(tokens[cursor + 1:stop])
                emit(kind, owner, name, tokens[item:stop + 1])
                index = stop + 1
                continue
            if kind in ("struct", "enum", "trait"):
                name = tokens[cursor + 1] if cursor + 1 < end else "?"
                body = top_stop(tokens, cursor + 2, end, matched, {"{", "(", ";"})
                if body >= end:
                    index = cursor + 1
                    continue
                if tokens[body] == "(":
                    close = matched.get(body, body)
                    semi = top_stop(tokens, close + 1, end, matched, {";"})
                    emit(kind, owner, name, tokens[item:semi + 1])
                    index = semi + 1
                    continue
                if tokens[body] == ";":
                    emit(kind, owner, name, tokens[item:body + 1])
                    index = body + 1
                    continue
                close = matched[body]
                emit(kind, owner, name, tokens[item:body])
                entries = split_entries(tokens, body + 1, close, matched)
                if kind == "struct":
                    for entry in entries:
                        entry = strip_attrs(entry)
                        if entry and entry[0] == "pub":
                            field = vis_after_list(entry)
                            emit("field", owner + "::" + name, entry[field] if field < len(entry) else "?", entry)
                elif kind == "enum":
                    for entry in entries:
                        entry = strip_attrs(entry)
                        if entry:
                            variant = next((i for i, token in enumerate(entry) if re.match(r"^[A-Za-z_]", token)), None)
                            if variant is not None:
                                emit("variant", owner + "::" + name, entry[variant], entry)
                else:
                    for entry in entries:
                        entry = strip_attrs(entry)
                        if entry:
                            member = next((entry[i + 1] for i, token in enumerate(entry[:-1]) if token in ("fn", "type", "const") and re.match(r"^[A-Za-z_]", entry[i + 1])), "?")
                            emit("trait_member", owner + "::" + name, member, entry)
                index = close + 1
                continue
            if kind == "mod" and cursor + 1 < end:
                name = tokens[cursor + 1]
                body = top_stop(tokens, cursor + 2, end, matched, {"{", ";"})
                emit("mod", owner, name, tokens[item:body])
                if body < end and tokens[body] == "{" and body in matched:
                    scan(body + 1, matched[body], owner + "::" + name)
                    index = matched[body] + 1
                else:
                    index = body + 1
                continue
            index = item + 1

    scan(0, len(tokens))
    return sorted(set(rows))

def at_head(path):
    result = subprocess.run(["git", "show", f"{BASE}:{path}"], text=True, capture_output=True)
    return result.stdout if result.returncode == 0 else ""

head, current = set(), set()
for path in PATHS:
    head.update(extract(path, at_head(path)))
    current.update(extract(path, pathlib.Path(path).read_text()))
delta = ([f"HEAD_ONLY|{row}" for row in sorted(head - current)] +
         [f"CURRENT_ONLY|{row}" for row in sorted(current - head)])
blob = ("\n".join(delta) + "\n").encode()
pathlib.Path("/private/tmp/br196-public-surface-v1.txt").write_bytes(blob)
print(f"public_surface_head={len(head)}")
print(f"public_surface_current={len(current)}")
print(f"public_surface_delta_lines={len(delta)}")
print("public_surface_delta_sha256=" + hashlib.sha256(blob).hexdigest())
print("delta_by_direction=" + ",".join(f"{key}:{value}" for key, value in sorted(Counter(row.split("|", 1)[0] for row in delta).items())))
print("delta_by_kind=" + ",".join(f"{key}:{value}" for key, value in sorted(Counter(row.split("|")[2] for row in delta).items())))
print("delta_by_file=" + ",".join(f"{key}:{value}" for key, value in sorted(Counter(row.split("|")[1] for row in delta).items())))
PY
```

```text
public_surface_head=1091
public_surface_current=1143
public_surface_delta_lines=278
public_surface_delta_sha256=5219a3521583f1a97ef8716a8457d21673fbe1d9b23668e215bd8c4363a5a69b
delta_by_direction=CURRENT_ONLY:165,HEAD_ONLY:113
delta_by_kind=enum:8,field:102,fn:112,mod:3,struct:21,trait_member:2,use:2,variant:28
delta_by_file=src/bin/monitor/br196_test_delivery.rs:58,src/bin/monitor/br196_transport.rs:15,src/bin/monitor/main.rs:2,src/bin/monitor/news_aggregator_init.rs:11,src/bin/monitor/notify.rs:23,src/bin/monitor/presentation_registry.rs:9,src/bin/monitor/push_templates.rs:111,src/bin/monitor/v17_sources.rs:12,src/news/aggregator/classifier.rs:2,src/opportunity/candidate_state.rs:22,src/opportunity/mod.rs:13
```

The following deterministic classifier covers all 278 rows. Classification is
an API compatibility inventory, not lifecycle or reachability evidence. A
category marked outside BR-196 ownership is preserved and compile-checked; it
is not silently reverted.

```bash
python3 - <<'PY'
from collections import Counter

rows = [row.rstrip("\n") for row in open("/private/tmp/br196-public-surface-v1.txt")]

def classify(row):
    _, path, _, owner, name, *_ = row.split("|", 5)
    if path.endswith("br196_test_delivery.rs"):
        return "BR196_TEST_PREIMAGE"
    if path.endswith("br196_transport.rs"):
        return "BR196_TRANSPORT_PREIMAGE"
    if path.endswith("presentation_registry.rs"):
        return "BR196_REGISTRY_PREIMAGE"
    if path.endswith("/main.rs"):
        return "MAIN_FRESHNESS_REEXPORT"
    if path.endswith("news_aggregator_init.rs"):
        return "NEWS_PIPELINE_API"
    if path.endswith("/notify.rs"):
        if name == "ReviewProviderTopN":
            return "BR192_R09_PUSHKIND"
        if name in {"PushLogError", "PinnedPushLogWriter", "for_namespace", "for_test_anchor", "deliver_authoritative_blocking", "eager_bind_push_log_capability", "NamespaceIsolation", "NamespaceOverrideRejected", "Persistence"}:
            return "BR192_PUSH_LOG_API"
        if name == "record_news_recommendation":
            return "NEWS_RECOMMENDATION_RETIREMENT"
        return "NOTIFY_PRESENTATION_API"
    if path.endswith("/push_templates.rs"):
        lowered = (owner + " " + name).lower()
        if "t0" in lowered:
            return "T0_API"
        if "testscope" in lowered or "testtemplate" in lowered or name in {"dispatch_all_for_test", "build_test_template_catalog"}:
            return "BR196_TEST_CATALOG_API"
        if "catalystreview" in lowered or name == "load_catalyst_review_snapshot_real":
            return "A10_API"
        if any(value in lowered for value in ("eventholding", "chainline", "validatedr08")) or name in {"build_event_calendar_macro_summary", "event_calendar_virtual_holdings", "render_event_calendar", "dispatch_r08_event_calendar_outcome", "validate_r08_public_source_binding_canonical_bytes"}:
            return "R08_API"
        if name.startswith("dispatch_r04") or name in {"render_review_lhb_gateway", "parse_r04_observed_at", "canonical_review_lhb_source_binding_for_test", "validate_review_lhb_source_binding_canonical_bytes"}:
            return "R04_API"
        if "paper" in lowered:
            return "PAPER_API"
        if "fund_inflow" in lowered:
            return "FUND_INFLOW_RETIREMENT"
        if "limitboards" in lowered or name == "render_limit_boards_shape":
            return "LIMIT_BOARDS_API"
        if name == "dispatch_post_session_review":
            return "REVIEW_ORCHESTRATOR_API"
        if name == "push_candidate_triggered":
            return "CANDIDATE_TRIGGER_API"
        if name == "load_news_to_idea_snapshot_real":
            return "NEWS_TO_IDEA_API"
        if name in {"dispatch", "counts_against_daily_budget", "push_holding_emergency", "push_holding_plan_recommendation", "record_cooldown"}:
            return "PRESENTATION_WRAPPER_API"
        return None
    if path.endswith("/v17_sources.rs"):
        return "V17_SOURCE_API"
    if path.endswith("/classifier.rs"):
        return "CLASSIFIER_API"
    if path.endswith("/candidate_state.rs"):
        return "OPPORTUNITY_CANDIDATE_API"
    if path.endswith("/opportunity/mod.rs"):
        return "OPPORTUNITY_MODULE_API"
    return None

counts, missing = Counter(), []
for row in rows:
    category = classify(row)
    if category is None:
        missing.append(row)
    else:
        counts[category] += 1
for category, count in sorted(counts.items()):
    print(f"{category}={count}")
print(f"classified={sum(counts.values())} unclassified={len(missing)}")
for row in missing:
    print("UNCLASSIFIED|" + row)
PY
```

```text
A10_API=17
BR192_PUSH_LOG_API=9
BR192_R09_PUSHKIND=1
BR196_REGISTRY_PREIMAGE=9
BR196_TEST_CATALOG_API=9
BR196_TEST_PREIMAGE=58
BR196_TRANSPORT_PREIMAGE=15
CANDIDATE_TRIGGER_API=2
CLASSIFIER_API=2
FUND_INFLOW_RETIREMENT=4
LIMIT_BOARDS_API=5
MAIN_FRESHNESS_REEXPORT=2
NEWS_PIPELINE_API=11
NEWS_RECOMMENDATION_RETIREMENT=1
NEWS_TO_IDEA_API=2
NOTIFY_PRESENTATION_API=12
OPPORTUNITY_CANDIDATE_API=22
OPPORTUNITY_MODULE_API=13
PAPER_API=8
PRESENTATION_WRAPPER_API=6
R04_API=7
R08_API=24
REVIEW_ORCHESTRATOR_API=2
T0_API=25
V17_SOURCE_API=12
classified=278 unclassified=0
```

The grouped disposition is exhaustive: BR-196 test/transport/registry and
test-catalog rows are the reviewed current preimage; notify presentation,
R-08, T-0, news pipeline, and news-recommendation retirement rows are explicit
Gate-B migration inputs; R-04, R-09, BR-192 push-log, A-10, paper, limit-board,
candidate-trigger, review-orchestrator, news-to-idea, v17 source, classifier,
and fund-inflow rows preserve the current owning contracts unless a row below
says otherwise; opportunity and freshness re-export rows remain outside
BR-196 ownership and are protected by workspace compilation. This includes
the removed `notify::record_news_recommendation`, all `T0Kind`/`T0Style`
variants and `EventHolding`/`T0AdviceParams` fields, the R-08 wrappers and
bindings, and the replacement news init/tick APIs.

Seven manifest paths are byte-relevant authorities but have no HEAD/current
surface delta: `health.rs`, `l6_sink.rs`, `webhook_alert.rs`,
`event/history.rs`, `event/replay.rs`, `monitor/alert.rs`, and
`opportunity/scheduler.rs`. X-02 replay and X-03 health are therefore planned
additions, not current delta rows; Gate B must add them to the post-Gate-B
manifest before claiming their typed gateways exist.

### 9.2 Per-symbol compatibility disposition

The reviewed current preimage, not HEAD, is the Gate B rollback baseline.
Therefore an existing HEAD→current delta is never silently reversed by a
BR-196 revert. “Preserve current” below means the scoped Gate B revert restores
the exact §1.2 current signature/visibility; planned additions are removed by
that revert.

| HEAD/current or planned API delta | Adopt/exclude and compatibility/migration disposition | Rollback disposition |
| --- | --- | --- |
| `notify::push_governor`: `pub` → private `#[cfg(test)]` | Adopt restriction; reject production shim; current callers remain test-only. | Preserve current private test-only form. |
| `notify::push_governor_v3`: `pub` → `pub(super)` | Adopt restriction; Gate B migrates the I-01 board bypass; no registry-external presentation caller may remain. | Preserve current visibility and restore reviewed raw I-01 call only as part of a full Gate-B revert. |
| `notify::push_source_fact_v3`: `pub` → private | Adopt core helper; public callers use `push_presented_source_fact_v3`; no shim. | Preserve current private helper. |
| removed `notify::record_news_recommendation` | Adopt retirement. It was a production-named write API; Gate B must not restore it as a logging-only compatibility shim under Rules 2.7/2.8. News recommendation persistence remains explicit through its current owning path. | Preserve removal. |
| removed `notify::push_governor_v3_with_sub_kind` | Adopt breaking removal; counted/sub-kind callers use typed counted gateways. | Preserve removal. |
| 25 `T0_API` rows: removed `T0Kind`/`T0Style` enums and variants, removed `EventHolding`/`T0AdviceParams` public structs/fields, and their current replacements | Adopt the current T-0 model as one coherent compatibility unit. Gate B may migrate the presentation terminal but must not reconstruct obsolete fields, variants, or wrappers; typed inputs must leave missing market facts explicit. | Preserve the complete current T-0 type/field/variant surface, not only the function names. |
| 11 `NEWS_PIPELINE_API` rows: removed `init_news_aggregator`/`tick_news_aggregator_batch` and current global registration/raw acquisition/receipted projection APIs | Preserve the current ingress split. Definitions and test callers cannot activate N-01/N-02; Gate B must use the current raw-acquisition → receipt → projection boundary and must not add an old init/tick shim. | Preserve the current news pipeline APIs and Disabled lifecycle evidence. |
| removed `push_templates::push_holding_plan_recommendation` | Adopt breaking removal; T-03 remains Disabled with no compatibility token. | Preserve removal. |
| removed `push_templates::push_holding_emergency` | Adopt breaking removal; T-04 remains Disabled with no compatibility token. | Preserve removal. |
| removed `push_templates::push_t0_advice` | Adopt migration; `main.rs:7415,7452` owns the typed counted replacement. | Preserve current typed path; BR-196 revert does not recreate wrapper. |
| removed `push_templates::push_t0_forbid` | Adopt breaking removal; T-06 remains Disabled. | Preserve removal. |
| removed `push_templates::push_paper_trade` | Adopt migration; `dispatch_paper_trade_daily` renders at `push_templates.rs:5498` and enters the counted gateway at `push_templates.rs:5568`. | Preserve current batch path. |
| removed `push_templates::dispatch_paper_trade_one` | Adopt migration to the batch dispatcher; reject one-off shim. | Preserve removal. |
| `dispatch_paper_trade_daily(hhmm)` → `dispatch_paper_trade_daily()` | Adopt breaking signature; `main.rs:6776` is the current production caller and workspace compile is the callsite proof. | Preserve zero-argument current signature. |
| removed `push_templates::dispatch_all_for_test` | Adopt migration; `main.rs:4655-4716` and `br196_test_delivery.rs` own closed acceptance. | Preserve removal; Gate-B revert restores the reviewed current acceptance path. |
| removed `push_templates::dispatch_r04_lhb_real` | Adopt migration; `main.rs:4234` → `push_templates.rs:7220,10052,10228` owns the typed outcome. | Preserve removal. |
| removed `dispatch_fund_inflow_top_daily` | Adopt breaking removal; M-01 remains SpecOnly. | Preserve removal. |
| removed `dispatch_fund_inflow_top_periodic` | Adopt breaking removal; M-01 remains SpecOnly. | Preserve removal. |
| removed `dispatch_post_close_fund_inflow_buy` | Adopt breaking removal; M-01 remains SpecOnly. | Preserve removal. |
| removed `v17_sources::push_policy_results` | Adopt breaking removal; S-02 remains Disabled because `classifier.rs` has no production caller; no compatibility wrapper. | Preserve removal; BR-196 revert must not resurrect the policy path. |
| removed `opportunity::candidate_state::push_disabled` | Exclude from BR-196 ownership: non-monitor opportunity-domain API. Risk is downstream compile break, checked by workspace compile; compatibility belongs to its owning change. | Preserve current absence; rollback only with the opportunity owner, never BR-196. |
| removed `opportunity::push_tier` | Exclude from BR-196 ownership: non-monitor opportunity-domain API with the same compile-time compatibility risk. | Preserve current absence; rollback only with the opportunity owner. |
| `dispatch_post_session_review(date,now,banner,due)->Outcome` → `(ReviewRunContext,due)->Result<Outcome,String>` | Adopt breaking fail-closed migration; sole production caller is `main.rs:4234`; no old-signature shim because it would bypass context/error ownership. | Preserve current signature; Gate-B revert must leave `main.rs:4234` coherent. |
| `dispatch_r04_lhb_outcome(date,now,banner)` → `(date,now)` | Adopt breaking migration; sole production owner is `push_templates.rs:7220`; banner is no longer caller authority. | Preserve current signature and caller. |
| `dispatch_r08_event_calendar_outcome(date,banner)` → `(date)` | Adopt breaking migration; current callers are `push_templates.rs:7229,8688`; no ignored-banner shim. | Preserve current signature and both callers. |
| 24 `R08_API` rows, including dispatcher/renderer/binding fields and added public `notify::push_r08_presented_source_only_with_binding` | Adopt as a single source-only chain. The public wrapper must validate the R-08 `EventCalendar` token and delegate to the private `notify::push_r08_source_only_with_binding`; the private helper must remain non-public and source-only. The obsolete checker expectation for a public helper is not compatibility authority. | Preserve the current public-wrapper/private-helper split. Never restore the old public helper merely to satisfy a stale checker. |
| `push_candidate_triggered: bool` → `Result<bool,String>` | Adopt explicit-failure migration; `push_templates.rs:5703-5708` handles both branches; no bool-flattening shim. | Preserve `Result` signature and explicit error handling. |
| added `br196_test_delivery::ManifestRow::push_kind` as `pub(super)` | Exclude from production presentation API; retain as scoped metadata accessor used by governance smoke. | Preserve reviewed current accessor; remove only with its owner. |
| added `notify::push_br196_governance_smoke_v3` as `pub(super)` | Adopt test-only exact-six gateway; it cannot activate a lifecycle. | Preserve reviewed current test seam. |
| added `notify::push_presented_v3` | Adopt generic uncounted typed presentation gateway. | Preserve reviewed current gateway. |
| added `notify::push_counted_with_binding` | Adopt T-05/T-10 counted typed gateway. | Preserve reviewed current gateway. |
| added `notify::push_counted_source_only_with_binding` | Adopt R-04 source-only typed gateway. | Preserve reviewed current gateway. |
| added `notify::push_r08_presented_source_only_with_binding` | Adopt R-08 public-source typed gateway. | Preserve reviewed current gateway. |
| added `notify::push_presented_source_fact_v3` | Adopt S-01/S-03/S-04/S-05 typed source-fact gateway. | Preserve reviewed current gateway. |
| added `notify::push_source_batch_v3` | Adopt A-10 typed source-batch gateway. | Preserve reviewed current gateway. |
| planned additive `notify::push_presented_replay_v3` | Add only for X-02; typed replay evidence preserves text, fresh ID, `replay_of`, and per-envelope result. | Scoped Gate-B revert removes API and restores the reviewed raw replay terminal; re-enabling raw terminal for release requires a new red-line review. |
| planned additive `notify::push_presented_health_webhook_v3` | Add only for X-03; typed startup-health evidence preserves `health_check_fail` and Disabled/Delivered/Error semantics. | Scoped Gate-B revert removes API and restores the reviewed direct webhook terminal; release remains blocked until re-reviewed. |

### 9.3 Path/module disposition

| Existing path | Disposition | Reason |
| --- | --- | --- |
| `src/bin/monitor/presentation_registry.rs` | retain only as routing metadata; prune 25 and add 3 | metadata cannot prove reachability; target count is 28 |
| `src/bin/monitor/br196_test_delivery.rs` manifest | replace lifecycle/count content | current projection and fixtures omit replay and inactive startup authority |
| `src/bin/monitor/br196_transport.rs` | retain typed non-production target/receipt authority | transport evidence cannot prove production reachability |
| `config/br196_non_production_feishu_targets.toml` | retain values; add BR-196 metadata citation only | closes Rule 2.10 path citation without changing target/threshold behavior; semantic hash is frozen in §8 |
| 25 Disabled descriptors in §2.2 | remove production tokens/fixtures; retain stable IDs as Disabled rows | no independently proved production chain |
| `P-05-virtual-watch-pilot-snapshot` | retain stable shape as Disabled without token/fixture | its sole upstream is explicitly empty |
| `I-01-intraday-market-board-flow` | route through typed gateway without renaming family/PushKind | independently reachable production bypass |
| `src/event/replay.rs` selector/assembler | adopt unchanged bytes/identity semantics | owns `push.source` selection, marker, fresh ID, `replay_of`, ordering and rate spacing |
| `docs/business_rules.md` BR-043 replay contract | adopt, then specialize only the presentation terminal under BR-196 | BR-043 remains authority for explicit force, `push.source` selection, fresh/replay identity, ordering, and rate spacing; BR-196 changes only the raw terminal to typed X-02 evidence |
| `src/bin/monitor/main.rs` | migrate X-02/X-03/I-01 terminals and retain all reviewed roots | current `main.rs:2533,2355` reaches raw replay, `main.rs:3717-3720` reaches health webhook, and `main.rs:7510` reaches raw I-01; exact API callsite migrations are in §9.2 |
| `src/bin/monitor/health.rs` | retain startup-health evidence owner | X-03 consumes its typed status; it must not invent missing health facts |
| `src/bin/monitor/webhook_alert.rs` | migrate direct HTTP terminal to X-03 typed presentation | preserve URL-disabled and transport/HTTP error semantics; direct `.post(...).send()` is the current bypass |
| `src/bin/monitor/l6_sink.rs` | retain typed-governor L6 terminal | its `notify::push_wechat` delegation is downstream of typed governance, not a fourth registry-external presentation |
| `src/bin/monitor/notify.rs` APIs | migrate per §9.2 | typed gateways replace raw production presentation entry points |
| `src/bin/monitor/push_templates.rs` wrappers | migrate/remove per §9.2 | preserve active typed owners and do not restore Disabled/SpecOnly shims |
| `src/bin/monitor/v17_sources.rs` | adopt typed source presentation and `push_policy_results` removal | exact renderer/source-fact gateway are Active evidence; S-02 remains Disabled |
| `src/news/aggregator/classifier.rs` | retain S-02 Disabled evidence | `classify_policy` has test callers only and cannot activate S-02 |
| `src/bin/monitor/news_aggregator_init.rs` | retain N-01/N-02 Disabled evidence | definitions/tests are not production callers |
| `src/event/history.rs`, `src/monitor/alert.rs`, `src/opportunity/scheduler.rs` | exclude from BR-196 migration; retain in API preimage | repository-wide scanner finds their public push/dispatch APIs unchanged |
| `src/opportunity/candidate_state.rs`, `src/opportunity/mod.rs` | exclude removed APIs from BR-196 ownership | `push_disabled`/`push_tier` compatibility and rollback belong to opportunity; workspace compile covers downstream risk |
| 14 rows in §2.3 | retain as SpecOnly with exact banners | catalog/history only; no production presentation contract |
| six governance-smoke cases | retain behind dedicated test-only governance capability | governance evidence only |
| existing TEST_CODE MagicLaw protocol | reuse only behind typed non-production target authority | default target selection is not authority |

### 9.4 R-08 checker and post-Gate-B manifest contract

The current BR-194 checker is intentionally recorded RED against the reviewed
preimage because it still expects the retired public helper:

```bash
set +e
bash tools/compliance/lib/check_br194_review_dependency.sh
rc=$?
printf 'check_br194_review_dependency_rc=%s\n' "$rc"
exit 0
```

```text
Traceback (most recent call last):
  File "<stdin>", line 93, in <module>
  File "<stdin>", line 33, in function_body
AssertionError: missing function marker pub async fn push_r08_source_only_with_binding(
check_br194_review_dependency_rc=1
```

Gate B must update `tools/compliance/lib/check_br194_review_dependency.sh`
instead of weakening the production API. Its positive assertions must follow
`push_templates::dispatch_r08_event_calendar_outcome` → public
`notify::push_r08_presented_source_only_with_binding` → private
`notify::push_r08_source_only_with_binding`, and prove the wrapper's
`EventCalendar` token-kind check/delegation plus the private helper's
source-only gate. A mutation restoring a public source-only helper, bypassing
the presented wrapper, accepting another PushKind, or delegating to the generic
counted gate must fail.

If Gate B changes that checker, the post-Gate-B implementation-attestation
manifest is not limited to the eighteen Rust authorities: it must also contain
the changed checker, its `tools/compliance/check.sh` registration, the R-08
caller/wrapper/helper, and the corresponding named tests. The BR-196 Rule-2.10
row already cites the checker. Acceptance requires the checker and the full
compliance runner to return zero; this documented current RED is not a waiver.

Rollback is a scoped Git revert of the eventual BR-196 implementation and
these BR-196 documents. It restores the reviewed §1.2 pre-Gate-B source
preimage as one coherent unit; it does not rewind unrelated HEAD→current API
deltas and does not hand-create shims.

Keep live acceptance disabled first. Never delete or rewrite production/test
delivery audits, replay audits, push logs, account data, or market evidence.
Rollback cannot reinterpret a dry-run, test, or fake receipt as production
delivery. Although a full scoped implementation revert mechanically restores
the reviewed raw replay/health/I-01 terminals, release remains blocked until a
fresh Gate A red-line review accepts those restored bypasses.
