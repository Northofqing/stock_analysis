# Tdx `HistoricalBars` — 已认领的 E2103 解码缺陷**次日仍完整复现**（2026-09-23 盘中）

2026-09-23 10:2x CST，盘中实测。承接 `2026-09-22-historical-bars-determinism-probe.txt`。

## 0. 摘要

**结论：不是新故障。** 2026-09-22 我方探针记录、你方已认领的 Tdx 解码缺陷
（`[E2103] response length mismatch: security bar row 0 is truncated`）在 **9/23 盘中逐字复现**，
形状覆盖面与昨日一致，**无任何改善**。

⚠️ **一个可能误导定位的表象差异**：生产链路里该故障被包装成
`gRPC HistoricalBars 查询失败: 服务端内部错误` + `reason_code=no_verified_batch`，
**看起来像一个泛化的内部错误**。原始探针证明底层根因仍是 E2103 —— 请勿按「新的
`internal` 故障」另起排查线，那是同一件事的表层。

## 1. 原始探针复现（2026-09-23 10:16:32 CST）

工具 `target/release/tmp_daily_probe`（无参，使用其默认形状），完整输出：

```
now=2026-09-23 10:16:32
RI_K(9)  start=0 count=40  NONE: ERR [E2103] response length mismatch: security bar row 0 is truncated
RI_K(9)  start=0 count=800 NONE: ERR [E2103] response length mismatch: security bar row 0 is truncated
RI_K(9)  start=1 count=40  NONE: ERR [E2103] response length mismatch: security bar row 0 is truncated
RI_K(9)  start=0 count=40  fq1 : ERR [E2103] response length mismatch: security bar row 0 is truncated
DAILY(4) start=0 count=40  NONE: ERR [E2103] response length mismatch: security bar row 0 is truncated
DAILY(4) start=0 count=800 NONE: ERR [E2103] response length mismatch: security bar row 0 is truncated
DAILY(4) start=1 count=40  NONE: ERR [E2103] response length mismatch: security bar row 0 is truncated
DAILY(4) start=0 count=40  fq1 : ERR [E2103] response length mismatch: security bar row 0 is truncated
```

**要点：8 种组合全失败，报文逐字相同。** 覆盖 `RI_K` 与 `DAILY` 两周期、
`start=0/1`、`count=40/800`、复权 `NONE`/`fq1` —— 与 9/22 的
`provider=Tdx → FAIL FailedPrecondition: [E2103] …` 完全一致。

失败与 `start`/`count`/复权**无关**，也与是否带日期范围无关 ⇒ 指向**行级解码**，
而非参数或形状问题。

## 2. 生产侧影响（2026-09-23 09:30:31 – 10:46:16，cutoff 10:46:29 CST）

| 项 | 值 |
| --- | --- |
| `HistoricalDailyBars` provider=Tdx `outcome=unavailable` | **254** 次 |
| 同一时段 provider=Baidu `outcome=available` | 554 次（正常） |
| 因日K取不到而被跳过的卖出评估 | **185** 次 |
| 受影响标的（去重） | **19 只** |

> **计数口径**（便于你方对账）：「被跳过的卖出评估」以 `[paper_sell] … 本 tick 跳过`
> 行计。⚠️ 每次失败在日志中产生**两行**（`日K获取失败…本 tick 跳过` 与
> `评估失败: … 日K获取失败…`），二者**均含**「日K获取失败」字样，按该串直接
> `grep -c` 会得到约 2 倍（352 行 ≈ 176 事件）。上表 185 为去重后的事件数。

受影响标的（部分）：`002780 300005 300137 300244 300347 300638 300792 301176
600018 688277 688359 688561`

生产原始报文（每次调用均为此形）：

```
[DataGateway][HistoricalDailyBars][BR-159] outcome=unavailable provider=Tdx
  source=review-data-gateway observed_at=2026-09-23T02:16:04.340Z source_at=absent
  batch_id=absent requested=1 accepted=0 rejected=1
  reason_code=no_verified_batch retryable=true audit_id=2926658

[paper_sell] 688277 评估失败: 688277 日K获取失败: GrpcBridge data gateway failed
  reason_code=no_verified_batch provider=None retryable=true:
  gRPC HistoricalBars 查询失败: 服务端内部错误 (记录 request_id, 停止无界重试)
```

**路由分化**：同一 `HistoricalDailyBars` 操作下 **Baidu 路由全程正常**（396 次
`available`，`reason_code=accepted`），**Tdx 路由全程失败**。故这是 **Tdx 单路由缺陷**，
非该 operation 整体不可用。

## 3. 业务后果（为什么这条不能挂着）

Tdx 日K 不可用 ⇒ 卖出规则无法评估 ⇒ **该止损的持仓卖不出去**。
今晨开盘后 6 笔卖出中 **4 笔是 ATR 动态止损**——它们恰好是在部分标的仍能取到数据时成交的；
日K 失败面扩大后，同类标的只能**每个 tick 重试并继续跳过**。

这是**风险保护能力被削弱**，不同于一般的行情缺失：我方无法通过降级继续提供卖出保护。

## 4. 请求

1. **确认 E2103 的修复排期**。该缺陷 9/22 已由你方认领，9/23 仍 100% 复现、
   无任何部分改善。请给出预计修复时间，或明确「短期不会修」，以便我方评估是否需要
   在 Tdx 路由之上加一层 failover（转 Tencent/Sina）作为临时缓解。
2. **请确认表象差异的归属**：生产侧看到的 `服务端内部错误` 是否就是 E2103 在上层
   网关的包装结果？若其实存在**两个**独立故障（解码缺陷 + 网关内部错误），
   请指出如何区分——我方目前只能看到包装后的报文。
3. 附：我方探针工具与形状已在上文列全，若需我方按特定形状（含显式日期范围）复测，
   请给出形状定义；9/22 我方曾提出「带日期范围形状不在你方确定性矩阵内」，
   本次探针已额外覆盖 `start=1`，仍然全败。

## 5. 附件

- 原始探针输出：本文 §1（可复现，工具 `target/release/tmp_daily_probe`）
- 昨日对照：`grpc_handoffs/2026-09-22-historical-bars-determinism-probe.txt`
- 生产日志：`logs/monitor-launchd-20260916.stderr.log`（无日期前缀，按 `audit_id` 锚定）
