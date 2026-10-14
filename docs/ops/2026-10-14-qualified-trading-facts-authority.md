# 2026-10-14 qualified trading facts authority（甲-1）：阻塞点、决策与实施状态

> 用户决策：问题「停牌这个事实从哪来」→ 答「甲-1」＝**由 Windows/桥侧交付该事实**。
> 同轮另一项决策：trading 侧那两处 5s 门 →「要」（一并放宽）。

## 1. 阻塞点（实测）

三处生产链**全部**被同一个 reason_code 挡住：

```
[候选入池][BR-183] 时代新材(600458) 无真实实时报价, 跳过入池 (红线 2.2):
    remote realtime quote 600458: 600458 qualified trading facts unavailable
    reason_code=trading_fact_contract_not_delivered
```

| 消费方 | 位置 | 现象 |
| --- | --- | --- |
| 候选入池（红线 2.2） | `src/opportunity/candidate_panel.rs:369` | 连续 14 次 tick 全 `recorded=0`（每次评估 7–14 个候选，逐个跳过） |
| 行情价带 | `src/bin/monitor/market_data.rs:618` | 报价路径要求 lifecycle=Listed + suspension=Trading + 价带 |
| AccountMode / 账户 metrics | `HostBR-108` 评估 | `metrics unavailable; evaluate conservatively: … trading facts unavailable` → 保守（Frozen） |

## 2. 三件事实的来源取证（2026-10-14）

| 事实 | 来源结论 | 证据 |
| --- | --- | --- |
| lifecycle（上市/退市） | **结构上有字段、来源上未证明** | 视图带 `listed_on`（`market_capabilities.rs:107`），但外部 API 文档写明腾讯来源**"来源未证明的上市日期…保持 unavailable"**；`security_lifecycle.rs`（BR-171）是可复用的证据保全实现，同样依赖已证明的源 |
| 价格带（涨跌停） | **结构上有字段、来源上未证明** | 视图带 `board`/`is_st`/`price_limit_percent`/`price_limit_version`，但文档写明**"涨跌停规则及规则版本保持 unavailable"**，且 `complete=false` 是常态 |
| 停牌（suspension） | **无源** | ① 两份合同无任何停牌字段；② 本地 metadata 视图 JSON 只有 `{code,name,board,is_st,listed_on,price_limit_percent,source_at}`；③ 外部 metadata 的 `status` 只有 `Available/Unavailable`（＝服务端能否供数，非交易所状态）；④ 正式停牌缓存 `mark_halted_period`（`data_quality.rs:180`）**无任何调用方**；⑤ `halt_status.rs` 文件头明文禁止用 K 线缺口做停牌授权（只能诊断） |

**载体结论（2026-10-14 追加）**：既有视图都**不能**承载本 facts：

- `SecurityMetadata`（腾讯）：只证明名称/ST/派生板块，上市日期与涨跌停规则未证明（见上）；
- `SecurityProfiles`（TDX F10）：文档原文**"F10 没有可信源时间，因此 `source_at` 为空"**，
  而本合同每条记录必须带非空 `source_at` → 证据不成立。

因此交付单已改为「**新增 payload / 由你方指定可承载的 op**」，并要求对方**从能证明这些
事实的来源**给出（交易所发布或自有权威源），不得把未证明字段重新打包。

另外：`market_capabilities.rs` 对 metadata 视图的注释明确写了
**"This view does not prove qualified trading facts."**，且生产里无人调用它——
即设计意图是**一份独立准入的 authority 合同**，而非复用 legacy 视图。

## 3. 已实施（Mac 侧，commit 待发布）

1. **新模块** `src/data_gateway/trading_facts_authority.rs`
   - 合同常量：`magic.market.qualified_trading_facts` / schema_version 1 /
     `contract_version = qualified-trading-facts-authority-v1`；
   - `parse_authority_record`：严格解析 lifecycle / price_regime / suspension / evidence，
     含 tick 整除、覆盖窗口升序不重叠、`source_at ≤ observed_at`、退市日晚于上市日等硬校验；
   - `publish_authority_batch(schema, version, records)`：**整批或全拒**（任一条非法 → 不发布任何记录），
     避免同批不同标的证据等级不一致；
   - 进程内存储 + `lookup_authority_record`。
2. **接线** `QualifiedTradingFactsGateway::acquire`（`qualified_trading_facts.rs:744`）：
   有已发布记录 → `admit` 后返回；无 → **行为与改造前完全一致**（`ContractNotDelivered`，fail-closed）。
3. **对端交付单**：`client-bundle/WINDOWS_TRADING_FACTS_AUTHORITY_DELIVERY_REQUEST_20261014_V1.md`
   （字段规格 + 硬约束 + 9 项回执清单）。
4. **trading 侧两处 5s 门放宽**（用户「要」）
   - `paper_trade.rs` `validate_realtime_quote_freshness`：5s 降为标注阈值，
     新增 `REALTIME_QUOTE_REJECT_AGE_MILLIS = 300_000` 作拒绝阈值；
   - `paper_ledger_execution.rs` `fresh()`：同款（标注 5s / 拒绝 300s）；
   - 未来时间仍硬拒；超标注阈值逐次 warn（非静默）；回退＝把拒绝阈值改回 5s/5。

## 4. 未完成（写清，不当承诺）

- **取数调用未接线**：monitor 定时向 `SecurityProfiles` 取本 schema 并调
  `publish_authority_batch` 这一步**尚未实现**。原因：对端尚未确认 payload 可服务；
  在确认前接线只会每分钟产生一批失败日志，无收益。
- **对端交付未完成**：Windows 侧需按交付单实现并回执。**这才是解封点**——
  在它交付前，候选入池仍 `recorded=0`、AccountMode 仍保守评估。
- 本模块**不影响**既有 `SecurityMetadata` v1 冻结合同（新增 schema 是独立路径）。

## 5. 解封链路（供后续验收对照）

```
对端实现 payload
  → 回执确认（交付单 §5 九项）
    → Mac 侧接线取数（SecurityProfiles → publish_authority_batch）
      → acquire 开始返回 Available
        → 候选入池 recorded>0 / AccountMode 有 metrics / 行情价带可用
```

每一步的验收口径：候选入池看 `recorded=` 是否 >0；AccountMode 看是否不再
`evaluate conservatively`；价带看 `qualified_price_band` 是否成功。
