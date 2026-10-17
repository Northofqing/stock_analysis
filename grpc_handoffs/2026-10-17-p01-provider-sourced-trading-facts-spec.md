# P0-1 规格单：QualifiedTradingFacts —— provider-sourced 观察值中间态

- 日期：2026-10-17
- 来源：Mac 侧 monitor（ExternalV1 消费方）
- 对应回执：`2026-10-17-windows-data-gaps-and-intraday-alert-evaluation.md` §2.1 / §4
- 决策：Mac 侧 operator 已拍板 **走 B 路线**（接受 provider-sourced 观察值中间态），不走"立项新权威源"（A 路线）
- 编号：**QualifiedTradingFacts 若立项，Operation 号自 67 起**（66 已被 `TradingCalendar` 占用，已核对）

---

## 一、本单要解决什么

回执 §2.1 指出：本侧（Windows）**没有任何权威停牌来源**，`listed_on` 与价格带均为非权威且无源时间，故按原规格**无法交付**。

本单据此**放宽语义**：不要求"权威"（authoritative），只要求**"可追溯的观察值"（provider-sourced observed value）**，并强制携带质量标记，由消费方自行降级使用。

**核心取舍**：拿得到 > 权威。三件事实（停复牌 / 上市日 / 价格带）在当前阶段用于**候选筛选与风险提示**，不用于"打板确认"这类硬判断。质量标记就是为了让这个边界在 wire 上可判。

---

## 二、Wire 定义

沿用现有 `QueryRequest` / `QueryResponse` 形状，新增一条能力：

```
operation: QualifiedTradingFacts          (建议 Operation 67)
request schema:  magic.market.qualified_trading_facts.request  @ v1
result  schema:  magic.market.qualified_trading_facts.result   @ v1
content_type:    application/json; charset=utf-8
```

### 2.1 请求

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| instruments | array | 是 | 元素同现有 `InstrumentId`（exchange / code / asset_class） |
| dates | array\<string\> | 是 | ISO 日期（`YYYY-MM-DD`），一次查询内不跨年；上限 250 个交易日 |
| include | array\<string\> | 否 | 子集选择：`lifecycle` / `price_band` / `suspension`；缺省=全部 |

### 2.2 结果：逐 (instrument, date) 一条记录

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| instrument | object | 是 | 同请求 |
| date | string | 是 | `YYYY-MM-DD` |
| **quality** | string | 是 | **`authoritative` \| `provider_observed`** —— 本单交付的通常是后者 |
| **source** | string | 是 | 来源标识（如 `tdx-smart`、`ths-metadata`、`tdx-quote-batch`），非空 |
| **observed_at** | string | 是 | ISO 8601 带时区，本侧实际观测到该值的时刻 |
| source_at | string | 否 | 源的原始时间；**不可得时置 null，这是本单明确允许的** |
| suspended | bool \| null | 否 | 停牌为 true；正常交易为 false；**无法判定时为 null（不得用 false 冒充"确定在交易"）** |
| listed_on | string \| null | 否 | 上市日 `YYYY-MM-DD`；未上市/已退市时按 `listing_state` 解释 |
| listing_state | string | 否 | `listed` \| `not_yet_listed` \| `delisted` \| `unknown` |
| limit_up | number \| null | 否 | 涨停价（源值原样，不做"昨收 ×1.1"推导） |
| limit_down | number \| null | 否 | 跌停价（同上） |
| price_band_source | string \| null | 否 | 价格带单独来源（可与 `source` 不同） |
| evidence | object | 是 | 同现有 `SourceEvidence` 形状（provider / batch_id / raw 片段摘要等） |

### 2.3 语义约定（关键，请按此实现）

1. **`quality=provider_observed` 不是免责声明，是降级指令**：消费方拿到它必须按"可能偏差"处理——用于筛选与提示，不用于终态判定。
2. **`suspended=null` ≠ `suspended=false`**：null 表示"无法判定"；false 表示"确定在交易"。不得把 null 折叠成 false（这是 BR-222 的同类要求：不补值）。
3. **价格带只接受源值**：禁止用"昨收 × 涨跌幅上限"推导。源没有就给 null。
4. **`source_at` 允许为 null**，但 `observed_at` 与 `source` 必须非空——这是"可追溯"的最低门槛。
5. **同一条记录内 quality 单一**：若生命周期与价格带质量不同，拆成两条记录，不得混装。

---

## 三、校验放宽点（相对原规格）

| 项 | 原规格（回执 §2.1 指其无法满足） | 本单放宽为 |
|---|---|---|
| 停牌来源 | 交易所权威源 | 任一非空 `source` + 非空 `observed_at` |
| `listed_on` | 权威元数据 + 源时间 | 同上传 + `source_at` 可空 |
| 价格带 | 权威源 + 源时间 | 源值原样 + 非空 `source` + `observed_at` |
| 质量 | 隐含要求 authoritative | 显式 `quality` 字段，允许 `provider_observed` |

**不放宽的部分**：`suspended` 的 null 语义、价格带禁推导、记录与请求的身份一致性、`complete` 标志的真实含义。

---

## 四、验收标准

1. **形状**：请求 `dates` 含 3 个交易日、2 只票 → 返回 6 条记录（或显式声明某日无数据）；`quality` / `source` / `observed_at` 非空。
2. **null 语义**：构造一只停牌票 + 一只正常票 → 前者 `suspended=true`、后者 `false`；再构造一只源无信息的票 → `suspended=null` 且**不得**等于 false。
3. **禁推导**：构造一只源缺失价格带的票 → `limit_up` 为 null，**不得**出现"昨收×1.1"的值。
4. **质量标记**：本单交付的记录 `quality` 应为 `provider_observed`；若某源确有权威性，可按 `authoritative` 交付并说明依据。
5. **空结果**：整批无任何记录时返回 `complete=true` + 空 records（区别于"部分覆盖"的 `complete=false`）。

---

## 五、Mac 侧配套改动（不需要对端做，仅告知）

1. **消费侧接受 `provider_observed`**：连板识别/快照只把它当"候选筛选依据"，硬判定仍要求 `authoritative`；这条边界会写进 Mac 侧适配器的断言。
2. **`suspended=null` 的处理**：按"未知"路由，不进入"在交易"分支。
3. 若对端最终选择不交付本单，Mac 侧保持现状（fail-closed，这三件事实相关的判断继续不可用）。

---

## 六、请对端答复

1. 上述 wire 定义是否可实现？字段有无需要调整（尤其 `evidence` 是否复用现有 `SourceEvidence`）？
2. **Operation 号**：确认从 67 起（66 已是 `TradingCalendar`）？
3. `dates` 上限 250 个交易日是否合适？对端内部是否有更小的实际限制？
4. 三个子集（`lifecycle` / `price_band` / `suspension`）能否独立交付？若只能整体交付，请说明。
5. 若选择不交付，请明确"不交付"，Mac 侧据此把相关判断标为长期不可用（不再反复催）。

---

## 七、边界

本单是**规格请求**，不构成合同变更批准，也不承诺对端排期。若对端采纳，需按既有流程出新版合同与 bundle，并由双方各自核对 pin（现役四字段见回执 §7）。
