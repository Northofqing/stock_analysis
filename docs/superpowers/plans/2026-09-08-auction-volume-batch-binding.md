# P-02 集合竞价同批数据绑定实施计划

日期：2026-09-08。本计划属于总目标 W01–W21 / 52 Migration Unit 的真实业务接线准备，完成后仍须继续 W16 全 actor/认证、W17–W21、全部 Unit 和生产门禁，不替代或缩小它们。T4D 已在源码4c07aaa、修正667ee4a通过104项相关测试、生产Clippy目标零及限定独立复核；不重做该片。

实施状态：Task 1 已完成，源码 `0dda4cc`，最终13项相关测试通过、bin Clippy本次目标零诊断，固定 `118d87c..0dda4cc` 独立Spec/quality审查通过且无待修问题。[中文结果与取舍](../../push-system/implementation-auction-volume-results-2026-09-08.md)记录共享raw修正、完整证据和剩余迁移范围；不重派已完成故障修复，不提升整个Unit状态。

## 权威合同与当前事实

- `docs/push-system/push-system-implementation-rfc.md:1607–1615` 与 `push-system-wbs.v1.json` 的 `MU-auction-volume-owner`：外层new_items与dispatcher snapshot.items须同批绑定；逐票有限正价格/量比，失败不得insert通知set。WBS明确“先修PreparedFacts”。
- `src/bin/monitor/main.rs:9703–9773` 首次获取涨停池，排序、排除已通知代码、取10；`push_templates.rs:5937` 的 dispatcher 又调用 `load_auction_volume_snapshot_real` 获取另一批、取另一组10。成功后入池使用内层批次，通知set使用外层批次，存在可直接确认的身份错位。
- 上游 `MarketAnalyzer::get_limit_up_stocks(date)` 返回 `Vec<TopStock>`，底层已验证Gateway批次，但此旧接口没有保留BatchEvidence。本片不得把普通Vec/现有日志冒称完整Foundation PreparedFacts/source authority；完整证据捕获及新框架迁移仍留在总目标。

## Global Constraints

- 仅在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905` 开发；不操作根工作树、`.env`、已有data、真实DB/provider/sink、PAM或生产monitor，不运行monitor业务命令、不替换二进制、不改变physical owner、不晋级。
- 保持交易日Auction及09:20–09:25窗口、30秒调度、原P-02 presentation/governor/发送通道、原发送后pushed_stocks写入及失败返回语义；不顺带迁移A02/P05/T08，不修改冻结八输入/SQL/catalog或其他Unit。
- 选票、渲染、pushed_stocks入池与通知set必须来自一次采集后同一不可变选中批次。排序前排除已通知和无效票；量比/价格须finite且>0，涨跌幅须finite；有效票按量比降序取最多10，等值保留原输入次序。不补造数据，空/无效批次不发送/入池/推进通知set。
- 原legacy `bool` 仍不是新Foundation typed terminal，不扩大其权限。发送失败、缺banner和任一recorder失败均不推进通知set；后续typed receipt/崩溃恢复/cursor持久化另行完整交付。
- 主控唯一Cargo/Git/文档/审查，实施agent不运行Cargo/Git/子agent/网络/真实系统；仅apply_patch及精确格式。不对两份大型既有Rust文件做无关格式化。

## Task 1 — 实际 P-02 同批采集、渲染与游标推进

文件owner：一个fresh实现agent独占 `src/bin/monitor/main.rs` 的P-02分支、`src/bin/monitor/push_templates.rs` 的P-02 loader/dispatcher以及对应测试。如需聚合可测试逻辑，允许新建 `src/bin/monitor/auction_volume_runtime.rs`（同agent登记module），不扩到其他模块。主控独占所有docs、Cargo/Git和review。

1. 先从现有P-02 loader提取无外部I/O的选票/快照构造seam，保持原逻辑，以包含非有限/非正量比或价格的具体输入写首个失败回归。交主控运行取得实际behavior RED；不得把编译失败、复制旧bug模型或未执行的测试算RED。此阶段只做使旧行为可测的最小提取，不预先修正。
2. RED后修正选票过滤；随后把真实main分支与dispatcher接到同一准备结果：每次tick只有一个涨停池采集，dispatcher不再重取。输入为现有真实TopStock、当天日期与已通知set；消息/入池/推进只用选中批次，不能返回另一个任意codes集合。优先小interface和复用原renderer/recorder，不新增通用registry或第二套业务状态机。
3. 通过生产实际使用的seam补齐：不同先后provider批次不会串用；排除已通知后的Top10含义与消息精确一致；缺量比/NaN/Infinity/非正price或ratio、所有无效、空数据；source失败、sink失败、recorder部分失败不推进；正常成功只推进实际展示/入池代码；原模板字段与价格保持来自同一行。外部source/sink/recorder可使用有实际输出/计数的隔离adapter，不能用只验证mock调用的测试替代真实选票/渲染/推进逻辑。
4. 原始provider证据尚未保留，不伪造时间/batch hash/VerifiedEmpty/typed receipt；本片报告准确区分已修同批绑定与后续完整Foundation producer/cursor迁移。代码冻结后交主控一次相关模块GREEN与monitor编译覆盖；未变lib测试复用既有证据。

验证：首个回归与最后合批都用明确非零filter的 `env CARGO_PROFILE_TEST_INCREMENTAL=true cargo test --bin monitor <exact_filter> -- --test-threads=1`，由主控根据实际新测试module确定filter；若新module与原renderer测试分开，使用libtest多filter一次合批。不得运行全monitor测试中未证实隔离的真实外部案例。定向rustfmt/diff检查及最终 `cargo clippy --bin monitor --no-deps --message-format=json`，只核对本次增量，不清理既有告警。

报告放本计划SDD workspace `task-1-report.md`，列完整实现、实际接口、RED/GREEN命令与结果、文件、行为限制、未完成范围。agent仅报告冻结与待主控验证，不自称通过。主控用派发前BASE制作完整固定diff，独立Spec/quality审查通过后记录此故障修复完成；整个MU仍不标迁移完成。

## 依赖与回退

该准备性修复不需要生产认证配置，也不打开W16 gate；依赖既有真实MarketAnalyzer、renderer、governor、recorder和WBS业务合同。它可在平台adapter未就绪期间推进，避免把安全工程等待等同于全部开发等待。上线仍受原批准/验证门禁约束。若验证失败继续修正，不回退为第二次采集。未来若需回退代码，以独立精确提交为单位且不自动删除业务数据。
