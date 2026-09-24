# 我方侧对账：你方 09-23 报的失败集 12/12 复测通过（2026-09-24 盘中）

承接 [盘中复测](2026-09-24-tdx-historical-bars-live-recheck.md) 与 [688277 停牌缺口说明](2026-09-24-688277-suspension-gap.md)。
你方 §2.1 的 **809 次 / 0 次成功** 那组标的是这条线最后没对完的账，本文把我方这半边先做成硬数据，
不等你方回报也能判定「路由本身是否健康」。

## 1. 全量复测：失败集 12 只 + 控制组 6 只 = **18/18 通过**

实例 `10.211.55.3:50051`，形状 `{"instrument":{"code":"…","exchange":"…","asset_class":"Equity"},"interval":"Day","limit":5}`，
`preferred_provider=Tdx`，`allow_unadmitted=true`。交易所按代码前缀给定（`6/9`→Shanghai，`0/3`→Shenzhen）。

失败集（取自你方 §2 的受影响标的）：

| 代码 | 交易所 | admission | provider | complete | 条数 | sourceAt |
| --- | --- | --- | --- | --- | --- | --- |
| 002780 | Shenzhen | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 300005 | Shenzhen | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 300137 | Shenzhen | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 300244 | Shenzhen | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 300347 | Shenzhen | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 300638 | Shenzhen | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 300792 | Shenzhen | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 301176 | Shenzhen | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 600018 | Shanghai | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 688277 | Shanghai | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 688359 | Shanghai | ADMITTED | Tdx | true | 5 | 2026-09-23 |
| 688561 | Shanghai | ADMITTED | Tdx | true | 5 | 2026-09-23 |

控制组（你方未报故障）：`600519 600396 000001 002131 600036 300750` —— 同样 6/6 `ADMITTED / Tdx / complete=true / 5 条 / sourceAt=2026-09-23`。

`sourceAt` 是 2026-09-23 属正常：今日盘中尚未收盘，按 BR-022 当前交易日的形成中行不返回。
**18 只里没有任何一只出现 `[E2103]`、空 K 线或 reason_code 失败。**

## 2. 一个复测前提（同样的坑，再说一次）

**exchange 必须与代码匹配。** 把 688277 写成 `Shenzhen`、或把 300005 写成 `Shanghai`，返回的是
空响应 → `Unavailable [E2005] retry exhausted`（`reason_code=provider_unavailable`），
服务端日志作 `all 3 attempts returned empty K-line`。今天这两条失败记录全部来自我方这种错形状探针，
与本路由的健康状况无关。你方 09-23 的探针文件里 300005 就是写成 `Shanghai` 的，请核对。

## 3. 我方实例侧今日的计数

- 今日（截至 10:0x CST）本实例**未收到你方任何业务调用**：日志中除我方探针外没有任何请求，
  也没有任何 `service_failure`。
- 自 2026-09-23 14:00:33 部署以来，`E2103` 在本实例日志中零出现（含归档全文检索）。
- 运行身份：PID 3956，起于 2026-09-24 09:05:51，二进制 SHA-256 `13E5C9F9…`（与 `target\release` 一致）。

## 4. 还差你方那一格

我方这半边到此是闭合的：路由健康、失败集全通过、E2103 零复现、688277 断档另有停牌证据。
**剩下唯一一项仍在你方**：修复后的本机 `127.0.0.1:18082` 路由，`provider=Tdx outcome=unavailable`
今日盘中是否归零（你方 09-23 16:12 自测为 `available=216 / unavailable=0`）。
若你方已改走本实例，则给一条成功调用的 `request_id` 即可，我这边立刻对账确认。

发出时间：2026-09-24 10:0x CST。