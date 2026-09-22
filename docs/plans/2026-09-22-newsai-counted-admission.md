# 修复计划: NewsAI counted 准入层 (2026-09-22 反查缺口 #1)

## 现状 (已核实的代码路径)

NewsAI 分析卡 (今早 55 张 "🧠 AI 新闻证据分析") 的投递链:

```
news_ai_shadow.rs ProductionNewsAiDeliveryPort::push
  └─ preflight_news_ai_analysis_v3 (notify.rs:2551)   [launch/audit/v14_gate_news_ai 门]
      └─ send_preflighted_news_ai_analysis_v3 (notify.rs:2604)
          └─ push_wechat_with_attempt_marker (notify.rs:2622)   ← 物理直推
          └─ v14_record_delivery (L7 analytics, sink=feishu)
```

- NewsAI 有自己的 BR-172 专用 durable 状态机 (reserve/begin/record, 独立表)
- **但没有 counted 准入**: 无预算竞争、无冷却头、无 delivery_decisions 决策行、
  无 30 槽分流语义 — 与 52 Unit 迁移的 counted 协调器是两套平行体系
- D-01 日推 funnel (dispatch_news_to_idea_daily → push_news_to_idea →
  push_counted_with_binding) 已 counted ✓ — 但今天候选池空未触发, 0 决策
- 评估 #4 (MU-news-ai counted binding) 落地的只是 v14_gate_news_ai 语境白名单
  (NewsAiCombinedAccount), 物理投递仍是直推

## 目标

NewsAI 物理投递前经过 counted 准入 (prepare envelope → 预算/冷却裁决 →
decision 行), 成功后 counted Delivered 收口。NewsToIdea kind 的 policy 行
(PerTicket 1200s Rolling, 计入预算) 复用为 NewsAI 的准入策略。

## 方案 (最小侵入)

1. `notify.rs` 新增 `push_news_ai_counted(text, delivery, reservation) -> NewsAiNotifyOutcome`:
   - 复用 `push_counted_with_binding` 的 prepare/admission 段 (envelope 材料:
     identity = delivery.identity().sha256(), code = target_code, business_date,
     PerTicket scope) — 不新造 binding 构造器, 参照 push_templates.rs:16822
     build_news_to_idea_counted_binding 的形态
   - 成功后返回 typed receipt, 调用方走既有 record_news_ai_delivered 收口
2. `news_ai_shadow.rs ProductionNewsAiDeliveryPort::push`:
   - send_preflighted_news_ai_analysis_v3 的物理段替换为 counted 投递
   - BR-172 专用状态机的 reserve/begin 保留 (与 counted 决策行并存, 双重
     审计; 或用 counted 决策行取代 begin 标记 — 二选一, 实现时定)
3. 预算语义: NewsToIdea counts=1 (分流规则: 盘中信息卡计入) — NewsAI 卡
   进入 30 槽竞争; 若用户想豁免改 policy 行 (决策点)
4. 测试: counted 准入拒绝 (预算满/冷却头) → NewsAI 不物理外发但状态机收口;
   准入通过 → Delivered 决策行 + typed receipt; 幂等 (同 identity 重入复用)

## 风险与前置

- 双重状态机并存期: counted 决策行 + BR-172 专用行, 审计口径需在 commit 里
  写清哪层是权威
- 若预算满 (今早 30 槽烧满类事故重演), NewsAI 卡会被 DailyBudgetFull 拒 —
  这是期望语义 (信息卡计入), 但需在报告里告知用户
- 需新 activation (src 变更) + 重启

## 验收

- 生产日志出现 NewsToIdea kind 的 counted observer 行 (decision=... Delivered)
  伴随 NewsAI 卡推送
- durable DB delivery_decisions 当日 NewsToIdea 行 > 0
- 预算满时 NewsAI 正确 RejectedDurable (不物理外发)

## 2026-09-22 反查补充核实 (LimitBoards / Announcement 排除)

- **LimitBoards (MU-limit-boards)**: counted dispatch 已接线
  (main.rs:10512 push_limit_boards_counted ×3 shape), 今日零产出根因 =
  竞价确认段 (9:20-9:25) 上游 LimitPools gRPC internal 全窗口失败
  (竞价案), 非接线缺口。上游恢复日自然产出。
- **S-01 公告 (MU-announcement)**: counted 接线在 source_fact 路径;
  今日 pushed=0 根因 = 前置漏斗全滤 (100 条: 86 分类排除 + 13 受众
  过滤 + 1 生命周期 + 87 已见重复去重), 无条目到达 push 段。属数据
  依赖行为, 非接线缺口; 若后续「公告永远 0 推送」持续多日需查漏斗
  分类是否过严 (单独排期)。
- **T-03 重试**: 已修 (5f68f2f, 每码 3 次/日上限)。
