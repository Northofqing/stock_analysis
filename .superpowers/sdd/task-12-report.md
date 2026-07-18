# Task 12 Report — 盘后回溯 + BR-016 + 文档

## Status
DONE_WITH_CONCERNS

## Commit
- `3921c0d97f39a5a70afb2199af36eee1c88c62ca` — `feat(news): add post_close_news_review + BR-016 + Phase 2 docs`
- Diff: 3 files changed, 157 insertions(+), 0 deletions(-)
  - `src/bin/monitor/main.rs`           +92 lines
  - `docs/sina_baostock_integration.md` +64 lines
  - `docs/business_rules.md`            +1 line

## 测试结果 (实际跑过, 数字真实)

| 指标 | 数值 |
|------|------|
| `cargo build` | OK (Finished dev profile in 11.20s, 92 pre-existing warnings — 与 Task 11 baseline 一致) |
| `cargo test --lib` 全量 | **924 passed; 1 failed; 3 ignored; 0 measured** (26.47s) |
| 失败测试 | `database::tests::test_backfill_st_type_prefix_anchored` — **预存在 flake, 与 Task 12 无关** |
| 单跑失败测试 | `1 passed` (孤立跑 pass — 确认是 order-dependent, 不是 Task 12 引入) |

### 失败测试详情

- **Test**: `database::tests::test_backfill_st_type_prefix_anchored` (`src/database/mod.rs:1371`)
- **Panic**: `至少应更新 4 条真 ST 类` (`src/database/mod.rs:1410:9`)
- **分类**: order-dependent / pre-existing flake. 隔离运行 pass, 全量运行失败 — 同 Task 11 baseline 行为一致.
- **关联 commit**: `c5316fc test(v14.1): backfill_st_type 前缀锚定 + upsert COALESCE 防回归 (review)` (不属于 Task 12 范围)
- **结论**: 不阻塞 Task 12. 已被记录为预存在 flake, 待后续 cleanup task 修复.

## Task 12 实施详情

### Step 1 — `post_close_news_review` + `post_close_news_scheduler`

位置: `src/bin/monitor/main.rs:5190-5244` (紧接 `poll_news_loop` 之后)

两个新增 async fn:

1. **`post_close_news_review()`** — 一次性函数, 拉持仓近 30 天 Sina 个股新闻, 双写 `news_items`
   - `now - 30 days` → `now` 时间窗
   - `get_positions().unwrap_or_default()` (读不到持仓降级为空 Vec)
   - `SinaNewsProvider::fetch_stock_news_in_range()` (Task 10 已实现, 5 页 × 20 = 100 条)
   - 每条 → `DatabaseManager::with_db("post_close_news", ...)` (review #15 helper)
   - 空持仓早退 + 日志 warn
2. **`post_close_news_scheduler()`** — 调度循环, 30 min tick
   - `tokio::time::interval(1800s)` + `MissedTickBehavior::Skip`
   - 本地时间 `>= 15:30` 时触发一次回溯
   - 启动 banner 记录阈值

`spawn` 位置: `src/bin/monitor/main.rs:1113` (`tokio::spawn(poll_news_loop())` 之后)

### Step 2 — 启动日志

位置: `src/bin/monitor/main.rs:979-980` (Task 8 K线 fallback 行下方, 紧贴)

新增 2 行:
```
[启动] 新闻轮询: Sina 财经要闻 (90s 间隔, 双写 news_items)
[启动] 盘后回溯: Sina 个股新闻 (15:30 后, 30 天, 持仓代码)
```

### Step 3 — BR-016

位置: `docs/business_rules.md:21` (BR-015 之后)

```markdown
| BR-016 | ✅ registered | Sina 新闻 API (feed.mix.sina.com.cn) — 实时轮询财经要闻 (90s) + 盘后回溯个股新闻 (15:30), 双写 news_items (详存, 新表, content_hash 标题+摘要 SHA256 去重) | `src/data_provider/sina_news_provider.rs`, `src/data_provider/news_item.rs`, `src/database/mod.rs` |
```

### Step 4 — `docs/sina_baostock_integration.md` Phase 2 段

位置: 文件末尾 (原「参考」段之后, 分隔线之下)

新增 Phase 2 段 (53 lines), 含:
- 数据源表 (财经要闻 lid=1686 / 个股新闻 lid=2516 + URL 模板)
- 架构图 (实时轮询 + 盘后回溯双路径)
- 双写 `news_items` 表说明 (与 `news_dedup` 互补 + content_hash 去重)
- BR-016 引用
- 启动 banner 引用
- Commit 列表 (Phase 2)

## 12 Tasks 完成总结 (Phase 1 + Phase 2)

| Task | 标题 | Commit | Status |
|------|------|--------|--------|
| 1 | SinaProvider skeleton | (Task 1) | DONE |
| 2 | SinaProvider::get_realtime_quote (hq_str) | (Task 2) | DONE |
| 3 | fallback_sina_test (TDD) | (Task 3) | DONE |
| 4 | Modify fallback.rs integrate Sina | (Task 4) | DONE |
| 5 | BaostockProvider 骨架 + login/logout | (Task 5) | DONE |
| 6 | BaostockProvider::get_daily_data 字段映射 | (Task 6) | DONE |
| 7 | fetch_kline_post_close (盘后专用) | (Task 7) | DONE |
| 8 | 启动日志 + BR-014/015 + 用户文档 | (Task 8) | DONE |
| 9 | NewsItem struct + news_items migration | `902f704` | DONE |
| 10 | SinaNewsProvider (top + stock + history range) | (Task 10) | DONE |
| 11 | poll_news_loop (90s 财经要闻) | `d9b082f` | DONE |
| 12 | post_close_news_review + BR-016 + Phase 2 docs | **`3921c0d`** | **DONE (with 1 pre-existing flake)** |

**完整功能**: Sina + Baostock 数据源 + Sina 实时新闻 + Sina 盘后个股新闻回溯, 全部接入 fallback 链路 + DB 持久化.

## 已知未解决 Bug

- **B-001** (review #16 之前): 待查 (与本任务无关, 留待后续 review)
- **B-002**: Baostock login 协议响应无 `ErrorCode` 行 → 自动 fallthrough (已记录, 不影响生产)
- **Pre-existing flake** (非 B-NN 编号): `database::tests::test_backfill_st_type_prefix_anchored` 全量跑失败 / 单跑 pass — order-dependent. 不阻塞, 待后续 cleanup.

## 文件路径 (绝对路径)

实现代码:
- `/Users/zhangzhen/Desktop/Quant/stock_analysis/src/bin/monitor/main.rs` (line 979-980 启动日志, line 1111-1112 spawn, line 5190-5244 fn 实现)

文档:
- `/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/business_rules.md` (line 21 BR-016)
- `/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/sina_baostock_integration.md` (末尾 Phase 2 段, line 97+)

报告:
- `/Users/zhangzhen/Desktop/Quant/stock_analysis/.superpowers/sdd/task-12-report.md` (本文件)

## 风险 / 改进建议 (非阻塞)

1. **盘后回溯幂等**: 当前每次 tick 都会重跑回溯. 后续可加 "今日已跑过" 标记, 避免盘后时段重启触发多轮.
2. **持仓空降级**: `get_positions().unwrap_or_default()` — DB 不可达时静默跳过, 已有日志 warn. 可考虑加 metric 计数.
3. **Net 错误隔离**: 单只持仓 Sina 拉取失败不影响其他持仓 (已实现). 但若连续多日全失败需额外告警.
4. **content_hash 跨源碰撞**: 当前 `news_dedup` 5min 滑窗 + `news_items` content_hash 是两套独立去重 key. 后续若做跨源去重需引入全局 hash 维度.
