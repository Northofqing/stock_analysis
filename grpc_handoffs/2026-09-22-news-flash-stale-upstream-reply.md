# 新闻快讯全量 stale 拒绝: 上游侧核实结论 (2026-09-22)

对 `2026-09-22-news-flash-stale.md` 的核实回复。结论先行:

1. **上游无法产生该文档描述的批次。** "批次 `source_at` = 2026-09-22 11:18:18(今天,
   新鲜)" 与 "749/749 条目 `occurred_at` 全部非 9/22" **不能同时成立** —— 前者在
   gRPC 边界被原子地强制为该批次最新一条记录自己的源时间, 见下节。这两条里至少有一条
   不是 Jin10 批次的读数。
2. **时钟家族可以定量排除。** 两侧主机实测偏差都是**亚秒级**(上游 VM −0.10 s;Mac
   ≈ −0.24 ~ −0.29 s), 而亚秒偏差只能在午夜前后 1 秒内翻转日期, 不可能在 09:30-11:55
   造成 749 条全量日期不一致。
3. **上游今天的内容是新鲜的、可复现的。** 2026-09-22 13:41 直连探针返回的条目
   `published_at=2026-09-22T13:41:05+08:00`; 故障窗口内的上游日志每一采样点的日期都是
   `2026-09-22`。
4. **本次不产生上游代码改动**, 理由见末节。需要的判别数据在下游侧, 而且是 9/20 就已经
   提出、至今未落地的"把值打出来"。

## 上游实测证据

### 内容新鲜(直连探针, 2026-09-22 13:41 CST)

仓库自带探针 `cargo run -p magic-jin10-rs --example live_probe`:

```
source=jin10-flash-v1 source_at=Some("2026-09-22 13:41:05") complete=true records=5
item_id=20260922134105184800 published_at=2026-09-22T13:41:05+08:00
  evidence_source_at=Some("2026-09-22 13:41:05") observed_at=1790056115.738261700
item_id=20260922134000001800 published_at=2026-09-22T13:40:00+08:00
...
live_probe_status=passed
```

即: 条目的日期直接来自上游 feed 自己的 `time` 字段, 逐字保留
(`crates/magic-jin10-rs/src/lib.rs:756-757` 与 `jin10_time`, `:867-904`), 上游**不**
用本机日期补造、**不**改写源时间。条目 id 前缀 `20260922...` 与 `published_at` 同日,
互为佐证。

### 故障窗口内上游时钟的日期是对的

上游 runtime 日志 `target/runtime/logs/grpc-server.stderr.log` 覆盖了本次窗口, 每一条
`ts=` 的日期都是 `2026-09-22`:

| 上游日志(UTC) | = 本地(CST) | 事件 |
| --- | --- | --- |
| 2026-09-22T01:13:03Z ~ 01:26:28Z | 09:13 ~ 09:26 | limit_pools 路由失败 35 次 |
| 2026-09-22T02:41:00Z ~ 03:17:09Z | 10:41 ~ 11:17 | current_auction_observations 失败 8 次 |

也就是说, 在 09:13-11:17 的每个采样点上, 盖出 `observed_at` 的那只时钟的日期就是
2026-09-22。这与 9/2 那次"上游时钟日期卡在 9/1"的形态**不同**。

### 两侧时钟(实测, 不是推断)

上游 VM(`10.211.55.3`, 即运行 gRPC server 的这台):

```
w32tm /query /status      -> Leap 指示符 0(已同步), 源 ntp.tencent.com,0x8
                             上次成功同步: 2026/9/22 9:43:39
w32tm /stripchart /computer:ntp.aliyun.com /samples:3 /dataonly
  13:52:30, -00.1002396s   13:52:32, -00.1019844s   13:52:34, -00.0882527s
```

Mac(经 SMB 由 Mac 侧盖 mtime 测得 —— `net time \\Mac` 仍是 System error 1707, 沿用
9/20 修订版 3 的结论, 不要再走那条路):

```
windows_before_utc = 2026-09-22T06:05:28.1475615Z
mac_mtime_utc      = 2026-09-22T06:05:27.9076521Z   <- Mac 盖的
windows_after_utc  = 2026-09-22T06:05:28.1936596Z
bracket_width_ms = 46.1      delta_vs_before_ms = -239.9
```

Mac 落在 46 ms 宽的括号**之外**、早 240 ms ⇒ Mac 比上游 VM 慢约 0.24-0.29 s, 比真实时间
慢约 0.34-0.39 s。**量级是亚秒, 日期是对的。**

## 关键更正: 批次 `source_at` 不是抓取时刻

`2026-09-22-news-flash-stale.md` 的两条证据都建立在一个字段语义上, 而这个语义与已发布
合同相反:

> "- Jin10 flash 批次: ... `source_at=2026-09-22 11:18:18` (批次抓取时刻 = 今天, 新鲜 ✓)"

`QueryResponse.source_at` **不是**批次抓取时刻, 而是**批次中最新的那一条记录自己的源时间**:

- 合同: `docs/integrations/grpc-external-api.md:125` ——
  "`QueryResponse.source_at` 只表示批次中最新记录…";
- 上游构造: `magic-jin10-rs/src/lib.rs:388-397` —— 按 `published_at` 降序排序后,
  批次 `source_at` 取**第一条记录**的 `evidence.source_at`(即最大值);
- gRPC 边界**原子强制**, `magic-market-composition/src/grpc_production.rs:4790-4797`:

  ```rust
  if index == 0 && source_at != batch_source_at {
      return Err(invalid_news_evidence(provider, "batch_source_at_mismatch", "source_at", ...));
  }
  ```

  并且 `:4748` 强制 `source_instant == published_instant`, `:4773` 强制
  `source_instant <= observed_instant`, `:4781` 强制降序。

**这条原子检查的直接后果**: 任何一批成功返回的新闻, 其 `source_at` **就等于该批最新
记录自己的 `published_at`**。所以:

- 若某批的 `source_at = 2026-09-22 11:18:18`, 则该批**最新一条**记录的
  `published_at` 就是 `2026-09-22T11:18:18+08:00` —— 内容日期是 9/22;
- 且同批其余记录按降序**不晚于**它, 日期不一致只可能是**更早**的日子。

⇒ "批次时间戳是今天" 与 "749/749 条目日期非今天" 对**同一批**是互斥的。要么那三个
`source_at` 读数来自**其它源**(CLS/ThePaper/Eastmoney)的批次, 要么 `stale` 判定里的
`fetched_at` 根本不是批次 `source_at`。这正是需要判别数据的地方。

顺带一个正面事实: 下游能收到这些批次, 说明边界原子检查全部通过 —— 即每条记录的
`evidence.source_at`、`published_at`、`observed_at` 在上游侧是自洽的, **判别所需的三个值
本来就在每一个响应里**, 缺的只是下游日志把它们打出来。

## 需要下游提供的判别数据

请确认 `src/news/aggregator/feed.rs:97` 的 `fetched_at` 取值口径 —— 现有交接把两种口径
混用了(9/20 回复修订版 2 记的是 `fetched_at` = 上游客证的 `observed_at`; 而本文档写的是
"抓取日", 读起来像下游本机 `now`)。这两种口径指向完全不同的结论, 必须先定死是哪一种。

然后对**同一条**被拒记录打印三个值:

```
item_id / occurred_at(原始字符串) / fetched_at(原始字符串) / batch source_at
```

9/20 的 `2026-09-20-news-pipeline-invalid-observation-time.md` §排查指引 2 已经提出过
同一件事("临时在 …:510 处把值加进日志"), 至今未落地; 本次无法定因, 根因就在这里。

判读规则(上游可以确认的部分):

- 若 `batch source_at` 是 **9/21**, 而条目 `published_at` 是 9/22 ⇒ 上游供了旧批次,
  是上游要查的问题(但见末节: 上游边界没有"窗口过旧"规则, 需另开 Gate A);
- 若 `batch source_at` 与条目 `published_at` 都是 9/22, 只有 `fetched_at` 不是 9/22
  ⇒ 问题在那个 `fetched_at` 的来源;
- 若三者都是 9/22 却仍判 stale ⇒ 判定式本身有问题(例如把 `fetched_at` 当成了
  批次 `source_at`, 或把日期做了两次转换)。

另外, 若怀疑 Mac 时钟在窗口内跳过, macOS 侧可直接取:

```
sudo sntp -sS time.apple.com      # 若有偏移, 会打印并校正
log show --last 8h --predicate 'process == "timed"' | tail -50
```

并确认 Mac 在 09:30-11:55 期间是否经历过睡眠唤醒/时间校正。

## 为什么本次没有上游代码改动

已排除的候选:

- **"上游时钟日期卡住"** —— 定量排除: 两侧偏差亚秒级(上节), 且上游日志在窗口内每个
  采样点的日期都是 9/22。
- **"上游 feed 内容日期未翻日"** —— 今天不可复现: 13:41 探针返回的全部是 9/22 条目;
  且与本文档自己引用的批次 `source_at` 读数互斥(上节)。
- **上游该对"窗口整体过旧"硬拒(fail-closed)** —— 本次**没有**采纳, 不是拒绝该方向:
  它在本次故障中**不会触发**(上游内容当时按文档引用的读数就是 9/22), 而且会把合法的
  安静窗口(节假日、隔夜、盘前)从"完整空窗"变成硬失败; 阈值需要自己的证据与 Gate A。
  如果上面的判别数据最终证明上游**真的供了旧批次**, 上游会就"GlobalNews 窗口过旧"单独
  开 Gate A, 届时再谈阈值。

上游当前对 GlobalNews 的登记语义是: 返回真实源时间、逐条保留 evidence、由消费者决定
新鲜度; 上游只保证批次内部自洽与不伪造。这个语义没有在这次事件里被破坏。

## 上游侧的排除清单(避免重复劳动)

- 不要再跑 `net time \\Mac`(System error 1707); 要用 Mac 时钟就用上面的 SMB 括号法或
  `sntp`。
- 不要用日志时间戳反推时钟偏差(9/20 修订版 2 已列): 写入延迟抖动 0~1.5 s。
- 不要假设"批次 source_at = 抓取时刻"; 见上文合同与原子检查。