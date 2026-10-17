# P0-1 规格单确认回复（Mac 侧）

- 日期：2026-10-17
- 对应回执：`2026-10-17-p01-provider-sourced-trading-facts-evaluation.md`
- 状态：**待 Mac 侧 operator 签发**（本文件为草稿，签发前不代表已确认）

---

## 结论

**两点全部接受，请按回执 §6 起草 Gate A 设计。**

---

## 1. 接受「suspension 不交付 + typed Unsupported」

- 接受。消费侧按 **unknown 路由**：`suspended=null` 与"未提供该子集"同等对待，**不进入"确定在交易"分支**。
- 认可回执的理由（"连 false 语义都无法给出 → 全 null 占位会被误读为已尽力查询"），**`typed Unsupported` 优于 null 占位**，本侧不会把它当作待补功能反复催。
- **影响记录**：依赖停牌事实的契约类判断（如快照 I-03/T-03 的停牌分支）在 Mac 侧**标为长期不可用**，除非另行走"立项权威源"路径。连板识别不依赖该字段，不受影响。

## 2. 接受 `price_band` 的「仅观测快照日」口径与边界

- 接受。以 `price_band_source` 为可用性门：**null = 该日价格带不可用**，消费侧不得用其他方式推导（禁"昨收 × 涨跌幅上限"）。
- 接受 **`instruments × dates ≤ 500 行`**：Mac 侧观察池规模为每日几只到十几只，单日查询远低于上限。
- 接受 **`dates` 必须落在已签入的 SSE 2026 交易日历内**（超出按 `InvalidRequest` 处理）。
- 接受记录 schema 用单数 `magic.market.qualified_trading_fact`，请求 schema 名维持 `...request`。

### 2.1 我方据此增加的一项义务（仅告知，不需对端处理）

由于历史日期无价格带来源，**Mac 侧必须在每个交易日查询一次并落库**，否则该日价格带永久缺失。该动作由 Mac 侧调度承担，不在本 op 范围内。

## 3. 对 §4.2（null 语义验收）的回应

回执指出原验收第 2 条（构造停牌票验 `true`/`false`）无法满足——**撤回该条**，改为：

- **验收**：显式请求 `include=["suspension"]` → 返回 `typed Unsupported`（reason: no admitted suspension source）；不请求时响应中不出现 `suspended` 字段（或恒 null 且 `quality` 注明该子集未交付）。
- 其余验收条（形状 / 禁推导 / 质量标记 / 空结果）按回执 §4 的口径执行。

## 4. 请对端在 Gate A 设计中明确的三点

1. **`price_band_source` 的确切枚举值**（回执 §1 提到 `tencent-quote-batch`）——Mac 侧要据此做断言。
2. **`Unsupported` 的 typed 错误形状**（错误码 / reason 串 / 是否带 capability 标识），以便消费侧精确分支，而不是靠字符串匹配。
3. **`listing_state.delisted` 恒为 `unknown` 时**，消费侧如何区分"确实在交易"与"退市/未知"——若只能靠 `listed_on` + 请求日派生，请在设计中写明派生规则。

## 5. 边界

本确认不含合同变更批准、不含实施排期承诺。若 Gate A 设计采纳上述口径，后续按既有流程出新版合同与 bundle，双方各自核对 pin（现役四字段见 `2026-10-17-windows-data-gaps-and-intraday-alert-evaluation.md` §7）。
