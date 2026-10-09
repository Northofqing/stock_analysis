# 永久交付证据索引

本页是简短导航，全文证据在Desktop外private durable archive；不是生产receipt。首次源码/脚本交付commit `1ce18c2850d19b4bd0908bce6225049c61b0d19d`；首次5selected release source `f3761320cac5044999a53cdbd7a2acb57107a6de`，随后只重建assistant于1ce（其他compiled inputs不变、4bin旧hash一致）。版本名中f376是首构建标签，不冒充所有bin同一source。各报告头部旧pending是历史快照，最终独立rereview优先：Tasks1–5均local clean，Task4 3/0/0、Task5 2/0/0；Task6独立审查PASS；wholebranchreview提出F-Q01，最终修复及F-R01已实现，待同一scoped rereview/root最终门禁。最终选用下述final-fix候选，首次两次构建的身份与测量仍保留。

保存根：`/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-evidence/20261009-task6`。`task5-reproduction/`是完整75文件/1,009,488,344bytes，包含old/new executables、两份immutable DB、原55payload与18fix supplement、全部raw measurements/failed诊断。`task5-preservation-manifest.json`逐文件原source_path、length、SHA；没有在新路径重测。旧exe是复现证据，不能安装生产。

`review-evidence.tar.gz`保留全SDD任务brief/report/review/rereview/diff/rawlog/probe文件及exact原v2plan。`review-evidence-manifest.json`逐member原path/length/SHA及archive整体身份；解压可恢复全文，避免SDD清理丢失失败/warnings/决策。root最终Task6/wholebranchreview保存到同根独立late supplement，不覆盖早期archive。原plan在ignored `docs/plans/2026-10-09-project-restructure-and-agent-plan.md`，immutable copy在archive的original-plan路径，二者身份可核对。

检查导航：Task1最后25Rust+10Python；Task2初26/修复20；Task3最后9assistant+9boundedserial+2bin，provider8复用、修复2one test内16variants；Task4最后25lib（另FIFOchild1）+6bin，store1复用；Task5 audit25/ignored1、capture4、SLI4、directCriterion4cases。不要将不同scope结果加成全suite成功。Task3两次bounded失败/中断、两次编译失败、Task2错误cwd/探索raw缺失、Task1旧raw缺失与Task4/5失败原上下文在完整报告和archive；缺失raw仍明示缺失，不补造。

大audit fixture100000-row RSS 157,536,256→32,075,776（-79.639%），100-row latency8.958573→11.813715ms（+31.8705%）、RSS+5.11%。基线oldcompile最初失败/中止143，cargo bench自动展开bins中止143，directsmoke4成功；optimized Criterion和threewaybuild未测。paging仍O(history) hashing、同一read snapshot增加WAL retention/non-WAL写阻塞风险；receipt/rusqlite仍full-vector。

## 4. 八项 parent rulings（必须完整永久保存）

前7项取progress Decisions；第8项取后续 Integration ruling8。这些是明确任务取舍/授权边界，不是代码推断或等待用户重新确认的理由。

| # | Binding ruling / 理由 | 代价与必须保留的 pending |
| --- | --- | --- |
|1|实施E1–E4核心，E5结构/facade/workspace重构保持conditional；plan要求先用测量证明收益 |可能需要后续一次独立本地重构；本交付不能冒称E5已实现 |
|2|旧rolling-head cancellation与历史monetary adjudication需各自human disposition；一般development授权不包含对unknown事实裁定 |生产block继续保留，直到人类明确处置。不能靠rollback/seed/delete/自动resolve清头 |
|3|watchdog采用现有可用local alerts，准备optional heterogeneous配置但不替用户选择新付费账户/channel |移动端告警仍缺credentials/channel；local spool非Feishu/phone receipt或用户阅读证明 |
|4|SELL/StreakLeader以read-only preview/research交付，qualified lots/independent trading facts缺失；E5 counted facade冻结 |v0不能发送qualified SELL reminder或建立可执行收益；现有public source ContractNotDelivered，future counted owner/deadline接线另做 |
|5|E3首交付采用measured full-prefix paging与真实veto benchmarks；private DB timing保留library-test seam；day check仅observational直到genuine capture/counting joins |full historical counted-replay、三个Criterion domains部分pending。原ReplayRunner force重写IDs，dry-run无counted authority；privateDB/admission不应为benchmark公开。Task5no-run可用test profile，优化Criterion runtime未跑即未测 |
|6|preview保留BR-234现有engine semantics，同时明确ATR单位冲突，promotion前需单独解决 |paper_sell ATR=mean(high-low) CNY/share，StopLoss按百分比使用；不能静默normalize改变交易风控。v0仍有已知legacy规则歧义，runtime前需targeted risk correction |
|7|optional bounded PhaseA仅支持明确reviewed当前DeepSeek non-thinking，MiniMax/legacy/unknown requested models pre-network refuse；legacy adapters不改 |bounded MiniMax比较等待独立capped thinking/billing review；DeepSeek trial需process-local model/role/endpoint/pricing/tokenizer framing/cash ceiling配置。known cache可验证但不默认discount；源自官方协议，不代表真实账单通过 |
|8|Task6必须修复真实默认join并用实际Rust producer0/76/4096fixtures验证：candidate CLI65536 input/131072 request，完整report独立有限cap按实测选择，manifest2MiB/output8MiB保持；保留全部qualifications/disputes、cash ceiling/rounded reservations/no-refunds、2attempts、time/body/contentcaps，默认offline0calls |更多有限input memory；更大的正常input reservation可能被较低用户ceiling拒绝，不能自动扩钱。默认registry正常三臂需localfake/preflight可运行；custom超大registry仍合理早拒绝。非E5/新authority，不能仅加免责声明关闭真实默认路径缺陷 |

Ruling8的数字来源是 `discovery-artifact-join-report.md`：source-transcribed default scorecard34857、base15007、with-outcomes system+prompt52475；4096窗仅key/punctuation下界2543616已超过旧2MiB，典型shape pretty window-container10257232。**这些不是Rust CLI实测产物尺寸或performance数字。** Task6须永久保留其计算方法/假设，再另外保存实际producer full report/manifest/comparison byte sizes、选定有限caps和三臂本地结果；不能用旧review的“允许大prompt降级”覆盖后来的parent ruling8。


外部验收：不把family缺失、source ContractNotDelivered、public observed flags、hash或local模型receipt升级authority。R1/R5、H08/PIT、real lots/fees/close、ATR歧义、mobilechannel、真实provider/framing/pricing/billing、20sessions人类价值、installed/live process/activation观察、historicalmoney/Uncertain均独立pending。详见archive当前报告/acceptance-map/最终发现索引；原discovery runtime只读观察带日期，不假称今天复验。

## Task6实际交付

已被替代的历史候选：`/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first/v20261009-f3761320c`（字节保持不变，不用于最终安装）。`manifest.json` 374,368bytes，SHA-256 `1cfbacd046a658ac7afd2417e77a5aa81126b395100b1b7bc857e2d204ec581c`；28allowlisted文件21,144,247bytes、977runtime preconditions、990source inputhash。source/CLI/resource/schema/Python3.9.6/toolchain/profile/explicitroot/每bin两次buildprovenance在manifest。prepared/staged，未安装/加载/activation/restart。

原weekly launcher412bytes SHA `dcd0a23ec6e5dbb1f7d447cf93c0f456dc0affda944565d183c6d4d7c0ab4ce6`、weekly plist1367bytes SHA `be8e22d72b7ac36ef26b17d7e839913de103e38529b0e9e386059eb827b06ed2` 在 `/Users/zhangzhen/.local/share/stock-analysis-candidates/rollback-v20261009-f3761320c` 的同basename文件0400保留；原watchdog plist absent。现有monitor/bridge/daily job/bin/source/config/activation仅read-onlyhash观察，无write。activation declared expected hash `a22dae891895e0082a44263b5e99eeb6dbb9b1fc95da8b19b6e9a6b90f754db7` 与 diskbinaryhash分列，非live process attestation。

实际Rust producer library13相关checks（13pass/6.85s）：0/76/4096 report73080/263255/10330539bytes，0/228/12288windows；原scorecard/qualification/dispute retained，localfake两modelarms/每fixture均完成，with-outcomes system+prompt52560/request55453，framing20→input52580；offline/zeroceiling0calls。report64MiB/manifest2MiB/output8MiB与CLIinput65536/request131072有限；unsupported/custom仍可拒绝。

实际release probe41checks（EXIT0）在foreign cwd及第二root：真实CLI0/76/4096 reportJSON73185/263360/10330644、reportMD56822/63769/63787、manifest每份19878bytes；comparisonJSON104313/104313/104315、MD113849/113849/113851。不同source标签/路径与offlinearm使bytes不同于libraryfake产物，不能混淆。真实wrapper→双format/0400rawregistry→assistant也通过；localfakewatchdog、全部sibling imports、exact versionpaths/plists、28文件hash/private/singlelink/exclusive/nooverwrite通过。otool显示五bin只依赖本机macOS system libraries，非跨OSportableclaim。

检查/失败历史：scoped lib首次E0425旧test常量名+E0063 fake缺reservation编译失败，修正后13pass；installer temp `/var` symlink fixture首次5errors，canonical resolve后5pass、最终provenance5pass/0.304s。first selected releaseEXIT0/15m14s、882既有library warnings；自查stdout输出cap缺口已修复。`cargo test --release --bin assistant_review` 由于devtokio test-util featuregraph会重编未改optimizedlib，已停止143，不算成功；唯一directrustc bin harness首次漏toml extern编译失败，补齐匹配f376 release deps后3pass/1.21s（stdout/file共同8MiB+1零write与exact8MiB正例，原2bin回归）；normal assistant-only rebuild同rootEXIT0/3.93s。其余源码/lib未改，13lib检查复用。

HTML --draft/render和--draft --check均exit1 `file_set_mismatch pair=current`；root已证实base228 source915 vs冻结manifest548（367extras）、当前932（384extras），继承的architecture CI gap。current/frozencatalog/renderer不扩展，旧HTML保持stale/provisional，不能宣称所有CI绿色。root只读证据task-6-preexisting-catalog-evidence.json在losslessarchive。

立即查看保留的synthetic离线示例（不证明生产来源/收益）：

```sh
/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-final-fix/goal-first/v20261009-f3761320c/bin/assistant_review --report /Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-evidence/20261009-task6/release-probe/cli-review-76.json --manifest /Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-evidence/20261009-task6/release-probe/cli-manifest-76.json --registry /Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-final-fix/goal-first/v20261009-f3761320c/resources/signal_registry.toml --as-of 2026-10-08T16:00:00+08:00 --completed-session 2026-10-08
```

最终候选默认installer的已跑plan命令为 `/usr/bin/python3 -B /Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-final-fix/goal-first/v20261009-f3761320c/scripts/prepare_goal_first_delivery.py --bundle /Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-final-fix/goal-first/v20261009-f3761320c`，只hash/check/输出plan；显式install仅原子发布version，job持久安装/加载仍独立步骤，见RUNBOOK。无重复weekly、RunAtLoad=false、无初始catchup；新watchdog60s/RunAtLoad=false使用已有早期只读health路径。

Losslessarchive实际保存并全member复核：117members / 12873491 raw bytes；archive 3584946bytes / SHA-256 `d426bf8304a44f279cdf351850b28e364fe1a572b80fd5cd6c53ef99555a4eb5`。逐memberlength/SHA与原路径在该root/review-evidence-manifest.json。原plan26216bytes/SHA `a0185c672e2f7932c963c0c9820de9f2601f3c442a982956ca331e5b5184c702` 与original-plan private copy/archive member核对一致。

最终收尾时，由主任务追加保存 Task 6 后续审查、全分支最终审查与本地集成证明：[最终补充归档](/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-evidence/20261009-task6/late-review-evidence.tar.gz)及[逐文件长度与哈希清单](/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-evidence/20261009-task6/late-review-evidence-manifest.json)。这些文件将在最终收尾时写入并核验，最终结果以其中记录为准；此处不表示全分支审查或集成已经完成。既有不可变归档继续保留。

最终修复候选：`/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-final-fix/goal-first/v20261009-f3761320c`。manifest375227bytes/SHA-256 `eebdc2199d2890d5fa9955d5929103c1eb07cfc2fed30823f8f975e20a34080f`；28allowlisted payload files21,156,899bytes，含manifest共29文件21,532,126bytes，977preconditions/990source hashes。全局源码/packager `180248b02ad672a3eefa1131b2c06a60154514e1`；weekly实际第三次构建 `1dc5e6d877ecea4b9edea75b388381beab0e18f7`，assistant仍1ce，其余三bin仍f376。版本名仍是首次构建标签；计划目的地和编译root均未改变。原bin来源与实际运行源码身份不混记。

F-Q01两处registry入口已使用单descriptor的regular/no-follow/nonblocking/128KiB+1/stability读取，0644原TOML/rawSHA/embedded default兼容；F-R01把借用模块例外严格固定为已审查完整blob SHA，helper/tail名称shadowing不再能通过。旧schema/parser/default/family mapping原字节保留。weekly29pass；Python wrapper+installer19pass（首次两个test fixture错误记录保留）；shared assistant21pass/编译7m15s/执行7.31s；唯一weekly release第三构建EXIT0/7m22s，确实重编library。pin后仅installer6pass/0.367s，无Rust重编。

[最终修复证据](/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-evidence/20261009-task6/final-fix/pinned-package-check-results.json)包含最终各文件/来源核验和default/relocated plan各EXIT0；此前13项实际新weekly/wrapper/76预测双格式→离线assistant/FIFO等有限3秒检查复用，weekly与wrapper和所有其他包文件（除packager/manifest）SHA与执行版本一致。修复后weekly6847612bytes/SHA `5e64691601c84907486d9a88d23462788ab849669d96a22ef202aa21b6dc3168`；wrapper10519bytes/SHA `fc8c1dfa59fed61d0c36836e59e78d74556a7e3fa63e35561af119180e0498cb`。原四bin/resources/plists/launcher身份不变，来源fixture未改，默认0模型calls；旧41项其他包检查按未改字节复用。原全部证据/117memberarchive没有改写。

修复中间包（13probe实际执行时仍在final-fix路径）仅在执行结束后整树迁至 `/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-final-fix-pre-pin`；保留manifest375024bytes/SHA `664a47e45809a07c4e6088348b6fadc53f42d7bfc1f11b6084b087beca9f8a59`。精确旧→新映射与probe日志/结果身份见[迁移记录](/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-evidence/20261009-task6/final-fix/interim-relocation.json)，历史命令中的原执行路径未改写，不宣称曾在迁移后路径执行。最终rollback副本在 `/Users/zhangzhen/.local/share/stock-analysis-candidates/goal-first-final-fix/rollback-v20261009-f3761320c`；字节与原保存一致。所有包均未安装/加载；最终scoped rereview、本地集成与late supplement待root收尾。
