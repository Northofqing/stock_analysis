# R03产业链复盘：自动、手动与原消息恢复

日期：2026-09-14。主控全文读取独立报告、复核8份主要源码/目录摘要并直接核关键入口。新增3个Unit的静态追链，累计22个已追链、30个待追链；不是22个已迁移。未运行monitor、查询生产数据库或改变账户门与发送策略。

## 三个独立Unit不能合并计数

| Unit | 实际入口与状态归属 | 当前能否产生新的R03推送 |
| --- | --- | --- |
| MU-review-r03-auto | 常驻复盘schedule按日维护tasks[R03] | 账户阶段无条件返回AccountMetricsIncomplete，不调用R03来源/渲染/发送 |
| MU-review-r03-manual | --review单次调用及临时audit_state | 经过相同账户阶段，同样不能产生新R03通知决议 |
| MU-review-r03-stored-recovery | 启动恢复既存原业务日IndustryChain决议 | 只能处理已存原信封，不建立新R03业务分析或通知决议 |

归属由[机器目录](push-capability-catalog.v1.json#L9312)分别列出；[manual](push-capability-catalog.v1.json#L9326)与[stored-recovery](push-capability-catalog.v1.json#L9484)拥有不同状态。旧IndustryChain的BusinessDateOnce政策存在，不等于两个新消息入口已经接通。[通知政策](../../src/durable_delivery/model.rs#L447)

## 自动入口：开窗后仍停在账户阶段

1. 交易日19:00起每60秒检查；schedule按当前本地日期建立。实际attempt使用at_manual取得最近已完成交易日，manual_override=true。[时间门](../../src/bin/monitor/main.rs#L5656)、[attempt](../../src/bin/monitor/main.rs#L5717)、[常驻循环](../../src/bin/monitor/main.rs#L6122)
2. R03注册为LegacyAccountGate，进入account_required列表；生产循环直接为每个此类任务生成AccountMetricsIncomplete，没有读取账户批次或条件放行。这不是“行情偶尔缺失”的运行态结论，而是当前代码的无条件分支。[分类](../../src/bin/monitor/review_batch.rs#L418)、[分组](../../src/bin/monitor/review_batch.rs#L1138)、[阻断点](../../src/bin/monitor/push_templates.rs#L10072)
3. 失败固定在AcquireBatch，retryable=true，provider/source_time/evidence缺失；没有匹配的旧决议恢复结果时，自动调度按1/5/15分钟退避。[失败类型](../../src/bin/monitor/review_batch.rs#L800)、[退避](../../src/bin/monitor/review_batch.rs#L1236)
4. 当前dispatcher未调用dispatch_r03_industry_chain_outcome，结构测试也明确验证该调用缺席。源码中具名函数虽存在，但仅有包装调用及测试相关引用，不代表实际生产接线。[结构测试](../../src/bin/monitor/push_templates.rs#L12270)、[休眠函数](../../src/bin/monitor/push_templates.rs#L12584)、[包装](../../src/bin/monitor/push_templates.rs#L12789)
5. 原批次的其他任务及prediction回填、大宗交易、IPO侧路可执行，不能把它们的成功算作R03完成。[批次侧路](../../src/bin/monitor/push_templates.rs#L9891)

自动循环在计算due之前应用durable恢复结果；attempt之后再对克隆schedule应用一次，剔除已有durable证据的任务，剩余legacy审计成功后才提交内存状态。[调度次序](../../src/bin/monitor/main.rs#L6244)

## 手动入口：批次成功不代表R03成功

--review一次请求13项任务，使用最近已完成交易日，之后另建临时audit_state。它不推进常驻schedule，也不豁免账户依赖，R03仍停在同一无条件失败分支。[手动入口](../../src/bin/monitor/main.rs#L5527)

与自动入口不同，手动先跑dispatcher，后读取durable恢复结果。后者只把对应任务从legacy转移审计中排除，避免双写，不会把原batch内R03的AccountMetricsIncomplete改成Delivered。CLI根据原batch的Complete/Partial返回Ok，NoDelivery返回Err；整批Ok不能作为R03的发送证据。[结果处理](../../src/bin/monitor/main.rs#L5544)、[CLI批次结果](../../src/bin/monitor/main.rs#L5574)

## 启动恢复：只处理既存原信封

1. 普通启动绑定核心DB之后、激活producer之前执行ensure_startup_reconciled；失败阻断激活。P01独占补偿命令有独立的跳过规则。[启动门](../../src/bin/monitor/main.rs#L4810)
2. 恢复扫描全部日期的pending决议，不受交易日、19:00窗口或LegacyAccountGate限制。每个可恢复决议在本次fixed-point中至多调用一次resume_deliverable；继续重复出现则报错冻结producer，不形成发送重试循环。[恢复循环](../../src/bin/monitor/durable_delivery_runtime.rs#L2074)
3. R03必须已有精确IndustryChain/None/GLOBAL、原业务日及review_task_identity(date,R03)的immutable决议。恢复只消费原信封，不重新读取账户/龙虎榜/题材来源，不渲染替代正文，也不创建新claim。[映射](../../src/bin/monitor/durable_delivery_runtime.rs#L1539)、[原信封恢复](../../src/bin/monitor/durable_delivery_runtime.rs#L2113)
4. 恢复结果排队供对应日期schedule消费；任务身份、日期和basis/hash必须匹配，其他日期保留，不套用到今天。[消费与确认](../../src/bin/monitor/main.rs#L5465)、[绑定核验](../../src/bin/monitor/review_batch.rs#L1433)
5. R03不在“最近5个历史交易日、8项复盘任务”的补推集合里，没有无claim时按原日重建的新第四入口。[补推集合](../../src/bin/monitor/main.rs#L5735)

## Terminal与Delivered必须区别

durable hydration接受Accepted、Rejected、Uncertain、ManualRejected四种disposition，并统一把内存TaskScheduleState置为Terminal；is_due/has_unfinished_tasks据此停止常规调度。负面或未决结果也可以终止自动尝试，这不是已送达。[四种分类](../../src/bin/monitor/review_batch.rs#L1534)、[状态更新](../../src/bin/monitor/review_batch.rs#L1546)、[due判断](../../src/bin/monitor/review_batch.rs#L1575)

因此不能把“调度已无待办”“启动恢复成功”“--review部分完成”任一项升级为R03通知成功，更不能将Uncertain通过改名或恢复后重跑变为Delivered。旧消息恢复可存在而新的R03消息生产仍完全不可达。

## 后续改动与验收方向

- 真实账户批次接线：用完整、可验证的账户输入替换无条件失败，保留缺失/陈旧/不完整拒绝，不能删除门或伪造健康值来使消息发出。测试应走实际批次接口，分别验证可用、不可用和部分输入。
- 区分停止调度、等待人工处理、已确认发送；保留四种durable处置的原语义，以跨日和负面恢复反例验证，不自动重发Unknown。
- 分别验收auto/manual新消息路径与stored-recovery原信封路径，保持三种状态归属，防止同名R03被错误合并为共同完成权。
- 将来接通休眠dispatcher时，需补完整来源绑定：它目前的canonical仅(date,count)，未包含完整候选、涨停池批次与accepted/rejected材料。[来源绑定](../../src/bin/monitor/push_templates.rs#L12762)
- 休眠实现的positions/watchlist、前20候选、涨停池匹配、rejection audit与前5条chain渲染仅属潜在接线行为；当前新消息入口不可达，不能把这部分源码描述为生产实测。[潜在流程](../../src/bin/monitor/push_templates.rs#L12600)、[聚合](../../src/market_analyzer/limit_chain_review.rs#L37)

本报告只完成业务追链，不授权改账户政策、生产启用或重发。完整52个Unit、单用户事务/任务锁/防重和发布回滚验收保持，首批逐步替换顺序不变。

## 核查时主要输入SHA-256

```text
docs/push-system/push-capability-catalog.v1.json 0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3
src/bin/monitor/main.rs c87fa4bb56df1c786e11516f754316e0a3fb9d9f5f75aad8aa2a7cb02016488c
src/bin/monitor/review_batch.rs 8006c7bc6143bd410087b832c81636bc81bb1c047709fd0b0eb100ee77a56b2c
src/bin/monitor/push_templates.rs 5096e5697f67338b45346d11efa8f9ec988774c17a8f9623b280adeaeb2044f4
src/bin/monitor/durable_delivery_runtime.rs 53df56b8078453e9ab141766efed8ff72709ac821298adfd4f05bdeb4c92ec78
src/bin/monitor/notify.rs ce2112226a4c71cb12426edd151444e239c4844ccf50d49252a67be3af7c7bfa
src/durable_delivery/model.rs b14f970f6c3d28123d97d287564aa870eb07bead06c2dcf44ac2be536c86d0b9
src/market_analyzer/limit_chain_review.rs 4c7011c3841db29bfc56d622b04e3c544d22e306598f5e8d4c0ca49410ee6301
```
