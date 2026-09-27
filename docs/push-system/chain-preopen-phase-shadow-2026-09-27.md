# MU-chain-preopen：PhaseScheduler 只读 shadow

2026-09-27 接线范围：`monitor_loop` 原有 30 秒循环在本地时间 09:04–09:16 采样。采样放在旧发送和 miss 记录之后，避免延迟旧生产时机。旧 `ChainScheduleStore`、`run_scheduled_chain_analysis` 和 `record_missed_window` 仍是唯一生产 owner；shadow 只读旧 SQLite，再在内存里调用 `PhaseScheduler::tick`，不写 Foundation occurrence，不调用 producer、sink 或 business finalizer。

## 对照输入与日志

- 每次采样固定一个 `DateTime<FixedOffset>`、calendar date 和 `calendar::is_trading_day` 结果。旧窗口通过 `ChainPhase::Preopen.starts_in_window`、旧到期通过 `is_overdue`，新窗口由同一日期和时区的 `[09:05,09:15)` 构造。
- 旧 occurrence 是 `(preopen, calendar_date)`；新候选 ID 来自冻结 catalog 中的 `chain-preopen-timer`、`MU-chain-preopen`、相同 calendar date 和 `chain-preopen:{date}`。两者表示映射关系，字符串格式本来不同；日志并列记录两者，不能把哈希不同算作业务差异。最新完成的行情 business date 尚未进入本次只读对照。
- 日志前缀 `[chain-preopen-shadow]` 记录 observed_at、旧/新 occurrence、旧状态、新状态及 reason、窗口、due、miss、closed 和 `diff`。`foundation_persisted=false` 明确本切片仅是无状态策略投影。
- 缺少旧 SQLite 表示当天尚无尝试或 miss；已有数据库必须只读检查。检查失败只写警告，不干预旧调度。

`diff=true` 在旧弱接受后预期出现：旧状态已经 Closed，而 Foundation 没有持久 occurrence、权威 receipt 和 finalizer，不能据此关单。旧 miss 已持久记录后，新无状态投影仍会给出 Missed 候选，也会产生差异。两类差异必须由后续迁移分别解决，不能作为允许双发的理由。

## 推进门禁

此切片只验证策略边界和 identity 映射。下一步须把同一份 `PreparedFacts` 用于 old/new 决策，接入可恢复的 Foundation occurrence、业务日期/输入/载荷对照和权威完成凭证；观察真实 09:05–09:15 窗口并解释全部语义差异后，才可考虑切换 physical owner。冻结 catalog 当前仍为 PROVISIONAL；本次不更改 activation。
