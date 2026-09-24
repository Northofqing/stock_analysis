# InstrumentNews 失败定位与修复（含来源置顶行）— 2026-09-23

本文回复你们 InstrumentNews 每 30 分钟一次的失败。18:19 的第一版把你们 002131 的
**持续**失败归因于 limit 上限并建议压小 limit——**那个归因是错的**：你们的 limit=100
远低于我们当时给的上限。真正的原因是来源的置顶行，见第 5 节。第 4 节的供给数据本身
成立，但与你们的失败无关。

---

## 1. 缺陷一：任何调用方 limit 都失败（commit `483cff6`，已推送）

两个缺陷叠加，任一单独存在都会让**任何** limit 失败：

1. `execute_instrument_news` 不论调用方要多少条，一律按 200 向 Sina 请求
   （`provider_limit = PositiveU32::new(200)`）。你们的 limit 从未到达 provider。
2. BR-025 的分页停止证书用「**本页最新**时间戳」判定能否停页，等于让一页证明
   自己相对它刚贡献的行是冗余的。每读一页只能证明一页的量，五页最多证明 160 条。

修复前实测（`call-news.ps1`，同一份线上服务）：

```text
limit=1   Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
limit=2   Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
limit=3   Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
limit=5   Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
limit=10  Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
limit=20  Code: FailedPrecondition  Message: news pagination exceeds the 5-page bound
```

limit=1 也失败——这证明失败与你们的 limit 无关，正是缺陷 1 的指纹。

改动：按调用方 limit 向 provider 请求（与 global-news 一致）；证书改为「**本页最旧**
时间戳」；BR-025 原文那句 `newest` 同步改为 `oldest` 并写明理由；两条回归测试
（五页 × 40 行 / limit=200 必须证明通过；五页 × 39 行 / limit=200 仍必须显式失败），
在旧证书下用 production 同一条报错复现确认。

## 2. 缺陷二：来源置顶行让 002131 单只持续失败（commit `2cd8665`，已推送）

现象：你们的 `[盘后]` 每 30 分钟一次，5 只持仓里**只有 002131** 失败，其余 4 只
每次都正常：

```text
[20:32:15 INFO] [盘后][BR-163] 600667 ... 拉 41 条, DB 写 41 条, 失败 0 条
[20:32:15 INFO] [盘后][BR-163] 600703 ... 拉 100 条, DB 写 100 条, 失败 0 条
[20:32:15 INFO] [盘后][BR-163] 600396 ... 拉 77 条, DB 写 77 条, 失败 0 条
[20:32:15 INFO] [盘后][BR-163] 002421 ... 拉 26 条, DB 写 26 条, 失败 0 条
[20:32:15 WARN] [盘后][BR-163] 002131 Sina 个股新闻不可用: outcome=partial
    reason_code=internal retryable=false: GrpcExternalV1 data gateway failed
    reason_code=internal provider=None retryable=false:
    ExternalV1 InstrumentNews 查询失败: instrument-news page is not newest-first
```

## 3. 实测根因：来源自己置顶的那一行

直接抓 `sz002131` 页面（`page-order.ps1`、`page-head.ps1`，2026-09-23）：

```text
page=1 stamps=40 inversions=1
  INVERSION page=1 previous=2026-09-23 00:52 current=2026-09-23 16:39
page=2 stamps=40 inversions=0
page=3 stamps=40 inversions=0
page=4 stamps=40 inversions=0
page=5 stamps=40 inversions=0
```

第 1 页唯一那处逆序来自来源自己的置顶行：

```text
1. 2026-09-23 00:52 | https://wq.finance.sina.com.cn/company/detail/1017/1
   | [置顶] 利欧股份维权征集，专业律师团队待命，受损股民抱团取暖，速登记
2. 2026-09-23 16:39 | https://cj.sina.cn/articles/view/1850649324/6e4eaaec02002k03e
```

同一时刻抽查 7 只，只有 `sz002131` 带 `[置顶]` 行（`sh600519`、`sh600396`、
`sz000001`、`sh600667`、`sh600703`、`sz002421` 均无）——这正解释了为什么只有它
一只持续失败。

这一行不只制造假逆序：它的时间戳（00:52）比本页所有新闻行都旧，若参与本页时间极值，
会让本页「最新时间戳」比真实值更旧，进而影响分页判定。所以修复不是「放宽校验」，而是
把置顶行整行排除：

- 置顶行**仍按 BR-025 全量校验**（身份、MIME、发布时间、未来时间），校验后才排除；
- 排除范围：记录集、本页逆序校验、本页时间极值；
- 未带 `[置顶]` 前缀的真实逆序仍然显式失败（原来的失败路径没有被削弱）；
- BR-025 同步写明这一条与实测证据。

## 4. 上线与实测

部署：build → `stop.ps1` → copy（源/目标 SHA256 一致：
`13E5C9F986787A7BB3A872FDAC6ADBB34A8573D73AE1D4AAACAF4E746140E2A3`）→ `start.ps1`。

你们那次失败的同形状调用（`limit=100`、`from=2026-08-24`、`to=2026-09-23`，
即 `[盘后]` 的参数），修复后：

| 请求 | 结果 |
| --- | --- |
| 002131.SZ @ limit=100, 2026-08-24..2026-09-23 | `complete: true`，`pages-3`，55 条，逐条严格倒序，含 `wq.finance.sina.com.cn` 的行 **0** 条 |
| 002131.SZ @ limit=100, 当日 | `complete: true`，`pages-2`，6 条 |
| 600396 @ 199 / 600519 @ 196（缺陷一回归） | `complete: true`，`pages-5` |

你们自己的 `[盘后]` 周期也验证了：部署前 20:32:15 仍然失败，部署后第一个整点周期
21:02:15 **五只持仓全部 available**，002131 首次进入：

```text
[20:32:15 WARN] [盘后][BR-163] 002131 ... instrument-news page is not newest-first
[21:02:15 INFO] [DataGateway][SinaInstrumentNews][BR-159] outcome=available
    batch_id=sina-company-news:sz002131:1790168535.000000000:pages-3 requested=1
    accepted=55 rejected=0 reason_code=accepted
[21:02:15 INFO] [盘后][BR-163] 002131 Sina 个股新闻: status=available provider=Sina
    batch_id=sina-company-news:sz002131:1790168535.000000000:pages-3 拉 55 条,
    DB 写 55 条, 失败 0 条
```

这 55 条与我们离线同形状探针的条数一致。

## 5. 供给上限（有效，但与你们的失败无关）

Sina 2026-09-23 每页实际供给（直接抓页计数，非推断）：

```text
sh600396  page1..5 = 40 40 40 40 40   锚点 200，distinct URL 200，成记录 199
sh600519  page1..5 = 39 40 40 40 40   锚点 199，distinct URL 197
```

直接客户端探针（`MAGIC_SINA_LIMIT`）实测边界：

```text
limit=196  600396.SH 通过 records=196 pages-5  000001.SZ 通过 records=196 pages-5
limit=199  600396.SH 通过 records=199 pages-5  000001.SZ 失败（五页供给不足）
limit=200  失败：news pagination exceeds the 5-page bound
```

五页供给不足时，BR-025 不允许把不足的条数当「完整」交付，只能显式失败——这是规则
要求。你们的 limit=100 不受这一条影响。

## 6. 未做的选项

放宽 BR-025 的「最多五页」上限（第 6 页仍有内容）。这是业务规则变更，按仓内工程规则
需要 Gate A 设计加 provider admission 证据，我们没有擅自改。如果 limit=100 不够用，
我们再谈这一条。

## 7. 我们仍希望你们补的信息（非阻塞）

你们的失败日志只有 `stage` / `request_id` / `operation`，不含 instrument、limit、
start/end。这次我们是从你们 `[盘后]` 的 INFO 行反推出 5 只持仓与参数形状才定位到的。
若你们愿意在失败记录里补上 instrument 与 limit，下一次这类问题我们不必再反推——
这只需要你们改日志形状，我们这边不需要配合。

## 8. 不变的部分

HistoricalBars、限价池与其余 provider 路径本次未改动，修复后实测仍为 complete；
上述改动只触及 Sina 个股新闻的解析、取数上限与分页停止条件。
