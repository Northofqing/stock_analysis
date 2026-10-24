# 单票完整生命周期（ticket）实施方案

状态：进行中（2026-10-17 起草）
依据：operator 2026-10-17 给出的五条诉求 + 当日代码/数据实测
定位：这是**方案**，不是承诺。每阶段都写明"能验证什么"和"做不到什么"。

## 演化史

- 2026-10-17：起草，定 P1~P6 阶段与顺序原则。
- 2026-10-18：定位 P4 停供真因（schema v10 表缺失 + `{e}` 空渲染），探针法确立；观察池定向监控、竞价观测、淘汰规则上线。
- 2026-10-19：P1.1 调度上线；P3.1/P3.3、T+1 禁卖、P5 轮动信号与选票入口、P6 二层归因（CLI）完成；HoldingPlan 恢复投递。
- 2026-10-20：观察池选股表现存档（perf_log 16:05）上线——**本文件此前漏记，2026-10-22 补**。
- 2026-10-21：选票入口扩到 4 个（+P01 +持仓）；板块选票优先级（涨停>龙头>涨幅）；P1.4 P01 自动开票（推荐价=昨收）；P2 可买价完成；下游 stage 查询同步修（buy_zone_dispatch.rs:304 / ticket_eviction.rs:98）。
- 2026-10-22：P-02 stage 按时间切换（c7102301c）；`no_last_traded_price` 隔离重试（e014bb75f，未经过真实流量）；第一轮全面评审（见「调整记录」）。
- 2026-10-23：批 0 入库（eddb6d49e）；批 1 分型体系全部落地 + 批 2 七项接线 + 批 4.1/4.2 口径修正（621ea15cf，见「1.7 实施记录」）。
- 2026-10-24：批 2 补 2.5 概念扩展、2.6 龙虎榜归档接线；批 5 两项探源出结论（5.1 商品源可用、5.2 提案零成交）；2.4 探针证明降级不成立。
- 2026-10-24（下午）：日线改走**通达信盘后文件**（`tdx_day_import`），6,187,698 行进库、覆盖 5,229 只、2021-08 起（0.11）；桥内补齐 Tencent 7 字段并重启（见 0.12）。

## 调整记录

### 2026-10-22 第一轮全面评审（代码 + 数据对账）

对账方法：`scripts/phase_acceptance.py` + 生产库直查 + 日志字节锚点 + 回测。

**A. 状态修正（文档与实际不符）**

| 项 | 原文 | 实际 | 处理 |
|---|---|---|---|
| P6 归因 benchmark | 已完成 | CLI-only，main.rs 零调度，表停 10-16 缺 3 个交易日 | 降级为未达标，待接调度 |
| 关联公司 | 未做 | 15:52 作业每日在跑（花 DeepSeek 费用），产出 object_code 153/153 全 NULL 不可用 | 改为「产出不可用」 |
| ticket_priority | 未提 | 15:44 每日计算但只落日志，无表无推送无消费者 | 记入缺陷清单 |
| perf_log | 未提 | 16:05 每日 D+1/D+3/D+5 存档，是唯一在跑的选股效果跟踪 | 补记（演化史） |

**B. 缺陷立案（待修，按优先级）**

1. 同 code 多活跃票无防护：002696/002724/603278 各 2 张活跃；find_active_ticket 只取最新 → 旧票逃逸淘汰规则成孤儿；明日计划查询不按 code 去重。
2. entry_proposed 价位一次性：propose 只扫 observing（ticket_entry_proposal.rs:96），32 张票价位冻结在 10-21；归因锚定价口径未定。
3. 提案与淘汰规则矛盾：603096 同日收到买入提案（18.71-19.08）与淘汰（-22.6%）。
4. position 事件幂等缺失：603915「新建仓 500 股」重复写 6 次。
5. 明日计划绕过投递库直推：无 L5 门禁、无审计归档、P1.2 永远记不到它。
6. P3.3 行情断时空转刷 WARN（88 次）无升级出口；P3.1 新闻事件 0 条，该路径从未被真实数据验证。
7. 板块资金流信号链断裂（分三段，实证）：
   ① 盘中推送链一直活——2026-10-20 11:27 Delivered「机器人概念 主力+65.95 亿」；
   ② 盘后 f62 落库 2026-10-21 才通（10-16/19/20 只有 f3 涨幅榜，f62 零行；
   10-21 起 f62 有真实数据，当日 top=电子 +58.35 亿）；
   ③ 消费端零接线——观察池选股只用 f3 涨幅排名，f62 资金流 top 板块不进任何选股/复盘/thesis。
   后果实证：10-20 资金流第 1 名的机器人概念因涨幅排名第 11 差一名未进选股，两信号无交叉。
   历史回填说明：10-21 之前的资金流只能从投递归档 rendered_content 解码补，f62 表补不了。
8. **全市场日线自 10-17 起断更（正在发生）。** stock_daily 每日行数：10-16 = 1248 只，
   10-19 = 54、10-20 = 53、10-21 = 47——只剩观察池/持仓/涨停池碰过的票。
   连锁后果：① 板块轮动选票的「上一完整交易日」判定退化为 `HAVING COUNT(*)>=50`
   （ticket_watch_pool.rs:228），54 行技术过线但候选面塌缩——10-21 实锤 boards=30
   候选只有 7 只（30 板块×3 只应约 90）；② attribution_benchmark 的市场基准
   （需样本≥50）从此不可信；③ perf_log 的胜率没有同日基准可对照，超额算不出；
   ④ 新进池票若日线未刷新，P2 提案 no_data 跳过。修复：补 daily_bars_build 全市场
   10-17 至今，并把全市场日线纳入调度或加覆盖度告警。
9. **P1.1 观察池判重用进程内 static（WATCH_POOL_LAST），重启即重放。** 10-21 同一晚跑了
   3 轮，catalyst 事件 24 条 = P01 3 条 + 轮动 7 只×3 轮——同一批票的「首次入池」事件
   重复写 3 遍。明日计划在 10-20 已因同款教训改用文件 marker，P1.1 没跟着改。
10. **涨停阈值 9.85 不分板。** 20cm 板（300/301/688）涨 9.85~19.5% 不是涨停，但被
    pct_chg>=9.85 计为涨停：实测 151 个样本 / 93 只票（占 20cm 板全部 >=9.85 样本的
    58%）。同一阈值同时用于：分型规则（近 5 日有涨停）、P1.1 涨停优先选票、两块回测。
    主板（002/600/603/000/605/001/601）无此问题（>=19.5% 为 0）。
11. **perf_log 基准口径不统一。** 16:05 存档统一用 open_date 收盘作基准
    （watch_pool_perf.rs:11）——对 15:40 开票的观察池票合理（次日才能买），
    但 P01 票 09:00 开票、当日可买，D+1 指标系统性地晚算一天。
12. **D3 预算层只有总量没有集中度。** 30%/周 限总额，不限同板块集中度——轮动入口的票
    天然同板块扎堆（10-19/20 批次 34 只全是板块来源），同时触发时预算会全部压进一个板块。

**C. 运营决策（operator 拍板）**

| 编号 | 决策 | 结果 |
|---|---|---|
| D1 | 情绪型票允许突破买，超买门对情绪型豁免 | 通过 |
| D2 | 持股周期不定全局值，按票的趋势分型（每日重算，型变参数变） | 通过 |
| D3 | 组合预算 = 单周新增仓位上限 + 优先级排序花钱，只标注不拦截 | 通过，30% |
| Q2 | 情绪票入场价 | 竞价价 + 盘中突破确认，两者都给 |

**D. 数据否决（回测推翻原提议）**

原拟「竞价低开 >2% 放弃」→ 全库 1977 个涨停次日样本回测否决：
低开 >2% 子集是唯一正期望分组（开盘买→收盘 +1.16%、胜率 47.7%、n=304）；
高开 >6% 才是死亡区（-1.97%、胜率 28.4%、n=338）。
**改为：不设低开放弃线；高开帽 >6% 不给买价。**

**E. 回测证实**

- 回踩模型对情绪票不适用：涨停次日盘中触及 ma10(±1%) 仅 15.4%（221/1436）。
- 10-21 实发 33 个提案中 18 个（55%）按新分型是情绪型——发的是统计上不会成交的价。
- 竞价/开盘买情绪票裸基线为负期望（均值 -0.36%，n=1977；连板 -0.40%/首板 -0.35%）：
  D1 解决「价能成交」，不保证「成交能赚」，正期望要靠高开帽 + 板块热度过滤的子集规则，
  该规则上线前无数据支持，先观察记录两周。
- 边界：样本来自 stock_daily（约 1200+ 只），生存者偏差未排除；竞价维度用开盘价近似，
  历史竞价数据从未落库。

**F. 拓展立项**

见 8.5：拓 1.1 / 1.3 / 2.1 / 2.3 / 3.1 / 4.1 / 5.1 / 5.2 / 6 可直接排期；
拓 2.2 / 3.2 需先探源或定口径。实施批次：批 0 修复 B.1/B.5 → 批 1 分型体系（Q2 已拍）
→ 批 2 纯接线拓展 → 批 3 预算层（Q3 已拍）→ 批 4 探源。可复用资产盘点见 8.6。

## 一、目标（operator 原话拆解）

1. 以周为周期（按板块轮动而非固定周），周末/周初用新闻催化 + 上周板块轮动迹象，
   在周一开盘与集合竞价找出热点板块最热的票，给出**可买入价格**；T+1 当天不考虑卖出。
2. 盘中密切关注该票：新闻、盘中数据、公司公告，以及**直接/间接关联公司**。
3. 次日集合竞价 + 盘中，判断**持有 / 加仓 / 做 T**，且每个动作要有**明确价格**。
4. 同时关注板块轮动，准备下一个板块的股票。
5. 无论盈利亏损都要有**复盘归因**：为什么赚、如何继续；为什么亏、如何避免。

## 二、现状（2026-10-17 实测，不含推测）

已完成（地基）：

- `ticket_lifecycle` / `ticket_event` 两张表 + `(code, open_date)` 唯一索引。
- `stage` 为 TEXT 无 CHECK 约束（枚举可演进）。
- 手工工具：`bin ticket_open` / `bin ticket_show`。
- 观察池生成器：`monitor/ticket_watch_pool.rs`（新闻催化路），`bin watch_pool_build --dry-run`。
- op 67 解析层：`data_gateway/provider_observed_trading_facts.rs`（8 单测）+ 请求构造
  `build_external_native_query_request::QualifiedTradingFacts`。

未接（缺口，按实测）：

- **monitor 零 ticket 调度**：`grep ticket src/bin/monitor/main.rs` = 0 处。
- **推送 → ticket_event 零接线**：`append_event` 只有观察池生成器与手工 CLI 调用。
- **生产库 0 行**：ticket_lifecycle 0 / ticket_event 0。
- **无"可买价"**：`PoolCandidate` 只有 code / note，没有价格字段。
- **板块轮动路未接**：`save_board_rotations` 零生产调用，`board_rotation_daily` 停在 2026-07-16。
- **关联公司未做**（直接/间接都没有）。
- **HoldingPlan 停供**（2026-09-23 起 owner 读取拒绝）。
- **逐票归因未做**：`advice_outcome_tracking` 只接 T0Advice 一种 push_kind。

## 三、阶段设计

### P1 让票真的被跟踪（一切的前置）

P1.1 monitor 每日调度观察池 — **已完成（2026-10-19 上线，10-21/10-22 两轮加固）**

- 位置：`src/bin/monitor/main.rs::market_loop`，按 `static X_LAST: Mutex<Option<NaiveDate>>` 模式。
- 时机：交易日 15:40 之后（盘后，新闻评估已产出），每日一次。
- 动作：`ticket_watch_pool::build_watch_pool(db, date, lookback=5, dry_run=false)`。
- **四个选票入口（2026-10-21 扩到 4 个）**：

  | 入口 | source | 数据来源 | 状态 |
  |---|---|---|---|
  | 新闻催化 | `news_catalyst` | `news_ai_assessment` | **断供**（停 10-14，见已知约束 6.2） |
  | 板块轮动 | `board_rotation` | 当日板块榜 top10 的成分股 | 已通 |
  | P01 盘前热点 | `p01_preopen` | `LimitPools`（前一交易日涨停池） | 已通 |
  | 实际持仓 | `holding` | `stock_position` | 已通 |

- **板块轮动的选票优先级（2026-10-21 改，`8b41528ae` + `7e1f13b3f`）**：

  ```
  今日涨停 > 板块龙头 > 日线涨停 > 涨幅降序 > 涨幅缺失（BR-222 不补 0）
  ```

  - `LIMIT_UP_PCT = 9.85`，用 `pct_chg` 判涨停，**不依赖 `stock_daily.is_limit_up`**
    （该列在 `kline.rs:650` / `repository.rs:92,254` 硬编码 false，不可信）。
  - 板块龙头取实时 `BoardFlows.leader_code`，不落库（避免触发 BR-159 验链）。
  - **板块成分股必须用东财 BK 码**：`board_ranking_daily`（东财 `BK1512 生猪养殖`）与
    `board_member`（TDX `tdx:industry:融资融券`）**名字和代码交集都是 0**，语义也不同
    （行业细分 vs 风格题材）。所以 `BoardConstituents` 按 BK 码走 ExternalV1 通道。
- 验证：日志 `[观察池] boards=N board_candidates=M board_added=K total=T opened=O appended=A`。
  **`opened=0` 不等于失败** —— 选中的票已在库里时会走 `appended` 补事件路径。
  判"有没有产出"看 `board_candidates > 0`。
- 做不到：输入源断供时（`news_ai_assessment` 无新增）只会开存量票，日志必须如实报 0。

P1.2 推送 → ticket_event 自动记账（best-effort）

- 位置：T0Advice / HoldingPlan / CloseCall 推送成功后（durable delivery 已确认 Delivered）。
- 动作：`append_event(ticket_id, event_at, event_kind, push_kind, occurrence_identity, summary)`。
- 约束：**best-effort**——写失败只 `log::warn`，绝不影响推送；票不存在则跳过（不自动开票）。
- 验证：推一条 T0Advice 后，对应 ticket 的事件时间轴多一行。
- 做不到：只覆盖"有 ticket 的票"；没有 ticket 的推送不回溯补建。

P1.3 op 67 价格带每日落库

- 原因：对端明确"价格带只在观测快照日有值"，错过当天就永久丢失（不能用时再查）。
- 动作：每交易日盘后查一次 `QualifiedTradingFacts`（include=lifecycle+price_band），
  落 `stock_daily` 或独立表（字段：code / date / limit_up / limit_down / price_band_source / evidence）。
- 验证：表中每个交易日的行数 = 当日查询成功的票数；`price_band_source` 非空。
- 做不到：非快照日返回 null（按 BR-222 保持 NULL，不补值）。
- **前置**：op 67 当前只有请求构造，没有业务调用点；本项即第一个调用点。

### P1.4 P01 推送 → 自动开票（第三个开票入口，2026-10-21 新增）

- 原因：P01「盘前热点」每天 09:00 推 3 只昨日涨停股，但 P1.2 的记账范围只有
  `T0Advice / HoldingPlan / CloseCall`，且明确「票不存在则跳过（不自动开票）」。
  结果是推完就断——不进观察池、不留推荐价、无法算 D+1/D+3，也无法验证选股是否有效。
  实测 10-21 推的 002696/002724 当日续板，但库里没有任何记录能追溯这次推荐。
- 动作：`run_p01_once_with_ports` 在 `P01RunOutcome::Delivered` 之后调用
  `record_p01_tickets`，对每只入选票 `open_ticket(stage='observing', source='p01_preopen')`
  + `append_event(kind='catalyst', push_kind='PreopenNewsHot')`。
- 记账字段：`P01BoundHead` 新增 `recommend_price / streak / sealed_amount / turnover / break_count`，
  在 `bind_p01_sources` 里按 code 从 `LimitPools` 的 `LimitPoolEntry` 取。
  **这些字段不进 `canonical_source_bytes`**，所以不影响 source binding hash 和 decision identity。
- 推荐价口径：**前一交易日收盘价**。P01 在 09:00 推送，集合竞价尚未结束，拿不到当日实时价，
  涨停池的 `price` 是唯一可锚定的价格。note 里写明「推荐价」来源，避免被当成当日成交价。
- 约束：
  - best-effort：取连接/写库失败只 `log::warn`，绝不回滚已完成的推送
  - `open_ticket` 是 `INSERT OR IGNORE` + `(code, open_date)` 唯一索引，同票同日重复推送不产生第二张票
  - 票已有活跃 ticket 时（如 003029 由板块轮动先开），只补事件，不新开
- 验证：日志出现 `[P-01][bookkeeping] opened ticket#N <code> <name> price=Some(..)`；
  `SELECT COUNT(*) FROM ticket_lifecycle WHERE source='p01_preopen'` 增长。
- 做不到：**历史回填拿不到推荐价**——`delivery_decisions.envelope` 里只有
  `ordered_limit_pool_record_hashes`，没有价格字段。回填脚本
  (`backfill_p01_tickets.py`) 只能写 NULL，按 BR-222 不补推导值。
  另：2026-08 ~ 2026-10-09 的 15 个业务日推送正文格式不同，解析不出股票代码，未回填。

### P2 可买价（观察池 → 入场参考）— **已完成（2026-10-21，`32628135f`）**

- 实现：`src/monitor/ticket_entry_proposal.rs::propose_for_observing`
  **复用 `buy_zone::compute`**，不另写价位算法。
- 输出：**参考区间 + 触发条件**，不是单一价格。实测产出例：
  `22.82-23.28｜回踩 ma10 23.05 附近可买；确认条件：量比>1.2｜放弃 19.34`
- 分型给区间（按 `buy_zone` 的均线判型）：
  - **多头**：`entry = ma10 ±1%`，放弃价 `max(ma20, 20日低) × 0.99`
  - **空头 / 纠缠**：上沿 `min(close×1.01, ma5)`，下沿不低于 20 日低
  - **已在 20 日低附近**：只报放弃价，不报买入区间
- 跳过规则（不推进，写日志说明原因，**不编区间**）：
  - 连板 / 近 5 日涨幅过大 / 偏离 ma5 超买 → `skipped_overheat`
  - 数据不足（无 `stock_daily`）→ `skipped_no_data`（BR-222）
  - 已在支撑位 → `skipped_at_support`
  - 同日同票已有 `entry_proposed` → `skipped_duplicate`
- 写入：`ticket_event(kind=entry_proposed)` + `ticket_lifecycle.stage='observing' → 'entry_proposed'`。
- **下游必须同步改 stage 查询**（2026-10-21 修）：
  `buy_zone_dispatch.rs:304` 与 `ticket_eviction.rs:98` 由 `stage = 'observing'`
  改为 `stage IN ('observing','entry_proposed')`，否则票推进后从明日计划消失、且永不淘汰。
- 验证：日志 `[入场参考] scanned=N proposed=P overheated=O at_support=S no_data=D duplicate=U errors=E`；
  `SELECT COUNT(*) FROM ticket_event WHERE event_kind='entry_proposed'` 增长。
- 实测（2026-10-22 10:24 验收）：`entry_proposed` 事件 33 个、状态 32 个，`observing` 从 40 降到 10。
- 做不到：不预测涨停；无 `stock_daily` 覆盖的票不给区间（只标注"数据不足"）。
  P01 回填那批（603096/603533/605058/002343/600825/000530/002696/002724）就是这一类，
  它们的 `observing` 会一直停着，直到补齐日线。

### P3 定向监控（观察池的票）

- **P3.1 新闻/公告按 ticket 的 code 过滤 — 已完成（2026-10-19）**
  - 盘后回溯（`main.rs::post_close_news_review`）：标的从「只持仓」扩到
    「持仓 ∪ `ticket_lifecycle` 活跃票」。以前观察池的票根本没有新闻数据，
    优先级打分里的「新闻催化」维永远 0 分。
  - 盘中扫描（`watch_pool_monitor::scan`）：新增第三条规则——查 `news_items`
    最近 10 分钟、未被 `news_dedup` 标记的新闻，命中就报「新闻：{标题}」。
  - 去重：通知后写 `news_dedup` key=`watch_news:{id}`，30 秒一轮不会重复。
  - 边界：新闻票必须在行情批次里才报（拿不到价格就没有意义）。
- **P3.3 盘中同板块关联票异动 — 已完成（2026-10-19）**
  - `watch_pool_monitor::scan` 第四条规则：观察池的票属于哪些板块（`board_member`），
    这些板块里今天涨幅 >= 5% 的其他票就报，文案「同板块「风电」000099 XX +6.2%」。
  - 每个板块只保留涨幅最高的一只，挂在主票的 `reason` 里，不单独成条。
  - 阈值 5% 而非 3%：板块内每天一堆 3% 的票，报出来是噪音。
  - 单轮最多查 200 只关联票行情（`PEER_QUERY_LIMIT`），保护行情接口。
- **关联公司（直接/间接）仍然做不到，原因是数据不是代码**：
  `chain_relation_candidate` 77 条记录 `object_code` **全部为 NULL**，
  `object_name` 是「钢材」「原料药」「航空煤油」这类产业环节——`chain_ai` 的
  系统提示词明确要求写「具体的大宗商品或原材料」而不是公司（理由：只有具体到
  商品才能盯价格）。所以这一路盯的是**商品价格**，不是关联公司股价。
  要做「直接关联公司」得改提示词让它输出上市公司代码，或接股权/供应链数据源。
- **T+1 当日买入禁卖 — 已完成（2026-10-19）**
  - `position_change_detect::t1_locked_quantities`：当前快照 vs 上一份的
    `quantity` 增量 = 当日买入股数（系统拿不到券商「可用数量」字段，
    `UserPositionItemInput` 只有 quantity / cost_price）。
  - `holding_plan`：整仓都是当日买入时，浮盈 > 5% 不给减仓，降级 `Hold`，
    文案「浮盈 X% 但当日买入 N 股 T+1 锁定，今日不可卖」。
  - 读锁定量失败时 `warn` 并出声，不静默退化成「可以卖」。
  - 边界：两次快照之间买入的都算锁定；隔多日才导入一次会多锁一天（偏保守）。
  - 只挡 `Reduce`，不挡 `StopLossReview`（止损是核查提示，不是卖出指令）。
- 验证：某票当天新闻命中时，事件时间轴出现 `news_catalyst` / `announcement` 行；
  持仓票在买入当日浮盈 >5% 时，HoldingPlan 应输出「T+1 锁定」而不是减仓价。

### P4 次日动作 + 价格

- 路径 A（推荐）：修 HoldingPlan（真因已有观测，待盘中 tick）→ 复用其动作与价格。
- 路径 B（备选）：`ticket` 专属次日决策模块，复用 T0Advice 的价位计算，但**不新造一套风控**。
- 验证：次日 9:25-9:35 对观察池票输出 `hold/add/t0` + 价位；写入 ticket_event。
- 做不到：HoldingPlan 未修好前，"减仓/止损"类结论只有 T0Advice 一条腿。

#### 排查记录（2026-10-18）

**现象**：2026-09-23 09:46 之后，每日 5 只持仓票全部
`[I-04][T-03] code=xxx 原 owner 读取拒绝: holding_plan_owner_read_failed`，
HoldingPlan 决策不再写入。

**证据**：

```
delivery_decisions  HoldingPlan 按日：
  2026-09-21  290
  2026-09-22    5
  2026-09-23    5     ← 全部 Delivered，时间 09:46-09:47（北京时间）
  之后          0

state 分布：RejectedDurable 2859 / Delivered 161 / ManualResolvedRejected 6
```

错误消息只有前缀，`{e}` 渲染为空 —— 底层错误没有 Display 文本。

**已排除**（逐条实测）：

| 假设 | 证据 | 结论 |
|---|---|---|
| 表缺失 | `immutable_audit_outbox` / `delivery_disposition_payloads` / `task_transition_payloads` 都在 `durable_delivery.sqlite3`（19 张表） | ✗ |
| 多进程争用 shm | `lsof` 显示只有 monitor 打开该库（2 main + 3 shm fd） | ✗ |
| schema 版本不符 | `PRAGMA user_version=9`，代码用 `open_existing_monitor_schema9`；启动时 catalog 校验通过 | ✗ |
| `OccurrenceContext` 校验 | 该函数各分支错误都有具体消息（`ticket_not_equity` / `ticket_code_invalid`） | ✗ |
| 当日无决策导致失败 | `load_strict_owner` 无记录时返回 `Ok(None)` → `Missing`，走 fresh 分支，不报错 | ✗ |
| database_binding 缺失 | 只在 teardown 时为 `None`；进程正常运行是 `Some` | ✗ |

**待定**：真实错误在 `with_connection` 或 `capture_sql_binding` 内部，
但 Display 为空，无法从日志判断。

**已做**：把 5 处 `format!("...: {e}")` 改成 `{e:#}`（anyhow alternate 输出完整因果链），
位于 `src/bin/monitor/durable_delivery_runtime/holding_plan.rs:40,65,102,204,235`。

**验证时点**：周一 10:30 I-04 首次触发时，日志会带完整错误链。
拿到真实错误后按错误类型修（结构问题改 SQL/校验；状态问题改流程）。

### P5 板块轮动接力

- **P5 轮动信号 — 代码完成（2026-10-19），等数据**
  - `src/monitor/board_rotation_signal.rs` + CLI `board_rotation_build`
  - 表 `board_rotation_signal`（不动旧 `board_rotation_daily`，
    后者要求 `news_title` 非空，硬塞空串等于造假）
  - 三态：`mainline`（连续在 top10）/ `new`（今天新进 top10）/ `fading`（昨天在、今天掉出）
  - 判定用**名次**不用涨跌幅：涨幅可能因复权/口径差异不可比
  - 单日保护：`has_comparable_days` 检查，只有一个交易日时直接报错退出
  - **2026-10-16 实测**：报「之前没有第二个交易日的板块榜，无法判断轮动」✓
  - **数据缺口**：`board_ranking_daily` 目前只有 10-16 一天真实数据。
    2026-10-18 那 175 行是周日手工跑 CLI 写入的 10-16 副本（逐行相同），已删除。
- **P5 选票入口 — 已接通（2026-10-19）**
  - 桥接 `BoardConstituents` 新增 `board_code` 口径。此前 `fetch_board_constituents`
    只能调 `gateway.memberships(code)`，方向是「个股→板块」；provider 的
    `BoardConstituentRequest { board_code, limit }` 本来就支持按板块查成分，但
    `BoardDataGateway` 没有公开入口（`board_constituents_raw` 是 `pub(super)`，
    `board.rs::board_constituents` 要 `VerifiedBoardBinding`，而 binding 是空列表）。
    改法：`board_runtime::constituents_by_board()` 复用 `board_constituents_raw`
    + 与 `fetch_memberships` 同一套 evidence 校验（改校验返回的 `board_code` 一致），
    `delegate::fetch_board_constituents` 优先读 `board_code`，保留 `codes` 兼容路径。
    提交在 `.worktrees/tdx-local-provider-20260923` 的 `479e761d2`。
  - 新表 `board_member(board_code, board_name, kind, instrument_code)`，
    CLI `board_member_build --from-ranking --date <日> --top 10`。板块名先试
    `tdx:concept:<名>`，失败再试 `tdx:industry:<名>`（新浪概念榜与 TDX 分类不同）。
  - 观察池 `ticket_watch_pool::collect_board_rotation` 接第二路：当日板块榜 top10 →
    `board_member` 成分股 → 按**上一完整交易日**涨幅取每板块前 3（`BOARD_PER_BOARD=3`）。
    用上一交易日而非当日：`stock_daily` 当日只有活跃票样本（10-19 仅 9 只），
    拿它排序会把 1241 只里的强票挡在外面。
  - monitor 15:40 调度：先 `build_from_ranking(10, 10_000)` 再 `build_watch_pool`。
  - **2026-10-19 实测**：top10 板块里 5 个成功、5 个报「服务端内部错误」
    （海上丝路/海工装备/氢燃料/军工航天/抗癌）；成功的是国产软件 261、风电 400、
    仿制药 73、民营医院 78、草甘膦 17 只，共 829 行。观察池 16 候选
    （2 新闻 + 14 板块），开票 14 只 `source=board_rotation`。
  - **已知限制**：约一半新浪概念名在 TDX 板块里对不上（返回内部错误而非空），
    目前只能记 `failures` 跳过。板块覆盖不全 = 选票面偏窄。
  - 桥接重启后**首次监听要等约 140–260 秒**（`DatabaseManager::init` 跑 migrations），
    这段时间内所有数据调用报 `unavailable`。重启桥接要留足观察窗口，
    不要在这个窗口里判定启动失败。
- 每日产出"下一个板块"的候选，进观察池（source=board_rotation）—— 已接。
- 验证：`board_rotation_build --date <日>` 输出「主线/接力/退潮」各几个板块；
  `watch_pool_build --date <日> --dry-run` 能看到 `[board_rotation]` 行。
- 做不到：板块轮动的"早期信号"本质滞后；只能做提示，不能当买入依据。

### P6 逐票复盘归因

- **P6 二层归因 — 已完成（2026-10-19）**：`src/monitor/attribution_benchmark.rs`
  + CLI `attribution_benchmark_build`，落表 `attribution_benchmark_daily`。
  - `stock_pct`    `stock_daily.pct_chg`
  - `market_pct`   当日全市场等权平均，**样本 >= 50 才采纳**，否则 NULL（BR-222）
  - `excess_pct`   `stock_pct - market_pct`，即选股超额
  - `board_pct`    **留 NULL**：板块日线只有 2026-10-16 一天（新浪概念快照），
    D3 板块成分映射未落地，填一个估出来的值比 NULL 更糟。
  - 实测 2026-10-09：817 只全部带基准，市场 +0.69%，
    300413 超额 +19.29 / 300340 +15.20 / 688561 +10.52。
  - 实测 2026-10-16：18 只，样本不足 → `market_pct` / `excess_pct` 全 NULL。
  - 补样本：`daily_bars_build --codes` 可显式指定标的，
    `stock_concepts` 里有 1265 个代码，补完全市场后基准即可采纳。
- 已有的 `paper_attribution_daily` 是**按 signal_family 统计 PnL/胜率**，
  停在 2026-08-28、`attribution_run_audit` 0 行，与本轮的二层拆分不冲突。
- 验证：`attribution_benchmark_build --date <日> --show` 能看到个股/市场/超额三列。
- 做不到：归因是统计相关，不是因果；报告必须写明"假设执行、非真实收益"。

## 四、顺序原则

1. **P1 必须最先**——没有自动化，后面每个阶段都只有 dry-run 能跑。
2. **P1.3 与 P1.1 并行**——价格带错过当天永久丢，时间敏感。
3. **P2 依赖 P1.1 的票**，P4 依赖 P1.2 的时间轴，P6 依赖 P4 的动作记录。
4. **P5 可独立**——不依赖 ticket，是另一个数据源接线。
5. **P3 的弱关联版可以晚做**——它有替代品（人看新闻），P2/P4 没有替代品。

## 五、明确不做

- 不自动改规则/不自动调参（AGENTS §2.10、`performance_feedback.rs:4`）。
- 不补 0、不推导涨跌停价（BR-222 / BR-205）。
- 不给"保证盈利"的单一买价；只给区间 + 触发条件。
- 不做股权穿透式的关联公司分析（无数据源）。

## 六、P4 停供真因（2026-10-18 定位并修复）

### 现象

`HoldingPlan` 自 2026-09-23 09:47 之后 0 决策，累计 103 次：

```
[I-04][T-03] code=xxx 原 owner 读取拒绝: holding_plan_owner_read_failed
```

`{e}` 渲染为空，看不到真实原因。

### 真因

`src/durable_delivery/coordinator_holding_plan.rs` 的 `authority_sha()` 遍历
`AUTHORITY_TABLES`（17 张表）时**无条件** `SELECT rowid,* FROM {table}`：

```rust
for table in AUTHORITY_TABLES {
    let sql = format!("SELECT rowid,* FROM {table} WHERE ...");
    let mut query = c.prepare(&sql)?;   // ← 表不存在就到这里炸
```

而第 8 张 `delivery_correlation_observations` 是 **schema v10 才加的**表
（`schema.rs:357`，注释写着「Correlation is an audit edge, not a delivery authority」）。

monitor 用 `open_existing_monitor_schema9` 打开生产库，生产库 `user_version=9`，
**没有这张表** → `prepare` 报 `no such table` → 被上层包装成
`holding_plan_owner_read_failed`。

### 为什么之前排查没找到

`open_existing_monitor_schema9` 的 `monitor_schema9::verify()` **不检查这张表**
（它只校验 v9 的表），所以 coordinator 能正常打开、日志显示一切正常，
只有在真正读 occurrence 时才炸。而且错误被 `{e}` 空渲染吞掉了。

### 怎么提前定位的（不用等交易时段）

写了一个探针 `src/bin/holding_plan_probe.rs`：

- `inspect_holding_plan_occurrence` 是 lib pub；
- `CoordinatorConfig::production()` 接受任意 owner identity
  （不匹配的 owner 走 `Ok(None)` → `Missing`，不是错误）；
- 所以任意时刻都能复现同一条读取路径，不必等 10:30 的 I-04。

第一次跑就拿到：

```
sqlite durable-delivery failure: no such table: delivery_correlation_observations
```

### 修复

`authority_sha()` 里对缺失表跳过，并 frame 一个 `"absent"` 标记：

```rust
if !table_exists(c, table)? {
    frame(&mut h, b"absent");
    continue;
}
```

理由：schema.rs 自己写明这张表只是审计边、不是投递权威，缺失合法；
frame `"absent"` 保证哈希仍随 schema 状态变化（表被创建后哈希会变，
`binding.validate` 会正确报 `exact_owner_sql_witness_changed`）。

其他 16 张表缺失仍然报错（fail-closed 不变）。

### 验证

```
$ holding_plan_probe 601811 2026-10-16
[probe] coordinator 已打开
[probe] OK: Missing        ← 修复前是 no such table
```

### 未验证的部分

- 生产库（v9）上的真实读取：要等运行根部署后跑 probe 确认。
- `Owned` 分支从未被测到过（生产库里 9-23 之前的 HoldingPlan 记录也不会有
  correlation 观测，所以历史决策读出来都是 `Missing`）。
- 修复后能不能真的产生新决策：要等下一个交易日的 I-04 触发。

## 七、已知约束（2026-10-22 补，此前散落在代码注释和排查记录里）

这一节记的不是"待做项"，是**已经确认的系统性约束**。每条都改变了某段代码的正确写法，
但之前没有写进任何文档，导致同一类问题反复被当成新故障排查。

### 7.1 DataMode 是全局的，L5 用它判所有推送 —— 会误伤不依赖行情的推送

```
push_l5/governance.rs:181
  ctx.data_mode > profile.data_mode_min → Deny("data_quality")

v14_adapter.rs:1515  is_live_trading_kind() 里的 16 个 PushKind → data_mode_min = Degraded
                     其余 → Down
```

`DataMode` 由**全局能力集**决定（`data_mode.rs`：Quote staleness > 120s → Unsafe），
但**推送的数据来源各不相同**：

| 推送 | 实际数据源 | L5 依据 |
|---|---|---|
| P-02 竞价量能 | op 61 `CurrentAuctionObservations` | 全局 DataMode |
| HoldingPlan / T0Advice | `RealtimeQuotes` | 全局 DataMode |
| 收盘总结 / 复盘 | 日线 + 账户快照 | 全局 DataMode |

2026-10-22 实测：op 61 `outcome=available accepted=50`，数据完全正常，但因为
`RealtimeQuotes` 失败导致 `DataMode=Unsafe`，P-02 被 `Deny(data_quality)` 拒推。

**结论**：L5 的 `data_mode_min` 与推送真实数据源之间没有映射关系。
要修有两步：短期把不依赖实时报价的 kind 移出 `is_live_trading_kind`；
长期建 `PushKind → 所需数据能力` 的映射表。**这两步都还没做。**

### 7.2 `RealtimeQuotes` 是价格型功能的共同上游，它一断全断

2026-10-22 实测：`Sina` 与 `Tencent` 双 provider 同时返回
`no_verified_batch / 服务端内部错误`，连锁后果：

```
P3.3 同板块关联票        62 次「关联票查询失败」→ 规则空转
[竞价][Scanner]          consumer_unavailable=scanner_realtime_quote_not_acquired
P-02 竞价量能            DataMode=Unsafe → L5 Deny(data_quality)
持仓价格检测             「持仓价格检测暂停」
```

**排查顺序**：任何"价格型推送没发""票没被跟踪"的现象，**先看 RealtimeQuotes 通不通**，
再往下看业务逻辑。直接查业务层会绕远。

### 7.3 集合竞价的量比只有 final 有，live 只有 09:15 首段

```
AuctionStage::for_now()   （2026-10-22，c7102301c）
  < 09:25  → Live
  >= 09:25 → Final
```

- `live` 阶段的 `auction_volume_ratio` 只在 **09:15 首段**有值，其余时点返回 null。
  convert 层对此是宽容的（`auction_volume_ratio: Option<f64>`，`numeric` 校验用
  `.flatten()` 跳过 None），**不判失败**。
- 但 `prepare_auction_volume_snapshot:8546` 仍把无量比的票整条丢弃
  （2026-10-22 09:20 实测 `valid_rows=0 / missing_volume_ratio_rows=50`）。
  **这条没改**——量能榜没有量比就没有排序依据，改成"照收"需要先定
  09:25 前的排名口径。
- 结论：**09:25 前 P-02 不该产生量能榜**，这是数据属性决定的，不是 bug。

### 7.4 `no_last_traded_price` 的隔离逻辑尚未经过任何真实流量

`e014bb75f` 加了 `KnownReasonCode::NoLastTradedPrice` 和
`GrpcSource::realtime_quotes_with_isolation()`（二分隔离坏票）。
但截至 2026-10-22，生产日志里该 reason_code 出现 **0 次**——Windows 还没开始发这个码。

**这行代码是空转的**。验证方法：等日志出现
`[观察池] RealtimeQuotes 被 no_last_traded_price 拒绝, 二分隔离后 取到 N 只 / 隔离 M 只`
才算真正跑过。

另：proto 的 `ErrorDetail`（全部 35 份版本）都**没有** instruments 列表字段，
所以"按消息里的票列表剔除"在现有契约下无法实现，二分是唯一可行解。
要和列表方案，需要 Windows 在 `ErrorDetail` 加 `repeated string instruments = 12`。

### 7.5 个股新闻通道断供（2026-10-14 起）

`news_ai_assessment` 最后一条 **2026-10-14**，之后 7 天零新增。
后果：观察池的 `news_candidates=0`，四个选票入口里的"新闻催化"这一路
**七天没有进过一张票**。

排查入口（未查）：`main.rs:9333` 的 `admitted` 过滤。

**注意区分**：全球新闻路径正常（`news_ai_global_critical_v1` 一直在跑，
`seq` 持续增长），断的只是个股那条。

## 八、运营决策与拓展（2026-10-22 拍板）

### 8.1 三项已拍板

| 决策 | 内容 | 影响面 |
|---|---|---|
| D1 | 情绪型票**允许突破买**：超买门（连板/近 5 日涨幅/偏离 ma5）对情绪型豁免，放弃条件改为市场信号（竞价弱）而非涨幅计数 | P2 入场模型、P01 链路 |
| D2 | 持股周期**不定全局值**，按票的趋势分型：情绪型短周期、趋势型长周期；型每日盘后重算，型变参数切换 | 淘汰规则、复盘窗口、候选 TTL |
| D3 | 组合预算 = **单周新增仓位上限 + 优先级排序花钱**，超出只标注不拦截（不接熔断） | position_sizing、明日计划 |

### 8.2 分型规则（草案）

- 情绪型：`source='p01_preopen'` 或 近 5 个交易日内出现过涨停
  （`stock_daily.pct_chg >= 9.85`，与 P1.1 的涨停口径一致，不依赖 `is_limit_up`）。
- 趋势型：其余，走 `buy_zone` 现有回踩模型。
- 每日盘后重算一次分型，型变则生命周期参数当日切换。

### 8.3 分型参数表

| 参数 | 情绪型 | 趋势型 |
|---|---|---|
| 入场方式 | 竞价价 / 突破确认（待 Q2 定） | ma 回踩区间（现有 buy_zone） |
| 超买门 | 豁免涨幅与偏离门 | 保留现状 |
| 放弃条件 | 竞价弱（建议低开 >2%） | 击穿放弃价（现有） |
| 淘汰 | 5 自然日 | 10 自然日（不变） |
| 复盘窗 | D+1 / D+3 | D+5 / D+10 |

### 8.4 待拍板参数（均已拍定，2026-10-22）

- Q2 情绪票入场价口径：**已拍 = 两者都给**。09:25 竞价价直给
  + 盘中突破确认价提醒（依赖 RealtimeQuotes，断供时降级为只发竞价价）。
  **执行参数按 2026-10-22 回测修订**：不设低开放弃线（涨停次日低开 >2% 子集是
  唯一正期望分组：开盘买→收盘均值 +1.16%、胜率 47.7%，n=304）；改为**高开帽**——
  竞价涨幅 >6% 不给买价（该子集均值 -1.97%、胜率 28.4%，n=338，是唯一的死亡区）。
- Q3 单周新增仓位上限：**已拍 = 30% 总资产**，超出部分只标注排队、不拦截。
- Q4 竞价上浮保成交：**默认 0.5%**（待 operator 可改）。
- Q5 突破确认基准：**默认昨日最高价**（分钟锚点可靠性未验证，不采用）。

### 8.5 拓展项清单（2026-10-22 评审结论）

能直接排期（无需决策）：竞价板块热度榜（拓 1.1）、买入理由存档 + 理由失效出池（拓 1.3）、
监控事件分级（拓 2.1）、板块内地位变化（拓 2.3）、止盈/目标价侧（拓 3.1）、
fading × 持仓逐票预警（拓 4.1）、按入口归因周报（拓 5.1）、亏损票对照组归因（拓 5.2）、
财报排雷标注 + 归因加业绩维（拓 6，见下）、资金流消费接线（拓 7，2026-10-22 operator 批准，排批 2）。

拓 7 板块资金流消费接线（对应缺陷 B.7）：f62 盘后落库 10-21 已通，**剩的是消费**——
① 观察池选股加一路「f62 资金流 top 板块」入口（与涨幅榜并行，资金来源标注进 ticket note）；
② thesis 存档（拓 1.3）理由可写「主力净流入第 N」；③ 复盘周报加资金流维度。
历史缺口（10-21 前）只能解投递归档补，不回填 f62 表。

拓 6 财报/盈利的角色定位（2026-10-22 评审）：**排雷 + 归因，不进选票打分**。
短线票行情由资金/情绪驱动，财报是季度低频信息，打分只会系统性错过最强票；
但 T+1 下买入当日不能纠错，暴雷是唯一不能自愈的风险——排雷是标注（不挡买入）。
现状：F10 文本已 100% 覆盖活跃票（stock_f10_profile 44/44，四类定性文本，无结构化数字）；
实锤 002076 文本含「退市」、002131/002421 含「立案」（002131 还是持仓票）、多只含减持/解禁，
无人消费。pe/eps/roe 槽位存在但硬编码 None（outcome_daily_bars.rs:2535 / review.rs:3005）。
d15 券商研报/盈利预测 op 合同里有定义，生产可调性未验证（探针同 holding_plan_probe 思路）。
时效：10 月底三季报披露窗口，业绩变脸/预增是当前最强催化与雷，个股新闻通道断供正堵死此窗口。
代价声明：关键词抽取会误报（「退市」常现于模板化风险提示），需抽上下文 + 两周调词表。

需先探源/定口径：商品行情源（拓 2.2，chain_relation_candidate 已有商品名但无价格源）、
推荐价 vs 实际成交偏差复盘（拓 3.2，摊薄成本归因口径未验证）。

明确不做：自动调参、股权穿透式关联公司、开盘集合竞价自动执行。

### 8.6 可复用资产盘点（2026-10-22 代码全扫，含死活证据）

**A 类（活着，可直接接入）**

1. `candidate_trigger_selection` 表 + `render_candidate_triggered/invalidated` 模板
   （0 行/0 调用，骨架在）→ 触发确认闭环（批 1，模板复用不重写）。
2. `catalyst_watchlist_daily`（258 行至 10-20）+ `catalyst_watchlist_outcome`
   （135 行，含次日涨跌/是否续板/streak_today）→ 情绪票复盘（批 1）。
   注意：与 ticket 是并行两套打板跟踪，开工前先定「合并（推荐）还是分工」。
3. `entity_linker.rs`（规则消解新闻→股票，零 LLM 成本，零调用）→ P3.1 新闻断供降级（批 2）。
4. `concept_peers.rs`（stock_concepts 1265 只概念标签，覆盖比 board_member 广）
   → 拓 2.3 板块内地位变化（批 2）。
5. `render_auction_volume` 板块聚合版 + op 61 → 拓 1.1 竞价板块热度榜（批 2）。
6. ReviewLhb 推送链路活着（10-19/10-21 Delivered，网关直取，不依赖死于 7-12 的 lhb_daily 表）
   → 情绪票复盘「谁在买」席位维度（批 2）。
7. `render_event_calendar` 模板（从未接线）+ event_calendar/economic_calendar 网关
   → 排雷日历（批 2，拓 6 配套）。

**B 类（死了，设计可挖）**

- `prediction_*` 全家（samples/verifier/outcome_tracker，76 行死于 7-17）：
  「预测 vs 结果」验证框架的完整结构 → 拓 5 归因口径参考，别重新发明。
- WatchlistTracking 推送（死于 9-24）：证据——「只跟踪不行动的观察表推送」价值低，不复活。
- SectorTop / SectorAnomaly 模板（8-14 后未投递）：板块榜推送要复活时可用。

**C 类（明确不做，现阶段）**

- 多 agent 买卖指标辩论系统：项目不存在此类设施（LLM 仅在 news_ai / chain_ai / g5b
  三处做文本分析与口径治理）。规则指标的归因还没跑顺，现阶段加 LLM 信号只会多一条
  无法归因的线。若未来立项，DeepSeek 通道与 g5b 治理是现成地基。

## 十、执行细节（逐步）

> 通用前提：
> - 所有新增一次性调度用**文件 marker 判重**（`data/<feature>/<date>`），不用进程内 static；
> - 所有新增推送走 `notify::push_wechat` + 本地归档（`data/<feature>_archive/<date>.txt`），
>   **暂不新建 durable push_kind**（避免 v14 指纹 / L5 门禁变更），后续批次再收编；
> - 每批部署按规则 3：主仓 release 编译 → `diff -rq 主仓/src 运行根/src` 对齐 →
>   运行根 release 编译 → **盘后/深夜重启**（盘中不重启）；
> - 每批验收 = 连续 3 个交易日日志出现专属成功行 + 表/文件产出可查（规则 22）。

### 批 0 前置修复（当天可动）

**0.1 全市场日线补齐与调度（修 B.8）**

- 手工补数（今晚盘后，盘中禁止）：`STOCK_DB=<运行根>/data/stock_analysis.db daily_bars_build
  --codes <stock_concepts ∪ board_member.instrument_code ∪ ticket ∪ 持仓>`；HistoricalBars
  返回全历史，一次补齐 10-17 至今。
- 代码：`daily_bars_indicators.rs` 新增 `collect_full_universe_codes()`；新增调度块 **21:30**，
  marker `data/daily_bars_full/<date>`；完成后 `SELECT COUNT(*) FROM stock_daily WHERE date=<today>`，
  `<800` → `log::warn` + push_wechat「日线覆盖异常：今日 N 只」。
- 验证：次日 `stock_daily` 当日行数 ≥800；日志 `[日线全量] targets=N written=M failed=K`。

**0.2 同 code 多活跃票防护（修 B.1）**

- 位置：`database/ticket_lifecycle.rs::open_ticket`。
- 动作：INSERT 前先 `find_active_ticket(code)`；命中且 stage ∈ {observing, entry_proposed}
  → 不新开，`append_event(kind='catalyst', summary='重复入选（已有 ticket#N）')` 并返回旧 id。
  stage=holding/exited 不拦（持仓票归快照管）。
- 配套：`buy_zone_dispatch.rs:302` 观察池查询改 `SELECT code, MIN(source), MIN(open_date) …
  GROUP BY code`（去重）。
- 验证：日志 `[观察池] opened=0 appended=N` 的 N 含重复入选；SQL 同 code 活跃票 ≤1。

**0.3 明日计划归档 + 去重（修 B.5 缺口）**

- 位置：`main.rs:12757` 推送成功后。
- 动作：写 `data/buy_zone_archive/<date>.txt` 全文（字符数=日志 `[可买价] 明日计划生成 N 字`），
  写失败 `log::warn`；去重见 0.2 配套。
- 验证：归档文件存在且字数一致；正文同 code 只出现一次。

**0.4 P1.1 marker 化（修 B.9）**

- 位置：`main.rs:12135` 观察池块。marker `data/watch_pool/<date>`，成功（含 opened=0、appended>0）
  后写；失败不写、保留重试。static 只留作当日快速跳过缓存。
- 验证：当晚重启多次，日志 `[观察池] boards=` 每交易日只出现一次。

**0.5 position 事件幂等（修 B.4）**

- 位置：`main.rs:12068` 快照比对写事件前。
- 动作：`SELECT 1 FROM ticket_event WHERE ticket_id=? AND event_kind=? AND date(event_at)=?
  AND summary=?`，存在则跳过。
- 验证：同摘要同日重复事件（603915 式 ×6）消失。

**0.6 实施记录（2026-10-22，代码已改）**

| 项 | 落地位置 | 实际动作 |
|---|---|---|
| 0.1 | `monitor/daily_bars_indicators.rs:86` | 新增 `collect_full_universe_codes()`：`stock_concepts ∪ board_member.instrument_code ∪ 活跃 ticket ∪ open 持仓`；今日实测口径 **2965 只** |
| 0.1 | `bin/monitor/main.rs`（15:55 块之后） | 新增 21:30 全量块：marker `data/daily_bars_full/<date>`；成功写 `[日线全量] targets= written= coverage_today=`；`coverage < 800` → `log::warn` + push_wechat 告警 |
| 0.1 | `bin/daily_bars_build.rs` | 新增 `--all`（与 21:30 同一口径），作为手工补数 / 21:30 路径的验证入口 |
| 0.2 | `database/ticket_lifecycle.rs:135` | 同 code 命中活跃票 → 直接返回既有 id，并补写 `catalyst` 事件「重复入选（已有 ticket#N）」（同日幂等） |
| 0.2 | `monitor/buy_zone_dispatch.rs:302` | 观察池查询改 `MIN(source)/MIN(open_date) … GROUP BY code` |
| 0.3 | `bin/monitor/main.rs`（明日计划推送前） | 正文归档 `data/buy_zone_archive/<date>.txt`，写失败 `log::warn` |
| 0.4 | `bin/monitor/main.rs`（观察池块） | marker `data/watch_pool/<date>`，内容 `news_candidates/board_candidates/total/opened/appended`；成功才写、失败保留重试 |
| 0.5 | `ticket_lifecycle.rs::event_exists_same_day` + `monitor/position_change_detect.rs:269` | 写事件前查 `(ticket_id, event_kind, date(event_at), summary)`，命中跳过 |
| 附 | `bin/monitor/main.rs`（板块成分循环） | `gn_` 板块成分失败不再逐条 warn（预期缺口），合并成一行 info；BK 码失败仍 warn |

与方案原文的偏差（以代码为准，后续不再回改方案）：

1. **0.2 原文「holding/exited 不拦」，实现为「任何 `close_date IS NULL` 的活跃票都复用」**（含 holding）。
   理由：一只票同一时间只应有一张票；持仓票的加仓/减仓走 HoldingPlan，不需要第二张 observing 票，
   而选股信号以 `catalyst` 事件落在原票上，不丢信息。exited 的票 `close_date` 非空，自然不拦，可重新开票。
2. 0.1 的 marker 内容写覆盖数（`<date>` 文件里就是 coverage），日志行同时打印，便于次日不查库也能判断。
3. 21:30 块放在 `market_loop` 盘后段（15:55 轻量轮之后），与 19:00 严格复盘、16:00 明日计划互不阻塞；
   2965 只串行拉取在同一 loop 内约需数分钟，21:30 之后该 loop 无其它调度。

改前现网实测（2026-10-22 盘后只读查询）：

- `stock_daily` 逐日行数：10-14/15/16 = **1248**；10-19 = **54**；10-20 = **53**；10-21 = **49**；10-22 = **45**。
- 同 code 多活跃票 3 组：`002696`（#62 #64）、`002724`（#63 #65）、`603278`（#37 #66）。
- ticket 55（603915）`position_opened` 在 2026-10-20 有 **6 条**。
- 明日计划观察池：44 行 → 按 code 去重后 **41** 行（修好当天即少 3 行重复）。

**0.7 验证与容量实测（2026-10-22）**

编译 / 单测：

| 检查 | 命令 | 结果 |
|---|---|---|
| 类型检查 | `cargo check --bin monitor` | 通过（仅存量 unused import 警告） |
| debug 构建 | `cargo build --bin daily_bars_build` | `Finished dev in 9m40s`，891 条警告全部为存量 |
| 运行根 release | `cargo build --release --bin daily_bars_build --bin monitor` | `Finished release in 14m28s`；改事务后重建 `8m35s` |
| 单测（新增） | `cargo test --lib ticket_lifecycle` | `open_ticket_reuses_active_ticket_and_records_repeat_selection ... ok`（1 passed / 0 failed，5622 filtered out） |

单测覆盖的行为（B.1 回归）：同 code 第二次入选复用既有票 id、不新增行；重复入选写一条
`catalyst`「重复入选（已有 ticket#N）」；同日再入选不重复写事件；票关闭（`close_date` 非空）后可重新开票。

SQL 级改动在现网只读库上逐条验证：`event_exists_same_day` 查询可执行、`GROUP BY code` 去重
（44→41）、全市场口径查询（2965）。

容量实测（`--all` 2965 只，运行根 release，串行 HistoricalBars）：

- 正常段约 **66 次请求/分钟**，平均 1.55 次/只（含 provider 回退）；
- 失败码走 `router_sources_exhausted`，单只 30~60 秒，尾段退化到 **约 2 次/分钟**；
- 请求结果分布：`available` 4373 / `partial` 115 / `unavailable` 115（≈5% 非 available）；
- 写入：原实现每条 INSERT 独立隐式事务，实测 **≈1500 行/分钟**（2965 只 × 30 天 ≈ 8.9 万行 ≈ 1 小时）；
  已改单事务 `BEGIN IMMEDIATE … COMMIT`，中断整批回滚，写入降到秒级。

结论与待办：**全量一轮 40 分钟起步，失败码越多越长**，21:30 那一班必须给单只设硬超时
（建议 10s）并跳过已知失败码，否则会拖到 22:10 以后；写入已不再是瓶颈。

**0.8 容量三项修正（2026-10-22 夜，operator 选「先修再跑」）**

| 问题 | 改法 | 位置 |
|---|---|---|
| 21:30 全量占住 `market_loop`，把同链路的盘后块顶住（10-22 实测 15:55 那班被挤到 21:16 才结束，且多只票 `HistoricalBars 服务端内部错误`） | 拆成独立长驻任务 `full_universe_bars_loop()`：`tokio::spawn` 后挂进 `background_tasks` 生命周期；`market_loop` 里只留注释指向它 | `bin/monitor/main.rs` |
| 失败码每只 30~60 秒，尾巴把一轮拖到 90 分钟 | 单只硬超时 `FETCH_TIMEOUT = 10s`，超时计 `failed` 并跳过；失败明细只留前 20 条，其余只计数 | `monitor/daily_bars_indicators.rs` |
| 「全部拉完再一次性事务」——中途被杀一条都不写 | 每 `PERSIST_CHUNK = 200` 只一个事务提交，批间打 `[日线均线] 进度 i/N fetched= failed=`；失败批回滚、已提交批保留 | 同上 |
| 单轮没有上限，桥被挤住时会跑通宵 | `FETCH_BUDGET = 45 分钟`：到点停止取数，把已拿到的批次提交后返回，剩余留给下一轮或 CLI 补 | 同上 |
| 观察池块重放会重复写 catalyst（10-22 重启后 ticket 18/22/54 各多一条） | 新增 `append_catalyst_once()`：同日同票同摘要只写一次，`report.appended` 只统计真正写入的 | `monitor/ticket_watch_pool.rs` |

`DailyBarsReport` 增加 `failed` 字段，`log()` 改为
`targets= fetched= persisted_bars= with_ma20= failed=`；`daily_bars_build` 汇总行同步。

补数结果：手工三次尝试都没跑完——第一次运行根旧二进制不认 `--codes`（只刷了 46 只）；
第二、三次被失败码尾巴拖住（60~90 分钟仍在拉取阶段）手动终止，**未提交任何行**。
改成独立任务 + 10s 超时 + 每 200 只分批提交后，由 21:30 任务重新拉取，结果见 0.9。

**0.10 2026-10-24 补数复盘：被对端 HistoricalBars 阻断（实际结果）**

- 用新 CLI（10s 单只超时 / 每 200 只一事务 / 45 分钟预算）跑
  `daily_bars_build --date 2026-10-23 --all`：
  `targets=2965 fetched=0 persisted_bars=0 failed=270`，`stock_daily` 当日仍 **0 行**。
- 真因不在超时设置：单票直调桥（000001, days=30）返回 `router_sources_exhausted`，
  四路全灭 —— Tdx `[E2103] declared bars without row bytes`、
  Tencent `day bar row has 7 fields; expected 6`（即 handoff 第 3 项那处「已修未部署」）、
  Sina `daily amount is unavailable at 2026-09-04`（质量门整批拒）、Baidu 403。
- 结论：**全市场日线在补上之前，任何依赖它的验收都不成立** ——
  perf 的 D+N（除活跃票）、市场基准（n=47~53）、P6 归因样本、5.2 成交偏差的基准价。
- 已做的事：把实测报文写进 `handoff-windows-externalv1-20261024.md` 第 3 项并升级为 P0；
  周报（4.3/4.4）代码不依赖 10-23 数据，可先跑，10-22 批次会显示空行。

**0.11 2026-10-24 盘后文件通路：日线改走通达信 vipdoc（实测结果）**

- 0.10 的四路全灭是**实时行情服务器协议**的问题；复盘用的日线本来不必走它。
  通达信盘后数据（`C:\new_tdx64\vipdoc\{sh,sz}\lday\*.day`）是本地定长文件，
  自带成交额、约 5 年历史、无限流 —— operator 2026-10-24 指出这条，确实是更合适的源。
- 落地：`src/bin/tdx_day_import.rs`（新）。记录 32 字节：
  `date u32(YYYYMMDD)`、`open/high/low/close u32(分)`、`amount f32(元)`、`volume u32(股)`、`reserved`。
  入库口径：价格 /100、volume /100（股→手，与 `stock_daily` 既有口径一致；
  用 000001 10-22 对账：TDX 129,764,872 股 → 1,297,648.72 手，与桥给的值逐位相同）、
  amount 原样、`pct_chg` 用文件内前收算。只收 A 股（`sh6*`、`sz00*`/`sz30*`），
  指数（sh000/sh880/sz399）与 B 股（sh9*/sz2*）跳过；不动 `ma*`/`volume_ratio`。
- 传输：Windows VM 里 `\\Mac\Home` 是**离线缓存**（Windows 侧 `dir` 能看到 4,846 个文件，
  Mac 侧一个都没有）；最后走「VM 内 tar 打包 → `curl -T` PUT 到 Mac 的临时上传端」，
  315 MB 两个包约 30 秒传完。
- 实测（2026-10-24 11:47）：`files=5227 rows=6187698 errors=0 range=2021-08-02..2026-10-22 elapsed=289s`；
  `stock_daily` 从 49,016 行 / 3,194 只 → **6,187,758 行 / 5,229 只**；
  2026-10-22 覆盖 47 → **5,218** 只。库 2.68 GB → 4.6 GB。
- 边界：TDX 本地只保留约 1,264 根（2021-08 起），更早的历史没有；
  文件 mtime 停在 10-22 16:00，**2026-10-23 仍缺** —— 要在 Windows 上跑一次
  「盘后数据下载」后再同步。
- 未做：MA 回补（导入行 `ma5/10/20`、`volume_ratio` 为空，15:55 任务只算当日）、
  21:30 增量同步、把 `daily_bars_build` 的 provider 路径降级为备用。
- 补：周报重算（2026-10-24 12:16，`ticket_weekly_report --date 2026-10-23`）——
  超额列第一次出真数（此前一律「样本不足」）：

  | 入口 | n | 胜率 | 均值 | 中位 | 超额 |
  |---|---|---|---|---|---|
  | P01 | 4 | 75% | +6.63% | +5.64% | +7.48% |
  | 持仓 | 1 | 100% | +1.64% | +1.64% | +1.81% |
  | 轮动 | 29 | 45% | +1.24% | -0.09% | +0.81% |
  | 新闻 | 2 | 50% | -4.01% | -4.01% | -5.09% |

  本周 41 只、D+1 可算 36（原 35）、D+3 可算 18（原 17）。
- 观测（未根治）：`descriptor_attestation_ambiguous: N descriptors match the pinned main identity`
  （perf 报 2、周报报 3），r2d2 记 ERROR 但进程正常跑完；疑似与连接池多连接有关，
  尚未定位，列入待查，不影响本次结论。


**0.12 2026-10-24 桥内修复：Tencent 接受 7 字段 + 重启（实际结果）**

- 归因更正：0.10 写的「死在 Windows 侧 provider 路由」是错的，已向对方收回两次。
  `router_sources_exhausted` 是本机 `data_gateway::review::HistoricalDailyBars` + `router` 的输出，
  请求不出 Mac（对方 10:02 用 ExternalV1 实测 Tencent 30 条成功，同源 crate 不同 pin）。
- 修复：`magic-market-data-rs @ 75ee2a2` 的 `magic-tencent-rs` 日线行要求恰好 6 字段，
  除权日行带第 7 个元素（分红对象）。已 vendored 到
  `.worktrees/tdx-local-provider-20260923/vendor/magic-tencent-rs` 并加 `[patch]`，
  改为非分钟线接受 6/7 字段、忽略第 7 个（仍 Unadjusted）。release 4m20s。
- 部署：10:15 换二进制（旧的在 `grpc_market_server.bak-20261024`），10:20 监听（285s）。
- 探针：默认路由 `provider=Baidu complete=true`，000001/30 根含 2026-10-23；
  但 `daily_bars_build --all` 仍 `fetched=0` —— Tencent 不再报 7 字段，
  改成 **`daily amount is unavailable`**（`historical_bars.rs:863` 要求每根都有 amount）；
  Sina 同一道门，Tdx 空 payload，Baidu 403。四路都不给 amount，桥这条路对日线不构成解锁。
- 结论：日线的正解是 0.11 的盘后文件；桥保持现状（RealtimeQuotes 等价格型功能正常）。
- 待查：显式 `preferred_provider` 四种全部 `router_sources_exhausted`（生产不传，
  不阻塞；与默认路由落到 Baidu 相矛盾，需要单独复现）。

**0.9 2026-10-22 夜间验收（实际结果）**

- 21:30 全量任务：**没有跑成**。22:39 重启后新任务在 22:45 启动，但本机桥在此之前
  已经因 `Too many open files (os error 24)` 停止服务（旧进程 EMFILE，22:39:34），
  所有 HistoricalBars 请求都在 10s 客户端超时处被取消（`The operation was cancelled`）。
- 桥重启：`kickstart -k` 后主线程停在 `StatementIterator::next`（`sample` 证实），
  即启动期的全链审计；大库上耗时约 **19 分钟**，前几次重启都在审计中途被杀，看起来像卡死。
  最终 23:29 恢复监听。
- 顺带修掉一个基础设施问题：桥的 plist 没有 `SoftResourceLimits`，继承系统
  `maxfiles=256`，旧进程 22:39 就是被 fd 耗尽打死的。已加
  `SoftResourceLimits.NumberOfFiles = 8192`（备份 `com.northofqing.grpc-market-server.plist.bak-*`）。
- 为不再和桥的启动抢锁：今晚 **停掉 monitor、写 `data/daily_bars_full/2026-10-22` marker
  跳过全量**，23:29 重新拉起 monitor（PID 19189，状态 0）。
- `stock_daily` 当日覆盖仍为 **47**（10-21=49、10-22=47），**没有补到全量**。

遗留（明早开盘前）：用新 CLI 补 10-22——
`STOCK_DB=<运行根>/data/stock_analysis.db ./target/release/daily_bars_build --date 2026-10-22 --all`，
新版本自带 10s 单只超时、每 200 只一事务、45 分钟预算，跑完看 `fetched / failed` 与覆盖。

### 批 1 分型体系 — **已完成（2026-10-23）**

**1.1 分型（regime）计算（D1/D2 的地基）— 已完成（2026-10-23）**

- 存储：**不加列**——用 `ticket_event(kind='regime', summary='emotion|trend|<reason>')`，
  每日 **15:42**（观察池建完）对 active tickets 重算，取每票最新一条为当前型。
  理由：ticket_lifecycle 加列要动 schema 版本（v9 钉死，P4 教训）。
- 规则：情绪型 = `source='p01_preopen'` ∪ 近 5 交易日出现涨停
  （阈值按 1.2 分板）；其余趋势型。
- 验证：日志 `[分型] emotion=N trend=M`；SQL 每票最新 regime。

**1.2 20cm 阈值分档（修 B.10）— 已完成（2026-10-23）**

- 新增 util `limit_up_pct(code)`：300/301/688/689 → **19.5**，其余 → **9.85**；
  替换 `ticket_watch_pool::LIMIT_UP_PCT` 全部用法、regime 判定、回测脚本。
- 边界：ST 5% 板无名称源可判别，不处理，文档注明。

**1.3 情绪票竞价买价（Q2=c / Q4=0.5%）— 已完成（2026-10-23）**

- 位置：`auction_watch_loop`（main.rs:14437），09:25-09:32 `observe()` 拉完调出口。
- 动作：取 regime=emotion 活跃票，按竞价涨幅分档：`>+6%` 不给价（「高开过多不追」）；
  `+2~+6%` 给竞价价标「轻仓」；`-2~+2%` 给 竞价价×1.005 标「保成交」；`<-2%` 给竞价价
  标「低开博弈区」。组一条 push_wechat，文案必带「竞价买入当日 T+1 不可卖」；
  marker `data/auction_price/<date>`。
- 验证：日志 `[竞价买价] emotion=N quoted=Q skipped_high=K`；marker 存在。

**1.4 盘中突破确认（Q5=昨日最高价）— 已完成（2026-10-23，模板未用，见 1.7 偏差 2）**

- 位置：`watch_pool_monitor::scan` 新增第五规则。缓存：每日 09:26 / 13:01 预热
  entry_proposed 票的 `buy_zone::compute` 与昨高（`stock_daily.high`），内存 HashMap，
  扫描只比价不查库。
- 条件：现价 ≥ 昨日最高 且 当日涨幅 <6% → push `render_candidate_triggered` 文案
  （1 票/日限 1 次，复用 alerted_at）；写 `candidate_trigger_selection` +
  `ticket_event(kind='entry_triggered')`。
- 边界：RealtimeQuotes 合同无量比字段（2026-08-31 注释），**不加量能条件**；
  无分钟线回测，上线后观察记录两周。
- 验证：日志 `[触发确认] scanned=N triggered=M`；表行数增长。

**1.5 价格失效（entry_invalidated）— 已完成（2026-10-23，板块退出条件未做，见 1.7 偏差 3）**

- 条件任一：现价 < 放弃价（缓存同上）；所属板块退出 top10（C 规则日检）；thesis 失效
  （批 2）；时效——趋势型超 3 交易日、情绪型跨日。
- 动作：push `render_candidate_invalidated` + `ticket_event(kind='entry_invalidated')`；
  **只作废价格，不淘汰票**（stage 不变）。
- 验证：日志 `[价格失效] n=`；事件表增长。

**1.6 淘汰/复盘窗参数化（D2）— 已完成（2026-10-23）**

- `ticket_eviction`：emotion → `MAX_OBSERVE_DAYS=5`；trend → 10（现行为）；型别取 1.1 事件。
- `watch_pool_perf`：emotion 批 D+1/D+3、trend 批 D+5/D+10 分列输出。
- 验证：淘汰日志 reason 带型别；perf_log 两段。

**1.7 实施记录（2026-10-23，批 1 全部落地，commit 621ea15cf）**

批 0 先入库成独立 commit `eddb6d49e`（工作区原本挂着 8 个未提交文件，
没有可回滚基线就无法判断本批改动是否真把问题修好）。

| 项 | 落地位置 | 实际动作 |
|---|---|---|
| 1.1 | `monitor/ticket_regime.rs`（新）+ `main.rs` 15:37 块 | `Regime::{Emotion,Trend}`、`classify`、`resolve`（无事件则现算）、`recompute_active`。写 `ticket_event(kind='regime', summary='emotion\|理由')`；**只在型别/理由变化时追加**，值相同跳过（重启重放不产生重复行）。`observe_days()` 5/10、`perf_periods()` D+1/D+3 与 D+5/D+10 由这一处定义，淘汰/复盘/竞价买价/触发确认全部复用。 |
| 1.2 | `ticket_regime::limit_up_pct` | 300/301/688/689 → 19.5，其余 9.85。替换 `ticket_watch_pool` 的 `LIMIT_UP_PCT` 常量与 `buy_zone` 的 `LIMIT_UP_PCT_THRESHOLD`（原 9.8，20cm 板连板数虚高 → 超买门误杀）。回归测试：`300750` +10.02% 判趋势型、`002696` +10.02% 判情绪型。 |
| 1.3 | `monitor/ticket_auction_price.rs`（新）+ `auction_watch_loop` | 复用同一批 `AuctionRow`（不额外拉 op 61）。分档：>+6% 不给价 / +2~+6% 轻仓 / -2~+2% 上浮 0.5% 保成交 / <-2% 低开博弈区；正文固定带「竞价买入当日 T+1 不可卖」。推送成功才写 marker `data/auction_price/<date>`，失败保留重试。无情绪型票 → 不推空消息，日志写「不是失败」。 |
| 1.4 | `monitor/ticket_entry_trigger.rs`（新）+ `watch_pool_monitor_loop` | 每日 09:26 / 13:01 两个槽位预热 `entry_proposed` 票的昨高 + `buy_zone` 区间 + 分型（内存 HashMap），扫描只比价。条件：现价 ≥ 昨高 且 当日涨幅 < 6%。命中写 `candidate_trigger_selection`（PK 天然 1 票/日一次）+ `ticket_event(kind='entry_triggered')`，即时推送。 |
| 1.5 | 同上 + `main.rs` 15:42 块 | 盘中：现价 < 放弃价 → `kind='entry_invalidated'`（摘要不带现价，带进去永远判不出重复）。盘后：趋势型 > 3 个交易日、情绪型跨日未触发 → 同一事件。**只作废价格，不改 stage**。 |
| 1.6 | `ticket_eviction` + `watch_pool_perf` | 淘汰上限取 `ticket_regime::resolve(...).observe_days()`，reason 带型别；`watch_pool_perf` 分两段输出（情绪 D+1/D+3、趋势 D+5/D+10），CLI `--days` 仍可显式窗口。 |

与方案原文的偏差（以代码为准）：

1. **1.1 调度从 15:42 提前到 15:37**。15:38 的淘汰要用当日分型决定观察期上限，
   排在淘汰之后等于永远用昨天的型；分型只依赖日线，15:37 可算，不需要等 15:40 观察池。
2. **1.4 没有用 `render_candidate_triggered` 模板**。该模板必填 `grade` / 三类证据质量 /
   仓位上限，这些在本链路没有数据源，硬填等于编造（BR-222）。改为简明文案
   （`⚡ 突破确认 代码 名称 现价 涨幅 — 型别｜突破昨高 …；区间 …`），
   `candidate_trigger_selection` 照原方案落表，后续接上证据质量再换模板。
3. **1.5 的「所属板块退出 top10」条件未实现**。`board_member` 只覆盖成分拉取成功的板块，
   「表里没有」≠「不属于」，用它作废价格会误杀；明日计划的 C 规则已经在做同一件事，
   再叠一条会重复扣票。已改为 2.8 的「所属板块退潮」标注（提示不拦截）。
4. **1.6 的 perf 分母变了**：来源从 `board_rotation/news_catalyst` 扩到
   `+p01_preopen/money_flow/holding` —— 原来 P01 与资金流票完全不在选股表现里，
   等于只统计了一半推荐。

验证（本机，2026-10-23 深夜，**非交易时段**）：

- `cargo check --lib --bin monitor` 通过（仅存量 unused 警告）。
- `cargo test --lib ticket_`：33 passed / 0 failed（含 1.1 的 5 个、1.2 的板别阈值、
  1.3 的 5 个分档、1.4/1.5 的 4 个判据）。`cargo test --lib watch_pool_perf` 同批跑。
- **没有产出可验**：分型/竞价/触发全部依赖交易时段与实时行情，今晚只能证明「编得过、测得过」。
  真正的验收是下一交易日：日志出现 `[分型] scanned= emotion= trend=`、
  `[竞价买价] emotion=N quoted=Q`、`[触发确认] 预热`、`[价格失效] 时效检查`，
  且 `scripts/phase_acceptance.py` 新增的「批1.1/1.3/1.4/1.5」四项至少达到「部分」。
- 运行根未部署：本批只改主仓；部署按规则 3（两次 release + `diff -rq` + 盘后重启）。

### 批 2 接线拓展（2026-10-24：11 项完成 9 项）

**2.1 09:25 板块热度（拓 1.1，并入 1.3 同一条消息）— 已完成（2026-10-23）**：竞价 rows 按 board_member 聚合，
输出 top5「板块名 + 竞价均涨幅 + 竞价量比中位」；不足 5 个按实际。

**2.2 thesis 存档与理由失效（拓 1.3）**：open_ticket 时写 `kind='thesis'`
（板块/催化/来源）；每日 **15:50** 核对板块是否仍在 top10（board_ranking_daily 前一交易日），
不在且连续 2 日 → `kind='thesis_invalidated'`；eviction 增第三规则（理由失效出池）。

**2.3 资金流选票入口（拓 7）— 已完成（2026-10-23）**：观察池新增 `collect_money_flow_boards`——读
`board_ranking_daily rank_fid='f62'` 前一日 top10 → 成分股取涨幅前 3，source='money_flow'；
日志 `mf_candidates=N`；10-21 前无 f62 数据则跳过（不补造）。

**2.4 新闻降级（P3.1 配套）**：`news_ai_assessment` 连续 N 日无新增时，
post_close_news_review 与盘中 scan 新闻规则改用 `entity_linker` 对 news_items 近 5 日匹配
→ `kind='news_mention'`，news_dedup 去重。

**2.5 概念扩展（拓 2.3）— 已完成（2026-10-24）**：P3.3 关联票 = board_member ∪ concept_peers；日志带来源分布。

**2.6 ReviewLhb 接入复盘 — 已完成（2026-10-24）**：LHB 复盘生成后，对活跃 emotion 票匹配，写 `kind='lhb'`
摘要（席位/净买额）；链路走网关直取，不依赖死表 lhb_daily。

**2.7 排雷（拓 6 部分）— 已完成（2026-10-23）**：每日 **16:20** 扫 F10 四类文本关键词
（退市/立案/预亏/商誉减值/减持/解禁，取原文 40 字片段），命中写 `kind='risk_flag'`；
render_plan 渲染「⚠ 有雷」；event_calendar 可用则加解禁/财报日历行。
边界：关键词误报，两周调词表。
补充（2026-10-22 路由盘点）：ExternalV1 合同里有 `Announcements` / `MarketAnnouncements`
capability（见 grpc_source.rs STATIC_OPENING_CAPABILITY_FAMILIES），Mac 侧零消费——
公告通道不是不存在，是没接线；F10 关键词扫是过渡，公告 op 探针通过后可升级为权威源。

**2.8 fading × 持仓（拓 4.1）— 已完成（2026-10-23）**：**15:36**（轮动信号 15:34 之后）fading ∪ board_member
× 持仓/ticket，render_plan 头部逐票标注「所属板块退潮」。

**2.9 监控分级（拓 2.1）— 已完成（2026-10-23）**：scan 规则分级——L1（价格 ±3%、新闻命中）→ push_wechat
即时（限 1 次/票/30min）；L2（tick 变动、关联票）→ 仅日志（现行为）。

**2.10 止盈/目标价（拓 3.1）— 已完成（2026-10-23）**：render_plan 持仓段加目标价 = 20 日最高、
移动止盈线 = max(成本, 近 5 日低) 破位提示；不做自动卖。

**2.11 priority 排序接线（拓 8）— 已完成（2026-10-23）**：render_plan 新买入段按 `rank_watch_pool` 分数排序，
标注 ①②③ 建议顺序。

**2.12 实施记录（2026-10-23，批 2 完成 7/11）**

| 项 | 落地位置 | 实际动作 | 验收看什么 |
|---|---|---|---|
| 2.1 | `ticket_auction_price::board_heat` | 竞价 rows 按 `board_member` 聚合，只取成员 ≥2 的板块，按竞价均涨幅取前 5，量比给中位数；并入 1.3 同一条消息（不另推）。 | 竞价推送正文出现「📊 板块热度」段 |
| 2.3 | `ticket_watch_pool::collect_money_flow_boards` | 把板块路抽成 `collect_board_picks(board_date, rank_fid, source, label, …)`，f3→`board_rotation`、**当日 f62**→`money_flow`（当日榜不在才退最近一日）。f62 无数据返回 0，不补造。 | 观察池日志 `mf_boards= mf_candidates= mf_added=`；`phase_acceptance.py` 的「批2.3」 |
| 2.7 | `monitor/ticket_risk_flag.rs` + `main.rs` 15:52 | F10 四类原文扫 6 个关键词（退市/立案/预亏/商誉减值/减持/解禁），命中写 `kind='risk_flag'` + 抽 ±20 字片段，一条消息汇总推送；`render_plan` 持仓段与新买入段渲染「⚠ 有雷（标注，不挡操作）」。 | 日志 `[排雷] scanned= hits= written=`；明日计划正文出现「⚠ 有雷」 |
| 2.8 | `buy_zone_dispatch::render_plan` | 读 `board_rotation_signal status='fading'`，持仓票与新买入票逐票标「⚠ 所属板块退潮」。 | 退潮板块的持仓票正文多一行 |
| 2.9 | `watch_pool_monitor_loop` | L1（价格 ±3%、新闻命中）→ `push_wechat` 即时，沿用 30 分钟同票同因去重；L2（较上次急变、同板块关联票）→ 仅日志。 | 日志出现 L1 推送；L2 仍只进日志 |
| 2.10 | `buy_zone_dispatch` 持仓段 | 目标价 = 20 日最高（参考）；移动止盈 = max(成本, 近 5 日低)，跌破提示；不自动卖、不改建议动作。 | 持仓段出现「目标价」「移动止盈」两行 |
| 2.11 | `buy_zone_dispatch` 新买入段 | 按 `ticket_priority::rank_watch_pool` 分数降序，①②③ 标建议顺序并附分数；取不到分数退回建议仓位排序并在正文写明。**只排序不拦截**。 | 新买入候选按分数从高到低，第四名起用数字 |

| 2.5 | `concept_peers::ConceptIndex` + `watch_pool_monitor::load_peer_alerts` | 原来 `peers_of` 每次调用全表扫 `stock_concepts`（30 秒一轮 × 十几只票 = 十几次全扫）。改成 `ConceptIndex::load` 一次装载 + `peers_many` 按票分组；P3.3 的关联票改为 `board_member ∪ 概念共现`，文案区分「同板块」/「同概念」，同概念只报最强一只。 | 日志里出现 `同概念「光伏、储能」…`；单轮行情请求数不增加（受 `PEER_QUERY_LIMIT`） |
| 2.6 | `monitor/ticket_lhb_sync.rs`（新）+ `lhb_sync_loop` 21:40 | 探针实测投递归档 35/35 条正文含「营业部/席位/净买/机构专用」，因此**不新增 op**：只读 `durable_delivery.sqlite3`，解析 `N. SZ300296 排名净买 1.83亿` + 买入席位前两席，给活跃**情绪型**票写 `kind='lhb'`（同日同摘要幂等）。marker `data/lhb_sync/<date>`，当日无 LHB 推送不写 marker、5 分钟重试。 | 日志 `[龙虎榜] decisions= parsed= matched= written=`；`SELECT COUNT(*) FROM ticket_event WHERE event_kind='lhb'` |

偏差（与 2.3 原文）：原文写「前一交易日 f62」，实现改为**当日 f62**。原因是成分映射
（`eastmoney_members`）由调用方按**当日**榜单 top 板块拉 BK 成分——用前一交易日的榜，
名字/代码对不上，入口会永远选出 0 只。`board_ranking_daily` 的 f62 在 15:30 已写，
15:40 观察池读得到；当日没有才退最近一日（此时成分可能缺，`mf_candidates=0` 属预期）。

未做（保持原状，没有半实现）：

- **2.2 thesis 存档/理由失效**：存档一步即可（`open_ticket` 时写 `kind='thesis'`），
  但「连续 2 日不在 top10 → 理由失效并出池」与明日计划的 C 规则、2.8 的退潮标注
  是同一判据的第三次实现，先确认要合并还是分工，再动手（方案 8.6 A.2 也要求先定这件事）。
- **2.4 新闻降级（entity_linker）——探针结论：这条降级不成立，建议撤销。**
  2026-10-24 实测：`news_items` 逐日仍在进（10-19 629 / 10-20 571 / 10-21 689 /
  10-22 292 / 10-23 202 条），但 7 天里只有 **48 个不同 code**，且每个 code 都是
  「我方按票去拉」（`post_close_news_review` 拉持仓∪活跃票 30 天）——
  它是**按票拉取的存储**，不是全市场新闻流。所以：
  ① 堵的是 AI 评估段（`news_ai_assessment` 最后一条 2026-10-14），不是新闻采集；
  ② 在 48 只票上做实体识别没有增量，`entity_linker` 降级提供了不了新覆盖；
  ③ P3.1 的新闻监控已经在用 `news_items.code`，不受 AI 断供影响。
  真正该做的是查 AI 评估为什么停（另立排查项），不是再加一条读同一份窄数据的链。

### 批 3 预算层（Q3=30%）— 未开工（见 3.4）

**3.1 周预算状态**：本周已用 = 本周新增持仓成本（快照 `position_opened` 对应
quantity × snapshot cost_price 求和）；预算 = 总资产（user_account_summary，缺则
最近 real_account_snapshot）× 30%；余量 = 预算 − 已用。

**3.2 集中度**：单板块本周新增 ≤ 15%（预算一半）；超限票在明日计划标「板块集中度高·排队」。

**3.3 排序花钱**：候选按 priority 分数降序，逐票标「建议顺序 / 占用预算 / 剩余预算」；
超预算标「排队」——**不拦截**。

**3.4 状态（2026-10-23）：未开工。** 前置是一件没定的事：本周新增仓位的权威口径。
方案写「快照 `position_opened` 对应 quantity × 快照 cost_price」，但 `position_opened`
事件只在相邻两份快照的差量里产生，跨周的第一份快照没有基线；用
`trade_ledger` 的推断买入又是另一套口径（BR-215 允许它缺）。两个口径对不上时
预算会算错，而预算层只标注不拦截——错标注比没有标注更误导。先定口径，再写代码。

### 批 4 复盘归因（2026-10-23：4.1/4.2 完成）

**4.1 perf_log 口径修正（修 B.11）— 已完成（2026-10-23）**：基准价按 source 分——`p01_preopen` → open_date
**开盘价**（当日可买）；其余 → open_date 收盘价。

**4.2 基准对照 — 已完成（2026-10-23）**：批次胜率旁加「全市场等权同日收益」；样本 <800 标「基准不可信」。

**4.3 入口归因周报（拓 5.1 + 5.2）**：**周五 17:00** 聚合按 source × D+1/D+3
胜率/均值/超额；亏损票条件对照附后；push_wechat 简化版（报告整理简化后推送）。

**4.4 归档**：周报同时写 `data/perf_log/<date>_weekly.txt`。

**4.5 实施记录（2026-10-23，4.1/4.2 完成，4.3/4.4 未做）**

- 4.1 已完成：`watch_pool_perf::render` 基准价按 source 分——`p01_preopen` 取
  `open_date` 开盘价（09:00 开票、当日可买），其余取收盘价；来源扩到
  `board_rotation / news_catalyst / p01_preopen / money_flow / holding`。
- 4.2 已完成：批次汇总每个窗口多一列「D+N 市场」= 同日全市场等权收益 + 样本数；
  样本 < 800 标「不可信」（当前日线覆盖只有几十只，这一列会如实显示不可信）。
- 未做：4.3 入口归因周报（周五 17:00）与 4.4 归档 —— 依赖 4.1/4.2 的产出先跑满一周，
  且要新增周日历判断（节假日顺延），排在下一批。

### 批 5 探源

**5.1 商品行情源（拓 2.2）— 探源完成，源可用（2026-10-24）**：探新浪/东财商品接口可用性；可用则
chain_relation_candidate 商品名→价格日更；不可用则关闭该项并记录。

**5.2 成交偏差（拓 3.2）— 脚本完成，样本待数据（2026-10-24）**：先做离线分析（entry_proposed 区间 vs 快照 cost_price
偏差分布），口径确认后再上报表。

**5.3 探源结果（2026-10-24，实测）**

5.1 商品行情源：**可用**。新浪内盘期货连续合约从 Mac 直连返回真实价格：

```text
nf_RB0 螺纹钢连续 3042.000   nf_I0 铁矿石连续 670.500
nf_CU0 铜连续    108420.000  nf_AL0 铝连续    23575.000
nf_J0  焦炭连续   1931.500   nf_ZC0 动力煤    空（合约无报价）
hf_CL  纽约原油   103.156    hf_GC  纽约黄金   4006.366
```

- 覆盖 `chain_relation_candidate` 的高频品种：钢材→RB、铜→CU、电解铝→AL、
  黄金→GC/AU、天然气→外盘（无内盘对应）、动力煤→**无报价**；
- 电力/纸张/医药中间体/电子元器件这类**没有期货合约**，只能找现货指数（生意社/百川），
  HTML 抓取脆弱，先不做；
- 下一步（未做）：建 `commodity_price_daily(code, name, price, source, date)` +
  品种映射表，日更落库；无合约品种保持 NULL 并标注「无源」，不补造。

5.2 成交偏差：**脚本可用，样本为 0**（`scripts/analyze_entry_deviation.py`）。

```text
entry_proposed 事件 37 条（10-21 33 条 / 10-22 4 条）
最新持仓快照 2026-10-22T23:35+08:00
可测样本 0：区间内 0 / 低于下沿 0 / 高于上沿 0
未纳入：老仓 0 / 提案后快照里没有该票 37
```

结论：**10-21 的 33 个提案，到 10-22 收盘为止一个都没进快照** —— 也就是
「提案给的价位」与「实际操作」这两组数据至今零交集。这不是脚本问题：
10-23 没有导入快照，最后一根快照是 10-22 23:35。
下一步：等下一份快照导入后重跑，才能谈「给价偏高/偏低」；在此之前不上报表。

### 跨端依赖（Windows / ExternalV1，2026-10-22 路由盘点）

外发文档：`docs/handoff-windows-externalv1-20261022.md`（→ 对端回执
`…-response.md` → 我方答复 `…-macreply.md`）。

**对端回执要点（2026-10-22 15:40–15:56 mTLS 实测）：**

- **P0（新增，比原第 1 项紧急）**：push2.eastmoney.com 家族被源出口 IP（116.237.144.102）
  端点级拦截，自 10-21 20:38 起。影响 BoardConstituents(Eastmoney) / BoardFlows(f3/f62) /
  MarketRankings / FundFlow 分钟线 / BoardMemberships(Eastmoney)。实测我们 10-22 的
  f62 榜落库 **0 行**，f3 新浪臂 175 行正常。对端三选一（等/换 IP/IPv6），我们选等。
  **原判断「板块榜 f3/f62 均已通」作废。**
- 原第 1 项（5 个板块失败）**责任在我们**：这 5 个板块只有新浪码 `gn_*`、无 BK 映射，
  我们的请求按设计回退到 `tdx:industry:<名>`，TDX 两个目录里没有这些名字。
  修正：只有 `BK…` 码的板块走 Eastmoney 臂，`gn_*` 不再回退 TDX。
- 第 2 项 no_last_traded_price：对端 63d488c 已于 10-22 09:00 上线并按契约发码
  （消息形如 `no last-traded price … N instrument(s): codes`，retryable=false）；
  我们 09:00 后日志仍 0 次——**通道归属待确认**（我们行情走本机桥，可能不是直连 ExternalV1）。
- 第 3 项 typed instruments：字段 12 可用但需对端 Gate A。**operator 拍板 = 要**，
  请对端走 Gate A 增加 `repeated string instruments = 12`；字段上线前我们维持二分隔离，
  不另做文案解析。字段上线后：解析 instruments → 剔除缺价码 → 重试（调用量 2N → 2）。
  **Gate A 已拍（2026-10-22）**：D1a / D2a / D3a / D4a / D5a，附三条补充要求
  （顶层字段=最终失败臂列表、交付附 Gate C 真实样本、文案逐字节不变）——
  见 `docs/handoff-windows-externalv1-20261022-gatea-decision.md`。
  交付后我方动作：更新 vendored proto 并重生成客户端 → Gate C 样本验证解码 →
  切换为列表剔除。
- 第 4 项量比/成交额：`amount`（当日累计成交额）已在每条 quote 里；`volume_ratio` 仅竞价
  记录有。1.4 的量能条件改用「amount vs 5 日均额」，不追新字段。
- 第 5 项 HistoricalBars：根因是对端 Tencent 解码器不接受除权日第 7 元素（分红对象）；
  已修未部署。**我们的日线走本机桥**，需确认桥是否共用同一 crate；另：带 start/end 的
  请求会被 Tencent 臂整体拒绝，桥需改为「省略日期 + limit」。

真需要对端的（更新后）：
1. P0 恢复（等待）+ 恢复后按 BK 清单回记录数。
2. 本机桥 RealtimeQuotes/HistoricalBars 的通道归属答复（转发 or 桥内实现）。
3. `no_last_traded_price` 消息格式视为合同承诺（已答应）。

**不是对端问题的（明确排除）**：个股新闻断供（news_items 每日 570~690 条在进，
AI 评估 Mac 侧断）、公告通道未接线（Mac 侧）、竞价 op 61/涨停池已通。
