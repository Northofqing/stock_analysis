# QualifiedTradingFacts（op 67）：Mac 侧准备清单与联调安排（2026-10-17）

- 回执方：Windows 侧 ExternalV1 数据面
- 状态：**实现已完成并合入 main（commit `e6bd4d6`），尚未部署**。线上仍为 `8523c1e`（66 RPC，contract `dba66a27…`）。部署时 `contract_sha256` 会变化。
- 依据链：`2026-10-17-p01-provider-sourced-trading-facts-spec.md` → 我方可行性评估（2/3 子集）→ `2026-10-17-p01-spec-confirmation.md`（两点全接受）→ Gate A 设计 D1a–D8a 全案拍板（2026-10-17）→ 实施完成。
- 性质：**协调与准备清单**，不是上线通知；本回执不承诺部署日期。

## 1. 最终合同（与你们确认的两点一致）

- 操作：`MarketDataService.QualifiedTradingFacts`，第 67 个只读操作。
- 请求 schema：`magic.market.qualified_trading_facts.request` v1；记录 schema：`magic.market.qualified_trading_fact` v1。
- 交付子集：`lifecycle` ✓、`price_band`（仅观测快照日）✓、`suspension` ✗（显式请求 → typed `Unsupported`）。
- GetCapabilities 中该操作的 provider 为 `Composition`，scope 原文标注「provider-sourced observed facts… no suspension source; no historical price band」。

### 1.1 对你们三问的最终答案

1. **枚举值**：`quality` 恒为 `provider_observed`；`source` ∈ {`tdx-security-metadata`, `tencent-market-statistics`}；`price_band_source` 为 `tencent-market-statistics` 或 `null`。
   **注意**：与我们 10-17 可行性评估中的草案不同，定稿值是 `tencent-market-statistics`，不是 `tencent-quote-batch`。依据：涨跌停价本来就由腾讯 `MarketStatistics` 这一既有 wire 记录提供（实测 600519：`upper_limit=1444.27`、`lower_limit=1181.67`，带完整批次证据），新 op 直接投影该记录；`tencent-quote-batch` 作废。
2. **`Unsupported` 的 typed 形状**：gRPC `UNIMPLEMENTED`；binary metadata 键 `magic-error-detail-bin` 解码为 `ErrorDetail{request_id, operation, provider, reason_code="capability_unadmitted", retryable=false, admission=Unadmitted, provider_attempts…}`；message 携带原因原文（`no admitted suspension source exists for qualified trading facts`）。请按 `reason_code` 分支，不要做字符串匹配。
3. **`listing_state` 派生**：`listed_on ≤ date` → `listed`；`listed_on > date` → `not_yet_listed`；`listed_on` 缺失 → `unknown`；**`delisted` 永不产出**（无退市来源）。`listed` 不构成"在交易"证明。

### 1.2 请求边界

- `instruments`：1..=50，去重，仅沪深京 A 股个股权益。
- `dates`：1..=250，去重，`YYYY-MM-DD`，**必须落在签入的 SSE 2026 交易日修订内**（2025/2027 会被拒）。
- 行数：`instruments × dates ≤ 500`。
- `include`：可省略（= `lifecycle`+`price_band`）；显式空数组无效；显式 `suspension` → 上述 `UNIMPLEMENTED`。
- 行序：`instruments` 输入顺序 × `dates` 升序；`complete=true` 是 fail-closed 语义（任一批次失败或缺失标的即整单失败）。

## 2. 你们现在就能做（不等部署）

1. **解析器适配新记录 schema**：新增 `price_band_evidence` 字段；`limit_up`/`limit_down` 是裸 `f64`（不是 `{value,unit}`）；`listing_state` 是字符串；`quality`/`source`/`price_band_source` 按 §1.1 的封闭枚举处理。
2. **错误分支**：`UNIMPLEMENTED + reason_code=capability_unadmitted`（suspension，不要重试）；`INVALID_ARGUMENT`（越界/日历外日期/未知字段）；`FAILED_PRECONDITION`（批次缺标的/缺源时间）。三类语义不同，建议分别记录。
3. **语义边界写进消费代码**：`suspension` 一律按 unknown 路由（禁止"空数组=无停牌"或默认 false）；`listed ≠ 在交易`；`price_band_source=null` 表示该日价带不可用；观测快照日 = 腾讯统计批次 `source_at` 的上海自然日（不是请求日、不是服务器当天）。
4. **桥侧路由白名单**：把 `QualifiedTradingFacts` 加入 `build_external_query_request`（目前只接 BoardFlows/GlobalNews/InstrumentNews/MarketAnnouncements/MoneyFlows/SecurityMetadata 六个 op；不加则调不到）。
5. **先用 §4 的 SYNTHETIC 帧把解析器和单测跑通**，不要等部署。

## 3. 部署后必须做（与部署同一窗口）

6. **重钉**：接收新 client bundle（`contract_sha256` 变化），校验 bundle version + source commit + `manifest.sha256` + GetHealth 四字段一致后重钉。重钉完成前你们 fail-closed 是设计内行为，不是故障。
7. **联调验收**：`lifecycle` 正例（listed/not_yet_listed/unknown 各一）、快照日 `price_band` 正例、`suspension` 负例（预期 `UNIMPLEMENTED`）、日历外日期负例（预期 `INVALID_ARGUMENT`）。
8. **回写证据**：把重钉结果（bundle 版本 + `contract_sha256` + `source_revision`）写入本通道，形成闭环；同时补上 `2026-10-16.1` 重钉的书面确认（该确认目前仍缺）。

## 4. SYNTHETIC 样例帧（构造数据，仅作解析器夹具）

**以下所有数值、批次号、时刻均为构造样例，非生产数据，不得作为证据、不得用于验收。**

请求：

```json
{
  "instruments": [{"exchange": "Shanghai", "code": "600000", "asset_class": "Equity"}],
  "dates": ["2026-10-15", "2026-10-16"],
  "include": ["lifecycle", "price_band"]
}
```

响应 envelope 关键字段（合成）：

```json
{
  "provider": "Composition",
  "batch_id": "tdx:1792203685:security-metadata:qualified-trading-facts",
  "complete": true,
  "observed_at": "2026-10-17T10:21:25+08:00",
  "source_at": null
}
```

记录 1（非快照日：价格带为 null）：

```json
{
  "instrument": {"exchange": "Shanghai", "code": "600000", "asset_class": "Equity"},
  "date": "2026-10-15",
  "quality": "provider_observed",
  "source": "tdx-security-metadata",
  "observed_at": "2026-10-17T10:21:25+08:00",
  "source_at": null,
  "listed_on": "1999-11-10",
  "listing_state": "listed",
  "limit_up": null,
  "limit_down": null,
  "price_band_source": null,
  "evidence": {"provider": "Tdx", "source_at": null, "observed_at": "1792203685", "batch_id": "tdx:1792203685:security-metadata"},
  "price_band_evidence": null
}
```

记录 2（快照日：价格带来自腾讯 MarketStatistics，数值为构造值）：

```json
{
  "instrument": {"exchange": "Shanghai", "code": "600000", "asset_class": "Equity"},
  "date": "2026-10-16",
  "quality": "provider_observed",
  "source": "tdx-security-metadata",
  "observed_at": "2026-10-17T10:21:25+08:00",
  "source_at": null,
  "listed_on": "1999-11-10",
  "listing_state": "listed",
  "limit_up": 10.55,
  "limit_down": 8.63,
  "price_band_source": "tencent-market-statistics",
  "evidence": {"provider": "Tdx", "source_at": null, "observed_at": "1792203685", "batch_id": "tdx:1792203685:security-metadata"},
  "price_band_evidence": {"provider": "Tencent", "source_at": "2026-10-16T16:14:47+08:00", "observed_at": "1792203713.191533300", "batch_id": "tencent-web:1792203713.191533300:market-statistics"}
}
```

`include=["price_band"]` 时：`source` 变为 `tencent-market-statistics`，`listed_on`/`listing_state`/`evidence` 的 lifecycle 语义不产出（前两者为 null，`source_at` 携带统计批次源时间）。错误帧仅为 `ErrorDetail` 结构（见 §1.1），无 record body。

## 5. 协调与时间

- 现在：你们做 §2 的 1–5；我方不部署、不改线上。
- 部署：窗口由双方确认后另行通知；部署与你们的重钉必须同窗口，中间态 Mac 会 fail-closed。
- 本回执不构成部署日期或验收完成承诺。

## 6. 边界声明

本回执不授予数据权利、不改既有合同、不承诺未拍板的时间表。SYNTHETIC 样例仅用于解析器与单测夹具；任何生产判断必须以部署后的真实批次为准。
