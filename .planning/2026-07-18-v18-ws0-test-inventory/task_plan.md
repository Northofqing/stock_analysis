# v18 Workstream 0 测试盘点 计划

## 目标
盘点 v18 Workstream 0 在动 PR 前会触及的代码、单测和 codepath，并据此给出 PR 拆分建议与必查项。

## 阶段

- [x] 阶段 1：拿到 v18 重设计（前置对话）
- [x] 阶段 2：明确单测、跨模块调用、unwrap_or_default 分布
- [x] 阶段 3：测绘 codepath 与数据迁移影响
- [x] 阶段 4：把盘点写到 `.planning/2026-07-18-v18-ws0-test-inventory/findings.md`
- [x] 阶段 5：列出 PR 拆分顺序与必须回答的问题

## 输出
- 8 节 findings 文档：
  1. 关键数字概览
  2. 上下游 codepath 测绘
  3. 数据库迁移影响
  4. 推送集成测试影响
  5. PR 必附内容与拆分顺序
  6. 必须回答的 WS0 PR 问题
  7. 与 v18 active 文档的关系
  8. 后续盘点建议

## 约束
- 仅盘点，不改 v18 active 文档。
- 不动代码、不改阈值与业务规则。
- 推动 PR 拆分（W0a–W0f）作为对 active 文档 §15 的实质性补充。
