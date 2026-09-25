# 持久宏观执行器的 EconomicCalendar 退役

目标：`chain_post_close` 的 `Gateway(5)` 在新执行及重启恢复后都不发送已退役的 Local `EconomicCalendar` RPC；保留历史请求、终态、审计和哈希链的原义，并写入可校验的 `operation_retired`、`NotCalled` 终态。新 `EconomicReleaseObservations` / `Schedule` 仍使用独立身份，不能借旧槽位。

## 当前事实

- `Durable::admit(Step::Data { query: Gateway(5) })` 会经 `AuthorizedMacroAttempt::execute` 到 Local 旧 RPC。普通 `GrpcMarketClient::query` 的退役门不覆盖此路径。
- v12/v13 `chain_post_close_macro_query_terminals` 的 `cause_kind` 固定 `CHECK`；`LocalRouteUnavailable` 只对真实 `ObservedUnavailable` 计划有效。Rust 单独加 cause 或把已连接路线伪装成断连都不成立。
- 其他表以 `(intent_id,run_version)` 外键引用终态表。SQLite 最小试验表明只开 `defer_foreign_keys` 换表后 `COMMIT` 仍报外键失败；在迁移事务前暂关外键、事务内换表与 `foreign_key_check`、提交后恢复外键的试验可行。正式方案还须满足本仓的冻结 DDL、catalog attestation 和旧行校验。
- 当前工作树最新布局是 13；v13 的 Macro 写入 guard 限定最新布局=13。新布局除了终态表还须重新封印相关 guard。

## 实施阶段

1. **冻结迁移规格**：列出所有指向终态表的外键、v13 Macro guard、v14 新 cause 的 SQL 形状与恢复规则；优先使用独立 v14 退役终态表，避免换写旧表及临时关闭外键。用含旧终态和下游引用的数据库夹具验证原子升级及回滚。
2. **v14 schema 与读写**：增加冻结 v14 DDL、digest/catalog 注册、升级/校验入口；新 cause `OperationRetired` 限定 `Gateway(5)`、无新 attempt、`NotCalled`，允许旧计划已有 request plan 但不把它当作新调用，保留全部旧行。`NativeOutcome::Economic` 由确定性 `GatewayError::retired_operation` 重建，审计理由一致。
3. **执行器**：新计划及可恢复旧计划在 `Gateway(5)` 入队时，先写退役终态再返回本地决策；若旧计划已有 attempt/result/terminal，则按原事实继续恢复，不改写。其他 Gateway 与 Web 查询行为不变。
4. **验证**：先用包含旧引用行的 v13 数据库跑 v14 升级、旧哈希/审计不变、拒绝篡改和回滚；再跑真实 Local loopback 的无旧 RPC、新终态、断电重开与其他四源继续执行。按 AGENTS.md 限定 Cargo target，必要时扩到跨模块验证。

完成条件：新/旧可继续执行的 v14 run 均不会发旧 RPC；退役记录在 SQL、Rust 恢复、审计和报告中一致；升级已有 v13 库不丢数据，历史事实仍可读；定向验证通过。未满足前保持本计划进行中，不把普通客户端退役门算作持久执行器完成。
