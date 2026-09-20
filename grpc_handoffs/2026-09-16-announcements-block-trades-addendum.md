# 补充交接：公告请求与大宗交易响应绑定

日期：2026-09-16。接续 [GD-001～013](2026-09-16-downstream-data-gaps.md)，新增 GD-014、GD-015。来源为本轮新完成的 [新闻/虚拟/盘后侧路静态追链](../docs/push-system/news-virtual-side-routes-call-chain-2026-09-16.md)，其原报告 SHA-256 为 `c4e1e4e8f39081c1d20527c5633771c7c723d56a5a9b64840161701435a2dd41`，25 项文件快照已核。代码引用均指隔离开发树 W，不指代当前生产制品；本补充没有调用服务、读取生产库或验证远端返回。

## GD-014：公告日期与条数只存在于本地意图，未进入实际请求

- 优先级/分类/证据强度：P1；上游/协议合同待确认 + 请求意图与 wire 分离；直接静态合同差异。本轮没有调用 Announcements，不能写成服务端实际返回错日或超量。
- 时段/影响：盘后 IPO 催化；复盘历史补推也会经过同一侧路。客户端不能据此证明拿到的是指定业务日、最多 300 条的公告。
- 所需数据：实际 wire 中可重证的业务日期、limit、排序/截断和历史范围语义；返回批次的 provider/source/source_at/observed_at/batch_id；逐行 announcement_id、code、published_at、category、canonical URL；请求集合与返回集合的绑定结果。
- 实际调用：[EventCalendarGateway::market_announcements(date,limit)](/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905/src/data_gateway/event_calendar.rs:34) 用 date/limit 计算本地 acquisition request hash，但随后调用无参数 announcements_async；[真实请求](/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905/src/data_gateway/grpc_source.rs:3268) 的 Operation::Announcements 参数是 `{}`。
- [转换器](/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905/src/data_gateway/grpc_source/convert.rs:1401) 未按该 date 筛选或按 limit 截断；[IPO 消费者](/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905/src/bin/monitor/push_templates.rs:7763) 请求 date/300，或复用仅以 date 标记的进程内公告 records 缓存。缓存标签不是远端已经执行此日期请求的证明。
- 责任边界：服务端确认 Announcements 空参数的实际默认日期、条数、排序、历史范围和返回批次证据；客户端负责把业务意图、实际 wire 以及返回集合分别保存和校验。不能仅改本地 hash 就称协议已绑定，也不能先宣称服务端无历史数据。
- 最小改动候选：先确认现有 operation 的版本化合同；若支持 date/limit，显式传入并绑定响应；若只支持固定范围，保留真实范围并明确拒绝不受支持的历史意图，或按原始记录时间执行有证据的本地筛选。不得用本机日期给返回行补造 published_at。
- 验收：分别传 D1/D2 与不同 limit，记录真实请求、批次范围、原始记录与转换结果；注入跨日/超量/时间缺失、VerifiedEmpty、Unavailable；历史补推不能把今天公告标成历史当日。新增数量限制或接口变更须形成明确合同，不能默默丢记录。

## GD-015：大宗交易 wire 已带日期，但本地审计与消费投影未闭合

- 优先级/分类/证据强度：P1；客户端审计/投影丢失与消费口径不一致；直接静态合同差异。本轮没有调用 BlockTrades，不能写成服务端实际返回错证券、错日或重复行。
- 时段/影响：盘后复盘内的大宗协议成交侧推 MU-block-confirm；同日不同 ReviewTask、手动和历史补推均可能调用，不是独立盘中实时成交源。
- 所需数据：规范化请求 codes/date 与其审计 hash；provider/source/source_at/observed_at/batch_id；逐成交稳定 row ID、code、traded_at、price、volume 及明确单位/精度；若要断言成交类型、实时确认或交收期，还须分别有对应来源字段，发送时间独立保存。
- [BlockTradesGateway](/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905/src/data_gateway/block_trade.rs:38) 的 acquisition request hash 只含 codes；[实际 wire](/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905/src/data_gateway/grpc_source.rs:3389) 已正确传 codes/date。因此这是本地审计身份缺日期，不应误报为服务端没有收到日期。
- [转换器](/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905/src/data_gateway/grpc_source/convert.rs:2050) 保留 code、可选 traded_at、price、volume 等，但不校验返回证券是否属于请求集、traded_at 是否属于请求日。[侧推投影](/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905/src/bin/monitor/push_templates.rs:8026) 把 name 写成 code、hhmm 写成发送时本地时间，volume 从 f64 转 u32，并固定 Agreed/realtime=true/NextSession；原成交时刻、交易身份和 volume 单位没有形成这些断言的证据。
- 责任边界：服务端确认成交行稳定身份、volume 单位/精度、成交日期与成交类型/确认/交收字段能否提供；客户端把 date 纳入审计身份、核返回集合/时间、保留原 batch/row，并停止把固定值或发送时间冒充已证明的成交事实。当前未验证服务端是否已严格过滤，不能把客户端缺校验写成服务端实际返回错行。
- 最小改动候选：显式保存原 codes/date/wire/hash/批次/行身份，按约定单位做可检查的数量换算；缺类型/交收/实时确认依据时标明不可用，而非补默认。逐成交事实投递状态另由下游负责，code 的 300 秒冷却不是成交行完成记录。
- 验收：同 codes 不同 date 的审计身份不同；错 code、错日、缺时间、重复行、同票多笔、单位/小数/数量边界分别验证；原始成交时间与发送时间独立；不得因首笔同票成功就漏掉其他合法成交。

## 不应归咎于行情 gRPC 的相邻问题

本轮另核到：[交易/风控](../docs/push-system/trade-risk-call-chain-2026-09-16.md) 的 OrderUpdate 缺生产发布、TradeEventSource 未注册、账户完整性缺券商同步水位；[虚拟观察仓](../docs/push-system/news-virtual-side-routes-call-chain-2026-09-16.md) 从空字符串提取候选。它们分别需要事件源/账户源/客户端接线，不能用“gRPC 有行情了”作为完成证据。通知未知结果、冷却和恢复问题保留在业务报告，本补充不授权生产修复。
