# 2026-09-16 client-bundle 双合同实施计划

最新状态（2026-09-17，后续执行）：历史三个异常用例已通过独立静态审查并应用，尚待动态验收。Macro 新十类事实损坏矩阵首跑在注入前暴露测试重连计数时序问题；已加有界接收屏障，保留零新增RPC/TCP和15秒预算，不修改生产恢复规则。二者正在同一冻结输入下运行31项精确回归（`macro-v12-facts-and-history-negatives`，13:56:36Z启动），未取得终态前不称通过。真实v12 External完整路径候选的4项重要审查问题已提交修订，独立复审中，尚未应用。完整S2/S3、S4–S6、52Unit和上线验收保持未完成；未部署或重启monitor。

最新状态（2026-09-17 21:02，以下旧时刻段落为历史记录）：首条历史 attempts 恢复修复已通过14项直接测试和22项受影响回归，包含真实关库重开、三条完整 attempts 保留及零新增连接/RPC；[限定验收](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s3-history-c-first-green-verification.md)记录733输入和实际日志。真实 v12 全路径专门测试与历史负例仍待，不能称完整S3 C通过。Macro v12十类事实损坏候选已关闭两项静态审查问题，尚待应用和运行；完整S2/S3、S4–S6及52Unit范围保持，未部署或重启monitor。

最新状态（2026-09-17 20:40）：S3 C历史恢复缺口已真实复现：在线能识别3条attempts，正式写入成功，但数据库恢复无法取得相同Accepted trace。修正测试完整名后，14项实际运行13通过/1预期失败；[核验证据](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s3-history-c-first-red-verification.md)保留首轮筛选错误和真实日志。四文件最小历史catalog修复已独立静态审查通过，正在应用，尚无动态GREEN/真重开后验收结论。Macro v12四类新事实损坏矩阵仍并行准备；完整S2/S3、S4–S6及52Unit范围不缩减，未部署或重启monitor。

最新状态（2026-09-17 20:01，主控已核实际终态）：External 原生订阅/自选股更新及错误解析、能力目录刷新/双端点隔离的 12 项合批回归全部通过；独立审查通过，旧 Local 事件两项兼容证据仍有效。详[本批验收](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-events-b2-final-verification.md)。这完成事件客户端切片与 S3 B2，不等于完整事件消费者、S2/S3 或上线完成。S3 C 历史恢复首反例的两个测试/方法绑定问题正在修订；Macro Task4 新事实损坏矩阵并行准备。完整 52 Unit、S4–S6 等范围保留；未部署、未重启生产 monitor。

最新状态（2026-09-17 19:27，主控已核实际终态）：S3 B1 有效catalog下的完整reason/boolean/provider矩阵与0/1/16/17边界4/4通过，独立审查通过；External订阅/watchlist首行为反例准确报尚未接入，正在替换为原生实现。详[6项实际证据](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-events-red-b1-matrix-verification.md)。B2真实刷新/双端点隔离与C历史恢复观测接缝在并行准备，尚未应用；完整S2/S3、S4–S6及原52Unit目标仍未完成。未部署，不把本机测试当线上验收。

最新状态（2026-09-17 18:47终态，已由主控核验）：Local事件两项回归已通过；首次编译暴露的probe字段误调用已窄修，同批probe九项单测与Macro十一项共22/22通过，733输入一致。详[最终合批证据](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-14-chain-macro-recovery/macro-clean-probe-local-final-verification.md)。此前Listener/legacy/attempts在线60项证据保持。External订阅/watchlist首RED草稿独立审查发现缺少重连/Replay观察，正在补齐，尚未应用；完整S3矩阵与历史恢复、S4–S6仍未完成，未部署。下文带时刻段落为历史状态。

最新状态（2026-09-17 18:25，以本段为准）：legacy测试适配修正已应用，含Listener新字段、profile拒绝及原错误/控制/数据客户端的60项精确回归全部通过，相关独立静态审查通过；[实际核验](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s3-listener-legacy-runtime-verification.md)记载范围、输入与限制。Local事件两项集成回归待验；External订阅/watchlist只在准备。完整catalog矩阵和历史恢复接线未完成，不把本批通过称为整个S2/S3完成。当前Macro安全基线测试统一编译，普通源码冻结；未部署。

最新状态（2026-09-17 18:16，以本段为准）：56项相关回归已终态55通过/1失败，原4项attempts反例全部转绿；唯一失败是旧Macro测试只改profile而未构造真实External transport，修正已独立静态审查，正在两文件应用。Listener保留新字段的实现已应用并通过独立Spec/Quality静态审查，尚待动态验证；主控准备60项精确回归及Local事件两项集成回归。完整attempts结构矩阵/历史恢复、External订阅与watchlist、新三产品及后续迁移仍未完成，未部署。证据见[本批运行核验](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s3-online-and-listener-runtime-verification.md)。

最新状态（2026-09-17 17:47，以本段为准）：在线 attempts 合同校验及同端点能力证据传递已落代码，限定独立静态审查通过；56项相关回归（含原4项真实反例）已开始统一编译，尚无运行通过结论。Listener新字段首反例已落测试；Macro最小4桶test-only计量合入同一候选，三类运行分别验收。完整attempts有catalog矩阵、历史恢复接线、事件三路径及S4–S6仍待，未部署。主控已核实际10项源变化与各任务before补丁一致，不把静态审查当动态验收。

最新状态（2026-09-17 17:25，以本段为准）：混合历史真实迁移/追加/重开测试已通过，见[限定验收](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-mixed-history-final-verification.md)。同批原Full再次空正文，Macro预算问题仍未解决。新版attempts四反例已真实失败，在线校验与同endpoint能力证据传递正在实现；完整结构矩阵、历史恢复接线及事件三路径仍待，不称S2/S3完成。未部署。

最新状态（2026-09-17 17:13，以本段为准）：26项回归已终态25通过、1失败；此前三类合法失败writer/重开、迁移/摘要、篡改拒绝及原Full路径均通过。唯一新增混合历史测试的精确错误文案预期已按实际先触发的External校验修正，独立静态审查通过，当前正复测mixed与原Full。临时profile已清除；新版attempts首RED测试已应用但尚未运行，生产实现未改。事件订阅/Listener/watchlist仍走Local客户端，已单列S2剩余工作，不能称完整S2或适配完成。未部署。[26项日志](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/grpc-cleanup-mixed-failure-r2.log)。

最新状态（2026-09-17 16:33，以本段为准）：16项合批已真实终态13pass/3fail，732输入前后及当前一致。原四项迁移/摘要与Data版本损坏回归通过；新三合法failure用例把机器错误码误当展示diagnostic而失败，尚需纠正并跑到真重开断言。混合历史、profile清理及新版attempts候选仍未应用，不能称整体适配完成。未部署。[本批日志](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/grpc-v12-result-partition-and-failure.log)。

当前状态（2026-09-17 16:20，以本段为准）：历史RawResult V2分流修复已落代码并通过独立静态审查；版本损坏用例已同步V2→未知V3，三类合法失败writer/reopen测试也已补齐独立membership零调用断言并应用。原13项与新增3项共16项正在统一编译验证，尚无运行通过结论。相关源码冻结，未部署。混合历史/native恢复、新版attempts及其后续接口仍继续，不能把静态审查作为整体完成。

当前状态（2026-09-17 15:54，以本段为准）：此前39项限定回归及独立审查通过；新增真实V2数据库9种篡改拒绝已实测通过，但安全合批总计8/13，暴露旧布局RawResult V2被v12兼容读取器误分类的迁移缺陷（4例），另有旧版本损坏测试假设失效（1例）。正在按关联begin修复，不降级协议或改写历史事实。详[诊断](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-v12-migration-compatibility-diagnosis.md)。合法失败writer/reopen三例草稿尚未应用，独立审查要求补parent membership零I/O断言。新版attempts、事件、新产品及完整S2–S6仍待；未部署。

当前状态（2026-09-17 13:32，以本段为准）：新增及相关消费者 **39项回归全部通过**；主控核732输入前后与验证时一致，详[本轮证据](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-route-codec-matrix-current-evidence.md)。I-1/I-2与真实成功同库重开子项独立复审已通过；三路查询/异常响应与V1 strict/V2封闭失败测试完成运行，新增测试独立复审进行中。真实DB篡改、合法失败材料writer/reopen、新版attempts接线、事件与新产品尚未完成，S2及全部适配不能关闭。Macro原路径再次复现15秒超时，正实施有测量依据的期望摘要复用候选，尚无修复通过证据。未部署或操作生产monitor；下文带时刻的状态保留为历史记录。

本次新增文档：[2026-09-17.1 增量适配、问题与分工](2026-09-17-client-bundle-incremental-update.md)。

新增更新核对（2026-09-17）：当前 R/client-bundle/README.md 已标识 **2026-09-17.1**，source commit 为 `098021444d7b3c0dea4732c1b5a03e8773047cfb`。主控实跑 `shasum -a 256 -c manifest.sha256`，7 个公开文件全部匹配；R/W 的 `market.proto` 仍同 SHA `2f2037a00250b90bbd30525be2e7e9ad2e64c6dd30defc150642b9f993896a7d`。这证明当前公开包的文件一致性和 proto 未变，**不证明文档语义、部署服务身份或真实服务兼容性未变**。接口 Agent 正核新版公开文档及 metadata，旧 9 月 15 日基线的验收不自动扩展为新版全部验收；不读取/复制部署私钥或 token。后文的版本与验收记录保留历史语境。

最新进展（2026-09-17 13:01）：S1b 原生控制开发验收完成；S2 尚未完成。I-3 unknown-group 兼容问题已通过真实回归及限定独立复审。I-1 调用绑定、I-2 证据格式和 I-5 真实成功同库重开子项已实现，本轮14项回归全部通过，含拒绝新连接后的零连接/零RPC恢复；[运行证据](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-binding-restore-runtime-verification.md)列明具体范围，独立复审进行中。I-4 三路/异常响应测试开始应用；V1 strict/V2失败材料四项测试已准备，真实DB篡改及消费者回归仍待。新版 attempts 历史 Capabilities 接线、后续 event/新产品仍待。Macro 原完整成功路径新通过一次，但此前超时尚未解释，不能认定稳定性修复。全部改动仍未部署，未重启生产 monitor。

状态：**S0 双协议生成、S1a 错误身份与恢复链、S1b 原生控制开发验收完成；完整 S1–S6 尚未完成，未部署**。S1a 的 102 项行为回归、全部目标编译检查与独立 Spec/Quality 审查均通过，详见[开发验收证据](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s1a-final-verification.md)。依据：[接口审计](2026-09-16-client-bundle-interface-update.md)、公开 bundle `2026-09-15.1`，以及实际调用点。下列时间线保留历史失败与修复过程，以最新验收为准。

并行分工：接口 Agent `/root/grpc_error_profiles` 完成 **S1a 错误解析与恢复链**；`/root/grpc_s1a_review` 独立审查，主线程负责测试/集成。当前 `/root/grpc_native_control_red` 独占 S2 原生数据接口代码实施；`/root/grpc_s1b_control_review` 只在专属目录准备新版attempts回归，Macro作者准备未应用耗时计量补丁。主线程负责证据核对和统一编译，同一构建窗口只允许一名普通源码作者。S1a 覆盖 S1 的 profile 方法身份和 S3 必需的错误原文一致性：修复旧 parser 忽略 field11、误接纳冲突 standard/trailer 的问题。S1b 已完成原生控制，后续继续 native 业务请求、响应与事件客户端；不把错误链修复或双生成当成实际客户端已整体切换。

2026-09-17 00:07 更新：[S1b 首个真实失败](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/grpc-native-control-fields-red.json)已复现。真实本地 External SystemService 返回新 Health 字段，但当前客户端捕获的 `observability/build_identity` 为 `(None,None)`；编译成功、两控制调用及原鉴权/ID/连接复用断言已执行，最终1项行为断言失败。720构建输入前后及验证时一致，log SHA `10a5b991e950bf80126860b9c845ac39810b5434c6a45065e5767b7aa3726176`。已按[限定 GREEN brief](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s1b-green-brief.md)授权原生控制、能力缓存与同profile恢复修复；尚未通过GREEN/审查，不包含业务query/event整体迁移或生产服务兼容验收。主线v12拒绝矩阵15/15和独立审查已通过，默认debug完整Macro仍有空正文失败，分别记录、不相互覆盖。

22:47 更新：[S1a 真实失败记录](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/grpc-error-profiles-conflict-red.json)已取得，716 源输入前后一致。旧实现错误保留了 `Cailianpress/provider_unavailable/true`，断言要求冲突 detail 三项均不可采信；0 pass/1 fail、exit 101（编译成功）。现进入 GREEN 修复，仍未声明修复完成。

23:13 更新：S1a 候选已提交，独立审查与主控验证并行。[首轮候选验证](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/grpc-error-profiles-green.json)在新增 malformed trailer 测试构造处发生 `E0308`，尚未运行到 11 项测试；718 项构建输入前后一致。原作者正在窄修复该测试，保留失败证据，不把编译失败记为修复通过。未切换实际 External transport 或部署 monitor。

23:21 更新：测试构造已窄修复；[11 项新增回归](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/grpc-error-profiles-green-r2.json)与[91 项既有回归](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/grpc-error-profiles-regressions.json)均通过（exit 0），覆盖旧客户端/鉴权/重试、Local Benchmark/ChainBatch 与 6 条真实同库恢复路径。两批 718 项输入均前后一致；63 条既有 lib-test 告警保留。独立审查和全部目标编译检查尚待收口，因此尚不将 S1a 标为最终验收，更不代表 S1–S6 或上线完成。

23:26 更新：独立审查 Spec PASS / Quality Approved（无本片阻断项）；`cargo check --locked --offline --all-targets` 成功终态，718 项输入前后与当前一致。[S1a 最终验收](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s1a-final-verification.md)已落盘。所有 Cargo 已结束，未部署/重启 monitor；保留现有编译告警，不宣称无告警。

S0最终[独立审查](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s0-review.md)通过，无严重/重要问题，131条既有告警列为次要问题。下一步是typed方法身份和真实External客户端转换，不把生成模块已经存在当成调用链已经切换。下面22:23“审查尚待”保留为当时记录。

2026-09-16 22:23：[C0 GREEN](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/grpc-dual-contract-descriptors-green.json)实际通过（1pass/exit0，716输入前后及核验时一致，日志SHA `bb6c179d37c088892c55c4388821bcb0231900e07bbee1a16f24f7fe4e47ac58`）。Local完整descriptor与旧同参数基线逐字一致，生成Rust代码也未变；External descriptor完整等于独立公开合同编译结果，并验证新61/62/63与私有方法/字段隔离。临时merge helper已移除，可由本地开发归档恢复，未删用户原有代码。独立审查尚待；131条既有告警保留。**这里只解决构建隔离，不代表真实External客户端、三个产品或恢复链已迁移。**

2026-09-16 22:08开发证据：[C0捕获](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/grpc-dual-contract-conflict-red.json)在Rust编译成功后取得预期行为失败（0pass/1fail，exit101）：旧Local与新公开原文分别能通过protoc，生产合并函数处理新版时因61/62重号失败。主控核715构建输入前后及验证时一致，日志SHA `e809f0222a15c35da9c1dfbcc98c2627a29b5ab51835039865144c7367de94f3`；不是缺依赖或新接口不存在的编译错误。源码提取保持旧Local生成字节/descriptor不变，尚不能称接口已修好。GREEN授权仅S0双生成、实际descriptor回归，真实External客户端与业务接线仍按后续切片推进。

`R=/Users/zhangzhen/Desktop/Quant/stock_analysis`；`W=R/.worktrees/push-reliability-20260905`。下文构建/合同/测试路径相对 W；模块路径 `grpc_client/`、`grpc_contract/`、`data_gateway/`、`market_domain/`、`push_foundation/`、`bin/` 均省略 `src/` 前缀；公开资料路径相对 R。本轮仅写本文件；源码、bundle、Cargo、git、网络/RPC、数据库及 monitor 均未改动或执行。实施须在主控分配文件所有权与构建窗口后进行。

## 已定合同与交付标准

Local 从冻结的独立 proto 生成，保留原公开 0..60、`ChainBatch=61`、`BenchmarkBars=62`、两个私有 RPC、`QueryResponse.source=11`、`BenchmarkAuditState/BenchmarkErrorDetail`。External 逐字编译新版公开 proto，保留公开 61/62/63，不追加私有声明、不改号、不修改公开 descriptor。

两份生成代码可保持同一 wire package/method path，但输出目录、descriptor 和 Rust module 独立。现有 `pb::magic::market::v1` 可暂保留为明确的 **Local frozen** 路径；新增 `external_pb::magic::market::v1`。共享业务身份使用 typed method/domain 值，跨 profile 的转换按方法显式匹配，禁止 `as i32` 后再用另一套 `Operation::try_from` 解释。

完成意味着：真实 `connect_client_bundle` 路径使用 External 生成客户端，现有三项 External 查询和新三项查询均有 typed 请求、响应转换与能力门；Health/Listener 新字段、错误 attempts、原始证据及恢复保持对应合同；Local 成功和错误链继续兼容。仅双生成、增加 allowlist、单测通过或返回 JSON 都不足以宣称整个任务完成。线上兼容另需提供方真实样本，不能用 loopback 代替。

## 核对后的改动接缝

| 现有位置 | 实际耦合及切片要求 |
| --- | --- |
| `build.rs:29-44,49-94`；`grpc_client/pb.rs` | 当前每次从 bundle 追加 Local 扩展并生成唯一 module。首先解除 Local 对最新 bundle 的追随。 |
| `grpc_client/client.rs:72-82,265-322,333-435` | `profile` 虽已分离，data/system/events 三个客户端仍全是 Local pb；普通查询也先走 Local `is_implemented`。新增 External 类型必须到达这些真实调用点。 |
| `grpc_client/envelope.rs:61-110`；`grpc_client/unary_attempt.rs:47-117` | `QueryResult` 仍含 Local `AdmissionState/CanonicalPayload`；External acquisition authority 当前写入伪装成 Local 的 wire response.source。应分别解析 wire，再投影 domain envelope。 |
| `grpc_client/errors.rs:297-304,338-390,455-499` | 错误先经 Local Operation 过滤；standard/trailer 比较的是解码后 message，旧类型会丢新版字段。恢复也复用同一 decoder。 |
| `grpc_client/{macro_attempt,board_attempt,external_control_attempt}.rs` | 会保存 request/response/status 原始 bytes；`unary_attempt` 是共享执行入口。必须先保存原始 profile wire，再转换，不能把转后的 Local message 重编码当 External 原文。 |
| `data_gateway/grpc_source.rs:846-933,2635-2639,2953-3072` | External 能力与 ready cache 使用 Local Operation/raw i32；Health 门仅 live+ready。 |
| `data_gateway/grpc_source.rs:480-505,2891-2950,3954-3986` | Benchmark 专用错误严格核 raw 62；ChainBatch 走 Local raw 61。两条真实路径保留。 |
| `push_foundation/intent_store/chain_post_close_macro_codec.rs:167-188,637-669,848-898,1043-1063` | 控制与业务恢复直接解码 Local pb，且重新判断 Health/capabilities；只改在线客户端会留下恢复漏洞。 |
| `data_gateway/economic_calendar.rs:39-67`；`grpc_source.rs:3333-3340` | `latest_releases(limit,country)` 当前审计有参数，wire 却走旧 `EconomicCalendar {}`；新 observations 入口必须让参数真正进入请求。旧方法不静默换语义。 |
| `data_gateway/grpc_source/convert.rs:140-190`；`market_domain/provider_id.rs` | 通用 Local converter 期待单个 JSON array payload，且当前 ProviderId 没有 HithinkFinance。External 是逐记录 payload；竞价 route 与逐条来源须分开保存。 |

## 切片顺序

每片只做一个可观察结果，按失败用例 → 最小实现 → 相关回归推进。S0 可独立先落；S1 后可准备 S3/S4 的独立文件，主控依次集成共享文件。S5 三个产品逐个完成，不一次写全套测试再补实现。

### S0：冻结 Local，新增独立 External 生成（最小首片）

文件：`build.rs`、`grpc_client/pb.rs`、`grpc_client/mod.rs`、W `client-bundle/market.proto`；新增 `contracts/local_bridge_v1/market.proto`、`contracts/local_bridge_v1/upstream_20260905.proto`（旧公开输入归档）、`grpc_client/external_pb.rs`、`tests/grpc_contract_profiles.rs`。

1. 先核对 W 旧 proto 的已知 SHA 并逐字归档到 `contracts/local_bridge_v1/upstream_20260905.proto`；有差异即保留当前文件并重新审查。冻结内容取自 **该旧 proto + 当前 Local 扩展的完整结果**，保留所有旧字段及 Benchmark 专用错误，不从新版删三行拼回旧合同。后续 build 不再动态合并公开 proto，旧归档不作为额外 protobuf 编译输入。
2. 输出分别为 `OUT_DIR/local_bridge_v1/magic.market.v1.rs` 与 `OUT_DIR/external_v1/magic.market.v1.rs`，各有自己的 `descriptor.bin`。使用显式 include 路径，不让同名 package 覆盖输出。
3. 主控已明确授权在 S0 用 `apply_patch` 将 R 新版公开 `client-bundle/market.proto` 精确同步到 W 的同名文件；仅同步这一公开文件，不复制凭据、不改 R。验证 R/W 文件 SHA 一致。两份合同均使用仓内相对路径构建，普通构建自包含，不新增外部 proto 路径环境开关，也不依赖 R 的绝对路径。
4. 给两份原始 proto 和生成 descriptor 分别计算 SHA-256。登记新 proto、旧输入归档、构建设置及工具链版本到 **gRPC 专属 capture**；现账户 capture 未追踪 `contracts/**`，不能复用其 665 输入证据。`rerun-if-changed` 覆盖实际编译输入与相关生成配置。

最小首反例 C0：在独立临时构建夹具中，用新版公开输入执行旧的“合并 Local 扩展后 codegen”路径，应观察到 61/62 重号导致 codegen 失败；不替换 R/W bundle。首测试名 `grpc_dual_contract_new_public_input_preserves_local_and_external_wire_contracts`；红色证据是实际 protobuf 重号错误，不是缺少新 Rust module 的编译错误。修复后对两份真正生成的 descriptor 验证：Local 61/62 名称、source11、BenchmarkErrorDetail 存在；External 61/62/63 名称正确、63 个 MarketData RPC、无 source11/私有 RPC/BenchmarkErrorDetail。预期值来自公开 proto 和冻结 Local 合同，不从实现映射表反推。

验收：两份真实生成产物可编译，现有 Local fixture trait 无须增添公开三方法；记录 C0 的实际失败与成功日志。此片只交付构建隔离，**不声称 External 已切换**。

### S1：typed method、domain envelope 与按 profile 解码

文件：新增 `grpc_contract/methods.rs`、`grpc_client/external_envelope.rs`；调整 `grpc_contract/mod.rs`、`grpc_client/{envelope,errors}.rs` 和相关调用方。`ops.rs/schema.rs` 的现有表暂明确为 Local，不把 0..62 测试改成统一 0..63。

最小 Interface：`LocalMethod`、`ExternalMethod`；需要统一保存时使用带 profile 的 `MethodIdentity`。Local 61/62 与 External 61/62/63 是不同 typed 值。External 的能力身份可以覆盖公开方法，但可调用请求保持已交付 allowlist；“有 Operation”不等于“可 query”。已存在的兼容入口只按明确方法白名单转换，拒绝 Local 私有方法进入 External。

分别解析 Local/External Request、QueryResponse、Capability、普通 ErrorDetail。转换后的 envelope 保留 provider、batch、时间、完整性、diagnostic、records；共享 admission/record 类型改为 domain 数据，不借 Local QueryResponse 中转。Local source11 原样保存，External acquisition authority 只来自认证连接上下文。BenchmarkErrorDetail 继续只交给 Local 专用处理器。

首个语义反例 C1：通过 profile parser/调用 Interface 输入 operation 字段相同的 61（最小 protobuf 字段片段 `10 3d`）。Local ChainBatch 结果/错误只能得到 Local ChainBatch，External Auction 只能得到 External Auction；将 Local typed method 交给 External 必须在发送前失败。再覆 raw 62、63、错误 request_id/方法号及 Local Benchmark 专用 detail。检查 domain 身份和错误分类，不只检查两个整数相等。

验收：共享错误不再只有无 profile 的 `Option<i32>` 身份；所有 External decoder 能保留新版字段。需要兼容 raw operation 时必须连同 profile/method 保存。Local 私有业务路径继续接受 61/62，不改号。

### S2：让真实 External transport 使用新生成客户端

文件：`grpc_client/client.rs`；可新增 `grpc_client/external_client.rs` 收纳外部 router；迁移 `external_v1.rs`、`external_control_attempt.rs`、`unary_attempt.rs`；接入 `grpc_source.rs`、`bin/grpc_bundle_probe.rs`。

保留现有连接/TLS/鉴权机制，将客户端内部 transport 明确分为 Local 与 External。先走通现有 `SecurityMetadata` → `GlobalNews` → `InstrumentNews`，每次验证完整 wire/request_id/schema/provider/response 转换。`query_external_op` 与 External ready cache 改用 ExternalMethod；不能先经过 Local implemented 集合，也不能最终仍调用 Local generated stub。

Health、Capabilities、Listener/stream/watchlist 的 External 方法同样转向 External generated system/events；Local 方法保留 Local 合同。Listener 新 replay/subscriber/agent counters 与 Health observability 保留为可缺失的观测值，不升级市场数据准入。

首反例：External loopback 回新版 Health identity 和 raw capability61，实际 bundle 客户端应保留 identity 并把 61 标成 Auction；Local capability61 仍是 ChainBatch。随后现有三查询从同一外部连接完成。测试 server 使用真正的 External trait，非旧 Local trait 加字段。

验收：真实连接构造、普通请求、控制请求、事件路径均选对生成客户端；未知/未交付方法仍在发送前拒绝。鉴权失败、重试 request_id 稳定性、External source authority 的原回归保持。

### S3：错误尝试链保真及一致性

文件：`grpc_client/errors.rs`；新增 `grpc_client/provider_attempts.rs`；`unary_attempt.rs`、`external_control_attempt.rs` 与恢复调用方只接新 Interface。

首反例：同一 status 的 standard 和 custom trailer 旧字段完全相同，仅 field11 attempts 不同。当前 Local decoder 会丢 field11 后误判一致；新 decoder 必须拒绝冲突。先比较原始 detail bytes 一致性，再按正确 profile 解码；单边合法仍支持，malformed 保持拒绝。不放宽 status/detail 冲突规则。

保留公开六字段及原始次序，最多 16 项，不截断为“合法前 16 项”；不依赖自然语言 message。0/1/16 项、17 项、单 trailer、两边一致/冲突、未知 provider/outcome/reason 和缺字段均有用例。恢复必须产生相同的安全 typed 分类及 attempts。

2026-09-17.1公开合同已补齐词表及组合规则（`client-bundle/grpc-external-api.md:1026–1053`），替代本计划最初“缺词表”的前提。必须实现1..=16项、ordinal从1严格连续且保持wire顺序；provider逐字来自同一endpoint的Capabilities、至多64且无控制字符。selected仅selected/false/false；rejected按公开20项reason且false/false；failed按公开6项可重试和7项不可重试reason，terminal独立。不以本地known-provider枚举签发资格。

任一不满足合同或缺历史catalog证据时，整链解释状态unsupported，不抹去顶层status/reason/retryable，不截断成“合法前16项”，原bytes仍保留于既有限制载体。合法并有同endpoint证据的链必须能supported，不能以永久unsupported替代完成。Macro已有同episode原Capabilities bytes、request/endpoint/authority与ready-result/data-begin关联足够派生历史catalog；在线和重开采用相同证据，不增加SQL/冗余JSON/同TCP token，也不以当前Capabilities补旧记录。字段解释与顶层重试策略保持分离。

验收：在线与恢复的 attempts 不丢失，不能因丢失未知字段使两份冲突 detail 变得相等；原有状态码/重试策略不因未验证 attempts 被偷偷放宽。

### S4：Health 身份资格与缓存/恢复一致

文件：新增 `grpc_client/build_identity.rs`；调整 `external_control_attempt.rs`、`grpc_source.rs`、`bin/grpc_bundle_probe.rs`；持久化接 S6。

首反例：`live=true, ready=true` 但 contract 或 binary 与可信期望不一致，必须拒绝进入 External ready cache。以独立 fixture 期望身份测试匹配、缺身份、identity_error、缺 source/binary、各字段不匹配；旧缺字段只得到明确 Legacy/Unverified 状态，不能当身份已认证。

ExpectedIdentity 使用公开/可信部署材料中的 service version、source revision、descriptor SHA、binary SHA；bundle 版本不是 service version，metadata source_commit 不是二进制身份证明。没有提供方 hash 计算口径时，本地 descriptor SHA 只代表本地生成产物。缺期望材料返回明确缺证据，不从第一次 Health 自行建立信任。

能力缓存绑定已验证的 profile、descriptor/build identity 和连接上下文；重连或恢复使用同一身份资格函数校验现有证据。恢复本身不发 RPC；证据不足时按 S6 停在待准入，不能借资格检查重放已确认或 Unknown 请求。运行计数不参与身份或行情新鲜度判断，不把进程重启后的计数回落视为市场证据冲突。

验收：在线 gate、probe、durable gate 共用同一规则；身份不匹配的 Health 即使 ready，也不能授权后续查询。仅有 mock 期望身份时只声明客户端规则已验证。

### S5a / S5b / S5c：三个完整产品切片，逐个交付

共同文件：`grpc_client/external_v1.rs`/外部 router、`grpc_source.rs`、`data_gateway/mod.rs`；新建 `data_gateway/current_auction_observations.rs`、`economic_release_observations.rs`、`economic_release_schedule.rs`。转换实现在对应新模块或 `grpc_source` 子模块，避免多人同时扩写 `convert.rs`。

每个切片都要有 typed Request、公开 schema/version/provider 的精确 wire 构造、真实 generated RPC 路由、能力/身份门、逐记录 typed conversion、公开 Gateway/GrpcSource 入口及 loopback 用例。schema 均为 v1，`allow_unadmitted=false`；记录是逐条 canonical payload，不走 Local 单数组解码器。

| 切片 | 首个失败用例与最小完成行为 |
| --- | --- |
| S5a CurrentAuctionObservations / 61 | `stage=live` 请求，返回合法 `auction_price=null, auction_volume_ratio=null, auction_unmatched=-321`，应得到保留这些值的 typed record。stage 用枚举；验证非空、唯一 instrument、stage 回显及请求/返回身份；顶层 HithinkFinance 与逐条 Tonghuashun 分别保存。仅约束公开定义的数值，不凭字段名断言量比必须正数。保留空 source_at，无 trading_date、不拆买卖量；不能让该结果满足 P-02 严格日期/源新鲜度门。 |
| S5b EconomicReleaseObservations / 62 | `limit=20,country=中国` 必须在实际 RPC payload 中出现；完整空 records 得到“本次滚动窗口 VerifiedEmpty”，不投影为某国家/某日/未来日历为空。非空记录保留 scheduled/released 时间与原 evidence，验证 released_at 与原 source_at 同一时刻、批次 source_at 为最新原时间。新增明确入口，可复用 EconomicReleaseFact；旧 EconomicCalendar 和其 durable identity 不自动替换。 |
| S5c EconomicReleaseSchedule / 63 | inclusive start/end 跨多日期，同一 release_id 在不同 release_date 出现时仍保留两条；同一 `(release_id,release_date)` 重复或冲突拒绝。typed 日期保持 date-only，按 `(release_date,release_id)` 验证顺序、请求范围和 limit；最多 inclusive 366 天、limit 1..100。source_at 永远不从日期、午夜、observed_at 或 release_last_updated 合成；空批次结论仅限所请求 FRED 范围。 |

每片续加非法/空/重复请求、错误 schema/version/provider、request_id/op 错号、partial/diagnostic 响应、缺/多/重复记录、record/batch evidence 冲突。竞价返回缺失证券时至少不能构造完整请求覆盖；是否允许窄结果由明示合同决定。公告、Auctions、原 EconomicCalendar 等现有方法不因新名称改变准入。

Observations 的最大 limit、竞价最大证券数未在当前公开段落交付；先使用已有明确调用需要（如 observations 的 20），其他本地限制标明是客户端策略，不能把 Schedule 的 100 或 RealtimeQuotes 的 60 冒充这些 RPC 的合同。新的未来日程消费者只接 Schedule，发布事实消费者只接 Observations。

验收：六个 External 已交付查询（旧三 + 新三）均有 typed end-to-end loopback；probe 可显式选择三种新请求并展示安全的身份/证据摘要。竞价 route 不被 `parse_provider` 静默改写为 Tonghuashun/Custom；逐条 provider 保持原值。

### S6：迁移 durable 接缝，清除 External 旧类型入口

文件：`grpc_client/{macro_attempt,board_attempt,unary_attempt,external_control_attempt}.rs`；`push_foundation/intent_store/chain_post_close_macro_codec.rs`、`chain_post_close_position_concept_rpc.rs` 及相应测试。此片与账户作者协调，只改协议 capture/restore，不改推送内容或账户语义。

按真实持久化语义区分 profile：Macro 的 GlobalNews 可 External；EconomicCalendar/SemanticSearch 当前保留各自已授权 Local 行为；Board/DragonTiger 既有 Local 不因公共 RPC 同名而变成 External。新产品若本轮未进入 durable 消费，不伪造新的推送步骤；其 status/response decoder 仍须支持带身份的保存/恢复验收。

原始 bytes 记录同时绑定 contract profile、typed method、request_id、descriptor 身份；控制响应再绑定已验证 build identity。新格式明确版本。历史记录保留并读取其实际版本：有可证明 profile+method 上下文的旧记录按冻结合同恢复，缺完整身份的旧 External 控制证据标记未验证；无标签 raw61/62 拒绝猜测。不得通过重编码旧记录“补出”新字段或身份。

恢复必须保留已确认事实、原请求 ID、原期限及 Unknown 禁止重发规则。缺身份不能触发原 Health/Capabilities 的再次发送，不能重置 deadline、重试序号或把已确认结果降回未发送。若确需新的资格检查，只能在原流程合同允许时建立显式、独立、可审计的新 effect；否则停在待准入。此要求同样适用于 S4 的恢复校验和升级回退。

首反例：先捕获 External response/status（含 identity/attempts），恢复时换成 Local profile 或不同 descriptor，应失败；同 profile/同身份恢复的 typed record/错误证据与在线一致。再覆盖旧 Macro fixture 可读、未知新版本拒绝、缺身份旧 Health 不授予 ready、重连身份变更失效、错误双载体冲突。恢复缺身份的已确认/Unknown 控制记录时，测试断言原请求 ID/期限/事实保持且没有新发送；允许新资格检查的流程须另验证新 effect 的独立审计与授权。

验收：`restore_persisted_status_error`、`QueryResponse::decode`、Health/Capabilities decode 的每个调用点都有已证明 profile；External 原始响应不会经 Local 类型重编码。Local ChainBatch 的 domain JSON 与 Benchmark receipt 现有存储不做无关迁移。

## 文件所有权与可并行工作

| 所有者/工作包 | 独占文件及可并行条件 |
| --- | --- |
| gRPC 集成作者 | `build.rs`、新 contracts、pb modules、`client.rs`、`envelope.rs`、`methods.rs`、`unary_attempt.rs`、`grpc_source.rs`、所有 mod/export 文件。共享入口由一人顺序集成。 |
| 错误/身份工作包 | S1 Interface 固定后可写新 `provider_attempts.rs`、`build_identity.rs` 及各自测试；`errors.rs`、control 文件改动交集成作者合入，不能同时写。 |
| 产品工作包 | S1 Interface 固定后可在三个新 product 文件各自完成 typed 请求/转换测试；router/export/GrpcSource 接线由集成作者统一完成。 |
| durable 工作包 | S2/S3/S4 Interface 固定且主控授予 ownership 后，独占上述 codec/attempt 测试；与账户作者冲突的文件先交接。 |
| 账户推送作者 | `push_templates` 及当前已占用账户文件仍归原作者；gRPC 不改，也不接管其构建证据。 |

此表是文件分工候选，不自动授权再开 agent 或扩大写范围。Local fixtures 保留旧 trait；只迁移真正 External 的 `external_control_loopback_fixture.rs`、`external_mtls_attempt_tests.rs` 和混合 Macro fixtures 的 External 分支，新增 External fixture 的三 RPC 方法。`tests/support/grpc_fixture/{handlers,data,events}.rs` 是 Local 集成 fixture，不机械改成 External。

## 验证与证据包

以下是实施后的命令，**本轮未运行**。先由主控提供独立 gRPC capture/target 路径与构建窗口；在 W 使用仓内两份合同正常构建，不依赖 R 或额外 proto 路径环境变量。新测试统一使用 `grpc_dual_contract` 前缀便于精确筛选。

1. `cargo test --offline --test grpc_contract_profiles`：S0 两份真实 descriptor、各自 raw61/62 归属、无私有声明污染 External。
2. `cargo test --offline --lib grpc_dual_contract`：新增 profile/typed 查询、attempts、身份与恢复回归；测试数必须非零，记录每片 red/green 结果。
3. `cargo test --offline --lib grpc_client`：现有 retry/auth/External mTLS/Local 客户端回归；网络依赖仅测试自建 loopback，不连接部署服务。
4. `cargo test --offline --test grpc_channel_e2e`：Local 查询与事件不回退；按实际改动运行 Macro/Board/Benchmark 的精确测试过滤器，不把数据库/生产路径带入本任务验证。
5. `cargo check --offline --all-targets`：捕获 probe、测试 trait/literal、durable codec 的编译遗漏；如缓存缺依赖，记录缺项交主控，不自行联网下载。

capture 至少列明：gRPC 所有源输入（含新增 contracts/旧 proto 归档/测试）、本次引用的 R 公开 metadata 摘要、R/W 公开 proto 同步摘要、rustc/protoc/codegen 版本、每条命令与退出码、测试数、实际生成代码/descriptor 路径和 SHA。R metadata 只作来源证据，不成为 W 编译依赖。测试前后核对输入没有被并行作者改动；变化后只把原测试归属原 capture，必要时重验受影响部分。

| 摘要对象 | 当前已知值/要求 |
| --- | --- |
| 旧 Local 基础 proto 文本（W） | `8730bce3c20e170cf8f58047336ae06d3a5e9080d81568dee71e7b0882063332`，来自审计；不是最终冻结文件 SHA。 |
| 完整 frozen Local proto 文本 | S0 生成后单独登记；包含私有字段/错误，与上一行区分。 |
| 新 External proto 文本（R） | `2f2037a00250b90bbd30525be2e7e9ad2e64c6dd30defc150642b9f993896a7d`，构建前重新核对。 |
| Local / External descriptor.bin | 各自实际生成后分别计算，不使用任一 proto 文本 SHA 代替。 |
| 部署 binary / source revision / service version | 来自可信提供方构建材料并与真实 Health 比较；客户端自身编译 SHA 不能证明服务端制品。 |

还须静态搜索 `pb::`、`Operation::try_from`、`as i32`、`GrpcError::from`、`restore_persisted_status_error` 和 `QueryResponse::decode`：逐项确认 External 没经过 Local decoder。记录明确白名单，不要求删除合理的 Local 使用。

## 缺失输入、运行验收与回退

- **Attempts 公开补件**：请提供方在 `client-bundle/grpc-external-api.md` 的错误章节（或公开且纳入 manifest 的独立附件）补 `provider_attempts` 的完整 provider/outcome/reason 词表、ordinal 起点/顺序/重复规则、terminal/retryable 合法组合及脱敏 wire 示例。现有 proto 只有 string/bool/u32，文档只保证有序且最多 16 项。未补前可交付有界保真与 Unsupported，不能声称完整闭合语义已验证。
- **身份公开补件**：提供实际部署的 service_version/source_revision/descriptor SHA/binary SHA，以及 descriptor 原件或其生成与哈希口径；现 `bundle-metadata.json` 没有后两项。应由提供方新增公开 deployment/build identity manifest 或单独交付可信证明，本任务不改 bundle 填值。
- **Local 部署证据**：提供实际 Local descriptor/构建身份、ChainBatch 与 BenchmarkBars 成功 response、普通 ErrorDetail 和 BenchmarkErrorDetail 样本。当前代码冻结61/62，只证明仓内合同，不能证明正在运行的服务。
- **新 RPC 样本**：授权真实只读验收后，收集三 RPC 的非空/完整空/失败响应、capability、Health/Listener 与 detail trailer；分别记录日期/时间、provider、schema、batch/request identity。需要现场窗口或 provider credentials 的用例标明未取得，不补造成功。

提供方补件不阻止独立代码/loopback 切片推进，但会限制“生产就绪/兼容已验证”的结论。真实运行验收须另获主控授权；本计划不授权服务重启、部署、数据库变更或发送推送。

回退以单个切片/开关停用新 External 查询为单位，保留冻结 Local 和已捕获原始证据。新 durable 格式落盘后不能直接用不识别该版本的旧二进制恢复；先停用新格式写入或继续用新 reader，不能删除/重写历史 bytes 来让旧 reader 通过。不得通过把公开61/62改回私有解释来“回退”。

计划核查结论：已覆盖独立生成、真实客户端/控制/事件接线、三个 typed 产品、错误 attempts、Health 身份、缓存与 durable 恢复；尚无实现、Cargo 或真实 RPC 验收证据。
