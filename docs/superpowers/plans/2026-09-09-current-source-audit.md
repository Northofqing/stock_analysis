# 历史规范与强制当前源码审计分层

日期：2026-09-09。状态：待前置统一checker源码验收后实施；本文不是current目录或当前draft已通过的证明。

目标：履行已批准Q59/Q64/Q65/Q66/Q91/Q92/Q93/Q95/Q105，以独立机器审计版本准确覆盖当前源码，同时保持RFC、WBS和运行时注册表原有规范身份。不能通过关闭旧目录的当前检查而留下没有current检查的空窗。

规格来源为 `docs/push-system/grill-decisions-2026-09-02.md`；原硬化计划任务3/4/6仍保留。当前增量依据为 `docs/push-system/current-code-audit-delta-2026-09-09.md`，其中107条原始漂移的归类及候选声明已完成，不重复开展同一调查。

## Global Constraints

- 只在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905` 的 `codex/push-reliability-20260905` 分支开发；不修改根工作树、Rust、Cargo、config、生产monitor、数据库、provider/LLM/sink/订单、认证、部署或远端Git。
- 不修改八份冻结输入/manifest、九份v18/v19来源、已批准决策、历史catalog/evidence JSON、RFC/SQL、WBS及其嵌入摘要。历史路径、SHA、65 kinds / 102 producers / 52 Units及运行时include_bytes身份保持原字节；本批不批准Unit晋级、恢复STARVED或激活OPT-IN/INACTIVE。
- current是当前源码事实审计，不是新运行时目录、RFC权威或生产迁移证书。所有新审计产物保持PROVISIONAL；不得为使strict全绿改动状态规则。
- Ruby2.6标准库、真实本地Git对象、正式RustEvidence定位器；不增加依赖、网络、全局/跨轮缓存或历史-only跳检选项。父线负责计划/docs/Git和验收，单一新实施者独占本Task源码/测试/机器产物；前置checker写入者冻结并完成审查后才能接手共享文件。
- 正式检查完全只读，保留文件及索引字节/mtime；只有显式生成命令能写其固定的current派生产物。测试只在自有临时Git根改动fixture；保存证据，不清理用户或历史交接材料。

## Task 1: 原子交付两层验证及完整current审计

### 文件与公共接口

- 新建 `docs/push-system/push-current-capability-catalog.v1.json`：以固定历史kind/producer/Unit身份为沿革，记录当前证据关系及独立architecture分组，不改变迁移身份或权威。
- 新建 `docs/push-system/push-current-evidence-manifest.v1.json`：绑定current catalog原始字节、历史catalog及manifest原始字节，以及完整当前源码文件/evidence。
- 新建 `docs/push-system/push-current-capability-catalog.md`：由current机器材料生成，显式区分“当前源码审计”与“历史实施规范”，含四时段业务目录及非迁移架构证据。
- 修改 `scripts/architecture-docs/catalog.rb`、`render-catalog.rb`、`check.rb` 及相应 `test/catalog_test.rb`、`test/check_test.rb`、`test/support/document_check_fixture.rb`。如当前schema/渲染内容需要独立内聚实现，可新增唯一 `scripts/architecture-docs/current_audit.rb`；不能引入任意版本/插件注册框架或复制整套旧校验器。
- 既有 `Catalog.validate(root, strict:)`、`check-catalog.rb --draft|--check` 和统一 `check.rb --draft|--check` 的公共使用保持兼容。内部强制组合历史与current检查；不能要求调用者自行选择其中一份。
- `render-catalog.rb` 既有默认历史输出与check行为保持；新增明确的 `--current` 选择固定current输出，不接受任意输出路径。统一checker独立检查两份目录Markdown的新鲜度，遇到任一内容/状态错误不得隐去其它独立错误。

### 固定版本、沿革及语义覆盖

1. current源码pin选用已存在的 `aef7972965f610ed418049593dfff1d55341772e`；实施前重验该commit为当前HEAD祖先，且完整 `src/**/*.rs`（含隐藏路径）、Cargo.toml、Cargo.lock原字节集合与工作树相同。后续仅docs/tools提交不应要求pin等于HEAD；若Rust/Cargo实际已变，不可悄悄刷新pin或hash，先解释新增变化并更新相应审计。
2. 两份current JSON明确schema version、PROVISIONAL及current-source-audit角色。current manifest按原始字节SHA绑定current catalog和两份历史JSON，并校验固定相对路径；解析后JSON等价不能替代字节绑定。历史文件被改动，即使JSON意义不变也失败。
3. 当前源码已核对548文件（历史469+新增79，新增46生产/33测试）；生成时仍从实际Git tree和工作树分别枚举完整集合，不把这些数字写成通用校验器对任意fixture的固定数量。所有supporting和测试文件必须有文件SHA，不能用代表性symbol数量代替文件覆盖。
4. 当前kind/producer/Unit沿用已批准65/102/52身份集合和双向owner/occurrence/phase关系。ACTIVE/INACTIVE/STARVED/OPT-IN、PaperBuy/Watchdog排除、四时段Epic及52个原子Unit不得因发现Foundation模块而自动改变；本批没有新产品激活授权。
5. 107条历史对当前漂移不能仅批量替换hash：八项正文变化需落实新的函数主体及真实依赖关系；73条正文不变仅派生行号；18份文件变化与79新增文件进入完整manifest。复用已经完成的增量审计与两个候选JSON，但生成前用正式locator复核全部当前evidence。
6. 9个既有文件新增声明与29个新模块代表声明是候选，不是最终evidence数量上限。对八项实质变化，薄wrapper之外的新prepare/execute/shared helper如承载审计结论，也须加入必要symbol及明确依赖；禁止用wrapper哈希声称验证其整个调用链。
7. 新增独立architecture引用域，至少记录分组身份、代表性evidence、supporting文件和明确能力边界；不把它并入producer/Unit反向关系。分组的代表性引用必须独立满足，producer的trigger/source/authority/policy及evidence_ids闭包也必须独立满足，不能将两者列表取并集来掩盖任一方缺项；未引用、重复ID/locator、丢失文件或未知引用均失败。同一声明如果确实被两个范围使用，可以引用同一evidence，但必须说明各自证据含义，不能复制相同locator制造两份伪独立证据，更不能因此提升Unit状态。
8. calendar/Foundation schema/acquisition readers等基础设施与真实auction-source/N02读取边分别表述。保留production固定拒绝、两个effect仅cfg(test)绑定、startup仅测试入口以及未找到生产caller等负向证据；源码声明、测试通过和实际生产可达必须分开。
9. 不扩张RustEvidence的四种kind，也不将无主体 `mod name;`、trait、struct或pub use伪装成可定位函数。新解析视图/Git批读使用前置任务验收后的正式实现，不重做同一性能优化。

### 强制双层验证与错误合同

10. 历史pair按固定历史Git tree验证完整文件、符号、enum、关系及baseline；current pair按自身精确Git tree和当前工作树独立验证相同类别的完整性。历史schema/reference错误不能使current不运行，current缺失/损坏也不能遮住历史/SourceCatalog错误。
11. 原先“历史pair同时核对workspace”的删除，必须与“current在同一公共入口强制存在并验证workspace”的实现、材料及测试同批交付；不得先提交历史-only通过状态、稍后再补current。两份current JSON之一缺失时，draft和strict均失败。
12. 保留原稳定reason code，并为可能冲突的历史/current诊断增加明确pair/origin标识；同一ID在两域失败时不能被uniq吞成一条。两个独立域的错误必须分别可定位到文件或evidence。
13. strict保留历史两项provisional、RFC/WBS provisional、工作树dirty，以及新current各自真实的provisional；内容错误不降级为发布阻断。current内容对齐的最终当前draft应通过；strict只剩有证据的发布状态，不能残留hash、文件集、symbol、引用或新鲜度内容错误。
14. SourceCatalog及Q1–Q108校验独立保留；RfcSpec/WBS继续读取历史规范，不给RFC metadata新增current字段、不替换固定SHA或Unit快照。新current正常演进不应迫使旧规范重生成。
15. 目录Markdown freshness基于各自固定输入和正式renderer。仅PROVISIONAL/worktree_dirty不能阻止对应Markdown检查；任一pair内容无效时可不做该pair派生比较，但独立有效pair、RFC/WBS和HTML继续检查。

### TDD与最终验收

16. 第一条真实行为反例为两提交Git fixture：历史pair只匹配B；工作树已演进至C；current pair匹配C commit及workspace。旧实现因用历史pair核对workspace而失败；新实现通过。同一fixture随后分别移除两份current材料均失败，证明没有历史-only空窗；不能以mock Catalog成功替代。
17. 在同一fixture族逐项验证：历史与current同时损坏分别报告；current pin有效祖先且仅docs提交继续通过；源码/Cargo/隐藏文件新增或漂移失败；两层baseline/hash/symbol/行号错误继续拒绝；current自身及历史原字节绑定被空白改动打破；非法JSON/schema、链接、缺文件、不存在或非祖先commit失败。
18. architecture域正/反例需覆盖独立声明合法引用、supporting文件闭包、无效引用、删除唯一引用、把必需业务引用只留在architecture列表仍失败，以及重复locator；producer/Unit/kind集合和状态不变。保持旧bodyless/Unicode/CRLF/duplicate/路径及批协议拒绝语义。
19. 真实RFC/WBS兼容断言覆盖：新增current不影响旧规范通过；将RFC旧metadata替成current值失败；旧catalog被改动且仅同步整体SHA仍不能绕过Unit snapshot。无需重跑未改动全部Rust/HTML浏览器套件；具体定向Ruby用例在实施报告列明。
20. 更新统一checker真实fixture，必须同时含历史B和当前C；分别current缺失、漂移及current Markdown陈旧时，总入口失败，两个模式都不跳过；保留此前RFC/HTML/CI/只读断言。新增current不改变当前只有rfc HTML target的事实，第二target留下一任务。
21. 实施者完成受影响Catalog/checker套件、renderer定向回归、RFC/WBS历史兼容定向测试、Ruby语法和限定diff检查，并记录准确命令/终态与TDD证据。源码冻结后父线复用同版结果，不重复跑同套件。
22. 父线必须在真实隔离树运行 `ruby scripts/architecture-docs/check.rb --draft` 与 `--check`，记录内容错误清零或具体失败；另验证历史冻结输入及Rust/Cargo与本Task基线原字节未改。仅历史对齐fixture通过不满足该项。
23. 对固定Task BASE..SOURCE做一次独立Spec/Quality审查；修复经原实施者及定向复审。父线在docs/push-system记录current材料hash、源commit、真实命令/错误、语义边界和剩余项，并更新README；不以本Task完成关闭全目标。

## 后续依赖与边界

当前架构蓝图消费本批强制current目录/证据及已完成的增量审计，以新路径单独保存，不覆盖八份冻结输入。蓝图第二HTML、旧renderer wrapper、两目标统一freshness与实际CI/发布证据随后交付；完整W15/W16/W17/W19等运行控制面、52 Unit迁移/shadow/切换/回滚及生产门禁仍保留原目标。

回滚只允许将本批源码/新增审计制品作为同一逻辑单元反向提交，不单独保留历史-only入口，不删除历史材料或生产状态。远端CI、实际部署及生产验证仍需对应权限和外部证据，本计划不新增这些权限。
