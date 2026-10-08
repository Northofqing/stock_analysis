# LocalBridge 新闻审计兼容修复（2026-10-08）

用户授权修复数据推送故障。正式桥接旧制品以旧身份算法读取合法的 `news_ai_identity_v3/news_ai_v2` 记录，启动在第 1184 条评估失败，18082 无监听，monitor 行情请求失败。外部 GlobalNews 数据源仍工作。

## 源码与制品

独立旧 provider-host 分支以 `1337bed7b20033ea2ec1e15efc3783f60667cd08` 为基线，回补已上线消费者使用的 `eea99a71b` 新闻身份/冻结恢复/审计校验逻辑，使用等价 legacy provider 类型。修复提交 `e63e2fce7fe12382d42e14332d99cf42369f6f6e` 已推送到 `codex/local-bridge-news-v3-20261008`。该宿主已从当前主线移除；本次不将旧宿主或旧依赖重新合入消费者主线。

HithinkFinance 不在宿主的旧 provider 枚举及既有被接纳新闻来源中；保留旧 provider 集合，未扩展数据来源。只为回补回归增加已锁定的 tempfile 开发依赖。新版 N01 counted 物理投递的测试保留于 monitor 主线，旧宿主未增加该投递种类或启用推送。

Desktop 外构建根 `/Users/zhangzhen/.local/share/stock-analysis-bridge-v3-hotfix-20261008/source`，594 个构建输入与 Git 修复提交逐字节一致。`cargo build --locked --offline --release --bin grpc_market_server --bin grpc_local_readiness_probe --target-dir ../target -j 2` 成功。桥接 SHA-256 `a13ed075def5896cc9226ef99b78a1adc51f2faa9834cd391a73149d3ad913de`，只读行情探针 SHA-256 `cd4164f2b3a11d367a6d678d0f64e5dba6b36e56d9a90b9d173e3e2637f96aed`。

## 验证

- 原制品在独立的原始 NewsAI 表副本上仍报第 1184 条身份错误，退出 1；独立重算 V3 身份与原记录一致，旧算法结果与错误日志一致。
- 定向 `cargo test --locked --offline --lib database::news_ai::tests --target-dir ../target -j 2`：34 通过、0 失败，覆盖旧/V3 混合重开、冻结材料、缺失/篡改/未知格式拒绝、事务回滚及恢复队列。未运行全量测试，未声明当前主线全部平台能力通过。
- 宿主生成的 LocalBridge descriptor 与生产 monitor 冻结 descriptor 完全一致，SHA-256 `2627b0e71d8581f0aff5c147ef6b9987e29b749ac5c99e4ef59d62ae8854d6ca`。
- 新 release 读取包含 1,357 条评估、174 条 V3 恢复快照及完整相关审计链的独立副本，启动通过且未使用 fixture。600396、000001 的 Health、Capabilities、报价、盘口、日线探针均成功。T0Evidence 返回 `time_untrustworthy=true`，不能据此声称可信的可执行 T0 数据。

## 正式切换范围

2026-10-08 10:52 CST，原生产根仅替换独立 `grpc_market_server` 制品，并通过正式 `launchctl load -w` 恢复桥接，PID 97959。生产 monitor PID 3643、制品 SHA `a7376a14cde1f6be3025d6dbbdedf61401c34154f3da928a8e3db30c4950d20b`、741 个源码/配置/构建输入及 activation 字节保持原版。因此沿用现有 monitor activation，未触发消费者源码发布、Schema14 迁移或资金链启用。

主库 dev/ino `16777220/154379673`，持久投递库 `16777220/154266271` 保持原文件；未复制替换生产数据库、修改不可变新闻材料或裁定/重放 Uncertain。旧制品和 activation 原件保存在修复构建根的 rollback 目录。旧制品自身不兼容当前 V3 状态，回退制品不能作为已恢复服务的证据。

## 上线后回读

10:59:15 CST，正式实例完成原主库初始化及 legacy/V3 审计校验，18082 开始非 fixture 监听，原第 1184 条身份错误未再出现。10:59:44 monitor 自动重连；实际接纳持仓报价 5 条、涨停池 39 条及板块资金流 10 条。生产实例的 600396、000001 探针均退出 0，Health、Capabilities、报价、盘口及日线读回成功；T0Evidence 仍明确标记时间不可信。

11:02:07 最终健康回读：monitor 进程、快照与心跳新鲜，数据模式为 `Degraded`，账户模式为 `Frozen`，总健康判定仍是 `unhealthy`。11:00:42 首次观察到数据模式从 `Unsafe` 改善为 `Degraded`；300 秒稳定窗口内通知被既有节流规则跳过，模式已生效，未强制补发或重放。

仍缺 `Kline`、`MoneyFlow`、`News` 的 monitor 数据资格；单证券日线探针成功不等同于 monitor 完整 Kline 资格。MarketMoneyFlows 明确返回 `unsupported_contract`，板块资金流成功也不等同于该契约可用。报价来源仍有暂时失败与熔断，已观察到腾讯恢复接纳，不据此宣称持续稳定。账户指标不完整，最新确认账户为 9 月 28 日、估值为 9 月 30 日；这些日期保持原值。本次没有补齐上游契约、授予 T0 时间可信度或接入实时账户，后续仍按 H08 数据资格范围推进，整体不能称为完全健康。

11:02:08 对上线前原始键集合逐行回读：评估 1,357、评估链 1,357、投递事件 3,861、投递事件链 3,861、投递卡片 417、恢复快照 174 条，六表原有行及精确行哈希全部保持一致；允许运行期间正常追加新审计记录。正式 monitor PID、源码/配置输入、制品、activation 与两库文件身份再次保持原版。最终回执为证据根的 `final-deployment-receipt.json`。

证据根：`/Users/zhangzhen/.local/share/stock-analysis-bridge-v3-hotfix-20261008`。后续发布即使 LocalBridge 协议未改，也需针对现有数据库 codec 验证独立桥接制品的启动兼容性。

## 用户反馈未收到飞书：后续核对

前文 11:02 回读仅为当时的桥接及行情接纳证据，没有验证用户客户端收消息。用户随后明确反馈飞书未收到信息。11:41 健康回读已为 `Frozen/Unsafe`，缺 Quote、Kline、MoneyFlow、News、OrderBook；不能沿用前一快照宣称持续恢复。

通过真实飞书 OpenAPI 只读回读，11:05:49 的 DataMode 与 11:31:18 的 NewsFlashAggregated 消息均存在、未删除，目标匹配现配置的 Stock 机器人私聊。DataMode 三条当日 counted 记录的远端正文与原始 UTF-8 envelope 字节完全一致；反复打印同一 Delivered observer 是历史终态读回，不能算新发送。最新模式通知及聚合消息均查询到一名已读用户，但这不等同于当前用户客户端收到提醒，也不证明此会话符合其预期。没有改收件人、手动发送测试消息或重放旧投递。

独立真实 mTLS/Bearer Health 在 11:47:53 确认：生产 endpoint `127.0.0.1:50052` 与 Windows `10.211.55.3:50051` 均返回新版本 source `841e4ae7a9df62be0c4536fa9009c66089d282bf`、descriptor `abf28a3e0028488a7579da4d961e1a7c1408482bdc0500122c1956d225e480cf`、binary `7407a03b744dc4a0040257c5c8de4cef49ec283d5ddc3529fe4506ef04832a4f`。现 Mac monitor 的编译信任仍为原 `4e4995f8`/`0c448554…`/`517e0b4c…`，新 ExternalV1 新闻和证券身份请求被 `external_connection_unqualified` 拒绝；独立 fresh probe 也拒绝不匹配的 Health。50052 可实际转发，不因 lsof 无本地监听而推断端点离线。

读取另一 Windows 任务“评估项目数据准确性”的直接用户请求“用最新代码 编译重启”，确认新版上游升级得到用户授权。故障是两端版本未协调，不能称为未经授权的候选抢占。Windows 任务“R08 FuturesDelivery 上游合同与部署”已收到精确元组与更正上下文，正在提供新版公开合同及精确绑定。用户随后明确要求保留最新 Windows gRPC 服务，并在 Mac 最新主线修改接收端；后续开发按该方向推进，不回退 Windows。本节核对时尚未变更正式服务、编译信任或 activation。SDK 的原 CI 关键覆盖率 89.40%/95% 缺口仍保留，不以 Health 或本机测试代替原门禁通过。

后续只读证据根 `/Users/zhangzhen/.local/share/stock-analysis-feishu-readback-20261008`，包含 `feishu-platform-readback.json`、`endpoint-diagnosis.json` 和 fresh probe 日志。源数据资格、跨端兼容及用户预期会话仍待闭合。
