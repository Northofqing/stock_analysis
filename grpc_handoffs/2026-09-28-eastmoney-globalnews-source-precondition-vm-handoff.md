# Eastmoney GlobalNews 来源准入修复与部署交接（2026-09-28）

## 原始故障与定位

初始诊断时，R-08 服务在 VM `10.211.55.3:50051` 以 PID `18008`、提交 `69c6ef19` 运行。本机 monitor PID `23247` 的 GlobalNews 请求在 VM `grpc-server.stderr.log` 的 2026-09-28 09:55–09:57 CST 记录为 `stage=source_precondition_failed`；例如 request ID `1790560547846-23247-3873` 对应本机约 09:55:48 的 Eastmoney 失败审计。服务端该路径以 `FailedPrecondition` 和 `Unadmitted` 拒绝，彼时不能证明新闻数据合格。

本机原 `KnownReasonCode` 未登记 `source_precondition_failed`，因此 `safe_wire_reason_code` 把真实原因折叠成 `internal`，误导了 `news_flash_source_failure_v1` 审计和 opening 探针。已在本仓提交 `d9f80181` 保留该安全原因码；定向测试覆盖 ExternalV1 错误 wire → Gateway → NewsFlash，确认仍为 `Unadmitted`、无事件产出。该提交只修正诊断，不将 Eastmoney 数据放行。新本地 release 与运行验证另见部署记录。

VM 直接请求 `https://roll.eastmoney.com/finance.html` 时观察到 HTTP 200、UTF-8 与 40 条 finance 行；先前的抽样没有覆盖完整页中所有文章域名，不能代替真实 gRPC 验收。

2026-09-28 12:35 CST 从本机使用正式 mTLS + Bearer bundle、`GlobalNews` v2 `preferred_provider=Eastmoney`、`{"limit":1}` 发送只读请求 `codex-eastmoney-diagnostic-1790570107`。VM 返回 `FailedPrecondition`，原始状态经令牌/URL 脱敏后为：`Eastmoney protocol error: Eastmoney news article host "bank.eastmoney.com" is not an admitted global-news host`。VM `crates/magic-eastmoney-rs/src/news.rs::normalize_global_article_url` 的精确 host 集合确实缺少该域名。东方财富公开的[银行频道](https://bank.eastmoney.com/)使用此域名；这证明其为一方域名，不自动证明任意路径、分页或该批次其余记录合格。

## VM 修复、回归与运行回执

VM Codex 在隔离分支 `fix/eastmoney-bank-host-20260928` 修复了两个由真实滚动页暴露的精确频道域名。第一版 `19bb682` 加入 `bank.eastmoney.com`，同源构建提交 `cc2d4500` 部署后，`limit=20` 曾返回 `ADMITTED`、20 条，其中 2 条来自银行频道。但动态页面随后出现[东方财富外汇频道](https://forex.eastmoney.com/)的文章链接。本机请求 `codex-eastmoney-postdeploy-1790591612` 再次得到 `FailedPrecondition`，原因为 `forex.eastmoney.com` 未在精确 host 集合中，因此没有把银行频道的一次成功当作整条链路完成。

第二版提交 `4e4995f8d3f2c7cd504d1dec0f238e6d4b4fc02c` 加入精确的 `forex.eastmoney.com`。服务端回归先以完整页链接复现同一句拒绝错误，再验证合法链接放行、相似域和恶意子域、错误路径继续拒绝；新闻相关 14 项通过。VM 报告全工作区测试、Clippy、格式、文档构建、合规和文档链接检查均退出 0。该分支工作树干净，未推送 GitHub、未合并主分支。

VM 运行中的 gRPC PID `384` 监听 `10.211.55.3:50051`，TDX agent PID `3300`。`GetHealth` 返回 `live=true`、`ready=true`，版本 `0.2.0`、提交 SHA、descriptor SHA `0c4485545dbfd0979a7d5ea206c840f39fd504ed62fb7eef92f1940bdc9c2f41`、exe SHA-256 `517e0b4c31bb42330f4bc2a0395e3af385a164212414a65775bed40c9eb87ae3` 均与实际运行制品和 `2026-09-28.2` 公开 bundle 匹配。bundle manifest 9/9 通过，descriptor 与上一版相同。VM 连续四次正式 mTLS + Bearer、`limit=20` 请求均为 `ADMITTED`、`complete=true`、20 条，包含 `forex.eastmoney.com`；证据请求 `eastmoney-forex-evidence-20260928-1` 的外汇记录 `source_at=2026-09-28 18:36`，记录和批次的 `batch_id` 一致。本机独立请求 `codex-eastmoney-postdeploy-1790592991` 也返回 `ADMITTED`、完整 20 条，其中 `finance.eastmoney.com` 19 条、`forex.eastmoney.com` 1 条，`source_at=18:47`。

VM 停机脚本初次由普通权限运行时读取进程父子关系被 Windows 拒绝，指定 PID 的 agent 也无法由该权限停止；按已授权范围提升权限后完成切换。上一版 exe 与公开 bundle 保存在 VM `target/runtime/archive/eastmoney-forex-predeploy-20260928`，旧 exe SHA-256 为 `9a755009a46be608025fcfbe05ab81ac48dad7394dde8a5971a640cab7d70bf4`。

## 本仓接收与复验

本仓 `client-bundle` 从 VM 运行根只复制六个变更的公开文件，认证资料保持在原位；本仓与生产运行根的 manifest 均 9/9 通过。`src/grpc_client/build_identity.rs` 将 bundle 身份编入本机二进制，所以从 Desktop 外生产运行根重建 monitor（SHA-256 `a2589f0115d6f3ee89bf11d7714c7bd4bce58f6d3e5bcbc12b06831ffc5b8b34`）与探针（SHA-256 `3341470560daf9ee52fb0c046b74f91d93d01396f3c9b61252cc18cfc745e98e`）。没有复制仓库尚未部署的 D10 源码；bundle 不在 selection activation 的配置哈希输入中，旧 activation `ba4087db…` 保留。

新探针的 `--opening` 返回 Health `deployment_build_identity=matched`，Eastmoney、CLS、Jin10、ThePaper 四家各有真实 `ADMITTED` 新闻记录，静态路由 `opening_static_ready=true`、`attempts=9/9`、`failed_routes=none`。R-08 计划交割日定向探针仍返回 2026-09 四条 `ADMITTED`，`confirmed_delivery=false`。本机只重启 monitor，`gui/501/com.stockanalysis.monitor` 新 PID `17089`、单实例运行且 cwd 为生产根，本地桥接 PID `56417` 未重启。19:12:03 CST 主库初始化完成，19:12:06 本地 gRPC 桥已连接，19:12:10 monitor 的 `GlobalNews-Eastmoney` 审计为 `outcome=available`、`accepted=20`、`rejected=0`，19:12:22 的 OpeningReadiness 记录了 Eastmoney 静态路由。其他来源和 D08/D10/D14、确认型 R-08 的剩余事实仍按各自合同验收，不能由本次 Eastmoney 成功代替。
