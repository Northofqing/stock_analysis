# 龙虎榜复盘 MU-review-r04：四个真实入口与盘后产业链的区别

2026-09-13，隔离树HEAD b3742ab；静态源码核对，未运行monitor、真实provider或生产库。独立只读报告的14份文件SHA经主控复核，关键入口、dispatcher、准入和完成分支另由主控直接阅读。本记录新增一个已追链Unit，不代表它已经完成新框架迁移。

## 四入口共用什么

目录登记的四producer为review-r04-auto、review-r04-manual、review-r04-backfill、startup-resume-review-lhb。前三条在需要新准备时进入同一个R-04 dispatcher；启动恢复消费已存通知信封，不重跑dispatcher/provider。它们按原复盘业务日共用ReviewLhb的通知claim，不与盘后产业链的CHAIN_POST_LAST共享完成权。

| 入口 | 实际触发/日期 | 完成与恢复 |
| --- | --- | --- |
| 自动 | 主循环启动独立scheduler，每60秒、交易日19:00后；先恢复持久调度事实，再取due任务 | 返回后clone→再恢复持久事实→排除durable任务→处理旧结果→写审计→更新state |
| 手动--review | 取全部复盘任务，以当前时刻选择最近已完成交易日，明确manual override | Complete和Partial都返回Ok；不能据CLI成功断言R-04独自送达 |
| 历史补推 | 启动就绪后及每日窗口各触发扫描；最近5个历史交易日，不含今天，旧到新，8-task集合含R04 | 先恢复原date/task claim；无claim才重新准备；Delivered跳过，RejectedDurable经代码显式授权后恢复，未知状态不盲发 |
| 启动恢复 | core DB绑定后、常规producer前，覆盖全部已有业务日 | 最多100轮本地恢复；Reserved或已授权RejectedDurable消费原信封；活跃外部lease及UncertainManualReview保留，失败阻止producer继续 |

证据：[main的启动/手动/自动/补推入口](../../src/bin/monitor/main.rs)4810、5071、5366、5502、5717、5735、5930、6122、6244；[运行日期](../../src/bin/monitor/review_batch.rs)6–94；[启动恢复](../../src/bin/monitor/durable_delivery_runtime.rs)2074；[可投递状态分类](../../src/durable_delivery/coordinator.rs)6271。行号对应下列冻结SHA。

## 新准备的业务链

1. dispatcher由context确定业务日及资格时钟，SourceOnly首阶段包含R04。已有当日R04 durable decision时直接解释，不再取来源；没有记录且资格时钟达到21:00才调用来源。[dispatcher与R04](../../src/bin/monitor/push_templates.rs)9891、13147、13169。
2. 来源是DragonTigerGateway::market_review(复盘业务日,5,5)。Gateway经同一DragonTiger桥并写一次R-04采集审计，**不是盘后产业链的100/5000请求**。[Gateway](../../src/data_gateway/dragon_tiger.rs)55；[R04 loader](../../src/bin/monitor/push_templates.rs)13158。
3. Available还要求Eastmoney、source/batch非空、source_at解析日期等于业务日、observed_at可解析；证券净额正且有限，披露身份有效，每条披露完整买五卖五且金额有效。VerifiedEmpty成为NoData；Gateway错误按typed retryability失败；binding/token不合法永久失败。[准备校验](../../src/bin/monitor/push_templates.rs)12907–13040、13242。
4. 独立renderer保留每条TRADE_ID及席位；counted binding固定原业务日、task occurrence、源材料和正文，经launch/v14门进入durable runtime。实际通知完成权在immutable decision/BusinessDateOnce claim，不在Gateway或schedule state。[通知入口](../../src/bin/monitor/notify.rs)2935、2998；[durable运行时](../../src/bin/monitor/durable_delivery_runtime.rs)2126、2199。
5. 调度Terminal还包括NoData、Disabled和永久Failed；可重试失败按1/5/15分钟等待。已恢复的Rejected/Uncertain也可能停止调度，但不是Delivered。[调度状态](../../src/bin/monitor/review_batch.rs)1208、1247、1549、1560。

## 已发现的边界与待处理项

- **自动入口与21:00注释不一致。** 自动scheduler实际调用attempt_post_session_review；该函数使用at_manual，dispatcher因此把资格时间置为23:59:59。Pending任务从19:00可进入R04请求；不能按注释声称自动一定等到21:00。这是源码条件事实，尚未生产复现；本次来源接线不顺带更改时间政策。证据：[main](../../src/bin/monitor/main.rs)5717、6179、6277；[override](../../src/bin/monitor/push_templates.rs)9903；[Pending资格](../../src/bin/monitor/review_batch.rs)1567。
- **单任务补推仍有额外效果。** backfill无claim时只把R04放入due，但非测试dispatcher后段仍执行预测回填、大宗交易、IPO侧路；它不是无额外效果的“只取龙虎榜”函数。[代码](../../src/bin/monitor/push_templates.rs)10032–10071。
- **不能借R04完成权给chain。** chain按请求自然日用100/5000，可选来源失败降级Unavailable、只投影净额万元；其旧mode直接保存报告并发送，false/Err仍可能返回Ok再封CHAIN_POST_LAST。复用R04 renderer/claim会改变请求、降级、消息类别及当日去重归属。[chain来源](../../src/pipeline/chain_analysis/fetchers.rs)205–257；[mode](../../src/app/modes.rs)157；[timer](../../src/bin/monitor/main.rs)8758。
- **有旧durable不等于完成新迁移。** R04已有通知信封恢复；尚无本次逐Unit六门禁/实际切换/自然时段观察证据。chain的新持久适配器仍在DragonTiger处显式停止，[龙虎榜恢复验收](dragon-tiger-recovery-validation-2026-09-13.md)单独推进。

## 可复核源码身份

以下是本记录的关键冻结文件SHA-256，源码改变后须按实际差异重新判断，不用HEAD相同推断文件相同：

```text
c87fa4bb56df1c786e11516f754316e0a3fb9d9f5f75aad8aa2a7cb02016488c  src/bin/monitor/main.rs
8006c7bc6143bd410087b832c81636bc81bb1c047709fd0b0eb100ee77a56b2c  src/bin/monitor/review_batch.rs
5096e5697f67338b45346d11efa8f9ec988774c17a8f9623b280adeaeb2044f4  src/bin/monitor/push_templates.rs
ce2112226a4c71cb12426edd151444e239c4844ccf50d49252a67be3af7c7bfa  src/bin/monitor/notify.rs
53df56b8078453e9ab141766efed8ff72709ac821298adfd4f05bdeb4c92ec78  src/bin/monitor/durable_delivery_runtime.rs
5af5abb067a7979f5b05ae0991316270a3231117716484c9f05fd0632420441b  src/durable_delivery/coordinator.rs
bfcbccee66e0ee948a0eb5b3b724496b16588570e8de22101be8ea81f5133069  src/data_gateway/dragon_tiger.rs
```

没有读取历史decision、真实部署状态、远端接受或用户已读；也没有修改上述业务代码、冻结catalog、RFC或架构蓝图。
