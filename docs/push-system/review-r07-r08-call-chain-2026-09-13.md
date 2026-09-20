# MU-review-r07 / MU-review-r08 四入口只读调用链

日期：2026-09-13。范围：静态核对目录登记的两个Unit、各四个producer及当前实现；未运行 Cargo、monitor、provider、数据库、网络或生产命令。本记录不代表已迁移。主控全文核报告，直接复核发布门、去重策略、R08前置/四源加载与硬失败、R07通知准入，并核9项来源摘要一致；结合原13个单元，现15个已追链、37个待追链，非迁移完成数。

## 共用入口、日期与完成次序

- 目录登记R07为`review-r07-auto/manual/backfill/startup-resume-tomorrow-watch`，R08为对应三个review入口加`startup-resume-event-calendar`；目录仅作定位索引，完成权仍以源码为准。[catalog](push-capability-catalog.v1.json):4858、4948、5028、5107、5201、5285、8072、8158。
- 两者都是`SourceOnly`，不受后段账户指标缺失门阻断；R08在首个封闭SourceOnly组，R07在下一组，均先于`account_required`统一失败结果。[review_batch.rs](../../src/bin/monitor/review_batch.rs):481-513；[push_templates.rs](../../src/bin/monitor/push_templates.rs):9927-9968、10083-10117。
- 自动入口每60秒tick，只在A股交易日19:00后运行；当日schedule初始所有任务Pending，因此R08从19:00可due，R07再由21:00来源门筛选。[main.rs](../../src/bin/monitor/main.rs):5656-5660、6122-6135、6179-6183、6244-6269；[review_batch.rs](../../src/bin/monitor/review_batch.rs):1207-1213、1559-1580。
- 自动attempt虽构造`at_manual(now)`，R07 preflight明确不看`manual_override`，仍以同日真实`eligibility_time`等到21:00；R08无该门。源码中“R07手动跳过21:00”的旧注释与可执行条件冲突，不能当政策证据。[main.rs](../../src/bin/monitor/main.rs):5717-5731；[review_batch.rs](../../src/bin/monitor/review_batch.rs):32-39、86-92、1703-1727。
- 手动`--review`取13任务和“当前时刻最近已完成交易日”；R07同日21:00前仍为ExpectedWait，R08立即可跑。手动另建临时schedule做hydrate/audit；批次Partial也返回Ok，故CLI成功不是任一Unit已Delivered。[main.rs](../../src/bin/monitor/main.rs):5527-5541、5556-5567、5579-5605。
- backfill扫描最近5个已验证历史交易日（不含今天，旧到新）的8任务集合；历史context固定原业务日且跨日资格时钟为23:59:59，R07的21:00门打开。[main.rs](../../src/bin/monitor/main.rs):5735-5750、5787-5807、5930-5948；[review_batch.rs](../../src/bin/monitor/review_batch.rs):42-55、86-92。
- backfill每任务先恢复原`date/kind/task_identity`；无decision才重新取源。Delivered跳过；RejectedDurable显式授权后再恢复；其他状态（含Uncertain/ManualRejected）不盲发。零投递只作NoData/FailedAttempt统计，不创建通知claim。[main.rs](../../src/bin/monitor/main.rs):5904-5927、5930-6009。
- 普通启动在core DB绑定后、所有常规分支/producer前执行all-date fixed-point reconciliation；只有完成后设置`producer_ready`。它恢复已存信封，不受19/21点、交易日或5日扫描限制，也不重新取业务源。[main.rs](../../src/bin/monitor/main.rs):4810-4849；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):1166-1184、2074-2123。
- 新投递共同次序是：source/renderer完成 → immutable envelope `prepare` → 本地reconcile → Reserved才resume唯一sink → 再reconcile → 读取decision终态并排队hydration。[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):2126-2176、2199-2250。
- schedule消费时验证transition、basis、hash、业务日和task identity；Accepted/Rejected/Uncertain/ManualRejected都可把本地任务置Terminal，因此Terminal不等于Delivered。[review_batch.rs](../../src/bin/monitor/review_batch.rs):1447-1556。
- 自动路径先hydrate再due，attempt后clone→再次hydrate→剔除durable任务→仅对legacy结果写audit→commit；hydrate ack先持久标记applied，再替换本地schedule。[main.rs](../../src/bin/monitor/main.rs):5465-5488、6257-6303；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):2383-2442。

## MU-review-r07：TomorrowWatch

### 四入口差异

| producer | 实际差异 | 同一完成门 |
| --- | --- | --- |
| auto | 交易日19:00起due，但同日21:00前ExpectedWait；失败后由schedule按1/5/15分钟退避 | 原业务日、`TomorrowWatch/None/GLOBAL`、`review_task_identity(date,R07)` |
| manual | 最近已完成交易日；同日21:00前同样等待；临时schedule不直接更新auto内存state | 同上，已有decision/hydration才能跨入口关联 |
| backfill | 历史日先确保精确日closing valuation，再先resume；仅无decision重建且正文加`[补推]` | 同上；原信封恢复时正文不重渲染 |
| startup | all-date恢复原immutable envelope，不调用候选/LHB/涨停链/持仓或renderer | 同一BusinessDateOnce decision；barrier不是新完成键 |

证据：[main.rs](../../src/bin/monitor/main.rs):5815-5867、5930-6009；[push_templates.rs](../../src/bin/monitor/push_templates.rs):8317-8389、8733-8738；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):1625-1737；[model.rs](../../src/durable_delivery/model.rs):439-442。

### 新准备链、来源与写入

1. preflight会在任何来源之前调用`resume_review_task_occurrence`；有既存decision即解释其状态，无记录才进入四源runner。这比R08的只读inspect更强，会对Reserved/已授权Rejected执行恢复。[push_templates.rs](../../src/bin/monitor/push_templates.rs):8317-8389；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):1682-1720。
2. A档候选从当前DB-only candidate context取Strong且价格有限正；代码没有把该context绑定到`trading_date`。这是历史无decision重建会读“当前候选”的直接边界。[push_templates.rs](../../src/bin/monitor/push_templates.rs):8412-8451。
3. 先读取`persisted_valuation_view_for_date(trading_date)`；龙虎榜请求`market_review(trading_date,5,5)`，取净买入Top5，再以该交易日strict settled-close或同日valuation补名补价，仍缺即逐条排除。[push_templates.rs](../../src/bin/monitor/push_templates.rs):8454-8553。
4. 涨停链请求`r03_upper_limit_pool(trading_date)`，聚合前3链leader；价格仅接受同日strict settled-close或同日valuation。[push_templates.rs](../../src/bin/monitor/push_templates.rs):8556-8640。
5. 做T读取持仓，限Holding、至少且整百股、成本正；价格只用精确日valuation，缺失逐条排除。四源最终按code首胜去重。[push_templates.rs](../../src/bin/monitor/push_templates.rs):8643-8687。
6. 普通dispatcher未见最终业务表保存；可见业务写入仅backfill在R07之前可能幂等补建closing valuation。其后持久写属于通知decision/audit/hydration，而非观察池业务记录。[main.rs](../../src/bin/monitor/main.rs):5815-5865；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):2137-2175。

### 渲染、通知与强弱结果

- 四源即使产出正文，只要没有非空LHB batch仍返回NoData；空观察池也为NoData。逐源错误仅warn/skip，因此“有正文候选”与“允许通知”不是同一条件。[push_templates.rs](../../src/bin/monitor/push_templates.rs):8687-8716、8733-8755。
- 通知binding只证明Eastmoney LHB：`source_at`日期必须等于业务日，记录为正有限净额；canonical投影仅`code|net_amount`，task basis的snapshot/batch也仅LHB。[push_templates.rs](../../src/bin/monitor/push_templates.rs):8212-8309。
- renderer正文和presentation token后走通用counted gate，再进durable runtime；政策是Global/BusinessDateOnce/86400秒，完成权不是schedule、四源DB或共享预算。[push_templates.rs](../../src/bin/monitor/push_templates.rs):8757-8825；[notify.rs](../../src/bin/monitor/notify.rs):2886-2933；[model.rs](../../src/durable_delivery/model.rs):439-442。
- 源码只有在durable `Delivered`且hydration身份/basis有效时才返回对应已投递任务结果；Delivered缺hydration仍是可重试失败。Rejected/ManualRejected/Uncertain映射永久失败，其他未定状态映射可重试失败。[push_templates.rs](../../src/bin/monitor/push_templates.rs):6887-6979。
- 迁移缺口QR02：非LHB三源没有统一不可变batch lineage；当前LHB binding与rendered hash不能证明四源属于同一业务日，尤其历史无decision重建的候选context会漂移。迁移时不能把现有BusinessDateOnce成功扩张解释成“四源业务快照已持久完成”。

## MU-review-r08：EventCalendar

### 四入口差异

| producer | 实际差异 | 同一完成门 |
| --- | --- | --- |
| auto | 交易日19:00起立即due，无21:00门；retryable失败按1/5/15分钟退避 | 原业务日、`EventCalendar/None/GLOBAL`、R08 task occurrence下的Rolling decision |
| manual | 最近已完成交易日、13任务批次、临时schedule；Partial可Ok | 同一occurrence，不是手动独立去重键 |
| backfill | 历史日先resume；无decision才重新请求公共四组件；R07估值前置失败不阻断R08 | 同一Rolling owner；统计NoData不等于已有decision |
| startup | all-date恢复原公共来源binding和正文，不重新请求四组件 | 同一Rolling decision；启动barrier不新建业务事件 |

证据：[main.rs](../../src/bin/monitor/main.rs):5842-5867、5930-6009；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):1533-1622、2074-2123；[model.rs](../../src/durable_delivery/model.rs):442。

### 公共来源、时间区间与重复抑制

1. R08先只读inspect精确业务日/task occurrence；有decision则不取provider。无记录时，`reminder_date=next_trading_day(review_date)`，并发请求CNInfo当日公告（limit 300）、CFFEX reminder年月合约、Sina美股指数和USD/CNY。[push_templates.rs](../../src/bin/monitor/push_templates.rs):11287-11369、11392-11433。
2. 四组件固定为CNInfo/`cninfo-market`、CFFEX/`cffex-official-notice`、Sina/`sina-web`两项；组件顺序严格、batch ID各异且组件不可重复。[push_templates.rs](../../src/bin/monitor/push_templates.rs):10497-10525、10561-10577、11069-11083。
3. 公告`published_at`必须落在business_date且不晚于observed；CFFEX publication不晚于reminder/observed，只投影`delivery_date==reminder_date`，按contract/product/url/last-trading-date严格升序从而拒绝重复或乱序；指数/汇率仅要求source不晚于observed。[push_templates.rs](../../src/bin/monitor/push_templates.rs):10527-10555、10586-10637、10639-10680、11223-11241。
4. CFFEX是硬门：gateway Err立即保留typed `GatewayError.retryable`返回，当前main明确宣告该能力unsupported且EventCalendar保持retryable、禁止sink。CNInfo/指数/汇率才可作为显式degraded组件。[main.rs](../../src/bin/monitor/main.rs):4487-4492；[push_templates.rs](../../src/bin/monitor/push_templates.rs):10237-10303、11415-11450。
5. 公告Available会先写进程内`REVIEW_ANNOUNCEMENTS_CACHE`供A11复用；未见R08业务表最终保存。即便CFFEX随后失败，该缓存副作用可已发生，但不会产生R08通知decision。[push_templates.rs](../../src/bin/monitor/push_templates.rs):11328-11369、11415-11431。
6. canonical binding保存业务日、下一交易日、四组件批次/事实、可选缺失集、正文hash、task basis；R08允许CFFEX VerifiedEmpty及最终0 item Delivered hydration。[push_templates.rs](../../src/bin/monitor/push_templates.rs):10795-10828、10902-10959、11084-11146；[push_templates.rs](../../src/bin/monitor/push_templates.rs):6934-6939。

### 通知owner与强弱结果

- presentation token固定EventCalendar；专用public-only校验和v14 gate后才进入durable runtime，不能借combined-account gate。[notify.rs](../../src/bin/monitor/notify.rs):2963-3036。
- 编译政策是Global/Rolling/86400秒，不存在BusinessDateOnce claim；preflight按`business_date + EventCalendar + None + GLOBAL + task_identity`查询既有review occurrence。Rolling冷却与精确occurrence去重都属于通知owner，不是公告缓存或调度state。[model.rs](../../src/durable_delivery/model.rs):442；[durable_delivery_runtime.rs](../../src/bin/monitor/durable_delivery_runtime.rs):1539-1622。
- CFFEX unsupported/其他首批Gateway失败发生在信封`prepare`之前：只有内存task失败、退避/Terminal与append audit，跨重启没有持久task版本可恢复。旧运行若从未形成decision，启动恢复不能补造原typed failure artifact。
- 迁移缺口QR01：必须为“尚无通知decision的来源失败”提供真实持久任务终态/版本恢复，且保留GatewayError retryability；不能将一次schedule Terminal、共享审计或后来的新provider batch冒充原失败完成证据。

## 未验证项与核对时源码身份

- 未查询任何历史decision、claim、rolling head、hydration或业务表；未验证部署配置、真实provider响应、TransportAccepted、远端接收或用户已读。
- 启动恢复的共同机制可达性已核，但未证明本环境存在R07/R08旧信封；恢复不会重新执行当前source freshness/业务时窗门。
- 未展开provider内部采集审计/数据库实现；“未见业务表保存”仅指上述producer/dispatcher可见调用链，不否认gateway内部自身审计。
- 复用了公开R04报告的共享入口定位，但所有R07/R08结论重新以当前源码核对。

```text
0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3  docs/push-system/push-capability-catalog.v1.json
5916504c51f28373194946ce693266dceb076948c6afbec1de1d8ddba513a4ea  docs/push-system/review-r04-call-chain-2026-09-13.md
c87fa4bb56df1c786e11516f754316e0a3fb9d9f5f75aad8aa2a7cb02016488c  src/bin/monitor/main.rs
8006c7bc6143bd410087b832c81636bc81bb1c047709fd0b0eb100ee77a56b2c  src/bin/monitor/review_batch.rs
5096e5697f67338b45346d11efa8f9ec988774c17a8f9623b280adeaeb2044f4  src/bin/monitor/push_templates.rs
ce2112226a4c71cb12426edd151444e239c4844ccf50d49252a67be3af7c7bfa  src/bin/monitor/notify.rs
53df56b8078453e9ab141766efed8ff72709ac821298adfd4f05bdeb4c92ec78  src/bin/monitor/durable_delivery_runtime.rs
b14f970f6c3d28123d97d287564aa870eb07bead06c2dcf44ac2be536c86d0b9  src/durable_delivery/model.rs
```
