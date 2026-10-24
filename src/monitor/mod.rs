//! 实盘监控模块（Phase 0-5 全部实现）。
//!
//! 子模块：
//! - data_quality:   数据质量门
//! - rate_budget:    请求预算与退避
//! - scanner:        阶梯轮询扫描器
//! - detector:       异动检测规则引擎
//! - signal_state:   信号状态机
//! - auction:        集合竞价扫描
//! - alert:          分级告警推送
//! - risk:           风控叠加
//! - signal_fusion:  信号融合
//! - checklist:      盘前/收盘总结
//! - prediction:     预测追踪闭环

pub mod adaptive;
pub mod advice_feedback_report; // 2026-10-16: 建议质量报告 (自进化回路第③段, 仅审计)
pub mod advice_outcome_backfill; // 2026-10-16: 建议结果滚动回填 (T+1/T+3/T+5)
pub mod alert;
pub mod alert_log;
pub mod attribution; // v10 P1 G5a 异动即时归因 (规则快归因, P95 ≤ 2s)
pub mod attribution_benchmark; // P6 二层归因: 个股 vs 市场 (2026-10-19)
pub mod attribution_deep; // G5b 深链归因 (盘后 LLM 深链, 2026-08-22 落地)
pub mod auction;
pub mod auction_watch; // 2026-10-18: 观察池竞价观测 (09:25 stage=Final)
pub mod board_member; // 板块 ← 成分股 反查表 (2026-10-19)
pub mod board_ranking_snapshot; // 2026-10-18: 板块榜每日落库 (f3 涨幅 / f62 净流入)
pub mod board_rotation_signal; // P5 板块轮动: 主线/接力/退潮 (2026-10-19)
pub mod buy_zone; // 2026-10-17: P2 可买价 (按票: 位置/买点区间/放弃价)
pub mod buy_zone_dispatch; // 2026-10-18: 盘后可买价推送 (16:00 明日计划)
pub mod chain_ai; // 2026-10-18: AI 产业链候选 (推断, 不参与决策)
pub mod checklist;
pub mod code_review; // 2026-10-18: 票级复盘 (paper_trades 聚合, 不推测原因)
pub mod concept_peers; // 2026-10-18: 概念共现 (从 stock_concepts 现算)
pub mod daily_bars_indicators; // 2026-10-17: 日线+均线补齐 (与 authority 门解耦, P2 数据基础)
pub mod data_mode; // v12 PR2-2.1
pub mod data_quality;
pub mod detector;
pub mod entity_linker;
pub mod event_bus;
pub mod f10_profile; // 2026-10-18: TDX F10 公司资料落库 (经营/行业/提示/股本)
pub mod g5b_analysis_v2;
pub mod g5b_empty_v2;
pub mod g5b_physical_v2;
pub(crate) mod g5b_selection_v2;
pub mod holdings_bridge; // 2026-10-20: Windows 持仓桥（TDX 导出快照，只读 HTTP）
mod integration;
pub mod news_ai;
pub mod news_monitor;
pub mod news_outcomes;
pub mod outcome_data;
pub mod position_change_detect; // 2026-10-18: 加仓/减仓识别 (相邻快照比 quantity)
pub mod position_sizing; // 2026-10-18: 仓位建议 (风险预算, 只建议不阻断)
pub mod prediction;
pub mod push_job;
pub mod rate_budget;
pub mod risk;
pub mod scanner;
pub mod signal_fusion;
pub mod signal_state;
pub mod ticket_auction_price; // 2026-10-23: 情绪票 09:25 竞价买价 + 板块热度 (批 1.3/2.1)
pub mod ticket_entry_proposal; // 2026-10-21: P2 入场参考 (observing → entry_proposed，复用 buy_zone)
pub mod ticket_entry_trigger; // 2026-10-23: 盘中突破确认 + 价格失效 (批 1.4/1.5)
pub mod ticket_event_sync; // 2026-10-17: 投递记账 (Delivered → ticket_event; 观察池/持仓才建票)
pub mod ticket_eviction; // 2026-10-18: 观察池淘汰 (超期/跌幅, 只管 observing)
pub mod ticket_lhb_sync; // 2026-10-24: ReviewLhb 归档 → ticket_event(kind=lhb)，只记情绪型活跃票
pub mod ticket_priority; // 2026-10-18: 观察池票优先级 (5 维加权排序)
pub mod ticket_regime; // 2026-10-23: 分型 (情绪型/趋势型) —— D1/D2 地基，写 ticket_event(kind=regime)
pub mod ticket_risk_flag; // 2026-10-23: 排雷标注 (F10 关键词, 只标注不挡买入, 批 2.7)
pub mod ticket_stage_sync; // 2026-10-18: 持仓快照驱动 stage 自动流转 (观察→持有→退出)
pub mod ticket_weekly_report; // 2026-10-24: 选股周报 (source × D+1/D+3 胜率/均值/中位/超额, 批 4.3)
pub mod ticket_watch_pool; // 2026-10-17: 观察池生成 (新闻催化; 板块轮动待接写入方)
pub mod trade_ledger; // 交易流水: 手工录入 + 快照差分推断 (2026-10-19)
pub mod trading_fact_snapshot; // 2026-10-17: op 67 价格带每日落库 (快照日有值, 错过永久丢)
pub mod watch_pool_monitor; // 2026-10-18: 观察池定向监控 (30s 异动扫描)
pub mod watch_pool_perf; // 2026-10-20: 观察池选股表现 (N 日收益/胜率)
