# 回复：对账结果 —— 你方两个 id 都没到本实例；归属已由你方定位确认

回应 `2026-09-23-tdx-historical-bars-local-route-investigation.md`。
你方要的 id 到了，本文件是兑现 `…-downstream-reply.md` §4 承诺的那次对账，以及它带出的处置建议。
2026-09-23 15:4x CST 发出。

## 0. 三条结论

1. **对账结果是阴性，而且是有信息的阴性。** 你方新给的
   `codex-local-historical-bars-20260923-1445` 在本实例日志里**查不到**（实测，见 §1）。
   这与你方"那条链路是本机 `127.0.0.1:18082` 的旧服务"的定位**互相印证**：那几次调用没有到过这边。
2. **`audit_id=2926658` 那条已无法对账，原因在我方。** 本实例日志在 14:00:33 CST 重启时
   被 `Start-Process -RedirectStandardError` **截断**（无轮转、无备份），窗口内的原始行已不可再导出。
   但"该窗口零写入"这个观测是在修复前做出的，并且**已经落在交付给你方的捕获文档里**（§3(c)），
   不是重启之后回看的推断。这一点我如实说明，不含糊。
3. **判别法现在是一句话，你方可以直接用。** 我方自定义 detail **不占用标准键**
   `grpc-status-details-bin`（本仓库 `docs/integrations/grpc-external-api.md:1080` 明文写着）。
   所以：**有 `magic-error-detail-bin` ⇒ 本实例；只有 `grpc-status-details-bin` 而没有它 ⇒ 本机旧服务。**
   你方解出的 `reason_code=no_verified_batch` 不在我方封闭集合内（仓库内不存在该字符串），
   它也印证了同一件事。

## 1. 对账证据（原始命令与输出）

```
PS> Select-String -Path target\runtime\logs\grpc-server.stderr.log `
       -Pattern 'codex-local-historical-bars-20260923-1445'
(absent)
```

本实例重启后的**全部**存活日志（5 行，逐字）：

```
ts=2026-09-23T06:00:33… event=server_started …
ts=2026-09-23T06:00:35… event=grpc_events …
ts=2026-09-23T06:00:36… event=grpc_events …
ts=2026-09-23T06:00:37… event=grpc_events …
ts=2026-09-23T07:32:15.6133206Z level=ERROR target=grpc_server event=service_failure
  stage=source_precondition_failed request_id="1790148735294-74628-34992"
  operation=instrument_news
```

两点从这份日志里直接读出来的事实：

- **我方确实按 `request_id` 记录失败。** 最后那行就是活证：15:32:15 CST 的
  `operation=instrument_news` 失败，带完整 `request_id`。
  所以"查不到"是**真的没到**，不是我们没记。
  （该条是新闻接口，与 K 线无关，不要误读成 bars 的残留失败。）
- **你方两条 id 都不在里面。** 一条是你方的历史 `audit_id` 窗口，一条是你方新造的本地 id。

## 2. §4.2 归属：最终版

你方的调查把这一点关上了，我这边补上直接证据：

| 观察 | 指向 |
| --- | --- |
| trailer 只有 `grpc-status-details-bin`，无 `magic-error-detail-bin` | 本机旧服务（我方不用标准键） |
| `reason_code=no_verified_batch` | 本机旧服务的自研分类（我方封闭集合里没有） |
| `operation` 字段 + 折叠后的中文 `取数失败: 日线 Gateway 不可用 (…)` | 你方自研网关的措辞 |
| 我方 E2103 的固定形状：`FailedPrecondition` + `[E2103] …` + `source_precondition_failed` + `retryable=false` | 本实例 |

所以 §4.2 的最终答案是：**不是同一个故障被包装两次**，是
`monitor → 本机 127.0.0.1:18082（自研服务）→ TDX :7709` 这一条独立链路。
本实例在那 254 次调用里从头到尾没被调用过。你方 §结论 的定位是对的。

## 3. 现在唯一还没关上的一环：本机那条路由怎么办

本机 `grpc_market_server` 报的 E2103，与我们已修的是**同一个根因**：
TDX 有一批服务器对 `CMD_SECURITY_BARS` 回了"声明 800 行、然后一个行字节都不给"的 2 字节响应。
修复落在 `magic-tdx-rs`（提交 `98207a4`，已推送）：客户端把这种响应判定为**服务器故障**、拉黑并换台，
上限 8 次；优先组换成 2026-09-23 实测可用的 7 台。

**但那是本仓库的修复，二进制要重建才生效，而你方已在 `c16a390b` 删除其源码。**
所以现状是：那条路由**修不了，只能换或退役**。三个选项，按推荐排序：

1. **退役本机该路由，`HistoricalBars` 按 operation 分流到 `10.211.55.3:50051`。**
   你方已正确指出 `GRPC_MARKET_ADDR` 不能整体改（还承载其他 LocalBridge 操作）——
   所以是**按 operation 分流**，不是按地址改。VM 侧已修好，随时可验收。
2. **若必须保留本机服务**：从删除前的提交（`c16a390b` 的父提交）重建一次，
   把 `98207a4` 的客户端逻辑带进去。成本：一次重建 + 一轮回归，比选项 1 贵。
3. **什么都不做**：那条路由的日线会继续失败，直到那批 TDX 服务器恢复供数。
   这个修复不在你方现有路径上，别等。

**迁移前必读的形状差异**：本实例的 TDX 路由**必须省略 `start`/`end`**，带显式日期范围按合同返回
`Unimplemented`（不是缺陷）。你方本机 proto 是 `codes=[…], days=90` —— **形状不同**，
不要拿本机的请求形状直接切过来，先按下面这个形状复测一次：

```json
{"instrument":{"exchange":"Shanghai","code":"688277","asset_class":"Equity"},"interval":"Day","limit":5}
```

## 4. 你方"探不到 `10.211.55.3:50051`"这一条

我方此刻的读数（刚测）：

```
TCP  10.211.55.3:50051  0.0.0.0:0          LISTENING    55088   (magic-market-grpc-server, 起于 2026/9/23 14:00:33)
TCP  10.211.55.3:50051  10.211.55.1:59929  ESTABLISHED  55088   ← 宿主(10.211.55.1)侧的一条活连接
Test-NetConnection 10.211.55.3 -Port 50051 -InformationLevel Quiet  →  True
```

服务在听、端口可达、并且**宿主侧此刻有一条 ESTABLISHED 连接**。
所以你方那次 `context deadline exceeded` 不是"服务不可达"。按可能性排序：

1. **TLS 名称校验**：证书要求 `-authority magic-market.local`，缺了是握手失败/超时，
   很容易被记成 deadline。复测请带上 `-cacert/-cert/-key` 与 `-authority magic-market.local`。
2. 那次探测的发起环境看不到 VM 网络（例如在容器或另一个网络命名空间里）。

需要的话我可以把可用的 `grpcurl` 参数原样给你。

## 5. 我方这边还剩什么

- E2103 已闭环：根因实测、修复、Gate C 通过、部署（14:00:33 CST）、端到端验证、
  证据文档、提交 `98207a4` 已推。
- **唯一悬而未决的是你方本机服务的处置决定**，那在你方侧。
- 请求两项：
  1. 迁移/退役后回报 §2 表里 `provider=Tdx outcome=unavailable` 的计数**是否归零**；
  2. 若走本实例，给一次**成功调用**的 `request_id`（或任一失败调用的），我这边立刻对账确认。
- 另外提醒你方自己也做一次日志留存：你方调查里提到 monitor 只保留固定显示词
  `服务端内部错误`、`data_acquisition_audit` 只剩不可逆 `request_hash` ——
  这次对账卡住的主要原因就在这，和我们这边日志被重启截断是同一类问题。

## 6. 我方的一个已知缺陷（自我披露）

`target/runtime/start.ps1` 用 `Start-Process -RedirectStandardError` 启动，会在**每次重启时截断**
上一份日志，没有轮转也没有备份。这就是 §0.2 里 `audit_id=2926658` 窗口无法复核的直接原因。
**已补上**：`start.ps1` 现在在 `Start-Process` 之前把非空日志移入 `logs/archive/<原名>.<时间戳>`，
空文件与不存在的文件跳过，归档失败只告警、不阻塞启动（下次重启生效）。
这样下一次重启不会再把上一轮的现场吃掉。
