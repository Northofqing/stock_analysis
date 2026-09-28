# Eastmoney GlobalNews `source_precondition_failed` 交接（2026-09-28）

## 当前证据与边界

R-08 服务仍在 VM `10.211.55.3:50051` 运行，部署进程 PID `18008`，服务端提交 `69c6ef19`。本机 monitor PID `23247` 的 GlobalNews 请求在 VM `grpc-server.stderr.log` 的 2026-09-28 09:55–09:57 CST 记录为 `stage=source_precondition_failed`；例如 request ID `1790560547846-23247-3873` 对应本机约 09:55:48 的 Eastmoney 失败审计。服务端该路径以 `FailedPrecondition` 和 `Unadmitted` 拒绝，**尚未证明新闻数据合格**。

本机原 `KnownReasonCode` 未登记 `source_precondition_failed`，因此 `safe_wire_reason_code` 把真实原因折叠成 `internal`，误导了 `news_flash_source_failure_v1` 审计和 opening 探针。已在本仓提交 `d9f80181` 保留该安全原因码；定向测试覆盖 ExternalV1 错误 wire → Gateway → NewsFlash，确认仍为 `Unadmitted`、无事件产出。该提交只修正诊断，不将 Eastmoney 数据放行。新本地 release 与运行验证另见部署记录。

VM 直接请求 `https://roll.eastmoney.com/finance.html` 时观察到 HTTP 200、UTF-8 与 40 条 finance 行；容器、分页、标题、URL host/path 和时间顺序的初步检查均可通过。这不能排除服务端更深层的字段、时效、去重或证据约束失败，也不能用页面可访问替代真实 gRPC 验收。当前服务端具体拒绝条件仍待定位。

## VM 待完成

1. 用同一生产 GlobalNews 请求与 request ID 关联服务端完整诊断，定位触发 `source_precondition_failed` 的确切字段、规则和输入样本。保留原始响应摘要及来源时间；不要在日志或交接中输出凭据。若源本身无法满足合同，应明确标记 unavailable，不以空批次或默认值绕过准入。
2. 修正解析或来源资格化规则时增加能失败的真实样本回归，包含正常页、异常页、时间/URL/身份约束与分页覆盖；保持 `FailedPrecondition + Unadmitted` 对不可资格化输入的拒绝。若 wire 或公开 bundle 变化，发布同源版本化 descriptor 与 manifest；若无变化，说明不变并核对 hash。
3. 在非交易时段构建、部署，并给出 VM commit、构建命令、定向测试、运行 PID、binary SHA-256、Health build identity、公开 bundle/descriptor SHA-256、至少一条部署后真实 Eastmoney GlobalNews RPC 的原始状态、record 数、来源证据、request ID 及服务日志。只有实际 `ADMITTED` 与下游同源复验通过，才可将 Eastmoney 从失败项关闭。

## 本仓接收与复验

收到 VM 回执后先核对运行身份与公开合同，再用 `grpc_bundle_probe --opening` 及同源 GlobalNews 路径复验。对成功响应确认原始记录、来源时间、排序、数量、身份和准入证据；对失败响应确认原因码保真且仍不产出新闻事件。保留当前其他 CLS、Jin10、ThePaper 路由的状态，不用它们的成功替代 Eastmoney 验收。

远程 Codex 控制接口在本次追加诊断消息时未返回确认；此文档是可追溯交接材料，**不代表 VM 已收到或完成修复**。
