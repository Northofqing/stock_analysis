# W16 激活合同裁决

日期：2026-09-08。状态：下述读取编码、Shadow 范围澄清及批准/事务顺序为已确定的工程合同；真实授权、owner adapter 和线上接管尚未实现或批准。本文是新增裁决，不改写原始问答、冻结 SQL 或八份输入快照。

## 背景和依据

- [原始 Grill 决策](grill-decisions-2026-09-02.md)：Q13 新路径先 shadow、同 occurrence 恰有一个 owner；Q17 Foundation 不改变 owner；Q18 禁用新生产但保留恢复；Q24/Q31 多 Unit 可 disabled/shadow、一次只晋级一个 owner；Q52 单一 activation 状态机；Q80 新代回滚恢复目标 owner；Q98 期望与执行分开，Q99 人批准执行。
- [冻结蓝图输入](../Project_Architecture_Blueprint.md) §24.18：Foundation 默认不改变 physical owner；shadow 无发送/业务最终化；排空保留恢复。它是导入语义快照，不是本轮部署证明。
- [实施 RFC](push-system-implementation-rfc.md) 的 common_fence 绑定 Unit、当前代、manifest、owner；初版 `Shadow / PhysicalOwner / None` 错把新 shadow actor 的限制扩为整个 Unit。

## 决定一：完整性读取的编码

采用已有 canonical-v1 的 `domain + NUL + 按键排序、无空白 JSON object`。稳定事件身份沿用 `PromotionV1`，恰含 generation、unit_id；manifest 内容使用 `ActivationManifestV1`，含除 manifest_sha256 外全部19列；journal 内容使用 `PromotionJournalV1`，含除 canonical_sha256 外全部14列。列名、null、非负整数与原始文本语义保持冻结 SQL，不从调用者补字段，不改变原 W15 v2。

历史版本 SHA 的实际内容及关联可以核验，但 raw inspector 不证明部署版本/认证。保留完整历史，并显式区分未登记、journal 跟齐、一个未执行末代；缺行不是 Disabled，多个未执行代不允许跳过。完整校验全库后才选择 Unit。

收益：后续认证和写入复用完整真实事实；不再等待生产配置才能验证读取。代价：编码改变必须新增 domain，raw 结果仍不能授予发送或 Ready。验收为独立 golden bytes 和逐字段/断链/跨 Unit/只读副作用反例，不能只测 SHA 格式。

## 决定二：Shadow 无 owner 的对象是 shadow actor

Unit 的实际负责身份仍在当前 manifest.physical_owner；shadow actor 没有独立 physical ownership。执行权限从当前已执行 manifest/journal、实际部署/owner 和批准证据共同取得，不从状态名、None 文本或历史 token 取得。

| 当前已执行历史 | 新发生工作 | shadow 路径 | 恢复职责 |
| --- | --- | --- | --- |
| Initialize → Disabled，证据确认已有启用 legacy | 保持原批准范围；新框架关闭 | 关闭 | 当前负责身份委派 |
| 初始 Disabled → Shadow | 保持前代 owner 与原准入 | 共享 facts 的纯比较 | 当前负责身份委派 |
| Shadow → Active | 旧 actor 撤权后，批准目标 owner 获得范围 | 不另授独立权限 | 原 intent/terminal 身份继续 |
| Active → Draining | 不新建 occurrence/prepare | 不重新取数 | authority/finalizer/reconciler/隔离继续 |
| Draining → Disabled | 保持关闭，需排空证据 | 关闭 | 不删除未决事实/责任 |
| 排空后的 Disabled → Shadow | 保持前代关闭 | 仅已捕获 facts 比较 | 当前负责身份委派 |
| Rollback → 同 Unit 历史目标 | 本次批准明确恢复目标原准入，使用新代 | 随目标准入 | 只使用当前 fence |
| 确无既有生产且无恢复责任 | 经批准表达无 owner，不授予 actor | 仅独立许可的隔离比较 | 无 |

“初始/排空后”不是新增持久状态或可变标志。Initialize 从认证基线建立准入；EnterShadow 保留前代准入；Drain/Disable 关闭；Activate 建立批准范围；Rollback 递归读取严格更早的精确目标。原始行自洽不能证明初始化证据真实。

回滚到历史初始 Disabled **可以**在新批准明确授权时恢复原 legacy 范围；回滚到排空后的 Disabled 仍关闭。两者均必须验证当前 build/schema/source 兼容、操作人/窗口、旧 owner 撤权以及原 Accepted/Uncertain 去重保护，不因旧状态可读而自动重启旧 binary。

所有 legacy/new 副作用 actor 呈交当前同一 `(unit_id,generation,manifest_sha256,physical_owner)`，再核验 actor/action capability 和 gate。owner 不变而代数改变也撤销旧 token；finalizer 代表当前负责人执行，不成为第二 owner。多个 Unit 各自持有元组，不用一份全局 owner 替代。

### 排除的替代方案与代价

- 整个 Shadow Unit 写 None 并停旧路径：违背 Q13/Q17。
- 单独给 legacy 放行或保留旧 token：违背共同当前 fence，形成第二权限真相。
- 增加状态或改冻结 DDL：现有不可变历史已能表达，不需要第二状态机。
- 仅检查 owner 字符串：未认证身份、旧 binary 或路径旁路仍可破坏唯一 owner。

采用历史准入投影的代价是必须实现完整可测的派生规则，以及真实初始 incumbent 证据和每个 legacy 副作用入口的覆盖。文档澄清不证明这些实现存在。

## 实施责任与未决项

T1 只证明冻结 DDL 的内容/链/关联，保留原始 owner，不签发准入或认证。上述 owner 保留、历史准入、回滚恢复在 T2 的认证操作计划验证器和 T4/T5 的执行许可/协调流程强制；后者还须在副作用临界区重查并覆盖旧 binary，不能用 raw facts 的私有类型冒充权限。

真实主机/系统、allowlist 管理者、制品批准根、受保护 source 根、监督器持久协调和时钟/撤销仍缺生产配置；已向用户询问，未填造默认值。无兼容 fence 的旧 binary 必须经真实监督器确认停止并撤权，或先交付覆盖完整副作用面的兼容接线；不能同时承诺无缝运行和已认证撤权却没有证据。

配额仍使用现有全 Unit `Activate/Rollback` 查询，不因 owner 字符串相同而私自豁免。无 Activate 的 shadow/无 owner 变化部署不消耗名额。如果某个 conformance Unit 需要“同 owner 的 Activate 也免名额”，必须先明确该操作分类与 Q36/现有查询的衔接，不能用更容易通过的查询替换冻结规则。

多 Unit snapshot/material v3 的精确字段及 stream 版本尚未交付。下述 C 决定确定事务顺序，但实际认证和监督器 adapter 仍待，不宣称 W16/W15 或52个 Unit 已完成。

## 决定三：先批准与同事务写入的衔接

RFC 的“先批准新 manifest”指独立控制面持久保存精确 manifest 字节及批准包，不是先提交 activation 表行，更不是预写成功 journal。准备制品、关闭旧入口和等待在途动作结束均在 SQLite 写锁外完成。

同一 `BEGIN IMMEDIATE` 中重新认证命令/时间、核验完整历史和 generation、查询全 Unit 当日 `Activate/Rollback`，然后依次 INSERT manifest、确认目标 owner 已安装且 paused、INSERT 成功 journal、COMMIT。锁内只允许有期限的已准备实例确认，不允许长期排空。发生任一步错误不得留下半条成功历史；数据库回滚不能解释为外部 owner 已恢复。

COMMIT 后仍须新连接核对精确 manifest/journal，并由真实监督器确认实际 owner、批准和 gate，才能签发当前执行许可。提交确认丢失不得重试推进代数或再次切换；相同命令重查只证明持久事实存在，不自动证明 owner 正确或 Ready。不同字段的“相同命令”必须拒绝。

收益是同时满足先批准、同事务 CAS/日额和已执行 journal 三项约束；代价是实际监督器必须提供有界 paused 确认及跨进程持久协调，认证/时钟/真实 opener 也必须接线。当前先开发内部事务引擎，不暴露生产 writer、不自造认证类型；真实 adapter 和完整 T3/T5 验收仍保留。冻结 DDL 不变。
