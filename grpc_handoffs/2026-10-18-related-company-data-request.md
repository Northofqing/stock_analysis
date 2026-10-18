# 关联公司数据需求（Mac 侧 → Windows 侧）

日期：2026-10-18
提出方：Mac 侧 monitor（stock_analysis）
状态：需求，未实现

---

## 一、为什么需要

Mac 侧要做「盘中跟踪」，目前能拿到的只有：

- 个股行情（RealtimeMarketQuotes）
- 个股新闻/公告（InstrumentNews / MarketAnnouncements）
- 同板块成分（BoardConstituents）

缺的是**票与票之间的传导关系**：当 A 出了利好/利空，B 会不会跟着动、为什么。

没有这层关系，"新闻催化"只能作用于被直接点名的那一只票，无法解释板块联动、无法提前埋伏上下游。

---

## 二、需要什么（三层，按优先级）

### 第 1 层：同板块成分（**已有，不需要做**）

`BoardConstituents` 已经能拿到"某板块有哪些票"。
Mac 侧已有 `stock_concepts`（1265 只票的概念映射）。

**这一层不用 Windows 做。**

### 第 2 层：产业链上下游（**需要，优先级最高**）

定义：A 与 B 存在**供需传导**关系，且方向可辨。

    A --供应商--> B      （A 的产品是 B 的原料/部件）
    A --客户----> B      （A 是 B 的主要客户）

字段需求：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| subject_code | string | 是 | 主体票代码，如 "600703" |
| object_code | string | 是 | 关联票代码 |
| relation | enum | 是 | `supplier` / `customer` / `competitor` |
| strength | enum | 否 | `core`（主营占比高）/ `minor`（边缘业务） |
| evidence | string | 否 | 依据来源，如"年报前五大客户"、"互动易问答" |
| as_of | date | 是 | 该关系的观测日（关系会变，需要时效） |

语义要求：

- **方向必须明确**：A 是 B 的供应商 ≠ B 是 A 的供应商
- **不接受推导值**：不要用"同行业"自动推成"上下游"
- **可以只覆盖重点票**：先覆盖 Mac 侧观察池和持仓的票（当前 7 只），逐步扩展

### 第 3 层：股权关联（**需要，优先级次之**）

定义：A 与 B 存在**股权或控制关系**。

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| subject_code | string | 是 | 主体票 |
| object_code | string | 是 | 关联票 |
| relation | enum | 是 | `holds_stake`（持股）/ `same_controller`（同一实控人）/ `subsidiary`（子公司） |
| stake_pct | f64 | 否 | 持股比例（%） |
| as_of | date | 是 | 观测日 |

---

## 三、希望的形式

**首选**：新增一个 op（例如 `RelatedCompanies`），一次查询返回某只票的全部关联。

请求：

```json
{
  "codes": ["600703"],
  "layers": ["supply_chain", "equity"],
  "as_of": "2026-10-18"
}
```

响应：

```json
{
  "as_of": "2026-10-18",
  "records": [
    {
      "subject_code": "600703",
      "object_code": "002131",
      "layer": "supply_chain",
      "relation": "supplier",
      "strength": "core",
      "evidence": "2025 年报前五大供应商",
      "source": "annual-report",
      "source_at": "..."
    }
  ]
}
```

**次选**：如果对端已有现成的批量导出（CSV/JSON 落盘），Mac 侧可以定时拉取并本地建表，不必做成 RPC。

---

## 四、Mac 侧会怎么用

1. **盘中传导提醒**：观察池的票出异动 → 查它的关联票 → 关联票是否同步异动（真假联动）
2. **新闻传导**：某票利好 → 它的客户/供应商是否值得提前关注
3. **板块轮动归因**：某板块涨 → 是龙头带动还是全链联动

---

## 五、边界

- **不需要实时**：关系是低频数据（季度/半年更新一次足够），但需要 `as_of` 标明时效
- **不需要全市场**：先覆盖观察池 + 持仓（当前 7 只），验证有效后再扩
- **宁可少而准**：一条错误的上游关系会导致错误推荐，比没有更糟

---

## 六、当前 Mac 侧已有的、和这个需求容易混淆的东西

说明清楚，避免重复建设：

| 名称 | 实际内容 | 是否是关联公司 |
|---|---|---|
| `BoardConstituents` | 板块成分股 | 是（同板块，最弱的一种关联） |
| `chain_batch`（op 61）| **涨停板块统计**（board_name + 涨停数 + 连板数 + 成分）| **否** —— 名字叫 chain，实际不含上下游 |
| `stock_concepts` | 票→概念标签 | 部分（同概念） |
| `concept_cooccurrence` | 概念共现（当前 0 行，无写入方）| 否 |

特别说明 `chain_batch`：它的字段是 `upper_limit_count` / `continuous_count` / `members`，
描述的是"哪些板块今天有涨停"，**没有任何上下游或股权字段**。
如果 Windows 侧原本打算用这个来满足"关联公司"，那不够。
