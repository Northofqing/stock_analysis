# 2026-10-13 monitor 标的加载阻塞接续

执行范围仍为 [保留范围](2026-10-07-active-scope.md)：H08、outcome 和日常 monitor 稳定性。冻结平台不作为前置。

## 新运行故障：心跳与业务评估必须分别判断

10-13 08:39 CST 只读 `monitor --health --json` 退出1：进程心跳新鲜；账户/数据 banner 的最后观察是10-12 10:51 CST，账户评估10:38，不能把旧 `Degraded` 当作当前可用数据。raw 新闻快照仍属于上一 boot `89475`，现 boot 为 `24054:1791771847567258000:0`；新 boot 的新闻来源恢复尚未观察。

实际正式 launchd monitor PID24054、独立 bridge PID23932；二进制仍分别为 `d459ec8e566c460aef45742e431fe87439c25c6ab83ef0b201af61850dcbcc8e` 和 `a13ed075def5896cc9226ef99b78a1adc51f2faa9834cd391a73149d3ad913de`，activation `2b2ca75f96f4ca8663073320554edc9bca69ad566edfd1c2b4fbdcdcffd40069` 未变。PID 已不同于10-12晨间记录，重启原因没有在本轮核实。

08:40:59 的1秒 OS stack sample 中，93/93次采样均显示：

```text
main / tokio::join!
  → monitor_loop
  → TieredScanner::load_portfolio_targets
  → portfolio::store::load_watchlist
  → JoinInner::join / pthread_join
```

对应 `watchlist-security-metadata` 线程正在其独立 current-thread runtime 中等待。源码确认该同步调用位于和新闻、DataMode共用的主 future 中；这一阻塞会使其他 joined loop 无法再次 poll。该采样证明采样时的阻塞链，不能推断过去22小时一直在同一位置，也没有证明 Windows/传输底层唯一根因。采样原件90802B/SHA-256 `a757fc17480adbe85a135db39f02ee51fd3670dc4625f60226cd5b636ae2f073` 保存在本工作树私有 `.planning/2026-10-13-retained-observation/`。

## 最小源码修复与验收

同步 scanner 标的加载移入 `spawn_blocking`，等待时让其他主 loop 继续 poll。自选证券元数据的异步等待限制为35秒；超时返回 `watchlist_metadata_acquisition_timeout`，释放该 future，原线程正常返回后 join。源失败仍失败，worker异常转为加载错误，沿现有30秒重试路径处理。没有空批、假名称、陈旧名称 fallback 或来源资格放宽。

这只约束可取消的异步等待，不是同步数据库/I/O硬中断，也不是远端 worker 停止证明。若同一加载线程卡在同步 I/O，其他主 loop仍隔离，但该 scanner 批次仍不可用。真实止损/行情恢复还需要合格来源与市场时段。

metadata回归先保持原等待行为，`cargo test --locked --offline --lib watchlist_metadata_deadline_rejects -- --test-threads=1` 实际退出101/0通过1失败，断言 `stalled metadata must finish at its deadline`；不是编译失败。修复后 `cargo test --locked --offline --lib portfolio::store::tests::watchlist_ -- --test-threads=1` 4项通过，覆盖期限/cancellation、正常结果与来源错误保留、名称/证据投影及空批拒绝；152条既有lib-test warning保留。

`cargo test --locked --offline --bin monitor scanner_target_loading_tests -- --test-threads=1` 2项通过，覆盖同步加载停住时其他joined loop继续运行，以及原源错误和worker异常的失败路径；6条既有bin-test warning保留。两个目标合计6项通过，直接代码复核没有发现；本轮未另安排独立peer，不称独审已通过。不追加全库测试、check、build或clippy；此后的release是具体部署候选需求。没有上线或新 boot 的自然恢复证据。

## H08 原双端任务的最新边界

Windows 原会话10-13最新回合已可读，仍为停服、联调未通过。10-12 V2原始启动制品为 `9658a0e19e84ccd2fca657c3dcb5410b9a92a319c4deeabdd1ea4930d2c3acb5`，descriptor `abf28a3e0028488a7579da4d961e1a7c1408482bdc0500122c1956d225e480cf`；`841e4ae7`仅base HEAD，另有dirty编译输入指纹，不能当原正式841同制品。

随后 Mac prelaunch实际CLI1/RPC0，Windows exact-owned STOP完成。V2关闭原件2374B/SHA `b6efda1cf1fbcc0d7afd9eeaeabed176f25517a64898b1e723262af114c119b3`、final868B/SHA `e79aa1d655a0e054de64e8d4e6ec9b0bfe5678f3e0faa13b2f2c5482e2765a2f`。该窗口及GO/START均为历史，自动重试暂停。

原 Mac 恢复任务已完成 nullable `pdf_url` 兼容及 public grpcurl 保护路径的离线修复/独立SOURCE接受。共享 `MAC_ISOLATED_GRPC_TOOL_CUSTODY_ROOT_SOURCE_ACCEPTANCE_20261012_V1.json` 15945B/SHA `4ac13d4649f6d073db72e0d2cc3e239de66334152109304b873c9b0dfacf7f15` 状态为 `SOURCE_COMPLETE_INDEPENDENT_REVIEW_ACCEPTED_RUNTIME_NOT_AUTHORIZED`。本轮只核对并复用，没有重复开发、测试或独审，也没有读/哈希私钥及Token。

新一次原Windows会话协调返回 `An earlier turn submission is not yet confirmed`；发送及消费未确认，不反写为成功、不重发或绕过。共享文件的旧精确消费与此次应用消息分别记账。下一次 live 窗口继续由原恢复任务协调，不能自行重启或套用旧期限；正式 current critical95、同版真实consumer RPC与自然新闻→AI→候选→真实推送整链尚未验收。

## outcome、看护及生产边界

- 10-12日任务自然21:17..21:41:39退出1；verified0、deferred13、24 deferred窗口，原四条周末预测日期错误保留。不人工改日期或虚构收益/分母。
- watcher自然退出0、本地观测保存；手机适配器unconfigured/attempted0，不称手机送达。新版本周报仍等10-16自然运行。
- 主库device/inode保持，独立paper head1/events3；durable schema9、Delivered1118、RejectedDurable4007、Uncertain76、ManualResolvedRejected10。没有DB写、paper重seed或Uncertain重放。
- [10-12 raw期限精确候选](../ops/2026-10-12-raw-news-deadline-activation-review.md) 仍未收到人审答复、未安装，1001输入及封存制品不变。10-13 09:00窗口已错过，不得沿用；heartbeat不是批准。此次scanner修复不在该候选内，新窄候选准备包含原raw三文件与main/store共5份源码、1001真实build输入，拟10-14 09:00CST生效；实际build/typed材料/dry-run完成后才呈现精确人审。不能改旧封存tuple或在旧批准内换代码。

后续顺序：完成此slice验证/源码提交 → 同版窄候选与精确审阅 → 动态单实例、来源及Uncertain边界 → 新boot自然评估/新闻观察；H08恢复后才有界合格回填。其余冻结工程继续冻结。

## 09:22接续：新候选已经封存，仍未安装

源码及本交接初稿已提交9cb3f2cba73b04b7e12f7bd7305e07cdc9d32be0，primary master FF与atomic push均成功，master/business两个远端完整OID实读一致；root clean，primary两份Blueprint字节保持。共享SOURCE-only包 MAC_MONITOR_SCANNER_STALL_SOURCE_ONLY_20261013_V1.json：3170B/SHA ec90d27e8ba71742a76d659f6409c6194aa3a14374c63d8da46ef468f2f9e07b 已写入并JSON/字节回读，peer消费未知；不反写失败应用消息为成功。

[新精确激活审阅单](../ops/2026-10-13-monitor-stability-activation-review.md)已准备，拟10-14 09:00CST；1001项build清单SHA 8976972ffbcd42dbb250e193a9c92725be6e6b7afae53476dd949edb5e9529e5；正式monitor SHA e30c721dda1ac84e79f9b40c41c0477c0d46ed9ed30c32062357bfaa452bd2b2；typed配置 abc6bc51dd36ca990d708849e87155901a831d290e9ed5e5b6701fa735fdebd8；265B activation SHA af08a95e9531e3606c31240cfa4553d770b6fca8cdb557ce4ec47ee5bb27b713。

正式root release0/503.76秒；独立private-root诊断库0/452.82秒、typed preview/verify0，实际正式monitor隔离dry-run0/2.44秒。诊断库不可安装；正式库核验工具对旧runtime配置只读验证0，无DB/RPC。新配置实际runtime typed复核必须在精确批准安装后完成。没有追加全库测试或声称独立peer已复核。制品/输入/五文件回退已封存，原候选字节不变。

09:22末查monitor24054/bridge23932、旧制品与activation哈希、paper version3/events3保持；banner和raw快照仍stale，进程心跳新鲜。Delivered1118/Rejected4007/Uncertain76及pending0保持，未写DB、重启或安装。新候选仍待人审，错过拟生效时点保留旧实例。

## 今日Windows后继窗口：真实控制调用有进展，业务尚未验收

本轮构建期间原恢复任务推进了新的隔离联调，09:23读取其公共原件后接续；前文V2是历史，不是最新窗口。Windows公共关闭回执 WINDOWS_ISOLATED_GRPC_WINDOW_CLOSED_20261013_V1.json：1994B/SHA 8b485b3b9dc3b9f6fee14d051c9099184b979ecdb8b5e048ff3c130cc3a2be92 报告：新nonce 76639775…从00:53:54Z启动，Mac实际4个native调用中Health和缺失/错误Bearer通过；缺失客户端证书控制为TLS_REFUSAL_NOT_PROVEN，Caps和业务查询均未运行，attempt consumed1、CLI1。并非完整Auth或业务通过，也没有证明服务允许无客户端证书。

Windows exact STOP0，final867B/SHA 25cd48840184a5923d686ddb146102e51cb229a94f4dc79aa016273187fcec17 为STOPPED/active0/primary_signaled/cleanup[]；01:02:28Z无50051监听是其点时证据。新窗口也已消耗并关闭，不重用GO、重试或延长。

WINDOWS_MAC_TLS_FAILURE_DIAGNOSIS_RECEIVED_20261013_V1.md：2528B/SHA 81f763b149bc834721e95da56d089a64e2f576d186b44db74b3f986209fa3dd5 确认已读取3783B Mac安全诊断：只支持context_deadline_exceeded/failed_to_dial，明确certificate-required/TLS-alert/x509 marker均false，首次framing失败predicate未知。不能把超时/native1当TLS拒绝证明，TLS配置/网络根因仍未知。此处为已读取的owner公共证据，本轮不冒独立读取私密stderr或运行native控制。原恢复任务继续最小SOURCE-only诊断设计；不重复实施/换身份Token/降oracle或发起新live窗口。正式critical95、真实Caps/provider资格和五项业务仍未验收。
