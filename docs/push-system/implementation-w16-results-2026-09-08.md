# W16 激活与所有权实施记录

日期：2026-09-08。完整 W16 未完成；此记录区分规范纠错、完整性读取、实际认证与生产接管，不以局部绿灯替代全目标。

## 已完成：Shadow 范围纠错

`e7bf3d5` 将 RFC 与机器校验器的 Shadow owner=None 限定到新 shadow actor，保留 Unit 实际 incumbent；初始 Disabled 与排空后 Disabled 的准入从不可变已执行历史和真实批准推导，不新增状态表，不复用旧 token。

依据及矩阵见 [合同裁决](activation-contract-decisions-2026-09-08.md)。原 Q13/Q17 和八份输入字节保留；真实身份、准入投影和执行许可仍需实现。

- 公共 CLI 回归先失败再通过：1 run / 11 assertions，全绿，接受正确 scope 后突变回旧 whole-Unit scope 并验证拒绝。
- `ruby scripts/architecture-docs/test/rfc_spec_test.rb`：session79925，exit0，296 runs / 3884 assertions，0 failures / errors / skips，165.568s。
- 实际输入哈希门禁 `check-rfc-inputs.rb --root .` 与 `check-rfc.rb --root . --draft` 均通过；不是 strict 发布通过。
- 独立限定 Spec/quality 审查 Approved，Critical/Important/Minor 均无。

## 已完成限定实现：全 Unit 激活事实读取

T1 实现通过自有安全只读事务读取完整 manifest/journal、校验内嵌 schema、全部原始字段、内容/稳定身份 SHA、连续代际链、合法边与完整关联。公开入口自行加载 bundled catalog，先核验全库再选择 Unit；未登记、journal 跟齐、一个未执行末代分别表达，缺行不默认为 Disabled。

内容 domain 为 ActivationManifestV1 / PromotionJournalV1，稳定身份沿用 PromotionV1；历史版本字段保留原值，不假装与当前部署已认证相等。RawActivationFacts 不是部署认证、配额许可、当前 owner fence 或 W15 Ready。

首轮 session44265：编译2m20s，12项为10 passed / 2 failed，3.22s，43项既有 warning。实际失败是 hot journal 在 schema 阶段拒绝的分类与测试预期不同，以及造损坏 fixture 修改 sqlite_master 被当前 SQLite 拒绝。该轮没有执行成功的 hot-journal 目录字节断言，不能声称已证明该反例零写入。

原实现代理改用真实同名表重建，按实际错误传播验证 hot journal 的拒绝和目录字节；另外补充完整六代已执行循环/同 Unit 合法回滚正例及中间 predecessor 损坏反例。生产只读/schema 内核没有为测试放宽。

源码提交 `1c16380`，原始任务 BASE `f48ddfc`，五个新文件及 module 声明；中途 `e7bf3d5` 是独立已审查的文档/Ruby 任务，不混入 Rust 验收范围。

- `cargo test --lib push_foundation::activation_facts_tests -- --test-threads=1`，session57866，exit0，**14 passed/0 failed/0 ignored**，3.54s，编译2m24s。完整六代、合法回滚、实际中段断链、表替换和 hot-journal 目录字节断言均执行通过。
- `cargo test --lib push_foundation:: -- --test-threads=1`，session99791，exit0，**196 passed/0 failed/1 helper ignored**，23.32s，缓存编译1.75s。ignored 是由父 POSIX 锁回归显式运行的 helper，不是跳过业务验收；共享 schema/锁/store/recovery 相邻回归通过。
- `cargo clippy --lib --message-format=json`，session45449，exit0，1m22s。完整当前 fingerprint 经 jq 核对163项有位置既有 warning，Foundation/采集审计目标零诊断。上述 lib-test 仍报告43项既有 warning，不称全仓零告警。
- 六文件定向 `rustfmt --edition 2021 --check --config skip_children=true` 与 `git diff --check` 均通过。

限定独立 Spec/quality 审查 Approved，无 Critical/Important。T1 按完整性读取范围收口，不据此标 W16 或生产认证完成。审查和主控裁决保留两项证据限制：

1. 这里实际测量的是数据库/sidecar 字节和目录变化，没有 provider/sink 计数器。入口只接受 path/Unit，实际调用为 bundled catalog→安全读事务→schema/FK/原始行/全链校验；没有对应外部执行端口。不新增与调用链无关的计数器来假证零调用，完整W16/T7和W17的真实端口计数仍待实现。
2. 同名表替换 fixture 同时删除了三个触发器，因此该测试证明组合 schema 损坏被拒绝，不能单独证明由列类型替换触发拒绝。下次触碰该 fixture 时恢复原触发器以隔离这个反例；记为非阻塞测试改进，不抹去此限制。原内嵌schema精确比较实现未变，相邻共享schema回归已通过。

外部部署真实性、实际 owner 权限、生产source配置与整个W16仍未验证，是明确的后续范围。monitor/config/Cargo/冻结SQL及蓝图两份输入相对原始BASE无差异；未以静态检查推断生产运行健康。

## 完整剩余范围

- T2：真实操作员/部署/source package/日历和批准验证；生产平台及根配置尚未给定。
- T3：同一 IMMEDIATE 事务内 generation CAS、全 Unit 日配额与 manifest/journal 写入。
- T4/T5：legacy/new 四类 actor 的共同当前 fence、真实监督器和旧 binary 撤权、准入历史投影、非原子切换/恢复/rollback。
- T6/T7：W15 全 Unit 部署集合及显式版本消费、同快照查询/启动、操作员入口和完整门禁。
- W15 真正来源上下文/认证/恢复及调度联结，W17–W21，52个 Unit 的纵向迁移与真实发布证据仍未完成。

开发和测试均隔离于 worktree/临时库；本轮没有启动、观察、替换生产 monitor，没有调用真实 provider/sink/PAM，也未进行 owner 晋级或真实库修改。
