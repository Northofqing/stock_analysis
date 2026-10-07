# 完整平台合入主仓库与上线状态（2026-10-07）

## 用户目标与已完成合入

用户本轮明确要求“全部合入主仓库 全面上线”。原新闻窄候选只保留准备记录，本轮按完整平台推进。

已将本聊天已完成的平台完整开发分支 `codex/platform-roadmap-implementation-20261002` 快进合入主仓库默认 `master` 并非强制推送：

- 合入前：`b7e12bb5c4729a7956124b2db121c7e338e21922`。
- 合入源码：`cde590dc09c4f01a0727200241489667c399bf59`，421 个提交、396 个文件；树 `bc85ab8d7ade717a16c7eab6edec44b3f57835a2` 与开发树完全相同。
- 实际 `ls-remote` 回读的远端 `master` 完整 OID 相同。主目录 `/Users/zhangzhen/Desktop/Quant/stock_analysis` 快进后干净。
- 远端 HEAD 实际指 `master`；本地缓存 HEAD 曾指 `main`，该分支仍是初始提交，不能据缓存选择发布分支。

本次将已完成的代码和其中明确保留的拒绝路径纳入主仓库。H01–H17 中未实现的资格、接线及真实证据仍按[剩余开发交接](../handoffs/2026-10-06-platform-remaining-work-handoff.md)验收；合入不改变这些状态。

## 本轮发布检查

1. 原 `cargo fmt --all -- --check` 退出 1；实际失败涉及 69 个 Rust 文件。后继按原输出限定这些路径执行 rustfmt，不手改业务逻辑、DDL、准入、阈值或 Rust 字面量；原失败和路径清单保留。
2. 421 提交的原 `git diff --check` 将两个封存 Windows RPC 输入的 CRLF 行尾报为尾空白。原件字节和哈希保持；补与其他封存合同一致的 `.gitattributes` 精确路径声明，以识别 CRLF，未全局放宽空白检查或修改 Git 配置。
3. `ruby scripts/architecture-docs/check.rb --check` 实际退出 1：旧蓝图/v19 文档输入 hash、current 证据路径集合与源码 hash 不一致，历史/current 目录仍是 `PROVISIONAL`，另有 RFC/HTML 差异。清单更新需要真实 source evidence 与审阅；不能直接改状态、删除旧材料或跳过 strict checker 取得成功。
4. GitHub Actions 仓库级开关原为 `enabled=false`，因此首次主分支 push 没有 CI。已按全面上线的验证需求恢复现有 CI，原 workflow/检查器/门槛保持；新 push 的实际运行和终态须单独回读，不把仓库开关打开算作 CI 通过。

先前定向验证可在原范围内复用（最新 H02 manifest 新 6 + 旧 V1 6 项通过，修复批次另有对应回执），不据此声称完整工作区、发布 CI 或新制品通过。仅格式和声明变更按 AGENTS 内容/diff/格式检查验证；真正发布行为还需下述 release 和业务验证。

## 生产现场与当前门禁

10:27 CST 只读快照：正式 monitor PID `14998`、bridge `56417`，均为原 launchd 实例；monitor SHA `a7376a14cde1f6be3025d6dbbdedf61401c34154f3da928a8e3db30c4950d20b`，activation 文件 SHA `3c8b49dba568a9e4530ce5598802ea6e6e6ba7e015ffd0fdcd297ea5c0de16b5`。实际运行根 `/Users/zhangzhen/.local/share/stock-analysis-runtime`，源码仍 `f517f45c91ec9489f63d0156a1b8f9cf45400c3b`。

- 两库 dev/ino：main `16777220/154379673`，durable `16777220/154266271`；durable schema 9，995 Delivered、3988 RejectedDurable、6 ManualResolvedRejected、78 UncertainManualReview。
- 78 条按种类：DataMode 73、CloseCall 3、T0Advice 1、WatchlistTracking 1。实际组摘要 `2223d83d38c491b4c4bc6608f0636aca939b454bc57bfecc10760e9fb6f63290`；未裁定、删除或重发。
- 健康进程/heartbeat/snapshot 新鲜，但业务 `Frozen/Unsafe`，账户指标不完整；缺 Quote、MoneyFlow、News、OrderBook。当前 news_dedup 表 0 行，不能伪造跨午夜现场验收；global critical 审计 1932 条也不等于全部推送验收。
- 实际 VM 为旧 `4e4995`，完整 Mac 开发版绑定较新的 ExternalV1。Windows `7174f09f0b34d082bc584180ba4260d53f379d6c` 的原 CI critical `35392/39817=88.89%` 未达 95%；overall `84.96%` 通过其 80% 门槛。新 WIP/离线测试和不同版 Health 不授权切换。
- 原 schema14/v1-v2 fallback `caafa228` 三目标 release 制品和隔离 dry-run 回执存在；未据此取得当前真实库的兼容回退、迁移及 activation 资格。v2 历史写入后禁止直接换回旧 v1 reader 或恢复旧数据库。

## 后续发布顺序与 owner

1. **Mac**：修实际主仓库验证问题，保历史 source evidence 与失败，取得原 CI/检查终态。
2. **Windows 原任务**：已成功同步用户全面合入/上线授权，继续验证过的源码提交、上游主仓库合入、同 HEAD 原 CI 和真实关键覆盖率修复；不得降低 95%。Bash CPUID 机制证据不等于运行库修复。
3. **双方**：合格新 SDK source/binary/descriptor 原 tuple → Mac 精确重绑 → Desktop 外完整 normal release → 同制品隔离 dry-run → 兼容 schema14/v2 回退与迁移方案。
4. **发布 owner**：形成候选/回退、输入清单、配置 hash、future9 activation 精确审阅单。工具和 CLAUDE 要求人工审阅，旧 Wave0/Wave1 批准不覆盖新候选。
5. **切换前**：重查 PID/lease/全部 writer、真实源/水位/数据、账户/资金/seed、Uncertain 人工决策及两库身份；动态门失败保现网和原数据。按 launchd 单实例流程切换，禁止 nohup 或双 physical owner。
6. **切换后**：等待实际 DB 初始化、桥接重连、配置生效、同版真实业务 RPC/消费/回执与自然观察，才记录对应 Production Verified。M0–M7 全部达标及 M8 实施/不实施裁定前 heartbeat 保持。

需要明确区分：全部已完成源码合入、发布制品验证、正式服务切换、全部能力验收。当前完成第一项；现网未执行本轮完整平台安装或重启。

详细本机证据在 `.planning/2026-10-07-full-platform-rollout/`，原合入回执另存 `.planning/2026-10-07-news-dedup-rollout/validation/master-integration-receipt.json`；不提交私有配置、数据库、Token、证书或大包。
