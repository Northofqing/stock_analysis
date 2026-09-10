# W15 当前 v3 持久记录查询：实施记录

日期：2026-09-10。状态：本内部查询任务完成；初版及孤立分支修复均已提交、定向验证通过，独立初审质量 Approved，修复限定复审确认问题关闭、没有新 Critical/Important。BASE=`e22c881bb2705e246c4e4ca3f483572dfb225d91`，初版 SOURCE=`c7371102a870862caf47084697dcbdf158adf724`（六个 Rust 文件、661+/2-），修复 SOURCE=`14def95c80c0f30b0b64ff8e90c576be4dbcc91f`（两个 Rust 文件、126+/4-）。没有生产接线、部署或 monitor 操作。

合同见[当前记录查询计划](../superpowers/plans/2026-09-10-readiness-current-record-query.md)。这项内部入口为后续同快照认证消费提供真实持久记录，不是公开健康检查、CLI，也不是 W15/W16 完成。

## 行为和证据

| 行为 | 源码与测试 |
| --- | --- |
| 在同一只读事务验证目标完整链和当前 head 链，拒绝合法但已过时的记录 | [load_current](../../src/push_foundation/readiness_store.rs#L172)；[历史拒绝且旧 load_record 保持兼容](../../src/push_foundation/readiness_query_tests.rs#L253) |
| 区分当前、合法历史、缺失和孤立记录；孤立记录保留旧 `RecordMissing`，不归为历史 `HeadConflict` | [真实 canonical SQLite 四分类反例](../../src/push_foundation/readiness_store_tests.rs#L85)；比较拒绝前后目录字节，不修复孤立分支 |
| 固定顺序：当前记录 → v3 → 记录内完整部署集合 → activation 全集合重读 → 再查当前记录并精确比较 | [查询实现](../../src/push_foundation/readiness_query.rs#L85)；[同一持久记录及52 Unit断言](../../src/push_foundation/readiness_query_tests.rs#L130) |
| 非选中 Unit 改代、启用/恢复配置改变、另一数据库集合不一致均拒绝 | [非选中 Unit](../../src/push_foundation/readiness_query_tests.rs#L156)、[配置](../../src/push_foundation/readiness_query_tests.rs#L182)、[跨库](../../src/push_foundation/readiness_query_tests.rs#L210) |
| activation 重读期间真实提交后继记录，查询必须拒绝旧 head | [真实 SQLite checkpoint 反例](../../src/push_foundation/readiness_query_tests.rs#L350)；不是 sleep 或 mock 返回值 |
| 新入口明确拒绝 v2，旧 v2 存储仍可读 | [v2 兼容反例](../../src/push_foundation/readiness_query_tests.rs#L304) |
| 缺文件、错误 namespace、损坏的实际 snapshot 字节均拒绝；错误/Debug 不回显路径或 protected URI | [真实损坏与脱敏用例](../../src/push_foundation/readiness_query_tests.rs#L413)、[typed error](../../src/push_foundation/readiness_query.rs#L58) |

普通成功/拒绝用例对实际 readiness 与 activation 临时目录逐文件比较 bytes，确认无新文件或 sidecar/库字节改变。竞态用例允许测试 checkpoint 提交合法后继，随后验证旧历史保留、新 head 正确及 activation 目录不变；不能将这个测试整体说成零写入。

只复用现有 SQLite 读取/完整链、v3 assessment 与 activation fixture；两个既有 helper 调整为 `pub(super)`。旧 `load_record/load_head/append`、schema/codec/canonical、冻结输入与 Cargo 均未修改；新增 module 没有公开 re-export。

## 验证记录

主线已完整读取初版及修复报告，核对准确提交范围。下表区分修复后的最终验证与未改夹具的初版验证；不将旧结果冒称修复后的重跑结果。完整原始材料保留在本地 `.superpowers/sdd/2026-09-10-readiness-current-record-query/`，公开结论及关键证据记录在本文，不要求读者取得被 Git 忽略的工作区。

| 命令 | 结果 |
| --- | --- |
| `cargo test --lib push_foundation::readiness_query_tests -- --test-threads=1` | 修复后 8 passed / 0 failed；session 41252、exit 0，运行30.05s |
| `cargo test --lib push_foundation::readiness_store_tests -- --test-threads=1` | 修复后 12 passed / 0 failed；exit 0，运行12.40s |
| `cargo test --lib push_foundation::readiness_deployment_set_tests::readiness_v3_real_set_flows_through_recovery_and_the_existing_store -- --exact --test-threads=1` | 初版 1 passed / 0 failed、exit 0；修复未改该夹具，未重复运行 |
| 改动文件的 `rustfmt --edition 2021 --config skip_children=true --check` | 初版六文件、修复两个文件分别 exit 0 |
| `cargo clippy --lib --message-format=json` | 修复后 exit 0、build-finished success；完整捕获780行 JSON、159条 warning，目标文件诊断为空；没有逐条 BASE 对照，不称全仓零告警或全部既有 |
| 主线分别暂存初版六文件和修复两文件后的 `git diff --cached --check` | 均 exit 0；未将主线文档修改混入代码提交 |

修复轮只执行一次 Clippy，耗时384.85秒，完整 stdout SHA-256 为 `aec1bed6d283981ed992ac9584537b53fec225ca31cd81049d6bb3d8c9c1b085`，stderr 为 `d44134fc2bde17d009fc56924b1f75cb98974c04d9c744cda56dd5a49e9265e0`；主线重新解析全流、核对计数、成功事件和两份文件 hash，未重跑编译。`--lib` 的诊断结果不表示已额外执行 test-target Clippy。

初版 TDD 证据边界：先写测试后分别得到缺方法 `E0599` 和缺 module `E0583` 的编译 RED；没有取得独立运行期断言 RED，不能表述为旧实现的运行期回归已复现。没有通过临时编造错误产品实现补造失败。一次多余测试过滤词实际只跑1项，未作为最终整批证据；格式命令曾有路径拼写错误，修正后正确命令通过，错误不计为行为 RED。报告保留全部过程。

上述限制描述初版。修复轮新增真实孤立分支反例，旧代码实际运行失败：`left: Err(HeadConflict)`、`right: Err(RecordMissing)`，exit101、1 failed；最小修复后同一精确命令 1 passed、exit0。这关闭了本次发现的回归，不追溯补齐初版运行期 RED。修复中另一次误拼过滤词实际运行0项，不计验证，随后正确完整 query 命令通过。初版 Clippy 虽 exit0，但完整流未保存；本轮完整流同样不补回那份历史证据。

未重跑全仓、全部 Foundation、Ruby/Chrome 或任何生产程序。测试数只对应本记录的覆盖范围，不能据此换算52 Unit迁移完成率。

## 独立审查和后续

独立初审使用正式 `review-e22c881..c737110.diff`，范围为完整六文件提交，同时核验 Spec/Quality：功能/范围符合、Task Quality Approved，C0/I0/M3。Minor 分别为初版运行期 RED 限制、activation 缺失/损坏尚未经新入口直接测试、Clippy 非目标告警/证据范围；三项及处理边界在此保留，交后续整体验收复核。这不是整项目审查或上线许可。

主线随后针对旧错误语义补核，确认：若 snapshot/event 可完整解码，但目标不在已提交 head 的祖先链内，旧 `load_record` 返回 `RecordMissing`；初版 `load_current` 却将它和合法历史祖先一并归为 `HeadConflict`。两者都拒绝读取，但没有保留计划要求的孤立记录分类。原实现者已补真实 SQLite 反例并最小修复；合法历史仍返回 `HeadConflict`，孤立记录改为 `RecordMissing`，同事务完整链校验及旧接口不变。独立限定复审使用 `c737110..14def95` 两文件修复范围，确认该项 ADDRESSED，反例有效，没有新 Critical/Important/Minor；未重复初版审查或测试。初版三项证据/覆盖限制继续保留，不因本轮通过而自动清除。

仍必须由后续真实认证器对返回的**同一个 record**核对权威数据库身份、启用/恢复配置、business date/可信时间、source/acquisition/recovery 真实性、owner/build；health/readiness/CLI 共同消费认证结果。当前入口返回的只是 candidate，不签发 ReadyGate、发送或恢复权限；两次只读检查不提供跨库原子性，也不保证返回后仍当前，执行临界区仍需复验。

完整 W15–W21、旧 v5–v9 审计兼容、52 Unit 适配/灰度/回滚和真实 CI/生产验收继续保留。当前审计/蓝图固定的 `aef7972` 是此前源码快照，不能称覆盖本提交；后续连贯批次将正式刷新，不放宽 freshness、不改冻结历史输入。
