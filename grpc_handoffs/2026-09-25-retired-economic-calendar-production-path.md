# 退役 EconomicCalendar 的生产宏观搜索调用（2026-09-25）

给上游数据／协议维护者及本项目后续 S5 接线者的交接。本文件基于仓内公开合同和本地调用链；本次没有声称取得服务端请求日志或新产品真实批次。

## 问题与本地处理

- `client-bundle/grpc-external-api.md` 明确 `EconomicCalendar` 当前未准入；已发布观测是 `EconomicReleaseObservations`，未来日期级日程是 `EconomicReleaseSchedule`。两者语义和 Provider 身份不同，不能把旧 operation 改名后继续解释旧数据。
- 现有生产链 `monitor` → chain preparation → `SearchService` → `macro_news::legacy` → `runner` 仍包含 `Gateway(5)`；该槽的持久/审计身份是 `EconomicCalendar`，旧 Local 请求载荷为 `{}`。调用链之前可到达 `data.economic_calendar(req)`。
- 下游在 `Legacy::open` 为 `Gateway(5)` 放入 `operation_retired`、不可重试的不可用终态，并按原审计槽结算。宏观报告显示该原因，其他新闻源继续独立查询；旧 RPC 不再由这条生产宏观搜索路径发出。没有改写历史 `EconomicCalendar` identity、SQL 约束或既有批次。
- `EconomicCalendarGateway::latest_releases` 与 `GrpcSource::economic_calendar_async` 的直接入口也返回不可重试的 `operation_retired`，不再打开 Local 桥或发送旧 RPC；Gateway 仍使用原能力名与请求散列写审计失败记录。
- 普通 `GrpcMarketClient::query(EconomicCalendar, ...)` 在构造业务请求前返回带 `Jin10`、`operation_retired`、`retryable=false` 的 `Unimplemented`，即使调用者绕过 Gateway 也不触发旧 RPC。该门不适用于独立持久执行器的 `AuthorizedMacroAttempt`，不能据此宣称它已关闭。
- `chain_post_close` 的独立持久宏观执行器现有显式 v14 入口：连接的 Local 路线写入独立 `OperationRetired` / `NotCalled` 终态，不生成新 `Gateway(5)` 请求计划，也不向旧 RPC 发请求；四个新闻源继续执行。旧计划如已有 Gateway(5) attempt，则保持失败关闭，不把可能已发生的调用记成 `NotCalled`。旧 v12/v13 数据库仍需显式迁移，不能因代码存在而声称所有存量执行器已经切换。
- v12/v13 的 `chain_post_close_macro_query_terminals.cause_kind` 有固定 `CHECK`，其他终态表以 `(intent_id,run_version)` 外键引用它。v14 因此新增独立退役终态表，保留旧表与旧行，不关闭外键；SQL guard、审计及 Rust 恢复共同校验退役事实。
- 定向验证：`cargo test --lib retired_economic_calendar_does_not_initialize_transport` 通过（2026-09-25）；它证明 `GrpcSource` 的旧入口返回退役错误时没有初始化 Local 连接，不覆盖持久执行器的重放路径。
- 普通客户端与映射分别经 `cargo test --lib retired_economic_calendar_query_is_rejected_before_wire_io`、`cargo test --lib map_query_error_preserves_retired_economic_calendar_reason` 通过（2026-09-25）；前者在无服务通道上立即返回，后者保留退役分类。
- v14 三条定向测试通过（2026-09-25）：catalog 封印、空库迁移、真实 Local loopback 无旧 RPC 与重开恢复。真实 v12 五 Gateway 终态经 v13→v14 迁移后，旧终态、StageFinal、审计链逐行保持，外键检查为零；未来版本拒绝测试也通过。2026-09-26 的 v13 已连接旧计划只有 Gateway(5) request plan、无 attempt 的 v14 续跑也通过，旧请求事实保留且旧 RPC 零调用。另一份真实 v12 Web pace 库含 dimension 外键引用行，其一致性副本升级 v14 后旧事实、审计链和外键关联保持不变。v14 Models/Search/Report 完整链与重开无远程重放通过，v13 共用测试复测通过。`BusinessIntentStore::chain_post_close()` 当前仍拒绝生产调用，未接自动迁移。v12 完整成功用例在本机触及固定 15 秒预算，不能作为通过证据；v12 五 Gateway 已确认后的预算到期用例通过。

## 后续产品接线边界

当前源码已有三个新产品的独立 Gateway/`GrpcSource` 入口。`gateway_queries_qualified_external_auction_without_local_61_collision`、`gateway_sends_limit_country_through_qualified_external_62`、`gateway_sends_range_through_qualified_external_63` 三条定向真实 loopback 用例于 2026-09-25 通过；它们验证客户端请求、能力门和响应转换，不是生产宏观消费端或真实服务批次验收。

1. 已发布数据应按 `EconomicReleaseObservations` 的 Jin10 观测合同接线；未来发布日程应按 `EconomicReleaseSchedule` 的 FRED 日期级合同接线。先冻结各自请求、响应、来源时间及完整性，再决定哪个消费端需要哪一类数据。
2. 新产品应有新的 acquisition identity 与审计批次，不得借 `Gateway(5)` 的旧 `EconomicCalendar` 名称覆盖历史行。历史数据保持原义；切换消费端时定义版本化投影。
3. 真实验收需分别保存 request_id、profile、规范业务 payload、Health/Capabilities 构建身份、响应 envelope 与原始来源证据。`ready=true` 和空结果都不足以证明新产品已可用于业务判断。

原先退役调用的改动只关闭上述现有生产入口；S5 三个新 ExternalV1 产品虽已有 Gateway 入口，尚未接入生产宏观消费端。不能把宏观报告里的 `operation_retired` 当成新产品 `VerifiedEmpty`。
