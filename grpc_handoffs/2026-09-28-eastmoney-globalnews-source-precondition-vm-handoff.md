# Eastmoney GlobalNews `source_precondition_failed` 交接（2026-09-28）

## 当前证据与边界

R-08 服务仍在 VM `10.211.55.3:50051` 运行，部署进程 PID `18008`，服务端提交 `69c6ef19`。本机 monitor PID `23247` 的 GlobalNews 请求在 VM `grpc-server.stderr.log` 的 2026-09-28 09:55–09:57 CST 记录为 `stage=source_precondition_failed`；例如 request ID `1790560547846-23247-3873` 对应本机约 09:55:48 的 Eastmoney 失败审计。服务端该路径以 `FailedPrecondition` 和 `Unadmitted` 拒绝，**尚未证明新闻数据合格**。

本机原 `KnownReasonCode` 未登记 `source_precondition_failed`，因此 `safe_wire_reason_code` 把真实原因折叠成 `internal`，误导了 `news_flash_source_failure_v1` 审计和 opening 探针。已在本仓提交 `d9f80181` 保留该安全原因码；定向测试覆盖 ExternalV1 错误 wire → Gateway → NewsFlash，确认仍为 `Unadmitted`、无事件产出。该提交只修正诊断，不将 Eastmoney 数据放行。新本地 release 与运行验证另见部署记录。

VM 直接请求 `https://roll.eastmoney.com/finance.html` 时观察到 HTTP 200、UTF-8 与 40 条 finance 行；先前的抽样没有覆盖完整页中所有文章域名，不能代替真实 gRPC 验收。

2026-09-28 12:35 CST 从本机使用正式 mTLS + Bearer bundle、`GlobalNews` v2 `preferred_provider=Eastmoney`、`{"limit":1}` 发送只读请求 `codex-eastmoney-diagnostic-1790570107`。VM 返回 `FailedPrecondition`，原始状态经令牌/URL 脱敏后为：`Eastmoney protocol error: Eastmoney news article host "bank.eastmoney.com" is not an admitted global-news host`。VM `crates/magic-eastmoney-rs/src/news.rs::normalize_global_article_url` 的精确 host 集合确实缺少该域名。东方财富公开的[银行频道](https://bank.eastmoney.com/)使用此域名；这证明其为一方域名，不自动证明任意路径、分页或该批次其余记录合格。当前问题的直接拒绝点已定位，服务仍为 Unadmitted。

## VM 待完成

1. 在 VM 的隔离开发工作树核对上述请求 ID、原始页面中 `bank.eastmoney.com` 的 `/a/<numeric-id>.html` 链接和完整页其余行。只在证据符合已冻结路径/时间/分页合同后，将**精确域名**纳入文章 URL 资格集合；保留禁止任意子域、相似域、userinfo、query/fragment 和非数字文章 ID 的拒绝，不用宽松后缀匹配绕过。
2. 修正解析或来源资格化规则时增加能失败的真实样本回归，包含正常页、异常页、时间/URL/身份约束与分页覆盖；保持 `FailedPrecondition + Unadmitted` 对不可资格化输入的拒绝。若 wire 或公开 bundle 变化，发布同源版本化 descriptor 与 manifest；若无变化，说明不变并核对 hash。
3. 在非交易时段构建、部署，并给出 VM commit、构建命令、定向测试、运行 PID、binary SHA-256、Health build identity、公开 bundle/descriptor SHA-256、至少一条部署后真实 Eastmoney GlobalNews RPC 的原始状态、record 数、来源证据、request ID 及服务日志。只有实际 `ADMITTED` 与下游同源复验通过，才可将 Eastmoney 从失败项关闭。

## 本仓接收与复验

收到 VM 回执后先核对运行身份与公开合同，再用 `grpc_bundle_probe --opening` 及同源 GlobalNews 路径复验。对成功响应确认原始记录、来源时间、排序、数量、身份和准入证据；对失败响应确认原因码保真且仍不产出新闻事件。保留当前其他 CLS、Jin10、ThePaper 路由的状态，不用它们的成功替代 Eastmoney 验收。

远程 Codex 控制接口在本次追加诊断消息时未返回确认；此文档是可追溯交接材料，**不代表 VM 已收到或完成修复**。Parallels 命令链路现可读取 VM 项目；VM 工作树中另有未提交的内容发现、iWencai 任务改动，Eastmoney 修复必须隔离，不能把这些改动一起构建部署。
