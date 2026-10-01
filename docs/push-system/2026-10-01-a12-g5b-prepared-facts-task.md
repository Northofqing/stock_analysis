# A-12 / G5b 归因投递完成边界任务卡（2026-10-01）

## 当前可安全完成的切片

`main.rs` 的 G5b 空输入分支不再写 `G5B_LAST_RUN`。15:05–15:20 每个 tick 可重新筛选生产合格告警，包括 15:20 分钟内的晚到事件；空输入不调用 LLM 或 sink。已有分析批次仍保留当日封口，避免模型重跑和不明投递重复发送。此变更没有宣称 G5b 归因投递已完成。

## 阻断完整修复的身份和存储缺口

| 单元 | 已有事实 | 缺失的完成依据 |
| --- | --- | --- |
| A-12 AttributionDaily | `compute_epoch_daily` / `commit_effective_window` 可生成报告；`persist_report_revision` 只把全文 Markdown 按内容摘要追加保存；`build_attribution_daily_counted_binding` 使用业务日 occurrence 和**推送摘要文本** SHA。 | 摘要原始字节、对应报告 revision/来源 head 与 counted decision ID 没有共同持久冻结。binding/token 准备失败发生在 durable row 创建前；其后 `ATTRIBUTION_LAST_RUN` 仍封日。仅从 Markdown 或业务日重算摘要可能取得不同 revision/字节。 |
| G5bAttribution | `append_deep_attribution_row` 在 LLM 返回后向 `data/g5b/{date}.jsonl` 追加结果；`build_g5b_counted_binding` 用告警事实 hash 作 occurrence，用摘要 SHA 绑定 payload。 | 归因 row、LLM receipt、精确摘要字节与 counted decision ID 无原子 checkpoint、可查询 revision 或唯一约束。JSONL 追加失败、binding/token 失败以及 sink 结果不明时，重跑 LLM 可能产生新内容/新费用；按告警 key 盲发可能重复。 |

counted 路径的 durable `Delivered` 可映射为 `PushOutcome::Pushed`；但调用点没有把对应 decision ID、receipt 与报告/模型 revision 持久关联成业务 finalizer。`Denied / SinkError` 又折叠了多个准入、终态和故障原因，不能据此判断 `Reserved/RejectedDurable` 可否恢复或 `Uncertain` 是否需要人工裁定。因此本任务不把单独的 `Pushed` 或本地报告/模型保存当成业务完成回执，也不因失败直接清除两个 `LAST_RUN`。

## 后续实施顺序

1. 为两个 Unit 分别冻结不可变 PreparedFacts：业务日、来源 head/报告 revision（G5b 为生产告警事件身份与模型调用结果）、完整渲染字节及摘要 SHA、schema/版本。G5b 必须先保存模型结果和 provider receipt，后续恢复只读取保存的摘要，不能再次调用 LLM；保存结果失败时不能伪造已准备状态。
2. 以 PreparedFacts 身份持久关联 counted decision ID、投递 attempt/终态和业务完成游标。A-12 同日报告改版必须明确新 revision 的校正通知规则，不得复用原日 occurrence 静默改写已发送内容。G5b 同事件不同模型结果须判为身份冲突，不能产生第二张卡。
3. 提供只读 inspect 和受 fence 约束的恢复：仅重放同一已保存 payload 的 `Reserved` 或明确可重试的 `RejectedDurable`；`Uncertain` 不自动重发，交人工裁定；`Delivered` 的恢复只补业务 finalizer。完成游标仅由上述权威终态推进，进程内日期位仅作节流。
4. 定向故障测试覆盖报告/row 已存但 token、binding、sink 或 finalizer 失败；跨进程恢复保持字节与 revision 不变且 G5b LLM 只调用一次；重复/冲突告警、15:20 晚到、`Uncertain` 零自动重发。随后按 M1 真实 sink 回执和单 owner 观察验收。

任务完成前，A-12 与 G5b 均维持现有保守封口；不得通过只改日期标志、直接读取旧 JSONL 或合成投递回执来宣称完成。
