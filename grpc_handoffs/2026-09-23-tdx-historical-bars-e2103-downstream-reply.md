# 请回传 `audit_id=2926658` 那条链路的 gRPC request_id —— 用于对账

> 下游核查、历史证据边界和本机修复验收见
> [TDX 本机路由调查与修复记录](2026-09-23-tdx-historical-bars-local-route-investigation.md)。
> `audit_id=2926658` 的历史 request_id、原始 status/trailer 未被保存，无法回传；
> monitor 实际连的是 `127.0.0.1:18082`，并未直连 `10.211.55.3:50051`。

承接 `2026-09-23-tdx-historical-bars-e2103-upstream-reply.md` §7 的最后一项。
E2103 本身已修复（14:00:33 CST 部署），本文件只为一件事：**要那几个 id 来对账**。

## 1. 为什么必须要你方给

我方服务端的失败记录**按 `request_id` 索引**，没有别的维度可以反查。
你方 §2 给的 `audit_id=2926658`、时间窗、标的都是你方网关的标识，在我方日志里不存在。

而按我方证据，**你方 §2 的失败窗口（09:30:31–10:46:16 CST）我方服务端日志零写入**
（上一行停在 01:26:31Z，下一行是 14:00:33 CST 的重启）。
也就是说那 254 次调用没有到达本实例的 handler。要确认这一点、而不是停在推测上，
只需要一个 `request_id`：在我方日志里查一次，命中就能定位，查不到就证明它没到这边。

## 2. 请回传这五项

| 项 | 说明 |
| --- | --- |
| `request_id` | 你方发出的 `QueryRequest.context.request_id`；失败时它也会出现在 trailer `magic-error-detail-bin` 的 `ErrorDetail.request_id` 里 |
| `status.code()` | **未包装**的 gRPC 状态码（不要先折叠成 `no_verified_batch`） |
| `status.message()` | 同上，原样；`服务端内部错误` 是包装后的措辞，我方需要包装前的字符串 |
| `trailing_metadata()` 原文 | 特别是 `magic-error-detail-bin`（base64） |
| 目标地址 + 调用时刻 | 你方 Tdx 路由连的是哪个 `host:port`；以及该次调用精确到秒的时间戳（带时区） |

一次失败调用即可，不必给 254 条。

## 3. 怎么取（三步，都很小）

1. 在你方 DataGateway 把异常折叠成 `no_verified_batch` **之前**那一层，打印原始状态：
   如果是 Python gRPC：

   ```python
   except grpc.RpcError as e:
       logger.error("grpc failed code=%s details=%s trailers=%s",
                    e.code(), e.details(), e.trailing_metadata())
       raise
   ```

2. 把 `trailing_metadata()` 里的 `magic-error-detail-bin` 用 base64 解出来。
   里面是 `ErrorDetail`：`request_id`、`reason_code`、`retryable`、`provider`、`provider_attempts`。
   我方发出的 `reason_code` 是封闭集合，落在这个集合外的一定不是你方以为的那件事：

   ```
   capability_unadmitted | source_precondition_failed | invalid_evidence
   internal | provider_route_exhausted | provider_route_stopped
   ```

3. 如果暂时改不动网关代码，用一次 `grpcurl -v` 手工打一发也行。`-v` 会把
   response trailers 原样打出来，那里面就有 `magic-error-detail-bin`。

## 4. 我方拿到之后会做什么

- 用 `request_id` 在 `logs/grpc-server.stderr.log` 里查 `service_failure` / `provider_failure` 记录。
- **命中** ⇒ 那一层确实是我方，按记录的 `stage` 直接定位（`stage=internal` 与
  `stage=source_precondition_failed` 是两条完全不同的线）。
- **查不到** ⇒ 证明这些调用没进本实例的 handler。那么问题在你方网关到
  `10.211.55.3:50051` 之间的那一段，或者根本没有发到这个实例上 —— 这两种情况
  你方都能自己往下查，我方这边没有可查的东西了。

## 5. 顺带一个自检问题

你方 `provider=Tdx` 那条路由，目标是不是 `10.211.55.3:50051`（`magic-market.local`）？
如果不是，或者你方有多个候选实例，请一并告知 —— 那基本就解释了 §3(c) 的日志空窗。

## 6. 另外：现在可以直接复测

Tdx 日线已恢复（14:00:33 CST 起）。形状必须是**省略 start/end** 的那种：

```json
{"instrument":{"exchange":"Shanghai","code":"688277","asset_class":"Equity"},"interval":"Day","limit":5}
```

带显式日期范围按合同返回 `Unimplemented`，那不是缺陷，别作为复测项。
请回报 §2 表里 `provider=Tdx outcome=unavailable` 的计数是否归零。
