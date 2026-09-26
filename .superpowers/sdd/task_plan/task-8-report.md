# Task 8 — BR-171 持久候选与人工裁定

基线 `19c95e2d`，2026-09-27。唯一 writer；使用 executing-plans、TDD、codebase-design、karpathy-guidelines 和 verification-before-completion：既有计划约束行为，小 Interface 收口身份/审计/事务，逐片真实 RED/GREEN。未访问生产 DB 或外部服务；不部署。

状态：Task8 本地源码实现完成；首轮独立 review 的 I01/I02 已按 RED→GREEN 修复，待修复 diff 复审；不等同于生产启用或整个项目完成。

## 开工复核与裁定

- `historical_bars.rs:296` 的发现目前固定 unavailable；普通 daily 两入口直接从 bridge 结果构造 admitted，没有读本地确认。CLI review/confirm 每次重新发现，不能离线重开。
- `daily_change_confirmation.rs:1136` 外部 append 当前自开 IMMEDIATE；需抽出事务内版本，让新 decision 与 v1/v2 alias 原子提交，原 v1/v2 SQL/内容/hash 规则不变。
- outcome `project_magic_tdx_batch` 已拥有 request、provider ordered content、available evidence 与 transport attempt preimages；只有该已验证 Adapter 可制造 opaque discovery。普通 HistoricalBars 没有 raw-discovery 合同，返回版本化 discovery unavailable，不从错误文本或 CLI JSON 制造事实。
- 新 `DailyChangeReviewV1` append-only event schema 集中保存 Candidate/Observation/Decision/renewal revision。stable fact、immutable first snapshot、later observation、revision 独立。review token 绑定候选/事实/原始 snapshot/持久 expires_at，不绑定每次重新采集的 batch。
- review policy v1：发现后7个自然日（上海无DST，七日时间间隔），精确到 expires_at 不可再作新决定；重启不续期，renewal 新 revision。相同终态决定重试先恢复原 receipt；不同决定/operator/reason 冲突。A→B→A不复活旧token。拒绝不写正向确认；已有legacy确认不得用普通Reject撤销。
- Interface：qualified Adapter 的 `discover_on_conn`；DB-only `review_on_conn` / `decide_on_conn` / `renew_on_conn`；统一 `admit_on_conn`。CLI不持有discover构造权限。库内派生状态和hash-chain统一验证，错误typed返回。
- Parent已批准 **GlobalSchemaCatalogV3**：只接受合法旧三基态 + 完整PaperLedgerV1 + 完整DailyChangeReviewV1，精确对象/索引/trigger与无额外对象；V1/V2/PaperLedger冻结不改。schema仅显式maintenance owner安装；普通startup和consumer不懒建；生产迁移、重资格、激活留Task10。

## TDD 计划

1. discover→文件SQLite关闭重开→review/confirm，同事务旧alias；Pending不授权。
2. observation轮换、事实/规则revision、7日边界/renewal、拒绝与精确准入。
3. 双连接并发、事务故障、幂等与篡改；中央CatalogV3/旧bytes保持。
4. outcome qualified raw接线与普通finalizer fail-closed；CLI可注入runner离线review/confirm/reject与输出失败。
5. 相关confirmation/historical/outcome/CLI限定回归、diff-check、自审、单独提交。

## Slice 1 — persisted discover/reopen/Confirm

`cargo test --lib task8_discover_reopen_review_confirm_uses_original_snapshot -- --nocapture --test-threads=1`：session52895 **RED**（编译3m15s，测试0.03s），qualified discover仍Unavailable。实现append-only事件链、原snapshot/token/expiry恢复、确认事务内旧alias后，同命令session12862 **GREEN 1/1**（编译3m11s，测试0.05s）。旧append公开入口保持自开事务，新增只供review owner调用的in-transaction helper，旧SQL/hash没有改写。

追加裁定：纯legacy exact事实且从未进入v3 scope继续准入，discover返回AlreadyConfirmed而不制造可Reject的Pending。已有v3 A-confirm→B→A则latest revision优先于旧alias，Pending/Rejected均不准入；新revision显式Confirm可引用原exact alias，不改旧operator/reason/bytes，新裁定信息只写新decision。正在用Slice2行为测试验证，尚未宣称完成。

## Slice 2 — observations/revision/TTL/admission

`cargo test --lib task8_ -- --nocapture --test-threads=1`：session85720 **RED 1/5通过**（编译3m16s/测试0.15s）：observation为空、B仍revision1、renewal及admission尚Unavailable。实现后同命令session63598 **GREEN 5/5**（编译3m17s/测试0.28s）。纯batch轮换token/首次snapshot/expiry不变，新observation按acquisition批次幂等；事实/规则变化或显式到期renewal生成新revision，A→B→A旧token无效；latest v3 Pending/Rejected压住旧alias，而新Confirm可引用其不可变receipt。

当前规则合同生产仅 `br171-close-change-v1`；规则变更测试只接受 `cfg(test)` 的 `TEST_CODE_rule_revision2`，不在生产擅自发布第二个规则。所有交易/资格合同尚未自动扩展。Slice3正在验证并发、回滚、namespace/catalog与链尾篡改，未部署。

## Slice 3 — transaction/catalog safety

`cargo test --lib task8_ -- --nocapture --test-threads=1` session12386 **RED 7/10通过**（编译3m16s/测试0.78s）：缺扩展旧库读取missing table；删除immutable trigger仍可读；完整V3被判future generation。两真实连接Confirm/Reject单胜、幂等/冲突，以及alias链写入失败对v1/v2/decision整体回滚已通过。实现同事务main/temp精确namespace、generation与sqlite_sequence高水位核对（恢复trigger后的尾删仍拒绝），缺扩展仅允许旧exact确认只读；V3精确基态+PaperLedgerV1+ReviewV1独立hash域与重新资格诊断。旧DDL/fixture/hash保持不变。

实现后 `cargo test --lib -- task8_ paper_ledger_catalog_v2 daily_change_confirmation::tests --nocapture --test-threads=1` session13638 **GREEN 19/19**（编译3m15s/测试4.30s）。

## Slice 4 — shared Gateway/CLI boundary

`cargo test --lib -- task8_cli_runner task8_gateway_ --nocapture --test-threads=1`：session69378首次仅夹具编译错误（private schema helper / ProviderId无as_str），不算行为RED；修正后session61585 **RED 0/3**（编译3m50s，测试0.09s）：公共发现仍旧错误码、真实收口零候选、离线runner未实现。

实现：outcome结构/transport资格后生成不可反序列化的raw authority，冻结请求/response/provider-ordered/window/transport全量preimages；历史finalizer核对authority与选中batch/证券，留存完整已准入lifecycle context（不是另行声称原RPC wire已保留），持久候选后统一读取准入。普通同步/异步daily入口也在返回Admitted之前读取同一准入，无raw合同零候选且返回`daily_change_discovery_unavailable_v1`。CLI默认DB-only Review，显式Confirm/Reject/Renew；SQLite URI mode=ro/rw不创建文件、不运行DatabaseManager启动迁移，原snapshot输出/flush先于决定。

首次GREEN编译session26836发现RequestEvidenceColumns无Serialize，改为明确冻结其现有三个字段，未改旧codec。`cargo test --lib -- task8_ paper_ledger_catalog_v2 --nocapture --test-threads=1` session15106 **15/16通过**（编译4m05s/测试7.63s）：三个接口用例GREEN；自审追加的同一provider batch被不同outcome请求再次观察测试取得真实RED（Conflict）。现在observation identity绑定完整qualified snapshot；stable fact/token/原始snapshot不变，重复同一完整观察幂等。

实际接线：`OutcomeDailyBars::acquire → project_magic_tdx_batch → opaque raw-review evidence → HistoricalBars共同BR171 finalizer → DailyChangeReview`。普通日线没有raw discovery authority，但用同一DB准入接口读取已有精确确认；未确认返回版本化discovery_unavailable且零候选。CLI以candidate_id读取原snapshot，只用DB完成review/confirm/reject/renewal，不重新发RPC、不接收JSON作为证据、不懒建schema；审阅输出flush失败先于决定写入。

## 最终限定验证

`cargo test --lib -- task8_ database::daily_change_confirmation::tests database::global_schema_catalog_v1::tests paper_ledger_catalog_v2_old_receipt data_gateway::historical_bars::tests monitor::data_quality::tests::br092_ --nocapture --test-threads=1`：session81480 **GREEN 46/46**，编译4m34s、测试22.15s。覆盖新观察修复、真实SQLite并发/回滚/重开、CLI共享runner、Gateway真实收口、旧确认链、全部全局catalog黄金/三基态/精确对象、V2/V3旧receipt不可授权、相关BR092。

`cargo test --bin confirm_daily_change -- --nocapture`：session24996 **GREEN 3/3**，编译2m36s、测试0.01s。该目标同时成功编译非test library；不额外重复check/build/clippy。`git diff --check`通过。格式化仅涉及任务文件/抽取函数，不改旧DDL字节。

## 改动文件与复核路径

| 文件 | 关键职责/证据 |
| --- | --- |
| `src/database/daily_change_review.rs:91` | 共享DB-only action runner，scope断言、预览flush、决定后输出失败恢复。 |
| `src/database/daily_change_review.rs:314`、`:326`、`:535` | 观察身份、全链+高水位校验、qualified discover与新revision。 |
| `src/database/daily_change_review.rs:617`、`:697`、`:741` | Confirm/Reject事务、renewal、latest-v3优先的统一准入。 |
| `src/database/daily_change_review_schema_v1.rs:2`、`:45` | 中央DDL/immutable triggers与同事务main/temp精确namespace/generation；普通启动不安装。 |
| `src/database/daily_change_confirmation.rs:1147`、`:1185` | 旧alias事务内append与精确只读receipt引用，原链内容不改。 |
| `src/database/global_schema_catalog_v1.rs:1285`、`:1397`、`:1666` | V3独立参考、域分离hash、三合法基态+完整扩展精确比对；V1/V2冻结材料不变。 |
| `src/database/global_schema_v1.rs:1627` | 老审计receipt不能授权V3，明确需重新资格。 |
| `src/data_gateway/historical_bars.rs:284`、`:317`、`:672`、`:810` | 两普通入口/共同finalizer/outcome接线，缺raw合同不造候选。 |
| `src/data_gateway/outcome_daily_bars.rs:420`、`:742` | private构造的raw authority，仅在真实project资格成功后建立，绑定批次与证券，冻结全部现有outcome preimages。 |
| `src/data_gateway/grpc_source.rs` | 7行Gateway future bridge适配，复用原有有总时限的同步runtime，无新连接owner。 |
| `src/bin/confirm_daily_change.rs:116` | 明确已有DB的URI ro/rw打开，不走启动迁移，不允许JSON导入事实。 |
| `src/database/mod.rs` | 仅注册两个module，未加入自动schema install。 |
| `src/database/daily_change_review_tests.rs`、`src/data_gateway/historical_bars_review_tests.rs` | tempfile DB/两连接/共享runner/真实finalizer行为回归；300005数值仅TEST_CODE synthetic fixture。 |

## 自审、闭包与明确未部署项

- `rg has_exact_daily_change_confirmation`：Gateway/CLI不再直接读legacy alias；剩余仅review内部兼容分支、legacy模块自身API/测试。旧API是原始确认审计/兼容读，不作为v3经济准入。原append API保留兼容，新的唯一operator命令只走candidate decision；新Confirm引用原alias且不改operator/reason/hash。
- opaque discovery构造闭包：生产唯一在historical finalizer收到`OutcomeReviewEvidence`且核对完整batch/证券后；raw authority生产唯一在outcome真实`project_magic_tdx_batch`成功之后构造。TEST_CODE直接构造全由`cfg(test)`隔离，无业务env可开启；CLI没有构造入口。
- 当前仅支持review policy v1/rule v1；未来规则必须显式发布。未知版本/部分schema/影子对象/破坏trigger/链尾缺失拒绝，不回退legacy。纯legacy没有v3 scope时保持exact兼容；v3新revision Pending/Rejected不会借旧alias复活。
- **Task10**：生产CatalogV3 migration、backup/lease/重新资格、部署/激活、真实候选审核与任何实际股票Confirm均未执行。本任务不替用户裁定300005或其它股票。
- 普通HistoricalBars raw-discovery合同仍未交付：固定返回版本化unavailable，不能声称普通CLI在线发现已可用。outcome只在真实due运行且取得合格raw evidence时发现；本任务没有伪造due来主动发现。
- 测试走真实隔离SQLite、共享CLI runner及Gateway收口；outcome注入资格fixture的附加amount/source字段明确为synthetic，不声称已验收真实服务或完整生产wire。Lifecycle snapshot是现有已准入context（保留Available/Unavailable/VerifiedEmpty及证据），非新增raw lifecycle RPC合同；Task9继续处理生命周期资格。
- 全链验证/完整snapshot留存尚未做规模压力测试；未跑无关全量、release、网络或生产测试。无后台monitor变更。

## 首轮独立 review 修复（2026-09-27）

首轮报告 `.superpowers/sdd/task_plan/task-8-review.md` 为 REQUEST_CHANGES，确认两个 Important：合法 CatalogV3 被 paper 经济消费者的 `generation == 2` 硬门拒绝；listing metadata unavailable 仍可发现并 Confirm 新候选。

- I01 RED：`cargo test --lib task8_catalog_v3_preserves_paper_projection_and_adjudication -- --nocapture --test-threads=1`，1/1 失败，V3 返回 `IntegrityFailure("...unknown catalog generation")`。修复后同命令 1/1 GREEN。PaperLedger 只接受 generation 2，或 generation 3 + 精确 PaperLedgerV1 + 精确 DailyChangeReviewV1；未知 generation、缺失 review namespace、缺 trigger 仍拒绝。新增 V2 seed/fill → V3 投影一致 → V3 quarantine 裁定行为测试，以及 V3 缺失/篡改 review namespace 的拒绝测试。
- I02 RED：`cargo test --lib task8_gateway_unavailable_listing_metadata_cannot_create_candidate -- --nocapture --test-threads=1`，旧行为返回 `manual_confirmation_required` 并产生候选。修复后相同测试 GREEN：BR-171 的 lifecycle confirmation evidence 必须有 admitted listing date；Unavailable 保留为 typed `listing_date_context_unavailable`，在 discover 前终止且事件表零候选。旧 nullable legacy 字节未改写。
- 修复后 `cargo test --lib task8_ -- --nocapture --test-threads=1`：17/17 GREEN。
- 修复后原限定回归加新增用例：`cargo test --lib -- task8_ database::daily_change_confirmation::tests database::global_schema_catalog_v1::tests paper_ledger_catalog_v2_old_receipt data_gateway::historical_bars::tests monitor::data_quality::tests::br092_ --nocapture --test-threads=1`：49/49 GREEN。
- `cargo test --bin confirm_daily_change -- --nocapture`：3/3 GREEN。`git diff --check` 通过。

覆盖边界保持诚实：当前 raw authority 行为测试由 `cfg(test)` synthetic fixture 穿过共享 finalizer/SQLite，并未穿过真实 `OutcomeDailyBarsGateway::acquire` 的远端 transport。生产代码的构造闭包静态位于 `project_magic_tdx_batch` 成功后，但真实服务、production CatalogV3 迁移/重新资格与 end-to-end 采集证据属于 Task10；在此之前不声称 raw Adapter 已经生产验收。
