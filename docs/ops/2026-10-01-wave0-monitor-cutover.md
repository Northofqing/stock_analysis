# Wave 0 monitor 生产切换记录（2026-10-01）

## 制品与激活

用户批准了冻结源码 `1f0fc6a7f2c90916066a937e128500c532ae1c85` 的精确 activation 候选，生效时刻为 2026-10-01 19:00 CST。生产运行根为 `/Users/zhangzhen/.local/share/stock-analysis-runtime`。本批只安装封存清单中的 27 个源码路径、release monitor 和获批 activation；没有覆盖生产数据库、client bundle 或本地桥二进制。

| 项目 | 生产安装后的 SHA-256 |
| --- | --- |
| `target/release/monitor` | `851a5fb9f384e5ffbb437ef04979bd21d26fa1991f11a264229d5eb93703d658` |
| `config/selection/selection_activation.v1.json` | `f574faf1868fc6e1a3963b83793e79fbcbb4a875116f864a29e4280e6a206b60` |
| activation `expected_config_hash` | `17a3fa4a2389f272e2cf8818aa713e973c1cb00c6f008545163f3bd71f81cf4c` |

18:40 CST 完成旧进程、锁、制品、持久库和本地桥的动态复核，保存只读一致的持久库快照。19:12 CST 后停旧 monitor PID 17089 并确认投递锁释放。旧 monitor、activation 和源码 tar 均在 `/private/tmp/stock-analysis-wave0-rollout-20261001/` 留有回退副本；数据库不在回退覆盖范围。

20:16 CST 首次启动新 PID 2548 时，selection gate 给出 `activation_not_effective`。原因是生产旧 `src/.DS_Store` 和 `src/agent/.DS_Store` 被激活算法计入可执行输入，而预检脚本错误地将它们列为例外。停新实例并确认锁释放后，将两个文件可逆地移到上述切换目录。严格复核生产根 **735/735 个输入均与封存清单相同，且 `src/config` 无额外文件**；用同版准备工具复算配置哈希得到获批值。正式 activation 文件没有改字节，也没有重新生成。

20:27 CST 单实例启动 PID 4371。启动日志不再有 selection gate 禁用；进程打开生产主库与持久投递库，并独占 `monitor-delivery.lock`。本地桥 PID 56417 保持运行，未随本批重启。

## 启动与投递恢复

PID 4371 在 20:34 CST 完成 core DB 初始化，migrations 用时 427764 ms。只读采样显示启动时大量时间消耗在数据采集审计链复核。持久投递启动固定点为 `progress=3 resumed_sink_calls=0 manual_review_boundaries=78`。

首次启动的 PID 2548 曾产生一条有 Feishu 权威 `Accepted` 回执的 DataMode 决策，停机时处于 `AcceptedAuditPending`。PID 4371 只做本地审计和 disposition 恢复，未再次调用 sink；该决策已成为 `Delivered`，其 delivery audit 引用和不可变审计均已落盘。20:35 CST 持久库 schema v9 只读聚合：`Delivered=981`、`RejectedDurable=3988`、`ManualResolvedRejected=6`、`UncertainManualReview=78`，没有其他待处理决策。78 条 Uncertain 仍需外部证据与人工裁定，不能自动重发。

## 尚未通过的生产门禁

- 新进程的心跳和健康快照最终均新鲜，但 `monitor --health --json` 返回 `banner_unhealthy`：账户 `Frozen`、数据 `Unsafe`，缺 Quote、Kline、MoneyFlow、News、OrderBook。20:42 CST 后 DataMode 快照按约 60 秒刷新；启动时 `NewsMonitor::new` 同步等待证券元数据 RPC，曾把首次定时刷新推迟约 8 分钟。该路径正在隔离修复。
- 新进程实际接纳了 Tdx board-memberships 批次；R-08 announcements 为 `invalid_request`。只读诊断表明本地桥旧二进制对 `MarketAnnouncements` 返回 `UNIMPLEMENTED`，并非回补日期错误。ExternalV1 有先前 300 条完整准入的独立 canary，后续路由修复仍须定向验证、新 activation 和单独部署。Global FX、BlockTrades 等路径亦报告 `no_verified_batch`。
- `selection-v2` 的 GlobalSchema authority 尚未接线；PaperLedger 仍要求显式 seed/cutover。Wave 0 的激活与启动不等于全平台上线。52 个 Unit 的逐项 owner、外部回执、finalizer 和自然运行窗、VM 数据合同、研究与 Gate P 证据仍按平台路线图逐项验收。

下一批若修改 `src/` 或 `config/`，重新生成完整可执行输入哈希和 activation，完成要求的人工复核后才重启生产实例。执行前使用 `scripts/verify_executable_input_manifest.py --activation-ready` 严格检查；activation 自身另核批准的 SHA-256。回退时先停新实例、核对持久写入和 Uncertain 水位，只恢复匹配的 binary/source/activation，不覆盖数据库或重发已接纳投递。
