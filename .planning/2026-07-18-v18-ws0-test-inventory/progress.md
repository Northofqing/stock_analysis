# 进度

## 2026-07-18

### 本日任务
盘点 v18 Workstream 0（DataEnvelope + DecisionRecord）的测试与 codepath 影响面。

### 已完成
- 完成 v18 active 文档的 12 项事实性 / 一致性审阅与 5 项落地导向设计建议（前置对话）。
- 用户选择"先做 Workstream 0 测试盘点"。
- 在 `.planning/2026-07-18-v18-ws0-test-inventory/findings.md` 输出完整盘点。

### 盘点关键数字（用于 PR 估算）
- 受影响 src 文件：17 个（库内 9 + 集成测试 8）
- 库内受影响单测：~185 个
- 集成测试受影响：≥15 个
- `unwrap_or_default` 红线命中：4 处高危 + 12 处置换候选
- `bin/monitor/` 总行数：27,793 行（main.rs 8,355、push_templates.rs 12,114、notify.rs 2,831）

### 决定
- 不修改 v18 active 文档；先回归到 active 文档本身补 3 条（§14 / §15 / §16）让 WS0 落地路径可执行。
- Workstream 0 拆 6 个 PR（W0a–W0f），每个独立通过。
- 数据库迁移仅追加新表，不动既有 `paper_trades` 等表行。
- P0 不删 `DataMode`，先叠 DecisionHealth layer。
- 推送 codepath 在 WS0 阶段保持 dual-path + 启动 banner，30 天后切换。

### 当前阶段
把盘点交给用户。本任务结束。
