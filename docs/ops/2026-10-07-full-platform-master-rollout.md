# 完整平台合入主仓库与上线状态（2026-10-07）

## 用户目标与已完成合入

用户本轮明确要求“全部合入主仓库 全面上线”。原新闻窄候选只保留准备记录，本轮按完整平台推进。

已将本聊天已完成的平台完整开发分支 `codex/platform-roadmap-implementation-20261002` 快进合入主仓库默认 `master` 并非强制推送：

- 合入前：`b7e12bb5c4729a7956124b2db121c7e338e21922`。
- 合入源码：`cde590dc09c4f01a0727200241489667c399bf59`，421 个提交、396 个文件；树 `bc85ab8d7ade717a16c7eab6edec44b3f57835a2` 与开发树完全相同。
- 实际 `ls-remote` 回读的远端 `master` 完整 OID 相同。主目录 `/Users/zhangzhen/Desktop/Quant/stock_analysis` 快进后干净。
- 远端 HEAD 实际指 `master`；本地缓存 HEAD 曾指 `main`，该分支仍是初始提交，不能据缓存选择发布分支。

本次将已完成的代码和其中明确保留的拒绝路径纳入主仓库。H01–H17 中未实现的资格、接线及真实证据仍按[剩余开发交接](../handoffs/2026-10-06-platform-remaining-work-handoff.md)验收；合入不改变这些状态。

## 本轮发布检查

1. 原 `cargo fmt --all -- --check` 退出 1；实际失败涉及 69 个 Rust 文件。后继按原输出限定这些路径执行 rustfmt，不手改业务逻辑、DDL、准入、阈值或 Rust 字面量；原失败和路径清单保留。
2. 421 提交的原 `git diff --check` 将两个封存 Windows RPC 输入的 CRLF 行尾报为尾空白。原件字节和哈希保持；补与其他封存合同一致的 `.gitattributes` 精确路径声明，以识别 CRLF，未全局放宽空白检查或修改 Git 配置。
3. `ruby scripts/architecture-docs/check.rb --check` 实际退出 1：旧蓝图/v19 文档输入 hash、current 证据路径集合与源码 hash 不一致，历史/current 目录仍是 `PROVISIONAL`，另有 RFC/HTML 差异。清单更新需要真实 source evidence 与审阅；不能直接改状态、删除旧材料或跳过 strict checker 取得成功。
4. GitHub Actions 仓库级开关原为 `enabled=false`，因此首次主分支 push 没有 CI。已按全面上线的验证需求恢复现有 CI，原 workflow/检查器/门槛保持；新 push 的实际运行和终态须单独回读，不把仓库开关打开算作 CI 通过。

先前定向验证可在原范围内复用（最新 H02 manifest 新 6 + 旧 V1 6 项通过，修复批次另有对应回执），不据此声称完整工作区、发布 CI 或新制品通过。仅格式和声明变更按 AGENTS 内容/diff/格式检查验证；真正发布行为还需下述 release 和业务验证。

## 生产现场与当前门禁

10:27 CST 只读快照：正式 monitor PID `14998`、bridge `56417`，均为原 launchd 实例；monitor SHA `a7376a14cde1f6be3025d6dbbdedf61401c34154f3da928a8e3db30c4950d20b`，activation 文件 SHA `3c8b49dba568a9e4530ce5598802ea6e6e6ba7e015ffd0fdcd297ea5c0de16b5`。实际运行根 `/Users/zhangzhen/.local/share/stock-analysis-runtime`，源码仍 `f517f45c91ec9489f63d0156a1b8f9cf45400c3b`。

- 两库 dev/ino：main `16777220/154379673`，durable `16777220/154266271`；durable schema 9，995 Delivered、3988 RejectedDurable、6 ManualResolvedRejected、78 UncertainManualReview。
- 78 条按种类：DataMode 73、CloseCall 3、T0Advice 1、WatchlistTracking 1。实际组摘要 `2223d83d38c491b4c4bc6608f0636aca939b454bc57bfecc10760e9fb6f63290`；未裁定、删除或重发。
- 健康进程/heartbeat/snapshot 新鲜，但业务 `Frozen/Unsafe`，账户指标不完整；缺 Quote、MoneyFlow、News、OrderBook。当前 news_dedup 表 0 行，不能伪造跨午夜现场验收；global critical 审计 1932 条也不等于全部推送验收。
- 实际 VM 为旧 `4e4995`，完整 Mac 开发版绑定较新的 ExternalV1。Windows `7174f09f0b34d082bc584180ba4260d53f379d6c` 的原 CI critical `35392/39817=88.89%` 未达 95%；overall `84.96%` 通过其 80% 门槛。新 WIP/离线测试和不同版 Health 不授权切换。
- 原 schema14/v1-v2 fallback `caafa228` 三目标 release 制品和隔离 dry-run 回执存在；未据此取得当前真实库的兼容回退、迁移及 activation 资格。v2 历史写入后禁止直接换回旧 v1 reader 或恢复旧数据库。

## 后续发布顺序与 owner

1. **Mac**：修实际主仓库验证问题，保历史 source evidence 与失败，取得原 CI/检查终态。
2. **Windows 原任务**：已成功同步用户全面合入/上线授权，继续验证过的源码提交、上游主仓库合入、同 HEAD 原 CI 和真实关键覆盖率修复；不得降低 95%。Bash CPUID 机制证据不等于运行库修复。
3. **双方**：合格新 SDK source/binary/descriptor 原 tuple → Mac 精确重绑 → Desktop 外完整 normal release → 同制品隔离 dry-run → 兼容 schema14/v2 回退与迁移方案。
4. **发布 owner**：形成候选/回退、输入清单、配置 hash、future9 activation 精确审阅单。工具和 CLAUDE 要求人工审阅，旧 Wave0/Wave1 批准不覆盖新候选。
5. **切换前**：重查 PID/lease/全部 writer、真实源/水位/数据、账户/资金/seed、Uncertain 人工决策及两库身份；动态门失败保现网和原数据。按 launchd 单实例流程切换，禁止 nohup 或双 physical owner。
6. **切换后**：等待实际 DB 初始化、桥接重连、配置生效、同版真实业务 RPC/消费/回执与自然观察，才记录对应 Production Verified。M0–M7 全部达标及 M8 实施/不实施裁定前 heartbeat 保持。

需要明确区分：全部已完成源码合入、发布制品验证、正式服务切换、全部能力验收。当前完成第一项；现网未执行本轮完整平台安装或重启。

详细本机证据在 `.planning/2026-10-07-full-platform-rollout/`，原合入回执另存 `.planning/2026-10-07-news-dedup-rollout/validation/master-integration-receipt.json`；不提交私有配置、数据库、Token、证书或大包。

## 原45a73caca CI终态与第一批修复

恢复Actions并推送后，精确 `45a73caca4bbb11bb11f9465070c87f2c1ddedec` 的原三个run均失败：Rust CI `37562970073` 停架构检查；compliance `37562970126` 停离线检查；coverage `37562970290` 停 fontconfig 系统库构建。Rust全量测试和覆盖阈值执行尚未到达，不能将它们写作测试失败数或实测覆盖率。原日志、steps与摘要保留。

第一批修复：

- 三份原workflow补足 Linux fontconfig/freetype、SSL、SQLite、PAM、pkg-config、protoc、ripgrep；不改 features、profile、检查器或覆盖阈值。YAML语法通过，是否修复实际Linux构建由新同HEAD CI裁定。
- 恢复八份冻结输入/来源目录中的两份被改写原件，真实Git blob精确匹配原SHA。后继健康和熔断说明完整保留在[设计输入恢复说明](../architecture/current/2026-10-07-frozen-design-input-recovery.md)和Git历史。`rfc_inputs_valid`、`source_catalog_valid` 实际通过；当前源码审计与PROVISIONAL等strict条件仍未关闭。
- 纳管主目录原有但被忽略的 BR174 脚本，原字节复制；`--self-test` 与当前源码实际扫描都通过。
- 纳管回填脚本缺失的共享 timeout helper。监护进程保持命令42退出、成功输出、monotonic截止，并在截止或自身取消时终止新建的自有进程组。四个真实边界测试通过：42、成功stdout、截止2且TERM-ignoring孙进程停止、监护进程SIGTERM取消143且孙进程停止；现有回填失败传播检查也通过。只在TEST_CODE临时目录运行，没有真实回填或生产库写入。

剩余合规失败已有原件：移除的本地provider宿主/TDX路径仍被业务规则列为active；T08未引用logging-only壳；盘中overlay和量比的0.0哨兵与缺少必需回归；BR194静态检查的账户phase/schema版本与后继实现尚需逐项核对。处理时保留真实来源/未知/拒绝，不能删门禁、增加白名单或用数值替代缺失数据来通过检查。当前源码audit须更新调用链和内容证据，不能批量刷新哈希或直接晋级目录状态。

## 第二批合规与盘中缺失字段修复

第一批修复已提交并实际快进合入、推送 `master` 和开发分支：`2954abf6fe348e7c91a80e18f37aacfd776ed1cb`。新同 HEAD Rust CI `37564408930` 仍停在架构检查；compliance `37564408933` 中 BR174 与回填/timeout 已通过，另四域仍失败；coverage `37564408925` 已通过系统依赖安装并进入工作区编译，尚无 measured coverage。原日志保留。

第二批按实际失败修复：

- 盘中 `StockSnapshot` 的量比和主力净流由 `Option` 表示。主循环保持缺失，只跳过依赖该字段的规则，独立价格规则照常检测；真实零和负流保留，排名只包含有真实流量值的股票。
- 原无生产 caller 的 `push_candidate_invalidated` 日志壳已移除；P05 shared owner、三个 kind 的真实 presentation 与既有恢复边界保留，更新受影响的原测试范围。
- 业务规则限定 11 条 Code 列的 15 个失效 active 路径，按两次真实删除提交指向现在的 Gateway 调用、转换和纯校验。历史 Intent、来源与 pending 状态保留；新指针不证明上游宿主/RPC，亦不恢复已退役的 selection 管线。247 条规则检查通过，181 条既有/新指针引用警告真实保留。
- BR194 静态检查按现行来源阶段/账户阶段分开：来源任务完成后才读 banner，缺失完整性保持 false，R03 只在账户完整且 exact task 条件内 dispatch，拒绝分支无 provider/sink。声明核到 schema14 并逐 reopening 分支检查三个扩展 catalog。原 70 项和新增 8 项破坏变异全部拒绝。旧 `verify_br194_review_join.py` 仍是 schema9-only，不能用它签发 schema14/v2 生产 join。
- `bash tools/compliance/check.sh --policy pr` 实际退出0；这是离线检查，未签发生产 freshness 或上线资格。冻结 current-source 审计、RFC/WBS 状态与严格架构门仍开放，不能跳过 strict、批量刷新哈希或改 PROVISIONAL 标签取得绿。

盘中回归先取得四项真实 PASS。审阅再发现格式化层仍把观察到的零当作缺失、T+1 卡会填缺失价格为0.00；后继修正只按 Option 呈现实际值，缺价格省略该行，并增加零值显示和真实/缺失 T+1 价格的行为验证。最终范围、回执与提交在本机持久计划续记，未据先前四项替代后继结果。

本批直接复核了实际 diff、全部三个 StockSnapshot 构造边界、真实缺失/零/负流/量比与价格呈现、来源/账户调用顺序和扩展重开拒绝。当前没有另一名独立 reviewer 复核这份最终差分；原完整平台其他切片的独立报告仅在其对应范围有效。完整候选还需 normal release、同制品隔离 dry-run、必要独立复核及生产门禁。

后继最终 `no_silent_fallback_test` 五项行为测试全部通过，包括真实零显示和缺失/真实 T+1 价格；原正常 monitor 与其余默认 bin 在该 Cargo 目标构建中通过编译。已有 warning 保留：lib 888 项、monitor 14 项；没有运行或声称 Strict Clippy/完整工作区测试通过。相关 overlay、P05 和原 detector/alert/integration 回归另按实际结果补充，不复跑无关目标。

11:14 CST 再次只读核验：正式 monitor14998、bridge56417 仍是原 launchd；monitor、bridge、activation 三 SHA 与10:27原件一致，两库 dev/ino不变，durable schema9与995/3988/6/78计数不变。Health退出1，账户Frozen、数据Unsafe；本轮没有二进制安装、activation重发、重启或数据库写入。

Windows完整源码已保留双方历史合入并推送其远端默认 `main`：`2b225b71148425d18c82a3dcc552f1444f3595cd`，树 `e25961a4e3887ff2697e59fe6a28616a20a3ec93`，远端完整OID独立回读吻合。新源码交接包141成员/2,720,976字节核验通过；Mac另从远端浅取同提交、核70个Git blob，52份原字节一致、18份仅working CRLF/Git LF差异，原件未改写，包外ACK已送达原Windows任务。该源码包原CI `37564892679` 的终态、95%关键覆盖、同版binary/descriptor/真实RPC未交付；现网仍旧版，不据源码合入签发 SDK 部署资格。

### P05 锁失败与 owner 身份重复回归

扩展 P05 原回归首次为8通过/1失败，错误在旧 durable-runtime 测试打开 coordinator 的 capability OFD marker，EAGAIN35。失败原log与回执保留；本次直接修改的shared-unit七项在该次已通过。独立 TEST_CODE 临时文件的原生OFD探针表明分离范围成功、相同范围返回35，未触碰生产或真实Cargo文件。

随后发现 monitor owner 身份仅由PID与墙钟推导，重复时刻不保证唯一。先提取同语义私有时钟 seam，真实行为回归取得RED（两个身份相等）；加入进程内checked atomic序号后，重复时钟和八个并发同钟owner两项通过，原P05九项重新通过。没有改coordinator、锁范围、拒绝语义或测试线程数。原失败未记录owner绑定，不能断言它完全由这一因素造成；新代码及回归关闭的是可复现的身份重复缺陷。

### 本批最终验证及上游最新终态

最终锁定 offline 定向回归共51项通过、0失败：缺失字段/呈现5、CandidateTriggered绑定2、overlay6、P05原范围9、owner身份2、detector15、alert7、monitor integration5。最后三个库过滤目标分别执行，复用同一编译，没有重复追加 check/build/clippy。完整离线合规、78项BR194破坏变异、`cargo fmt --all -- --check` 和 `git diff --check` 均通过。库测试编译仍有152项 warning；普通库先前888项和monitor14项warning记录保留。未运行本批完整工作区/全部features、Strict Clippy、normal release或同制品dry-run，不将定向通过等同发布CI通过。

Windows精确 `2b225b71148425d18c82a3dcc552f1444f3595cd` 的原CI `37564892679` 已终结：audit通过，overall `69340/81600=84.98%` 达80%，critical `35411/39819=88.93%` 未达95%，原检查器退出1。新终态包37成员/36,946,674字节已逐件核验，包外ACK范围为原件读取，原失败未改写；分母不变时还差2418个关键覆盖行，后续源码变动须重新实测。已成功通知原Windows任务继续真实handler/adapter行为测试与同HEAD原CI，合格后再交付新SDK source/binary/descriptor和真实RPC；旧服务不切换。

11:14 CST快照中News已恢复，最新缺失集合是Quote、MoneyFlow、OrderBook；10:27四项缺失仍保留为历史观察。账户指标不完整、Frozen/Unsafe及78条Uncertain状态没有因此关闭。全面上线仍依赖严格架构源审计/RFC资格、完整CI、同版SDK与兼容回退、真实数据/资金/人工裁定以及新精确activation审阅和自然生产观察。

## bb725b05 原 CI 后继与失效集成入口

第二批实际提交并同步两远端及干净主目录：`bb725b05e8c5716fe96751a29fcd699f8fb0e744`，树 `8f277506833bc946e45479b6b78ccf7b43f97a52`。原Linux compliance `37566852932` 已通过离线合规，后续 Cargo 在启动测试前退出101：`no test target named e2e`。该目标不存在于当前Cargo、主目录或仓库Git历史；不能将这一入口错误算作测试行为失败，亦不能从本机offline通过推断原run全通过。原Rust CI `37566852954` 仍停严格架构证据/PROVISIONAL，Coverage `37566852951` 尚未取得终态或实测阈值。

后继保留原六个存在的关键集成回归和单线程design-contradiction范围，以仓库真实 `grpc_bridge_e2e`、`grpc_channel_e2e`、`durable_delivery_counted_cutover` 接替失效目标，另将 Cargo 锁定。新增范围使用既有fixture与隔离durable owner；不创建空test壳、忽略失败或跳过原预测/排序/freshness门禁。实际新增范围结果另录，工作区/完整原CI终态仍须回读。

新增范围实际发现两类旧预期：桥接测试仍将刻意关闭的raw FuturesDelivery当成功、将已退役EconomicCalendar当invalid evidence；通道仍断言旧40项声明。按真实 `1cc28f425`/`419befbfa` 和 `df4202502` 合同变更修正既有断言，保持明确不可重试拒绝，并核当前41个唯一operation、MarketAnnouncements与BenchmarkBars必需声明。没有修改production Gateway、transport、准入或 fixture advertisement。

当前三个目标合计18项通过（durable6、bridge2、channel10）；前两次各自真实失败及日志保留，只对后继修改的目标补验证。Cargo metadata确认workflow全部10个integration target存在、旧e2e缺失；YAML语法、两个测试文件rustfmt及diff-check通过。此为本机受控fixture/隔离owner证据，不是同版Windows真实业务RPC；原全部lib及其余原关键回归保持CI范围，新Linux完整step仍待实际运行。

## 947c1ed2c 编译器漂移与后继一致性修复

集成入口已提交并同步主目录及两远端：`947c1ed2c6ed6fd047dcfb08a6325d46791e6beb`，树 `ee611473de427b8fd117bc18e14f4e7aae24480a`。其原Linux compliance `37567506680` 的offline门仍通过，已进入正确测试范围的依赖编译；实际后继退出101是 `ethnum 1.5.2` 的E0512，原日志中 `TryFromIntError` 已为8bits，原transmute源为0bits，尚无test harness运行。

完整原日志确认 `stable` action本轮安装Rust `1.99.0 b940084d7`；仓库BR-252合同、coverage原workflow和本机已验证Cargo均固定 `1.95.0 59807616e`。后继将Rust CI与compliance的compiler对齐现行固定合同，Rust CI显式安装rustfmt/clippy。没有改Cargo.lock、第三方源码、features、profile、覆盖阈值或strict检查范围。YAML语法和合同版本一致性另验；Linux依赖编译是否解除、完整测试与覆盖率仍由新HEAD原CI裁定。

本机另外启动原 `cargo clippy --locked --offline --all-targets --all-features --message-format=json -- -D warnings`。首次全程尚无compiler JSON；进程采样固定在Cargo `PathSource`/`list_files_gix`/包输入fingerprint，而根目录有25GB未忽略的 `.replay-build-records` 生成原件。按完整PID/父进程/参数/cwd重验后只向本次Cargo发SIGINT，原driver退出255、两输出均空，记为主动中止、Clippy未执行，不是lint通过或lint失败。后继只把此生成目录加入.gitignore；原记录未移动/删除，源码、Cargo输入、合同及回执未改写。指纹阶段和真实严格诊断另据后继日志，不能仅由ignore声明声称性能或全部Clippy门已通过。

### 严格 Clippy 实际终态与剩余发布工作

生成目录忽略后约30秒已进入编译；后继严格命令实际退出101，原普通库报1222项、库测试报643项，共1865条error级诊断，按lint/信息/位置去重为1425条。这不是1425个已证实业务缺陷；存在同一根类型/未接线声明及重复目标的诊断。较大类别包含未使用声明/导入、error/enum布局、函数复杂度及可见性；async借用、锁文件打开语义等已按位置单列需复核，不能直接机械改状态或授权边界。没有新增allow、关闭`-D warnings`、删除必要测试或用批量cargo fix签发通过。日志SHA `36050e309c9c9f3d638d517704930c7c299e78c09b067bfe9a7bb6ee3b2e5962`，stderr SHA `f70650615ac31eb91986dcfb93fd2f22d5fbc52027577bb8c3960d67508f98a5`；本机remediation index保留逐项定位。由于库失败，不能声称其余所有targets的严格检查已执行完成。

编译器/生成目录修复已经提交并同步 `master`：`ab936ec011c1eb1e774495f92b97678970b5f4c1`。原master push连接中断退出128，后继独立 `ls-remote` 证实两个远端均收到同一完整OID；没有重复push或重新派发CI。该提交只改两workflow、gitignore与本文，Cargo/Rust源与上述Clippy输入相同。

全面上线的剩余次序：逐模块关闭实际源码质量与架构/RFC资格，核新HEAD原CI及完整测试/覆盖；完成H01–H08的真实资格和正式资金/决策/执行接线，取得Windows合格新SDK及真实RPC；准备Desktop外同版release、隔离dry-run、schema14/v2兼容迁移/回退及精确activation审阅；再逐项核单实例、真实数据、资金seed和78条Uncertain人工决策，切换并积累自然观察。资金口径B及是否包含当前持仓已向用户提出资料请求，不能默认为某个金额。H09–H17远端设施、52Unit/治理/研究与成熟窗口仍按原交接退出标准推进；源码合入不宣称这些阶段已完成。

11:58 CST最终只读复核：monitor14998、bridge56417及三个runtime文件的字节/SHA/dev/ino与11:14原件一致，两库dev/ino、schema9及995/3988/6/78计数保持。banner/heartbeat仍新鲜，Health退出1、Frozen/Unsafe、账户不完整，最新仍缺Quote/MoneyFlow/OrderBook。本轮完整源码合入、CI修复与发布准备没有成为正式安装、重启或生产账本写入；当前没有已准备并获审的新完整activation候选。


## 开发完成后上线：schema14 预检续行（2026-10-07）

用户最新要求“开发完成 然后上线”。在最终375a基线上继续开发，完整上线退出条件保持。后继增加[隔离副本检查器](2026-10-07-schema14-snapshot-check.md)：复用原 schema12/13/14 目录及 G5b/P05 内容/owner/修订/完成校验，无生产 coordinator、迁移、provider/sink 或批准 issuer。完整 BR194 外部审计 join 仍待开发，schema9-only 原工具不变。首轮测试因当前 rusqlite 无 total_changes 方法退出101，仅测试夹具错误；后继查询 SQLite 的 SELECT total_changes()，原日志保留，实际最终结果后补。

gRPC 导入边界清理把两个仅测试使用的名字移入各自测试模块。第一次实际 gateway 定向77项为76PASS/1FAIL；原扫描守卫发现退役 EconomicCalendar 仍在 HOOKED_OPS，启动 banner 误将其算作已接线。后继移除该名字并在现有退役用例断言不再声明；没有恢复退役能力或修改 LocalBridge fixture 的41项合同声明。该声明集合和实际接线集合分别验收，复验结果后补。

Windows 新源码 e1dc7ef1588bda8076067e86b540b07311146c21 已实际推至main；141成员/1,578,149字节全部核验、3源码 blob 独立 Git fetch 逐件完全匹配，包外 Mac ACK 范围是字节与源码读取。新增15项真实注册处理链测试及2105工作区通过是 Windows 原执行报告，Mac 没有代跑或据此签新SDK运行身份。精确新原CI37570852744已terminal FAIL：overall69654/81600=85.36%，critical35572/39819=89.33%<95；分母不变仍差2257关键覆盖行。原失败未改写或重试；结果和后继继续有意义行为测试/同版资格/RPC要求已成功发给原Windows chat。

生产仍以11:58CST只读快照为当批最后已核证范围；本续行未安装二进制、重发activation、重启、迁移或写生产库，不能把旧快照写作新的现场核验。此前将持仓、现金一并列为待用户提供的判断已按下节数据库核验撤回；正式资金/正向F2/paper issuer、受控Financial资格、完整CI、同版SDK、78逐项人工裁定及自然观察依旧未关闭。

### 本批最终开发证据

固定的9项Rust输入前后SHA一致：schema14库回归6、gateway77、候选partial decoder1、CLI链接/三类sidecar边界2，合计86次目标测试执行PASS，0FAIL。格式、完整offline合规及diff检查PASS。原gateway76/1和test API E0599失败原件保留。新普通库编译886项warning、库测试152项warning仍保留；没有声称最新Strict Clippy、全部工作区、正常release、同制品dry-run或真实WindowsRPC通过。

本批直接源码复核覆盖只读/事务/临时遮蔽/目录内容校验、CLI稳定副本与声明边界，没有新的独立审查批准记录。公开返回类型是观察数据，没有发行任何运行能力。详细范围见新操作文档；完整BR19414外部join、真实迁移回退及其余H01–H17退出条件继续按计划。精确提交及两个远端OID以实际Git回执为准。

## 已有持仓和账户资料核验更正（2026-10-07）

用户指出持仓已经在数据库。13:16 CST 对正式主库 `/Users/zhangzhen/.local/share/stock-analysis-runtime/data/stock_analysis.db` 以 SQLite `mode=ro`、`query_only=ON` 和同一读事务核验，确认此前“还缺现金、持仓信息”的结论不准确：

- `user_position_snapshot` 已有38条用户确认快照、明细表249条。按现有 accessor 的有效时间排序，最新为2026-09-28 18:15 CST的完整5只持仓；数量、成本、名称均存在，header的5项与实际明细相符，来源为 `user_confirmed_full_snapshot`。
- `user_account_summary` 已有40条记录；最新现金、总资产、市值、仓位与当日盈亏均存在，来源为 `user_confirmed_screenshot`。其有效时间与上述持仓完全相同，满足现有账户/持仓时间绑定。
- `real_account_snapshot` 的最新事实较早，不能仅因该表较旧就声称账户资料不存在；`stock_position` 是确认快照的本地投影，不能用它代替确认来源或合并成额外持仓。`position_adjustments` 当前0条。
- 当前读取时快照年龄约211小时。生产原日志13:15:47明确记录 `BR-103 account summary is stale`；`compute_account_mode_metrics_blocking` 的现行96小时限制会在纸面账本指标计算之前拒绝该汇总。这里的账户不完整应归为既有事实的时效拒绝，不能写作缺持仓或从未提供现金。后继重新估值也不改变用户确认快照的时间。

H04的接续先直接复用这些资料制作精确 seed/资金分配材料。真正剩余的是正式策略预算与持仓分配授权、完整Financial资格、唯一批准issuer及生产模拟账本接线；历史总资产记录本身不签发可花预算。已有资料整理和实现不再等待用户重复提供持仓/现金；只有具体方案仍需用户作出的选择，才在材料完成后提出。

原始金额、逐仓明细及查询/日志回读保存于本机忽略目录 `.planning/2026-10-07-full-platform-rollout/existing-account-position-readback.json`，未纳入远端。此次只读核验及文档更正通过内容复核与 `git diff --check`，无行为修改，不运行Cargo；未更改快照时间、96小时门、生产库、activation或实例。
