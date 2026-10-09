# 小范围生产路径基准

本轮新增基准尚未测量。源码位于 `benches/intraday_tick.rs`；没有修改生产接口权限、初始化 singleton、读取正式数据库或调用行情 provider。

## Fixture manifest

| 基准 | 固定输入 | 实际生产路径 | 测量外 setup |
| --- | --- | --- | --- |
| 到期预测第一页/中页 | native header 0/0，8,192 条预测，页长 256；一半未到期、四分之一已核验；固定 high-water 8,192 | `DatabaseManager::get_due_predictions_page` | frozen native prediction_tracker DDL/index、事务填充、公共 `AttributionDatabaseSession::ReadOnly` 的不可变副本与池构造、high-water读取和结果核对 |
| 日线批量插入 | 64 只 `TEST_CODE_BENCH_*`，固定日期 2026-10-09；全量 OHLC/MA 字段，观察数据不具行情资格 | `DatabaseManager::save_daily_batch` 的单连接/事务/UPSERT/逐行原资格失效删除 | 每轮新私有 tempdir/native stock_daily DDL/index；现有 qualified_daily_trading_status DDL直接取自 src/database/mod.rs；公共 `AttributionDatabaseSession::AppendOnly` 的 WAL/池/四组 scoped attribution schema 构造 |

写端使用 `iter_batched(..., BatchSize::PerIteration)`，每轮独立数据库；返回 fixture 所有权，池关闭及 tempdir 删除在测量外，最多保留一个 fixture。只使用现有 public 构造路由，不为 bench 建新权限。两类基准均为封闭合成输入，不能证明行情准入、生产整日吞吐、实时交易能力或历史 PIT。

最小运行命令（仅此 bench 中的 SQLite 路径）：

```sh
cargo bench --bench intraday_tick -- production_sqlite --sample-size 10 --warm-up-time 0.2 --measurement-time 1 --noplot
```

Criterion 默认输出目录的统计产物必须与当次源码、构建、固定输入及宿主条件一起保存；运行前不填写耗时结论。既有三组构建测量针对 commit `5d28374`：cold 563.86 秒、未修改增量 2.09 秒、注释编辑增量 43.17 秒、target 4.80 GB；不能替代本轮新基准或改动后的增量性能测量。
