# 统一文档校验入口与CI接线

日期：2026-09-09。状态：本批产品实现、验证及限定复审完成，首版1a10586、最终源码7a150b2。BASE=`aef7972965f610ed418049593dfff1d55341772e`。实际记录见[中文实施结果](../../push-system/implementation-unified-document-checker-2026-09-09.md)；首次tracer未取得事前RED的过程偏差作为一次明确例外保留，不修改第9项历史要求或冒称全过程合规。当前审计/两目标/真实CI与整体迁移仍未完成。

本计划落实已批准Q64/Q92/Q105/Q106及原文档硬化任务6的统一执行入口。当前分支仍有107项Catalog内容漂移，蓝图current及第二HTML尚未交付；本入口的实现/夹具测试通过不等于原任务6验收完成，更不等于完整推送迁移完成。既定最终要求仍是实际当前审计对齐后draft通过、strict仅保留有证据的发布阻断、两份HTML新鲜度及真实CI执行证据。

## Global Constraints

- 仅在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`、分支 `codex/push-reliability-20260905` 开发。禁止修改根工作树、Rust、Cargo、config、生产monitor、数据库、provider/LLM/sink/订单、.env、部署或远端Git状态。
- 不修改规范RFC/SQL、八份冻结输入/manifest、v18/v19原始来源、既有catalog/evidence JSON、WBS或其生成正文；不改变既有Catalog/RfcSpec/RfcInputs/HtmlBuilder的验证规则或固定资源。新代码不得过滤107项漂移、将其降为PROVISIONAL或仅查旧Git来假装当前通过。
- Ruby2.6标准库；不引入gem/npm/联网安装。正式checker完全只读，不生成/修复HTML、Markdown、manifest或Git索引，不执行生产脚本、Rust、消息/图表源内容。仅测试代码可在自有临时根建立/修改Git及文档夹具；不能改用户Git配置、分支、索引或refs。
- 一个实施agent独占本Task代码/测试/CI和实施报告，父线负责计划/证据/Git提交/中文交付。全部手写文件用apply_patch；只运行本批明确限定的Ruby测试，不跑Cargo或无关历史全套，不再派agent。
- CI修改只发生在本地隔离分支，不推送/触发远端workflow；保留现有触发、权限及Rust步骤。是否通过真实CI与声明是否接线是两个独立事实。保留SDD和临时取证资料，不删除交接证据。

## Task 1: 可直接执行且不隐瞒失败的统一门禁

### 输入、文件和接口

- 新建 `scripts/architecture-docs/check.rb`：薄CLI及校验组合，不新建通用插件框架或第二套schema实现。
- 新建 `scripts/architecture-docs/test/check_test.rb`：通过真实子进程CLI验证组合行为及CI声明；若夹具准备确实需要独立文件，可新增唯一 `scripts/architecture-docs/test/support/document_check_fixture.rb`，不能require带minitest/autorun的既有测试文件作为helper。
- 修改 `.github/workflows/ci.yml`：增加明确strict步骤和获取历史基线对象所必需的checkout配置。
- 性能证据修订后允许修改 `scripts/architecture-docs/rust_evidence.rb`、`scripts/architecture-docs/catalog.rb` 和已有 `scripts/architecture-docs/test/catalog_test.rb`：仅做下节定义的同语义重复工作消除，不改变冻结材料、公开校验结论或历史证据解释。
- 父线后续新增中文结果并更新README；不由实施者修改父线文档。

CLI：`ruby scripts/architecture-docs/check.rb --draft|--check [--root ROOT]`。恰好一个模式；root缺省按脚本仓库位置确定，不依赖调用者cwd。`--help`单独调用exit0；缺模式、未知参数、重复模式/重复root、互斥参数或额外位置参数exit2，使用说明写stderr；不静默接受缩写选项。内容/缺失/漂移/路径/发布门禁失败exit1，诊断写stdout；成功exit0。诊断保留原稳定reason code及path/id信息并去重，不把详细失败压成通用PASS/FAIL，不输出环境、秘密或无关stacktrace。

### 组合顺序与只读边界

1. 先验证root为合法目录，拒绝根symlink；缺失/普通文件root作为内容失败，不崩溃。
2. `RfcInputs.validate(root)`独立先执行。`RfcSpec`依赖失败可能提前返回，不能因此漏掉八输入检查。
3. `Catalog.validate(root, strict: mode == :check)`，其中包含SourceCatalog、真实Git baseline、当前代码文件集/字节、符号/枚举/引用。不能require四个旧CLI或通过退出码为0的mock代替组合。
4. Catalog无内容错误时，读取既有固定pair并以 `Catalog.render` 验证 `docs/push-system/push-capability-catalog.md` 原字节新鲜度。通过现有安全读取边界拒绝文件及父目录symlink/hardlink，缺失/陈旧用稳定错误码；不写入。两个既有catalog/manifest的精确 `provisional path=...` 及 `worktree_dirty` 只是发布状态，不能阻止strict检查Markdown；它们仍全部保留在最终错误中。如果Catalog内容失败，派生渲染可不执行，但不得掩盖独立RFC/HTML错误，不要为了判断内容是否有效再重复执行整套Catalog。
5. `RfcSpec.validate(root, strict: mode == :check)`，保留其SQL/决策/类型/状态合同、WBS数据及嵌入新鲜度检查。不要调用 `Wbs.validate(... freshness: false)`。
6. 两模式都必须调用 `HtmlBuilder.check(root, 'rfc')`；捕获其Invalid作为内容错误，绝不调用普通构建修复。此时builder实际仅注册rfc；结果明确报告 `html_targets=rfc`，不声称蓝图已检查。第二目标交付后须扩展该公共门禁；此依赖仍列为原任务6/7未完成项。
7. 内容错误与发布错误共同返回；一个组件失败不能使其他独立组件不运行。strict不能因工作树干净或HTML已存在而通过PROVISIONAL，不能提供忽略error/绕过baseline的CLI选项。
8. 既有Catalog会启动只读Git命令；在checker子进程内设置 `GIT_OPTIONAL_LOCKS=0` 防止status可选索引刷新，不改变全局配置或父进程环境。验证前后文件/HTML/源文/manifest及Git索引的字节和mtime不变；不得声称checker没有子进程。

### 真实集成fixture与TDD

9. 第一条tracer先写测试：统一CLI尚不存在时，真实合法临时根执行 `--draft` 应exit0并报告 `architecture_docs_valid` 与 `html_targets=rfc`；实际RED后才实现组合。每轮一项可观察行为，不一次写完整假想测试再实现。
10. 正例必须让真实Catalog、RfcInputs、RfcSpec、WBS和HtmlBuilder同时运行。使用本地已有 `07781bf386aafdf202851ae928efee8920387058` Git对象及该基线完整src/Cargo源码，与当前正式RFC/SQL/目录/WBS/八输入/来源文件/脚本/模板/本地资产配套；通过正式build生成RFC HTML和正式renderer生成目录Markdown。fixture只复制固定规范材料、八输入manifest声明路径及SourceCatalog来源/decisions组成的真实依赖闭包，不复制/纳管整个docs或无关未来计划；同一测试进程复用一次建好的完整基准，每个反例使用隔离副本。可以用自有临时Git根与本地只读clone/归档，不联网、不修改源仓库、不以一个简化enum冒充65-kind集成正例。清楚标明此fixture是历史代码对齐正例，不能替代实际当前分支验收。
11. 同一夹具族逐项验证：kind缺失/重复；来源字节SHA漂移；symbol字节/派生行号或缺失；Q决策行缺失；W01--W21缺项及单独WBS正文陈旧；目录Markdown缺失/陈旧；HTML缺失/字节篡改、模板/实现/固定资产漂移；八输入与RFC依赖同时损坏仍分别报告。断言具体对应错误码，不能仅断言任意非零使无关failure冒充覆盖。
12. WBS-stale案例在修改RFC嵌入段后重新用正式build生成最新HTML，再运行checker，证明WBS错误独立于HTML陈旧。HTML-stale案例则保留其他输入合法；父目录/文件链接失败与missing/malformed/unknown CLI行为也应有正式入口回归。
13. strict在干净、内容对齐、CI已接线的自有Git fixture中仍exit1，只包含既有两个catalog PROVISIONAL及rfc/wbs状态错误；再叠加内容漂移时必须同时保留内容错误，特别覆盖仅目录Markdown陈旧仍报其具体错误，不能被PROVISIONAL遮住。dirty fixture额外得到worktree_dirty。不得把strict全通过作为本临时版本的预期。
14. 无root的正式脚本在另一cwd执行，仍定位其自身fixture仓库，而非调用目录；需要复制同版实际scripts到临时根来执行，不能改常量/mockroot。所有检查模式均有文件和索引前后不变的断言。

### CI与最终本批验证

15. 在现有Checkout步骤为 `actions/checkout@v4` 增加 `with: fetch-depth: 0`，保证真实历史baseline对象可用；不增加网络安装步骤。Rust安装/格式步骤前新增独立run：`ruby scripts/architecture-docs/check.rb --check`。不使用条件绕过、continue-on-error、命令拼接、echo或额外参数替代该精确合同。
16. 验证实际本地ci.yml含唯一目标步骤、顺序在Rust检查前、checkout历史深度，现有触发与其他步骤保持。真实 `RfcSpec.ci_rfc_gate?` 对实际文件可识别；既有 `rfc_spec_test.rb` 的相关CI定向用例须继续通过，不必重跑无关所有规范测试。
17. 实施者运行新 `ruby scripts/architecture-docs/test/check_test.rb` 一次最终合批、所有新增Ruby语法检查、相关CI定向回归及限定diff检查；保存真实命令、终态输出和TDD记录。父线读取后复用未变证据，不因只提交docs重跑相同套件。若代码仍有改动则补覆盖变更的验证；对于已跑合批中的测试夹具修正，明确保留原通过项并定向补验受影响行为，不能将这种分段证据说成修正后完整套件重新通过。若夹具依赖闭包收窄，需用真实组合、strict只读、综合漂移与WBS独立陈旧检测直接验证新夹具仍满足合同。
18. 父线在实际当前隔离树执行正式 `check.rb --draft` 和 `--check`，记录实际全部内容错误及发布错误，确认与已知Catalog漂移一致并定位任何新增错误；当它们仍非零时，原硬化任务6“当前draft通过/strict只剩发布阻断”保持未完成。独立固定BASE..SOURCE任务审查覆盖本入口、测试与实际CI diff，不做整个分支重复审查。

### 实测性能问题及必要修订

首条真实GREEN已取得：76060终态，1 run / 7 assertions / 0 failures，408.770351s。一次正式render-catalog和一次完整Catalog扫描占主要时间。主线只读抽样显示push_templates.rs共67条证据，单次mask约0.523s、locate约0.550s；main.rs共32条，单次mask约0.318s、locate约0.311s。每条证据重做整文件mask，历史/当前又各做一遍；另有469次独立git show，单次抽样约0.054–0.075s。这是实现重复工作，不是必须牺牲的证据覆盖。

本机Git实测2.50.1。批量协议依据[Git 2.50.0 cat-file手册](https://git-scm.com/docs/git-cat-file/2.50.0#_batch_output)的对象顺序/字节长度帧，以及[ls-tree手册](https://git-scm.com/docs/git-ls-tree#_output_format)的NUL分隔原始路径；只使用普通批量读取，不启用内容转换、过滤或跟随symlink。该文档事实不是新实现已通过的证据。

19. 为同一验证轮的同一份不可变Rust源字节建立可复用的词法视图，只做一次mask供多symbol定位使用。保留既有 `RustEvidence.locate(source, symbol, kind)` 的接口、错误优先顺序、原始字节SHA、CRLF/Unicode/代码注释处理、行号及完整impl匹配语义；新增视图可以是源字节绑定的小对象，不引入全局/跨运行缓存、基于mtime的结果缓存或暴露给调用方的跳检选项。当前文件的SHA及symbol应来自同一份捕获字节；缺失/损坏/重复声明仍逐项得到原错误。
20. Catalog历史blob读取改为一个真实Git批量读取过程，仍验证每个manifest文件及完整历史文件集。可先以 `git ls-tree -r -z` 取得path→blob object ID，再只用已验证的hex object ID向 `git cat-file --batch` 发请求，按真实长度解析blob帧；不得把含换行的路径直接拼成批协议。必须验证对象类型/ID/长度/终止符及命令状态，缺失、截断、类型错误均失败，不能遗漏文件或把异常变成空成功。无跨调用缓存，不写Git/源码，不依赖Git新版本的非标准选项。
21. 逐项TDD验证复用后的正式CLI多symbol/同文件、当前字节变化后仍报漂移、baseline与当前独立损坏、duplicate/missing/多种locator及CRLF原字节语义、文件名含空格/换行的真实临时Git案例。必须保持既有Catalog回归，不能以测试spy统计mask调用次数替代语义验证；性能用相同真实首条组合场景的实测前后时间报告，不设置依赖机器负载的硬秒数断言。批量Git读取若引入解析器，其拒绝缺帧/截断/非blob的验证也要有具体证据。
22. 允许同一实施者在本Task内完成上述两项局部优化；不并行派第二个实施者编辑校验库。参数边界/CI检查可先继续，性能修订完成前不启动会重复数分钟扫描的长套件。最终本批验证增加受影响的 `catalog_test.rb`；其他未变RFC/HTML套件按既有范围复用，不跑Rust。当前107条内容漂移必须保留，不以提速为由降级。

### 后续依赖与回滚

独立current审计版本必须保留旧规范catalog、manifest、RFC/WBS和运行时注册表原字节，只承担当前代码事实审计；真正消除107项漂移需当前源码语义核对，不能只机械刷新hash。蓝图第二HTML/兼容wrapper、两目标统一检查和真实CI运行仍继续。该本地代码可通过后续反向提交回滚；不删除旧证据或历史产物，不把回滚解释为生产授权。
