# H16 本地周报入口

按 [当前范围](../handoffs/2026-10-07-active-scope.md)，用现有原件生成描述性周报。独立 CLI 为 `weekly_outcome_review`，不接 monitor 调度或新的推送 kind，不调用 provider、sink、回填、初始化、迁移、seed、预算或策略配置写入。

## 运行

CLI 自动发现，不需要修改 Cargo target 注册。已有稳定、规范化的 SQLite 副本可以直接读：

```bash
cargo run --offline --jobs 2 --bin weekly_outcome_review -- \
  --database /private/path/snapshot.db \
  --from 2026-09-28 --to 2026-10-04 \
  --observed-at 2026-10-08T00:52:00+08:00 \
  --format json --output /private/path/weekly-review.json
```

实际编译使用自己的工作树和独占 `--target-dir`，不在生产缓存或正式运行根构建。运行库不应通过 `cargo run` 或 DatabaseManager 初始化；应先构建本地 CLI，随后显式指定可执行文件。默认格式是 Markdown，不传 `--output` 时写标准输出。输出文件使用 create_new；已经存在则拒绝覆盖。`--from/--to` 是 1–7 个自然日的原报告范围，日期必须规范；程序列出其中实际完成的交易日。`--observed-at` 必须为上海 `+08:00`，省略则使用当前上海时刻。盘中、休市和后续完成交易日不会被硬补为报告内的成熟日。

活跃 WAL 原库用 SQLite 只读 backup wrapper，避免已有 detached 复制路径的非空 `-shm` 边界：

```bash
python3 scripts/weekly-outcome-review.py \
  --database /absolute/path/to/source.db \
  --binary /absolute/path/to/weekly_outcome_review \
  --from 2026-09-28 --to 2026-10-04 \
  --observed-at 2026-10-08T00:52:00+08:00 \
  --format markdown --output /private/path/weekly-review.md
```

wrapper 仅用 Python 标准库，原库 `mode=ro`、`query_only=ON`，在一个只读事务内调用 SQLite backup。只在临时 0700 目录内规范化副本为 DELETE journal，并把副本改为 0400；副本非空 WAL、来源 inode 改变、缺原库、主文件符号链接或覆盖副本时拒绝。本机系统 SQLite 在关闭 DELETE 副本后可能遗留 SHM：仅在副本无 WAL 帧、所有 backup 连接关闭、只读重开确认持久 journal 为 DELETE 后，删除该临时副本的孤立 SHM；不删除原库 sidecar。SQLite backup 有 30 秒上限。临时副本在退出时清理。CLI 随后复用 `AttributionDatabaseSession::ReadOnly` 的 detached 私有读端，未调用 DatabaseManager::init，也未对原库执行 DDL。JSON 留存输入副本路径及 source main SHA-256；该 SHA 是主文件来源定位，不是 live WAL 序列、市场资格或 PIT 证据。保留当次 wrapper 命令才能定位原库路径。

局部事实缺表/缺列或 paper 完整性失败时仍产出报告，机器读端通过 `status=unavailable`、`value=null` 和 `reason` 表达缺口；报告成功不代表业务恢复。参数、源快照或输出边界失败则 exit 2。原行总量超过预测 4096 行或 paper/audit 100000 行上限时对应读段不可用，不截断分母。

`--output` 在 Unix 上使用 exclusive create 和显式 0600。wrapper 额外传递原来源标签及临时副本清理标记：JSON 留存 `input_source.original_source_label`、`temporary_snapshot_deleted_after_run`、规范化完整副本主文件 SHA-256 和请求范围。标签由 wrapper/操作人提供，CLI 不另行打开原库，不据此签资格；临时副本路径在退出后不存在。

## 每周本地运行

已提供 `scripts/run-weekly-outcome-review.sh` 与 `scripts/launchd/com.stockanalysis.weekly-outcome-review.plist`，由最终操作人安装至现有 runtime 的 `bin/` 和正式用户 launchd。模板每周五本地主机时间 20:30 运行，进程 `TZ=Asia/Shanghai`；安装时须确认主机本地时区也为上海，因为 launchd CalendarInterval 使用主机时区。模板 `RunAtLoad=false`、`KeepAlive=false`，不会在安装时触发周报或失败后无限重启。

launcher 调用同一个 wrapper，默认根为 `$HOME/.local/share/stock-analysis-runtime`，可用 `STOCK_ANALYSIS_RUNTIME_ROOT` 指定已经准备好的独立运行根。只读主库为 runtime `data/stock_analysis.db`，可执行文件为 runtime `bin/weekly_outcome_review`。不加载 `.env`、不制造绑定或账户事实；如有已经批准的纸面绑定，运行环境需明确提供现有 binding，否则原 effective 读端按无绑定边界处理。

wrapper 的 `--weekly-output-root` 模式按同一上海观察时刻取得当前周一到周日自然范围；交易日、15:00 边界与成熟窗口仍由 Rust 核验日历判断。例：周五盘中只列到周四已完成交易日，休市周只列真实已完成日，周一开盘前可没有本周已完成交易日。JSON/Markdown 串行读取同一只读 backup、同一 `observed_at`，不会二次读取原库。每次在 runtime `reports/weekly-outcome-review/` 下创建独立 0700 版本目录，`review.json`、`review.md` 和 `run-status.json` 为 0600，不覆盖旧版本。

读取、CLI 或输出失败保持真实非零，并保存已经完成的原件与 `run-status.json` 的完成列表/失败阶段/退出码；例如 JSON 已完成、Markdown 子程序 exit 23，JSON 保持且周报任务返回 23。失败不会冒称业务恢复，也不会发送消息。完整副本仍在临时目录清理；成功/失败的本地报告版本不清理。30 秒 backup 上限适用于真实库；超时要保留失败并评估，不能跳过边界或改用生产初始化。

开发切片最初只验证了限定 CLI 测试目标。2026-10-08 后续已统一构建并安装普通 CLI、wrapper 和周五 20:30 plist，正式 runtime 实际只读周报运行一次 exit0，详见 [最终实际发布记录](../handoffs/2026-10-08-retained-scope-closeout-release.md)。周报不发送飞书或增加消息 kind；生产报告成功不证明资格、paper 账本或客户端投递恢复。launcher 显式使用已存在的 `/usr/bin/python3`（本机 3.9.6）；wrapper 保持标准库 3.9 兼容。

## 口径

| 读段 | 事实与限制 |
| --- | --- |
| 原表 extent | 日线表/状态表的全量原始行数与可读性，不等于独立资格。JSON source_extent_boundary 明示从日线推断的状态和 OHLC 不能证实生命周期、历史可用时刻或 PIT；资格仍由逐窗口核验，不将表原行数当合格数 |
| 交易日 | 使用仓库核验日历及上海 15:00 完成边界，列出实际完成交易日；无交易日不证明恢复 |
| T+1/3/5 | 保留原 pred_date/target_date，另外计算日历成熟日；分列本周原预测、本周成熟与截至本期历史积压，避免休市周掩盖旧欠数。有效原 pred_date 决定本周归属，其他坏 target/id/direction 仍计本周 invalid |
| 原结果对 | 仅为原 `actual_change_tN/hit_tN` 数值对计数，不直接采用旧缓存胜率；半缺、方向矛盾、非有限数或与当前端点不同的记录进入坏记录 |
| 重新核对观察 | 所有预期交易日均需独立逐日状态，且两端有效日线 close 与原结果一致。缺状态、明示停牌、缺价、未成熟、可核对但尚未记录分别列出，保持 NULL，不回填 |
| 完整预测资格 | 当前 legacy 读端缺完整窗口、历史 available_at/PIT 等资格，可靠预测样本保持 unavailable/null；后来缓存的状态不授予完整资格。重新核对仅作描述性收盘涨跌观察 |
| paper 原记录 | 快照全量原 Filled 总量/最新时间、本期状态、原未成交原因与卖出原行独立呈现；全量诊断包含未来行，非截至观察时刻。原 Filled 不作可靠胜率、执行率或可成交净收益分母。Filled 沿原经济读端 ts，NotFilled/Invalidated 使用原终态 updated_at；规范 UTC 转上海，坏时间不补造日期。完整上海时间晚于 observed_at 的原行在 future_timestamp_rows/future_timestamps 单列，不进入本期状态/未成交/退出 |
| 原 order_audit | 按实际 source / side / outcome 分组本期原尝试与原失败原因；完整上海时间晚于 observed_at 的原行在未来诊断单列，不进入本期尝试。未来诊断保留原 row ID/上海时间，Markdown 显示计数。不将缺原记录归因于用户未执行，不以这一原行统计替代审计链/成交校验 |
| paper effective | 复用现有显式绑定/LegacyRaw 的 effective 与 economic-position 读端，先验证整个原账本。累计超卖等失败让可靠 paper 样本、费用及净收益不可用，保留原原因。现有能力按日截止；任何返回的有效成交时间晚于 observed_at 时，整个可靠 paper unavailable，不在周报中另裁剪经济账本。无绑定不会制造资金资格；账户摘要不是 scope/seed |
| paper 退出 | 通过完整性检查时，只列本期闭合的完整生命周期，开放周期右删失；有原时间、fill IDs 与退出原因，legacy 缺 terminal 行数单列 |
| 费用 | 现有 `lot-rates-v1` 最低佣金/印花税情景估算分为本期 fill 成本与本期闭合周期的全部成本。实际结算费用、可成交净收益保持独立 unavailable；无 fill/闭合周期时金额为 null，不能写零收益 |
| 物理送达 | 本入口未读取独立 durable/card-to-row 回执，物理送达分母始终 unavailable；预测、观察涨幅和 paper 分母不能代替它 |
| 下周动作 | 依据实际缺口生成 H08、有界回填、原件复核与自然交易日观察事项；不自动调整策略、预算、治理、发消息或更改 Uncertain |

Markdown 的缺口正文最多展示 20 项；JSON `predictions.value.windows` 保留全部逐项缺口及原 row ID。原件超过上限时整体拒绝对应读段。旧 schema 的 T+n 列缺失会显式记录 `schema_gaps`，不通过 DDL 补列。

## 验证

```bash
cargo test --offline --jobs 2 --bin weekly_outcome_review -- --test-threads=1
/usr/bin/python3 -m unittest discover -s scripts/tests -p test_weekly_outcome_snapshot.py -v
```

使用临时数据库和可控时刻，覆盖盘中/休市、旧列缺失、ObservedOnly 日线、后来缓存资格、窗口中间状态缺失、停牌、未来/坏证据、结果矛盾、未记录结果、原日期/时间坏记录、原未成交原因、累计超卖、现有情景费用、无周期 null 与输出覆盖拒绝。测试不调用真实 provider/sink，不访问正式库，不部署或发送飞书。真实原库只读报告由当次操作人另行执行并保存证据。
