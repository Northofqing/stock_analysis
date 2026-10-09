# 小范围生产路径基准

本轮7个基准已实际测量通过，源码提交 `cec84fd9fe150bfa75117da3add7c58403495764`，benchmark源SHA256 `b3b72d75faa9bd899127fc8cb806486b116968ee13e504e70351d298cb336c7b`。源码位于 `benches/intraday_tick.rs`；没有修改生产接口权限、初始化 singleton、读取正式数据库或调用行情 provider。

## Fixture manifest

| 基准 | 固定输入 | 实际生产路径 | 测量外 setup |
| --- | --- | --- | --- |
| 到期预测第一页/中页 | native header 0/0，8,192 条预测，页长 256；一半未到期、四分之一已核验；固定 high-water 8,192 | `DatabaseManager::get_due_predictions_page` | frozen native prediction_tracker DDL/index、事务填充、公共 `AttributionDatabaseSession::ReadOnly` 的不可变副本与池构造、high-water读取和结果核对 |
| 日线批量插入 | 64 只 `TEST_CODE_BENCH_*`，固定日期 2026-10-09；全量 OHLC/MA 字段，观察数据不具行情资格 | `DatabaseManager::save_daily_batch` 的单连接/事务/UPSERT/逐行原资格失效删除 | 每轮新私有 tempdir/native stock_daily DDL/index；现有 qualified_daily_trading_status DDL直接取自 src/database/mod.rs；公共 `AttributionDatabaseSession::AppendOnly` 的 WAL/池/四组 scoped attribution schema 构造 |

写端使用 `iter_batched(..., BatchSize::PerIteration)`，每轮独立数据库；返回 fixture 所有权，池关闭及 tempdir 删除在测量外，最多保留一个 fixture。只使用现有 public 构造路由，不为 bench 建新权限。两类基准均为封闭合成输入，不能证明行情准入、生产整日吞吐、实时交易能力或历史 PIT。

## 实际运行入口与结果

原 `cargo bench --bench intraday_tick` 会为了 `CARGO_BIN_EXE` 自动构建同包所有bin（[Cargo官方目标选择说明](https://doc.rust-lang.org/cargo/commands/cargo-bench.html#target-selection)）；观察到普通库完成后继续编译monitor/多个probe，于1818.47秒主动停止，exit -2。这个调用不构成性能或测试通过证据。

随后直接 `rustc` 编译原文件，精确复用刚完成的普通非cfg(test) opt3库和同profile的6个rlib（stock_analysis、criterion、rusqlite、tempfile、hex、chrono），没有改source或重新编库。原文件路径保留，两处include_str!仍引用原冻结DDL/源码字节；编译11.86秒exit0。私有JSON记录了完整compiler argv、rlib/source SHA及原native library目录。运行必须带 `--bench`，否则Criterion普通bin只运行test模式，不产生测量。

实测命令（用上述隔离制品，所有7病例）：

```sh
./intraday_tick-isolated --bench --sample-size 10 --warm-up-time 0.2 --measurement-time 1 --noplot
```

| 病例 | Criterion时间估计 | 95%置信区间 |
| --- | --- | --- |
| veto输入齐全 | 57.481µs | 56.225–59.590µs |
| veto bias否决 | 60.976µs | 57.537–64.736µs |
| veto缺资金/财务 | 58.609µs | 56.657–59.950µs |
| veto配置关闭 | 13.459ns | 13.402–13.552ns |
| 预测到期首256行 | 1.4391ms | 1.3824–1.4986ms |
| 预测到期中256行 | 1.5473ms | 1.4757–1.6347ms |
| 新库64行日线批插 | 13.3457ms | 12.1424–14.4759ms |

总运行12.46秒exit0，sample10/warmup0.2秒/请求measure1秒；写入case采用flat sample的mean，其余为slope estimate。日线case自动扩到约1.3秒，setup/清理不计入上述写入耗时。原始estimates/sample/tukey/日志均保留，不仅保存终端汇总。

宿主MacBookPro16,1/i7-9750H/64GiB，执行时有独立incremental cold编译及Parallels/应用负载，load start约40.86/70.54/63.19、end34.45/67.66/62.29。这里只建立有界fixture和原函数可测的初次观测，**不构成空闲机器基线、优化提速或20%退化门禁校准**，不能替代真实整日流、行情准入、实盘执行或PIT验证。此前无相同输入/宿主的旧结果，不作虚构前后性能对比。

证据在私有 `~/.local/share/stock-analysis-deployments/20261009-goal-first-closeout/`：`bench-target-isolation-recommendation.json`、`isolated-benchmark-build-final.json`、`isolated-benchmark-measurement.json/.log`、`criterion-measurements.json`、`target/criterion/`。既有三组构建0测量针对提交 `5d28374`：cold563.86秒、未修改2.09秒、注释编辑43.17秒、target4.80GB；目标/profile不同，不能与本轮bench编译耗时比较。
