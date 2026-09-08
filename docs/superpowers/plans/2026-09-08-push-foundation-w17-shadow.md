# W17 共用事实影子执行与精确差异内核

日期：2026-09-08。状态：实施中；不是 W17 全量或生产迁移完成声明。

## 目标与依据

落实 `docs/push-system/push-system-implementation-rfc.md` 的「影子精确比较」「影子副作用」（1110–1138 行）及 WBS `W17-contract`。复用 W04 的 `PreparedFactsSnapshot::shares_instance_with` 和 W05 的实际 `DecisionProjector`、`JobDecision`、`PreparedPush`，不重新定义一套字符串决策。

W15/W16 的真实来源认证仍缺生产身份/受保护根配置。2026-09-08 的实际调用映射确认 BR159 采集后写审计并丢弃回执，生产调用没有可信 namespace、业务日和部署绑定；不能通过给旧行补描述符绕过。这里提前执行不依赖该外部配置的 W17 内核，完整 W15/W16 和 W17 的激活/真实业务端口接线继续保留。

## Global Constraints

- 仅修改隔离工作区 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`，分支 `codex/push-reliability-20260905`；根工作树、生产 monitor、真实数据库、provider/sink/PAM/订单、`.env`、owner、部署权限不操作。
- 不修改八份 RFC 输入、目录/WBS/蓝图、既有规范化 v1 字节或冻结 25 对象 SQL；不增加依赖、生产配置、自动晋级或第二套状态机。
- 影子结果仅是本次执行的比较证据，不是认证、Production Ready、physical-owner 许可、业务完成或全进程副作用证明。没有调用适配本接口的旧路径不能获得零副作用证明。
- 父代理独占 Cargo/Git、门面注册和中文文档；实现代理不得运行 Cargo/Git、启动其他代理或触碰生产。只用 apply_patch 编辑，定向 rustfmt，禁止全仓 cargo fmt。

## Task 1: 共用输入、精确 typed diff 与计数拒绝端口

### 完成条件

交付可实际执行两条纯投影路径的内核，不只是接受两份调用方自报的 hash。唯一输入是一次捕获的 `&RunContext` 和 `&PreparedFactsSnapshot`；两条路径都使用这份 context 和共享 Arc facts，包括模型输出。返回结构化差异与本次两条路径的端口计数；报告不能由调用方直接构造为通过。完整生产业务适配、W16 激活门禁及逐 Unit migration 不在此内核任务中，不能因此关闭 W17。

### 文件与接口

- 新增 `src/monitor/push_job/shadow.rs`：执行、比较、私有报告构造及拒绝能力。实现保持单一职责；必要时将实际拒绝端口实现放入新 `shadow_effects.rs`，不得为尚未出现的扩展再分层。
- 新增 `src/monitor/push_job/shadow_tests.rs`：通过实际执行接口测试。允许复用现有 `context::capture_fixture` / `projection::projector_fixture` 等 cfg(test) 构造器；不修改其业务合同。
- 父代理在 `src/monitor/push_job.rs` 注册模块及最小必要导出；具体导出名由实现代理在报告给出。优先小接口隐藏完整比较规则；不暴露可清零计数器或伪造通过报告的构造器。
- 可只读参考 `facts.rs:568–587`、`projection.rs:264–427,713–821,879–1110`、`policy.rs:781–827` 和现有 W04/W05 测试。不得修改既有领域字节格式、扩大 JobDecision 构造权限或添加测试专用生产后门。

### 行为

1. 先验证 facts 的 run-context digest 绑定，错配时不执行任何路径。执行输入不可变，必须把相同 context 实例和共享 facts 实例交给两个受约束的同步纯投影回调/适配器；不调用 capture、provider、LLM、业务查询或墙上时钟。若接口允许观察值带回其输入，必须验证 context 指针/Arc 实例身份，不能仅比较相同字节的另一次采集。
2. 回调使用实际 W05 JobDecision/PreparedPush。为 Ready 验证 context、facts、Unit、occurrence 和 semantic digest 绑定，完整决策按实际 typed `Eq` 比较（包括所有 Ready 字段）；NoData 的 evidence digest 必须绑定本次 facts，不能接受两侧一致但来自其他事实的决策。保留 non-Ready 全部字段，不把不同原因、重试/抑制业务时间视为诊断差异。
3. 比较 SemanticProjection digest（存在性也比较）、Ready rendered SHA 与原始字节、ReasonCode、完整 CompletionDirective（schedule、cursor、retry、manual）。Ready 必须有匹配的 semantic projection；不能允许双方漏传摘要而通过。诊断元数据只允许 `attempt_id`、`latency`、`diagnostic_timestamp` 三项排除；无可配置 ignore list、不 trim 文本、不忽略业务日期/时钟/模板/来源/模型/策略。比较输出采用闭合 typed difference 项，确定性排序且不暴露 payload/渲染文本/模型内容。
4. 单独列出八类拒绝能力：`provider_second_call`、`llm_recompute`、`business_db_write`、`durable_db_write`、`cursor_advance`、`candidate_watchlist_outcome`、`paper_order_fill`、`transport_send`。在执行接口内创建本次计数器，向两条路径提供仅拒绝的能力，不能注入真实网络/数据库/订单实现。请求必须先计数再返回拒绝，绝不执行被请求动作；调用方忽略拒绝并返回相等决策也不能清除失败。计数不可重置、不可由调用方预置，使用不会溢出回零的计数方式。
5. 比较差异映射既有 `ReasonCode::ShadowSemanticDiff`；任何端口尝试映射 `ReasonCode::ShadowSideEffectAttempted`。保留双重失败的结构化信息，不由一次成功覆盖先前失败。回调显式失败不能投影为 Match；报告携带双方状态。未执行路径不能以默认零计数冒充已执行成功。
6. 结构化报告绑定 Unit、context digest、facts digest、本次路径执行状态和八端口计数；通过只表示本内核覆盖的路径语义一致且所有受控端口尝试为零。报告 Debug/Error 不泄漏输入事实、渲染文本、模型内容或任意回调错误字符串。不得称该 Rust 回调接口能沙箱化任意全局 I/O；尚未适配的全局调用属于真实迁移接线门禁。

### 验证

使用实际 W05 projector、一次 PreparationCapture 和真实 JobDecision，不用自报 hash 模拟成功路径。覆盖：

- 一次实际 capture，两个执行路径收到同一 context / Arc facts；模型输出包含在共享事实内；事实/context 错配在 callback 前拒绝；等值但独立采集实例不能冒充同一次观察（若该输入路径存在）。
- Ready 相等、七种 JobDecision 分支的相等/不同分支/字段差异，Ready 文本有意空白变化、语义变化、NoData 证据错配、重试及抑制业务时间变化。
- 完成提案 schedule/cursor/retry/manual 差异均检测；诊断三项独立变化仍相等，业务 captured time 不能排除；projection 缺失/错配、wrong-Unit/context/facts Ready 拒绝。
- 八类端口逐一真实请求拒绝，即使回调吞掉错误仍阻断；混合多次请求精确计数；每次执行独立且不能覆盖上一次报告；显式回调失败和未执行路径不会成为成功。
- 在相同测试接口断言无 provider/LLM 重算、DB/游标/候选/订单/发送动作被执行；测试的是本内核注入的拒绝路径，不冒称生产全局端口已纳管。Debug 脱敏。

父代理在实现冻结后运行一次目标及相邻合批：

```sh
env CARGO_PROFILE_TEST_INCREMENTAL=true cargo test --lib -- --test-threads=1 monitor::push_job::shadow_tests:: monitor::push_job::tests::
cargo clippy --lib --no-deps --message-format=json
```

再做改动文件定向 rustfmt --check、git diff --check 和 RFC 输入验证。已有 lib Clippy 163 条 warning 是基线，要求新改动文件无新增诊断，不宣称全仓零告警。失败由原实现代理修复，最终证据对应修复后的受影响代码。

### 交付报告

实现代理写同目录 `task-1-report.md`，列实现、接口导出名、覆盖测试、改动文件、自审和限制；Cargo/Git 未运行必须如实注明由父代理补入验证记录。冻结文件后报告，父代理测试/提交，再按固定原始 BASE..HEAD 做独立 Spec + Quality 审查。

## 后续仍待

- W16 真实认证/完整共同 fence/监督器及 source/context 接线不变；本内核不替代它们。
- W17 真实 old/new 业务适配与八端口纳管、当前激活证据消费、所有目录适用 Unit 的回放样本；不靠纯内核测试解除生产门禁。
- W18–W21、52 Unit 实际迁移、审批/观察/rollback 证据和全目标完成审计不变。
