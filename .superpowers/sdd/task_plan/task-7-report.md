# Task 7 — Gateway provider 归因全覆盖

基线：`d59148123c756813f0d424ead0337ee55133ed41`。状态：源码完成，限定验证通过，待独立 review。仅源码/隔离测试，不读取生产库、不部署、不回填历史。

## 实施合同

- 复用 `review::audit_routed_gateway_result`：成功取 batch evidence，失败取 typed error provider，缺失为 Custom；不把 Custom 写回原错误。
- 固定产品准入与审计归因分离。GlobalNews、TopN、Auction/Jin10/Fred 的合同拒绝继续生效，确有 selected provider 的拒绝归于该 provider。
- Benchmark 的服务端专属 receipt、客户端 transport/admission、OutcomeDailyBars 已准入 TDX 成功保留显式合同；无 schema/旧行变化。
- 测试通过真实 Gateway 公共方法或 macro settle 真实调用的收口 helper，使用 test-only 数据库；仅外部 Query 边界允许注入返回值，不替换审计/validator。

## TDD 日志

1. `cargo test --lib d15_attribution_ -- --nocapture --test-threads=1`：session67579/19221 分别是测试夹具的编译错误（GatewayError 未实现 PartialEq；ResearchReportsGateway 类型名错误），改为逐字段断言与实际 ResearchDataGateway；不计行为 RED。
2. 同命令 session95867：编译 3m15s，运行 0.21s，8 条失败。其中 6 条真正 RED：Auction 错源 Sina 被归为 HithinkFinance；Index Sina 错误被记 Tencent；macro/TopN 被记 Eastmoney；SecurityLifecycle 被记 Tdx；LocalBridge InvalidArgument 丢失 Sina。另两条是夹具修正（consensus EPS 显式 null，research 使用 TEST_CODE_ 证券命名空间），不记业务缺陷。
3. 同命令 session87043：编译 3m16s，运行 0.34s，原 8 条 GREEN。新增 TopN 准入用例真正 RED：Sina pair 最终拒绝，但已落 `available` 审计。修复为 pair 原子准入在先，两条 routed 审计在后。四个重复错误调用分支合并为同一个收口，不保留默认 Eastmoney 的不可达错误兜底。
4. 增强隔离验证：错误 fixture 由 proto ErrorDetail → tonic Status → GrpcError 解码；历史每条 receipt 经完整链验证，不只是比较 provider；所有查询注入仅 `cfg(test)`、共享测试 guard 退出即清除，生产无该 seam。
5. 首次组合命令把多个 filter 放到 Cargo 参数端，被 CLI 拒绝（`unexpected argument`）；改为 libtest `--` 后多个 filter，未运行测试，不计 RED。
6. 组合命令 session82145 **54/54 GREEN**（编译 3m17s，运行 0.94s）：9 个 d15 用例 + mapper + 三个固定产品 validator/真实本机 qualified mTLS fixture + TopN 原合同 + review/Benchmark 专用 receipt 与 append-failure 回归。

```sh
cargo test --lib -- d15_attribution_ data_gateway::grpc_source::tests::map_query_error_ data_gateway::current_auction_observations::tests:: data_gateway::economic_release_observations::tests:: data_gateway::economic_release_schedule::tests:: data_gateway::capital::br240_transport_neutral_tests:: data_gateway::review::tests:: --nocapture --test-threads=1
```

7. 自审发现 `security_lifecycle.rs:3` 明确 pinned Magic TDX。初始 mixed fixture 错将 Sina 成功视为可接受；将其纠正为 Tdx 成功 + Sina/None 失败，并另加错源成功必须被拒绝的独立 RED。此项保留原准入合同，不以 routed 改造扩大产品允许的源。
8. `cargo test --lib d15_attribution_lifecycle_fixed_tdx_contract_rejects_other_success -- --nocapture --test-threads=1`：session2489 真正 RED（编译3m15s/运行0.17s），Sina corporate-actions 被当 VerifiedEmpty。已新增固定 TDX 准入函数并置于两个 routed audit 之前。

### Owner 兼容裁定：当前 routed 与 frozen durable 分离

发现不能全局改变旧 `map_query_error`：`review::restore_gateway_error` 的旧 invalid_request 合同只允许 provider=None；`chain_post_close_board.rs:2051` 与 `chain_post_close_dragon_tiger.rs:1332` 从旧 raw status 重投影并逐字段/字节比较。强行补 provider 会改变旧冻结材料；放宽旧 decoder 也不能解决 raw→old bytes 不一致。

Owner 明确裁定：保留私有、显式命名的 legacy/durable mapper，仅供既有 Board/Dragon/Constituent 持久 codec/replay/专用 receipt 新写。当前普通 Query/独立 Gateway acquisition 使用独立 current routed mapper，wire provider 保真；不改变 frozen decoder/schema。测试使用固定 golden JSON/error 文本证明旧映射可恢复，并由同一新 wire InvalidArgument(Sina) 的普通公共 Gateway 测试证明新审计为 Sina。durable 专用收口仍是兼容 allowlist，不宣称属于普通 routed acquisition。

- `cargo test --lib d15_attribution_frozen_durable_invalid_request_bytes_still_restore -- --nocapture --test-threads=1`：session29514 真正 RED（编译3m15s/运行0.00s），新全局映射输出 Some(Sina)，旧 byte contract 要求 None。
- 实现双入口后 session3057 因子模块 `grpc_source_macro.rs` 的四处 `super::*` 旧函数名引用编译失败，不是行为 RED。同步收口：只供 durable LocalBridge codec 的 `map_macro_error` 保持 frozen；ordinary `news_outcome/economic_outcome/web_outcome` 使用 current。补四个 GlobalNews source + Economic 的真实 macro projection→audit 路径测试，并与 frozen macro mapper 对照。
- 最终 mapper 闭包：`query_op` 与独立 `dragon_tiger_async` 的实际 terminal projection 均 current；Board/Constituent/Dragon 的持久 session/resume/completion/restore 仍 frozen。`dragon_tiger_query_session` 是创建请求阶段（无 RPC/无 provider），普通入口与 durable owner 共享，真正查询终态已分别映射。OutcomeDailyBars 的 dedicated transport 仍显式保留原映射，Benchmark 的独立 query/receipt 完全未改。
- 没有修改 `restore_gateway_error`、持久 codec、DDL、schema generation、旧 fixture 文件或任何历史审计行。

## 入口闭包与保留清单

实际执行以下 5 组检索；行号为本次实现工作树（复核可重跑）：

```sh
rg -n 'audit_gateway_result\(' src/data_gateway
rg -n 'audit_gateway_result(::[^ (]+)?\(' src/data_gateway
rg -n 'audit_gateway_result|audit_routed_gateway_result|audit_blocking_join_failure' src/data_gateway
rg -n 'audit_macro_query|retain_security_identities_observation' src
rg -n 'unwrap_or\(ProviderId::|unwrap_or\(provider\.provider_id\(\)\)' src/data_gateway
```

- 普通/泛型显式调用只剩 `outcome_daily_bars.rs` 已准入 TDX 成功、`review.rs` routed 实现内部与无调用者的固定 worker helper。TopN 只剩两条 routed 泛型调用。
- `review.rs:487/508/525/542/567/584/681/700` 是 Benchmark：服务端/provider/library TDX receipt 与客户端 transport/admission Custom receipt 分离；不改变 ownership、Unknown、去重复审计。
- `review.rs:1426–1623` 是底层/helper 实现、receipt wrapper，不是遗漏产品入口；其 `unwrap_or` 全为 Custom。
- `review.rs:1628` 的 `audit_blocking_join_failure` 当前无生产调用者；增加注释明确仅非 routed 固定任务所有权，不能用于从请求推断 provider。
- `market_capabilities.rs:270/318` 的 SecurityIdentity receipt owner 已按成功 evidence/错误 provider/Custom 选择，保留独立 request hash/receipt；selector 旁标明 allowlist。
- `review.rs` 测试区和 `benchmark.rs:3486` 是隔离验证 fixture；不迁移为产品入口。
- `GlobalNews` 的四源公共 fetch 与 `search_service/macro_news/legacy.rs:607` 共用已迁移 macro helper；Economic 退役入口与 `legacy.rs:612` 共用已迁移 helper。helper 签名/request hash 未改变，未恢复 EconomicCalendar RPC。
- `grpc_source.rs` 剩余 `unwrap_or(Custom)` 为 readiness/diagnostic 独立审计收口，已按错误 provider/Custom 保真，不猜默认源；没有 `unwrap_or(固定provider)` 审计路径。

边界：OutcomeDailyBars 的 before-provider bridge/worker 错误仍按原专用 TDX receipt 语义处理（`outcome_daily_bars.rs:620/633`），不宣称整个项目所有错误都属于一般 routed 模型。未修改旧审计、数据库模块/schema、proto、客户端资格/连接 owner 或生产记录。

## 改动文件与自审

- 普通调用者迁移：`block_trade.rs`、`company.rs`、`consensus.rs`、`dragon_tiger.rs`、`event_calendar.rs`、`futures_delivery.rs`、`global_market.rs`、`index.rs`、`research.rs`、`sina_instrument_news.rs`。
- 产品准入/配对：`capital.rs`（双指标先准入、后双审计）、`security_lifecycle.rs`（两个 TDX 准入门独立于实际审计主体）。
- 固定产品错源归因：`current_auction_observations.rs`、`economic_release_observations.rs`、`economic_release_schedule.rs`；只改变错误 provider，不改变 envelope/record 准入条件。
- macro 收口：`global_news.rs`、`economic_calendar.rs`；`grpc_source_macro.rs` 明确 frozen durable 与 current ordinary projection。
- transport mapper/test-only Query 边界：`grpc_source.rs`。原 frozen mapper 仅重命名，原历史文本/字段/decoder 保留。
- 测试模块：`d15_attribution_tests.rs` 与 `mod.rs` 注册；覆盖 18 个异步公共入口、同步 index、两个生命周期子结果、TopN 双审计、实际 macro projection/settle、旧完整审计链。
- allowlist 注释：`review.rs`、`market_capabilities.rs`、`outcome_daily_bars.rs`。这些文件无行为更改。

自审关注：已识别并修正两个“改 helper 就会弱化合同”的点（TopN、生命周期），以及“直接改 mapper 会破坏旧字节”的持久回放边界。Query fixtures 只验证已选定 route 的下游处理，不模拟上游完整主备选择算法；三个新产品的真实本机 mTLS fixture 继续验收现有资格路径。历史链验证运行于测试临时库，不是生产历史重写。

未覆盖/运维边界：未跑全量测试、未构建 release、未访问外部行情服务/生产数据库，未进行 activation/monitor 重启；历史错误归因保留原行，后续生产验收属 Task10。原 durable 专用 receipt 保持既有投影合同；若需要升级其新写归因，要独立版本化，不能将本次普通 routed 修复当作旧 durable codec 升级。

## 最终验证

session43038：上述相同组合命令 **58/58 GREEN**，编译3m13s，测试1.02s；其中13个 `d15_attribution_` 用例。覆盖新审计 provider、None/未知→Custom、primary/fallback/VerifiedEmpty、TopN双请求身份与先准入、固定TDX生命周期双子结果、Auction/Jin10/Fred拒绝合同、四源macro与Economic真实收口、retired零RPC、frozen golden恢复及完整历史链。

既有三个固定产品的 qualified mTLS localhost fixture、Benchmark ownership/Unknown/服务端回执、BR-159 append-failure、原TopN约束和旧mapper用例均GREEN。测试输出68条既有warning，无新增未处理的编译错误。测试后只有一行 TopN 注释由“保留”改成准确的“尝试”，无行为变化。

最终 `git diff --check` 通过；五组 closure 均已运行并记录 allowlist；额外 mapper 全库检索确认旧泛名 `map_query_error(` 已无调用，普通 query_op/Dragon terminal/macro收口走 current，明确持久专用边界走 frozen。没有在相关测试通过后追加同目标 check/build/clippy。

## Fix round 1 — I01 GlobalNews 错源成功 envelope

基线 `19fa6002`，依据 `task-7-review.md` 的 Important I01。前文“全覆盖”结论被此 review 限定：统一审计 helper 不能纠正转换器提前写错的 provider；本轮补齐真实 Ok(query) 路径，不涉及 Task8。

### 真实 RED 与夹具纠正

1. `cargo test --lib d15_attribution_global_news_wrong_source_success_envelopes -- --nocapture --test-threads=1`，session19570：编译3m12s、测试0.17s，失败在测试错误地把 `audit_outcome` 写为 `invalid_evidence`；现有合同是 `partial`，原因码才是 `invalid_evidence`。仅修正测试预期，不算行为 RED。
2. 同命令 session84443：编译3m15s、测试0.19s，失败在同源 External 成功夹具的 Eastmoney `source_at` 带秒；该 provider 合同是分钟。改为 `2026-09-25 15:59`，并将同源成功独立为测试，未修改生产验证器。不算行为 RED。
3. `cargo test --lib -- d15_attribution_global_news_ d15_attribution_frozen_global_news_wrong_source --nocapture --test-threads=1`，session36004：编译3m17s、测试0.20s，**2 GREEN / 1 真实 RED**。Local/External × public Gateway/macro settle 四路，实际 Cailianpress 均返回 `Some(Eastmoney)` 且真实 SQLite acquisition audit 为 Eastmoney。未知/空 provider→None/Custom、同源 Available/VerifiedEmpty 及旧 durable 两格式 reopen golden 通过。

测试输入先以 Local/External protobuf encode/decode、各自真实 envelope parser 解析，再只在 RPC 边界注入；普通转换、公共 Gateway 或 `news_outcome`→macro audit、数据库 append 不 mock。错误仍须拒绝，保留原 capability/partial outcome/invalid_evidence reason/nonretryable/message/request hash，旧审计行/hash 前缀不变。

### 最小实现与兼容闭包

- `GrpcSource::current_global_news_query_result` 复用原准入/转换，只在错误结果将 provider 改为实际 `selected_provider` 的闭集解析值；缺失或未知为 None，审计收口为 Custom。没有放宽固定产品合同，也没有改变成功数据。
- 原函数体只重命名为 `legacy_durable_global_news_query_result`，`convert::external_global_news` 原实现原样保留。普通 `global_news_async` 与 `grpc_source_macro::news_outcome` 显式使用 current。
- `chain_post_close_macro_codec::gateway_for` 显式使用 frozen。额外调用闭包发现 full journal 的 `DataResult` 也通过 `NativeOutcome::project` 复用普通成功投影，因此其 capture/reopen 均经 `project_frozen_native` 隔离 GlobalNews **成功 envelope** 分支；现有 wire Err、Economic/Web 投影没有变化。
- 新 golden 从旧 raw response 构造 canonical DataResult 后 encode/decode 重开，分别走 original single-source `RawResult::project` 和 full-journal `DataResult::project`；断言两种 profile 的旧 requested-provider 错误 JSON、native/hash、整个 canonical result bytes 原样。该测试是持久 codec 重开，不声称进行了生产 DB 重开。
- `rg -n 'current_global_news_query_result|legacy_durable_global_news_query_result|global_news_query_result|external_global_news\(' src` 确认普通两个入口走 current；frozen 只由 current 的兼容转换基底、两类 durable owner 使用。另两个直接 external converter 调用是 `grpc_bundle_probe` 运维诊断和 mTLS测试，不是 routed acquisition audit。`NativeOutcome::project` 在普通 macro runner 保持 current；full journal 的成功新闻不再借用它。

改动仅五个源码文件：`d15_attribution_tests.rs`、`grpc_source.rs`、`grpc_source_macro.rs`、`chain_post_close_macro_codec.rs`、`chain_post_close_macro_native.rs`，以及本报告。无 converter body、旧 fixture、schema、DDL、decoder、历史数据更改；未访问生产库、外部服务或部署。

### Fix round 1 GREEN / 自审

```sh
cargo test --lib -- d15_attribution_ data_gateway::global_news::tests:: data_gateway::grpc_source::convert::tests::br238_external_global_news macro_codec::tests:: --nocapture --test-threads=1
git diff --check
```

session46831：**40/40 GREEN**，编译3m18s、测试1.23s。其中16个 D15 用例（含新增三项）、3个 External GlobalNews converter 用例和其余21个 frozen macro codec 回归；`data_gateway::global_news::tests::` 本身没有匹配用例，不将空过滤器算验证。原有68条warning，无新编译错误。编译期间后续改动仅为两处新增代码的换行格式及文档，不改变语义；最终 diff-check 通过。

关键证据：`grpc_source.rs:3576` current 归因、`:3598` frozen 旧函数；`grpc_source_macro.rs:178` ordinary macro；`chain_post_close_macro_codec.rs:1764` original durable；`chain_post_close_macro_native.rs:175/213/244` full journal capture/read 成功投影隔离。新增行为测试在 `d15_attribution_tests.rs:591/676`、`chain_post_close_macro_codec.rs:2598`。

自审：错误分类、reason、retryability、文本和请求 hash 不变；仅 current error provider 变化。所有真实 routed 路径仍拒绝错源，没有“错误先审计成成功”；未知/缺失不猜请求源。旧 durable 仍沿用其既有专用投影，包括旧 requested-provider 拒绝归因；升级它的新写语义需另立版本合同，本轮不伪称已改。未重写历史行、未更换旧 golden。没有跑全量、release、activation、monitor 或真实服务探测；Task10 实际上线验收仍未执行。
