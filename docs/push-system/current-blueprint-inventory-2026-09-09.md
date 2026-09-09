# 当前架构蓝图：规模、调用链与运行边界核对

日期：2026-09-09。状态：PROVISIONAL，仅为新版蓝图输入核查，不是完整蓝图或生产验收。Rust/Cargo/README事实基线=`aef7972965f610ed418049593dfff1d55341772e`；初核HEAD=`1a105863f7e380e3b7f5fa916ea9226d0e498e95`，追加核对HEAD=`6e58f1e2d79bd183451c96d0dbfc9ab1db9f20f3`，这些文件相对基线无差异。没有运行Cargo metadata、编译、Rust测试、monitor或生产查询；并行文档工具测试不属于本核查的运行时证据。

## 统计口径与旧快照差异

旧[蓝图](../Project_Architecture_Blueprint.md#L8)记录514个Rust文件/379107行、61个公开模块。旧MD/HTML属于八份冻结输入，本核查不覆盖它们。新统计采用Git跟踪文件和物理行数，而不是推断运行状态。

| 项目 | 当前静态结果 | 证据与限制 |
| --- | --- | --- |
| 全仓Rust | 594文件 / 445884行 | `git ls-files '*.rs'`；NUL路径列表交 `wc -l`，包含测试/bench/build脚本 |
| src Rust | 546文件 / 432363行 | 跟踪文件中的 `src/**/*.rs` |
| tests Rust | 46文件 / 13251行 | 41个顶层候选加5个嵌套支持文件，不是46个独立测试target |
| bench / build.rs | 1文件/121行；1文件/149行 | 根 `benches/intraday_tick.rs` 与 `build.rs` |
| manifest/package | 一个Cargo.toml，stock_analysis 0.1.2 | [Cargo.toml](../../Cargo.toml#L1)，无 `[workspace]`；没有把静态检查称为metadata结果 |
| 公开library顶层模块 | 62 | [lib.rs](../../src/lib.rs#L10)的顶层 `pub mod`，新增[push_foundation](../../src/lib.rs#L58) |
| binary候选 | 28个唯一名称 | [Cargo显式4项](../../Cargo.toml#L11)加静态auto-discovery候选，重名去重；resolved targets未验证 |
| integration-test候选 | 41 | `tests/`顶层 `.rs`，manifest无 `[[test]]`；嵌套support不另算target |
| gRPC consumer目录 | 静态声明40项 | [implemented_operations](../../src/grpc_contract/ops.rs#L77)；本轮未执行其测试，也未验证生产可达性或host capabilities |

全仓Rust比旧快照多80文件/66777行。推送审计的548项则是 **546个src Rust + Cargo.toml + Cargo.lock**，由[code_path?](../../scripts/architecture-docs/catalog.rb#L336)定义（校验工具源码c2e33a2，ff94eca未改工具），不含tests/benches/build.rs；它与594个全仓Rust文件不是相互矛盾的数字。不能只凭push manifest覆盖就宣称全项目蓝图已全面审计。

## 代码热点更新

| 代码域 | 旧快照物理行 | 当前物理行 |
| --- | ---: | ---: |
| src/bin | 75666 | 76769 |
| src/database | 73767 | 74566 |
| src/push_foundation | 未列 | 47285 |
| src/data_gateway | 30555 | 30873 |
| src/selection | 25967 | 25967 |
| src/durable_delivery | 18856 | 23690 |
| src/monitor | 12853 | 23600 |
| src/event | 13159 | 13840 |

performance 15697、pipeline 14893、grpc_client 3074、grpc_contract 930、market_domain 1968行与旧表相同。这些是规模事实，不是复杂度或业务迁移完成率。

## 运行入口及不能推导的结论

- 默认CLI由[default-run](../../Cargo.toml#L5)指向stock_analysis，其[main](../../src/main.rs#L25)和[monitor main](../../src/bin/monitor/main.rs#L4471)存在；这不证明当前进程、部署或唯一实例状态。
- 外部provider-host不是本package的服务端，见[README边界](../../README.md#L28)及[独立部署说明](../../README.md#L99)。[build_server(true)](../../build.rs#L24)仅生成API；测试fixture的server不能画成生产server。
- Foundation是当前公开library能力，[模块说明](../../src/push_foundation/mod.rs#L1)明确不选择或迁移生产数据库。对src排除该模块后搜索push_foundation，仅发现lib.rs的公开声明；两个main没有直接引用。仅凭此不能证明生产接线，应继续保持“库级实现、生产集成未验证”的边界。
- 新版仍需逐域核对配置、CI、数据流、存储、错误处理及生产调用关系，结合[当前推送审计增量](current-code-audit-delta-2026-09-09.md)与正式current机器目录后再生成第二HTML。12项静态核查不能替代全架构覆盖。

按 `architecture-blueprint-generator` 的证据分层，将源码事实、静态target候选与未验证部署分开；未把旧蓝图的CURRENT标签或旧metadata结果直接沿用为新验证。

## 认证、配置与日志的必要更正

以下追加核对针对旧蓝图§17/19/20；Rust事实仍为aef7972，CI声明为1a10586引入且7a150b2未修改。仅静态读代码/公开模板，未读取 `.env`、真实凭据、生产库或运行服务。

| 主题 | 当前可证明的事实 | 旧说明应如何收窄 |
| --- | --- | --- |
| Operator认证 | [CLI入口](../../src/main.rs#L27)及[winrate入口](../../src/bin/winrate_simulator.rs#L101)调用PAM；[required默认false](../../src/auth/operator.rs#L52)，未启用则直接成功 | 这是opt-in CLI认证；函数注释虽写monitor，但不能据注释宣称daemon已调用，更不能写默认强制认证 |
| gRPC凭据 | [Bearer敏感标记/Zeroizing](../../src/grpc_client/auth.rs#L25)、[bundle无Debug与秘密零化](../../src/grpc_client/bundle.rs#L10)、[路径/读取替换保护](../../src/grpc_client/bundle.rs#L145) | 未发现0700/0600权限校验，旧文件权限断言撤回；源码保护不等于生产配置已验证 |
| 配置加载 | [monitor启动load_all](../../src/bin/monitor/main.rs#L4798)，[load_all](../../src/config.rs#L646)加载strategy.toml与chain.toml，存在保留旧快照/默认值或unavailable分支 | 两份TOML不是全部配置；design_contracts.toml属于治理工具，不是Rust runtime配置 |
| 热重载 | [CLI schedule](../../src/app/schedule.rs#L174)每轮覆盖式读取 `.env`；monitor的TOML只在启动加载 | 不扩大为TOML或所有配置热重载；未读取真实环境值 |
| 市场连接 | [首次桥构造](../../src/data_gateway/grpc_source.rs#L1063)读取地址/bundle；[LocalBridge token](../../src/grpc_client/auth.rs#L12)另读GRPC_MARKET_TOKEN；[ExternalV1首次操作](../../src/data_gateway/grpc_source.rs#L2504)要求bundle并做health/capability gate | “只接受地址与bundle”不完整；无Cargo provider feature不等于只有两个环境输入 |
| business DB | [monitor固定生产身份](../../src/bin/monitor/main.rs#L3650)后覆盖进程DATABASE_PATH；[CLI](../../src/main.rs#L63)仍接受该变量 | “默认路径或DATABASE_PATH”不能一概用于monitor，不建议用环境变量绕过固定身份 |
| 日志格式 | [CLI](../../src/main.rs#L42)含本地毫秒/level/target；[monitor](../../src/bin/monitor/main.rs#L4476)是本地秒/level | 旧统一格式描述只适用于CLI |

主线另直接读取operator默认值及monitor数据库绑定函数，确认上述关键边界；没有切换认证模式或改生产数据库路径。

## 可观测性、CI与启动说明

- [事件bus](../../src/event/bus.rs#L46)提供进程内counter snapshot；[MonitorMetrics原型](../../src/bin/monitor/metrics.rs#L1)能构造Prometheus registry/text，但静态未发现composition root注册、实例使用或9090 listener，应保留INACTIVE prototype。未发现OpenTelemetry/tracing exporter不等于运行环境没有旁路监控。
- [typed重试](../../src/grpc_client/retry.rs#L13)有界，InvalidArgument/Unauthenticated/PermissionDenied/Unimplemented四类不会被retryable=true升级为重试；不能把这扩大为所有合同错误，FailedPrecondition边界见下文。[生命周期监督](../../src/bin/monitor/main.rs#L4218)及[终止路径](../../src/bin/monitor/main.rs#L5548)支持异常退出2。“所有同步边界都由spawn_blocking隔离”未经全面证明，不沿用此全称。
- [Rust CI](../../.github/workflows/ci.yml#L17)已声明Rust安装前的strict文档门禁，其后保留fmt及all-targets/all-features的clippy/test。旧蓝图需补此声明；checker源码7a150b2的实际draft/strict验证分别107/112错误，是前置版本证据，不作为正在实施的current审计结果。远端run未执行。
- [compliance workflow](../../.github/workflows/compliance.yml#L34)仍引用不存在的 `--test e2e`，当前没有tests/e2e.rs；workflow中也未接线check-no-magic-dependencies.sh all。[Coverage/Gate C](../../.github/workflows/coverage.yml#L17)与[PR模板检查](../../.github/workflows/pr-template-lint.yml#L8)仅证明静态声明，未证明实际CI成功。这些遗留问题不因新checker接线而消失。
- [README启动顺序](../../README.md#L97)是先host、独立probe、再monitor的运维建议；[probe main](../../src/bin/grpc_bundle_probe.rs#L107)是独立程序，monitor未调用它，不能把建议画成同一进程的强制调用链。
- monitor实际先取得lease/绑定审计产物、启动JSONL、加载TOML、绑定业务库并执行durable reconciliation，再将opening readiness作为后台任务启动；关键代码见[启动前置](../../src/bin/monitor/main.rs#L4564)、[JSONL与配置](../../src/bin/monitor/main.rs#L4750)、[数据库/恢复](../../src/bin/monitor/main.rs#L4940)、[后台启动](../../src/bin/monitor/main.rs#L5527)。此顺序是静态调用事实，不是本轮进程运行证明。

## monitor产物应按具体owner列出

| 产物 | 生产路径及代码证据 |
| --- | --- |
| business DB | `data/stock_analysis.db`，[mode-owned绑定](../../src/bin/monitor/main.rs#L3650) |
| durable DB | `data/durable_delivery.sqlite3`，[固定身份](../../src/durable_delivery/model.rs#L75) |
| event audit | `data/event_audit`，[dispatcher](../../src/event/dispatcher.rs#L286) |
| immutable delivery audit | `data/durable_delivery_audit`，[append](../../src/event/durable_delivery_append.rs#L214) |
| event JSONL | `data/event_bus`，[main](../../src/bin/monitor/main.rs#L3464) |
| monitor lease | `data/locks/production/monitor-delivery.lock`，[main](../../src/bin/monitor/main.rs#L3518) |
| 推送日志 | `data/push_log`，[notify](../../src/bin/monitor/notify.rs#L1598) |

`reports/`仅由特定命令/通知按需生成，不是monitor必然产物。未发现Git跟踪的Dockerfile、Compose、systemd、Kubernetes或Helm部署单元，不能据此断言仓库外没有部署。这里不验证任何路径当前存在、权限合规、内容正确或运行中的owner身份。

## CLI与数据平面的调用顺序核对

本节由主线对旧蓝图§6/7/9和实际调用函数静态核对，Rust仍固定aef7972；没有执行CLI、`dry-run`、远端请求或测试。旧注释里的 `src/pipeline/run.rs` 当前不存在，实际逐票执行在analyze.rs，不能沿用失效路径作为证据。

| 关系 | 实际代码事实 | 对蓝图/验收的影响 |
| --- | --- | --- |
| 模式选择与选票 | [main](../../src/main.rs#L76)先处理schedule及chain，之后先build_stock_list，再选LHB/review/普通分析 | 不能把所有模式画成选票之前互斥分流；LHB/review也可能先触发选票阶段的数据依赖 |
| deep输入 | [bootstrap](../../src/app/bootstrap.rs#L89)关闭macro/LHB/涨停扩展，但持仓追加仍独立；[run_analysis](../../src/app/modes.rs#L20)有非空显式stocks时重新使用原参数，否则使用装配列表 | “deep只分析过滤后的输入列表”不准确；应区分显式参数与装配/过滤结果，不改行为来迁就旧图 |
| 单票并发与失败 | [run](../../src/pipeline/mod.rs#L539)使用buffer_unordered(max_workers)，过滤None；[process_stock](../../src/pipeline/analyze.rs#L1097)有120秒超时，失败返回None | 单票失败不一定让整轮返回Err；分析结果数不等于全部候选成功 |
| dry-run | [process_stock_inner](../../src/pipeline/analyze.rs#L1137)先fetch_and_save_data，之后才检查dry_run；[取数与保存](../../src/pipeline/data.rs#L30)调用真实gateway并在已有DB可用时尝试写K线 | dry-run只跳过后续分析，不是全流程无网络/无写入模式；不能作为生产环境安全只读验收命令 |
| 数据与分析持久化 | [日线准入/新鲜度](../../src/pipeline/data.rs#L30)失败会阻断本票；K线保存失败仅warn；[模拟持仓/分析结果保存](../../src/pipeline/analyze.rs#L1199)失败则返回None | 持久化步骤有不同失败语义，不是一条统一原子事务；这是模拟持仓路径，不据此宣称券商下单 |
| 推送成功语义 | [逐票发送](../../src/pipeline/analyze.rs#L1252)失败记日志后仍Some；[汇总门槛](../../src/pipeline/mod.rs#L635)要求非空、允许通知、非dry-run、非single_notify；[汇总发送](../../src/pipeline/summary_notify.rs#L107)失败日志后仍Ok | CLI/分析返回成功不证明通知已送达；报告保存失败会传播，图表失败则继续，不能合并这些结果 |
| Gateway空批次 | [GatewayBatch](../../src/data_gateway/review.rs#L110)区分Available/VerifiedEmpty，但Available类型本身允许空Vec；[退市过滤](../../src/app/bootstrap.rs#L191)对其业务要求非空、完整且逐票身份匹配 | “带evidence”不等于任意业务都可消费空批次；具体准入在消费者/能力边界核对 |
| acquisition审计 | [audit_gateway_result_with_receipt_state_in](../../src/data_gateway/review.rs#L1242)先核provider，调用record_data_acquisition取得receipt后才返回结果；[全局入口](../../src/data_gateway/review.rs#L1372)要求已初始化DB | 审计追加失败会拒绝该路径的batch，但不能仅凭这个helper存在声称所有gateway调用都已接线 |
| 惰性桥与并发 | [bridge_for](../../src/data_gateway/grpc_source.rs#L1065)缓存Arc、首次捕获地址/bundle；[query_op](../../src/data_gateway/grpc_source.rs#L2405)从锁内clone client后在锁外await | 连接缓存与查询并发分开；不从“process-wide”名称推导严格一次构造或每轮重载配置 |
| 两类RPC路径 | [通用query_op](../../src/data_gateway/grpc_source.rs#L2405)走GrpcMarketClient；[BenchmarkBars专用入口](../../src/data_gateway/grpc_source.rs#L2427)直接使用生成RPC并保留专用审计结果；[ExternalV1](../../src/data_gateway/grpc_source.rs#L2489)独立连接/能力检查 | 不能把所有40项声明画成相同JSON dispatcher，也不能把LocalBridge协议profile称成本仓provider实现 |

## gRPC重试：不能从注释推导更强保证

旧蓝图§7称Unavailable会重查health、FailedPrecondition不会因远端metadata升级为重试；实际通用路径需要更正：

- [query循环](../../src/grpc_client/client.rs#L202)重用同一request及request_id；RetryBackoff/RetryBounded均先调用backoff再重试，没有在这个循环里调用get_health。ExternalV1首次health/capability检查是另一条控制流，不是每次重试复查。
- [retry_decision](../../src/grpc_client/retry.rs#L13)在retryable=false时拒绝所有重试；retryable=true时只强制排除上述四类，FailedPrecondition会进入RetryBackoff。无显式metadata时FailedPrecondition才默认NoRetry，不能把后一种测试/默认值外推为全部情况。
- [默认RetryPolicy](../../src/grpc_client/retry.rs#L45)最多4次总尝试，退避基数1000ms、上限60000ms；[backoff](../../src/grpc_client/retry.rs#L58)实际上未使用jitter_ms字段。旧注释提到jitter不能证明已实现随机抖动，也不能把通用策略自动套到BenchmarkBars专用RPC。

这些是静态可证明的文档更正与后续行为核对输入，不是本轮已修复重试、CLI返回语义或生产认证。源码保持原字节；是否需要改变既有业务行为，要按相应迁移任务及验收合同实施。

## 数据架构：目录、迁移能力与运行库分开

旧蓝图§14及附录D–G经限定只读核查后，不能把声明对象数量当作当前生产数据库实测：

| 源码集合 | 已核对形状 | 口径与证据 |
| --- | --- | --- |
| legacy generation-1 | 53表 / 44索引 / 63 trigger | [冻结TSV](../../src/database/fixtures/global_schema_legacy_catalog_v1.tsv#L5)，主线实际awk计数复核53/44/63；不是运行库扫描 |
| selection-v2 | 12表 / 5索引 / 53 trigger，单一mode/phase共70对象 | [基础常量](../../src/database/selection_v2.rs#L18)、[完整DDL plan](../../src/database/selection_v2.rs#L4169)；17个static trigger只是子集，另有9个stage、24个append-only及3个mode-specific trigger |
| durable schema v9 | canonical bootstrap DDL中的18表 | [版本](../../src/durable_delivery/schema.rs#L9)、[canonical DDL](../../src/durable_delivery/schema.rs#L145)；不把v3/v4/v5迁移中间表累计进最终表数 |
| durable DecisionState | 14个enum变体 | [当前声明](../../src/durable_delivery/model.rs#L1022)，旧800–814行已漂移；不是库中状态分布 |
| Foundation基础SQL合同 | 6表 / 1索引 / 18 trigger，共25个managed persistent objects | [固定registry](push-system-foundation.v1.sql#L15)；TEMP审计/probe对象不计入；25不是25张表，也不代表Foundation模块所有存储 |

selection transitional/final会替换部分声明，production/test的symbol trigger也互斥选择，不能把这些版本/模式叠加计数；见[phase与mode生成](../../src/database/selection_v2.rs#L4102)。完整单一模式计数另有[源码断言](../../src/database/global_schema_catalog_v1.rs#L794)，本轮未运行该测试，更未查询生产phase。

[global schema catalog](../../src/database/global_schema_catalog_v1.rs#L1)明确只提供数据库侧目录证据，不拥有maintenance lease、migration、startup等权限。当前[DatabaseManager::init](../../src/database/mod.rs#L2442)仍走legacy初始化/迁移，最终[selection_schema_authority为None](../../src/database/mod.rs#L2654)。[verified-owner构造器](../../src/database/mod.rs#L2679)是另一种库级能力，静态未发现调用；[selection production apply](../../src/database/global_schema_v1.rs#L392)仍有拒绝边界。旧图不能把global owner画成默认启动已使用的总入口。

“属于冻结catalog”和“由增量模块负责DDL”不是互斥分类。例如[closing_valuation_item/run](../../src/database/fixtures/global_schema_legacy_catalog_v1.tsv#L16)已经在冻结53表中，[run_migrations](../../src/database/mod.rs#L2995)仍调用其模块owner建表。旧“增量owner一律不属于53表”的全称撤回；其他模块重叠不能由这一例外推断已经全部核对。

Foundation的[冻结SQL](push-system-foundation.v1.sql#L5)仍自述PROPOSED/仅临时库验证，是规范材料角色；同时[FoundationSchemaMigration::apply_to](../../src/push_foundation/migration.rs#L101)已经有库级实现，接受调用者路径、运行固定DDL并事后attest。这两种事实不能混为“迁移器尚未实现”或“生产已迁移”。[模块边界](../../src/push_foundation/mod.rs#L1)仍明确不选择/迁移生产DB，当前main/monitor的生产调用未证实。新蓝图应描述实际库能力并链接规范，不复制未来DDL为生产现状。

本节未读取或连接任何数据库；没有运行迁移、descriptor检查或生产schema attestation。readiness/activation的其它存储按下节实际owner/连接来源单列，不能用基础SQL的25对象替代全模块盘点。

## Readiness与activation：两套数据库边界，三种版本口径

主线读取实际DDL、schema validation、store读写及activation消费函数后，确认不能把Foundation所有存储都画进基础SQL的同一张库图。

| 边界 | 源码事实 | 不可推导的生产结论 |
| --- | --- | --- |
| 独立readiness schema | [版本1](../../src/push_foundation/readiness_store_schema.rs#L24)；[DDL](../../src/push_foundation/readiness_store_schema.rs#L30)包含schema、snapshot、recovery_event、head四表及9个触发器，没有显式CREATE INDEX | 不等于现存生产库已有4表；SQLite自动索引不计入“0个显式索引” |
| 初始化与文件身份 | [initialize_database](../../src/push_foundation/readiness_store_schema.rs#L147)接受调用者path/namespace；[创建流程](../../src/push_foundation/readiness_store_schema.rs#L201)先构造内存镜像，再create_new认领、写入并校验自有文件 | 代码没有选定固定生产readiness路径；本轮没有创建或打开此数据库 |
| 同库兼容边界 | [validate_schema](../../src/push_foundation/readiness_store_schema.rs#L625)重建期望DDL，比较全部非sqlite_*对象，另核唯一header及外键 | 额外放入基础库25对象会违反此schema相等要求；它不是基础库同一main schema的追加迁移 |
| store能力 | [ReadinessRecordStore](../../src/push_foundation/readiness_store.rs#L114)仅借用配置、构造不I/O；[读取](../../src/push_foundation/readiness_store.rs#L144)使用只读事务；[append](../../src/push_foundation/readiness_store.rs#L172)进入写事务，实际[追加及head CAS](../../src/push_foundation/readiness_store.rs#L298)有实现 | 这是已实现内部库能力；本次排除测试文件的src搜索未发现initialize_database或ReadinessRecordStore::at的组合调用，不作为生产接线证明 |
| payload版本 | [snapshot/material domains](../../src/push_foundation/readiness_snapshot.rs#L21)存在v2/v3，[版本上下文](../../src/push_foundation/readiness_snapshot.rs#L61)决定[canonical domain](../../src/push_foundation/readiness_snapshot.rs#L558) | payload v3不是readiness DB schema v3，不定义第三套表或证明数据库已升级 |
| activation读取 | [inspect_raw_activation_facts](../../src/push_foundation/activation_store.rs#L45)接受caller数据库，经rollback-only读连接和[基础schema attestation](../../src/push_foundation/activation_store.rs#L60)，读既有manifests/journal | 有[deployment set内部调用](../../src/push_foundation/activation_readiness.rs#L349)，不能写“全仓没有调用”；仍未找到main/monitor接线，内容检查不授予运行或owner权限 |
| activation写入 | [apply_activation_candidate](../../src/push_foundation/activation_transaction.rs#L108)接受caller已经打开的可写连接；[模块合同](../../src/push_foundation/activation_transaction.rs#L1)明确不认证连接或命令，应用coordinator需另提供真实认证与批准 | 没有新建表或自行选择DB；本次非测试src搜索只见该writer定义，不证明生产已执行或操作员批准已交付 |

限定搜索命令为 `rg -n 'initialize_database\(|ReadinessRecordStore::at|apply_activation_candidate\(|inspect_raw_activation_facts\(' src --glob '*.rs' --glob '!**/*test*.rs'`；实际结果是initializer、activation writer和reader三处定义，加activation_readiness内部调用；限定写法的ReadinessRecordStore::at没有匹配。该负向结果仅覆盖上述调用写法和搜索范围；不用名称搜索单独证明所有动态/别名调用不存在，也不把源码中的测试fixture绑定算生产调用。

因此新版应分别画出：基础SQL合同及其库级迁移/activation读写；独立readiness schema v1及其store；存入readiness表的snapshot/material v2/v3载荷。当前可证明的是这些库能力与依赖关系，不是两套生产文件已部署或W15/W16已全部完成。

## 依赖方向、Rust示例及测试证据的边界

旧蓝图§16/18/19可以保留“单package中的显式端口、并非严格分层”的结论，但关键边要对应实际代码：

- [gateway readiness审计](../../src/data_gateway/grpc_source.rs#L1427)直接依赖DatabaseManager与acquisition audit；[database的非测试转换](../../src/database/mod.rs#L704)构造selection schema类型；[pipeline持仓跟踪](../../src/pipeline/position_tracker.rs#L22)同时导入gateway、database、monitor::risk、risk及模拟执行端口。它们是现存跨层依赖，不能画成已经完成的纯分层重构。
- [database测试](../../src/database/mod.rs#L5472)确有gateway helper调用，但这条边在测试中；不能把测试反向依赖与上一条非测试依赖画成同一种生产调用，也不能据几个样例声称已计算全仓强连通分量。
- [AuthoritativeSinkPort](../../src/durable_delivery/model.rs#L1589)仍是Send+Sync trait及Arc类型别名，旧1176行证据应更新；[legal_transition](../../src/durable_delivery/coordinator.rs#L8553)仍使用显式pair白名单，旧7123行不可继续作为当前定位。trait存在与某个生产sink已绑定分别证明。
- [block_on_async](../../src/lib.rs#L121)在multi-thread runtime内block_in_place、在其它已存在runtime flavor中panic、无runtime时创建current_thread runtime；该函数本身没有超时。[with_timeout另一个入口](../../src/lib.rs#L177)接受显式timeout_secs，不能把顶部“默认30s”的注释套到所有同步桥调用，也不能据注释认定所有同步调用都已隔离。
- [unified_data_architecture](../../tests/unified_data_architecture.rs#L142)扫描具体host/import/transport规则；[其reqwest检查](../../tests/unified_data_architecture.rs#L101)依赖精确路径白名单及按首个cfg(test)截断的文本规则，不是Rust调用图证明。[test_design_contradiction](../../tests/test_design_contradiction.rs#L26)执行的是阈值配置/源码约束脚本，不验证通用依赖环。两类测试的源码存在不代表本轮已运行或覆盖全部架构边界。
- [intraday_tick bench](../../benches/intraday_tick.rs#L9)实际引用criterion，但[决策函数](../../benches/intraday_tick.rs#L36)是bench内定义的mock逻辑；不把它的结果当生产monitor tick、数据库延迟或真实业务吞吐量。本轮未运行bench或确认resolved harness。

这些补充只修正蓝图证据口径；没有变更运行时的同步桥、状态机、依赖关系或测试策略。

## 四时段统计须使用规范目录的主归属

主线对[历史机器目录](push-capability-catalog.v1.json#L7)实际解析计数，并与尚在最终验收中的current材料逐kind比较`kind/primary_phase/status/producer_ids`，结果完全一致。旧蓝图§24.3–24.6小标题的5/7/22/31不能沿用为规范primary_phase统计：

| 主归属时段 | 规范kind数量 |
| --- | ---: |
| 盘前 | 10 |
| 集合竞价 | 6 |
| 盘中 | 21 |
| 盘后 | 28 |
| 合计 | 65 |

同一规范的状态计数为ACTIVE 36、INACTIVE 22、STARVED 5、OPT-IN 2。计数依据是JSON的kinds数组，不是消息条数、实际运行任务数、已迁移Unit数或全部时段触发次数；跨时段producer仍保留自己的phase_epics和occurrence，不因选择一个primary_phase改变调度。新版须按正式目录的这套口径生成或引用，并保留102 producers/52 Units的独立含义，不用旧蓝图小标题重新分配身份。

本次比较时current catalog SHA为`7c7f485a7cf15a38d9bb10bfb867b2ecbba1f35353f49545afd35ab37e0a1882`，manifest SHA为`221fefb01241bcb7b925b15d4490f07839fa04ec2281de59ae96ee8c5f3a6240`；这只记录所读材料身份，最终是否符合全部语义/门禁仍需前置Task冻结后验收。新蓝图不得用上述计数或暂冻SHA替代正式验收结果。

## Selection：进程开关、发布材料与真实stage接线分开

旧§10–11经并行只读核查及主线复核关键条件后，应区分以下层次。未读取环境值、执行activation gate或查询生产schema；不据文件日期推断当前进程已Enabled。

| 层次 | 当前事实及证据 | 旧蓝图应如何改写 |
| --- | --- | --- |
| 服务启动 | [service_enabled_from_environment](../../src/selection/process_bootstrap.rs#L268)将MONITOR_ENABLED转小写后与true比较，不trim；[分类](../../src/selection/process_bootstrap.rs#L275)在无显式参数且未启用时Disabled；[main](../../src/bin/monitor/main.rs#L4545)直接返回 | 不是大小写敏感的精确true，也不是所有显式子命令都受无参数服务开关限制；selection评估只在Operational分类中发生 |
| selection发布材料门 | [evaluate_production_selection_v2_activation](../../src/selection/activation_gate.rs#L40)按当前时间及checked-in材料检查有效期、准备证据和calendar路径；[calendar_authority_complete](../../src/selection/activation_gate.rs#L128)仅检查三路径exists | 没有独立env opt-in不代表总是开启；三路径存在不等于日历内容/来源身份已完整认证；这是进程侧能力判定，不是DB schema放行 |
| 默认数据库边界 | gate的[schema排除说明](../../src/selection/activation_gate.rs#L16)与[main布尔投影](../../src/bin/monitor/main.rs#L4604)分开；前文DatabaseManager默认仍无amended authority | disabled日志的providers/database/sinks/schedulers零计数限定selection能力，不是整个monitor无网络/数据库副作用；也不能从该日志反证其它功能未运行 |
| 已接线消费者 | [新闻初始化](../../src/bin/monitor/main.rs#L4719)受selection开关控制；[同tick消费者](../../src/bin/monitor/main.rs#L7821)另要求交易/竞价session，调度NewsAI并调用候选入池 | 当前Track A实际[候选链](../../src/bin/monitor/news_aggregator_init.rs#L1169)是ticker LLM→execution_quote→push_recorder，写legacy pushed_stocks；不是完整v2 ingress/admission/sample阶段链 |
| v2持久化owner | [SelectionV2PersistenceOwner](../../src/selection/persistence_v2.rs#L67)已有按值请求入口，[commit_production](../../src/selection/persistence_v2.rs#L153)自行获取DB/connection/audit writer | 限定非测试src搜索commit_config_activation、commit_generation、commit_source_ingress只见声明；接口规则可保留，但不能画成当前业务已经组合执行，亦未验证包外调用 |
| outcome调度 | [post_session_review_scheduler](../../src/bin/monitor/main.rs#L6252)每60秒、selection开启时调用settle_tick；[settle_tick](../../src/selection/outcome_v2.rs#L1083)在无amended-schema authority时返回默认summary | 这条scheduler调用确实存在，但进入函数不等于走到provider或落outcome；不把源码注释的“队列全空”当生产查询结果 |

这几层不是新增合同：它们解释当前生产入口与已实现库能力的真实差距。新蓝图可展示v2 owner内部的阶段顺序，但必须与当前Track A及outcome早退路径分别画出。

## AI与Agent：三套调用栈及不同失败语义

旧§15不能用“caller→registry→provider”覆盖全部AI调用，也不能写“默认分析不编排多agent”。当前调用栈至少要分为：

| 调用栈 | 实现入口/消费者 | 能力与失败边界 |
| --- | --- | --- |
| LlmRegistry + LlmProvider | [registry](../../src/llm/registry.rs#L28)，NewsAI、ticker等按role选择 | [select](../../src/llm/registry.rs#L86)从已加载provider中选首个可用项，fallback发生在选择阶段，不是请求失败后自动跨provider重试；实际keys/网络可用性未验证 |
| GeminiAnalyzer多agent | [独立配置](../../src/analyzer/mod.rs#L306)、宏观、标准分析及[6-agent文本流水线](../../src/agent/multi_agent/mod.rs#L36) | 有独立provider/config选择，不经过LlmRegistry；宏观和重点股链的准入/降级分别列在下方 |
| AgentRunner ReAct | [备用run_react_analysis](../../src/deep_analyzer.rs#L349)自行装配client、tools、validation及critic | 不是当前standard/deep/review的统一中心；[迭代耗尽](../../src/agent/loop_runner.rs#L416)若有草稿会返回带未通过警告的Ok报告，不是所有critic失败都返回Err |

具体区别：

- [ticker extractor](../../src/llm/ticker_extractor.rs#L53)使用普通chat_json，不要求上游receipt；输入为空或解析不出hits会得到空列表，API错误经`?`传播。[实际入池caller](../../src/bin/monitor/news_aggregator_init.rs#L1169)对无provider/错误warn后返回(0,0)，没有头部注释所称chain-mapper关键词fallback。没有入池与错误原因需要分别记录，不能声称所有LLM调用都有receipt。
- [NewsAiProducer构造](../../src/bin/monitor/news_ai_shadow.rs#L168)在test process不装analyzer；[candidate_execution](../../src/bin/monitor/news_ai_shadow.rs#L145)将新assessment和已有审计结果恢复分开。无模型可拒绝新分析但仍在delivery gate允许时投递已有结果；有模型但不能delivery时可只创建assessment。[模型adapter](../../src/monitor/news_ai.rs#L1695)强制receipt、[45秒期限](../../src/monitor/news_ai.rs#L34)、失败不换provider，legacy keyword接口明确拒绝。不能写“模型不可用就整个NewsAI停止”。
- [宏观入口](../../src/app/bootstrap.rs#L88)在非deep-analysis下MACRO_AI_ENABLED默认开启；[宏观流水线](../../src/analyzer/macro_rec.rs#L73)的MACRO_AGENT_PIPELINE也默认开启，4专家后融合，失败回单prompt。实际是否有可用模型/新闻仍另受准入，不把默认配置当生产请求证明。
- [标准pipeline](../../src/pipeline/mod.rs#L350)仅非空GEMINI_API_KEY才构造其ai_analyzer；[深度增强gate](../../src/pipeline/mod.rs#L561)要求非dry-run及有analyzer，之后[重点股选择](../../src/pipeline/mod.rs#L405)默认最多15只，每只300秒期限，DEEP_ANALYSIS_CONCURRENCY默认3，实际[buffer_unordered](../../src/pipeline/mod.rs#L485)并发。头部“顺序执行”注释过时；单票失败/空结果/超时保留标准分析。并发3是stock级，不是系统所有LLM请求的总并发上限。
- AI_AGENT_PIPELINE在[配置](../../src/analyzer/mod.rs#L349)默认true，但限定src搜索只见字段构造/存储，未见它约束[run_text_pipeline](../../src/agent/multi_agent/mod.rs#L36)；不能把“总开关”注释当真实控制效果，更不能建议依赖它证明多agent已关闭。
- ReAct默认[validator集合](../../src/agent/validation.rs#L73)只有GrossMargin及ConsensusDeviation；[重复工具保护](../../src/agent/loop_runner.rs#L167)按tool与canonical args计数，第三次才阻断。它们是具体保护，不是任意模型输出可信或critic已通过的保证。

以上均为源码核查；没有调用模型、读取凭据、改AI开关或运行交易/推送。AI默认值、选择期fallback、receipt准入和未通过草稿是四种不同事实，新蓝图及后续优化不能混为一种“已安全降级”。

## v18/v19：实际来源与旧蓝图转述分开

[Q58](grill-decisions-2026-09-02.md#L76)批准纳管九份来源，[固定来源目录](../../design-source-catalog.v1.json#L10)逐份保存原始SHA、自声明版本/状态与裁决。当前隔离树实际九份为v18.1–v18.5、v19推送模板目录及v19.0–v19.2；本次以实际文件列表与目录核对，不从旧蓝图章节标题推断还有原文。

旧蓝图另提及[v18.0系列](../Project_Architecture_Blueprint.md#L1668)及[v19.3](../Project_Architecture_Blueprint.md#L1746)，但当前隔离树这两个来源目录没有对应独立原文，也没有其README入口。新版可以保留“旧蓝图历史转述，独立原文未纳管”的覆盖说明，不能虚构已阅读这些原文、其当前SHA或不存在的文件链接；这不代表用户其它工作树或包外资料不存在。

[v18.2–v18.5来源记录](../../design-source-catalog.v1.json#L23)的目录名与自声明v20.x/v20.0有明确冲突；[v19模板记录](../../design-source-catalog.v1.json#L75)是历史57-kind快照；[v19.0记录](../../design-source-catalog.v1.json#L86)保留退役规则指针冲突。新蓝图须逐份说明实际吸收、仍为设计及当前证据不足之处，不能仅改版本标签或沿用历史统计。冻结source catalog原有supersession指针保持，不借本次新版蓝图反写历史来源裁决。
