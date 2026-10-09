# 实际持仓初始化模拟账本接线

用户在本轮明确要求“直接接线”。本次范围是在普通 monitor 主库上显式初始化并绑定独立 PaperLedgerV1，使用刚入库的实际持仓作期初；沿用现有模拟买卖和风控。不恢复此前冻结的正式预算 B、V2/F2 平台工程，不接真实券商，不裁定旧争议成交。

## 事实起点

- 用户截图时间为 2026-10-09 21:13（日期来自提交上下文，截图时间精度为分钟）。正式库已追加 position snapshot 40、account summary 42、real account snapshot 12；5 只持仓和原 projection 一致。
- 私有原始证据与入库收据：`/Users/zhangzhen/.local/share/stock-analysis-runtime/data/private_evidence/2026-10-09-position-import-2113-c6124ac14803/`。原图、逐票价格/数量/成本、同批现金与总资产都有来源；不将截图价格当新鲜执行报价。
- 切换前正式库 `application_id=0 / user_version=0`，未安装 PaperLedger namespace。此次显式初始化新增精确 namespace 与来源证明，保留 0/0 库头和主库文件身份；单纯设置环境变量不能替代初始化。
- 现有 main 与 bridge 共用正式数据库，但 bridge 是独立已有构建。保持主库身份和原对象，不切换全局 catalog。

## 执行与验收

| 工作 | 负责 | 验收 |
| --- | --- | --- |
| A：窄初始化、来源校验和 preview/apply CLI | paper_snapshot_bootstrap | 同批 DB 事实与原图/导入收据/截图解释相符；原子新增精确 namespace 与 seed；幂等；拒绝错 hash、旧/部分 namespace；历史事实不变 |
| B：生产绑定、独立模拟账户指标、已有买卖路径与单位修正 | paper_snapshot_runtime | 新 epoch 不受旧争议污染；真实后续快照不刷新 paper 资金；Unsafe、报价时效、独立价格资格、T+1/FIFO/费用/幂等保持；ATR 金额转 StopLoss 百分比明确验证 |
| C：兼容性与发布准备 | paper_activation_deploy_audit | 同一数据库、独立 bridge 兼容；匹配新 library 的 activation helper；单 owner、恢复材料与观测路径完整 |
| D：独立复审、最小充分验证、正式发布和回读 | root | 精确 source/config/build 身份；同制品隔离 dry-run；future selection activation；新模拟账户 binding、seed/head/5 lots/现金回读；无生产模拟试单或人工历史裁定 |

## 运行语义

新模拟期初按截图市价估值，截图成本作为 reference 留存；模拟收益从切换点开始，不能冒称真实账户既往收益已重置或亏损消失。首次期初零变化表示“从期初以来”，不冒称实时当日盈亏。未知可卖事实保守到下一已验证交易日。盘口/新闻缺失继续按实际数据模式拒绝成交，成功初始化不等于行情或完整实盘能力恢复。

当前状态：已于 2026-10-09 正式接线并发布。实现、独立复审、114 项相关检查、普通 release 构建和 `monitor --test --push-dry-run` 均通过。完整正式库副本的普通 release 初始化、重复 apply、普通运行适配器也通过。正式主库已提交 1 账户、seed/起始收盘基准 2 事件，并设置生产绑定；停写窗口比较的 8 张原有事实表完全不变，0/0 库头和原主库文件身份保留。

运行回读已核实：桥接 Health/Capabilities 就绪，selection activation 的配置 hash 匹配且 gate=enabled；新版 monitor 的 executable 身份匹配发布制品，数据库绑定、durable startup fixed point 与新 boot 心跳均正常，账户指标 `account_metrics_complete=true`。正式库只读 CLI 返回 `already_applied=true`，确认 5 lots、现金 7,254.94 元、市值 47,742.00 元、权益 54,996.94 元，估值时间保持 21:13，没有新造时间戳。

发布代码提交为 `b05e8e7324e041b3548dab12d126b7e17cc85633`。selection activation 生效时间为 22:24:42.908350（北京时间），monitor 22:34:12 启动，22:35:56 的启动核验通过。健康汇总仍为 `unhealthy`：账户沿用原 Frozen 策略，行情为 Unsafe；此时缺失 Quote/Kline/MoneyFlow/News/OrderBook 的合格能力，不能把进程就绪称为行情恢复。durable 恢复保留 80 个历史人工审查边界，未代为裁定；启动阶段没有恢复发送调用或生成模拟成交。

验证覆盖初始化 8 项、模拟账户/收盘 8 项、ATR 单位 1 项、既有 PaperLedger 70 项、owner 11 项、候选消费恢复 1 项、初始化 CLI 1 项，以及 monitor 横幅/账户门/指标 14 项。新增旧历史范围回归的初次失败源于测试夹具使用不合法的小写模式标签；修正为既有事实合同要求的 `Normal`/`Full` 后 8 项全部通过，运行时守卫没有为测试放宽。

## 操作入口与边界

`paper_account_activate --db EXISTING_DATABASE --manifest REQUEST_JSON` 只预览，返回封口的 `prepared_request`、绑定、目标文件身份和期初投影。正式执行必须使用这个完整请求，再加 `--apply`；初始化、来源证明、期初账本及本次获准的盘后起始基准在同一事务提交。已经执行的相同请求返回当前账本，后续实盘导入不会重复入金或加仓。

`PAPER_LEDGER_ACCOUNT_BINDING` 必须使用 CLI 返回的完整 JSON。配置后，账户指标、模拟买卖与恢复只读取该 epoch；旧争议成交不能成为新资金或模拟亏损。后续交易日的日收益要求当日价格和上一个已验证交易日的模拟收盘权益；缺失时保持未确认，不把累计收益当日收益。

这张盘后截图建立的是 2026-10-09 新模拟账户起始收盘基准，并非官方交易所收盘报价。期初可卖事实未知，按下一已验证交易日 2026-10-12 处理。上线不制造成交；生产数据模式与新鲜价格准入仍决定是否能执行模拟订单。

发布过程与私有运行证据写入 `/Users/zhangzhen/.local/share/stock-analysis-deployments/20261009-actual-holdings-paper/`。其中保留正式库一致性备份、原版本与配置恢复材料、测试结果、普通 release 演练与切换收据。旧 monitor 回退只恢复可用性；禁用绑定后新账本保持休眠，不能把旧程序当等价的模拟资金运行版本，也不能在已提交交易后盲目恢复旧库。
