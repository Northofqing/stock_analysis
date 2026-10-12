# 四路新闻采集期限：精确激活审阅单（2026-10-12）

**待人审、未安装。** 依据 [当前工作范围](../handoffs/2026-10-07-active-scope.md)，本次窄候选用于 monitor 新闻采集稳定性。仓库 `selection_activation_prepare` 要求人工 review，旧 Wave0/Wave1 或 5fb81 激活批准不用于本候选。

## 精确候选

| 材料 | 身份 |
| --- | --- |
| 构建源码提交 | `90470ac7f6d2da5bc85922ed2e9e8f60ab46088f` |
| 行为修复提交 | `b6063450677496d28a6ef3c4e1418daec4f10f31`，已合入 master 并推送 |
| 1001项构建输入清单 SHA-256 | `40a7cefaf08d3d7202818d5016733baebe5b680e6e914f5863bcfa25d55a63e9` |
| 正式 monitor | 52143552 bytes；SHA-256 `75aedce1d420872d2c7cd33ca4df17c467479ad09f4c988e9960315d470da572` |
| 正式 library | 233065936 bytes；SHA-256 `96f8a413c88cd58ce1eda23a5323b40e72a46a21a51996557e1fb488f2b1d5c4` |
| 预期配置哈希 | `3a797e83765e98eb9ef60d6a8f0140ff5504ea39b28b633e54ba5c40a35b35d2` |
| activation候选 | 265 bytes；SHA-256 `3cd65aa50f73776463c8b7d2b77fa51dd93db63701aca1f196233489b2c356c0` |
| 候选署名 / reviewed_at | `zhangzhen` / `2026-10-12T01:00:45.827661000Z`；该预填字段不表示人审已完成 |
| effective_from | `2026-10-13T01:00:00.000000000Z`，即 **2026-10-13 09:00 CST** |
| 原板块材料到期 | 2026-10-22 20:00 CST；未重封来源材料 |
| 正式编译运行根 | `/Users/zhangzhen/.local/share/stock-analysis-runtime` |

私有封存目录：`/Users/zhangzhen/.local/share/stock-analysis-deployments/20261012-raw-news-deadline/`。候选正文、1001项完整输入、正式制品、历史失败、typed回执和回退均在此目录，具体文件为 `selection_activation.candidate.v1.json`、`build-input-manifest.json`、`sealed-production-artifacts/`、`candidate-preparation.json`、`cutover-and-rollback-recipe.json`。

## 改动及已验证范围

与现网999个既有静态输入逐件比较，只有以下3项变化：`src/news/aggregator/raw_v2.rs`、`src/news/aggregator/raw_v2_breaker_tests.rs`、`src/data_gateway/grpc_source.rs`。四路原raw acquisition各加35秒cooperative期限，超期完成原breaker失败记录并报 `global_news_acquisition_timeout`；原成功、typed error、10次阈值/60秒冷却、数据身份和投递守卫保留。

- 原最终源码46项定向方法通过；6个相关输入SHA未变，本轮复用原RED/GREEN证据，没有重复跑测试。
- 实际正式root的 release library+monitor：exit0，567.96秒。封存制品SHA与上表一致。
- 实际正式monitor在自身私有test root执行 `--test --push-dry-run`：exit0，2.72秒；日志有正确test root及完成标记。没有手机外发验收结论。
- 初次封存漏掉两份被编译的 docs/push-system 文件，release exit101保留；补入与原5fb字节一致的文件后修正构建通过。真实构建清单为1001项。
- 正式root库直接读取stagingroot，被 `diagnostic_repository_root_release_forbidden` 正确拒绝。另编译绑定自身私有候选根的诊断library，仅用于同源码typed材料准备和验证（exit0）；它不能安装为现网库。正式制品独立封存。
- 当前正式root的只读typed核验器已读回原配置哈希及旧激活门enabled（exit0），没有打开DB或发RPC。**安装后必须用正式root库重新核验上表精确配置哈希与真实门状态**，诊断预览不能代替此步骤。
- 完成本轮限定源码/prepare helper自审，未另启独立agent复核；不冒称全库测试、同版真实业务RPC或恢复验收。

35秒只覆盖可取消的四路raw异步采集，不是同步I/O硬中断、全部新闻循环截止或Windows worker停止证明。上线后需要同boot自然采集时长、明确原因及breaker恢复观察。

## 当前生产及切换条件

09:02 CST末查：旧monitor PID89475 / SHA `d459ec8e566c460aef45742e431fe87439c25c6ab83ef0b201af61850dcbcc8e`、独立bridge PID88699 / SHA `a13ed075def5896cc9226ef99b78a1adc51f2faa9834cd391a73149d3ad913de` 仍在运行；原activation SHA `2b2ca75f96f4ca8663073320554edc9bca69ad566edfd1c2b4fbdcdcffd40069` 未变。实际运行根、DB device/inode/schema、paper head/events前后相同；76条Uncertain保留。health仍Frozen/Unsafe，缺Quote/Kline/MoneyFlow/News/OrderBook。

WindowsV58报告仍未恢复gRPC，历史V13关键覆盖率92.4653%低于95%，新增源码尚无最终测量。本候选不会提供缺失的历史/价格/实时来源资格，缺来源继续明确拒绝或deferred。H08合格回填和收益观察仍等待真实数据。

批准后执行精确切换：重新读回当时source/完整材料、正式PID/executable与单实例锁、原DB/账户head、durable未决身份和来源边界；任何新发布、材料/认证变更或无法满足的门禁先停止切换。生效前按原launchd流程停旧monitor和bridge，精确安装已审3份静态输入、monitor及activation；用正式root核验typed哈希，在生效时点到达后先验证原制品bridge，再启动monitor并等待初始化fixedpoint、同boot心跳和真实业务边界。

回退材料已封存：旧3份源码、monitor、activation及两份plist。失败时按同一单实例顺序恢复静态文件和正式jobs，保持独立bridge原字节、CLI/tools及生产数据库。不得恢复live DB文件、重复账户初始化、自动重发或裁定76条Uncertain。

**如果批准或动态检查错过该生效时点，保留旧实例，不沿用过时候选。** 后续新候选需另行精确复核。批准此表即只批准本表元组；不签发Windows恢复、行情资格或outcome收益完整性。
