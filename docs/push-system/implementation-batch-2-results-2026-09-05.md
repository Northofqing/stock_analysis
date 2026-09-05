# 第二批推送可靠性：来源、目录与源码证据交付记录

> 当前仍在实施：Task1–Task6的来源、工具和源码业务审计已通过独立复核；Task7正在统一生成机器目录、证据manifest与中文视图，本报告尚未完成整批验收。不是Foundation Ready、生产上线或全量方案完成声明。

## 范围与边界

- 实施分支：`codex/push-reliability-20260905`，保留隔离worktree，不自动合并、推送或部署。
- 本批起点与Rust证据基线：`07781bf386aafdf202851ae928efee8920387058`。它包含首批R08错误分类和告警/G5b输入隔离修复；本批不改运行时Rust。
- 原工作区的67项混合源报告与本分支65项枚举分开。PaperBuy/Watchdog没有移入本分支，不能用原报告行号充当这里的已验证证据。
- 本批只落实已批准的来源治理、可校验目录与稳定源码证据前置；完整RFC、WBS重算、离线HTML、CI、运行时Foundation及交易窗口晋级另有后续门禁。

## 已通过的六个任务

### Task1：不可变设计来源

九份v18/v19设计与108题批准决策表已按原字节导入，controller逐文件比对原件/目标字节及SHA一致。具体来源、自声明标题/版本/状态和裁决见[来源目录](../../design-source-catalog.v1.json)。

v18.2–v18.4文件自声明v20.x，v18.5自声明v20.0，记录为版本标签冲突而不是擅自认定“放错目录”。历史57-kind模板快照保持历史身份，不作为当前目录。旧正文中的退役规则引用原样保留，活跃README仅补充当前适用范围说明。

`SourceCatalog.validate(root)`及公开CLI校验文件哈希、相对路径/真实路径边界、schema、重复ID/path、完整Q1–108覆盖。独立复核发现的NUL路径及无效UTF-8异常已通过边界校验和CLI回归修复，没有用捕获所有异常掩盖错误。

提交：`53f3c87`、`f30c4a4`。误跟踪的临时报告在`f30c4a4`取消跟踪，磁盘原件保留；未清理历史来源或数据。

### Task2：目录、定位和渲染工具

工具提交`07da502`、修订`435fa91`。能力包括：

- 全src Rust及Cargo文件集合、基线Git与当前字节双重核对；文档提交不要求重置代码基线。
- 完整函数/枚举/模块标识符定位、impl完整头定位、原始item哈希及派生行号；符号歧义失败，不用签名片段挑选同名函数。
- kind、producer、MigrationUnit双向引用及共享完成状态归属核对；人工业务语义仍需另行源审计，工具通过不证明语义正确。
- draft只跳过PROVISIONAL和非ignored脏状态；来源、枚举、文件、符号、哈希或引用漂移仍然失败。
- Markdown只在显式`--write`时写指定输出；`--check`只读。输出symlink（包括悬空链接）和硬链接被拒绝，防止覆盖其他文件。

独立复核发现并关闭了悬空链接写出根目录和签名片段绕过同名歧义两项Important。词法定位器不展开宏、不替代Rust编译器，enum提取限定fieldless PushKind形状；无法可靠定位的形状必须失败。

### Task3：65-kind入口与无caller清单

工作表提交`32d4b02`，审查修订`10b38e9`。源码枚举与主表机械比对为65行、65个唯一kind、无缺失/额外/重复；15个无生产caller项均按全`src`反向搜索，排除了枚举、metadata、renderer、适配器、fixture、测试和smoke命中。monitor/news/P01自动及补偿、`--push`、复盘自动/手动/补推、CLI单股/汇总/产业链及09:05/15:30链路均已列入；仅保存文件的市场复盘与无调用方的legacy alert明确排除。

独立复核补出了两项重要边界并已关闭：SnapshotStale除15:10定时器外还会在服务启动时跨时段执行；PaperReview午盘today与“已完成交易日、exact T+1”条件冲突，补齐历史快照也不能恢复该入口。另澄清只有R04手动复盘可绕过21:00门，R07仍需等待。Task3只冻结入口与初判，新闻、状态驱动、复盘owner仍分别由Task4–Task6核验，不能将初判当成最终目录。

### Task4：新闻、来源事实与持久发送边界

新闻章节提交`5c91eb8`，精度修订`d15f35b`。共审计13条producer/分类分支，另保留PolicyHit无producer裁决；形成10个主完成键域、9个候选MigrationUnit和5个具体未决。独立复核无Critical/Important，并关闭了公告legacy回退措辞与L4键域派生关系两项Minor。

关键结论是：P01自动与补偿共享`p01:{business_date}`/GLOBAL持久claim，但补偿不能接管Scheduled模式留下的Reserved信封；普通D01与NewsAI虽然共用NewsToIdea kind，却使用不同的完成状态，NewsAI当前没有进入BR-192 counted coordinator；N01没有权威强度源且没有生产Critical reservation，源码也没有证明它与N02共享日配额。Announcement、D01/I02、Earnings/Analyst均存在上游状态在通知完成前推进的风险，工作表已逐项记录重试与不确定边界。

### Task5：状态驱动、竞价/盘中与交易边界

状态章节提交`46f24e9`，修订`ce581b5`。共核对37条producer/入口形态，形成25个候选边界族/迁移单元和5个具体未决。独立复核发现并关闭了AttributionDaily/G5bAttribution虚构L4冷却和producer计数错误；两者实际只有各自外层日期门，`cooldown_secs=None`不会读写L4冷却。

源码还证实：DataMode可用`EstablishedSilently`推进`LATEST_DATA_MODE`，发送失败会清pending；独立`--push`进程在Account/Data banner初始化前运行并退出，使I01/I02/I03/D01/HoldingPlan手工入口接线存在但不可达；PaperSell先落业务成交再通知，失败后当日成交防重会阻止自然重建；候选失效结果、快照与外层双dispatcher不原子；Attribution/G5b、15:05快照警告、午盘PaperReview及板块timer均存在不按发送确认推进状态的路径。Task6已明确接收枚举外CLI和09:05/15:30产业链timer。

### Task6：复盘、补推、side-route与枚举外入口

复盘章节提交`ed1a702`，修订`c0a4029`。13个ReviewTask状态为7 ACTIVE、4 INACTIVE、2 STARVED；形成21个审计边界族、17个候选Unit和6个具体未决。独立复核发现并关闭了R08 typed retryability、A10来源时间错误分类、产业链timer非交易日范围、BlockTradeConfirm真实L4 kind及定位精度问题。

关键校正包括：自动scheduler也构造`at_manual`，使R04自动19:00路径绕过21:00门，而R07仍等待；R12因技术K线能力常量为false而禁止新producer，但既有durable decision仍可独立恢复；backfill只扫描8个counted任务且A01明确排除，每个单任务batch仍会执行Block Trade/IPO side-route；BlockTradePriceRange上游固定传None而下游必填，当前恒拒；产业链09:05/15:30 timer没有交易日guard，发送false/Err又被内部吞为Ok，可能跨calendar date重复发送同一business date。单股/汇总/产业链CLI的文件保存和bool返回均不是durable receipt。

## Task3–Task7及整批验证（尚未完成）

首次真实审计确认65个enum、467个src Rust文件加两份Cargo文件及16组源码事实，并发现旧范围未列出的`--push`手工入口与P01补偿入口。实现者拒绝用概括性owner或占位producer生成目录；现按入口/no-caller、新闻、状态驱动、复盘四个切片审计，Task7才统一生成目录、完整manifest和Markdown。这些机械数量不等于已完成生产者审计或部署验证。

| 已执行验证 | 结果及对应版本 |
| --- | --- |
| 来源CLI测试 | f30c4a4：11 tests / 91 assertions，通过 |
| 工具CLI测试 | 435fa91：25 tests / 307 assertions，通过 |
| Task1/Task2独立及修订复核 | 无未关闭Critical/Important；真实业务目录属于Task3 |
| 真实根目录draft/strict/render | Task2结束时按阶段预期报告两个JSON缺失，不能称为目录通过 |
| 首次真实审计 | 65 enum / 469 scope files / 16组直接源码事实；未生成成品、无提交，剩余边界已拆分 |
| Task3入口/无caller清单 | 10b38e9：65/65；15个无生产caller；修订复核PASS |
| Task4新闻边界 | d15f35b：13条producer/分支；10个完成域；9个候选Unit；5个具体未决；复核PASS |
| Task5状态/交易边界 | ce581b5：37条producer/入口；25个候选边界族/Unit；5个具体未决；复核PASS |
| Task6复盘/枚举外边界 | c0a4029：13 task（7/4/2）；21边界；17候选Unit；6未决；复核PASS |
| 整批最终验证/最终review | 待Task7完成后执行 |

Task1留下一个非阻塞Minor：不存在的root目前诊断为`catalog_missing`，不影响失败退出，但诊断可更精确；交给整批review最终定级。

## 本轮裁决与代价

1. 先完成来源/目录/证据前置，再做后续运行时迁移：Q29/59/91及Foundation先行约束要求精确边界；代价是本批不直接改善线上通知行为。
2. 冻结全部src Rust与Cargo，而不只冻结引用文件：防止新producer藏在新增文件中绕过检查；代价是无关源码变化也需重新审核证据。
3. 修正SectorTop/SectorAnomaly“共享timer”的初稿错误：实际各有状态；代价是不能直接沿用旧家族分组与工期估算。
4. v19.0导入时仅机械移除工具多加的末尾LF：原文无LF，先断言目标等于原文加LF再处理，源文件不改；代价是多一步需核验的格式归一化，最终SHA必须与原件一致。
5. 保留两份历史来源的六处既有行尾空格：原字节约束优先，未修改全局whitespace配置；代价是整批裸range diff检查保留六个明确诊断，其他文件仍须无格式错误。
6. impl符号使用仅折叠空白的完整声明头：防止同类型不同impl被混用；代价是复杂未支持形状需换用精确函数证据或后续扩展，不能猜测定位。
7. 将原Task2拆成工具Task2和真实目录Task3顺序复核：分开验证工具正确性与业务判断，原要求全部保留；代价是多一次任务交接/复核，真实目录交付前不得宣布本批完成。
8. 不允许用概括性owner或占位producer填满65项：按剩余风险增加入口/no-caller、新闻、状态驱动、复盘四个审计切片，再由Task7组装；代价是增加顺序任务与独立复核，并多一份可追溯审计工作表，Task7通过前本批仍未完成。
9. P01自动与补偿按同一业务日claim进入同一迁移单元，但保留render mode和Reserved恢复限制：避免把同一日事件拆成两条通知；代价是补偿不能盲接自动模式的不确定信封，必须先对账或显式恢复。
10. 普通D01与NewsAI即使共用NewsToIdea kind也拆为不同迁移单元：前者是进程内memo/L4，后者是assessment identity与追加式delivery event；代价是目录和迁移要维护两套完成语义，NewsAI在接入真实TransportAccepted前不能宣称可靠送达。
11. N01与N02按当前源码分开额度和完成域，不落实设计中的“共享配额”假设：N01没有生产Critical reservation，N02只有窗口状态；代价是未来接入权威强度源或共享额度时必须重新审计并修改目录，当前不能提前复用N02活跃性证明N01可用。
12. 将`--push`盘中手工入口登记为“已接线但当前受阻”，不按dispatcher存在判为可用：新CLI进程在banner初始化分支之前退出；代价是后续若要恢复手工推送，必须先重构启动上下文并重新验证其与定时入口的完成owner。
13. DataMode的模式确认状态与通知完成分层记录：`EstablishedSilently`和失败后清pending都不能作为送达证明；代价是运行时迁移不能只复用`LATEST_DATA_MODE`，需要独立的通知attempt/receipt/恢复状态。
14. AttributionDaily与G5bAttribution只登记实际外层日期门，不虚构L4冷却：`cooldown_secs=None`直接放行且不读写冷却；代价是后续可靠化需要显式新增持久通知owner，不能依赖现有L4补偿重复或漏推。
15. 以实际调用参数裁决复盘时间门，而不按入口名称区分自动/手动：自动attempt同样使用`at_manual`，所以R04在19:00可提前，R07仍等待；代价是修复时必须先决定这是有意策略还是接线错误，并避免一刀切取消manual override。
16. 将R12与BlockTradePriceRange按当前能力门/必填输入判为INACTIVE，同时保留既有R12 decision恢复：避免把潜在dispatcher当活动producer；代价是恢复历史信封和创建新通知必须作为两个不同能力测试。
17. backfill只承认源码中的8任务白名单，A01不虚构历史扫描入口；单任务batch仍执行Block Trade/IPO side-route：代价是后续要隔离side-route副作用，否则一次补推扫描可能重复触发与目标task无关的通知。
18. 产业链09:05/15:30 occurrence同时保留business date与calendar date，显式记录无交易日guard和`Ok(false/Err)`封日：代价是可靠化需新增持久cursor、交易日门和不确定发送恢复，不能复用报告文件或内存日期状态。

## 保护与未检查项

原目录源码差异指纹、160个unmerged索引项及08-31 alerts/G5b历史文件在Task1闭环后复核未变。隔离区09-05事故参考件也与本批开始时一致；这不撤销[首批样本保全事故](implementation-batch-1-results-2026-09-05.md)的原结论，不再将其称为未变的原始快照。

格式例外仅为v18.4第219/278行、v18.5第109/138/142/433行原有空白，并由来源SHA防止扩大。最终检查须分别记录这六处历史诊断与新增文件结果。

NOT CHECKED：完整RFC / WBS / 离线HTML / CI / 运行时Foundation / 部署 / 真实接收 / 全套Rust测试。旧二进制读取首批新gateway_source审计标签的兼容回退限制仍有效；本批不提供上线批准。
