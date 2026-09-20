# W17 影子执行内核：实现与验证记录

日期：2026-09-08。当前状态：本计划 Task1 的隔离影子执行内核已完成、验证并独立审查通过；完整 W17 和生产迁移仍未完成。

开发目录：`.worktrees/push-reliability-20260905`，分支 `codex/push-reliability-20260905`。实施计划：[W17 共用事实影子执行与精确差异内核](../superpowers/plans/2026-09-08-push-foundation-w17-shadow.md)。原始基线 `322ad45`，计划提交 `12694f9`，源码 `89128f3`。

## 本轮解决的问题

W04/W05 已有不可变采集与类型化投影，但此前没有统一执行两条影子路径、检查输入实例、比较完整结果并收集拒绝端口计数的内核。本轮新增 `execute_shadow`，为后续各推送单元的旧/新纯投影适配提供同一个检查入口。

该入口不授予发送或激活权限，也不能证明任意 Rust 回调未访问隐藏的全局 I/O。只有真正适配到此接口的路径，才能使用其本次执行计数；生产路径、八类真实端口和 W16 激活门禁仍待接线。

## 行为和证据

| 要求 | 实现位置 | 可观察行为 |
| --- | --- | --- |
| 一次采集、共用上下文和模型事实 | `shadow.rs:279`；既有 `facts.rs:579` | 两条回调收到同一 context 和 Arc facts；输入绑定错误时两条均不执行 |
| 防止等值的另一次采集冒充同一次观察 | `shadow.rs:348` | 观察引用必须通过 context 指针及 facts Arc 实例校验 |
| Ready/NoData 的事实绑定 | `shadow.rs:365` | Ready 核对 Unit、occurrence、context、facts、实际语义投影；NoData 核对 verified-empty 与证据摘要 |
| 精确 typed diff | `shadow.rs:408` | 比较真实 JobDecision、语义投影、原始渲染字节/SHA、原因及完成提案四维；差异确定性排序 |
| 仅排除三项诊断 | `shadow.rs:113` | 仅 attempt_id、latency、diagnostic_timestamp；不排除业务时间、重试/抑制时间或文本空白 |
| 八类副作用全部拒绝 | `shadow.rs:16,93` | provider 重取、LLM 重算、业务库写、durable 库写、游标、候选/观察结果、纸面订单成交、发送；先计数再拒绝，无可注入真实动作 |
| 吞掉拒绝不能通过 | `shadow.rs:258` | 决策即使一致，任何非零尝试也产生 shadow.side_effect_attempted；与语义失败同时保留 |
| 不把未执行或回调失败当成功 | `shadow.rs:181,279` | 明确记录双方状态；只有两侧完成、无差异、全部计数为零才为 Match |
| 证据不泄漏正文 | `shadow.rs:121,165,172,230` | Debug/Error 不输出事实、渲染文本、模型内容或回调任意错误字符串 |

所有 Rust 路径均相对 `src/monitor/push_job/`。`src/monitor/push_job.rs` 仅注册和导出新接口；`context.rs` 仅新增三种 cfg(test) 夹具以验证换 Unit、换 occurrence、换业务时间，原生产构造权限及规范化字节不变。

## 验证过程

- 第一轮 session80591：编译 exit101，测试未运行。局部测试回调的参数被指针比较推断为原始指针，触发 E0308 和两条 E0631。原实现代理调整类型推断顺序，保留全部真实指针/Arc/单采集断言，无 unsafe。
- 修复后 session85503 exit0：**69 项通过、0 失败、0 ignored**，包括 17 项新测试和 52 项既有 push_job 测试。编译 4m17s、执行 2.98s；43 项既有 data_gateway 告警，本批文件无诊断。
- 四个 Rust 文件定向 rustfmt --check 与 git diff --check 已通过。
- RFC 输入验证返回 `rfc_inputs_valid`，八份冻结输入未改。
- 最终 lib Clippy session64768 exit0，2m08s，build-finished success；163 项旧位置告警，`src/monitor/push_job` 下零诊断。
- 固定原始 `322ad45..89128f3` 独立审查：**Spec compliant / Quality Approved，Critical0 / Important0 / Minor1**。Minor 为既有测试告警，保留最终全分支审查处理。审查读取时尚未完成的 Clippy 已由父线终态证据补齐；生产适配等 CannotVerify 项确认仍未完成，没有被本任务关闭。

详细执行记录在本计划对应 `.superpowers/sdd/2026-09-08-push-foundation-w17-shadow/`，包括 brief、report、validation 和 ledger。

## 顺序调整和仍未完成项

真实采集调用映射已经确认：`monitor/main.rs:3834` 静态诊断及 `:3902` live 路径没有 W15/W16 认证采集上下文；BR159 在采集后写审计，`grpc_source.rs:1483,1658,1710` 丢弃回执。生产身份根和部署授权配置缺失时，不通过补历史描述符伪造来源认证。因此先推进这个不依赖外部认证配置、但迁移必需的 W17 内核。

完整 W15/W16 认证、监督器、共同 fence、真实来源与查询/启动接线仍未完成。完整 W17 还需要旧/新业务适配、真实八端口纳管、激活证据消费和逐单元回放样本。W18–W21、52 Unit 实际迁移和发布验收未完成；本记录不改变原总范围。

本轮没有观察、启动或替换生产 monitor，也没有操作真实数据库、provider、sink、PAM、订单或 owner。
