# P01/N02 专用终态接入 W16 业务恢复 worker

日期：2026-09-11。状态：实施中。BASE=`6836d18d4b4d1c6377eb10a981f3533258c8784a`。

依据：[W16 设计](../specs/2026-09-08-push-foundation-w16-activation-design.md)与[完整实施计划 T4](2026-09-08-push-foundation-w16-activation.md#task-4--t4-四类-actor-的共同-fence-与真实-owner-adapter)。当前实际 BusinessEffect 只消费 Generic authority；两种专用 reader 已由 W13/W19 使用，但尚未进入该 worker 的真实重查、业务写入与完成证明链。本计划完成这一具体效果接线，不缩减完整 W16/W15–W21/52 Unit 目标。

## Global Constraints

全部项目操作只在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`。不读取生产 `.env`/DB/凭据，不启动或观察生产 monitor，不调用真实 PAM/provider/LLM/sink/order，不部署、不远端 Git。保留当前 production identity/broker 的拒绝构造。只允许 Test namespace、工作树 `data/test/TEST_CODE*` 临时 authority、临时 business/control DB、本地可控进程与计数测试 sink。所有本地文件编辑用 apply_patch。

单一 Rust writer；主控独占 Cargo/Git/公开文档，代理不得运行 Cargo/Git 或创建子代理。测试前冻结源码、记录摘要；唯一 Cargo 队列按实际 session ID 等待，未终止不启动另一条。只运行已核对隔离的定向测试，不运行真实启动或全仓未审计 suite。

不得改冻结 SQL、已有 Generic effect/result/completion-proof 字节及 domain；不得以 cached dedicated record 代替每次真实 authority 重读。不从执行客户端接收任意路径、source、callback 或“已认证”布尔值。这个 task 不交付专用发送、具体 Unit cursor、整日 N02 gate、全局 startup barrier 或生产身份。

## Task 1 — 接通两类专用 authority 的完整业务恢复效果

### 授权、ownership 与依赖

本任务在固定隔离树实施，继承以下硬约束：不读取真实 `.env`/数据/凭据，不启动或观察 monitor，不进行网络/PAM/真实消息/交易/部署；全部 fixture 为 Test namespace 和临时目录。单一 Rust writer 使用 apply_patch，主控运行唯一 Cargo 队列、Git 和文档；实现代理不得启动子代理、Cargo 或 Git。源码冻结时不得编辑 Rust。
既有相邻回归的限定例外：`dedicated_transport_tests.rs` 的8项 W13 测试使用临时业务库、合成终态和 fake source，其中 `Namespace::Production` 只是夹具枚举值。主控已完整核对该文件：没有生产路径/构造器、真实来源、网络或进程调用；允许原样运行这8项兼容测试，不把它们记为生产认证或本任务真实 Test authority 的替代证据。新增专用测试仍严格使用 Test namespace。

仅允许修改这些已有文件：

- `src/push_foundation/activation_business_effect.rs`：将真实 terminal 选择/重查扩展为闭集 Generic/P01/N02，保持单 worker 及已有实际恢复算法。
- `src/push_foundation/activation_fence.rs`：需要的专用测试装配，不开放 production 成功构造，不改变现有 operation/lifetime 协议。
- `src/push_foundation/dedicated_transport.rs`：仅与专用 route/descriptor/查询映射复用直接相关的受限 interface。
- `src/event/dispatcher.rs`：增加只读、由已固定 capability 派生的资源绑定投影，复用现有目录链验证，不另写保护根算法或改变追加/锁协议。
- `src/event/mod.rs`：仅复用现有 N02 窗口解析主体为私有 helper，增加消费固定年份资源绑定的 crate-private 查询入口；原有查询语义、解析验证和追加协议不变。绑定必须在实际锁/打开 FD/读取的同一临界区核对，不能仅在查询前后另做投影，防止读取期间换入再换回文件。
- `src/push_foundation/mod.rs`、`activation_generic_process_tests.rs`、`activation_business_effect_tests.rs`、`activation_business_process_tests.rs`：必要的模块/闭集 child-role 注册和受影响回归；不改既有期望值使测试失去意义。实际 BusinessBroker role 位于 generic process harness，非早期 fence process harness。

按需要新建 `src/push_foundation/activation_business_source.rs`（内部闭集来源与绑定）、`activation_dedicated_business_effect_tests.rs`（挂在 business_effect 的 `dedicated_tests`）、`activation_dedicated_business_process_tests.rs`（真实进程测试）。不编辑 W19/SLA 生产模块；其 fixture 仅作样板，不能复制整套已有测试。若确需其他文件，向主控说明具体调用依赖后再接管。

### 当前可复用事实

1. P01 coordinator 的 `activation_storage_binding()` 已在实际连接校验中返回路径/dev/inode/environment/owner-instance；源读取 `inspect_p01_dedicated_terminal` 保留同日 once claim、Scheduled/Compensation 和原始 disposition。实例 owner 字符串不是文件 owner 身份。
2. N02 AuditDispatcher 已持有完整目录 FD 链，`read_authoritative_year` 使用实际年份 lock 与 JSONL 全链验证；没有可供 effect 固定的资源绑定投影。新增投影必须从其已存在 capability 派生并重验；不在外层对 caller 路径做 metadata 后当权威，不调用会创建文件的 preflight 来补冷读条件。
3. W13 的 `inspect_p01_dedicated` / `inspect_n02_dedicated` 已进行精确 business/terminal 映射。`FixedDedicatedAuthority` 是缓存记录，不能用作 finalizer 的第二次权威重查。
4. W19 `finalization_sla_tests.rs::Case` 是真实临时 authority 样板；其 N02 `write_n02` 写死 Accepted，传 uncertain sink 参数不会产生 N02 Uncertain。新测试必须显式构造真实 Uncertain stage 并走 exact append。
5. N02 已有隔离 occurrence 约定是 `news-flash-window` + 可解析窗口 key；未知约定拒绝。这不是 production 注册，不能给未知输入默认 H0930。

### 实现合同

1. 保留现有 Generic 路径及其 effect canonical bytes/hash、result schema 和 completion proof 原义。新增专用效果使用独立 `ActivationDedicatedBusinessRecoveryEffect/v1` domain，纳入完整 scope、业务 snapshot/config/policy、真实 source 资源绑定、authority class/schema、精确模板/required channel；N02 还包含从持久 snapshot 验证的 window。不能只绑定 route 文本或复用旧 Generic 同名 domain 塞新字段。
2. 使用私有闭集来源，不接受任意 verifier/source trait 作为运行时可选权限。P01 持有真实 coordinator；N02 持有真实 AuditDispatcher。可以增加仅 cfg(test) 的专用 fixture 装配；生产构造仍拒绝，Test 不能通过配置变成 Production。
3. N02 只读资源投影返回从实际固定根派生的 namespace、根对象 identity 与必要路径/元数据；重查复用 `PinnedAuditRoot::validate_complete_chain`。错误脱敏，不泄露路径/批准主体。投影是资源观察而非 W16 生产来源认证。namespace 必须与 business snapshot/scope 相等，不能借 dedicated mapper 从 intent 填 namespace 掩盖跨根读取。
4. 对专用 class、正式 Unit、全局 subject、模板、policy owner/允许 authority、channel 和 N02 occurrence/window 在业务恢复写入前校验。source 资源/绑定发生变化时拒绝，不重绑定新对象。未知来源/缺 authority/未封口不能被升级 Accepted。
5. 每次 W09 qualification / finalizer 提交前的 authority 查询都经过当前 worker 的 `authorize_business` 与实际资源重验，再调用真实 dedicated reader。不能在第一遍读到 Accepted 后缓存复用。source read 失败走现有错误/隔离语义，不补 sealed audit、不发送、不扩大到另一 intent/window/业务日。
6. 复用现有 `reconcile_current`、业务 finalizer 和精确持久确认：同 worker 许可覆盖 lease、准备、提交/错误记录；最终重新打开业务库核对状态、版本、lease、完整 transition chain，产生已有 BusinessRecoveryResult/完成证明。新效果 digest 精确绑定其来源，使 request/source/route 替换不能复用旧 operation。operation Replay/Unresolved 的既有语义保持，不将后续恢复成功回写为旧 operation 成功。
7. Accepted 可完成这一 Foundation intent；Rejected、Uncertain、Missing、PendingSeal 保持现有拒绝/人工/等待边界；失败不调用 sink/append、不改变源 authority 字节或其他 intent 的链。Foundation Completed 不自动执行 P01 once claim 或 N02 settle/cursor。

### 测试与可观察验收

采用纵向 TDD：先提供一条经实际 broker/真实临时 dedicated authority 的缺功能测试，交主控运行 RED，再实现并运行相同测试 GREEN；如 RED 因新 interface 尚不存在而编译失败，应如实记录，不写成生产缺陷已运行复现。随后补齐两种 authority 和失败矩阵，不把同一缓存 record 调两次当真实 reader 集成。

- P01 / N02 Accepted：真实持久 source → broker worker → Foundation Completed → 新连接 inspect 全链/结果/证明；重复相同 operation 精确 replay，源 authority 与发送计数不变。
- 两类 PendingSeal、Missing、Uncertain；P01 Rejected；N02 真实 Uncertain stage。N02 未封口、缺 lock、错 window/未知 family/key 均不得写出 Completed。保留现有 manual disposition，不造新的权限。
- 同文案但不同 namespace/source root/inode、Unit、模板/channel/policy、window、snapshot、request scope/actor/class/generation/owner 的反例；拒绝发生在越范围业务写入之前。其他 intent/其他窗口持久数据完全不变。
- deterministic 真实资源替换：第一次查询后、第二次查询前更换 P01 DB 或 N02 根/leaf，确认没有按旧 Accepted 提交。只能改自己创建的 temp fixture；不改底层防护使测试通过。
- N02 同锁读取身份：实际打开的年份 lock/JSONL 对象必须匹配 effect 已绑定的对象；读取期间换入再换回的反例不能仅靠查询前后两次投影宣称覆盖。不得消费替换对象的终态形成完成证明。
- 真实进程：两类 authority 至少各有 broker/client 进程通过实际 Unix wire 的正常完成/replay，并覆盖 requester 结束后 worker 许可仍持续、quiesce 等待/拒绝直到完成；至少一类在 qualification 后 broker 死亡，重开不得凭 staged/result/EOF 造成功或重发。复用现有 child-role 协议，不运行实际 monitor。
- 独立 literal canonical 期望证明专用 effect 完整字段与 domain，实际 encoder 字段变体改变 digest；原 Generic golden 保持不变。

预期定向命令（新增模块名称须与实际注册匹配，测试数必须大于零）：

```bash
cargo test --offline --lib push_foundation::activation_business_effect::dedicated_tests -- --test-threads=1
cargo test --offline --lib push_foundation::activation_dedicated_business_process_tests -- --test-threads=1
cargo test --offline --lib push_foundation::activation_business_effect::tests -- --test-threads=1
cargo test --offline --lib push_foundation::activation_business_process_tests -- --test-threads=1
cargo test --offline --lib push_foundation::dedicated_transport_tests -- --test-threads=1
```

相邻 W19/dispatcher 只运行主控逐项核对为 Test fixture 的精确测试；不整体执行 event/monitor suite。对实际改动路径运行 `rustfmt --edition 2021 --check --config skip_children=true` 与 `git diff --check`。唯一目标 Clippy 使用 `cargo clippy --offline --lib --message-format=json`，比较本次前后实际诊断；不加 `--tests` 或清理旧告警。主控保存源码前后摘要/命令/实际终态；静态检查不替代上述行为/进程证据。

实现代理完成后自审，写本计划私有 workspace 的 task-1-report.md；主控提交源码，再安排一次独立 Spec/Quality task review，修复只作限定复审。未通过不标 Task complete。该完整目标尚未到全分支验收/部署阶段，不清理其他计划、不反复全分支 review。

### 文档与完成边界

主控在 docs/push-system 新增本任务实施记录，更新 W16 结果与 README，列真实通过/未运行项目、production roots/专用发送/具体 cursor/完整四 actor/52 Unit 的剩余边界。本任务源代码与最终定向行为、进程验证、独立审查都通过后才关闭本 Task；不能将仅 source query wrapper 或仅测试装配称为完整 W16 或生产接管。
