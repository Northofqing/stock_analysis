# 盘后产业链报告：实际调用链与通知封口缺口

日期：2026-09-10。源码BASE为`20f215d56387dc233b8496c5c5d69022363f1030`，下述行号对应此提交。范围为[MU-chain-post-close](push-capability-catalog.v1.json#L9470)，唯一producer为`chain-post-close-timer`；不把盘前timer或CLI算成本次另行核对的Unit。独立只读追链后，主线核对关键控制流；未运行Cargo、业务函数、生产monitor或读取真实数据。

## 结论

**报告分析成功不等于通知成功，但当前timer把两者混为一谈。** mode收到通知`Ok(false)`或`Err`只写日志，最终仍返回`Ok(())`；timer随后写入当日`CHAIN_POST_LAST`，不再尝试。这是当前源码允许的结果，不是已证明某天实际漏推。

当前链路直接调用旧NotificationService，未接入Foundation业务intent、最终化和owner共同fence。本文只有分析，没有修复、放宽来源门、修改冻结目录或进行生产切换。

## 从盘后窗口到完成门

| 环节 | 现行逻辑 | 证据 |
| --- | --- | --- |
| 时间入口 | intraday_loop较早捕获host-local的now；15点且minute在30..35才检查当日内存日期位 | [main.rs:8878](../../src/bin/monitor/main.rs#L8878)、[实际timer](../../src/bin/monitor/main.rs#L8978) |
| 完成状态 | 先读CHAIN_POST_LAST，未封口时await mode；仅mode返回Ok才写Some(today)，检查和写入不在同一锁持有期间 | main.rs:8979–9002 |
| 分析业务日 | mode另取一次Local::now，由latest_completed_trading_day_at取最近已收盘交易日；非交易日回退 | [modes.rs:112](../../src/app/modes.rs#L112)、[calendar.rs:599](../../src/calendar.rs#L599) |
| 核心来源 | blocking worker按该业务日读取真实涨停池；初始化/provider/join等硬失败通过双问号返回，不封口 | modes.rs:115–121；[实际观察入口](../../src/market_analyzer/limit_up.rs#L497) |
| 主线业务效果 | 非空池先获取概念、聚类并写chain_daily，再做持仓诊断和后续分析；空池则直接生成“0只”报告 | [chain_analysis/mod.rs:630](../../src/pipeline/chain_analysis/mod.rs#L630)、[cluster_and_persist](../../src/pipeline/chain_analysis/mod.rs#L379)、[写库](../../src/database/concepts.rs#L161) |
| 文件保存 | 分析后另取完成时HHMM，与业务日组成报告名；先保存到reports，再通知 | modes.rs:164–175；[save_report_to_file](../../src/notification/service.rs#L376) |
| 通知结果 | true记info；false/Err只warn；末尾仍无条件Ok，timer据此封日 | [modes.rs:175](../../src/app/modes.rs#L175)、main.rs:8988–8995 |

## 需要修复或明确的行为

1. **通知失败仍封口。** 无渠道时[send](../../src/notification/service.rs#L125)直接Ok(false)；有渠道时逐渠道发送，把异常计为失败，最后只返回success_count是否大于0（service.rs:141–266）。因此零渠道或所有渠道失败，都会被mode吞成Ok并推进日期位。源/分析/文件的硬Err则不封，不能把所有失败混写成同一行为。
2. **部分成功被压成一个bool。** 一个渠道成功、其他渠道失败也返回true；没有逐要求渠道的完成身份、强回执或不确定态。当前可能配置十类渠道（service.rs:53–101），Custom还会逐URL发送（240–253）。这不是“全部必达渠道已接受”，也不是统一Foundation TransportAccepted；实际启用/必达集合未读取。
3. **自然日与业务日不一致。** 交易日guard只在并行market_loop内（[main.rs:9435](../../src/bin/monitor/main.rs#L9435)），不保护本timer，两者最终join（11276）。若周末常驻，周六/周日的日期门各自为空，但都可能分析同一周五。这个反例依赖程序实际运行、日历及来源可用，不等于生产已经重复。
4. **内存日期位不提供重启/跨进程幂等。** 正常单个intraday_loop顺序await，不应声称它必然并发重入；但同日重启或另一进程各有独立日期位。即使同进程存在第二个loop，check/await/set分离也不是原子claim。报告文件不是替代cursor。
5. **报告同名会覆盖。** 名称仅含business_date和完成时HHMM，不含calendar occurrence/producer/内容身份；[fs::write](../../src/notification/service.rs#L394)会覆盖同名文件。不同自然日解析到同一业务日且同分钟完成，或同日重启/其他调用同分钟完成，都可能碰撞。落盘成功发生在通知前，不能用文件存在证明已推送。
6. **窗口不是严格效果截止时间，也没有窗口外补偿。** timer使用较早捕获的now，前面的快照/盘后评估可能耗时（main.rs:8878、8925–8978），15:34捕获的一轮可在实际15:35之后才进入chain。反之整个窗口没有有效tick，后续15:35的tick不会补执行；失败还要经过循环末尾30秒sleep（9431），不保证有下一次窗口内机会。
7. **重试会重复业务效果及外部分析。** chain_daily先于后续报告/发送写入，按(date,concept)执行INSERT OR REPLACE（concepts.rs:185–198）；它不是和通知一起提交的事务。不能只把mode的false改成Err并无限重跑整条链，就宣称幂等问题已解决。

## 来源与时点不能在迁移时丢失

- mode读取财联社20条，最多15个标题作为背景；空批/VerifiedEmpty/Gateway错误先降为None（modes.rs:124–154）。**非空涨停池继续分析时**，None或空白文本还会触发[search_macro_news(3)](../../src/pipeline/chain_analysis/mod.rs#L981)的15秒在线搜索回退，超时返回空文本；不等于最终一定“无新闻背景”。空涨停池在此前已返回报告，不执行这段回退。
- 龙虎榜helper使用**其执行时自然日**而不是mode固定的business_date（[fetchers.rs:184](../../src/pipeline/chain_analysis/fetchers.rs#L184)）。Gateway错误降为空map；但Available中的非法金额/空代码/重复代码在[map_lhb_reviews](../../src/pipeline/chain_analysis/fetchers.rs#L217)返回硬Err，caller的问号终止分析（mod.rs:713）。因此不是所有龙虎榜错误都降级，也不能把该日期自动宣称与核心来源同批。
- 补涨字段不满足发布合同仍显式Unavailable，逐簇LLM失败可退回仅聚类（mod.rs:653–707、462–529）。实际迁移必须保留这些来源/降级事实与首次模型输出，不能在影子或恢复路径重新取数、猜测来源身份或把Unavailable当VerifiedEmpty。

## 后续交付约束

先固定本producer的calendar窗口、解析后的business_date、输入与不可变报告，取得合法owner执行许可后才允许效果；现有目录字符串只描述家族，不等于已注册生产occurrence算法。要求渠道的接受证据、明确未发送、部分成功及结果不确定必须分别保留，最终化据合法owner/fence和强终态推进独立durable通知cursor。已接受但后置失败必须先协调，不因任意Err盲重发。

平台时区、实际渠道/必达配置、节假日追加、已写报告/DB以及历史漏发或重复均未知；本文没有读取运行态。当前源码入口核对累计为**9个Unit，43个尚未逐链完成**，不是迁移认证比例，也不意味着此Unit已修复。新增发现留待正式current源码审计/蓝图刷新消费，不手改冻结快照。
