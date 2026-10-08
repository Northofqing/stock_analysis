# 保留范围开发、验证与实际发布（2026-10-08）

本轮可独立完成的源码开发、定向测试、独立审查和正式运行目录发布已完成。行为源码为 `a41c5d9eb28537dfb0ee6352c02586cadce2ce3d`，已合入并推送 `stock_analysis/master`。下文记录真实生产读回；历史数据资格、旧 paper 账本事实和飞书客户端收件仍有明确未完成项，不能将发布成功解释为全部业务恢复。

接续 [当前范围](2026-10-07-active-scope.md)、[每日脚本开发交付](2026-10-08-outcome-daily-job-closeout.md) 和 [H16 周报入口](../ops/weekly-outcome-review.md)。本记录发生在这些开发切片之后，作为本轮实际安装及验收依据。后续仅文档提交不改变此处的构建源码和制品。

## 范围及源码交付

| 范围 | 本轮结果 |
| --- | --- |
| H08 | 保留 Windows 最新 `841e4ae7a9df62be0c4536fa9009c66089d282bf` / service `0.2.0`；Mac 最新接收端实际读验。缺资格、源矛盾和实时源故障通知原 Windows 任务，未放宽资格 |
| outcome | 每日回填脚本源码化；只读选取 pending、查询失败停止、保留部分成功、传播真实子进程退出码。原交易日与资格合同不变 |
| monitor | 耗时 overlay 完成后重新取得完整报价批次，在 scanner 消费前及逐股检查原 5 秒时效、原 tick 日期和连续交易窗口；失败暂停该 scanner tick。持仓集合重读，缺资金流保持缺失 |
| H04–H06 | 沿用现有 paper 读端；没有正式资金 issuer、seed、cutover 或 F2 |
| H16 | 新增只读描述性周报 CLI、快照 wrapper、双格式原件及每周五 20:30 的 launchd 入口；不预注册实验，不增加消息 kind |
| 原不确定投递 | 新增只读 inspector，输出人工核对材料；不裁定或重发。79 条原件中，本次观测的当前滚动 scope 阻断为 4 条 |
| 冻结范围 | H01/H02/H03/H09/H10/H14/H17 全部平台工程保持冻结；其余路线只做上述保留业务所需支持 |

独立审查发现的周报坏 target/direction 漏计、同日未来 paper/audit 误纳两项 P2 已修复并通过回归。部署 helper 的候选源码绑定和初始化前读回问题也已修复，最终审查无未关闭发现。未修改原历史成交数量、旧预测起始日期或投递终态。

## 实际验证与边界

| 检查 | 实际证据 |
| --- | --- |
| 只读 Uncertain inspector | 5 个隔离测试通过；原工具执行 `f0bcb5` 有结果，没有另存独立原始测试日志；最终源码未变 |
| 每日脚本 | 10 个隔离测试通过；原脚本吞掉 exit 7/19/23 的失败示例已复现。另经隔离 launchd 实际执行：两个子进程原 exit 7/19、部分写入保留、外层及 launchd exit 7，符合预期；未触碰生产计划/库/provider/sink |
| monitor | 9 个相关 Rust 测试通过，原提前取价缺陷有失败示例；覆盖消费前整批重取、跨午休/收盘及日期保护 |
| 周报 | 最终 15 个相关 Rust 测试、系统 Python 3.9 的 8 个 wrapper 测试通过；同一快照/时刻、未来事实拒绝、累计超卖不可用和输出失败保留均有验证 |
| 正常 release 模板 | 同一正式 monitor 制品 CLI 与隔离 launchd shadow 分别验证相同 61 个模板：失败 0，无真实外部发送或 receipt audit 写入。是 61 个模板的两种执行路径，不计作 122 个不同模板 |
| Schema9 副本 | 普通 release 读端重开原库一致副本，93 个对象、49 项政策及各行保持；无真实 sink |
| 最新 Windows 接收 | 普通 release 接收读验收到 `300274` 的 48 根 15 分钟线：09-29、09-30、10-08 各 16 根；服务版本/source/contract/binary 身份核对一致。只证明该尾段接收，不出具历史资格 |
| 正常周报 CLI | 真实约 2.5 GB 主库的只读一致快照及 JSON/Markdown 均成功；原数据库身份、catalog 和账户历史保持 |
| 预测 CLI 副本 | 原真实 exit **1** 保留。37–40 起始日期落在 07-11/07-12 休市日，13 项/24 窗口 deferred、verified 0；初始化/原件保留核验通过，业务回填没有完成 |

共 **47 个不同的定向测试**，按改动影响选择目标，没有运行冻结路径或全仓全量测试。编译证据复用通过的目标；release 用于实际上线。未验证额外整批 RPC 的完整交易日负载及长暂停场景，也没有将离线测试当作用户飞书收件证明。

## 普通 release 与生产读回

私有证据目录为 `/Users/zhangzhen/.local/share/stock-analysis-candidates/retained-scope-closeout-20261008`（0700）；正式运行根为 `/Users/zhangzhen/.local/share/stock-analysis-runtime`。日志、快照及回执留在本机，不提交原始数据库或凭据。

构建冻结 1,739 项输入，普通 release、锁定依赖，生成 monitor、selection_activation_prepare、backfill_daily、backfill_predictions、weekly_outcome_review 五个制品。审查前的 `c7c9e2464` 候选已归档且没有安装；最终 `a41c5d9eb` 沿同一构建路径复用正常缓存，未变文件逐项核对 Git 内容后保持其原 mtime，周报修复实际重新编译。最终输入前后 hash 一致，没有降低优化、接口、SDK 或激活门槛。

| 已安装项 | SHA-256 |
| --- | --- |
| monitor | `64ecd57b5e0e951db9bcb1ea6187b196c4c47775a6b7163377fc0d7adb9d3be6` |
| weekly_outcome_review | `d73ed30fe2ef44ae8cefba0289949bc3603cd4c1e5fc04915e5fbe90145c8c99` |
| daily_prediction_verify.sh | `81a22d730f1ab6b1b12c3ad5c5f7e76125d992ce93c20f4ccfd4ff592c4c42ec` |

23:20:31 上海时间完成制品安装，原进程退出；未来 activation 生效时刻为 `2026-10-08T15:24:28.406782000Z`，配置 hash `732a68684276efe560ba37b4ee69f588ebd7057b81fac9c2e8cccc20553cf898`。未更换 Windows 服务或本机 bridge 制品。

本机 bridge PID 4663 于 23:24:27 完成数据库初始化及 18082 端口就绪；monitor PID 5201 于 23:24:55 启动，23:26:03 完成初始化。启动固定点为 `progress=0 resumed_sink_calls=0 manual_review_boundaries=79`，没有启动重发。23:27 先确认两个新实例初始化，再读取原库，健康读回后再次核对同一 PID，避免把未完成初始化的存活进程认作验收。

主库、durable 库及凭据的原 inode 保持；主库 catalog 401、durable Schema9/49 项政策保持。5,091 条原投递全部字段及 envelope、79 条 Uncertain 保持，没有新投递或审核状态过渡；用户已确认的账户历史 41 条摘要、39 个持仓 header、254 个 item 均保持原样。

## 真实任务运行

23:28:17 已恢复原每日任务的 loaded 状态，**原每天 21:17 计划不变**。新脚本及两个真实工具的安装路径/hash 已核对；本次没有 kickstart 生产回填，新的下一次自然运行尚未观测，不能称生产每日回填已成功。

周报计划为 **每周五 20:30，Asia/Shanghai**，`RunAtLoad=false`、`KeepAlive=false`，使用 `/usr/bin/python3` 3.9.6。正式周报任务已实际启动一次并 exit 0，同一只读副本、同一 `2026-10-08T23:28:17.956452+08:00` 时刻生成两种格式；没有 provider、sink、资格发行或数据库写入。原数据库身份、catalog 和全部已确认账户历史保持。

本次版本目录：

`/Users/zhangzhen/.local/share/stock-analysis-runtime/reports/weekly-outcome-review/2026-10-05_2026-10-11__20261008T232817956452__420ac0ff2be74fa4a81e8fe23d020fd5`

其中 `review.md`、`review.json`、`run-status.json` 均为 0600，版本目录 0700。请求周为 10-05 至 10-11，实际完成交易日只到 10-08；没有把未来日期计为已完成。报告列示原日线 12,363 行、旧推断状态 9,338 行、预测 76 行、原 paper Filled 2,291 行；这些原表范围不证明独立资格。可靠预测/PIT 样本、可靠 paper 结果和物理投递分母均明确为 unavailable，而非零样本收益或已恢复。

## Windows 通知与尚未完成项

已向原聊天 **“R08 FuturesDelivery 上游合同与部署”**（`01a0e0cf-2276-7512-96ee-3a94bdfa8ca5`，Windows host）通知资格/源问题。该任务已确认登记原日志 5 项观察；Mac 已读取其登记回执。最终发布及盘后健康事实已另行补发通知，没有新增采购、合同放宽或 Windows 部署要求。

| 未完成事实 | 当前处理及界限 |
| --- | --- |
| H08 历史资格 | 用户没有历史产品；历史停复牌、生命周期、价格带/tick、历史 available_at/PIT 仍 `ContractNotDelivered`。9,338 条旧状态全为日线推断，不能替代独立证据；ObservedOnly 材料不签发合格 outcome |
| Windows 源数据 | 龙虎榜 Core 净额矛盾、TopN HTTPS 无响应，以及原 MoneyFlow internal error/无 verified batch、原报价消费时超过 5 秒已提供真实材料。OrderBook 只持有 capability 缺失，仍 PendingMaterial；缺原 request_id/wire 的部分不伪造请求原件。登记不等于修复完成 |
| 旧 paper 账本 | `BR255 attribution_epoch_cumulative_oversell` 仍真实阻断可靠 paper/economic 结果和账户指标；缺原买入或显式归属裁定，未更改成交数量、删除历史或 seed 掩盖 |
| outcome 原数据 | 37–40 休市日错误、资格与成熟窗口缺口保留。未将原非零工具退出码或 deferred 改报成功 |
| 79 条 Uncertain | 原终态待人工原件核对；当前滚动阻断 4 条，不代表全部 79 条都阻断当前任务。没有自动裁定/重发 |
| 飞书收件及自然交易窗口 | 尚未证明用户客户端收到本轮集合竞价、盘内监控、复盘或归因。61 模板 dry-run、进程存活、平台历史回执都不能代替收件验证；没有手动发送测试消息。10-09 竞价及盘内自然窗口尚未发生 |

**最终 23:34:32 上海时间读回：overall unhealthy，账户 Frozen，数据 Unsafe；缺 Quote、MoneyFlow、News、OrderBook，Kline 此时已不在 banner 缺失能力列表。** 两个原运行实例及心跳正常；Eastmoney、Cailianpress、Jin10、ThePaper 四个原始新闻源均成功拉取且 breaker closed。这只证明四路原始拉取恢复，不能据此宣称 News 能力合格、账户解冻、交易可用或飞书收件。该盘后健康观测也不是新的盘内 raw RPC 故障原件。

## 原件索引与回退

私有证据目录内的重要回执：`review-readback.json`、`release-build-receipt.json`、`release-push-dryrun-receipt.json`、`shadow-dryrun-receipt.json`、`schema9-copy-canary-receipt.json`、`normal-minute15-canary-receipt.json`、`normal-weekly-readonly-canary-receipt.json`、`prediction-cli-copy-canary-receipt.json` 及其 `classification`、`production-install-receipt.json`、`production-after-start-readback.json`、`production-daily-job-restored-receipt.json`、`production-weekly-report-receipt.json`、`production-final-observation.json`。

旧源码/配置/合同/Cargo/build 输入、原制品、daily 文件、各 plist 以及新增 weekly 路径的原缺失状态均有私有回退材料，见 `rollback-static-receipt.json`；没有执行回退。当前源码与正常制品保持上述版本。独立审查原件保留在集成工作树 `.planning/2026-10-08-retained-scope-closeout/`，不会因结束聊天删除。
