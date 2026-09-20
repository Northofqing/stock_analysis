# MU-review-r13 / MU-review-a10：名单生成、次日核对与恢复缺口

日期：2026-09-13。主控已全文读取独立报告、核对16项输入SHA及下列关键源码。新增两个Unit的静态追链，累计19个已追链、33个待追链；不是19个已迁移。没有查询生产数据、运行monitor或改变发送策略。

## 入口与完成身份

R13使用`WatchlistTracking`，A10使用`CatalystReview`；均为GLOBAL、BusinessDateOnce，但各自的`review_task_identity(date,task)`不同，不能共用完成权。[任务目录](push-capability-catalog.v1.json)、[通知政策](../../src/durable_delivery/model.rs#L447)

| 入口 | 日期、调用与恢复行为 | 证据 |
| --- | --- | --- |
| 自动 | 交易日19:00起每60秒检查；先恢复调度状态，再计算due，实际attempt使用at_manual和最近已完成交易日 | [时间门](../../src/bin/monitor/main.rs#L5656)、[attempt](../../src/bin/monitor/main.rs#L5717) |
| 手动 --review | 一批13项；Complete/Partial均可使命令成功，不证明R13或A10各自送达 | [手动批次](../../src/bin/monitor/main.rs#L5527) |
| 历史补推 | 最近5个历史交易日，不含今天；先补估值，失败仍继续。已有Delivered直接跳过；无claim才按原日重新进dispatcher，RejectedDurable走原信封授权重试 | [日期和任务集](../../src/bin/monitor/main.rs#L5735)、[单项补推](../../src/bin/monitor/main.rs#L5930) |
| 启动恢复 | 核心库绑定后、producer前恢复全部日期的pending通知；只恢复原信封，不补做下述业务保存 | [启动门](../../src/bin/monitor/main.rs#L4810)、[恢复实现](../../src/bin/monitor/durable_delivery_runtime.rs#L1166) |
| A10额外 --push | 使用当前自然日，Evening或Outside直接执行A10；不经过review quiet-hour preflight，A01健康准备失败也继续A10 | [当前日期](../../src/bin/monitor/main.rs#L1481)、[调用分支](../../src/bin/monitor/manual_push.rs#L88)、[批次静默门](../../src/bin/monitor/review_batch.rs#L1663) |

单项补推仍进入完整post-session dispatcher，附带prediction回填、block-trade、IPO等侧路，不是纯粹单一发送。[dispatcher](../../src/bin/monitor/push_templates.rs#L9891)

## R13：从原名单到核对推送

1. 解析checked_date并查询精确R13既有通知决议；存在即返回，不再读名单或行情。查询错误为可重试失败。[前置检查](../../src/bin/monitor/push_templates.rs#L9792)
2. 查询`watch_date < checked_date`最近一份名单，按日期/id倒序、成员按position/ordinal读取。这里没有验证它恰好属于上一交易日；无名单为NoData，读取/结构错误为可重试失败。[查询](../../src/database/catalyst_watchlist.rs#L202)
3. 依原leading后other顺序，每只读取2根日线；最新一根必须匹配checked_date且已结算，再计算涨跌、涨停类型和连板。单只失败/不足/日期错/未结算进入skipped；至少一只成功就继续，全部跳过才失败。[核对循环](../../src/review/watchlist_tracking.rs#L95)
4. 渲染成功子集及规则结论。source canonical保存名单日期、全部成员code/name/streak及成功结果code/close/change_pct/limit_up/streak_today；snapshot_size是成功结果数量，skipped及失败原因未绑定。[正文](../../src/review/watchlist_tracking.rs#L196)、[来源字段](../../src/bin/monitor/push_templates.rs#L9595)
5. counted通知结果转为Delivered后，才保存outcomes；唯一键为(watch_date,checked_date,code)，失败只warn，不改变Delivered。[通知后保存](../../src/bin/monitor/push_templates.rs#L9857)、[数据库保存](../../src/database/catalyst_watchlist.rs#L264)

## A10：从完整批次到次日名单

1. 先查询精确A10通知决议，存在即复用；无claim才调用实时gRPC loader。请求ChainBatch只传date，取第一条响应记录并要求返回trading_date等于请求日。[dispatcher](../../src/bin/monitor/push_templates.rs#L13967)、[gRPC转换](../../src/data_gateway/grpc_source.rs#L3847)
2. 只消费`chains.first()`；空chains为NoData。首条须有非空主题、至少3个成员且计数一致，成员名称/代码非空、continuous_count合法、inputs至少提供一项可解析观察时间。[转换约束](../../src/review/catalyst_review.rs#L86)
3. 沿首条chain的原成员顺序，前3为leading，再取3为other，最多展示/保存6只；member_count仍是全量成员数。本消费者没有重新排序其他chain或成员。评分、观察点缺失时按原结构规则推导，不能当作独立预测来源。[选集](../../src/review/catalyst_review.rs#L151)、[确定性推导](../../src/bin/monitor/push_templates.rs#L14038)
4. source canonical包括日期、主题、最大input观察时间、batch_id/content_hash、全量计数、选中成员code/name；未展开绑定其余chain、成员streak、版本及inputs/rejections。原信封固化正文，不等于本消费者独立验证了完整批次哈希。[来源绑定](../../src/bin/monitor/push_templates.rs#L14094)
5. 通知Pushed后才保存最多6只watchlist，事务幂等；保存或join失败只warn，通知仍Delivered。这份名单是后续R13的输入。[通知后保存](../../src/bin/monitor/push_templates.rs#L14131)、[幂等落库](../../src/database/catalyst_watchlist.rs#L110)

## 待解决问题及验收方向

- 两项均有“已通知、业务表未保存”的恢复缺口：进程崩溃或保存失败后，dispatcher preflight及backfill的Delivered分支跳过，启动恢复仅处理通知信封。A10漏存还会影响R13父名单。需独立保存业务进度，并证明恢复只补业务事务、不重复通知。[R13](../../src/bin/monitor/push_templates.rs#L9873)、[A10](../../src/bin/monitor/push_templates.rs#L14131)、[补推跳过](../../src/bin/monitor/main.rs#L5973)
- R13用原leading数量切成功outcomes前缀；若leading中有被跳过者，other可能被算入前排，改变结论。应按原成员身份对应结果，并以“前排缺数据、其他成员成功”的反例验证。[错误切片](../../src/review/watchlist_tracking.rs#L200)
- R13的部分成功仍可Delivered，且未保存skipped事实；以后数据到齐也不会自动补齐该日名单。需明确部分完成与完整核对的区别，恢复时不能篡改原通知身份或盲目重发。[部分结果](../../src/review/watchlist_tracking.rs#L158)、[完成映射](../../src/bin/monitor/review_batch.rs#L962)
- R13“T+1”仅由查询早于checked_date的最近快照实现，不能证明上一交易日。应绑定验证过的交易日/原名单版本，并明确陈旧名单分类。[日期选择](../../src/database/catalyst_watchlist.rs#L220)
- A10无claim历史补推仍请求实时远端loader，未调用本地stored replay；是否重新计算由远端实现决定，本次未运行远端验证。已有stored loader只供独立名单补录工具使用，不能据此声称monitor已按首次批次回放。[实时loader](../../src/review/catalyst_review.rs#L177)、[stored loader](../../src/review/catalyst_review.rs#L193)、[补录工具](../../src/bin/backfill_catalyst_watchlist.rs#L1)
- A10的--push绕过批次静默门且用自然日，入口政策不一致；需要明确共同窗口与业务日规则。目录仍写“失败收集后尾部Ok”，但当前已改为Err，不能把旧摘要当现状。[当前错误返回](../../src/bin/monitor/manual_push.rs#L110)、[静默门](../../src/bin/monitor/review_batch.rs#L1663)

上述是源码缺口与后续验收要求，尚未修复或生产复现。全部52个Unit、单用户任务锁/事务/防重、发布与回滚要求不减少；复杂可信身份平台仍排除。

## 核对时关键源码SHA-256

```text
src/bin/monitor/main.rs c87fa4bb56df1c786e11516f754316e0a3fb9d9f5f75aad8aa2a7cb02016488c
src/bin/monitor/manual_push.rs d4613908df2f24ed241d5d528cb20032be96b786877554afd3cc587c94a1d7e2
src/bin/monitor/push_templates.rs 5096e5697f67338b45346d11efa8f9ec988774c17a8f9623b280adeaeb2044f4
src/review/watchlist_tracking.rs c66f8ce93332c8333ddad932f702ce735ecc0ded29cd53f1e49d92a4de25d1f3
src/review/catalyst_review.rs 0e7ee72841c49b50abb6e376610f2c2a9dba45e822b5c620b1db9fb39b16bb11
src/database/catalyst_watchlist.rs 6b0396982bfb1cb0e8609829f509935482cf837b921509ad9a1992d4c26ec136
src/data_gateway/grpc_source.rs 358ad584fc546e18194871900d214920762c93cf611a4cf3d05f622ddf3916d7
```
