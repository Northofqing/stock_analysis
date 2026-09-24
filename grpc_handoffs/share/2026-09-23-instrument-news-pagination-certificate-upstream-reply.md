> **更正（2026-09-23 21:10）：本文件第 5 节把 002131 的持续失败归因于 limit 上限，那个归因是错的。真正原因是来源的置顶行，见同目录 2026-09-23-instrument-news-pinned-placement-correction.md。第 4 节的供给数据仍然成立。**

# InstrumentNews 分页证书修复与供给上限 — 2026-09-23

本文回复你们 InstrumentNews 每 30 分钟一次的 `source_precondition_failed`。
结论：**根因是 b 侧两个缺陷叠加，已修复并上线**；同时给出今天实测的
provider 供给上限，以及我们仍需你们确认的一项信息。

---

## 1. 根因（实测，非推断）

两个缺陷叠加，任一单独存在都会让**任何**调用方 limit 失败：

1. `execute_instrument_news` 不论调用方要多少条，一律按 200 向 Sina 请求
   （`provider_limit = PositiveU32::new(200)`）。你们的 limit 从未到达 provider。
2. BR-025 的分页停止证书用「**本页最新**时间戳」判定能否停页，等于让一页证明
   自己相对它刚贡献的行是冗余的。每读一页只能证明一页的量，五页最多证明 160 条。

修复前实测（`target/runtime/probe/call-news.ps1`，同一份线上服务）：

```text
limit=1   Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
limit=2   Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
limit=3   Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
limit=5   Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
limit=10  Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
limit=20  Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
```

trailer（`magic-error-detail-bin`）解码为 `reason_code=source_precondition_failed`、
`operation=instrument_news`、`retryable=false`。注意 limit=1 也失败——这证明失败
与你们的 limit 无关，正是缺陷 1 的指纹。

## 2. 改动（commit `483cff6`，已推送 main）

1. 按调用方 limit 向 provider 请求（与 global-news 处理一致）；返回的完整批次仍然
   先全量校验，再套 `captured_through` cutoff。
2. 证书改为「**本页最旧**时间戳」：后续页严格更旧，这才是证明边界。
   BR-025 原文那句 `newest` 同步改为 `oldest`，并写明理由。
3. 两条回归测试：五页 × 40 行、limit=200 必须证明通过（在旧证书下用 production
   同一条报错失败，已复现确认）；五页 × 39 行、limit=200 必须仍显式失败。

## 3. 上线与实测

部署路径：build → `stop.ps1` → copy（源/目标 SHA256 一致）→ `start.ps1`；
重启时日志归档步骤再次生效（`logs/archive/` 现有 4 个归档文件）。

| 请求 | 结果 |
| --- | --- |
| 600519 @ limit=20 | `ADMISSION_STATE_ADMITTED`，20 条，`pages-1`，complete |
| 600396 @ limit=199 | `ADMISSION_STATE_ADMITTED`，199 条，`pages-5`，complete |
| 600519 @ limit=196 | `ADMISSION_STATE_ADMITTED`，196 条，`pages-5`，complete |
| HistoricalBars | `ADMISSION_STATE_ADMITTED`，Tdx，complete，`tdx-smart:1790156688:333` |

小 limit 现在只读一页（`pages-1`），不再为 20 条去翻五页。

## 4. 供给上限：你们现在能用多大 limit

Sina 今天每页的实际供给（直接抓页计数，非推断）：

```text
sh600396  page1..5 = 40 40 40 40 40   锚点 200，distinct URL 200，成记录 199
sh600519  page1..5 = 39 40 40 40 40   锚点 199，distinct URL 197
```

直接客户端探针（`MAGIC_SINA_LIMIT` 旋钮）实测边界：

```text
limit=196  600396.SH 通过 records=196 pages-5  000001.SZ 通过 records=196 pages-5
limit=199  600396.SH 通过 records=199 pages-5  000001.SZ 失败（五页供给不足）
limit=200  失败：news pagination exceeds the 5-page bound
```

**建议把 InstrumentNews 的 limit 压到 ≤195。** 超过五页供给的部分，BR-025 不允许
把 199 条当 200 条「完整」交付，只能显式失败——这是规则要求，不是缺陷。

## 5. 仍失败的那次调用（2026-09-23T09:32:15.467Z，本地 17:32:15）

```text
ts=2026-09-23T09:32:15.4677336Z level=ERROR target=grpc_server event=service_failure
   stage=source_precondition_failed request_id="1790155935161-74628-41908" operation=instrument_news
```

这次发生在修复部署（本地 17:22）之后。我们的失败记录只有 `stage` / `request_id` /
`operation`，**不含 instrument、limit、start/end**，所以我无法判定它是：

- (a) limit 超过五页供给（第 4 节），还是
- (b) 带 range 时，range 内可证明条数 < limit，而五页内又走不到 range 起点之前。

请提供这次调用的 instrument、limit、是否带 start/end，我们即可定位。若你们同意，
我们也可以给失败记录增加 instrument/limit 字段——但那会改变日志记录形状，需要你们
确认不会影响你们的解析。

## 6. 另一个选项（我们未做，属于业务规则变更）

放宽 BR-025 的「最多五页」上限（Sina 第 6 页仍有内容）。这是业务规则变更，按仓内
工程规则需要 Gate A 设计加 provider admission 证据，我们没有擅自改。如果 ≤195 对
你们不够用，我们再谈这一条。

## 7. 不变的部分

HistoricalBars、限价池与其余 provider 路径本次未改动，修复后实测仍为 complete；
上述改动只触及 Sina 个股新闻的取数上限与分页停止条件。
