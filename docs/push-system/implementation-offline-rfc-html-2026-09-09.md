# RFC 离线 HTML：实施记录

日期：2026-09-09。状态：本批RFC离线HTML实现、最终测试和独立限定复审均完成；不代表整体改造、双份蓝图/RFC产物或CI完成。BASE=`eb19ee7a11100e40d93affc81f0ef2d1b144b4bd`，首版源码=`ca2708d43201fbddad8bf0ad862d596fc10a6011`，首轮修复源码=`1d1a38923d5786f47a49894c672089d8d4d4b326`，最终源码=`ca1b5814111f4220a395c403496b3084aab4a268`。具体合同见[实施计划](../superpowers/plans/2026-09-09-offline-rfc-html.md)。

## 本批目标

从不变的规范 `push-system-implementation-rfc.md`、独立模板和固定本地资源生成单文件RFC HTML；保留中文正文、代码/表格、完整Markdown原字节及来源hash，提供搜索、主题、折叠、打印和图表能力。生成状态保持PROVISIONAL，`--check`只读检测缺失/陈旧，`--draft`不跳过内容/路径/资产校验。

既定RFC输出为 `docs/push-system/push-system-implementation-rfc.html`。八份输入里的蓝图MD/HTML均保持原字节，不覆盖原文件。两份HTML总目标保留；蓝图current化/派生路径、兼容wrapper与统一checker/CI尚待后续接线，不因当前目标集合只有rfc而宣称全部完成。

## 已取得的基础证据

- 既有RfcInputs纯临时测试97514：12 runs / 238 assertions / 0 failures / 0 errors / 0 skips，8.493819s。该基线不是新builder通过。
- 实际Ruby2.6.10；Node22.11.0带原生WebSocket。新代码须保持Ruby标准库，不引入gem/npm运行依赖。
- 官方 [Mermaid安装说明](https://mermaid.js.org/intro/index.html)与[固定11.17.2官方包](https://registry.npmjs.org/mermaid/-/mermaid-11.17.2.tgz)作为资源来源。下载包17796365 bytes，经官方发布元数据的SHA512 integrity核验；仅解包脚本/许可证/package.json，不执行安装脚本。
- 原样纳管 `scripts/architecture-docs/assets/mermaid.min.js`：3572661 bytes，SHA256=`581ed7d74bd9048d0e3a91363927d72ef22942d7722546b27f7cc29e35390eb8`；MIT LICENSE：1089 bytes，SHA256=`ec9fb67dcb25eccc416ed56e1aab819222c805a2a4bfe4cb19e7556bf2ffde80`。元数据存于同目录 `mermaid-manifest.v1.json`，安装后二次比较通过；单独资源校验不能替代浏览器离线渲染。
- 实际只读RFC内容244825 bytes，SHA256=`a5e9d188d02b70899f3d1e548c9563a83650e614b5dcbdc4b2bbfc295a23ae8a`。当前117个标题、54个标准表格、json/sql两块代码，无Mermaid图。图表通过独立fixture验收，未改RFC造图。
- 已批准创建独占临时profile的HeadlessChrome152.0.7977.83，CDP getVersion成功；未使用用户Chrome配置/账号，没有打开外部网页。实际页面验收结果见下节。

## 开发验收关注点

RFC实际使用跨行行内代码（50–51行）、类型尖括号、数字方括号实体及空行后两条没有新表头的SameState规范行（748–749行）。renderer不能把类型当HTML、把实体双重转义、逐行拆坏code span或静默丢弃孤立规范行。完整源字节保留、真实CLI回归及独立DOM检查共同验证，不只比文件存在或hash字段出现。

## 首版源码对应验收（修复版须另行覆盖）

| 检查 | 实际结果与证据 |
| --- | --- |
| 正式 CLI 行为测试 | `ruby scripts/architecture-docs/test/build_test.rb`：20 runs / 588 assertions / 0 failures / 0 errors / 0 skips，seed 13802，48.269724s，47008 终态 exit0 |
| 新文件语法及正式生成 | 四份 Ruby `ruby -c` 与 `node --check` 通过；真实 RFC 与独立图 fixture 均通过正式 `build.rb rfc --draft` 生成，24545 exit0 |
| 真实浏览器 | 12417 exit0：`headings=117 tables=54 diagrams=3 diagram_errors=1 fullscreen=entered_exited injection=blocked network_attempts=0`；覆盖搜索、主题、折叠、打印、图源与实际全屏；导航前已拦截 HTTP(S)，报告的是请求尝试数而非仅成功数 |
| 确定性与只读检查 | 重复构建返回 `html_current target=rfc`；`--all --check --draft` 返回 `html_current targets=rfc`；生成前后完整字节及 mtime 相等，67924 exit0 |
| 冻结输入 | 同批 `check-rfc-inputs.rb --root .` 返回 `rfc_inputs_valid`；RFC 内嵌 Base64 解码后与原 Markdown 全部244825字节一致 |
| 手写源码空白 | 6 份手写文件、manifest、LICENSE 的限定 `git diff --cached --check` exit0；不含原始第三方 JS，见下述窄范围例外 |
| 独立审查 | 固定 `eb19ee7..ca2708d` 的规格/质量审查当时返回 Needs fixes：0 Critical / 3 Important / 0 Minor；后续两轮修复及最终结论见下文 |

验收原始记录保存在本隔离树 `.superpowers/sdd/2026-09-09-offline-rfc-html/`：`final-build-tests.txt`、`final-browser-smoke.txt`、`final-generation.txt`、`final-repeat-check.txt`、`final-artifact-proof.json`。`final-repeat-check.txt` 中的普通 `git diff --check` 发生在新文件暂存前，仅覆盖当时已跟踪的未暂存修改，不是整个最终新增文件集的空白通过证据。

首版产物为4213377 bytes，SHA256=`b66de1b8e9dbdb17cc96105efa210e3e5e79149a2e12754eb2eaf0b21046a2de`，已由下节修复版替代；不再把首版文件hash当作当前交付。页面状态保持 `PROVISIONAL`；实现、模板、源材料或固定资源的漂移均纳入新鲜度检查。

原始官方 Mermaid JS 含30处尾随空白；其全量暂存 diff 检查实际 exit2。已重新验证 JS 与官方包提取文件逐字节相等，LICENSE 同样相等。为保持固定发行文件及内嵌字节保真，保留这些告警，不修剪 JS、不增加全局 Git 空白豁免。生成 HTML 内对应嵌入原字节也适用此窄范围例外；不能因此声称未限定范围的全量 diff 检查全绿。

## 修复版实际验收

本批独立审查发现：`markdown_renderer.rb:60-90` 的注释扫描先于 code span，输入行内代码 `<!--` 会吞掉后续正文；另缺 Mermaid 围栏内 click/HTML/init 指令的真实安全断言，以及 source/template/assets/output 父目录 symlink 的正式 CLI 回归。三项均交回原实施者，round 1/5 修复提交为1d1a389；限定复审确认三项全部ADDRESSED，但发现新的Important：未闭合反引号的状态跨空白段落持续，使下一段合法HTML注释显示为正文。第二轮仅修renderer段落边界及对应回归，模板/browser不改；下表为首轮已验证范围，不自动替代第二轮证据。

| 修复项 | 实际证据 |
| --- | --- |
| 行内代码与注释 | 正式CLI反例先失败1 run/5 assertions，修复后1/15通过；匹配反引号、不同长度、跨行span、闭合注释及未闭合字面均覆盖 |
| 父目录symlink | 正式CLI 1/33通过；源/输出共享父目录分别覆盖缺失输出不创建、已有输出字节/mtime不变，另有模板/资产父目录拒绝 |
| 修复后回归 | 本批22 runs/638 assertions/0 failures/0 errors/0 skips，seed44486；后续仅模板继续修改，受影响定向检查1/56通过（seed47347），未将其冒充最终全套重跑 |
| Mermaid实际失败→修复 | 旧模板90790触发恶意节点；只锁定配置的版本51377仍残留xlink外部导航；惰性清理最初因未声明xlink前缀解析失败，经具体SVG诊断后补兼容处理，并在进入页面前移除可执行节点、链接与事件属性 |
| 修复版真实浏览器 | 49272 exit0：`headings=117 tables=54 diagrams=4 diagram_errors=1 fullscreen=entered_exited injection=blocked network_attempts=0`；三类正常图及恶意图安全渲染，非法图明确回退；真正点击图节点后无回调、执行或导航 |
| 重复生成与输入 | 99687 exit0：重复构建current、`--all --check --draft`仅明确报告targets=rfc、`rfc_inputs_valid`；生成前后完整字节和mtime相同 |

首轮修复 [RFC 离线页面](push-system-implementation-rfc.html) 为4214805 bytes，SHA256=`e10dbf6bc0e9a3227e1f0d579a3421518db0216f86b864cc13b6c60400c3f749`；模板SHA256=`aab4d78f24d0b124c679020e8062b44d91e9be94ba318f51838d5a2424d13af8`。该产物已由下节第二轮修复版替代。内嵌Markdown仍与规范原字节一致。父线实际记录见同SDD目录 `fix1-namespace-browser-smoke.txt`、`fix1-final-artifact-proof.json` 及实施者 `task-1-report.md` 的修复段。

生成HTML中的Mermaid原字节已完整匹配本地固定资产；其30条空白诊断全部位于该资产的内嵌区域，无额外空白诊断。HTML相对空文件的 `git diff --no-index --check` 实际exit3（新增差异及空白），不是全量检查通过；修复4份手写文件的暂存diff检查exit0。

原独占浏览器临时目录和进程后续实际丢失，父线核对后重建新独占profile，重新复制真实输入并正式生成夹具，再取得上述49272结果；不是把超时当作退出或沿用失效句柄。Chrome平台自身有显示/GCM/updater日志，“0请求”只指被验收页面发起的HTTP(S)尝试，不是整个浏览器进程或操作系统无网络活动。

## 第二轮段落边界修复

源码提交=`ca1b5814111f4220a395c403496b3084aab4a268`；只修改renderer与相关测试，模板和浏览器脚本不改。正式CLI反例先失败1 run/9 assertions，修复后1/27通过；覆盖空行、带空白字符的空行，同段跨行code span、跨空行完整注释隐藏及未闭合注释字面保留。相关注释/code-span和真实RFC定向回归4 runs/117 assertions全部通过，seed49308；Ruby语法通过。独立第二轮限定复审最终为Approved：段落边界问题ADDRESSED，原I1/I2/I3保持关闭，本次修复范围无Critical/Important/Minor；不是全项目审查通过。

父线在最终ca1b581上实际执行 `ruby scripts/architecture-docs/test/build_test.rb`：23 runs / 676 assertions / 0 failures / 0 errors / 0 skips，seed57616，102.997891s，38028终态exit0。原始完整输出为同SDD目录 `final-ca1b581-build-tests.txt`。这是一次真实完整运行，不是把旧22/638与新增1/27相加；复审报告曾出现的23/665已明确撤回，不作为证据。最终限定复审依据及更正见 `review-fix2-verdict.md`。

最终RFC HTML为4214805 bytes，SHA256=`e5b39334735dbf87dc4d5cc76861677fd6822828f9a2f54e0c8cdd698617aa14`。正式重生成94747 exit0；真实RFC和5图夹具重生成前后除build-metadata外的完整字节哈希均相同，模板与browser SHA也不变（`fix2-render-equivalence-proof.json`）。这支持复用49272的实际浏览器验收，但不代表第二轮重新运行了Chrome；新增段落行为由上述CLI回归证明。

最终重复构建与只读检查10742终态exit0，RFC及夹具的完整字节和mtime均不变，证据为 `fix2-final-checks.txt`、`fix2-final-artifact-proof.json`。与最终源码匹配的[RFC离线页面](push-system-implementation-rfc.html)随本记录纳管；仍是PROVISIONAL，不构成严格发布批准。

最终四文件暂存核对：路径集合精确匹配README、实施记录、计划和正式HTML；三份手写文档的限定diff检查exit0，新增相对链接全部存在。全量暂存diff检查实际exit2，30条诊断精确对应HTML第1278–4914行内嵌官方资源的原始尾随空白，没有其他诊断；记录见 `final-docs-staged-checks.json`。本例外不掩盖内容/输入/安全校验失败。

专用Chrome已通过Browser.close返回确认，其9693进程句柄终态exit0，版本为Chrome/152.0.7977.83；没有关闭用户浏览器或操作生产monitor。证据见`fix1-browser-close.txt`。

## 并行分工与剩余边界

另一只读 agent 已完成下一批统一 checker 的真实集成 fixture/CLI 验收准备；父线负责结果文档、产物纳管与收尾，不让多个实施者同时修改当前代码。下一批准备已识别：现有简化 fixture 不能直接拼成总正例，必须保留真实 baseline commit、八输入、WBS freshness 与正式 HTML 构建链；这只是准备，不是 checker 已实现。

当前实际前置门禁：RFC draft通过；旧Catalog draft有107项内容漂移（1文件集、18文件SHA、80符号行号、1缺失旧符号、7符号SHA），原因是后续Rust实现已变化而历史目录仍绑定07781bf。已完成其中8项实质变化的只读语义核对，尚未更新当前审计目录。不能忽略这些失败或仅验证旧commit来让统一draft变绿，也不能机械覆盖RFC固定依赖JSON。

尚未完成：蓝图 current 派生路径与内容、第二个构建目标及兼容 wrapper、当前审计证据演进、统一 `check.rb`、实际 CI 与严格发布证据。全部运行时迁移仍按原总计划继续，不以本批测试数量推算完成率。未操作生产monitor或生产数据库。
