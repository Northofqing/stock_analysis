# v19.15 推送模板简化设计

## 1. 背景与动机

### 1.1 用户反馈 (4 个测试问题)

1. **A股市场预览需要去掉** — `--test` 路径中 generate_market_overview_text_blocking 调用
2. **新闻 Ranker 需要去掉** — `--test` 路径中 NewsRanker 演示
3. **龙虎榜盘中拿不到数据** — 用户建议"根据实盘换手率排序数据展示"
4. **信号复盘 MVP-02 和 MVP-03 注释看不懂** — 模板文字"做T建议: 推 0 条 [MVP-2 待启用]" 太术语化

### 1.2 现状 (v19.14)

| 模板 | 状态 | 问题 |
|---|---|---|
| A股市场概览 (无 PushKind, 通用) | --test 调用 | 用户不要 |
| NewsRanker | --test 调用 | 用户不要 |
| R-04 龙虎榜 | --test 调用 | 盘中无数据, 但模板硬编 "0 数据" |
| 信号复盘 R-05 | --test 调用 | 模板中 MVP-2/MVP-3 术语用户看不懂 |

## 2. AGENTS.md 红线分析

**问题 3 (龙虎榜盘中换手率替代)** 直接撞 AGENTS.md §2.1 红线:

```
MUST 数据源失败显式报错, 不降级到假数据。
MUST 生产路径禁止 mock 数据。
MUST 持仓、交易、净值来自真实账户, 不编造。
```

**用户建议的"用换手率排序"实际上是降级到假数据**:
- 龙虎榜 (LHB) 数据源 = 东方财富 API, 盘后 21:00 才更新
- 盘中真实情况就是 API 返回空
- 用换手率替代 = **用另一个数据源编造一个"龙虎榜"**, 违反 §2.1

**正确做法**:
- R-04 龙虎榜: 盘中显式说"今日无龙虎榜数据 (盘后 21:00 才更新)"
- **新加 T-13 盘中换手率 Top10 模板** — 这是真数据, 不冒充龙虎榜
- 两个模板分离, 用户清楚知道一个是真龙虎榜数据, 一个是真换手率数据

## 3. 设计方案

### 3.1 问题 1: 去掉 A股市场预览

**做法**: `--test` 路径移除 `generate_market_overview_text_blocking` 调用

**影响**: 
- R-02 盘面走向 仍然推送 (真接指数数据)
- 只移除独立的"市场概览"模板
- 注释保留 R-02 是替代品

### 3.2 问题 2: 去掉 NewsRanker

**做法**: `--test` 路径移除 NewsRanker 演示
**影响**:
- NewsRanker 模块保留 (供后续 news_monitor_loop 真实场景用)
- 只移除 --test 演示触发

### 3.3 问题 3: 龙虎榜 ≠ 换手率 (新增 T-13 模板)

**做法 A**: R-04 盘中显式标注"今日无数据 (盘后才更新)"
**做法 B**: **新增 T-13 盘中换手率 Top10 模板**, 跟 R-04 分离

T-13 模板字段 (v12 §14.x 新增):
```text
🔄 盘中换手率 Top10 (HH:MM)
  1. {name}({code}) 现价¥{price} 涨跌{chg:+.2}% 换手{turnover:+.2}%
  ...
数据源: 实时行情 (非龙虎榜)
辅助建议, 非下单指令
```

**为什么不合并 R-04**: 用户原话"龙虎榜拿不到可以根据换手率" — 实际是两个不同数据源. R-04 = 盘后席位数据 (真), T-13 = 盘中换手率 (真). 合并会让用户分不清是真龙虎榜还是真换手率.

### 3.4 问题 4: MVP-2/MVP-3 注释改清晰

**改前**: "做T建议: 推 0 条 [MVP-2 待启用, 当前占位]"
**改后**: "做T建议: 推 0 条 (v19.15+ 启用, 当前占位 — 详见 docs/v12-dev-plan.md §MVP-2)"

**改前**: "候选(影子): 样本不足 (MVP-3 待 ≥30 笔触发, 当前 0 笔)"
**改后**: "候选(影子): 样本不足 (转正需 ≥30 笔影子样本, 当前 0 笔 — 详见 v12-dev-plan.md §MVP-3)"

## 4. 实施步骤

### 4.1 v19.15a: 移除 --test 路径中 A股市场预览 + NewsRanker

```rust
// 删除
let overview_txt = tokio::task::spawn_blocking(|| {
    generate_market_overview_text_blocking()
});
push_wechat(&overview_txt).await;

// 删除 NewsRanker 演示 spawn_blocking
let ranked_news = tokio::task::spawn_blocking(|| { ... });
```

### 4.2 v19.15b: 新增 T-13 盘中换手率 Top10 模板

**push_templates.rs**:
```rust
/// v12 §14.1 T-13 TurnoverTop 模板渲染 — 字段顺序严格对齐 ...
pub fn render_turnover_top(hhmm: &str, entries: &[TurnoverEntry]) -> String {
    let mut out = format!("🔄 盘中换手率 Top10 ({} 盘中)\n", hhmm);
    for (i, e) in entries.iter().enumerate() {
        out.push_str(&format!(
            "  {}. {}({}) 现价¥{:.2} 涨跌{:+.2}% 换手{:.2}%\n",
            i + 1, e.name, e.code, e.price, e.change_pct, e.turnover_pct,
        ));
    }
    out.push_str("数据源: 实时行情 (非龙虎榜, 龙虎榜盘后 21:00 才更新)\n");
    out.push_str("辅助建议, 非下单指令\n");
    out
}
```

**main.rs --test 路径调用**: 用 `data_provider` 拉全市场换手率 Top 30 → 排序取前 10.

### 4.3 v19.15c: 信号复盘模板术语改清晰

```rust
// 改前
"T0_recommendations_pushed: 0" → 模板输出 "推 0 条 [MVP-2 待启用, 当前占位]"
// 改后
"T0_recommendations_pushed: 0" → 模板输出 "推 0 条 (v19.15+ 启用 — 详见 docs/v12-dev-plan.md §MVP-2)"
```

### 4.4 v19.15d: R-04 龙虎榜模板盘中显示数据状态

**不**替换龙虎榜内容, **只**优化说明:
```text
// 改前
"⚠️ 龙虎榜: 今日 + 历史 均无数据"

// 改后
"⚠️ 龙虎榜: 盘中无数据 (盘后 21:00 才更新)"
"如需盘中活跃票观察, 见 T-13 盘中换手率 Top10 模板"
```

## 5. 验收

- [ ] --test 路径无 A股市场预览 / NewsRanker 推送
- [ ] --test 路径有 T-13 盘中换手率 Top10 推送
- [ ] R-04 盘中显示"盘中无数据 (盘后才更新)", 不显示换手率
- [ ] R-05 信号复盘 "MVP-2/MVP-3" 注释改为清晰说明
- [ ] cargo test --lib 全部 pass
- [ ] 没有 AGENTS.md §2.1 违规 (无降级到假数据)

## 6. 风险与缓解

### 6.1 风险: 移除市场概览 + NewsRanker, 用户后悔

**缓解**: 模块代码保留, 通过 env var 开关可临时启用

### 6.2 风险: T-13 换手率模板数据源失败

**缓解**: data_provider::fetch_xxx 返空时, 模板显示 "⚠️ 数据源不稳定, 跳过", 跟 R-04 风格一致

## 7. 不在本设计范围

- ❌ 龙虎榜 API 切换到 24h 实时: 真实 API 没有 24h 数据, 这是事实
- ❌ 北向资金真实数据源: 当前新浪 API 假成功返 0, 等换东方财富 main API
- ❌ paper_trades 写入路径: 等 PR3-3.5 完整接入
- ❌ 候选转正 (MVP-3): 等影子样本 ≥30 笔触发

## 8. 时间估算

| 阶段 | 估计耗时 | commit |
|---|---|---|
| 4.1 移除市场预览/NewsRanker | 10min | v19.15a |
| 4.2 新增 T-13 换手率模板 | 30min | v19.15b |
| 4.3 信号复盘术语改清晰 | 10min | v19.15c |
| 4.4 R-04 龙虎榜显示状态 | 5min | v19.15d |
| 测试验收 | 15min | — |
| **总计** | **~1h** | 4 个 commit |