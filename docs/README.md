# docs/ — 文档总索引

> **归档规范**: BR-029 文档演进路线归档规范（2026-07-11 落地）
> **结构**: 10 个版本文件夹（按演进顺序）+ 1 个演进前史归档目录 + 7 份根级文档
> **总文件数**: 168（不含 `.DS_Store` / `.pdf`）

---

## 版本演进路线

| 阶段 | 时间窗 | 主轴 | 文件数 | 入口 |
|---|---|---|---|---|
| **v9.x** | 2026-06-15 ~ 06-30 | 全项目设计 + 流程纪律 + P0 风控 + 已知 bug + 根因 | 31 | [README](v9.x/README.md) |
| **v10** | 2026-07-01 | 盘中监控与回顾 | 6 | [README](v10/README.md) |
| **v11** | 2026-07-02 ~ 07-04 | 口径不一致 / P0 系列改造 | 15 | [README](v11/README.md) |
| **v12** | 2026-07-02 ~ 07-06 | Trading Assistant + Push Templates 雏形 + 模板验证 | 12 | [README](v12/README.md) |
| **v13** | 2026-07-06 ~ 07-09 | Push Templates 实施发布 + B-002~B-007 bug 诊断 | 20 | [README](v13/README.md) |
| **v14.x** | 2026-07-08 ~ 07-11 | 数据源扩展（Sina/Baostock/QMT）+ v14.2 推送架构 + B-008~B-010 | 19 | [README](v14.x/README.md) |
| **v15.x** | 2026-07-11 ~ 07-12 | 推送治理与演进设计 | 5 | [README](v15.x/README.md) |
| **v16.x** | 2026-07-12 ~ 07-14 | 工程规则、风险与数据治理 | 14 | [README](v16.x/README.md) |
| **v17.x** | 2026-07-14 ~ 07-16 | 事件/推送迁移与持久化 | 11 | [README](v17.x/README.md) |
| **v18.x** | 2026-07-16 起 | 研究—模拟交易—复盘闭环与受控实盘准备 | 5 | [README](v18.x/README.md) |

## 演进前史归档

| 文件夹 | 内容 |
|---|---|
| [`_archive/pre-v9-history/`](_archive/pre-v9-history/) | v2-v7 架构演进 + v3-v6 项目计划 + 早期优化报告（已被 v9.x 取代，git 100% 可恢复）|

## 根级文档（不属于任何版本）

| 文件 | 用途 |
|---|---|
| `ENGINEERING_RULES_V2.md` | 工程规则 v2 |
| `business_rules.md` | 业务规则注册表（含文档归档 BR-029 与 v18 闭环规则 BR-038 ~ BR-042） |
| `业务规则清单-registry.md` | 业务规则中文清单 |
| `crontab.example` | crontab 模板 |
| `sina_baostock_integration.md` | Sina + Baostock 数据源接入文档 |
| `emquant-api-integration-plan-调研-2026-06-05.md` | EMQuant API 接入调研 |
| `EMQuantAPI_CPP_Mac.pdf` | EMQuant API 三方文档（macOS C++） |

## 文件命名规范（BR-029）

格式：`<版本>-<日期 YYYY-MM-DD>-<skill>-<作用>.md`

**skill 取值清单**（已使用 14 种）：

| skill | 含义 | 用途 |
|---|---|---|
| `brainstorming` | 设计/spec | 架构设计、流程设计 |
| `implement` | 实施/完成报告 | 编码实现 |
| `writing-plans` | 计划/排期 | 项目计划、dev-plan |
| `executing-plans` | 实施日志 | 实施过程日志 |
| `grill-with-docs` | 评审/审计/复盘 | 差距审计、复盘告警 |
| `review` | 评审/诊断 | 评审报告（被动方视角） |
| `requesting-code-review` | 评审请求 | 请求评审方视角（已停用，改用 `review`）|
| `diagnosing-bugs` | bug 诊断 | bug 根因分析 |
| `rootcause` | 根因归档 | 根因 E/F/G 专项 |
| `operations` | 发布/部署 | release-notes、deployment、broker 调研 |
| `changelog` | 变更日志 | 流程变更记录 |
| `progress` | 进度跟踪 | working 进度 |
| `benchmark` | 基准 | 性能基准 |
| `acceptance` | 验收 | mvp 验收 |

**活跃 spec 标记**: 文件名末尾加 `-active`（如 `v13.0-...-push-templates-spec-active.md`）。

## 当前活跃文档（引用优先）

| 版本 | 文档 | 路径 |
|---|---|---|
| **v13** | 推送模板 spec（活跃基线） | `docs/v13/v13.0-2026-07-05-brainstorming-push-templates-spec-active.md` |
| **v13.10.1** | 推送数据修正 + 降噪 release | `docs/v13/v13.10.1-2026-07-08-operations-release-notes.md` |
| **v14.2** | 推送架构 spec（当前活跃，2026-07-11） | `docs/v14.x/v14.2-2026-07-11-brainstorming-push-architecture-active.md` |
| **v14.x** | 主开发计划 | `docs/v14.x/v14.x-2026-07-11-writing-plans-master-development.md` |
| **v18.x** | 量化平台闭环中文整合设计（当前活跃） | `docs/v18.x/v18.0-2026-07-16-brainstorming-quant-platform-closure-design-active.md` |

## 整理动作记录

| 日期 | 动作 | 备注 |
|---|---|---|
| 2026-07-11 | 按 BR-029 首次落地 `docs/` 演进路线整理 | 移动 + 重命名 110+ 文件；新增 8 份 README；新增 BR-029 |
