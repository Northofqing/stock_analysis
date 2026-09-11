# P-01 / N-02 专用业务恢复接线：本任务完成

日期：2026-09-11。状态：本任务完成；修后10项专用测试通过，另37项相关代码未变的回归保留通过证据；三处测试运行失败均已修正并覆盖验证，最终Clippy相对基线无新增诊断。源码为 `3fed7aa7990cba79aebdb248017b1c5dfb21839f`，独立规格/质量审查通过，0项严重/重要问题；2项低优先级建议保留，3项跨任务补核已由主控完成。隔离分支为 `codex/push-reliability-20260905`，开发 BASE 为 `6836d18d4b4d1c6377eb10a981f3533258c8784a`；这不关闭完整W16或Unit迁移。

## 这次实际接什么

按[本任务实施计划](../superpowers/plans/2026-09-11-dedicated-business-recovery-fence.md)，将两类已有专用终态 reader 接入真实 broker worker 的业务恢复和 finalizer，而不是增加另一层只查终态的接口。

- [闭集来源](../../src/push_foundation/activation_business_source.rs)保留 Generic，加入持有真实 coordinator 的 P01，以及持有真实 AuditDispatcher 的 N02。不能由执行客户端提供任意路径、callback 或“已认证”布尔值。
- [业务恢复效果](../../src/push_foundation/activation_business_effect.rs)复用现有恢复、最终化、重新打开业务库及全链确认；每次终态查询仍检查当前 worker 许可。两种专用效果使用 `ActivationDedicatedBusinessRecoveryEffect/v1`，不改原 Generic 的 `ActivationBusinessRecoveryEffect/v1` 身份字节。
- [N02 资源绑定与读取](../../src/event/dispatcher.rs)来自已经固定的目录能力，并绑定业务年份的 lock/JSONL 对象。实际持锁、打开文件与读取时核对预期身份；仅在查询前后各检查一次，无法排除读取期间换入再换回文件，因此不足以完成该合同。
- [窗口查询](../../src/event/mod.rs)复用原有窗口解析和验证规则，新增内部的带资源绑定查询；不知道的 occurrence/window 不得默认成 09:30。不是整日 gate 或生产注册的交付。

## 已取得的运行证据

全部为 Test namespace、隔离临时库和测试渠道；没有读取生产配置、启动或观察生产 monitor，也没有调用真实 provider、消息渠道、PAM 或交易。P01 fixture 的发送只向计数测试 sink 建立初始终态，恢复阶段不增加发送。

| 检查 | 实际结果与限制 |
| --- | --- |
| P01 首条真实 Accepted 恢复与 replay | 相同精确测试 exit0，1 passed；Completed/version3/三条转换，重放不增加发送计数，原终态不变。不是全部 P01 故障路径通过 |
| N02 首条真实 Accepted 恢复与 replay | 相同精确测试 exit0，1 passed；真实 schema-v5 attempt/terminal → worker → Completed，全链重读；重放后审计文件完整字节和计数不变 |
| 原 Generic + 两条专用正例合批 | 2026-09-11 01:22:09 上海时间结束，exit0，16 passed / 0 failed / 0 ignored，运行11.10s；原14项包括独立 literal golden、实际 encoder 变体和原恢复反例，两类专用各1项 |
| 第一次合批的权限限制 | 14 passed / 2 failed；两项均在临时 UnixListener::bind 被沙箱 EPERM 拦截。申请并获准同范围本地 Unix socket 测试后，完整16项复跑通过；没有跳过或改弱断言 |
| 新专用同进程组首轮 | `dedicated-full-1`：exit101，8 passed / 2 failed / 0 ignored，实际10项，运行5.47s。状态矩阵、P01库替换、N02缺锁和打开FD后换回反例通过；来源类型编码变体及错误渠道写入边界失败，待修正，不报整组通过 |
| 新专用真实进程组首轮 | `dedicated-process-1`：exit0，2 passed / 0 failed / 0 ignored，运行2.20s。首项遍历P01/N02，经实际测试子进程、Unix wire、请求进程退出、worker继续、quiesce、完整结果/证明和重启replay；第二项验证P01资格确认后broker死亡仍未决、不新增投递尝试/结果。此证据绑定首轮源码，修后仍须复核 |
| 修正注册/编码后的47项合批 | `dedicated-final-matrix-1`：exit101，46 passed / 1 failed / 0 ignored，运行27.79s。原两处失败已关闭；唯一剩余失败为N02完整literal用追加文件后的目录链接数比对固定根时的原始观察。专用进程2项、旧Generic行为14项/进程4项、W13 8项、W19 4项、dispatcher 5项均通过 |
| 修正观察时点后的专用组 | `dedicated-final-fixture-1`：exit0，10 passed / 0 failed / 0 ignored，运行7.34s。两类完整literal、P01 8个/N02 22个来源字段变体、错误渠道、状态与替换反例全部通过。此次只改专用测试夹具，其他37项相关代码未变；这是两次运行的对应证据，不是一次47项全过 |

合批实际 Cargo 命令：

```bash
cargo test --offline --lib push_foundation::activation_business_effect:: -- --test-threads=1
```

前后源码摘要一致。两条专用测试的 RED 是缺新接口的编译错误，不是生产缺陷的运行复现；装配期间出现过 fixture 字段类型错误及修错同名位置，均保留失败记录，修正后才取得纯缺接口 RED 和上表 GREEN。此版本有44条 lib-test warning（基线43）；新增未使用的 route 方法仍待本任务收口，不宣称静态零诊断。

新矩阵首轮的两条命令分别使用过滤器 `push_foundation::activation_business_effect::dedicated_tests` 与 `push_foundation::activation_dedicated_business_process_tests`，其他参数同上；两次捕获均确认源码前后摘要相同，lib-test warning均为43条。首轮失败不是编译错误或权限失败：P01完整literal先匹配，随后修改descriptor的authority_class未改变实际编码；错误渠道经原恢复器写入同态 `FinalizerTerminalRefInvalid`，与本任务注册时应拒绝错误绑定的边界不符。

已修正descriptor类型进入实际专用编码，并在专用注册时只读核对真实source/route，不缓存其终态、不修改原运行期错误记录规则。Missing/PendingSeal及非Accepted仍保留原恢复边界；代价是多一次注册读取，来源暂不可读时须重试注册。每次worker资格与最终化仍独立重查。N02暂停钩子在注册后安装，确保替换反例覆盖的是实际worker两次读取；修后验证见上表。

N02的 `root_links` 是固定目录能力时保留的观察值，不是每次当前目录链接数必须相等的断言；现有目录链验证仍检查真实对象、类型、权限及所有者，允许正常增加文件改变目录链接数。最后一处修正仅让测试在dispatcher构造后、日志追加前独立采集元数据，不读取被测binding来拼期望，也没有修改目录防护语义。

全部12项变动Rust路径的定向格式检查、差异空白检查通过。首轮最终Clippy为189条告警，比188条基线仅新增一个只供测试使用的Arc导入；已仅加 `#[cfg(test)]` 限定，反向字节摘要核对证明没有夹带行为修改。最终 `dedicated-final-clippy-2` 使用 `cargo clippy --offline --lib --message-format=json`，exit0，188条告警/0错误，与基线诊断多集比较新增0、移除0；不是零告警项目。37项邻域中的8项W13使用临时库里的合成 `Namespace::Production` 枚举和fake source，没有真实生产路径或认证；它们仅证明旧映射兼容，不替代本次真实Test authority验证。

阶段证据绑定以下源码快照；后续源码继续开发，不能把这些通过数直接当作最终版本的验收：

| 文件 | 本次合批 SHA-256 |
| --- | --- |
| activation_business_effect.rs | `facf0311068d5e6c1edafae010a73b989a409348647567d5dd711071895c27ea` |
| activation_business_source.rs | `a1a429b9181fc855a6001a284795a6deb620e1214823a2f5ffe48b6d6176b282` |
| event/dispatcher.rs | `19fd6da55776f85011c08b4368ffa34adcef416f75e16e7307d5a75174a48e36` |
| event/mod.rs | `d4b1588c3b03981de317edfd6d52917e524cfaf48571d6b3875d84045e7d2fa5` |
| activation_dedicated_business_effect_tests.rs | `8d1fea364a76058e531268b1e057f23e85542cc91033144848faadafff8b8f32` |

## 独立审查与主控补核

独立审查覆盖固定 `6836d18..3fed7aa` 差异、实际失败/修后测试记录与Clippy诊断比较，结论为Spec compliant / Task quality Approved。审查没有重跑测试或扩大到生产。主控另完成以下补核：

- P01连接级对象防护、同日once claim、Scheduled/Compensation及专用字段映射：复核当前真实读取入口；coordinator相对[已审阅的运行期防护](implementation-durable-runtime-schema-guard-2026-09-10.md)源码8ee1e13未变，保留[不确定终态读取](implementation-recovered-uncertain-read-2026-09-08.md)与本次W13/W19回归证据，不重新宣称全面生产认证。
- W09/finalizer、完整业务链、worker寿命、Replay/Unresolved：沿用[W16已验收的T4B/T4C/T4D](implementation-w16-results-2026-09-08.md)与本次实际进程证据。reconciler/finalizer/IPC相对667ee4a未变；intent_store后来只增加SLA/库存读取接口，主控已核对对应差异，旧写入/全链算法未被改写。生产身份和broker拒绝入口也已重新核对。
- 文档、格式与差异：公开材料已纳管，链接、最终12源文件摘要、定向rustfmt和暂存/非暂存差异检查通过；这是主控完成的检查，不归为审查者执行。

两项低优先级建议保留到后续整体审查：P01进程测试可从attempt/result计数增强为稳定排序的完整逻辑行快照；43条测试/188条Clippy既有告警仍需单独治理。当前未发现恢复路径写入source，但不把计数断言夸大为全字段不变证明。

## 完整目标仍未完成的范围

生产身份和 broker 构造继续拒绝，测试装配不提供生产权限。这一恢复任务也不交付专用物理发送、P01 once claim/N02 settle 等具体 Unit 完成标记、全库启动恢复、完整四角色切换或52 Unit迁移。[完整 W16 剩余范围](implementation-w16-results-2026-09-08.md#完整剩余范围)及[全目标迁移证据边界](remaining-migration-evidence-2026-09-08.md)保持有效。
