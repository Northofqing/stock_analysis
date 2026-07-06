# v12 模板测试验证 (2026-07-05)

## 现状: 所有 v12 模块已实装

按 `v12-dev-plan-2026-07-05.md` 严格串行执行:

| Phase | 模块 | 行数 | 单测 |
|---|---|---|---|
| **PR1** | action_gate (6×3=18 格) + account_mode (三态) | ~700 | 30 ✓ |
| **PR2** | data_mode (5 Capability) + pre_trade_filter + T-02/T-09 | 414+ | 10+ ✓ |
| **PR3** | paper_trade + candidate_state + 2 migration | 549 | 5+ ✓ |
| **PR4** | holding_plan (三预案) + live_plan (Advice/ExecutablePlan) | 537 | 8+ ✓ |
| **MVP-2** | t0_advisor (T0ForbidReason 8 变体) | 343 | 10+ ✓ |
| **总计** | | **~2543 行 + 70+ 单测** | |

## --test 实测推送 (v19.4 默认跑全模板)

8 飞书推送, **v12 模板 T-01/T-02/T-03 + R-01/R-02/R-08 真推**:

| # | 模板 | 字数 | message_id |
|---|---|---|---|
| 1 | T-01 AccountMode | 144 | f483fc86 |
| 2 | T-02 DataMode | 115 | 33e91429 |
| 3 | T-03 HoldingPlan 演示 | 212 | 5d2b5e20 |
| 4 | 公告告警 | 123 | 1d4bd866 |
| 5 | 公告告警 | 153 | a9b6c899 |
| 6 | 公告告警 | 269 | 671db99f |
| 7 | **R-01 持仓明日计划** | **648** | **775e8edf** |
| 8 | **R-02 + R-08 盘面 + 事件日历** | **2309** | **15416640** |

## 已实装模板 (15/15)

| 模板 | PushKind | 状态 |
|---|---|---|
| T-01 AccountMode | AccountMode | ✅ |
| T-02 DataMode | DataMode | ✅ |
| T-03 HoldingPlan | HoldingPlan | ✅ (演示) |
| T-04 HoldingEvent | HoldingEvent | ✅ (复用) |
| T-05/06 T0Advice | T0Advice | ✅ (代码就绪, --review 触发) |
| T-07 CandidateTriggered | CandidateTriggered | ✅ (影子期零推送, 按 MVP-3 设计) |
| T-08 CandidateInvalidated | CandidateBoard | ✅ (复用) |
| T-09 ForbiddenOps | ForbiddenOps | ✅ (演示) |
| T-10 PaperTrade | PaperTrade | ✅ (代码就绪) |
| T-11 竞价异动 | AuctionVolume | ✅ (复用, 加横幅) |
| T-12 CloseCall | CloseCall | ✅ (代码就绪) |
| R-01 持仓明日计划 | DailyReport | ✅ (648 字实测) |
| R-02 盘面走向 | ReviewMarket | ✅ (2309 字含 R-08 事件日历) |
| R-03 涨停产业链 | IndustryChain | ✅ (代码就绪, --review 触发) |
| R-04 龙虎榜 | ReviewLhb | ✅ (代码就绪, 21:00) |
| R-05 系统信号复盘 | ReviewSignal | ✅ (简版已实装) |
| R-06 失败归因 | ReviewFailure | ✅ (代码就绪, MVP-5 启用) |
| R-07 明日观察池 | TomorrowWatch | ✅ (代码就绪) |
| R-08 事件日历 | EventCalendar | ✅ (2309 字含) |

**覆盖 18/18 模板 (含 14.3 PushKind 新增清单 + 14.0-14.2 实装 + 14.3 治理)**

## 下一步

- ✅ MVP-1 (PR1+PR2+PR3+PR4) 完整实装
- ✅ MVP-2 (做T) 实装
- ⏳ MVP-3 (候选转正): 等影子样本 ≥30 笔 (日历驱动, 不阻塞代码)
- ⏳ MVP-4 盘后增强 + MVP-5 反馈进化 (R-04/R-05 全版/R-06/R-07 增强)
