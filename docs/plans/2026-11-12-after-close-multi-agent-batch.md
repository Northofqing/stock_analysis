# 盘后多 Agent 研判批次（方案 A）— 2026-11-12

## 目标

在不改动 19:00 复盘 R-11、不产生交易/推送副作用的前提下，增加一个
日线落库后的个股研究批次，覆盖“当前持仓 + 少量 open 提案”。

## 范围与口径

- 触发：工作日 22:10（日线 21:30 落库后）；不是 15:25 的 R-11。
- 标的：
  - 最新 `user_position_snapshot` 的全部 code，按数量×成本降序；
  - `stage=entry_proposed`、最新 decision `open`，且无
    `entry_triggered/invalidated/expired` 事件的提案，按最新成交额取前 3；
  - 去重、截断到 9 只。
- 执行：`src/bin/deep_analyze_batch.rs`，复用
  `deep_analyzer::run_and_save`（6 工具 + 6 分析师 + 辩论 + 仲裁）。
  - 并发 2，单票 90 秒（默认），总预算 1200 秒；
  - 独立 probe DB `data/deep_analyze_batch_probe.db`，不把分析审计写主库；
  - 只写 `reports/details/{report_date}_{code}.md` 和同名 `.report.json`；
  - 不推送、不产生 `ticket_decision`/交易动作。
- 选择可用 `--dry-run` 预览，不调用模型。

## 验证（2026-11-12）

- `cargo check --bin deep_analyze_batch` 通过；解析单测 1/1 通过。
- 日线门：批次启动前要求 `stock_daily[business_date] >= 800` 行，否则直接失败，
  不用旧日线冒充当日；2026-11-12 17:45 实测为 0 行，按预期拒绝。
- dry-run（2026-11-12）：9 只 = 6 持仓（600396/001216/600667/600703/002131/002421）
  + 3 提案（002436/300757/605488）。
- 单票实跑 600396：33.9s，报告 `reports/details/20261111_600396.md`，
  sidecar 含 `composite=39`；`batch_2026-11-12.json` 记录 ok=1。
- 17:36 运行时最新日线是 11-11，所以报告日期是 11-11；22:10 跑才会用
  当日 11-12 日线。

## 监控与边界

- Cron：`deep-analyze-batch-after-close`（job `12f775c884e8`），工作日 22:10。
- 先 shadow 3 个交易日：看成功/失败、耗时、报告问题率；不推送、不改 R-11。
- 模型成本约 12 次 LLM/只（来自 R-11 代码结构，不是 token 账单）；
  9 只属于可接受的首批，不直接扩到全部 68 张票。
- 若 21:30 日线未完成，freshness gate 会拒绝；批次只记录失败，不重试。
