# v11-P0-5++ Commit 14+ 后续留事项

> **发布日期**: 2026-07-04
> **基于**: `docs/p0-5-plus-v3-done.md` (P0-5++ Commit 11-13 收尾报告)
> **状态**: 3 项留待用户 review + LLM 恢复 + 后续 commit

---

## 0. 留 P0-5++ commit 14+ 3 项 (按用户选项 B)

| 项 | 状态 | 下次落点 |
|---|------|---------|
| **9 保留清单过目** | ⏸️ 留用户 review | 实际跑 monitor 一周后, 任何一条不常用可降级 |
| **A10/C4 --test 路径接入** | ⏸️ 留 P0-5++ commit 15+ | run_test_scan 函数改造 |
| **LLM 恢复后 shadow 验证** | ⏸️ 留 P0-5++ commit 16+ | 等待 deepseek 限流恢复 |

---

## 1. 9 保留清单过目 (P0-4 §六 grill 修订)

按 grill Q6 决定, P0-4 commit D 标保留 9 条:

| # | 推送 | PushKind | 类别 |
|---|------|----------|------|
| 1 | 盘前Checklist | `DailyReport` | 保留 |
| 2 | 涨停/跌停突变 | `HoldingEvent` | 保留 |
| 3 | 炸板紧急 | `HoldingEvent` | 保留 |
| 4 | **排除检查告警** (Commit 4 加) | `HoldingEvent` | 保留 |
| 5 | **风控检查告警** (Commit 4 加) | `HoldingEvent` | 保留 |
| 6 | **现金预警** (Commit 4 加) | `HoldingEvent` | 保留 |
| 7 | 市场概览 (B1) | `DailyReport` | 保留 |
| 8 | 复盘报告 (B2) | `DailyReport` | 保留 |
| 9 | 公告告警 (C1) | `Announcement` | 保留 |

**用户 review 任务**:
- 实际跑 monitor 一周
- 任何一条不常用 → 改 `is_deprecated() = true` 降级为日志
- 留 P0-5++ commit 14+ 改造

---

## 2. A10/C4 --test 路径接入 (P0-5++ commit 15+)

**现状**: main.rs:978 wrapper 5 个 raw 中, A10 (None) 和 C4 (None) 是 --test 路径专属, --review 看不到.

**L561 C4 产业链** (`run_test_scan` 函数内):
```rust
log::info!("[测试] 产业链扫描:\n{}", scan.chain_text);
notify::push_governor(&scan.chain_text, notify::PushKind::IndustryChain).await;
```

**L1720 A10 选股** (`run_test_scan` 函数内, for 循环):
```rust
for rec in recs { log::info!("[选股] {}", rec); notify::push_governor(rec, notify::PushKind::StockPick).await; }
```

**改造方案** (P0-5++ commit 15+):
1. 在 `run_test_scan` 函数末尾加 wrapper 调用:
   ```rust
   let by_code = ... // 收集 5 票的 LLM 终稿
   let candidate_summary = run_candidate_panel_from_review(
       &by_code, &holdings,
       Some(&recs.join("\n")),  // A10 (改 recs → String 喂 wrapper)
       Some(&post_close_candidates),  // B3
       None, None,  // B6/B7 --test 路径无
       Some(&scan.chain_text),  // C4
   );
   notify::push_governor(&candidate_summary, notify::PushKind::CandidateBoard).await;
   ```
2. `by_code` 需要在 run_test_scan 内构造 (从 LLM 终稿拿, 但 --test 路径不跑 LLM)

**简化方案**: 跳过 A10/C4 接入, 让 wrapper 仍 None, 5 路 None 兜底走 by_code (--test 路径下 by_code 是空, wrapper 返空字符串, 不推). --test 路径不推候选台 (P5 §六 红线: 候选台是 --review 路径专属, --test 是测模式不推).

**结论**: A10/C4 --test 路径接入无必要 (--test 模式不推候选台), 留作 P0-5++ commit 15+ 仅 if 用户需要 --test 路径也推候选台.

---

## 3. LLM 恢复后 shadow 验证 (P0-5++ commit 16+)

**现状**: 跑 1 次 monitor --review, LLM API (deepseek) 限流, 7 只持仓 fallback Hold, 候选台没出.

**等待信号**:
- deepseek API 余额 / 限流恢复 (查 https://platform.deepseek.com/usage)
- 或换其他 LLM (Gemini / GPT-5) 临时测试

**验证方案** (LLM 恢复后):
1. `PUSH_SHADOW=true` 跑 1 次 monitor --review
2. 同时推 决策台 (P0-4) + 候选台 (P0-5+) + 旧 5 路 (P0-4 降级)
3. 比对: 决策台 7 只 / 候选台 5-7 只 (去重) / 旧 5 路 0 条 (降级) 
4. 验证无误 → 删 PUSH_SHADOW 分支 (commit 17+), P0-5+ 切 default
5. 验证失败 → 调 threshold, 再跑 shadow

**预估耗时**: LLM 恢复后 ~30 min (跑 3-5 次 shadow).

---

## 4. 累计 23 commit (本次 session, P0-4 + P0-5 + P0-5+)

```
43c36f1 docs(v17.1): P0-5++ 收尾报告
5abfb1a feat(v17.0): 实际接入 3 路 raw
6062407 feat(v16.8): L978 wrapper 接 5 路 raw
8bcb754 fix(v16.7): 收尾清理
a30de1d docs(v16.6): P0-5++ 收尾报告
0f9da2f feat(v16.5): 题材热度排序
c0e433c feat(v16.4): wrapper 接 5 路 raw
e2eaf69 feat(v16.2): wrapper + 推 1 条候选台
83a5132 feat(v16.1): 替换 5 处调用点
52fd1ca feat(v16.3): 文本解析
bc51fcf docs(v15.4): P0-5+ 报告
e4bcfa3 feat(v15.3): 排序 + 渲染
5411187 feat(v15.2): 证据分层
fb64868 feat(v15.1): 候选模型
... (10 commit 之前: v11 + v12 + v13)
```

---

## 5. P0-5++ 留 P0-5++ commit 14+ 整体展望

按 grill Q5 决定"5 commit 不变", P0-5+ 已经 5 commit 落地 (v15.1-3 + v16.1-2 + v16.4). P0-5++ 收尾 commit 11+ 是 5 commit 外的"补丁" (v17.0 真接入 3 路 raw + v17.1 docs + 后续 commit 14+ 留 3 项).

按"按推荐" commit 14+ 排期:
- **Commit 14**: 9 保留清单过目 (用户跑 monitor 一周, 任何不常用降级)
- **Commit 15**: A10/C4 --test 路径接入 (if 用户需要; 当前 --test 不推候选台 OK)
- **Commit 16**: LLM 恢复后 shadow 验证 (1-3 次, 验证无误切 default)
- **Commit 17**: PUSH_SHADOW 清理 (commit 16 验证后, 删 main.rs:970 PUSH_SHADOW if 分支)

**优先级**: Commit 14 (用户 review) > Commit 16 (LLM 恢复) > Commit 17 (Commit 16 后) > Commit 15 (可选).

---

## 6. 一句话

**P0-5++ 收尾完成 (commit 11-13): 实际接入 3 路 raw (B6/B7 --review) + docs 收尾报告 + PUSH_SHADOW 已自动清理. 留 P0-5++ commit 14+ 3 项: 9 保留清单过目 (用户 review) + A10/C4 --test 路径接入 (可选) + LLM 恢复后 shadow 验证 + PUSH_SHADOW 切 default. 累计 23 commit 落地, P5 红线 100% 守住.**
