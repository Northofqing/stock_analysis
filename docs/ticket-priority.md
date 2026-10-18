# 票优先级（ticket_priority）

> 状态：实现完成，数据源 2/5 → 3/5（2026-10-18 板块打通）
> 代码：`src/monitor/ticket_priority.rs`（343 行）、`src/bin/ticket_priority_show.rs`（65 行）
> 调度：每日 15:44（晚于 15:30 板块榜、15:40 观察池生成）

## 一、这个功能回答什么问题

**不是"该不该买"，是"先看哪只票"。**

观察池里可能同时有十只票，人的注意力有限。优先级给一个排序，
让你先看最有把握的那几只，而不是从头翻到尾。

所以它只排序，**不产出买卖结论**。买不买、买多少，由可买价、仓位建议和
你自己的判断决定，优先级不替你做这个决定。

## 二、五维与权重

| 维度 | 权重 | 数据源 | 归一化 | 状态 |
|---|---:|---|---|---|
| 新闻催化 | 30% | `news_ai_assessment` 近 5 日命中数 | 5 次命中 = 100 | ❌ 断供 |
| 板块强度 | 20% | `board_ranking_daily` 当日 f3 匹配 | 8% 涨幅 = 100 | ✅ 2026-10-18 打通 |
| 可买价 | 25% | `buy_zone` | 三道超买门 + 位置 | ✅ 有数据 |
| 仓位建议 | 15% | `position_sizing` | 建议仓位百分比 | ✅ 有数据 |
| 概念共振 | 10% | `concept_peers` 与池内其他票共享概念数 | 5 个共享 = 100 | ⚠️ 池内仅 1 只，恒 0 |

权重取舍的理由：

- **新闻 30% 最高**：新闻是选票的入口（"按板块轮动、按新闻催化"），
  一只被三条新闻同时提到的票，比没有催化的票更值得先看。
- **可买价 25% 第二**：有催化但位置不对（超买/贴支撑）的票不能买，
  所以它要能压过单纯的新闻热度。
- **板块 20%**：个股强不如板块强。板块起来了，票才有持续性。
- **仓位 15%**：仓位建议本质是止损距离的函数，反映"这只票好做不好做"。
- **共振 10% 最低**：共振是加分项，不是必要条件，权重不能高到把
  一只独立强势的票压下去。

## 三、缺失维度从分母剔除，不补 0

这是这个模块最重要的一条设计。

如果某维没数据就记 0 分，那"没数据"和"数据很差"就分不出来了：
一只新闻源正常但当天没被提及的票，会被判得比"新闻源挂了"还低。
这是错的。

所以实现上：**该维不参与计算，权重从分母里去掉**，
输出里也不显示这一维（`[催化 0 / 可买 0 / 仓位 0 / 共振 0]` 里没有"板块"，
就说明板块那维当时没数据）。

代价：不同票可能在不同维度上可比性下降。当 5 维只剩 2 维时，
"总分"只反映那 2 维，不能横跨时间比较。

## 四、每维的数据口径

### 新闻催化

```sql
SELECT COUNT(*) FROM news_ai_assessment
WHERE created_at >= <as_of - 5 天> AND <命中该代码或该票名称>
```

`NEWS_HITS_FULL = 5.0` —— 5 次命中记满分。

### 板块强度

```sql
SELECT MAX(b.change_pct)
FROM board_ranking_daily b
JOIN stock_concepts s ON 1=1
WHERE b.trade_date = ? AND b.rank_fid = 'f3'
  AND s.code = ?
  AND instr(s.concepts, b.board_name) > 0
```

`BOARD_PCT_FULL = 8.0` —— 板块涨 8% 记满分。

**取 MAX 不取平均**：一只票同时属于 5 个概念，只要最热的那个在涨就够了，
平均会稀释掉真正的强势。

**匹配靠文本包含**：`stock_concepts.concepts` 是 JSON 数组文本，
直接 `instr` 判断板块名是否出现在里面。简单但要求两边命名一致 ——
这也是为什么行业板块用不了（见下）。

### 可买价

来自 `buy_zone`，已含三道超买门（连板/偏离均线/量比）。
超买直接判 0 并标注"不参与"。

### 仓位建议

来自 `position_sizing`（2% 风险预算 / 止损距离，下限 3%、上限 10%、单票上限 30%）。

### 概念共振

来自 `concept_peers`：该票与池内**其他**票共享的概念数量（≥2 才算）。
`RESONANCE_FULL = 5.0`。

池里只有 1 只票时恒为 0 —— 不是算错了，是没有可比对象。

## 五、2026-10-18 的关键修复：板块从"无源"到"有源"

### 原来的问题

1. **对端 `delegate.rs::fetch_board_ranking` 是硬编码空壳**：

```rust
async fn fetch_board_ranking(fid: &str, top_n: usize) -> Result<Fetched, FetchFailure> {
    let _ = (fid, top_n);
    Err(FetchFailure {
        provider: None,                    // 字面量，不是运行时探测
        reason_code: "no_verified_batch",  // 字面量
    })
}
```

`provider=None` 不是"provider 挂了"，是代码里写死的。这个 op 从实现起就没通过。

2. **东财 clist 端点从本机不可用**：

```
push2.eastmoney.com/api/qt/clist/get   502 (nginx/1.26.2)
push2his.eastmoney.com/api/qt/clist    000
同一 host 的 stock/get                 200 ✅  (真实数据：600396 华电辽能)
```

同一个域名下 `stock/get` 通、`clist/get` 不通，说明是该路径后端不可用，
不是网络问题（Surge 规则是 `DOMAIN-SUFFIX,eastmoney.com,DIRECT` 直连）。

### 现在的方案

`fetch("f3")` 走两级：

1. 东财 gRPC 桥（全字段）
2. 失败时回退**新浪概念板块** `newFLJK.php?param=class`（GBK，13 字段逗号分隔）

实测（2026-10-18）：

```
175 个概念板块，98 个能对上 stock_concepts（56%）
601811 命中：百度概念 +1.37%、参股金融 +1.12%、含H股 +0.74%、体育概念 -0.20%

board_ranking_build --date 2026-10-16
[板块榜] f3=30 f62=0 persisted=30 errors=1
回读 top5：
  gn_dfj  大飞机   +3.30%
  gn_ssgn 三沙概念 +3.21%
  gn_wxdh 卫星导航 +2.62%
  gn_jght 军工航天 +2.49%
  gn_jmrh 军民融合 +2.17%
```

**为什么不用新浪行业板块**（`newSinaHy.php`，飞机制造/摩托车/水泥行业）：
行业名在 `stock_concepts` 里**逐名验证均为 0 匹配**，拉进来只会挤占
top30 名额，还会污染榜单。

**为什么 f62 留空**：新浪不提供主力净流入，TDX 板块成分也没有。
用推算值填等于编造（BR-222），所以明确报"该榜无备用源"。

> 完整的失败证据链与所有补偿方案见 [`data-source-fallbacks.md`](data-source-fallbacks.md)。

## 六、当前状态与缺口

| 项 | 状态 | 阻塞点 |
|---|---|---|
| 代码 / 调度 / 单测 | ✅ 完成（3 passed，15:44） | — |
| 板块 f3 | ✅ 2026-10-18 打通（新浪概念板块） | — |
| 板块 f62 | ❌ 无源 | 东财 clist 502；新浪不提供净流入 |
| 新闻催化 | ❌ 断供 | 东财 `np-listapi` 全栏目 `totle_hits=0` |
| 概念共振 | ⚠️ 恒 0 | 观察池只有 1 只票 |
| 排序效果验证 | ❌ 未验证 | 池里 1 只票，排不出先后 |

### 新闻这条的证据链

```
[NewsFlashGate][BR-137] intake input=67 buffered=0 duplicate=61 stale=6
[NewsAI][BR-172] assessment retained ... existing=true   ← 评估的都是老新闻
news_ai_assessment  MAX(created_at)=2026-10-14T06:50:34Z  ← 之后零新增
```

不是抓取失败、不是 AI 评估失败（`failed=0`）、不是落库失败 ——
是源端内容重复轮转。东财新闻接口实测：

```
column=1..8 全部 {"code":"1","message":"success","data":{"list":[],"totle_hits":0}}
```

**待验证**：对端支持的 4 个 provider（Eastmoney / Cailianpress / Jin10 / ThePaper）
各自还有没有新鲜内容。如果 Cailianpress 活着，切 provider 即可，不用写新抓取器。

## 七、怎么验证

```bash
STOCK_DB=<运行根>/data/stock_analysis.db ./target/release/ticket_priority_show --date 2026-10-16
```

看三件事：

1. `标的=N` —— 池里有几只票。N=1 时排序无意义。
2. 括号里的维度名 —— 缺哪维就不显示哪维（证明剔除生效）。
3. 每只票的分数构成 —— 高分是不是来自真实优势，而不是别处缺数据。

**有效验证需要**：池里 ≥3 只票，其中有的能买有的不能买，板块榜有当日数据。
只有 1 只票时，看到的只是"单票得分"，不是"排序"。

## 八、边界

- **只排序，不给买卖结论**。不写 `ticket_lifecycle.stage`，不拦交易，不设熔断。
- **产业链候选（`chain_relation_candidate`）不进优先级**。那是 AI 推断，
  未经人工确认不得影响注意力排序。
- **不跨日比较**。缺失维度剔除意味着不同日期覆盖的维度可能不同，
  总分不可比。
- **不补 0、不推算**。某维没数据就不参与，不用行业常识或历史值填补。

## 九、提升路线（按收益排）

1. **新闻换源** —— 权重最高（30%），一通就能让排序有意义。
   优先验证对端 Cailianpress / Jin10 是否还活着。
2. **观察池扩容** —— 共振和排序都需要 ≥3 只票。依赖第 1 条。
3. **板块 f62** —— 只有在找到真实主力净流入源之后才做。
   在那之前留空，不要用成交量推算。
4. **权重重估** —— 等 5 维都有数据、跑够 2 周后再看权重是否合理。
   现在调权重等于调一个只有 2 维的公式，没有意义。
