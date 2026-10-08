# H16 周报开发验证 — 2026-10-08

工作树：`/Users/zhangzhen/.codex/worktrees/retained-weekly-closeout/stock_analysis`。
基线：`55d9eca9cca7e5581ee3362661dedc2659349b53`。
分支：`codex/retained-weekly-closeout-20261008`。

## 交付范围

- 独立 `weekly_outcome_review` CLI，自动 bin 发现，不改 Cargo、共享模块、monitor main、回填入口或日常回填脚本。
- 显式原 `--from/--to`、上海 `--observed-at`、标准输出/新建输出文件、JSON/Markdown。复用现有 `AttributionDatabaseSession::ReadOnly`、核验日历、effective fills 与 economic-position 读端。
- 标准库只读 SQLite backup wrapper；runtime launcher 与每周五 20:30 的 launchd 模板。JSON/Markdown 共享同一 backup 和观察时刻，每次新建 0700 版本目录、0600 原件；保留已完成原件与失败状态，子程序非零原样返回。
- 操作文档：`docs/ops/weekly-outcome-review.md`。

## 最终验证

```bash
cargo test --offline --jobs 2 \
  --target-dir /Users/zhangzhen/.local/share/stock-analysis-candidates/retained-scope-closeout-20261008/weekly-test-target \
  --bin weekly_outcome_review -- --test-threads=1
/usr/bin/python3 -m unittest discover -s scripts/tests -p test_weekly_outcome_snapshot.py -v
plutil -lint scripts/launchd/com.stockanalysis.weekly-outcome-review.plist
bash -n scripts/run-weekly-outcome-review.sh
git diff --cached --check
```

结果：CLI 13/13 通过（0.29 秒）；系统 Python 8/8 通过（3.163 秒）；plist、shell 语法及 staged diff 检查通过。保存干净结果 `rust-tests.log` 与 `python-tests.log`。系统解释器为 `/usr/bin/python3` 3.9.6，链接 SQLite 3.51.0；测试通过实际 launcher 调用该解释器和临时 runtime 根，不访问正式 runtime。全部 Cargo 构建限定 bin、offline、jobs=2；仅一次 clone 主 target 到独占 candidate test target，从未原地编译主 target。

覆盖真实完成日、周初/周末/休市/15:00、76 条旧待验原件、旧 schema 缺列、ObservedOnly、后来缓存独立状态、中间日缺状态、明确停牌、未来/坏证据、原结果矛盾、未记录结果、坏日期/UTC 时间、原未成交原因、累计超卖、现有情景费用、空周期 null、输出拒绝覆盖及 0600。WAL backup 用临时 live DB 验证来源 inode、catalog、main/WAL 字节和原数据保持不变，完整副本包含已提交 WAL 数据。

系统 Python 首次完整验证发现 DELETE backup 关闭后遗留 32768 字节 SHM。已复现并修复：先拒绝非空副本 WAL，在所有 backup 连接关闭后只读重开确认持久 DELETE，才清除私有副本的孤立 SHM。原库 sidecar 不删除、不规范化。修复后上述系统解释器全目标通过。此前两个 Rust fixture 使用了实际休市日 2026-09-25 和未知买入原因，按仓库真实日历/现有经济读端修正 fixture 后通过；未改领域规则。

## 报告语义和限制

- 原 pred_date/target_date 不补造；T+1/3/5 分本周原预测、本周成熟、截至本期历史，分别记录缺资格、停牌、缺价、未成熟、坏记录与可核对但未记录。逐项 JSON 保留原 row ID 与缺口；Markdown 缺口最多 20 项。
- 缺完整窗口和历史 available_at/PIT 证据，可靠预测样本为 unavailable/null。后来缓存状态只支持当前快照描述性收盘观察；原已记录结果不直接成为可靠胜率。ObservedOnly close 不授予合格窗口。
- 原 Filled、原未成交/卖出原因、order_audit 尝试独立呈现。effective 完整性失败（含累计超卖）保留失败原因，可靠 paper 费用/净收益不可用。账户摘要不授予 binding/scope/seed 资格。
- 验证通过的 paper 周期仅做既有算术和 lot-rates-v1 情景估算；分开本期 fill 成本和本期闭合全周期成本。实际结算费用、可成交净收益及物理消息送达分母保持独立 unavailable/null；无样本不写零收益。
- SHA-256 定位规范化完整 backup 主文件；JSON 标明原来源标签及临时副本会清理，不能把该 SHA/标签当资格或 live WAL 序列证明。
- source backup 超时上限 30 秒，真实 2.5 GB 主库的只读 backup/最终普通 CLI 产物/实际报告由主任务后续验证。launchd 模板尚未安装，需正式操作人确认主机上海时区和已批准运行环境。
- 未部署、重启 monitor、写正式 DB、调用真实 provider/sink、发送消息或调策略；未执行全量 Cargo/额外 check/clippy。

一次额外 `cargo build --profile test --offline --jobs 2 --bin weekly_outcome_review` 尝试因不同普通库 feature 集重新编译，按主任务要求中断（TERM，退出 143）。该普通 binary 构建未验证成功，不作为交付证据；最终普通 CLI 由统一 release 构建提供。
