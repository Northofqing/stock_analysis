# 2026-10-06 新闻强度审计与 completion 上线

07:35 CST 已核验新闻修复源码提交 `37ed88757c63dc2d35941969eec41449e2e4f043` 部署到 `/Users/zhangzhen/.local/share/stock-analysis-runtime`。隔离新闻工作树保留该提交，未远程 push。release monitor SHA-256 `0de7d703e0971feb07d88442855c16c277e8be9bf0775b4232e3157f4d4d62d8`，14 个新闻源码叶与构建候选同字节。

改动增加真实 receipt 绑定的独立 strength、SQLite 不可变审计及完整回读；模型调用前检查同新闻文本历史并取得固定五槽之一，已提交分数与 slot 在同一 blocking owner 内交接。关闭/取消保留原 score，接收按 deadline 优先；N02 阶段先于有界 ready drain，新旧分数均进入单一 NewsFlashGate 与专用持久投递。纯 raw SourceOnly 保持 Neutral/0。

实际验证：最终同版 library new7、monitor new1、related10，共 18 distinct 方法全部 PASS；独立 Source 与 DATA 复核无剩余发现。release 两目标 EXIT0；同制品 `--test --push-dry-run` EXIT0、61 类、failed=0、external_process_attempted=0、receipt_audit_appended=0。早期编译/fixture/旧拒绝文案失败记录均保留；最后一处修复恢复 v5/v6 原拒绝文案并单独标识 v7，未改旧断言。

旧 monitor PID81292 已卸载并退出。源码14、monitor 与 activation_prepare 安装后，在真实生产根与原主库运行 activation 工具；保留打印的15分钟模板，经 Root 最终复核选择未来90秒的紧凑 JSON+LF。expected_config_hash=`979314e347c6e9383ee1f2835234e7e467ce5c00be39db23e87848cc0cc7b0f8`，effective_from=`2026-10-05T23:28:57Z`。

正式 `launchctl load -w` 启动 monitor PID70479；07:32:28 数据库初始化完成，07:33:24 gRPC 桥连接，07:33:25 new_analysis/governed_delivery_recovery=enabled 且 completion receiver 注册。桥接 PID56417 保持；单 monitor、cwd/binary 与两原数据库路径及 dev/ino 核对通过。未复制替换数据库，未裁定 Uncertain 或重放历史。

07:33:28 四家 GlobalNews outcome=available：Eastmoney20、Jin10 17、CLS20、ThePaper12，共69条。接纳数不能代表每条新鲜可推送或远端 Accepted，ThePaper 仍含旧发布时刻。

07:35:30 `monitor --health --json` EXIT1：monitor_running/snapshot_fresh/heartbeat_fresh=true；全局账户 Frozen、数据 Unsafe，account_metrics_complete=false，缺 Quote/Kline/MoneyFlow/News/OrderBook。新闻启动恢复不代表全局健康关闭。

运行回执与静态回退备份在 `/Users/zhangzhen/.local/share/stock-analysis-news-critical-rollout-20261006`；最终回执 `deployment-final-receipt-fix6.json` SHA `0b238b73648808cc8f1084ac8bb6a85222968d0001a6c556f81511eeebdd293c`。回退只恢复同根源码、二进制与 activation，保留当前 DB/WAL/审计/投递状态。

尚未验收自然 N01 scored event/远端 Accepted、N02 自然窗口或30秒现场延时。真实空 instruments 宏观消息的独立 GlobalCritical purpose 正在另一个 Source 包开发，尚未纳入本版本；非空非法证券不回退为宏观。


## 09:16 CST 后继：Global importance 与 News freshness

部署提交 `1c7aeca770fbd05276f1f05760cc69c916620300`；Source17/完整740输入、monitor与activation_prepare均安装，monitor PID85249 / bridge56417，原DB身份保持。release monitor SHA `d04e64ab569cf3856eeb1c949211d577220e192ae9950f7b196f33ee2364b944`，最终17定向回归及独立DATA接受，release与同版61项dry-run成功，未外发演练消息。

Root上一安装脚本用秒精度写effective_from，违反严格 UTC nanosecond时间格式，导致后续激活校验宽映射proposal_missing及59项未激活模板。保留原ACT/审批、两次拒绝演练、只改同瞬时时间格式的修复记录及独立诊断；61项约束未降低。新安装使用9位纳秒，实际新配置 hash `bccc907659fbf1464294de3e53d4f35adec89b2b87660280a5f64806fe110edf`，生效 `2026-10-06T01:14:01.833356000Z`。本节纠正上节旧activation格式，不将旧启动/采集证据扩成有效selection资格。

09:15四源 available 合计71：20/19/20/12。09:16 fresh heartbeat/snapshot但全局仍Frozen/Unsafe，缺Quote/Kline/MoneyFlow/News/OrderBook；News的逐记录资格/状态正在诊断。部署回执及静态回退在 `/Users/zhangzhen/.local/share/stock-analysis-news-global-health-rollout-20261006`，final receipt SHA `b8ca8490b8d7fb114875958e901979487b95d725809ad01d41d6842d7bad70eb`。

本版已有真正空instruments公共importance→完整audit/readback→同Gate/v8，原现5次工作/40次检查/5槽保持。公共Global实时批次仍受交易时段门，正在独立修正；未称假期即时推送、远端Accepted/N02自然窗口/30秒达成，也未部署资金或原生平台改动。


## 10:14 CST 后继：假期公共新闻与实际拒绝原因

新闻提交 `813bfc19a500609bda58d2e254e5c36166d4584c` 已部署。公共宏观新闻可在交易时段外进入原有单 worker，股票新闻仍沿用行情与时段准入；新增只读 NewsHealth 拒绝原因日志。新10项及独立复核通过，release与同制品61项隔离dry-run通过。monitor PID93285、bridge56417；两原数据库dev/ino保持，未复制或替换。未来activation为 `2026-10-06T02:13:14.965911000Z`，最终部署回执SHA `252250c6f9e9ea0880609d585b4b18b5fb842ac886cc54fc74a92fce236a2c73`，位于 `/Users/zhangzhen/.local/share/stock-analysis-news-public-session-rollout-20261006`。

初次三源observation晚于消费者wall被拒、金十发布内容过期。源码确认batch墙钟采样位于四请求完成之后，尚未证实本地采样顺序或远端时钟故障。10:18:58真实金十fresh2触发Updated；10:19:45健康快照missing不再含News，整体仍Frozen/Unsafe，缺Quote/Kline/MoneyFlow/OrderBook。这只是该冻结窗口的新闻健康证据。

真实AI轮次另暴露来源时间parser拒绝unix-ms/上海naive格式、模型将uncertainty输出为数字而严格schema要求解释字符串、少量schema成功后复合audit拒绝。新修复正在隔离Source阶段；没有通过放宽时效、评分、回执或配额恢复推送，尚未观察真实N01远端Accepted。已开始将NEWS修复三路合并到DEV，保留既有P05/OutcomeTracker/平台Source，未把合并中代码部署或声称完整平台完工。
