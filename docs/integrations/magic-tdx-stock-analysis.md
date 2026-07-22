# magic-tdx-rs 接入记录

## 范围

`MagicTdxProvider` 使用 `magic-market-data-rs` 的 TDX 客户端作为现有
`DataFetcherManager` 的第一优先真实行情源。当前接入日线、实时行情和证券名称；
旧 RustDX、腾讯和东方财富源继续作为失败回退。

## 数据边界

- 日线只接受真实 TDX 返回的 OHLCV，并继续执行 BR-092/BR-147 的正数、连续性、
  重复日期和相邻收盘变化校验。
- 实时行情的来源时间必须能从 TDX `servertime` 解析；解析失败直接返回错误，
  不用进程本地时间冒充来源时间。
- 资金流、集合竞价和其他 TDX 未提供的能力保持显式不可用，由专用数据源负责。
- 任何传输、协议或质量失败均进入回退链，不创建默认价格、成本或收益。

## 回滚

移除 `MagicTdxProvider` 注册及 Cargo path 依赖即可恢复旧 provider 链；不删除已有
`stock_daily`、用户快照或收盘估值记录。
