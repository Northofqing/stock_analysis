# W15/W16 → P-02：认证与运行时接线缺口

2026-09-11范围变更：用户已决定仅供本人使用，本次按[单用户本地模式](single-user-local-scope-2026-09-11.md)取消复杂可信身份认证前置。下文保留原方案的源码证据，不再将身份发行方、多角色审批或认证broker列为本地交付必需阻断；实际context接线、数据有效性、任务锁和防重复效果仍需完成。

日期：2026-09-10。源码固定在 `8ad4f9fe56fbe6de954456e675d23208518431aa`，工作树为 `.worktrees/push-reliability-20260905`。本文是源码与实施前置核对，不是新认证 adapter 的交付记录。没有运行生产 monitor、PAM、真实数据库或消息渠道，也没有改变现有推送行为。

## 结论：不是只缺一份配置，也不能宣布所有开发都被配置阻断

当前同时缺少三段实际入口：生产操作者认证成功路径、生产 broker 成功构造、可信运行注册进入 RunContext 工厂的路径。三个缺口分别有下方可定位的代码证据。已有候选集合、只读重查、Unix 连接身份观察和隔离进程测试不能补齐它们；设置环境变量或公开一个构造器也不能完成接线。

[W16 设计的 needs-context A](../superpowers/specs/2026-09-08-push-foundation-w16-activation-design.md#module-与信任根)已明确允许接口、隔离实现和拒绝路径先行，真实平台、身份发行方与受保护根则仍是生产认证前置。因此，“外部配置待确认”和“认证代码未实现”必须分账；不能把完整 W16 标成“开发完成，等待配置”。[当前蓝图 §10](../architecture/current/Project_Architecture_Blueprint.md#L303)也已指出独立 monitor 未见 PAM helper 调用；本次是当前源码复核，不是首次发现该事实。

## 逐项源码证据

| 接点 | 实际代码与调用事实 | 对后续开发的约束 |
| --- | --- | --- |
| 旧 PAM 开关 | [load_auth_config](../../src/auth/operator.rs#L52)只把 `MONITOR_AUTH_REQUIRED=1` 解释为 required，其他值及缺失均为 false；[跳过分支](../../src/auth/operator.rs#L81)和[PAM 成功分支](../../src/auth/operator.rs#L113)都返回 `Ok(())` | 返回值不能区分跳过与认证成功，不是 W16 身份证明；不改变旧开关行为来冒充新认证 |
| PAM 实际调用者 | [主 CLI](../../src/main.rs#L30)与[winrate_simulator](../../src/bin/winrate_simulator.rs#L101)调用 helper；[独立 monitor main](../../src/bin/monitor/main.rs#L4342)从环境/日志/CLI bootstrap 开始，搜索未发现调用或库内包装路径 | helper 名称及注释不证明 daemon 已强制认证；即使补接调用，也没有返回规范主体或精确批准 |
| 原始批准比较 | [compare_raw_claims](../../src/push_foundation/activation_authorization.rs#L277)验证绑定；[Production 分支](../../src/push_foundation/activation_authorization.rs#L296)明确拒绝；[成功值](../../src/push_foundation/activation_authorization.rs#L352)只是 `RawClaimComparison`，实际比较 caller 在 [cfg(test) 策略](../../src/push_foundation/activation_authorization.rs#L380)内 | 不能把 Test 正例或自报 issuer/subject/revocation 字段提升为生产批准 |
| 生产身份入口 | [refuse_without_production_trust_root](../../src/push_foundation/activation_authorization.rs#L356)返回 `Result<Infallible, AuthorizationRefusal>`，分支全部为 Err；[DualControl](../../src/push_foundation/activation_authorization.rs#L370)也没有成功分支 | 真实身份发行、角色授权、精确在线批准、撤销与跨重启防重放都不能仅靠配置该函数获得 |
| Unix peer 观察 | [serve_connection](../../src/push_foundation/activation_fence_ipc.rs#L96)在非测试代码中调用 `observe_unix_peer`，将真实连接 UID/GID 传给 broker 校验 | 不能说整个授权模块“只有测试代码”；内核连接事实仍不证明规范人类身份、实际部署或批准 |
| 生产 broker | [EffectBroker::production](../../src/push_foundation/activation_fence.rs#L638)固定返回 `ProductionRefused`；已有可运行成功 fixture 位于该文件测试条件编译部分 | 真实 Unix 协议、Generic/业务恢复 effect 的隔离证据有效，但当前没有可由生产入口初始化的认证 broker；不能只加环境变量就开启 |
| 持有 FD 的制品观察 | [OpenedDeploymentMaterial::observe](../../src/push_foundation/activation_deployment.rs#L282)保留实际 FD，前后 metadata/hash 校验来自同一描述符；其注释明确未证明受保护、无链接的打开来源 | 内容未漂移不等于受信管理员批准；仍需受保护 opener、所有者/权限及实际运行制品绑定 |
| 工厂输入与注册 | [CatalogRunBinding](../../src/monitor/push_job/context.rs#L189)、[RunContextInput](../../src/monitor/push_job/context.rs#L204)字段私有；[工厂](../../src/monitor/push_job/context.rs#L221)和[捕获方法](../../src/monitor/push_job/context.rs#L267)存在于非测试代码，但实际 binding/input 构造仅在 [cfg(test) fixture](../../src/monitor/push_job/context.rs#L497)内 | 缺的是受约束运行注册入口，不是所有类型都只在测试存在；`:178` 的 W06 注释不能充当实际接线证据 |
| 当前 W15 查询 | [load_current_v3_candidate](../../src/push_foundation/readiness_query.rs#L71)取 current record、重读全部署集合、再读 current record，返回第一份完全相等的候选记录 | 已有 v3/current 查询不重做；返回类型及[模块边界](../../src/push_foundation/readiness_query.rs#L1)仍不认证来源，不授予未来执行许可，也不是跨库原子快照 |

独立只读 agent 搜索了 `src/`、`tests/` 与工作树 Rust 源文件中的类型、构造、调用、别名/import/re-export；检查了 `cfg`、二进制入口归属及 `build.rs`，未发现生成的认证入口。主控复核上述关键分支、源码 pin 和摘要。负面调用结论仅覆盖该源码树，不证明任何部署进程的状态；没有借生产进程读取补证据。

## 实施依赖：哪些必须先有真实证据

```text
真实身份发行方 + 受保护配置/批准根 + 实际部署/source 绑定
  ├─ W16：认证请求 → 真实 broker/owner → 四类 actor 当前许可
  ├─ W15：认证同一份 v3 record → health / probe / CLI 共同投影
  └─ 运行注册：批准代次/build/source/模板 + 日历 authority → RunContext
                                                        └─ P-02 同次 facts → 两个真实 adapter
```

这是接线依赖图，不把 W15 Ready 新增为 W16 的正式循环依赖；W16 的 WBS 依赖仍是 W06/W08/W11/W12。W15 认证快照也不替代 W16 在副作用临界区取得的当前许可。[正式依赖与目标](../superpowers/specs/2026-09-08-push-foundation-w16-activation-design.md#目标依据与范围)保持不变。

| 待完成工作 | 平台/外部配置确认前可先行 | 不能提前宣称完成的证据 |
| --- | --- | --- |
| 身份接口与拒绝语义 | 分离原始引用、连接观察、认证主体与操作批准；保持 Test/Production 隔离；对伪造/过期/错绑定做隔离反例 | 真实身份发行方签发的规范主体；不能由普通字符串/UID 或可跳过 PAM 成功值代替 |
| 受保护材料加载与批准存储 | 按已批准合同设计受限接口和隔离实现、路径替换/权限/重放反例；不选择仓库内自报 production allowlist | 所选平台真实 opener/ACL 语义、批准管理员与防回滚根、撤销/轮换/可信时间接线 |
| catalog → context 映射 | 校验已有 Unit/producer/family/phase 关系；验证缺输入必须拒绝；完整比较输入绑定 | source/version、模板、audience、completion policy、build/generation 的受约束来源；不能补测试常量或开放私有字段 |
| 四 actor 接线 | 继续实际调度创建、采集准备、发送、完成/恢复 effect 的隔离接线与撤权测试，复用已完成 broker/Generic/Finalizer 能力 | 生产 broker 成功初始化、真实旧 binary 排空/资源撤权、实际 owner、52 Unit 各自无旁路证明 |
| W15/P-02 消费 | 复用已有 v3/current 查询、一次来源观察、冻结完整提案及 W17 比较机制；设计真实消费者的失败路径 | 同一认证 snapshot 的三消费者；P-02 两个真实 adapter、八端口纳管、dispatcher 消费被比较的同一旧提案；实际缺量比不能 Ready |

表中“可先行”是尚可开展的开发工作，不是本轮已实现；平台选择也不自动完成右栏。后续不能只累计 raw/candidate 工具或 Test 构造器来替代右栏的正式验收。

## 外部待确认项与实施顺序

已向用户提出一个部署平台问题：本机 Mac、独立 Linux，或已有指定环境。本轮未获得可解释的选择，不把默认选项或用途不明输入当批准。确认平台仅用于选定实现，不授权启动、approve/apply 或切换生产。

[W16 needs-context A](../superpowers/specs/2026-09-08-push-foundation-w16-activation-design.md#module-与信任根)仍需明确实际身份发行方、allowlist/制品批准/source locator 管理者、受保护存储和持久防回滚根、轮换撤销及可信时间策略。这些是非秘密的部署合同；无需用户在聊天中提供密码、验证码、token 或私钥。不能自填用户名、UID、路径、签名根或生产批准记录。

落实顺序沿原 T2 → T4/T5 → T6/T7，允许原计划已明确的独立任务并行：

1. 先确定实际平台/发行方，完成生产专用身份与受保护根 adapter；不复用旧 `Result<()>` 充当已认证身份，不修改旧 monitor 认证行为作为替代交付。
2. 由认证部署流程向实际 broker/owner 生命周期提供初始化材料，按原协议完成批准、quiesce、paused owner、有界事务确认及重新认证；接四类实际效果，旧路径兼容仍按初始/排空后/rollback 的既定矩阵。
3. 让 W15 对同一当前记录完成来源/部署/时间/恢复认证，再供 health/probe/CLI 消费；运行注册从同一批准来源获得 context 所需值，接 P-02 完整事实/提案/实际消费。[P-02 具体接线验收](p02-shadow-integration-handoff-2026-09-10.md#下一任务的可观察验收)不缩减。
4. 继续 W18 真正的权限、独立审计和执行响应，以及 W19–W21 安全/故障矩阵/发布门禁与全部 Unit 迁移。本文不改 [52 Unit 剩余范围](remaining-migration-evidence-2026-09-08.md#对后续开发与排期的影响)，不新增迁移完成数。

## 证据身份与验证边界

以下为本次实际读取文件的 SHA-256；行号对应上述源码 pin，后续源文件变化应重新核对，不能只替换摘要。

| 文件 | SHA-256 |
| --- | --- |
| `src/auth/operator.rs` | `b12b810d239a29b6e1b44929b8d935b3de9587286bf42354ce3d1edec9a09ab3` |
| `src/push_foundation/activation_authorization.rs` | `98e3b37de6e6c8a14f75f3896fe30889f55d87d0b8f8a3bacdd74a237a9bc7ff` |
| `src/push_foundation/activation_deployment.rs` | `e67c1f60435f9adcdb28da0fb062a903f7b6b1dc07b0a2ccfa78e8027b0db086` |
| `src/push_foundation/activation_fence.rs` | `b19794a01fd1a258c656b0f3c7a211628bdf563ba055520d15d2acb09d40a805` |
| `src/push_foundation/activation_fence_ipc.rs` | `0387b067a284347da4f70d45c0cc94409736bdccd788b061277440fb2226cd2f` |
| `src/push_foundation/readiness_query.rs` | `2185f42bc042078fb9c73a21f40b9af4e5525a64753b7160fe939d8f9ae4adbe` |
| `src/monitor/push_job/context.rs` | `7d19f1f2a3ba43b15ba43f35a0a9fdf0c8c61b562539f71f1492c63e22db360c` |
| `src/bin/monitor/main.rs` | `c87fa4bb56df1c786e11516f754316e0a3fb9d9f5f75aad8aa2a7cb02016488c` |

本轮仅交付接线证据文档及入口索引，不改 Rust/Cargo/运行配置，不重跑既有行为测试，不把静态核对写成新运行期测试通过。文档检查覆盖链接文件、数字行号、上述源码摘要及差异空白；Markdown 标题锚点另按实际标题核对。当前蓝图及其双 HTML 是固定来源制品，本轮不覆写或刷新其旧 pin。完整目标保持未完成。

三份涉及文档的实际只读检查结果为139个本地链接、52个数字行号、8份源码摘要，均通过。该检查只证明文件/行号存在与字节身份；链接是否支持正文结论仍以源码阅读和限定事实复核为依据，不能把链接计数当运行期验收。

独立限定事实复核未发现实质性错误；建议将“可先行”的直接证据指向设计原文，并把两个函数链接定位到定义行，三项均已落实。不据此声称完整 W16 规格/代码审查或生产验收通过。
