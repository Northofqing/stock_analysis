# 52 个迁移单元：当前完成证据与剩余核对边界

2026-09-11范围调整：按用户决定采用[单用户本地模式](single-user-local-scope-2026-09-11.md)。旧剩余清单中的复杂可信身份、多角色批准及外部认证平台从本次范围排除，不计作已开发完成；52个Unit的实际业务接线、事务/防重、故障恢复和发布回滚验收不减少。

日期：2026-09-08。只读核对；不是生产盘点或上线认证。范围为隔离开发树，深入 7 个源文件；没有读取生产数据库、启动 monitor 或变更激活状态。

2026-09-10 补充：[MU-auction-candidates 调用链](auction-candidates-call-chain-2026-09-10.md)已按409dbaf源码核对A-02/P-05/T-08共用外门、两次读取、冷却和快照/预测效果顺序。累计7个Unit可指出实际旧业务入口，另45个尚未逐项追完；这仍不是迁移完成率。下面6/46是2026-09-08当次核对的历史计数，不删除原始范围，也不把新增静态反例当生产复现。

2026-09-10 本轮增量：[MU-limit-boards 调用链](limit-boards-call-chain-2026-09-10.md)按1931014核对三个盘中连板producer的实际来源、字段拦截、预先通知集合、展示截断和共享L4键。最新累计为8个Unit已定位实际旧入口及主要完成门，另44个尚未逐链完成；7/45与6/46保留为历史进度，不构成迁移完成百分比。当前上游主力净流为None而下游要求Some的源码缺口尚未修复，也未作生产复现。

2026-09-10 后续增量：[MU-chain-post-close调用链](chain-post-close-call-chain-2026-09-10.md)按20f215d核对盘后timer、核心/补充来源、业务落库、报告及多渠道通知。当前累计9个Unit已定位实际旧入口与主要完成门，43个尚未逐链完成；此前8/44等为历史口径。通知失败被mode吞成Ok后封日、自然日/业务日错位及窗口/恢复缺口尚未修复；没有运行态或Unit迁移验收。

2026-09-10 最新增量：[MU-holding-plan调用链](holding-plan-call-chain-2026-09-10.md)按8daa8bf核对定时、手动和启动恢复，补明手动banner初始化顺序、非原子日表、内容级decision与同批来源缺口。累计10个Unit已定位实际旧入口及主要完成门，42个尚未逐链完成；此前9/43等是历史口径。新增内容为静态证据，未修复、未运行生产，不计算迁移完成百分比。

2026-09-10 后续修复：[手动入口实施记录](implementation-manual-push-bootstrap-outcome-2026-09-10.md)对应初版de990c8、最终41e7762，实际runner已接真实健康准备及失败Err。两个原缺陷均先RED后修复，初审A-01测试接口问题也已修正；修后10项新回归/静态检查与限定复审通过，未改的7项scheduler回归证据保留。A-01健康失败保留A-10继续执行；不改变精确时窗与P-01 owner。HoldingPlan同批来源、日表/rolling完成权与有效修订准入仍待；10/42调用链口径不变，也没有增加任何完整迁移认证数。全仓旧格式差异和生产证据限制详见实施记录。

2026-09-10 最新来源切片：[持仓来源实施记录](implementation-holding-plan-frozen-source-2026-09-10.md)对应112ff8f，实际manual/periodic共用准备已消除重复持仓读取并保留快照/行情批次证据，单次本地时间驱动同轮提案。真实来源RED后修复，27项最终定向测试/静态检查通过，独立Spec/Quality通过，仅保留既有告警Minor。来源认证、新鲜度与日表/有效修订/再次发送资格仍待；10个已追链、42个未逐链的口径不变，不增加迁移认证数量。

## 结论

WBS 与能力目录均包含 52 个迁移单元，Unit ID 集合一致。**目录完整、测试通过、旧业务入口存在，都不等于完成 Foundation 迁移。** 本次能为 6 个 Unit 指出实际旧业务调用，另外 46 个尚未逐项追完调用链；不能把后者记成“未实现”，也不能报告虚假的已迁移百分比。

全部 52 个 Unit 的完整迁移认证数量仍未知。本次没有获得真实激活代次、受保护来源根、执行身份、owner 切换、六门禁、自然交易时段观察及远端接收的完整证据包。此结论是“尚不足以认证”，不是“已证明 0 个实现”。

## 能证明的实际旧业务入口

| Unit | 当前源码证据 | 不能据此推出 |
| --- | --- | --- |
| MU-p01 | `src/bin/monitor/main.rs:5483` 启动独立 scheduler；`:4993` 调用专属补偿 | Foundation 已接管、业务 finalizer 与新 owner 已上线 |
| MU-news-flash-aggregate | `main.rs:7769` 预约当前权威；`:7790` 消费真实 reservation | N02 已完成新框架迁移和生产验证 |
| MU-auction-volume | `main.rs:9748` 调用竞价 dispatcher 并传通知集合 | 完整 W17 影子执行与切换已完成 |
| MU-account-mode | `main.rs:5468` 调用账户状态通知 hook | 已使用迁移后的 finalizer/owner |
| MU-data-mode | `main.rs:5473,5486` 调用状态 hook 并启动常驻 loop | 已完成全链迁移 |
| MU-snapshot-stale | `main.rs:5477` 调用过期提醒 | 同 Unit timer/恢复全链均已核对 |

上述行号对应本次未改动的 main 基线。Unit 归属依据 `push-capability-catalog.v1.json` 中对应 migration_units，分别见约 8695、8756、8813、9024、9008、9088 行。

## 为什么现有“注册/集合”不能直接计为迁移数

- `src/bin/monitor/presentation_registry.rs:10,401,405` 的 58 项表注册的是呈现 family/kind/producer/renderer 接点，不是 52 Unit 的 Foundation 激活表，两个数量不能相除。
- `src/bin/monitor/durable_delivery_runtime.rs:1331,1357,2199` 的真实旧发送接口和 envelope 构造、`:1166` 的启动恢复，证明旧 durable 路由，不证明已使用 Foundation intent、业务最终化和 W16 owner fence。
- `src/monitor/push_job/catalog.rs:1,14,237` 明确机器目录与生产接线是不同证据。
- `src/push_foundation/activation_readiness.rs:349,538` 的持久读取和全目录校验覆盖全部 Unit，但 `:632` 明确允许 `Unregistered`。全目录 52/52 只能证明枚举完整。
- 同文件 `:392–409` 拒绝将未注册 Unit 声明成启用 producer/recovery；`:1–4` 明确候选集合不认证来源、owner、二进制或执行权。`:888` 附近的 TEST 样本不能提升为真实生产注册。
- `src/push_foundation/activation.rs:75` 与 `activation_deployment.rs:1–4` 将数据库声明和当前部署事实分开，真实受保护根/source/opener 认证仍需适配。

## 对后续开发与排期的影响

优先接点是让真实 runtime 的 producer/recovery 配置及受约束来源包消费已有全 Unit 部署集合，而不是继续增加目录常量。读取接口位于 `activation_readiness.rs:83,349,360`；启动/恢复边界位于 `main.rs:4954,5479` 和 `durable_delivery_runtime.rs:1166`。必须保持 namespace/catalog/generation/manifest 的真实绑定；缺来源事实时保留 Unregistered/Blocked，不自动注册、启用或调整晋级次序。

本次核对只补强“迁移完成证据”这一部分。完整剩余范围仍包括 W15/W16 的认证与共同 fence/监督器、W17 的真实业务和效果端口、W18 操作权限、W19 告警/解决时限与留存安全、W20 故障矩阵、W21 发布门禁及真实远端 CI。2026-09-10 状态校正：离线双 HTML、统一校验器和本地 CI 配置接线已分别交付，不能继续笼统列为未开发；实际运行/发布证据须另计。相关局部完成记录见 [当前开发入口](README.md)。

WBS 的历史人力工时不能直接当作 AI 剩余开发时间，42 个晋级交易日下限也不能代替代码开发排期。下一份可信总排期应以逐项已验收成果扣减、未接线依赖和独立的上线观察约束为依据；本文件不作新的总工期承诺。
