# 当前项目架构蓝图：实施记录

日期：2026-09-10。状态：本蓝图与双目标HTML计划完成。Markdown Task1及Task2源码`abeabf6bda611113629ffa8b36cd8dceb55c7126`均通过独立规格/质量审查；完整受影响测试、真实双HTML生成/门禁及新蓝图浏览器验收通过。两份实际HTML及中文记录已在`3283eaf`纳管；最终整体审查的3项Minor由`fa991e2`收口，唯一限定复审3/3关闭、无新增问题。此结论不表示全项目或生产迁移完成。

## 已交付

[当前架构蓝图](../architecture/current/Project_Architecture_Blueprint.md)已在隔离分支提交 `c1e24d0cad4fd1c0be8a52109c73b13da9f8eefd`，实施BASE为`ee0db4a0a927ee1372bfe354a5e185013cddc3c4`。旧蓝图MD/HTML及冻结RFC、来源、机器规范保持原件。

- 18节正文覆盖进程、数据、业务、推送、AI、存储、认证/配置、测试/CI、扩展与维护；旧26章和A–I附录及专项小节共68项对应说明公开保存在[覆盖表](../architecture/current/Project_Architecture_Blueprint.md#17-冻结旧蓝图逐章节覆盖裁决)。
- 四时段覆盖65 kinds、102 producers、52 Units；全部入口链接正式目录的触发、输入、完成权威、失败策略和现存问题，不用代表性代码替代全量身份索引。
- 九份冻结v18/v19来源与额外七份Git资料分别列原字节SHA和实际吸收边界；594个全仓Rust文件与548项push manifest分别说明。
- 库级/测试专用/条件/未接线、默认入口与外部部署分开；未跟踪的外部proto不归入Git基线，也不以静态阅读序号冒称wire ID。

## Task1定稿版本的验证证据

Task1提交时Markdown为1029行、190257字节，SHA-256：`ab3422d2bffbaa7a3b0ee3c25522e84fd574c06cd456a206fad6f0c7b8c46b73`；这是下方两处小修之前的版本。Rust/Cargo来源仍为`aef7972965f610ed418049593dfff1d55341772e`；catalog/manifest绑定采用[047b4ab最终审计材料](implementation-current-source-audit-2026-09-09.md#task2-unit层残余说明纠正)，状态保持PROVISIONAL。

| 验证 | 实际结果 | 不覆盖什么 |
| --- | --- | --- |
| 父线固定身份/链接检查 | exit0；35标题、唯一H1、6图源、1577个真实渲染链接、227标题锚点、1335行锚点；0错误 | 行号在界内不证明段落语义，也不是HTML浏览器验收 |
| 父线业务表精确对账 | exit0；65种kind的时段/status/producer及52个Unit的phase/owner/producer逐项相等；102入口在Unit表恰出现一次；JSON定位行属于对应声明 | 不证明52 Unit已迁移或生产启用 |
| 实施者来源检查 | 215个引用Rust/Cargo文件匹配已验收manifest摘要；52个补充文件匹配ee0db4a；16原文匹配准备/实施两Git基线；68项旧章节定位存在 | 无Cargo metadata或生产来源/部署检查 |
| 真实当前Markdown安全渲染 | exit0；35标题、33表、6图源；未写HTML | 图源计数不是Mermaid浏览器成功数 |
| 独立限定审查 | Spec符合；Quality Approved；抽查实际selection gate、Foundation测试绑定、投递白名单和回测/设计边界 | 不认证全部Rust动态行为、远端CI或生产推送 |

私有工作底稿保留实际命令、原始终态与限定审查，不作为公开页面必需依赖。旧初稿的四项链接失败已随定稿修正；没有把初稿检查冒充最终结果，也没有为Markdown重跑未变Rust套件。

## Task2源文档增量核对

开发BASE为`d78acbd899c2973feef491cfefd1859eefda02b3`。父线只读检查实际exit0：当前MD逐字节等于该BASE版本加两项批准替换，没有其他正文变化；v19.1改为“原设计的5日收益验证闭环”，旧A.1标签移除src/lib.rs两侧反引号，链接目标仍为旧蓝图#L1833。

修改后为190259字节，SHA-256：`07d80a516332b013274caf200ef354e67e6d648babf2fd70a19aa4a75f559c85`。current两JSON仍匹配047b4ab已验收摘要；Rust/Cargo相对aef7972、冻结输入相对Task2 BASE的diff均为空。来源预检本身不证明HTML通过；后续实际完整内嵌字节和A.1链接验证见下节。

## Task2双目标实际验收

使用[当前蓝图HTML](../architecture/current/Project_Architecture_Blueprint.html)和重生成的[RFC HTML](push-system-implementation-rfc.html)，不覆盖旧蓝图。两份页面仍为PROVISIONAL，完整内嵌各自Markdown原字节，metadata绑定实际模板、构建实现及固定Mermaid资产。新蓝图实际HTML包含修复后的A.1链接、35个标题、33张表和6份完整原图源。

| 验证 | 真实结果 |
| --- | --- |
| 受影响Ruby合批 | `ruby -I scripts/architecture-docs/test -e 'require File.expand_path("scripts/architecture-docs/test/build_test.rb"); require File.expand_path("scripts/architecture-docs/test/check_test.rb")'`，exit0，55 runs / 1128 assertions / 0 failures / 0 errors / 0 skips，1115.977797s |
| 最小真实fixture抽取回归 | Catalog有效隔离树、历史B→当前C及非祖先/来源缺失三项定向，3 runs / 28 assertions，全通过；不mock或跳过Catalog |
| 实际首次生成 | `ruby scripts/architecture-docs/build.rb --all --draft --root .`，exit0，依次写入rfc、blueprint |
| 确定性与只读 | 重复同一生成命令、`build.rb --all --check --draft --root .`及兼容命令`ruby scripts/render-architecture-blueprint-html.rb --check --draft --root .`均exit0；bytes/mtime不变 |
| 统一内容门禁 | `ruby scripts/architecture-docs/check.rb --draft --root .`，exit0，明确报告`html_targets=rfc,blueprint` |
| 严格发布门禁 | `ruby scripts/architecture-docs/check.rb --check --root .`，exit1；只剩历史/current四项PROVISIONAL、RFC/WBS两项PROVISIONAL及当时的worktree_dirty，未出现内容错误；不是严格发布通过 |
| 保护快照 | 668项文件；首次仅两份HTML改变，重复及检查阶段无保护文件变化，Git index的bytes/mtime相同；测试/browser脚本与父线记录因并行开发明确不在该快照范围内 |
| 新蓝图实际浏览器 | 独立`--blueprint`入口exit0；35标题/33表/6图正常SVG，单次render故障仅1图可读回退、其他5图正常；搜索/主题/折叠/打印/全屏通过，页面HTTP(S)请求尝试0 |

首次Ruby合批曾有一项测试自身的二进制/UTF-8比较错误，修正期望字面为`.b`后完成上表真实整批复验；没有把失败轮或定向通过冒充整批通过。当前源码没有因此修改模板、renderer或资产。

| 制品 | 字节数 | SHA-256 |
| --- | ---: | --- |
| 当前蓝图HTML | 4075422 | `42e4b1f9e3816de126defb251ca97879ade022a977c358b510d6ee144581d076` |
| RFC HTML | 4214805 | `3fc9aba16e079569f2037f0399c5bd7aa79969d647b1b785528e46493be4ed70` |

两HTML分别相对`/dev/null`执行完整`git diff --no-index --check`，实际各exit3；各30条尾随空白诊断逐行等于原始官方Mermaid内嵌位置，没有额外诊断。仅保留既有原字节例外，不增加全局豁免，不把该结果描述为不限定范围的diff检查全绿。11份手写源码/测试的正式暂存diff检查exit0；公开文档本轮新增26个本地链接/行界/标题锚点检查无错误。

Chrome/152.0.7977.83使用独立临时profile，Browser.close已返回确认且自有进程exit0。平台显示/updater日志另保留，不把“页面0请求”扩大为整个浏览器/操作系统零网络；未重跑旧RFC中未变的安全调查。浏览器关闭后再次核对两MD及两HTML摘要相等，随后才提交源码；提交不改变产物实现指纹。

开发期间有一次agent命令误传workdir，执行只读git status/diff/sha/rg时进入根工作树；未执行文件修改、生成器、测试或显式Git写命令，但未设置GIT_OPTIONAL_LOCKS，不能证明根index没有被刷新。发现后停止访问，未尝试修复根树既存改动。本批全部实现、验收和提交均在隔离树内。

## 后续收口

Task2按[双目标计划](../superpowers/plans/2026-09-09-current-blueprint-offline-html.md)完成独立规格/质量审查；SOURCE为abeabf6，正式差异包固定d78acbd..abeabf6，仅11份源码/测试及两处新MD修订，主线实际产物证据另供审查。结论Spec符合、Quality Approved、C0/I0/M2。最小目标识别的正式RED为1 run / 2 assertions、exit1、旧target_unknown；测试装配阶段的NameError不计为行为RED。

最终整体审查固定`ee0db4a..3283eaf`，技术通过的三项Minor均在`fa991e2b4c4c27e0b9ac511ab8b1e85ea4b25833`关闭：browser usage准确表达endpoint加至少一个profile；新增仅改变manifest合法JSON空白、正式Catalog仍有效、旧SHA声明写前拒绝的独立反例；W15 §13和README精确导航共同纳管。最终定向合批2 runs / 14 assertions、0失败/错误/跳过，未将此前55项冒充修复后全量重跑。唯一限定复审固定`3283eaf..fa991e2`，3/3 ADDRESSED、无新增问题。

父线以提交对象确认四个修复文件与工作文件一致，W15 §13标题、§11提示、README导航及11个源码链接有效。浏览器脚本仅usage常量改变，其他执行逻辑逐字节相同；其新SHA为`39a8c2ac1429abe7c397051633f3526a0f2dcead0a9f98ad5d4ddae2a092ecf0`。构建实现、两份Markdown/HTML、模板与资产未改变，因此保留原实际构建及浏览器验收证据，不把帮助文案变化说成脚本原字节未变。

远端CI、生产接收证明与完整运行时迁移仍未完成。后续按[W15当前记录查询计划](../superpowers/plans/2026-09-10-readiness-current-record-query.md)复用现有v3集合接线，认证与最终health/readiness/CLI仍为独立必要验收，不以内部候选入口替代。旧v5–v9审计顺序兼容、完整W15–W21及52 Unit迁移等仍按[开发入口](README.md)继续；[追加存储证据](implementation-durable-upgrade-2026-09-08.md#追加存储能提供的顺序证据)只是已核查的兼容设计输入，不是运行修复。
