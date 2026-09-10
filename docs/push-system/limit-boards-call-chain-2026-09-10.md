# 盘中首板、二板、三板+：实际调用链与未关闭缺口

日期：2026-09-10。源码BASE为`19310148fe84ab00652ec314e6c9f0c5461dcaef`，下述行号均以该提交为准。范围仅为`MU-limit-boards`，对应`limit-boards-first`、`limit-boards-second`、`limit-boards-third-plus`，集合依据[冻结目录](push-capability-catalog.v1.json#L8877)。只读agent追链后，主线独立核对来源、筛选、通知集合、展示截断、治理键及投递结果处理；未运行这些业务函数、读取生产数据或监控monitor。

## 最重要的结论

这三个producer的当前源码路径存在**缺字段导致整组选集为空**：实际涨停池投影把`main_net_yi`设为`None`，主循环又只保留该字段为`Some`的股票。这里的“结构性无可选行”是本提交的源码结论，不是实测某天没有推送，更不是断言线上二进制等于此提交。不得用历史catalog的ACTIVE标记代替当前可达性或部署证明。

即使以后正式接入主力净流字段，下面的发送前封口、展示截断与三形态共用冷却问题仍需处理。**本次只有分析，没有修复连板业务，也没有放宽来源规则或修改冻结目录。**

## 从实际入口到数据消费

| 环节 | 现行逻辑 | 源码证据 |
| --- | --- | --- |
| 生命周期 | `board_notified`和`board_level_cache`在market loop中初始化为空；是当前循环生命周期的code集合，不是持久化`[session,code]`复合键 | [main.rs](../../src/bin/monitor/main.rs#L9557)：9557–9563 |
| 盘中入口 | 仅Morning/Afternoon分支采集涨停池与持仓行情；LimitBoards消费前者，不用持仓行情补字段 | main.rs:10041–10081 |
| 涨停池来源 | `get_limit_up_stocks`仅转统一gateway；兼容投影取观察对象中的股票Vec | [market_analyzer/mod.rs](../../src/market_analyzer/mod.rs#L95)：95–100；[limit_up.rs](../../src/market_analyzer/limit_up.rs#L557)：557–564 |
| 字段投影 | 股票的`volume_ratio`和`main_net_yi`均为None；盘中resolve原样保留该Vec，不做join | limit_up.rs:280–297；[intraday_market.rs](../../src/bin/monitor/intraday_market.rs#L49)：49–77 |
| 连板级数 | 对未通知且未缓存codes，每轮最多查40票；批量日线查询全部成功才extend缓存，失败不新增缓存 | main.rs:10227–10256；[market_data.rs](../../src/bin/monitor/market_data.rs#L407)：407–436 |
| 级数算法 | 至少3条日线；如果最新记录是当日则跳过它，检查前两日连续涨停，输出1、2或3 | market_data.rs:372–405 |
| 资金筛选 | 先排除`main_net_yi=None`，资金降序取全局前50，再按级数分三组 | main.rs:10265–10317 |
| 展示与发送 | 三组按首板、二板、三板+顺序分别render、取不同token、await通用发送，但都忽略返回值 | main.rs:10322–10397 |

因此`main_net_yi=None`并不会进入“主力暂无”的显示回退：回退表达式位于已经过滤None的循环内部（main.rs:10275–10305）。连板级数查询又发生在资金过滤之前，所以“最终无可选行”不代表前面没有查询日线。

## 接源后仍会触发的静态反例

下面均以“上游已经合法提供主力净流，相关数据及治理满足准入”为条件，不冒称当前缺字段路径已经实际发生这些投递。

### 1. 发送前推进通知集合，失败后失去重试机会

main.rs:10294–10296先执行`board_notified.insert(code)`；只有首次insert成功才生成展示行。render/token/发送位于后面的10322–10397，结果未用于回退集合。下一轮**真正阻止重新生成消息的是再次insert返回false**，10229的contains仅是前面跳过重复级数查询，不能单独证明不重试。

例如一只首板进入集合后，render/token拒绝或sink失败，下轮即使恢复也不再生成该code的行。下游即使释放冷却，也不能解除上游这道已推进的集合门。

### 2. 第11行以后可能已封口却从未展示

每组renderer只取前10行，见[push_templates.rs](../../src/bin/monitor/push_templates.rs)中的`render_limit_boards_shape`（BASE:10110–10128）；通知集合却在分组及展示截断前，对全局前50中有级数的股票提前推进。若其中有11只首板，第11只已被insert，却不在首板卡中，后续循环也不再生成它。

“全局前50再分组”也不同于“每组独立Top10”：某形态在总榜排名较后可能没有展示。尚未进入前50的行没有被提前insert，未来排名改善仍有机会；不能把这一点误写为全部尾部候选永久丢失。

### 3. 三个展示token没有形成独立冷却身份

三个registry tuple确实不同（[presentation_registry.rs](../../src/bin/monitor/presentation_registry.rs#L333)：333–350），但[push_presented_v3](../../src/bin/monitor/notify.rs#L2877)只保留`PushKind::LimitBoards`并调用通用governor。三个caller都传`code=None`；[v14_gate](../../src/bin/monitor/v14_adapter.rs#L401)继续传`sub_kind=None`。

`LimitBoards`采用默认1800秒、Global范围（notify.rs:388–488），event.kind为`limit_boards`且code仍空（v14_adapter.rs:1168、1266–1275）。[L4键](../../src/push_l4/dispatcher.rs#L133)由kind、business_identity、sub_kind组成，因此三个形态均是`("limit_boards", "", "")`。它不是counted kind（[durable映射](../../src/bin/monitor/durable_delivery_runtime.rs#L2274)：2274–2321），也不走source-fact身份（v14_adapter.rs:1366–1376）。

在正常顺序调用、首个非空形态实际通过治理且成功提交L4的条件下，后续形态在该30分钟窗口内会被同一个键Deduped；调用方忽略该结果，且它们的codes也已提前insert。成功commit与后续reserve判断见v14_adapter.rs:976–1002、dispatcher.rs:123–180；Deduped传回见notify.rs:2301–2303。不能只给token改名字就声称隔离了身份。

### 4. 内存完成与投递证据脱节

`board_notified`在进程/循环重新进入时为空；[v14 stack](../../src/bin/monitor/v14_adapter.rs#L53)虽打开持久L7，但仍用`Dispatcher::new()`初始化空的L4表（53–75；dispatcher.rs:99–105）。在“此前已发成功、同日重启、上游仍给出合法同票、其他治理准入允许”的条件下，这两道内存门自身不能阻止再次发送；这不是已证明发生的重复推送。

通用尾段先取得sink的bool结果，再写L7/hash-chain并settle L4（notify.rs:2338–2418）：sink为false时不占冷却；sink为true后，即使后置审计失败也commit，再返回SinkError。因此“所有失败都把集合清掉立即重发”也不正确，须区分物理未尝试/失败与可能已经接受后的审计不确定态。

此外，`push_wechat`把`SimulatedWithoutPhysicalAttempt`折叠为true（notify.rs:3133–3138），所以通用`Pushed`不能直接充当真实transport回执。当前结构化审计使用同一`limit_boards_v1`与空code（notify.rs:580–590、2377–2384）；这些键没有逐shape/row完成身份。正文可能含形态标题，不能据此宣称“所有记录都无法辨认形态”，但正文可辨认也不等于强完成证据或持久化owner。

## 后续实施约束

- 先确定主力净流、量比和连板日线的正式来源合同及同批绑定；不能直接删掉字段门或填0来制造推送恢复。
- 选中行、展示Top10、实际投递提案和通知集合推进必须使用同一批明确的行；三个producer的展示身份与投递/完成身份必须保持可区分。
- 完成政策须区分未发送、冷却抑制、明确失败、已接受但审计不确定及强终态，不能把bool/Deduped/任意错误一概当成功或重发许可。
- 仍需逐Unit的真实来源、intent/不可变提案、receipt、finalizer、恢复owner及W16共同fence证明；调用链已核对不等于Foundation迁移完成。

本次将“可指出实际旧业务入口并追踪主要完成门”的累计核对从7个Unit推进到8个，另44个尚未逐链完成。52个Unit的完整迁移认证数量仍未知。此新发现应进入下一次正式current源码审计/蓝图刷新；本记录没有手改冻结目录或生成制品来假装门禁已通过。
