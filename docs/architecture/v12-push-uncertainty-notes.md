# v12 推送重构 — 不确定点登记

> 范围: src/bin/monitor/ 下 push 重构期间，遇到的不影响本次开发的暂存疑问。
> 决议时机: 对应 PR 开工前再回头看。优先级 P0=影响模板输出正确性 / P1=影响治理规则 / P2=纯风格。

## Q-01 [P1] push_governor 签名扩展
**问题**: 当前 `push_governor(text, kind)` 不接 banner / AccountMode / DataMode。
§14.0 要求所有交易建议类第 1~2 行强制带横幅，但当前 push_governor 调用点分散在 main.rs，banner 渲染由 push_templates.rs 完成（拼进 text）。
**当前决策**: 把 banner 拼进 text 头部，由 push_templates.rs 统一负责，push_governor 签名不变。
**遗留**: push_governor 内部要做 banner-prefixed 判断（识别 text 第 1 行是否以 `[` 开头），以决定是否走"交易建议类"治理（冷却/预算）。暂定: 用 PushKind 自身来标记（新增方法 `requires_banner()`），不动 push_governor 签名。
**状态**: 待 §14.3 治理规则接入时确认。

## Q-02 [P0] AccountMode / DataMode 类型尚未存在
**问题**: §14.0 横幅需 `account_mode: {Normal, ReduceOnly, Frozen}` 和 `data_mode: {Full, Degraded, Unsafe}`。
两者在 v12-dev-plan §1.3 / §2.1 才会有正式结构体（PR1 / PR2 任务）。本次 push_templates.rs 必须定义本地轻量枚举作为模板入参，等 PR1/PR2 合入后再接真值。
**当前决策**: push_templates.rs 内定义独立 `pub enum AccountMode { Normal, ReduceOnly, Frozen }` 和 `pub enum DataMode { Full, Degraded, Unsafe }`，从 notify.rs / main.rs 暴露成 `From` 接口（PR1/PR2 时再接）。
**状态**: PR1 开工时决议。

## Q-03 [P1] T-08 候选失效的 PushKind
**问题**: v12-push-templates.md §14.1 T-08 标注 "PushKind::CandidateBoard(复用)"，但模板标题是 "📋 候选失效"。原 CandidateBoard 主用于候选台卡片（多头/排序），复用会导致 label 误导。
**当前决策**: 复用 CandidateBoard，模板渲染函数内部根据内容区分（"📋 候选触发" vs "📋 候选失效"）。
**遗留**: 如果后续 PushKind 增加过多，可拆 `CandidateTriggered` / `CandidateInvalidated` 两个变体。本次保持单变体。
**状态**: MVP-3 转正前决议。

## Q-04 [P1] banner 拼接到 push_governor 的方式
**问题**: 横幅内有 emoji + 中括号，飞书 / 微信显示宽度差异。banner 拼到 text 头部后, push_governor 没法识别"这条就是交易建议类"（治理规则 §14.3 区分日预算时需知道）。
**当前决策**: push_templates.rs 暴露 `fn render(kind: PushKind, ...) -> String`（含 banner 拼接），调用点直接传完整 string 给 push_governor。push_governor 按 PushKind 查表决定是否计入日预算（见 Q-05）。
**状态**: §14.3 治理规则接入时决议。

## Q-05 [P0] 日预算 30 条计数位置
**问题**: §14.3 治理规则 3: 交易建议类合计 ≤30 条/日（沿用 daily_important_max, notify.rs/signal_state :283）。但 daily_important_max 现状是不是按 AlertLevel::Important 计数？PushKind 没接 level。
**当前决策**: 暂不接入预算计数。本次只做模板渲染 + push_governor 复用现有冷却（PUSH_VERBOSE 保留降级）。预算逻辑留待 PR4 接入时统一。
**状态**: PR4 任务 4.3 开工时决议。

## Q-06 [P2] T-11 竞价异动复用 AuctionVolume 但模板加横幅
**问题**: 现状 AuctionVolume 是降级（默认 log）。T-11 标注 "复用现有，加横幅"。横幅 + 时间 = 交易建议类前置条件，但 AuctionVolume 本身不在交易建议范围。
**当前决策**: 横幅可在 T-11 内条件拼接（不强加）。本次先实现 T-11 函数，是否调用由主循环决定。
**状态**: MVP-4 (T-11) 开工时决议。

## Q-07 [P1] R-04 龙虎榜数据源
**问题**: R-04 模板需要"上榜原因 / 买席卖席 / 主线一致 / 次日风险"。龙虎榜数据源在 monitor 内未稳定接入。
**当前决策**: 模板函数实现为占位版本（字段填空），等数据源确认后再接真值。
**状态**: MVP-4 开工时决议。

## Q-08 [P2] R-05 / R-06 / R-07 盘后数据来源
**问题**: R-05 信号复盘 / R-06 失败归因 / R-07 明日观察池 — 都依赖 execution_tracking / performance_feedback / tomorrow_watchlist 模块，MVP-1~MVP-5 才陆续到位。
**当前决策**: 模板函数定义接口 + 签名 + 单测覆盖（用 stub 数据）。生产数据接入留到对应 MVP。
**状态**: MVP-4 / MVP-5 开工时决议。

## Q-09 [P1] dispatch() 集成测试被 env-var race 阻塞
**问题**: `push_templates::dispatch()` 集成测试需要 `V10_DRY_RUN_PUSH=1` 才能返回 true，但 cargo test 默认并行执行, 多个测试共享 `std::env` 变量, 出现间歇性失败 (`ok1=false` 但其实 V10_DRY_RUN_PUSH 已被其他测试 unset)。
**当前决策**: 把 dispatch 集成测试改为「只验证签名 + 类型」; 真集成留到 CI 用 `--test-threads=1` 或单独 bin harness 跑。
**影响**: §14.3.1/3/4 的三个 gate (cooldown / budget / mode-block) 已有独立单测覆盖 (push_templates 24 个, 全绿), dispatch 集成层只是 glue, 风险可控。
**状态**: PR1 验收前应补一个 `--test-threads=1` 串行集成测试, 或改用 `serial_test` crate。

## Q-10 [P2] notify.rs 新增 PushKind 触发 dead_code 警告
**问题**: 我加了 8 个 v12 PushKind (`AccountMode/DataMode/HoldingPlan/T0Advice/CandidateTriggered/ForbiddenOps/PaperTrade/CloseCall` + 6 个 R-* 盘后). cargo build 报 `multiple variants are never constructed` 警告.
**当前决策**: 不加 `#[allow(dead_code)]` — 警告本身是"待 PR1~5 消费"的有意信号.
**影响**: 仅警告, 不阻塞编译/测试.
**状态**: 各 PR 合入相应消费者后警告自动消失.

## Q-20 [P0] 已修复: account_mode Frozen 被数据缺失降级
**问题**: `account_mode.rs:100` 数据缺失分支早返回 ReduceOnly, 不论 prev 是什么. 与 BR-021 "Frozen 必须等下一交易日盘前重置" 矛盾.
**触发**: 盘后 ledger 写入延迟 → data_complete=false → prev=Frozen 被强制改成 ReduceOnly → T-01 推送"Normal→ReduceOnly" 假变更 → 审计污染 + 误开新仓.
**修复** (2026-07-05): 加 0 分支 `if matches!(prev, Some(Frozen))` 优先于数据缺失分支. 测试 `missing_data_does_not_override_frozen` → `missing_data_keeps_frozen`, 断言 Frozen.
**影响**: 全部账户模式审计 + T-01 推送.

## Q-21 [P0] 已修复: paper_trade.simulate 不检查 INSERT OR IGNORE 结果
**问题**: `paper_trade.rs:148` `diesel::sql_query(...).execute(...)` 返回的 rows_affected 被丢弃. 重复 plan_id 调用返 Ok 而 DB 未插行.
**触发**: 同 plan_id 第二次 simulate (例如 T+1 重评) 返回新 status (NotFilled), 但 DB 仍 Filled. execution_tracking 后续结算按 Filled 算 → 业绩归因错位.
**修复** (2026-07-05): 新增 `PaperOutcome { result, inserted: bool }`. 调用方根据 `inserted` 决定是否启动 T+1 跟踪. simulate 返回类型由 `PaperResult` 改为 `PaperOutcome`.
**影响**: paper_trades + execution_tracking + MVP-5 performance_feedback 业绩统计.

## Q-22 [P1] 已修复: query_chain_held_count 非确定性选 chain
**问题**: `position_tracker.rs:121` `SELECT chain_name ... LIMIT 1` 无 ORDER BY, SQLite 按 rowid 返回. 同 code 多 chain 时不同时间查询可能返回不同 row.
**触发**: 同 code 多主题混合持仓, 集中度告警时有时无, BR-015 真集中度被绕过.
**修复** (2026-07-05): 加 `ORDER BY buy_date DESC, id DESC LIMIT 1`, 取最新建仓的 chain.
**影响**: BR-015 集中度告警可信度. 后续 PR 接 chain 聚合 (方案 B) 时再做彻底改造.

## Q-11 [P0] freshness 测试 pre-existing flake
**问题**: `freshness::tests::validate_nav_freshness_passes_recent_date` 在 2026-07-05 03:xx CST 跑失败 (断言 `validate_nav_freshness(today)`).
**根因**: freshness.rs 内有交易时段/日历判断, 凌晨时段可能落入"非交易日窗口". freshness.rs **未在本次改动中触碰** (git diff 已确认).
**当前决策**: 与本次重构无关, 不修. 已记入 docs/architecture/v12-push-uncertainty-notes.md Q-11.
**状态**: 待 freshness 模块单独修. 不阻塞 v12 推送重构合入.

## Q-24 [P1] 已修复: monitor --review 路径未接 v12 R-01~R-08
**问题**: `run_review_only_inner` 走老的 `stock_analysis::review::report::generate_daily_report_with_ledger` 路径, v12 模板 (`render_daily_report` / `render_review_market` / `render_industry_chain` / `render_review_lhb` / `render_review_signal` / `render_review_failure` / `render_tomorrow_watch` / `render_event_calendar`) 全部未挂入.
**修复** (2026-07-05 commit 861ed64): 在 `run_review_only_inner` 末尾新增 v12 R-01~R-08 块, 包 spawn_blocking (避免 sync Diesel 在 async context panic), 真实数据 (ledger/positions/trades) 装配, R-01/R-02/R-08 真推到飞书.
**影响**: --review 模式现在能走完整 v12 盘后推送链.

## Q-25 [P1] 已修复: 持仓数据误删 (8 stocks 7 恢复)
**问题**: 跑 MVP0-A E2E 验证时 `rm -f data/stock_analysis.db*` 清空旧 DB, 误删 6/30 前累积的 7 只持仓 (华电辽能/达实智能/德展健康/利欧股份/三安光电/建业股份 + 中京电子).
**根因**: MVP0-A 设计想让 run_migrations 重建表, 但旧数据未先 backup. 应 `--move-then-init` 而非 `--rm-then-init`.
**修复** (2026-07-05 commit 861ed64): 从 `data/stock_analysis.db.bak.184151` (6/30 备份) 恢复, 旧空 DB 改名为 `.empty_<ts>.bak` 保留.
**遗留**: 合肥城建 (002208) 在 6/9 就已平仓, 不在 6/30 备份里, 仍缺失 (符合历史).
**教训**: 后续 DB 测试严禁 `rm -f data/*.db*`, 必须先 backup.

## Q-26 [P2] monitor_loop T-02 钩子每分钟跑
**问题**: `monitor_loop` 内每个 tick 都调 `evaluate_data_mode_hook(None)`, 但 1 分钟粒度 + DataMode 状态基本不变 → 99% 都被 `is_changed()` 拦截不推.
**影响**: 微小性能开销, 但无功能问题.
**建议**: 改为 5 分钟粒度 (用 `Instant::now()` 计时), 状态变更时立即推 (不等间隔).

## Q-27 [P2] R-03/R-04/R-06 真实数据源待接
**问题**: R-03 涨停产业链需要拉 60 日 K 线 + 计算 board_level (周日数据空, 仅 log). R-04 龙虎榜需要 lhb_daily 表真实数据 (周日表空, 仅 log). R-06 失败归因需要 execution_tracking 表真实数据 (目前空, 仅 log).
**根因**: v12 §13 MVP-3/4/5 部分模块 (execution_tracking / lhb_daily) 数据积累需真实盘后落库, 当前表空.
**影响**: 仅周日 + 数据空, 不影响生产. 真实盘后会逐步填充.
**建议**: 周一盘后跑 monitor --review, 验证 R-03/R-04/R-06 真实数据接入.