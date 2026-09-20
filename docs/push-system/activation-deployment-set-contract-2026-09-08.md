# W16 全 Unit 部署集合合同

日期：2026-09-08。状态：本文集合编码与版本选择为已接受的工程决定；全catalog原始候选构造/读取/重读已实施至`e0cdd0d`并通过限定复核，见[实施结果](implementation-w16-results-2026-09-08.md)。v3消费者与实际认证尚未完成，不授予生产批准。依据为[W16设计](../superpowers/specs/2026-09-08-push-foundation-w16-activation-design.md)的全范围消费要求和[合同裁决](activation-contract-decisions-2026-09-08.md)；不改变八份冻结输入、Foundation SQL 或已经持久化的旧版本。

## 背景、决定与代价

当前W15 snapshot/material v2与stream v1只有单份generation/manifest/build，而Core可涉及多个Unit。选择独立 `ActivationDeploymentSet/v1` 描述完整部署集合，并采用新 `OperationalReadinessSnapshot/v3`、`OperationalReadinessMaterial/v3`、`OperationalReadinessStream/v2` 承载后续集合消费。旧v2候选和stream v1保留原字节/语义，legacy snapshot v1继续拒绝，未知版本拒绝。拒绝用最大代、任一Unit或把集合hash塞进旧manifest字段替代集合。

代价是新增显式版本分派和逐Unit缺口处理；集合完整性不认证source/实际binary/owner，生产发布仍需T2/T4/T5。snapshot/material v3具体wire与recovery跨stream衔接在T6后续接线时实现和验收，不能因本集合编码已确定就标整个T0/D或T6完成。冻结recovery event/domain/schema不随之自动升级。

## 集合规范编码

使用现有canonical-v1：ASCII domain、单个NUL、按字段名排序且无空白的JSON object。domain恰为 `ActivationDeploymentSet/v1`；集合摘要是这些完整bytes的SHA-256，不纳入自身hash，不接受自报摘要替代重算。

顶层字段恰为：

| 字段 | 内容 |
| --- | --- |
| schema_version | 整数1 |
| namespace | 复用既有namespace_value编码 |
| catalog_sha256 | 当前MachineCatalog精确hash |
| calendar | object：calendar_id、authority_sha256、utc_offset_seconds（整数28800） |
| enabled_producers | 去重验证后按ProducerId文本排序的数组，不推断“全部启用” |
| recovery_units | 去重验证后按UnitId文本排序的数组，明确仍负恢复责任的Unit，不开启新工作 |
| shared_dependencies | 按dependency_kind文本排序的object数组：dependency_kind、contract_id、contract_version、sha256；精确覆盖当前Core要求的六种依赖，不批量NotRequired |
| units | 按UnitId文本排序的全部catalog Unit条目，不仅是启用/选中Unit |

每个units条目恰含 `unit_id,activation_status,generation,manifest_sha256,journal_event_id,journal_sha256,desired_state,physical_owner,build_commit,build_sha256,source_binding_sha256`。

- 完整读取为CaughtUp时，activation_status=`CaughtUp`，其余取该Unit当前已执行manifest/journal及精确绑定的source包声明。source包声明的namespace/Unit/generation/manifest必须一致，包摘要等于manifest.source_contract_sha256（设计来源绑定条款要求绑定该包实际bytes）；此相等本身仍不是source认证。
- 确为Unregistered时，activation_status=`Unregistered`，除unit_id和activation_status以外九字段均显式null。不得改成Disabled、generation=0或省略整行。该Unit不能出现在启用producer所属集或recovery_units内，且不得夹带source包声明。
- 任一Unit Pending都拒绝构造可供后续部署一致性消费的集合，不回退旧执行代；仍可通过T1完整raw inspector观察故障。
- enabled_producers、recovery_units、source包声明、shared_dependencies不得重复、未知或范围冲突。所有已登记CaughtUp条目都必须有精确source声明；不同Unit允许不同generation/build，不声称所有manifest版本hash等于当前全局部署。

## Interface 与验证

单一构造/读取interface内部复用T1全库同事务检查、当前catalog、不可变日历join及Core依赖闭集；不把选Unit、归一化、缺行处理或摘要组合交给各调用者重复实现。scope查询从同一集合派生Core启用Unit并集恢复Unit、或精确producer/occurrence Unit；集合仍保留全catalog登记状态。

日期观察用于核验编译日历覆盖和authority声明，不进入集合身份；业务日期/采样时间留给snapshot context，避免同一部署因重复观察时间不同而换身份。重新读取时精确比较完整集合，任一相关Unit代、manifest/journal/build/source、启用集、恢复责任、日历或共享依赖变化均拒绝沿用旧观察；此比较不是跨库原子事务或未来执行许可。

验收包括：真实临时activation数据库多Unit不同代/制品、52Unit显式闭合、未知/重复/遗漏/恢复Disabled/启用未登记、任意Unit Pending、错误source精确join、独立固定canonical bytes及hash、每一字段改变、排序稳定、重读漂移。生产root/opener/source/owner认证仍需真实流程，不能新增自由构造的Verified部署集合。
