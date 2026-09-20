# 会话交接 — 2026-09-20（MU-st-price 接线 = 52 Unit 迁移 1/52）

## 0. 起点不变（继承 2026-09-19 交接的四条）

1. 工作目录是 worktree `.worktrees/push-reliability-20260905`（分支 `codex/push-reliability-20260905`），主检出 master 不含这些提交。
2. 生产 monitor PID 763 从主检出制品运行，未触碰；本工作树代码仍未上线。
3. "重构完成" = 52 Unit 逐个接进 counted 持久投递。本会话完成 **第 1 个**：MU-st-price。
4. 接线配方：`unit-wiring-recipe-2026-09-19.md` 仍有效，且已被本会话验证为可机械执行（含 RED/回归/复审/实跑验收全链）。

## 1. 本次交付（1 提交，`47cf502`）

**MU-st-price (T-16 ST 涨跌幅变更提醒) 接进 counted 持久投递。**

- durable PushKind +`StPriceLimitChanged`（ALL 23→24）；policy 行 `(PerTicket, 86400s, Rolling)`，**计入 30 条/日预算**（用户裁决）
- occurrence = `st-price:{业务日}:{code}` 每票每日一次（用户裁决）；canonical 只含业务事实（name/hhmm 不进 identity，BR-250）
- `dispatch_st_price_limit_changed` 改走 `acquire_token → push_counted_with_binding`；删除 banner 提前失败（counted 门缺 banner 时 Denied，fail-closed 保持）
- 调用点 Pushed|Deduped 计成功 → 重试可越过已成功前缀（旧 L4 冷却陷阱消除）

**两条已核实的残余行为（记入 commit message，非回归）**：
- 事实漂移重试（now_price 变）仍 CooldownConflict 停在首票 — Rolling 86400 内容寻址固有语义，严格优于旧 L4
- 43/83/87/88 北交所别名码从"照推"变为 `Denied(HistoricalAliasRequired)` — fail-closed 且语义正确

**流程证据**：RED×2 先失败 → 新测试×5 全绿 → push_templates 313 / br196 23 / v14 27 / notify 64 全绿 → `monitor --test --push-dry-run` **EXIT 0 families=56 failed=0（实跑）** → 独立复审 9 问全过、2 条修正落地 → 提交。证据日志：`.superpowers/sdd/2026-09-20-t16-st-price-wiring/t16-wiring-evidence.log`（gitignored，未纳管）。

## 2. 接线可复制模板（比配方更具体的经验）

- **选 Unit 优先序**：PushKind/descriptor/BR-196 清单都**已存在**的 Unit（如 T-16）只需 3 个触点（policy 行 + 映射臂 + dispatcher 重写），绕开配方触点 4-6 的 7 处计数常量陷阱。raw `push_governor_v3` 调用点清单：main.rs 1888(SnapshotStale)/8803,8893(PaperSell)/9062(AttributionDaily)/9162(G5b)/9227,9607,9620,10757(IntradayMarket)/9871(VirtualWatch)。
- **Deduped 语义真相**（复审 Q5）：durable 层**没有** Deduped 状态 — 同 occurrence 同事实字节 → decision_identity 回放返回 **Pushed**；`PushOutcome::Deduped` 臂对 counted 路径是防御性死代码。写注释别写错机制。
- **交易所解析**：`resolve_production_equity` + `require_a_share`（`instrument_identity.rs:302`），比 holding_plan 的 `starts_with('6')` 启发式正确（BJ/别名码 fail-closed）。
- **lib 套件 flaky 甄别法**：同环境 `git diff > patch; git checkout -- <files>; 跑套件; git apply` 做基线对照（勿用 stash — 共享栈）。本次基线 15 失败 vs 改动后 11/19，同家族不同集合 = 预存顺序依赖。

## 3. 进度与下一步

**2026-09-20 续 (用户指令: 剩下全部接完 → 改自主连续模式「直接干完, 不要一直输入继续」): 已完成 15/52 — T-16 (47cf502) + A-12 (a892b85) + G5b (8f68fc0) + I-01 盘中轮动 (cc828b7) + I-02 新闻催化 (0277cc9) + BR-033 大宗盘中确认 (00128fd) + A-11 IPO 阶段催化 (f5226e9) + SnapshotStale 快照过期提醒 (9a05561) + LimitBoards 涨停板板数榜 (2941664) + DataMode 数据模式卡 (7e2a806) + A-02 竞价重推 (e90201e) + T-08 候选失效 (7fba9f3) + S-01 公告源事实 (a9cb117) + S-05 分析师上调 (c833c18) + D-01 新闻到灵感 (753c0b3)。剩余完整清单: .superpowers/sdd/2026-09-20-remaining-unit-inventory.md (A 类 9 个接线单元 / B 类缺陷源批次锁定跳过 / C 类已覆盖或 INACTIVE)。**

- 决策规则 (用户 2026-09-20 裁决): 每日必达类豁免预算 (BR-237 语义); 4 个结构性源缺陷 Unit (OrderAlert/FrozenSide/VirtualWatch/PaperSell) 跳过待修源、单独成批。
- 下一批接线顺序: registered-template 类 (BlockConfirm/IpoCatalyst 等) → SnapshotStale (不在 BR-196 清单, 需全 7 触点含计数常量陷阱) → 最后缺陷源成批。
- **复杂单元进展 (U14-U16, 已提交)**: ① v17_sources 管线 counted 接入模式 = 汇合点 push_normalized_event 加 counted 分支 (kind match + 防御臂 counted_source_kind_not_wired), S-01/S-05 已接; ② 源事实 kind 的 occurrence 含 source 段防跨源 event_id 碰撞; ③ br137 报告口径随去重机制迁移更新 (回放 Pushed 无物理重投, 决策状态守卫保证); ④ D-01 第七触点全套后 GOVERNANCE_SMOKE 仅剩 T-11; ⑤ news-ai-same-tick 家族自营 reservation 流 (v14_gate_news_ai 直连 prepared gate, 不经 counted 协调器) — 同 kind 三引擎残余行为; ⑥ 子代理机制本机休眠下不可靠, U15 起改主会话自审 (10 问留痕)。
- **N-02 撤回经验 (U13, 未提交)**: 前提核实必须确认 kind 全部生产路径的推送架构是否同构 — N-02 有直接推 (NewsFlashGate) 与 reservation 结算流 (main.rs:7864 push_flash_reservations → NewsFlashNotifyOutcome Terminal/RolledBack/Uncertain) 双路径, 半转换 fail-closed 会掐掉聚合卡 → 撤回, 剩余复杂单元需管线级方案 (v17_sources 源事实管线 / reservation 结算流 / smoke 清单处理)。
- **DataMode 经验 (已提交 7e2a806)**: ① **既有行为测试是旧语义权威** — br116 双送达测试推翻「镜像 L4 默认 1800s」推理, 改 WindowMode::None (BR-116 已确认状态对精确去重不设粗粒度冷却); ② 模式变迁卡用变迁对 (old:?/new:?) 做 occurrence, 不用 BusinessDateOnce (一日可多次变迁); ③ 健康告警 retry_authorized=true (状态变迁事实) 与 I-01/I-02 false (时刻锚定内容) 的边界 = 补发内容是否仍有效。
- **LimitBoards 经验 (已提交 2941664)**: ① 旧 L4 默认行 (`_ => Some(1800)` / `_ => Global`) 也是镜像对象 — grep 无显式行≠无冷却, 查默认臂; ② 同 kind 多 shape (L-01/L-02/L-03) 三合一 helper 转换一处全覆盖; ③ 复核实锤: counted kind 在 v14_gate_prepared 直返 Approved 不跑 L4 dedup + coordinator.prepare 先回放决策再评估 Rolling head → 同槽重跑回放 Pushed 不被 head 挡 (retry_authorized=false 组合安全)。
- **SnapshotStale 经验 (已提交 9a05561, 全 7 触点首例)**: ① 计数常量陷阱全套共 **9 处** (配方触点 6 低估了: 还有 main.rs TemplateTestSummary::validate 硬编码矩阵两分支 + dry-run 测试 fixture) — dry-run 首跑 EXIT=2 是唯一能暴露 validate 矩阵的方式 (单元测试 fixture 各自为政不互验); ② 全 7 触点 = descriptor 数组类型×2 + ACTIVE_PRESENTATIONS + ALL_PUSH_KINDS + kind-cover + lifecycle 矩阵 (validate 期望 + 测试字面量 default/v2 分支) + descriptor 计数 (消息串+比较常量) + family/kind total + EXPECTED_CATALOG_TOTAL + rendered preview + main.rs 接受数; ③ 复审代理「出生即停滞」出现 (transcript 156B 两次), TaskStop+SendMessage 两连恢复仍有效; ④ requires_banner=true + 旧调用 None banner → 计数门与旧 v14_gate 同取 LATEST_BANNER 行为等价 (T-16 先例); ⑤ 健康提醒 retry_authorized=false 论据 (days_behind 时刻锚定) 与 A-12 归因 true 的边界 = 内容是否按 business_date 内容寻址可重放。
- **A-11 经验 (已提交 f5226e9)**: ① 每日一次全市场 digest → Global + BusinessDateOnce, 修复旧「跨日期共享 kind-全局 1800s 冷却」缺陷 (补推昨日 18:50 可挡今日 19:00); ② 空 code 旧形态不意味 Ticket 维度缺失 — 是 kind-全局; ③ 复盘类无 banner → CountedSourceOnly 门; ④ 复审恢复: TaskStop + SendMessage 双连有效 (本轮复审代理停滞 13 分钟后经此法恢复并给出裁定)。
- **BR-033 经验 (已提交 00128fd, registered-template 类首个)**: ① 名称含 Intraday 实际是 19:00 盘后 review side route — 分流按**实际语义** (盘后复盘 → 豁免预算) 而非 kind 名; ② 批量多票循环必须 **PerTicket** scope (Global 会首票 claim 头阻塞同批其余票); ③ business_date 穿线传 **trading_date** (锚定交易业务日) 而非 Local::now(); ④ 复核实锤 requires_banner=false 的 kind 计数门走 **CountedSourceOnly** (BR-241 公共源形态), 注释别写 CountedCombinedAccount; ⑤ canonical 枚举用 Debug 表示 — 枚举改名会变 identity (低风险); ⑥ 复审提醒: policy 行变更 → config_hash 变化, 未来上线重启前必须重发 BR-183 activation; ⑦ retry_authorized=true 依据 = backfill 重跑 side route 是既有补偿路径 (main.rs:6115 → dispatch_post_session_review)。
- **I-02 经验 (已提交 0277cc9)**: ① **配方第七触点: GOVERNANCE_SMOKE_IDENTITIES (br196_test_delivery.rs) + main.rs smoke 块** — counted kind 若在 smoke 清单里, generic smoke dispatch 被 counted_binding_required 拒 → dry-run EXIT 2。前四轮 kind 恰都不在 smoke 块才没暴露。修复 = 移出清单 (P-01/R-03 先例: capability_unavailable 出声, 保留渲染跳过 dispatch) + cardinality 测试同步。② 生产与手工工具共用 dispatcher 时转一处全覆盖 (dispatch_news_catalyst_daily)。③ 同 I-01: 事件驱动一次性调用 → retry_authorized=false 保真。
- **I-01 经验 (已提交 cc828b7)**: ① 同 kind 混合语义 — policy 行按主导语义分流, 残余行为入 commit message; ② retry_authorized=false 偏离论据 (内容时刻锚定+进程内补偿已存在); ③ 前提核实穷举全部 dispatch 家族 (manual_push 手工工具会被 fail-closed 波及); ④ v14_adapter quiet_hour/br137 测试断言 kind 每轮轮换; ⑤ catalog 计数测试有 3 处断言; ⑥ 复审代理机器休眠停滞 → SendMessage 恢复+限时裁定指令。
- 每 Unit 流程: 前提核实 → RED → 实现 → 回归 → 独立复审 → 提交。证据日志: `.superpowers/sdd/2026-09-20-u12-candidate-invalidated-wiring/u12-wiring-evidence.log`。
- **经验补充 (A-12 轮)**: ① schema.rs seed 计数改消息串必须同步改比较常量 (否则 panic "must have N rows, got N" 自相矛盾); ② include_str!("main.rs") 源码扫描守卫存在 (attribution_epoch_runtime.rs), 改调用点必须同步守卫 seam; ③ LAST_RUN 类无条件设置语义要保真, 推送失败补偿靠 durable 决策而非进程内重试; ④ 复审核实: Uncertain 需人工裁定 (启动对账只补 Reserved/Rejected-retry), commit message 别写错。
- gRPC 数据问题记录约定 (用户指令): 记入 `grpc_handoffs/` 目录。
2. 配方 §6 的**单一事实源收敛**（触点 6 计数常量从 descriptors() 派生）— 接第 2 个 Unit 前做能省一半维护成本。
3. 52 Unit 全部接线后：上线决策（`EffectBroker::production()` 恒 `ProductionRefused`，需发布决策 + 制品 + 回滚 + 启动对账）— 仍是未授权项。

## 4. 边界

- 未部署、未改生产闸、未写生产库、未启停 monitor；提交在 worktree 分支，未并入 master。
- 本 handoff 与 `.superpowers/sdd/` 证据日志均在 gitignore 范围（本文件需加 `!` 规则或 `-f` 才纳管，沿袭 2026-09-19 的做法）。
