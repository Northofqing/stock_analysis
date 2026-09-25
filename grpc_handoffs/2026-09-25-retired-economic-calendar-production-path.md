# 退役 EconomicCalendar 的生产宏观搜索调用（2026-09-25）

给上游数据／协议维护者及本项目后续 S5 接线者的交接。本文件基于仓内公开合同和本地调用链；本次没有声称取得服务端请求日志或新产品真实批次。

## 问题与本地处理

- `client-bundle/grpc-external-api.md` 明确 `EconomicCalendar` 当前未准入；已发布观测是 `EconomicReleaseObservations`，未来日期级日程是 `EconomicReleaseSchedule`。两者语义和 Provider 身份不同，不能把旧 operation 改名后继续解释旧数据。
- 现有生产链 `monitor` → chain preparation → `SearchService` → `macro_news::legacy` → `runner` 仍包含 `Gateway(5)`；该槽的持久/审计身份是 `EconomicCalendar`，旧 Local 请求载荷为 `{}`。调用链之前可到达 `data.economic_calendar(req)`。
- 下游在 `Legacy::open` 为 `Gateway(5)` 放入 `operation_retired`、不可重试的不可用终态，并按原审计槽结算。宏观报告显示该原因，其他新闻源继续独立查询；旧 RPC 不再由这条生产宏观搜索路径发出。没有改写历史 `EconomicCalendar` identity、SQL 约束或既有批次。
- `chain_post_close` 的独立持久宏观执行器和 `EconomicCalendarGateway` 尚有旧身份/调用代码；本次未将其误报为已经迁移。启用这些路径前，须另行迁移或加入相同门。

## 后续产品接线边界

当前源码已有三个新产品的独立 Gateway/`GrpcSource` 入口。`gateway_queries_qualified_external_auction_without_local_61_collision`、`gateway_sends_limit_country_through_qualified_external_62`、`gateway_sends_range_through_qualified_external_63` 三条定向真实 loopback 用例于 2026-09-25 通过；它们验证客户端请求、能力门和响应转换，不是生产宏观消费端或真实服务批次验收。

1. 已发布数据应按 `EconomicReleaseObservations` 的 Jin10 观测合同接线；未来发布日程应按 `EconomicReleaseSchedule` 的 FRED 日期级合同接线。先冻结各自请求、响应、来源时间及完整性，再决定哪个消费端需要哪一类数据。
2. 新产品应有新的 acquisition identity 与审计批次，不得借 `Gateway(5)` 的旧 `EconomicCalendar` 名称覆盖历史行。历史数据保持原义；切换消费端时定义版本化投影。
3. 真实验收需分别保存 request_id、profile、规范业务 payload、Health/Capabilities 构建身份、响应 envelope 与原始来源证据。`ready=true` 和空结果都不足以证明新产品已可用于业务判断。

原先退役调用的改动只关闭上述现有生产入口；S5 三个新 ExternalV1 产品虽已有 Gateway 入口，尚未接入生产宏观消费端。不能把宏观报告里的 `operation_retired` 当成新产品 `VerifiedEmpty`。
