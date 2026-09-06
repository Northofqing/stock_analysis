# 第二批：Foundation前置来源、目录与源码证据实施计划

> 执行要求：使用subagent-driven-development逐任务实施和独立复核。勾选完成只表示本批交付，不表示完整Foundation或生产就绪。

**目标：** 让后续迁移能够机器核对“正在改哪个版本的哪条生产路径、由谁推进完成状态”，并在代码、设计来源、枚举覆盖或证据漂移时失败。

**架构：** 纳管的JSON保存人工审计语义；Ruby标准库工具负责源码定位、完整哈希、枚举/引用/归属校验。规范代码基线是隔离分支07781bf，原混合源新增PaperBuy/Watchdog只记录为明确排除项，不混进本分支目录。

**技术栈：** Ruby 2.6.10标准库、JSON、Markdown、Git只读查询；不改运行时Rust、数据库、配置、传输或模板。

**已批准设计：** 原工作区docs/push-system/grill-decisions-2026-09-02.md中的Q16/Q29/Q32/Q59/Q64/Q66/Q67/Q70/Q74/Q91/Q93/Q94/Q95/Q103/Q104/Q108；push-documentation-hardening-plan.md的来源治理、目录/证据任务；comprehensive-reanalysis-2026-09-05.md的§7–§9。用户已多次确认并要求“继续”，本批不更改产品决策或晋级顺序。

**执行与测试接口：** 原设计已批准命令行文档校验seam。本批使用`ruby scripts/architecture-docs/check-sources.rb --root ROOT`和`ruby scripts/architecture-docs/check-catalog.rb --root ROOT --draft|--check`，通过真实临时目录及临时Git仓库测试成功/失败退出码与诊断。不切换进程cwd/env，不读真实运行库或发消息。

原工作区只读输入根：`/Users/zhangzhen/Desktop/Quant/stock_analysis`。执行根：其`.worktrees/push-reliability-20260905`。复用原分支`codex/push-reliability-20260905`，本批起点`07781bf386aafdf202851ae928efee8920387058`。

## Global Constraints

- 产品代码、配置、数据库、.env、运行日志及已有事故参考件不修改；原目录只维护docs入口和planning记录。所有实现/提交在已批准隔离分支内，不合并、不push、不部署。
- 不激活INACTIVE/STARVED/OPT-IN，不修改owner或Q44晋级顺序；目录中的ACTIVE仅代表源码接线，不代表制品已经部署或消息被接收。
- 九份v18/v19来源与批准决策表按原字节导入，导入后SHA逐一复核；不得改写来源以消除版本标签冲突或历史规则引用。
- 新测试仅使用各自临时目录/临时Git仓库，默认并行安全；不运行旧归档写盘测试或全仓Rust测试，不清理任何现存项目数据。
- 不使用filter_map、tally、非标准gem或网络。修改用apply_patch，文件导入必须验证原字节；不执行全仓格式化。
- 本批目录及来源保持PROVISIONAL；draft不豁免内容/SHA/symbol/覆盖漂移。严格目录检查须拒绝PROVISIONAL或脏工作区，不将局部检查冒充完整RFC/WBS/HTML/CI/生产门禁。
- 未改旧65条历史报告或原67条冻结报告；65来自本批代码enum，不写成永恒常数。四时段是Epic，MigrationUnit身份按producer/occurrence family/completion owner，而非kind一对一分配。

## 文件与接口合同

`design-source-catalog.v1.json`顶层：`schema_version:1`、`status:"PROVISIONAL"`、`provenance:"user_workspace_snapshot"`、`approved_decisions:{path,sha256,question_count:108}`、`sources:[...]`。每项包含`id,path,sha256,title,self_version,self_status,ruling,conflicts,superseded_by`；版本/状态未声明时使用null并记录原因，不编造值。`conflicts`、`superseded_by`为数组，裁决为中文；历史文档自己引用的退役规则不当作现在的规则。

`push-evidence-manifest.v1.json`顶层：`schema_version:1,status:"PROVISIONAL",baseline_commit,files:[{path,sha256}],evidence:[{id,path,symbol,kind,symbol_sha256,start_line,end_line}]`。`files`覆盖基线全部src/**/*.rs及存在的Cargo.toml/Cargo.lock，当前同范围集合必须一致，避免新文件新增producer逃过检查；允许保守地阻止无关源码漂移。`kind`只允许`rust_fn`、`rust_enum`、`rust_impl`、`rust_mod`。path为不含`..`的相对文件路径，symbol是完整声明名（函数名/枚举名/impl目标/mod名），不是行号或任意文本片段；不允许重复ID或歧义定位。文件和symbol均校验基线Git字节以及当前字节。行号由定位派生，不能单独充当证据身份。

`push-capability-catalog.v1.json`顶层：`schema_version:1,status:"PROVISIONAL",baseline_commit,scope,enum_evidence_id,kinds,producers,migration_units,excluded_worktree_additions`。kinds每项：`kind,primary_phase,status,producer_ids,evidence_ids,note`。phase固定中文盘前/集合竞价/盘中/盘后；status为ACTIVE/INACTIVE/STARVED/OPT-IN。producer每项：`id,kinds,phase_epics,occurrence_family,completion_owner,migration_unit_id,trigger,source,authority,policy,evidence_ids,known_gaps`；trigger/source/authority/policy各包含中文说明及对应evidence_ids。Unit每项：`id,producer_ids,completion_owner,occurrence_families,phase_epics,note`。没有producer的INACTIVE保留空producer_ids并有明确禁用/无caller证据与说明，不能虚构正在运行的producer。

所有非INACTIVE kind必须有source-reviewed producer；有renderer但不可达的producer保留明确不可达/缺输入语义。共享kind不强行合并producer；共享原子completion owner的producer必须放在同一Unit并解释原因。completion owner必须按实际状态标识及其occurrence/key范围区分，不能因为处于同一函数、同一状态类型或同一数据库而合并。目录是源审计候选，不自动冻结完整迁移顺序或工期。

`rust_impl`的symbol细化为去掉`impl`关键字和末尾左花括号后的完整impl头，仅折叠空白、保留泛型/trait/for/where。支持普通inherent/trait及常见泛型头；同规范化头重复时失败，不按类型名任取一个impl。不支持或不能可靠定位的形状必须明确失败；这是词法定位器而非完整Rust解析器。实际目录优先引用精确下游函数及枚举，不要求为了使用impl而扩大证据范围。

`scope`必须明确隔离代码基线、源码审计≠部署证明、完整RFC/运行时Foundation/离线HTML/CI/工期尚未完成。`excluded_worktree_additions`精确记录PaperBuy/Watchdog及“仅原混合工作树存在，本分支未移入”的原因，不给它们伪造本分支源码证据。

## Task 1: 不可变设计来源与校验接口

**文件：** 新增九份下表来源、docs/push-system/grill-decisions-2026-09-02.md、design-source-catalog.v1.json、scripts/architecture-docs/source_catalog.rb、check-sources.rb、test/source_catalog_test.rb。只更新隔离分支的docs/v19.x/README.md中退役规则/版本标签的说明，不改其他历史正文。`.gitignore`由controller精确放行，不由worker整体放开docs。

九份来源的path及导入期望SHA：

| path | SHA-256 |
| --- | --- |
| docs/v18.x/v18.1-strategic-gap-analysis.md | 7f74b5abe4c20d6be099239878f27483a52d44cfa6ac126e1c750e916567c3f2 |
| docs/v18.x/v18.2-backtest-direction.md | 946770a2bb66f1e9eeb90e87d4cd4f7b13434e1f377409ef891be84c5a5178f2 |
| docs/v18.x/v18.3-backtest-implementation.md | b63393561c17252524cbafabf322a5e4624b9e31b60a668af5b1334bc11283c2 |
| docs/v18.x/v18.4-factor-zoo-design.md | 7cf4040e11698c4f67aef54d2471d855d00cd9f31d178b7dfc1676c762977ce7 |
| docs/v18.x/v18.5-production-readiness-design.md | 7265e80311074f774ba2194c3728622bbf3e6b4f5f26499bdca8d76b7fff067f |
| docs/v19.x/push-template-catalog.md | c6e0fc8bce6d4fe668222837052425a07a6db6918c68b5fc8121efe1beced4d2 |
| docs/v19.x/v19.0-operational-clarity-design.md | da8f141e2c5aee942ea29539e80ff267dac339ca678c4fe7b764df133dc69284 |
| docs/v19.x/v19.1-review-enhancement.md | 26ca82982ebdc8c6cab9251dc00f250b57a33e5e90821db25705c87be23ad2ef |
| docs/v19.x/v19.2-ai-analysis-improvement.md | ac7b2430ee5cf043314bd37aea622fbc16e42c27e4cea6bb9e50ef787b3b99ea |

- [x] 校验原文件SHA，按原字节导入，校验目标SHA。批准表同样先后比对SHA。若apply_patch未保留末尾字节，报告实际差异，不擅自声称相同或修改原文件。
- [x] source catalog逐项抄录真实标题/声明：v18.2–v18.4标题为v20.x，v18.5为v20.0；只描述“位于v18.x且文件自声明v20的版本标签冲突”，不说文件放错。push-template-catalog是97f28b9/57-kind历史快照，不能作为当前65-kind目录。
- [x] 在公开CLI写正常临时fixture的失败测试，随后实现最小成功路径。测试必须通过Open3调用CLI，不只直接调用内部helper。fixture使用Dir.mktmpdir和File.binwrite，仅写自己拥有的临时根；source catalogue fixture可用最小一项且不硬编码项目九项数量，项目级九项校验另列。

```ruby
out, err, result = Open3.capture3(RbConfig.ruby, CHECK_SOURCES,
                                '--root', fixture_root)
assert result.success?, out + err
File.binwrite(File.join(fixture_root, 'docs/source.md'), "changed\n")
out, err, result = Open3.capture3(RbConfig.ruby, CHECK_SOURCES,
                                '--root', fixture_root)
refute result.success?
assert_includes out + err, 'source_sha_mismatch'
```

- [x] 公共实现接口为`ArchitectureDocs::SourceCatalog.validate(root)`，返回字符串错误数组；所有读路径先检查相对路径/真实路径在root内，错误包含稳定原因码和path/id。CLI成功exit0，验证失败exit1，未知参数exit2；不修改任何输入文件。
- [x] 逐个增加并验证：source缺失/字节漂移、重复ID/path、非法或越界路径、catalog schema/status不合法、缺必需字段、批准表SHA变化或108题缺失/重复（按两段表抽取完整Q1–108，不把superseded Q71删掉）。所有fixture使用108题的完整小表，再逐案制造缺失/重复；question_count不得配置成其他数量来绕过批准表覆盖。
- [x] `ruby scripts/architecture-docs/test/source_catalog_test.rb`及`ruby scripts/architecture-docs/check-sources.rb --root .`通过；项目级确认九份固定hash/批准表字节相同，所有导入路径可纳管。Ruby语法及`git diff --check`通过。
- [x] 仅提交本任务文件，报告写task-1-report.md，包含RED/GREEN命令输出、源导入前后hash、剩余未交付的整体门禁。不要运行或改写原工作区的归档、源码、索引。

## Task 2: 稳定symbol定位、目录校验和渲染工具

**拆分说明：** 原Task2的工具和真实业务审计拆成Task2→Task3顺序实施/独立复核，原验收要求全部保留。Task2不生成真实目录来填补尚未审计的业务判断。

**文件：** 新增scripts/architecture-docs/rust_evidence.rb、catalog.rb、check-catalog.rb、render-catalog.rb、test/catalog_test.rb。复用已复核Task1的SourceCatalog.validate(root)，不改变其错误数组合同。真实catalog/manifest/Markdown由Task3交付。

- [x] 建立ArchitectureDocs::RustEvidence，屏蔽普通/原始字符串、字符、行/嵌套块注释后做声明和配对花括号定位。同名歧义失败；按文件与接口合同支持rust_fn/rust_enum/rust_impl/rust_mod，完整原始item字节（声明行至闭合行，包括实际换行）产生SHA/行号。不得拿行号、相似文本或去注释后字节当身份；常见泛型/生命周期和不支持形状的失败关闭需有测试。
- [x] 先通过公开check-catalog CLI的最小临时Git仓库测试正常目录成功（实现前失败），再逐项增加负例。fixture用Dir.mktmpdir/File.binwrite和真实git -C fixture_root init/add/commit；仅当次commit的-c user.*参数，不改全局git配置、cwd或env。fixture无需真实65项，但需完整108题source校验输入。
- [x] 冻结逻辑对manifest中的文件逐一比较基线Git字节与当前字节，files集合精确覆盖基线全部src/**/*.rs及存在的Cargo.toml/Cargo.lock，当前同范围集合相同；新文件、隐藏目录文件与注释变化也不得绕过。Git基线须完整真实commit且为当前HEAD祖先，不在检查时自动刷新任何期望。
- [x] ArchitectureDocs::Catalog.validate(root, strict:)返回原因码字符串错误数组并调用SourceCatalog；验证既定schema所有字段类型/状态/引用、enum精确集合、kind→producer→Unit双向一致、共享completion-owner的Unit归属、phase合法、排除项不在enum、evidence无重复/悬挂/歧义。严格模式另拒绝PROVISIONAL和非ignored脏状态；draft仅跳过这两个条件。文件与symbol错误必须同时可诊断，不被状态检查掩盖。
- [x] 公开CLI负例覆盖enum缺失/新增/重复、catalog重复kind、缺producer/owner、共享owner拆Unit、悬挂evidence、source/源码字节漂移（含注释）、symbol改名/重复、伪造行号/哈希/无效commit/非祖先、字符串/注释中假声明、非法路径和symlink逃逸、严格PROVISIONAL/dirty及draft内容漂移。无效JSON、非法结构与未知参数明确失败；CLI未知参数exit2、验证失败exit1、有效draft exit0。
- [x] render-catalog.rb按四时段生成kind/status/producer/Unit/具体symbol证据，并单列enum外路径和原工作树未移入项。--root ROOT --check只读比对，陈旧/缺失exit1；--write仅写该根内docs/push-system/push-capability-catalog.md，不重写证据。临时根测试两次生成幂等。
- [x] 自审后完整运行catalog_test.rb与来源测试一次、新Ruby语法及新文件diff --check通过。真实根check-sources应通过；真实check-catalog在Task3尚未交付时必须明确报告catalog/manifest缺失，不为了使其通过造假目录。CLI输出明确NOTCHECKED完整RFC/WBS/离线HTML/CI/运行时Foundation/部署/真实接收。
- [x] 只提交五个工具/测试文件，不force-add ignored scratch、不提交controller plan。task-2-report.md记录RED/GREEN命令/输出、公开接口及剩余真实业务目录任务。Task2 BASE=6412b58，完成工具独立review后才进入Task3。

## Task 3: 65-kind入口与无caller清单

**文件：** 新增docs/push-system/push-source-audit-worksheet-2026-09-05.md。它是人工源码审计工作表，不是最终machine catalog；后续Task4–Task6按独立diff追加，Task7消费并重新核对。

- [x] 以07781bf的PushKind enum为集合，工作表用可机械抽取的单表逐项列65个kind且仅一次，包含初判phase/status、真实producer入口symbol或无caller裁决、owner待核状态；一条kind多producer可列多个入口。用一次性Ruby对比工作表kind集与RustEvidence.enum_variants，失败不得提交。
- [x] 对INACTIVE/no-caller集合逐kind做全src负向caller审计，排除enum声明、metadata、renderer、适配表、测试和smoke；absence需记录命令/匹配分类，优先补充明确disabled/no_producer/preflight符号作为正证据。不能因旧报告说无caller就通过。
- [x] 收录全部已识别入口面：monitor/news/P01 scheduler、P01 compensation、--push run_daily_pushes、复盘auto/manual/backfill、CLI单股/汇总/chain和09:05/15:30 chain；仅save file的run_market_review_only不是发送producer，AlertManager无production caller仅作排除。
- [x] 将前次partial task-3-report中的16项已验证源码事实重新核对后写入工作表“已证实风险/关系”，并把News/状态驱动/复盘的待核owner分别指向Task4/5/6；不得把pending写成已证实完成。
- [x] 只提交工作表；report task-3-report.md追加集合对比、负向审计命令/结果和剩余owner清单。无真实JSON/manifest/生成Markdown，不改工具或Rust。完成独立review后才进入Task4。

## Task 4: 新闻、来源事实与持久发送边界审计

**文件：** 仅追加push-source-audit-worksheet-2026-09-05.md的新闻审计章节，不改Task3的65-kind集合行。

- [x] 完整核对Announcement、PreopenNewsHot/P01 scheduler+compensation、NewsToIdea普通D01与NewsAI、NewsCatalyst、NewsFlashCritical/Aggregated、PolicyHit、EarningsBeat/Miss、AnalystUpgrade，以及N01/N02的source/trigger/authority/policy/occurrence/completion owner。
- [x] NewsAI从admitted same-tick facts经assess/preflight/send到durable occurrence/finalize；区分本地audit、sink attempt、TransportAccepted，不从日志文案升级authority。N01/N02区分共享quota和各occurrence/settle owner。
- [x] P01自动与补偿是否复用同一schedule occurrence/claim按源码裁决；D01 smoke fixture排除。Earnings opt-in位于provider I/O之后、AnalystState observe先于发送等顺序要有直接symbol证据。
- [x] 每项写明确owner标识/key范围、失败是否推进/回滚/重试和多入口归属建议；无法证明的点保留具体未决，不用概括文案。提交仅工作表追加，report task-4-report.md，独立review后进入Task5。

## Task 5: 状态驱动、盘中/竞价及交易相关边界审计

**文件：** 仅追加同一工作表的状态驱动章节。

- [x] 完整核对DataMode pending/retry/confirm、HoldingPlan、T0Advice、CloseCall的counted binding与外层timer；VirtualWatch剩余路径；IntradayMarket三个producer；MarketActionAlert两入口；PaperSell盘中/盘后；CandidateBoard/CandidateInvalidated/AuctionRepush；SectorTop/SectorAnomaly；大宗/ST/ETF及其他盘中/集合竞价producer。
- [x] 对每个producer记录真实触发、source、authority、policy、occurrence与completion owner标识/key；区分业务状态落库、发送结果、外层闸门和子快照。共享函数/DB/类型不自动合并Unit，共享实际原子owner不得拆。
- [x] 明确已证实风险：PaperSell先成交后通知且共用code/day Filled；候选失效bool/快照推进及空集缺失；预检窗口结构不可达；sector独立timer；AccountMode Frozen副推不受主完成列证明。提交仅工作表追加，report task-5-report.md，独立review后进入Task6。

## Task 6: 复盘、补推及side-route边界审计

**文件：** 仅追加同一工作表的复盘章节。

- [x] 对13个ReviewTask逐项核对dependency、source/dispatcher/policy、自动ReviewScheduleState(date,task)、manual临时audit state、backfill durable claim/retry作用域；不同入口如共享真实decision identity需说明，不能按同一struct合并。
- [x] 核对R08首批新GatewayError永久/重试分类与旧二进制兼容限制；RejectedDurable授权重试、Uncertain不盲重发；NoData/Disabled/permanent Failed/ExpectedWait各自对终态的影响。
- [x] 核对block trade与IPO side route、LHB/chain/单股/汇总CLI边界；side route不是ReviewTask时不借用其完成状态，只有保存文件的路径排除。
- [x] 提交仅工作表追加，report task-6-report.md，包含每个Rxx入口/owner及无法证实项。独立review后进入Task7。

## Task 7: 真实目录、证据manifest与中文生成视图

**文件：** 新增docs/push-system/push-capability-catalog.v1.json、push-evidence-manifest.v1.json、push-capability-catalog.md。使用已复核Task3–Task6工作表，但仍对每条最终关系及symbol做源码核对。

- [x] 使用既定schema覆盖真实65-kind精确集合；所有非INACTIVE有source-reviewed producer，INACTIVE空producer列表有明确禁用/无caller证据。收录enum外CLI单股/汇总/chain、09:05/15:30 chain及`--replay-force`，PaperBuy/Watchdog只作未移入排除；Q54运维webhook明确排除出业务回执目录并保留独立审计待办。
- [x] 逐producer填trigger/source/authority/policy/evidence、occurrence/completion owner，并按实际共享owner形成MigrationUnit和四时段Epics；工作表中的pending不得进入成品。目录是候选，不冻结完整迁移顺序/工期。
- [x] manifest冻结07781bf全部467个src Rust文件和Cargo.toml/Cargo.lock，使用已复核locator产生原始item SHA及派生行号。基线/当前字节、集合、symbol、enum和引用全部由工具核对，不自动刷新。
- [x] 明确运行source check、真实draft、严格检查、一次Markdown --write、--check和二次生成幂等；strict只剩PROVISIONAL/dirty，不能隐藏内容错误。记录真实时延及kind/producer/Unit/evidence/file准确数量。
- [x] 三个成品及后续审查修订均按精确范围提交，report task-7-report.md。保持PROVISIONAL；不写Foundation Ready、部署/TransportAccepted/用户已读、完整RFC/WBS/HTML/CI或未经精确WBS的工期。独立审查发现并关闭普通启动恢复、`--replay-force`和词法定位边界遗漏；最终限定复核无未关闭Critical/Important/Minor。

## 整批验证和交付

**完成状态：** 本批来源、目录、工具、生成视图与独立双轴审查已完成；最终HEAD为`767a76e7deeeca4022133c940d13ba4930789529`。目录保持PROVISIONAL，严格检查仅因两个JSON的发布状态失败，不是内容失败。运行时Foundation、完整RFC/WBS、离线HTML、CI、部署、真实接收和全套Rust测试仍不属于本批完成范围。

源码始终保持07781bf。运行两个Ruby测试文件、来源校验、目录draft、Markdown freshness、两次生成幂等、所有新Ruby语法、git diff --check；严格失败必须明确为PROVISIONAL/dirty而非掩盖内容错误。全套运行时Rust测试、全量RFC/WBS/离线HTML/CI不在本批验收内，不复用首批绿灯宣称它们完成。

导入核对后的格式例外：v18.4来源第219/278行、v18.5来源第109/138/142/433行原有行尾空格，保留原字节和已批准SHA；整批裸range `git diff --check`会保留这六处诊断，其他文件仍须零格式错误，不更改全局whitespace配置。v19.0原文无末尾LF；apply_patch导入时多加的唯一LF经原目标字节断言后机械移除，原文件保持只读，目标SHA重新一致。

中文结果写docs/push-system/implementation-batch-2-results-2026-09-05.md，原目录implementation-status入口更新。本批按Task1→Task7顺序实施/分别review，最后对07781bf以来整批review。保留来源及先前事故参考件，分支/worktree不清理、不自动合并。
