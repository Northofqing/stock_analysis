# 旧版本升级后继续恢复：审计逻辑尾接续

日期：2026-09-08。状态：准备实施；前置恢复Uncertain读取任务已完成至61e9d16（55项/静态/限定复审通过）。启动时将实际BASE记录到本计划SDD；不使用HEAD~1推断范围。

## 目标与依据

验证并保障“已有真实未决attempt→正式数据库升级→恢复追加审计→实际终态读取”完整链路。新audit必须连接同decision的唯一逻辑尾，而不是物理最大rowid；损坏链不能自动修复或被当作正常接续。

当前已核实：coordinator.rs::enqueue_audit按rowid DESC选前驱；schema.rs::migrate_schema_v4_to_v5按predecessor/audit identity重建行序，未保留rowid。这是待实际验证的兼容性风险，不是已复现的生产故障。来源、实际历史表夹具的副作用与安全路径见`.planning/2026-09-06-push-foundation-runtime/audit-append-order-source-gap-2026-09-08.md`。前批reader已验证“已封口链重排行序仍可读”，没有验证升级后新追加。

## Global Constraints

- 只在隔离树.worktrees/push-reliability-20260905和现有codex分支工作；不操作根工作树、生产monitor、真实业务DB、provider/LLM/sink/订单/PAM、.env、owner、批准、部署、网络。
- 不修改冻结Foundation SQL、durable schema、八份RFC输入、catalog/WBS/蓝图、canonical/hash域或外部接口；不修生产数据或重写既有链。若真实opener暴露必须改schema的兼容缺口，先保留实际失败并报告父线，不能在夹具中偷偷修FK，也不能降为“改了rowid就算真实升级”。
- 唯一实施agent只编辑下列两份Rust文件和本Task报告；父线独占Cargo/Git/中文docs，代理不运行Cargo/Git、不派子代理。保留历史测试、逐行安全边界和独立断言，只定向fmt。
- 测试仅使用自有TEST_CODE临时数据库与既有受限MemoryAppendPort。Fixture只stat隔离树生产命名对象以保护不变性，不读其数据库；清理仅沿原inode限定机制。不新建无界线程或等待，不用全局数据库初始化/固定test.db。

## Task 1: 真实升级后追加连接唯一逻辑尾

### 文件

- 修改：src/durable_delivery/tests.rs、src/durable_delivery/coordinator.rs。
- 只读：src/durable_delivery/schema.rs及必要model.rs。
- 父线：本计划、SDD证据、中文结果与docs/push-system/README.md。

### 第一条实际行为反例

1. 复用w12_foundation_envelope/prepare_reserved与真实begin_attempt建立Foundation-bound AttemptInFlight；保持原attempt未产生sink结果。可用真实heartbeat_attempt构造固定来源的更长链，不手插伪audit/receipt或修改rowid。
2. 从实际链独立取得唯一逻辑尾；确认旧迁移排序后的物理尾与逻辑尾确实不同，再实施升级。确定性样本要有实际依据：固定owner/时间/业务输入，或记录明确有界的真实API样本构造；不能假定任意多行链必然重排错尾，不循环猜测到无限成功。
3. 快照保留目标decision的真实envelope/attempt/reservation及audit原字节、hash、predecessor。现有downgrade_replay_schema_v4_for_test(false)是历史表形状夹具，会增加另一个合成Delivered样本且删replay表；明确证明目标数据保留、目标无replay引用，不把合成样本当Foundation来源。
4. 释放旧coordinator全部Arc，用DurableDeliveryCoordinator::open正式重开并实际执行v4→当前v9。不能以initialize_test_schema替代关键升级；升级前后验证目标快照一致、实际FK完整、version正确，不能只比规范化DDL manifest。
5. 升级后通过真实reconcile_foundation_decision（精确完整binding）恢复目标，隔离helper合成decision；以实际inspect_foundation_terminal要求相同attempt的Uncertain及零sink/provider调用或result。独立核对新FenceRevoked.predecessor必须等于升级前逻辑尾。预期失败必须来自真实接续/读取行为，编译失败不作RED。
6. 首条建议测试名durable_delivery::tests::audit_logical_tail_recovers_after_real_v4_upgrade。测试-only冻结给父线运行，取得实际RED前不改writer。

### 最小修复与完整验证

- 若实证指向enqueue_audit，沿原事务接点按持久identity/predecessor选择同decision唯一逻辑尾，空链只用于合法初始创建；不能用timestamp/rowid/max字符串排序代替逻辑尾。不增加新的持久序号或写入协议。
- writer合法链可能有Pending审计尾，必须保留按前驱逐项封口的既有追加流程；不要把reader的全部前驱已Appended条件搬进writer，不能凭尚未封口误判结构损坏。
- 对已存在的分叉、环、断开的链或跨decision前驱歧义应拒绝并回滚本次写入；不能挑任意候选、把非空坏链当新根或“补接修复”旧数据。复用已有原子事务，不增加发送或恢复权限。
- 同一条实际升级反例转GREEN；补合法未升级接续、跨decision隔离、无/多尾与断链拒绝、失败无状态/审计新增，以及重启重读/重复恢复不二次追加。使用真实入口及独立结果，不能只调用新私有helper自证。
- 保留Generic/P01恢复Uncertain、迟到结果、合法中间conflict、PendingSeal/原终态防篡改，以及原schema升级/FK回归。按本Task实际影响与测试副作用确认最终合批命令，不运行未经安全核对的整个durable/全仓suite。
- 单Cargo队列；保存实际session直到终态。相关测试合批后lib Clippy、定向fmt、diff与冻结输入校验；警告分既有/新增，不加allow掩盖。
- 固定启动BASE..SOURCE做一次Task规格/质量审查；修复仅限定fix范围复审。中文文档记录真实升级夹具与生产归档的区别，不冒称生产迁移已执行。

## 完整目标边界

这是W09/W12/W20所需恢复和升级兼容性的一条实际链路；不完成W15/W16来源认证与运行接管、W17真实影子业务、W18权限、完整W19、全部52Unit迁移、离线HTML/checker/实际CI或生产门禁。相关工作继续保留；不能用局部通过缩小全部完成目标。
