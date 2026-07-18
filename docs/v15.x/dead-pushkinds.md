# Dead PushKinds 归档 (v17.x 清理批)

> **2026-07-17 勘误**：本审计把已登记为 BR-033/BR-034、仍受业务规格约束的大宗交易能力误判为可删除死代码。两项能力已恢复，`DISPATCH_TABLE` 恢复为 15 行；真实事件源未注册或失败时按 BR-087 显式不可用，不再把“零调用者”直接等同为“无业务承诺”。下文保留为当时审计记录。

> 按 v17.7 §2.4 / v17.8 AC90 要求建立的单一事实源。
> 恢复方式: `git log --all --oneline -- docs/v15.x/dead-pushkinds.md` 找到删除 commit 后 `git revert`。
> 前置: 每项删除前均通过"逐变体调用链审计"(从 dispatch/push_governor 实参反向追到 main.rs), 单行 grep 不作为证据 (v17.5 §2.2 勘误教训)。

## 已删除 (2026-07-16, 审计确认 0 生产调用链)

| PushKind | 批次 | 原用途 | 审计证据 | 删除内容 |
|----------|------|--------|----------|----------|
| OptimalClose | v17.5 | B3 优选候选 (移交候选台) | 仅 enum+label+adapter 映射, 无 dispatch | enum 变体 + label + v14_adapter 映射 |
| VolumeWatchlist | v17.5 | B6 放量·自选 (移交候选台) | 同上 | 同上 |
| VolumeRealTrade | v17.5 | B7 放量·实盘优选 (移交候选台) | 同上 | 同上 |
| BlockTradeIntradayConfirm | v17.8 | T-18 创业板大宗盘中确认 (v13.1 新规, 未接数据源) | dispatch fn 存在但 0 调用者 | enum + dispatch fn + render fn + params + BlockType/Board/SettleType 枚举 + DISPATCH_TABLE 行 + 测试 |
| BlockTradePriceRange | v17.8 | T-19 北交所大宗价格区间 (v13.1 新规, 未接数据源) | 同上 | 同上 |

注: `CandidateSource::OptimalClose/VolumeWatchlist/VolumeRealTrade` (候选台源枚举, main.rs:11162-11166) 与 PushKind 同名不同物, **保留**。

## 审计后保留 (spec 原名单内, 不删)

| PushKind | spec 原判 | 审计结论 |
|----------|-----------|----------|
| CandidateTriggered | v17.5 "enum 仅存" | **活链路** main.rs:8182 → dispatch_candidate_triggered_daily (ENABLE_CANDIDATE_LIVE 影子开关控制) |
| VirtualWatch | v17.5 "enum 仅存" | **活链路** main.rs:8896 → dispatch_virtual_watch_daily (P-05) |
| CandidateInvalidated | v17.5 "enum 仅存" | 半死 (push_candidate_invalidated 无调用者), 与 Triggered 同家族, 同批复审 |
| AuctionRepush | v17.5 | spec 即要求保留 (历史兼容) |
| FactorIC / SectorTier / CapitalVerify | v17.6 "grep 0 命中" | **活链路** main.rs:8523; 已走 sub_kind 分流 (daily_report_router) |
| Announcement / PolicyHit / EarningsBeat / EarningsMiss / AnalystUpgrade / MarketActionAlert | v17.7 "直接删除" | 无生产调用但为 active spec targets (2026-07-16 用户方案 B: 保留待接数据源) |
| PostFixedPriceOrder / PostFixedPriceFill | v17.8 "0-caller" | **活链路** T-14/T-15 管道 main.rs:10069/10081 |
| StPriceLimitChanged | v17.8 "0-caller" | **活链路** main.rs:6561 + 10151 (v59 F2 修复) |
| EtfClosingCallAuction | v17.8 "0-caller" | **活链路** main.rs:10221 (v59 F2 修复) |

## 计数

- 删除前 PushKind 变体: 57 → 删除后: **52**
- DISPATCH_TABLE: 15 → **13** rows (v17.6=3 + v17.7=6 + v17.8=4)
