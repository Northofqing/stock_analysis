# v19.x — Operational Clarity

> **状态：** 设计阶段，待评审
>
> **主轴：** v18 在写 spec 但 src/ 没落地 → 仓库用 `log::error!` 假装有审计 → 运行 8 小时 80% log 是噪音。v19.x 改换思路：**先治运行体验（停得下来、看得到、恢复得了），再回头补架构**。
>
> **规则基线：** `AGENTS.md` §§2.1–2.10、`docs/ENGINEERING_RULES_V2.md` §§1–2、`CLAUDE.md` Completion Rule + Spec Evidence Rule
>
> **痛点证据：** `/private/tmp/stock_analysis_monitor.log`（19,843 行 / 7h49m）

## 推荐入口

[v19.0 Operational Clarity 设计](v19.0-operational-clarity-design.md) 是当前主设计文档。它整合痛点实证、设计原则、版本划分、PR 拆分与核心模块设计。

## 文档清单

| 文档 | 作用 | 状态 |
| --- | --- | --- |
| [v19.0 设计](v19.0-operational-clarity-design.md) | 主设计：痛点 + 4 版本 11 PR + 4 个核心模块 | 评审中 |
| [Push Template Catalog](push-template-catalog.md) | 57 个 PushKind、生产接线、治理与审计边界 | 现状审计 |
| [v19.1 复盘增强](v19.1-review-enhancement.md) | SignalTracker + R10 信号验证，从投递审计升级为业绩复盘 | 新增 |
| [v19.2 AI 改进](v19.2-ai-analysis-improvement.md) | AI 分析可回测验证、统一 LLM 基础设施、清理死代码 | 新增 |
| [v19.3 全天推送工作流](v19.3-push-workflow.md) | 盘前/竞价/盘中/盘后/全天 5 时段推送组织；BR-223 补齐 4 族断线模板 | ✅ 已实施 |

## 演进位置

- v18.x 不动（spec 保留，作为远期架构目标）
- v19.x 是落地前置条件
- v20+ 才回头把 v18 spec（DataEnvelope / AuditJournal / DecisionRecord）落地到 src/

## 不可妥协的边界

v19.x 不重写 v18 spec，不引入新业务规则，不动阈值与配置。**只让现有系统在运行层面不再痛**。

## 与既有工作的关系

- 复用 v17.x L1–L7 推送管线（结构不动，加 quiet mode 与 test isolation）
- 复用 v18 active §6.3 的"WORM ≥ 5 年"承诺（v19.x 承认 P0 不满足，仅作 v18.5 目标）
- 在 v19.x 完成后再回头评估 v18 active §16 Gate P 的量化条件

## 痛点驱动（实测，非想象）

| 痛点 | log 证据 | 治 |
| --- | --- | --- |
| log 噪音 80% | 19,843 行 / 7h49m | v19.1 PR-4 轮转 + PR-3 错误码聚合 |
| 系统 24/7 跑 | "等待交易时段" 散布全文 | v19.0 PR-1 Quiet Mode |
| Banner 是 UI 但 UI 坏 | banner unavailable 1,587 次 | v19.0 PR-2 BannerSnapshot |
| 数据源无熔断 | "一致预期 data 为空" 8,172 次 | v19.2 PR-7 CircuitBreaker |
| 测试/生产边界不清 | `--test` 与生产 log 同文件 | v19.3 PR-10 `--test` isolation |
| 无 dashboard 只有 log | 1.9MB 单文件 grep 5 秒 | v19.1 PR-5 `--health` CLI |
| log 不轮转 | 1.9MB 单文件 | v19.1 PR-4 轮转 |
| 错误粒度不够 | 全 `[BR-113]` 字符串 | v19.0 PR-3 StructuredError |
| 健康失败也静默 | log:31 webhook 未配置 | v19.2 PR-9 多层通知 |
| Banner / DataMode 不同步 | log:19832-19833 | v19.0 PR-2 BannerSnapshot 集中 |
