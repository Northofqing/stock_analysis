# 保留范围晨间生产观察（2026-10-10）

本轮依照 [当前范围](2026-10-07-active-scope.md) 与主仓库交接，先读取实际发布及 Git；只做生产只读核验、自然任务回读、原 Windows 任务续办和接续记录。H01/H02/H03/H09/H10/H14/H17 继续冻结，H04–H06 沿已接线的独立 paper，H16 描述性周报。没有新增源码行为、生产安装或人工裁定。

## 以实际现网取代旧待办

| 项目 | 本轮核验 |
| --- | --- |
| Git 基线 | master `5fa902994310ac9bb350adbb19597f1c81c769bf`；当前工作树 `codex/business-closeout-20261009` 从6ade快进至同一基线，无未提交源码 |
| 行为源码 | `5fb81f1fedb9c5b80d2ab6cd846f9f57d53882f5`，见 [推送重点及实际发布](2026-10-10-push-clarity-and-account-details.md) |
| monitor | PID89475，SHA `d459ec8e566c460aef45742e431fe87439c25c6ab83ef0b201af61850dcbcc8e` |
| 独立 bridge | PID88699，SHA `a13ed075def5896cc9226ef99b78a1adc51f2faa9834cd391a73149d3ad913de` |
| 进程归属 | 两个 `ps` 实际 command、磁盘文件及 `lsof` 物理 executable inode 对应；原launchd标签运行，最后exit0 |
| activation | 文件 SHA `2b2ca75f96f4ca8663073320554edc9bca69ad566edfd1c2b4fbdcdcffd40069` 与已执行发布回执相符；本轮未重新签发 |
| 发布回执 | 33693 bytes / SHA `b17fe35e94a07f4b45fd6cae963eb7625deb462eeaa0eda758df83f13b6cb5b4`，实际读取并与上述安装字节比较 |

[实际持仓 paper 初始化](2026-10-09-actual-holdings-paper-wiring.md)、[工具闭环及四条人工取消](2026-10-09-goal-first-closeout.md) 已由原用户任务执行，不重做seed、19条恢复或四条取消。08:30 CST Health只读快照：1114 Delivered、4006 RejectedDurable、76 Uncertain、10 ManualResolvedRejected，pending/deliverable0。其余76条未知仍保留，原物理Unknown没有变成已送达。

旧 10-09 晨间 `af66e214`/79条/四取消待答复，以及 Status诊断未安装的队列已过期。`2a0ddb6a` 的关联诊断现已随上述新monitor安装，本轮实际看到自然日志；不再另发原旧候选。

## 当前优先故障：上游 code8

本轮 `monitor --health --json` 实际exit1，进程/快照/心跳新鲜；Frozen/Unsafe、metrics incomplete、Quote/Kline/MoneyFlow/News/OrderBook缺失。新模拟账户仍沿10-09原估值日期，跨日不冒称实时完整。

四路 raw GlobalNews（Eastmoney/Cailianpress/Jin10/ThePaper）均 breaker open、last_successful_pull_at为空，持续 `external_query_unavailable`。生产同进程自然 FailureCorrelation 日志进一步给出 ExternalV1 GlobalNews/InstrumentNews/SecurityMetadata 的实际数值 `grpc_code=8`；standard/trailer解码观察均absent。这确认当前情报采集被拒绝，不把Health ready、进程存活或休市当成新闻恢复。

日志采样是最多256KiB尾部中的24条选定记录，每条保留原行、客户端method/profile/request关联哈希、mapping墙钟和carrier状态。已核真实进程/executable，但不是完整RPC capture、server stage或原wire；`status_mapped_at` 不能变成transport receive-complete，absent不能证明原wire是否存在空carrier。没有新增手动provider/业务探针。

### Windows 原件与续办

- 已实际读取 `WINDOWS_RUNTIME_RESOURCE_EXHAUSTION_DIAGNOSIS_20261009.json`：66295 bytes / SHA `6fbc2bab7f7ea734fde9841289c3f5508d635b0040b545c5285cc0c041c7df44`。其同版业务原件确认blocking gate拒绝；占用槽的底层挂起根因未确定。
- 已实际读取 `WINDOWS_RUNTIME_RESOURCE_REPAIR_PREP_20261009.json`：20510 bytes / SHA `b90b7a4c84803e69978cc81befda9da9cfabf6463d239f1a21003879986ec283`。一个Hq缓存重连的同线程锁重入已修并定向验证，但尚未证明生产注册TdxSmart路径进入该缺陷；补丁未形成新的已部署release，原95%质量门失败未豁免。
- 新Mac共享包 `MAC_NATURAL_RPC_CODE8_20261010.json` 为26806 bytes / SHA `6ceb64d6b15dc727a4f1873de4b2838d8e67467ba0f2e32ca40c27127597857d`；保存本轮同进程自然失败与已发布身份、原Windows两份报告的哈希，以及日任务回读。原件/凭据不纳入Git。
- 已沿原 `R08 FuturesDelivery 上游合同与部署`（`01a0e0cf-2276-7512-96ee-3a94bdfa8ca5`）成功续办；真实新turn `01a1233a-eb02-7833-80c1-3d24d1ebc6fe` 为inProgress，表示将先核包并继续生产调用路径源码定位。截至cursor9仍inProgress；Windows随后反馈生产注册相关公开接口的第二次握手回归已红绿验证，并反馈共享客户端8路调用/3轮连接断开通过，修复把stop信号与线程handle成组管理并唤醒等待heartbeat。这里是对端进行中的源码进展，Mac尚未收到最终封存源码、完整回归/发布或生产槽归属证据，不能记作根因修复或业务恢复完成。包的最终独立回读回执也未收到。

下一项优先把8个占用blocking槽映射到实际handler/Smart调用路径，形成最小回归/源码修复及可审同版候选。保留旧失败，按适用质量门、精确source/descriptor/executable、兼容恢复、单实例与原任务人审边界交付；peer消息不授予部署。上游版本改变时，Mac需匹配经过验证的新编译身份，不能从响应学习expected identity或放宽5秒门。

## outcome 与周报：自然运行已回读

10-09 21:17:00–21:24:59 的正式日任务自然运行：selection_exit0、daily_exit0、prediction_exit1、wrapper exit1。实际报告pending13、verified0、deferred13、deferred_windows24；id37–40的原7/11–12周末起点错误仍在。安装脚本与当前源码 SHA均 `81a22d730f1ab6b1b12c3ad5c5f7e76125d992ce93c20f4ccfd4ff592c4c42ec`。这证明失败传播正确，不证明预测已回填；没有改原日期、补零或掩盖错误。

10-09 20:30自然周报exit0，实际读取JSON/Markdown/evidence manifest/run-status：

- review.json SHA `25ec36b8798db922727d7f617b529e2e25429bf9858e0bcbfa1a71440ca52548`；review.md SHA `b6ad08aa93c1349219d91cf8603d1711ab13bfd2f093c5bf005450fca2e3a1a9`；evidence manifest SHA `7d67fee398deb618898e81d9e018b149cae0fa456188cb40d5448ccf0268460f`。
- 76原预测；T+1/3/5各自历史范围 revalidated_observations0、missing_qualification70、invalid6，收益均值和命中率均null；原记录对不能成为有效分母。
- 20:30发生在后续新账户和closeout工具安装之前。这次自然运行不能验收新native账户/自动三臂闭环的下一次自然任务；该新版只读执行证据见原23:56接续，下一自然周报为10-16。

看门狗自然任务last exit0，stdout持续local_observation_persisted、stderr为空；手机配置仍unconfigured，不冒称手机送达。H08完整独立逐日状态/生命周期/价格规则/PIT仍ContractNotDelivered，42正向停复牌事件与1097 ObservedOnly证券不补造历史资格。

## 验证与后继

本轮没有修改Rust、脚本、配置或运行制品；只检查实际身份、原日志、自然报告、消息派发和文档/diff。不复跑旧27测试、全量Cargo、release、回填、原取消工具或模拟试单。前序源码测试/独立复审依原精确发布记录复用，不声称新的源码验证。

持久原件与计划：开发工作树 `.planning/2026-10-10-retained-observation/`。下一步是Windows资源故障源码及同版真实业务恢复，其次是H08合格历史及有界回填、日任务与周报自然观察；日资金流不替代实时MoneyFlow，普通OrderBooks不替代strictT0。保留范围持续维护，冻结工程不恢复。
