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

## Q-11 [P0] freshness 测试 pre-existing flake
**问题**: `freshness::tests::validate_nav_freshness_passes_recent_date` 在 2026-07-05 03:xx CST 跑失败 (断言 `validate_nav_freshness(today)`).
**根因**: freshness.rs 内有交易时段/日历判断, 凌晨时段可能落入"非交易日窗口". freshness.rs **未在本次改动中触碰** (git diff 已确认).
**当前决策**: 与本次重构无关, 不修. 已记入 docs/architecture/v12-push-uncertainty-notes.md Q-11.
**状态**: 待 freshness 模块单独修. 不阻塞 v12 推送重构合入.