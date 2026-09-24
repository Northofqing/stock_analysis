# 2026-09-22 19:00 复盘批次 R-08/R-09: 上游侧核实结论

对 `2026-09-22-review-upstream-failures.md` 的核实回复。结论先行:

1. **R-08 与 R-09 不同源, 且都不是"服务端某共享组件"故障。** R-08 的 operation 现已
   完全正常(实测 300 条 complete); R-09 是一条**真实的上游传输故障, 且不在服务端**。
2. **先纠正一个映射: R-08 的 "EventCalendar 事件日历" 对应的是 `MarketAnnouncements`
   (巨潮全市场公告), 不是 `EconomicCalendar`。** 后者按设计 fail-closed, 而且它**不可能**
   返回 `internal` —— 如果 R-08 真的调的是它, 你方会拿到 `UNIMPLEMENTED`。
3. **R-09 的真因已定位到一条 URL 路径**: `push2.eastmoney.com` /
   `push2delay.eastmoney.com` 的 `/api/qt/clist/get`。同一主机、同一 TLS 会话下的
   **其它路径全部正常**。该路径被四个能力共享 —— 这是"共享组件"的正确答案, 但它是一条
   **共享的上游 URL 路径, 不是服务端组件, 也不在 R-08/LimitPools/HistoricalBars 的链路上**。
4. **你方第 3 条建议已采纳并落地**, 同时修复了一个已确认的上游缺陷(LimitPools 路由早停)。

---

## 1. 映射纠正: R-08 是 `MarketAnnouncements`, 不是 `EconomicCalendar`

`docs/data-sources-inventory.md:29` 记录你方 `src/data_gateway/event_calendar.rs:18-19`
(R-08-announcements)依赖巨潮资讯网 cninfo-market, 即上游的 `Operation::MarketAnnouncements`
(BR-161 按交易日拉取全市场公告)。文件名 `event_calendar.rs` 是这条链路上游被叫成
"EventCalendar" 的来源。

`EconomicCalendar` 是**另一个** operation, 状态完全不同:

- 金十免费日历/API 于 2025-12-01 退役, 该 operation 上游**未准入**, 只保留显式诊断路径;
- 对它的调用返回 `UNIMPLEMENTED` / `capability_unadmitted`(`grpc-external-api.md:12,490,725`);
- 因此它**不会**产生 `internal`。

判别方法在你方日志里: 若 R-08 的失败码是 `UNIMPLEMENTED` 就该查日历语义; 是 `internal`
就只可能来自 `MarketAnnouncements` 链路。**若要复跑, 请用
`{"start":"YYYY-MM-DD","end":"YYYY-MM-DD","limit":300}`, 不要发 `{}`** ——
`{}` 会得到 `InvalidArgument: missing field start`, 合同明确禁止用它调用这个 operation。

## 2. R-08 实测: 当前完全正常

2026-09-22 22:13 CST(19:00 批次之后 3 小时), 对生产 registry 实测:

| 请求 | 结果 |
| --- | --- |
| `{"start":"2026-09-22","end":"2026-09-22","limit":300}` | OK · `Cninfo` · **complete=true · 300 条** · `source_at=2026-09-22T20:44:10+08:00` |
| 同上 `limit=50` + 显式 pin `Cninfo` | OK · `Cninfo` · complete=true · 50 条 · 同一 `source_at` |
| `{"start":"2026-09-15","end":"2026-09-22","limit":300}` | OK · `Cninfo` · complete=true · 300 条 |
| 对照: `{}` | `InvalidArgument`(合同禁止) |

**19:00 那一次无法回溯定因**, 原因是当时服务端**没有留下任何记录** —— 这正是你方第 3 条
指出的问题, 现已修复(见 §5)。现在再发生同类失败, 可以用 request_id 直接定位。

## 3. R-09 实测: 真实上游传输故障, 定位到一条 URL 路径

### 3.1 从服务端看

2026-09-22 22:14 CST, 生产 registry 实测, 全部返回同一个结果:

```
Code: Unavailable
Message: HTTPS transport error: all Eastmoney provider Top-N HTTPS endpoints failed
         without one valid response: https://push2delay.eastmoney.com/api/qt/clist/get
         attempt 3: https://push2delay.eastmoney.com/api/qt/clist/get?pn=1&pz=100&...
reason_code=provider_unavailable   retryable=true
```

覆盖矩阵(**全部同形失败**, 所以与请求无关):

| 变量 | 取值 | 结果 |
| --- | --- | --- |
| kind | `VolumeRatio` / `MainNetInflow` | 都失败 |
| 路由 | pin `Eastmoney` / 不 pin(自动) | 都失败 |
| limit | 10 / 100 | 都失败 |
| trading_date | 2026-09-22(当日) / 2026-09-21(上一交易日) | 都失败 |

你的 `provider=Some(Eastmoney)` 与 `retryable=true` 与此完全一致; 你方的
`source_transport_failed` 就是对这个 `Unavailable` 的分类, 是对的。

### 3.2 从本机直连看(关键判别)

同一台主机上直接发请求, **同一主机、同一 TLS 会话**, 只有一条路径失败:

| URL | 结果 |
| --- | --- |
| `https://www.eastmoney.com/` | 200 |
| `https://push2his.eastmoney.com/api/qt/stock/kline/get?...` | 200 |
| `https://datacenter-web.eastmoney.com/api/data/v1/get?...` | 200 |
| `https://push2.eastmoney.com/api/qt/ulist.np/get?...` | **200** |
| `https://push2.eastmoney.com/api/qt/stock/fflow/kline/get?...` | **200** |
| **`https://push2.eastmoney.com/api/qt/clist/get?...`** | **连接被重置** |
| **`https://push2delay.eastmoney.com/api/qt/clist/get?...`** | **连接被重置** |
| **`http://push2.eastmoney.com/api/qt/clist/get?...`**(80 端口) | **空回复** |

`clist` 用最短查询串(`?pn=1&pz=5`)与完整查询串、`pz` 取 5/10/100 都是同样结果, HTTP 与
HTTPS 都失败。**⇒ 不是主机不可达、不是 TLS、不是 DNS、不是请求形状, 而是
`/api/qt/clist/get` 这条 URL 路径级的阻断/重置。**

### 3.3 共享范围: 四个能力

按端点常量(`crates/magic-eastmoney-rs/src/`):

| 能力 | 端点 |
| --- | --- |
| `ProviderTopNRankings`(R-09) | push2 + push2delay `/api/qt/clist/get` |
| `BoardFlows`(板块资金流) | push2 `/api/qt/clist/get` |
| `MarketRankings` | push2 + push2delay `/api/qt/clist/get` |
| `PostCloseFlows` | push2 + push2delay `/api/qt/clist/get` |

实测确认其中两个(2026-09-22 22:18 CST):

- `BoardFlows` → `UNAVAILABLE`, 报文指名 `push2.eastmoney.com/api/qt/clist/get`
- `PostCloseFlows` → `UNAVAILABLE`, 报文指名 `push2delay.eastmoney.com/api/qt/clist/get`

**所以你方"是否同源"的答案是: 有共享, 但共享的是一条上游 URL 路径, 不是服务端组件。**
它**不解释** R-08、LimitPools、HistoricalBars: LimitPools 走
`push2ex.eastmoney.com`(不同主机), 与 clist 无关。

### 3.4 这条路径为什么断, 还需要一次判别

本机(`10.211.55.3`)没有直连公网: **所有**域名都解析到 `198.18.0.0/15`(RFC 2544 保留
测试网段, 不可能是东财真实地址), 直连真实 IP 全部不可达 —— 出网经过 Mac 宿主上的
fake-IP 代理。因此"阻断"发生在这条出网路径上, 两个候选:

- (a) 代理侧针对该 URL 的规则/上游连接失败;
- (b) 代理的**出口 IP 被东财针对 clist 反爬拦截**(clist 是被抓取最多的端点, 被 RST
  而不是 403/429 也符合边缘反爬形态)。

区分方法: 换一条出口(例如不经代理直连, 或换出口 IP)再取同一 URL。**这一层不在上游仓库
内**, 上游能确认的是: 服务端没有把它变成 `internal`, 分类(`provider_unavailable`,
retryable=true)是正确的, 且四个能力的失败可以归到同一个原因。

## 4. 顺带确认并修复的上游缺陷: LimitPools 路由早停

这是本次核实中发现的**唯一一处上游代码缺陷**, 与你方同日 09:15–09:25 的 LimitPools
`internal` 窗口对应, 已修复。

- BR-059 原文的推进理由: 两种结果推进到下一个候选, "**because each is a fact about that
  candidate rather than about the request**"。
- Eastmoney 是 LimitPools 的**第一个**注册候选, 其适配器要求源端 `qdate` 等于请求交易日。
  盘前 `qdate` 还是上一交易日, 于是第一个候选返回 `FailedPrecondition`, 而它当时被当作
  "不可重试且非 scope decline" ⇒ **路由停止, 后面两个候选从未被询问**。
- 服务端当天自己的记录正是这个形态: 09:13–09:24 共 35 条
  `event=provider_route_failure stage=provider_route_stopped attempt_count=1
  attempts=Eastmoney:source_precondition`。
- 实测(过去交易日 2026-09-21, 过去日期是同一段代码的确定性替身): Eastmoney 返回
  `FailedPrecondition`(qdate 2026-09-22 ≠ 2026-09-21), 而 **Tonghuashun(Upper, 完整 10 条)
  与 HithinkFinance(Upper 10 / Broken 10 / Lower 2)都返回完整且日期正确的批次**;
  未 pin 的生产路由失败。对当前日 2026-09-22 三者全部成功。
- 修复: `execute_limit_pool_route` 增加第三个推进条件。Gate A 见
  `docs/superpowers/specs/2026-09-22-limit-pool-source-precondition-fallthrough-design.md`。
  唯一客户端可见变化: 仍然失败的那种情况由 `provider_route_stopped` 变为
  `provider_route_exhausted`(更准确, 因为路由确实试遍了所有候选)。

**明确不声称**: 09:15–09:25 窗口是否恢复**未**验证 —— 该窗口只在盘前存在, 本次证据在收盘后
取得。可证伪的复跑: 下一个交易日在 09:15–09:25 发同一请求, 读
`stage=provider_route_exhausted attempts=...`。若尝试列表里出现了后两个候选, 人为早停已消除;
若窗口仍然失败, 剩余原因是候选自身的就绪度, 不是这个缺陷。

## 5. 你方第 3 条建议: 已采纳并落地

新增一条有界服务端记录, 覆盖所有"服务端自己失败"的 fail-closed 出口:

```
ts=<RFC3339> level=ERROR target=grpc_server event=service_failure \
  stage=<reason_code> request_id="<id>" operation=<operation>
```

- 覆盖 `source_precondition_failed` / `invalid_evidence` / `internal` /
  `provider_unavailable` / `blocking_worker`;
- **永不携带失败消息**(BR-057: 上游文本不进日志), 只带关联键与分类, 都是低基数、仓库自有的;
- 纯请求错误(你方自己写错的请求)仍然不记录, 避免一个坏客户端把自己的错误变成服务端日志量。

**为什么 `provider_unavailable` 是关键一条**: 它是唯一由两个分支产生的 reason code,
原先只有一个分支记录。实测 2026-09-22 全天, R-09/BoardFlows/PostCloseFlows 的整段故障在
服务端 stderr **零记录** —— 当天日志总共 90 行, 没有任何一行提到这三个 operation。也就是说
**一次持续 3 小时以上、影响四个能力的 Provider 传输故障, 在服务端是完全不可见的**; 而现在
`grep 'request_id="<你方 id>"'` 可以直接回答"这个请求服务端做了什么"。

关于**按 operation 健康度自检**: 我们**没有**加主动探测 —— 那会引入额外的上游调用, 并且
有把 fail-open 引入数据路径的风险。现在能做的是按 operation + reason code 聚合失败记录。

## 6. 你方第 2 条: HistoricalBars 逐码 flaky —— 实测是确定性的, 不存在逐码状态

2026-09-22 22:24–22:28 CST, 形状 `interval=Day, limit=5, 无 start/end`, 6 个码
(600519 / 601398 / 000001 / 300750 / 002594 / 920403)× 7 条路由, **连跑两遍**:

| 路由 | 6 个码的结果 | 两遍是否一致 |
| --- | --- | --- |
| 自动 | **全部 OK** · `Tencent` · complete=true · 5 条(920403 为 1 条) | 一致 |
| `Baidu` | **全部 `Unimplemented`**: trading-calendar / adjacent-session / corporate-action continuity evidence remain unproved | 一致 |
| `Tencent` | 全部 OK · complete=true · 5 条(920403 为 1 条) | 一致 |
| `Sina` | 全部 OK · complete=true · 5 条 | 一致 |
| `HithinkFinance` | 全部 `InvalidArgument`: explicit start date is required | 一致 |
| `EmQuant` | 全部 `InvalidArgument`: 需要显式 start/end 且排除未结束的源交易日 | 一致 |
| `Tdx` | 全部 `FailedPrecondition [E2103]`: response length mismatch · security bar row 0 is truncated | 一致 |

原始输出附在同目录 `2026-09-22-historical-bars-determinism-probe.txt`。

**结论:**

- **没有任何一条路由出现"同请求不同码成功/失败交替"**: 每条路由对 6 个码的表现完全一致,
  第二遍与第一遍逐字一致。⇒ 你方"服务端对部分标的的 verified batch 校验状态不一致"的假设
  **不成立** —— 服务端在这条链路上没有逐标的的状态, 结果是 (提供方, 请求形状) 的纯函数。
- **"部分码 Baidu 成功" 在这个形状下无法复现**: Baidu 对 6 个码**一律**是 scope decline
  (`Unimplemented`), 两遍都是。`Baidu` 的失败是准入事实(交易日历/相邻交易日/公司行为
  连续性证据未证明), 不是逐码的。
- 每条路由的失败都是**确定性的合同/准入事实**, 不是 flaky: Baidu 是 scope, HithinkFinance
  与 EmQuant 是请求形状(需要显式日期), **Tdx 是解码失败**。
- **自动路由当前是好的**(走 Tencent, 6 个码全部 complete)。

**顺带发现一个上游侧缺陷(与你方本次报告无关, 另行跟进):** `Tdx` 路由对每个码都返回
`FailedPrecondition [E2103] response length mismatch: security bar row 0 is truncated`,
100% 可复现 —— 这是上游 TDX 日线解码的确定性缺陷, 我们会单独处理。

**需要你方确认的判别数据**: 你方失败时的**确切请求形状**(是否带 `start`/`end`、`limit`
取值)与 **request_id**。带显式日期范围的形状与上面的形状走的是不同的准入分支, 你方的
`no_verified_batch` 归类需要对上其中一支才能定因; 现在服务端侧已经可以用 request_id 对齐。

## 7. 需要你方提供的判别数据

1. **R-08 实际调用的 operation**: 请打出你方 `event_calendar.rs` 那次失败响应的
   `code` 与 `reason_code`(`internal` 还是 `UNIMPLEMENTED`)。这决定它是
   `MarketAnnouncements` 链路问题还是日历语义问题。
2. **R-09 的出口**: 若你方另有一条不经同一代理的出网路径, 请对
   `https://push2.eastmoney.com/api/qt/clist/get?pn=1&pz=5` 取一次, 用来区分
   §3.4 的 (a)/(b)。
3. **HistoricalBars 失败时的确切请求形状**(是否带 `start`/`end`、`limit` 取值)与
   当时被判为 `no_verified_batch` 的 code 列表(见 §6)。
4. 后续所有上游失败请求请带上 request_id, 现在服务端侧可与之对齐。
