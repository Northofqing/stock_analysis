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

## Task 3: 65-kind真实源审计目录与证据交付

**文件：** 新增docs/push-system/push-capability-catalog.v1.json、push-evidence-manifest.v1.json、push-capability-catalog.md。使用Task2已经独立复核的CLI/locator，原则上不改工具接口；如发现真实阻点，向controller报告具体证据，不改Rust源码迎合工具或目录。

输入：本分支src/bin/monitor/notify.rs真实enum及生产源码。原工作区all-push-kinds-2026-09-05.md、comprehensive-reanalysis-2026-09-05.md、蓝图§24只提供审计线索；必须重新查本分支07781bf代码，不能把混合源行号或历史逻辑当已验证事实。首批R08/G5b修复以当前代码为准。

- [ ] 使用既定schema人工审计所有实际enum项，kind集与enum精确相等；本基线预期65，校验器不能写死65。phase/status根据真实可达性与禁用/缺源/opt-in条件，不由level/展示注册表推断ACTIVE；没有producer的INACTIVE使用空列表和明确禁用/无caller证据。
- [ ] 逐producer记录trigger、source、authority、policy、occurrence及实际completion owner，各关系关联真实symbol。强制核查NewsToIdea普通D01与NewsAI；IntradayMarket盘中概览/09:10不可达预检/15:05持仓过期；MarketActionAlert账户冻结/OrderUpdate；PaperSell盘中/盘后；CandidateBoard/CandidateInvalidated/AuctionRepush共享外层tick及子快照；SectorTop/SectorAnomaly各自timer；复盘、大宗及业绩家族。不能用一个main函数代替全部dispatch/source/finalize证据。
- [ ] 收录enum外CLI单股/汇总、产业链CLI及09:05/15:30定时报表。producer的kinds可为空但需说明原因；无生产caller的AlertManager只作排除说明。原工作树PaperBuy/Watchdog仅记录未移入，不伪造本分支证据。
- [ ] 按producer/occurrence/completion owner构建MigrationUnit及phase Epics，共享实际owner的producer不可拆分；同函数/同状态类型/同数据库不等于共享owner。复盘自动/手动/历史补推等入口按真实状态作用域说明；无独立通知游标的CLI明示缺失，不能虚构新authority。
- [ ] evidence manifest冻结07781bf386aafdf202851ae928efee8920387058的全部src Rust文件与Cargo元数据集合；使用已复核locator产生原始item哈希和派生行号，当前字节必须一致。不得手填虚假行号、盲取同名首项、在检查时自动刷新期望；人工语义审计与机器结构一致性检查明确区分。
- [ ] 通过现有工具验证真实来源和目录draft；严格检查只能剩PROVISIONAL/dirty而非隐藏内容错误。明确生成一次Markdown，再执行--check和二次生成幂等；Markdown须能看到四时段、逐producer具体业务关系及symbol证据、enum外路径、已知风险和范围限制，不仅是kind名称表。
- [ ] 只提交三个业务目录/证据/Markdown文件。task-3-report.md报告准确kind/producer/Unit/evidence/file数量、源码审计证据、CLI命令/输出、无法证实的具体关系。保持PROVISIONAL，不新增工期总数或Foundation Ready标签，不将源码接线当部署/TransportAccepted/用户已读证明。
- [ ] 若某条逻辑、owner或lexer定位不能在此基线证实，报告具体阻点由controller处理，不用概括性占位文案宣布全量已核验。完成独立spec+quality review后进入整批验证与最终review。

## 整批验证和交付

源码始终保持07781bf。运行两个Ruby测试文件、来源校验、目录draft、Markdown freshness、两次生成幂等、所有新Ruby语法、git diff --check；严格失败必须明确为PROVISIONAL/dirty而非掩盖内容错误。全套运行时Rust测试、全量RFC/WBS/离线HTML/CI不在本批验收内，不复用首批绿灯宣称它们完成。

导入核对后的格式例外：v18.4来源第219/278行、v18.5来源第109/138/142/433行原有行尾空格，保留原字节和已批准SHA；整批裸range `git diff --check`会保留这六处诊断，其他文件仍须零格式错误，不更改全局whitespace配置。v19.0原文无末尾LF；apply_patch导入时多加的唯一LF经原目标字节断言后机械移除，原文件保持只读，目标SHA重新一致。

中文结果写docs/push-system/implementation-batch-2-results-2026-09-05.md，原目录implementation-status入口更新。本批按Task1→Task2→Task3顺序实施/分别review，最后对07781bf以来整批review。保留来源及先前事故参考件，分支/worktree不清理、不自动合并。
