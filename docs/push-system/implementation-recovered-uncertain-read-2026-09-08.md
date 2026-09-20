# 恢复分类“不确定投递”的读取修复：实施记录

日期：2026-09-08。状态：**本计划完成，最终55项回归及限定复审通过**。计划`03a2e93`，原始BASE`3cca5cc`，初版源码`a2429b3`，修复`fb55d32`/`61e9d16`。此为恢复读取与实际消费者的局部交付，不代表完整W19或上线完成。

## 为什么需要修复

同样是“不确定投递”，项目目前存在两类真实来源：提供方明确返回不确定结果；发送尝试超出租约、恢复器无法确认结果而进入隔离。后者没有提供方结果记录，却已有真实撤权事件、恢复分类和终态审计。统一读取器仍要求提供方结果，可能把这类已封口隔离状态读成来源错误，导致已有恢复与健康指标不能正确识别它。

源码依据（本批BASE）：

- `src/durable_delivery/coordinator.rs:5535–5589`：过期恢复保存FenceRevoked、RecoveryClassified和Uncertain disposition，其来源hash指向真实fence evidence。
- `coordinator.rs:6443,6545`：Uncertain统一进入只接受authoritative sink result的校验，缺结果行即拒绝。
- `coordinator.rs:2970,3048,6300`：Generic/P01两路实际终态reader共用上述校验接点。
- `src/push_foundation/generic_transport.rs:510–584`、`dedicated_transport.rs:522–584`：下游按已验证的处置及原始字节/hash消费；Uncertain不要求伪造提供方payload。
- `reconciler.rs:617`：真实Uncertain应进入ResolutionRequired，不自动重发；SLA/metrics应保留未决分类，不能当Accepted样本或业务完成。

## 已实现的数据流

在原读取接口内分别严格校验提供方结果或原始恢复事件，绑定decision、attempt、租约、撤销后的fence、event/audit/disposition的原字节和hash。缺失、重复、错误归属或冲突仍拒绝，不能采用“没有sink记录就算成功”的回退。合法迟到的非权威结果仍不能把恢复Uncertain升级为Accepted。

验证贯穿真实恢复、两个终态reader及已有SLA/库存指标消费，并检查重复读取/重启无写入或二次发送。第一条Generic真实恢复回归先得到0passed/1failed：恢复状态已封口且没有sink调用或结果行，读取器却要求一条authoritative Uncertain evidence join。最小修复后，同一测试得到1passed/0failed；原始输出见同计划SDD的red-1.txt和green-1.txt，后续实际消费者已纳入第一轮54项合批。

| 处理点 | 当前实现及证据 |
| --- | --- |
| 明确区分两类来源 | coordinator.rs::validate_uncertain_delivery_evidence：没有恢复事件时仍走原sink严格校验；有恢复事件且有任意authoritative sink时拒绝，不能catch后回退 |
| 核对真实恢复身份 | validate_recovered_uncertain_delivery_evidence：current attempt、原fence、checked_add后的撤销代次、lease/revocation时间、恰好一组恢复event/audit均需一致 |
| 核对原始事件 | validate_recovery_attempt_event：canonical/hash、稳定event/audit identity、decision/attempt/kind、Appended与非空ref、持久时间交叉校验 |
| 核对前驱和真实根 | validate_recovery_fence_predecessor_chain、load_sealed_audit_chain_node、validate_prepare_genesis_audit：先按decision一次聚合拒绝同前驱多后继，再沿持久身份到真实prepare根，必须含当前LeaseGranted；不用rowid推断历史顺序，不逐节点重复全表扫描 |
| 保留原始证据 | validate_current_disposition_canonical重验disposition，结果直接返回持久FenceRevoked bytes/hash，不合成sink payload |
| 不提升投递状态 | 两路实际Generic/P01 inspector及原SLA/metrics消费保持Uncertain；接受/完成样本为空，合法迟到非权威回执不触发重发或完成 |

上述符号对应a2429b3、fb55d32及最终61e9d16。业务策略、原始证据字节及恢复写入协议均未改变。

## 正反例与审查修复

- 实际过期恢复、P01读取、迟到非权威Accepted、重启重复只读、PendingSeal、事件缺失/重复/错绑定/字节破坏、权威冲突和fence溢出：见tests.rs中w19_recovered_uncertain_测试。
- 真实中间DecisionIdentityConflict必须合法保留；原RejectedAuditPending根经显式授权重试后恢复也合法；真实链只改变物理rowid后读取不变。后者不等于执行过旧版本升级，也不证明升级后新追加正确。
- 自环前驱曾经被错误接受；精确反例先失败（fix-red-1.txt），fb55d32补真实逻辑链和两种prepare根，最终54项通过。恢复Case时序也已纠正为业务创建早于prepare/attempt。
- 第一轮限定复审发现：FenceRevoked跳过真实中间conflict后，与conflict成为同一前驱的两个后继，遍历仍能到合法根而错误接受。第二个精确反例先实际失败：0passed/1failed，编译1m05s、运行0.34s；fix-red-2.txt记录真实inspector错误接受。61e9d16增加16行同decision多后继歧义校验，同一反例在最终55项合批中通过，限定复审I1已关闭。
- 两个实际消费者回归分别位于finalization_sla_tests.rs::recovered_uncertain_generic_and_p01_routes_are_not_accepted_samples及finalization_metrics_tests.rs::recovered_uncertain_generic_and_p01_inventory_counts_real_disposition_without_acceptance。不以人工构造成功report替代持久来源读取。

## 边界与完整剩余项

最终定向回归为**55 passed / 0 failed / 0 ignored / 3271 filtered**，编译55.65秒、测试42.17秒；完整输出lib-fixed-round2-final-validation.txt。它包括12个新增durable回归、2个实际SLA/metrics消费者回归与41个原邻域，旧断言没有削弱。

lib Clippy成功，有159条既有警告，无新增或全Task改动行诊断；首次输出因工具额度截断后，以相同源码的缓存运行补采完整诊断，未重复55项测试。过滤记录clippy-fixed-round2-final.jsonl与摘要clippy-fixed-round2-summary.json保留诊断/位置及build终态，未声称保留全部构建artifact。四Rust文件定向格式检查、diff、八份冻结输入检查通过。

初审及两次限定复审均有固定提交范围；最后I1已关闭、M1恢复时序已关闭，无新增Critical/Important/Minor；M2既有警告留全分支最终triage。完整证据保留于`.superpowers/sdd/2026-09-08-recovered-uncertain-terminal-read/`，最终结论见fix-review-2.md。代码提交未改变已验证字节，不因文档提交再跑相同测试。

本批只读修复不改变恢复写入协议、schema、canonical/hash域或发送策略，不新增自动重试、人工处置或生产权限。测试仅使用隔离树Test命名空间数据库；不启动、监控或替换生产monitor。

Q39要求的是告警/解决时限；本批不提供缺失的告警事件、可信severity和下一有效时段证据，也不意味着完整W19完成。[实施计划](../superpowers/plans/2026-09-08-recovered-uncertain-terminal-read.md)保留完整验收条件；W15/W16/W17生产接线、其它Unit迁移、文档工具及真实上线门禁继续推进。
