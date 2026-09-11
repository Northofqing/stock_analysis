# 盘后产业链：真实结果、持久恢复与完成接管的设计边界

日期：2026-09-11。初始源码6ddbf51。本文件是后续实施约束，不是已经修复或生产验收。

## 当前正在推进什么

[逐目标观察计划](../superpowers/plans/2026-09-11-notification-attempt-observation.md)已在初版bd143fe、最终修复de38876保留真实通知方法的结果：一个 Custom URL 配置项对应一次原有调用；true是弱接受，false/Err保守保留Unknown。旧send继续委托同一次真实发送并投影“任一弱成功”，不产生第二次发送。修后16项定向测试、静态检查及限定复审通过；仅该前置Task完成，以下持久恢复和真实timer接管仍待实施。

原因是[现有盘后调用链](chain-post-close-call-chain-2026-09-10.md)把通知false吞成Ok，再推进CHAIN_POST_LAST；直接改为Err会让下一tick重跑来源、模型、chain_daily写入和发送。微信/飞书的方法失败还可能包含已接受的早先请求，不允许据此整包重发。

## 三种事实必须分开

| 事实 | 可证明 | 不能证明 |
| --- | --- | --- |
| pipeline返回报告并保存成功 | 这次业务分析/本地文件步骤完成 | 通知已到达、完成游标可推进 |
| 旧渠道true / 逐目标弱观察 | 对应渠道方法报告成功，可保留部分成功与未知 | 已注册必达集合完成、强终态、用户已读或重启幂等 |
| 绑定的强authority终态及注册完成策略 | 重验后可申请相应owner的最终化 | 其他Unit也完成、别的渠道可借用该回执 |

现行“任一成功”bool与RFC的“BestEffortAccepted=全部配置目标弱接受”不是相同条件。部分弱接受时旧bool仍可为true，但不得把它当成TransportAccepted。完整目标不通过重命名这些状态完成。

## 已核对的可复用机制与缺口

- [兼容结果类型](../../src/monitor/push_job/delivery.rs#L285)可保存绑定后的配置目标、实际尝试和弱结果，但需要真实intent/Unit/occurrence等输入；本次调用内目标序号不满足跨重启身份合同。
- [兼容完成分支](../../src/monitor/push_job/policy.rs#L970)明确保持KeepOpen、CursorNever、NeverRetry。不能把弱观察写入后就强行标Completed。
- [业务intent存储](../../src/push_foundation/intent_store.rs)已有初始不可变快照、报告原字节和CAS/恢复检查。规范快照对部分外部事实仅保留摘要，并非完整原采集/模型内容档案；目前没有完整的报告保存进度和逐目标弱尝试journal接口。
- [现成强通知运行时](../../src/bin/monitor/durable_delivery_runtime.rs#L1073)使用magiclaw-cli-typed-v1；IndustryChain路由[绑定R03](../../src/bin/monitor/durable_delivery_runtime.rs#L1549)，不能当作盘后timer的现成完成owner或十渠道通用适配器。

因此下一纵向计划应在已明确指定的业务库中受控扩展，而不是添加第三套旁路数据库。schema/权限/不可变保护必须另列合同，不能悄改冻结Foundation DDL；生产DB路径和部署不在本次操作范围。

## 后续实施顺序与验收

执行入口：[盘后持久恢复实施计划](../superpowers/plans/2026-09-11-chain-post-close-recovery.md)，基线42ce098。已开始真实pipeline固定准备结果Task；存储检查点、timer和强完成仍分项待验收，下列完整目标不变。

1. 固定盘后独立occurrence、业务日、来源与首次模型结果/报告，保留真实降级原因；在实际产生事实的入口捕获，不从报告反推来源。盘前、CLI和R03不共用完成凭证。
2. 将准备artifact与保存进度接入同业务库。恢复读取原事实和原字节；已存在不同内容的文件是冲突，不覆盖。chain_daily等前置业务写入也要有对应进度，不能以重跑pipeline代替恢复。
3. 网络调用前持久记录开始状态，逐目标返回后追加弱观察。启动记录后崩溃、结果写入失败、部分成功或Unknown均保持未决；重启不能推断“没发过”。不得自行补发剩余目标或整包重发。
4. 实际timer/启动入口消费该流程；只在交易日15:30≤t<15:35获得新工作资格，进入前重查时间，不借先前tick的旧now过窗启动。窗口外仅恢复原事实，新补偿必须有独立授权。
5. 按真实渠道/必达策略和认证注册接入强authority、共同owner隔离及盘后独立cursor。接受后最终化失败只恢复最终化；不得重发。cursor、业务状态和稳定事件在同业务事务提交。
6. 逐Unit取得unit/failure/crash/shadow/dedup/rollback门禁、独立审查与实际发布证据。仅弱观察或仅内存缓存通过，不关闭完整迁移条目。

上述代码可在显式Test namespace、合成来源/模型、本地HTTP和临时业务库中推进。真实生产身份/受保护根、必达渠道与实际路由、生产schema部署及owner切换批准仍是独立要求，不由测试构造器补齐。

已向用户异步询问盘后实际哪些渠道必须成功；未回复前不改变旧渠道和任一成功投影，也不因此暂停独立工程。完整52 Unit及W15–W21目标保持未完成。

## 本轮额外发现：飞书长报告分段前提

当前[飞书格式化](../../src/notification/feishu.rs#L144)先转换普通Markdown分隔符/标题标记，再调用只识别原标记的[分段器](../../src/notification/wechat.rs#L93)。对公开发送入口的普通长报告，不能据内部chunked方法名推断实际会分成多片；目前会进入单section截断。逐目标观察任务的真实入口测试已经验证此特征及card→text回退，同时末尾空标题反例证明并非所有输入都不能多片，详见[实施记录](implementation-notification-attempt-observation-2026-09-11.md)。未修改原算法；飞书普通长报告完整原文分片另列后续问题，弱成功不能声称原报告完整送达。
