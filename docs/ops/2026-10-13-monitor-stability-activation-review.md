# 2026-10-13 monitor 稳定性窄发布：精确激活审阅单

状态：**已于 2026-10-14 09:19 CST 手工切换安装并启动（晚于拟生效时点 19 分钟，用户明确授权破例）**；实际执行记录见 `/Users/zhangzhen/.local/share/stock-analysis-deployments/20261013-monitor-stability/cutover-20261014-091932/04-outcome.md`。原排期 owner `monitor-10-14` 在切换时点前已不存在于本机调度存储，09:00 无自动切换发生。此候选替代已错过10-13 09:00窗口的[旧raw-only候选](2026-10-12-raw-news-deadline-activation-review.md)，不更改旧封存文件或沿用旧批准。新发现的scanner同步等待会阻塞新闻与DataMode，旧候选不包含此修复。

## 10-13 精确批准与执行安排

用户在本chat对本页完整元组直接答复 **“批准这个精确候选（推荐）”**。批准问题标识为`["request_user_input_async","call_f6fa35734fc84e9e946874a2881cb60d",0]`；不重复请求同一元组批准。私有`human-approval-receipt.json`已写入制品目录并回读，SHA-256 `b8a31266bf02f089526510d798822d6151f9db7f45c9110592562482da085a41`。接收时实际核对1001项source/candidate构建输入、封存制品、activation、现网原静态输入及回退、bridge/env/launchd plist一致；这些只读检查不代替切换时动态门禁。

专用同chat heartbeat **`monitor-10-14`** 已创建并回读为ACTIVE，安排 **2026-10-14 08:55 CST** 续办一次；09:00生效且门禁通过后启动新版本。原每日`automation`仍为08:30，10-14仅只读预检及准备，由专用任务担任唯一生产切换owner。结果记录后停用专用任务，保留日常维护。定时任务需要本机及应用在运行；错过时点按本页规则保留旧实例，不晚装。

批准后未改封存候选字节、`reviewed_at`或生效时点，未安装、重启或写数据库。原`candidate-preparation.json`和`cutover-and-rollback-recipe.json`的未批准标记保留为准备时历史状态，后继授权以这份直接人审回执为准。批准范围不覆盖H08资格、Windows联调或真实送达。

## 精确候选

| 项目 | 精确值 |
| --- | --- |
| 源码/行为提交 | `9cb3f2cba73b04b7e12f7bd7305e07cdc9d32be0`；包含原raw修复`b6063450677496d28a6ef3c4e1418daec4f10f31` |
| build输入清单 | 1001项（999静态＋2原嵌入文档），177845B；SHA-256 `8976972ffbcd42dbb250e193a9c92725be6e6b7afae53476dd949edb5e9529e5` |
| 正式monitor | 51929680B；SHA-256 `e30c721dda1ac84e79f9b40c41c0477c0d46ed9ed30c32062357bfaa452bd2b2` |
| 正式library（用于核验工具） | 232671976B；SHA-256 `b07e254c89ad815a12e978801a0ca534d6e89f859eb6a1ddcb824e719cbce304` |
| typed配置哈希 | `abc6bc51dd36ca990d708849e87155901a831d290e9ed5e5b6701fa735fdebd8` |
| 精确activation文件 | 265B；SHA-256 `af08a95e9531e3606c31240cfa4553d770b6fca8cdb557ce4ec47ee5bb27b713` |
| 候选署名字段 | `zhangzhen`；候选reviewed_at `2026-10-13T01:20:49.355380000Z`，预填字段不代表已获人审 |
| 拟生效时点 | **2026-10-14 09:00 CST** / `2026-10-14T01:00:00.000000000Z` |
| 正式编译生产根 | `/Users/zhangzhen/.local/share/stock-analysis-runtime` |
| 正式编译隔离test根 | `/Users/zhangzhen/.local/share/stock-analysis-deployments/20261013-monitor-stability/candidate` |

私有制品目录：`/Users/zhangzhen/.local/share/stock-analysis-deployments/20261013-monitor-stability`。精确activation为`selection_activation.candidate.v1.json`；正式制品仅来自`sealed-production-artifacts/`。原board有效期仍至2026-10-22 20:00CST，不重新封印。

## 范围与已完成验证

相对现网恰好5份静态源码变化：

- `src/bin/monitor/main.rs`：同步scanner加载移至`spawn_blocking`，原失败批次仍失败并沿30秒重试路径处理。
- `src/portfolio/store.rs`：元数据异步等待35秒cooperative期限；超时明确`watchlist_metadata_acquisition_timeout`、释放future，不用假名称或空批fallback。
- 原b606三文件：`src/data_gateway/grpc_source.rs`、`src/news/aggregator/raw_v2.rs`、`src/news/aggregator/raw_v2_breaker_tests.rs`，四路raw异步35秒期限与原因保留。

metadata真实runtime RED退出101/1失败；最终`cargo test --lib portfolio::store::tests::watchlist_`4通过、`cargo test --bin monitor scanner_target_loading_tests`2通过（均locked/offline/test-threads1）。原raw的41项及缓存恢复5项证据按未改变输入复用，不称本轮重跑或全库测试。当前两文件直接代码复核无发现；本轮没有独立peer，不称新独审已通过。

普通release library+monitor构建0/503.76秒；独立private-root诊断库构建0/452.82秒，typed preview0、材料verify0。实际正式monitor `--test --push-dry-run`退出0/2.44秒，日志验证其test根和`[v30] --test 完成`。这只是隔离synthetic/dry-run，不是手机送达、provider资格或真实推送验收。

诊断库SHA `201e3cd0ba285c80985164f4a2be57594da9f84d80380c7e10627034e72d2b45`只用于private候选同根API预览，**不得安装为生产库**。保留原compile-time root守卫；安装后必须使用正式root库构建的`runtime_materials_verify`核验实际runtime的精确typed配置哈希。该工具当前只对旧runtime材料成功读回`b9d1f9ec…/enabled`，未核新候选已安装；不打开DB或发RPC。

两处期限只约束可取消异步等待，不是同步I/O硬中断或远端worker停止证明。若scanner同步I/O停住，其他joined loop已隔离，该scanner批次仍不可用。不上调来源、风控、数据模式或资金权限；独立bridge、CLI、tools、env与活数据库保持。

## 最新实际生产与Windows状态

10-13 09:22CST末查：monitor24054/SHA `d459ec8e566c460aef45742e431fe87439c25c6ab83ef0b201af61850dcbcc8e`、bridge23932/SHA `a13ed075def5896cc9226ef99b78a1adc51f2faa9834cd391a73149d3ad913de`、activation SHA `2b2ca75f96f4ca8663073320554edc9bca69ad566edfd1c2b4fbdcdcffd40069`。新候选未安装；进程心跳新鲜，但账户/数据banner仍停在10-12 10:51，raw新闻快照属于旧boot。旧模式值不当当前业务资格。只读paper head/events保持3版本/3事件；durable Uncertain76、pending0、Delivered1118、RejectedDurable4007、ManualResolvedRejected10；不重放或回滚。

Windows今日独立联调窗口V1的公共关闭回执报告：Mac真实4个native调用中Health和缺失/错误Bearer控制通过；缺失客户端证书控制为`TLS_REFUSAL_NOT_PROVEN`，Caps及业务未运行。原诊断只支持`context_deadline_exceeded`，不能把超时当拒绝证书证明。Windows exact STOP完成、01:02:28Z无监听为其点时证据；本轮读取这些公共原件，不冒独立读取Mac私密stderr或重跑native控制。原恢复任务继续SOURCE-only诊断设计；本候选不重试该窗口、降TLS oracle、读Token/私钥、启动Windows或改变正式95门槛。真实同版consumer RPC、qualified历史价格/outcome和自然新闻→AI→候选→真实渠道整链仍未验收。

## 批准后的切换及回退

仓库`selection_activation_prepare`要求此精确候选人审；旧Wave0/Wave1/10-12请求不覆盖新元组。批准后、生效前重查1001输入/制品/activation、board有效期、实际launchd owner/PID/executable与单实例lease、DB身份/schema/paper head、durable原未决身份及来源边界；遇到新发布/配置或认证变化/兼容性未知停止切换，不能冒当前stale banner为fresh。缺合格来源保持Unavailable/Frozen，批准本地维护不代表H08已完成。

按原单实例流程停旧monitor/bridge，等旧PID及lease关闭；精确安装5份已审静态源码、封存monitor与activation。用正式root工具复核typed哈希；到拟生效时点再核enabled，然后先启动原bridge、再monitor。等待初始化fixedpoint、实际executable、同boot心跳以及自然账户/数据评估和raw来源恢复观测，不能只看PID、旧快照或dry-run。

失败时停新实例、恢复原5份静态文件/monitor/activation和原job owner；不恢复活DB备份、不撤销自然账本/投递事件、不重seed、不重复原19恢复/4取消或76 Uncertain投递。精确步骤和回退材料在私有目录`cutover-and-rollback-recipe.json`及`rollback/`。

**拟生效时点已过或动态门禁失败，保留旧实例，不擅自更换候选或晚装。** 本精确人审已获批准，下一步按上述安排动态复核和切换；[开发与双端交接](../handoffs/2026-10-13-monitor-scanner-stall.md)记录实际源码和余项。

## 2026-10-14 实际切换结果（后补记录）

- **时点偏离**：09:19 CST 手工安装并启动，晚于拟生效时点（01:00:00Z）约 19 分钟；用户明确授权破例（原文选择"现在手工执行"）。原条款"不晚装"本次被显式覆盖。
- **门禁**：13 项只读门禁全部通过（封存 monitor `e30c721d…`/51929680B、lib `b07e254c…`、activation `af08a95e…`/265B、build-manifest `8976972f…` 均与配方逐字节一致）。
- **安装核验**：装后 monitor `e30c721d…`、rlib `b07e254c…`、activation `af08a95e…`、5 份静态源码与候选 5/5 一致；正式 root 工具返回 `ACTUAL_RUNTIME_TYPED_HASH_AND_GATE_MATCH`（config_hash `abc6bc51…`、gate `enabled`、exit 0）。
- **启动**：bridge 先起（PID 92789，init 17m27s 后就绪，GetHealth grpc-status 0 / live=1 ready=1 RUNNING），monitor 后起（PID 96160，lease 交接为 `96160:…`）。
- **恢复证据**：主线程由 `load_watchlist → pthread_join`（切换前 859/859）变为正常 `CachedParkThread::block_on → Inner::park`；`pthread_join` 采样 0；0 panic；30 秒新增 36 行；同 boot 心跳与真实账户/数据评估均刷新（banner-v2 boot_id `96160:…`）。
- **Scanner 修复生效**：由"无限同步等待"变为"失败后 30 秒重试"（`[盘前] Scanner 标的批次加载失败，30 秒后重试`）。
- **未恢复（超出本候选范围）**：`external_transport_unavailable`（ExternalV1 证书）导致 `account_mode=Frozen`、`data_mode=Degraded`、`missing_capabilities=[Kline, MoneyFlow, News, OrderBook]`，推送仍受数据与 Frozen 限制。

完整逐项证据：`/Users/zhangzhen/.local/share/stock-analysis-deployments/20261013-monitor-stability/cutover-20261014-091932/04-outcome.md`。
