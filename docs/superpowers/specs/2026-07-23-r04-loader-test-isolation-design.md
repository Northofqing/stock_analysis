# R-04 龙虎榜加载器测试隔离设计

## 背景

`br140_r04_preserves_wait_and_missing_producer_outcomes` 直接调用生产
`fetch_recent_lhb`。当外部龙虎榜接口在测试日期返回真实记录时，测试会从
“能力缺失”分支进入 dry-run 投递分支，结果取决于网络与上游数据，违反测试/生产
隔离要求。

## 设计

- 保留 `dispatch_r04_lhb_outcome` 作为唯一生产入口，并继续注入真实
  `fetch_recent_lhb`。
- 提取一个私有的 `dispatch_r04_lhb_outcome_with_loader`，接收一次性、可发送到
  blocking worker 的加载闭包。
- 生产闭包仍读取东方财富真实龙虎榜；不增加 mock、缓存或空数据 fallback。
- 单元测试只向私有 seam 注入确定性的 `unavailable:` 错误，验证 BR-140 在
  21:00 前返回 `ExpectedWait`、到点后返回 `Disabled(lhb_producer)`。
- 闭包仍在 `spawn_blocking` 中执行，保持阻塞 HTTP 不进入 Tokio worker。

## 失败模式

- 加载器返回 `unavailable:`：能力缺失，返回 `Disabled`。
- 其他业务错误：返回可重试 `Failed`。
- blocking worker panic/cancel：返回可重试 `Failed`。
- 返回完整空批次：返回 `NoData`。

## 旧模块关系

| 模块 | 处理 | 原因 |
| --- | --- | --- |
| `market_analyzer::lhb_review::fetch_recent_lhb` | 采用 | 生产真实源不变 |
| `dispatch_r04_lhb_outcome` | 采用 | 保持外部接口和调度语义 |
| 测试直接访问生产网络 | 拒绝 | 结果不确定且违反 2.5 |

## 回滚

独立提交回滚：`git revert <commit-sha>`。回滚不修改生产数据、配置或审计文件。
