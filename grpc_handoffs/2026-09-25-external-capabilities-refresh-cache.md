# ExternalV1 Capabilities 刷新后状态目录仍使用旧缓存（2026-09-25）

性质：本地 gRPC 客户端数据解释缺陷，不是服务端返回格式异常，也不是 TDX 上游故障。影响 `GrpcSource` 缓存命中后的 ExternalV1 查询错误状态：本次资格检查拿到新 Capabilities，但后续数据调用克隆的共享客户端仍持有旧 provider catalog。

## 复现与处理

- `GrpcSource::ensure_external_connected` 缓存命中时克隆 `GrpcMarketClient`，在克隆上重新执行 Health、Capabilities 和方法准入，然后直接返回。`GrpcMarketClient::get_external_capabilities` 只更新这个克隆持有的 `external_provider_catalog`；`query_external_op` 再从 `external_client` 取另一个克隆，因此 provider attempts 仍按旧目录解释。
- 新的 mTLS loopback 回归让首次有效目录只发布 Eastmoney，经历一次 Unavailable 和一次错误 request ID 的刷新后，发布仅含 Cailianpress 的新目录。修复前，成功刷新后查询 Cailianpress 的完整 provider attempts 仍被判为未支持；定向测试在该断言处失败。夹具使错误状态的 provider 跟随请求，避免按调用序号构造响应造成假阳性。
- 修复在 Health、Capabilities、目标方法准入全部成功后，将更新了目录的客户端写回共享缓存。刷新失败时不替换缓存，本次查询继续由资格检查返回错误；成功刷新后再读取的客户端使用新目录。

## 验证边界

- `external_cached_capabilities_refresh_updates_status_catalog`：先在 Cailianpress 被误判为未支持的断言处失败；修复后验证首次 Eastmoney 可解释、失败刷新不放行、Cailianpress 成功刷新后可解释且旧 Eastmoney 不再被接受。
- 此处只处理在线 `GrpcSource` 缓存中 provider catalog 的更新。Macro durable 重连 build identity 的独立问题仍见 [交接记录](2026-09-25-external-reconnect-build-identity-gap.md)。
