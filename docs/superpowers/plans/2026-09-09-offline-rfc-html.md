# RFC 离线 HTML 构建与验收

日期：2026-09-09。状态：本批Task 1完成。BASE=`eb19ee7a11100e40d93affc81f0ef2d1b144b4bd`，最终源码=`ca1b5814111f4220a395c403496b3084aab4a268`。本批落实已批准Q60/Q68/Q70/Q92/Q95/Q96/Q106/Q108及冻结硬化计划Task5的RFC垂直交付；不把单份RFC交付代替两份HTML、统一checker/CI或完整改造。

## 依据与启动时事实

- 规范：`docs/push-system/grill-decisions-2026-09-02.md`；输入原字节边界：`docs/push-system/rfc-input-manifest.v1.json`、README“八份不可变输入”；验收要求：冻结 `docs/push-system/push-documentation-hardening-plan.md` 的Task5–7。
- 启动时没有build.rb、模板、本地Mermaid或旧renderer脚本。Ruby2.6.10；当时RfcInputs只读校验基线12 tests/238 assertions通过（97514），Git隔离树干净。原rfc_spec strict只有HTML存在检查，不证明离线或新鲜度。
- RFC现2425行：117标题（H1=1/H2=59/H3=5/H4=52）、54标准表格、json/sql两块围栏，无Mermaid。RFC不得为了测试改写或增图；图表验收使用独立自有fixture。
- 蓝图原件不覆盖；之前current派生路径/解冻选择未获回答。本批只固定原计划既定RFC输出，蓝图目标与旧脚本兼容wrapper后续明确，不能用 --all 隐瞒暂时仅有一个已授权目标。

## Global Constraints

- 仅在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905` 的既有codex隔离分支开发；不操作根工作树、Rust、config、生产monitor、DB、provider/LLM/sink/订单、.env或部署。不修改规范RFC Markdown、八份冻结输入/manifest、来源catalog/WBS或v18/v19原件。
- Ruby标准库兼容2.6；不得引入gem、filter_map、tally。构建时不联网、不执行代码围栏/导入历史生产脚本、无系统时钟或绝对本地路径进入生成HTML。浏览器验收只用自有临时profile与本地fixture，网络请求被拦截；不使用用户Chrome配置或真实账号。
- 唯一实施agent拥有下列新代码/测试/模板及本Task报告；父线拥有第三方资产取得/校验/纳管、最终命令/Git/中文docs；不并发编辑同文件。代理可运行本批纯临时Ruby定向测试，不跑Cargo/旧全套测试/Git写入/网络，不再派代理。
- 所有手写文件用apply_patch；固定第三方发行文件通过正常下载/解包安装并验证原字节，禁止重新压缩或手改发行JS/许可证。保留原始日志、临时取证与SDD材料，不删除交接证据。

## Task 1: 从规范 RFC 生成可验证的单文件离线页面

### 文件与职责

- 新建 `scripts/architecture-docs/build.rb`：CLI，薄参数/退出状态层。
- 新建 `scripts/architecture-docs/html_builder.rb`：受限target映射、输入校验、模板/本地资产加载、确定性文档模型、只读比较与原子输出。
- 新建 `scripts/architecture-docs/markdown_renderer.rb`：可复用Markdown子集（正文/标题/表格/代码/列表/引用/链接），不含文件系统或网络。
- 新建 `scripts/architecture-docs/templates/document.html.erb`：独立HTML/CSS/展示JS，搜索/导航/主题/折叠/打印/图表。
- 新建 `scripts/architecture-docs/test/build_test.rb`：通过正式CLI验证可观察行为，不mock内部算法。
- 新建 `scripts/architecture-docs/test/browser_smoke.mjs`：无npm依赖的Node WebSocket/CDP浏览器验收程序；只连接父线创建的隔离Chrome，验证页面/交互与网络边界。Node是否支持原生WebSocket先实际核对，若不具备报告父线，不擅自引入依赖。
- 父线取得固定 `assets/mermaid.min.js`、`assets/mermaid.LICENSE` 和 `assets/mermaid-manifest.v1.json`；版本11.17.2，官方npm包。manifest记录精确版本、包URL/SHA512 integrity、脚本/许可证bytes+SHA256；实际值父线取得后写入。构建必须逐项验证元数据结构/精确文件名及实际字节，不仅展示hash。
- 父线用正式CLI生成 `docs/push-system/push-system-implementation-rfc.html`；中文结果/README/本计划随最终证据更新。

### CLI与事实权限

1. `ruby scripts/architecture-docs/build.rb rfc [--root ROOT] [--check] [--draft]`；`--all`表示所有已注册构建目标，目前明确输出目标集合仅rfc。没有target/未知target、重复或冲突参数、额外位置参数均exit2；`--help`exit0。默认root由脚本仓库相对路径决定，不用调用者cwd隐式选生产材料。
2. 固定映射rfc → `docs/push-system/push-system-implementation-rfc.md` → 同名`.html`，不提供任意输出/路径穿越/覆写冻结蓝图选项。`--all`不可与target并用，并明确报告 `targets=rfc`；文档说明不是两份HTML完成。
3. 读取并验证八输入manifest契约/字节（复用RfcInputs，不重写）；RFC/模板/资产采用真实根包含关系、逐路径分量拒绝symlink、普通文件校验；输出必须位于既有合法父目录，拒绝symlink/hardlink、目录或输入同文件。允许输出尚不存在，不为非法路径创建目录。不建立通用恶意并发文件系统安全权限承诺；沿用工作树本地工具模型。
4. 内容/hash/路径/资产错误exit1，输出稳定可定位reason code，不泄露环境或打印无关stacktrace。`--check`完全只读，缺文件和字节陈旧均exit1，不能修复。普通构建仅当不同才原子写入（同目录Tempfile→flush/fsync→rename），原输出/源材料在失败时不变，相同内容不改mtime。
5. 本批只产出PROVISIONAL。`--draft`只允许临时状态生成/检查，不绕过任何内容/哈希/路径/陈旧校验。不带--draft时明确 `html_status_provisional` 非零失败且无写入，不能因当前git干净把规范RFC升为Implementation-Ready。完整strict发布与CI另批接线。

### Markdown与页面合同

6. 渲染UTF-8中文、H1–H6及稳定唯一锚点/层级导航、跨行普通段落、粗体、行内代码（含跨行code span）、列表/引用、表格/围栏代码。普通 `<SourceRef>` 等文本必须可见，先区分代码再处理强调；`data/**`和算式乘号不变。实体按一次HTML语义处理，不双重转义数字实体，也不将解码结果再次当HTML或链接执行。
7. 相对/片段/HTTPS普通文献链接可点击；禁止javascript/data可执行scheme；图片/原始HTML不引入运行时外部资源。规范注释不显示为正文，但必须保留在内嵌原始源字节中。所有未支持语法至少安全显示原文字面内容，绝不能静默吞掉规范行。
8. 表格检测需要合法表头/分隔行；RFC:748/749的孤立两条SameState规范行保留为可见普通文本，不擅自合并到前表。合成fixture覆盖转义竖线与code span里的竖线，不能简单split丢列。围栏json/sql按原始空格/换行展示，不执行或格式化。
9. 页面内嵌源Markdown Base64原字节、原始SHA256、模板SHA256、renderer/builder/CLI实现SHA256（源码逻辑变化也需可检测陈旧）、Mermaid资产版本/许可/hash与PROVISIONAL边界。不给页面注入生成时间/机器路径/自动gitHEAD，连续构建逐字节一致。输入含`</script>`等必须无法逃逸数据容器。
10. 独立模板保留可用目录/标题搜索、主题切换、章节折叠、打印（打印展开内容）、图表缩放/全屏/源码回退。普通文献URL允许存在，不以任意http字面出现判外部运行依赖；运行时script/style/font/image必须自包含，默认不fetch，不从CDN或动态模块加载。
11. Mermaid完整发行脚本内嵌，strict配置，不执行源文click/HTML指令；成功时展示SVG并保留图源。非法图明确失败并可读原图源，不以fallback冒充成功。RFC无图，须真实flowchart与sequence/state类独立fixture在断网浏览器成功产生SVG，检查无脚本/资源网络请求。

### TDD与最终验收

12. 第一条tracer：自有临时目录中放最小规范Markdown/真实本地模板及资产/合法冻结输入fixture，通过正式CLI `rfc --draft`，要求缺失HTML创建、中文/原始SHA与源字节保留。builder不存在前实际失败并记录；随后只补足该垂直行为，继续逐项RED→GREEN。代理允许运行自己明确限定的Ruby测试，每轮保存命令/真实输出，不先写几十条不存在接口的水平测试。
13. 补真实CLI测试：重复构建字节与mtime不变；--check缺失/陈旧/模板变化/实现变化拒绝且无写；源/资产/许可/manifest错误；draft也拒绝内容漂移；strict临时状态失败；参数错误；文件/父目录链接与hardlink拒绝；转义/多行代码/实体/孤立行/表格的独立预期；源码脚本注入无逃逸。
14. 真实RFC回归不修改RFC：核对117标题/54表格/两块完整代码，并定点核对跨行code、类型尖括号、数字实体、两条孤立行、52 Unit层级；从内嵌源取回的完整字节精确一致。
15. 父线正式生成真实RFC HTML，重复构建与 `--all --check --draft`通过；八冻结输入前后校验不变。浏览器程序等待文档ready/图render终态（有明确超时），通过真正的DOM检查搜索/主题/折叠/图SVG/图源/打印样式；CDP拦截http(s)请求并报告页面发起的尝试，0尝试才证明没有依赖网络，不能只看资源404。
16. 最终 `ruby scripts/architecture-docs/test/build_test.rb`、相关既有rfc_inputs测试、所有新Ruby `ruby -c`、JS `node --check`、八输入check、git diff --check；只跑文档相关安全套件，不重新编译未改Rust。父线记录最终源码对应证据并提交，独立固定BASE..SOURCE规格/质量审查，修复只做限定复审。

### 非本Task完成项

本批完成的是实际RFC单文件离线生成及验证。蓝图current化/新输出路径与wrapper、两目标--all、统一check.rb/实际CI、严格发布证据及全部运行时迁移仍继续保留，不用“目标集合目前只有rfc”偷换两HTML总目标。

## 最终验收记录

- [x] 正式CLI、确定性构建、只读新鲜度检查、冻结输入与路径/资源校验完成；最终完整测试23 runs / 676 assertions / 0 failures / 0 errors / 0 skips，seed57616，102.997891s，38028终态exit0。
- [x] 真实RFC的117标题、54标准表格及完整244825字节源材料保真；最终HTML4214805 bytes，SHA256=`e5b39334735dbf87dc4d5cc76861677fd6822828f9a2f54e0c8cdd698617aa14`。
- [x] 真实隔离浏览器49272终态exit0：4个SVG成功、1个非法图明确回退，实际全屏进入/退出，恶意图click/HTML/init及原始HTML注入被阻止，页面HTTP(S)请求尝试0。第二轮仅renderer/测试变化，完整非metadata页面、模板和browser字节等价证明支持复用该证据；不宣称再次运行Chrome。
- [x] 最终重复构建和只读检查10742终态exit0，完整字节及mtime不变；八份冻结输入保持原字节。专用Chrome已确认关闭，9693终态exit0；未操作用户浏览器、生产monitor或数据库。
- [x] 固定首版及两轮限定规格/质量审查完成，最终Approved；原三项发现和段落边界回归全部关闭。撤回曾误写的23/665，最终套件证据仅采用真实23/676，不合并历史计数。
- [x] 中文实施记录、README、本计划及正式HTML共同纳管。详细命令与证据索引见[实施记录](../../push-system/implementation-offline-rfc-html-2026-09-09.md)及本隔离树 `.superpowers/sdd/2026-09-09-offline-rfc-html/`。

验收例外：官方Mermaid发行JS及HTML内相同原字节含30条尾随空白，保留固定发行hash和许可证，不格式化资源、不增加全局Git豁免；手写文件限定检查通过，不把全量空白检查描述为全绿。HTML仍是PROVISIONAL。此完成状态只覆盖本Task，不解除上节全部剩余项。
