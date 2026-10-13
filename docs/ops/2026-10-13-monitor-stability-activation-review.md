# 2026-10-13 monitor 稳定性窄发布：精确激活审阅单

状态：**已准备，等待精确人审，未安装**。此候选替代已错过10-13 09:00窗口的[旧raw-only候选](2026-10-12-raw-news-deadline-activation-review.md)，不更改旧封存文件或沿用旧批准。新发现的scanner同步等待会阻塞新闻与DataMode，旧候选不包含此修复。

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

**未批准、拟生效时点已过或动态门禁失败，保留旧实例，不擅自更换候选或晚装。** 当前自动维护仍保持，下一步等待本精确人审；[开发与双端交接](../handoffs/2026-10-13-monitor-scanner-stall.md)记录实际源码和余项。
