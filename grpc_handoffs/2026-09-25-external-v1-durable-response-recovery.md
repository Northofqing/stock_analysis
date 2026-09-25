# ExternalV1 历史响应恢复：Local 解码串线（2026-09-25）

性质：本地客户端持久化恢复缺陷，不是上游服务数据异常。关联 [S6 实施计划](2026-09-16-client-bundle-implementation-plan.md#s6迁移-durable-接缝清除-external-旧类型入口)。

## 复现与处理

- `RawResult` V1 已从 Macro 请求计划证明 `ExternalV1 / GlobalNews / request_id`，但恢复响应时仍调用 Local `QueryResponse::decode`。External 响应的空 field 11 在其合同中合法，经 Local 类型重编码却不能保持原 bytes，历史结果被误判为 `SchemaRejected`。新定向用例先失败，失败位置是该 Local 解码分支。
- 现在 V1 在 profile 与方法可证明为 External GlobalNews 时使用原生 External 响应解析；V2 仍先验证保存的 descriptor/wire 证据，再复用同一原生解析。Local V1 保留原有严格解码。旧记录按原版本恢复，没有重编码写回或补造 descriptor 身份。
- 另一处已于 `e43be277` 修复：旧 V1 Health 可保持历史 `Ready` 投影，但缺已验证构建身份时，v11 驱动与持久化 begin 都不能据此追加 Capabilities 或数据 effect。重开数据库的回归确认原请求 ID、期限及已确认事实保留，新增 RPC 为零。

## 验证边界

- `legacy_external_response_with_proven_method_uses_frozen_external_decoder`：先失败后通过。
- `raw_result_v1_canonical_snapshot_rejects_unknown_duplicate_and_noncanonical_protobuf_without_id_drift`、`raw_result_v2_restores_exact_external_payload_with_zero_length_source11`：通过。
- 旧 Health 恢复、正常 V2 控制恢复和 Health 不就绪回归：通过；`git diff --check` 通过。

这仅关闭上述两处本地恢复缺口。S6 的新持久化格式还需显式绑定 profile、typed method、request ID、descriptor 和控制响应的 build identity，并补齐跨 profile/descriptor、未知版本、status 双载体冲突及 Unknown 不重发矩阵。未据此宣称完整 S6 验收或生产部署完成；若后续发现服务端返回的数据本身不合合同，另在本目录记录上游交接。
