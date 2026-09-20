# P-02 集合竞价同批数据修复结果

日期：2026-09-08。源码 `0dda4cc`，原始基线 `118d87c`。相关测试、静态检查及独立规格/质量审查通过，本片故障修复完成。它是 `MU-auction-volume` 的业务准备修复，不代表完整迁移、新发送主体接管或上线验收完成。

## 修复的业务问题

原 `main.rs` 首次获取涨停池，选出未通知的 Top10；随后 dispatcher 再次获取涨停池并独立取 Top10。消息和入池记录来自第二批，而外层通知集合标记第一批，可能把未展示的股票标为已通知。原内层只排除缺量比，没有完整拒绝 NaN、Infinity 和非正价格/量比。

修复后每个原竞价扫描 tick 只采集一次。`load_auction_volume_tick_real` 返回完整原始股票列表及由其派生的 P-02 选中快照；dispatcher 不再拥有采集步骤。原始列表继续供同分支持仓检测使用，不被 P-02 过滤或排序影响。

| 业务规则 | 当前代码证据（源码 `0dda4cc`） | 行为验证 |
| --- | --- | --- |
| 原交易日竞价、09:20–09:25、30 秒扫描不变 | `src/bin/monitor/main.rs` 竞价分支，修复仅替换批次加载/派发部分 | 固定提交 diff；未运行真实交易时段 |
| 一次采集，同时保留原始列表和选中快照 | `push_templates.rs::load_auction_volume_tick_with`、`load_auction_volume_tick_real`；`main.rs` 解构并使用这两个结果 | `auction_volume_p02_uses_one_source_batch_for_message_records_and_cursor`、`auction_volume_p02_preparation_preserves_shared_raw_stocks` |
| 先排除已通知及无效数据，再稳定降序取最多 10 只 | `push_templates.rs::prepare_auction_volume_snapshot`；价格/量比 finite 且 >0，涨跌幅 finite | 数值反例测试；`auction_volume_p02_filters_notified_before_stable_top10` |
| 消息、入池和通知集合消费同一组股票 | `push_templates.rs::dispatch_auction_volume_snapshot_with`、`dispatch_auction_volume_daily` | 首批/第二批不同的来源 adapter；实际模板输出、记录价格/代码和最终通知集合断言 |
| 缺 banner、发送失败、任一入池失败均不推进通知集合 | 同一 dispatcher 的失败返回和末尾 `notified.extend` | `auction_volume_p02_failures_do_not_advance_cursor` |
| 空批次、全无效、来源失败不产生可发送快照 | 准备函数及 `main.rs` 的 snapshot 分支 | 空/无效、来源失败专项；全已通知时共享原始列表仍保留 |

原注册展示、governor 和发送通道继续使用 `T-11-auction-volume / AuctionVolume / auction_volume_dispatcher / render_auction_volume`，没有新建旁路发送通道。

## 验证记录

- 数值行为 RED：59879，实际执行 1 项，0 passed / 1 failed，exit 101。旧选择函数接受 `RATIO_NAN`、`RATIO_INF`、`PRICE_NAN`、`PRICE_INF`、非正值及无效涨跌幅，与只接受 `VALID` 的独立预期不符。这条 RED 不声称同时证明旧双采集。
- 首轮接线编译：68494，E0425，遗漏后续持仓检测所需 `limit_stocks`，0 项行为测试。已修为保留共享原始列表，并增加对应回归；编译失败不算行为 RED。
- 最终相关合批：96224，`env CARGO_PROFILE_TEST_INCREMENTAL=true cargo test --bin monitor -- --test-threads=1 p02 t11_auction_volume blocking_market_data::tests`，**13 passed / 0 failed / 0 ignored**；7 项 P-02、1 项原 T11 模板、5 项 blocking_market_data；711 filtered，编译 1m07s，测试 0.03s。
- 最终生产配置静态检查：96833，`cargo clippy --bin monitor --no-deps --message-format=json`，exit 0，3m15s。本次两份修改文件没有诊断；lib 有 163 项既有定位告警，monitor 有 2 项位于未修改的 `attribution_epoch_runtime.rs:39` 和 `news_ai_shadow.rs:402`。完整计数使用本次 compiler-artifact 对应 fingerprint，不把汇总行算成新增告警。
- 定向 `push_templates.rs` 格式检查、`git diff --check` 通过。`main.rs` 整文件格式检查仍有基线已有的 `overlay_net_yi` 换行差异，没有为通过检查改动无关代码。
- 独立审查范围：固定 `118d87c..0dda4cc`，包含计划及源码两个提交；结论为 **Spec compliant / Task quality Approved**，Critical/Important/Minor 均无。定点核查确认实际调用无旧 loader 残留、持仓 detector 仍用共享原始列表、renderer 展示全部所选行。diff 无法证明的执行权限边界由主控本轮工具记录核对：只有隔离编译/测试/静态读取及本地 Git，没有生产操作。完整 Foundation 项仍列为未完成，不由本审查豁免。

## 实施取舍与未完成项

1. 在完整生产认证接线尚未就绪时，先落实 WBS 已要求的真实 P-02 同批故障修复。它可独立推进，但仍使用旧投递结果语义；不能代替完整 Foundation producer、typed receipt、崩溃恢复及持久游标迁移。
2. 一次采集保留两种视图：完整原始列表用于既有持仓检测，P-02 快照用于本通知。若把选中 Top10 误作共享原始输入，会改变另一个消费者的业务行为；因此有专门回归保护原顺序和 `main_net_yi`。

发送成功但入池部分失败时，已成功写入的行不会由本片回滚，通知集合不推进。旧投递去重对后续重试的影响、可靠补记和崩溃恢复仍需完整 Unit 迁移解决，不能宣称已经实现 exactly-once。

没有伪造来源证据、采集时间、batch hash、VerifiedEmpty 或强终态。本片未读取真实数据库、调用真实 provider/sink、启动或观察生产 monitor、替换 release 二进制、改变发送主体或执行上线批准。

完整 W01–W21、W15/W16 接线、52 个 Migration Unit 和真实发布门禁仍按[总实施范围](implementation-w16-results-2026-09-08.md)继续，局部验证不提升整体验收状态。
