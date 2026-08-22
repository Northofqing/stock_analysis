# BR-135 Unsafe 心跳去重设计

**日期：** 2026-08-22

**状态：** Gate A — 用户已批准在隔离工作区实施

**规则：** AGENTS 2.1、2.2、2.4、2.7、2.8、2.10；BR-116、BR-135、BR-148、BR-225c

**取代关系：** 本文取代 `2026-07-20-persistent-data-mode-reminder-design.md` 中“同一 Unsafe 每 30 分钟外发”的决策；旧文档保留为历史，不被覆盖。

## 1. 问题与目标

当前 BR-135 会在同一 Unsafe 状态持续期间每 30 分钟把完整状态再次发送到外部渠道。用户确认这类重复正文没有新增决策价值。

本次目标：

1. 首次进入 Unsafe、模式变化、Unsafe 缺失能力集合变化及恢复，继续走既有受治理外部通知。
2. 同一 Unsafe 事件指纹持续不变时，每 30 分钟只写不可篡改审计，不调用外部 sink。
3. 审计失败显式报错并在下一次 60 秒调度时重试，不伪造已记录状态。
4. 不改变 freshness 阈值、DataMode 判定、风险限制、账户数据或订单路径。

## 2. 方案

采用事件指纹状态机，将“有新事实的状态通知”和“无变化的存活审计”分开：

- 新事实：外部 T-02 通知。
- 无变化且满 30 分钟：仅内部 hash-chain 审计。
- 无变化且未到期：no-op。

不采用通用 cooldown：其窗口到期后仍可能外发，不能表达“一次事故只在事实变化时再通知”。不采用交易时段过滤：DataMode 安全状态必须跨休市持续评估。

## 3. 稳定事件指纹

Unsafe 事件指纹由下列字段组成：

```text
版本 + DataMode + 按 Capability::ALL 固定顺序排列的 missing 集合
```

指纹不包含当前时间、动态 age 或 ETA。输出限制由 DataMode 固定映射；如果影响事件身份的规则变化，必须提升指纹版本。

## 4. 数据流

```text
启动一次 + 60 秒独立调度
  → 读取真实 capability freshness
  → evaluate DataMode
  → 生成稳定事件指纹
      ├─ 首次非 Full / 模式变化 / Unsafe 指纹变化
      │    → T-02 DataMode 受治理外发
      │    → 确认后提交模式、指纹、心跳起点
      ├─ 同一 Unsafe 指纹，距上次确认满 30 分钟
      │    → 写 outcome=Deduped、channel=internal_audit 的权威审计
      │    → 成功后只推进心跳时间，不调用 sink
      └─ 同一指纹且未到期
           → no-op
```

Full 首次建立仍可静默；Unsafe 到 Full/Degraded 的通知继续服从 BR-225c 的恢复防抖，本次不改变该规则。

## 5. 组件决策

| 模块 | 决定 | 原因 |
| --- | --- | --- |
| `monitor::data_mode::evaluate` | 采用 | 真实 capability freshness 仍是唯一健康输入 |
| `PersistentUnsafeReminder` | 重塑为事件指纹状态机 | 统一决定外发、内部心跳、静默和确认点推进 |
| `data_mode_monitor_loop` | 采用 | 保留唯一、跨市场会话的 60 秒所有者 |
| `T-02-data-mode` | 采用 | 首次异常和真实事实变化继续受治理外发 |
| `T-02-data-mode-reminder` | 退休 | 无变化心跳不再是用户消息 |
| `stock_analysis::event::publish_delivery` | 采用 | 复用既有加锁、hash-chain 与同步审计，不新增第二套写入器 |

## 6. 审计契约

内部心跳固定使用：

- `kind = data_mode_unsafe_heartbeat_v1`
- `outcome = Deduped`
- `channel = internal_audit`
- `code =` 可复算的稳定事件指纹；权威审计只保存不可逆 subject hash
- 本地 warn 日志包括 BR-135、missing 集合和 `external_delivery=suppressed_unchanged`

它表示治理层确认状态仍为同一事件，因此抑制外部投递；不表示 sink 成功。审计写入失败时不推进确认点。

## 7. 失败模式

| 失败 | 行为 |
| --- | --- |
| capability 输入不可用 | 显式 error；不外发、不审计、不提交状态 |
| 状态锁中毒 | 显式 error；不提交状态 |
| 外部通知失败 | 保留上次确认模式和指纹；下一轮重试 |
| 内部心跳审计失败 | 不调用外部 sink；保持到期；下一轮只重试审计 |
| 单调时钟回退 | 显式 error；状态不变 |
| 进程重启 | 不猜测历史确认点；按首次真实评估重建状态 |

## 8. 验收标准

1. 初次 Unsafe 产生一条外部 T-02；同一指纹持续两小时不再产生外部持续异常消息。
2. 同一期间每满 30 分钟产生一条 `data_mode_unsafe_heartbeat_v1` 权威审计。
3. Unsafe missing 集合变化会产生新的外部 T-02；动态时间字段变化不会。
4. 心跳审计失败后仍保持 due，下一次调度重试；失败不会回退成外部通知。
5. `T-02-data-mode-reminder` 不再列为 Active presentation；`T-02-data-mode` 保持 Active。
6. 定向测试、fmt、strict clippy、全量 test、compliance、coverage 与 release build 执行并留证。

## 9. 回滚

对本变更提交执行 `git revert`，重建 release，并恢复上一个已验证二进制。回滚不得删除或改写既有推送日志、投递审计、账户、订单或行情证据。
