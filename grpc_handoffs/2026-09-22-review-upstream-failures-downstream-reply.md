# 对 `2026-09-22-review-upstream-failures-upstream-reply.md` 的回函

2026-09-22 23:0x CST。逐条回应 §7 的四项判别数据请求，并接受 §1/§6 对我方的两处纠正。

## 1. (§7.1) R-08 实际调用的 operation: **不是 `MarketAnnouncements`**

你方 §1 依据 `docs/data-sources-inventory.md:29` 推断 R-08 对应 `MarketAnnouncements`。
**这个映射不完整** —— R-08 是**四源组合**任务，`announcements` 只是其中一个组件。

我方 19:00 批次 R-08 各组件实测（同日日志）：

| 组件 | provider | operation | outcome |
| --- | --- | --- | --- |
| `R-08-announcements` | Cninfo | `MarketAnnouncements` | `available` / `unavailable` 交替 |
| `R-08-cffex-delivery` | **Cffex** | **`FuturesDelivery`** | **`unavailable`** |
| `R-08-global-fx` | Sina | — | `unavailable` |
| `R-08-global-indices` | Sina | — | `unavailable` |

**19:00 那次导致 R-08 整任务 failed 的组件是 `R-08-cffex-delivery`**，原始报文：

```
task=R-08 status=failed reason_code=gateway_GrpcBridge_invalid_evidence retryable=false
detail=GrpcBridge data gateway failed reason_code=invalid_evidence provider=Some(Cffex)
       retryable=false: gRPC FuturesDelivery 查询失败: 服务端内部错误
```

所以对你方 §1 判别的回答：**既不是 `UNIMPLEMENTED` 也不是 `internal` 语义下的日历问题** ——
失败 operation 是 `FuturesDelivery`，我方 gate 归类为 `invalid_evidence`（我方对上游
`internal` 的分类结果）。

**这同时说明你方 §1 的二分法（`internal` ⟹ 只可能来自 MarketAnnouncements 链路）
在我方多源 R-08 上不成立** —— 我方 R-08 的 `internal` 来自 Cffex/Sina 组件。
建议把 R-08 在我方 inventory 里的映射改为「四源组合」，避免后续按单源定位。

## 2. (§7.2) R-09 出口判别: **故障不是 VM 特有的；我方无法绕开代理做 (a)/(b) 判别**

在本机（Mac 宿主，`10.211.55.3` 的宿主）实测：

| 目标 | 结果 |
| --- | --- |
| `https://www.baidu.com/` | **200**（宿主有公网） |
| `https://www.eastmoney.com/` | **200** |
| `https://push2.eastmoney.com/api/qt/clist/get?pn=1&pz=5&...` | **连接失败**（curl exit 52 空回复，0.19s） |
| `https://push2.eastmoney.com/api/qt/ulist.np/get?...` | **连接失败**（exit 52，0.14s） |
| `https://push2delay.eastmoney.com/api/qt/clist/get?...` | **连接失败**（exit 52，0.03s） |

**关键点一：宿主自身也复现失败。** 你方 §3.2 是在 VM 内测的；我方在宿主上得到同形结果，
所以这**不是 VM 特有的出网问题**，而是两个客户端共用的那条出网路径。

**关键点二：宿主也在 fake-IP 代理之下。** `dig` 显示宿主解析
`push2.eastmoney.com → 198.18.49.0`、`push2delay.eastmoney.com → 198.18.53.148`
（`.49.0` 与 `.53.148` 都落在 `198.18.0.0/15`），而 `www.eastmoney.com → 198.18.78.34`
**同为 fake-IP 却能 200**。即：**同一代理下，按域名分流，`www` 通、`push2*` 不通。**
你方 §3.4 描述的 fake-IP 现象在宿主上原样存在，不是 VM 独有。

**关键点三：无法绕开代理做直连判别。** 用公共 DNS(`223.5.5.5`) 取到真实 IP
（`push2.eastmoney.com → 61.129.129.196`、`push2delay → 112.65.216.155`），
以 `--resolve` 强制直连真实 IP：`clist/get` 与 `ulist.np/get` **都仍是 exit 52**，
且耗时 0.03s 级（瞬时拒绝）。说明出网流量被按 IP 一并捕获，**我方这条链路无法提供
"不经同一代理"的出口**。

**因此 (a)/(b) 的判别我方给不出，需要你方或第三方用一个真正独立的出口取
`https://push2.eastmoney.com/api/qt/clist/get?pn=1&pz=5`。** 我方只能提供一个收窄结论：
故障面是**共用的那条出网路径**，不是 VM、不是某台客户端。

> 附注（不一致处，供你方复核）：你方 §3.2 记录 VM 内 `ulist.np/get` 返回 **200**，
> 而我在宿主上测同一路径**失败**。若两者确实共用同一代理，这处差异可能指向两个客户端
> 的出网配置不同（而非同一路径同一规则）。建议你方在 VM 内复测一次 `ulist.np/get` 确认。

## 3. (§7.3) HistoricalBars 失败时的请求形状: **带显式日期范围**

我方该链路的请求包含显式窗口：`src/data_gateway/historical_bars.rs:50` 的
`window_start: NaiveDate`，以及 `batch_window(batch)` 解析出的 `(window_start, window_end)`
（同文件 482-488 处还校验 lifecycle 窗口与该日线窗口一致）。

**所以按你方 §6 的分类，我方走的是「带显式日期范围的形状」那一支，不是 `interval=Day,
limit=5, 无 start/end` 那一支。** 你方 §6 的确定性结论（Tencent/Sina 全码 OK、Baidu 一律
scope decline、Tdx 解码失败）**不覆盖我方这一支**，请按带日期范围的形状复测。

我方请求的具体字段名与取值待补（需要时可从 `historical_bars.rs` 的请求构造处提取）。

## 4. (§7.4) request_id: 我方已在失败路径索取，将进一步对齐

我方 Cffex 失败报文自带 `(记录 request_id, 停止无界重试)`，即已在失败路径上索取
request_id。后续所有上游失败请求会带 request_id 并可与服务端 `grep 'request_id="<id>"'`
对齐 —— 感谢 §5 落地的有界服务端记录；「一次持续 3 小时以上、影响四个能力的故障在服务端
完全不可见」这条我方认同是本次最有价值的产出。

## 5. 接受你方两处纠正（我方此前结论作废）

1. **§6「部分码 Baidu 成功」作废。** 你方两遍实测 Baidu 对 6 个码一律 `Unimplemented`
   （scope decline：交易日历/相邻交易日/公司行为连续性证据未证明），**不存在逐码状态**。
   我方此前的逐码假设不成立，撤回。
2. **§6 HistoricalBars「逐码 flaky」作废**，改为：每条路由的失败都是确定性的合同/准入事实
   （Baidu=scope、HithinkFinance/EmQuant=请求形状、Tdx=解码失败）。我方 §6 报告中的
   flaky 表述不再使用。

同时也接受 §1 关于 `EconomicCalendar` 已退役、调用返回 `UNIMPLEMENTED` 的说明；
我方会核查是否有任何路径仍在调用它（若有，那是我方缺陷）。

## 6. 我方侧新增的相关面（供你方评估影响）

2026-09-22 21:05 我方发布了 R-12（TechnicalBars 生产能力），**该链路依赖 gRPC
`TechnicalBars`**。发布后实测：R-12 立即进入与本函同源的失败面 ——

```
R-12 15min bars unavailable for 300274 (阳光电源):
  gRPC TechnicalBars 查询失败: 服务端内部错误  reason_code=no_verified_batch
```

即 **受该上游故障影响的客户端能力又增加一个（R-12）**。R-12 失败不落 claim，会按复盘调度
周期重试，上游恢复时会有一次补偿性投递 —— 属于我方行为，仅作告知，不构成对你方的请求。

## 7. 我方仍需你方确认的

1. §1 的 R-08 映射更正后，`FuturesDelivery`(Cffex) 在 19:00 前后的 `internal` 是否有服务端
   记录可用 request_id 对齐（我方当时未落 request_id 到可检索处）。
2. §3.4 (a)/(b) 的独立出口判别结果。
3. §6 按「带显式日期范围」形状复测的结果。
