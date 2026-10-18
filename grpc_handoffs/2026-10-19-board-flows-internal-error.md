# BoardFlows 在 Windows 服务端返回内部错误（Mac 侧复现记录）

日期：2026-10-19
提出方：Mac 侧 monitor（stock_analysis）
对应：`2026-10-18-data-source-gaps-response.md`（其中 D1/D2 判定为「已实现，用 BoardFlows」）

---

## 现象

Mac 侧调 `BoardFlows`（op 36），**两条通道都失败**，错误完全一致：

### 通道 1：Mac LocalBridge（`GRPC_MARKET_ADDR=http://127.0.0.1:18082`）

```
GrpcBridge data gateway failed
reason_code=provider_transport
provider=Some(Eastmoney)
retryable=true:
gRPC BoardFlows 查询失败: 服务端内部错误 (记录 request_id, 停止无界重试)
```

### 通道 2：ExternalV1 bundle → Windows（`GRPC_MARKET_CLIENT_BUNDLE=...`，127.0.0.1:50052）

同一错误，逐字符一致。

三类 `category` 全部失败：`Industry` / `Concept` / `Region`。

---

## 请求

按 `E2319F5` 文档的 schema：

```json
{
  "category": "Industry",
  "interval": "Day1",
  "limit": 5
}
```

实际发送时**同时带了 `kind` 字段**（Mac LocalBridge 的 `fetch_board_flows` 读 `kind`），
两套参数都在，两种通道都能解析：

```json
{
  "kind": "Industry",
  "category": "Industry",
  "interval": "Day1",
  "limit": 5
}
```

---

## 已排除的假设（逐条给证据）

| 假设 | 证据 | 结论 |
|---|---|---|
| 参数名写错（`kind` vs `category`） | 已同时传 `category`+`interval`+`kind`，仍然失败 | 排除 |
| descriptor 是旧的 | 新旧 `market.proto` 的 sha256 完全一致（`e973df8f3e06a727…`） | 排除 |
| 身份钉拒绝连接 | `src/grpc_client/build_identity.rs:311` 注释明写：2026-10-15 已把四项身份全部降级为**观测信号**，漂移只告警不阻断 | 排除 |
| Mac 侧到东财被拦 | 走 ExternalV1 到 Windows 也是同一错误；Mac 的 push2 拦截不影响 Windows | 排除 |
| op 未注册 | 回复里给了 `grpc_production.rs:3152-3171` 的 `admitted(Operation::BoardFlows, "Eastmoney", ...)` | 排除 |

`provider=Some(Eastmoney)` 说明**请求已经到 provider 层**，是 provider 内部失败，不是准入/参数/传输问题。

---

## 需要 Windows 侧确认的三件事

1. **`BoardFlows` handler 是否真的调到了 `magic-eastmoney-rs::board_flow`**
   服务端日志里这条 request 的堆栈或 error 原文是什么？
   （Mac 侧只能看到 gRPC 的 `INTERNAL`，拿不到服务端详情）

2. **Windows VM 到 `push2.eastmoney.com` 是否可达**
   `board_flow.rs` 用的是 `category=m:90+t:2` 那套接口（行业/概念/地域）。如果这条路径被代理拦了，
   表现就是 provider 内部错误。

3. **对照测试**：`BoardMemberships(preferred_provider=Eastmoney)` 是否正常
   你们回复里说「线上已用 preferred_provider=Eastmoney 验证 600519/000338/600396 返回 eastmoney\* 记录」。
   如果那个能通而 `BoardFlows` 不能，说明是 `BoardFlows` 单点问题，不是东财整体不可达。
   **这个对照能直接定位问题范围**，麻烦一起给结论。

---

## Mac 侧已完成的部分（等这个修好就能用）

- `board_ranking_snapshot::refresh_board_rankings` 已改为 `fetch_both()`：
  一次 `BoardFlows` 调用，涨幅按 `return_pct` 重排成 f3，净流入用原序成 f62；
  整体失败才回退新浪（只有涨幅，没有净流入）。
- `BoardMemberships` 已支持 `preferred_provider`（`board_attempt.rs` / `client.rs` /
  `grpc_source.rs::memberships_session` 三处已加参数，收盘后产业链那条链路传 `None` 保持 Tdx）。
- 探针：`src/bin/board_flows_probe.rs`，三类 category 逐一验证。

**所以只要 `BoardFlows` 在服务端能返回数据，Mac 侧不需要再改任何东西。**

---

## 附：Mac 侧拿到过的唯一一次成功（供参考）

2026-10-18 深夜，同一份 Mac 代码、同一条 LocalBridge 通道，曾成功返回过一次：

```
[BoardFlows] === 行业 (Industry) === 成功 5 条
1. BK1036 半导体    -0.76%  主力净流入 34.22 亿  领涨 688825 长鑫科技
2. BK1204 国防军工  +4.46%  主力净流入 29.34 亿  领涨 601606 长城军工
3. BK1211 汽车      +2.13%  主力净流入 28.07 亿  领涨 600418 江淮汽车

[BoardFlows] === 概念 (Concept) === 成功 5 条
1. BK0917 半导体概念 -0.18%  主力净流入 39.81 亿
2. BK0490 军工      +2.50%  主力净流入 37.23 亿
...
```

约 40 分钟后同样的调用开始稳定失败。这个「先成功、后持续失败」的形态，
更像上游限流/连接被掐，而不是代码路径不通。**如果是限流，请告知配额**，
Mac 侧可以加间隔或降频。
