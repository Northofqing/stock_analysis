# Architecture Blueprint 重制发现

## 2026-09-05 第二批前置调查

- 当前隔离源码验证SectorTop/SectorAnomaly各自使用last_sector_top/last_sector_anomaly（monitor_loop约10998/11016），不是共享timer。已修正第二批初稿，completion owner必须包含真实状态标识和key范围，不能按同函数粗分组。
- main.rs直接NewsToIdea引用中的7050/7094属于push_e2e_news_modules测试fixture；真实D01在news_monitor_loop重要公告分支调用dispatch_news_to_idea_daily。机器匹配引用不等于生产可达性，目录审计要防止这种误计。

- 当前隔离源码07781bf与原混合工作树不同：两个新增PaperBuy/Watchdog尚未移入，不能直接把67-kind临时报告当成65-kind干净开发基线目录。
- 已批准Q29要求精确目录后才能冻结估算，Q59/Q91要求path+symbol+baseline/hash身份，Q64要求漂移失败；原文档硬化计划相应Task1/Task3/Task6仍未完成，是运行时Foundation前置。
- 现有DeliveryEnvelope的decision_identity包含rendered_content_sha256，是既有durable层合同；Q89要求新的业务intent identity不含payload hash，不能直接改旧identity破坏回放兼容。这项运行时合同留待完整RFC/intent切片，不顺手改coordinator。

## 2026-09-05 首批隔离开发发现

- 已获得“允许”，独立开发区为.worktrees/push-reliability-20260905，分支codex/push-reliability-20260905；下方预检的等待许可状态是历史记录。
- R08现在从GatewayError保存机器语义到ReviewTaskOutcome及ReviewTaskTransition，不再依据诊断文字决定必需CFFEX失败的retryable。永久终止只适用于当前调度实例；旧二进制不认识新gateway_source，发布前须准备兼容回退制品。
- 新告警记录显式origin；LegacyUnknown只表示兼容旧档案，不提供生产来源认证。生产读取、G5b选取、模型调用及落盘均拒绝Test/TEST_CODE；默认归档I/O同时利用测试进程与TradingEnv隔离。
- 开发区原始样本保全发生执行偏差，不能用后续测试前后哈希稳定替代最初的保真。完整经过和两项裁决落在docs首批结果中；原目录用户源代码及历史数据独立核验未变。

## 2026-09-05 实施预检

- 用户授权已从分析变为解决问题。既有approved合同继续有效；开发授权不等于处理用户所有Git冲突或生产晋级。
- git-dir/common-dir相同，当前不是隔离worktree；branch=master。160个unmerged索引项仍在，相关核心文件存在多个stage，但src/tests中无行首冲突标记。不能把“未合并索引”直接推导为“已证实编译失败”。
- 已有worktree属于其他任务或prunable，不擅自复用/删除；可推荐从a673043创建本次专用分支，并选择性迁入已核对的推送改动。需要用户同意该基线/隔离选择。

## 2026-09-05 重新分析：采用本节及 docs 报告纠正旧推断

- 已交付 `docs/push-system/comprehensive-reanalysis-2026-09-05.md`（463行），67项视图、约406KB的去正文聚合证据、只读取证脚本与校验脚本。
- 五日最终口径：analytics true799（含N02八条）；durable按Accepted时间119、按business date Delivered116（三条历史补推）；混合本地声明918不等于用户收到。443个RejectedDurable不是443次远端失败。N02按accepted_at纠正源时间混入时段统计。
- 全量匹配408卖出+29买入卡，共437个唯一Filled；9/1 fill到发前日志最大1130秒，中位位置值535秒。先整盘扫描后返回Vec再发卡是源码机制；通知恢复仍缺独立outbox。
- R-08五日259次dispatcher失败，256invalid_evidence、3no_verified_batch；review102条都retryable=true。源码将GatewayError字符串化后固定failed(true)，需保留typed retryability；R-03同样102条但属于固定缺能力，不是现金截图问题。
- CandidateBoard当前确认空候选提前return，失效diff不执行；非空又先persist快照后发送，是两个不同失败点，应分别做fixture。
- 校验器检查67项、119+8回执记录、437笔唯一匹配、21个源码SHA/设计来源SHA及文档链接；没有执行冲突工作树Rust构建或远端收件箱验证。

- 09-04 paper_buy=29、paper_sell=23、NewsAI=38，Watchdog 09-03/04 各一条，本轮不再沿用旧截止。08-31 typed Accepted 含三条较早业务日补推，须按 accepted_at 与 business_date 分栏。
- NewsAI assessment_id 以 batch/item/target/version 形成；content_hash 是完整评估 hash 不是新闻正文 hash。跨批同新闻同股票的多条评估是重推候选，不可直接规定 120 条应压到 14 条。
- R-03 account gate 由 push_templates.rs:9799 的无条件循环生成 AccountMetricsIncomplete，不读取今日账户快照，因此补录持仓不等于修复 R-03。
- Watchdog 当前按原调度内注册，发送结果无条件 mark_fired；review Err 也 satisfy，与新闻分支仅 Accepted satisfy 不一致；09-04 review 表逾期至22:45才满足却无 fired，不能证明哨兵覆盖全天。
- generic CLI 发送并非只看 exit=0：它校验 message_id/platform_msg_id，但回到上层压成 bool 丢失 receipt identity。NewsAI 本地 hash-chain 不能单独升级为 typed TransportAccepted。

- HTML 嵌入 Markdown 与磁盘 Markdown 逐字节一致，SHA 为 a1acf98e…；但仍从 jsDelivr 加载 Mermaid，因此“文本自包含”不等于 Q60/Q68 的完全离线从零构建。正文 35 个二级章节（含附录）。
- 未冲突的当前 `notify::PushKind` 新增 `PaperBuy` 和 `Watchdog`；固定 65 的验收已过时，需要用 HEAD/工作树集合差分生成数量，并为新项注明未验证部署。PaperSell/NewsToIdea 本来就在旧 65 中，上轮说“65-kind 漏掉两者”不准确，真正遗漏的是同 kind 的多 producer/完成 owner。
- 上轮把 legacy pushed=true 加 durable Accepted 合称779条“用户可见消息”，越过证据强度；必须复查 dry-run/weak sink、authority records 与业务时间。不能把 Delivered 终态直接叫用户已读。

- 当前 HEAD 仍为 a673043，但有 160 个 unmerged 路径。蓝图 09-03 声称“当前工作树/Implementation-Ready”，已不适合解释当前混合源状态。CLAUDE.md 明确只是项目上下文，无额外开发门禁；未找到适用 AGENTS.md。
- docs/push-system 目前只有 108 项 Grill 决策和文档硬化计划；RFC、catalog、manifest 均尚未交付。用户先前已要求文档落 docs，本轮将直接形成可审查复核报告，不以助手上一轮自设的确认门阻断。
- 本地已有 09-04/09-05 dispatcher/event 数据，可扩展旧四日窗口。蓝图 Markdown 2057 行、HTML 4659 行，需要验证嵌入文本哈希后逐节复核。

## 真实持仓截图（2026-08-31 20:42）

- 账户汇总：总资产 70,856.42；证券市值 51,904.00；可用 18,952.42；可取 18,929.77；持仓盈亏 -22,019.00；当日盈亏 +181.65；仓位 73.3%。
- 德展健康：数量 1,650；现价 3.300；成本 9.408；持仓盈亏 -3,054.14（-64.924%）；当日盈亏 -85.00（-4.899%）。
- 利欧股份：数量 9,500；现价 4.750；成本 7.231；持仓盈亏 -4,961.88（-34.311%）；当日盈亏 +300.00（+3.261%）。
- 合肥城建：数量 4,348；现价 10.870；成本 15.930；持仓盈亏 -2,024.15（-31.764%）；当日盈亏 +23.81（+0.274%）。
- 达实智能：数量 13,400；现价 3.350；成本 4.394；持仓盈亏 -4,176.46（-23.760%）；当日盈亏 +360.00（+2.761%）。
- 华电辽能：数量 13,740；现价 13.740；成本 16.978；持仓盈亏 -3,237.71（-19.072%）；当日盈亏 -407.16（-1.460%）。
- 三安光电：数量 5,236；现价 13.090；成本 19.720；持仓盈亏 -2,652.13（-33.621%）；当日盈亏 -12.00（-0.229%）。
- 建业股份：数量 4,030；现价 20.150；成本 29.713；持仓盈亏 -1,912.53（-32.185%）；当日盈亏 +2.00（+0.050%）。
- 截图显示的每行第二列为“现价/成本”；持仓盈亏金额明显不是用“总持仓数量 ×（现价-成本）”直接计算，可能存在部分仓位不可用、历史摊薄或界面“市值”第二行并非数量的语义，写入前必须以项目 schema 定义为准。
- `migrations/v18-real-account-snapshot/up.sql` 的 `real_account_snapshot` 是不可更新、不可删除的账户级证据表，字段能完整容纳截图顶部的账户汇总，但不容纳逐股明细。
- `src/portfolio/user_position_snapshot.rs` 定义完整持仓快照输入：每项只接收 code、name、quantity、cost_price；快照包含 effective_at、确认时间、证据哈希并按 code 排序、拒绝重复代码。
- `stock_position` 是本地持仓投影；加载逻辑要求 quantity > 0 且为 100 的整数倍。截图中合肥城建 4,348 和三安光电 5,236 不满足整手约束，说明截图第一列第二行可能是“市值”而非数量；截图表头明确为“股票/市值”，不能把这些数字直接当数量。
- 用“市值 ÷ 现价”精确还原股数：德展健康 500、利欧股份 2,000、合肥城建 400、达实智能 4,000、华电辽能 1,000、三安光电 400、建业股份 200；全部为整手，逐项反算市值无误，合计 51,904.00。
- 按上述股数与截图成本计算总成本约 73,922.60，浮亏约 22,018.60；与界面持仓盈亏 -22,019.00 仅差 0.40，符合佣金/显示精度差异，进一步确认股数推导正确。
- 正式持仓入口是 `import_user_position_snapshot --database ... --snapshot ...`；它先保存不可变完整快照，再自动把 `stock_position` 投影与最新确认快照对齐。
- 2026-09-03 截图中的 6 项证券市值为 `9360 + 4088 + 13560 + 13570 + 1389 + 3954 = 45921`，与账户摘要证券市值完全一致，因此截图已覆盖全部持仓。
- 由“市值 ÷ 现价”可精确得到持股数：利欧股份 2000、合肥城建 400、达实智能 4000、华电辽能 1000、三安光电 100、建业股份 200；均为整百股。
- 正式账户汇总入口是 `import_user_account_summary --database ... --summary ...`；写入 append-only `user_account_summary`，来源固定为 `user_confirmed_screenshot`。
- `/private/tmp` 已存在日期为 2026-09-03 的持仓、账户汇总和 real-account JSON；必须先逐字段核对内容与数据库现状，不能因文件存在就假定已导入或直接复用。
- `position_snapshot_20260903.json` 与截图逐项一致：证券代码为 `002131`、`002208`、`002421`、`600396`、`600703`、`603948`，数量和成本均与截图可验证字段一致。
- `account_summary_0903.json` 与截图的总资产、证券市值、可用现金、仓位比例和当日盈亏一致；其 schema 不保存可取现金或持仓盈亏。
- `account_snapshot_0903.json` 额外保存可取现金 `7226.66`、持仓盈亏 `-19054.79` 和原图 SHA，但是否应写入 `real_account_snapshot` 要结合其 30 秒新鲜度/用途及当前数据库状态判断。
- 当前原图 `/Users/zhangzhen/Downloads/IMG_3813.PNG` 的 SHA-256 是 `9963d061e1b4cfeb8070f70243a289cf5f4e43fd9f3f1f2d8232154bc48a3eb5`，文件时间 `2026-09-03T19:14:33+08:00`。
- 候选 `account_snapshot_0903.json` 内记录的原图 SHA 为 `b0e1b18c...`，与当前附件不一致，不能把该文件当作当前截图的证据直接导入；若需要 real-account evidence，必须重新生成并重新过 schema 校验。
- 生产数据库为 `data/stock_analysis.db`，约 1.0 GiB，存在 WAL；备份必须通过 SQLite 一致性备份完成，不能直接复制主文件。
- 目标表均存在：`user_position_snapshot`/`item` 是不可变完整持仓快照，`user_account_summary` 是 append-only 汇总，`real_account_snapshot` 以 `evidence_sha256` 唯一，`stock_position` 是可变 open/closed 投影。
- 数据库已存在截图对应记录：`user_position_snapshot.id=25`，effective_at `2026-09-03T19:14:00+08:00`，6 项，evidence `bea5b4c6...`；`user_account_summary.id=27` 的 5 个业务字段与截图完全一致。
- `user_account_summary` 没有业务唯一约束，重复运行 importer 会追加重复行；既然 ID 27 已存在，本轮不得再次执行该 importer。
- 当前 open `stock_position` 计数为 7，大于截图的 6 项；按 BR-215 这可能是未确认关闭的旧投影，必须查明而不能从截图推断平仓并删除。
- 当日快照 ID 25 的 6 项明细与截图逐项一致；`stock_position` 的第 7 个 open 条目是截图未出现的德展健康 `000813` 500 股。BR-215 明确把它保留为 `unconfirmed_open`，截图不能授权删除或平仓。
- `real_account_snapshot` 最新记录仍是 2026-09-02；今天的候选没有入库。
- 今天截图满足 `证券市值 45921.00 + 可用 8614.42 = 54535.42`，与总资产 `70795.65` 相差 `16260.23`。`real_account_snapshot` 强制总资产等于市值加可用现金，因此不得伪造字段强行导入；差额可能是冻结/在途资金，但截图和现有 schema 都未提供可核验字段。
- 2026-09-03 当日记录出现前未找到同日一致性备份；现阶段只能为当前已写入状态补做一致性备份，不能声称它是导入前备份。
- 当日两个已用输入文件均创建于 `19:21:15+08:00`，数据库当日记录也在该时段落库；当前检查时间为 19:47。
- 文件系统剩余约 108 GiB，足以为约 1.0 GiB 数据库创建 SQLite 一致性备份；`data/private_evidence/2026-09-03` 尚不存在。
- `monitor`（PID 15541）和 `grpc_mark...`（PID 37699）正持有数据库及 WAL；不得停进程或直接复制主文件。SQLite `.backup` 可在在线连接下取得事务一致性快照。
- 已创建导入后状态备份 `data/private_evidence/2026-09-03/stock_analysis_after_20260903_position_and_summary_import.db`；immutable 模式完整性检查返回 `ok`，计数为持仓快照 25、账户汇总 27、real-account 快照 9、open 投影 7。
- 生产库精确差分：19:14 的持仓快照恰有 1 条，6 个期望明细与实际双向差异均为 0；19:14 的账户汇总也恰有 1 条，ID 27 的全部可存字段匹配截图。
- 2026-09-03 的 `real_account_snapshot` 为 0 条；未强行导入。截图未被当前 schema 表达的账户价值为 `16260.23`，不能假定为可用现金。
- 备份 SHA-256 为 `f19619ab1dc9ef3a6822a1c2d021df8bf8f20399e90d34521af39d347c6f5602`，大小 `1089986560` 字节；原图 SHA 再次核验为 `9963d061...`。
- 最终 fresh 验证时，生产库完整性仍为 `ok`，最新快照仍为 ID 25；当日持仓和账户汇总都唯一，未发生重复追加。

## 2026-08-31 至 2026-09-03 真实推送复盘

- 生产证据源至少包括：`data/durable_delivery.sqlite3`、`data/durable_delivery_audit/durable_delivery_v1.jsonl`、`data/push_analytics.db`、主业务库、`data/dispatcher_log/<date>.jsonl`、`data/event_bus/<date>.jsonl`、`data/review_audit/<date>.jsonl`、`data/g5b/<date>.jsonl` 和 monitor 日志。
- 主业务库中与本轮相关的表包括 `pushed_stocks`、`selection_event_completions`、`selection_event_inbox`、`news_ai_delivery_event(_chain)`；强投递终态仍需由独立 durable DB 和不可变审计证明。
- 广域 `data/` 扫描会混入大量 `data/test/**`，生产统计必须从明确的 production root 读取，避免测试回执污染真实发送数量。
- 最近四日 production JSONL 规模：dispatcher `1232/558/521/159` 行，event bus `305/504/237/137` 行，review audit `108/51/50/20` 行（依次为 08-31/09-01/09-02/09-03；09-03 为截至审计时的部分日）。
- 强投递生产库约 34 MiB、不可变审计约 12 MiB，最后写入时间均为 09-03 19:03；`push_analytics.db` 最后写入 19:00。当前主 monitor 日志持续增长，审计必须按时间和关键词定向读取，不能整文件加载。
- durable DB 有 decision、attempt、state event、sink result、disposition、audit outbox、cooldown、daily budget、business-date claim、task transition 和 terminal replay 等事实表；这是判断 Accepted/Uncertain/重复的首要来源。
- `delivery_decisions` 可按 `business_date/push_kind/state` 统计；`sink_results` 提供 `result_kind`、`authoritative_for_state`、channel/provider/message IDs 与 accepted_at；`task_transition_payloads` 提供 append/hydration 状态，可直接检验“已送达但业务未完成”。
- `push_analytics` 的 `pushed` 只是 analytics 层布尔观察，必须与 durable sink result 分开统计；主业务库 `pushed_stocks` 还包含 outcome/consumed 状态，也不能单独作为物理接受证明。
- 三类 JSONL 顶层结构：dispatcher 为 `{ts,kind,success,snapshot_size,error}`，event bus 为事件 envelope，review audit 为 hash-chain `{payload,prev_hash,record_hash}`。
- 最近四日 durable 决策仅涉及 10 个 PushKind；按日 Delivered/RejectedDurable 为：08-31 `21/213`、09-01 `31/178`、09-02 `24/20`、09-03 截至 19:03 为 `20/13`。
- 所有 Delivered 都有 `Accepted` 且 `authoritative_for_state=1` 的 sink result；查询窗口内没有 Uncertain 或非权威 Accepted。RejectedDurable 没有物理 sink result，不能与传输失败混为一谈。
- review 类 `task_transition_payloads` 在查询窗口内全部是 `Appended + Applied`，已接受但业务 hydration 未完成的可见缺口为 0。
- 高频业务的 RejectedDurable 数量远大于实际 Delivered，尤其 08-31 HoldingPlan `138`、T0Advice `51`，09-01 HoldingPlan `76`、T0Advice `99`。方案必须把“正常冷却/去重拒绝”与“业务或传输故障”分开，否则告警和成功率会严重失真。
- 09-03 dispatcher 样本显示 R-08 的 CFFEX `invalid_evidence` 被标为 `retryable=false`，但从 00:36 到 20:12 仍按约 15 分钟持续执行；“单次不可重试”和“scheduler 是否应在本日继续调度”语义发生分裂。
- A-11 的 `no IPO announcements today` 被记录为 `success=false`，这更接近正常 NoData，而非系统失败。当前 dispatcher 二值 success 不能区分 NoData、Disabled、ExpectedWait、RetryableFailure 和 NonRetryableFailure。
- Rejected durable canonical 都是 sink 前拒绝（`attempt_identity=null`）；还需从 canonical reason 字段聚合，判断是正常冷却/日限额，还是输入/策略错误。
- dispatcher 四日最突出的 `success=false` 量：A-11 NoData 287 次、I-03 失败 249 次、N-01 295 次、N-02 276 次、R-08 203 次、R-12 Disabled 130 次、T-14 未注册 435 次、T-15 未注册 453 次、P-04 NoData 40 次。大量已知 Disabled/NoData/non-retryable 状态仍被周期轮询并记成失败。
- N-02 四日只有 7 次 dispatcher success；N-01 明确 disabled，却仍和 N-02 一起重复记录 provider mapping 缺失。这表明调度层没有在 capability/readiness 处做深模块式短路，失败复杂度泄漏到了每个 tick。
- durable Rejected payload 只保存 `denial_identity`、`retry_authorized` 等字段，没有稳定 `reason_code`；现有表面查询无法直接把 424 次 RejectedDurable 分成 cooldown、budget、business-date dedup 或其他策略拒绝，观测接口过浅。
- `push_analytics` 四日 `pushed=true/false` 为：08-31 `44/280`、09-01 `410/15`、09-02 `160/5`、09-03 截至 19:00 为 `69/5`。对应 push-log Markdown 数为 `41/407/159/69`，高度相关，但这仍是 legacy bool/日志证据，不等于 typed receipt。
- 09-01 的高峰由 `paper_sell=254`、`news_to_idea=120` 主导；09-02 为 `paper_sell=131`，09-03 为 `news_to_idea=48`。这两个非统一 durable 权威路径贡献了绝大多数用户可见推送，风险优先级应高于低频 CLI 或文档型迁移。
- `intraday_market` 的 `pushed=false` 实际是 `sink_name=deduped` 且 governance=Approve，四日为 19/15/5/5；再次证明单一 `pushed` 布尔值把“已成功抑制重复”误表示为未推送。
- 08-31 另有 `data_mode` quiet-hour Deny 122 次、`news_to_idea` data-quality Deny 139 次；治理拒绝量很大，但后续 09-01 又爆发 120 条 news-to-idea，说明系统缺少以 occurrence/audience 为中心的上限与异常速率门禁。
- 09-01 的 254 条 PaperSell 全部使用唯一 event_id，但集中在 11 点 134 条、13 点 116 条；它不是同 ID 重复，而是逐持仓物理消息形成的业务洪峰。NewsToIdea 同日 120 条也全部为唯一 event_id。
- push-log 内容证实 PaperSell 是“一只虚拟持仓一条卖出通知”，NewsToIdea 是“一条新闻一条 AI 证据分析”。当前消息粒度直接等于数据记录粒度，缺少“同一运行批次聚合成摘要”的深模块。
- G5bAttribution 在 08-31、09-02、09-03 各有一个 event_id 被 `pushed=true` 两次，时间相隔数秒；这是最近四日可直接证明的 3 组重复物理调用候选，应提升为 P0 幂等修复。
- 最近四日绝大多数真实消息走 legacy analytics/push-log，而 durable 只覆盖少数 counted/review/T0/Holding paths。以“65 个 PushKind”为唯一 catalog 会漏掉实际流量最大的物理 emitter，目录主键应升级为 `DeliverySource/OccurrenceFamily`，PushKind 只是一个字段。
- 峰值速率达到 PaperSell 每分钟 40 条、NewsToIdea 每分钟 20 条；这已经不是“模板体验”问题，而是需要物理发送前强制批次聚合、速率预算和爆发熔断的可靠性问题。
- 主业务库 `pushed_stocks` 的 D-01 行与物理推送并非一一对应：08-31/09-01 分别有 143/141 个 D-01 记录，而 analytics 物理候选为 0/120；09-03 有 76 个业务记录但 48 条物理候选。因此 business record、delivery intent 和 sink receipt 必须是分离实体。
- `selection_event_completions` 在最近四日查询窗口没有记录，不能为 D-01 提供可靠的“业务游标已完成”交叉证明。
- review audit 四日聚合显示：R-03 `account_metrics_incomplete` retryable failure 80 次、R-08 `source_transport_failed` retryable failure 80 次；已知 Disabled 的 R-02/R-05/R-06/R-12 各记录 8 次。相较之下，真正 delivered 的 A-10/R-04/R-09/R-11/R-13 各 4 次，R-07 为 3 次。
- R-08 存在直接矛盾：dispatcher 的底层 provider 错误写 `retryable=false`，review task transition 却统一写 `retryable=true`。重试分类散落在不同 caller，证明需要单一 `FailureDisposition` authority。
- R-03 因账户证据不完整仍被四日累计重试 80 次；这类等待用户输入/新快照的任务不应按固定 tick 当网络错误重试，应转为 `BlockedOnInput` 并由新快照事件或低频探测唤醒。
- review audit 已经具备比 dispatcher 更好的 typed status/reason/retryable 模型，但同一事实没有成为 scheduler 的唯一接口，导致日志分类正确而执行策略仍错误。
- durable 覆盖范围内四日共有 96 个 Delivered/Accepted 和 424 个 RejectedDurable；全部 decision、attempt、disposition、audit outbox、task hydration 都已进入终态，无 Pending/Uncertain/未 hydration 积压。
- 23 个需要业务 task hydration 的 Accepted 全部 Applied；Accepted→Applied 平均 `104.551s`、最大 `217.131s`，均低于方案的五分钟硬上限。现有 durable 深模块在已接入路径上实际表现良好，应保留并扩展，而不是重写。
- 424 个 Rejected 的 `denial_identity` 与 cooldown/budget reservation 及其 event 都无法 join；拒绝原因在持久证据模型中不可查询，需在不改变 terminal safety 的前提下新增稳定 `ReasonCode`/denial class。
- legacy analytics 按其显式 `+08:00` 时间分类后的 `pushed=true`：08-31 盘前/竞价/盘中/盘后=`2/1/22/19`，09-01=`3/20/375/12`，09-02=`0/0/151/9`，09-03 截至 19:00=`2/4/56/7`。用户消息负担主要集中在竞价与盘中。
- 首次 durable 时段统计直接截取 `accepted_at` 小时，尚未处理 UTC 格式，导致盘后 review 被误归盘中；该结果作废，必须统一换算到 Asia/Shanghai 后再使用。
- durable `accepted_at` 统一从 UTC `Z` 加 8 小时后，96 个 authoritative `Accepted` 的四日实际分布为：08-31 盘中 15、盘后 6；09-01 盘中 24、盘后 7；09-02 盘中 18、盘后 6；09-03 截至 durable 审计切点盘前 1、盘中 14、盘后 5。合计盘前 1、集合竞价 0、盘中 71、盘后 24。
- 09-03 唯一盘前 durable 接受是 `PreopenNewsHot`（09:00）；`HoldingPlan` 实际集中在 09:45--10:06，属于盘中而不是盘前；`T0Advice` 与 `CloseCall` 也主要在盘中，review 类和 `TomorrowWatch` 主要在 19:00--21:01 盘后。
- “计划业务时段”和“实际触发时段”必须分开保存：实际时段应由带交易日、交易所日历和 Asia/Shanghai 时区的 `RunContext` 计算，并对计划时段与实际时段漂移做机器校验，不能按模板名推断。
- 蓝图 §24.5 已把 `HoldingPlan` 归入盘中，和四日真实接受时间一致；时段分类本身没有因此被反证。真正被生产数据反证的是首批风险顺序：当前前 10 个 Unit 从 CLI report、两次产业链和 Attribution 开始，而用户可见洪峰 `PaperSell`/`NewsToIdea` 被放在后续“弱 authority/调度/语义”大类。
- 当前正式 `docs/push-system/` 只有 Grill 决策与“文档硬化实施计划”，尚未生成计划中的 `push-system-implementation-rfc.md`、机器 catalog 或 evidence manifest。因此这次校正应先进入待生成 RFC/目录，不能继续把蓝图 §24.10--§24.19 的暂定顺序视作已经冻结的实施规格。
- 原方案已经把 MigrationUnit 定义为 `(producer, occurrence family, completion owner)`，这一模块 seam 可以保留；但机器目录不能只精确核对 65 个 enum kind，还必须将 enum 外和 legacy analytics 中真实存在的物理 emitter 纳入同一 `DeliverySource/OccurrenceFamily` 目录，否则最高流量路径仍在治理边界之外。
- 代码入口初查确认 PaperSell 在 `monitor_loop` 中接收 `scan_and_sell*` 返回的 `Vec<PaperSellResult>` 后逐项调用 `push_governor_v3`，因此一次扫描会产生 N 次物理发送；盘中与 15:30 盘后都复用这一逐项模式。这与 09-01/09-02 的突发发送量一致，聚合 seam 应位于“扫描结果批次 → 单一 PreparedPush”之间，而不是 sink adapter 内部盲目合并。
- NewsToIdea 的 `dispatch_news_to_idea_daily` 用进程内 `D01_LAST_PUSH` 做 1 小时/票 memo，且只在 `push_news_to_idea` 返回 true、可选虚拟买入也成功后才写 memo；若消息已推送而随后的虚拟买入失败，函数返回 false 且不写 memo，下一 tick 具备重复发送同一业务建议的路径。这里同时耦合 delivery completion 与交易副作用 completion，必须在原子 Unit 中拆成独立状态，不能只加限流掩盖。
- G5b 路径在持久化每条归因结果后逐条调用 `push_governor_v3(..., code=None)`，不检查 `PushOutcome` 是否确认送达就递增 `done`；其 global cooldown 身份又缺少股票/归因记录业务键。真实数据中的同 event_id 二次 pushed 需要继续向调用身份和进程/重启边界追查，但“分析完成数冒充送达数”已由代码直接证明。
- G5b 物理日志进一步证明这里不是“同一段文本被 transport 重发”，而是同一业务标的在一个批次内被 LLM 重算出不同正文后连续发送：08-31 同一 `event_id=3c1708f8c8f44bc7` 两条，09-02 同一 `event_id=69367927269e8df8` 两条，09-03 同一 `event_id=77bc51b37214c115` 两条；同组正文 hash/长度不同，但股票和事件类型相同。幂等键必须来自业务 occurrence，不能来自 rendered bytes。
- 08-31 两条 G5b 实际发送正文明确包含 `TEST_CODE_000001 测试`、`测试告警，无实际数据支撑`，且 analytics 记录 `sink_name=feishu, pushed=1`。这是测试证据进入生产物理通道的直接证据，优先级高于普通重复：catalog、RunContext 和 authority 必须把 `namespace/environment` 纳入不可绕过的发送身份并在 sink 前 fail-closed。
- 09-02 G5b 对山东玻纤连续发送 3 条不同归因，09-03 对三安光电连续发送 3 条；其中各有两条共享 event_id。即使修复完全相同 event_id，用户仍会收到同标的/同事件簇的多条近义消息，因此还需要 `topic_key + source occurrence` 聚合，而不只是 event_id 去重。
- PaperSell 的 09-01 push-log 恰有 254 个匹配文件，正文展示为大量不同证券的逐票成交卡，初步支持“真实批量卖出后的逐项通知洪峰”而非同一条 transport 重发；但仍需和 `paper_trades` 按日期、状态、证券和 plan_id 做数量对账后才能把 254 全部认定为业务成交。
- `paper_trades` 对账已经闭合：09-01 恰有 254 条 `sell/Filled`，对应 254 个唯一 code 和 254 个唯一 plan_id；09-02 恰有 131 条，亦是 131 个唯一 code/plan。两日 PaperSell push-log/analytics 数量与成交事实 1:1，因此它不是 delivery 重试故障，而是“大批业务成交 → 每笔一条通知”的消息粒度设计问题。
- 同一窗口内，09-01 另有 113 条 buy/Filled、15 条 buy/NotFilled，08-31 有 123 条 buy/Filled、10 条 buy/NotFilled；PaperSell 洪峰反映模拟组合在短时间内大规模换仓。推送层可以聚合用户消息，但不能通过 drop/限流吞掉逐笔成交审计；正确 seam 是逐笔不可变业务事实 + 批次级用户摘要 + 可查询明细。
- 09-01 的 120 条 NewsToIdea 物理文件只覆盖 66 个唯一“标的”显示值，已证明至少存在大量同票多消息；样本显示同一新闻标题会映射到多个概念/标的，每条消息带独立 evidence hash、evaluation audit 和 delivery identity。还需按“标题/源事件 + 标的 + evidence/identity”分组，区分合法多新闻与重复重新分析。
- NewsToIdea 的业务持久链在 09-01 形成 120 组完整 `reserved → sink_started → delivered → prediction_linked`，09-02 为 6 组，09-03 为 48 组；它并非缺少自己的 durable 证据，而是其 occurrence 身份把同一“标题+标的”的重复分析认成了 2--4 个不同合法 delivery identity。问题落在 authority 之前的 source/assessment identity seam。
- 09-01 可直接看到同一 `标题+标的` 的 2--4 条消息全部拥有不同 evidence hash 和 delivery identity：例如短剧新闻映射的多个代码各 4 条、银行新闻的多个代码各 3 条。按 content hash、assessment ID 或 delivery identity 去重都无效；必须在 LLM 前构造稳定 `source_event_id + target_security_id + analysis_version` occurrence，并把重复 source aliases/batches 归并到同一 canonical source event。
- NewsToIdea 已有五年保留的 assessment/delivery event 链，不应把它降级成 generic legacy bool 或重写为较弱 coordinator；应做 `DedicatedAuthoritative` conformance，并在其前方加 source canonicalization、topic/target aggregation和用户摘要策略。
- 数据库根因已经缩小：重复组的 `source_item_id` 完全相同，但 `source_batch_id`、`source_identity_sha256`、`input_evidence_sha256` 各不相同。例：标的 `300413` 对同一财联社 item `2470319` 在 09:30、09:35、09:41、09:47 四个轮询 batch 各分析/发送一次；`600919` 对 item `2470609` 在 13:47、14:03、14:14 三次。当前 source identity 把抓取批次纳入业务事件身份，导致同一上游 item 每次轮询都变成“新事件”。
- 因此 NewsToIdea 的首要修复不是任意 20 分钟/1 小时 cooldown，而是身份模型：`CanonicalSourceEventId = provider + source_item_id (+ source revision)`；`source_batch_id` 只属于采集 lineage，不参与“是否已经为该标的分析/发送”的 occurrence 主键。若同 item 内容发生真实修订，必须用显式 revision/content-change policy，而不是批次时间制造新身份。
- 源码逐行坐实上述结论：`news_ai_assessment` 的 source 索引包含 `source_batch_id`（`src/database/news_ai.rs:66-69`）；`canonical_assessment` 把 batch/item/target/version 组成 `CanonicalSourceIdentity`（`:652-658`）；`core_assessment_id` 再明确哈希这五项（`:714-726`）；delivery identity 被强制等于 assessment ID（`:1001-1005`）。所以轮询 batch 变化必然贯穿 source→assessment→delivery 全链，现有 durable 层只是在正确地执行一个过宽的业务身份定义。
- 环境保护并非完全不存在：NewsAI target 在持久化时调用 `validate_symbol_for_current_env`（`news_ai.rs:583-586`）。但 08-31 G5b 仍把 TEST_CODE 发到 Feishu，说明环境 seam 没有统一下沉到所有 physical sender；Foundation 必须将 namespace 校验放进唯一 authority port，而不能依赖各 producer 自觉调用。
- Review 代码也直接证明失败处置分散：`ReviewTaskOutcome` 已有 NoData/ExpectedWait/Disabled/Failed 等类型，但 `account_metrics_incomplete` 构造函数硬编码 `retryable=true`（`review_batch.rs:889-895`）；scheduler 又按这个 caller 提供的布尔值生成下一次尝试（`:1195-1207`）。这正是 R-03 四日 80 次轮询的代码原因，而不是 provider 暂时故障。
- Review scheduler 的 1/5/15 分钟退避本身实现清晰（`review_batch.rs:1208-1220`），问题是它只消费 caller 的 `retryable: bool`，无法表达“等账户快照版本变化”“等 capability generation 变化”“本交易日永久不可用”等唤醒条件。优化应把 `bool + next_attempt` 升级为 `FailureDisposition::{RetryAt, BlockedOnInput, BlockedOnCapability, Terminal}`，并让 scheduler 只依赖这个深模块接口。
- `Disabled`/`NoData` 已在 review scheduler 中被正确设为 Terminal（`:1195-1197`），`ExpectedWait` 也能等待确定发布时间（`:1198-1200`）；不应推倒重写整个 scheduler。应保留状态机，收敛 R-03/R-08 等错误构造点，并把相同 contract 复用到 dispatcher。
- 当前 36--69 人日估算仍不可复算：正式 docs 只引用已消失的 W01--W21 `98--142` 小时明细。最近生产数据还新增/提升了至少四个明确工作面：NewsAI source identity 迁移、PaperSell 批次摘要、G5b namespace/occurrence 修复、FailureDisposition/事件唤醒。工期必须在 exact emitter catalog 与这些 Unit 拆分后重估，不能直接沿用原范围下界。
- 将 legacy `pushed=true`（有对应 push-log/Feishu sink）与不重叠 kind 的 durable authoritative Accepted 合并，四日可证用户可见发送总量为 779：08-31 `65`、09-01 `441`、09-02 `184`、09-03 截至各证据切点 `89`。按 Asia/Shanghai 时段合计：盘前 8、集合竞价 25、盘中 675、盘后 71；86.6% 集中在盘中。
- 09-01 的 PaperSell 254 + NewsToIdea 120 = 374，占当日 441 条的 84.8%；四日这两类合计 559，占全部 779 条的 71.8%。所以总体方案的第一个 production vertical slice 应围绕这两个 occurrence family，而不是低频 CLI compatibility；否则即使旧前十 Unit 全做完，主要用户噪声仍基本不变。
- durable Accepted 的 kind 与 legacy analytics 的 true-kind 在本窗口不重叠：durable 是 P01/Holding/T0/Close/review，analytics 主要是 DataMode/IntradayMarket/PaperSell/NewsToIdea/G5b/IndustryChain/NewsFlash/IPO/SnapshotStale。上述 779 是跨两套事实源的并集，不是简单重复计数。
- `pushed_stocks` 继续显示它是业务候选/结果而非交付游标：08-31 至 09-03 每日为 143/141/22/76 行，且大量 outcome 是 blank 或 NewsCatalyst，与 65/441/184/89 的物理发送完全不对应。
- `selection_event_completions` 按真实列 `completed_at` 查询，四日窗口为 0 行，不能作为任何上述消息的 completion authority；不能把表“存在”写成能力“已接通”。
- `real_account_snapshot` 最新事实仍停在 09-02 22:11，09-03 为 0；当天截图因总资产与证券市值+可用现金相差 16,260.23 而无法进入当前 schema。R-03 在缺当天完整账户证据时持续失败是输入 readiness 问题，正确状态是 `BlockedOnInput(snapshot_date/version, missing_fields)`，不是每 15 分钟网络重试。
- 综合 completion 对账：PaperSell 的 385 条有 1:1 Filled 业务事实；NewsToIdea 的 174 条都有 delivered+prediction_linked 链，但 occurrence 定义过宽；durable 96 条全部权威 Accepted 且需要 hydration 的 23 条全 Applied；G5b 则只有分析持久化与 legacy bool，`done` 不代表送达。应分别修复语义，不应统一压成一个 `pushed` 布尔值。
- PaperSell 的正确分层也由执行顺序证明：`already_sold_today` 先做业务幂等（`paper_sell.rs:431-436`），`simulate_with_audit_evidence` 先写 Filled/order audit（`:438-468`），最后才返回 `PaperSellResult` 给 monitor 发送（`:470-481`）。因此聚合通知不会改变成交 authority；但通知失败也不能回滚成交，completion policy 必须明确为 `BusinessCommittedThenNotifySummary`。
- G5b 的 `top_events_for_deep` 只按 level 稳定排序后 truncate（`attribution_deep.rs:264-275`），完全没有 namespace、event identity、code/category/topic 去重；08-31 两个 TEST_CODE 输入因 triggered_at 不同被当成两条。修复 seam 应位于 top-N 选择前，且必须保留被折叠记录的审计引用。
- 账户证据还揭示事件唤醒的细节：09-02 有两条 `real_account_snapshot`，evidence hash 不同但资产/市值/现金 payload 完全相同；若 R-03 仅监听“新增 row/evidence SHA”仍会无效重跑。`BlockedOnInput` 应等待 canonical `AccountEvidenceContentHash` 或 readiness generation 变化，而不是任意 append。
- 以最近四日并集看，盘前 8 条（DataMode 4、NewsFlash 2、SnapshotStale 1、P01 1）；集合竞价 Epic 25 条（NewsToIdea 24、DataMode 1）；盘中 675 条（PaperSell 385、NewsToIdea 150、T0 41、IntradayMarket 43、HoldingPlan 23、其余 33）；盘后 71 条（DataMode 29、durable review 23、G5b 8、IPO 8、其余 3）。四时段架构应保留，但每个 Epic 的 P0 完全不同。
- NewsToIdea 09-01 的 120 个 delivered assessment 只对应 14 个唯一 source item、61 个唯一 `source_item+target` 对和 26 个抓取 batch。把 batch 从 occurrence identity 移除，历史 replay 理论上先从 120 降至 61（减少 49.2%）；再按 source event 把最多 5 个 target 合成一张卡，可降至 14 张（减少 88.3%）。后者是用户呈现策略，必须保留 61 个逐目标 assessment/审计事实。
- PaperSell 09-01 的物理发送发生在少数密集簇：11:25--11:32 共 134 条，13:09--13:11 共 81 条，单分钟峰值 40。代码已经先完整返回 `Vec<PaperSellResult>` 再逐条发送，所以新增显式 `scan_run_id` 后可做到“每个非空扫描批次最多一条摘要”，不需要改成交引擎或在 sink 猜测时间窗口。
- 蓝图现有 §24.17 已提出 `topic_key` 与 severity→即时/摘要，方向被真实数据验证；需要做的不是另造方案，而是把它从后续体验优化提升为 P0 contract，并给出 source event、target、scan run、摘要 receipt 的可执行身份/完成语义。

### 生产数据驱动的三种整体路径

- 路径 A（推荐）是“安全止血 + 薄 Foundation + 风险纵切”：保留现有 durable deep module 和原子 MigrationUnit seam，先下沉 production namespace fence，再依次迁移 NewsToIdea source identity/摘要、PaperSell scan-run 摘要、G5b canonical occurrence/receipt、R-03/R-08 FailureDisposition；随后再处理原计划的 CLI/产业链/Attribution/Candidate/LimitBoards 和低频路径。它直接覆盖 71.8% 消息量与测试数据外泄，同时避免重写已经闭合的 96 条 durable 链。
- 路径 B 是“原顺序不变，只先补观测”：实施风险最低，但即使完成原前十 Unit，PaperSell/NewsToIdea 的主要洪峰仍不变，且 NewsAI 继续为同一 source item 重复付出 LLM 与物理发送成本；不符合此次真实数据揭示的优先级。
- 路径 C 是“一次性统一所有 sender/identity/scheduler”：最终表面最整齐，但会同时改动 65-kind、NewsAI 五年审计、paper business completion、durable authority 与 scheduler，shadow 难以定位语义差异，回滚半径过大；不推荐。

### 推荐路径 A 的合同边界

- 保留两个机器目录：`PushCapabilityCatalog` 精确覆盖 65 个 kind/状态；新增 `DeliveryOccurrenceCatalog` 覆盖每个物理 producer + occurrence family + completion owner，并允许一个 kind 映射多个 occurrence、enum 外 sender 映射零或一个 kind。后者才是 migration/promotion 主键。
- 身份拆为 `CanonicalSourceEventId`、`OccurrenceId(namespace, business key, policy version)`、`AttemptId` 和 `RenderedContentHash`；batch/采集 lineage 不得冒充 source event，rendered bytes 不得冒充业务 occurrence。
- 结果拆为三层：`EvaluationDisposition`（NoData/Disabled/Blocked/Eligible）、`DeliveryDisposition`（Suppressed/Accepted/Rejected/Uncertain）、`BusinessCompletion`（NotRequired/Pending/Applied/Failed）。避免 dispatcher/review/analytics 再用一个 bool 混合三件事。
- scheduler 只消费 `WakeCondition::{At, OnInputRevision, OnCapabilityGeneration, NextSession, Never}`；R-03 等 canonical account content 改变再唤醒，R-08 non-retryable provider 能力缺失等 capability generation 改变，NoData/Disabled 当次终止。
- 聚合必须发生在业务事实和 authority 之间：PaperSell 每条 Filled 事实不可变，通过 `scan_run_id` 组成 summary membership；NewsAI 每个 source-target assessment 保留，通过 canonical source item 组成一张多标的卡。摘要 Accepted 只完成摘要 intent，不篡改单项业务事实。
- 全局物理发送预算以 `audience + phase + topic + severity` 为键；Emergency/Important 的即时策略与 Info/Research 摘要分开。预算拒绝必须留下 typed Suppressed reason，不计作 failure，也不得删除审计事实。

### 推荐后的风险顺序与临时工期

1. Foundation-Safety：production namespace fence、三层 outcome、稳定 occurrence/RunContext、双目录、shadow/replay corpus。
2. P0-N：NewsToIdea identity v2；保留旧五年链，新增稳定 source event/revision 与 source-target 唯一约束，再做一源事件一摘要。
3. P0-P：PaperSell `scan_run_id` + summary membership + BusinessCommittedThenNotifySummary completion。
4. P0-G：G5b 测试命名空间 fail-closed、top-N 前 canonical event/topic 去重、送达结果与 analyzed/persisted 分栏。
5. P0-S：统一 FailureDisposition/WakeCondition，先接 R-03/R-08，再推广到 dispatcher 的 Disabled/NoData/T-14/T-15。
6. P1：原方案中仍会错误推进 completion 的 Attribution/产业链/15:05/Candidate/LimitBoards 等路径。
7. P2：其余 weak authority 迁移、已有 strong authority conformance、inactive/starved catalog-only 收尾。
- 旧 `36--69` 人日不能直接复用；在其范围上，只有 NewsAI stable identity migration、生产 replay/burst corpus、统一 namespace fence 属于明确新增净工作，PaperSell/G5b/Review 已部分包含在旧 Unit 下界。当前更诚实的临时包络是 **41--78 个 8 小时等效开发日**；Foundation + 上述 P0 Production Verified 约 **14--24 个工程日、3--5 个交易周**，全量 rollout 约 **8--12 个交易周、11--18 周日历跨度**。exact occurrence catalog 完成后必须重新基线，不能把该区间当承诺日期。

### 推荐方案的生产 replay / promotion 门禁

- namespace gate：用 08-31 `TEST_CODE_000001` 事实 replay 时必须在 authority 前得到 typed `EnvironmentMismatch`，provider/LLM/sink 调用均为 0；production receipt 中 namespace 必须为 production。
- NewsToIdea gate：09-01 的 120 个旧 delivered occurrence 必须确定性投影为 61 个 canonical source-target assessment、14 个 source-event summary intent；同 `source_item_id+target+revision` 跨 26 个 batch 不得产生新 occurrence。历史 120 条链原样保留，不回写或删除。
- PaperSell gate：09-01/09-02 的 385 个 Filled/code/plan 事实必须一条不少；每个显式 `scan_run_id` 最多一个 summary intent/物理 Accepted，summary membership 可反查全部成交，通知失败不得回滚成交或再次执行 sell。
- G5b gate：测试 namespace 0 物理请求；同一 business-date/topic 的多条分析保留逐项证据，但只生成一个盘后 research digest；`analyzed/persisted/delivery_accepted/business_finalized` 分栏，任何一项不得冒充另一项。
- scheduler gate：把最近四日 R-03/R-08 各 81 条失败 replay 后，R-03 只能在 canonical account readiness generation 改变时再运行；R-08 `retryable=false` 只能等待 capability generation 或人工重新启用；A-11 NoData、Disabled、Deduped 均不得计入 failure rate。
- 共通 gate：按 evaluated occurrence、eligible intent、physical request/Accepted 三个分母分别计数；零未解释 duplicate、零测试到生产、零超龄五分钟 finalizer、零 business/delivery completion 混写。每个 physical-owner promotion 继续遵守一天一个 Unit 与可回退 activation manifest。

### Fresh 审计切点

- 最终复算时间为 `2026-09-03T20:38:23+08:00`；analytics 最后事实为 `19:00:07+08:00`，durable 最后 decision 更新为 `11:01:53Z`（`19:01:53+08:00`）。09-03 是部分交易日/盘后窗口，后续自然推送可能改变当日总量。
- fresh SQL 仍为每日 `65/441/184/89`、四时段 `8/25/675/71`、合计 `779`；PaperSell Filled `385`，NewsToIdea 09-01 delivered `120 → 14 source items / 61 source-target pairs`，selection completion `0`，09-03 real-account `0`。
- live 日志在复盘期间增长：09-03 dispatcher 从 159 增至 160 行，review audit 从 20 增至 22 行；fresh 聚合使 R-03/R-08 各从 80 增至 81，dispatcher R-08 从 203 增至 204。早先静态数量必须以本切点为准。
- 正式账户汇总入口是 `import_real_account_snapshot --database ... --evidence ...`；`data/stock_analysis.db` 是 README 和运行代码约定的主业务数据库。
- 历史持仓快照已确认证券代码：德展健康 `000813`、利欧股份 `002131`、合肥城建 `002208`、达实智能 `002421`、华电辽能 `600396`、三安光电 `600703`、建业股份 `603948`。
- 当前数据库最新完整持仓快照是 2026-08-27 15:12:19，共 7 项；与新截图相比，华电辽能从 500 股/成本 19.491 变为 1,000 股/成本 16.978，其余数量不变，截图提供的成本覆盖为最新事实。
- 当前 `stock_position` 投影与 2026-08-27 快照一致；导入后预计只更新华电辽能数量与成本，其余 6 项保持不变。
- 图片文件 SHA-256 为 `86406ff82dcbad53d84cba7d3fd8c4860e602876a2d0aaaea0309600bd0412b3`，文件修改时间为 `2026-08-31T20:42:42+0800`，与截图状态栏 20:42 一致，可作为 capture/effective 时间。
- 写入前数据库 `PRAGMA integrity_check` 返回 `ok`；持仓快照 22 条、账户快照 6 条、open 投影 7 条。
- BR-215 投影对齐只改写完整快照中已确认证券的 name/quantity/cost，保留原始 buy_date；不会删除或静默关闭快照中缺失的 open 记录，而是显式报告 `unconfirmed_open`。本次新旧快照代码集合相同，预期为空。
- 正式持仓 schema 不保存逐股现价/当日盈亏；逐股真实事实保存为 code/name/quantity/cost，截图顶部资产/市值/现金/盈亏/仓位另存 `real_account_snapshot`。这与历史截图导入方式一致。
- 写入前一致性备份已生成：`data/private_evidence/2026-08-31/stock_analysis_pre_position_and_account_import.db`（846,213,120 bytes）；以 immutable 只读模式检查返回 `ok`，并确认备份中持仓快照/账户快照/open 投影计数为 22/6/7。
- 持仓 importer 返回 `inserted=true item_count=7`；BR-215 对齐结果 `updated=1 inserted=0 unchanged=6 unconfirmed_open=[]`，与写入前预期完全一致。
- 账户 importer 返回 `account_snapshot_id=8 inserted=true daily_pnl_is_null=false`。
- 完成前 fresh 回读：数据库完整性 `ok`；最新持仓快照与期望 7 项差异 0；open 投影与期望差异 0；最新账户快照的全部截图字段与原图 SHA 匹配。
- 由截图现价重新计算：市值 51,904.00、成本基数 73,922.60、未计界面精度/费用的浮亏 -22,018.60；与截图 -22,019.00 一致到 0.40 元差异。

- `architecture-blueprint-generator` 已安装到 `/Users/zhangzhen/.codex/skills/architecture-blueprint-generator`。
- 已完整读取其 322 行 `SKILL.md`；固定产物名为 `Project_Architecture_Blueprint.md`。
- Skill 要求覆盖：架构检测、概览、多层图、组件、依赖、数据、横切关注点、通信、技术特定模式、实现模式、测试、部署、扩展、代码示例、架构决策、治理、新开发蓝图。
- 本任务是现有代码架构文档的 bounded creative rewrite，不改变系统结构或产品行为。
- 当前工作树不存在上一轮生成的 `ARCHITECTURE.md` 和 root planning files；本轮将重新扫描并生成正式 blueprint。
- `.planning/.active_plan` 指向另一个现有任务，不应被本任务覆盖。

## 2026-08-30 当前工作树复核（批次 1）

- `git status --short` 仅显示本轮新增的 `task_plan.md`、`findings.md`、`progress.md`，尚无产品代码改动。
- `src/lib.rs` 当前仍公开 62 个顶层模块；Rust 源文件 487 个、合计 397,900 行。
- `Cargo.toml` 默认运行目标仍为 `stock_analysis`，默认 feature 是 `magic-gateway`。
- `src/bin` 有 38 个顶层 `.rs` binary 文件；另有 `src/main.rs` 和目录式 `src/bin/monitor/main.rs`，合计预期 40 个 binary target，后续用 Cargo metadata 对账。
- `tests/` 当前有 44 个顶层 integration test 文件，清单与上一轮调查一致。

## 当前工作树复核（批次 2）

- `cargo metadata --no-deps` 确认 package `stock_analysis` 版本 0.1.2：1 个 library target、40 个 binary targets、44 个 integration-test targets、1 个 benchmark、1 个 build script。
- feature 只有 `default = [magic-gateway]` 与 `magic-gateway`；默认 feature 绑定 14 个 `magic-*` provider/router/composition 依赖。
- 三个主运行入口是 `src/main.rs:25-26`、`src/bin/grpc_market_server.rs:6-7`、`src/bin/monitor/main.rs:4471-4472`，均为 Tokio async main；主 CLI 与 monitor 固定 4 worker threads。
- `build.rs` 从唯一 `grpc/market.proto` 生成 client/server；注释明确 Operation 61/62 是本地 server 扩展。`grpc_market_server` 启动时拒绝 `DATA_GATEWAY_GRPC=1`，阻止 provider 进程自调用形成环。
- monitor 在 `src/bin/monitor/main.rs:5485-5487` 并发运行 `monitor_loop`、`news_monitor_loop`、`data_mode_monitor_loop`；`monitor_loop` 定义在 8339 行附近。

## 当前工作树复核（批次 3）

- 已重新抽取 30 个主要模块族的 `mod.rs` 声明；当前 `opportunity` 还包含 `winrate`，应纳入新蓝图，不能沿用旧清单遗漏。
- 最大代码域（Rust 行数）依次为：`bin` 75,844、`database` 73,271、`data_gateway` 58,149、`selection` 25,954、`durable_delivery` 18,834、`performance` 15,594、`pipeline` 14,893、`event` 13,159、`monitor` 12,630。
- 代码体量集中在运行编排、数据库、统一数据网关，而不是均匀分布；蓝图必须单独展开这三个热点及其边界。
- 主要业务域的模块声明与现代码一致：selection、decision、risk、trading、opportunity、review/performance；基础设施域包括 grpc_client/server/contract、event、notification、push_l1/l2/l4/l5/l6/l7、database/data_gateway。

## 当前工作树复核（批次 4）

- `src/grpc_contract/ops.rs:152` 的测试继续冻结 `implemented_operations().len() == 40`；`OptionData` 是冻结枚举但未实现的反例。
- 首次查询错误使用了不存在的 `grpc/market.proto`；实际路径需从 `build.rs` 继续确认后改查，不能把该错误路径写入蓝图。
- selection-v2 的最终表集合由 `src/database/selection_v2.rs:18` 与 `src/database/global_schema_catalog_v1.rs:27` 双重声明并校验，数量仍是 12。
- durable delivery schema 在 `src/durable_delivery/schema.rs:155` 起独立建表，核心表覆盖 decision、policy catalog、audit outbox、cooldown、once claim、daily budget、attempt、sink result、manual resolution、payload 和 append-only event 族；后续精确抽取 18 张表。

## 当前工作树复核（批次 5）

- gRPC 上游唯一合同真实路径为 `client-bundle/market.proto`；`build.rs:26-39` 读取它、幂等合并本地扩展到 OUT_DIR，再用 `tonic_prost_build` 同时生成 client/server。
- `build.rs` 还从 `Cargo.lock` 提取并冻结 `magic-tdx-rs` 的精确 Git revision；这是构建期供应链约束。
- 冻结 Operation 枚举值覆盖 0..=62（含 Unspecified 共 63 个值），而 `src/grpc_contract/ops.rs:77-128` 明确列出 40 个生产实现。
- implemented 集合包含 56-60 的兼容/派生入口以及本地扩展 61 `ChainBatch`、62 `BenchmarkBars`；未实现 operation 仍有完整方法名映射，但客户端应以 `is_implemented` 区分。
- 第二次命令尾部仍误查了 `proto/market.proto`；已从同一输出的 `build.rs` 与 `find` 确认正确路径，下一批直接读取 `client-bundle/market.proto`。

## 当前工作树复核（批次 6）

- selection-v2 final catalog 是 12 tables + 5 indexes + 17 static triggers；另为 9 张 stage tables 生成成员约束，为 relation/evaluation/sample 三张表生成 symbol 约束。
- selection-v2 payload schema 当前 final 集为 5 个：config activation v1、source ingress v2、generation v3、outcome claim v2、outcome v3；transitional 集保留 outcome v2。
- durable delivery 独立 schema 精确包含 18 张表：decision/policy/audit、cooldown 与 budget reservation/event、attempt/sink/event、review replay、manual resolution、delivery/task payload。
- `DecisionState` 在 `src/durable_delivery/model.rs:800-814` 有 14 个状态；合法转换由 `src/durable_delivery/coordinator.rs:7123` 的显式状态对函数控制，不能按字符串自由迁移。

## 当前工作树复核（批次 7）

- `client-bundle/market.proto` 当前上游 Operation 枚举冻结到 60；build script 合并本地 61/62 后，生成端才形成 0..=62 的完整枚举。
- durable decision 状态机当前明确允许 20 条 transition；包括失败后从 `RejectedDurable -> Reserved` 重试，以及 `UncertainManualReview` 经人工处置回到 accepted 审计链或进入 manual-rejected 链。
- legacy global schema fixture 文件首部自证冻结形状为 53 tables + 44 explicit indexes + 63 triggers；本批使用了错误的 tab 分隔符而未得到干净计数，下一批改用 `|` 重新对账。

## 当前工作树复核（批次 8）

- 按真实 `|` 分隔重新统计，`global_schema_legacy_catalog_v1.tsv` 精确得到 53 tables、44 indexes、63 triggers；53 张表清单已重新抽取。
- `data_gateway::grpc_source::bridge_for` 仅在 `DATA_GATEWAY_GRPC=1` 且 operation 未列入 `DATA_GATEWAY_GRPC_DISABLED` 时返回桥；连接本身是 lazy，方法调用层 fail-closed。
- `HOOKED_OPS` 与实际 `bridge_for("...")` 调用点由 `src/data_gateway/grpc_source.rs:5388` 的源码扫描测试保持一致，避免启动 banner 与真实桥接覆盖漂移。
- 架构边界主要由 `tests/unified_data_architecture.rs` 的源码导入约束保障：Magic provider 只能位于 `src/data_gateway/**`（显式豁免单列），不是靠多 crate 物理隔离。
- 当前确有可见反向依赖：`data_gateway/grpc_source.rs` 使用 database audit/manager，`database/mod.rs` 又使用 selection types 并在测试中调用 data_gateway；蓝图应标为已知模块循环，而不能虚构严格单向 Clean Architecture。

## 当前工作树复核（批次 9）

- 配置实现是单文件 `src/config.rs`：`load_all()` 读取 `config/strategy.toml` 与 `config/chain.toml`，运行态快照通过 `ArcSwap`/`RwLock` 暴露；读取或投影失败时保留前值/默认值。
- 主 CLI 和 monitor 启动加载 `.env`；只有 schedule path 在 `src/app/schedule.rs:230` 用 override 方式重载 `.env`。蓝图不能声称 monitor 全量热重载。
- selection 还使用 JSON 配置/日历/activation 文件；`config/design_contracts.toml` 与 BR-196 非生产飞书目标也属于治理配置。
- gRPC client 的 external bundle 同时要求 HTTPS endpoint、server name、CA、client certificate/key、instance bearer token；凭证使用 `Zeroizing`，authorization debug/tracing 被测试为隐藏。
- client endpoint timeout 为 35 秒；Unavailable/DeadlineExceeded 走受限重试，指数退避上限 60 秒；invalid/auth/permission/unimplemented/failed-precondition 默认不可重试，远端 typed retryability 也不能把 invalid request 变成可重试。
- server 暴露 health/capabilities；40 个 capability 来自 `implemented_operations()`，错误详情保留 provider/reason_code/retryable 分类。
- 本批的 `src/config` 与 `src/auth.rs` 查询路径不存在；真实实现分别是 `src/config.rs` 与 `src/grpc_client/auth.rs`，以上结论按真实文件输出记录。

## 当前工作树复核（批次 10）

- 存在两套 event bus：`src/event/bus.rs` 是带 publish outcome/metrics/shutdown 的通用 `EventEnvelope` broadcast bus；`src/monitor/event_bus.rs` 是 monitor-domain `MonitorEvent` 的全局 broadcast bus。两者不可合并描述成同一类型。
- JSONL replay 默认 dry-run；force replay 生成新 id 并保留 `replay_of`，跳过 delivery audit event，publish failure 会计入失败而非假定成功。
- durable immutable audit adapter 写入独立锁定、hash-chained JSONL 并在 `fsync` 后返回；生产目录默认 `data/durable_delivery_audit`，依赖 Unix openat/mkdirat/flock 语义。
- durable delivery 的端口包括 `AuthoritativeSinkPort` 与 `ImmutableAppendPort`；协调器是 `DurableDeliveryCoordinator`，状态和持久化不等同于旧 push_l4 内存 dedup。
- push 分层实际并存：L1 SignalEvent、L2 template metadata、L4 dispatcher/dedup、L5 governance、L6 sink/router、L7 analytics SQLite；`src/lib.rs` 明确注释 L3 缺失，实际 render 位于 monitor `push_templates`。
- L6 提供 Console/HTTP/Wechat/Feishu sinks；但生产 monitor 另有 `notify.rs` 与 `durable_delivery_runtime.rs`，蓝图应明确“基础库分层”与“生产接线”不是同义词。
- 本批附带查询了不存在的 `src/bus.rs`；通用 event bus 的真实路径为 `src/event/bus.rs`，另有目录式顶层 `src/bus/` 模块。

## 当前工作树复核（批次 11）

- `event::Dispatcher` 以 event_type 精确匹配，registry 校验重复注册；`AuditDispatcher` 的生产 authority 固定在 `data/event_audit`，拒绝把任意 caller path 提升为可写审计 authority。
- `event/jsonl_writer.rs` 自述是 non-authoritative observation/replay projection；生产 delivery evidence 先由 AuditDispatcher 的 hash-chain + `sync_data` 提交。JSONL 投影失败仍会导致 monitor lifecycle 失败。
- monitor service mode 的 main loops 当前是 4 个：`p01_scheduler_loop`、`monitor_loop`、`news_monitor_loop(selection_v2_enabled)`、`data_mode_monitor_loop`。
- monitor background tasks 当前是 7 个：dry-run reporter、monitor event consumer、post-close news、post-session review、position-chain refresh、opening static readiness、opening live readiness。
- 长运行生命周期由 `supervise_long_running_lifecycle` 同时监督 main loops、background tasks、SIGINT 与 JSONL writer；致命失败 exit 2。
- `monitor_loop` 内部最后并行 join `intraday_loop` 与 `market_loop`。慢 event consumer 的 broadcast lag 会显式记录丢失条数后继续。

## 当前工作树复核（批次 12）

- monitor 启动顺序包含：终端模式短路、生产 dry-run 拒绝、singleton lease、selection-v2 activation gate、test durable namespace、delivery mode 校验、audit preflight、durable artifact eager bind、JSONL writer、DB/config、startup reconciliation，之后才启用长期 producer loops。
- monitor 明确日志声明 `delivery audit mode=synchronous_durable; bus=observation_only`；全局 durable reconciliation 未到固定点会阻止 producer activation。
- default CLI 先执行 operator auth，再验证至少一个 AI key 和通知配置；DB 初始化失败只记录“数据不会入库”并继续，这与 monitor 的审计/持久化 fail-closed 行为不同。
- default CLI 股票池实际来自手工/环境列表、宏观 AI、龙虎榜 Top10、涨停池、持仓，再通过统一 SecurityIdentity gateway 做退市过滤；deep-analysis 模式禁用自动扩展。
- 默认单次与龙虎榜模式进入 `AnalysisPipeline`；另有 schedule、chain-analysis、market-review、deep-analysis 分支。schedule 每轮只重载 `.env` 并重建股票池，未重新调用 `config::load_all()`。
- CLI 单次流程构造 `PipelineConfig` 时注入 worker、dry-run、notify 和 DQ freshness 阈值；结果按 sentiment score 排序展示，pipeline 内负责更宽的报告/保存/通知逻辑。

## 当前工作树复核（批次 13）

- `AnalysisPipeline` 是 default CLI 的应用服务：历史 bars 经 `HistoricalBarsGateway` 获取并保存，逐股分析使用有界异步并发；结果模型是宽 `AnalysisResult`，含 technical/news/score/veto/risk 等多类投影。
- pipeline 可选执行关键股票 deep enrichment、产业链段落、回测摘要、reports 文件与 NotificationService；持仓跟踪同时依赖 database、monitor::risk 与 risk 模块。
- selection-v2 不是一个简单筛选函数，而是阶段化/可恢复工作流：config activation → source ingress/admission → feature/evaluation/sample → outcome claim/settlement，阶段 persistence 返回 receipt 并通过 read-back 验证。
- selection outcome 由唯一 `OutcomeSettlementOwner` 编排；它先清空恢复队列并重新校验 due/receipt/audit/DB binding，再持久 claim，之后才请求 provider，最后持久 outcome receipt。
- selection-v2 生产能力受 activation runtime 分拆控制；generation-only release 与 outcome capability 可以独立禁用，不能把“代码存在”写成“默认已启用”。
- outcome claim 还使用 descriptor-relative、no-follow 的跨进程文件锁；生产数据库未初始化会 fail-closed。

## 当前工作树复核（批次 14）

- monitor 启动探测 `stock_analysis::broker::detect_and_register()`，但代码注释明确用户确认的账户快照在真实 broker 接入前仅用于展示，完整 account metrics 与 trade-sync watermark 不可用时风险/复盘能力保持受限。
- trading 域由 order_safety、risk_adapter、paper_trade/paper_sell 和 legacy paper_engine 构成；需进一步用精确调用点区分 active simulation 与被禁止的 legacy engine。
- `LlmProvider` 是 async trait，基础 `chat_json` 与带真实上游 model/response receipt 的 `chat_json_with_receipt` 分开；不支持 receipt 的 provider 显式返回 `ReceiptUnavailable`，不会伪造回执。
- `LlmRegistry` 按环境配置的 role/fallback 选 provider；ticker 提取失败可业务降级为空列表。AgentRunner 有 toolbelt、validator、fallback、重复调用防循环和事实表。
- multi-agent pipeline 代码真实存在（domain slices、analysts/debate/arbitrator/cost board），宏观推荐由 `MACRO_AGENT_PIPELINE` 可选启用；default CLI 的 `--deep-analysis` 也是显式分支，不能把 agent 子系统描述成所有股票分析的强制主路径。
- 本批查询了不存在的目录 `src/broker/`；顶层 broker 实现为单文件 `src/broker.rs`。

## 当前工作树复核（批次 15）

- active paper paths are evidenced by `decision/intraday_monitor.rs` and `trading/paper_sell.rs`: obtain broker execution quote, preserve real limit-up/down flags and observed_at, then call `paper_trade::simulate` / `simulate_with_audit_evidence` and persist paper trade + order audit.
- `order_safety.rs` is shared fail-closed validation for simulated/paper orders. This repository does not contain real order-routing execution; broker currently supplies quote/probe capability.
- business DB uses Diesel SQLite with r2d2 pool and a `OnceCell<DatabaseManager>` singleton. Selection/attribution sensitive reads additionally use descriptor-bound connection identity, query-only/PRAGMA checks, retained proofs and namespace-swap detection.
- DatabaseManager owns operational and optional attribution/selection connection sources; all checked-out/rebuilt connections are configured and verified.
- durable delivery uses a separate rusqlite store/runtime and separate schema/artifact namespace; it must be drawn as physically isolated reliability storage, not another table family inside the Diesel business pool.

## 当前工作树复核（批次 16）

- 精确调用点确认：legacy `paper_engine::run_once` 已从 production `monitor_loop` 隔离，函数本身返回 disabled error；原因是缺少 BR-201/BR-205 guarded owner/source-backed price limits。
- active 生产模拟路径仍包括 `IntradayMonitor -> paper_trade::simulate`，以及 monitor 盘中/盘后调用 `paper_sell::scan_and_sell* -> simulate_with_audit_evidence`；position ledger invalid 时 sell 扫描暂停。
- durable runtime production manifest 明确数据库为 `data/durable_delivery.sqlite3`；test 必须使用 path-safe `DURABLE_DELIVERY_TEST_CODE` 的隔离 namespace，并检查 foreign-CWD 不得触碰生产 artifacts。
- 本批假设了不存在的 `src/durable_delivery/store.rs`；实际 rusqlite connection/coordinator 位于 `src/durable_delivery/coordinator.rs`，生产 namespace binding 在 monitor-local `durable_delivery_runtime.rs`。

## 当前工作树复核（批次 17）

- README 当前明确生产建议：`grpc_market_server` 使用 default feature 承载固定 revision Magic providers，`monitor` 用 `--no-default-features` 构建/运行并设置 `DATA_GATEWAY_GRPC=1`。
- `client-bundle/` 保存 remote mTLS CA、client cert/key、Bearer token、proto 与 connection descriptor；README 要求目录 0700、敏感文件 0600，并提供 opening probe 后再启 monitor 的顺序。
- 仓库没有根级 Dockerfile/docker-compose/systemd unit；部署文档是命令/环境变量与 probes，不能画成容器编排平台。
- 有 `tools/compliance/check.sh` 和一组 `tools/one_shot` 迁移/回填/健康/验证脚本；40 binaries 中相当一部分也是 probes/backfills/importers，而非常驻服务。
- CI 首次 glob 因 zsh 对不存在的 `*.yaml` 立即报 `no matches found`，未得到 workflow 内容；下一批改用 `find` 逐个读取。

## 当前工作树复核（批次 18）

- 当前 4 个 GitHub Actions workflows：`ci.yml`、`compliance.yml`、`coverage.yml`、`pr-template-lint.yml`。
- 主 CI 执行 fmt、strict clippy `--all-targets --all-features -D warnings`、tests `--all-targets --all-features`。
- coverage 固定 Rust 1.95.0 与 cargo-llvm-cov 0.8.7，收集 complete workspace coverage，并按 `config/design_contracts.toml` 与 diff base 执行 Gate C threshold。
- compliance 安装 SQLite/Python，运行 offline compliance、选择的 unit/e2e tests 与 PR spec evidence；PR-template lint 只在 monitor/push template/notify 相关路径触发。
- README 确认 provider host 与 monitor 共用同一个 business database，但 provider implementation 只链接在 server；durable delivery DB/audit artifacts 仍独立。
- `.gitignore` 忽略 `/docs`、`/config`、`/tests`、`/.github` 等本地内容，因此正式可见 blueprint 放根目录；文档仍可引用这些当前存在的代码/配置/测试证据。
- README 的 client-bundle 声称 60 RPC，`client-bundle/market.proto` 上游到 60；build.rs 再加 61/62 本地 RPC，需在蓝图区分 upstream 与 merged local contract。

## 生成决策

- 计划复读确认：生成根目录 `Project_Architecture_Blueprint.md`，使用 portable Mermaid flowchart/sequence/state 表达 C4-oriented 层级，避免依赖非标准 C4 renderer。
- 实现模式代码示例将直接摘自 `LlmProvider`、`AuthoritativeSinkPort`/`ImmutableAppendPort`、`implemented_operations`、`bridge_for`、`DatabaseManager` 和 `legal_transition`，保持短小并附源路径/行号。
- 完整性附录将逐项列出 62 顶层 modules、主要 submodules、40 binaries、44 integration tests、40 implemented gRPC operations、14 decision states、53/12/18 schema tables。
- 状态标签统一为 CURRENT、CONDITIONAL、COMPAT、INACTIVE、EXTERNAL；代码推导的 architecture decisions 明确标记为 INFERRED，不冒充已有 ADR 文件。

## 当前工作树复核（批次 19：增量 DDL）

- 53-table legacy frozen catalog 不是业务 DB 的全部现存 DDL；代码还有独立 owner 管理的增量表族，蓝图必须单列，不能把 53 误写为“数据库总表数”。
- 主要增量表族：attribution epoch/report/audit/chain；benchmark segment/manifest；catalyst watchlist；news AI delivery event/chain；paper inventory failure audit/chain；daily-change v2；selection-v2 generation cadence/audit closures；`candidate_trigger_selection`、`holding_plan_daily`、`push_analytics`。
- 这些增量 DDL 分散在 `database/attribution_epochs.rs`、`attribution_reports.rs`、`benchmark_segments.rs`、`catalyst_watchlist.rs`、`news_ai.rs`、`paper_inventory_failure_audit.rs`、`selection_v2_generation_journal.rs`、`decision/holding_plan.rs`、`push_l7/sqlite_store.rs`；测试临时表和迁移中间表不得计入生产 catalog。

## 蓝图对账（批次 1）

- `Project_Architecture_Blueprint.md` 当前 1,324 行、78,876 bytes，包含 1 个 H1、33 个 H2、54 个 H3、18 个 Mermaid blocks；总 code fences=50，为偶数，结构闭合。
- 从文档抽取的 53 个实际 evidence paths 全部存在，missing=0。
- 首次集合对账脚本在 zsh 下把多行 command substitution 当成一个参数，导致 `file name too long`/regex error；这是验证脚本错误，不是文档缺项。下一次用 `while IFS= read -r` 逐项核验。

## 蓝图对账（批次 2）

- 逐行集合对账全部通过：modules 62/62、binaries 40/40、tests 44/44、implemented gRPC ops 40/40、DecisionState 14/14、legacy tables 53/53、selection-v2 tables 12/12、durable tables 18/18，missing 均为 0。
- 本机没有 `mmdc`，因此不声称完成浏览器级 Mermaid 渲染；对 18 个 Mermaid blocks 做了 header 与 `[]`/`()`/`{}` 平衡检查，structural_errors=0。
- 文档结构、证据路径、inventory 完整性已通过静态核验；进入 verification-before-completion 的 fresh test 与工作树检查。

## 最终验证（批次 1）

- 已完整读取 `verification-before-completion` skill，按“fresh evidence before claims”执行。
- fresh command `cargo test --test unified_data_architecture -- --test-threads=1` exit 0：15 passed、0 failed、0 ignored；统一数据边界与 provider ownership 的现行架构测试通过。

## 最终验证（批次 2）

- fresh 全量文档核验 exit 0：required sections missing=0、53 evidence paths missing=0、18 Mermaid blocks malformed=0。
- fresh inventory 对账再次全部通过：62 modules、40 binaries、44 tests、40 gRPC ops、14 states、53 legacy tables、12 selection tables、18 durable tables，所有 missing=0。
- `git diff --check` exit 0；`git status --short` 仅有本任务生成的 blueprint 与 3 个 planning artifacts，未修改产品代码。
- blueprint 最终静态快照：1,324 行、6,443 words、78,876 bytes；SHA-256 `fa0a76005473fb4babd28aa68b0a05218a4128a7572d3061c7f1d9be801e02c3`。

## 完成审计

- 重新逐项读取计划：6 个阶段全部满足；已安装 skill 的 `SKILL.md` fresh 检查存在且为 322 行。
- 本任务只新增 documentation/planning artifacts；没有产品代码、配置、测试或 CI 文件被修改。

## 网页化扩展

- 用户已批准 bounded 设计：单文件静态网页，包含响应式目录、搜索、折叠、状态着色、18 张 Mermaid 图、源码证据和失败回退。
- TDD seam 已由批准的设计确定为 browser-facing HTML public contract，不测试内部生成细节。
- planning session catchup exit 0，无未同步上下文报告。
- 本机没有 pandoc、常见 Markdown CLI、Playwright 或 Chromium；Node.js 24.10.0 可用，因此采用无构建依赖的静态 HTML 生成方式。
- 网页将预生成完整 HTML 正文，同时内嵌原始 Markdown（Base64）与 SHA-256；只有 Mermaid 图增强依赖 CDN，失败时每张图仍显示原始 Mermaid 文本。
- 仓库没有可复用的架构网页模板；旧 HTML 主要是报告产物，不适合作为交互式架构导航壳。
- Markdown 已确认包含 18 个 Mermaid 代码块；网页验收将同时核对源文件哈希、图数量和关键交互控件。
- TDD RED fresh evidence：`ruby /private/tmp/verify_architecture_blueprint_web.rb ...` exit 1，唯一失败为目标 HTML 尚不存在。
- GREEN 生成结果：静态网页正文在构建时完整展开并内嵌原始 Markdown；目录、全文筛选、章节折叠、主题、打印、图缩放/全屏均为浏览器原生 JavaScript，无后端依赖。
- 页面脚本静态编译通过；Quick Look 能把目标 HTML 识别并渲染为网页缩略图，说明文档不是仅供 HTTP server 使用的空壳。
- 1600px 首屏视觉检查通过：固定侧栏、搜索工具栏、响应式主栏、架构标题、8 个事实指标和 5 类状态图例均无重叠或截断；暗色信息密度符合架构控制台用途。
- 系统自带 `/usr/bin/tidy` 与 `/usr/bin/xmllint`，可继续用于 HTML 标记级检查；Quick Look 不执行页面脚本，因此其空目录不代表浏览器目录失败。
- Apple 自带 Tidy 是 2006-10-31 版本，不识别 HTML5 的 `aside`/`nav`/`header`/inline SVG，exit 2 属于验证器能力不匹配，不能作为页面标记错误证据；改用针对 HTML5 结构的项目验收。
- 本机存在 Safari 与 `safaridriver`，可尝试 WebDriver 实际运行验证；若系统未启用 Remote Automation，不应使用 `--enable` 擅自修改系统设置，届时以 Quick Look + 静态/契约验证为边界。
- 可选 Node DOM/browser 测试包（jsdom/linkedom/happy-dom/playwright/puppeteer）均未安装；不为本静态交付引入大型依赖。
- 扩展验收首次运行因系统 Ruby 缺少较新 `Array#tally` API 而失败，属于测试兼容性问题；改为兼容写法后重跑。
- 增强验收通过：源 SHA/内嵌字节一致，33 H2、54 H3、25 code blocks、25 tables 与 Markdown 一一对应；18 个图都有原文回退，所有 HTML id 唯一且 ARIA controls 有目标。
- 最终阶段已完整读取 `verification-before-completion` skill；后续完成声明只依据本阶段重新运行的命令和完整输出。
- 最终 fresh verification：HTML 4,108 行 / 314,595 bytes，SHA-256 `3d97373deb4a64a168f5aa3cce16791b8160c89ce304f6a8109604920e923a76`；网页验收、内嵌 JS 编译和 `git diff --check` 均 exit 0。
- `git status --short` 仅列出本任务的 HTML、Markdown 与 3 个 planning artifacts；产品代码未修改。

## 浏览器运行态验证扩展

- 用户要求继续；将范围收敛到此前尚未自动化覆盖的真实浏览器运行态，不扩展或改写已由代码对账的架构事实。
- 已重新完整读取 Architecture Blueprint Generator 与 planning-with-files skills；运行态验证仍以 Markdown 唯一事实源和 18 张图完整性为边界。
- planning session catchup exit 0 且无 unsynced report；重读计划确认当前仅 B1 进行中，产品代码仍不在修改范围。
- 本地端口 8765 当前无监听者，可用于隔离静态服务；SafariDriver 随 Safari 26.6.2 提供。
- 已启动只绑定 `127.0.0.1:8765` 的临时静态服务与 `127.0.0.1:4444` SafariDriver；没有使用 `safaridriver --enable` 修改系统安全设置。
- Safari WebDriver 明确返回 session not created：必须由用户在 Safari Developer Settings 手动启用 Allow remote automation。遵守范围约束，不自动改变该设置，转查现有本地浏览器/cache。
- 没有 Playwright/Puppeteer 浏览器 cache，但 `/Applications/Google Chrome.app` 实际存在；此前只检查 PATH 上的 `google-chrome` 因而漏检。改用 app bundle 内的 Chrome executable 做 headless/CDP 验证。
- Chrome 151.0.7922.174 已用 `mktemp` 创建的独立 user-data-dir 启动，监听本机 CDP 9222；不会读取/修改日常 Chrome profile。
- 浏览器验收首次普通沙箱调用因连接本机 CDP 返回 EPERM；提升到明确获批的本地连接后脚本成功运行，但暴露真实问题：18 张 Mermaid 图未全部渲染成功。当前尚未确定具体失败图或根因。
- 诊断快照稳定复现：Mermaid CDN HTTP 200、runtime 已加载，DOM/87 个目录/33+54 标题完整；18 图中仅 2 个 SVG 成功、16 个失败。失败集中为 Base64 `atob` invalid 或解码后出现乱码并触发 Mermaid lexical error；18 个可见 fallback 原文均完整正确，故故障边界位于隐藏 Base64 source → `decodeDiagram`，不在 Markdown 图内容或 CDN。
- 根因已定位到生成 HTML 的两处正则：预期 `/\\s+/g` 实际落盘为 `/s+/g`（HTML 3909、4031）。`decodeDiagram` 因此删除 Base64 中所有小写 `s`，造成 payload 损坏；搜索 normalization 也错误删除字母 s。该差异完全解释 atob invalid、乱码和仅偶发解析成功。
- 单一修复假设：只把这两处 `/s+/g` 恢复为 `/\\s+/g`，可同时恢复 Base64 完整解码与搜索空白归一化；浏览器验收仍是现成 RED 测试。
- 最小修复后的同一 Chrome 验收证实假设：Mermaid runtime/CDN 正常，18/18 `data-rendered=true`、18/18 SVG、0 个 render failure；桌面无 page-level 横向溢出。随后验收在“mobile sidebar is onscreen”处失败，这是后续独立检查，尚需区分产品布局问题与 `.22s` CSS transition 的测试时序问题。
- 移动侧栏时间采样确认页面正确：点击瞬间 body/ARIA 已开启，sidebar 仍处于 `translateX(-310.08px)`；350ms 后（CSS transition 为 220ms）left=0、transform=0。失败属于测试过早读取，已用条件对应的 settled 断言替换。
- 验收随后只剩两条无 URL 的 404 resource log；需从 HTTP server/Network 层定位资源再判断是否页面缺陷，尚不直接忽略。
- HTTP access log 精确定位每次唯一 404 为浏览器自动请求 `/favicon.ico`；页面 HTML 与 Mermaid CDN 均为 200。单一修复是在 `<head>` 内嵌 data-URI favicon，消除无关网络请求而不增加文件/后端依赖。
- favicon 修复后的 fresh HTTP access log 只有 HTML 200、没有任何 404；Chrome `Log.enable` 仍回放一条先前页面的 buffered 404，说明剩余失败属于 CDP log session 污染而非当前页面请求。验收应在 reload 前清空 Log/console 与本地收集数组，继续保持“current run 零错误”断言。
- 完整 Chrome 151 回归 exit 0：87 nav links、33 H2、54 H3、18/18 Mermaid SVG + 18 fallbacks；搜索 `durable delivery` 得到 10 处/6 节，折叠 33/展开 0，dark→light 主题切换，图缩放/reset/fullscreen 均通过。
- 390×844 emulation 无页面横向溢出；菜单可见，点击后 ARIA 正确且 350ms settled left=0，backdrop 可关闭。Mermaid CDN HTTP 200，current-run browser errors=[]。
- Chrome 生成两张真实运行态截图：`/private/tmp/Project_Architecture_Blueprint.browser.png` 和 `/private/tmp/Project_Architecture_Blueprint.diagram.browser.png`。
- 真实浏览器首屏截图视觉通过：87 项侧栏目录已填充，搜索/工具栏、Hero、8 个事实指标、状态图例均无重叠或截断。
- 原“首图截图”只捕获到首张图之前的架构结论/代码热点表，未把 SVG 纳入画面；这是截图取景不足，不是 DOM/SVG 验收失败。改用元素 page-coordinate clip 重新截取首张 `.diagram-shell`。
- 元素坐标截图复核通过：FIG 01 工具栏、网格画布、外部操作者/Magic Market/remote bundle、系统核心节点、连线与标签均实际可见；超出视口的右侧节点由 diagram-stage 横向滚动与缩放控件承载，未造成 page-level overflow。
- 最终阶段发现旧 `/private/tmp/verify_architecture_blueprint_web.rb` 已被系统清理；正式产物不受影响。为保证 fresh evidence，将重建自包含静态 verifier，并加入 `/s+/g` 回归与内嵌 favicon 断言。
- 最终 fresh verification 全部 exit 0：Chrome runtime 18/18 SVG、87 nav、搜索/折叠/主题/图控件/390px mobile、browserErrors=[]；静态契约源字节 exact、33+54 标题、25 code blocks、25 tables、18 fallbacks、ID/ARIA/regex/favicon guards；2 段 inline JS compile；`git diff --check`。
- HTML 当前 SHA-256 `c433208ba47e2179676ec234f37cd5f8354c00bb055c50b1b697bc18783fb709`；Markdown 仍为 `fa0a76005473fb4babd28aa68b0a05218a4128a7572d3061c7f1d9be801e02c3`。
- final `git status` 还显示 selection 配置、8 个 data_gateway 文件和 `src/grpc_client/errors.rs` 的并发/用户修改；本任务未写这些文件，保持原样。正式新增仍是 HTML/Markdown 与 3 个 planning artifacts。
- 已用 Ctrl-C 正常停止本任务启动的隔离 Chrome 与 HTTP server；两者 exit 0。
- cleanup fresh check：`lsof` 对 8765/9222 均 exit 1（无监听者）；重读计划确认 B1-B4 无剩余阶段。

## 最近改动与架构网页漂移审计

- 用户要求整理最近改动并判断架构网页是否需要调整；本轮是基于证据的 review/report，不默认获得改写网页或产品代码的授权。
- 已重新完整读取 Architecture Blueprint Generator 与 planning-with-files skills；以 2026-08-30 蓝图和当前代码/版本控制状态为双基线。
- planning catchup exit 0；当前分支 `master`。
- 8 月 30 日蓝图之后出现一组重大架构提交：移除 Magic dependencies、把市场领域类型迁入 `market_domain`、强制远程 market data transport、删除本地 provider gateway branches/local TDX transport、删除仓内 gRPC provider host、完成 grpc-only migration；随后修复 gRPC business-state wire 语义并加入 BR-249 NewsAI 产业链/告警恢复。
- 当前未提交产品改动已收敛为 `config/selection/selection_activation.v1.json` 与 `src/bin/monitor/main.rs`（25 insertions/28 deletions）；此前出现的 data_gateway/grpc_client 改动已进入提交。另有 `.brooks-lint-history.json` 与本任务文档未跟踪。
- 现有蓝图明确描述“仓内 `grpc_market_server` + default Magic provider + local/library route”，与当前提交历史存在结构性漂移；网页需要调整已是确定结论，仍需精确定位章节和新清单。
- 当前代码直接证据：`src/lib.rs:226` 已写明 provider host lives outside repository，`src/lib.rs:232` 暴露 `market_domain`；`grpc_source.rs` 是 remote provider host bridge。蓝图却仍在至少 20 处引用仓内 `grpc_market_server`、`grpc_server`、`magic-gateway`、local library route、provider/monitor 双进程与 40 binaries。
- 当前未提交 `monitor/main.rs` 还改变 paper-sell 运行态：从默认暂停、`PAPER_SELL_ENABLED=1` 才启用，改为 FIFO ledger 修复后默认放行、仅 `PAPER_SELL_DISABLED=1` 逃生禁用。蓝图中“position ledger invalid 时 sell 扫描暂停”的当前态也将过期。
- selection activation receipt 未提交改动只更新 expected config hash/effective/reviewer；影响配置激活证据，不改变组件拓扑。
- 首次 inventory 脚本因系统 Ruby 不支持 `filter_map` 失败；按兼容写法重跑，不能沿用旧 62/40/44 数字。
- 兼容 inventory fresh 结果：public modules 61（旧 62）、binaries 26（旧 40）、top-level integration tests 41（旧 44）。新增 `market_domain`，删除 `grpc_server`/`magic_compat` 等后净 module count -1；删除 14 个 probes/server/tools 后 binary count -14；测试 fixture 移入 `tests/support` 且删除部分旧 transport tests 后 top-level test count -3。
- 从 Magic dependency removal 起至当前 HEAD 的迁移规模为 155 files、3029 insertions、36332 deletions，属于架构重构而非局部实现变化。
- `src/lib.rs:224-233` 当前定义：gRPC client/contract 共享 `grpc/market.proto`，provider host lives outside this repository，client repository owns provider-neutral `market_domain` types；没有 `pub mod grpc_server`。
- 当前 README/.env/Cargo/tests 中已无蓝图搜索所用的 `magic-gateway`、`grpc_market_server`、`DATA_GATEWAY_GRPC` 等旧运行模式标记，旧页面部署与 feature 章节不能只换名称，需重画边界。
- README 当前事实源已彻底切换：生产进程只经版本化 gRPC 消费外部 provider-host；本仓不含 provider implementation/server target/local fallback；失败时显式失败。代码结构新增 `market_domain`，本地 tonic fixture 只在 `tests/support/grpc_fixture`。
- Cargo 当前没有 `[features]` 或 `magic-gateway`，也没有 Magic provider dependency；生产构建命令只包含 `monitor`/`grpc_bundle_probe`，因此蓝图 feature matrix、`--no-default-features` 和 provider server 构建说明必须删除。
- `grpc_contract::ops::implemented_operations()` 仍由测试锁定为 40，所以“40 ops”数字可保留；但语义必须从“仓内 server implemented set/handlers”改为“client contract registry + 外部 provider capabilities 协商”。
- `data_gateway::grpc_source::bridge_for` 当前无 local/remote switch，始终构造 process-wide remote `GrpcSource`，地址来自 `GRPC_MARKET_ADDR`，可选 external client bundle；原 `DATA_GATEWAY_GRPC` 条件路由图必须替换为强制远程、fail-closed 流程。
- 蓝图 evidence-path 粗对账找到 3 个确定已删除的核心路径：`src/bin/grpc_market_server.rs`、`src/grpc_server/delegate.rs`、`src/grpc_server/mod.rs`（另外两个命中为叙述片段假阳性）。这些路径出现在系统上下文、调用链、错误语义、扩展指南和部署证据中。
- 新 `scripts/check-no-magic-dependencies.sh` 是当前治理门：禁止 Magic Cargo/Git dependency、`magic-gateway` cfg、TDX lock attestation、所有 upstream Rust path，以及 grpc server/provider-only targets。蓝图 Governance 必须加入该 gate。
- `tests/unified_data_architecture.rs` 自身仍保留 “magic_compat reviewed boundary” 注释/allowlist 等历史措辞；虽然当前无对应源码且 no-magic 脚本提供更强 gate，这属于架构治理测试的次级文档债，建议与蓝图更新一起清理，但不是网页渲染问题。
- BR-249 是现有组件内的重要能力扩展而非新顶层 subsystem：NewsAI request 增加 `NewsAiChainContext`（chain_daily + position-chain 证据）；detector 增加 `ChainRisk`；R-03 industry-chain dispatcher/盘中推送增强；durable delivery 对 `RejectedDurable` 增加显式 retry authorization 和 startup producer-ready 屏障。
- BR-249 未新增 NewsAI/durable 表：`database/news_ai.rs` 仅测试适配 chain context，durable coordinator 复用 `delivery_decisions.retry_authorized`。`closing_valuation_run/item` 两表当前存在，需继续确认蓝图增量 DDL 目录是否已覆盖。
- 蓝图对 paper sell 的明确 CURRENT 结论仍是 invalid ledger 暂停；当前未提交代码相反。因此若网页现在更新，应把该项标为 “WORKTREE / pending commit”，避免把未提交运行态冒充稳定 HEAD。
- `docs/ARCHITECTURE.md`（更新 2026-08-31）已经提供当前权威的高层替代图：external providers → external provider-host → versioned gRPC → grpc_client/contract → data_gateway admission → monitor/selection/review/decision → durable delivery。网页应以此为新 Level 1/2 骨架，再保留原蓝图更深的业务/存储细节。
- 当前部署只有本仓 `monitor`/CLI/probes + 外部 provider-host；业务 SQLite 与 isolated durable-delivery SQLite 仍成立。测试拓扑则是 `tests/support/grpc_fixture` 的 test-only tonic server，不应画进生产容器图。
- 代码热点需要刷新：`src/data_gateway` 从旧 58,149 行降到 30,555；`src/bin` 当前 75,259、database 73,350、selection 25,953、durable 18,856、monitor 12,732。新增热点/边界应列 `market_domain` 1,968、grpc_client 3,074、grpc_contract 930。
- no-magic 脚本当前未在 `.github`/tools/Cargo 中检索到自动 wiring，只在 README/architecture docs 作为命令出现；网页 Governance 应准确写成“仓库提供的静态 gate/推荐验证命令”，不能声称 CI 已强制执行，除非后续接入 workflow。
## 2026-09-01 最近改动与架构网页漂移审计（补充）

- `monitor` 的四条主循环仍由 `src/bin/monitor/main.rs` 中的 `tokio::join!` 汇合；后台任务集合仍在同一组合根构造，但现有网页中的源码行号已经漂移，需要按当前代码刷新。
- 当前 `monitor` 二进制本地组合模块除既有通知、复盘、交付、健康检查等模块外，还明确包含 `blocking_market_data`、`closing_valuation_runtime`、`data_mode_probe`、`intraday_market`、`news_aggregator_init` 等，网页的组合根清单需要补齐并更新证据路径。
- BR-249 没有新增 NewsAI 或 durable-delivery 数据表；它扩展了行业链上下文、`ChainRisk` 告警、历史业务日 review backfill、被拒交付的显式重试授权，以及 closing valuation 的精确日期检查。
- 当前网页的增量表清单遗漏了已经存在的 `closing_valuation_run` 与 `closing_valuation_item`；即使本次提交没有新建表，数据所有权清单仍应补全。
- 审计期间模拟卖出调整已提交为 `e2503f2`（2026-09-01 06:28 +0800）：从“默认暂停、显式启用”改成“默认允许、`PAPER_SELL_DISABLED=1` 显式禁用”，selection activation 凭据同步更新。它现在属于最新 HEAD，网页 10.1 的旧说明必须直接更新，不再标记为 WORKTREE。
- `monitor` 当前仍有 4 条主循环：P01 scheduler、行情 monitor、新闻 monitor、data-mode monitor；后台任务已经从网页记录的 7 个变为 8 个，新增启动期 `review_backfill`，其余为 dry-run reporter、event consumer、盘后新闻、盘后复盘、持仓行业链刷新、开盘静态就绪、开盘实时就绪。
- `DecisionState` 当前仍为 14 个状态，BR-249 改的是 `RejectedDurable` 的重试授权语义，不改变状态数量；网页状态目录可保留数量，但恢复原则和序列说明需要扩展。
- 网页标题结构暴露出多个必须联动更新的章节：系统上下文、容器/部署拓扑、统一数据平面、gRPC 调用链、monitor 任务树、配置与 feature flags、lazy bridge 路由、构建部署、扩展蓝图、ADR、完整 inventory 与证据矩阵。
- `closing_valuation_run`、`closing_valuation_item` 的 DDL 位于 `src/database/closing_valuation.rs`，均有 append-only update/delete trigger，是可直接补入网页数据所有权清单的源码证据。
- 2026-09-01 Fresh Cargo metadata：61 个 `src/lib.rs` public modules、27 个 binary targets、41 个 integration-test targets。网页的 62/40/44 已漂移；binary 数应以 Cargo targets 而非 `src/bin` Rust 文件数为准。
- `bash scripts/check-no-magic-dependencies.sh all` fresh exit 0，说明当前仓库通过 no-Magic 静态架构门禁；该脚本未发现 CI 接线证据，因此网页只能表述为仓库门禁命令，不能宣称 CI 已执行。
- 网页仍有大量确定过期的生产路径与机制：`grpc_market_server`、`grpc_server::*`、`magic-gateway`、`DATA_GATEWAY_GRPC`、local gateway、同宿主 provider/monitor 双进程，以及旧的 62/40/44 inventory。
- Cargo metadata 的完整当前 target 快照是：1 library、27 binaries、41 integration tests、1 benchmark、1 custom build target。27 个 binary 中仍包含 `grpc_local_readiness_probe`，但不再包含 `grpc_market_server` 或已删除的 provider-only probes。
- `implemented_operations()` 仍返回 40 个 operation，测试也显式断言 40；因此网页 C 附录的数量可保留。不过 `src/grpc_contract/ops.rs` 的注释仍说“delegate 实现”“本地 server 继续提供”，这与 provider-host 已拆出仓库的当前架构冲突，应作为代码注释债务一并记录。
- 当前 `git status` 只有未跟踪的 blueprint、审计 planning 文件和 `.brooks-lint-history.json`，没有 tracked worktree 修改；HEAD 为 `e2503f2`。
- HTML 的 `data-source-sha256` 等于当前 Markdown 的 SHA-256 `fa0a7600…e02c3`，说明网页确实嵌入了同一版 Markdown；两者都还是 2026-08-30 基线。更新时应先改 Markdown，再完整重生成 HTML 并刷新 hash/date，不能只手改网页正文。
- `docs/ARCHITECTURE.md:8-28` 已成为 provider-host 外置、单 gRPC 市场数据路径的当前权威说明；`README.md:28-44` 同时确认没有 provider implementation/server target/local fallback，测试 server 只在 `tests/support/grpc_fixture`。
- `src/data_gateway/grpc_source.rs:1-6,1063-1087` 直接证明 bridge 只面向远程 provider-host、lazy construction、连接失败 fail-closed；旧网页的 runtime switch/local route 图应整体替换，不能局部改名。
- `src/bin/monitor/main.rs:5479-5539` 是当前任务树证据；`8246-8274` 是 paper-sell 默认放行与 `PAPER_SELL_DISABLED=1` 逃生口证据。
- 代码注释本身还有两个迁移遗留：`src/lib.rs:225-227` 把 proto 路径写成已不准确的 `grpc/market.proto`；`src/grpc_contract/ops.rs` 多处保留“本地 server/delegate”措辞。网页应依据真实构建/调用路径，不应复制这些过期注释，并可把它们列为架构文档债务。
- BR-249 NewsAI 影响有直接证据：`NewsAiChainContext` 位于 `src/monitor/news_ai.rs:567-581`，上下文被纳入 prompt/hash；`src/bin/monitor/news_ai_shadow.rs:393-427,503-509` 从 `chain_daily` 与持仓行业链读取，失败时降级为空上下文而不阻塞逐条评估。
- 新增的 `AlertCategory::ChainRisk` 在 `src/monitor/detector.rs:64,82,99,435`，网页业务能力地图/事件推送图应把行业链风险作为 monitor 既有链路的新事件类别，而不是画成独立服务。
- closing valuation 两表及 append-only triggers 的精确证据为 `src/database/closing_valuation.rs:66-74`；`has_persisted_valuation_for_date` 在 145-170 行，是 review backfill 的轻量精确日期判断。
- no-Magic gate 明确检查依赖、feature、上游 Rust path、`src/grpc_server` 和 provider-only targets（`scripts/check-no-magic-dependencies.sh:27-60`），可以进入网页“测试/质量门禁”和“禁止架构捷径”章节。
- Durable/review 精确锚点：`authorize_rejected_retry` 在 `src/durable_delivery/coordinator.rs:3606`；monitor runtime 接线在 `durable_delivery_runtime.rs:1748`，`runtime_producer_ready` 在 1762；`ReviewRunContext::at_business_date/backfill` 在 `review_batch.rs:10-62`；启动与日内 backfill 入口在 `main.rs:5504-5505,5961-5962,6383`。
- 旧蓝图的 P0 漂移集中在 `Project_Architecture_Blueprint.md:32-48,66-258`：系统边界仍把 provider SDK 放入本系统、运行/部署图仍画 in-repo server、本地/远程路由 switch 和 server delegate 调用链。
- P1 漂移集中在任务树、配置/实现模式、构建部署和扩展治理：后台任务写成 7 个；仍描述 `magic-gateway`/`--no-default-features`、lazy feature/runtime routing、provider server 构建启动；新增 gRPC operation 指导仍要求改本仓 server handlers。
- ADR-I01/I02/I06 的迁移方向已经反转：当前应记录“provider-host 是外部系统，本仓 client-only，单一 remote/fail-closed transport”；旧的本地 fallback、Magic import boundary 和 compatibility strangler 结论不能继续标 CURRENT。

## Fresh verification（2026-09-01）

- `cargo test --locked --offline --test unified_data_architecture -- --test-threads=1`：15 passed，0 failed；编译有既存 dead-code warnings。
- `cargo test --locked --offline --lib grpc_contract::ops::tests::implemented_set_is_40_and_within_62 -- --exact`：1 passed，0 failed；确认 implemented set 仍为 40。
- Fresh verifier：61 public modules、27 binaries、41 integration tests、1 benchmark、1 custom build；HTML embedded source hash 与 Markdown hash 一致。
- 旧 Markdown 中仍命中：`grpc_market_server` 11 次、`magic-gateway` 8 次、`DATA_GATEWAY_GRPC` 6 次、`grpc_server::` 3 次。这是网页需要重绘而不是轻量修字的直接证据。
- `git diff --check` exit 0；当前只有未跟踪的 blueprint、planning artifacts 和 `.brooks-lint-history.json`，本轮未改 product code 或 blueprint 产物。
# 全部推送项业务逻辑逐行审计发现（2026-09-01）

- 本轮当前尚未将目录名等同于实际推送项；将从 runtime composition root 和调用点反向确认完整集合。
- 工作树已有用户的 attribution 相关修改和既有文档/规划产物；本轮只读审计，不触碰这些产品改动。
- 推送代码不是单一模块：主要生产面包括 `src/bin/monitor/notify.rs`、`push_templates.rs`、monitor composition root，以及 `push_l1/l2/l4/l5/l6/l7`、`notification/`、`durable_delivery/`、`event/` 和数据库审计表。
- 文件名清单已确认存在 L1、L2、L4、L5、L6、L7，仓库没有 `push_l3` 目录；是否为刻意缺层将以模块注释为证，不从缺失本身推断。
- 粗搜索显示 `notify.rs` 同时包含 `PushKind` 分类、governor、外部通道、去重结算、push log 与大量契约测试，是建立全量集合和共用发送路径的首要证据源。
- 测试中存在“counted push 必须经 durable binding”“测试环境不得真实外发”“旧 generic governor 不得重新承载 counted kind”等静态架构约束，后续需与生产调用逐项交叉验证。
- 核心审计规模：`notify.rs` 7,725 行、`push_templates.rs` 21,968 行、`main.rs` 12,937 行、`durable_delivery_runtime.rs` 5,549 行；分层 push/notification/durable/event 相关文件合计约 76,324 行。
- 对 `PushKind::...` 的全仓名称抽取获得约 65 个业务名称（另有方法名伪匹配）；必须以 enum 定义、dispatch table 和调用点继续做集合差分。
- `notify.rs` 的主要共用链路锚点已定位：`PushKind`(45)、`DispatchRow`(675)、dispatch audit(920)、governor(2213)、普通 delivery(2312)、counted binding(2891)、source-only(2937)、物理 WeChat(3133)、counted finalization(3653)、Feishu/CLI transport(4329/4432)、通道配置解析(4628+)。
- `push_templates.rs` 同时包含“周期 dispatch”“单事件 push”“盘后 review/counting”三类入口；仅公开函数列表不足以证明它们被 composition root 调用。
- `notify.rs:45-197` 当前 enum 实际定义 65 个 `PushKind`；文件头“35 条/30 个”等注释与当前集合已经漂移，不能作为数量证据。
- `PushKind::is_deprecated` 在 `notify.rs:203-209` 无条件返回 `false`，所以 enum 注释中的“降级”不是当前 governor 的默认禁发依据；另有 `is_legacy_v17_5` 仅标记 CandidateTriggered/CandidateInvalidated/VirtualWatch (`:211-225`)。
- PushKind 元数据把 `FactorIC/SectorTier/CapitalVerify` 映射成稳定的 `DailyReportSubKind`，注释明确这三项必须走 BR-192 immutable binding，不能走 generic governor (`notify.rs:228-286`)。
- `level()` (`notify.rs:289-348`) 将 HoldingEvent/MarketActionAlert 定为 Emergency，多数交易/复盘/新闻项为 Important，剩余项默认 Info；该等级是横切元数据，不等于生产可达性。
- composition root 的已定位运行调用覆盖：snapshot stale、account/data mode、盘中日批、盘后 review、news flash、T0/holding/close call、attribution、候选/竞价/涨停板、虚拟盘、行业链、板块、交易流水等；下一步逐处读取窗口与 gate。
- `push_templates.rs` 还含 `build_test_template_catalog` 和 monitor `--test` preview/真实 sink 冒烟逻辑；必须排除测试 catalog，避免把演示模板计入生产推送项。
- L1 `SignalEvent` 是标准事件载体，业务字段为 event_id/source/kind/code/ts/payload/severity (`push_l1/event.rs:16-36`)；payload 数值缺失用 Option 明示 (`:81-205`)。
- L1 去重身份为 SHA-256(source_kind:code-or-global:bucket_ts) 前 8 字节 (`push_l1/event.rs:211-229`)；source fact 使用 provider identity 的独立 domain (`:231-238`)。时间桶是精确 kind match：高频/紧急 1 秒、板块/资金/新闻 10 秒、盘后/日历/静默 5 分钟，未知默认 10 秒 (`:240-278`)。
- 分层目录明确只有 L1事件、L2模板、L4 dispatcher、L5治理、L6 sink、L7 analytics；L3 validation 由 L2 trait/dispatcher 组合承担，需在后续引用实现行证明，不把缺少目录误报为缺功能。
- L7 SQLite 记录 event/template/version/time/severity/data mode/validation/governance/pushed/render length/sink/user/errors (`push_l7/sqlite_store.rs:288-316`)，查询与统计严格解析非法值并返回错误 (`:251-285,319-407`)。
- 分层栈不是仅测试代码：`src/bin/monitor/v14_adapter.rs` 生产适配 L1/L2/L4/L5/L7，`notify.rs:2313+` 把 approved event 送到 L6 或 legacy transport；`main.rs:4702-4713` 初始化 L6 router。但 L6 物理路由只有在 `STOCK_ANALYSIS_PUSH_V6_ENABLE=1` 时启用 (`notify.rs:2339+`)，默认仍走既有通道。
- `src/lib.rs:58-63` 的注释明确 L2 当前只有 metadata/data mode，模板渲染仍由 `push_templates::render_xxx` 完成；因此业务模板审计必须以 monitor 模板文件为主，不能只审 L2 registry。
- L5 治理顺序是静默期、冻结态、数据质量、每日上限 (`push_l5/governance.rs:144-197`)：静默期会拒绝但 QuietHour 自身豁免 (`:159-165`)；冻结态只告警且继续放行 (`:167-175`)；数据质量更差时拒绝，唯有强类型 DataSourceDown + profile opt-in 豁免 (`:177-187`)；达到每日上限拒绝 (`:189-194`)。
- L5 静默时段固定为本地 02:00≤hour<06:00 (`push_l5/governance.rs:206-210`)。
- 跨模块引用表明 L4/L5/L7 生产行为集中通过 `v14_adapter.rs`，通用外发集中于 `notify.rs`；`src/bin/v14_e2e.rs` 只是演示/验收 binary，不能作为 monitor 生产可达性证据。
- `PushKind::cooldown_secs` 的现行规则位于 `notify.rs:388-460`：状态变更/持仓事件无冷却；per-ticket 常见为 1/5/20/30/60 分钟或每日；盘后类多为 1 日；未知项默认 30 分钟。快照虽然注释称按日，但 metadata 实际为 300 秒，日去重依赖调用方 (`:403-404`)。
- `cooldown_scope` (`notify.rs:463-489`) 只把一组明确的证券级事件设为 PerTicket；Announcement 为 External（由 SignalStateMachine 专管），Attribution/G5b 和所有其余项均 Global。
- BR-192 counted 入口 `push_counted_with_binding` 先要求 kind 属于 counted catalog、通过 launch gate、用不可变 occurrence/business-date 生成治理事件，再交 durable runtime (`notify.rs:2886-2933`)；旧 L4 dedup 在这里若返回 Deduped 被视为错误而非成功。
- R-04 龙虎榜的 source-only counted 入口只允许 `ReviewLhb` 且先验证正文与 canonical binding (`notify.rs:2935-2961`)；R-08 event calendar 同样被封成固定 kind 的 public-source-only 入口 (`:2963-2996`)。
- source-only 通用内核按 launch→governance→durable deliver 顺序执行 (`notify.rs:2998-3037`)；source fact 入口从 evidence 派生 kind/code 并校验 presentation token (`:3039-3066`)，防止调用方拿宽松 profile 配错业务种类。
- NewsFlashCritical/Aggregated 使用独立强一致链：token-kind 校验、launch/audit health、reservation 绑定治理、物理通道 preflight、不可变 attempt、30 秒物理发送、L7、不可变 terminal、L4 commit/rollback (`notify.rs:2678-2858`)；它不复用普通 boolean-only sink 语义。
- `presentation_registry.rs:42-399` 是当前 production-owned 卡片形态的封闭目录：固定 58 个 `(family_key, PushKind, producer seam, renderer/assembler seam)` tuple；token 只能由四元组精确匹配取得 (`:405-421`)。这比 PushKind enum 更接近“生产展示项”，但仍不能覆盖直接 governor/无 token 的少数路径。
- 58 个 descriptor 中有同 kind 多形态：HoldingEvent 两个（T-04/T-04B）、T0Advice 两个（建议/禁止）、LimitBoards 三个（一板/二板/三板+）；所以“58 个模板形态”不等于“58 个业务种类”。
- presentation registry 已覆盖账户/数据模式、持仓/T0/候选/虚拟盘/竞价/尾盘、盘后 R 系列、IPO catalyst、盘前盘中新闻/轮动、固定价格/ST/ETF/大宗、v17 source 六类、新闻 critical/aggregated 等；未在 registry 的 enum 变体要单独证明是否走 raw governor、仅作为 durable subkind 或无生产者。
- v14 gate 对 counted kind 强制只接受 counted context，否则 `counted_binding_required` (`v14_adapter.rs:762-785`)；analytics/store/context 任一不可用均 fail closed (`:786-817`)。
- daily cap 从 L7 查询当天“成功推送”的同用户/同 template 数量，查询或数值转换失败即审计并拒绝 (`v14_adapter.rs:819-857,1038-1071`)。
- L5 拒绝和 L4 dedup 都先写 L7；审计写失败也拒绝 (`v14_adapter.rs:858-881,919-944`)。NewsAI 与 counted kinds 不由 legacy L4 拥有去重 (`:884-899`)；普通项才 reserve，成功后由 notify commit，失败 rollback (`:901-1027`)。
- cooldown 计算会对非 source-fact Announcement 或缺 code 的 PerTicket 返回 None (`v14_adapter.rs:959-973`)；这是重要边界：缺证券身份的票级推送不会获得通用 L4 冷却保护。
- `PushKind::stable_template_id` 不是静态表，而是从 Debug/PascalCase 逐字符转 snake_case 再加 `_v1` (`notify.rs:569-591`)；dispatch table 中部分旧快照 ID（如 `factoric_v1`）与此算法的 `factor_i_c_v1` 可能不同，但 dispatch table 注释明确仅审计快照，调用方仍用原方法 (`:650-695`)。
- dispatch table 实际有 20 行而非注释的 15：3 low-priority + 12 v17.7/v17.8 + PaperSell + SnapshotStale + ReviewBacktest + WatchlistTracking + AttributionDaily + G5bAttribution (`notify.rs:695-915`)；注释/测试命名存在历史漂移。
- durable delivery 自己有独立的 23-kind 强类型 catalog (`durable_delivery/model.rs:169-226`)：HoldingPlan/HoldingEvent/T0/Candidate/CloseCall/ForbiddenOps/PaperTrade、7+ 条 review/daily、SectorTop/SectorAnomaly、IndustryChain/PositionReview/Backtest/Watchlist/CatalystReview/PreopenNewsHot。FactorIC/SectorTier/CapitalVerify 是 `DeliverySubKind`，不是独立 durable PushKind (`:300-330`)。
- durable policy 版本当前为 5、全局日预算 30 (`durable_delivery/model.rs:8-24`)；生产库路径固定 `data/durable_delivery.sqlite3`、attempt lease 默认 120 秒，路径/owner identity 不满足约束即拒绝 (`:67-166`)。
- 普通 governor 入口首先拒绝 counted kind，确保唯一准入 owner (`notify.rs:2217-2232`)；后续仍需完整核对 legacy visibility、launch gate、v14 gate 和 delivery 结算行。
- 精确集合差分完成：65 enum kinds；presentation registry 覆盖 54 个唯一 kinds / 58 个形态；未注册 11 个是 FundInflow、FactorIC、SectorTier、CapitalVerify、WeeklySOP、StockPick、NewsRanked、PaperSell、SnapshotStale、IpoListingApproval、IpoProspectus。后续逐个核对直接路径/子类/禁用状态。
- 54 个 presentation kinds 中 23 个落在 durable catalog，31 个走非-counted 或专用 NewsFlash/SourceFact 路径；这说明“有 registry”不代表“durable”。
- `push_templates.rs:1-15` 自述职责是纯模板拼接、不接通道、不写库、不读行情；实际同文件后半也包含 dispatcher/orchestration，属于文件职责已扩张，最终报告需按函数事实而非文件头概括。
- 横幅 `BannerCtx::render` 明确区分完整/不完整账户指标，不完整时只显示“已确认/缺失”并追加账户/估值说明；Degraded/Unsafe 时追加“不含承接判断” (`push_templates.rs:164-233`)。纸面交易风险上下文在数据/账户证据不足时 fail closed (`:236-307`)。
- 前 14.x renderer 的字段顺序由函数显式固定；例如 T-01 输出时间、模式迁移、原因逐条、生效限制、解除条件和“非下单指令” (`push_templates.rs:313-339`)，T-02 输出旧/新数据模式、缺失项、限制、账户状态、ETA/尾注 (`:341-376`)。
- R-08 legacy event-calendar renderer 会在空 holdings 时写“无实盘持仓/虚拟仓” (`push_templates.rs:1653-1667`)；但新的宏观摘要对 audience 不可验证时明确写“持仓关系不可判定”并保留 top facts (`:1690-1757`)。两条语义必须按实际 dispatcher 区分，不能混述。
- 模板通用护栏在缺盘口时逐行查找“承接”，只允许四个自我否定短语，否则返回错误 (`push_templates.rs:1768-1811`)。
- AccountMode 编排先决定 Insert/ReusePending/NoChange；仅 Pushed 后把同一 audit row 标记 pushed，失败保留待重试 (`push_templates.rs:1817-1901`)。
- 11 个无 presentation 项的生产引用已核对：SnapshotStale 在 `main.rs:1934` 直接 governor；PaperSell 在 `main.rs:8860,8951` 直接 governor；FactorIC/SectorTier/CapitalVerify 只在 runtime 映射为 DailyReport subkind (`durable_delivery_runtime.rs:2307-2315`)；IpoListingApproval/IpoProspectus 在启动审计明确 disabled=no_producer (`main.rs:5652+`)；FundInflow/WeeklySOP/StockPick/NewsRanked 在生产代码只有 metadata/map 或测试固定清单，没有发送 caller。
- `StockPick` 字符串仍作为 CandidateSource 输入映射 (`push_templates.rs:3548`)，但这不是 PushKind::StockPick 投递；它被候选台合并后由 CandidateBoard 形态输出，必须避免同名误判。
- durable production/test namespace 强互斥：生产若带 `V10_DRY_RUN_PUSH=1` 直接拒绝；测试反而必须带该开关 (`durable_delivery_runtime.rs:1126-1164`)。
- counted 发送在 admission 前必须完成 startup reconcile；P-01 compensation 是唯一受严格 scope/date/text/canonical binding 检查后可绕过全局 startup barrier 的例外 (`durable_delivery_runtime.rs:1289-1351`)。
- durable envelope 必须由已注册 policy 取得 cooldown scope，且 occurrence/evidence/source binding/subject/rendered content 全部非空 (`durable_delivery/model.rs:611-650`)；presentation-gated envelope 还必须精确匹配 durable kind/subkind (`durable_delivery_runtime.rs:1354-1369`)。
- T-01 AccountMode 的生产函数会校验 persisted prev/evaluation prev 一致，首次评估用 current→current 而不伪造 Normal；新 transition 先落库、渲染、投递，Frozen 新迁移另触发一次 MarketActionAlert，只有 Pushed 才确认原日志 (`push_templates.rs:1901-2067`)。
- 普通 governor 完整顺序已核对：拒 counted → legacy/low-priority/audit log → LaunchGate → 按 source fact/source batch/smoke/ordinary 选择唯一 v14 gate → Approved 才进入 delivery (`notify.rs:2227-2308`)。
- 普通 delivery 在 runtime audit degraded 时 sink 前阻断；L6 env=1 时 route，否则 `push_wechat`；随后写 L7、delivery hash chain 并结算 dedup (`notify.rs:2310-2406`)。物理 sink 已接受后，即使后置审计失败也 commit identity 防重发，但对上层返回 SinkError 暴露审计故障 (`:2386-2419`)。
- LaunchGate 对 Emergency 永远放行，其他按当前发布 stage 判定 (`notify.rs:2440-2450`)；通道名 dry-run 显式记为 `dry_run` (`:2452-2460`)。
- L6 adapter 默认注册 ConsoleSink + MagiclawSink；MagiclawSink 把 boolean 结果转 SinkResult，health check 只检查 dry-run/配置入口，不执行真实 HTTP probe (`l6_sink.rs:48-105,111-145`)。所以 L6 router 的 Console 成功不能单独代表用户已收到，最终 aggregate 语义需以 SinkRouter 实现行核对。
- 快照过期提醒真实触发：启动 + 15:10，最新账户摘要缺失/日期非法/未过期/<5 交易日都不推；达到 5 个交易日后用进程内当日 reservation，Pushed 或 Deduped 才封口，失败释放重试 (`main.rs:1883-1943,8925-8927`)。
- PaperSell 当前默认启用，只有 `PAPER_SELL_DISABLED=1` 暂停并每 30 分钟告警一次 (`main.rs:8246-8274`)；盘中每 30 秒 tick 与盘后 15:30 各自 scan，卖出后按票直接 governor，非 Pushed 会告警 (`:8842-8876,8936-8966`)。
- `br196_test_delivery` 的 fixed-disabled/active manifest 是 monitor `--test` 展示/治理验收目录，不可直接当成生产 scheduler 的可达性结论；例如 CandidateBoard 被 test manifest fixed-disabled (`br196_test_delivery.rs:712-724,868-920`)，但 main 仍有生产 dispatcher caller，必须以 runtime 调用进一步裁决。
- `br196_test_delivery` 实际只被 main 的测试/自检路径用于 build manifest/catalog/governance smoke (`main.rs:6683-6882`)；生产启动只捕获 NewsFlash capability snapshot (`main.rs:4732`)。因此其 fixed-disabled 是测试清单生命周期，不会阻断普通生产 governor。
- CandidateBoard 当前确有主循环调用 (`main.rs:9788`) 且对应 dispatcher 会读取真实候选、组装并尝试发送 (`push_templates.rs:7790+`)；不能因 test manifest 的 fixed-disabled 将它报为无 producer。
- IpoCatalyst 当前由盘后 review dispatcher 调用 (`push_templates.rs:9724`)，其自身读取/复用当日公告批次、关键词分类、静态供应链映射，必要时再查 TDX 板块/成分/证券身份；无公告或 provider failure 静默短路但有日志 (`push_templates.rs:7418-7615+`)。同样与 test manifest 的 fixed-disabled 存在生命周期口径差异。
- NewsFlashCritical 在主新闻循环启动时明确打印 `disabled=no_authoritative_strength_provider` (`main.rs:7641-7654`)，所以 registry/token 和专用发送代码存在不代表 critical producer 已启用；Aggregated 仍需读取后续 gate 逻辑单独判断。
- 当前明确 capability-unavailable 的模板包括：R-02 无完整 review-date market overview batch、R-05 无 delivery settlement outcome、R-06 无证据绑定失败分类源 (`push_templates.rs:9820-9821,12941-12951`)；T-17 无 ETF auction producer (`main.rs:11212-11222`)；旧 close summary/position summary/close review/tomorrow candidates/virtual T+1 均在数据或 counted binding 不完整时显式跳过 (`main.rs:11242-11276`)。
- 普通投递“物理成功、后置审计失败”会返回 SinkError但 commit 去重，属于 deliberately no-resend 的不确定交付语义；报告必须提醒运营不能把 SinkError 一律当作未发送 (`notify.rs:2386-2412`)。
- 模板层还有一层进程内非计数冷却：`dispatch_outcome` 先检查 mode（当前恒不阻断），再查 `(kind, code)` cooldown，最后调用 presentation gateway；只有 Pushed 才写本地 cooldown (`push_templates.rs:14527-14600`)。因此普通项同时受模板本地 cooldown 与 v14/L4 cooldown 两层保护。
- `PeriodicDispatchResult` 把真实空批次、Pushed、Deduped 都视为 confirmed；任何 Denied/SinkError 则变成 Failed (`push_templates.rs:14617-14659`)。这决定 scheduler 是否推进计时器/封口。
- v17 normalized source 六类先做 kind→PushKind 与精确 presentation tuple 映射，再验证 event；公告/政策/业绩/评级走 SourceFactEvidence，MarketActionAlert 走普通 presentation gate (`v17_sources.rs:542-619,714-787`)。批处理逐条验证，无效项 skipped，不 fallback (`:790-809`)。
- EarningsBeat/Miss 生产分类默认禁用，仅 `EARNINGS_BEAT_ENABLED=1` 才打开，因为 report period 与 forecast year/口径未绑定 (`v17_sources.rs:811-825+`)；enum/registry 为 active shape 不等于默认 producer active。
- NewsFlash 当前同时存在旧 `push_flash_decisions` boolean 路径和新 `push_flash_reservations` 强审计路径；主 loop 需确认只使用后者。新路径逐 reservation 取得 exact token、调用专用 transaction、再按 Accepted/Rejected/Uncertain settle (`news_aggregator_init.rs:1057-1125+`)。
- 盘后 review 有 13 个强类型 ReviewTask（测试冻结 `ALL.len()==13`，`review_batch.rs:2847-2868`）；ReviewRunContext 把 business date 与观察时钟分离，手动运行打开 21:00 门，历史 backfill 固定业务日并标 backfill (`review_batch.rs:6-94`)。
# 2026-09-01 全部推送项逐行审计：增量证据（调度与复盘）

- `src/bin/monitor/review_batch.rs:408-447` 定义 13 个盘后复盘任务：R-02、R-03、R-04、R-05、R-06、R-07、R-08、R-09、R-11、R-12、R-13、A-10、A-01。
- `src/bin/monitor/review_batch.rs:449-505` 给出任务标签、依赖和来源类型；R-04/R-07/R-08/R-09/R-11/R-12/R-13/A-10/A-01 属于 SourceOnly，R-03 属于旧账户依赖，R-02/R-05/R-06 保守归为未完成来源。
- `src/bin/monitor/review_batch.rs:1570-1687` 是生产预检：测试模式停用 SourceOnly；A-10 在静默时段延期；R-02/R-05/R-06 明确 Disabled；R-04 正常需 21:00（手工触发可绕过）；R-07 固定等到 21:00；R-09 固定等到 15:35，未来交易日失败。
- `src/bin/monitor/review_batch.rs:829-940` 将结果区分为 delivered/no_data/wait/deferred/disabled/failed；底层 `Deduped` 被当作不可重试失败，quiet-hour 拒绝可重试，其他拒绝不可重试，sink error 可重试。
- `src/bin/monitor/review_batch.rs:1507-1519` 是批处理 due 判定，说明“注册任务”不等于“到时一定发送”，还要经过计划时间和上述预检。
- `src/opportunity/scheduler.rs:13-44` 默认批处理时间为 09:00/15:30，增量扫描每 5 分钟；展示推送窗口固定为 09:00、10:30/11:00/14:30、19:00。
- `src/opportunity/scheduler.rs:86-104` 使用精确小时分钟匹配；非这些分钟返回 Outside。
- `src/bin/monitor/main.rs:1470-1476,1490-1660` 的 CLI `--push` 已不拥有 P-01；盘中窗口调 I-01/I-02/I-03/D-01 和计数型 T-03，晚间调 A-01/A-10；Outside 分支仍会调用 A-01/A-10 兜底，需在最终报告标为“窗口语义与兜底语义并存”。
- `src/bin/monitor/main.rs:5483` 开始的常驻运行路径拥有唯一 P-01 调度器；`src/bin/monitor/main.rs:12330-12379` 的测试用源代码断言约束该唯一所有权。
- `src/bin/monitor/p01.rs:1488-1563` 是 P-01 的到期分类和常驻循环；边界测试位于 `:1984-2024`，明确计划窗口起点包含 09:00、终点不包含 09:15，补偿只允许当日交易日且计划窗口已关闭。
- `src/bin/monitor/notify.rs:3133-3387` 是 monitor 推送的真实物理发送入口；其结果区分“尝试并成功/失败”“未做物理尝试的模拟”“尝试前拒绝”，不能仅按 bool 理解。
- `src/notification/` 的 `NotificationService` 仍被分析流水线、摘要通知和告警模块使用（`src/pipeline/mod.rs:36,354,637`、`src/pipeline/summary_notify.rs:8-65`、`src/monitor/alert.rs:7,93`），但 65 个 monitor `PushKind` 走的是 `src/bin/monitor/notify.rs`，这是两套并存的推送子系统，最终报告必须划清范围。
- `src/bin/monitor/notify.rs:3141-3387` 的物理路径先绑定 runtime namespace；dry-run 只落 push_log 后返回模拟成功；真实发送也必须先持久化 push_log，失败则在物理尝试前拒绝。默认发送类型是飞书（`:4628-4647`）；飞书配置 webhook 时走 HTTP，否则走 magiclaw CLI，微信走 HTTP（`:4649-4662`）。
- 同一物理路径会构建 no-proxy、2 秒连接/30 秒总超时的 HTTP client，确保或拉起 magiclaw daemon，获取/刷新动态 token，解析目标，再标记 sink 已开始；真正发送若鉴权失败会清缓存、重新签发并重试一次（`src/bin/monitor/notify.rs:3187-3387`）。
- P-01 在发送前先检查耐久 claim；已经 Delivered/Rejected/ManualResolved/Uncertain 时直接以权威状态收敛；Reserved 可 resume，但补偿模式禁止恢复 Scheduled 来源的旧 claim；无 claim 才加载、渲染、绑定来源并调用计数型 sink，之后再次 inspect，缺失权威 claim 时即使底层返回 Pushed/Deduped 也判终态失败（`src/bin/monitor/p01.rs:1284-1392`）。
- P-01 计划窗口是 `[09:00,09:15)`，常驻循环每 30 秒 tick 且错过 tick 直接 Skip；日内终态失败后用 `terminal_business_date` 停止当天重试，retryable/awaiting reconciliation 则仍可在窗口内再检查（`src/bin/monitor/p01.rs:1488-1601`）。
- 服务启动先跑 AccountMode、DataMode、SnapshotStale，再以 `tokio::join!` 同时运行 P-01、行情监控、新闻监控、DataMode 监控；盘后新闻、盘后复盘及启动欠账补推另起任务（`src/bin/monitor/main.rs:5464-5525`）。
- L6 基础库 `SinkRouter` 空路由直接失败；逐 sink 串行发送，每个 sink 按 `max_retries` 重试，一个失败不阻止后续 sink，但最终只要任一失败就整体 Err；基础库 `Default` 只注册 `ConsoleSink`（`src/push_l6/sink.rs:117-213`）。生产 monitor 不使用该默认构造，而在 adapter 中显式注册 `ConsoleSink + MagiclawSink`（`src/bin/monitor/l6_sink.rs:129-151`）。
- 仓库存在多个历史推送目录/文档（例如 `docs/v19.x/push-template-catalog.md`、`docs/v19.x/v19.3-push-workflow.md`），其中“✅ 在推”与当前代码的禁用/无 producer 状态已有偏差；最终状态必须以当前调用图、生产预检和 source capability 为准，文档只能作历史背景。
- `push_templates.rs:9546-9724` 是盘后总路由：R-04/R-08/R-09/R-07/R-11/R-12/R-13/A-10/A-01 按任务分支进入各自实现，随后还额外触发大宗交易复核和 IPO 催化；这说明复盘 task catalog 与实际副作用集合不是一一对应。
- `presentation_registry::acquire_token` 的通用宏位于 `push_templates.rs:34-48`；普通模板先用固定四元组（family/kind/producer/renderer）取 token，再进入唯一 generic gateway。固定宏调用覆盖 VirtualWatch、AccountMode、T-14/T-15/T-16/T-17、两类大宗、AuctionVolume、AuctionRepush、IpoCatalyst、CandidateBoard、I-01/I-02/I-03/D-01/A-01、CandidateInvalidated、DataMode（各调用点 `:877,2017,4933,5011,5066,5113,5162,5203,5972,7264,7620,7854,14057,14075,14172,14190,14212,14277,14456`）。
- 计数型模板不能走上述 generic gateway；它们在 P-01、PaperTrade、R-07/R-08/R-09/R-04/R-13、通用复盘 helper、SectorTop/SectorAnomaly 等位置显式构造 `CountedDeliveryBinding` 后调用 `push_counted_with_binding`。CandidateTriggered 的注释明确当前来源缺少耐久生命周期 transition owner，因此固定 fail closed（`push_templates.rs:14235-14250`）。
- 三种涨停板展示不是模板 dispatcher，而是 `monitor_loop` 内取得三个不同展示 token 后直接调用 `push_presented_v3`（`main.rs:10348-10405`）；它们共享 `PushKind::LimitBoards`，所以 kind 级冷却/日限额不是三个独立项。
- `main.rs:5464-5487` 的启动前推送顺序为 AccountMode → DataMode → SnapshotStale，随后主循环；`main.rs:8796` 起的 `monitor_loop` 是绝大多数交易时段类 producer，`news_monitor_loop` 在 `:7571`，盘后复盘 scheduler 在 `:6252`。
- 归一化来源事件必须有 event_id/title/source，除 PolicyHit 外必须有 code，强度/确定性在 0..100，observed_at 不能在未来；除 MarketActionAlert 外必须有“今天”的 published date，stale 一律拒绝；Earnings/Analyst 还必须绑定批次证据且 source/observed_at/metadata 与批次一致（`src/news/aggregator/source_event.rs:308-383`）。
- `v17_sources::push_normalized_event` 对 Announcement/PolicyHit/EarningsBeat/EarningsMiss/AnalystUpgrade 构造 source fact 后走 source-fact gate；MarketActionAlert 走普通 presentation gate；每个事件只调用一次，不做 fallback/retry（`src/bin/monitor/v17_sources.rs:714-787`）。
- EarningsBeat/Miss 当前默认禁用，因为分类未绑定报告期/预测年度/口径；只有 `EARNINGS_BEAT_ENABLED=1` 才开放，禁用时启动告警一次且每 30 分钟节流告警（`v17_sources.rs:811-845`）。
- MarketActionAlert 只由 `MonitorEvent::OrderUpdate` 生成，event identity 是 code/action/shares；内存状态只接受变更，未变更直接跳过（`v17_sources.rs:177-220`）。
- `monitor_loop` 内有两个并行子循环：虚拟盘/归因类 `intraday_loop` 与行情 `market_loop`，最终 `tokio::join!`（`main.rs:8796-8815,11293-11295`）。旧 paper_engine 退出链被明确隔离，不做 provider/order 调用（`:8799-8810`）。
- PaperSell 盘中随决策 tick 扫描，盘后 15:30 再扫；只有得到真实风险上下文才执行，每票走 `PushKind::PaperSell + code`（`main.rs:8816-8876,8928-8967`）。是否真正开放还由 `paper_sell_paused` 控制。
- A-01 午盘虚拟仓快照在 13:00--13:04 首 tick 触发，但无论 dispatcher bool 成败都会立刻记录当天已执行，故同日不重试（`main.rs:9007-9022`）。
- AttributionDaily 在 15:05--15:20 窗口内，成功才记当天完成，失败随 tick 保留重试；输入含持仓报价、日/30日窗口计算、落库和 Markdown 文件，之后才推摘要（`main.rs:9058` 起）。
- I-09 SectorTop 与 I-09A SectorAnomaly 每小时尝试；无论成功失败都重置计时器。I-09A 读取财联社 20 条、取前 10 条标题拼接，失败用空归因文本（`main.rs:10990-11048`）。
- T-14 每 15 分钟、T-15 每 5 分钟扫描 trade pipeline；两者要求 banner，只有 dispatcher confirmed 才更新时间戳，因此失败会在主循环下一 tick 重试（`main.rs:11052-11094`）。
- T-16 在 09:30 后批量检查 ST 持仓，成功即封口当天；T-17 到 14:57 直接记录 `disabled=no_etf_auction_producer` 并封口，不取数不发送（`main.rs:11098-11123,11212-11222`）。
- T-12 在 14:55 后生成逐票 counted binding，Pushed/Deduped 都算确认；只有所有票确认才封口，失败保留重试（`main.rs:11138-11210`）。
- 收盘后的旧 DailyReport、持仓汇总、账户收盘复盘、次日候选和虚拟 T+1 汇总均因缺少不可变 counted/source 合同明确停用（`main.rs:11242-11276`）。
- G5b 在 15:05--15:20 独立运行：无告警则当天封口；有告警但无 LLM provider 不封口、窗口内持续提醒；最多分析 `DEEP_ATTRIBUTION_MAX_EVENTS` 条，每条成功先写深链归因行再直接治理推送，批次内无论单条成功/失败，全部尝试后当天封口以避免重复计费（`main.rs:9143-9238`）。
- 持仓快照新鲜度提醒在 15:05 读取用户确认快照：非空仓快照超过 6 小时、从未导入或读取错误才推，借用 `PushKind::IntradayMarket`，无论推送结果当天都不重试（`main.rs:9241-9294`）。这是“一个 kind 多业务语义”的明确实例。
- 行情 detector 的 MainInflow/MainOutflow/VolBurst/BoardBreak 等内存告警全部因没有耐久 lifecycle owner 而 fail-closed；LimitUp/LimitDown 也只进入状态机后调用 `reject_unbound_alert_delivery`，不会外发；炸板仅 log capability unavailable（`main.rs:10555-10712`）。
- AccountMode 除启动时评估外还有 08:30 当日一次重置；08:30 后启动会补做，只有 hook 成功才封口，失败每 30 秒重试（`main.rs:9370-9397,9430`）。
- P-03 CandidateTriggered 位于 `market_loop` 已进入交易时段之后，却要求 `session == Closed` 且时间 `[09:00,09:15)`；注释直接承认原 P-01 放在这里结构不可达。再叠加 CandidateTriggered dispatcher 固定缺 counted binding，当前应判“结构/能力双重不可达”（`main.rs:9590-9616` 与 `push_templates.rs:14235-14250`）。
- 09:10 行情预检的失败告警复用 `IntradayMarket` kind；探测一执行就记录当日完成，外发失败也不重试，只有 09:20 竞价逻辑另行兜底（`main.rs:9618-9688`）。
- VirtualWatch 仅 Confirm 模式、早盘、候选非空且价格仍全为 0 时初始化；报价依次从持仓、涨停池、统一实时行情补齐，正价项先持久化 snapshot，再调用 P-05 dispatcher；调用结果被丢弃且日志无条件写“已推送”（`main.rs:10103-10238`），存在可观测性假成功。
- LimitBoards 只处理有主力净流的涨停股，批量最多查询 40 个板级、排序最多取 50 个；一旦 `board_notified.insert` 就先在内存标记，再渲染/取 token/发送，因此后续发送失败同日也不会再尝试该股票（`main.rs:10241-10416`）。
- P-02 AuctionVolume 在 09:20 后每个竞价 tick 取当日涨停池、按量比降序选未通知前 10；只有 dispatcher 返回 true 才把这些 code 加入 `auction_vol_notified`，banner/发送失败可在竞价窗口重试（`main.rs:9692-9773`）。
- A-02 AuctionRepush 与 P-05 CandidateBoard 在同一竞价 tick 成对执行，二者都成功才封口；任一失败或空候选都保留到下一 tick，另由各自 600/1800 秒 cooldown 防重复（`main.rs:9775-9801`）。
- 该块随后把 `post_close` 写死为 `String::new()`，却试图从其文本解析候选来填 `virtual_observation`（`main.rs:9803-9851`）；全文件对 `virtual_observation.push` 的生产写入需要再做唯一性核验。若无其他写入，Pilot/Confirm 的 VirtualWatch 实际不可达。
- T-03 HoldingPlan 使用用户确认快照 + 持仓行情；空仓静默，单票缺行情/成本非法跳过；盈亏 >+5% Reduce、<-3% Add、否则 Hold，并给出成本派生的减仓区/支撑/压力/止损；binding 是 `holding-plan:{date}:{code}` 的票级 InternalDurable（`main.rs:8520-8620`）。另有 `holding_plan_daily` 表做跨重启一日一票（`:8460-8511`）。
- T-12 CloseCall 同源，但只对相对成本跌幅 <= -3% 的持仓生成“尾盘跳水”提示，binding 是 `close-call:{date}:{code}`（`main.rs:8624-8707`）。
- 全文件只有 `main.rs:9841` 一处给 `virtual_observation` push，而它读取的 `post_close` 在 `:9803` 固定为空。因此当前 Pilot/Confirm 两条 VirtualWatch 常驻生产路径都没有候选输入，renderer/dispatcher 可用但实际零生产。
- PaperSell 从 2026-09-01 起默认开放；只有 `PAPER_SELL_DISABLED=1` 才暂停，禁用 banner 首次打印、跳过告警每 30 分钟节流（`main.rs:8246-8274`）。
- P-04 PaperTrade 在竞价非 Auction 分支（注释为 09:15--09:20）每 30 秒调用 `dispatch_paper_trade_daily`；只消费当日已持久化的完成态，没有可投递记录时返回 false 并记 info（`main.rs:10043-10057`）。真实逐行校验与 durable identity 在模板 dispatcher 内。
- T0Advice 每 30 秒到期，`prepare_t0_messages` 生成逐票 counted message；全批 Pushed/Deduped（空批也算）才推进计时器，任一 Denied/SinkError 或取数失败保留立即重试（`main.rs:10735-10804`）。
- 盘中 `IntradayMarket` 主视图每 5 分钟读取 Concept 板块 1 日资金流 Top10；真实空结果推进计时器，数据/任务失败不推进，投递只有 confirmed 才推进（`main.rs:10810-10855`）。
- I-03 每 15 分钟调用产业链 dispatcher，必须有 banner，只有 confirmed 才推进；T-03 每 30 分钟生成持仓建议，并以 `holding_plan_daily` 过滤当日已推，Pushed/Deduped 才写一日一票表，全部确认才推进 timer（`main.rs:10857-10986`）。
- T-01 文案逐字段输出时间、旧→新模式、全部原因、限制、解除条件和“非下单指令”（`push_templates.rs:313-339`）；T-02 输出旧→新数据模式、缺失项、逐条输出限制、账户状态和可选 ETA（`:341-376`）。
- T-01 orchestration 先严格核对持久行与 prev/evaluation，再决定 NoChange/Insert/ReusePending；初次评估用 current→current，不伪造 Normal；先插审计行再发，只有 Pushed 才把同一行标 pushed=1，失败保留 pending 下轮复用（`push_templates.rs:1817-2025,2060-2069`）。Frozen 的非初始新迁移还会额外发一条 MarketActionAlert（`:2027-2057`）。
- T-02 首次建立状态静默，不发；真正 transition 才根据 Full/Degraded/Unsafe 生成限制并发。Degraded 禁盘口承接判断并标数据降级，Unsafe 还禁价格建议、只保留风险类；只有 Pushed 或静默建立算 confirmed（`push_templates.rs:14361-14477`）。
- T-03 renderer 输出 banner、票名代码时间、动作、现价/成本/可用股、可选减仓区、支撑/压力/止损、无效条件和理由（`push_templates.rs:405-461`）；T-04 renderer 输出触发、价格/涨幅/距止损、建议和可用股（`:463-492`）。
- T-05 renderer 明确展示 Magic TDX 批次/源时间、状态/趋势、均价/ATR、量能/五档比、卖出与接回观察区、价差、数量、触发/失效，并警告仍需 30 秒内券商可用持仓与 T+1 校验（`push_templates.rs:518-567`）。
- T-07 renderer 虽存在完整的候选等级、主题、价格区、仓位上限、新闻/量能/K线/盘口证据和不买条件（`push_templates.rs:601-700`），但 producer/binding 前述已 fail closed；T-08 仅输出原状态→Invalidated及原因，T-09 输出 banner/结论/多条禁因（`:703-743`）。
- I-01 从实时板块涨幅排行取 30 个并评分；主攻从全体最高涨幅选，另用确定性关键词分别找 tech/power/robot；正涨板块占比 >=2/3 为 Spreading、>=1/3 为 Diverging、否则 Fading（`push_templates.rs:2520-2599`）。只要主攻或任一家族非空就推，真实空视为 confirmed empty（`:2612-2664`）。
- I-02 优先读最新 board_rotation，解析最多 9 个 stocks，强制有效 6 位 code/name/有限涨幅；绝对涨幅 >20% 保留真实值但警告。无 rotation 再回退 chain_daily；构造时 LLM tickers 优先，reason 优先 LLM 原因，否则按主题映射固定“板块共振”短语（`push_templates.rs:2670-2755,2874` 起）。
- P-04 每个 paper_trades 终态必须满足：正 id/plan、环境隔离的 A 股身份、buy/sell、正价格、正且 100 股整数倍数量、三种终态；Filled 必须有正 fill_price，另外两种必须有原因；还必须有 virtual_reason、合法 account/data mode、唯一 order_audit 与 hash chain、quote/terminal 时间（`push_templates.rs:5319-5511`）。
- P-04 SQL 用 plan/source/reason/side/code/price/qty/outcome/fill/failure 做精确联接，只读本地当天 Filled/NotFilled/Invalidated；任何歧义重复 audit row 都整批拒绝。每行以 terminal transition 构造票级 counted binding；最终所有行 Pushed/Deduped 才返回成功（`push_templates.rs:5513-5693`）。
- I-02 dispatcher 的 LLM 是可选增强：prompt 要求 1--9 只 A 股，接受对象或数组返回；二次过滤 6 位数字 code、importance clamp 1..10 且低于 4 丢弃，同 code 取重要度最高；LLM 失败/空则回退原 theme 路径（`push_templates.rs:2981-3050` 起）。
- I-03 来源为空时 confirmed empty；有补涨候选且配置 LLM 时让模型按主链/龙头/候选生成逐票 trigger，空/失败回退原 trigger。发送成功后把补涨候选和真实价格写 `pushed_stocks`；该审计写失败会把整个周期结果改为 Failed（`push_templates.rs:3337-3468`）。
- T-14/T-15 共用注册的 `TradeEventSource`，但各自重新抓 pending events；所有事件必须有效 A 股 code/name、正价、正且 100 股整数倍数量、type 为 order/fill。T-14 还要求非空 order_id + status，T-15 要求 next_session_carry；任一非法项整批失败，空目标类型是 confirmed empty（`push_templates.rs:4700-4878`）。
- T-16 dispatcher 本身展示 ST 类型、5%→10% 等规则参数、持仓/成本/现价和新止盈止损；大宗 BR-033 只接受创业/科创协议大宗、实时确认、合法百股数量/正价，BR-034 必须有正当日均价和非空价格区间（`push_templates.rs:5029-5088,5127-5215`）。
- D-01 从统一候选批取第一名；source_count >=3/2/else 对应 Starting/Fermenting/Diverging，涨幅 >5% DoNotChase、>0 BuyDip、否则 Observe；主题优先候选映射否则来源标签（`push_templates.rs:3888-3935`）。候选批来源包括四个 `data/p5_sources/*.jsonl` 文件类型、持仓与 chain 数据，并严格绑定完整报价/市场统计批次（`:3517-3655` 与候选装配段）。
- D-01 可用 LLM 生成最多 3 条非空具体原因，失败回退候选 evidence；另有进程内一小时/票 memo，只有 push 成功后才写（`push_templates.rs:4075-4159` 起）。当 action=BuyDip 时还会取 broker execution quote、固定 100 股、用当前纸面账户状态调用 `paper_trade::simulate`，并写 pushed_stocks（`:3952-4053`），需确认该副作用相对通知成功的顺序。
- A-02 过滤掉价格/热度缺失候选，Strong 档优先、再按热度降序取 Top5；文案展示首来源、现价和热度（`push_templates.rs:7184-7275`）。
- R-09 并非全市场排名：代码在文案明确声明“Eastmoney 单响应 TopN”；两组各 1..20 行必须严格匹配 metric/unit/source ordinal/date/filter/A股身份，并保留 provider declared total/inspected count。它把两批来源和渲染内容绑定后直接交耐久 envelope 投递（`push_templates.rs:6226-6323,6680-6777`）。
- News monitor 默认 120 秒轮询，只在 `NewsMonitor::should_run()` 的 08:00--22:00 窗口执行；每 tick 用协调器保证 critical/earnings/L2/announcement/reset/flush/banner/sleep 每阶段恰好一次，公告阶段只有自选加载 Ready 时进入（`main.rs:7293-7454,7571-7583,7671-7677`）。
- 公告受众 = 注册自选 + 24 小时内用户确认持仓；无快照/读取失败/过期/确认空仓都只排除持仓，仍保留自选并显式告警（`main.rs:7464-7528`）。
- 业绩/评级 provider 仅 15:00 后调用；EarningsBeat/Miss 还受默认关闭 gate，AnalystUpgrade 则按 config poll 间隔/状态店追踪（`main.rs:7914-7953` 与 `v17_sources.rs:811-845`）。
- 公告只走 EventCalendarGateway，当日最多请求 300 条并在完整批次后套关键词；代码缺失时用概念索引或统一名称反查。归一化 route 对每个原始输入给出 pushed/分类过滤/lifecycle过滤/受众过滤/重复/失败，只有 disposition=Pushed 才允许后续 legacy AlertEvent 进入 `pushed`，其他全部抑制（`main.rs:8000-8124`）。
- D-01 与 I-02 的事件触发条件不是“有任意新闻”，而是上述 `pushed` 中至少一条 `AlertLevel::Important`；两者共享本轮触发，分别推个股候选和板块催化（`main.rs:8126-8192`）。
- NewsFlash 每 tick 先从不可变 event authority 预检，才抓每 feed 20 条原始全球新闻并投影；所有投影失败必须先写不可变失败审计，审计不可用则 reservation/sink fail-closed；reservation 前重新 reconcile 新鲜 authority，之后只走 `push_flash_reservations`（`main.rs:7685-7817`）。
- N-01/N-02 的 public SourceOnly 路径与 `selection_v2_enabled` 无关；selection_v2 只控制同一 raw batch 后续的 NewsAI/候选 ingress，且仅交易/竞价时段执行（`main.rs:7819-7870`）。因此 N-02 是否在推不能按 selection-v2 开关判断。
- 聚合窗口固定 09:30/11:30/13:00/15:00，首个 tick 有 5 分钟容差；N-01 固定 banner 声明无权威 strength provider（`news_aggregator_init.rs:136-149`）。新 owner 对每个 reservation 取 exact token、执行专用 physical/audit transaction，再按 Accepted/DefinitiveRejected/Uncertain 或 pre-sink rejection 精确 settle；只有 Accepted 计数（`:1057-1145`）。
- N-01 不只是“启动 banner 显示禁用”，在当前 SourceOnly 实现中还结构性不可生成：`NewsFlashAggregator::reserve()` 把阈值参数命名为 `_critical_threshold` 且完全不用；所有合格输入只进入缓冲区，只有到上述四个固定窗口才构造 `NewsFlashAggregated` 预留（`src/news_aggregator_init.rs:478-609`）。回归测试在 SourceOnly ingest 后只要出现 N-01 reservation 就直接 panic（`:1475-1476`）。
- N-02 聚合会按 strength 排序、取前三条；同一窗口有未解决 reservation 时返回该 reservation 等待恢复而不另建第二个（`src/news_aggregator_init.rs:520-605`）。因此其幂等边界是窗口 reservation，不是单条新闻。
- 公告归一化路由在生命周期、分类和受众校验后尝试 durable dedup claim；投递未成功会释放 claim；durable dedup 存储不可用时会显式回退 L4 治理而非整体 fail-closed（`src/bin/monitor/v17_sources.rs:324-500`）。
- T-14/T-15 的 production dispatcher 依赖全局 `TRADE_EVENT_SOURCE`（`push_templates.rs:4688-4705`），但全仓 `register_trade_event_source` 只有定义、没有生产调用；因此 monitor 的周期入口虽可达，当前每次都会在取数边界返回 `TradeEventSource is not registered`，不能判为真实在推。
- PolicyHit 的 `classify_policy` 要求非 research-only、完整 governed evidence、非空 title/source、provider publication date，并构造全局无 code 的 source event（`src/news/aggregator/classifier.rs:315-359`）；但生产代码没有调用该分类器，`SourcePushKind::PolicyHit` 的直接构造只存在测试，因此当前是“能力存在、无 producer”。
- D-01 的纸面买入副作用顺序已确认：先 `push_news_to_idea`，只有推送返回 true 且 action=BuyDip 才执行 100 股虚拟买入；虚拟买入失败会让 dispatcher 返回 false 且不写 1 小时 memo（`push_templates.rs:4140-4187`）。
- 盘后 dispatcher 先并发完成 registered ReviewTask，再在非测试环境额外执行预测样本回填、大宗交易侧推和每日 IPO 催化；账户依赖任务当前统一产生 `AccountMetricsIncomplete`，不调用 provider/renderer/sink（`push_templates.rs:9546-9818`）。
- ReviewTask 有 13 个；其中 R-04/R-07/R-08/R-09/R-11/R-12/R-13/A-10/A-01 为 SourceOnly，R-03 为 LegacyAccountGate，R-02/R-05/R-06 为保守未分类依赖（`src/bin/monitor/review_batch.rs:408-505`）。自动 scheduler 仅交易日 19:00 后运行（`main.rs:5786-5790,6252-6460`）。
- Review preflight 直接禁用 R-02/R-05/R-06；R-04/R-07 等到 21:00（手工仅绕过 R-04），R-09 当天要等 15:35；A-10 静默时段延期（`review_batch.rs:1570-1687`）。可重试失败按 1/5/15 分钟退避，Delivered/NoData/Disabled/不可重试失败进入终态（`:1190-1240`）。
- R-12 又有一层常量 `R12_TECHNICAL_BARS_PUBLISHED=false`，在 loader/provider 前返回 Disabled（`push_templates.rs:9020-9110`），所以虽然它在 task catalog 和 19:00 路由内，生产不可投递。
- R-07 聚合四类次日观察：A 档未触发、龙虎榜净买入 Top5、涨停链前三龙头、整百股可做 T 持仓；全部以当日正收盘价派生 ±2% 区间和 -5% 止损，去重后仍必须有完整 LHB provider batch 才能构造 counted binding（`push_templates.rs:8047-8475`）。
- R-11 要求用户确认账户摘要和 review_date 精确收盘估值；空持仓仍可推“无持仓”，持仓行业按市值 Top5+其他汇总，可选 deep-analyzer 文本不阻塞；最后走 counted 任务投递（`push_templates.rs:8797-9012`）。
- R-08 并发取 CNInfo 公告、CFFEX 交割、海外指数、USD/CNY；允许部分组件降级但随后把可用批次完整绑定并走 R08 source-only counted gateway（`push_templates.rs:10983-11210`）。R-04 则 21:00 后取 Eastmoney 龙虎榜 top-five 完整批次，verified-empty 终态 no_data，其余走 source-only counted（`:12732-12932`）。
- A-10 读取可见产业链批，按成员数/连板数/持续性派生 0..100 分与明日观察点，走 counted；成功后保存 T+1 watchlist，保存失败只 warn、不反转已投递状态（`push_templates.rs:13551-13755`）。R-13 次日读取该快照并核对行情，成功推后落 outcomes，落库失败同样不反转（`:9447-9538`）。
- CandidateInvalidated 并非无 producer：CandidateBoard 每轮先读上一候选 code 快照，上一轮有而本轮无的 code 逐个发 T-08；随后采 Strong 预测样本、把本轮快照写盘，最后才发 CandidateBoard（`push_templates.rs:7752-7865`）。副作用顺序意味着 CandidateBoard 发送失败时 diff 快照已经推进；失效名称也因股票已不在本轮 entries 而回退为 code。
- CandidateTriggered 有三重阻断：主循环所在交易时段与它要求的 Closed 09:00--09:15 结构冲突；dispatcher 先要求样本/人工开关；即便首次通过，`push_candidate_triggered` 被传入 `promotion_evidence=None` 会再次因证据缺失返回 Shadow，而且其 counted binding capability 明确不可用（`main.rs:9590-9616`, `push_templates.rs:5714-5835,14235-14268`, `src/opportunity/candidate_state.rs:31-61`）。
- HoldingEvent 只有 renderer；生产主循环注释明确 legacy summary 在渲染前禁用（`main.rs:9520-9523`），内存告警统一 `reject_unbound_alert_delivery`，不能构造 durable counted evidence（`:11514-11530`）。
- 大宗交易两 kind 实际由 19:00 盘后 side route 触发，并非名称所称的盘中 owner：按自选+持仓 code 查 BlockTradesGateway，300/301/688 走协议大宗确认，8/4/920 开头走北交所价格区间（`push_templates.rs:7684-7735,9700-9721`）。
- AttributionDaily 的实现与注释不一致：注释称成功才记 `ATTRIBUTION_LAST_RUN`，但代码在完成计算/落库/写 Markdown 后，不论 `push_governor_v3` 返回 Pushed/Deduped/Denied/SinkError 都立刻把当天标完成，故 sink 失败不会在 15:05--15:20 重试（`main.rs:9060-9122`）。
- G5b 每条先得到 LLM 结果并追加深链归因行，再调用普通 governor；push outcome 只记录日志，`done += 1` 仍执行，整批全部尝试后当天封口，故“成功数”其实是分析/持久化成功数而非确认投递数（`main.rs:9143-9238`）。
- SnapshotStale 只有快照落后最近交易日至少 5 个交易日才推；使用 begin/finish in-flight gate，只有 Pushed/Deduped 才提交当日完成，失败保留重试（`main.rs:1900-1965`）。另一个 15:05 的 6 小时快照预警复用 IntradayMarket kind 且无论投递结果都当天封口（`:9241-9294`），两者不是同一推送项。
- P-01 只在已验证 A 股交易日 `[09:00,09:15)` 运行，证据日严格为上一交易日；每 30 秒 tick，补偿只允许同一业务日且 09:15 后（`p01.rs:101-150,1488-1601`）。它请求上一日涨停池 200 条、从 chain projection 取前三条主线首票、精确绑定证券身份及逐票新浪新闻，所有票新闻总数为 0 即终态拒绝（`:19-20,484-758,770-1035`）；文案每主线只展示最新一条新闻，但其余记录 hash 仍保留在 canonical binding（`push_templates.rs:14744-14811`）。
- 仓库还有独立 `src/notification` 子系统，配置 10 类渠道：企业微信、飞书、Telegram、邮件、Pushover、Custom、Server酱、钉钉、Slack、Discord（`src/notification/config.rs:3-38,51-132`）。它由分析 pipeline 的单股报告和汇总报告调用（`src/pipeline/analyze.rs:1246-1260`, `src/pipeline/mod.rs:637-732`），不经过 monitor 的 PushKind、presentation token、L4/L5/L7 或 durable delivery。
- `NotificationService::send` 顺序遍历所有已配置渠道；任一渠道成功即整体 `Ok(true)`，即使其余渠道失败；没有渠道则 `Ok(false)`（`src/notification/service.rs:124-272`）。带图只有邮件真正发图，其他渠道降级文本（`:275-374`）。
- 分析 pipeline 存在确认语义 bug：单股与汇总调用都对 `Ok(_)` 记录“推送成功”，因此 `send()` 的 `Ok(false)`（无渠道或全部失败）也会被日志冒充成功（`src/pipeline/analyze.rs:1246-1257`, `src/pipeline/summary_notify.rs:107-114`）。
- `monitor::alert::send_alert` 注释称 Emergency 全渠道、Important 微信+飞书、Info 飞书/邮件，但三个 match 分支实际都调用同一个 `self.send(text)`，没有任何级别路由差异（`src/monitor/alert.rs:162-180`）；而 `push_alert` 在生产 `src` 中没有 caller，当前只是可用 helper。
- BR-196 test manifest 本身已漂移：`ALL_PUSH_KINDS` 固定为 63，漏掉后来新增的 PaperSell 与 SnapshotStale；`FIXED_DISABLED_KINDS` 又把生产真实调用的 CandidateBoard 和 IpoCatalyst 列为 disabled（`src/bin/monitor/br196_test_delivery.rs:706-788` 对照 enum `notify.rs:45-197`）。因此该 manifest 只能用于测试预览，不能证明 runtime 活跃状态。
- Production presentation registry 是 58 个精确四元组、覆盖 54 个唯一 kind；token 只在 family/kind/producer/renderer 四字段完全匹配时签发（`presentation_registry.rs:42-421`）。它证明“允许的展示形状”，不证明 producer 可达。
- 普通 governor 先拒绝任何 counted kind，再做 launch gate、source evidence 互斥、v14 gate；sink 前 delivery audit 已 degraded 会 fail-closed。默认走 `push_wechat`，L6 仅 `STOCK_ANALYSIS_PUSH_V6_ENABLE=1` opt-in；物理成功后即使 L7/hash-chain 写失败也提交去重并返回 SinkError，避免重复发送（`notify.rs:2227-2438`）。
- LaunchGate 源码头注释说 Shadow 不推，但实际 `should_push_user` 明确 Shadow/Live 全推、Gray 只让 critical；monitor 的 `launch_gate_check` 又让 Emergency 无条件绕过（`src/opportunity/launch_gate.rs:110-140`, `notify.rs:2440-2450`）。以可执行代码为准。
- monitor 物理发送默认是飞书；配置 webhook 则 HTTP，否则 magiclaw CLI，只有显式 `MAGICLAW_SEND_TYPE/SEND_TYPE=wechat` 才微信（`notify.rs:4628-4662`）。namespace 先于 dry-run 分支绑定：Prod 配 `V10_DRY_RUN_PUSH=1` 会直接拒绝，Test 则反过来强制必须开启；合法 Test dry-run 写 push_log 后返回模拟成功，上层会按已投递推进冷却/状态（`durable_delivery_runtime.rs:1126-1164`, `notify.rs:3141-3180,4325-4327`）。
- ReviewTask 的 `Deduped` 与普通周期任务语义不同：普通 periodic 把 Pushed/Deduped 都视为 confirmed，盘后 `ReviewTaskOutcome::from_push_outcome` 却把 Deduped 变为不可重试 Failed，随后 scheduler 将其终态化（`push_templates.rs:14617-14659`, `review_batch.rs:829-940,1190-1220`）。
- 当前 checked-in `config/strategy.toml` 把 `news_window_start_hour/end_hour` 设为 0/24，不是代码默认 8/22；`NewsMonitor::should_run_at` 对该值判全天运行（`config/strategy.toml:23-25`, `src/monitor/news_monitor.rs:113-132`）。news monitor 仍默认每 120 秒 tick（`main.rs:7293-7310`）。
- monitor 在 `main` 中确实调用 `dotenvy::dotenv()`（`main.rs:4473-4476`）。安全开关核验显示当前 `.env` 仅显式 `ENABLE_CANDIDATE_LIVE=true`；未覆盖 STAGE/L6/dry-run/PaperSell/Earnings/send-type。故当前配置按代码 fallback：Shadow（实际全推）、L6 off、Prod dry-run off、PaperSell on、Earnings off、默认飞书；T-07 即使人工开关开，仍因第二道调用传 `promotion_evidence=None` 被拒。
- 不读取任何凭据值的配置存在性核验显示：monitor 默认飞书 CLI 所需的 `FEISHU_TO` 已配置；独立 NotificationService 只有邮件五项（sender/password/receivers/server/port）完整配置，未检测到其余九类渠道变量。这个结论仅证明配置路径可选中，不等于已做真实网络投递验收。
- NewsFlash 专用事务比普通消息更严格：L6、dry-run/boolean-only、HTTP transport 都因不能返回 typed remote receipt 在物理尝试前拒绝；只接受 CLI（`notify.rs:4196-4241,2678-2858`）。当前安全配置存在性恰好选择“默认飞书 + 无 webhook + FEISHU_TO”，因此 N-02 的 transport preflight 在配置层可通过；是否真实送达仍取决于 magiclaw CLI/远端回执，审计没有执行真实发送来验证。
- 最终集合核验：`PushKind` 为 65 项，production presentation tuple 为 58 个（覆盖 54 个唯一 kind），durable catalog 为 23 项；按生产调用图归类为 37 项有当前 producer、2 项路径可达但新输入枯竭、2 项默认关闭可 opt-in、24 项禁用/阻断/无 producer，合计 65。
- 验证结果不能写成全绿：`cargo test --bin monitor` 默认并行执行为 703 passed / 6 failed / 4 ignored；6 个失败都在 durable runtime 隔离/终态回放测试。随后精确复跑 BR-192 失败项为 1/1 通过，BR-194 terminal replay 组单线程为 14/14 通过。这支持“并行共享状态干扰”的推断，但没有证明根因或修复。
- 独立通知模块 `cargo test --lib notification:: -- --test-threads=1` 为 18/18 通过；测试通过不消除 pipeline 对 `Ok(false)` 误记成功的静态语义问题。
- `MarketSession` 的真实阶段边界是 Closed(<09:15)、Auction(09:15--09:25)、Closed gap(09:25--09:30)、Morning(09:30--11:30)、LunchBreak(11:30--13:00)、Afternoon(13:00--15:00)、AfterHours(>=15:00)（`src/calendar.rs:508-559`）。时段归类必须以此为准，而不是变量或注释名称。
- P-03 与 09:10 行情预检共同位于 `market_loop` 已通过 `while !is_market_active()` 之后的 `session == Closed` 分支；09:10 时 loop 尚在外层等待，09:15 后 session 已是 Auction，因此两者在当前结构都不可达（`main.rs:9442-9457,9590-9688`）。这修正了此前只确认 P-03 结构不可达、却未同步标注预检不可达的遗漏。
- 产业链报告还有两条不经过 PushKind/governor 的真实定时推送：09:05--09:14 盘前和 15:30--15:34 盘后都调用 `run_chain_analysis_mode(true)`；该函数先保存报告，再直接调用 `NotificationService::send`，对 `Ok(false)` 正确记录 warning（`main.rs:9398-9427,8974-9003`, `src/app/modes.rs:106-180`）。
- T-15 注释声称撮合窗口是 15:05--15:30，但其周期调用实际嵌在只处理 Morning/AfterNoon 的盘中分支，15:00 后会退出；再叠加 `TRADE_EVENT_SOURCE` 未注册，当前既没有正确盘后调度，也没有数据源（`main.rs:10060-10080,11052-11094,11227-11229`）。
- 产业链定时报告的通知失败不会向调度器传播：`run_chain_analysis_mode` 对 `Ok(false)` 和 `Err` 都只记 warning，随后统一返回 `Ok(())`；09:05/15:30 外层据此写当日完成。因此注释所谓“失败保留重试”只覆盖取数/分析/保存失败，不覆盖所有渠道失败或通知异常（`src/app/modes.rs:164-182`, `main.rs:9415-9425,8988-9000`）。
- 蓝图 HTML 在仓库中没有可发现的生成脚本或 Markdown renderer 命令；现有 HTML 自包含 article、动态目录 JS、Mermaid base64 源、完整 Markdown base64 源和 source SHA。同步策略必须保留页面壳，机械插入新增章节，并更新章节数、生成日期、嵌入源与 SHA，不能只改可见 article。
- 新章节放在“新开发治理模板”之后、“蓝图维护触发器”之前，编号为 24；原维护触发器顺延为 25。这样专项方案位于当前事实/扩展/治理之后、维护规则之前，且不把规划内容冒充 CURRENT 实现。
- 蓝图第 24 节最终补齐 65-kind 逐项业务表：盘前 5、集合竞价 7、盘中 22、盘后 31；状态精确为 ACTIVE 37、STARVED 2、OPT-IN 2、INACTIVE 24，集合无 missing/extra/duplicate。
- 每个 65-kind 行都写入触发/source/核心筛选/完成或重试语义，并至少附一项当前 `.rs:line`；自动检查了 133 组完整文件引用，文件均存在且最大行号未越界。
- HTML 第 24 节由 Markdown 子集受限机械转换，产生 19 个 h3、14 张表与 132 个表格行；Markdown/HTML 的 65 个 `(PushKind, status)` 元组逐行相同。
- 最终蓝图 Markdown SHA-256 为 `8a1150524092089866095185259d6a615c7e0f8b1fff742292fc75e27e33df37`；HTML 的 body metadata、sidebar、source note 和内嵌完整 Markdown 四处已同步。
- 第 24.18 节原先只有六阶段方向表，没有回答“由 Codex 单独开发时的改动方案和周期”；现已补为 C0--C6、W01--W21、M1--M4 的正式执行路线图。
- C0--C6 区间机械相加为 10--16 个有效开发日；计划基线取 10--15 日，只有当多个任务同时触及上界时才进入 16--20 日悲观区间，不能通过削减 C6 回归来压缩日期。
- 计划只迁移活跃/高风险路径，不把 24 个 INACTIVE enum 项顺带启用；真实渠道验收、外部 source owner 补建和生产观察不混入自动化有效开发日。
- 蓝图的正式归档路径应服从仓库 `docs/` 总索引，而不是技能默认文件名所暗示的仓库根目录；最终 canonical 路径为 `docs/Project_Architecture_Blueprint.md/.html`。

# 2026-09-02 Grill 后确认的推送实施基线

- 用户逐项确认 Q1--Q55，并最终授权回写蓝图；Grill 期间未修改文档或产品代码。
- 10--15 个有效开发日与现有 W01--W21 的 98--142 小时、20%--30% 缓冲不相容；纵向 Unit 门禁进一步增加独立 shadow/cutover/cleanup 成本，暂定规划包络为 36--69 个 8 小时等效开发日。
- C0--C6 只能保留为能力标签，不能继续作为横向执行顺序；实施拓扑改为 Foundation → 纵向 MigrationUnit → Final cleanup。
- 首版审计下界为约 35 个 MigrationUnit：10 个 P0/错误完成 owner、18 个弱 authority/调度/语义单元、7 个已有强 authority conformance 单元；正式数量必须由 Foundation exact catalog 冻结。
- 工程开发与生产 rollout 分开计量：每交易日最多 promotion 一个 physical owner，预计 7--10 个交易周 rollout，整体 Production Verified 暂定 10--16 周日历跨度。
- transport typed receipt 与人工确认严格分开为 TransportAccepted/ManualConfirmedAccepted；弱回执渠道保持 COMPAT/BestEffort，不能推进 authoritative 通知游标。
- P01/N02 不重写高保证状态机，通过统一 AuthoritativeDeliveryPort 的 DedicatedAuthoritative adapter 做 conformance；普通 governor/boolean-only 路径逐项迁移。
- business DB 新增 additive push_notification_intents；durable DB 继续拥有 attempt/fence/receipt。rollback 关闭 producer/scheduler，但保留 authority/finalizer/reconciler 和旧路径 fence。
- activation 使用单一带版本 manifest，Unit 状态为 Disabled/Shadow/Active/Draining；artifact 可含多个 shadow-only Unit，但一次只 promotion 一个 physical owner。
- Production Verified 要求零未裁定 Uncertain、零超龄 finalizer backlog、零已知重复、真实 Accepted+AlreadyDelivered 灰度、完整 fault/rollback matrix、默认并行测试无未解释失败，以及文档/hash 同步。
## 2026-09-02 v18/v19 蓝图覆盖审计（进行中）

- `docs/v18.x` 共 9 份 Markdown，`docs/v19.x` 共 6 份 Markdown；当前 `Project_Architecture_Blueprint.md` 除 `AuctionRepush` 名称外，未显式标注 v18/v19 设计来源或覆盖状态。
- `v18.2`、`v18.3`、`v18.4`、`v18.5` 文件标题实际均标为 `v20.x`/`v20.0`，目录归属与文档自报版本存在冲突，蓝图不能把它们无条件表述为 v18 已实现设计。
- 后续必须把每份文档拆为 CURRENT / PARTIAL / PROPOSED / SUPERSEDED，并以源码或现有测试为准；目录位置和设计稿文字本身不是实现证据。
- v18 README 与中文整合设计均明确写明“尚未开始实现”；其权威目标是 Data Contract Gate、DecisionRecord、Paper Order/Fill/Ledger、Attribution/ModelChangeProposal 四模块，以及 Gate P/Gate L。蓝图当前未收录这套目标架构和“Gate L 前不得称实盘”的边界。
- v19 README 明确写明主设计仍在“设计阶段，待评审”，并把 v18 spec 定位为 v20+ 才回头落地；但它单独声称 v19.3 全天推送工作流已实施。故 v19 不能按一个整体标成 CURRENT，必须逐项核验。
- v18 设计中的 WORM/Object-Lock ≥5 年、DataEnvelope、唯一 DataHealthSnapshot、DecisionRecord、可重放 PaperLedger 等均为目标合同；其自己的证据边界明确反对将设计就绪等同实现完成。
- v18 四核心模块 companion 再次明确“仅设计；未开始实现”，建议新增 `src/data_contract/`、`src/decision/`、`src/paper_ledger/`、扩展 `src/review/`/`src/performance/`，并以 `AuditJournal`（远端 WORM/Object-Lock）为关键事件真相；这与当前蓝图中的现存模块 inventory 必须分层表达。
- v18 路线图是 6 个顺序 workstream，而不是已执行记录：数据合同 → 研究注册 → 决策/风控 → paper ledger → outcome attribution → live readiness review。Gate L 工作流明确“只设计、不写 broker order code”。
- v18 的核心产品边界可直接补入蓝图：推送/人工持仓/模拟 fill 均不等于券商真实成交；这是长期不变量，不依赖模块是否实现。
- v19.0 是 11 PR/5--6 周的 `评审中` 设计；源码搜索未找到其拟建目录/类型 `src/banner/snapshot.rs`、`src/breaker/mod.rs`、`src/error/mod.rs`、`src/log/rotate.rs`、`BannerSnapshot`、`StructuredError`、`--health` 实现，因此不能标 CURRENT。其设计内还存在 breaker threshold=5（PR 表）与 threshold=10（风险段）的自相矛盾，实施前要 ADR/规则冻结。
- v19.1 仍标 `Design`，提议 JSONL `src/review/signal_tracker.rs` + 新 R10；当前没有该文件/类型，但代码出现 BR-232 `prediction_tracker` 的候选样本写入/5 日回填（`src/database/mod.rs` 与 `src/bin/monitor/push_templates.rs`），属于“目标部分被另一实现吸收”，需以 PARTIAL/SUPERSEDED 表达，不能按原设计验收。
- v19.2 仍标 `Design`：AI 5 日验证依赖 v19.1；统一 LLM、死代码清理是否落地需逐项搜证。其历史 IC=-0.0775 不是当前值证据，蓝图只能记录设计风险与重新测量门禁。
- v19.3 自称 BR-223 已实施，但其 PushKind 数量仍是 57→59，而当前 enum/蓝图为 65；末尾实测又明确 AuctionRepush 未推、Quote Unsafe、新闻早间未命中。因此该文档是历史接线记录，不再是完整“权威版”。当前蓝图 §24 的 65-kind catalog/状态与源码行证据应覆盖并上位替代它。
- `push-template-catalog.md` 固定在 `master@97f28b9`、57 PushKind，且明确要求刷新；当前 65-kind 审计已使其枚举、数量、producer 状态和投递解释过期。它的“枚举/renderer/测试不等于生产接线”方法仍有效，但事实清单应标为 SUPERSEDED BY 蓝图 §24。
- v18.1 是 `Strategic Research/Directional Analysis`，不是架构合同；其中“优先券商 API/小资金实盘”的建议与 v18.0 “Gate P 后才可单独申请 Gate L”冲突。安全上必须以 v18.0 为准，不能把 v18.1 的路线直接落入实施计划；盈利/年化承诺也不属于架构完成标准。
- `v18.2-backtest-direction.md` 文件标题明确是 `v20.x` Strategic Planning，提出 `src/backtest/` 事件引擎、成本/涨跌停/因子/归因四层与 11 周路线；当前无 `src/backtest/`。应视为误归档的 v20 提案，不能算 v18 CURRENT，也不宜和 v18.0 PaperLedger 双重建模而不先 ADR 收敛。
- `v18.3-backtest-implementation.md` 同样自报 `v20.x / Implementation Design`，给出约 1,150 行 `src/backtest/` 伪实现；当前目录不存在，示例还含重复/截断字段等不可编译片段。它是实现草案，不是已经增加到架构或代码的模块。
- `v18.4-factor-zoo-design.md` 自报 `v20.x / Implementation Design`，提出 55 因子、Polars 与 5 种回测模式；当前无 `src/backtest/factor`，文内有重复字段/函数和未经基准支撑的“快 100 倍”等目标表述。蓝图只应把它列为候选研究提案，正式实施前需 point-in-time、缺失语义、成本与统计检验 ADR，不能把 55 因子表当 CURRENT inventory。
- `v18.5-production-readiness-design.md` 自报 `v20.0 / Draft`，提出 Redis/Postgres、REST/WebSocket、JWT/RBAC、Docker/Kubernetes、Web UI。它与当前单机 Rust/SQLite/gRPC 拓扑以及推送专项“本轮不引入 Redis/新微服务”边界不同；且日期/测试失败/覆盖率为历史未复核快照。应列为独立未来平台提案，不并入当前目标架构，除非另行批准 ADR 和产品范围。
- 最终落位：蓝图新增 §25，明确 v18/v19 “目标已纳入、实现不冒充”；v18 四模块均为 PARTIAL，Gate L 默认关闭；v19 通用运行面为 PROPOSED/PARTIAL；v19.3 被 §24 上位替代。推送 36--69 日基线不包含 platform-wide v19、v18 Gate P 或 v20 提案。

## 2026-09-03 方案审查证据

- 审查基线不是固定提交：HEAD 为 `a673043`，而 `docs/Project_Architecture_Blueprint.md/.html`、`task_plan.md`、`findings.md`、`progress.md` 均为 untracked；同时 selection activation 与 push template 正在被并行修改。因此本轮只能审查采样时点的工作树，不能声称审查对象可由该 commit 重现。
- §24.12 明确禁止 capability catalog 形成第二份手写清单（蓝图 `:1292-1298`），§24.18 又要求 65 enum/41 非 INACTIVE 等 exact-match（`:1474-1483`）；但仓库尚未发现受版本控制的蓝图生成/校验器，当前 Markdown 中的 65 行事实表将和未来 machine catalog 形成手工双源，维护约束还未落成可执行门禁。
- “Foundation Release（零行为变化）”同时包含 additive business table、startup/background reconciler、readiness、告警和 supervisor scheduler（`:1470-1481`）；而 readiness 设计可阻止 monitor 进入生产或让部署门禁失败（`:1381-1411`）。这不是严格的零行为变化，只能表述为“零 physical-owner/零 sink 行为变化”，否则验收条件不可证。
- 应用结果合同中的 `AlreadyDelivered` 只携带 `decision_id + authority`，`ManualConfirmedAccepted` 只携带 `resolution_id + evidence_sha256`（`:1328-1336`）；但同节要求它们可以推进本 occurrence 的 authoritative 游标（`:1341-1348`）。接口尚未暴露 occurrence、payload hash、receipt/resolution binding 或原 terminal version，finalizer 无法仅凭该返回值证明“确认的是同一业务意图”。
- 现有 durable 模型已经有更强的绑定：`DeliveryEnvelope.decision_identity` 哈希包含 business date、kind/subkind、scope、occurrence、evidence、subject 与 rendered-content hash（`src/durable_delivery/model.rs:573-609,611-689`）；Delivered 校验又要求 disposition 与唯一 authoritative sink join（`src/durable_delivery/coordinator.rs:6279-6355`），人工 accepted 也绑定 decision/attempt/envelope/disposition/audit（`model.rs:1400-1425`, `coordinator.rs:3830-3943`）。因此新应用层不应把强证据压扁成弱枚举；应返回/接受可重新验证的 terminal disposition reference，finalizer 必须按 decision + intent expected version 验证。
- §24 retention 只要求推送 terminal intent/receipt/audit “至少 90 天”，并把以后政策留给 ADR（蓝图 `:1562-1568`）；§25 又明确 v18 Gate P 的共享审计目标是远端 WORM/Object-Lock 五年并有 retention probe（`:1672-1677`），原 v18 计划也规定关键 paper-loop evidence 少于五年即 Gate P 失败（`docs/v18.x/v18.0-2026-07-16-writing-plans-implementation-roadmap.md:68-73`）。范围虽不同，但文档未显式说 90 天只是 push-migration operational minimum、不得覆盖更强业务/合规保留期，存在被实施成全局 TTL 的风险。
- §24 的 35 Unit 明确只是 machine catalog 前的下界（`:1483-1494`），却直接作为每 Unit 4--8 小时乘数推导 36--69 日和 10--16 周（`:1572-1581`）。该数学作为 planning envelope 可以，但不是可承诺基线；Foundation exit 应包含一次正式 re-baseline/置信区间，否则“冻结后才估”与当前对外周期数字并存。
- 跨库协议只描述“业务事务写 Prepared intent → runtime 投递 → authority accepted → finalizer → accepted-but-not-finalized reconciliation”（`:1359-1366`），没有规定：Prepared intent 在创建 durable decision 前崩溃如何重新 claim、并发 runner 的 lease/CAS、intent 唯一键和 immutable payload hash、expected-version 永久冲突如何进入人工处置、Rejected/NotDelivered/Uncertain 如何回写业务 intent。它不能直接变成数据库状态机；至少要补状态/转移表与 crash-point 测试矩阵。
- rollback 合同要求新 producer 可关闭，但 authority/finalizer/reconciler/fence 必须继续（`:1359-1366,1553-1553`），同时 failure matrix 又要求 `pending intent binary rollback`（`:1566-1569`）。若回退到 Foundation 之前的旧 binary，这些组件根本不存在；所以应明确“只能逻辑 rollback 或回到 Foundation-capable N-1”，并建立 app binary ↔ business schema ↔ durable schema ↔ manifest schema 兼容矩阵。
- activation manifest 目前只规定 schema version、四态与自身 hash，并用受控重启改变 owner（`:1411`）；却没有 artifact/executable/catalog/schema hash、reviewer/effective time、generation/CAS、前序 manifest/promotion journal。现有 selection activation 已证明生产 gate 实际需要 exact config hash、时间序、artifact validity（`src/selection/config_activation_v2.rs:222-269`）、executable revision（`:319-332`）和运行中输入突变检查（`:443-468`）。仅记录 manifest hash 不能证明“审核的代码就是运行的代码”，也不能机器强制“一交易日只 promotion 一个 Unit”。
- 当前 Markdown/HTML 自身仍同步：HTML `body` 与 sidebar 都声明 Markdown SHA `6c2a...`（HTML `:242,251`），等于当前 Markdown SHA。但同步流程不可复现：仓库搜索只有两份蓝图成品，没有生成/验证脚本；§24.19 的“唯一 catalog 派生/自动对账”要求（Markdown `:1613`）尚未转成 CI 或受版本控制工具。
- 蓝图证据已经发生采样漂移：§25 声称基线为 commit `a673043`（`:1624`），当前工作树的 `push_templates.rs` 在 2,793 行起新增实现并使后续行号整体移动；例如 §25.8 仍引用旧的 `push_templates.rs:5714-5745,7821-7850...`（`:1745`）。若读者打开当前文件，这些精确行号不再指向声明代码。必须要么 pin `a673043:path:line`，要么由 CI 对当前 revision 重新生成证据锚点。
- 工期计算引用“此前 W01--W21 已有 98--142 小时”（蓝图 `:1574`），但当前 Markdown/HTML 已删除 W01--W21 定义；全蓝图只剩这一处引用。其明细只残留在 untracked planning artifacts（`task_plan.md:218`, `progress.md:158`），正式方案读者无法审计 98--142 小时从何而来。这使 36--69 日的基数不可复算。
- 每个 Unit 的 promotion 门禁列出了 shadow/自然 occurrence/真实 transport/停线条件（`:1541-1551`），但 test/lint/check 的 fresh receipt 只出现在 Program 最终退出标准（`:1592-1599`）。测试不能延迟到所有 owner 已迁完；Foundation 和每个 Unit 都应有独立 pre-promotion gate，至少覆盖公开接口、crash points、old/new semantic projection、no-side-effect shadow 与 rollback。
- v18.2--v18.5 的标题确实自报 v20.x/v20.0，但“误归档”只由目录/标题冲突推断，未有 owner 决议；当前 README 直接写“误归档的未来提案”（`docs/v18.x/README.md:28-31`）。审查口径应改为“版本标签冲突、按 v20 proposal 解释、待文档 owner 决议”，避免把推断写成已确认历史事实。
- §25 的部分源设计没有进入 Git：HEAD 只跟踪 v18.0 四份文档和 v19.3；本地 `v18.1--v18.5`、`v19.0--v19.2`、`push-template-catalog.md` 被 `.gitignore:16` 的 `/docs/*` 匹配，且不在 `git ls-files`。因此 §25 对它们的结论无法从 `a673043` checkout 重现；需选择“正式纳入版本控制”或“记录外部 evidence URI + content SHA”，不能继续以本地文件路径冒充提交证据。
- 方案声称 Implementation-Ready，但最关键的 `CompletionPolicy` 只有名字：它被放入 catalog、要求所有游标声明、并被用来决定人工确认是否推进（蓝图 `:1292-1298,1347-1348,1522-1528`），全文没有 enum/variant、输入输出、允许结果映射或 side-effect contract。若不先冻结它，caller 解释 `PushOutcome` 的分叉只会搬到 finalizer 内部。
- `PreparedPush` 与 `PreparedFacts` 也没有关系定义：domain module/`JobDecision::Ready` 使用 `PreparedPush`（`:1292,1316-1324`），Foundation/shadow 又以 `PreparedFacts` 为共用输入（`:1474,1480,1511-1516`）。需要明确 Facts（一次取数、不可变）→ SemanticProjection（可比较）→ PreparedPush（exact bytes/identity）的单向类型流，否则 old/new exact-match 和“provider 只调一次”没有可测试接口。

## 2026-09-03 当前架构蓝图同步更新

- 当前 canonical 产物已经迁到 `docs/Project_Architecture_Blueprint.md/.html`；根目录旧产物不存在，不能沿用上一轮路径。
- HEAD 已从上一轮的 `e2503f2` 前移到 `a673043`，新增 BR-250 NewsAI display name、BR-178 selection authority fail-closed、BR-255 attribution epoch/backfill 等实现，必须纳入本次重扫。
- 工作树存在 `.gitignore`、selection activation、版本 README 和 `src/bin/monitor/push_templates.rs` 并行改动；只在蓝图内引用当前采样事实，不改这些文件。
- 现有蓝图已扩展到 §24 推送专项、§25 v18/v19 覆盖、§26 维护触发器；更新基础架构章节时必须保留这些已批准内容。
- Fresh 2026-09-03 inventory：514 个 Rust 文件、379,107 行；61 public modules、28 binary targets、41 integration-test targets、1 benchmark、1 custom-build、1 library。28 binaries 比 9 月 1 日多出的目标是 BR-255 新增 `attribution_backfill`。
- 当前热点：`src/bin` 75,666 行、`src/database` 73,767、`src/data_gateway` 30,555、`src/selection` 25,967、`src/durable_delivery` 18,856、`src/performance` 15,697、`src/pipeline` 14,893、`src/event` 13,159、`src/monitor` 12,853；蓝图旧 `data_gateway=58,149` 偏差最大。
- Markdown 当前 2,027+ 行，基础 CURRENT 章节 1--23 仍是旧架构；§24/§25 为后续加入的 PROPOSED/HISTORICAL 审计内容，更新时应按 section boundary 精确替换而不是全文件重写。
- BR-255（`1abeb9d`）新增 `attribution_backfill` binary，并把 monitor 15:05--15:20 归因价格来源从会被五秒 freshness 拒绝的 realtime quote 改为 `HistoricalBarsGateway` 当日收盘价；归因窗口在 active epoch 生效首月截断到 effective date，daily persistence/read-back 保留 epoch authority。
- BR-250（`cbe63f4`）为 NewsAI 卡片通过统一证券身份 Gateway 注入 display-only 名称；名称不进入 identity、证据 hash、prompt 或 DB，恢复路径缺名时降级为代码，不能在蓝图中误写成新的权威字段。
- BR-178（`6e69fea`）在 selection-v2 schema authority 未接线时显式跳过 recovery/due tick，并一次性告警；它强化了“代码存在不等于能力已发布”的 activation/fail-closed 边界，不改变 selection-v2 的未发布状态。
- 基础章节全文核对确认旧架构贯穿而非孤立措辞：§2--§7 的 system/container/deployment/data/gRPC，§17 feature，§18 lazy routing/build，§19 test count，§20 build/start，§21 extension，§22 ADR，§23 governance 都仍以 in-repo provider server/local fallback 为前提。
- §8 启动门大体仍成立，但证据行号已漂移；§8.2 后台任务必须由 7 改 8 并加入 startup review backfill；§8.3 应补 `blocking_market_data`、`closing_valuation_runtime`、`data_mode_probe`、`intraday_market`、`news_aggregator_init`。
- 不应全盘重写仍有效的业务深层内容：CLI pipeline、selection-v2 stage ownership、durable state machine、双 event bus、business/durable DB 隔离和推送专项可以保留，只刷新受到近期提交影响的事实、图和行号。
- 附录 A 当前把已删除的 `grpc_server`/`magic_compat` 当顶层模块，遗漏 `market_domain`；data_gateway 子模块仍列已删除的 `magic_tdx*`。应以 fresh `src/lib.rs`/`mod.rs` 声明替换。
- 28 个 current binaries 中新增 `attribution_backfill`；已删除 provider-only server/probes/replays 不得保留。41 个 integration targets 的精确 Cargo metadata 列表已取得，旧 B 附录中的 `magic_*` 与 `br192_t0_counted_binding` 等三项已删除。
- 附录 C 的 40 implemented/22 frozen operation 数量仍可保留，但解释必须从“本地 delegate 已实现”改为“本仓 consumer contract catalog；外部 provider-host capabilities 必须协商满足”。
- legacy 53、selection-v2 12、durable 18、DecisionState 14 的现有完整目录尚未被最近提交改变；`closing_valuation_run/item` 已在附录 D，但 §14.3 的 module-owned incremental 表说明仍需补其 owner/append-only 语义。
- 附录 H/I 必须删除 provider/monitor 双进程、Magic import boundary 和 `KEEP_LOCAL_OPS` 债务，改为 external provider-host、client-only repo、no-Magic gate，以及保留的源码注释/legacy naming 债务。
- `Cargo.toml` 当前没有 `[features]` 且没有 Magic provider 依赖；tonic/prost 是客户端合同依赖。`build.rs` 仍生成 server trait，是为了合同/test fixture 编译能力，不代表仓库存在 production server target。
- `build.rs` 与 `Cargo.toml` 注释仍有 `grpc/market.proto`、`grpc_market_server`、Magic revision 等迁移前措辞；本次不改产品源码，但蓝图必须显式列为注释债务并以 `client-bundle/market.proto`、Cargo targets 和 README 的真实边界为准。
- README 当前明示 provider-host 独立部署、本仓无 provider 实现/server target/local fallback；生产构建只列 monitor/client probe，`data_provider` 被定义为委托统一 Gateway 的进程级缓存 facade。
- current `data_gateway` 公开模块已无 `magic_tdx`、`magic_tdx_selection`、`magic_tdx_t0`，新增独立 `t0_evidence`；`market_domain` 11 个子模块承接 provider-neutral 类型。附录 A.2 应据此更新。
- Current monitor 任务树仍为 4 个 main loops + 8 个 supervised background handles；`review_backfill` 等待 durable `runtime_producer_ready` 后启动。主循环锚点仍在 `main.rs:5479-5539`，monitor 内部 intraday/market join 在 `:11296`。
- 当前 `src/bin/monitor/main.rs` 的部分旧注释仍声称 paper sell 因账本错误暂停，但实际 `paper_sell_paused` 已默认 false、只在 `PAPER_SELL_DISABLED=1` 时暂停；蓝图必须以函数行为和最新提交为准，不能复制附近旧注释。
- 旧蓝图“没有 Prometheus exporter”与仓库事实冲突：Cargo 依赖 `prometheus`，且 `src/bin/monitor/metrics.rs` 定义 `:9090/metrics` 指标模块。还需确认它是否由 production composition root 实际启动，再决定标 CURRENT 或 INACTIVE。
- Prometheus 复核结果：`metrics.rs` 没有被 `main.rs` 以 `mod metrics` 接线，文件本身只定义 registry/encode，没有 HTTP listener；因此 production 仍无 Prometheus exporter。蓝图应写成“存在未接线的 metrics prototype/dependency，当前无可达 exporter”，而非完全不存在相关代码。
- CI current facts：`ci.yml` 运行 fmt、clippy all-targets/all-features、test all-targets/all-features；coverage 使用 llvm-cov；no-Magic 脚本未接入 workflow。`compliance.yml` 仍引用 Cargo metadata 中不存在的 `--test e2e`，应在蓝图中标为 CI 配置漂移风险而不是宣称 compliance 全部可运行。
- §10 应更新 paper-sell 默认放行边界，并在“复盘归因”补 BR-255：15:05--15:20 使用 HistoricalDailyBars 收盘价，另有 `attribution_backfill` 运维入口；不新增服务或数据库。
- §11 激活状态应补 BR-178：selection schema authority 未接线时 recovery/due tick 明确跳过；selection-v2 仍属 CONDITIONAL/未发布，不能因新增 skip gate 改标 CURRENT。
- §12 恢复原则应补 `RejectedDurable` 只有显式 `authorize_rejected_retry` 后才能回到 Reserved，并补 startup `runtime_producer_ready` 对 review backfill 的屏障；14-state 数量保持不变。
- §14.3 incremental owners 应新增 closing valuation 两张 append-only 表；BR-255 继续使用已有 attribution epoch 表族，不引入新表。
- §15 应补 NewsAI v2 的 chain context 与 BR-250 display-only target name，明确 chain 字段进入 prompt/hash、name 不进入 identity/hash/DB；`ChainRisk` 属 monitor 事件类别而非新 AI service。
- current evidence anchors 已刷新：NewsAI chain `news_ai.rs:589`、display name `:120,282-288,1236`，BR-178 `outcome_v2.rs:1106`，closing valuation exact-date `closing_valuation.rs:153`，BR-255 close prices `monitor/market_data.rs:88` 与 monitor call `main.rs:9084`。
- HTML 是自包含 shell + pre-rendered `<article>` + 每图 base64 Mermaid source + `blueprint-data` 全量 Markdown base64 + SHA 元数据；可在不改交互壳的前提下机械重建 article/diagram/source/hash/date。
- 本机没有 pandoc/cmark/marked/markdown-it/showdown/commonmark；继续依赖临时一次性转换会保留不可复现债务。本次应新增仓库内受限 GFM renderer/validator，覆盖蓝图实际使用的 heading/table/list/blockquote/code fence/Mermaid/inline code/link/bold 语法。
- 生成器必须 fail-fast 校验 section 数、Mermaid 数、source hash、内嵌 Markdown byte equality、HTML id 唯一和标签闭合，并保留现有搜索、折叠、主题、打印、图缩放/回退 JS。
- Pre-edit stale scan 命中基础章节及附录的全部已知旧机制；此外 §24.10 仍写“provider/monitor 双进程拓扑”，也必须改为“external provider-host + 本仓 monitor/CLI”的真实系统边界，避免 PROPOSED 章节继承旧前提。
- HTML 可安全替换边界为 `<article id="blueprint-content" ...>` 到对应 `</article>`；页面尾部 `blueprint-data` 为 base64 Markdown，source note/body/sidebar 三处 SHA 与 header date/hero metrics 也需机械刷新。
- 现有交互 JS 从 DOM 动态生成目录、搜索和状态 token；renderer 只需保持 `.blueprint-section`、`.section-title-row`、`.section-body`、`.diagram-shell/.diagram-source` 合同，不必重写 JS。
- 已按 current code 重写蓝图 §2--§7：系统边界现在是 external provider-host → versioned gRPC → 本仓 client/data_gateway；部署、容器、数据流和时序图均移除 in-repo server/local fallback。
- activation 四态没有合法转换表和原子 owner-switch 协议。方案只写 `Disabled/Shadow/Active/Draining` 与一次一个 physical owner（`:1411,1517-1518,1553`），但没有规定旧 caller 何时读取同一 gate、Active→Draining/rollback 的 CAS、Weak/BestEffort 旧路径如何被 fence。对不共享 durable decision 的 NotificationService，单靠新 coordinator dedup 无法阻止 cutover 瞬间双发。
- 现有 startup health 失败仅日志/告警后继续启动（`src/bin/monitor/main.rs:5087-5103`）；新方案的 `CoreUnready` 则禁止 monitor 进入生产（蓝图 `:1406-1409`）。这再次证明 Foundation 会改变生产可用性；还需冻结 gate 的公开输出/exit semantics，避免“进程活着但部署门禁失败”只存在于日志而没有可供 deployer 查询的接口。
## 2026-09-03 架构蓝图同步：附录残留

- 蓝图主体已切换为“仓外 provider-host + 本仓 client-only”的当前边界，但附录 C/H/I 仍残留 `production implemented`、`provider/monitor 双进程`、`grpc_market_server.rs` 与 `KEEP_LOCAL_OPS` 等历史措辞。
- 维护触发器已移到第 26 节；最终维护说明和附录证据矩阵需要同步当前章节编号与 28 binaries / 41 integration tests / 61 public modules。
- 附录残留现已清除；第 26 节自动对账基线同步为 61 modules / 28 binaries / 41 integration tests / 40 consumer-used operations。
- 主体的系统上下文、部署、数据平面、gRPC 合同与 monitor 任务树已统一表达当前“仓外 provider-host、本仓 client-only”实现，并明确 test fixture server trait 不构成生产 server target。
- 最近 8 个提交核对确认：HEAD 为 BR-183 重发激活；核心架构相关提交依次包含 BR-255 归因每日链恢复、BR-178 selection authority 缺失时显式跳过、BR-250 display-only 证券名、paper sell gate 解除、BR-249 NewsAI/告警源/估值/复盘链恢复。
- Push 专项的现有文档基线仍声明 65 PushKind、58 presentation tuples / 54 unique kinds、23 durable counted kinds；需要用现行源代码/测试进一步验证，而不能只沿用旧页面统计。
- `src/durable_delivery/model.rs` 当前以 `PushKind::ALL: [Self; 23]` 直接固定 counted catalog 为 23；monitor-local `notify::PushKind` 是更宽的展示/治理枚举，两者不能混称。
- 源码机械计数确认 monitor-local `notify::PushKind` 仍为 65 个变体；`presentation_registry.rs` 仍以长度为 58 的静态数组固定 production presentation descriptors，因此第 26 节这两项未发生漂移。
- 现有 HTML 是可保留的单文件交互外壳，但正文仍是 2026-08-30/09-02 旧快照，包含 62 modules、40 binaries、44 tests、内置 `grpc_market_server` 与 `magic-gateway` 等已失效事实。
- HTML 的运行依赖明确：`#blueprint-content` 预渲染正文、`.blueprint-section` 折叠、`.diagram-shell`/`.diagram-source` Mermaid 图源、`#blueprint-data` 完整 Markdown、body/source-note SHA-256；生成器必须同步更新这些字段，不能只替换肉眼正文。
- 网页 hero 还硬编码 62/40/44 和“已实现 gRPC 操作”；生成时应改为 61 modules、28 binaries、41 tests、40 consumer-used gRPC ops，并新增 HISTORICAL 状态 chip。
- 每个 Mermaid figure 的必要 DOM 包括可缩放 action buttons、raw preview、fallback details 与 base64 `.diagram-source`；现有前端 JS 可继续复用这一结构，无需重写页面行为。
- 当前蓝图正文使用的 Markdown 子集是 H1-H3、连续 blockquote、普通段落、无嵌套列表、GFM 表格、horizontal rule 与 fenced code/Mermaid；未发现 Markdown links、嵌套列表或依赖任意 raw HTML 的正文，因此可用受限、可审计的标准库 renderer 精确覆盖。
- Renderer 首次 `ruby -c` 稳定复现于标题正则：Ruby regex literal 中的未转义 `#{1,4}` 被解析为字符串插值语法，而非 Markdown 的 1--4 个 `#` 量词。根因局限于单个正则；最小修复是把字面 `#` 转义后重跑同一语法检查。
- 标题正则最小修复后 `ruby -c` 通过；首次完整渲染随后在内部 `embedded Markdown mismatch` 守卫处失败，且 HTML 尚未写入（守卫发生在写入前）。需先比较源/解码后的字节编码与长度，再决定修复点。
- Base64 调试证明 source/decoded 均为 164,756 bytes 且 SHA-256 完全一致；失败原因是 Ruby `UTF-8` 与 `ASCII-8BIT` 字符串编码标签使 `==` 返回 false。将守卫明确改成二进制字节比较后，完整生成成功：35 sections、19 diagrams、SHA-256 `e2febdfd9a9c95a842a5037d81c5aa76cdfacc28b9b61297f3efffda73750340`。
- 首次 `--check` 暴露幂等问题：Ruby squiggly heredoc 会去掉 metrics/status HTML 的公共缩进，而二次替换正则错误地依赖 8 空格前缀。生成内容本身正确；根因是模板匹配对格式空白过度敏感，应让这两个边界匹配忽略缩进。
- 空白边界修正后 `--check` 幂等通过。旧快照词扫描为零结果；独立字节验证确认 embedded Markdown 与源文件完全相同、19 个嵌入 Mermaid source 与 Markdown fence 一一相同、35 sections 正确、全页 HTML ids 唯一。
- 初版证据引用检查器把反引号中的时刻（如 `09:15`）和上下文短文件名误判为 `path:line`，产生大量假阳性；后续边界检查应只匹配显式 repo-relative path（`src/`、`tests/`、`docs/`、`config/`、`scripts/`、`.github/` 或根文件）。
- 收紧规则后共检查 137 个显式 repo-relative `path:line` 引用：文件缺失 0、起始行越界 0。
- 近期变更已落在对应架构所有权中：BR-249/250 属于 NewsAI/monitor/durable 链，BR-255 属于 HistoricalBarsGateway + attribution owner，BR-178 属于 selection-v2 authority gate；paper sell 与 inactive legacy paper engine 已明确分开。
- 网页 2 段可执行 inline JavaScript 均通过 `new Function` 语法编译；本机存在 Google Chrome，可进行真实 headless browser 验证。
- Headless Chrome 实际加载成功且无 Runtime/Log exceptions：35 sections、19 diagrams、117 级联目录 links、19/19 Mermaid 均渲染；页面/正文 SHA 一致。
- 交互实测：搜索 `BR-255` 返回“1 处 / 1 节”，主题切换到 light，折叠按钮把 `aria-expanded` 置为 false。初始断言使用了不存在的 `.collapsed` 类，需按页面 JS 的实际隐藏机制补查 section body 状态。
- 折叠实现实际使用 `.is-collapsed`（不是 `.collapsed`），并同步按钮符号与无障碍文案；浏览器断言本身需要按这个明确实现重跑。
- 最终静态门禁阶段再次运行 HTML `--check` 通过；Cargo metadata 能正常解析当前 package，no-Magic guard 未输出违规项。
- Rust 证据测试通过：`unified_data_architecture` 15/15、`tool_binary_process_isolation` 8/8、`grpc_contract::ops::tests::implemented_set_is_40_and_within_62` 1/1。编译输出存在项目原有 dead-code warnings，但无失败。
- 临时 headless Chrome 会话在验收后已终止；最终 `lsof` 确认 9333 无监听，不作为项目运行单元保留。
- 第 26 节已补充唯一网页同步流程；最终再生成 SHA-256 为 `a1acf98ec960880934285d1a71ecf6fba068d809b08174ab51871a91645f2f75`。最终 `--check`、embedded Markdown、19 Mermaid sources、unique ids、137 个显式 evidence refs 与 2 段 inline JS 均通过。
- 最终 worktree 复核显示既有用户修改仍在；本次新增/更新范围限于两份架构蓝图、HTML renderer 和已有 planning artifacts，没有改动产品代码、配置或用户当前文档改动。

## 2026-09-03 本次文档改动目标符合性复审

- 固定比较点为 HEAD `a673043acb9390605d2a43fc3ee2ad01488f633e`；这是 unstaged/untracked WIP 审查，没有 `HEAD..HEAD` 新提交。tracked 文档 diff 是 `.gitignore`、docs 总索引、v18/v19 README 与 v19.3 历史提示；核心蓝图和 renderer 均为 untracked deliverable。
- 最新蓝图已把当前架构同步到 2026-09-03：顶部明确 external provider-host、gRPC-only、61 modules/28 binaries/41 integration tests，§24/§25 仍保留为 PROPOSED/coverage 审计；本次不是旧 SHA `6c2a...`，当前 Markdown/HTML source SHA 为 `a1acf98e...`。
- 先前“HTML 生成器不存在”的问题已部分修复：新增 `scripts/render-architecture-blueprint-html.rb`，§26 明确 `ruby scripts/render-architecture-blueprint-html.rb [--check]`；脚本可重建 article、指标、diagram source、内嵌 Markdown 和 SHA。脚本文件没有 executable bit，直接运行 permission denied，但文档要求通过 `ruby` 调用，因此不构成文档命令错误。
- renderer 内置 fail-fast 目前只有 table width、fence closure、替换锚点唯一、section/diagram 自洽、embedded Markdown/hash 与 `--check` byte equality；planning 记录中提到的 HTML ID 唯一、标签闭合、inline JS/evidence ref 校验不是该脚本的内置门禁，仍依赖未落盘的一次性命令或人工复验。
- `code-review` 要求的 `docs/agents/issue-tracker.md` 不存在；本轮无 issue/commit spec，Spec 轴改用对话中的八项明确目标与 `task_plan.md` 已批准设计作为来源。
- 四阶段目录结构本身符合目标：盘前 5、集合竞价 7、盘中 22、盘后 31，总计 65、唯一 65；状态计数为 37 ACTIVE、24 INACTIVE、2 STARVED、2 OPT-IN。
- “逐行有证据”不再成立：蓝图 `:1768` 将 BR-232 Candidate promotion/采样/回填/test isolation 分别指向 `push_templates.rs:5714-5745,7821-7850,9190-9225,9695-9699`；当前相应实现实际从 `:5771`、`:7874`、`:9261`、`:9758/:9766` 开始，旧范围至少有三段落在 PaperTrade、R-12 或其他 task dispatch 上。
- §24 catalog 也受同一漂移影响：CandidateBoard 的 `:7752-7865` 只覆盖到当前函数开头/失效 diff，未覆盖 `:7874` 后的采样、`:7905` 后的快照/发送；这与 `:1103` 宣称每项连续覆盖 gate、状态写入、sink、回执的口径冲突。
- 正式文档只出现一次 `Q1--Q55` 总称，没有 Q1 至 Q55 的逐题选择→约束→章节映射；虽然许多决策已经写入方案，仍无法审计 55 个确认是否全部、准确落位。
- 工期 `36--69` 日依赖“此前 W01--W21 98--142 小时”，但正式 docs 中没有 W01--W21 明细；明细只在未跟踪 planning artifacts，干净交付无法复算估算。
- `CompletionPolicy` 仅被要求声明，没有类型、variants 和各类游标的映射；`PreparedPush` 与 `PreparedFacts` 的关系未定义；cross-DB intent 只有 Prepared/Finalized 叙述而没有完整状态转移/冲突/crash matrix；activation manifest 未绑定 executable/catalog/business schema/durable schema。因此 `docs/README.md:14` 的 `Implementation-Ready` 是过度声明。
- v18/v19 的“目标与实现分开”已在 §25 实质完成，但它引用的 9 份补充设计稿仍被 `.gitignore:16` 排除且未跟踪；fresh checkout 只能看到 README/v18.0/v19.3，不能复核 §25 的全文档裁决。
- `docs/v19.x/README.md:7` 仍把已删除的 `AGENTS.md`、`ENGINEERING_RULES_V2.md` 和退役 CLAUDE 规则写成当前规则基线；这是存量问题但与本次修改后的 active README 冲突，违反 `RULES_RETIREMENT.md:12,19-22`。
- HTML 同步链可验证当前两份文件一致，但不是真正 clean generation：renderer 在 `:259` 读取目标 HTML 当模板，HTML 丢失时无法仅凭 Markdown+脚本重建；Mermaid 在 HTML `:241` 仍依赖未锁 patch/integrity 的 CDN。应拆出受版本控制模板/静态资产，或降低“自包含、机械生成”表述。
- Fresh verification：`ruby scripts/render-architecture-blueprint-html.rb --check` 报 35 sections/19 diagrams/SHA `a1acf98e...`；`ruby -c` 与 `git diff --check` exit 0；架构测试分别 15/15、8/8、1/1 通过。测试只支持当前架构事实，不消除上述文档规格缺口。

## 2026-09-03 推送文档硬化实施发现

- 用户已确认 Q56--Q108 的推荐方案；Q71 最初选择“四阶段 Unit”，随后通过 Q74 明确修正为“四阶段 Epic + physical producer/occurrence/completion owner 原子 Unit”。
- 当前 `.gitignore` 采用 `/docs/*` 顶层屏蔽，只放行两份蓝图；要纳管 `docs/push-system/` 与 9 份 v18/v19 来源，必须同时放行目录本身和目录内精确文件，不能只写深层 negate 而不放行父目录。
- 当前蓝图 §24.10--§24.19 是待迁移的 PROPOSED/实施内容；§24.1--§24.9 是现状审计；§25.1--§25.8 是版本设计覆盖事实，§25.9 含计划/周期，应移到 RFC 或改为链接。
- `.github` 在 ignore 中但既有 workflows 已受跟踪，因此可在现有 `ci.yml` 中增加文档门禁而不改变 ignore 策略；本轮不新增独立 workflow。
- 旧 renderer 是单文件脚本且目标 HTML 反作模板；新结构应将 renderer、模板、离线资产和 validator 集中到 `scripts/architecture-docs/`，保留一个统一入口。
- 全仓只保存了 Q1--Q55 的选择结果和主题汇总，没有原始问题文本；若要形成“逐题选择→约束”而不虚构问题，需要从本次会话记录恢复原始 Grill 提问，或明确把矩阵标为根据已批准约束重建。
- 现有 `ci.yml` 只有 Rust fmt/clippy/test；文档检查可作为同一 job 中 Rust 编译前的纯 Ruby 标准库 gate，避免引入运行时或网络依赖。
- 新生成器需兼容当前较旧 Ruby；已实测缺少 `Array#filter_map` 与 `Enumerable#tally`，实现与测试不得依赖这些较新 API。
# 2026-09-06 第二批来源、目录与源码证据完成

- 隔离分支最终HEAD `767a76e`；Task1–Task7及最终双轴审查通过。最终目录冻结65个kind、102个producer、52个MigrationUnit、195条完整symbol证据/33个Rust文件和469个源码/Cargo文件。
- 最终审查补出普通启动all-date恢复、枚举外`--replay-force`以及`rust_impl`同行/opaque type/属性空白边界；均经公开CLI RED/GREEN和限定复核关闭。Q54健康webhook明确只从业务回执目录排除，仍需独立ops-alert审计。
- 第二批没有改运行时Rust；价值是把四时段入口、source、authority、policy、completion owner和恢复边界固化为机器可查且会因漂移失败的事实底座，不是线上行为已修复。
- 后续仍需完整RFC/WBS、蓝图/离线HTML/CI、运行时Foundation、52 Unit迁移及生产交易日验收。历史36–69工程日/7–10交易周不可直接沿用，须按52 Unit和W01–W21重算；Codex单开发者可执行，产品裁决/生产晋级/Uncertain处置仍需人。
# 2026-09-06 第三批RFC/WBS预检

- 旧硬化计划任务2要求完整类型、DDL、状态转换、崩溃恢复、调度/激活、W01–W21和Unit估算，单任务过大；第三批需拆成来源输入、领域合同、持久化协议、运行门禁、WBS、最终验证六个顺序审查面。
- Task4复核确认调度occurrence在业务intent产生前就需要自己的并发控制，不能借用`push_intents.version`；最终合同显式固定`ScheduleOccurrence.version:u64`、零值初始化、成功转换加一、溢出拒绝、原状态/expected-version/generation/fence/ReasonCode绑定、状态与转换证据原子提交、CAS零行零副作用及冲突重读禁止盲重试。
- 单交易日跨Unit physical-owner晋级配额必须在activation DB的`BEGIN IMMEDIATE`内按权威业务日窗口全局查询；紧急rollback可突破一次晋级上限，但必须新generation/新journal并阻断当日后续晋级。
- 运行readiness将ACTIVE producer契约缺失定义为`ProducerUnready`，将有效契约下某次occurrence输入不可用定义为`BlockedOnInput`；非交易日只产生scheduler evaluation事实，不制造occurrence或NoData/Disabled业务intent。
- 隔离分支已有108项决策、65-kind/102-producer/52-Unit目录与195条源码证据，但不存在`Project_Architecture_Blueprint.md/html`、全量再分析、最近五日证据和硬化计划；直接写RFC会让规范引用依赖160冲突的主工作区。应先逐字节导入相关输入并用SHA清单冻结。
- RFC的深模块seam应只有一个应用结果合同；复杂性藏在prepare/project/deliver/finalize、业务intent、transport authority和finalizer实现中。P01/N02作为专用authority adapter保留，不再建立第三套完成真相。
- 旧36–69工程日/7–10交易周只能作为历史对照；正式估算必须消费当前52 Unit并分别计算工程时间、单owner交易日晋级下限和外部/样本等待后的日历关键路径。
- 隔离分支当前实现基线明确区分两层：`durable_delivery::PushKind::ALL` 只有23个counted kind，`DeliveryEnvelope`已包含source/render哈希、occurrence、subject、policy和可选`TaskBinding`，`DecisionState`已有14态；它不能被文档误写成65个业务kind已统一接入。RFC必须定义65-kind业务合同如何适配、扩展或通过intent/finalizer对账。
- 当前机读目录已fresh回读为65 kind、102 producer、52 MigrationUnit；52个Unit ID从`MU-announcement`到`MU-review-r03-stored-recovery`均存在，但现有JSON没有估算、依赖、完成策略和晋级观察字段。正式WBS应以这些Unit ID为外键，不能重新手工造一套数量不同的迁移清单。
- 现有运行时权威结果是`AuthoritativeSinkResult::{Accepted,Rejected,Uncertain}`，恢复还暴露`ScheduleHydration`；RFC新增的应用层结果必须与这些现有类型形成明确映射，同时把“传输接受”“业务完成”“人工处置”分层，避免第二套终态真相。
- 源码映射不是简单的65−23：`durable_kind_and_sub_kind_with_override`实际把26个monitor kind投影到23个durable kind，其中FactorIC/SectorTier/CapitalVerify复用DailyReport sub-kind；其余39个没有直接映射。RFC必须机械核对这26→23全集，并按目录状态处置剩余39个。
- 计划审查指出CLI BestEffort不能塞进Blocked或升级为TransportAccepted。统一应用合同需保留`BestEffortAccepted/PartiallyAccepted/NoChannelConfigured/AllChannelsFailed`，但用独立compatibility evidence，禁止生成`VerifiedTerminalRef`或推进权威业务完成。
- 业务库finalization不是两步最终一致即可：intent CAS与append-only transition必须同一个本地事务；CAS零行不能追加事件，事件约束失败必须回滚intent。跨business/durable两库仍不宣称原子。
- 当前仓库和可达Git历史只保存“旧W01–W21合计98–142小时”，没有名称/逐项工时；新WBS必须标`reconstructed_2026-09-06`，旧总数只作历史对照，不能伪称恢复原表或强行拟合。
- Q44的10类P0生产晋级顺序仍有效；PaperSell/NewsAI等近期高流量事实只提高设计、回放和预修复优先级，若要提前改变physical owner仍需新增产品裁决。
- 精确WBS显示单开发者全范围基线为828.99h，20%缓冲后994.79h/124.35工程日；交易安全维度为42次physical-owner晋级，另有76个风险观察session。118是“观察不重叠”的保守串行场景，42才是Q36直接给出的晋级交易日下限，不能把二者或63个外部等待日混进工程小时。
- Q44 rank1的权威蓝图只明确default CLI单股/汇总typed BestEffort；`MU-cli-chain`虽技术上同为enum外NotificationService报告且无durable cursor，但没有产品晋级顺序批准，因此保持`approved_promotion_rank:null`，待新增裁决。rank9仅含7个ACTIVE ReviewTask，R03三个STARVED Unit同样保持null。
- WBS数值合同不能先经Float：JSON直接解析BigDecimal，再以Rational和整数分执行half-up；每行存储PERT后再汇总。非法输入诊断用collection index/JSON path，避免Hash插入顺序改变稳定错误数组。
- Task2审查证明“文档有关键字”不足以证明合同成立：旧校验器能放过payload hash进入intent身份、AlreadyTerminal绕过精确绑定、删除P01/N02正文、run_id退出canonical hash四类相反语义。现已改为结构化规范表+固定v1 profile+真实mutation测试，并完成中文化；71/1165全绿且独立复审0 finding。
- Task3持久化必须坚持三层事实分离：业务`push_intents`/transition、durable terminal authority、activation manifest/promotion journal。跨business/durable库不伪称原子；唯一原子边界是业务库内finalization CAS与transition append同一事务。
- `push_intents`在正式RFC中承担业务本地intent/outbox事实，避免新增第二张可独立变更的业务outbox真相；dispatcher通过稳定intent/decision identity、lease generation和expected-version CAS恢复。若未来运行时需要队列形态，应做兼容投影而不是复制权威状态。
- activation期望状态与晋级执行事实必须分表：manifest每代不可变，rollback创建新generation；promotion journal仅追加。业务生命周期、activation生命周期、authority eligibility也必须保持三张规范表，不能压成一个混合状态枚举。
- Task3最终DDL把初始`NoData/Disabled`建模为不可变`job_decision_kind`加四列发送材料整组NULL；Ready必须整组非空。非发送事实可以进入`ResolutionRequired`隔离，但不能伪造材料或进入`AwaitingFinalizer/Completed`。
- 状态迁移ReasonCode已成为受约束的状态机输入：CAS按合法边限制reason，append-only transition的reason必须等于CAS后的intent reason；不再存在状态与原因两套事实。
- 独立SQLite脚本以DDL前TEMP快照、25个受管对象登记和固定v1签名阻断缺失/弱化/额外挂表对象；无关legacy对象保留。脚本入口为sqlite3 CLI并以`.bail on`保证错误停止，不可原样交给library execute_batch。
- 所有SHA/Git及稳定intent/event身份同时约束TEXT类型、字符长度、BLOB字节长度和小写十六进制，封堵SQLite `length(TEXT)`遇NUL截断的绕过；摘要与原始BLOB是否匹配仍由后续应用实现重算。
# 2026-09-06 第三批最终审查与收口发现

- 第三批最终提交 `aad7ac1` 已把批准裁决、最近证据和当前 65/102/52/195 目录收敛为中文 RFC、可执行 SQLite 规格、状态/恢复/运行门禁及精确 WBS；它是规格完成，不是运行时修复完成。
- `ManualConfirmedNotDelivered` 不能继续落入无出口的 `ResolutionRequired`；最终合同新增独立业务终态 `NotDelivered`，只允许经认证、原 decision 精确绑定、terminal ref、独立 operator audit 与 expected-version CAS 进入。它解除未决阻断，但不推进游标、不授权重发、不算 Accepted/ProductionVerified 成功样本。
- 四个 runtime milestone 已按当前 52 Unit 定义：Foundation Ready、P0 Production Verified、Architecture Release Candidate、Program Production Verified；全部仍为 NotAttained。默认并行、双库分别备份与 Test restore、N/N-1 删除资格和 tail cleanup 均是未来运行验收，不由文档测试替代。
- Q4 外部兼容独立于 COMPAT authority：CLI invocation/arguments/exit/output、配置 key/default/scope、订阅/audience/required channels、template identity/rendered bytes 均需迁移前 golden；破坏性变化必须单独批准和版本化。
- RFC 已纳入恰好 55 行 Q1--Q55 冻结选择追踪，validator 拒绝缺行、重复、错误选择、失效章节或证据引用；历史 grill 字节未改写。
- CI release detector 必须同时证明窄 workflow envelope、真实 trigger、受支持 runner、active/fail-closed job/step、受支持 shell 和 exact checker command。公开 CLI 最终独立攻击 120 个反例全部拒绝，15 个最小/事件/bash/sh 正例接受；不再把 `if: '${{ false }}'`、`shell: echo {0}`、缺 trigger、未知顶层或 YAML 1.1 yes/no 当已执行门禁。
- whole-batch `git diff --check` 的 exit 2 仅来自两份逐字节冻结输入共 11 处 Markdown 双空格 hard break；修复波自身 diff-check 全部通过。输入 manifest 需要保持原 SHA，因此未修剪这 11 处，也未把全批误报为 diff-check 通过。
