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

**2026-09-20 续 (用户指令: 剩下全部接完, 分流规则已裁决): 已完成 4/52 — T-16 (47cf502) + A-12 (a892b85) + G5b (8f68fc0) + I-01 盘中轮动 (cc828b7)。下一 Unit: registered-template 类 (BlockConfirm/IpoCatalyst/NewsCatalyst 等, 逐一核实源健康; NewsCatalyst 已因 v14 seam 轮换成为熟面孔)。**

- 决策规则 (用户 2026-09-20 裁决): 每日必达类豁免预算 (BR-237 语义); 4 个结构性源缺陷 Unit (OrderAlert/FrozenSide/VirtualWatch/PaperSell) 跳过待修源、单独成批。
- 下一批接线顺序: registered-template 类 (BlockConfirm/IpoCatalyst/NewsCatalyst 等, 逐一核实源健康) → SnapshotStale (不在 BR-196 清单, 需全 7 触点含计数常量陷阱) → 最后缺陷源成批。
- **I-01 经验 (已提交 cc828b7)**: ① 同 kind 混合语义 (每日提醒×2 + 周期信息卡×1 + 手工工具×1) — policy 行 per-kind 粒度无法拆分预算豁免, 以主导语义 (盘中信息卡) 计预算, 残余行为入 commit message; ② retry_authorized=false 与前三轮 true 的偏离: 内容含时刻锚定+进程内补偿已存在时, durable 补发只推过时卡 (R-02 flood 风险), 旧注释「失败不重试」保真优先; ③ generic governor fail-closed 会波及手工工具路径 (manual_push → dispatch_registered_outcome) — 前提核实必须穷举 kind 的所有 dispatch 家族, 不只是 main.rs 调用点; ④ v14_adapter 测试 quiet_hour 断言用 uncounted kind, 每轮接线需轮换; ⑤ catalog 计数测试有第 3 处独立 len() 断言 (A-12 教训的完整形态); ⑥ 复审代理机器休眠会停滞 (本轮 2h+), 催促/恢复机制: SendMessage 恢复后要求跳过验证直接裁定。
- 每 Unit 流程: 前提核实 → RED → 实现 → 回归 → 独立复审 → 提交。证据日志: `.superpowers/sdd/2026-09-20-i01-intraday-market-wiring/i01-wiring-evidence.log`。
- **经验补充 (A-12 轮)**: ① schema.rs seed 计数改消息串必须同步改比较常量 (否则 panic "must have N rows, got N" 自相矛盾); ② include_str!("main.rs") 源码扫描守卫存在 (attribution_epoch_runtime.rs), 改调用点必须同步守卫 seam; ③ LAST_RUN 类无条件设置语义要保真, 推送失败补偿靠 durable 决策而非进程内重试; ④ 复审核实: Uncertain 需人工裁定 (启动对账只补 Reserved/Rejected-retry), commit message 别写错。
- gRPC 数据问题记录约定 (用户指令): 记入 `grpc_handoffs/` 目录。
2. 配方 §6 的**单一事实源收敛**（触点 6 计数常量从 descriptors() 派生）— 接第 2 个 Unit 前做能省一半维护成本。
3. 52 Unit 全部接线后：上线决策（`EffectBroker::production()` 恒 `ProductionRefused`，需发布决策 + 制品 + 回滚 + 启动对账）— 仍是未授权项。

## 4. 边界

- 未部署、未改生产闸、未写生产库、未启停 monitor；提交在 worktree 分支，未并入 master。
- 本 handoff 与 `.superpowers/sdd/` 证据日志均在 gitignore 范围（本文件需加 `!` 规则或 `-f` 才纳管，沿袭 2026-09-19 的做法）。
