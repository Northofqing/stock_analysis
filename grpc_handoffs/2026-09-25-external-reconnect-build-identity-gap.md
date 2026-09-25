# ExternalV1 重连后的 build identity 资格缺口（2026-09-25）

性质：客户端持久化恢复与连接资格问题；目前是代码路径审计发现，尚未用服务端 A/B 切换夹具复现。不是 TDX 上游数据故障。关联 [S6 实施计划](2026-09-16-client-bundle-implementation-plan.md#s6迁移-durable-接缝清除-external-旧类型入口) 与 [本地响应恢复记录](2026-09-25-external-v1-durable-response-recovery.md)。

## 现有证据

- Macro 的 V3 Health 结果从实际合格响应保存 `VerifiedBuildIdentity`；V3 Capabilities 结果从此前已确认的同 endpoint/authority Health 响应复制该身份。`CapabilitiesResponse` 自身没有 build identity，`ControlRawResult::bind_external_identity` 也没有查询当前连接的服务端构建身份。
- v11 驱动和 v12 驱动在一次运行内保留 Health 完成后的 `GrpcMarketClient`，可将它交给 Capabilities；进程重启时这个内存对象消失。v11 的 `connected = None` 以及 v12 的 `connected_external = None` 会使后续 Capabilities 或数据 attempt 经 `PreparedExternalEndpoint::connect_once` 在相同 URI 上新建连接。该连接只记录 endpoint/profile/authority，没有 Health build identity 校验。即使不重启，传入的 tonic channel 也可能在底层重连；现有代码没有针对物理连接变化的构建身份证明。
- 若 Health 在服务端构建 A 上确认，之后同一 URI 切到构建 B，旧 Health 身份可能继续授权 B 的 Capabilities 或数据 effect。V3 持久化格式能够证明“历史 Health 来自 A”，不能单凭 endpoint 字符串证明“新连接仍是 A”。这里尚未声称已经观察到生产数据串线。
- 现有 v11 回归 `single_user_external_macro_confirmed_health_reopens_and_continues_only_original_capabilities` 明确要求重启后继续原 Capabilities。单纯在缺内存连接时拒绝该调用，会改变既有恢复语义；若要保留这项行为，必须先设计并记录独立的新连接资格 effect，再调整 v11/v12 的恢复驱动和验收测试。

## 修复边界与验收

1. 先以 A 的 Health V3 Ready 为已确认事实，关闭原连接并在原 URI 提供构建身份不同、但合同格式仍合格的 B；通过真实数据库重开覆盖 v11/v12 在 Capabilities 前、Capabilities 后及数据重试前的恢复。断言原计划、请求 ID、期限、已确认事实不变，未经新的身份资格证明时 B 上的 Capabilities/数据请求为零。
2. 不能把旧 Health 请求重发并覆盖原结果，也不能把 Capabilities 的旧响应或 endpoint/authority 相同当成 B 的身份证明。若协议允许新的资格检查，应成为独立、持久化、可审计的新 effect，并明确与后续 Capabilities/数据 effect 的依赖；否则停在待准入状态。还需处理逻辑 channel 底层自动重连。
3. 通过 A/B 切换回归后，再更新 [External durable 响应恢复记录](2026-09-25-external-v1-durable-response-recovery.md) 的 S6 边界。目前已有的 v12 Health 回执取消 Unknown 测试只证明未确认 effect 不重发，不覆盖已确认 Health 后的连接变化。
