# 旧版本升级后继续恢复：审计逻辑尾接续

日期：2026-09-08；更新：2026-09-09。状态：本批有限迁移修复完成，最终67项测试、静态检查及独立规格/质量审查通过。实际BASE为1868b3ff6884c54f995a87153b36e27a3db027db，SOURCE为77cc3bc568f8502ea9a06a7e03439b1a2eb65b3d；不使用HEAD~1推断范围。前置恢复Uncertain读取已完成至61e9d16；已被旧迁移重排库的后续兼容规则仍未完成。结果见[实施记录](../../push-system/implementation-durable-upgrade-2026-09-08.md)。

## 目标与依据

验证并保障“已有真实未决attempt→正式数据库升级→恢复追加审计→实际终态读取”完整链路。对于本例升级前完整的单decision链，新audit必须仍连接升级前唯一逻辑尾，不能因迁移重排行序选错前驱。既有合法跨decision审计依赖仍需保留；损坏链不能自动修复或被当作正常接续。

启动时已核实：coordinator.rs::enqueue_audit按rowid DESC选前驱；旧schema.rs::migrate_schema_v4_to_v5按predecessor/audit identity重建行序，未保留rowid。55345随后实际复现隔离夹具的错误接续；77cc3bc已在两次复制中保留rowid。不是已复现的生产故障。来源、实际历史表夹具的副作用与安全路径见`.planning/2026-09-06-push-foundation-runtime/audit-append-order-source-gap-2026-09-08.md`。前批reader只验证已封口链重排行序仍可读，本批补正式升级后的新追加路径。

## Global Constraints

- 只在隔离树.worktrees/push-reliability-20260905和现有codex分支工作；不操作根工作树、生产monitor、真实业务DB、provider/LLM/sink/订单/PAM、.env、owner、批准、部署、网络。
- 不修改冻结Foundation SQL、durable schema 的DDL/版本、八份RFC输入、catalog/WBS/蓝图、canonical/hash域或外部接口；不修生产数据或重写既有链。真实opener已暴露提交兼容缺口，按下述19131裁决仅扩展迁移实现的非DDL逻辑；不能在夹具中偷偷修FK，也不能降为“改了rowid就算真实升级”。
- 唯一实施agent只编辑下列授权Rust路径和本Task报告；父线独占Cargo/Git/中文docs，代理不运行Cargo/Git、不派子代理。保留历史测试、逐行安全边界和独立断言，只定向fmt。
- 测试仅使用自有TEST_CODE临时数据库与既有受限MemoryAppendPort。Fixture只stat隔离树生产命名对象以保护不变性，不读其数据库；清理仅沿原inode限定机制。不新建无界线程或等待，不用全局数据库初始化/固定test.db。

## Task 1: 真实升级后追加连接唯一逻辑尾

### 文件

- 修改：src/durable_delivery/tests.rs；src/durable_delivery/coordinator.rs 仅限已取证且已删除的临时诊断，writer 本批保持不变；按19131/55345裁决，src/durable_delivery/schema.rs 仅限 migrate_schema_v4_to_v5 的两次复制保留 rowid、尾部全库FK错误明细和验证后延迟计数重置。
- 其余 schema.rs 及必要 model.rs 只读。
- 父线：本计划、SDD证据、中文结果与docs/push-system/README.md。

### 第一条实际行为反例

1. 复用w12_foundation_envelope/prepare_reserved与真实begin_attempt建立Foundation-bound AttemptInFlight；保持原attempt未产生sink结果。可用真实heartbeat_attempt构造固定来源的更长链，不手插伪audit/receipt或修改rowid。
2. 从实际链独立取得唯一逻辑尾；确认旧迁移排序后的物理尾与逻辑尾确实不同，再实施升级。确定性样本要有实际依据：固定owner/时间/业务输入，或记录明确有界的真实API样本构造；不能假定任意多行链必然重排错尾，不循环猜测到无限成功。
3. 快照保留目标decision的真实envelope/attempt/reservation及audit原字节、hash、predecessor。现有downgrade_replay_schema_v4_for_test(false)是历史表形状夹具，会增加另一个合成Delivered样本且删replay表；明确证明目标数据保留、目标无replay引用，不把合成样本当Foundation来源。
4. 释放旧coordinator全部Arc，用DurableDeliveryCoordinator::open正式重开并实际执行v4→当前v9。不能以initialize_test_schema替代关键升级；升级前后验证目标快照一致、实际FK完整、version正确，不能只比规范化DDL manifest。
5. 升级后通过真实reconcile_foundation_decision（精确完整binding）恢复目标，隔离helper合成decision；以实际inspect_foundation_terminal要求相同attempt的Uncertain及零sink/provider调用或result。独立核对新FenceRevoked.predecessor必须等于升级前逻辑尾。预期失败必须来自真实接续/读取行为，编译失败不作RED。
6. 首条建议测试名durable_delivery::tests::audit_logical_tail_recovers_after_real_v4_upgrade。测试-only冻结给父线运行，取得实际RED前不改writer。

### 最小修复与完整验证

- 2026-09-08 错尾实证及兼容裁决：55345 已在正式 open 提交成功、原数据/FK/version 校验通过之后，实际证明 FenceRevoked 接错前驱。为保留既有合法跨decision独立分量的追加顺序，本批在 v4→v5 的两次复制中显式传递原 rowid 数值（不只排序），不改DDL/原始审计字节及 writer 语义。测试保留“旧排序会产生不同尾”的独立见证；修复后要求升级前后的全部 audit identity→rowid 映射相等和最终前驱正确，而不再要求修复后的迁移继续实现旧重排。该内部实现断言调整与55345实际行为RED一并供独立审查，不能删实际接续/终态断言。
- 此裁决不是“已升级库都安全”：对已经被旧迁移重排的 v5–v9 数据，完整单链可从逻辑链接推导，多个独立合法分量则缺少可靠的原追加顺序，不能用当前版本、timestamp、任意rowid或hash排序当权威。该兼容处理仍属于完整目标的未完成项，需后续明确可靠顺序来源和不削弱跨decision合同的接线；本批不得以此宣布全部审计兼容或全项目完成。

- 2026-09-08 正式提交诊断裁决：19131 实际证明全部 schema SQL 成功、hook=1、version9、foreign_keys=1、defer_foreign_keys=1、FK违规=[]，失败位于 COMMIT。允许在 v4→v5 已有精确 self-FK 和全库 FK=0 验证之后，同一事务紧邻执行 defer_foreign_keys OFF→ON，错误传播；保持 foreign_keys=ON、原 COMMIT/rollback 逻辑及后续延迟约束语义。全库验证不通过绝不重置。不改变DDL/排序/writer，不推广到通用提交逻辑；纠正该分支 FK 明细为 pragma 的实际 table/rowid/parent/fkid 列，rowid可空。该方案需真实验证，不能先报升级修好或目标错尾已复现。
- 本次扩展必须覆盖正式 open、缺 predecessor / 非 outbox 坏 FK 的拒绝与完整回滚、重置之后新产生的延迟 FK 仍在 COMMIT 拒绝。原始 bytes/hash/ref、旧库 version与DDL回滚均独立核验；删除取得证据后的临时回滚日志。已有 reader/跨decision 合同不变。

- 2026-09-08 前置诊断裁决：91580 的实际 FK 明细证明历史 fixture 的 self-FK 留在临时名，并非生产迁移错误。允许仅在测试 helper 创建历史表时直接引用最终表名，并独立验证 FK 声明及全库 FK=0；不允许事后修改已有数据或削弱断言。该修正不代表目标错尾反例已复现，仍需正式 open 后的实际行为 RED。

- 实际RED后再裁决最小修复位置。不能预先假定所有合法数据库均为单decision独立链：既有w16_scoped_recovery_blocks_on_another_decisions_pending_audit_predecessor要求跨decision前驱先由global恢复封口，随后目标仍完成并可重复读取。修复必须同时保留该原断言、升级前的真实追加顺序和本例唯一逻辑尾；不得为通过新反例改变既有合法依赖合同。
- writer合法链可能有Pending审计尾，必须保留按前驱逐项封口的既有追加流程；不要把reader的全部前驱已Appended条件搬进writer，不能凭尚未封口误判结构损坏。
- 若修改选尾算法，须区分真正的分叉/环/损坏引用与已支持的跨decision等待依赖；不能仅凭“外来前驱”或单decision局部断开就拒绝后者。坏证据不得挑任意候选或新建根来“修复”；必要失败保持原事务回滚，不增加发送或恢复权限。
- 同一条实际升级反例转GREEN；本批补原 rowid/全部原字节不变、坏FK拒绝与失败无状态/审计新增，以及重启重读/重复恢复不二次追加。保留合法未升级接续和跨decision隔离正例。逻辑选尾的无/多尾/环/断链规则不得默默删除，随“已升级库兼容处理”保留为未完成验收；不以本批未改writer冒称那些规则已实现。使用真实入口及独立结果，不能只调用新私有helper自证。
- 保留Generic/P01恢复Uncertain、迟到结果、合法中间conflict、PendingSeal/原终态防篡改，以及原schema升级/FK回归。按本Task实际影响与测试副作用确认最终合批命令，不运行未经安全核对的整个durable/全仓suite。
- 单Cargo队列；保存实际session直到终态。相关测试合批后lib Clippy、定向fmt、diff与冻结输入校验；警告分既有/新增，不加allow掩盖。
- 固定启动BASE..SOURCE做一次Task规格/质量审查；修复仅限定fix范围复审。中文文档记录真实升级夹具与生产归档的区别，不冒称生产迁移已执行。

## 完整目标边界

这是W09/W12/W20所需恢复和升级兼容性的一条实际链路；不完成W15/W16来源认证与运行接管、W17真实影子业务、W18权限、完整W19、全部52Unit迁移、离线HTML/checker/实际CI或生产门禁。相关工作继续保留；不能用局部通过缩小全部完成目标。
