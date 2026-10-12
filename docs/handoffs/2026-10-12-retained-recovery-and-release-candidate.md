# 保留工作：上游恢复与窄发布候选（2026-10-12）

范围继续以 [10-07裁定](2026-10-07-active-scope.md) 为准。源码修复已提交，本轮完成release和具体审阅材料；**尚未生产安装**。

## 本轮交付

[精确激活审阅单](../ops/2026-10-12-raw-news-deadline-activation-review.md) 已完成。构建源码90470ac7f、行为b60634506，正式monitor SHA `75aedce1d420872d2c7cd33ca4df17c467479ad09f4c988e9960315d470da572`；1001项输入、普通release、隔离dry-run、同源码诊断typed校验、正式root旧配置核验及兼容静态回退均封存。新配置哈希 `3a797e83765e98eb9ef60d6a8f0140ff5504ea39b28b633e54ba5c40a35b35d2`，候选SHA `3cd65aa50f73776463c8b7d2b77fa51dd93db63701aca1f196233489b2c356c0`，候选生效10-13 09:00 CST。待精确人审及切换前动态复查。

源码/依赖未改，复用原46方法及真实RED2失败记录；本轮没有追加全库测试。两项本地准备失败（遗漏2份embedded docs、正式根拒绝staging）均保留并纠正准备流程。诊断library单独绑定私有候选根，正式monitor/library仍绑定原runtime；原守卫和源码保持。部署后仍须正式root typed验证，不以预览代替生产门。

## 最新事实与Windows

- 09:02末查现网仍为5fb81f1f：monitor89475、bridge88699、旧制品及activation未变。心跳新鲜，health exit1/Frozen/Unsafe；四路raw仍 `external_transport_unavailable`，health曾因漫长循环报snapshot_stale。paper head/events及两DB身份/schema未变，76 Uncertain不处理。
- 10-11自然日任务21:17–21:48:50，selection0/daily0/prediction1/wrapper1；verified0、13deferred、24deferred windows。id37–40原7/11–12周末起点及原错误保留，未手工重跑或改日期。
- WindowsV54–V58原公共文件实际读取/记录hash。V13完整测量在对端验核：overall86.1093%，critical92.4653%<95，当前该历史分母还需1013行；raw报告SHA `f1ea129a8d48b1f02953d055a4d422fc2f2bcb1f5e0ffd5e7e4dd368148c965f`。Mac读的是公开状态回执，未收到完整28MB raw/report/source归档用于逐件独立审计。当前新增源码/Smart回归还需新正式测量，旧分数不是当前95通过。
- V57/V58报告Windows常规系统信任选择已获该原任务直接人审，并准备非默认SDK test-support；不是Mac私有执行、人审/secret handoff或服务恢复回执。本任务只读，未运行旧synthetic receiver、处理Token或改信任配置，也不重复已解决/已有挂起问题。
- 新续办消息的应用失败仍保留。但V17精确回执2604B/SHA `718cd1fa3f0bc5fa81712e0a5efd2ff48673a0aa54d7267bf9aa2cab25d6faf4`，确实消费原Mac包12394B/SHA `2d620ebcf5db155f70adc28c06e2139f6ed577fe17f1b05821635e92209027e2`；V20再确认。**文件消费已闭合，应用message送达不反写为成功。**
- 本轮两次原Windowschat只读分别host unavailable/timeout；共享公共状态仍有更新。新增公共ACK `MAC_RETAINED_RECOVERY_AND_DEADLINE_CANDIDATE_ACK_20261012_V1.json` 3902B/SHA `21a57999838837617e5ad9b71e0e6a51389ba6002f4992949c6574f2c797ae1c` 已写入并读回，对端消费尚未确认；未重复发送应用消息。

## 接续顺序

1. 当前本地慢采集故障的窄候选已可审；批准后重查精确hash/时效/原来源与单实例、账户和Uncertain边界，按原流程切换。错过生效时间不安装旧候选，必要后继另审。取得同boot自然期限及恢复观察后再确认稳定性落地。
2. 沿原Windows任务继续已有授权的关键回归、真实95%、完整native release及受限认证交接；需要新source/descriptor/executable静态身份与同版真实RPC，不从响应自学expected身份或以Health代替消费者接纳。
3. H08合格范围到位后再有界回填原候选/预测；缺来源、未成熟和历史错误保留unavailable/deferred/NULL。新工具自然周报仍待10-16。

保留monitor日常维护，冻结平台工程不恢复。既有账户初始化、19条恢复和4条已获人审取消不重做；没有生产安装、DB回填、新人工裁定或手机测试推送。

私有计划及证据：当前工作树 `.planning/2026-10-12-retained-observation/`；候选/回退在 `~/.local/share/stock-analysis-deployments/20261012-raw-news-deadline/`。primary两份Architecture Blueprint未提交改动属于其他工作，不纳入本轮提交。
