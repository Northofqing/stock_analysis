# 集合竞价观测交接（L1 范围）

日期：2026-10-18
对应：`2026-10-18-data-source-gaps.md` D7
结论：**D7 在 Windows 侧按 L1 快照收口。** 9:15–9:20 的逐步变化不由 Windows 提供。

## Windows 提供什么

`CurrentAuctionObservations`（append-only 第 61 个只读 op）：

- 请求 schema：`magic.market.current_auction_observations.request`
  `{"instruments":[...],"stage":"live|final"}`
- 1..=100 只 A 股，每只票恰好一行，按请求顺序。
- 字段：竞价量、竞价额、开盘价、最新价、昨收、涨幅、量比、`auction_unmatched`。
- `auction_volume` 已由手转股；`auction_unmatched` 是源端单一有符号原值，
  不映射买卖方向，不能拆成 bid/ask 队列。
- 零价格映射为 `null`；零量/零额保留为 0。
- 无 `trading_date`；record/batch `source_at` 缺失；`observed_at` 是 Fuyao 响应组装时间。
- `stage=live` 在竞价窗口内使用，`stage=final` 在窗口后使用。

规则依据：BR-060；接口文档 `grpc-external-api.md` 的 CurrentAuctionObservations 节。


## 可用时段（2026-10-18 实测补充）

- `stage=final` 只在源端 `auction_phase=closed` 时可用。夜间/盘前调用会返回
  `Protocol("final auction response reports a non-closed auction phase")`，这是
  fail-closed，不是接口坏了。
- 09:25:00 整点可能仍是 `matching`，建议 09:25:05 之后重试 2–3 次，或先用 `live`
  再切 `final`。
- 2026-10-16 盘中 probe 拿到过 `records=1`；2026-10-18 23:xx 实测 `final` 不可用。
  具体每日可用到几点没有测，以 `auction_phase=closed` 为准。
- 如果看到 `invalid type: map, expected f64`，那是 Mac 侧 4 个比率字段
  （`{value, unit}`）的旧解析器问题，2026-10-16 已修；确认运行的是修复后的 binary。

## Windows 不给什么

- 9:15–9:20 的逐笔/逐步序列；
- 未匹配买量/卖量拆分（源只有一个 `auction_unmatched`）；
- L2 逐笔委托/成交；
- 交易日、Provider 源时刻、买卖方向推断。

## Mac 侧需要做什么（L1）

1. 只要竞价快照：直接调 op 61，`stage=live` 在窗口内轮询，用 `observed_at` 自行建时间序列；
   不要伪造 `source_at` 或 `trading_date`。
2. 如果要 9:15–9:20 的逐步变化：Mac 自己接 L1 行情源（券商/终端），
   Windows 不提供；L1 的字段对齐、时间戳、存储由 Mac 负责。
3. 不要把 `auction_unmatched` 当买卖队列；不要用零值补缺失；不要把快照当逐笔。

## 以后如果要在 Windows 供 L1 序列

需要：L1 数据合同（字段/频率/时间戳/许可）+ Gate A + 新 op 或扩展现有 op；
L2 逐笔另需交易所授权。当前不立项。

## 2026-10-19 修复（09:25 窗口）

Fuyao 在 09:25 撮合后先返回 `phase=matched`、`status=final`，旧二进制只认
`closed`，所以 09:25–09:30 的 `stage=final` 会整批报
`final auction response reports a non-closed auction phase`。新二进制
（source `56fba28` / binary `1388a861…`）接受 `matched` 与 `closed`；
`matching` + `final` 仍拒绝。`contract_sha256` 未变，Mac 只需重钉 source/binary。
