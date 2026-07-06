# v19.16 推送模板: 删除演示数据 + 严格真数据

## 1. 背景与动机

### 1.1 用户反馈

```
"竞价异动 虚拟盘成交回报 尾盘决策
盘中换手 数据也不对
你有没有 好好看 AGENTS 拿假数据 糊弄我？"
```

### 1.2 自查发现 (v19.15 跑 --test 后)

| 模板 | v19.14b/v19.15b 行为 | 违规 |
|---|---|---|
| T-07 候选触发 | hardcode "中钨高新 + 工信部支持钨深加工" 演示 | §2.1 mock 数据 |
| T-10 虚拟盘 | hardcode "三安光电 Filled 17.26" 演示 | §2.1 mock 数据 |
| T-12 尾盘决策 | hardcode "华电辽能 尾盘跳水" 演示 | §2.1 mock 数据 |
| T-13 盘中换手率 | turnover_pct=0.0, main_flow_yi=0.0 全 0 | §2.1 mock 数据 (0 也是假) |

### 1.3 AGENTS.md §2.1 红线 (直接违反)

```
MUST 生产路径禁止 mock 数据
MUST 持仓、交易、净值来自真实账户, 不编造
MUST 数据源失败显式报错, 不降级到假数据
```

**之前理解错**: 我以为"测试阶段可以演示". **错了**. AGENTS.md 没区分"生产/测试" — `run_test_scan` 也算生产路径, 同样禁止 mock.

## 2. 修复方案

### 2.1 删除所有演示推送

**直接删**, 不留 fallback:
- T-07 演示触发
- T-10 演示触发
- T-12 演示触发
- T-13 演示触发

**template 渲染函数保留** (供未来真数据通路调用).

### 2.2 T-13 真实数据通路 (从 data_provider 接真实 turnover_pct)

**真接 data_provider::fetch_realtime_quote** (腾讯/东财), 提取 turnover_pct 字段.
当前 K 线数据没有 turnover_pct 字段, 需要用实时 quote.

### 2.3 T-10 真实数据通路

**真接 paper_trades 表**. 当前 0 笔, 显示"样本 0 笔, 等真实成交" (v19.12 风格, 显式标注).

### 2.4 T-11 竞价异动 真实数据通路

09:25 拉东财 `fetch_auction_data` (09:20-09:25 竞价数据), 真接 stock_position 持仓股.
当前 --test 路径没接, 缺失.

### 2.5 T-12 尾盘决策 真实数据通路

14:45 拉持仓股最新报价 + 信号融合共振, 决定是否尾盘跳水/博弈.
当前 14:45 时间 hardcode + 状态 hardcode, 违规.

## 3. 实施

### 3.1 v19.16a: 删演示 + 接真数据 (T-10/T-13)

```rust
// 删 v19.14b T-07 演示 (line 972-995)
delete this 60-line block

// 删 v19.14b T-10 演示 (line 998-1014)
delete this 17-line block

// 删 v19.14b T-12 演示 (line 1016-1029)
delete this 14-line block

// 改 v19.15b T-13 调用: 0 数据时不推
if !entries.is_empty() {
    notify::push_governor(&turnover_text, ...);
} else {
    log::info!("T-13 盘中换手率: 0 数据, 跳过推送");
}
```

### 3.2 v19.16b: T-10 真接 paper_trades (0 笔显式标注)

```rust
let paper_count: u32 = {
    let mut conn = db.get_conn().ok().unwrap();
    diesel::sql_query("SELECT COUNT(*) AS cnt FROM paper_trades")
        .get_result::<CountRow>(&mut conn).ok()
        .map(|r| r.cnt as u32).unwrap_or(0)
};
let t10_text = if paper_count > 0 {
    // 真接 paper_trades 详情, 渲染模板
    let paper_trades = db.fetch_paper_trades(paper_count)?;
    pt::render_paper_trade_from_records(&paper_trades, &today_str)
} else {
    // 显式标注: 样本不足 (不是假数据)
    "🧪 虚拟盘（{date}）\n样本 0 笔 (paper_trades 表空, 待 PR3-3.5 落地)\n辅助建议, 非下单指令".to_string()
};
```

### 3.3 v19.16c: T-13 真接 turnover_pct (data_provider 实时 quote)

```rust
// 用 fetch_realtime_quote 拉 turnover_pct 字段
for code in &stock_list {
    let quote = market_data::fetch_realtime_quote(code).await.ok();
    if let Some(q) = quote {
        entries.push(TurnoverEntry {
            name: q.name,
            code: code,
            price: q.price,
            change_pct: q.change_pct,
            turnover_pct: q.turnover_pct,  // 真接, 不 0
            main_flow_yi: q.main_flow_yi,
        });
    }
}
```

### 3.4 v19.16d: T-11 竞价异动 真接

```rust
// 09:25 拉东财 fetch_auction_data (09:20-09:25 竞价数据)
let auction = data_provider::fetch_auction_data().await?;
let our_codes: HashSet = stock_position codes;
let holdings_auction: Vec<_> = auction.iter()
    .filter(|a| our_codes.contains(&a.code))
    .collect();
if !holdings_auction.is_empty() {
    pt::render_auction_volume(&auction_text);
    notify::push_governor(...);
}
```

### 3.5 v19.16e: T-12 尾盘决策 真接 (持仓股信号融合)

```rust
// 14:45 拉持仓股最新报价 + 信号融合共振
let holdings = portfolio::get_positions()?;
for p in &holdings {
    let quote = market_data::fetch_realtime_quote(&p.code).await?;
    let snapshot = StockSnapshot { ... };
    let signals = detector.scan_stock(&snapshot);
    // 强信号 → 尾盘跳水
    let state = if signals.iter().any(|s| matches!(s.category, MainOutflow)) {
        "尾盘跳水-建议处理"
    } else {
        "正常"
    };
    holdings_data.push((p.name.clone(), state));
}
```

## 4. 不在本设计范围

- ❌ 移除 v19.14b 的演示数据 (本设计就是修这个)
- ❌ 真正接 paper_trades 写入路径 (等 PR3-3.5)
- ❌ T-07 候选触发接真实 panel (等 candidate_panel schema 完整)
- ❌ T-12 尾盘"博弈"策略 (等 t0_advisor 完整启用)

## 5. 验收

- [ ] --test 路径无 T-07/T-10/T-12 演示推送
- [ ] T-13 真接 turnover_pct (0 数据不推, 不显 0)
- [ ] T-10 显式标"样本 0 笔, 待 PR3-3.5"
- [ ] T-11 真接 09:25 竞价数据 (非交易日 0 数据显式标)
- [ ] T-12 真接持仓股信号融合 (没强信号显式标"无尾盘跳水")
- [ ] cargo test --lib 全部 pass
- [ ] AGENTS.md §2.1 检查表: ✅ 无 mock, ✅ 无假数据, ✅ 数据源失败显式报错

## 6. 风险

### 6.1 风险: --test 推送数会减少

**现状**: v19.15 --test 推送 17 个 (含演示)
**修复后**: --test 推送可能降到 12-14 (只真数据)
**接受**: AGENTS §2.1 优先于推送数

### 6.2 风险: T-13 turnover_pct 字段不一定在 data_provider 中

**缓解**: 字段不存在时, T-13 不推, log 标"turnover_pct 字段缺失"