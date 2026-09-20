# 剩余源边界交接 (2026-09-20, 接线收尾时核实)

## 1. T-14/T-15 (PostFixedPriceOrder/Fill) — 真实券商回报 feed 边界
- 状态: TradeEventSource (push_templates.rs:5721) 注释明示 "No default/
  mock source is installed" — BR-087 真实成交事件边界。生产从未注册
  register_trade_event_source → fetch_pending_trade_events 恒 Err →
  dispatcher 出声短路零推送。
- 修复前提: 需要真实券商委托/成交回报 feed (MonitorEvent::OrderUpdate
  事件总线生产者的真实数据源)。虚拟盘账本不产生真实 TradeEvent。
- 交接: 该边界为**外部集成前置**, 无本地代码可修。接线前需:
  ① broker connector 产出 TradeEvent 流; ② 注册 register_trade_event_source;
  ③ 然后按配方接线 T-14/T-15 (policy 行 + 映射臂 + dispatcher 转换)。

## 2. VirtualWatch (P-05) / PaperReview — STARVED 数据问题
- VirtualWatch: catalog STARVED — 候选台 P-05 管道无活动 owner/Unit;
  dispatch_candidate_board 现只承载 CandidateBoard + CandidateInvalidated
  (已接线), VirtualWatch kind 无生产调用点。
- PaperReview: STARVED (paper-review-daily-auto/manual/push/noon 四路由
  无数据源), 非 ReviewTask 结构。
- 交接: 两者均为**数据源缺位** (非代码缺陷)。修复需先定义数据源
  (候选台虚拟盘监控 feed / 虚拟盘复盘数据集), 再按配方接线。

## 3. 竞价上游 0.000 (已接线单元的数据问题, 沿用记录)
- 见 2026-09-20-auction-upstream-data-caveat.md: A-02/P-02/T-11 接线
  后 dispatcher 有有限正价 guard 出声降级, 上游 (VM magic-market.local)
  修复后自动生效。

## 4. Frozen 永续 (AccountMode 已接线, 数据现状说明)
- AccountMode (T-01) 已接线 (WindowMode::None 无冷却)。Frozen 自 7/15
  永续 = 设计常态 (reset 需完整 metrics 恒不满足) 且不 gate 微信推送 —
  接线不改变该现状; 账户模式变迁卡在 Frozen↔Normal 变迁时照常投递。
