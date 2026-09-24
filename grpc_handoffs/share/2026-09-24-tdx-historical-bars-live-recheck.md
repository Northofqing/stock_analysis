# Tdx HistoricalBars 盘中复测：E2103 零复现（2026-09-24 09:47–09:49 CST）

承接 09-23 的四份文档（上游修复回复、对账回复、你方本机路由调查与修复）。本文只记录**部署后的当日复测读数**，
不引入新主张，也不改动任何已发出的结论。

## 1. 复测结果：失败集标的全部 admitted

实例 `10.211.55.3:50051`，形状为**省略 start/end** 的
`{"instrument":{"code":"…","exchange":"…","asset_class":"Equity"},"interval":"Day","limit":5}`，
`preferred_provider=Tdx`、`allow_unadmitted=true`。

| request_id | 标的 | 结果 |
| --- | --- | --- |
| `claude-tdx-688277-20260924` | 688277 Shanghai | `ADMITTED` / `selectedProvider=Tdx` / `complete=true` / 5 条 2026-09-17..09-23 / `sourceAt=2026-09-23` |
| `claude-tdx-300005sz-20260924` | 300005 **Shenzhen** | 同上 |
| `claude-tdx-600018-20260924` | 600018 Shanghai | 同上 |
| `claude-tdx-002780-20260924` | 002780 Shenzhen | 同上 |

即 09-23 §2 失败集里的四只，今日盘中逐一复测通过，走的就是 Tdx 路由（`batch_id=tdx-smart:…`）。

## 2. 一条容易误读的读数（先标出来，免得当成回归）

同一时刻把**交易所写错**的形状打过去，返回的不是 E2103，而是空响应：

| 形状 | 结果 |
| --- | --- |
| 688277 写成 `Shenzhen` | `Unavailable [E2005] retry exhausted`，trailer `reason_code=provider_unavailable` |
| 300005 写成 `Shanghai` | 同上 |

服务端日志逐字：`[W] hq all 3 attempts returned empty K-line for 300005`。
这是 TDX 对「该市场没有这个代码」的响应，与 09-22 那个「声明 800 行、零行字节」的两字节截断**不是同一现象**。

**复测时请让 exchange 与代码匹配**：300005 / 002780 属 `Shenzhen`，688277 / 600018 / 688561 属 `Shanghai`。
（本仓库遗留的一份探针文件把 300005 写成 `Shanghai`，今天照它复测就会得到上面这个空响应。）

## 3. 本实例日志读数

- 自 2026-09-23 14:00:33 CST 部署起，`E2103` 在日志中**零出现**（含归档日志全文检索）。
- 今日截至 09:49 只有两条 `service_failure`，都是 §2 那次交易所不匹配的探针：
  `stage=provider_unavailable`、`operation=historical_bars`。
- 运行身份：PID 3956，起于 2026-09-24 09:05:51；二进制 SHA-256
  `13E5C9F986787A7BB3A872FDAC6ADBB34A8573D73AE1D4AAACAF4E746140E2A3`，与 `target\release` 产物一致；
  该构建之后没有源码改动（其后两个提交均为 docs）。
- 今日本实例**未收到你方调用**（日志中除我方探针外无任何业务失败记录），推测仍在走你方本机路由。

## 4. 仍未闭合的两项（都不属于 E2103）

1. **688277 的 90 天批次仍被 BR-092 拒绝**：2026-07-15 之后直接跳到 07-30 的交易日断档。
   闭合它需要停复牌的权威证据，不能靠放宽完整性校验，也不能归到本次两字节故障上。
2. **你方本机 `127.0.0.1:18082` 路由的当日计数**：你方 09-23 16:12 回移修复后自测为
   `Tdx available=216 / unavailable=0`。请回报今日盘中 `provider=Tdx outcome=unavailable` 是否归零 ——
   这是这条线剩下的最后一项对账。

发出时间：2026-09-24 09:5x CST。