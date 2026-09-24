# TDX E2103 修复后的本机对账（2026-09-24）

## 已闭合的范围

上游 [失败集复测](share/2026-09-24-tdx-failed-set-sweep.md) 报告 18/18 条短窗口请求通过。本机 `127.0.0.1:18082` 的独立只读请求 `codex-local-tdx-recheck-300005-20260924`，以 `codes=["300005"], days=5` 查询，返回 `ADMITTED / Tdx / complete=true / 5 条`，最后一条已结算日为 2026-09-23。`grpc-market-server.err.log` 中检索 2026-09-24 的 E2103 为零。以上支持 **TDX 两字节截断故障已修复**；不能据此宣称所有日线窗口都已可用。

## 本机仍失败的范围

同一服务、同一代码的 90 日请求 `codex-local-tdx-recheck-300005-90-20260924` 返回 `Internal / no_verified_batch`。服务端错误文本指出最终 BR-171 准入失败：2026-07-16→07-17 收盘价 `18.59→14.87`，计算涨跌 `-20.0108%`，需要证据绑定的人工确认。服务端仍使用有 `grpc-status-details-bin mismatch` 的旧错误 trailer；客户端不能从这条错误恢复完整的失败分类。这不是 E2103，也不能自动放宽 BR-171。

生产库 `data_acquisition_audit` 在 2026-09-24 09:28–10:50 CST 的 `HistoricalDailyBars` 快照（截至 `2026-09-24T02:50:53.623Z`）：

| provider | outcome / reason_code | 行数 |
| --- | --- | ---: |
| Tdx | available / accepted | 1291 |
| Tdx | partial / manual_confirmation_required | 264 |
| Tdx | unavailable / no_verified_batch | 147 |
| Tdx | unavailable / grpc_bridge_sync_timeout | 12 |
| Baidu | unavailable / router_sources_exhausted | 325 |
| Baidu | partial / router_batch_rejected | 31 |

这些是服务端和客户端共用数据库的**审计行数**，不是去重的请求数或 TDX 物理调用失败数。原客户端在任意桥接失败时默认审计为 `Tdx`，所以表内 `Tdx / no_verified_batch` 不能用于证明 TDX 自身仍失败。该归因已改为成功按批次证据、失败按错误中的提供者；缺少提供者证据则记 `Custom`，仅影响后续新审计行，不改写历史记录。

上游 [688277 停牌缺口交接](share/2026-09-24-688277-suspension-gap.md) 提供 07-16 至 07-29 停牌证据，并报告 90 日序列的真实缺口。本机 BR-092 对超过 5 个交易日的缺口仍拒绝；这需要有证据的停复牌例外，不能用 E2103 修复或无条件放宽连续性校验来处理。

本机只读请求 `codex-local-tdx-recheck-688277-90-20260924` 也复现这一独立拒绝：TDX 批次在 BR-092 被判为 `2026-07-15` 后应有 `07-16`、实际下一行为 `07-30`；其他注册源未形成合格批次，最终 RPC 为 `Internal / no_verified_batch`。这条请求没有出现 E2103。

## 后续处置边界

- 保留 BR-171 人工确认门槛；300005 的 90 日窗口须先完成独立事实核验与当前批次的证据绑定确认。
- 688277 的长窗口须在本机 BR-092 建立可验证的停复牌例外后重新验收；短窗口和长窗口分别计数。
- 按新的 `Custom` 归因重新观察本机路由失败，再决定哪些属于提供者、准入规则或桥接传输。旧审计行无法追溯出被遮蔽的原始 provider/trailer。

## 下游审计修正上线验收（2026-09-24 11:11 CST）

定向回归 `cargo test --lib historical_bridge_failure_does_not_claim_tdx_provider -- --test-threads=1` 通过，`monitor` 和 `selection_activation_prepare` 的 release 构建通过。新 `monitor` SHA-256 为 `d76e4d8a2e88ad0318d59ade63ad6ba2dca4cf2a110f5feb4854aa9d48c728a2`。激活修订值 `1b0207c5365e0261254106d274531b340b70df113aaad4a4d808875a991d7c70` 从 11:07 CST 生效；launchd 在生效后重启，PID `59531` 于 11:07:20 启动。从此次启动日志锚点第 `646349` 行起，`capability=disabled` 为 0。

截至 `2026-09-24T03:11:05.257Z`，此次启动后的 `HistoricalDailyBars` 审计已有 `Tdx / available / accepted=26`、`Tdx / partial / manual_confirmation_required=5`、`Baidu / unavailable / router_sources_exhausted=8`、`Custom / unavailable / no_verified_batch=3`。`Custom` 新行证明缺少提供者证据的桥接错误不再假记为 TDX；这些计数仍是审计行而非去重的 RPC 次数，也不代表长窗口规则问题已修复。
