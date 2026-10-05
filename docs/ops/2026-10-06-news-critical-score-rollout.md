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
