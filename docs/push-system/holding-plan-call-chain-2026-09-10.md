# 盘中持仓计划：三条旧入口与完成边界

日期：2026-09-10。源码基线 `8daa8bf2d8490807256a5bb6ed92b0a4ea8a5734`。本文件是只读源码核对，不是生产复现或迁移验收；没有运行 monitor、访问实际账户/行情/业务库或执行发送。完整限定调查见[证据记录](../../.planning/2026-09-10-holding-plan-call-chain/findings.md)。

后续状态：手动入口真实健康准备与失败退出已在初版 `de990c8`、最终 `41e7762` 修复。初审测试接口问题已修正，10 项新回归重新通过，7 项未改调度器测试证据保留，限定复审通过，见[实施记录](implementation-manual-push-bootstrap-outcome-2026-09-10.md)。下表与原问题保留审计基线及当时行号，不代表新源码仍有同样的手动错误；查看原实现应使用上方 Git 基线。日表、来源及重发资格缺口没有在此修复中消失。

## 精确身份

[MU-holding-plan](push-capability-catalog.v1.json#L8894) 的三个 producer 是 `holding-plan-periodic`、`holding-plan-manual`、`startup-resume-holding-plan`，主阶段盘中。可读 occurrence 是 `holding-plan:{date}:{code}`，但 counted decision 还绑定来源、subject、policy 和正文摘要；[实际策略](../../src/durable_delivery/model.rs#L409)为 PerTicket、Rolling 1800 秒并计日预算，不是 BusinessDateOnce。

## 实际调用顺序

| 入口 | 来源、发送与完成顺序 | 当前限制 |
| --- | --- | --- |
| 定时 | [盘中 1800 秒触发](../../src/bin/monitor/main.rs#L10890) → [准备整批提案](../../src/bin/monitor/main.rs#L8520) → 查日表并过滤 → presentation token → counted 投递 → 收集 Pushed/Deduped → [批尾写日表](../../src/bin/monitor/main.rs#L10950) → 全部确认才推进 timer | 日表写失败只记日志，不改变 confirmed；空 pending 批也推进 timer |
| 手动 `--push` | [按当前窗口分支](../../src/bin/monitor/main.rs#L1525) → Intraday 先读 banner → 五类 dispatcher，包括同一持仓准备/counting 链 → 汇总 failures → [无条件 Ok](../../src/bin/monitor/main.rs#L1651) → [主入口以 0 退出](../../src/bin/monitor/main.rs#L5330) | 不查/写 holding_plan_daily；当前新进程还会先因 banner 未初始化被拒绝 |
| 普通启动恢复 | [startup barrier](../../src/bin/monitor/main.rs#L4953) → [ensure_startup_reconciled](../../src/bin/monitor/durable_delivery_runtime.rs#L1166) → [跨日期恢复与有限次 resume](../../src/bin/monitor/durable_delivery_runtime.rs#L2074) → 同一 authoritative sink | 不重新 prepare，不回填日表；普通启动入口未按当前盘中窗口过滤 |

[notify counted 入口](../../src/bin/monitor/notify.rs#L2886)还检查呈现资格、launch gate 和治理；[durable envelope 构造](../../src/bin/monitor/durable_delivery_runtime.rs#L2199)绑定原正文及来源，再进入旧 durable prepare/reconcile/resume。已有 typed receipt 与 durable 路由不能被误说成裸 bool 发送，但也不证明 Foundation 已接管三个 producer。

## 审计基线已能由代码证明的问题

1. **新进程的盘中手动入口先读一个尚未初始化的 banner。** [LATEST_BANNER](../../src/bin/monitor/main.rs#L1672)初始为 None，[current_banner](../../src/bin/monitor/main.rs#L1680)明确返回错误；`--push` 分支在 [5316](../../src/bin/monitor/main.rs#L5316)，正常服务的健康评估写 banner 则在 [5468](../../src/bin/monitor/main.rs#L5468)之后。这里的 Intraday 是程序按当前时刻选出的窗口，不是新增 CLI 参数。不得用虚构 Normal/Full banner 修通；应接真实健康准备。即便补齐 banner，批内未确认仍只进入 failures，函数最终 Ok，会产生成功退出码。
2. **日表不是与投递原子提交的完成权。** [读取](../../src/bin/monitor/main.rs#L8461)在连接、建表或查询失败时返回空集合；[写入](../../src/bin/monitor/main.rs#L8495)吞掉错误。调用方在发送确认后才写表，写失败仍可推进 timer。代码允许“durable 已完成而日表未标记”的状态；manual/startup 又不维护此表，不能以它证明跨重启每日只推一次。没有据此宣称生产已重复发送。
3. **同 occurrence 可以产生不同 decision。** [正文时间](../../src/bin/monitor/main.rs#L8538)与[来源 observed_at](../../src/bin/monitor/main.rs#L8585)每次按本地时钟生成，[decision identity](../../src/durable_delivery/model.rs#L802)包含 source 与 rendered hash。即使 date/code/价格未变，不同调用也可能不是同一 decision；1800 秒冷却、周期日表和手动无日表三者不是同一个“每日唯一”合同。
4. **来源没有绑定到同一份可核验持仓和行情批次。** prepare 先读快照 A；[fetch_position_quotes](../../src/bin/monitor/market_data.rs#L175)又读最新快照 B 来选代码，缺失/过期时还可回退本地持仓代码。后续[行情投影](../../src/bin/monitor/market_data.rs#L219)仅返回 stocks，丢掉 TopStockBatch.evidence；最终 canonical 不含具体 snapshot identity/evidence/effective_at 或行情 batch/provider/source_at。正文确已冻结，但不能据此核验它来自哪份持仓、哪批行情。

另一个待明确的合同门是启动恢复时窗：catalog 主归属盘中，不等于恢复只能盘中执行。当前 barrier 不按当前 session/交易日过滤，可能继续处理既存 envelope；应按既有恢复、新鲜度与 owner 规则设计，不先定性为生产错发，也不能简单禁掉所有恢复。

## 后续最小开发方向

- 修通手动入口的真实健康准备与失败退出；使用隔离输入/受限 sink 验证，不运行生产 `--push`，不增加健康通知副作用或默认健康值。
- 一次固定用户持仓快照，按该快照精确代码取得保留 evidence 的行情批次；完整提案携带可核验来源，两条生成路径不能重新读取“最新”值。
- 依据下方已存在的裁决，不能将三个 producer 简化成“每日严格一次”，也不能允许无条件多发。下一步应明确何谓有效业务修订及再次发送资格，再统一持久完成权；不改 occurrence、冷却或重发权限来绕过这项缺口。

上述问题没有在 runtime schema guard 中顺带修复；其后的手动入口切片单独实现、取证，状态见文首链接。累计可指出实际旧入口与主要完成门的 Unit 仍为 10 个，另 42 个尚未逐链完成；这不是 10/52 的迁移完成率，也不是生产认证数量。

## 既有频次与恢复裁决补核

同日同票不等于同一 decision：[RFC 的 MU-holding-plan](push-system-implementation-rfc.md#mu-holding-plan--持仓计划)及[WBS 专属门禁](push-system-wbs.v1.json#L2271)要求 source/rendered hash 一致才共享 decision，并明确真实修订不能被日级展示名吞掉。[目录的 Unit 说明](push-capability-catalog.v1.json#L8894)也否定凭可读 occurrence 声称每日唯一。因此无需再次把“是否每日严格一次”当作完全未裁决的问题。

但这不授权所有 source hash 变化都再次推送。[目录 QS02](push-capability-catalog.v1.json#L3089)已将每次 `now` 引起的 canonical 变化、日表与 Rolling/1800 冲突列为缺口。业务有效修订的判定、仅时间戳变化是否算修订、何时重新取得发送资格，仍需落实合同；不能从哈希变动直接推断获准重发。

[启动恢复门禁](push-system-wbs.v1.json#L2286)明确只恢复原 immutable bytes/decision，不造新 occurrence，不以当前新来源替换既存载荷，也不授予新的生产资格；Uncertain 不盲发。本补核只解释既有设计，没有改写冻结 RFC/WBS/catalog 或修改重发规则。
