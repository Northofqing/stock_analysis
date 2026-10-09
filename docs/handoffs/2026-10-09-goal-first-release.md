# 10月9日：最新版已发布

新版主程序和日常验证工具已切换到正式运行环境。周报已启用，按上海时间每周五20:30运行；独立看门狗每60秒检查一次。主程序的配置激活、数据库启动校验、投递恢复检查和新进程心跳均已通过。

目前账户仍为Frozen，数据仍为Unsafe；发布后观察到Quote、Kline、MoneyFlow、OrderBook缺失。News能力已恢复，但独立的原始新闻恢复状态仍有降级。Kline是本次启动后的当前观察，不能说与切换前完全相同。80条人工裁定项继续保留。**发布完成不等于完整数据或交易能力已经恢复。**

## 发布内容与身份

运行根：`/Users/zhangzhen/.local/share/stock-analysis-runtime`，继续使用原数据库、WAL和生产锁。消费者源码与构建来源为`a9b3aa71251393bcb356c13645dd2a4519f547d4`；后续文档提交不改变这次二进制来源。

| 制品 | 实际发布位置 | SHA-256 |
| --- | --- | --- |
| monitor | runtime/target/release/monitor | `1b8755b1c09da71467fb66f595160c03764215b0d0f5d462ffbe780653e83f43` |
| backfill_daily | runtime/bin及target/release中的对应文件 | `af62219899875445a528ae5306506492cfa337d44cd299eaf0b7161a69445092` |
| backfill_predictions | runtime/bin及target/release中的对应文件 | `d37eb6bb08be58dca103a1a4c19b7798b73e90f80ea8523243c376983ddeb95e` |
| selection_activation_prepare | runtime/target/release中的对应文件 | `30dca71d0787331efd2cb47ece3f687a13cea5f83e14f87ca9567bd09b3f9dc2` |
| 独立LocalBridge | runtime/target/release/grpc_market_server，保留现行V3 hotfix | `a13ed075def5896cc9226ef99b78a1adc51f2faa9834cd391a73149d3ad913de` |

五个离线产品工具及脚本包安装于`runtime/tools/goal-first/v20261009-f3761320c`。包名沿用首次构建标签；每个工具的实际构建来源仍以manifest为准。包manifest SHA-256为`eebdc2199d2890d5fa9955d5929103c1eb07cfc2fed30823f8f975e20a34080f`，已核对28个payload文件。weekly任务直接使用版本包内的新版程序。

新monitor PID97621、bridge PID96490，均由原launchd标签管理；二进制磁盘哈希和运行进程的可执行文件inode已连接核对。旧PID6251/36126已退出，切换时未发现其他生产DB写者或生产锁替代owner。

activation预期哈希为`78e155d2990738d7f5fe640606d8f22a8170be56328d318d35fd9701af0da9fa`，未来生效时刻为`2026-10-09T09:43:58.479692000Z`（17:43:58上海时间），已实际验证gate=enabled。复用仍有效至10月22日的既有board材料；未重新制造来源证明。

## 实际验证

- Desktop外工作树按显式生产root构建四个目标及library，release通过，耗时7分55秒。第一次误把独立bridge作为本项目Cargo目标，构建前即失败；原失败日志保留，修正后的结果独立记录。
- 核对1000个公开静态文件及六个二进制目标路径；另补齐Cargo已声明的2301字节bench文件，运行根的locked/offline无依赖metadata解析通过。bench不属于activation的src/config/Cargo/build/十个contract枚举；不影响运行行为。
- 纯离线helper由同次构建library链接，不初始化DB，不调用provider/sink；生效前not_effective、生效后enabled均核对。正式activation CLI虽已更新，本次未借它初始化生产DB。
- bridge启动校验7分21秒，历史成功范围237–555秒；真实Health ready、四项Capabilities available。盘后RealtimeQuotes仍返回no_verified_batch，不能据此称行情完整。
- 新monitor核心DB绑定69164毫秒。17:50:46启动固定点通过：progress=0、resumed_sink_calls=0、foreign=0、manual=80、schedule_hydrations=18。原主库及durable库inode保持一致。
- 只读health为有效exit1/unhealthy；进程ok、心跳新鲜、新boot一致，账户与数据限制如上。独立看门狗三次自然启动后轮询覆盖两个约60秒间隔，进程恢复与去重通过；告警仅证明本地持久化，手机通道未配置。
- weekly无RunAtLoad补跑，今晚20:30的真实周报尚待自然执行；daily原21:17日程和脚本保留，两个实际bin别名已更新，未手工启动业务回填。

开发阶段已通过的相关测试和包探针复用，本次没有重跑全量测试。完整市场时段接纳、手机送达、H08/PIT、真实lots/费用/独立close、模型价格及≥20交易日用户价值仍须单独验收。模型新功能默认0调用，SELL/连板功能保持只读预览。

## 证据与回退

[发布收据](/Users/zhangzhen/.local/share/stock-analysis-deployments/20261009-goal-first/deployment-receipt.json)、[启动证明](/Users/zhangzhen/.local/share/stock-analysis-deployments/20261009-goal-first/startup-readiness-evidence.json)、[最终健康观察](/Users/zhangzhen/.local/share/stock-analysis-deployments/20261009-goal-first/health-final.json)、[独立主程序审计](/Users/zhangzhen/.local/share/stock-analysis-deployments/20261009-goal-first/monitor-audit.md)、[独立任务审计](/Users/zhangzhen/.local/share/stock-analysis-deployments/20261009-goal-first/final-job-audit.md)。

私有部署目录保存989份旧静态源码/配置/activation/二进制文件和三个原plist，另保存原weekly plist；新加入的bench原先不存在，其单独回退记录见build-closure-addendum.json。回退需按单owner顺序停服务，恢复精确旧静态字节及匹配activation，保留当前可读取V3状态的bridge制品，再等bridge就绪后恢复monitor。不得用数据库恢复撤销发布，不重发或自动裁定Unknown/Uncertain，不删除锁、运行证据或制品包。

包安装前的隔离检查固定旧monitor基线；本次正式主程序切换造成已知漂移，不能用旧基线再次判断当前部署是否完整。当前包完整性与正式发布状态由独立包校验、静态文件核对和上述进程/activation/启动证据确认。
