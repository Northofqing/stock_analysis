# docs/ — 文档总索引

> **归档规范**: BR-029 文档演进路线归档规范（2026-07-11 落地）
>
> **当前数据基线**: BR-158 / BR-159 / BR-164 / BR-168。公共金融和新闻采集必须进入 `src/data_gateway/**`；版本目录只记录历史。

## 当前执行入口

| 文档 | 用途 |
| --- | --- |
| [`ENGINEERING_RULES_V2.md`](ENGINEERING_RULES_V2.md) | Gate A→D、证据、失败和回滚合同 |
| [`business_rules.md`](business_rules.md) | 业务规则注册表 |
| [`superpowers/specs/2026-07-25-unified-data-final-cutover-design.md`](superpowers/specs/2026-07-25-unified-data-final-cutover-design.md) | 统一金融/新闻 Gateway 最终切换设计 |
| [`superpowers/specs/2026-07-29-config-readme-cleanup-design.md`](superpowers/specs/2026-07-29-config-readme-cleanup-design.md) | BR-181 runtime 配置所有权与 README 真实性设计 |
| [`integrations/magic-tdx-stock-analysis.md`](integrations/magic-tdx-stock-analysis.md) | Magic TDX 主源和统一 Gateway 接入边界 |

统一数据迁移当前仍是 **Gate B / In Progress**。文档中的能力清单不等于 Gate D 已通过。

Runtime TOML 由 monitor 启动时的 `src/config.rs::load_all()` 单次读取：
`config/strategy.toml` 和 `config/chain.toml`。`config/design_contracts.toml`
仅供合规检查；环境变量与 `.env` 是独立 runtime 输入。当前不支持信号热重载。

---

## 版本演进路线

| 阶段 | 时间窗 | 主轴 | 文件数 | 入口 |
|---|---|---|---|---|
| **v9.x** | 2026-06-15 ~ 06-30 | 全项目设计 + 流程纪律 + P0 风控 + 已知 bug + 根因 | 31 | [README](v9.x/README.md) |
| **v10** | 2026-07-01 | 盘中监控与回顾 | 6 | [README](v10/README.md) |
| **v11** | 2026-07-02 ~ 07-04 | 口径不一致 / P0 系列改造 | 15 | [README](v11/README.md) |
| **v12** | 2026-07-02 ~ 07-06 | Trading Assistant + Push Templates 雏形 + 模板验证 | 12 | [README](v12/README.md) |
| **v13** | 2026-07-06 ~ 07-09 | Push Templates 实施发布 + B-002~B-007 bug 诊断 | 20 | [README](v13/README.md) |
| **v14.x** | 2026-07-08 ~ 07-11 | 历史数据源实验 + v14.2 推送架构 + B-008~B-010 | 19 | [README](v14.x/README.md) |
| **v15.x** | 2026-07-11 ~ 07-12 | 推送治理与演进设计 | 5 | [事后复盘](v15.x/post-mortem-v15.1.1.md) |
| **v16.x** | 2026-07-12 ~ 07-14 | 工程规则、风险与数据治理 | 14 | [README](v16.x/README.md) |
| **v17.x** | 2026-07-14 ~ 07-16 | 事件/推送迁移与持久化 | 11 | [修订开发计划](v17.x/v17.x-dev-plan-revised.md) |
| **v18.x** | 2026-07-16 起 | 研究—模拟交易—复盘闭环与受控实盘准备 | 5 | [README](v18.x/README.md) |

## 演进前史归档

| 文件夹 | 内容 |
|---|---|
| [`_archive/pre-v9-history/`](_archive/pre-v9-history/) | v2-v7 架构演进 + v3-v6 项目计划 + 早期优化报告（已被 v9.x 取代，git 100% 可恢复）|

## 根级文档（不属于任何版本）

| 文件 | 用途 |
|---|---|
| `ENGINEERING_RULES_V2.md` | 工程规则 v2 |
| `business_rules.md` | 业务规则注册表 |
| `业务规则清单-registry.md` | 业务规则中文清单 |
| `crontab.example` | crontab 模板 |
| `emquant-api-integration-plan-调研-2026-06-05.md` | 历史 EMQuant API 接入调研；已被统一 Gateway 架构取代，不是生产实现说明 |
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
| **统一数据** | 金融/新闻 Gateway 最终切换（Gate A） | `docs/superpowers/specs/2026-07-25-unified-data-final-cutover-design.md` |
| **v13** | 推送模板 spec（活跃基线） | `docs/v13/v13.0-2026-07-05-brainstorming-push-templates-spec-active.md` |
| **v13.10.1** | 推送数据修正 + 降噪 release | `docs/v13/v13.10.1-2026-07-08-operations-release-notes.md` |
| **v14.2** | 推送架构 spec（当前活跃，2026-07-11） | `docs/v14.x/v14.2-2026-07-11-brainstorming-push-architecture-active.md` |
| **v14.x** | 主开发计划 | `docs/v14.x/v14.x-2026-07-11-writing-plans-master-development.md` |
| **v18.x** | 量化平台闭环中文整合设计（当前活跃） | `docs/v18.x/v18.0-2026-07-16-brainstorming-quant-platform-closure-design-active.md` |

## 历史交接（不得作为当前状态）

- [2026-07-22 monitor 观测与剩余工作交接](handoffs/HANDOFF_2026-07-22_MONITOR_AND_REMAINING_WORK.md)
  是当日历史快照；当前状态以活跃 spec、Git PR 和最新门禁证据为准。

## 整理动作记录

| 日期 | 动作 | 备注 |
|---|---|---|
| 2026-07-11 | 按 BR-029 首次落地 `docs/` 演进路线整理 | 移动 + 重命名 110+ 文件；新增 8 份 README；新增 BR-029 |
