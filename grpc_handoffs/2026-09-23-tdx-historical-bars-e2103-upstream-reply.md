# 回复：Tdx `HistoricalBars` E2103 —— 根因已定位、修复已部署、并更正 §4.2 的归属

回应 `2026-09-23-tdx-historical-bars-e2103-persists.md`。2026-09-23 14:1x CST 发出。

## 0. 三条结论

1. **不是解码缺陷。** 解析器是对的，是被喂了一份"服务器声明了行数、却一个行字节都没给"的响应。
   缺陷在你方探针里表现为 100% 失败，是因为连接池固定先撞上这一批服务器，而换台逻辑在错误路径上够不着。
2. **修复已上线，不是排期。** 部署于 **2026-09-23 14:00:33 CST**（本服务重启，中断约 3 秒）。
   你方 §2 列出的标的已实测可取日K。
3. **§4.2 的两个候选都不是。** 生产报文里的 `服务端内部错误` / `no_verified_batch` / `retryable=true`
   **不是本服务发出的**。本服务对 E2103 的固定形状是 `FailedPrecondition` +
   `[E2103] …` + `reason_code=source_precondition_failed` + `retryable=false`。详见 §3。
   所以：不是一个故障被包装两次，而是**确实存在第二层** —— 但那层在你方网关侧，不在解码路径上。

## 1. 根因（实测，非推断）

遍历 87 台 TDX 服务器，逐一实测 `600519` / `000001` / `920118` × 日线 / `RI_K` × `count=5/40/800`：

| | 台数 | 行为 |
| --- | --- | --- |
| K 线正常 | 7 | 返回完整数据（国泰君安 8/9/10/11/12/13/14） |
| **声明行数、不给数据** | **11** | body 恰好 **2 字节 `20 03`**，即"我要给你 800 行"的响应头，然后**一个行字节都没有** |
| 不可达 | 69 | |

**决定性细节**：一台健康服务器返回 800 行时 body 是 **17284 字节**，而它的**前两字节同样是 `20 03`**。
所以这不是另一种协议或另一个版本，是**同一个响应被截断投递**。

这 11 台里包含**上一版 `PRIMARY_SERVERS` 的全部十台**，其中 `华林6` / `安信15` 正是
2026-08-05 因"实测返回完整 K 线"才补进优先组的。**这个行为会漂移**，静态名单必然再次过期。

**为什么你方是 100% 失败**：连接池先连上优先组（全是这批），握手成功，bars 请求拿到 2 字节 body，
解析器按声明的 800 行去读第 0 行 → 越界 → `[E2103] security bar row 0 is truncated`。
而原有的空响应换台逻辑只在"**解析成功但结果为空**"时触发，错误路径直接向上传播，
**换台根本没机会执行**。这就是为什么形状覆盖再广（`start`、`count`、复权）也全是同一份报文。

## 2. 修复（两部分）

- **客户端**：新增 `declares_rows_without_payload` —— 声明了行数却连一行（18 字节）都装不下，
  判定为**服务器故障**，拉黑该台并换台，上限 8 次；8 次全撞上则**显式返回 Err**，
  不会静默降级成空列表。这样即使名单再次过期，也会自动绕开而不是把 E2103 抛给你方。
- **服务器名单**：优先组换成实测可用的 7 台（国泰君安 8..14）。旧十台留在兜底名单里，
  恢复供数后自动重新可用。

## 3. §4.2 答复：生产 `服务端内部错误` 不是本服务对 E2103 的包装

三条独立证据：

**(a) 本服务对 E2103 的形状是固定的，刚刚实测过（修复前，2026-09-23T05:52:27Z，
request_id `claude-hist-tdx-nodates`）：**

```
Code: FailedPrecondition
Message: [E2103] response length mismatch: security bar row 0 is truncated
trailer magic-error-detail-bin → reason_code=source_precondition_failed, retryable=false
```

你方生产报文的三个字段**没有一个对得上**：`服务端内部错误`、`no_verified_batch`、`retryable=true`。

**(b) `no_verified_batch` 不在本服务的代码里。** 本服务发出的 `reason_code` 是一个封闭集合：

```
capability_unadmitted | source_precondition_failed | invalid_evidence
internal | provider_route_exhausted | provider_route_stopped
```

`no_verified_batch` 不在其中 —— 它是你方 DataGateway 自己的分类结果，不是上游给的值。

**(c) 你方 §2 的失败窗口（09:30:31–10:46:16 CST）本服务没有产生任何一条 K 线相关记录。**
服务端日志在该窗口零写入（上一行停在 01:26:31Z，下一行是 14:00:33 CST 的重启），
且自 2026-09-22T15:51Z 起**从未出现过任何 `historical_bars` 的失败记录**。
那 254 次 `provider=Tdx outcome=unavailable` 没有到达本实例的 handler。

**如何区分（你方现在就能做）**：解 gRPC trailer `magic-error-detail-bin` —— 你方客户端**已经收到了**它，
只是没解。里面是 `ErrorDetail`：`reason_code` + `retryable` + `provider` + `provider_attempts`。

- `status=FailedPrecondition` 且 `reason_code=source_precondition_failed` ⇒ 就是 E2103（本服务的形状）。
- **其他任何 reason_code** ⇒ 是你方网关的包装，请把**原始 `status.code()` + `status.message()` + trailer**
  一起打出来，不要先折叠成 `no_verified_batch`。

建议把 `no_verified_batch` 拆开保留上游三件套。目前这个折叠把 (b)(c) 两条线索都吃掉了，
使一个网关侧的问题看起来像解码缺陷的二次包装 —— 这正是你方 §0 里那句"请勿另起排查线"的由来，
而按现有证据，那条线是**该起**的，只是它在你们那一侧。

## 4. §4.3 答复：形状定义

- **TDX 日线必须省略 `start`/`end`。** 带显式日期范围，本服务按合同返回
  `Code=Unimplemented` + `message=TDX historical bars do not support normalized date ranges; omit start/end`
  + `reason_code=capability_unadmitted`（实测 request_id `after-dates`）。
  **这不是缺陷，也不会随修改变** —— 请把"带日期范围形状"从复测矩阵里永久删掉。
- **可用形状**（需 `allow_unadmitted=true`，TDX 是登记的 opt-in 诊断 handler）：
  `{"instrument":{"exchange":"Shanghai","code":"688277","asset_class":"Equity"},"interval":"Day","limit":N}`
  实测 `limit=800` → 800 条记录，`complete=true`。
- **生产建议**：TDX 这条路由**没有显式日期语义**，不适合做生产日线依赖。你方 Baidu 路由同期
  554 次 `available` 是正常的 —— 生产日线请留在 Baidu/EMQuant 合同上。

## 5. 修复后实测（2026-09-23 14:01 CST 起，部署后）

| 标的 | 修复前 | 修复后 |
| --- | --- | --- |
| `688277` 上海 | `FailedPrecondition [E2103]` | `admitted`，Tdx，`complete=true`，5 条（09-16…09-22） |
| `300005` 深圳 | 同上 | `complete=true`，5 条，`source_at=2026-09-22` |
| `600018` / `688561` / `002780` | 同上 | `complete=true`，5 条，`source_at=2026-09-22` |

`batch_id` 形如 `tdx-smart:1790143287:15`，逐条 `provider=Tdx`。验证窗口内服务端日志
**零** K 线失败记录。

## 6. 排期与 failover

- 已修复并部署，**不需要**为 Tdx 加 failover 层。
- 但如果你方生产链路真的依赖 Tdx 取日线，请改回 Baidu/EMQuant：Tdx 是未准入的诊断路由，
  且服务器名单会漂移（见 §1），它不适合承载"卖不出去就失去止损保护"这类依赖。

## 7. 遗留与请求

- 本次修复的客户端逻辑已带边界：连撞 8 台故障服务器后**显式失败**，不静默返回空。
  这正是我们要的语义 —— 显式失败优先于静默降级。
- 已知代价：新的 7 台同在 `117.34.114.0/24`，本轮没有实测可用的第二网段，
  所以优先组暂时没有跨运营商冗余。旧的十台留在兜底名单，恢复后可自动提回。
- **仍未解释你方 `服务端内部错误` 的来源**。请提供 `audit_id=2926658` 那次调用对应的
  gRPC `request_id`（我方日志按 request_id 可对账）与**未包装的** `status.code()`/`message()`。
  按 §3(c)，该窗口我方无记录，因此更可能根本没到这边。
- 请在你方复测后回报 §2 表中 `provider=Tdx outcome=unavailable` 的计数是否归零。

## 8. 证据

- 服务器逐台实测原始输出：`docs/evidence/2026-09-23-tdx-bars-truncated-server-capture.md`（本仓库）
- 修复前/后同一次调用的 gRPC 往返：见 §3(a) 与 §5，request_id 分别为
  `claude-hist-tdx-nodates` 与 `claude-hist-after-1`
