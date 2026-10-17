# 对《P0-1 规格单：QualifiedTradingFacts——provider-sourced 观察值中间态》的可行性评估（Windows 侧）

- 对应来单：`2026-10-17-p01-provider-sourced-trading-facts-spec.md`
- 回执方：Windows 侧 ExternalV1 数据面
- 回执日期：2026-10-17（Asia/Shanghai）
- 性质：**可行性评估**。不含合同变更批准、不含实施排期承诺；可行性结论基于本仓现有已准入来源的静态核查，实施与否由 Windows 侧 operator 按 Gate A 流程拍板。

## 0. 结论先行

- **可实现子集**：`lifecycle` ✓、`price_band` ✓（**仅观测快照日**）；`suspension` ✗。
- **suspension 明确不交付**：本侧没有任何停牌来源（全 provider crate 核查），**连"确定在交易"的 false 语义都无法给出**——按来单 §2.3.2 的 null 语义，该子集只能永远 null，等于空字段，不应交付。
- wire 定义基本可实现，需 **3 处调整**（§3）。
- 若来单方接受 §3/§6 的口径，我方将按 Gate A 流程产出设计（建议实现形态：**组合级 op，复用已准入的 TDX `SecurityMetadata` 与腾讯行情批次，不新增 provider**），实施待 operator 拍板。

## 1. 逐字段可行性（静态核查）

| 字段 | 来源 | 可交付 | 说明 |
| --- | --- | --- | --- |
| instrument / date / source / observed_at / evidence | 组合层 | ✓ | evidence 复用现有 `SourceEvidence`（provider / source_at / observed_at / batch_id） |
| quality | — | ✓ | 本单交付记录一律 `provider_observed`（无 authoritative 来源） |
| source_at | — | ✓（允许 null） | TDX 元数据无源时间；按 BR-050 保持 null，不代填 |
| listed_on | TDX finance `ipo_date`（经 `magic-tdx-rs/src/adapter.rs:1844-1909` 的 `normalize_ipo_date` 归一，随 SecurityMetadata 批次下发） | ✓ | 源值原样；`observed_at` = 元数据批次观测时刻 |
| listing_state | 派生 | **部分** | `listed` / `not_yet_listed` 可由 `listed_on` 与请求日派生；`delisted` **无来源**（TDX/Hithink 均没有可用的退市日期字段）→ `unknown` |
| limit_up / limit_down | 腾讯行情批次 `upper_limit` / `lower_limit`（`magic-tencent-rs/src/lib.rs:213-214, 535-536`，源值、含上下限一致性校验） | ✓ **仅观测快照日** | 历史日期的涨跌停价本侧**没有来源**（行情快照不携带历史价格带）→ 历史日期置 null |
| price_band_source | — | ✓ | 有值 = `tencent-quote-batch`（该日价格带可用）；null = 该日价格带不可用（请以此字段作**可用性门**） |
| suspended | **无来源** | ✗ | 全仓核查：无停牌事件源/停牌日历源；竞价快照的 `data_status=suspended` 只在竞价窗口、且无法给出 false 语义，不构成逐日事实 |

## 2. 对来单 §2.3 语义约定的核对

1. `quality=provider_observed` 作为降级指令 ✓ —— 本单我们只有这一类。
2. `suspended=null ≠ false` ✓ —— 本侧不交付 suspension，因此不会出现任何 false；消费侧按"未知"路由（与来单 §五.2 一致）。
3. 价格带禁推导 ✓ —— 只发腾讯源值；源无 → null；绝不出现"昨收 × 1.1"。
4. `source_at` 可空、`source` / `observed_at` 非空 ✓。
5. 记录内 `quality` 单一 ✓ —— 本单无混装场景。

## 3. Wire 调整建议（3 处）

- **(a) `include` 缺省语义**：缺省 = `lifecycle` + `price_band`；**显式包含 `suspension` → typed `Unsupported`**（reason：no admitted suspension source）。不采用"全 null 占位"——null 虽合语法，但等于永不判定的空字段，容易误读为"已尽力查询"。
- **(b) `price_band` 的日期适用性**：仅**观测快照日**有值；其余日期 `limit_up` / `limit_down` 为 null 且 `price_band_source` 为 null。请在验收与消费侧以 `price_band_source` 判断该日价格带是否可用。
- **(c) 边界**：建议 **instruments × dates ≤ 500 行**（部署 payload 上限 512 KiB + 快照语义）；`dates` 上限 250 可谈，但必须落在**已签入的 SSE 2026 交易日历**内（超出 → typed `InvalidRequest`；2025 及更早不在签入范围）。另建议记录 schema 用单数 `magic.market.qualified_trading_fact`（本仓记录 schema 惯例），请求 schema 名维持来单写法。

## 4. 对来单 §四验收标准的逐条回答

1. **形状**：✓ 可满足（3 交易日 × 2 只票 = 6 行；非观测日的价格带为 null 并在响应中显式声明）。
2. **null 语义（停牌构造）**：✗ **无法满足**——没有停牌来源，造不出 `true`/`false`。请把该条改为"不交付 suspension，消费侧按 unknown 路由"。
3. **禁推导**：✓ 可满足（源缺 → null）。
4. **质量标记**：✓（全量 `provider_observed`）。
5. **空结果**：部分满足——空 instruments → 拒绝；请求内任一标的在源侧缺失 → **typed 拒绝**（fail-closed，不静默省略行）；"整批零行"仅在请求本身为空时出现，按 verified-empty 处理（`complete=true` + 空 records）。

## 5. 对来单 §六六问的答复

1. **可实现性**：可实现，需 §3 三处调整；`evidence` 复用 `SourceEvidence` ✓。
2. **Operation 号**：确认自 **67** 起（66 = `TradingCalendar`，已核对）。
3. **dates 上限**：250 可谈，但建议叠加"行数 ≤ 500 + 必须落在签入日历"（§3c）。
4. **子集独立交付**：`lifecycle` ✓、`price_band` ✓（快照日）、`suspension` ✗。
5. **是否交付**：我方选择**交付可实现子集**（不是整体不交付）；实施需 Windows 侧 operator 按 Gate A 拍板，本回执不含排期。
6. （无）。

## 6. 需要来单方确认的两点

1. 接受"**suspension 不交付 + typed Unsupported**"（消费侧按 unknown 路由）；
2. 接受 `price_band` 的"**仅观测快照日**"口径与 §3(c) 的行数/日历边界。

确认后我方起草 Gate A 设计（op 67、请求/记录 schema、组合来源、探针与台账），供 operator 拍板后进入实施。

## 7. 边界声明

本回执不授予任何数据权利、不改任何既有合同、不承诺未拍板的时间表。可行性结论仅基于本仓 2026-10-17 的静态核查；所有 null 语义按来单 §2.3 与 BR-050/BR-222 的"不补值"原则执行。
