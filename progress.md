# Architecture Blueprint 重制进度

## 2026-09-05 第二批开始

- 用户要求继续；隔离分支codex/push-reliability-20260905为07781bf，受跟踪工作区干净，linked worktree已验证，不是子模块。
- 沿用已批准的108项决策及文档硬化设计，先补齐Foundation依赖的来源/机器目录/源码证据门禁；不把根目录两个未移入的kind当成隔离代码已实现项，也不改变Q44晋级顺序。
- 已读取brainstorming/writing-plans/SDD、planning-with-files、using-git-worktrees、TDD、verification和codebase-design；计划先落docs后逐项实施/复核。session-catchup无新增恢复输出。

## 2026-09-05 首批代码实现与复核

- 最终whole-branch审查无Critical/Important；唯一Minor读取拒绝缺warning，由原实施者修订，controller仅接手Git提交元数据形成90275fe。9项alert回归通过，增量monitor构建3m41s exit0，四源格式/diff检查通过；最终限定复核已关闭该项且无新破坏。
- 首批开发候选交付完成，源码90275fe，docs记录阶段版本与实际验证，不把2f07ac2全组回归冒充90275fe的全量重跑。分支/worktree与取证材料保留，原master及其160个未合并项不动，不merge/push/部署；完整方案后续仍列未完成。

- 首批最终源码2f07ac2，计划/结果提交320dba1，均在独立分支。Task2公开I/O与未知origin补测后独立复核通过；事故披露/保留补救通过不等于原始保全要求通过。
- controller在最终源码fresh执行：monitor离线构建2m06s；R08 29、review54（1显式child-helper忽略）、alert9、G5b17、env_guard7、告警格式7全部通过；整组exit0。四源文件rustfmt及分支/工作区diff检查通过，现存非原始开发样本前后hash不变。
- 两份交付文档链接/行号边界检查通过，原目录分析校验166项通过。已派发最终whole-branch审查；未合并、未push、未部署。首轮文档提交被sandbox拒绝写worktree索引，按权限机制升级后成功，没有处理原master索引。

- 隔离分支已提交R08类型化错误修复977cde1、调度边界回归e4de707；独立复核通过。29项R08、54项review测试通过，1项子进程入口由父测试调起而忽略；不宣称失败状态跨重启持久化。
- 告警/G5b隔离实现b31a0cf已提交；controller独立离线构建exit0（5m14s）、alert8/8、G5b17/17及四源文件格式检查通过。独立审查要求补齐公开归档I/O错误测试，实施者正在修订。
- 执行偏差已披露：重复旧基线测试追加了开发区样本，随后误用恢复操作，MD最终277字节而初始为278字节。停止继续恢复/删除，保留为非原始参考件；当前JSON/MD在修复后测试前后哈希一致。原目录历史alerts及G5b样本哈希未变。另有7个无关格式化差异已精确还原，未进提交。
- 原目录产品src/tests/Cargo差异哈希持续为3b0f746129bcdd108680af75da354fe087c00f300416483fdcab01705b48064e，未合并索引仍160；无真实发送、provider/LLM调用、生产DB变更或部署。

## 2026-09-05 用户允许独立开发区

- 已创建 `.worktrees/push-reliability-20260905`，分支 `codex/push-reliability-20260905`，基线 a673043；初始状态干净。
- 已逐项检查首批相关差异：alert_log / attribution_deep / review_batch 文本与基线一致；push_templates 的本地173行差异属于R-07收盘价/名称修复，不依赖于本批R-08，暂不移入且不覆盖原改动。
- 首次离线构建运行中；build.rs依赖ignored proto，已只复制10424字节合同，SHA-256 `8730bce3c20e170cf8f58047336ae06d3a5e9080d81568dee71e7b0882063332`；未复制运行数据、配置或密钥。
- 随后基线验证：monitor构建通过；R08 28、alert_log 5、G5b 13旧测试通过；存在84个lib、43个lib test既有dead_code告警。alert旧测试在独立开发区写出了TEST_CODE默认归档，确认污染机制；原目录历史样本SHA未变。
- 实施计划在独立开发区docs/push-system；原目录docs增加implementation-status入口。按subagent-driven-development要求进行单任务实施及独立复核，不并行写同一批源码。

## 2026-09-05 用户批准开始解决

- 已将当前请求切换为实施授权，完成只读预检；尚未写产品代码或跑生产程序。
- 当前master普通checkout仍有160个unmerged索引路径，未发现Rust文本冲突标记；不能未经基线验证就声称不可编译，也不能把它当干净开发/提交基线。
- 已阅读brainstorming、writing-plans、executing-plans、using-git-worktrees及planning-with-files；既有设计视为用户已批准，不重复Grill。执行隔离要求导致暂停，待用户确认创建独立worktree。
- 原工作树、冲突、数据库及消息发送状态不变；未清理现有worktree或复制生产数据/密钥。

## 2026-09-05 全量重新分析

- 已完成本轮分析并落到 docs/push-system：中文总报告、全量67项视图、证据JSON、采集脚本、校验脚本和README。
- 取证截点 09-05 08:57:02–08:57:10。主样本为08-31至09-04；周六单列。799条本地true声明含8条N02 typed，另119条durable Accepted；437笔paper卡均唯一匹配，不再将数量直接判为重发。
- 已验证 HTML/Markdown 同步但存在 CDN Mermaid 依赖；发现 PaperBuy/Watchdog 两个新增 kind。旧报告中将bool记录等同用户实际收到、将67项仍写成65项的推论已在新报告纠正。
- 已将12类问题、v18/v19边界、原108题约束/新增建议、里程碑、人力和非冻结工期写入报告；没有开始运行时实现或继续要求用户确认已有落盘授权。
- fresh校验通过165项：enum覆盖、回执/成交匹配、源文件哈希和文件链接。`ruby -c`通过；未跑Rust测试、未改生产代码/配置/DB、未处理160个冲突、未发送消息。既有完整RFC/离线构建/CI计划仍未完成，不冒充专项完成。

- 已启动只读生产分析与 docs 报告交付，读取 planning-with-files、codebase-design/DEEPENING 和 karpathy-guidelines，完成会话恢复与 108 项决策回读。
- 已识别 160 个 unmerged 路径，保留用户冲突现状；本次不运行会触发生产的程序，不用源码存在代替部署证明。

## 真实持仓录入进度（2026-08-31）

- 已收到东方财富真实账户截图，开始查找项目正式持仓录入路径。
- 已确认当前工作树有其他未提交修改，本任务将避开这些文件。
- 已启动 R1：检查 v18 real-account snapshot 迁移、portfolio store 与 CLI。
- 已确认截图逐股左列第二行是市值而非持股数量；下一步从市值/现价推导并核对实际股数，继续定位正式导入命令。
- 已由市值/现价精确还原 7 只股票股数并对账总市值；已定位持仓与账户汇总两个正式 one-shot importer，目标库为 `data/stock_analysis.db`。
- R1/R2 完成：证券代码由历史快照确认；图片 SHA 与 capture time 已取得；数据库写入前完整性检查通过。进入 R3。
- 已复核 BR-215 对齐语义：仅更新确认持仓且不删除缺失记录；本次 7 个代码集合与数据库一致，可安全进入正式导入。
- 两份输入 JSON 的结构、数量、成本、账户恒等式、仓位比例和图片 SHA 校验通过。首次备份验收未找到可打开文件，已按安全约束暂停导入并转入备份诊断。
- 备份诊断完成：文件存在且 immutable 完整性检查通过；恢复正式导入流程。
- 真实持仓快照导入成功：新增 1 条不可变完整快照，更新 1 条投影（华电辽能），其余 6 条不变，无未确认 open 持仓。
- 账户汇总快照导入成功：新增不可变记录 ID 8，daily P&L 已保存。
- R3/R4 完成：fresh SQL 回读、图片 SHA 复核、JSON 校验、数据库完整性和工作树 whitespace 检查全部通过。

- 2026-08-30：用户明确要求安装 Architecture Blueprint Generator 并重新画架构。
- 2026-08-30：安装器 direct-download 卡住后改用 git sparse-checkout，安装成功。
- 2026-08-30：完整读取 blueprint、brainstorming、planning-with-files skills。
- 2026-08-30：用户批准 Implementation-Ready、C4-oriented Mermaid、代码示例、实现模式、decision records、extensibility 的重制设计；开始重新核验当前代码。
- 2026-08-30：完成当前 modules/targets/tests/gRPC/schema/state/runtime/CI/deployment 代码证据复核，开始写正式蓝图。
- 2026-08-30：已生成根目录 `Project_Architecture_Blueprint.md`，开始结构、清单、路径与 Mermaid 对账。
- 2026-08-30：结构/路径/Mermaid 静态检查通过；62/40/44/40/14/53/12/18 全量 inventory 对账无缺项。
- 2026-08-30：fresh architecture boundary test 15/15 通过；最终 blueprint 验证完成，准备交付。
- 2026-08-30：用户要求把完整蓝图网页化并批准设计；读取 brainstorming、architecture blueprint、planning 与 TDD skills，开始工具盘点。
- 2026-08-30：RED 已确认；网页公开契约验收因目标 HTML 尚不存在而以 exit 1 失败，失败原因与预期一致。
- 2026-08-30：验收覆盖源 Markdown 逐字一致性、SHA-256、18 个图及原文回退、33+ 个章节和目录/搜索/折叠/打印/主题控件。
- 2026-08-30：生成 `Project_Architecture_Blueprint.html`（314,595 bytes）；生成器确认 33 个 H2 章节、18 个 Mermaid 图，源 SHA-256 与 Markdown 一致。
- 2026-08-30：GREEN 验收 exit 0：内嵌源逐字一致、18 个图及原文回退齐全、章节与交互控件齐全。
- 2026-08-30：2 段可执行内嵌 JavaScript 均通过 Node 语法编译；macOS Quick Look 成功生成 1600px 网页缩略图，进入视觉检查。
- 2026-08-30：首屏视觉检查通过，无重叠/截断；确认系统具备 tidy/xmllint，继续做标记级验证。
- 2026-08-30：系统 Tidy 为 2006 版本，因不识别 HTML5 语义标签产生假阳性；已记录并从验收工具中排除。
- 2026-08-30：确认 Safari/WebDriver 可用性，准备在不修改系统设置的前提下尝试真实浏览器运行。
- 2026-08-30：增强验收覆盖标题/代码块/表格一一映射、ID 唯一性、ARIA target、响应式与 file-open-safe；修复验收脚本对旧 Ruby 的兼容性。
- 2026-08-30：增强验收 fresh exit 0：33 H2、54 H3、25 code blocks、25 tables 与源文档逐项一致；18/18 图回退、ID/ARIA/响应式/file-open-safe 均通过。
- 2026-08-30：完整读取 verification-before-completion skill，开始最终 fresh verification。
- 2026-08-30：最终 fresh verification 全部 exit 0：网页契约、2 段内嵌 JS 编译、`git diff --check`、源哈希与工作树检查均通过；完成交付。
- 2026-08-30：用户要求继续；启动浏览器运行态验证扩展，重新读取架构与 planning skills 并建立 B1-B4 计划。
- 2026-08-30：planning session catchup exit 0，无未同步上下文；重读 B1-B4 范围。
- 2026-08-30：确认 8765 端口空闲，SafariDriver 版本为 Safari 26.6.2 内置版本。
- 2026-08-30：启动隔离 HTTP 服务与 SafariDriver，准备创建 WebDriver session。
- 2026-08-30：Safari session 因 Remote Automation 未启用而被拒绝；已记录错误且不修改系统设置，切换到本地可用替代运行时盘点。
- 2026-08-30：停止 SafariDriver；发现可用 Google Chrome.app，转用 Chrome headless/CDP 路径。
- 2026-08-30：以隔离临时 profile 启动 Chrome 151 headless，进入 CDP DOM/console/screenshot 验证。
- 2026-08-30：真实浏览器验收发现 Mermaid 未达到 18/18；记录失败并转入根因诊断。
- 2026-08-30：诊断确认 CDN/DOM/fallback 正常，故障位于图源 Base64 解码路径；当前 2/18 SVG 成功、16/18 失败。
- 2026-08-30：根因定位为生成 HTML 中 `\\s` 被模板吞掉，落盘成 `/s+/g` 并破坏 Base64；记录单一最小修复假设。
- 2026-08-30：最小修复后 18/18 Mermaid SVG 真实渲染成功；验收继续推进时在移动侧栏位置检查失败，转入该检查的独立诊断。
- 2026-08-30：移动侧栏确认是 220ms transition 测试竞态，页面 350ms 后正确到位；验收继续在两条 404 resource log 处停止。
- 2026-08-30：HTTP access log 将 404 根因定位为缺少 favicon；准备以内嵌 data-URI favicon 最小修复。
- 2026-08-30：内嵌 favicon 后 fresh server log 无 404；识别 CDP Log.enable 回放旧 404，修正验收 session 隔离。
- 2026-08-30：完整 Chrome 运行态验收 exit 0；18/18 SVG、全部交互、移动端与零 current-run browser error 通过并生成两张截图。
- 2026-08-30：首屏截图视觉通过；首图截图取景未包含 SVG，调整为按 diagram 元素坐标裁切后重拍。
- 2026-08-30：元素裁切的 FIG 01 真实 SVG 截图视觉通过；进入最终 fresh verification。
- 2026-08-30：旧临时静态 verifier 被系统清理；已记录，改为重建最小 verifier 后再做最终验证。
- 2026-08-30：最终 fresh runtime/static/JS/diff 验证全部 exit 0；记录并保留并发产品修改，停止临时 Chrome/HTTP 服务。
- 2026-08-30：端口 8765/9222 均确认无监听；B1-B4 全部完成，准备交付运行态结论。
- 2026-09-01：启动最近改动与架构网页漂移审计，建立 R1-R4 计划；本轮先报告，不修改架构或产品代码。
- 2026-09-01：catchup 完成；获取 master 最近提交、工作树与 diff stat，确认发生 provider/data transport 级架构迁移且网页存在实质漂移。
- 2026-09-01：定位蓝图 stale markers 与未提交 paper-sell gate 变化；inventory 首次受旧 Ruby API 影响失败，记录后改用兼容统计。
- 2026-09-01：完成当前 61 modules / 26 binaries / 41 tests 统计；确认 155-file 大迁移与 provider host 外置、market_domain 内聚边界。
- 2026-09-01：核对 README/Cargo/grpc_contract/grpc_source，确认 40 ops 保留，但 provider ownership、构建矩阵、路由与部署图均需重写。
- 2026-09-01：对账证据路径、no-magic gate、架构测试与 BR-249；确认 3 个核心证据路径已删除，并识别 NewsAI chain/durable retry/paper-sell worktree 漂移。
- 2026-09-01：读取当前 docs/ARCHITECTURE 并重算热点；获得新高层拓扑与 LOC，对 no-magic gate 的实际自动化程度完成定性。
## 2026-09-01 最近改动与架构网页漂移审计（续）

- 已核对 BR-249 的 NewsAI、review backfill、durable retry 与 closing valuation 影响。
- 已确认本轮改动未新增对应 DDL，但网页数据清单原本遗漏 closing valuation 两张表。
- 已定位 `monitor` 当前组合根与任务树锚点；下一步核对精确任务列表和架构计数后形成调整矩阵。
- 已确认任务树为 4 条主循环 + 8 个后台任务（新增 review backfill）。
- 已确认 `DecisionState` 仍为 14，网页数量无需改变，但 rejected retry 的状态语义需要更新。
- 已定位需要联动更新的网页章节与 closing valuation DDL 证据。
- 审计期间 HEAD 前移到 `e2503f2`；已把 paper-sell 调整改归类为已提交架构行为。
- Fresh metadata 已确认 61 modules / 27 binaries / 41 integration tests；no-Magic 门禁 fresh exit 0。
- 已确认 40 个 gRPC operation 数量未变，但 contract 源码注释与新外置 provider 架构存在语义漂移。
- 已确认 HTML 与 Markdown 当前内容 hash 一致，二者需作为同一产物联动重生成。
- 已刷新工作区状态：无 tracked 修改，HEAD=`e2503f2`。
- 已取得 README、当前 ARCHITECTURE、grpc bridge、monitor 任务树和 paper-sell 的精确源码锚点。
- 已识别两处不应继续传播到网页的源码注释债务：proto 路径与 local server/delegate 措辞。
- 已取得 BR-249 NewsAI chain context、ChainRisk、closing valuation exact-date check 与 no-Magic gate 的源码锚点。
- 已完成 blueprint 章节级对照，形成 P0/P1 调整范围与 durable/review 精确锚点。
- Fresh architecture integration tests：15/15 passed；gRPC operation count test：1/1 passed。
- Fresh inventory/hash/stale-keyword verifier 通过；已具备交付调整建议的证据。
- 本轮审计未修改产品代码或 blueprint Markdown/HTML。
# 全部推送项业务逻辑逐行审计进度（2026-09-01）

- 已完整读取 `planning-with-files` 技能并执行 session catchup。
- 已检查工作树，确认存在与本轮无关的用户改动；本轮保持只读。
- 已追加 P1-P5 调查计划，开始建立推送相关代码清单。
- 已完成第一轮文件名、关键词和目录扫描；确认推送实现横跨 monitor、分层 push 模块、notification、durable delivery、event 和测试。
- 已统计关键文件规模并抽取函数/类型/PushKind 名称；进入 P1 的精确 catalog 与调用点核对。
- 已读取 PushKind 定义及 main.rs 生产调用锚点；发现 enum 数量注释已漂移，后续以代码集合与可达调用为准。
- 已开始逐层读取 L1/L2/L4/L5/L6/L7；先记录事件身份、时间桶和 L7 审计字段，下一步核对该分层栈的生产接线状态。
- 已确认分层栈在 monitor 中通过 v14_adapter 接线；L6 transport 是显式 env opt-in，治理层的静默/数据质量/每日上限规则已逐行核对。
- 已核对 PushKind 冷却/作用域与普通、counted、source-only、news-flash 四类发送入口，进入生产 presentation catalog 与业务模板逐项映射。
- 已完整抽取 58 个 production presentation tuple，并核对 v14 gate 的 fail-closed、L7 审计和 legacy L4 reservation/commit 语义。
- 已对账 20-row audit dispatch table 与 23-kind durable catalog；继续建立 65 enum、58 presentation、23 counted 和真实 producer 的集合差分。
- 已得到精确集合差分，并读取第一批 14.x renderer、横幅、账户模式重试语义。
- 已逐项核对 11 个无 presentation kind 的真实状态，并核对 durable namespace/startup/envelope admission。
- 已完整核对普通 governor 与 PaperSell/SnapshotStale 直接生产路径，并区分 BR-196 test manifest 与真实 runtime 可达性。
- 已核对 CandidateBoard/IpoCatalyst 的真实生产调用，识别 NewsFlashCritical、T-17 与若干 review capability 的显式禁用/不可用状态。
- 已核对模板层第二重冷却、v17 normalized source 六类与 review 调度上下文；继续逐 producer 查默认启用条件。
# 2026-09-01 全部推送项逐行审计：进度补记

- 已完成复盘任务集合、来源类型、预检时间窗、结果重试语义的代码取证。
- 已完成机会调度器四类窗口和 CLI `--push` 路由取证。
- 已确认 P-01 的生产所有权在常驻调度器，CLI 路径仅保留补偿/提示语义；下一步读取 P-01 内部 due 状态机及主任务 join 关系。
- 已定位 monitor 的物理传输入口与通用 `notification::NotificationService` 的调用边界；后者属于分析流水线/摘要/告警，不属于 65 个 `PushKind` 的统一治理路径。
- 已读取 P-01 claim/resume/load/render/push/post-inspect 状态机和 monitor 物理发送分支，正在补全精确行号与所有生产 producer 的分类矩阵。
- 已确认服务启动顺序、四个常驻主循环和三个盘后/补推后台任务；已确认 L6 路由的逐 sink 重试及“任一失败即整体失败”语义。
- 已定位历史 catalog，但发现状态陈述可能过期；已改用生产调用图 + capability + preflight 三证合一判断“当前是否在推”。
- 已建立 presentation token 到 generic/counted/source-only/news-flash 四条发送路径的映射，并确认 CandidateTriggered 当前强制 fail-closed、LimitBoards 三展示共用一个 kind。
- 已完成 normalized source 六类事件的输入合同和 Earnings 默认 gate 取证；已定位三大生产循环和各类 dispatcher 调用位置。
- 已读完 monitor_loop 的两子循环骨架，并完成 PaperSell/A-01/归因/I-09/T-14~T-17/T-12/收盘禁用项的触发与重试语义取证；其余盘中段继续细化。
- 已补齐 G5b、快照新鲜度提醒和内存行情 AlertEvent 全部 fail-closed 的逻辑，继续细读 08:30/竞价/持仓/板块段。
- 已完成 AccountMode 补偿、P-03 结构可达性、09:10 预检、VirtualWatch 与三类 LimitBoards 的触发/失败状态取证；继续补竞价和纸面交易段。
- 已补齐 P-02/A-02/CandidateBoard 竞价重试、T-03/T-12 数据判定；发现 VirtualWatch 候选文本变量被固定为空，正在核验是否存在其他写入路径。
- 已确认 VirtualWatch 全文件唯一候选写入读取固定空字符串，判定常驻路径零生产；并补齐 PaperSell 默认启用、P-04、T0、盘中市场视图、I-03/T-03 周期与重试语义。
- 已逐行核对 T-01~T-09 核心 renderer，以及 AccountMode/DataMode 的持久状态、重试与副推语义。
- 已完成 I-01/I-02 取数与分类主逻辑、P-04 终态审计链/SQL 精确联接/逐票 counted 投递取证。
- 已补 I-02/I-03 的 LLM 增强与 fallback、T-14/T-15 事件合同、T-16 与两类大宗交易准入。
- 已完成 D-01 候选分类、可选 LLM、memo 与纸面买入副作用定位，并完成 A-02、R-09 的严格排名/来源绑定逻辑取证。
- 已完成 news monitor 的轮询窗口、阶段合同、公告受众、公告唯一归一化 owner、D-01/I-02 真实触发条件取证；继续核验 NewsFlash N-02 与各 source 分类 producer。
- 已确认 N-02 的 SourceOnly 流程独立于 selection-v2，且 NewsFlash 采用 authority→failure audit→reserve→专用 transaction→settle 的强顺序；继续检查 gate 是否会生成 N-01 reservation。
- 已由实现与回归断言双证确认 N-01 在当前 SourceOnly aggregator 中永不生成；只有 N-02 固定窗口 reservation 可生产。
- 已核对 T-14/T-15 全仓 source 注册点与 PolicyHit 分类器调用图：前者没有生产注册，后者没有生产 producer；并确认 D-01 的虚拟买入严格发生在推送成功之后。
- 已补齐 13 个盘后 ReviewTask 的依赖分区、19:00/21:00 时间门、重试状态机和 R-04/R-07/R-08/R-11/R-12/R-13/A-10/A-01 业务细节。
- 已纠正 CandidateInvalidated 状态：它由 CandidateBoard diff 真实生产；同时确认 CandidateTriggered 是时间、promotion 和 counted capability 三重不可达。
- 已完成 AttributionDaily/G5b 的“注释与实际状态推进不一致”核验，并补齐 SnapshotStale、P-01 输入合同与文案裁剪规则。
- 已完成独立 NotificationService 的 10 渠道、pipeline 单股/汇总调用、告警 helper 和返回值语义审计；发现 `Ok(false)` 被上层记录为成功、告警级别路由仅有注释没有实现。
- 已对账 BR-196 63-kind test manifest、65-kind runtime enum、58 presentation tuples、23 durable kinds，并核实 LaunchGate/默认飞书/L6 opt-in/dry-run 的真实可执行语义。
- 已进一步核对 runtime namespace：生产 dry-run 实际 fail-closed、测试 dry-run 强制开启；并确认盘后 Deduped 与普通 periodic 的状态语义不一致。
- 已安全读取当前非敏感配置开关：新闻窗口实际 00:00--24:00，`.env` 仅开启 Candidate live；据此将“默认状态”和“当前仓库配置状态”分开归类。
- 已只按变量名核对物理渠道存在性：monitor 飞书 target 已配置；独立 NotificationService 当前配置为邮件渠道；未读取或输出任何凭据值。
- 已核对 NewsFlash typed-receipt preflight；当前配置形态（飞书 CLI）满足其 transport 条件，但未进行任何会真实外发的验收。
- 已完成 65 项最终状态矩阵：37 当前有 producer、2 路径可达但新输入枯竭、2 默认关闭可 opt-in、24 禁用/阻断/无 producer；四类合计复核为 65。
- 已运行最终验证：monitor 完整默认并行测试 703 通过、6 失败、4 忽略；对应 BR-192 精确复跑 1/1 通过，BR-194 terminal replay 单线程 14/14 通过。独立 notification 模块单线程 18/18 通过。
- 审计工作完成；未修改产品代码、配置或数据库，仅在既有审计记录文件中补充计划、发现和验证结果。
- 2026-09-02：按盘前、集合竞价、盘中、盘后重新对账全部 65 个 PushKind；跨时段与无时段项单列，避免重复计数。
- 补查独立 NotificationService 后确认产业链报告在 09:05 和 15:30 各有一条真实定时发送路径，不受 PushKind/governor 治理。
- 发现并记录 09:10 行情预检同 P-03 一样结构不可达，以及 T-15 的注释盘后窗口与实际盘中调度位置不一致。
- 阶段分类最终集合差分：enum=65、classified=65、unique=65，missing/extra/duplicates 均为空。
- 补充确认产业链 09:05/15:30 定时报告会吞掉通知层 false/error 并让外层封日；已纳入盘前与盘后风险说明。
- 已开始把推送专项架构方案落入 Project Architecture Blueprint；确认 Markdown 是源文件，HTML 为自包含生成页，但仓库未保存生成脚本。
- 已完成 Markdown 源章节：新增 PROPOSED 状态、推送问题基线、目标运行链 Mermaid、模块落位、统一结果合同、跨库 finalizer、PhaseScheduler/readiness、兼容迁移和五阶段验收；原维护触发器顺延为第 25 节。
- 已同步 HTML 可见章节、PROPOSED 样式、34 章节/19 图统计、第 19 张 Mermaid 图、维护触发器编号，以及完整内嵌 Markdown。
- 重新按最初验收标准检查后，确认专项架构方案仍缺 65-kind 逐项业务清单；已继续补入盘前 5、集合竞价 7、盘中 22、盘后 31 共 65 行，并为每行加入业务规则、完成/重试语义和代码行证据。
- 已补入不经过 PushKind 的 09:05/15:30 产业链、CLI 单股/汇总 NotificationService、AlertManager helper 与 10 类 channel 语义。
- 已完成集合对账：65 行无 missing/extra/duplicate；状态为 37 ACTIVE、2 STARVED、2 OPT-IN、24 INACTIVE；所有 65 行均有存在且未越界的 `.rs:line` 证据。
- 已用受限机械转换器重建 HTML 第 24 节可见正文，并保留页面壳、19 张 Mermaid source/fallback、动态目录、状态样式与内嵌 Markdown；最终源 SHA-256 为 `8a1150524092089866095185259d6a615c7e0f8b1fff742292fc75e27e33df37`。
- Fresh 最终校验通过：34/34 章节、19/19 Mermaid 图、65/65 PushKind 行双端一致、135 组源码引用均存在且未越界、161 个 HTML ID 无重复、section/title/control/body 34/34/34/34、HTML 标签栈闭合、inline JS 可编译、旧章节数/旧 SHA 无残留。

# 2026-09-02 推送改动方案与周期落盘

- 已把 Codex 单独开发的人力基线写入蓝图第 24.18 节；不再用前端/后端/测试人数估算。
- 已写入乐观 8--10 日、计划 10--15 日、悲观 16--20 日三点估算，以及从 2026-09-02 起的日历映射。
- 已拆分 C0--C6 七个周期、W01--W21 工作包、M1--M4 里程碑，并明确依赖、退出门禁、提交/回退粒度和五类关键风险。
- 已同步 HTML 第 24 节、内嵌完整 Markdown 与三个 SHA 展示锚点；新 SHA-256 为 `bdca25d5597889e89d318ec6d72595a0547d507701f138eb88c09e5ba02f296f`。
- Fresh 校验通过：34/34 章节、19/19 Mermaid 图、65/65 PushKind 双端一致、158 组源码引用存在且未越界、第 24 节 19 个 h3/18 张表、161 个 HTML ID 无重复、section/control/body 一致、HTML 标签闭合、inline JS 可编译、内嵌 Markdown 字节完全一致。
- 路径纠正：两份蓝图已从仓库根目录迁移为 `docs/Project_Architecture_Blueprint.md` 与 `docs/Project_Architecture_Blueprint.html`，并加入 `docs/README.md` 当前执行入口；根目录不再保留重复副本。

# 2026-09-02 Grill 决策与 v18/v19 覆盖闭环

- 已按 Q1--Q55 确认结果重写 §24.13--§24.19：结果合同、business intent/finalizer、PhaseScheduler/readiness、activation manifest、纵向 MigrationUnit、shadow/promotion、Uncertain SLA、人工裁定、数据库/保留、故障矩阵、里程碑与最终退出标准全部落盘。
- 已废止 C0--C6 横向执行和 10--15 日基线；新的推送专项基线为 36--69 个 8 小时等效开发日、7--10 个交易周 rollout、约 10--16 周 Program Production Verified。
- 已审计 `docs/v18.x` 9 份与 `docs/v19.x` 6 份 Markdown，新增蓝图 §25 的全文档覆盖矩阵、v18 四模块部分吸收对账、v19 11 PR 逐项状态、设计冲突、与推送专项的 scope/周期边界。
- 已确认 v18 四核心类型与 v19 BannerSnapshot/ErrorCode/SignalTracker 等 8 个关键定义在当前 `src/tests` 中为 0；同时用现有 backtest、paper FIFO/order audit、performance/attribution、prediction_tracker、rate_budget、metrics、test isolation 证明 `PARTIAL` 而不是简单“全无”。
- 已将 v18.2--v18.5 标为实际 v20.x/v20.0 误归档未来提案；将 v19 57-kind catalog 和 v19.3 标为历史快照，当前推送事实统一以上位 §24 的 65-kind 审计为准。
- 已同步 HTML 新增第 25 节、维护章节顺延第 26 节、35 章节统计、可见修订说明、内嵌 Markdown 与 SHA-256。
- Fresh verifier 通过：35/35 sections、19 Mermaid、65 PushKind、173 组源码证据均存在且行号未越界；§24 为 19 h3/23 tables，§25 为 9 h3/7 tables；172 个 HTML ID 无重复，section/control/body 一致，HTML 标签闭合，inline JS 语法通过，embedded Markdown byte-exact。最终 SHA-256：`6c2a0c828e15329c17381c83d4f3a44cacf6fec210e95a38e2a099abee32d374`。

# 2026-09-03 方案审查

- 以工作树 + HEAD `a673043` 为采样基线，只读审查蓝图 §24--§25；未修改蓝图、产品代码或配置。
- 已对照现有 durable envelope/disposition/manual-resolution 强绑定、selection activation gate、monitor startup health 和 v18/v19 原始文档。
- 结论：架构方向可保留，但仍缺跨库完整状态机、可验证 terminal ref、activation/cutover/rollback 兼容矩阵、每 Unit pre-promotion test gate，不能按“Implementation-Ready”直接开发。
- 文档交付存在可复现性阻断：蓝图未跟踪、9 份被 §25 引用的 v18/v19 源文档未跟踪且被 ignore、生成/校验器不在仓库、W01--W21 估算明细从正式蓝图消失。
- 当前 `push_templates.rs` 并行改动已让旧行号证据漂移；例如蓝图 §25.8 的 `5714--5745` 当前落在 P-04，而 CandidateTriggered 已从 `5767` 开始。
- Fresh 静态核验：Markdown 35 个 h2、§24 19 个 h3、§25 9 个 h3、65 个 PushKind 行、273 个行号引用；HTML body SHA 与内嵌 Markdown均精确匹配 SHA `6c2a0c...`。`git diff --check` 无输出。

# 2026-09-03 当前架构蓝图同步更新

- 已执行 planning session catchup，并确认 canonical 蓝图现位于 `docs/`。
- 已确认 HEAD=`a673043` 及当前 dirty worktree；本次只修改已授权的蓝图产物和必要辅助，不触碰并行产品/config 文档改动。
- 已把上一轮用户批准的 P0/P1 调整矩阵登记为实施计划，进入 U1 基线阶段。
- 已完成 fresh Cargo/LOC inventory：61 modules、28 binaries、41 integration tests、514 Rust files、379,107 lines。
- 已确认只需重写基础 CURRENT 章节和 inventories，保留 §24/§25 已批准的专项/历史内容。
- 已完成 BR-255/BR-250/BR-178 改动归类，并记录其对 attribution、NewsAI 与 selection-v2 架构描述的影响。
- 已通读基础 CURRENT 章节并确认需要跨 §2--§7、§17--§23 和附录联动更新；同时确定可保留的深层业务章节。
- 已取得 61 modules、28 binaries、41 integration tests 的精确 current lists，并完成附录 A/B/C/H/I 漂移分析。
- 已核对 Cargo/build/README 和 data_gateway/market_domain 声明，确认 client-only 构建事实与需要隔离标注的源码注释债务。
- 已复核 monitor 为 4 main + 8 background，并发现 observability 章节关于 Prometheus 的潜在反向漂移，进入接线核查。
- 已确认 Prometheus 模块未接线、无 runtime exporter；已核对 CI 并发现 no-Magic 未接线及 compliance `e2e` target 漂移。
- 已完成 §10--§15 的 recent-change 影响映射和 current symbol line anchors。
- 已分析 HTML 生成结构与本机工具可用性；决定保留现有页面壳并新增可复现的仓库内受限 Markdown→HTML 同步器。
- 已完成 pre-edit stale scan 与 HTML 替换锚点分析，并发现 §24.10 也需同步更新 provider 拓扑前提。
- 已更新 Markdown header、状态标签、current inventory/热点以及 §2--§7 的四张关键架构图和对应实现约束。
- 已更新 §8、§10--§15：4+8 任务树、paper-sell 默认行为、BR-249/250/255、BR-178 authority skip、rejected retry、review backfill 与 closing valuation owner 均已落入蓝图。
- 已分段更新 §16--§23 与 §24.10：依赖方向、认证配置、remote lazy bridge、测试/CI、构建启动、扩展指南、ADR、治理禁令及推送专项前提全部改为 external provider-host/client-only 架构。
- 已更新附录 A/B/C/H/I 与 §26：61 modules、28 binaries、41 tests、40 consumer-used operations，以及 provider-host 外置、fixture-only server trait、CI/metrics/source-comment 已知债务均完成对账。
- 已新增 `scripts/render-architecture-blueprint-html.rb`，可从 Markdown 重建正文、目录输入、9 个指标、19 个 Mermaid source/fallback、内嵌 Markdown 和 SHA，并提供 `--check` 幂等门禁。
- 已完成真实 Chrome 验收：35 sections、19/19 Mermaid、117 个目录链接、BR-255 搜索命中、折叠/主题交互与零 runtime exceptions；后续只新增了维护流程段落，并用同一生成器重新同步。
- 最终验证通过：renderer `--check`；embedded Markdown/diagram sources byte-exact；HTML ids 唯一；137 个显式 path:line 引用 0 缺失/越界；2 段 inline JS 可编译；Data Gateway 15/15、进程隔离 8/8、gRPC catalog 1/1。
- 最终 Markdown SHA-256：`a1acf98ec960880934285d1a71ecf6fba068d809b08174ab51871a91645f2f75`。既有 dirty worktree 已保留，本次未改产品代码和配置。
- 已终止浏览器验收用 headless Chrome，确认 TCP 9333 无残留监听。

# 2026-09-03 本次文档目标符合性复审完成

- 以 HEAD `a673043` + 当前 dirty worktree 为 WIP 基线，完成 Standards/Spec 双轴独立审查和主流程复核；结论为“部分符合，不可按 Implementation-Ready 验收”。
- 已验证四时段 65-kind 分类完整且无重复（5/7/22/31；37 ACTIVE、24 INACTIVE、2 STARVED、2 OPT-IN），§24/§25 的主体范围没有漏掉最初要求的大类。
- 已证实逐行证据存在语义漂移：§25.8 的旧行号落在 PaperTrade、R-12 和其他 task 分发代码，当前 BR-232 实现锚点已移动到 `push_templates.rs:5771,7874,9261,9758/9766`。
- 已证实正式交付缺少 Q1--Q55 逐题追踪矩阵、W01--W21 估算明细、完整 CompletionPolicy/cross-DB/activation 合同；9 份 v18/v19 来源仍 ignored/untracked。
- Fresh verification：renderer `--check`、Ruby syntax、`git diff --check` 均 exit 0；`unified_data_architecture` 15/15、`tool_binary_process_isolation` 8/8、gRPC catalog exact test 1/1。
- 本轮只维护 planning 审查记录，没有修改蓝图、产品代码、配置或 v18/v19 正式文档。

# 2026-09-03 推送文档硬化设计固化

- 已从本机会话记录恢复 Q1--Q55 原始 Grill 标题与选择，不再依赖摘要猜测；连同 Q56--Q108 形成 108 行完整 decision record。
- 已创建 `docs/push-system/grill-decisions-2026-09-02.md`，明确 Q3/Q9、Q11/Q16/Q31、Q71/Q74、Q48/Q88 的 supersession 关系。
- 已将决策记录和文档硬化实施计划的标题、说明、表头、任务与验收描述全部改为中文；代码类型、状态枚举、命令、路径和固定文件名保持原样。

# 2026-09-03 真实账户持仓快照导入

- 已读取 `planning-with-files` 并执行 session catchup；未发现需要恢复的输出。
- 已按原始分辨率复核东方财富截图：截图时间 19:14，共 6 只持仓，证券市值合计 `45,921.00`，与账户摘要一致。
- 已登记总资产、当日盈亏、持仓盈亏、可用、可取，以及 6 只股票的市值、现价、成本、持仓盈亏和当日盈亏；进入正式 importer 与数据库状态核对。
- 已定位正式入口：`import_user_position_snapshot` 保存完整不可变持仓快照并对账 `stock_position`，`import_user_account_summary` 追加账户汇总。
- 已发现 `/private/tmp` 存在 2026-09-03 命名的三份候选 JSON；下一步将与截图、schema 和数据库逐项核验后再决定是否使用。
- 已确认持仓 JSON 的 6 个代码、数量、成本与截图一致，账户汇总 JSON 的 5 个可存字段也全部一致；未发现手工录入差异。
- 已核验当前附件 SHA-256；发现候选 real-account JSON 内的图片哈希与当前附件不一致，已拒绝直接使用该候选证据。
- 已确认生产库存在 WAL，后续将使用 SQLite 一致性备份而非文件复制。
- 已只读确认目标表 schema；首次把完整 integrity check 与查询合并执行超过 30 秒，已拆分为快速状态读取和可跟踪的独立完整性检查。
- 快速回读确认数据库已包含当日持仓快照 ID 25 和账户汇总 ID 27；为避免 append-only 汇总重复，本轮停止重复导入，转入明细、投影、real-account 证据和备份核验。
- 当日 6 项持仓明细回读全部匹配；第 7 个 open 投影为截图未出现的德展健康，按 BR-215 保留为未确认关闭，不擅自删除。
- real-account 当日记录尚未写入；其输入会因总资产与“证券市值 + 可用现金”相差 `16260.23` 而违反 schema，不会通过伪造数据绕过。
- 未找到同日导入前备份；准备在不重复导入的前提下补做当前数据库一致性备份。
- 已确认磁盘空间足够，且候选输入和当日数据库记录均形成于 19:21 左右；下一步检查数据库占用者后创建当前状态备份。
- 已确认 monitor 与 gRPC 市场进程正在使用数据库；不干扰现有进程，采用 SQLite 在线一致性备份。
- 已完成 SQLite 在线一致性备份：`data/private_evidence/2026-09-03/stock_analysis_after_20260903_position_and_summary_import.db`；该文件明确表示检测到既有当日记录后的状态，不冒充导入前备份。
- 备份以 immutable 模式完成完整 `PRAGMA integrity_check=ok`，并回读四组表计数一致；D3/D4 完成，进入最终源库精确差分验收。
- 生产库精确差分通过：当日持仓/账户汇总各唯一 1 条，6 项持仓双向差异 0，账户汇总字段全部匹配；当日 real-account 行为 0，未绕过总账差额门禁。
- 已记录备份和原图 SHA-256；剩余源库完整性与 open 投影最终回读。
- 最终 fresh 回读完成：生产库 `integrity_check=ok`，最新快照 ID 25，6 项双向差异 0；账户汇总 ID 27 且唯一；唯一 `unconfirmed_open` 为德展健康 `000813`。
- 备份 quick/full integrity 均为 `ok` 且 SHA 稳定；D1--D5 全部完成。未重复运行 append-only 汇总 importer，也未写入不满足 BR-103 总账约束的 real-account 候选。

# 2026-09-03 最近四日生产推送复盘

- 已将本轮分类为架构级只读复盘：先用生产事实校正方案，在用户批准前不改 RFC、蓝图或运行代码。
- 已完整读取 `brainstorming`、`codebase-design`、`DEEPENING.md` 和 `planning-with-files`，并执行 session catchup。
- 首次技能批量读取因调用参数语法错误未执行；已用简化调用恢复并在计划中留痕。
- 已定位生产推送证据源和主业务库相关表；首次广域数据扫描混入大量测试产物并被截断，已收窄为生产根与 2026-08-31 至 2026-09-03 日期窗口。
- 已统计最近四日 production dispatcher/event/review 行数，并确认 durable DB、immutable audit、push analytics 与持续增长的 monitor 主日志均有当日数据；进入 schema 与字段语义核对。
- 已取得 durable/analytics/business 关键字段和 JSONL 顶层结构；下一阶段将以 durable terminal 为成功口径，同时把 analytics、dispatcher 和业务完成状态作为交叉证据。
- durable 首轮统计完成：最近四日分别有 21/31/24/20 条权威 Accepted，全部形成 Delivered；同时存在 213/178/20/13 条 sink 前 RejectedDurable，暂无 Uncertain，review hydration 缺口为 0。
- dispatcher 首次聚合因 `jq` 运算符作用域错误失败，已记录并改用“单行验形后括号化聚合”。
- 已抽样确认两类语义污染：R-08 `retryable=false` 仍被全天周期性重跑，A-11 正常无数据被计入 `success=false`；这将直接影响新方案的调度和观测模型。
- 已完成 dispatcher 四日聚合：Disabled、NoData、未注册 provider 与 non-retryable failure 被重复执行数百次；durable Rejected 又缺可直接查询的 reason_code，现有“成功/失败”观测不足以支撑运营判断。
- 已对账 analytics 与 push-log：08-31 至 09-03 分别记录 44/410/160/69 次 legacy pushed=true，主要由 09-01 PaperSell 254、NewsToIdea 120 和 09-02 PaperSell 131 构成；真实风险集中在少数高频非 durable 路径。
- 已确认高峰不是 event_id 重复：PaperSell/NewsToIdea 按单条业务记录逐条物理发送；同时发现 G5bAttribution 三个日期各有一次相同 event_id 二次 pushed=true。现有 65-kind catalog 无法覆盖全部实际 emitter。
- 已量化分钟级洪峰（PaperSell 40/min、NewsToIdea 20/min），并证实 `pushed_stocks` 业务记录与物理候选不一一对应；现有方案必须明确事实分层，不能再用一张表或 bool 兼任业务与投递状态。
- 已聚合 review audit，发现 R-03/R-08 各 80 次重复失败；R-08 在底层标为不可重试、上层却标为可重试，R-03 等待账户输入却按周期错误重跑。调度/失败分类需要集中到单一深模块。
- durable 健康度核验完成：96 个 Accepted 全部闭合，424 个 sink 前拒绝全部闭合，23 个业务 hydration 全部 Applied，最大耗时 217.131 秒；强投递核心可复用，但拒绝 reason 无法查询。
- legacy 时段统计完成，确认消息洪峰集中在竞价/盘中；durable 首次时段分类暴露 UTC 处理遗漏，已作废并转入统一时区重算。
- durable 已按 Asia/Shanghai 重算：96 个权威 Accepted 中盘前 1、集合竞价 0、盘中 71、盘后 24；`HoldingPlan` 实际落在 09:45--10:06。E2 的时段结论已校正，下一步核对蓝图/RFC 的计划分类是否与实际调度漂移。
- 已核对蓝图 §24 与 `docs/push-system/`：HoldingPlan 的盘中归类正确，但首批迁移顺序没有反映 PaperSell/NewsToIdea 的真实洪峰；精确 RFC、机器 catalog 和 evidence manifest 仍只是实施计划中的待办，尚无可冻结的正式规格。
- 已定位三条高风险代码 seam：PaperSell 对扫描结果逐票发；NewsToIdea 在已发送后若虚拟买入失败会返回 false 且不写 memo；G5b 持久化后逐条发送但无论送达结果都累计 done。下一步用物理日志和业务状态验证实际重复/完成模式。
- 已开始追查 G5b 重复明细与 push-log；首个 SQL 使用了错误表名，已由 `sqlite_master` 校正为 `push_analytics` 并记录，不改变已有统计。
- G5b 重复已由 analytics+正文交叉确认：同一业务标的在数秒内产生不同 LLM 文本并连续发送；08-31 还有 `TEST_CODE_000001` 测试告警实际进入 Feishu。方案需新增生产 namespace fail-closed 与业务 occurrence/topic 聚合，而非仅正文 hash 去重。
- PaperSell 已与业务库精确闭合：09-01 的 254 推送对应 254 个唯一 sell/Filled/code/plan，09-02 的 131 也完全一致；问题是逐笔事实直接映射为逐条用户消息。NewsToIdea 09-01 的 120 条只覆盖 66 个唯一标的，继续按新闻/标的/evidence 分组。
- NewsToIdea 的 120 条全部有完整四步持久交付链，但同一标题+标的被赋予 2--4 个不同 evidence/identity 后合法送达；应修 source occurrence canonicalization，而不是在 sink 层按正文或 receipt 去重。
- NewsToIdea 根因已由表数据定位：相同 `source_item_id + target_code` 因每轮 `source_batch_id` 不同而生成不同 source/evidence/delivery identity；E2 的真实时段、发送、重复与抑制分类完成，进入 E3 完成状态和游标对账。
- 源码确认 NewsAI 的 batch 被写进 assessment ID，随后 delivery identity 又等于 assessment ID；这不是 sink 缺陷。Review 则在 `account_metrics_incomplete` 构造点硬编码可重试，解释了真实的重复调度。
- Review 状态机已有 Terminal/Waiting/Retry/Deferred 基础，不需要重写；缺口是统一的失败处置与事件唤醒语义。原 36--69 人日又缺正式 W01--W21 明细，且未计入本轮新增的四个高风险工作面，必须重新基线。
- 已合并两套不重叠的发送证据：四日共 779 条可证用户可见消息，盘中 675（86.6%）；PaperSell+NewsToIdea 占 559（71.8%），生产风险排序已有明确数量依据。
- E3 对账完成：PaperSell/NewsAI/durable 各自的业务完成证据已闭合或明确指出身份问题；selection completion 窗口为空；09-03 real-account readiness 缺口解释 R-03 应进入 BlockedOnInput；G5b 仍无 authoritative completion。
- 代码/数据补强了三个 completion 设计：PaperSell 是成交先提交再通知；G5b top-N 目前无去重/namespace；账户事件唤醒必须按 canonical 内容代次，因为 09-02 已出现 payload 相同但 evidence hash 不同的重复快照。
- 已量化可验证的降噪上限：NewsToIdea 09-01 从 120 条按稳定 source-target 去重可到 61，按 14 个 source item 摘要可到 14；PaperSell 可按明确 scan_run 聚合，避免 40 条/分钟洪峰而不删除任何成交审计。
- E4 已形成三条路径；推荐“安全止血 + 薄 Foundation + 风险纵切”，保留 durable/atomic-unit 主架构，只把 namespace、News identity、Paper summary、G5b、FailureDisposition 提到首批。临时全量包络调整为 41--78 工程日，待 exact occurrence catalog 后重基线。
- 已定义基于四日生产 corpus 的 promotion 门禁，并在 20:38 fresh 复算确认 779 条总量不变；live review/dispatcher 继续增长，R-03/R-08 已各达 81 次。E5 建议完成，正式 RFC/蓝图修改等待用户批准推荐路径。
- 已创建 `docs/push-system/push-documentation-hardening-plan.md`，按 source governance、RFC、catalog/evidence、蓝图拆分、离线 HTML、checker/CI、最终验收七个任务拆解。
- Fresh self-review：decision rows=108、unique=108、missing=[]、duplicates=[]；`git diff --check` 通过。
- 依据 Q70，当前共享 worktree 不做 stage/commit；正式 spec 发布仍等待干净 Git baseline。
# 2026-09-06 第二批推送事实底座收尾

- Task1–Task7完成并通过最终Spec/Quality审查；分支HEAD `767a76e`，未合并、push或部署。
- Fresh验证：source 12/110、catalog 32/355、8个Ruby语法、check-sources、draft、render freshness通过；clean strict仅两个PROVISIONAL。Rust/Cargo相对07781bf零diff；原工作区160 unmerged及四份保护样本指纹不变。
- 最终制品：65 kind（36/22/5/2）、102 producer、52 Unit、195 evidence/33 Rust证据文件、469 frozen files。目录纳入replay-force，业务scope明确排除但不否认真实ops webhook。
- 已同步docs/push-system入口、第二批结果、未完成运行时/文档/发布三层清单和单开发者人力/待重算周期边界。
# 2026-09-06 第三批RFC/WBS启动

- 用户要求继续；已选择先完成正式实施RFC/WBS，不在160冲突主工作区直接做运行时Foundation。
- 已完整读取executing-plans、brainstorming、using-git-worktrees、subagent-driven-development、codebase-design、writing-plans、planning-with-files；现有108项批准设计和用户继续授权满足设计门，按SDD执行。
- linked worktree `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905` / `codex/push-reliability-20260905` 干净；刚完成source12/110、catalog32/355、draft/render和Rust/Cargo零diff，可作为文档批次基线。
- 旧硬化Task2过大且隔离分支缺RFC的蓝图/最近证据输入；已裁决第三批增加不可变输入治理并拆分合同/持久化/运行门禁/WBS/验证。正在编写可执行计划，尚未修改RFC或运行时代码。
- III1代码/目录预检完成：worktree仍干净；确认现有23-kind counted/14-state durable合同、`Accepted/Rejected/Uncertain`权威sink结果、65-kind业务目录与52个Unit之间的真实边界。下一步按此边界落第三批执行计划。
- 第三批计划已写入`docs/push-system/implementation-batch-3-rfc-wbs-2026-09-06.md`并提交`0bc6a2e`；计划复核最终Spec/Quality PASS，Critical/Important/Minor均0。审查修正了COMPAT结果、26→23/39映射、同库事务、保留期、W01重建、Q44顺序、WBS可复算字段和strict原因码。
- III2 Task1已按SDD派发：冻结八份蓝图/最近推送输入、建立SHA manifest和只读CLI门禁；当前未改Rust、生产库、配置或消息通道。
- III2 Task1完成：8/8输入、1,221,011字节与原件/brief/manifest/staged blob一致；输入测试12/238、来源12/110、真实CLI和语法通过。提交`bb3eccd`经独立Spec/Quality审查通过，Critical/Important/Minor均0；11处原Markdown行尾双空格保留原字节并由SHA管理。
- 计划Task1勾选提交`7143065`；III3 Task2已派发，brief明确四个治理SHA、类型最低字段、26→23映射、14 durable状态、COMPAT分栏、CompletionPolicy与ReasonCode最低覆盖。
- III3 Task2初稿提交`1280baf`后，独立审查发现两项Important：RFC自然语言未中文化、校验器会放过四类关键语义反转。修复提交`d6f516d`已将中文正文与结构化合同同时落盘，intent身份、AlreadyTerminal精确绑定、P01/N02规则、run_id canonical参与方式均有公开CLI mutation反例。
- Task2最终四套测试71/1165全绿，Rust/Cargo相对`07781bf`零差异；限定复审Spec/Quality PASS且Critical/Important/Minor均0。计划勾选提交`7e47af1`，III4 Task3已按增强brief派发，开始实现可执行SQLite DDL、三类状态表、七步跨库恢复和崩溃矩阵。
- III4 Task3初稿`5713b38`经独立审查发现4个Important/1个Minor；修复`f226d0f`补齐非发送条件组、状态/ReasonCode绑定、既有schema fail-closed和TEXT/NUL/身份约束。四套101/2161全绿；限定复审另跑19/675，Spec/Quality PASS且0 finding。计划状态提交`d763d5f`，III5 Task4以此为BASE。
- III5 Task4初稿`824da74`完成调度/readiness/shadow/activation/operator/retention/六门禁与实际样本合同；独立审查发现1个Important：ScheduleOccurrence缺显式版本模型。修复`507512a`补齐`version:u64`、typed transition request、原子CAS/冲突重读和直接mutation，四套245/3585全绿；限定复审Spec/Quality PASS且0 finding。计划状态提交`246b931`，III6 Task5以此为BASE。
- III6 Task5初稿`846fa1b`建立W01–W21、52 Unit逐项WBS、公开validator与幂等renderer；独立审查发现1个Important/2个Minor，修复`edbb509`将未经批准的CLI-chain移出Q44 rank1、改用BigDecimal/Rational精确half-up并稳定非法ID路径诊断。五套312/4011全绿，限定复审Spec/Quality PASS且0 finding。计划状态提交`d45dada`，III7 Task6以此为BASE。
- WBS重算：Foundation 281.34h + Unit 547.65h = 828.99h；20%缓冲一次后994.79h/124.35个8小时工程日。生产侧另列42个owner-changing Unit、42晋级+76观察=118个保守串行session，外部等待63工作日；旧36–69工程日/7–10交易周仅保留历史对照。
# 2026-09-06 第三批 RFC/SQL/WBS 最终完成

- 隔离分支 `codex/push-reliability-20260905` 的第三批最终状态提交为 `aad7ac16eef4007daedd62c100597bd10ea62e5b`；最终状态仅修改 README、结果和第三批计划三份文档，status-only 独立复审 PASS（0/0/0）。
- 最终 Spec 审查覆盖 Q1--Q108 与硬化计划，PASS（0/0/0）；最终 Quality 唯一残留的 CI 非执行绕过经三轮修复关闭，wave3 scoped Quality PASS（0/0/0）。原 NotDelivered、四级里程碑、Q4 外部兼容、55 行 trace、activation reason、README link 和 CI gate findings 全部关闭。
- clean `e35800a` 的五套完整文档测试为 418 runs / 5023 assertions，全部通过；RFC strict 仍精确四个发布阻断，catalog strict 仍精确两个 provisional，不把规格完成写成 Implementation-Ready。
- 当前规格基线仍为 65 kind / 102 producer / 52 MigrationUnit / 195 evidence；WBS 为 828.99h 基线、994.79h / 124.35 工程日（20% 缓冲）、42 promotion + 76 observation = 118 conservative sessions、63 外部等待工作日及 428 条件自然日。
- 本批未改 Rust/runtime、生产数据库、配置、模板或 workflow；未实现 W01--W21、未迁移/晋级 52 Unit、未生成 RFC HTML/统一 checker、未部署或取得真实 TransportAccepted。未 merge、push、deploy；隔离 worktree 保留。
