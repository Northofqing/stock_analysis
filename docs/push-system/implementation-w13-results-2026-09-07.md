# 推送 Foundation W13 实现与验证结果

**结论：** W13 已完成 P01/N02 专用 Conformance Adapter。P01 继续以既有 BusinessDateOnce durable claim 为唯一权威，同一业务日不因 Scheduled/Compensation render mode 产生第二 owner；N02 继续以 NewsFlash v5 append-only audit chain 的 exact `(business_date, window)` attempt/terminal 为权威，查询与完成结果不读取、修改或依赖 N01 critical accepted-event/quota。两类专用证据完成领域校验后统一交给 W09 复验，只有 exact 强终态才能形成 `DeliveryResult`。

**生产边界：** W13 仍是 crate-private、零生产接线。相对 W12 基线 `cdea840`，`src/bin/monitor`、notification、config、Cargo、migration/SQL 均无变更；没有 producer/caller、scheduler、cursor、activation manifest 或 physical owner 切换，也没有启动、重启、观察或替换生产 monitor。现有推送继续走原路径。

**验证日期：** 2026-09-07（Asia/Shanghai）

**分支/worktree：** `codex/push-reliability-20260905` / `.worktrees/push-reliability-20260905`

**代码验证 HEAD：** `971a5fa271e84f2b529b8f013ecfb362c5cb4371`

## 1. WBS 验收结论

WBS 的 W13 验收句是：

> P01同日claim不分render-mode；N02 accepted-window独立于N01 critical quota。

| 验收点 | 结果 | 代码证据 |
| --- | --- | --- |
| P01 同日 claim 不分 mode | reader 的唯一输入是 `business_date`，内部固定 `PreopenNewsHot/None/GLOBAL/p01:{date}`；query surface 没有 render mode、render SHA 或 source SHA | `src/durable_delivery/coordinator.rs:2933-3022`；`src/durable_delivery/tests.rs:4314-4381` |
| P01 exact terminal | claim→stored decision→canonical envelope/SHA→state/disposition→attempt/evidence 全链重验；返回 exact envelope/evidence，不接受调用方自报终态 | `src/durable_delivery/model.rs:1158-1200`；`src/durable_delivery/coordinator.rs:2960-3020,6196-6394` |
| P01 投影到 W09 | 固定 Unit/subject/template，复验 legacy envelope、source mode、render/source SHA、receipt channel 与 attempt/disposition，再由 adapter 自算 terminal binding | `src/push_foundation/dedicated_transport.rs:141-180,427-555,565-625` |
| N02 key 与 N01 分域 | typed key 只有四个 window；source port 只有 business date/window 参数，不存在 event、critical threshold、quota 或计数参数 | `src/event/mod.rs:256-289`；`src/push_foundation/dedicated_transport.rs:110-139` |
| N02 exact window authority | 读取完整年度权威链后，只选择 exact date/window；同 reservation ordinal 严增，只有 DefinitivelyRejected 可打开下一 attempt，每 attempt 仅一终态，Accepted 不可撤销 | `src/event/mod.rs:975-1186` |
| N02 投影到 W09 | 复验 v5 schema、kind/key/date、reservation/ordinal、attempt join、ordered sources/evidence、render、channel 和 typed receipt；terminal envelope canonical bytes 作为 evidence | `src/push_foundation/dedicated_transport.rs:182-393` |
| 强终态不可伪造 | `AuthorityTerminalRecord` 与固定 authority 均保持模块私有；source 自报 binding hash 不存在，adapter 计算后仍由 W09 全字段复验 | `src/push_foundation/dedicated_transport.rs:25-29,363-393,642-673` |
| 错误失败关闭 | Missing、Pending、source unavailable、解析/SHA/binding/channel/disposition 错误都返回 typed error，不形成强结果 | `src/push_foundation/dedicated_transport.rs:39-67,163-179,209-225` |

## 2. P01 权威链与 mode 隔离

P01 的读取和投影链固定为：

```text
Foundation 已持久化 Ready intent
  -> 从 Ready business_date 查询唯一 legacy BusinessDateOnce claim
  -> 固定 PreopenNewsHot / None / GLOBAL / p01:{date}
  -> 重验 stored envelope canonical bytes、SHA、legacy decision、policy version
  -> 重验 current disposition、attempt/fence、result/manual evidence
  -> adapter 复验 MU-p01 / Global / preopen_news_hot_v1
  -> 复验 Ready render SHA/source fingerprint 与 legacy envelope 完全相等
  -> 仅验证 Scheduled|Compensation source mode，不把 mode 写入 query/occurrence
  -> adapter 计算 P01 terminal binding
  -> W09 verify_terminal
  -> TransportAccepted / TransportRejected / TransportUncertain / AlreadyTerminal
```

Scheduled 与 Compensation 可以有不同 source binding 和 rendered bytes，但它们不能取得两个同日 claim。真实 SQLite 测试先让 Scheduled 成为 owner，再提交 Compensation，断言仍返回原 owner；随后 reader 只传日期并读回同一 legacy decision。这个运行证据比仅观察函数签名更强，位于 `src/durable_delivery/tests.rs:4314-4381`。

P01 source binding 是 `deny_unknown_fields` 的闭对象，schema 固定 `P01_SOURCE_BINDING_V1`，mode 只能是 Scheduled 或 Compensation。mode 只证明历史 envelope 的合法来源，不进入 application decision、Foundation occurrence 或 terminal binding。

## 3. N02 权威链与 N01 quota 隔离

N02 的读取和投影链固定为：

```text
Foundation 已持久化 Ready intent + typed NewsFlashWindow
  -> AuditDispatcher 在 retained-root/lock 下验证完整年度 hash chain
  -> 选择 exact news_flash_aggregated_v1 + business_date + window:{HH:MM}
  -> SinkAttempt(reservation, ordinal)
  -> Accepted | DefinitivelyRejected | Uncertain
  -> reader 验证 attempt 顺序、唯一 terminal、重试合法性和不可撤销 Accepted
  -> adapter 重新解析两个 PushRecord 并逐字段绑定
  -> exact terminal EventEnvelope canonical bytes/SHA
  -> adapter 计算 N02 terminal binding
  -> W09 verify_terminal
  -> DeliveryResult
```

四个合法 window 是 09:30、11:30、13:00、15:00；其他 label 在 typed parse 阶段被拒绝。N02 source trait 不接受 N01 accepted-event set、quota、committed/pending count，也不暴露写这些状态的方法。行为测试用两个不同的 N01 quota observation 驱动同一 N02 exact source，得到相同 query、binding SHA 和 evidence SHA，见 `src/push_foundation/dedicated_transport_tests.rs:916-952`。

年度读取不是信任日志文本：`AuditDispatcher::read_authoritative_year` 先验证 retained-root、完整 hash chain 和 closed envelope，再由 `PushRecord::try_from_authoritative` 验证 NewsFlash v5 payload。非 v5 行不会形成目标强终态；链损坏、未知字段、目标 join 冲突都失败关闭。

## 4. 状态与失败矩阵

| source 状态 | P01 结果 | N02 结果 | completion eligibility |
| --- | --- | --- | --- |
| Missing | `TerminalMissing` | `TerminalMissing` | 不产生结果 |
| Pending/Open | `TerminalPendingSeal` | `TerminalPendingSeal` | 不产生结果 |
| Accepted | `TransportAccepted` | `TransportAccepted` | PolicyBound，由 W09 policy 决定 |
| Rejected / DefinitivelyRejected | `TransportRejected` | `TransportRejected` | Never |
| Uncertain | `TransportUncertain` | `TransportUncertain` | Never，禁止盲重试 |
| ManualAccepted | `AlreadyTerminal(ManualConfirmedAccepted)` | 不属于 N02 v5 transport stage | 仅 policy 明确允许时可完成 |
| ManualNotDelivered | `AlreadyTerminal(ManualConfirmedNotDelivered)` | 不属于 N02 v5 transport stage | Never |
| source unavailable | `SourceUnavailable` | `SourceUnavailable` | 不产生结果 |
| wrong Unit/subject/template/channel | typed conformance error | typed conformance error | 不产生结果 |
| evidence/envelope/SHA/join 损坏 | typed conformance/authority error | typed conformance/authority error | 不产生结果 |

N02 合法重试仅为 `DefinitivelyRejected ordinal=n -> SinkAttempt ordinal>n`。Pending、Uncertain 或 Accepted 后出现新 attempt 都是冲突；多 Accepted、terminal 无 attempt、同 attempt 多 terminal、ordinal 重复/回退及跨日期/窗口 join 均失败关闭。reader 行为覆盖位于 `src/event/mod.rs:2050-2420`。

## 5. 双轴评审结果

### 5.1 Spec 轴

WBS 验收句、W13 设计 §5--§12、BR-241 和 BR-244 已映射到实现与测试。评审确认：

- P01 query/claim identity 不含 render mode；
- N02 query/source port 不含 N01 event/quota；
- P01/N02 都必须 exact requery，不把 bool、摘要、日志或 accepted-window 集合包装成 receipt；
- Accepted 不可撤销，Uncertain 不授予自动重试；
- adapter 不接管发送、retry、scheduler、cursor 或 activation；
- W09 是两类 strong result 的统一最终验证器。

交接时列出的四个待核查点中，非 v5 同 identity 最终只能 Missing/error、不能铸造强终态；年度全链读取和 occurrence 约定属于既有 authority/后续 Unit 接线合同；terminal trace metadata 改变会改变 exact evidence SHA，已有非同源行为测试。因此它们不是 W13 新行为缺陷。

### 5.2 Standards 轴

评审实际发现并关闭一个问题：`NewsFlashWindowTerminalRecord` 原先派生 `Debug`，会递归输出完整 attempt/terminal envelope，从而泄漏 ordered sources 和 remote receipt 正文。新增真实 authority reader seam 测试先证明 RED，再将 Debug 收敛为 attempt/terminal envelope ID；payload、provider、message ID、platform message ID 和 source event 均不再出现。修复提交为 `971a5fa`，实现位于 `src/event/mod.rs:291-312`，测试位于 `src/event/mod.rs:2298-2359`。

其余 Standards 结论：专用接口均为 crate-private；`FixedDedicatedAuthority` 不能由生产调用方自由构造；错误只带固定类别/字段名；没有新增第三状态机或逆向依赖；W13 production diff 没有新增 `unwrap/expect/panic/unreachable`。年度集合大小沿用既有完整权威链 API，W13 不擅自发明会截断审计的 retry/row cap。

## 6. TDD 提交链

| 提交 | 内容 |
| --- | --- |
| `1a161b3` | 冻结 W13 专用 adapter 设计 |
| `fb6b8c3` | 修订并冻结 N02 rejected-only retry 语义 |
| `3c866a6` | 冻结 W13 实施计划 |
| `da2a0f3` → `619041e` | P01 same-day exact reader RED/GREEN |
| `8cda7e3` → `df3416b` | P01 → W09 adapter RED/GREEN |
| `2be5072` → `2919075` | P01 disposition/corruption matrix RED/GREEN |
| `e7a7d73` → `a6d0233` | N02 exact window reader RED/GREEN |
| `da24bc6` → `788fe1a` | N02 → W09 与 N01 分域 RED/GREEN |
| `971a5fa` | 双轴评审发现并 TDD 修复 N02 authority Debug 泄漏 |

## 7. Fresh 验证

| 门禁 | 结果 |
| --- | --- |
| `cargo test --lib w13_ -- --test-threads=1` | PASS：12 passed / 0 failed |
| `cargo test --lib push_foundation:: -- --test-threads=1` | PASS：85 passed / 0 failed |
| `cargo test --lib durable_delivery::tests:: -- --test-threads=1` | PASS：126 passed / 0 failed |
| `cargo test --lib event:: -- --test-threads=1` | PASS：173 passed / 0 failed / 2 ignored |
| `cargo test --lib monitor::push_job::tests:: -- --test-threads=1` | PASS：52 passed / 0 failed |
| `cargo test --doc` | PASS：16 passed / 0 failed / 4 ignored |
| `cargo check --lib` | PASS；84 个既有目标外 warning；W13 新增 warning 为 0 |
| `cargo clippy --lib` | PASS，exit 0；163 个既有 warning；W13 文件无 warning |
| `cargo clippy --lib -- -D warnings` | 基线阻断：同一批 163 个既有 warning 被提升为错误；首错为 `src/data_gateway/futures_delivery.rs:15`，W13 文件无命中 |
| architecture docs 五组测试 | PASS：418 runs / 5023 assertions / 0 failures / 0 errors / 0 skips |
| production panic diff 扫描 | PASS：命中均位于测试；W13 production diff 新增 `unwrap/expect/panic/unreachable` 为 0 |
| production wiring relative diff | PASS：相对 `cdea840`，monitor/notification/config/Cargo/migration 为零变更 |
| `git diff --check cdea840...HEAD` | PASS |

现有 warning 基线不能写成 strict Clippy PASS；同样，W13 没有授权顺手修改 163 个目标外历史问题。nonfatal Clippy 与路径核对共同证明 W13 没有新增 warning。

### 7.1 Architecture docs 精确结果

| 检查 | 结果 |
| --- | --- |
| `rfc_inputs_test.rb` | PASS：12 runs / 238 assertions |
| `source_catalog_test.rb` | PASS：12 runs / 110 assertions |
| `catalog_test.rb` | PASS：33 runs / 393 assertions |
| `rfc_spec_test.rb` | PASS：295 runs / 3873 assertions |
| `wbs_test.rb` | PASS：66 runs / 409 assertions |
| `check-rfc-inputs.rb` | PASS：`rfc_inputs_valid` |
| `check-sources.rb` | PASS：`source_catalog_valid` |
| `check-rfc.rb --draft` | PASS：`rfc_spec_valid` |
| `render-wbs.rb --check` | PASS：`wbs_current` |
| `check-catalog.rb --draft` | NOT CURRENT：W07--W13 修改/新增源码后，冻结 manifest 的 file set、SHA、symbol line 已过期 |
| `render-catalog.rb --check` | NOT CURRENT：被同一冻结 manifest 校验阻断，未写文件 |

catalog NOT CURRENT 不能伪装为 PASS，也不等于 W13 代码失败。重新冻结需要独立复核完整源码集合、符号边界和 baseline commit，不能只改 SHA 消红。

## 8. 对当前项目的实际提升

- **P01 防双 owner：** 定时与补偿在 application 层可有不同呈现，但 durable completion 永远回到同一业务日 claim，避免同日重复推送。
- **N02 防额度串域：** 聚合窗口是否完成不再由 N01 critical quota、accepted-event set 或内存计数解释；两域互不升级、互不扣账。
- **防假成功：** bool、日志、运行摘要和 accepted-window 集合都不能进入 W09；必须重读 exact envelope/attempt/terminal/receipt。
- **防证据漂移：** date/window、reservation、ordinal、ordered sources、render、channel、receipt 和 SHA 任一漂移都会失败关闭。
- **统一完成语义：** P01/N02 保留各自高保证状态机，同时输出与 generic transport 相同的 W09/W02 强结果，后续 finalizer 不再写两套成功解释。
- **降低诊断泄漏：** authority Debug 只保留可定位的 envelope ID，不打印 source/receipt 正文。
- **零线上扰动：** 本切片没有生产 caller；当前实际推送数量、时点、内容和原 owner 均不改变。

## 9. 尚未完成

- W14：scheduler/due occurrence 与交易日、补偿、重启恢复统一；
- W15：source readiness、NoData/Disabled/Deferred 边界；
- W16：activation generation、single-owner fence 与配置原子性；
- W17：shadow harness、typed diff 与旧/新链并行比对；
- W18：operator 身份、授权、不可变审计、inspect/resolve 控制面；
- W19：Foundation 指标、SLO、告警与证据导出；
- W20：崩溃、重放、去重、回滚测试矩阵；
- W21：发布、灰度、晋级、回滚总门禁；
- 52 个 Migration Unit 的真实 route/channel、parent/child owner、业务 cursor 同库 CAS、shadow、六门禁、单 owner 晋级和观察；
- provisional catalog 的独立 re-freeze；
- 生产数据库 migration、真实 sink wiring，以及盘前、集合竞价、盘中、盘后实盘验收。

因此 W13 完成的是“P01/N02 既有专用权威可被 Foundation 精确读取并统一复验”，不是“P01/N02 已生产切换”，更不是“推送系统整体改造完成”。
