# 真实通知入口逐目标结果保留

日期：2026-09-11。初始源码：`6ddbf51`；隔离分支：`codex/push-reliability-20260905`。状态：本计划 Task 1 完成，初版 `bd143fe`、最终修复 `de38876`；修后16项定向测试通过、Clippy无新增诊断、限定复审Approved。完整推送目标及后续盘后接线仍未完成。详见[实施记录](../../push-system/implementation-notification-attempt-observation-2026-09-11.md)。

## 目标与依据

整体目标仍是完整推送方案交付。本任务补上盘后产业链安全恢复所需的真实输入：旧 `NotificationService::send` 把每个渠道（包括每个 Custom URL）的结果压成一个 bool，调用者无法区分无渠道、部分成功和有尝试但结果不明。

依据：[RFC DeliveryResult 与业务完成合同](../../push-system/push-system-implementation-rfc.md#类型deliveryresultproposed)、[盘后产业链当前缺口](../../push-system/chain-post-close-call-chain-2026-09-10.md)、[当前架构的弱/强证据隔离](../../architecture/current/Project_Architecture_Blueprint.md)。当前真实 `WeakOutcomeKind` 已定义 Accepted/Rejected/Unknown；不能再发明一个可冒充强回执的成功类型。

本任务不是以新 helper 代替真实调用：旧 `send` 必须委托新的逐目标发送入口，保留完全相同的路由、发送顺序、分片和任一渠道成功的 bool 投影。新入口产生的是本次本地观察，尚未绑定 Foundation intent/Unit/occurrence，也没有跨重启的稳定目标身份。

## 设计与依赖

1. 本任务：真实发送保留逐目标弱结果；旧 caller 原语义兼容。所有 false/Err 保守记为 Unknown，因为微信/飞书可能已有成功分片，HTTP 超时也不能证明未接收。
2. 后续盘后任务：抽取并固定业务日、采集/模型结果、报告字节和保存进度；实际 timer 消费通知结果，分析成功不得成为通知完成。不得简单 false→Err 导致下一 tick 整包重发。
3. 完整 Unit 接管：按原 RFC 接可信 occurrence/owner/fence、不可变 intent、既有持久存储、强终态及独立通知 cursor；已准备及未决事实跨重启恢复、窗口/交易日和有界补偿分别验收。不得新增内存日期位或临时旁路数据库后宣称完成；弱观察不能打开生产 durable 或推进强完成。

第2/3项仍在完整目标中，本任务不关闭盘后缺陷、W16、W17 或任何完整 Unit 迁移。实际必达渠道及生产认证保持原审批边界。

## Global Constraints

- 只修改隔离工作树，不读取真实 `.env`/生产数据库，不运行或观察生产 monitor，不调用真实 provider/LLM/sink/PAM/交易，不部署或远端 Git/CI。
- 测试只用显式构造配置、无代理的本地 HTTP 客户端、随机 loopback 端口、合成 TEST_CODE 内容和自有临时目录。需要沙箱外 loopback 权限时由主控申请；未获准不绕过。
- 一名 Rust 写入者；主控负责 Cargo、Git 和公开文档。子代理不得自行启动 Cargo、Git 写入、生产命令或子代理。源码冻结后主控运行单一 Cargo 队列。
- 保留十类渠道及 Custom 每 URL 发送一次的既有顺序。`send(&str) -> Result<bool>` 继续以至少一个渠道方法返回 true 为 true；不因报告生成再次发送，不改 `send_with_image`。
- false/Err 不能变成确定未投递或自动重试资格；全部失败仍可能包含成功分片。只保留 Accepted/Unknown 本地观察，不合成 Rejected、VerifiedTerminalRef 或 TransportAccepted。
- 结果及其 Debug 不携带 webhook URL、token、SMTP 凭证、报告正文或原始错误正文。目标位置仅为本次调用内的序号，不宣称跨配置/跨重启身份。
- 不改冻结 RFC/目录/蓝图、生产策略、DB schema 或依赖。仅定向格式检查，不处理全仓旧差异和告警。

## Task 1: 将逐目标弱结果接入真实 NotificationService::send

### 工作目录、要求及所有权

在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905` 工作。先读本任务全文，不读其他任务的私有工作目录。

实施者只可编辑：

- `src/notification/service.rs`：实际 `send` 和新增 `send_report`，或将报告相关实现放入下列新模块后在此委托。
- `src/notification/send_report.rs`（新）：小接口的本次逐目标观察对象及必要发送实现，不造独立重试器。
- `src/notification/send_report_tests.rs`（新）：通过真实发送入口测试。
- `src/notification/mod.rs`：新模块及所需重导出。
- `src/notification/config.rs`：仅在报告类型需要时给 NotificationChannel 添加无行为变化的 Copy/Clone/Debug/Eq/PartialEq 派生；不得改检测/配置规则。
- 本任务 `task-1-report.md`。

主控负责所有 Cargo/Git/公开文档与 ledger。实施者不得运行 Cargo/Git 写入或子代理；自检源码与定向 rustfmt 可自行执行。任何额外 Rust 路径先报告理由。

### 行为与接口合同

1. 提供 `NotificationService::send_report(&self, content: &str)`，返回由本次真实渠道执行构造的只读报告。报告拥有每次调用内的目标列表/结果，可检查是否确实未调用任何渠道及是否至少一个弱成功；旧 `send` 仅调用它一次并按任一弱成功投影 bool。
2. 使用已有 `crate::monitor::push_job::WeakOutcomeKind` 保留结果语义，不修改该类型。每个实际调用目标有 channel、调用内目标序号、结果；Custom 两个 URL 必须保留两个观察，即使 URL 相同也不得去重既有发送。目标序号不含 URL，不是 durable identity。
3. 每个渠道方法的 `Ok(true)` 记 Accepted；`Ok(false)` 和 `Err` 均为 Unknown。不提前停止剩余渠道，不自动重试，不把 Unknown 升格为 Rejected。无可执行目标时报告为空且 has_success=false。公开 available_channels 的旧行为保持；空 Custom 配置不制造实际调用。
4. 不向报告复制正文、URL/凭证、原始错误字符串，不借 report 构造强结果。文档明确 Accepted 仅指现有渠道方法报告的弱成功，并不证明完整原正文（旧分片/截断策略不变）、用户已读、必达集合完成或跨重启幂等。
5. 保留已有各渠道的协议校验和分片顺序；无需重写微信/飞书实现。逐目标报告可表达“其他渠道成功、一个渠道未知”；单一分片渠道内部的已成功分片不可被汇总成确定拒绝。当前不提供逐分片 receipt，也不允许据此恢复重发。
6. 不修改盘前/CLI/timer 的共享 wrapper，不修改通知完成门。本任务构建安全接线的必要输入；报告出来后仍未自动完成盘后修复。

### 垂直 TDD 与可观察验收

先提交给主控一个无渠道真实 `send_report` 测试（接口缺失的编译 RED 可诚实记录为接口 RED，不冒称生产缺陷复现），等待主控运行。实施对应最小实现并冻结交主控 GREEN；其后再按以下行为增加反例并实现，不先写整组测试再填实现。

- 无渠道：空报告、未调用、零弱成功，旧 bool=false；无网络/配置读取。
- 两个 Custom URL，一成功一明确 false：两个实际 HTTP 请求、两条不同目标位置观察、Accepted/Unknown、has_success=true。协议 false 也不得宣称强 Rejected。
- 所有目标 false，另覆盖非法协议正文/连接中断等 Err：保留每个 Unknown，不提前退出、不重试、不返回成功。
- 单渠道全部成功及混合内置渠道（微信/飞书/钉钉/Slack/Discord/Custom）：保留实际目标顺序和各自协议成功，返回弱成功。
- 微信至少两片，第一片成功而后一片失败；另覆盖第一片成功后响应不可解析的 Err：真实本地请求证明之前已发送，报告 Unknown，不能成为未尝试或确定拒绝。内容与 max_bytes 使用明确安全值，避免旧 max_bytes<200 下溢路径。
- 飞书通过公开send_report覆盖card失败→text fallback的失败/成功；普通带标题/分隔符的长报告验证现有单截断片特征。另外以末尾空标题 `TEST_CODE ` + 600个A + `\n### \n`、feishu_max_bytes=512 检查公开多片路径：若空标题因正则要求非空正文而保留，应有两片，首片card成功、后片card/text均失败时3请求且Unknown。必须以真实请求证明，不调用私有helper冒充公开路径；若实际前提失败，报告原始结果再裁决。修正飞书普通长报告完整原文分片属于后续独立业务问题，不在此改原算法。
- 同一组本地响应脚本分别通过 `send_report` 和旧 `send`，独立断言 bool 真值矩阵及请求次数；不是仅测试手工构造报告的 getter。
- Custom 相同 URL 配置两次仍执行两次并有两条结果；available_channels 中 Custom 却没有 URL 不生成伪尝试。
- 报告 Debug 只含渠道/序号/弱结果，不含测试 token、URL、报告正文或原始响应/错误中的敏感标记。

本地 HTTP fixture 必须有有界 accept/read/服务总寿命与可等待的实际 thread/task handle；读完 Content-Length 指定的请求正文，使用无代理 client，不因测试失败永久阻塞 join，不使用固定端口或 sleep 轮询生产。测试断言通过本任务接口及边界 HTTP 观察，不 mock 内部报告收集器，不从被测代码重算期望。

主控基线：`cargo test --offline --lib notification::service::tests::br111_ -- --test-threads=1`（5项纯协议测试，先审计）。新测试过滤器以实际声明为准，计划使用 `notification::send_report_tests::`。最终只合批新组及已审计的原 service 相关测试；原无断言 `test_generate_report` 不作为验收。再定向 rustfmt、`git diff --check` 与 `cargo clippy --offline --lib --message-format=json` 对比同配置基线。不得运行未审计全仓 suite。

### 报告、审查与回退

报告写在 `.superpowers/sdd/2026-09-11-notification-attempt-observation/task-1-report.md`，列实际变动、每轮 RED/GREEN 命令/退出码/断言、测试安全范围、未覆盖项及自审。未运行的命令不得写通过。

主控固定源码提交后提供 BASE..SOURCE 精确差异，派新独立代理审查规格与质量；重要发现回原实施者修复，Minor 保留到整体审查。整体目标未完成时不触发重复整分支收尾或删除私有证据。回退只撤回本任务局部源码提交，无 DB/配置/线上状态需要回滚；远端或生产操作仍需另行权限。
