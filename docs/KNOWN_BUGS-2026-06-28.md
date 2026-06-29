# 已知 Bug 列表 — 2026-06-28

> **最后更新**: 2026-06-28
> **范围**: 项目当前所有"未修"但**已知**的 bug

---

## 一、🔴 高优先级

### B-001: `test_ledger_roundtrip` 并发 flaky

**位置**: `src/portfolio/store.rs:280`
**表现**: 并行 `cargo test --lib` 时 5% 概率 `database is locked`
**根因**: SQLite 全局单例 + 多测试并行写同一 DB
**修复**: 每个 test 用独立 temp DB, 估时半天
**量化 PM**: CI 不稳, P0 修复

### B-002: 科创板日报 (行业垂媒) 缺失 — CLS 只抓主站电报

**位置**: `src/search_service/providers/cls.rs:54`
```rust
// 当前: 只抓 refreshTenTelegraph (主站电报)
let url = format!("https://www.cls.cn/api/cache?lastTime={}&name=refreshTenTelegraph", now);
```
**表现**: 
- 科创板日报是财联社旗下**独立垂直媒体** (聚焦半导体/新能源/AI 等硬科技), 走的是独立频道
- `refreshTenTelegraph` 主电报流**不包含**科创板日报的文章
- 6/27 半导体 CO2 激光技术突破等"硬科技行业新闻" → **系统永远拿不到**
- search query 是通用宏观锚点 (`"今日 A股 重大新闻 政策 产业"`), 不包含行业关键词 (半导体/CO2/光刻/晶圆)
- 巨潮 (cninfo) 抓交易所公告 → 不含行业媒体
- 上交所/深交所抓监管/IPO 公告 → 不含行业媒体

**量化 PM 影响**:
- v9 事件抽取②**永远拿不到**"技术突破"、"产能变化"等核心 EventType 的新闻
- dual_score 评分③ chain_score 缺数据, event_risk_score 封顶 70 (损失 30% 信号)
- 半导体/新能源/AI 等 3 个行业的事件驱动信号源**缺失**

**修复方案**:
1. 新增 `KcbDailyProvider` (科创板日报 RSS/API), 估时半天
2. search query 列表加 `"硬科技 半导体 芯片 光刻 晶圆 CO2 激光"` 维度 (5 个变 6 个), 估时 1 小时
3. 行业垂媒从 P1 提到 P0 (v9 前置依赖)
**估时: 1 天, 量化 PM 评级 P0**

### B-003: 4.9/4.10 函数实现但未接日报/周报

**位置**: `src/portfolio/store.rs:118-200` (`live_rolling_sharpe` + `strategy_correlation_matrix`)
**表现**: 函数写好但没有调用方
**修复**: 在日报末尾追加 rolling Sharpe, 周报加策略相关性矩阵, 估时 2 小时
**量化 PM**: 文档承诺债, P0 修复

### B-004: benchmark 7000 天拉取可能超时

**位置**: `src/pipeline/backtest_runner.rs:603, 718`
**表现**: 7 年回测拉一次, 失败时 fallback 静默 None
**修复**: 改成 365 天分页循环 + 显式 warn, 估时 2 小时
**量化 PM**: 回测结果缺基准不可信, P1 修复

---

## 二、🟡 中优先级

### B-005: config magic numbers 散落 (P3.1 修复不彻底)

**位置**: 多处 (`cross_source_count`, `winrate_pct` 等仍是 const 写死)
**修复**: 全局 `grep "pub const"`, 全部移到 `risk.toml`, 估时 1 小时
**量化 PM**: 边缘项易遗漏, P1 修复

### B-006: `lead_days` 字段未真正应用

**位置**: `src/opportunity/bom_kb.rs:42` (定义), chain_score 使用处 (未用)
**修复**: chain_score 加 `exp(-lead_days / 30)` 衰减, 估时 2 小时
**量化 PM**: 精度提升 5-10%, P2 修复

### B-007: `monitor/news_monitor.rs` 仍有死分支

**位置**: `src/monitor/news_monitor.rs:142` (修复 v9.3 codex review C-6: 抄错, 实际不在 event_bus.rs)
```rust
_ => {}  // 不可达
```
**修复**: 加 `#[deny(unreachable_patterns)]`, 估时 1 小时
**量化 PM**: 不会崩, P2 修复

### B-008: e2e 测试缺 notification 覆盖

**表现**: e2e 只覆盖 score/launch_gate, 不覆盖 notification 集成
**修复**: 加 notification end-to-end, 估时 1 天
**量化 PM**: 测试覆盖缺口, P2 修复

---

## 三、🟢 低优先级

### B-009: P1.4 动态滑点 σ/ADV 数据接入留作 v3

**位置**: `src/strategy/core.rs:466-489` `compute_dynamic_slippage()`
**表现**: 函数框架有, 永远 fallback 固定值
**修复**: K 线计算 20 日 σ + ADV, 估时 1 天
**量化 PM**: 小盘股回测仍不实, P3 修复

### B-010: 通用 dead code 扫描

**位置**: 多处 `if x { Y } else { Y }` 死分支
**修复**: clippy `#[deny(unreachable_patterns)]`, 估时 2 小时
**量化 PM**: 不会崩, P3 修复

### B-011: 文档承诺 vs 实际微差

**案例**:
- `src/notification/mod.rs:7` (修复 v9.3 codex review C-7: 之前引用 `docs/notification/mod.rs:7` 是幻象文件) 文档说"多渠道推送：企业微信、飞书、Telegram、邮件、Pushover" (5 个), 现在 `src/notification/config.rs::NotificationChannel` enum 实际有 10 个 (Wechat/Feishu/Telegram/Email/Pushover/Custom/ServerChan/DingTalk/Slack/Discord, P0-0 修过但 mod.rs 头部注释没更新)
- v9 流水线设计已落地但主实施未动

**修复**: 同步 `src/notification/mod.rs` 头部注释 + 移除幻象引用, 1 小时, P3

---

## 四、按优先级修复路径

| 优先级 | Bug | 估时 |
|--------|-----|------|
| **P0** | B-001 测试 flaky | 半天 |
| **P0** | B-002 科创板日报缺失 | 1 天 |
| **P0** | B-003 函数未接日报/周报 | 2 小时 |
| P1 | B-004 benchmark 超时 | 2 小时 |
| P1 | B-005 config 集中彻底 | 1 小时 |
| P2 | B-006 lead_days 应用 | 2 小时 |
| P2 | B-007 event_bus 死分支 | 1 小时 |
| P2 | B-008 notification e2e | 1 天 |
| P3 | B-009 σ/ADV 接入 | 1 天 |
| P3 | B-010 dead code 扫描 | 2 小时 |
| P3 | B-011 文档同步 | 1 小时 |

---

## 五、Bug 追踪表

| Bug | 发现时间 | 来源 |
|-----|---------|------|
| B-001 | 2026-06 早期 | master 分支 |
| **B-002** | **2026-06-28** | **用户提供新闻截图 (科创板日报 半导体 CO2)** |
| B-003 | 2026-06-28 | v9.1 验收 |
| B-004 | 2026-06-28 | v9.1 plan 阶段 A |
| B-005 | 2026-06-28 | P3.1 修复不彻底 |
| B-006 | 2026-06-28 | v9.1 plan 阶段 B |
| B-007 | 2026-06-28 | v9.1 实施 |
| B-008 | 2026-06 | master 分支 |
| B-009 | 2026-06-28 | P1.4 半成品 |
| B-010 | 2026-06-28 | 多文件残留 |
| B-011 | 2026-06-28 | 文档同步债 |
