# 推送 Foundation W12 实现与验证结果

**结论：** W12 已完成通用 Transport Authority Adapter。它把 W05/W07 已持久化并取得业务 lease 的 `Ready` intent，安全投影到现有 `DurableDeliveryCoordinator`；每个 decision 只绑定一个 required channel；投递后从 durable store 重新读取 envelope、attempt/fence、typed result、disposition 和 delivery audit，再交给 W09 做第二层 exact terminal verification。多渠道只通过独立 child intent 的强终态做纯归集，`Partial`、`Rejected` 和 `Uncertain` 均不能推进完成游标。

**生产边界：** W12 仍是 crate-private、零生产接线。没有修改 `src/bin/monitor`、真实 notification/sink composition、配置、Cargo 清单、生产数据库 migration 或任何 Unit physical owner；没有启动、重启或替换 monitor。现有推送仍走原路径，W12 通过不等于任何生产推送已切换。

**验证日期：** 2026-09-07（Asia/Shanghai）

**分支/worktree：** `codex/push-reliability-20260905` / `.worktrees/push-reliability-20260905`

**代码验证 HEAD：** `4e2839c6b351afc8797ff95ef265c122a53595c4`

## 1. WBS 验收结论

WBS 的 W12 验收句是：

> 逐 required channel 记录 typed 结果；Partial 拒绝游标，强 receipt 保留 exact bytes。

| 验收点 | 结果 | 代码证据 |
| --- | --- | --- |
| 每个 decision 一个 required channel | route、sink descriptor、Foundation envelope binding 和 Accepted receipt 四层绑定同一 `ChannelId` | `generic_transport.rs:52-94,162-244,315-343`；`model.rs:574-747,906-983` |
| typed 结果 | 只从 W09 验证后的 `DeliveryResult` 形成 channel observation；生产代码没有任意 observation constructor | `generic_transport.rs:345-511,516-532` |
| required set 精确 | required/observed 均要求非空、唯一、集合完全相等，并按 required 顺序输出 | `generic_transport.rs:534-607` |
| Partial 不推进 | `PartialRequiredChannels`、`RejectedRequiredChannels`、`UncertainRequiredChannels` 的 completion eligibility 固定为 `Never` | `generic_transport.rs:609-629` |
| COMPAT 不升级 | BestEffort、PartiallyAccepted、NoChannel、AllFailed、Blocked 均被强归集拒绝 | `generic_transport.rs:632-653` |
| 强 receipt exact bytes | durable 终态返回原 result canonical bytes；错渠道 receipt 转为 Uncertain 时，原 receipt canonical bytes 原样进入 evidence | `coordinator.rs:6072-6257,6285-6434`；`generic_transport.rs:315-343` |
| 无盲目重发 | 已有任何强终态先短路 sink；包括 `retry_authorized=true` 的 durable Rejected，重放总 sink call 仍为 1 | `generic_transport.rs:198-231`；`generic_transport_tests.rs:419-453` |

## 2. 端到端权威链

W12 的发送路径固定为：

```text
W07 已持久化 Ready intent
  -> AwaitingAuthority + 精确 owner/generation/until lease
  -> 校验 template、route、required channel、dispatch/verify 时间
  -> 用首次 PreparedPush snapshot 与 rendered bytes 构造 Foundation envelope
  -> prepare(exactly one sink)
  -> 已有终态先 exact requery；否则 resume 一次
  -> reconcile immutable append
  -> inspect_foundation_terminal 完整重验 durable join
  -> W09 verify_terminal 再验业务 intent/policy/template/binding/evidence
  -> DeliveryResult / RequiredChannelObservation
```

这条链不接受 renderer、provider 或 LLM capability；不写业务 `Completed`，也不推进 Unit cursor。只有 W10 finalizer 可以在第二次 authority requery 后写业务完成事实。

## 3. 兼容 envelope 与 DecisionId

历史 `DeliveryEnvelope.decision_identity` 与 Foundation W05 `DecisionId` 是不同语义，不能因为都是 64 位十六进制就直接互换。W12 增加可选 `FoundationDeliveryBinding`：namespace、application decision、intent、Unit、occurrence、business date、subject、audience、template、rendered SHA、source evidence 和 required channel 都进入 canonical envelope。

兼容规则为：

- 历史 constructor 永远保持 `foundation_binding=None`；`skip_serializing_if=None` 保证旧 canonical bytes 不新增字段；
- Foundation envelope 才以 W05 application decision ID 作为 durable decision identity；
- 绑定存在时，business date、occurrence、delivery subject hash、rendered SHA 和 source fingerprint 必须与 envelope 精确一致；
- 同一个 application decision 的 route、scope、channel、template 或首次 bytes 漂移，会由 coordinator 的 canonical envelope 冲突拒绝；
- legacy golden 固定为 decision ID `fd2b10332c1a463dcd5e9fc74e85f388e695bd27679ba61b45878691f5803056`、canonical SHA `5e431e42aa9db00e7a548d490fea575b8c8ba8f882d22d4ceb9ac1843f4fe32f`。

实现位于 `model.rs:574-747,751-1005`，golden/漂移测试位于 `tests.rs:7684-7800`。

## 4. durable 终态只读模型

`inspect_foundation_terminal` 不是 `decision_state()` 的别名。它先验证 stored envelope SHA、canonical bytes、Foundation binding 和 application decision，再把十四态分成：

- `Missing`：确实不存在 decision；
- `PendingSeal`：尚未成为 durable 强终态；
- `Terminal`：只有 Delivered、RejectedDurable、UncertainManualReview、ManualResolvedRejected 且所有权威联结完整。

Terminal 查询按处置分别保留：

| durable 事实 | W12 terminal disposition | attempt 语义 |
| --- | --- | --- |
| Accepted result + delivery audit | Accepted | 必须有 attempt/fence |
| Rejected result | Rejected | 必须有 attempt/fence |
| validated pre-attempt denial | Rejected | 明确无 attempt |
| Uncertain result | Uncertain | 必须有 attempt/fence |
| manual accepted | ManualAccepted | 保留人工处置，不伪装 transport accepted |
| manual rejected/not delivered | ManualNotDelivered | 保留人工处置 |

读取会重新验证 current disposition、唯一 authoritative result、attempt/fence、typed result columns、canonical SHA、delivery audit、manual evidence 和 immutable append reference。Accepted 与 manual accepted 若携带 receipt，还会再次比较 receipt channel 与 Foundation required channel。实现位于 `coordinator.rs:2886-2929,6072-6257,6285-6434,6878-7089`。

## 5. 渠道归集和完成边界

现有 coordinator 的不变量是一个 decision 恰好一个 authoritative sink。W12 没有把多渠道部分成功压成一次 Accepted，而是要求上层为每个 required channel 建立独立 child intent/decision，然后用纯函数归集：

| 观察组合 | classification | completion eligibility |
| --- | --- | --- |
| 全部强 Accepted，或政策允许的 ManualConfirmedAccepted | AllRequiredAccepted | PolicyBound |
| 部分 Accepted、部分 Rejected | PartialRequiredChannels | Never |
| 全部 Rejected/ManualConfirmedNotDelivered | RejectedRequiredChannels | Never |
| 任一 Uncertain | UncertainRequiredChannels | Never |
| 缺失、重复、额外 channel | 合同错误 | 不产生结果 |
| 任一 COMPAT/Blocked | StrongAuthorityRequired | 不产生结果 |

W12 只给出归集合同，不直接操作 parent cursor。parent/child owner、真实 route 与 Unit cursor 的同库 CAS 属于后续 Migration Unit。

## 6. 双轴评审实际发现并修复的问题

本轮不是形式审查，实际关闭了以下问题：

1. **渠道只在 sink descriptor 前置检查，不足以证明终态渠道。** 增加 Accepted receipt 的后置检查；错渠道改记 typed Uncertain；终态读取再次校验 Accepted/manual receipt channel。
2. **sink cardinality 曾由调用参数间接决定。** W12 adapter 现固定 `prepare(..., 1, ...)`，保持一个 decision 一个 authoritative sink。
3. **任意调用方可手工拼 observation。** production `RequiredChannelObservation::new` 被移除，只能由 `dispatch_required_channel` 生成；测试 constructor 限于 `cfg(test)`。
4. **durable retryable Rejected 的重放可能重新进入 resume。** prepare 后先查强终态，已有 Rejected 直接返回；重放两次总 sink call 为 1，重试必须有独立 authority。
5. **只校验 dispatch 时刻在 lease 内，verify 时刻可能已过期。** 现要求 `verified_at >= dispatched_at`，且两者都严格早于同一业务 lease 截止时间。
6. **错渠道 receipt 的 evidence 只验证了状态，没有证明 exact bytes。** 新增从 durable terminal canonical result 反读 evidence，并与原 `TypedReceipt` canonical bytes 做完全相等断言。
7. **终态损坏矩阵只覆盖 disposition。** 新增真实 Accepted 终态后分别篡改 envelope、result、delivery audit、attempt/fence 的四个隔离数据库场景；连同既有 disposition 场景，五类均 fail closed。
8. **两个仅测试 accessor 污染生产 warning 基线。** 加 `cfg(test)` 后 fresh `cargo check --lib` 从 86 回到既有 84 warnings，W12 新增 warning 为 0。

route 评审还确认了一个不能在 W12 伪造的边界：W06 machine catalog 只登记 Unit、producer、occurrence family、phase 和 completion owner，没有真实 transport route/channel。W12 因此只提供 crate-private route capability，并把首次 route/channel 冻结进 durable envelope；实际 Unit→route 注册和生产接线仍由后续垂直迁移完成。

## 7. TDD 提交链

| 提交 | 内容 |
| --- | --- |
| `e510ebf` | 冻结 W12 设计与实施计划 |
| `cbb3e6b` → `1c6750a` | legacy/Foundation envelope binding RED/GREEN |
| `c555b86` → `6e587a3` | terminal read model 首组 RED/GREEN |
| `770f37d` → `18742bc` | 六类 durable terminal disposition 完整化 |
| `175019a` → `e9e98b8` | generic transport + W09 adapter RED/GREEN |
| `c0c337a` → `a937a8a` | required-channel 归集 RED/GREEN |
| `ed30d2f` → `29cad96` | required channel、sink cardinality、sealed observation 修复 |
| `1dc1466` → `32837c7` | retryable rejection 重放与 lease 时间修复 |
| `5b56754` | 五类 authority join 损坏矩阵 |
| `d9f04e6` | 错渠道 receipt exact bytes 证据 |
| `4e2839c` | 测试 accessor 不进入生产构建 |

## 8. Fresh 验证

| 门禁 | 结果 |
| --- | --- |
| `cargo test --lib w12_ -- --test-threads=1` | PASS：18 passed / 0 failed |
| `cargo test --lib push_foundation:: -- --test-threads=1` | PASS：77 passed / 0 failed |
| `cargo test --lib monitor::push_job::tests:: -- --test-threads=1` | PASS：52 passed / 0 failed |
| `cargo test --lib durable_delivery::tests:: -- --test-threads=1` | PASS：125 passed / 0 failed |
| `cargo test --doc` | PASS：16 passed / 0 failed / 4 ignored |
| `cargo check --lib` | PASS；84 个既有目标外 warning；W12 新增 warning 为 0 |
| strict Clippy | 基线阻断：163 个既有错误，首个为 `src/data_gateway/futures_delivery.rs:15`；W12 文件无错误 |
| nonfatal `cargo clippy --lib` | PASS，exit 0；163 warnings；W12 文件无 warning |
| architecture docs 五组测试 | PASS：418 runs / 5023 assertions / 0 failures / 0 errors / 0 skips |
| production panic diff 扫描 | PASS：W12 production diff 新增 `unwrap/expect/panic/unreachable` 为 0 |
| production wiring relative diff | PASS：相对 W11，`src/bin/monitor`、notification、config、Cargo、migration/SQL 为 0 变更 |
| `git diff --check` | PASS |

### 8.1 实际文档检查入口

| 检查 | 结果 |
| --- | --- |
| `check-rfc-inputs.rb` | PASS：`rfc_inputs_valid` |
| `check-sources.rb` | PASS：`source_catalog_valid` |
| `check-rfc.rb --draft` | PASS：`rfc_spec_valid` |
| `render-wbs.rb --check` | PASS：`wbs_current` |
| `check-catalog.rb --draft` | NOT CURRENT：W07--W12 新增/修改 Foundation 与 durable 源码后，冻结 manifest 的 file set、SHA 和 symbol line 已过期 |
| `render-catalog.rb --check` | NOT CURRENT：先被同一 manifest 校验阻断，未写文件 |

后两项不能写成 PASS，也不表示 W12 代码错误。W12 WBS 不授权重签完整 65-kind 源码审计基线；需要单独执行 catalog re-freeze，重新确认完整 file set、符号边界、baseline commit 和派生 Markdown，不能只改 SHA 消除红灯。

## 9. 对当前项目的实际提升

W12 带来的提升是把“调用发送函数后返回 bool/Ok”升级为可重建、可核验的投递权威链：

- **防重复：** 重启或相同 decision 重放先查询 durable 强终态，不会因内存丢失重复发送；
- **防假成功：** sink descriptor 正确但 receipt channel 错误，也只能进入 Uncertain/人工隔离；
- **防部分成功误封口：** 多渠道缺一、部分拒绝或不确定都不能推进业务游标；
- **防证据漂移：** envelope、attempt/fence、result、disposition、audit 任一联结损坏都失败关闭；
- **兼容旧链：** 旧 envelope canonical bytes/identity 不变，现有生产推送不受本切片影响；
- **为迁移提供统一 seam：** 后续 52 个 Unit 不再各自解释 bool、重试和渠道成功，而是接入同一个 exact authority 合同。

这些收益当前只存在于隔离 Foundation 能力中。没有 Unit 接线前，它不会改变今天的实际推送数量、内容或调度。

## 10. 尚未完成

- W13：P01/N02 专用 conformance adapter；
- W14--W17：scheduler、readiness、activation generation/owner fence、shadow harness 与 typed diff；
- W18：operator 身份、审计 capability、inspect/resolve 控制面；
- W19--W21：指标、告警、发布和回滚总门禁；
- 52 个 Migration Unit 的真实 route/channel、parent/child owner、业务 cursor 同库 CAS、shadow、六门禁、单 owner 晋级和观察；
- provisional catalog 的独立 re-freeze；
- 生产 business schema migration、真实 sink wiring、灰度与盘前/集合竞价/盘中/盘后实盘验收。

因此 W12 完成的是“通用 durable transport 能被 Foundation 精确发送、重查并按 required channel 归集”，不是“推送系统整体改造完成”，也不是“生产已启用新链路”。
