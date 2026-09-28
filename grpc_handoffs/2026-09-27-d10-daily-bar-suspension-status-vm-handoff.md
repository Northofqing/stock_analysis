# D10：日线停牌状态与预测目标日结算，上游合同交接（2026-09-27）

## 状态与目标

**状态：上游事实合同待交付；本地常规日线写入尚不能证明 `is_suspended=0` 表示交易正常。** 本交接补充 [D17 / D20 权威交易事实交接](2026-09-27-d17-d20-authority-trading-facts-vm-handoff.md)，聚焦 D10 预测对冻结 `target_date` 的精确收盘价结算。请 `magic-market-data-rs` 先证明每个证券、日期的 Trading / Suspended 状态及来源覆盖，再给出版本化 wire 合同、真实 RPC 与部署回执。既有 v1 `HistoricalBars` 成功批次不提供这项证明。

## 本地已核实的事实漏斗

2026-09-28 本仓新增 `5fd99ff2`：预测起始日和目标日都要求独立资格表中的 `Trading`，缺行按 `Unknown` 保持 pending；旧默认 `is_suspended=0` 不再自动放行。8 项相关 lib 测试通过。这是**未部署的本地防误结算门禁**，没有上游状态写入适配；资格表暂沿用既有 `(code,date)` 键，仍待 VM 合同后补齐交易所、资产类别身份，不可当作 D10 已关闭。

| 环节 | 当前实现 | 结论 |
| --- | --- | --- |
| gRPC 日线视图 | `src/data_gateway/grpc_source/convert.rs::historical_bars` 接受 `code/date/OHLCV/amount/pct_chg/settled`，没有停复牌字段或逐日覆盖证据；转换时将 `KlineData.is_suspended` 固定为 `false`。 | 此 `false` 是本地默认值，不是源证明的 Trading。`settled=true` 仅是价格结算状态，不能代替交易状态。 |
| Gateway 准入 | `src/data_gateway/historical_bars.rs::AdmittedDailyBars` 绑定来源批次和已准入日线，没有每个交易日的停复牌权威状态；`review_bar_fact` 要求正数 volume/amount。 | 一段无日线的日期不能从“缺行”推出停牌；携带旧 close 的行也不能从正数价格推出 Trading。 |
| SQLite 持久化 | `src/database/kline.rs::persist_validated_kline_data` 只把 OHLCV 等字段传给 `upsert_daily_record`；公开的 `save_daily_record` / `save_daily_batch` 同样无停牌实参。`NewStockDaily` 与 `src/schema.rs` 未映射该列；`src/database/mod.rs` 创建/增量添加的 `stock_daily.is_suspended` 是 `NOT NULL DEFAULT 0`。 | 新插入行默认 0；UPSERT 没有覆盖此列，旧值可能保留。二者都不能表明当前批次证明该行正常交易。 |
| 读库与预测 | `src/database/repository.rs::stock_daily_to_kline` 再将 `is_suspended` 固定为 `false`。`src/monitor/prediction_verifier.rs::read_exact_close` 已只读取 `is_suspended=0` 的精确目标日 close。 | 该预测修复**只保护已显式标记为停牌的行**；常规写入默认 0 仍会把状态未知的陈旧价格当作可验证收盘价。D10 停牌闭环尚未完成，也不能称当前路径全面 fail-closed。 |

本地回归用例用明确设置的 `is_suspended=1` 行证明预测会保持 pending；它不证明生产 gRPC 会提供或持久化此事实。请勿把该测试或 688277 日线断档当作上游已交付证明。

## VM 需交付的版本化事实合同

1. **来源和语义。** 指出能对 `(交易所, 六位代码, 资产类别, 日期)` 证明 `Trading`、`Suspended` 的权威来源、适用市场和历史覆盖范围。`Unknown`、源不可达、缺页、仅有行情缺行、仅有上一日 close、仅有 `settled`，都不得编码为 `Trading`。若只能证明停牌事件而不能证明某日无事件的完整查询覆盖，应如实返回 unavailable。
2. **wire 形状与身份。** 选择并公布版本化方案：扩展 `HistoricalBars` 记录并提供停牌期无 bar 日的覆盖事实，或发布独立的资格化停复牌查询供下游按证券和日期精确绑定。VM 给出实际 RPC method、operation/schema/version、完整请求与响应 JSON/descriptor、旧版本兼容或显式拒绝策略。不得在原 v1 不兼容语义下静默增补可选 bool，再让缺字段落回 `false`。
3. **逐日证据与覆盖。** 每个状态绑定原请求身份、有效日期、状态、来源、`source_at`、`observed_at`、batch ID、源事件/公告稳定 ID 和原件摘要（若为停牌/复牌事件）；来源不发布 `source_at` 时明确标记不可得，不能用 `observed_at` 代填。对 `Trading` 明示“查询覆盖了该日期且无有效停牌事件”的依据、分页完整性和覆盖区间；对 `Suspended` 明示生效起止、是否包含当天及复牌首日边界。不同事实来自不同来源时分别保留来源与时效。具体字段名和枚举值由 VM 合同确定，本交接不预造 wire 字段。
4. **冲突及失败。** 身份/日期不符、重复或相互冲突的状态、仅部分覆盖、无法证明无停牌、源时间异常、权限不足和上游超时，都须返回可区分的 typed unavailable/conflict，不产出已资格化 `Trading`。实际停牌日没有 bar 时，响应也要能独立证明该日停牌；不能要求伪造一根零量或沿用旧价的 bar。

## 可执行 fixture 与真实 RPC 验收

VM 应在交付的版本化请求示例上执行以下案例，保存脱敏原始请求/响应、解码结果及所用权威原件。688277 是[既有停牌线索](share/2026-09-24-688277-suspension-gap.md)，在本轮仍须重新核验。

| 案例 | 输入与所需结果 |
| --- | --- |
| F1 停牌无 bar | 688277，请求覆盖 2026-07-15 至 07-30：逐个交易日核对 07-16 至 07-29 的停牌覆盖，并确认 07-30 复牌后的 Trading。缺行不能是唯一证据；对照 688561 同区间，不得继承 688277 的状态。 |
| F2 陈旧价格反例 | 寻找真实来源中“停牌日仍返回携带旧 close 的日线行”的样本，若存在，保存原始数据与权威停牌原件并要求结果为 `Suspended`；若不存在，明确报告不存在，并用受控服务端 fixture 注入该组合，证明不会产出 `Trading`。不要制造一条记录并宣称它是真实市场样本。 |
| F3 明确正常与未知 | 选择有权威全覆盖的正常交易日，返回可核对的 `Trading`；同证券分别制造覆盖截止日前后、部分页、空结果无覆盖、上游错误。只有全覆盖正常日可为 `Trading`，其余为 unavailable。 |
| F4 身份和边界 | 注入请求 A 返回 B、混合 A+B、重复 A、复牌日仍落停牌区间和相互冲突公告，全部 fail closed，不能把响应中的一条行情随意贴到 A 的目标日期。 |
| F5 部署后实测 | 对 VM 当前监听服务执行一次合同内真实 RPC；记录 Health build identity、服务 commit、运行 PID、binary hash、bundle/descriptor hash、请求 ID、原始响应及服务日志/trace。用完全相同的请求重放 F1 和 F3 中至少一例。 |

回执应列出每个 fixture 的实际结果、可交付与不可交付的市场/日期、定向测试命令及结果、合同包位置与 hash、服务端提交/构建/部署身份。若权威来源尚不存在，请明确标为 unavailable；不要为完成接口而推断停牌或默认正常。

## 本仓收到回执后的接线与复验

1. 核对 VM 源码 commit、公开 bundle/descriptor、Health 和运行 binary 同一身份，复跑上述真实 RPC，并逐字段比对原始证据。旧 v1 响应和缺状态响应不得进入已资格化状态分支。
2. 将状态和覆盖证据在 Gateway admission 中与证券/日期绑定；选择明确的三态持久化及历史行迁移策略，使未知状态不再借 SQLite 默认 0 冒充 Trading。随后更新 `NewStockDaily`、写入/UPSERT、`StockDaily` 读库和预测精确 close 查询；已显式停牌的旧行不能被后续无状态批次冲掉。
3. 用隔离库验证：起始日和目标日均须有资格化 Trading 与精确当日 close 才可结算；停牌目标日即使有旧 close 仍 pending；缺状态/部分覆盖/旧合同仍 pending；复牌首日仅在权威 Trading 与精确当日 close 均成立时结算；重复写入保持状态和证据一致。按实际影响扩大到相关定向测试与本地真实 RPC，不凭测试 fixture 宣称生产完成。

当前交接记录上游缺口和验收条件。本地已新增资格表与预测读门，**没有上游状态的生产写入，也没有部署该源码**。
