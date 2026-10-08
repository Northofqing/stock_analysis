# outcome 每日任务本地开发收尾（2026-10-08）

接续 [当前范围](2026-10-07-active-scope.md) 和 [最新发布及每日任务实际结果](2026-10-08-latest-code-rebuild-release.md)。本轮完成每日脚本的源码化与失败传播；没有运行生产回填、安装脚本或调整 21:17 launchd 计划。

## 源码行为

新增 `scripts/daily_prediction_verify.sh`，对应现有 plist 调用的 `runtime/bin/daily_prediction_verify.sh`。默认运行根、`STOCK_DB`、本地 gRPC 地址、10 日观察窗口和日线日志路径沿用原脚本，不读取 `.env`。`STOCK_ANALYSIS_RUNTIME_ROOT` 可显式选择另一完整运行根；必须为绝对路径，两个工具和数据库都绑定该根，脚本先切换到该工作目录。

SQLite 以只读方式选取尚有 NULL T+1/3/5 收益窗口的证券。原库不存在或查询失败时停止，不创建数据库，也不把查询失败解释为没有 pending。空 pending 仍调用预测 verifier。

日线命令失败后仍调用预测 verifier，保留已成功的写入。最终返回首个非零退出码，末尾单行 `[daily] report` JSON 同时列出 `selection_exit_code`、`daily_exit_code`、`prediction_exit_code` 和最终 `exit_code`；未调用的阶段为 `null`。两阶段同时失败时，两项真实退出码均保留。

JSON 的 `outcome_result_status` 固定为 `not_evaluated_by_wrapper`：这里只报告工具退出码，不能据命令 exit0 宣称全部窗口成熟或资格收益完成。现有 verifier 对缺数据/缺资格的 deferred 和坏原日期的 error 诊断原样保留。没有修改预测 verifier、原日期或收益合同。

## 验证

`python3 scripts/test_daily_prediction_verify.py -v`：10/10 通过，无 ignored。每项使用临时 SQLite、TEST_CODE 证券及模拟 backfill 子命令；不触发 provider 或真实 sink。覆盖双成功、日线失败仍验证、预测失败传播、双失败保留、空 pending、SQLite 原退出码、真实缺表只读失败、缺库不创建、工具缺失、绝对运行根以及两个工具的实际 CWD/数据库绑定。夹具在非零退出前写入临时表，独立核对部分成功没有回滚。

RED 使用只读取得的原生产脚本副本，仅把固定 ROOT 改为测试运行根：日线 exit7、预测 exit19、SQLite exit23 三项均因原脚本最终 exit0 而断言失败。实际原脚本未被修改。首次最终检查另暴露 macOS Bash 3.2 对未加花括号变量后紧接中文标点的解析问题；已显式使用花括号修复，原失败日志保留。

`bash -n scripts/daily_prediction_verify.sh`、`plutil -lint scripts/launchd/com.stockanalysis.prediction-verify.plist` 和 `git diff --check` 通过。没有修改 Rust、依赖、配置或 plist，未追加 Cargo、release 或全量测试。

本地证据位于开发工作树 `.planning/2026-10-08-outcome-closeout/`：baseline、副本隔离说明、RED/最终日志和输入 SHA-256 manifest。该目录不提交原始日志或私有数据。

## 未完成的外部验收

源码化不代表已安装。后续正式脚本安装和任务观察由发布入口另记，不能把夹具退出状态认作 launchd 实际回执。

此前真实预测 37–40 的周末起始日期保持原件；既有 13 项/24 窗口 deferred 和 verified0 的记录仍有效，本轮测试没有修复这些数据。用户没有历史资格产品，`QualifiedTradingFacts` 仍为 `ContractNotDelivered`。1097 证券采集仍为观测材料，不证明 Admitted/PIT，也不解除正式历史回填、D01 卡片结果和有效分母的资格缺口。
