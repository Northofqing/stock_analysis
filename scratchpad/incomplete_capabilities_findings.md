# 未完成能力盘点 (from Explore agent)

**重要否定结果**: 生产 Rust 代码中**没有发现**真正的 `TODO` / `FIXME` / `todo!()` / `unimplemented!()` / "not implemented" panic。`XXX` 匹配要么是例子、文档 placeholder 要么是 sentinel tag。**真正的未完成是 运行时接线、缺失 producer、spec-only 模块、不完整 runtime contract**。

## A. 半实现：代码有，但 runtime path / completion contract 缺失

### A1. PostFixedPriceOrder/Fill 缺真实事件源
- T-14 / T-15 push 有周期性 runtime caller + dispatch 机制，但**无生产 `TradeEventSource` 注册**
- src/bin/monitor/push_templates.rs (register_trade_event_source)
- docs/v19.x/push-template-catalog.md:139-148, 207-217
- 完成工作:
  1. 实现真实 adapter
  2. 启动时注册
  3. 保留 source identity/event time/freshness/per-row 失败语义
  4. 加 production-path integration test + canary 证明非空
- 阻塞: 权威 provider contract 未选

### A2. CandidateInvalidated 缺生产 trigger
- metadata / renderer / governed wrapper / E2E 全有，**唯一确认 call 在 test**
- src/bin/monitor/push_templates.rs:677, 7405, 13835
- docs/v19.x/push-template-catalog.md:215

### A3. Failure-attribution 缺生产 input loop
- src/review/failure_attribution.rs 有模块，但 R-06 `ReviewFailure` pipeline 不完整
- 缺: 可分类失败 schema + 来自实际决策的样本 + 调度 attribution
- 阻塞: 依赖 decision-to-outcome identity + 缺失的 `DecisionRecord` boundary

### A4. Push outcome 部分降级
- L4-L7 暴露 `PushOutcome`，但**兼容 dispatcher 仍 collapse 为 `bool`**
- 失去 governance denial / dedup / sink failure / 成功 区分
- src/bin/monitor/notify.rs, push_templates.rs
- 文档: docs/audits/2026-07-21-verified-project-backlog.md:53, 125-126

### A5. 核心 metrics 只实现了一小部分
- 缺: 源健康 / breaker 状态 / 推送结果 / 跳过作业 / 健康命令延迟
- src/bin/monitor/metrics.rs
- docs/v19.x/v19.0-operational-clarity-design.md:334-364

## B. Designed-but-not-built (spec 存在但无生产代码)

### B1. v19 operational-clarity 几乎全 design-only
- 缺失: `RunMode { Active, Quiet, Halted }`, `BannerSnapshot`, per-source `Breaker`, v19 `ErrorCode`, 日志 rotate, 专用 `--health` 命令
- src/monitor/mode.rs, src/banner/snapshot.rs, src/breaker/mod.rs, src/error/mod.rs, src/log/rotate.rs, src/bin/monitor/health_cmd.rs **全部不存在**
- 文档: docs/v19.x/v19.0-operational-clarity-design.md (评审中)
- 阻塞: v19 设计推迟到 v20+，WORM/Gate P 存储未决

### B2. 3 个 IPO PushKind 是 metadata-only
- `IpoListingApproval`, `IpoProspectus`, `IpoCatalyst` — enum + source mapping 有，但**无 renderer、producer、非测试 prod caller**
- src/bin/monitor/notify.rs, v14_adapter.rs:726-729
- docs/v19.x/push-template-catalog.md:131-137, 209-212

### B3. v18 四个核心安全抽象全 spec-only
- `DataEnvelope`, `AuditJournal`, `DecisionRecord`, `PaperExecution` lifecycle — 描述在 spec 里，**生产中不是 boundary**
- 部分 paper 代码存在: src/trading/paper_trade.rs, paper_engine.rs
- 阻塞: v19 设计推迟到 v20+，WORM/Gate P 存储未决

### B4. README "immutable audit" 声明只部分实现
- README.md:3-4 宣传 immutable audit
- 但 docs/audits/2026-07-21-verified-project-backlog.md:65, 72, 105-106: WORM provider 未选，`AuditJournal` port 缺失

## C. Orphan code

### C1. `run_auction_agent` test-only
- src/opportunity/auction_agent.rs:148 定义
- src/opportunity/auction_agent.rs:372-445 仅有 test call
- 阻塞: 与现有 auction monitor / candidate-selection pipeline ownership 待定

### C2. Block-trade dispatchers 无生产 caller
- src/bin/monitor/push_templates.rs:4834, 4880, 8382-8431
- T-12/T-13 PushKind 有 param struct + renderer + async dispatcher，**无非测试 caller**

### C3. 旧 `AuctionRepush` 故意断开但仍看着像 active
- 唯一生产 push 已被删
- src/bin/monitor/notify.rs:45, 173, main.rs:7083
- docs/v19.x/push-template-catalog.md:112-114

### C4. `notification::send_daily_report` library-only
- src/notification/service.rs:697 — 全 0 caller
- 阻塞: architectural choice between generic notification vs monitor's governed push

**Not orphan**: `run_closing_valuation_once` 被 src/bin/monitor/main.rs:3858 调用

## D. 数据 pipeline gap

### D1. 投递审计 持久化失败 但 push/event 文件仍存在
- 48h 观察: 验证的传输活动发生，但权威年度 audit 文件没前进
- 首次 append 在 legacy row 失败，然后 dispatcher 进入故意 poisoned-state repeat
- .planning/2026-07-20-monitor-48h/progress.md:49-60, 378-390
- 阻塞: BR-142 release blocker

### D2. 近期目录有数据，但不是完整 one-to-one delivery ledger
- 2026-07-21: 334 / 2026-07-22: 250 / 2026-07-23: 198 push files
- event_bus 2026-07-22.jsonl = 132 lines (与 push 250 不一致)
- 结论: **没有完整空目录证据**表明 push producer 整个缺失，但 7-22 数量差异确认这些存储**不能互换或当单一权威 ledger**

## E. 规划文档的 Open items

### E1. 48h master-release 观察**未完成**
- 累计 28:01:10 / 48 active hours
- .planning/2026-07-20-monitor-48h/progress.md:378-390
- 阻塞: BR-142 + 用户取消

### E2. 监控计划 phases 仍 pending
- restart pending after BR-142
- BR-142 compatibility work in progress
- 后阶段 monitoring/delivery pending
- .planning/2026-07-20-monitor-48h/task_plan.md:33-69

### E3. BR-140 缺 release proof
- post-session review 部分实现，但需完整 Gate D / merge / production canary
- docs/audits/2026-07-21-verified-project-backlog.md:33-35

### E4. P0 数据有效性 tasks 仍 open
- Eastmoney post-session K-line 恢复
- 异常涨跌幅校验 (用 listing/board rules)
- R-02 required index fields
- 持仓估值价 fresh
- 缺失 MAE/MFE 和涨跌停值必须 unknown 而非 0
- 原子化的 position 跟踪 + 结果保存
- 移除 global-database panic paths
- MarketAnalyzer error vs empty-result 区分
- docs/audits/2026-07-21-verified-project-backlog.md:94-107

---

## 总结 (Top 14)

1. T-14/T-15 callers 存在但无注册 production event source
2. `CandidateInvalidated` test/wrapper 完整但缺生产 transition producer
3. 3 IPO PushKind metadata-only
4. 2 block-trade PushKind dispatcher-only
5. `run_auction_agent` 完整但 test-only
6. `AuctionRepush` 仍是 disconnected legacy surface
7. v19 operational clarity 大部分未实现
8. v18 四个核心安全抽象 spec-only
9. Failure attribution 缺 decision-to-outcome production loop
10. Structured push outcome 在兼容边界被 collapse
11. 权威 delivery audit pipeline 在 legacy data 上失败
12. 48h release 观察停在 28:01:10
13. BR-140 仍需完整 release/canary proof
14. 多条 P0 数据有效性 paths 继续把 missing/error 转为可用的假值

**注**: `src/selection/` 未发现 actionable unfinished markers — 它的不完整（如果有）不体现为 TODOs/stubs/空函数体。
