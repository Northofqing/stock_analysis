# H02 SeedManifest 内容与身份接续（2026-10-07）

基线 `41664d8624ecb7e8dd84de4722f35d4604d47bdd`；工作树 `/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis`，分支 `codex/platform-roadmap-implementation-20261002`。

## 本片行为与边界

新 consuming warm/cold 入口沿原 Transformed/冷恢复 source，先完成真实 typed 原件、前驱/头链接、审计和 V1 event/projection 内容返回；原第二 reader 保持存活，再固定类型解码 SeedManifest。每账户在同一累计16MiB TargetWork 中预付普通工作额度，实际 typed manifest 留在原 owner 中；按历史 `(1, MONEY_MODEL, FEE_MODEL, SeedManifest)` 流式重算 SHA-256，核账户/epoch、非空 seed 身份、source_hash 形式和同一 effective/cutover 时刻。

规范化沿历史 Deserialize：对象顺序、空白、可选缺省、等价时区写法不改变 typed binding；未知历史字段仍按原 Deserialize 忽略；重复字段、缺必需字段及非整数 Money 拒绝。实际摘要保持小写64位；不能用 raw JSON SHA 替代。source_reference/approved_by 存在和 source_hash 形式只表示记录内部一致，未证明真实来源或人工批准。

全部实际返回只能接纳一次；失败保 first、原 raw 输入、已成功解码的 typed manifests 及晚返回，重复返回原值退回。成功必须实际关闭原 reader 并验证 source/target tails；Busy/漂移拒绝闭合。旧 links/audit/event-projection 入口保持原范围；已关闭的 checked owner 不重开 reader。

**本片只完成普通内容子片。** 固定 DTO 保守收费不证明本机 decoder/allocator footprint，不绕过 paid codec/layout/native/SQL/provider 资格。seed 经济有效性、Genesis/Fact metadata/执行/裁定/完整经济重放及 projection 相等仍待闭合；不签发 Financial、真实资金B、正式正向 intent、生产 activation 或数据资格。

## 验证

最终 **12个不同完整方法PASS，0FAIL，0ignored**：新manifest六项＋受影响旧V1六项。实际 warm/cold、历史 canonical golden/等价输入、SQL/DTO/账户与末行篡改、预算/首错/晚与重复返回、Busy close 和 target tail 均通过。旧入口仍只查 event/projection，不升级 manifest 资格。

942源码输入前后保持，manifest `ce9f1d869781ac3f4f88db7366398b3a224bd835acf0b0f288d649094341ae17`；实际harness `81d310495bb44dddb52b84f280f3529e2d936ee8d54cf8df5604dfeec837acf4`。新组六项log `0537aa67e4cd88dc67dad1d68a34c472702c2826d355d721b8fb465f406a2c44`，直接复用相同harness的旧组六项log `03d67d323e5b8cbf6a673b01051e7156d8313136ed3ccadca780cbd1b052afe3`。原日志、前后清单、回执和完整方法名保存在 `.planning/2026-10-07-v1-manifest-content/`。

首轮编译退出101，E0603为复用原cfg(test) helper不可见；保首失败log `7d66e843ac26d7c0aab35b324828dcc697c5f543e47d015ff72f9c7afdc767ca`。修复仅向原parent开放四个测试helper，没有生产能力扩展；首次没有测试执行，不作缺陷RED证据。

直接自审与相关格式/diff-check通过，未另派独立审阅，不写独立Approved。不追加全量/check/build/clippy/release/生产操作；无monitor调度或物理投递行为修改。

## Windows 与生产读回

Windows 已补收到上批修复实际提交材料：`client-bundle/mac-outcome-repair-source-20261007.1`，95成员/18809726B，manifest `70da960eabbcd2a241a3ac7109b23e286c21ac619df2b00a061a6349fed41aa1`。Windows 包外 ACK 及 Mac 再核匹配；ACK SHA `f49751f117904e60a5f2392f92f13a7313aaf0bdbe249e830c58f207172b3fec`。它只核保存材料的字节和回报提交绑定，没有重建Git对象或执行Mac测试，也不是独立审阅/真实RPC/正式安装。

原 Windows 用户任务 cursor47 active，继续 Bash 原生异常定位及 SDK 必要开发；已缩到 MSYS 信号处理/CPUID 栈空间路径，尚未作本机因果裁定或修复；7174 同HEAD CI critical88.89% 未达95%，记录保留。官方日状态字段产品研究已提供，实际历史文件/使用范围/完整性/生命周期/发布修订仍缺，不能由 OHLC 或当前状态补 Trading。

本次只读生产 launchd 仍 monitor14998/bridge56417，monitor SHA `a7376a14cde1f6be3025d6dbbdedf61401c34154f3da928a8e3db30c4950d20b`，bridge SHA `2546b74d3af6929b5a08de4303f506232c9988b8ff030ec994f45b642cd4b1e4`，activation SHA `3c8b49dba568a9e4530ce5598802ea6e6e6ba7e015ffd0fdcd297ea5c0de16b5`。实际 Health 仍 Frozen/Unsafe，账户指标不完整且缺Quote/MoneyFlow/News/OrderBook；无安装、重启或旧数据修改，未取得新Uncertain逐条裁定。

## 下一依赖

继续同 live reader 的 Fact/Genesis/metadata 与完整经济重放，再与 H01 原生/SQL 资格汇合；后续旧坏价19候选、总金额B/seed、SDK同版RPC、精确激活和自然观察各按原门禁。整体沿[剩余工作交接](2026-10-06-platform-remaining-work-handoff.md)，上批效果修复沿[修复交接](2026-10-07-outcome-integrity-repair.md)。
