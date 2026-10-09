# Goal-first 使用闭环收尾（2026-10-09）

## 当前范围

在用户“开发吧”以及既有直接发布授权下继续目标优先开发。普通release代码 `cec84fd9fe150bfa75117da3add7c58403495764` 已并入主项目并发布独立工具包 `runtime/tools/goal-first/v20261009-closeout1`。未修改既有交易monitor/bridge运行输入、activation、持仓导入或PaperLedger期初。随后用户明确批准取消四条Uncertain；为取得既有owner lease短维护停/启同制品monitor，取消和恢复均已完成。工作树起点为 `5d28374beb4c5f9d77d92310b33a0d028726ea2c`。

## 已实现

- 新模拟账户到周报：精确私有binding，原数据库path/device/inode对照不可变导入回执，同一只读SQLite backup/时间/原registry生成JSON与Markdown。新当前账户与旧期初前历史独立；缺/错绑定、复制原库、缺证明、未来截面不回退旧账本。
- 周报后自动生成一次Phase A三臂JSON/Markdown和状态收据。共用同一账户/字节/时点，默认零模型调用；错误保留周报与部分产物，不重复模型工作。账户观察是共同上下文，非历史策略结果或可执行报价。
- 盘后producer：准备/消费时间窗、读取预算、账户30秒时效、过期粘性、opaque candidate及durable admission复查。公开原始观察始终无发送资格；真实来源未齐时NotReady。没有重用paper lots或T-21确认卡冒充真实卖出建议。
- 可选独立手机适配器：本地状态先持久化，有限超时，明确受理/Unknown/退避，跨重启与事故世代去重。可变凭证路径在 `runtime/data/private_config/`，不破坏交易config activation。模板保持disabled，不复制到启用路径。
- SQLite微基准：真正public预测到期页读取（8192行、页256）与行情日线批量写入（每轮独立DB、64行）；构造、DDL、池/WAL初始化及清理不计入测量。

## 复审与验证

独立复审发现并已修两处问题：手机配置若落config会破坏activation身份；旧事故的retry事件会在同类新事故时复活。已移私有路径并匹配generation。最小充分验证共77项Rust、61项Python通过：assistant lib22、sell-reminder lib17、weekly bin33、assistant bin4、producer bin1；weekly/assistant Python24、打包8、watchdog29。未运行全量测试。周报legacy正向fixture改为已支持的NewsCatalyst codec，并额外验证未知family保持Unavailable且新账户仍可读取，没有放宽生产门。

六个独立bin普通 `cargo build --locked --offline --release` 成功（725.14秒，jobs4；不含测试helper）。33份allowlist文件按清洁提交与同次构建封装、hash/mode回验后原子发布；周报和watchdog两份launchd job已加载。周报保留周五20:30，watchdog每60秒。998份monitor绑定输入与原SHA相同，唯一改变为上述两份工具plist。没有重新seed、复制生产主库、重发activation或修改交易制品。

正式源库只读验收于10-09 23:56:54固定截面启动，175.88秒完成，exit0。JSON/Markdown、evidence manifest和一次自动三臂产物全部生成，原库native identity、actual事实和paper account/head/event字节摘要前后相同。新账户head2：现金7254.94、市值47742.00、权益54996.94、5只持仓、cutover后盈亏和模拟费用均0；估值仍来自21:13原截图，不表示实时行情。paper当日盈亏缺前日基准保持Unavailable，原实盘亏损不属于新epoch。旧2310条成交/50个未平周期及原争议金额独立分列。

三臂共用同一账户/截面/registry，artifact status=complete、model calls=0、成本=0；比较status=degraded明确保留缺少原family/version/PIT及合格历史的事实，未宣称模型运行或有效性通过。新watchdog已自然运行多个间隔，本地persist=committed，手机配置unconfigured、attempted=0；手机真实送达尚未验收。

正式周报目录：`runtime/reports/weekly-outcome-review/2026-10-05_2026-10-11__20261009T235654130784__fa6d7d14467b4418bb5dc4455d20c17a`。私有构建、检查、安装、回读与工具job回滚证据：`~/.local/share/stock-analysis-deployments/20261009-goal-first-closeout/`。

构建基线在冻结提交5d28374、独立fresh target、monitor dev/debug1、jobs4上完成。incremental=0：cold563.86秒、unchanged2.09秒、单文件注释修改43.17秒，target4798712362bytes。incremental=1另一个fresh target三次exit0：cold1176.50秒、unchanged3.22秒、单注释修改15.13秒，edit后target7002611784bytes。cold期间有Criterion自动bin编译和Parallels负载，不能将cold差异归因于incremental；warm/edit已无root编译/benchmark竞争，但其他应用负载仍保留。单文件edit观测缩短约65%，缓存增长约46%，只测一个monitor不足以启用全target矩阵默认；本轮不改全局Cargo/profile，Cargo既有复用仍有效。两份report/宿主条件/源码和工具SHA均保留在私有 `stock-analysis-candidates/closeout-performance-20261009-{nonincremental,incremental}/` 及发布目录。

7个Criterion病例实测exit0。预测首/中页时间估计1.4391/1.5473ms，64行日线批插13.3457ms；setup/清理在测量外。由于同宿主编译/虚拟机负载及仅10个样本，这些为初次观测，不能宣称优化提速或正式20%软门禁已校准。实际方法、置信区间与避免Cargo隐式全bin的方法见 `docs/performance/2026-10-09-production-sqlite-bench.md`；真实整日回放仍未验收。

## 已批准的四条取消

用户原话“4 条 Uncertain 阻断头的裁定 取消吧”已冻结并按既有已评审exact-four工具处理。三条CloseCall（600396、600703、603948）和一条T0Advice（600396）均为ManualResolvedRejected；各cooldown head和预算reservation释放，当前blocking Rolling=0、integrity issues=0。只追加本次授权/取消/审计，原物理Unknown保留，未授予重试、未调用sink/provider、未处理旧paper金额争议。

独立前后核验55,362原行保留，17行只改变明确许可字段，40条新增均属于exact4；Schema9/93对象及旧34,620,432字节不可变审计前缀保持，新增24项审计/32,624字节。原Uncertain80→76、ManualResolvedRejected6→10；其余76个未知记录保持原状。独立复核无未解决P1/P2。

为取得owner lease，monitor46478于23:50:09退出，原制品66547于23:51:56恢复；进程维护106.8秒，随后主库全校验约376秒，23:58:12同新boot完成DB bind和startup reconciliation，resumed sink calls=0。bridge44714及二进制、config、.env、activation、monitor plist和数据库物理身份不变。恢复后的process/snapshot/heartbeat fresh、account metrics complete；业务仍Frozen/Unsafe，PID就绪不等于行情恢复。

完整裁定回执：`stock-analysis-deployments/20261009-goal-first-closeout/uncertain-cancellation/completion-receipt.json`，SHA256 `089e0e556df239ce28398c798bc9284edd08781aaeb21fdc5d63a90d51f91c34`，绑定48份私有证据。不要重复运行取消工具或覆盖旧部署/activation回执。

## 外部待验收

- 合格实时行情，Windows资源耗尽修复及市场时段业务探针。Windows共享回执确认一个SDK缓存重连死锁已修且回归通过，但尚未证明它进入生产TdxSmartClient挂起路径；未部署、业务仍code8。日资金流不能代替实时MoneyFlow，普通订单簿不能代替strictT0。
- 真实可卖批次/预留/费用、15:00证券状态及数量来源合同；独立真人提醒的counted语义/owner接线、2分钟实测及正式发送。
- 手机通道/凭证及实际手机显示；HTTP成功只证明服务受理。
- 真实family/version/known-by/PIT历史，模型价格与账单合同、三臂人工评分和≥20完成交易日效果；模板生成不证明模型有效。
- 完整真实日事件capture/count receipt join后的回放。两笔历史paper争议金额仍待原裁定流程，不能从四条提醒取消授权推导出金额裁定。

E5 facade/workspace与平台工程仍按新方案冻结；真实券商执行不在本次范围。
