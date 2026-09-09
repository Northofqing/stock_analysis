# 当前架构蓝图：规模、运行入口与运维边界核对

日期：2026-09-09。状态：PROVISIONAL，仅为新版蓝图输入核查，不是完整蓝图或生产验收。Rust/Cargo/README事实基线=`aef7972965f610ed418049593dfff1d55341772e`；核查时HEAD=`1a105863f7e380e3b7f5fa916ea9226d0e498e95`，这些文件相对基线无差异。没有运行Cargo metadata、编译、测试、monitor或生产查询。

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

全仓Rust比旧快照多80文件/66777行。推送审计的548项则是 **546个src Rust + Cargo.toml + Cargo.lock**，由[code_path?](../../scripts/architecture-docs/catalog.rb#L312)定义（校验工具源码7a150b2），不含tests/benches/build.rs；它与594个全仓Rust文件不是相互矛盾的数字。不能只凭push manifest覆盖就宣称全项目蓝图已全面审计。

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
- [typed重试](../../src/grpc_client/retry.rs#L13)有界且不重试request/auth/permission/contract错误；[生命周期监督](../../src/bin/monitor/main.rs#L4218)及[终止路径](../../src/bin/monitor/main.rs#L5548)支持异常退出2。“所有同步边界都由spawn_blocking隔离”未经全面证明，不沿用此全称。
- [Rust CI](../../.github/workflows/ci.yml#L17)已声明Rust安装前的strict文档门禁，其后保留fmt及all-targets/all-features的clippy/test。旧蓝图需补此声明；真实当前门禁仍107/112错误，远端run未执行。
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
