# 2026-10-08 outcome 回填接续与 monitor 报价推送

保留 H08、outcome 修复和 monitor 日常稳定性；冻结 H01/H02/H03/H09/H10/H14/H17。H04–H06 使用现有 paper，H16 使用周报复盘。Windows `841e4ae7a9df62be0c4536fa9009c66089d282bf` 服务继续保留。本文接续 [当前接收端交付](2026-10-08-latest-windows-receiver.md) 和 [outcome 修复](2026-10-07-outcome-integrity-repair.md)。

## 回填实际结果

用户已明确没有历史交易状态、生命周期或价格资格产品。14:01 北京时间完成的是隔离观测采集，正式资格回填仍未完成。

| 项目 | 实证 |
| --- | --- |
| 原始快照 | 13:40:54 对正式库只读单事务快照，2702 候选 / 76 预测，副本 0400 |
| 证券集合 | 候选1076、pending预测58，并集1097；canonical request identity 不签发历史 venue/issuer 证明 |
| 预测证券观测 | 58证券 / 3380行，预期日期缺失0 |
| 原始并集观测 | 1097证券 / 29133行，调用或请求绑定失败0 |
| 缺日期 | 4证券共22个；不得推断为停牌或补零 |
| 时间窗口 | 从各证券原始候选/预测推导，至当时最近已完成交易日2026-09-30；休市开始仅扩展观察窗口，不改原预测日 |
| 资格 | `ObservedOnly / NotAdmitted / UnknownCoverage / NotCertifiedPIT` |
| 正式库后验 | 原2702候选与76预测逐字段保持；日线5070行 / 57证券 / latest08-21；qualified交易状态0 |
| 新资格收益 | 0；没有写正式历史日线、状态、收益或订单 |

缺口保留：002155 为08-20/21/24/25/26；002667 为09-30；601238 为09-14/15/16/17/18/21/22/23/24/28；601995 为09-15/16/17/18/21/22。缺口原因未知。

复用已发布624普通release探针，逐证券顺序采集、逐项fsync，保存实际原件与SHA；没有把保存的JSON包装成 `AdmittedDailyBars`。实际完整性、源发布时间、历史可用性、交易状态、修订和价格资格仍需各自证据。

76条原预测中，id37–40 从休市日开始；id36/37/38/76 的旧目标日期落在休市日，部分重合。有效开始日对应216个T+1/3/5窗口均已成熟，收盘价资格缺失。没有改写这些日期，也没有产生零收益或成功率。

`backfill_daily` 改用 `DatabaseManager::init_retained_monitor`，沿用已发布业务库范围，不安装 prospective Unit 平台表。既有 `--outcomes` 仍由原始记录推导范围，缺观察或资格时返回失败，不能据“采集结束”称正式回填成功。

H08已核对Windows来源材料包51成员与manifest，包外ACK更新了本次完整采集结果，并再次反馈既有Windows任务。公开月度停复牌表不是完整逐日状态，也不证明缺行日Trading；SKU说明、文件ACK和当前服务Health均不签发历史权利。`QualifiedTradingFactsGateway` 仍返回 `ContractNotDelivered`，没有导入真实产品或新建资格适配器。

私有证据：`/Users/zhangzhen/.local/share/stock-analysis-outcome-backfill-20261008/`，包括只读原始快照、1097份逐项回执和原件、完整覆盖报告、成熟度报告、`backfill-readiness-report.json`。原件与用户数据不提交Git。

## monitor 缺失推送的范围

用户明确要求新开agent，独立子agent `/root/monitor_push_missing` 调查并实现窄修复。今天旧monitor在09:20–09:24的50次竞价轮询连接本地桥接失败；旧竞价消费者也未取得scanner真实quote。盘内旧scanner等资金流后再使用报价，报价超过原5秒门槛；泛化告警又被BR192 unbound counted delivery拒绝。下午已发布624实例尚未经历新竞价窗口。

新入口独立取得原始 `AdmittedRealtimeQuotes`，不等待资金流。opaque `PreparedObservation` 绑定真实证券、provider/source、原source_at/observed_at/batch_id、名单来源日期和准确正文SHA；槽位只用于occurrence，不充当行情时间或资格。报价消费仍逐次校验原5秒门槛、数值和当天source日。

交易日09:20–09:25发送当前竞价报价观察；09:30–11:30、13:00–15:00按15分钟槽位观察。沿用原 `IntradayMarket/NONE/GLOBAL`、rolling 900秒冷却及日预算，实际频率受冷却/预算约束。槽内任何已存owner（含Pending、Delivered、Uncertain、Rejected）关闭当前临时报价槽；重启或报价变化不新建同槽卡。启动恢复会自动resume Reserved，因此已在该私有前缀的物理发送前增加原存储canonical/hash/源时刻复核：跨窗口或超过5秒均TypedRejected且retry=false，不允许重建Admitted能力或重发过期正文。原始batch时间字符串可以是Unix小数秒，原字节保留hash绑定；严格时效只取sealed quote的normalized原source_at/observed_at。

Frozen豁免只对持有原始quote的私有事实报告生效。正文保留真实Frozen、缺账户指标和旧名单日期；量比、资金流及涨跌停资格未取得时不判断竞价强弱或给交易指令。通用I01/T0/竞价建议Frozen门槛保持。原scanner的资金流前置延迟、BR192泛化告警拒绝和账户资格约束未在本轮解除，因此本轮不能宣称全部止损或交易建议已恢复。

## 验证与部署状态

回填命令：`cargo test --bin backfill_daily`，3/3。初始化范围复用624已验证的retained API及真实主库副本证据，没有为同一目标追加全量检查。

monitor目标：最终7项通过（另2项显式opt-in ignored不计入普通通过数）；membership目标2项通过；Schema9库7项、scanner quote库6项通过。显式真实副本canary另行opt-in运行，1项通过：真实Tencent sealed quotes、opaque factory、L5正向、原Provider准入，3份实际Schema9副本分别形成Delivered/Uncertain/Rejected，重开owner与envelope一致，每个内存sink仅1次；原quote等6秒后拒绝。每份原5082决定、78Uncertain、SQL catalog及49policy保持，正式main/durable inode保持。canary飞书调用0，内存回执不证明平台接收。

另一个真实物理guard副本canary 1/1通过：实际Tencent源14:31:49、观测14:31:53.273194，fresh检查通过；仍在14:30同槽的14:32:02.583737经真实Magiclaw入口拒绝旧quote，外部调用0。原5083决定/78Uncertain、catalog/49policy/Schema9和正式inode保持。编译错误、raw格式误判及9.526821秒旧quote的初次失败原件保留，最终修正与真实重取源验证分别可查。

行为已统一提交并推送master `fd07f1546611ea3088ae63cfa782729afa069074`（backfill范围修正 `f7ed6ac39` + monitor）。正常release构建完成（1714秒），3个程序均exit0，完整Git输入前后SHA保持。monitor SHA256为 `6251b299d5f4e36260e806203715d1aefb43ef88d9791b631aefd16d0bb289fe`。同产物CLI及正式shadow均完成61项render、失败0、外部调用0；shadow已卸载。旧验收脚本按59而初次判失败，原回执保留；源 `br196_test_delivery.rs` 的既有75总家族在selection启用且4feeds注册时明确应为61active/11disabled/3retired，两项差异为N01/N02已有新闻展示家族的生命周期，未解冻平台。CLI原执行被重新核对，不为相同产物重复执行。

正式切换在15:00–15:05新闻窗口结束后进行。bridge旧binary保持，15:14:44完成原库校验并监听18082；activation `expected_config_hash=5da8eb94c585fff304a3de0a7b6058f5b4b7539b0e043659fecbf18a36ac8e82` 于15:13:02生效。正式monitor PID34850于15:15:17启动，15:16:18完成原库初始化，15:16:21完成fixed point，resumed_sink_calls=0。bridge PID34187、monitor均由原正式launchd单实例管理，未变Windows841服务。

15:22只读回验：主库dev16777220/inode154379673、durable inode154266271保持；Schema9原catalog与49policy精确不变。原5088个envelope全保持；其中5087行全部字段保持，另1个AttributionDaily在切换前15:08:47已有平台Accepted（消息 `om_x100b6340593e14acc3e29071e4b16a6`），原状态AcceptedAuditPending在重启后完成审计结算为Delivered，仅state/updated_at变化，没有重发。初次“全部行字段不变”断言失败证据保留。原78Uncertain全文及状态保持。

正式当前进程心跳与快照fresh，但Health总体 `unhealthy/banner_unhealthy`，账户Frozen、数据Unsafe，缺Quote/Kline/MoneyFlow/OrderBook；盘后此时没有quote观察发生窗口。4个raw新闻pull恢复为ok只覆盖原始拉取，不证明内容/全SDK资格；89.40%/95的上游SDK全CI缺口仍未豁免。

15:19原2702候选/76预测全部字段保持；正常运行的日线缓存已有8315行/675证券/latest2026-10-08，相对14:01的5070行有变化，不能把全局缓存称未写。隔离1097采集任务未导入其NotAdmitted JSON；qualified状态仍0，原76预测及216成熟窗口尚无新资格收益，正常验证循环仍deferred216。缓存增长不等于历史状态/价格/PIT资格回填。

在切换前旧624实例15:01:41正常生成15:00新闻聚合（critical0/aggregate1）；飞书GET200/code0，以原 `news_flash_render.v1` domain SHA验证445字节原正文一致，消息 `om_x100b6347b39b4ca4c429d161a3944a9`，现配置Stock机器人p2p私聊，read_users=0。这是正常消息、原发布新闻路径，未做手工测试/重发；不证明人已在客户端收到。已向用户询问可见性/是否应发到群，尚未答复。

截至本轮检查，新版没有当前竞价/盘内实投证据，不能称客户端已收到报价报告。不把编译、内存canary或进程PID当真实飞书投递。下一次当前竞价窗口为2026-10-09 09:20–09:25北京时间，需真实窗口观察后才能确认竞价实发。

发布证据：`/Users/zhangzhen/.local/share/stock-analysis-candidates/retained-monitor-push-release-20261008/`。`production-after-start-readback.json`、`natural-1500-news-feishu-readback.json`、完整normal release及shadow/CLI记录均保存；原件/秘密不提交Git。部署reviewed_by表示本会话用户授权的Codex切换，不表示独立人工数据准入。
