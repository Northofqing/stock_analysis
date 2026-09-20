# W19 全量库存指标：实施与验收记录

日期：2026-09-08。状态：**本计划Task1完成；修复后107项相关测试、静态检查及限定复审通过**。不代表完整W19、生产接线或整体迁移已完成。

隔离工作树：`.worktrees/push-reliability-20260905`；分支：`codex/push-reliability-20260905`。原始基线 `3af4fb2`，计划提交 `c97bcfd`，初版源码 `8094ffc`，修复源码 `c10ec78`。仅本地提交，未推送、合并或部署。

计划：[持久化全量指标与覆盖范围](../superpowers/plans/2026-09-08-push-foundation-w19-inventory-metrics.md)。前置实现：[单条最终化延迟检查](implementation-w19-results-2026-09-08.md)，修复源码 `6f8713c`；其 78 项通过不代表本批修改后仍通过。

## 本批要解决的问题

单条检查能够发现一条推送的接受、完成、超时及历史矛盾，但不能回答整个选择范围里还有多少积压。旧 `scan_recovery_page` 只枚举四种恢复状态，且仅用于测试；直接复用会漏掉已完成、已处置和其他历史记录，不能作为全量指标来源。

本批从指定 namespace 的实际业务库枚举所有业务日、所有状态，再逐条读取真实投递来源并复用已完成的 SLA 判断。总数、分页及业务转换链属于同一读取事务；投递来源库仍是独立读取，不宣称跨库原子快照。

必须保留总库存、已检查、未检查和 Complete/Limited 覆盖状态。达到读取预算不等于扫描完成；缺路由、损坏记录或来源验证失败不能被跳过后呈现“没有问题”。人工接受、实际接受、业务完成、未决和失败独立计数；历史 Completed 之后又进入 ResolutionRequired 时，历史完成时长与当前未决年龄分开。

已纠正两个接口问题：绑定按 `(Unit, 持久化模板哈希)` 选择，使同一 Unit 的新旧模板证据可并存；报告保留观察时点、周期和两周期阈值，避免后续消费者混用不同时间或阈值的统计。对应反例已纳入最终107项验证。

## 已实现的数据链与边界

| 实际实现 | 行为与限制 | 源码证据 |
| --- | --- | --- |
| 指定范围全库存读取 | 一个source-owned DEFERRED事务同时读取总数、按业务日/ID稳定分页、snapshot及完整转换链；遍历全部业务日和状态，结束rollback | `src/push_foundation/intent_store.rs:2090` |
| 私有读取证明 | 只由业务存储构造已验证snapshot/chain组合，单条与批量共用SLA判断；不接受调用方手造成功report替代读取 | `intent_store.rs:1218`、`finalization_sla.rs:306` |
| 显式精确来源选择 | 使用实际Generic/P01/N02 reader；同Unit历史模板允许并存，同pair重复提前拒绝，缺Unit/错模板不自动回退 | `finalization_metrics.rs:56,393` |
| N02局部路由 | 从持久化occurrence解析现有NewsFlashWindow，仅支持明确的`news-flash-window`约定；未知约定显式拒绝 | `finalization_sla.rs:296` |
| 总数与错误分母 | 每条已检查记录贡献一个SLA状态或错误；保留total/checked/unchecked、Complete/Limited。读取预算1..100000、页大小1..1000；预算等于总量才可完整，空库存不签发健康许可 | `finalization_metrics.rs:63,89,196,246,393` |
| 独立统计轴 | 业务状态、SLA状态、实际/人工接受及不投递分别计数；历史完成不能遮蔽当前冲突或未决，ClockUncertain不贡献有效时间样本 | `finalization_metrics.rs:95,128,170,373`、`finalization_sla.rs:239` |
| 最小披露 | 报告只读、私有构造，保存范围/计数/时间；无消息正文、持仓、原始SQL错误、回执message_id或晋级成功布尔 | `finalization_metrics.rs:246,270` |

业务库存的一致快照不等于业务库与投递库的跨库原子快照。读取预算限制实际检查条数，不保证库存总数查询及排序的延迟上限；本批没有规模性能验收，也没有改冻结SQL或添加索引。

## 验证发现并修复的问题

1. **N02冷读会重建缺失年度lock。** 真实反例先保留两份隔离fixture的原lock，再分别调用单条/库存入口。初版返回了有效SLA而非SourceInvalid。`src/event/dispatcher.rs:800`现复用existing-only打开器；完整年度链、锁及叶节点身份校验保留，append/writer不改。缺失lock现在明确拒绝，不为读取补建文件。兼容变化同时影响新闻业务日reconcile，已加入12项相关event测试。
2. **业务Completed但当前终态冲突时漏计等待年龄。** 初版getter直接按业务Completed排除，即使冲突与阻断计数均为1也返回空年龄。`finalization_sla.rs:239`现只排除当前SLA Completed/ClockUncertain；新增实际库存反例验证Conflict仍保留400秒等待年龄，不计作正常完成。旧正常完成、时钟异常和历史ResolutionRequired行为均保留。

混合库存测试使用同一业务库/namespace中跨业务日的8条真实持久化记录：实际接受待完成、实际完成、完成后冲突待处理、人工不投递、来源缺失、待封存、NoData、Disabled；独立断言总数8、正常完成1、实际接受3、历史最大完成30秒、当前最大等待400秒及阻断2。跨页测试通过第二个真实连接写入，证明原读取仍见3条，新连接见4条，而非仅检查函数名称。

测试入口：`finalization_metrics_tests.rs:81`（N02冷读）、`:256`（混合8条）、`:660`（第二连接）、`:755`（Completed+Conflict）。损坏注入仅在隔离fixture临时移除并恢复原触发器，不修改冻结DDL或生产验证规则。

## 后续接入点核对

下表是对当前既有源码的只读核对，不表示这些接线已经实现。

| 既有接口 | 已确认的能力 | 本批指标不能替代的后续工作 |
| --- | --- | --- |
| `operational_readiness.rs:52,113,121` 的 DependencyKind 与依赖集合 | Core、Producer、Occurrence 的依赖要求及观察比较 | 这些集合没有全量 intent/SLA 输入。须独立消费完整覆盖、未决、冲突与超龄事实，不能从依赖 Available 推导零积压。 |
| `readiness_probe.rs:168` 的 CandidateReadinessInventory::evaluate | catalog 与调用方提供的 producer 依赖事实的候选覆盖统计 | 静态 producer 候选覆盖与实际持久化推送库存是不同分母；空库存也不是 producer 就绪或生产身份认证证明。 |
| `activation_transaction.rs:29,37,169` 的 candidate/coordinator/事务入口 | 既有内部写入引擎在锁内检查历史、版本、日配额，并要求 coordinator 重验批准与时间 | 接入健康事实仍须绑定当前 Unit/build/manifest/generation/窗口及全部晋级门禁；库存报告不能自行签发批准或替代六门禁。 |

依据为 [RFC](push-system-implementation-rfc.md) 的“通用晋级门禁”：超龄积压、未解决 Uncertain、readiness 不就绪及双库不一致须阻断晋级。库存指标只提供其中部分实际事实，不是完整健康结论。

## 验证状态

- N02真实RED：52970，编译成功3m15s，0通过/1失败/3303 filtered，0.45s；修复后53598合批106项通过。原RED在首次拒绝断言失败，不冒称当时后续无写入断言已通过。
- 初版独立完整审查范围为`3af4fb2..8094ffc`，发现0 Critical、1 Important（上述年龄遗漏）、2 Minor，未批准本批。
- I1真实RED：37150，编译成功2m24s，0通过/1失败/3305 filtered，0.61s；实际`None`、预期`Some(400s)`。确认行为失败后再最小修复，测试期望不变。
- **最终修复后35304：107 passed / 0 failed / 0 ignored / 3199 filtered**，编译1m19s、运行26.76s；17项指标、15项SLA、11项Generic、8项Dedicated、12项W09、18项W10、14项W08、12项event消费者。
- 最终lib Clippy17611：exit0、1m11s，163条既有warning，本批Foundation/dispatcher零诊断、无error；测试编译43条既有warning。目标未新增告警不等于全库零告警。
- 三份修复Rust定向格式、diff与RFC输入校验通过；只证明各自范围，不替代生产验证。固定`8094ffc..c10ec78`限定复审确认I1已关闭，无新增Critical/Important/Minor，Approved。

最终命令（隔离工作树内）：

```sh
env CARGO_PROFILE_TEST_INCREMENTAL=true cargo test --lib -- --test-threads=1 push_foundation::finalization_metrics_tests:: push_foundation::finalization_sla_tests:: push_foundation::generic_transport_tests:: push_foundation::dedicated_transport_tests:: push_foundation::terminal_authority_tests:: push_foundation::business_finalizer_tests:: push_foundation::tests::w08_ event::delivery_observation_tests::br244_ event::delivery_observation_tests::w13_n02_
cargo clippy --lib --no-deps --message-format=json
```

两项Minor仍有明确归属：`InventoryErrorCounts`的business_invalid/terminal_invalid只保存内部计数，结构化分项getter留后续指标消费者接入；43/163条既有warning留最终全分支统一审查。本批不通过扩大修复循环混入无关清理。

没有启动、监控、替换或重启生产 monitor；不读取真实数据库、消息正文或持仓，不调用 provider/LLM/sink/order，不修改生产 owner、批准或部署。

## 完整目标仍保留

本批之外仍需 W19 的生产健康/晋级消费、Uncertain 人工响应 SLA、保留期/法律保留/独立备份完整性和安全审计，以及完整 W15/W16/W17 接线、W18/W20/W21、52 个 Unit 迁移与真实验收。实现一份全量报告不能关闭这些工作。
