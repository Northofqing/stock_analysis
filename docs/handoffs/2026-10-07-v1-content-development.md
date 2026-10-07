# H02 V1 内容校验开发接续（2026-10-07）

基线提交 `9b533b993d1b36efa0d97e0a2ec8de90060082a1`，前序审计内容源码 `4ee64e79871893d81d504717cd6989eaf57b507f`。本片在既有隔离分支 `codex/platform-roadmap-implementation-20261002` 开发。

## 本片范围

新 consuming warm/cold 入口从原 `AdditiveStorageTransformed` 开始，先完成真实 typed 输入、前驱/头链接与审计内容返回，然后在仍存活的第二只读窗口扫描 V1 事件和存储投影。沿用同一个累计 16 MiB `TargetWork`，逐行先付额度，再流式序列化/哈希；无事件 payload 副本、hex String、第三 reader 或新预算。

源码见 [V1 扫描与持有者](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/database/global_schema_v1_event_projection_content.rs)，验收见 [V1 定向测试](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/database/global_schema_v1_event_projection_content_tests.rs)。审计入口也新增独立的 `return_retained` 事实：失败返回移入原 first 后，重复返回仍被拒绝，原错误对象与真实输入保留。该缺陷由状态转移自审发现，新增回归检测失败后第二次接纳；未声称已有旧版本运行失败证据。

事件摘要沿历史 `PAPER_EVENT_V1` JSON 六元组；存储投影摘要沿 `projection_bytes` 原 UTF-8 字节。每项真实结果与输入保持在原 owner 链，最后真实 close 和 source/target tail 成功才得到局部 checked owner。原 audit-only 和 links-only 入口范围保持。

**本片不是完整 H02 完成或生产上线证据。** Manifest 仍需历史 SeedManifest 类型规范化和身份校验；事件 metadata、Fact/Genesis/执行/裁定/经济重放、投影与重放结果相等、Financial/native/provider、真实 SQL/capture/COMMIT/source-tail 资格仍须后续闭合。本片不安装 runtime 或签发 activation。

## 验证状态

最终源码内容组 **12/12 PASS，0 FAIL，0 ignored**（V1 六项、audit 六项）；同一 harness 的 raw-links 三项和 history-wire 一项均通过，合计 **16 个不同完整方法**。实际结果、完整方法名、源清单和 harness/log SHA 保存于 `.planning/2026-10-07-v1-event-projection-content/`。直接自审不作为独立 Approved。

首轮 V1 五项全部通过，933 件源码前后不变；随后追加重复返回修复和审计先行屏障回归，最终新 V1 六项与原审计六项通过。首轮 harness 与最终源码分别记录，不累计重复的方法数量。

最终源码 933 件前后不变，manifest `9d2e5f5d5500b8bb4fddaacd2cfbbbb46ed8b6cf549a2934aef035874e7adcab`；harness `e76c5dc9d6e69258da9629458015bfed34582c2db632f9ea9c121bbfd8419cd2`，内容组 log `da50b4793bae93229a14f3680c076a66e7dbac620f3401591be0e9adb2c0d70e`。验证包含真实 warm/cold、SQL 内容篡改、独立历史 wire golden、预算耗尽、审计先行、late/duplicate 返回、busy close 和目标 tail；未执行全量、另行 check/build/clippy、release 或生产切换。

相关日志：raw-links `5622ce6472dc3180259cc31e4999124daad3910ab859b3454b0d14752621edf8`；history-wire `2e1625de6a669b3c0ab772a7ba43675d27270995e808178db13fda8e89a09bf2`。两次均核源码仍与最终 frozen 清单相同、实际 executable SHA 相同，不另行编译。

## Windows 同步

已读取 `windows-docs-checker-native-debug-20261006.1/REPORT.md`：真实主 Bash 两次退出 `0xC0000005`，LF/CRLF 都可失败；27 次观察中 7 次失败、20 次完成，仍无异常落点或因果修复。11 项财报 WIP 尚未提交，原 f57c CI critical 88.84% 未达 95%。此前 audit 单 job 重试已通过，不重做。

六个公告输入的 Windows ACK 已收到：manifest `d8b42930f0203ae615df8405c44ef5ef0af2d006ab30a06189b829886c919278`，2 成员/8322B/6 个样例；仅包回读，没有真实业务 RPC 或发布资格。

已续办原 Windows 用户任务 `R08 FuturesDelivery 上游合同与部署`，实际 cursor `01fd0a89-06f4-4387-b2f0-04271ac501a0:23` 回报 active/inProgress，任务明确正在缩小 Bash 复现边界。要求保留首失败、继续有意义覆盖切片及同 HEAD CI，不降门槛、不以偶发成功替代修复、不在仅更新交接后停止开发。

后继 cursor25：脚本位置不是必要条件，移除 `test -e` 后仍有一次异常；仅保留读循环的 10 轮均完成。仍在分离字符串操作、输入和子进程结束交错，尚无因果修复。Mac 已核全部 68 成员/6935672B/69 文件，诊断 manifest `ea12777a88b55d2ef0792c2e121387da1053de35fc88df6ba68a3d47f7042656`；包外 ACK 仅字节回读，不是 Mac 原生复现或质量门通过。

读取状态曾一次不可用，后继 cursor28 已恢复并确认同 turn active/inProgress。Windows 报告已缩小到仅保留两次字符串截取的 8 行副本，仍出现同类异常；并开始推进不依赖 Bash 排查的 SEC 元数据离线测试切片。新源码和 CI 终态尚未交付，不替换原 f57c 失败结论。

## 下一依赖

1. Manifest 规范化哈希与身份子片的后继实现在 [清单内容接续](2026-10-07-v1-manifest-content-development.md)；沿同一活 reader，已关闭的局部 checked owner 不得重开 reader。最终验证与源码身份以该文档为准，不提升完整 Financial 资格。
2. 完整历史经济重放与 Genesis/执行/裁定/投影相等，汇合 H01 原生 provider/SQL 资格。
3. 真实资金 B、正向 F2 与 paper 生产接线；同版 SDK/RPC 和精确 activation 审阅仍分别执行。

整体 H01–H17 状态沿 [剩余工作交接](2026-10-06-platform-remaining-work-handoff.md)。
