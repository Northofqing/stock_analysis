# 推送源码审计工作表（2026-09-05）

PROVISIONAL。源码基线为 `07781bf386aafdf202851ae928efee8920387058`，隔离分支 `codex/push-reliability-20260905`。本表只冻结入口调查结果；ACTIVE 表示源码有接线，不证明部署、TransportAccepted 或用户已读。四时段是 phase Epic，不是 MigrationUnit。完整 owner、最终 evidence manifest、RFC、运行时 Foundation、HTML、CI 和工期尚未完成。

行级证据简写：M=`src/bin/monitor/main.rs`；T=`src/bin/monitor/push_templates.rs`；N=`src/bin/monitor/notify.rs`；V=`src/bin/monitor/v17_sources.rs`；R=`src/bin/monitor/review_batch.rs`；P=`src/bin/monitor/p01.rs`；F=`src/bin/monitor/news_aggregator_init.rs`；A=`src/bin/monitor/news_ai_shadow.rs`。`文件:行 symbol` 指此基线实际源码位置，不是冻结 item 身份。Task4 核新闻 owner，Task5 核状态/盘中 owner，Task6 核复盘 owner；待核不表示该入口不存在。

## 65-kind 主表

初判状态区分：INACTIVE 包括无 caller 和明确受阻入口，二者在入口列分开；STARVED 指有 dispatcher 但当前生产输入链缺失；OPT-IN 指显式启用条件。多入口共用 kind 不表示共用完成 owner。

| kind | 初判 phase | 初判 status | 生产入口 symbol / 无 caller 裁决与行级证据 | owner 核验 |
| --- | --- | --- | --- | --- |
| HoldingEvent | 盘中 | INACTIVE | 无发送 caller；M:9526 monitor_loop 关闭 legacy summary；M:11523 reject_unbound_alert_delivery 明确 sink_calls=0；T:465 仅 renderer | 无活动 owner；Task5 复核禁用边界 |
| DailyReport | 盘前/盘后 | INACTIVE | 无发送 caller；M:9474、11223 monitor_loop 盘前/收盘 counted binding 缺失，M:11523 告警拒绝；T:1164 仅 renderer | 无活动 owner；Task5 复核禁用边界 |
| Announcement | 盘前/盘中/盘后 | ACTIVE | M:7571 news_monitor_loop → V:324 route_announcement_batch → V:364 route_announcements_with_provenance | Task4 待核 claim/source fact |
| AuctionVolume | 集合竞价 | ACTIVE | M:9758 monitor_loop → T:5937 dispatch_auction_volume_daily | Task5 待核 auction_vol_notified |
| VirtualWatch | 集合竞价/盘中 | STARVED | M:9950 pilot 副推、M:10229 → T:837 dispatch_virtual_watch_daily；M:9807 空 post_close，M:9842 唯一 vector 填充受其限制 | Task5 待核快照/外层状态 |
| LimitBoards | 盘中 | ACTIVE | M:10351、10376、10401 monitor_loop 的首板/二板/三板分支 | Task5 待核 board_notified/分形态键 |
| SectorTop | 盘中 | ACTIVE | M:11001 monitor_loop → T:16489 dispatch_sector_top_daily | Task5 待核 last_sector_top |
| FundInflow | 盘中 | INACTIVE | 无 caller；N:61 enum、N:501 label、V14 适配及 BR196 fixture，未形成生产 dispatch | 无活动 owner |
| AuctionRepush | 集合竞价 | ACTIVE | M:9785 monitor_loop → T:7220 dispatch_auction_repush | Task5 待核共享候选外层闸门 |
| FactorIC | 盘后 | INACTIVE | 无 caller；N:602 DailyReportSubKind、N:698 dispatch metadata；计算报告与 durable 映射不是发送入口 | 无活动 owner |
| SectorTier | 盘后 | INACTIVE | 无 caller；N:604 DailyReportSubKind、N:708 metadata；同名 sector_score 领域枚举不是 PushKind producer | 无活动 owner |
| CapitalVerify | 盘后 | INACTIVE | 无 caller；N:606 DailyReportSubKind、N:718 metadata 和 durable 映射 | 无活动 owner |
| WeeklySOP | 盘后 | INACTIVE | 无 caller；N:71 enum、N:506 label、适配表及 fixture | 无活动 owner |
| StockPick | 盘后 | INACTIVE | 无此 kind 的发送 caller；T:3548 CandidateSource::StockPick 是候选输入，不是独立推送 | 无活动 owner |
| IndustryChain | 盘后 | STARVED | R:499 dependency 将 R03 留在 LegacyAccountGate；T:9546 dispatch_post_session_review 返回账户依赖结果，T:12239 R03 dispatcher 未由此入口放行 | Task6 待核 R03/date |
| TurnoverTop | 盘中 | INACTIVE | 无 caller；T:1101 render_turnover_top、T:1129 load_turnover_top_real 仅定义/preview/tests，无生产 dispatch | 无活动 owner |
| CandidateBoard | 集合竞价 | ACTIVE | M:9789 monitor_loop → T:7790 dispatch_candidate_board | Task5 待核外层闸门与子快照 |
| NewsRanked | 盘中 | INACTIVE | 无 caller；N:920 dispatch_table_init_audit 在 N:943 明示 disabled=no_producer | 无活动 owner |
| AccountMode | 盘前/盘中/盘后 | ACTIVE | M:2329 evaluate_account_mode_hook → T:1901 push_account_mode_change；M:9389 盘前重置补偿 | Task5 待核 account_mode_log/log_id |
| DataMode | 盘前/盘中/盘后 | ACTIVE | M:2564 evaluate_data_mode_hook，M:2712 → T:14447 push_data_mode_change | Task5 待核 pending/retry 状态 |
| HoldingPlan | 盘中 | ACTIVE | M:8520 prepare_holding_plan_messages；M:10928 monitor_loop counted dispatch；M:1490 run_daily_pushes 手动入口 | Task5 待核 timer 与 counted binding |
| T0Advice | 盘中 | ACTIVE | M:8276 prepare_t0_messages → M:10760 monitor_loop counted dispatch | Task5 待核 last_t0_scan/票级键 |
| CandidateTriggered | 盘前 | INACTIVE | 有受阻 caller：M:9611 → T:5714 dispatch_candidate_triggered_daily；外层已等待行情 active 又要求 Closed/09:00–09:15，且有 promotion gate | Task5 待核 preopen_aux_pushed，禁止视作活动 |
| ForbiddenOps | 盘中 | INACTIVE | 无 caller；T:720 render_forbidden_ops 仅 renderer、preview/tests，durable/registry 仅映射 | 无活动 owner |
| PaperTrade | 集合竞价 | ACTIVE | M:10046 monitor_loop → T:5633 dispatch_paper_trade_daily，读取当日严格完成态 | Task5 待核成交记录与通知键 |
| PaperSell | 盘中/盘后 | ACTIVE | M:8860、8951 monitor_loop 两入口；M:8248 paper_sell_paused 默认放行 | Task5 待核共享 code/day/Filled 事实 |
| SnapshotStale | 盘前/集合竞价/盘中/盘后 | ACTIVE | M:1887 check_snapshot_staleness_and_notify；M:5477 服务启动调用无时段门，可跨时段；M:8926 定时 15:10–15:13 调用 | Task5 核两 caller 是否共享 LAST/SnapshotReminderGate 完成范围 |
| AttributionDaily | 盘后 | ACTIVE | M:9118 monitor_loop 归因报告发送 | Task5 待核 ATTRIBUTION_LAST_RUN |
| G5bAttribution | 盘后 | ACTIVE | M:9219 monitor_loop 深链归因摘要发送 | Task5 待核 G5B_LAST_RUN |
| CloseCall | 盘中 | ACTIVE | M:8637 prepare_close_call_messages；M:11156 monitor_loop 尾盘 counted dispatch | Task5 待核 close_call_pushed |
| ReviewMarket | 盘后 | INACTIVE | 有受阻复盘入口；R:1682 review_preflight 明确 Disabled，T:9820 dispatch_r02_review_market_real 缺完整市场批次 | Task6 待核 R02/date 禁用状态 |
| ReviewLhb | 盘后 | ACTIVE | T:9546 dispatch_post_session_review → T:12803 dispatch_r04_lhb_outcome；自动/手动/补推入口见下文 | Task6 待核 R04/date durable |
| ReviewSignal | 盘后 | INACTIVE | 有受阻复盘入口；R:1687 review_preflight 缺 append-only outcome 源，T:13012 dispatcher 禁用 | Task6 待核 R05/date 禁用状态 |
| ReviewFailure | 盘后 | INACTIVE | 有受阻复盘入口；R:1692 review_preflight 缺 classified failure 源，T:13021 dispatcher 禁用 | Task6 待核 R06/date 禁用状态 |
| TomorrowWatch | 盘后 | ACTIVE | T:9546 dispatch_post_session_review → T:8033 dispatch_tomorrow_watch_outcome | Task6 待核 R07/date durable |
| EventCalendar | 盘后 | ACTIVE | T:9546 dispatch_post_session_review → T:10983 dispatch_r08_event_calendar_outcome | Task6 待核 R08/date durable |
| ReviewProviderTopN | 盘后 | ACTIVE | T:9546 dispatch_post_session_review → T:6765 dispatch_r09_provider_top_n_outcome | Task6 待核 R09/date durable |
| PositionReview | 盘后 | ACTIVE | T:9546 dispatch_post_session_review → T:8797 dispatch_position_review_outcome | Task6 待核 R11/date durable |
| ReviewBacktest | 盘后 | ACTIVE | T:9546 dispatch_post_session_review → T:9104 dispatch_r12_backtest_outcome | Task6 待核 R12/date durable |
| WatchlistTracking | 盘后 | ACTIVE | T:9546 dispatch_post_session_review → T:9447 dispatch_r13_watchlist_tracking_outcome | Task6 待核 R13/date durable |
| PreopenNewsHot | 盘前 | ACTIVE | M:5483 → P:1531 p01_scheduler_loop；M:4993 → P:1462 run_p01_compensation_once | Task4 待核同 business_date occurrence |
| IntradayMarket | 盘前/盘中/盘后 | ACTIVE | M:10836 盘中总览、M:9284 15:05 过期提醒、M:9664/9677 受阻预检；M:1490 --push → T:2654 dispatch_intraday_market_daily | Task5 待核四入口作用域，不能合成一个 owner |
| NewsCatalyst | 盘中 | ACTIVE | M:8188 news_monitor_loop → T:2981 dispatch_news_catalyst_daily；M:1490 --push | Task4 待核来源/去重 |
| SectorAnomaly | 盘中 | ACTIVE | M:11038 monitor_loop → T:16500 dispatch_sector_anomaly_daily | Task5 待核独立 last_sector_anomaly |
| NewsToIdea | 盘中 | ACTIVE | M:8156 → T:4077 dispatch_news_to_idea_daily；M:1490 --push；M:7831 附近 schedule_from_same_tick → A:265 run_same_tick_batches 为另一路 NewsAI | Task4 分核 D01/NewsAI owner |
| CatalystReview | 盘后 | ACTIVE | T:9546 review batch → T:13622 dispatch_catalyst_review_daily_outcome；M:1490 --push → T:13827 daily wrapper | Task6 待核 A10/date 与手动入口 |
| IndustryChainIntraday | 盘中 | ACTIVE | M:10877 monitor_loop → T:3465 dispatch_industry_chain_intraday_periodic；M:1490 --push → T:3459 daily wrapper | Task5 待核 periodic/manual owner |
| PostFixedPriceOrder | 盘中/盘后 | STARVED | M:11069 → T:4726 dispatch_trade_pipeline_orders_result；T:4701 fetch_pending_trade_events 要求注册，T:4695 register_trade_event_source 全 src 无 caller | Task5 待核 last_post_fixed_order，缺源 |
| PostFixedPriceFill | 盘中/盘后 | STARVED | M:11085 → T:4805 dispatch_trade_pipeline_fills_result；同一未注册源，timer 独立 | Task5 待核 last_post_fixed_fill，缺源 |
| StPriceLimitChanged | 盘中 | ACTIVE | M:11115 → M:1699 dispatch_st_price_limit_batch → T:5038 dispatch_st_price_limit_changed | Task5 待核 st_price_pushed/票级键 |
| EtfClosingCallAuction | 盘中 | INACTIVE | 无 caller；T:5095 dispatch_etf_closing_call_auction 只有定义，M:11133 仅注释/未使用状态 | 无活动 owner |
| BlockTradeIntradayConfirm | 盘后 | ACTIVE | T:9700 附近复盘 side route → T:7684 dispatch_block_trade_review → T:5129 dispatch_block_trade_intraday_confirm；名称含 Intraday 但实际盘后调用 | Task6 待核非 ReviewTask side route |
| BlockTradePriceRange | 盘后 | ACTIVE | T:7684 dispatch_block_trade_review → T:5178 dispatch_block_trade_price_range | Task6 待核交易记录/票级键 |
| PaperReview | 盘中/盘后 | STARVED | 入口异质：M:9019 → T:4633 noon 在 13:00–13:04 传 today，T:4267–4274 exact T+1/已完成日门使其结构性受阻，补数据不能恢复；T:4600 daily、M:1490 --push 和复盘历史补推仅在 exact T+1 已完成且有合法历史记录时可消费，当前自产快照链缺输入 | Task6 核 daily/历史 A01；Task5 核受阻 noon/NOON_SNAP_LAST，勿将 STARVED 概括成仅缺数据 |
| CandidateInvalidated | 集合竞价 | ACTIVE | T:7790 dispatch_candidate_board 的差分子推，T:7824 丢弃 push_candidate_invalidated 结果 | Task5 待核快照与外层双层边界 |
| IpoListingApproval | 盘后 | INACTIVE | 无 caller；M:5652 run_review_only 明示 disabled=no_producer；不能把 IpoCatalyst 当此 kind | 无活动 owner |
| IpoProspectus | 盘后 | INACTIVE | 无 caller；M:5652 run_review_only 同一明确声明 | 无活动 owner |
| IpoCatalyst | 盘后 | ACTIVE | T:9724 review side route → T:7421 dispatch_ipo_catalyst | Task6 待核独立 IPO 日级状态 |
| PolicyHit | 盘前/盘中/盘后 | INACTIVE | 无生产源 caller；src/news/aggregator/classifier.rs:316 classify_policy 仅定义/tests，V:546/561 为映射，V:716 为通用 adapter；M:7874 注释不是调用 | Task4 复核无 caller，禁止虚构 policy tick |
| EarningsBeat | 盘后 | OPT-IN | M:7943 附近 → V:997 poll_earnings_and_analyst；V:818 earnings_classification_gate 默认关，仅 EARNINGS_BEAT_ENABLED=1 | Task4 待核 earnings map/事实键 |
| EarningsMiss | 盘后 | OPT-IN | 与 Beat 同一 classifier/gate，V:997 poll_earnings_and_analyst 按分类分支产生 | Task4 待核共享 earnings owner |
| AnalystUpgrade | 盘后 | ACTIVE | M:7943 附近 → V:997 poll_earnings_and_analyst → V:922 analyst_upgrade_event | Task4 待核 observe-before-send |
| MarketActionAlert | 盘前/盘中/盘后 | ACTIVE | M:5343 服务 EventBus 订阅 → V:207 handle_monitor_event；T:2027 push_account_mode_change 的 Frozen 副推 | Task5 分核 OrderUpdate/Frozen |
| NewsFlashCritical | 盘前/盘中/盘后 | INACTIVE | 有共用入口 M:7571 news_monitor_loop → F:1059 push_flash_reservations；M:7649、F:148 明示 no_authoritative_strength_provider，F:518 后当前仅构造窗口 reservation | Task4 核受阻 N01，不能按 N02 的活跃性推定 |
| NewsFlashAggregated | 盘中/盘后 | ACTIVE | M:7797 附近 news_monitor_loop → F:1059 push_flash_reservations；F:142 四窗口 09:30/11:30/13:00/15:00 | Task4 待核 window/date authority |

## 入口面与 enum 外路径

| 入口 | 源码证据 | 本次裁决 |
| --- | --- | --- |
| monitor 常驻 | M:8796 monitor_loop；M:5483 启动 resident tasks | 区分 intraday_loop 与 market_loop；后者先等待行情 active，不能用盘前注释证明可达 |
| 新闻常驻 | M:7571 news_monitor_loop、M:5485 启动 | 公告、业绩/评级、NewsAI、NewsFlash 的入口均在表中；PolicyHit 没有实际 source caller |
| P01 自动及补偿 | P:1531 p01_scheduler_loop；M:4993 调 P:1462 run_p01_compensation_once | 两入口必须保留；是否共享完成 occurrence 留 Task4 |
| 手动 --push | M:5325 → M:1490 run_daily_pushes | 按 OpportunitySchedule 窗口调用 I01/I02/I03/I04/D01/A01/A10；盘前明确转 P01 专用入口；dry-run 与 smoke 不算生产 |
| 自动复盘 | M:6252 post_session_review_scheduler；M:6449 spawn_post_session_review_scheduler | 进入 T:9546 dispatch_post_session_review；不同 ReviewTask 不是同一个完成键 |
| 手动复盘 | M:5632 run_review_only → M:5770 附近 run_strict_review_only_inner | 使用 at_manual；R:1707 仅 R04 可绕过 21:00 门，R:1719 后 R07 仍按 eligibility_time 保留 21:00 门；来源验证不豁免 |
| 历史补推 | M:5961 run_review_backfill → M:6060 backfill_one_review_task | 保留旧业务日和 claim 恢复入口，Task6 核无 claim、RejectedDurable 授权和恢复边界 |
| 快照过期启动/定时 | M:5477、8926 → M:1887 check_snapshot_staleness_and_notify | 启动无时段门；定时仅 15:10–15:13。两 caller 的共享完成范围留 Task5 |
| PaperReview 午盘/daily/历史 | M:9019 传 today → T:4633 noon；T:4600 daily outcome，M:1490 --push 及复盘补推 | noon 在 13:00–13:04 的 today 尚未完成，T:4267–4274 与 T:4425–4433 排除合法记录；daily/历史必须有合法记录且 exact T+1 已完成。不能推断补齐快照即可恢复 noon |
| CLI 单股（enum 外） | src/main.rs:95 后模式分派 → src/app/modes.rs:14 run_analysis → src/pipeline/mod.rs:505 AnalysisPipeline::run → src/pipeline/analyze.rs:1137 process_stock_inner、1255 send | single_notify/send_notification 控制；先保存分析结果，再发送；无独立持久通知完成游标，不虚构新的 authority |
| CLI 汇总（enum 外） | src/pipeline/mod.rs:643、723 → src/pipeline/summary_notify.rs:64 send_summary_notification_to、109 send | 单次 pipeline 执行的汇总；文件保存不是消息接收 |
| CLI 产业链（enum 外） | src/main.rs:85 → src/app/modes.rs:106 run_chain_analysis_mode、176 send | --no-notify 控制；独立于 monitor PushKind::IndustryChain 的受阻 R03 |
| 09:05 / 15:30 产业链（enum 外） | M:9416 CHAIN_PREOPEN_LAST、M:8988 CHAIN_POST_LAST → 同一 run_chain_analysis_mode(true) | 两个不同日期状态，窗口分别 09:05–09:14 / 15:30–15:34，不能按同函数合并 owner |
| CLI 定时/龙虎榜分析 | src/main.rs:78 schedule；src/app/schedule.rs:16 run_scheduled_analysis、239 pipeline.run；src/app/modes.rs:186 run_lhb_analysis、293 pipeline.run | 复用上述单股/汇总发送路径；是额外上游入口，不能漏掉或计作新 PushKind |
| 仅保存市场复盘 | src/app/modes.rs:83 run_market_review_only、99 save_report_to_file | 没有 send，排除发送 producer |
| 历史 AlertManager / legacy alert | 全 src `rg -n '\bAlertManager\b' src` 无匹配；src/monitor/alert.rs:93 push_alert 只有定义，`rg -n '\bpush_alert\s*\(' src` 没有 caller | 本基线连 AlertManager 类型也不存在；不沿用历史命名制造 producer |

PaperBuy、Watchdog 仅为原混合工作树追加项，本隔离分支没有移入；不纳入 65 集合，不引用原工作树行号。枚举外路径也不增补到 PushKind 主表。

## 负向 caller 审计方法与裁决

使用全 `src/**/*.rs`，未按目录排除测试后直接计数。先检索完整 kind 标识及 `PushKind::`/`SourcePushKind::`/`K::` 映射，再检索对应 renderer、dispatcher、分类器、注册函数的完整函数名；逐项区分声明、metadata、通用适配器、preview/smoke、`#[cfg(test)]` 测试及真实调用。字符串和注释匹配仅作线索。仅存在函数定义也不算 caller。

本次执行的命令形状如下；`KINDS` 表示逐项展开主表中的无 caller 标识，不是实际环境变量：

```sh
rg -n '\b(KINDS)\b' src --glob '*.rs'
rg -n 'PushKind::|SourcePushKind::' src/bin/monitor/main.rs src/bin/monitor/push_templates.rs src/bin/monitor/v17_sources.rs
rg -n 'render_turnover_top\(|load_turnover_top_real\(|render_forbidden_ops\(|render_daily_report\(|render_holding_event\(' src --glob '*.rs'
rg -n '\bclassify_policy\b|\bPolicyHit\b' src/news src/monitor src/app src/pipeline --glob '*.rs'
rg -n 'register_trade_event_source|TRADE_EVENT_SOURCE|TradeEventSource' src --glob '*.rs'
rg -n 'dispatch_etf_closing_call_auction\(|dispatch_r03_industry_chain_real\(' src
rg -n 'virtual_observation\.(push|extend)|virtual_observation =' src --glob '*.rs'
rg -n 'AlertManager|push_alert\(' src --glob '*.rs'
```

另外用只读 Ruby 对每个候选 kind 的完整词匹配汇总文件分布，再检查额外命中：`src/durable_delivery/model.rs:173` 起是另一窄枚举及策略表，`src/bin/monitor/durable_delivery_runtime.rs:2282` 起是 K→D 映射，`src/push_l4/dispatcher.rs:32` 是通用 dedup 辅助，均不能反向证明生产者。`src/event/{mod,envelope,push_record}.rs` 的 HoldingEvent 命中为测试事件。V14 指 `src/bin/monitor/v14_adapter.rs`，BR196 指 `src/bin/monitor/br196_test_delivery.rs`。

| 无 caller 集合 | 命中分类和补充正证据 |
| --- | --- |
| HoldingEvent、DailyReport | N metadata、presentation registry、V14/durable 映射、renderer、preview/tests；M:11523 及 M:11223 后明确拒绝未绑定告警/总结 |
| FundInflow、WeeklySOP | N enum/label/cooldown、V14 映射、BR196 fixture；无业务 dispatch |
| FactorIC、SectorTier、CapitalVerify | 上述 metadata 加 DailyReportSubKind/durable 子类映射；factor_report/factor_ic 为计算，sector_score 的同名 SectorTier 为领域类型；未接到发送调用 |
| StockPick | N/V14/BR196 加 candidate_panel 的 CandidateSource；候选输入不会发送此 kind |
| TurnoverTop、ForbiddenOps | T renderer/loader 定义、preview/tests、registry/adapter；无调用 renderer 后发送的生产链 |
| NewsRanked | N:943 的 no_producer 正证据与负向检索一致 |
| EtfClosingCallAuction | T:5095 dispatcher 定义及其内部 kind 参数、preview/tests、registry；无上游 caller |
| IpoListingApproval、IpoProspectus | N/V14/BR196 及 M:5652 no_producer；与 IPO 催化真实调用区分 |
| PolicyHit | classifier.rs:316 定义，447/454/483/503/512 均在测试；V:546/561 映射、V:633 preview、V:1761 测试；无生产构造源事件入口 |

上述是 15 个“无生产 caller”kind。另有 CandidateTriggered、ReviewMarket、ReviewSignal、ReviewFailure、NewsFlashCritical 的真实受阻入口，不能把它们一起写成“没有 producer”。IndustryChain/VirtualWatch/T14/T15/PaperReview 的 STARVED 初判也不等同无 caller。状态依据源码可达性/输入接线，不从 level、展示注册表或历史报告继承。

## 已证实风险与关系（前次 16 项复核）

1. **候选双层状态**：M:9785/9789 两 dispatcher 同 tick、双 bool 才封 `post_close_candidates_notified`。T:7790 空 batch 直接返回；T:7763 末行读取失败化 None；T:7771 写错丢弃；T:7824 失效推送 bool 丢弃，T:7853 在主卡发送前推进快照。外层闸门与子快照不原子；失败后差分消失、双 bool 与各 cooldown 交互留 Task5。
2. **VirtualWatch 缺输入与 PaperReview 时间门是两件事**：M:9807 附近空 post_close，经 M:9842 唯一 push 填充 vector；M:9944/10207 的快照写入均依赖该观察集合。PaperReview daily/历史入口只在合法历史记录的 exact T+1 已完成时可消费。noon 则在 M:9019 传 today，T:4406–4409 以 today 为 review_date；src/calendar.rs:586–591 在 15:00 前将 completed_through 设为前交易日，T:4267–4274 使合法记录 Pending/OutOfWindow，T:4425–4433 跳过。因此 noon 结构性受阻，补数据不能恢复；空输入 NoData 也不是通知完成。
3. **PaperSell 两入口仍活动**：M:8248 gate 仅显式 PAPER_SELL_DISABLED=1 暂停；M:8860/8951 在卖出结果后发送。src/trading/paper_sell.rs:308 already_sold_today 按 code、sell、Filled、date(ts) 查重，462 先 simulate_with_audit_evidence；成交事实不等于通知接收。Task5 核两入口共享范围。
4. **预检位置不等于盘前可达**：M:9450 附近先等待 is_market_active，M:9595 后又要求 Closed 和 09:00–09:15，导致旧 P03/09:10 预检结构受阻；独立 P01 不使用该状态。
5. **板块计时器独立**：M:10998/11016 是两个 timer；M:11002/11007、11041/11044 各自成功或失败都推进一小时，不是共享 completion owner。
6. **产业链返回值缺口**：src/app/modes.rs:176 对 Ok(false)/Err 只 warn，183 前仍 Ok(())；M:8988/9416 的两 timer 据 Ok 封日。CLI 同样不能据函数返回声称消息 Accepted。
7. **公告 claim 边界**：V:364 验证来源后过滤；V:454 附近 annroute:date:source:external_id 认领；V:489 后非 Pushed 释放，释放失败可压住当日重试，存储不可用明确降级 L4。Task4 继续核 authority。
8. **D01 不等于 NewsAI**：M:8156 是重要公告触发 D01，M:7050/7094 属 smoke；T:4077 的 D01_LAST_PUSH 按 code:name 一小时，先推送再按需 virtual buy，后者失败会在通知已发时不推进 memo。
9. **NewsAI 独立入口**：M:7821 后 selection_v2_enabled 且 trading/auction 时对同 tick admitted 批次 schedule；A:265→A:454 assess_candidate。观察到多种恢复结果，但 Task4 尚未冻结 durable owner，不把本地 audit 等同 Accepted。
10. **业绩与评级两张 map**：M:7922 后 HoldingEarnings/盘后窗口/our_codes 门；V:997 的 earnings/analyst 每 code timer 独立，来源取得后、最终发送前推进。V:818 gate 默认关闭但位于 provider 拉取之后，关闭分类不代表零 I/O。
11. **评级观察先推进**：src/news/aggregator/analyst_state.rs:70 observe，130 后先插入再返回 Upgrade；同 report/date 返回 Duplicate。V:1149 后才构造升级事件，失败不自动回滚观察事实。
12. **两路市场异常**：V:157 MarketActionState 的 code→(action,shares) 在发送前更新；T:2027 Frozen 副推丢结果；T:1844 finalize_account_mode_delivery 只以主 AccountMode Pushed 标 log_id。两消息不能共用成功证明。
13. **过期提醒入口和状态范围**：SnapshotStale 的 M:1887 函数由 M:5477 服务启动无时段门调用，也由 M:8926 在 15:10–15:13 调用；至少五交易日过期后使用 SnapshotReminderGate::try_begin/finish，两 caller 是否共享完成范围留 Task5。M:9284 的 IntradayMarket 使用 SNAP_REMIND_LAST，不要按同“快照”词合并。
14. **复盘阶段与 side route**：R:499 dependency、T:9546 batch 分阶段；R03 受账户依赖门，R02/R05/R06 由 R:1638 preflight 禁用；T:9700 后的大宗/IPO 为非 ReviewTask side route，测试环境先拒绝它们。
15. **复盘终态不是统一 Accepted**：R:1236 apply_for_run 按 date/task 更新，Delivered/NoData/Disabled/永久 Failed 均可 Terminal；可重试错误 1/5/15 分钟。自动/手动/补推作用域留 Task6，不能把同 struct 当单 owner。
16. **ST 与 ETF 不能按注释类推**：M:1699 先准备完整 ST 批再发，一项失败返回 Err，M:11115 外层 Ok（含零条）才置 st_price_pushed；T:5095 ETF 虽有 dispatcher，但全 src 无 caller。src/portfolio/mod.rs:161 实际读取 ST metadata，不沿用旧“标记写死”注释。

补充归因完成缺口：M:9118 后不论推送 outcome 都封 ATTRIBUTION_LAST_RUN；M:9238 在 G5b 整批尝试后封日，不能把首批输入修复说成通知完成修复；M:9019/9024 午盘 PaperReview 忽略 bool 封 NOON_SNAP_LAST。通用入口 N:2217 先拒绝 counted kind；source-fact 路径 V:716 与 MarketAction generic 路径不同。这里只记录源码，不运行任何实际发送。

后续交接：Task4 冻结新闻/P01/N01/N02 的 occurrence、source、authority、policy；Task5 冻结状态驱动入口和 enum 外 CLI/定时的实际完成边界，特别核 SnapshotStale 启动/定时共享范围和结构性受阻 noon 的 NOON_SNAP_LAST；Task6 冻结各 ReviewTask 及 side route，区分 PaperReview daily/历史 exact T+1 已完成门，并保留仅 R04 手动绕过 21:00、R07 仍等待的差异。最终 Task7 重新核对工作表后生成三份正式候选产物，不能把本表行号直接冒充 locator item hash。
