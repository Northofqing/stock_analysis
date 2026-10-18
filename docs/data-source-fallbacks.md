# 数据源失败与补偿方案登记

> 更新：2026-10-18。记录「原本想用什么 → 为什么不行（实测证据）→ 换成了什么 → 代价是什么」。
> 与 `data-sources-inventory.md` 的分工：那份讲准入边界（原则），这份讲当前实际状态（事实）。

## 总览

| 数据 | 原方案 | 实测结论 | 补偿方案 | 状态 |
|---|---|---|---|---|
| 板块涨幅榜 f3 | 对端 gRPC → 东财 clist | 对端是硬编码空壳；东财 clist 502 | 新浪概念板块直连 | ✅ 已落库 |
| 板块净流入榜 f62 | 同上 | 同上 | **无补偿**，显式报无源 | ❌ 留空 |
| 新闻（GlobalNews） | 对端 4 provider | 东财新闻接口全栏目 0 条 | 待定 | ❌ 断供 |
| TDX F10 公司资料 | 以为要 Windows 开发 | Mac 可直连，纯 Rust | 直接用，绕开 Windows | ✅ 已落库 |
| 产业链上下游 | TDX F10 客户/供应商 | F10 16 类里没有 | AI 从主营构成推断 + 人工确认 | ⚠️ 只到候选 |
| 上游商品价格 | — | 无任何源 | **无补偿**，不进优先级 | ❌ 缺 |

---

## 一、板块涨幅榜 f3

### 原方案

`BoardRankingGateway::fetch_top("f3", n)` → `bridge_for("BoardRanking")` → `127.0.0.1:18082` → 对端 `delegate.rs`。

### 为什么不行（两层，都实测过）

**第一层：对端是硬编码空壳，不是运行时故障。**

`stock-analysis-builds/20261013-.../source/src/grpc_server/delegate.rs`：

```rust
async fn fetch_board_ranking(fid: &str, top_n: usize) -> Result<Fetched, FetchFailure> {
    let _ = (fid, top_n);
    Err(FetchFailure {
        message: "板块排行 provider 未返回可保真的 batch evidence".to_string(),
        provider: None,                    // ← 字面量
        reason_code: "no_verified_batch",  // ← 字面量
        retryable: false,
    })
}
```

所以线上看到的 `provider=None retryable=false` **不是**探测结果，是代码里写死的。
这解释了为什么对端日志里查不到任何 `BoardRanking` / `ConceptHits` / `MarketRankings` 记录 ——
它连数据网关都没走到。

**第二层：即使直连东财也拿不到。**

```
push2.eastmoney.com/api/qt/clist/get       502 (nginx/1.26.2)
push2his.eastmoney.com/api/qt/clist        000
同一 host 的 stock/get                     200  ✅ {"f57":"600396","f58":"华电辽能",...}
```

同一域名下 `stock/get` 通、`clist/get` 不通。已排除代理因素 ——
Surge 规则是 `DOMAIN-SUFFIX,eastmoney.com,DIRECT`（直连），不是走节点。
也排除参数问题 —— `fs=m:90+t:3` 编成 `%2B`、加 `ut` token、加 `cb` 都试过，均 502。

### 补偿方案

新浪概念板块：`https://vip.stock.finance.sina.com.cn/q/view/newFLJK.php?param=class`

- GBK 编码的 JS 变量，每行 13 个逗号分隔字段
- 175 个板块，**98 个能对上 `stock_concepts`**（56%）
- 601811 实测命中：百度概念 +1.37%、参股金融 +1.12%、含H股 +0.74%、体育概念 -0.20%

`board_ranking_snapshot::fetch("f3")` 的两级顺序：
东财桥（全字段）→ 失败/空时回退新浪（只有涨跌幅和领涨股）。

### 为什么不用新浪行业板块

`newSinaHy.php`（飞机制造/摩托车/水泥行业）：行业名在 `stock_concepts` 里
**逐名验证均为 0 匹配**。拉进来只会挤占 top30 名额并污染榜单，所以只用概念板块。

### 代价

| 字段 | 东财 | 新浪 | 落库 |
|---|---|---|---|
| board_code / board_name | ✅ | ✅ | 有值 |
| change_pct | ✅ | ✅ | 有值 |
| leader_name | ✅ | ✅ | 有值 |
| main_inflow | ✅ | ❌ | **NULL** |
| vol_ratio / turnover | ✅ | ❌ | **NULL** |
| day1_ratio / day5_ratio | ✅ | ❌ | **NULL** |

不用 0 填充（BR-222）—— 0 是"没有资金流入"的有效观测值，会让下游把缺失误读成真实观测。

### 验证

```bash
STOCK_DB=<运行根>/data/stock_analysis.db ./target/release/board_ranking_build --date 2026-10-16
```

2026-10-18 实测：

```
[板块榜] f3=30 f62=0 persisted=30 errors=1
回读 top5：
  1. gn_dfj  大飞机   +3.30%
  2. gn_ssgn 三沙概念 +3.21%
  3. gn_wxdh 卫星导航 +2.62%
  4. gn_jght 军工航天 +2.49%
  5. gn_jmrh 军民融合 +2.17%
```

数据是 10-16（周五）收盘值，周日抓取时标注 `trade_date=2026-10-16`。
**注意**：非交易日抓取必须显式传 `--date`，否则会用错误的日期落库。

---

## 二、板块净流入榜 f62

### 为什么不做补偿

`f62` 的核心是**主力净流入金额**。新浪不提供这个字段，TDX 板块成分只给成员不给资金流。

可用替代只有"用成交量推算"或"用涨跌幅反推"，两者都是**编造观测值**：

- 成交量 ≠ 主力净流入（含散户、含对冲）
- 涨跌幅 ≠ 资金流向（缩量上涨和放量上涨含义完全不同）

所以 `fetch("f62")` 直接返回错误：

```
fid=f62: ...；该榜无备用源（新浪不提供主力净流入，强行推算等于编造）
```

`board_ranking_daily` 里 `rank_fid='f62'` 的行数为 0，下游读不到就是读不到。

### 恢复条件

找到提供真实主力净流入的源（东财 clist 恢复、或 Tushare/同花顺的板块资金流接口），
且能给出 batch evidence。在那之前不做。

---

## 三、新闻（GlobalNews）

### 现象

```
news_ai_assessment  MAX(created_at)=2026-10-14T06:50:34Z   ← 之后零新增
[NewsFlashGate][BR-137] intake input=67 buffered=0 duplicate=61 stale=6
[NewsAI][BR-172] completed pushed=0 ... failed=5            ← 每轮 5 条全失败
[NewsAI][BR-172] candidate failed ... model unavailable
```

### 真实根因：不是源枯竭，是 LLM 连不上

**这一条最初判断错了。** 最初看到东财新闻接口 `totle_hits=0` 就归因为"上游内容枯竭"，
但审计表显示东财一直在成功返回：

```
Eastmoney|eastmoney-web|available|accepted  90689 次   ← 一直成功
Cailianpress|...|available|accepted         103133 次  ← 也一直成功
```

真正的断点在 NewsAI 评估环节：

```
[16:00:37 WARN] [NewsAI][BR-172] candidate failed
error=model unavailable: [LLM] HTTP 错误:
https://api.deepseek.com/v1/chat/completions:
tcp connect error: Connection refused
```

每 tick 处理 5 条（`MAX_ASSESSMENTS_PER_TICK = 5`），全部因为 LLM 连不上而失败。
所以 `news_ai_assessment` 停在 10-14 —— **不是没新闻，是新闻评估不出来**。

### 关键区分

| 层 | 状态 | 证据 |
|---|---|---|
| 新闻数据源（对端 4 provider） | ✅ 正常 | 东财 90689 次 accepted、CLS 103133 次 accepted |
| NewsFlashGate 去重/时效 | ✅ 正常工作 | `duplicate=61 stale=6` 是设计内行为 |
| **NewsAI 的 LLM 调用** | ❌ **Connection refused** | 每轮 `failed=5` |
| `news_ai_assessment` 落库 | ⛔ 被上游阻断 | MAX=2026-10-14 |

### 已验证：API 本身没问题

```
DEEPSEEK_BASE_URL = https://api.deepseek.com
带 key 实测:  http=200  time=0.85s        ← 网络和凭据都正常
DNS:          api.deepseek.com → 115.231.230.218（真实 IP，非 fake-ip）
```

所以是 **monitor 进程内部的坏状态**（DNS 缓存或连接池），不是网络、不是凭据、不是 API 故障。

### 修复

重启 monitor 进程（`launchctl kickstart -k gui/501/com.stockanalysis.monitor`）。
进程重建后 DNS 缓存和连接池都清空，下一次 tick 就能重新解析。

**但这只是止血**：如果进程内 DNS 缓存再次被污染，还会复发。
根因（为什么进程会缓存到一个 refused 的地址）尚未确定。

### 修好 LLM 之后暴露的第二个断点

重启后 `failed=5 → failed=0`、`Connection refused` 归零，但两表仍然停在 10-14：

```
news_ai_assessment      1467 行  MAX=2026-10-14T06:50:34Z
news_ai_delivery_event  4253 行  MAX=2026-10-14T06:44:13Z
```

同时出现新的 ERROR（每次 5 条，与 `MAX_ASSESSMENTS_PER_TICK` 一致）：

```
[16:09:39 ERROR] [NewsAI][BR-172] manual-review notice unconfirmed; retry retained:
Ok(Err("delivery audit envelope: unsupported delivery outcome: manual_review
```

链条：

1. 评估结论是 `NewsAiPendingRecovery::ManualReview`（需要人工复核）
2. 写投递审计时，`src/event/envelope.rs:93` 的枚举**不认识 `manual_review`**
3. 于是审计写不进去 → 结果留在恢复队列里重试
4. `news_ai_assessment` 因此没有新增行

**所以新闻催化维度仍然读不到数据**，不是 LLM 的问题了，是
「评估结论落在 ManualReview，而投递审计不支持这个 outcome」。

要修这条需要：
- 在投递审计枚举里支持 `manual_review`，或
- 把 ManualReview 的结果单独落一张可查询的表（而不是当成投递失败重试）

两者都要改 `src/event/envelope.rs` 和 `src/database/news_ai.rs`，属于代码改动，尚未进行。

### 第三个断点：候选全是旧内容

`[NewsFlashGate] input=67 duplicate=61` —— 每轮 67 条里 61 条是已见过的。
东财返回的 20 条长期是同一批。CLS 虽然能返回今天的新闻（probe 实测 5 条 07:46-07:59），
但没有进入候选集（feed 选择或 limit 分配问题）。

即使 ManualReview 那条修好，只要候选全是旧内容，评估也不会有新结论。

### 数据源层仍然要关注

以下是独立于 LLM 故障的事实，将来接新源时需要：

| 源 | 接口 | 实测 |
|---|---|---|
| 东财快讯 | `newsapi.eastmoney.com/kuaixun/v1/getlist_102_ajaxResult_20_1_.html` | 200，今天的内容 |
| 新浪滚动 | `feed.mix.sina.com.cn/api/roll/get?pageid=153&lid=2516` | 200，`total=100029` |
| 同花顺快讯 | `news.10jqka.com.cn/tapp/news/push/stock/` | 200，有内容 |

以及：`np-listapi.eastmoney.com/comm/web/getNewsByColumns` 所有 column（1..8）都返回
`{"code":"1","data":{"list":[],"totle_hits":0}}` —— 这个**特定接口**确实空了，
但系统走的不是它（系统走对端 `GlobalNews` 的 4 个 provider）。

### 探测工具的正确用法

`global_news_probe` **不要**设 `GRPC_MARKET_CLIENT_BUNDLE`（那会走 ExternalV1，
未做 readiness 会得到 `external_bundle_invalid`）。走本地桥：

```bash
GRPC_MARKET_ADDR=http://127.0.0.1:18082 STOCK_DB=<运行根>/data/stock_analysis.db \
  ./target/release/global_news_probe
```

2026-10-18 实测结果：Cailianpress 返回 5 条今天 07:46-07:59 的新闻；
Eastmoney / Jin10 / ThePaper 各返回 `invalid_evidence`（服务端内部错误）。

---

## 四、TDX F10 公司资料

### 原判断（错的）

「Windows 只能拿到这些数据，产业链上下游拿不到」→ 推断 F10 也只能在 Windows 侧取。

### 实测

`magic-tdx-rs` 是**纯 Rust**，依赖 `tokio / encoding_rs / flate2 / crc32fast`，
**没有任何 `cfg(windows)`**。TDX 协议是纯 TCP 7709，Mac 直连即可。

`TdxF10Client` 已提供 `get_category / get_content / get_content_by_name / get_all_contents`，
分类常量（`经营分析`/`行业分析`/`股东研究`/`关联个股`…）也在 `profile/constants.rs` 里。
**不需要 Windows 开发任何东西。**

### 踩到的坑：分类名带填充字节

600396 的 16 个分类里，7 个名称是 68 字节（正常 12 字节），
后面跟着 `\0` 和 `\x01 \x02 \x04 \x0c` 之类的控制字节。

`trim_end('\0')` 去不掉（中间夹了非 NUL 字节），
`get_content_by_name("经营分析")` 精确匹配失败。

**补偿**：按**前缀**匹配出 `&F10Category`，再传给 `get_content` ——
它内部优先用 `category.filename_raw`（原始 GBK 字节），完全绕开名称匹配。
填充字节从头到尾没被"清洗"，只是不再参与匹配。

### 落库

表 `stock_f10_profile(code, category, content, byte_length, fetched_at)`，
主键 `(code, category)`，每次抓覆盖（F10 是"最新快照"）。

2026-10-18 实测 8 行：

```
600396  经营分析 23619 / 行业分析 18318 / 最新提示 20613 / 股本结构 5273
300750  经营分析 36890 / 行业分析 37448 / 最新提示 19027 / 股本结构 37864
```

### 数据归属

TDX F10 内容是港澳资讯灵通 V9.0 的编辑数据，**不是交易所披露原文**。
作为证据使用需单独评估来源归属与再分发。

---

## 五、产业链上下游

### 为什么 OEM 方案不行

TDX F10 的 16 类里**没有客户/供应商分类**。逐个查过：

- **关联个股** = 同大股东 / 同行业 / 股本相近 / 同地域。
  600519 的"同大股东"是香港中央结算有限公司（241 家，北向名义持有人）；
  600396 是 BARCLAYS BANK PLC（QFII）。**登记股东 ≠ 同一实控人**，对判上下游没有意义。
- **股东研究** = 控股股东 + 增减持。600396 只有"中国华电集团 20.92%"，与年报重复。
- **经营分析** = 主营业务 + 主营构成（收入占比、毛利率）。**这是唯一有用的**。

### 补偿方案：AI 从主营构成推断

`src/monitor/chain_ai.rs` + `src/bin/chain_ai_run.rs`：

- 输入：F10 经营分析里的【2.主营构成分析】段（含收入占比和毛利率）
- 输出：`chain_relation_candidate` 表
- **三张表严格分离**：

| 表 | 语义 | 谁能写 |
|---|---|---|
| `stock_f10_profile` | F10 原文 | 抓取器 |
| `chain_relation_candidate` | AI 推断（`source='ai'`） | 本模块 |
| `chain_relation` | 人工确认后的可信关系 | **本模块不写** |

### prompt 调优过程（重要）

第一版问的是"上下游 + 竞争者"，结果 7 条里 6 条是"竞争者"——
模型倾向于填最容易判断的那类（同行业=竞争者，几乎不用推理）。

第二版**只问上下游**，并要求：写具体大宗商品（不是"煤炭行业"而是"动力煤"）、
每条 evidence 必须给出"涨价→毛利升/降"的方向。

实测对比：

| | 第一版 | 第二版 |
|---|---|---|
| 600396 条数 | 7（6 竞争者） | 2（全上游） |
| 粒度 | "煤炭行业" | "动力煤" |
| 方向 | 无 | "涨价→毛利下降" |
| 引用具体数字 | 无 | "动力电池占收入69.38%、成本72.39%" |

300750 第二版输出：碳酸锂 0.90、新能源汽车(客户) 0.80、硫酸镍 0.75、
电解铜箔 0.70、硫酸钴 0.65、电解铝 0.60。

**关键证据**：碳酸锂那条的 evidence 引用了"动力电池系统占收入69.38%、成本72.39%"，
这两个数字是从 F10 主营构成表里读出来的，不是我喂给它的 —— 这是"真读数据"还是"套模板"的分界线。

### 代价与边界

- 粒度只到**行业/大宗商品级**，拿不到具体供应商公司名（F10 无此字段）
- 600396 的"天然气 0.55"是模型从行业常识推的（华电辽能是纯火电，主营只有电力/热力），
  人工确认时可直接否掉
- **全部标 `source='ai'`，展示必须带 `[AI推断·未核实]`**
- 不因"AI 说"就进 `buy_zone` / `ticket_priority`

### 未做

人工确认 CLI（`chain_review`）尚未编写。没有它，candidate 永远是推断，进不了决策。

---

## 六、上游商品价格（产业链信号）

### 目标

把"600396 盯动力煤、300750 盯碳酸锂"变成可用的打分信号（上游跌→毛利升→加分）。

### 为什么不做

**没有任何商品价格数据源**：

- TDX F10：只有公司资料
- 对端 gRPC：板块榜 f3/f62 都还不可用
- 主库：没有大宗商品价格表

没有价格，"上游涨价还是跌"无从判断。接进优先级等于挂一个空壳维度。

### 决定

产业链候选**只做展示，不进优先级**。等有商品价格源再谈打分。

理由：优先级决定先看哪只票。用未确认的推断去排序，
等于让 AI 的猜测决定注意力顺序。

---

## 新增的工具与依赖（2026-10-18）

### CLI

| 工具 | 用途 |
|---|---|
| `f10_probe <code>` | 验证 TDX F10 直连与分类名填充 |
| `chain_ai_run <code> [--skip-fetch]` | F10 取数 + AI 产业链分析 |
| `board_ranking_build [--date]` | 手工跑板块榜落库 |
| `global_news_probe` | 探测 4 个 GlobalNews provider |

### 模块

`src/monitor/f10_profile.rs`、`src/monitor/chain_ai.rs`、
`src/monitor/board_ranking_snapshot.rs`（改造）

### 依赖

```toml
magic-tdx-rs = { path = ".worktrees/tdx-local-provider-20260923/vendor/magic-tdx-rs" }
encoding_rs = "0.8"   # 新浪板块 GBK 解码
```

`magic-tdx-rs` 是 path 依赖，指向 worktree 里的 vendor。**上游 `magic-market-core`
走 git rev，本地 cargo 缓存已有**（`magic-market-data-rs-b56d463f5db752be`）。

### 表

| 表 | 主键 | 用途 |
|---|---|---|
| `stock_f10_profile` | `(code, category)` | F10 原文，每次覆盖 |
| `chain_relation_candidate` | `(subject_code, object_name, relation)` | AI 推断候选 |
| `board_ranking_daily` | `(trade_date, rank_fid, board_code)` | 板块榜快照 |

---

## 未解决项（按影响排序）

1. **新闻断供** —— 优先级权重最高的一维（30%）。东财接口确认无内容，
   需验证对端 Cailianpress / Jin10 是否还活着。
2. **板块 f62** —— 无真实主力净流入源。
3. **人工确认链缺失** —— `chain_review` 未写，candidate 进不了决策。
4. **观察池只有 1 只票** —— 概念共振恒 0，排序无法验证。依赖第 1 条。
5. **上游商品价格无源** —— 产业链信号只能展示，不能打分。
