# Magic TDX 盘中形态 Gateway 切换设计（BR-187）

**状态**：Gate A approved
**范围**：`IntradayShapeGateway`、`DataFetchService::get_intraday_shape` 及资金流两个消费者
**上游固定版本**：`magic-market-data-rs@660902ff93a07f18367dc16879cf67732accd25a`

## 1. 目标

修复 `DataFetchService::get_intraday_shape` 在生产环境恒定返回
`Unsupported`，让 Agent 资金流工具和 pipeline extra context 能消费一份具备
完整来源证据的盘中形态。

本次只恢复数据获取边界和既有形态投影，不改变资金流口径、提示词决策或
候选股评分。

## 2. 数据流

```text
consumer
  -> DataFetchService (成功结果 evidence-aware cache，命中仍校验 source_at <= 5s)
  -> IntradayShapeGateway (async)
  -> spawn_blocking
  -> MagicTdxGateway::get_t0_evidence_batch([code], observed_at)
  -> 同一 Magic TDX T0 batch 完整性/身份/5 秒时效门
  -> BR-187 单批次纯投影
  -> GatewayBatch<IntradayShapeFact> + BR-159 acquisition audit
  -> IntradayShape domain value
```

形态所需的昨收、今开、最高、最低、当前价和已完成五分钟线必须来自同一
`MagicTdxT0Batch`、同一 `batch_id`。不得再请求第二行情源补昨收或尾盘锚点，
不得恢复已删除的 Eastmoney 分时 HTTP 解析器。

## 3. 投影规则

1. 请求只含一个合法 A 股代码；返回必须恰好一条同代码记录且无 rejection。
2. batch 与 record 的 `batch_id/source_at/observed_at` 必须完全一致。
3. `observed_at - source_at` 必须在 `0..=5s`；不得因盘前、午休或失败重试放宽。
4. 只从该 record 的 `completed_five_minute` 过滤来源交易日记录；记录必须已按
   时间严格升序且不得为空。
5. `open/high/low/close` 分别使用同批 quote 的开盘、最高、最低、当前价；
   `pre_close` 使用同批 quote 的昨收。
6. 尾盘 30 分钟涨跌只在存在 `>=14:30` 的首根已完成五分钟线时计算，锚点为
   该 bar 的 close，终点为同批实时价；尚未到该时段时保持 `None`。
7. 非有限值、非正价格、OHLC 关系冲突、计算后日内涨跌绝对值大于 20% 都显式
   拒绝；大于 20% 的错误必须标记 `manual_confirmation_required`。
8. 形态分类沿用被替换 acquisition 模块的纯计算阈值：冲高回落、尾盘跳水、
   尾盘拉升、高开低走、低开高走、稳步推高、持续下行、剧烈震荡、窄幅整理。
9. 缓存必须保存 Gateway 的原始 `source_at`；每次命中按当前 UTC 时间重新执行
   `0..=5s` 门，不能沿用其他日线/资金流的 5 分钟或 1 天 TTL。

### 3.1 BR-171 settled-daily confirmation blocker

固定上游 T0 合同中的 `MagicTdxT0DailyBar` 不携带 provider provenance 或
settled-daily batch ID；当前 T0 顶层 `batch_id` 只由 quote 的 source time 和
quote prices 生成，不能冒充日线批次身份。T0 acquisition 同时没有覆盖该日线
窗口的 listing/corporate-action lifecycle evidence。

因此当前合同无法生成
`DailyChangeConfirmationQuery(daily_batch_id + lifecycle_batch_id + exact closes)`，
也无法安全查询已有 BR-171 ledger。遇到 settled-daily 相邻变化超过 ±20% 时：

- 保留阈值并输出 WARN；
- 整只 T0 record 失败关闭；
- reason 固定为 `manual_confirmation_contract_unavailable`；
- detail 明确缺少 `settled_daily_provenance_batch_id` 与 `lifecycle_evidence`；
- 禁止把 quote-derived batch ID 填进 ledger query，禁止自动放行。

只有固定上游发布 evidence-bound settled-daily provenance/batch contract，并在
同一 T0 admission 中取得 exact lifecycle evidence 后，才可增加 Pending → operator
confirmation → exact re-admission。此 blocker 不影响形态自身计算后超过 ±20% 时
继续返回 `manual_confirmation_required`。

## 4. 失败模式

| 失败 | 处理 |
|---|---|
| Magic TDX transport/protocol 失败 | `GatewayError::Unavailable`，可重试，不写缓存 |
| 空批次、部分 rejection、身份或批次不一致 | `invalid_evidence`，整批拒绝 |
| source time 超过 5 秒或来自未来 | `invalid_evidence`，整批拒绝 |
| 缺少今日已完成五分钟线 | `invalid_evidence`，不使用历史日替代 |
| 缺字段/坏价格/OHLC 冲突 | `invalid_evidence`，不补零、不估算 |
| T0 settled-daily 相邻变化超过 ±20% | `manual_confirmation_contract_unavailable`；当前上游缺 exact 日线 batch/lifecycle 证据，告警并失败关闭 |
| 形态自身计算后日内变化超过 ±20% | `manual_confirmation_required`，不得进入计算 |
| blocking worker panic/cancel | `blocking_task_failed` 并写 BR-159 审计 |
| BR-159 审计不可用 | 整体失败，不把未审计数据交给消费者 |

## 5. 旧模块关系

| 模块 | 决定 | 原因 |
|---|---|---|
| `MagicTdxGateway::get_t0_evidence_batch` | adopt | 已提供同批昨收、实时 OHLC、五分钟线和严格 5 秒门 |
| `MarketCapabilitiesGateway::minute_data` | reject for this join | 分钟线合同不含昨收；与其他 quote route 拼接会跨 provider/batch |
| 已删除 Eastmoney intraday HTTP | reject | consumer-owned transport 且无法满足同批证据 |
| 旧形态分类函数 | adopt semantics only | 纯计算规则可保留，获取与来源拼接不可恢复 |

## 6. 验证与发布

- focused unit tests：同批成功、部分批次、身份/批次错配、过期、坏价、
  `manual_confirmation_required`、尾盘锚点缺失。
- `rustfmt --check`（本次文件）。
- `git diff --check`。
- 根任务统一执行：
  - `cargo fmt --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test`
  - `bash tools/compliance/check.sh`

## 7. 回滚

回滚本设计对应的独立提交。回滚后生产形态能力恢复显式 unavailable；不得
恢复旧 HTTP acquisition、跨源拼接或默认形态。
