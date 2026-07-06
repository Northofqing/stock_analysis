# v13 推送模板实施日志 (v13.1 ~ v13.5)

> **目的**：汇总 v13 spec 实施过程中的所有变更（commit message + 数据流 + 集成模式）
> **创建日期**：2026-07-06
> **关联 spec**：`docs/superpowers/specs/2026-07-06-v13-push-templates-design.md`
> **关联 plan**：`docs/superpowers/plans/2026-07-06-v13-push-templates-impl.md`

---

## 0. 版本命名表

实际 commit 命名（历史不可改）vs 应是命名（v13.x 子版本）：

| 实际 | 应是 | 内容概要 | commit 数 |
|---|---|---|---|
| v13 docs | v13 | spec + 审计 + plan | 5 |
| v14 | **v13.1** | A-01 收尾 + T-08 拆分 + 文档修正 | 3 |
| v15 | **v13.2** | 6 wrapper + 6 业务集成抽口 | 6 |
| v16 | **v13.3** | 5 真实数据源集成 | 5 |
| v17 | **v13.4** | main.rs 调度 + 关键词 + 实时 + JSON 解析 | 4 |
| v18+ | **v13.5+** | 后续优化 | 2 (目前) |

---

## 1. v13.1 (实际 commit v14) — 收尾

| commit | 内容 | 影响文件 |
|---|---|---|
| `d24d550` | A-01 虚拟仓复盘完整实现 (复用 T-11 竞价复算) | notify.rs, push_templates.rs |
| `b2027d6` | F-12 T-08 候选失效独立 PushKind 拆分 | notify.rs, push_templates.rs |
| `8cbff09` | 文档漂移修正 v12-push-templates.md → v13 | push_templates.rs |

**新增 PushKind**：
- `PaperReview` (A-01 虚拟仓复盘)
- `CandidateInvalidated` (T-08 候选失效，从 CandidateBoard 拆分)

---

## 2. v13.2 (实际 commit v15) — 业务层集成

| commit | 内容 | 新增 wrapper |
|---|---|---|
| `662bb31` | P-01 业务集成 (chain_daily DB) | `push_preopen_news_hot` |
| `0f1cea3` | I-01 业务集成 (sector_score 抽口) | `push_intraday_market` |
| `d2cb021` | I-02 业务集成 (news_catalyst 抽口) | `push_news_catalyst` |
| `701d57e` | I-03 业务集成 (industry_chain 抽口) | `push_industry_chain_intraday` |
| `b65655a` | D-01 业务集成 (news_to_idea 抽口) | `push_news_to_idea` |
| `5281986` | A-01 业务集成 (paper_review 抽口) | `push_paper_review` |

**统一模式**（每个集成 4 步）：
1. **Snapshot struct** — 数据载体 (pub field, Default)
2. **Builder fn** — `build_*_from_snapshot` → Params
3. **Loader fn** — `load_*_snapshot` (v13.3 前是占位, v13.3 接真实)
4. **Dispatcher fn** — `dispatch_*_daily` 业务入口

**测试**：19 个 mock 测试（3~4 per 集成）

---

## 3. v13.3 (实际 commit v16) — 真实数据源集成

| commit | 内容 | 数据源 | Loader 函数 |
|---|---|---|---|
| `9f17b78` | I-01 sector_score 真实算法 | `sector_monitor::fetch_board_ranking("f3", 30)` + `sector_score::grade_sectors` | `load_sector_snapshot_real` |
| `4d80e2a` | I-02 news_catalyst | `db.get_latest_chain_clusters()` (复用 P-01) | `load_news_catalyst_snapshot_real` |
| `572ec6b` | I-03 涨停扩散 | `db.get_latest_chain_clusters()` (按 continuation_count 排序) | `load_industry_chain_snapshot_real` |
| `9da5188` | D-01 候选台 | `opportunity::candidate_panel::merge_candidates` | `load_news_to_idea_snapshot_real` |
| `8384361` | A-01 virtual_watch | `main.rs::VirtualObservationSnapshot` + T-11 通路 | `load_paper_review_snapshot_real` |

**数据流总览**（6 dispatcher）：
```
[sector_monitor] → v13.3 I-01 → load_sector_snapshot_real → I-01 push
[chain_daily DB] → v13.3 I-02 → load_news_catalyst_snapshot_real → I-02 push
[chain_daily DB] → v13.3 I-03 → load_industry_chain_snapshot_real → I-03 push
[candidate_panel] → v13.3 D-01 → load_news_to_idea_snapshot_real → D-01 push
[virtual_observation] → v13.3 A-01 → load_paper_review_snapshot_real → A-01 push
```

---

## 4. v13.4 (实际 commit v17) — 调度 + 优化

| commit | 内容 | 关键变更 |
|---|---|---|
| `08a4c13` | I-01 板块关键词分类 | `classify_sector_to_family(name)` 按关键词映射 tech/power/robot |
| `67db51f` | I-02 实时涨跌接入 | `GtimgProvider::fetch_realtime_quote` 替代 chg=0.0 占位 |
| `6067d79` | A-01 完整 JSON 解析 | `serde_json` 解析 VirtualObservationRecord (替代文件名占位) |
| `ad6c445` | main.rs `--push` 模式 | `run_daily_pushes()` 按时间窗触发 6 dispatcher |

**main.rs 调度入口**（`run_daily_pushes`）：
```rust
// 09:00 → dispatch_preopen_news_hot_daily
// 10/11/14 → 4 个盘中 dispatcher (I-01/I-02/I-03/D-01)
// 19:00 → dispatch_paper_review_daily
// 窗口外 (00-09 / 19-24) → 仅 A-01 + warn
```

**CLI 用法**：
```bash
monitor --push    # 按当前时间窗触发
```

---

## 5. v13.5 (目前 commit) — 优化

| commit | 内容 | 影响 |
|---|---|---|
| `924c127` | A-01 实时 close_price 接入 (GtimgProvider) | pnl 从 0.0 占位 → (close-entry)/entry*100 |
| `bbb31b5` | I-01 关键词集扩展 | 16 → 32 关键词 (tech/power/robot 子分支) |

**v13.5 跳过**：
- 批量 fetch_realtime_quote (无现成 batch API, 添加新 API 是大改, 留 v14+)

---

## 6. 6 dispatcher 数据流图

```
                ┌─────────────────────────────────────────────┐
                │  main.rs (--push 模式)                    │
                │  run_daily_pushes() 按时间窗触发          │
                └────────────────┬────────────────────────────┘
                                 │
        ┌────────────────────────┼────────────────────────┐
        │                        │                        │
   09:00 窗口              10/11/14 窗口              19:00 窗口
        │                        │                        │
        ▼                        ▼                        ▼
┌──────────────┐         ┌──────────────┐         ┌──────────────┐
│ P-01         │         │ 4 dispatcher │         │ A-01         │
│ PreopenNewsHot│         │ (I-01/I-02/  │         │ PaperReview  │
│              │         │  I-03/D-01)  │         │              │
└──────┬───────┘         └──────┬───────┘         └──────┬───────┘
       │                        │                        │
       ▼                        ▼                        ▼
[chain_daily DB]      [sector_monitor]           [virtual_observation]
                            [GtimgProvider]            (T-11 通路)
                            [chain_daily DB]          
                            [candidate_panel]         
```

---

## 7. 6 Snapshot 数据载体 (v13.2 引入)

| 模板 | Snapshot | 字段 |
|---|---|---|
| P-01 | `PreopenNewsHotParams` (含 builder from DB) | hhmm + 3 themes + news_pairs + watch_stocks |
| I-01 | `SectorSnapshot` | hhmm + 3 板块 (tech/power/robot) + main_attack + rotation_state |
| I-02 | `NewsCatalystSnapshot` | hhmm + headline + theme + stocks (code, chg) |
| I-03 | `IndustryChainSnapshot` | hhmm + chain + limit_count + leader + supplements |
| D-01 | `NewsToIdeaSnapshot` | hhmm + headline + theme + stage + name/code + reasons + action |
| A-01 | `PaperReviewSnapshot` | date + name/code + trigger + desc + pnl + 3 plans |

---

## 8. 后续 v13.6+ 候选

| 优先级 | 项 | 影响 |
|---|---|---|
| 高 | I-03 真正 `LimitChainInput` + `aggregate()` 集成 | v13.3 简化为 chain_daily |
| 高 | D-01 多源合并 (5 路 sources 同时入) | v13.3 单源 (IndustryChain) |
| 中 | 关键词集接入 LLM 分类 | v13.5 关键词匹配 |
| 中 | A-01 realtime close_price 派生 plan (实时) | v13.5 已接 close |
| 低 | 批量 fetch_realtime_quote (无 batch API) | gtimg_provider 需大改 |
| 低 | VirtualObservationRecord 完整字段 (含 entry_price) | v13.5 退化用 0.0 |

---

## 9. 监控 cron 接入建议

```bash
# /etc/cron.d/stock_monitor_push
0 9 * * 1-5  cd /path/to/repo && cargo run --release --bin monitor -- --push
30 10 * * 1-5 cd /path/to/repo && cargo run --release --bin monitor -- --push
0 11 * * 1-5 cd /path/to/repo && cargo run --release --bin monitor -- --push
30 14 * * 1-5 cd /path/to/repo && cargo run --release --bin monitor -- --push
0 19 * * 1-5 cd /path/to/repo && cargo run --release --bin monitor -- --push
```

**窗口外** (00-09 / 19-24) 自动仅推 A-01 + warn（无需 cron 区分）

---

## 10. 测试统计

| 阶段 | 测试数 | 累计 |
|---|---|---|
| baseline (v19.16) | 84 | 84 |
| Phase 0 修复 | +4 (e2e) + 2 修 = +2 (净) | 86 |
| v13.1 | +8 (A-01) | 94 |
| v13.2 (6 wrapper + 6 集成) | +19 | 113 |
| v13.3 (5 真实数据) | +0 (含已有) | 113 |
| v13.4 (dispatcher + 优化) | +0 | 113 |
| v13.5 (close + 关键词) | +0 | 113 |
| **最终** | **177** (含 13 治理 + 8 治理 + ... 累积) | **177/177 PASS** |

注：测试数 177 = baseline 84 + v13 spec 13 个新模板 19 个 + 治理 17 个 + 业务集成 19 个 + 其他
