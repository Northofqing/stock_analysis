# W16 T6 集合快照与恢复存储接线实施计划

日期：2026-09-08。原始基线在派发前固定；前序 P-02 修复已于源码 `0dda4cc`、中文结果 `69f04ba` 完成，不重派。本计划落实完整 W16 Task 6 的版本化持久消费部分；全目标仍为 W01–W21、完整 W15/W16、52 Unit 和真实发布门禁，不以候选持久记录替代认证就绪。

## 权威合同与现状

- [全 Unit 集合合同](../../push-system/activation-deployment-set-contract-2026-09-08.md) 已确定 `ActivationDeploymentSet/v1`、`OperationalReadinessSnapshot/v3`、`OperationalReadinessMaterial/v3` 和 `OperationalReadinessStream/v2`；旧 snapshot/material v2、stream v1 字节及语义不变，legacy snapshot v1 与未知版本继续拒绝。
- [完整 W16 设计](../specs/2026-09-08-push-foundation-w16-activation-design.md) 和[实施计划 Task 6](2026-09-08-push-foundation-w16-activation.md)要求全 Unit 集合进入实际 snapshot/material、恢复记录、store 和最终查询，而非选最大代或用某个 Unit 冒充 Core。
- 当前 `activation_readiness.rs` 已有全 catalog 候选集合、真实临时 activation 库读取与重读；`CandidateReadinessSnapshot`、`CandidateReadinessRecord` 和 `ReadinessRecordStore` 仍只支持单代 v2/v1。`readiness_probe.rs` 当前是纯目录候选清单，没有真实来源认证或 snapshot 查询。不得把此清单重命名为已认证 probe。

## Global Constraints

- 只在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905` 开发；不碰根工作树、`.env`、真实 data/DB/provider/sink/PAM/生产monitor，不替换二进制、不改变owner、不执行批准或上线。主控唯一Cargo/Git/文档/审查；实施agent不运行Cargo/Git/网络/子agent。
- 保留旧v2 snapshot/material、stream v1精确bytes/hash/字段语义与旧v1拒绝；新增内容不得套旧domain。保留冻结recovery event v1/domain/schema、readiness SQLite schema、Foundation SQL及八份输入，不能为方便版本支持改DDL或重写历史。
- 使用现有实际snapshot→recovery record→ReadinessRecordStore写入/加载调用链；不得只增加未使用的第二套codec/writer。全Unit集必须显式保留Unregistered和逐Unit generation/build/owner，不造Core scalar或自由构造的Verified/source authority。
- 相同版本/集合/日期/scope的恢复沿用现有显式claim及时间连续性校验；不同版本或集合的previous snapshot不得隐式续接。显式新stream起始不表示旧故障已恢复，旧stream及其pending记录必须保留。真实认证恢复衔接仍由完整T6/T2/T5接线，不以重新起链绕过它。
- 拒绝/解码错误只输出稳定类型和字段标签，不泄露protected URI、source/binary包或原始持久bytes。格式化限实际改动文件；不重复运行未改库基线，不因测试数量多声称生产已验收。

## Task 1 — 集合版 snapshot/material、恢复和实际存储的显式分派

### 文件 ownership

一个fresh实现agent独占：`src/push_foundation/activation_readiness.rs`、`readiness_snapshot.rs`、`readiness_snapshot_codec.rs`、`readiness_recovery.rs`、`readiness_recovery_codec.rs`、`readiness_store.rs`、`operational_readiness.rs` 及对应的 `activation_readiness_tests.rs`、`readiness_snapshot_tests.rs`、`readiness_snapshot_codec_tests.rs`、`readiness_recovery_tests.rs`、`readiness_recovery_codec_tests.rs`、`readiness_store_tests.rs`、`operational_readiness_tests.rs`。允许新建私有 `activation_readiness_codec.rs` 和专用 `readiness_deployment_set_tests.rs` 以避免现有文件膨胀；新module登记由主控修改 `src/push_foundation/mod.rs`，代理明确报路径。不得扩到真实monitor/transport/auth或改W15底层SQLite锁算法。

### 实现要求

1. 用封闭版本类型区分现有单代上下文和集合版上下文，共用namespace/business_date/captured_at、assessment/evidence访问；v3从完整 `ActivationDeploymentSet` 获取部署身份。旧构造入口可保留为明确v2路径，但任何v3路径都不生成占位generation/manifest/build或经v2字段绕行。内部类型和方法名可按现有风格落实，不能依赖调用者手工选取域或拼stream。
2. 固定v3 wire：沿用v2的公共assessment/evidence、namespace、business_date、captured_at和recovery_event_id字段；删除顶层单Unit的 `activation_generation`、`manifest_sha256`、`build_commit`，增加 `deployment_set`（完整集合v1的JSON object）及 `deployment_set_sha256`（重算domain+NUL+集合canonical bytes）。snapshot schema_version=3；material同字段但不含recovery_event_id，用独立v3 domain。不得把JSON字符串作为集合对象，也不得把集合hash伪作manifest。
3. 构造/解码验证集合与assessment的namespace/catalog/精确enabled producer集以及scope所属Unit闭合；Core覆盖启用Unit并集recovery_units，Producer/Occurrence精确所属Unit。现有evaluator只按enabled推导affected Unit，v3须在同一封闭评估路径从集合加入恢复责任：CoreUnready时affected Unit含恢复Unit，但不把这些Unit的producer加入enabled或错误开启新工作；Ready时仍保留空affected语义，完整集合另保留恢复责任。v2 evaluator与golden语义不变。共享依赖声明与对应要求/已有观察的contract/version/hash不得矛盾；缺失观察保持真实不就绪，不能补造Available/NotRequired。集合持久解码验证全部52条、重复/未知/遗漏、枚举/整数/摘要/NULL/排序、启用与恢复责任合法性，重编码逐字节一致。不把持久自洽解码当真实activation历史、source或owner认证，后续认证必须独立重读并精确比较。
4. `decode_readiness_snapshot`按准确domain和schema dispatch，v2走原语义、v3走集合语义，完整重评derived字段；未知版本、错domain/schema、v2混入集合字段、v3夹带旧scalar、非canonical/重复键/摘要或集合join错误均拒绝。抽取公共解析可复用，不复制整套算法。
5. `CandidateReadinessRecord`、`decode_readiness_record`按实际snapshot上下文选择material版本，沿用原RecoveryIdentity/v1和RecoveryEvent/v1编码/字段，重新计算event identity及前后join。不同snapshot版本/部署集合/scope/启用集/日期等不能作为continuation；同stream由不就绪转就绪仍需要原完整claims，拒绝错误时间/缺claim，不把版本升级当恢复。
6. `ReadinessStreamId::for_snapshot`闭集分派：v2原stream/v1不变；v3 stream/v2字段恰为namespace、business_date、catalog_sha256、scope、enabled_producers、deployment_set_sha256。同集合不同captured_at属于同stream；任一Unit/来源/配置/恢复责任变化进入不同stream。`ReadinessRecordStore`继续使用一个现有writer和chain reader，实际append/load_head/load_record/幂等重放/CAS/确认丢失均覆盖v3；链内版本/stream错配在读写两边拒绝。旧记录不升级、不修改、不自动延续到新stream。
7. 只读集合身份访问返回集合hash和逐Unit代，不提供伪全局generation；这是候选视图而非新probe命令或Ready许可。完整认证采集、跨库提交前后重查、query/health/CLI/T7和安全恢复跨stream衔接仍待完整T6，不将此片标整体完成，也不删除后续要求。

### 行为验收

- 实际临时activation库两个Unit不同generation/build构造全52条集合，进入v3快照、recovery record、现有store追加及重开后加载；包含未启用但负恢复责任的Disabled Unit。
- v3 snapshot/material/stream独立固定golden字节或摘要（期待值不能由被测encoder产出）；旧v2全部既有golden原文不改。v2和v3在同一临时readiness库共存，互不覆盖head或串接恢复历史。
- 集合字段/成员/启用集/恢复责任/日历/共享依赖变体、错hash、遗漏/重复/额外Unit、旧scalar夹带、未知域/schema、重复JSON键均有拒绝或身份变化证据。
- 同集合时间前进属于同stream；版本/任一Unit generation变更的predecessor拒绝；显式新stream genesis不携带旧恢复claim、不覆写旧Pending；不就绪→就绪缺claim拒绝、有效claim沿同stream成功。
- 实际store覆盖错误namespace、外来expected head、重复append、确认丢失重查和持久链伪造cross-stream拒绝；不只测试内存等式或未使用的mock。
- 错误/Debug脱敏；实际被改消费者编译覆盖。没有真实认证入口时准确保留未认证语义，不造正面认证测试替代生产前提。

不要求为每个新增字段单独跑RED；现有v2 golden为兼容基线，新v3行为与失败矩阵整批交主控验证。实现及自审完成后一次冻结owned文件，报告需要登记的新module和确切测试filters，由主控执行：

```bash
env CARGO_PROFILE_TEST_INCREMENTAL=true cargo test --lib -- --test-threads=1 readiness_
cargo clippy --lib --no-deps --message-format=json
```

主控先确认该filter只命中本任务及可复用隔离邻域、实际测试数大于零；必要时以明确module多filter代替，不运行真实provider或全monitor。修改后仅跑受影响合批及一次最终Clippy；定向rustfmt/diff检查。报告放本计划SDD目录 `task-1-report.md`，含实现、每项验证对应、未完成认证/查询范围及风险，不自称尚未执行的测试通过。

## 后续完整交付与回退

本片完成后继续T6认证集合采集/跨库重查/同snapshot查询、T2真实平台根、T4/T5执行与切换、T7，以及W17–W21/52Unit，不重开已完成T6A/P02/T4D。未修改schema，若需代码回退以精确提交为单位；已产生v3数据的环境不得用仅识别v2的旧binary冒充兼容，需后续N/N−1门禁决定，而不是改写历史bytes。
