# R-08 FuturesDelivery 月份意图未进入 gRPC 请求（2026-09-25）

给上游数据／协议维护者核对的合同问题。这里仅有本地下游源码证据；没有把 9 月 22 日的 `internal` 失败归因于这个请求形状，也没有本次真实 RPC 的服务端记录。

## 可复核的本地事实

- R-08 调用 `FuturesDeliveryGateway::cffex_contract_month(reminder_year, reminder_month)`（`src/bin/monitor/push_templates.rs`）。
- Gateway 把 `year:month` 写入本地 `acquisition_request_hash`，随后调用无参 `GrpcSource::futures_delivery_async()`（`src/data_gateway/futures_delivery.rs`）。
- `futures_delivery_async()` 发出的 LocalBridgeV1 operation 为 `FuturesDelivery`，业务 JSON 为 `{}`（`src/data_gateway/grpc_source.rs`）。因此本地审计所称的月份没有进入 wire 请求。
- R-08 消费端随后按 `reminder_date` 选交割事件；这个本地过滤不能证明服务端曾收到月份选择条件。
- 公开的 `client-bundle/grpc-external-api.md` 只给出 `FuturesDeliveryRequest` schema 名，未在本仓库冻结其字段或“全日历返回”语义。文档称当前服务使用 CFFEX 官方固定交割日历，不能据此推断 `{}` 是否是正确请求。

## 请上游确认

1. `FuturesDeliveryRequest` v1 的允许字段、默认范围、时区和空请求语义是什么？是否支持按 `year/month` 或明确日期范围查询？请给一条规范请求及其响应证据。
2. 若 `{}` 本来就表示完整官方交割日历，请明确完整性、覆盖年限和 `VerifiedEmpty` 的含义；本地将把审计身份拆成实际 wire 请求与后续月份投影，不再把月份写成已传给服务端的过滤条件。
3. 若服务端支持月份范围，请提供字段名与边界语义；本地再发送同一范围并以 `request_id` 对齐一条真实批次，验证返回日期及来源证据。

在合同未明确前，本地不猜字段、不将 9 月 22 日 `FuturesDelivery` 的 `internal` 失败解释为月份参数造成，也不放宽 R-08 对目标交割日和来源证据的校验。
