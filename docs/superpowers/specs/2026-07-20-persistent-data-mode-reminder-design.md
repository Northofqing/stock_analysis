# 数据模式持续异常提醒去重设计（BR-135 修订）

**原始日期：** 2026-07-20

**修订日期：** 2026-08-22

**状态：** Gate A — 已获用户批准，待实现与验证

**规则：** AGENTS 2.4、2.7、2.10；BR-116、BR-135、BR-148、BR-225c

**修订范围：** 本版本取代本文件早期“同一 Unsafe 每 30 分钟外发”的设计；首次状态通知、真实变化通知、恢复防抖和数据安全限制继续有效，历史版本由 Git 保留。

## 1. 问题与目标

当前 BR-135 会在同一 Unsafe 状态持续期间每 30 分钟把完整消息再次发到外部渠道。2026-08-22 的生产推送记录中，11:27 至 21:53 共出现 20 条，去掉消息内时间后只有 1 个唯一正文，19 条没有新增业务事实。

本次目标：

1. 首次进入 Unsafe、模式变化、Unsafe 缺失能力集合变化及恢复，继续走现有受治理的外部通知路径。
2. 同一 Unsafe 指纹持续不变时，每 30 分钟只写不可篡改审计，不调用外部 sink。
3. 审计失败必须显式报错并在下一次 60 秒调度时重试，不得伪造已记录状态。
4. 不改变 freshness 阈值、DataMode 判定、风险限制、账户数据或订单路径。

## 2. 备选方案

### 方案 A：状态机分流外发与审计（采用）

以稳定事件指纹区分“有新事实”和“无变化心跳”。新事实外发；无变化到期心跳仅写权威审计。优点是语义明确、可测试，不依赖通用 cooldown 的时长；代价是需要调整 BR-135 状态和模板清单。

### 方案 B：删除正文时间，依赖通用 cooldown（不采用）

实现最小，但 DataMode 的通用 cooldown 不是“整个事故周期只外发一次”的权威状态；窗口到期后仍可能再次发送，进程重启也不能保证语义。

### 方案 C：按交易时段过滤（不采用）

可以减少闭市噪音，但会把数据安全状态绑定到交易会话。周末或休市期间的真实能力故障仍需被评估和审计，因此不能作为根治方案。

## 3. 状态与指纹

Unsafe 事件指纹使用稳定、可复算的字段：

```text
版本 + DataMode + 按 Capability::ALL 固定顺序排列的 missing 集合
```

指纹不包含当前时间、age 秒数或 ETA，避免同一事实因动态展示字段被误判为新事件。Unsafe 下的输出限制由 DataMode 固定映射，规则变化必须提升指纹版本。

状态记录只在以下权威结果后推进：

- 外部通知：sink 为 `Pushed` 且强制投递审计全部成功；
- 内部心跳：`push.delivery.audit` 哈希链追加并同步成功。

Denied、Deduped、SinkError、锁失败、时钟回退或审计失败均不提交对应状态。

## 4. 数据流

```text
60 秒独立调度
  → 读取真实 capability freshness
  → evaluate DataMode
  → 生成稳定事件指纹
      ├─ 首次非 Full / 模式变化 / Unsafe 指纹变化
      │    → T-02 DataMode 受治理外发
      │    → 成功后提交模式、指纹和心跳起点
      ├─ 同一 Unsafe 指纹，距上次确认满 30 分钟
      │    → 写 outcome=Deduped、channel=internal_audit 的权威审计
      │    → 成功后只推进心跳时间，不调用 sink
      └─ 同一指纹且未到期
           → no-op
```

Full 首次建立仍可静默；Unsafe → Full/Degraded 的通知继续服从 BR-225c 现有 300 秒恢复防抖。本次不改变该规则。

## 5. 组件改动

| 模块 | 决定 | 原因 |
| --- | --- | --- |
| `src/monitor/data_mode.rs` 的 `PersistentUnsafeReminder` | 采用并收窄为“Unsafe 心跳状态” | 保留单调时钟、确认后推进、失败重试语义；增加稳定指纹判断 |
| `src/bin/monitor/main.rs` 的 60 秒独立调度 | 采用 | 健康评估仍需跨交易会话运行 |
| `src/bin/monitor/push_templates.rs` 的 T-02 变更通知 | 采用 | 首次异常和真实变化继续受治理外发 |
| `render_data_mode_reminder` / `T-02-data-mode-reminder` 外发模板 | 退休 | 无变化心跳不再是用户消息 |
| `stock_analysis::event::publish_delivery` | 采用 | 复用既有 5 年保留、加锁、哈希链和 `sync` 权威审计，不新增第二套审计写入器 |

## 6. 审计契约

内部心跳使用：

- `kind = data_mode_unsafe_heartbeat_v1`
- `outcome = Deduped`
- `channel = internal_audit`
- `code =` 可复算的稳定事件指纹（落盘为不可逆 subject hash）
- 本地 warn 日志记录 BR-135、当前模式、missing 集合和 `external_delivery=suppressed_unchanged`

该记录表示“治理层确认状态仍为同一事件，因此外部投递被抑制”，不是伪造 sink 成功。审计写入失败时不推进 30 分钟确认点，并在下一轮重试。

## 7. 失败模式

| 失败 | 行为 |
| --- | --- |
| capability 输入不可用 | 显式 error；不外发、不审计、不提交状态 |
| 指纹/状态锁中毒 | 显式 error；不提交状态 |
| 外部状态通知失败 | 保留上次确认模式和指纹，下一轮重试 |
| 内部心跳审计失败 | 不调用外部 sink；保持到期，下一轮重试审计 |
| 单调时钟回退 | 显式 error；状态不变 |
| 进程重启 | 不从非权威本地时间猜测确认点；按首次真实评估重新建立状态 |

## 8. 验收标准

1. 初次 Unsafe 产生 1 条外部 T-02；同一指纹持续 2 小时不再产生外部持续异常消息。
2. 同一期间每满 30 分钟产生 1 条 `data_mode_unsafe_heartbeat_v1` 权威审计。
3. Unsafe 的 missing 集合变化会产生新的 T-02 外部状态通知；动态 age/时间变化不会。
4. 心跳审计失败后仍保持 due，下一次调度重试；失败不会回退成外部通知。
5. `T-02-data-mode-reminder` 不再列为 Active presentation；`T-02-data-mode` 保持 Active。
6. BR-135 定向测试、全量 fmt/clippy/test、compliance、coverage 和 release build 按仓库 Gate 执行。

## 9. 可复现基线证据

生产消息归一化检查：

```bash
# 对 2026-08-22 含“数据状态持续异常”的正文去掉 (HH:MM) 后计数并散列
# 结果：total=20 unique_normalized_payloads=1 duplicate_instances=19
```

代码路径：

```bash
rg -n "BR-135|PersistentUnsafe|数据状态持续异常|T-02-data-mode-reminder" docs src
# src/monitor/data_mode.rs: 30 分钟 due 状态
# src/bin/monitor/main.rs: 独立 60 秒调度
# src/bin/monitor/push_templates.rs: reminder 进入外部 governed delivery
# docs/business_rules.md: BR-135 要求持续外发
```

实现前定向测试：

```bash
cargo test --lib monitor::data_mode::tests::br135 -- --test-threads=1
# 1 passed; 0 failed
cargo test --bin monitor br135 -- --test-threads=1
# 5 passed; 0 failed
```

## 10. 回滚

通过本变更 PR 的提交记录定位设计或实现提交，并对对应提交执行 `git revert`，恢复旧 BR-135 每 30 分钟外发语义。回滚不删除任何既有推送日志、投递审计、账户、订单或行情证据。
