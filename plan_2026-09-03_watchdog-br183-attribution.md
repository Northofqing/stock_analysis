# 2026-09-03: 三件事方案 (死信哨兵 / BR-183 减负 / 归因成效卡)

**背景**: 9/1-9/3 事故序列全是同一形状 —— 数据源/门失败 → fail-closed 静默 → 用户发现「没推送」。
上游日期卡 9/1(新闻全天零)、settled_close stub 恒 Err(涨停链/龙虎榜 2 天空)、竞价 Sina 0.000(3 天零成功)、Frozen 7/15 起永续。
同时投递层建设税过重: hash 覆盖全 src + untracked 磁盘文件 → 每次改 src 重启必须重发 activation,prepare 工具 ~7 分钟纯 sha2。

---

## P0 死信哨兵 (watchdog) — 让系统监控自己

**目标**: 每个交易日,「预期该出现的推送」没出现 → 主动给用户发「我可能坏了」卡,而非等用户问。

**现状接线点**:
- 复盘调度 main.rs:6262 `[复盘调度][BR-139] threshold=19:00 interval=60s` — 已有 60s 常驻 loop 可挂检查
- 归因 main.rs:9081 15:05-15:20 重试窗(2026-08-22 修复过「失败后永不再真」bug)
- 健康推送先例: `PushKind::ReviewFailure`(复盘失败推)、`SnapshotStale`(快照过期推)
- 39 个 PushKind,无 Watchdog variant

**设计**:
1. 新表 `watchdog_deadline (family TEXT, business_date TEXT, expected_before TEXT NOT NULL, satisfied_at TEXT, fired_at TEXT, PRIMARY KEY(family, business_date))`
2. 注册点(复用已存在的调度点,每处一行):
   - 09:30 新闻首波预期 → `news_first_wave` expected 09:45
   - 15:05 归因批 → `attribution_1505` expected 15:20(注意: 15:05-15:20 已有重试,哨兵站在重试窗之后)
   - 19:00 复盘 5 路 → `review_evening` expected 19:40
   - 竞价 A-02/P-05: **先豁免**(已知预存缺陷,等上游 magic 修,见 auction-sina-zero-upstream-todo)
3. 满足条件: 该 family 当日 audit/push_analytics 有 **attempted>0**(含 RejectedDurable? → 决策 D3)即算活;否则过 expected_before 触发一次
4. 触发: `PushKind::Watchdog`(新 variant,Info 级,全链路注册: display/cooldown/DISPATCH_TABLE row/v14_adapter) → 「⚠️ 系统静默: family=X 今日无任何推送尝试,预期 19:00 前。可能: 数据源坏/门拒绝/调度未跑」;按 (family, business_date) 幂等
5. 交易日判定复用 review 既有 business_date helper

**验证**: 单测(时钟注入)+ `--test` 干跑(注入过期 expectation → 恰好一次)+ 生产首日观察
**不做**: 不做数据新鲜度全量监控(上游 magic 单独立项);不做自动重启
**估时**: 1-2 会话

---

## P1 BR-183 仪式减负 — 把部署税从「每次重启」降到「每次改配置」

**事故引用**: 改 src 忘重发 activation → NewsAggregator 被静默关(BR-183 坑,记忆多条);hash 覆盖 untracked 磁盘文件(删文件=失配陷阱);prepare 7 分钟 sha2;effective_from 必须未来 + 启动一次性评估 → 部署窗口 race(本会话 17:52 当场发生过一次 reject)。

**现状**: config_activation_v2.rs — `activation_file.expected_config_hash ↔ prepared_snapshot.config_hash`(snapshot 含 chain/board/selection);**`executable_revision.hash` 已存在**(编译期 bake)→ 二进制版本已有独立指纹,只是没拿来独立使用。

**方案(推荐 B,拍板点 D1)**:
- **门 1(保留,真 gate)**: config 域 hash —— 只覆盖真正 gate 行为的配置(selection/,其余按需显式清单),不再 glob 全 src + untracked
- **门 2(新,banner-only)**: executable_revision 比对 → 二进制 ≠ activation 记录 → 启动 WARN banner「二进制已更新,activation 未同步;config 门按 config 生效」,**不阻止启动、不静默关任何 feature**(根治「忘重发 → 静默关」)
- prepare 工具: 窄作用域 → 秒级;effective_from 仍须未来(保留,防时钟 race)
- activation 文件自身排除逻辑保留;untracked 不再计入(只走 git ls-files 或显式清单)

**风险**: 改动 activation 机制本身,部署需要一次「最后的全量仪式」;audit 清单测试要同步(dispatch/activation 相关 7/7 类测试)
**估时**: 1 会话 + 1 次仪式部署

---

## P2 归因成效卡 — 让系统每天自答「我有没有用」

**关键事实**:
- PerformanceEngine 15:05 cron **已在写** `paper_performance_snapshot` (total_pnl/win_rate/sharpe, main.rs:9028)→ 总览已有,缺「按信号源分列 + 头屏推送」
- 账本 9/1(PaperSell gate 解除)→ 9/2(买入落账)→ 9/3(买入卡)才闭环 → **未来 2-4 周是首次能算可信分策略胜率的窗口**
- 归因脚手架齐: paper_attribution_daily / paper_attribution_epoch_daily / selection_outcomes / catalyst_watchlist_outcome / pushed_stocks / stock_position

**设计**:
1. 查询: paper_trades(fills + source) × 卖出成交收据 × stock_position reconcile → 每信号族(NewsCatalyst / T0Advice / 涨停 / 龙虎榜 / R-*)汇总: N 买入 / M 已平仓 / 胜率 / 盈亏 / 平均持有天数
2. 输出挂载(拍板 D4): 15:05-15:20 归因批附推一张「信号成效卡」(新 template `signal_pnl_v1`,可走独立 PushKind 或 DailyReportSubKind)——独立 PushKind 则不受 daily_report_router 8 callsite 欠账阻塞
3. **样本不足态**: 该族 N<5 → 显示「样本积累中」,不显示胜率(防过早结论)
4. 附一行风险脚注: 实盘持仓深度(如「6 只持仓 5 只亏损 >20%」),纯展示不 gate

**估时**: 1-2 会话(含 DailyReportSubKind 迁移欠账处理与否)

---

## 顺序与依赖

- 任何 src 改动按现行规则都要一次全量仪式 → **P1 是「最后一次全量仪式」**,做完后 P0/P2 部署只走 config 门(秒级)
- 建议: **P1 + P0 同批改动、同一次仪式部署**(省一次仪式),P2 随后;或先 P0 快赢再 P1(拍板 D2)
- P0 纯 monitor 侧独立;P2 数据面独立,与 P0/P1 无耦合

## 待拍板(4 个)

| # | 决策 | 选项 |
|---|---|---|
| D1 | BR-183 改法 | A 只收窄 config 域 / **B 双门(推荐)** / C 激进去门化 |
| D2 | 批次 | **P1+P0 同批一次仪式(推荐)** / 先 P0 单独快赢 |
| D3 | 哨兵「活」口径 | 尝试>0 即活 / 仅 Delivered 算活(严,易误报) |
| D4 | 成效卡挂载 | 15:05 归因批附推 / 19:00 复盘批次头屏 |
