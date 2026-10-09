# H08 / monitor Status 失败关联（2026-10-09）

依据 [当前范围](2026-10-07-active-scope.md)，接续 [实际已发布业务修复](2026-10-09-business-closeout-repair.md)。当前开发基线为 master `986141163b34701e4a5a297c13b25e868e1ba375`；现网行为源码 `af66e214` 与开发源码分开。

## 实际缺口与本次改动

Windows 只读报告已经确认：原 MoneyFlows/Tencent 的文字错误缺少原 request_id、数值 Status 和原 `magic-error-detail-bin`，不能判定网络、源数据或 worker 的最终原因。现有 `GrpcError` 严格解码后保留已验证 detail，Gateway 的消息却不包含这些上下文；畸形/不匹配 detail 被拒绝时，旧诊断无法关联客户端真实请求。

本次仅在 `src/grpc_client/errors.rs` 的普通与指定 decoder 两个 Status 解码入口增加 `[gRPC][FailureCorrelation]` 日志。复用解码器已经取得的 standard/trailer 字节，没有另取响应正文、序列化请求或复制原始载荷副本。默认 WARN 级别开启时才计算指纹；每次有数据请求上下文的 Status 映射记录一次，重试沿原策略，多个失败可共享原逻辑请求相关 ID。两个入口也用于候选原件回读/测试，单条日志不能证明新 RPC 已发生；必须结合实际运行进程和调用批次核验，回读时的墙钟不能冒充原接收时间。

| 字段 | 含义与边界 |
| --- | --- |
| `status_mapped_at` | 客户端 Status 映射时采样的 UTC 墙钟；不是 RPC 开始、transport receive-complete 或单调耗时 |
| `client_profile` / `client_method` | 原调用方的 profile-bound `MethodIdentity`；不是不可信回复宣称的 operation |
| `client_request_id_correlation` | 原调用方 request ID 的既有 domain-framed SHA-256；不是原 ID、请求正文或服务确认 |
| `grpc_code` | 原 `tonic::Status.code()` 数值；不由自然语言 message 推断 |
| `status_detail` | 原 standard details 的 bytes 长度与 SHA-256，或明确 absent/malformed |
| `error_detail_trailer` | 原 binary trailer 的 bytes 长度与 SHA-256，或明确 absent/malformed |

binary trailer 的缺失与存在零字节不同；tonic 的标准 `details()` 为空时沿既有解码器记录 absent，不能据此推断原 wire 是否携带过空标准 carrier。Control、unchecked 和空 request ID 没有数据请求观察，不制造 client 关联。原严格 dual-carrier、一致性、profile/请求绑定、provider、reason、retryable、资格与拒绝结果继续由既有解析流程裁定；诊断日志不签发接纳能力。

不记录原 request ID、原载荷、状态 message、Provider 返回原文、auth metadata 或凭据。指纹可让 Windows 以自己的原 request ID 和错误载荷关联安全 stage 日志；它不能恢复过去未保存的 wire、原请求顺序或完整根因。

请求关联算法沿现有实现：UTF-8 domain `stock_analysis.grpc_error.request_id.v1`，依次 SHA-256 `u64be(domain字节数) || domain || u64be(request_id字节数) || request_id`，输出前缀 `sha256:`。载荷指纹是原 bytes 的普通 SHA-256，不使用这一 framing。

## 现网和外部证据

10-09 08:27 CST 只读核对正式 monitor/bridge 各自 PID、文件 SHA 与发布记录相符；monitor 为 `25e5d4e0ebcb963354b9f219786907d6976d166618748954c370d95663310aef`，bridge 为 `a13ed075def5896cc9226ef99b78a1adc51f2faa9834cd391a73149d3ad913de`，activation 文件为 `270a069efa54f78d37ee14cd9c4867145e7395e7f2cfc2d62c364874951a4fdb`。进程/快照/心跳新鲜，四路 raw news 恢复 ok；banner Frozen/Unsafe、缺 Quote/Kline/MoneyFlow/News/OrderBook。投递只读快照为 1022 Delivered、3988 RejectedDurable、79 Uncertain、6 原 ManualResolvedRejected，局部 pending/deliverable 均0。六条原人工终态不是当前四条提案的执行结果。

在“调整项目工作优先级”读回确认用户已获四条取消提案及客户端显示问题；取消裁定和客户端查看结果均未答复，因此本任务不重复请求或触发相应操作。

Windows 最终时间映射 ACK 6452 bytes / SHA `4e1d92d73177192ddcf6638e987cdd6262b1b8f35b6a977299a98209cb4bac89` 已在本机读取，其引用的两个 Mac 输入和七个 Windows 文件均逐项匹配。111.2951 ms 的时间领先、约15472.95秒旧文章与未定根因仍分开；本次 Status 失败日志不解决成功批次的时钟问题。H08 完整逐日资格仍 ContractNotDelivered；有限42项正向停复牌事件不是完整日期状态、价格资格或 PIT。

## 验证与交付

定向错误解码验证原件与完整源码输入清单位于 `.planning/2026-10-09-rpc-failure-correlation/`。首轮27方法为26通过、1失败，exit101；新增用例将既有安全诊断误断言为空，实际既有行为返回固定 `[redacted-unclassified-status]`。仅修正新测试，明确验证固定脱敏值及原文不泄露，未改分类/拒绝/脱敏逻辑。首轮日志 SHA `a4bbec9bd252ac61791b2ba403aa59a896d7e06e545735fdace6586068181e44` 与957项输入不变回执保留。最终复验于10-09 08:51 CST完成：`cargo test --locked --offline --lib grpc_client::errors::tests::` 实际 exit0，27方法全部通过（25既有＋2新增），5413过滤未运行。957项源码输入在复验期间完全相同；输入清单 SHA `fc270504dbe57adbd0bb6eb3fe1ef59f8f27bde56cbf421e27f6f392395a435e`，最终日志 SHA `60fa21232045e217513205ecbd3f6503d18d5220da2e45cbc5b13bb66239d7b7`，`src/grpc_client/errors.rs` 文件 SHA `433f2a9d3c08d8bea6ee387b39145b43d89cab26159a3a56cea0793c9516f3cc`。原152条 lib-test warnings保留，没有额外 check/build/clippy 或冻结平台/全仓测试。按实际调用链与输出字段完成自审，未声称新的独立发布复核或真实RPC验证。

此源码片未生成新 release/activation、未安装或重启正式服务，未修改数据库、裁定 Uncertain、发送测试消息或改 Windows 服务。随下一项确需上线的保留范围候选验证实际日志；当前在原件不足时继续明确根因未定。

## 实际提交与 Windows 接收回读

源码及本交接首版提交 `2a0ddb6a2039bcfa2c279a8e51cb19dcf0bb7e77`，主目录 master 与当前 `codex/business-closeout-20261009` 精确快进并 atomic push，两远端完整 OID 已实际回读一致。旧 platform 分支名与当前工作树身份不同，按实际分支交付；没有强制推送或改写其他工作树。

共享 `MAC_RPC_FAILURE_CORRELATION_20261009.json` 为3710 bytes / SHA `60aa6433c000bcd7a9e29995f5f651ad778c0f4efe96e355624b7f8bceb1cad0`。原 Windows 任务真实完成，接收回执7852 bytes / SHA `5d76573c88304e0ba3d5f90cc6d179e474ca003b2835e820456fb8d9bb494cef` 已在 Mac 读取；协议文件 bytes/hash、UTF-8/u64BE 离线向量及回执引用的3项旧原件 SHA 全部匹配。Windows 核验的是文件和算法，没有独立执行 Mac 源码测试、签发新服务或产生新真实 RPC。Mac 读取 ACK 同样是 receipt-only。

Windows 回读确认旧 MoneyFlows/Tencent/TopN 同次原 request/status/stage/carrier 没有恢复；新的关联字段不能追补这些历史原件，ThePaper 时间根因与 H08 完整逐日资格仍未解决。10-09 08:57 CST 再核 monitor/bridge 文件 SHA 及 launchd PID45310/36126 与08:27基线一致，未复跑 Health 或业务RPC。诊断源码尚未形成生产候选；下一项若确需窄发布，先封存同版输入、正常 release 和影响路径验证，再按精确 activation/兼容回退/单实例边界执行。
