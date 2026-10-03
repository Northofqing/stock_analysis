# stock_analysis 开发与上线交接

更新日期：2026-10-03（Asia/Shanghai）。此文档交接当前开发状态；生产事实另附明确观察时间。

## 1. 接手目标与授权

完成无券商研究 / paper 平台 M0–M7 的必要开发、验证、分批生产上线与观察；M8 根据实际需求或容量证据裁定实施 / 不实施。用户已授权按依赖继续开发、提交并推送当前功能分支，也已授权协调现有 Windows Codex 解决 gRPC / 数据合同并反馈。

最近要求依次是：先提交再继续开发；询问剩余工期；整理交接。本次交接不创建新聊天，不更换生产 owner，不把长期目标标成完成。

- 常规源码、定向验证和当前 feature commit/push 可以继续执行。
- 生产 activation、资金 B / allocation / seed / cutover、VM 监听 / Provider capture / 固定36真实RPC仍有各自精确门禁。开发授权不替代对应人审；已批准的 Wave0/Wave1 元组只适用于原精确候选，不适用于新制品。
- 既有生产保持无真实券商接入。T-14/T-15 等依赖真实券商的入口保持原裁定。
- 不修改或合并 master，不创建 PR，不重启生产，不自动重发或裁定 Uncertain。需要新发布时，先准备具体可审候选，再按对应门禁执行。

## 2. 工作目录与 Git

| 项目 | 精确身份 |
| --- | --- |
| 当前工作树 | `/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis` |
| 分支 | `codex/platform-roadmap-implementation-20261002` |
| remote / upstream | `stock_analysis` / `stock_analysis/codex/platform-roadmap-implementation-20261002` |
| remote 地址 | `github.com:Northofqing/stock_analysis.git` |
| 最后源码提交 | `b005457e94138147f11af4def4240a2aa9d3996d` |

交接编写前实际 `git status` clean、upstream +0/-0；实际 `git ls-remote` 与上述完整 OID 一致。此文档随后单独提交，接手时以实际 Git HEAD 为准；文档提交不改变已验证源码。没有 `origin` remote。历史提交数不能当完成任务数。

先读取 [AGENTS.md](AGENTS.md)、[CLAUDE.md](CLAUDE.md)、[整体路线图](docs/superpowers/plans/2026-09-28-platform-complete-roadmap.md) 和本交接，再检查实际工作树。

## 3. 最近已完成并推送的切片

| 提交 | 已实现 / 已验证 | 能力边界 |
| --- | --- | --- |
| `f8e583b44b9d30cc7373b4b8b15c614ff964ca28` | actual Catalog6 loan、固定资金预算 Paper 执行及相关 Global/Paper 开发门禁；普通 debug monitor 和隔离 dry-run 已通过 | Catalog6 生产资格、真实 source、资金批准和生产 cutover仍未交付 |
| `5075d9ba4a93b47e2f661cb857643dbf4f3d2209` | original 同事务与真实 Copied RO 备份逐表逐行类型/值/rowid/sequence相等；Rows19及旧backup24/prospective14定向通过 | 只有 source↔backup 证明，没有 target/apply/restore/exchange/recovery资格 |
| `a1c5e80214dd773303cc17441c0d756e2bc514ee` | 先提交 Rows 独立复核记录 | 文档提交 |
| `eb52eca565616057ea930a7b5897dad1b21c7a1d` | Rows 在 spec/preflight/intent/复制角色前限定 main UTF-8；完整UTF-16LE/BE回归；20项通过，原P2关闭 | 固定预算和原 guard不变 |
| `4199a11d2c4f87edcb605d16c5875fb5ef50ba93` | F2 Top50原始候选范围捕获；6项通过，独立复核无剩余P1/P2 | 来源准备组件，不是正式投资决策或持久 occurrence |
| `b005457e94138147f11af4def4240a2aa9d3996d` | future Catalog7固定不可变存储合同及防覆盖；5项通过，独立复核无剩余P1/P2 | 没有Catalog7 reference/borrower、durable owner或生产接线 |

前批完整开发验证及局部历史在 [验证记录](docs/ops/2026-10-02-platform-development-validation.md)。不要用最后3组测试代称全库验证或 M0–M7 达标。

### F2 来源合同（必须保留）

源码：[pushed_candidate_scope_v1.rs](src/decision/pushed_candidate_scope_v1.rs)。

- actual callback-local Catalog6 loan 内，同 SQLite snapshot 读取原 `pushed_stocks` **11列**；不使用普通启动 DDL额外的 `created_at`。
- 未消费，严格前一小时文本界限；owner一次UTC clock、显式 +08毫秒上下界；总排序 `push_time COLLATE BINARY DESC,id DESC`，Top50是新版本策略定义。
- 范围只代表符合谓词的Top50，不代表完整小时池、全市场或账户 universe。i64 row id、重复 raw code、REAL price bits及原消费字段均保留。
- 固定界限：一般文本16KiB、metric64KiB、单行128KiB、选集1MiB、canonical8MiB；main仅UTF-8，预检实际存储类型/长度。9个文本字段经Binary运输后checked UTF-8解码。
- `CandidateScopeCaptureId` 是内容身份，非 `InvestmentDecisionId`、Recorded occurrence或审批能力。保留原opaque `DatabaseConnectionAuthority`；同内容异库也不能换源。
- identity实际未资格；lifecycle/price_regime/suspension因identity不可用而未请求。risk inventory/evaluation、cost/liquidity、B/allocation/manual approval缺口完整保存。日历只记录实际immutable API的covered hash/open/closed或coverage unavailable。
- 原cutoff用于mandatory tail与独立committed reader重捕获；当前不写持久记录。生产Catalog6仍在checkout前拒绝。

### 固定 Catalog7 存储合同（不能重开已修漏洞）

源码：[candidate_scope_observation_schema_v1.rs](src/database/candidate_scope_observation_schema_v1.rs)。

- 一张 `candidate_scope_observations_v1` 表及no-update/no-delete/no-reinsert触发器。
- logical occurrence唯一键：固定policy + owner UTC30秒slot Unix毫秒 + 显式revision。cutoff完整秒/纳秒须在该slot；revision为1..u32::MAX。
- scope canonical为1..8MiB BLOB，SHA-256为32B BLOB。DDL不认证digest/content/source/资格。
- 具名正数 `observation_row_id INTEGER PRIMARY KEY` 是物理surrogate；logical composite另设UNIQUE。BEFORE INSERT同时保护物理/逻辑键，阻断默认 `recursive_triggers=OFF` 下不同逻辑键但相同隐藏rowid的REPLACE删除历史。
- 保留普通rowid表，当前Rows不能直接支持WITHOUT ROWID。后继closed writer应在原事务内显式分配正数物理键并检查i64溢出；它不能成为决定身份。
- actual C6回归已证明：operation实际执行新DDL及version7，然后由现有尾部拒绝、整笔回滚到exact6；新readonly borrower重验成功。
- Global支持最大代际、Catalog6 literals、普通startup与生产路径均未改。不能仅提高supported max或直接在普通连接安装新表。

## 4. 实际验证证据与复用规则

下面是已结束的真实执行，均EXIT0。接手无需例行重跑未改范围；修改后按影响运行最小充分验证。

| 命令 | 实际结果 | 最终日志 SHA-256 |
| --- | --- | --- |
| `cargo test --locked --offline --lib rows_backup_` | 20 passed，0 failed；compile5m44s，runtime131.13s | `cd9770833e5bc3c7ce2fed6a83e1b1aa6b3bd18736067971bf3c5b288b10726e` |
| `cargo test --locked --offline --lib f2_candidate_scope` | 6 passed，0 failed；compile4m43s，runtime10.20s | `1ac82a7d744aaaa94fed84a724e204e123f2e580fec6e64a0f36b556c5411c24` |
| `cargo test --locked --offline --lib candidate_scope_schema_` | 5 passed，0 failed；compile4m30s，runtime2.24s | `40a97ee27782c6fbf561d82f6e2ea36597ec00c11a93cc39e6cd79ba351c7e41` |

以上原始日志和JSON回执位于当前工作树本地 `.planning/2026-10-02-platform-continued-implementation/` 与其 `validation/`；该目录被忽略，**没有推送原始日志**。新 clone只能获得本交接与tracked验证摘要，不应声称读到了本地原件。

关键回执文件：

- `validation/rows-utf16-final-20261003.json`
- `validation/f2-candidate-scope-final-20261003.json`
- `validation/catalog7-candidate-scope-schema-final-20261003.json`
- `validation/authorized-f2-source-and-catalog7-schema-push-20261003.json`

独立reviewer `/root/temp_sql_root_cause` 实际只读精确差异；没有代跑Cargo。root负责实际验证。

已知失败 / 工具经验：

1. Rows首UTF-16回归先证明64/96字节差异，再失败于fixture bootstrap WAL残留；不是预算路径RED。已复用既有isolated helper修fixture。
2. SQLite UTF-8 TEXT仍可含损坏字节；缓存Diesel2.3.7 Text解码使用unchecked UTF-8。不要把TEXT类型检查当Rust String安全证明；当前来源组件已改bounded Binary + checked decode。
3. F2首完整轮2PASS/4FAIL：多次独立评估复用累计CopyWork session导致真实预算耗尽，随后mutex poison。修复仅在每次独立测试评估创建fresh actual session；没有重置或增加生产32MiB预算。
4. C7首4PASS未覆盖隐藏rowid REPLACE；最后5项含四别名攻击才是收口证据。
5. 过滤器匹配0tests不算通过。Rows私有typed模块真实路径在 `database::global_schema_v1::rows::tests`。
6. 仅一位Cargo executor使用共享target；不要并行多次Cargo。lib编译常需4–8分钟，限定`--lib`或具体`--bin`，不因已有PASS追加同目标check/build/clippy。
7. scoped rustfmt用 `--config skip_children=true`；递归格式化曾发现无关既存差异。decision/mod.rs原approved/action声明排序差异保留，未为此扩大源码修改。
8. 文档任务仅内容核查和diff-check。`.planning`不可force-add；tracked但所在目录被忽略的ops文档使用 `git add -u -- <path>`。不要broad-add。

## 5. 下一项最小可验收切片

**从 b005源码基线继续 Catalog7 closed reference/borrower + immutable bounded observation owner。** 先冻结受影响路径、资源预算和发生身份；新旧代际分别严格验证，保持原financial codec和单owner。

建议顺序及完成标准：

1. 增加显式closed generation7完整reference/classification及受限actual borrower；原Catalog6 API继续只接受exact6，不允许未知objects/影子TEMP/foreign namespace。可复用机制，但不能拿VerifiedCatalog6冒充7。
2. 将原来源捕获接入同一个actual IMMEDIATE事务：closed UTC slot/revision → 原cutoff capture → 完整canonical/digest/新occurrence身份 → no-clobber append。source ID和发生身份分开；storage reader不给ApprovedPaperIntent。
3. strict stored reader先预算后加载BLOB，重验canonical/digest、logical/physical membership及原namespace。same key + exact bytes返原记录；same key + changed bytes冲突；不overwrite，不靠ignore/replace完成retry。
4. 所有可修改SQL hooks后精确tail，独立post-COMMIT reader使用原cutoff/原authority；Unknown保留真实已提交记录、不自动重放或重建实时资格。
5. meaningful actual tests：insert/cold reopen/exact retry/conflict、双coordinator race、catalog shadow/unknown拒绝、last-hook drift回滚、真实external child在新reader first-main-SQL前commit后的Unknown保存原记录。
6. 定向验证 + 必要独立复核 + 更新持久计划 + commit/push feature并核实际远端OID。不要为source日常编辑部署生产。

相关入口：

- [global_schema_paper_v6.rs](src/database/global_schema_paper_v6.rs)：当前HRTB loan、maintenance lease、mandatory tail及独立reader机制。
- [global_schema_catalog_v1.rs](src/database/global_schema_catalog_v1.rs)、[Diesel catalog capture](src/database/global_schema_catalog_diesel_v1.rs)：closed代际reference/classifier/CopyWork。
- [Global owner](src/database/global_schema_v1.rs)、[Rows](src/database/global_schema_rows_v1.rs)：整体模式/namespace/备份行保全。
- [实际C6测试](src/database/global_schema_paper_v6_tests.rs)：完整非空V1/V2 seed/genesis/financial fixture和当前来源/DDL测试。

本地计划（不在Git中）：

- `.planning/2026-10-02-platform-continued-implementation/task_plan.md`：主线最新追加状态；旧段按时间解读。
- 同目录 `catalog7-candidate-observation-contract-20261003.md`：已完成fixed schema合同。
- 同目录 `f2-candidate-source-slice-20261003.md`：已完成来源组件合同。
- 同目录 `formal-paper-issuers-next-slice-plan.md` 与 `formal-f2-evaluation-readonly-readiness-20261003.md`：formal身份、真实facts/risk/funds/positive factory后继要求；旧NO_CODE_GO是当时状态，当前fixed/source片已实施，但没有由此批准生产或全部formal owner。
- 同目录 `global-target-next-slice-readonly-readiness-20261003.md`：target/apply仅准备，没有目标源码。
- 原生产计划位于 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.planning/2026-09-29-platform-production/task_plan.md`，不在当前worktree；该文件旧段不可覆盖10/3新事实。

## 6. 整体剩余顺序

1. C7不可变观察持久owner及正式F2 identity：当前source/fixed DDL只是前置。
2. source-backed instrument、真实lifecycle/band/tick/suspension、整数执行价格/数量/有效窗、完整逐规则risk/cost/liquidity结果。
3. 显式B/allocation/seed/cutover批准、唯一positive intent factory和actual Paper consumer；固定B不得由默认本金、健康、f64投影或caller token制造。
4. Global target/apply/恢复及production requalification，保留原V1–V6历史。
5. 逐Unit同事实shadow、单physical owner接管、实际权威receipt/恢复/cleanup；Uncertain依人工证据裁定。
6. F4研究/决策/账本/归因关联、外部WORM/Gate P及自然运行、持有窗口和前瞻观察。
7. M6按数据与证据决定必要实现；M7策略保留/限制/淘汰；M8有触发证据才实施，否则有证据关闭。

估算只供排期：已识别可控核心主线约15–30有效开发日（每日8小时口径，含复核返工），完整上线暂按2–3个月以上量级预留。不是固定交付日期；真实数据合同、Unit数量/每交易日最多一个physical-owner晋级、自然窗口及未冻结研究范围需重新估算。

## 7. Windows Codex / gRPC 交接

现有聊天标题：**R08 FuturesDelivery 上游合同与部署**。

- threadId：`01a0e0cf-2276-7512-96ee-3a94bdfa8ca5`
- hostId：`remote-control:env_e_6ab6a791c27c832a98417a42584a1a39`
- 最后compact cursor：`a8c7a172-615a-48fe-b562-46ed52452e6d:3`；可用现有wait工具做一次有界状态核查，避免重复派发。旧cursor失效不代表任务或消息失败。
- 最新实读交付包：`/Users/zhangzhen/Desktop/Quant/stock_analysis/client-bundle/windows-d14-contract-research-20261003.1`；REPORT、manifest及25公共成员已实际no-follow bytes/SHA核验。manifest SHA：`d5766f82ac9127ab1dab05c69518ff009dbe5c8efd4a5b7d8188617b64bd5d40`。
- SDK基线9da925a8，交付文档提交 `62502520c75feee34bf9ed67aaa846f60b9d3948`：只有两份研究文档，不是新SDK能力、服务部署或真实RPC。最后实际compact状态completed/idle；Mac读取ACK工具发送成功，但未取得之后的新对方ACK，勿伪称已收到。

**当前Mac前置欠项：** 导出WG07正式request/result规范、实际qualification caller seam及消费fixture；明确定义新接口的精确窗口、as_of、instrument/adjustment、来源绑定预期交易日和逐代码终态。旧`days=90`不静默重解释。现有 `pending_daily_change_confirmations_async` 固定unavailable，`QualifiedDailyChangeDiscovery`只有test factory；review identities只接受`outcome-provider-sequence-v1`。

TDX native bar缺返回issuer/venue；Hithink v2是observation-only，source exhaustion/calendar Unknown、publication/revision NotProvided、PIT=false；六份旧SZSE短窗口不证明90日发现。需先完整source合同，再做typed qualification与兼容回归。不要新增无caller的观察脚本或永远unavailable的伪生产能力。

同版fixed36真实RPC、D14/D17/D20/R08 confirmed仍开放。Health/离线plan/合成fixture/不同版本业务结果不替代该验收。本次未新发Windows任务、监听、capture、stop、部署或运行RPC。

## 8. 生产事实与接手边界

最近生产观察是 **2026-10-03 09:28 CST的历史只读snapshot**，本交接未刷新生产：

- monitor PID4371、bridge PID56417当时同boot；正式根 `/Users/zhangzhen/.local/share/stock-analysis-runtime`，launchd管理。
- 当时Frozen/Unsafe、metrics incomplete，缺Quote/MoneyFlow/News/OrderBook；不能把该历史PID/状态当接手时fresh检查。
- 最后记录安装的是原Wave0制品（monitor SHA前缀851a5fb9、activation前缀f574faf1），本轮开发源码没有部署。完整候选哈希及原审批元组见原activation review/preflight，不从此处短前缀执行切换。
- 78条Uncertain是更早历史数，未重新计数；不得当新查询结果或自动裁定。
- source/config上线仍按未来effective_from的精确activation人审、hash、single-instance、数据库/lease、水位、source与Uncertain门禁执行，使用正式launchd流程。

主heartbeat最后持久记录ACTIVE，本交接未修改自动化。若另一个会话实际接手，先核当前任务/自动化执行状态并确定唯一源码与Cargo owner，避免双方同时开发、发布或验证。不要仅因交接就把M0–M7目标标Complete或停掉尚有用途的自动化。

## 9. 可直接给接手会话的指令

> 请先阅读仓库根目录 PLATFORM_HANDOFF.md、AGENTS.md 和 CLAUDE.md，核对实际HEAD、未提交改动和当前执行owner。沿用 codex/platform-roadmap-implementation-20261002；从最后已验证源码 b005457e 开始下一片 Catalog7 closed reference/borrower 与不可变观察持久owner，验证原cutoff exact retry/conflict、tail、独立reader、race/cold reopen和Unknown保留，再提交推送该feature。继续M0–M7全部上线及M8有证据裁定目标。不要把Top50来源捕获或固定DDL当正式F2/生产资格；不要复跑未改范围或绕过精确生产/VM运行/资金门禁。协调Windows时先读既有D14包，完成Mac WG07合同/caller前置，避免重复派发。


## 2026-10-03 接续：消息恢复与 ExternalV1 缓存鉴权修复

本轮源码由当前接续 chat 的 root 独占修改与 Cargo 验证，沿用原 feature worktree。生产恢复与开发验证分开：10/3 18:01 CST 已取得真实 Feishu DataMode Accepted/Delivered 和69条新闻；Windows兼容服务与Mac原Wave0制品已恢复。新开发源码尚未部署，78条历史 Uncertain 没有自动重发或裁定。生产恢复凭据在 `/Users/zhangzhen/.local/share/stock-analysis-runtime/ops/recovery-20261003/production-verification.json`，旧9:28 snapshot仅为历史事实。

修复两条实际行为：NewsAI人工复核提示以稳定notice identity、合法 `Denied` / `internal_audit` 事件发布，成功后才确认数据库notice；不是消息送达或人工裁定。ExternalV1 在每次获取连接时重新核验Health身份和Capabilities，缓存移出后再等待，失败或取消即释放原缓存；返回刚核验的连接，避免重新读缓存的并发panic。失败请求不在同次获取内重拨，下一次独立请求才重新准备bundle并完整鉴权。关闭reason-code映射新增已知 `external_connection_unqualified`，拒绝分类保持不变。

实际验证46项通过：`cargo test --locked --offline --lib external_cached_ -- --nocapture` 5项；同一已编译库harness覆盖公告路由4项、外部配置3项、开盘能力22项、notice数据库恢复1项；`cargo test --locked --offline --bin monitor news_ai_shadow::tests::br172_ -- --nocapture` 11项。库最终日志SHA256 `c081ea48ca6891eb36913a79b393fdb8f640582f61676c798df4c82505f7ce04`，monitor日志SHA256 `b87ac2a1726b0b28bf7453981c78ff2506c2f611f3d451efee97f8ceac3cac88`。首轮3PASS/1FAIL揭示reason映射遗漏，失败日志保留。独立复核发现测试服务器abort/join不能证明所有TCP连接已关闭；已改为有界等待tonic正常graceful shutdown完成再重绑定，窄复核无剩余发现。`git diff --check`通过；未执行release/生产切换或全量测试。

下一片 Catalog7 whole-catalog borrower 与不可变负面候选观察已在独立scratch完成，待root应用、实际测试及独立复核。它不是正式InvestmentDecisionId、资金批准或Paper执行资格；后续Global target、真实source合同、完整risk评估、资金seed/cutover、WORM及自然窗口仍需继续完成。


### 2026-10-03 Catalog7 持久候选观察（待独立复核）

完成same-runtime legacy/transitional/amended固定7全参考、精确header/SQL/geometry/FK/payload闭合，以及复用唯一Global事务/namespace/lease/累计CopyWork引擎的独立7proof。旧6接口仍硬性要求6；7生产入口在checkout前拒绝，不是生产migration。财务校验只机械共享原完整row replay，新增固定7 fee入口，无PRAGMA替换或codec/业务算法变化。

不可变观察owner自己取一次UTC时钟，policy固定intraday-unconsumed-pushed-row-top50-v1、30秒UTCslot和revision1。同原IMMEDIATE先读existing key：重试保存首次cutoff/bytes/ID，不重扫后续source；首次捕获原Top50后checked正数rowid append。类型/长度/digest/closed-canonical及流式JSON预算预检在owned decode前执行。最后hook之后验证原row及首次source，COMMIT后独立retained RO snapshot复核；真实子进程在fresh reader前改变source返回Unknown且保留已提交原事实。Scope/occurrence独立域及literal golden，不颁发InvestmentDecisionId、审批或Paper资格。

实际80项主harness检查全PASS：新candidate_catalog7 12、identity golden1、旧global_catalog6 22、原f2_candidate_scope6、受影响paper_book_v2_execution39；两项ignored仅供父测试精确reexec的子进程，由父测试实际运行并验证，非跳过关键行为。库构建4m39s；新12项运行43.80s，日志SHA2560f3376d71831707df13ed762197e2f3f251a44bdd17ca3050561e1588d8f55c8。累计检查复用同一复制且SHA核验harness，未重复Cargo build/check/clippy或运行全量。编译后仅清除新增unused import，静态确认无引用、rustfmt与diff check通过；behavior-neutral清理另有SHA记录。新源码尚未部署，独立spec/quality review尚待完成。

Catalog7独立全任务spec/quality复核已完成：无Critical/Important或需修改Minor；80项实际验证与未上线边界被复核。源码提交d2d1667b，紧接消息修复09a9370a。下一task严格限定实际amended6未批准target producer；不拿7观察或旧JSON/hash颁发migration/apply资格。


### 2026-10-03 M1/Catalog7 实际整合验证

在源码d12be0ed完成一次 `cargo build --locked --offline --bin monitor`，exit0（6m52s）；保存private复制并SHA核验的实际monitor制品，SHA256 `8b2308dc690884dda5f32874a42aa6aaecbb6b9dbf2ad993516aceda81ae745f`。执行 `monitor --test --push-dry-run`，独立TEST_CODE审计路径：59模板、4batches、failed0、external_process_attempted0、receipt_audit_appended0、live_opt_in=false。首次进程exit0但继承warn日志过滤导致验收计数不可见，保留该不足证据；同sealed制品显式info补取后计数完整，exit0（3.557s），实际日志SHA256 `15bbdb0f5a1249baea22159004f04a6d4929ae3d9b8ac5cc4e8d26d6150d0975`。未启用真实外发、未更换生产制品。现有counted-binding缺口仍原样拒绝，模板检查不代表正式业务source或生产验收通过。

## 10/3：实际未批准的 Catalog6 目标副本开发验收

本片从 a05b09f6 实施，固定配方 `requalification-exact-amended-catalog6-v1` 只接受实际 amended Catalog6，复制到同一精确代际的受控目标文件。其他 family/generation 在创建目标前拒绝。原 Global 独占租约、真实 Copied FD、源/备份/审计绑定继续保留；独立四槽日志记录 intent、created、copied、verified，冷启动只重开记录中的原 inode，不收养、截断或覆盖未知副本。

目标完整 catalog/header/geometry/payload/integrity 与逐表逐行 storage class、REAL bits、原始 TEXT/BLOB、rowid、双 EOF 和 sqlite_sequence 实际验证。原六流与目标四流分别固定；最终 reader 关闭、所有可变 hook 完成后重验全部原文件、目录、锁、sidecar 与记录，之后只有限编码。原/目标元数据各 16 MiB 持续累计，目标 catalog 与路径、记录、编码共用目标的唯一计量池；其他原校验和两侧 typed/comparator/transcript 继续原池，不提高或重置上限。

实际 **43 项定向 PASS**：14 个新目标 owner 用例、3 个共池/溢出用例、20 个原 Rows 用例、6 个受影响原备份用例。真实非空 V6 seed/genesis/财务 fixture 的复制与冷重开通过；count-preserving typed mutations 确实进入比较器，五种结构攻击均须匹配实际 catalog 拒绝。前期元数据归属、测试 sidecar 生命周期、writable_schema 攻击被提前阻止的失败均保留，未把 poisoned-lock 连锁失败当独立缺陷或通过。

本地 `validation/dev-20261003-target-final-acceptance.json` 记录不同编译产物的复用来源：fix2 业务源码与最终相同，fix3/fix4 仅修改两个 Test 消费者及最后一个 Test 攻击/断言，未重复未改范围。最终 fullcatalog 日志 SHA `af00af39328819051c4f2fb6c2c56515cf6265e4f48034a559dd240bd51e8527`，最终封存 lib harness SHA `3e07c6450562d05f55395ee28cf1d893bdc36095c13bb6d78b35258e3277a5b2`；未运行全库、release 或部署。

独立全任务 spec/quality 审查待进行。本片输出明确 `approval=not_granted`、`maintenance_receipt=not_created`、`exchange=not_implemented`、`apply_supported=false`；不代表历史到最终代际迁移、production requalification、Paper 或全平台完成。后续继续 WG07 完整窗口与复核消费、逐规则风控和完整不可变 F2，再处理真实源、明确资金、批准迁移/切换及 WORM/自然观察。


### Task2 审查 I1 修正实际验收（待范围复核）

独立全任务审查发现 original source/backup journal metadata 误扣 TargetWork；现由保留的原 RowsSpecWork 在每次 origin loan 的任何原验证/回调之前支付原 reservation，移除该项错误目标扣费。两池数值上限不变、不重置或退款。新增真实 Catalog6 cap 回归验证两次借用累计，第三次差1字节时在原校验、回调和目标扣费之前拒绝。

修正后实际4项定向 PASS：新增累计原 metadata 回归、真实财务副本/冷重开/四流、未知partial与缺槽冷拒绝、原rows work不能重置。仅一次 `cargo test --locked --offline --lib` 编译，之后复用同SHA封存harness，不重复43项原范围或全库。实际新回归日志SHA `b63353667aaa2f28415f492e5614662a21ea0b194e902d6f2f7622d10df71450`；harness SHA `f92dc1ca43774a5ed403799699b50b7539294366004842f484e1112822bd8d27`；本地 `dev-20261003-target-reviewfix1-acceptance.json` 保留四项命令、原日志与源码归属。I1范围复核待完成，未部署或取得批准迁移/切换资格。


Task2 I1 范围独立复核已通过：原 reservation 在原校验/回调之前累计扣保留的原 meter，fresh/cold 路径及四项日志/源码/hash核验一致；无新 Critical/Important、无其余观察。源码检查点 f6bebc2f。完整 Task2 审查与唯一 I1 修复闭合，当前开发片完成，仍无批准迁移、切换、上线或 Paper 资格。继续 Task3 WG07 完整窗口与既有复核账本/后续精确消费。
