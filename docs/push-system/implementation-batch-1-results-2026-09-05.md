# 推送可靠性首批开发结果（2026-09-05）

当前状态：两项代码修复均完成实现、任务级独立复核和整批定向验证，最终源码提交2f07ac2；整批代码审查待完成。不是完整方案或生产验收完成声明。开发区样本保全曾失败，补救披露通过不等于事故被逆转。

## 开发边界

- 用户已授权从a673043建立独立开发区，分支 `codex/push-reliability-20260905`；未合并master、未push、未部署。
- 原master的160个未合并索引项与用户源代码保持原样。未复制或修改生产数据库、.env、密钥，不发真实消息、不调用真实provider/LLM。
- 沿用原分析的四时段规划与108项决策；本批只处理R-08错误语义和告警/G5b测试污染，不切换物理投递owner。
- 本计划与结果在独立分支的 `.gitignore` 中获得精确例外；未整体纳管其他ignored文档或运行目录。

## 1. R-08：必需CFFEX错误不再被统一转换成可重试

对应真实样本：原报告五日R-08 dispatcher失败259次，其中256次为invalid_evidence；review却有102条统一retryable=true。数字属于冻结历史快照，不是本批修复后的线上结果。

改动：

1. `ReviewGatewayFailure` 保存capability、provider、audit_outcome、reason_code、retryable及诊断文字。业务结果与审计transition复用同一结构。
2. CFFEX必需批次失败在字符串转换前返回类型化结果；调度使用Gateway原retryable，机器分类使用稳定reason_code，不再被诊断里的http/request误导。
3. 永久错误在当前调度实例中终止重试；临时错误保持原退避。终态preflight、verified-empty、可选组件降级、成功模板和原physical owner不变。

源代码：[review_batch.rs](../../src/bin/monitor/review_batch.rs) 的 `ReviewGatewayFailure` / `ReviewTaskOutcome::gateway_failed` / `ReviewScheduleState::apply_for_run`；[push_templates.rs](../../src/bin/monitor/push_templates.rs) 的 `dispatch_r08_event_calendar_outcome_with_loader`。

提交：`977cde1`（实现）、`e4de707`（补齐一分钟调度边界回归）。

验证：新增永久错误回归先失败，原因是19:00应用永久失败后23:00仍due；最小修复后通过。临时错误测试验证next_attempt=19:01、19:00:59.999不due、19:01 due。R08过滤29通过；review_batch过滤54通过，1项为由父测试显式启动的辅助进程入口而被忽略。独立审查发现缺少临时错误调度断言，补齐后复核通过。

限制：失败终止只覆盖当前内存调度实例，尚未实现跨重启source-failure封锁。新代码可读旧existing_source_failure记录，但旧二进制不能解析新gateway_source标签；生产发布前需要具备兼容能力的回退制品，不能直接回退a673043后读取新审计。

## 2. 告警/G5b：基线复现与实现目标

基线5项alert_log旧测试全部通过，却在独立开发区写出默认生产相对路径 `reports/alerts/20260905.jsonl`（326字节，含TEST_CODE_000001）和md（278字节）。这证明“测试绿灯”并不等于“测试没有污染运行输入”。对应原08-31实际G5b污染样本，未删除任何原文件。

已实现b31a0cf：显式测试归档及记录origin；生产读取、G5b选取/模型调用/落盘前的准入检查；复用已有测试进程识别，覆盖library不带cfg(test)的默认I/O保护；正常旧记录兼容读取。测试构造要求可规范化的已存在目录，拒绝默认生产子树及`..`/符号链接别名。只增加tempfile dev依赖，Cargo.lock仅增加根包依赖项。

独立审查要求的补测已实现：在每例临时目录内把预期JSONL/Markdown文件路径预建为目录，通过公开AlertLog::append_jsonl/append_md/append_batch验证I/O错误传播；混合读取样本新增未知origin，断言最终仍只接收正常legacy与Production记录。修订提交2f07ac2，alert 9/9、G5b 17/17通过，Unix symlink测试分支增加条件编译；修订复核通过，无新增Critical/Important问题。

兼容边界：缺少origin的正常旧记录标为LegacyUnknown并保持可用，不能据此宣称已经验证其生产来源。本批隔离显式Test及已证实的TEST_CODE污染，不自动给全部历史档案补造生产证明。

测试证据强度：TEST_CODE被错误选中的新增回归有真实行为RED（0通过/1失败）再GREEN；构造器/模型拒绝接口在开发过程也出现过缺少API/variant的编译失败，那些只算编译反馈，不冒充行为缺陷复现。新增公开I/O与未知origin用例是审查阶段补齐的回归覆盖。

## 源码证据索引

以下行号以隔离分支的源码提交为准，不适用于原目录含R-07等改动的混合工作树。表中生产代码在b31a0cf已固定，后续审查补丁仅追加测试；同时给出符号，避免后续行号漂移导致误认。

| 行为 | 源文件与起始行 | 可核实的代码依据 |
| --- | --- | --- |
| 必需CFFEX保留类型 | `src/bin/monitor/push_templates.rs:11080` | loader后检查Err，gateway_failed返回早于map_err字符串化；终态preflight在11055附近先执行 |
| 错误快照 | `src/bin/monitor/review_batch.rs:826` | ReviewGatewayFailure保存六字段，from_gateway_error逐项读取原错误 |
| 调度重试来源 | `src/bin/monitor/review_batch.rs:864` | ReviewTaskFailure::retryable直接读gateway snapshot，不对reason做文字匹配 |
| 审计稳定分类 | `src/bin/monitor/review_batch.rs:1324` | gateway能力及reason_code组成分类，transition保留完整快照 |
| 显式临时归档 | `src/monitor/alert_log.rs:44` | AlertLog::for_test规范化路径并拒绝production子树，origin=Test |
| 默认I/O隔离 | `src/monitor/alert_log.rs:77` | ensure_io_allowed同时检查测试进程与TradingEnv，先于创建目录/打开文件 |
| 历史输入过滤 | `src/monitor/alert_log.rs:158` | read_today_records在生产实例跳过不合格记录及反序列化错误，并warning |
| 来源兼容/统一准入 | `src/monitor/alert_log.rs:237` | serde默认LegacyUnknown；is_production_eligible拒绝Test及TEST_CODE前缀 |
| 模型调用前拒绝 | `src/monitor/attribution_deep.rs:123` | assess先返回IneligibleRecord，provider尚未调用 |
| 选取与输出拒绝 | `src/monitor/attribution_deep.rs:275` | top_events_for_deep先过滤再排序限额；append_deep_attribution_row:301先准入及运行环境检查再建目录 |

默认Markdown读取/统计仍保留既有返回类型，不提供历史文本的来源认证；本批G5b入口使用的是结构化JSONL过滤。JSONL/Markdown双写不是原子事务，公开I/O错误会返回，但失败前已成功的另一份写入不自动撤销。本批没有把归档改造成事务outbox。

## 3. 可复现构建与基线

编译需要原项目ignored输入 `client-bundle/market.proto`，10424字节，SHA-256 `8730bce3c20e170cf8f58047336ae06d3a5e9080d81568dee71e7b0882063332`。本工作区只补齐此合同且保持原字节，没有下载或替换provider实现。这是复现前提，不声称任意新checkout无需额外输入。

本机验证工具链：rustc 1.95.0（59807616e）、cargo 1.95.0（f2d3ce0bd）、libprotoc 35.0。

实际命令在独立worktree执行：

```bash
cargo build --offline --bin monitor
cargo test --offline --profile dev --bin monitor r08
cargo test --offline --profile dev --bin monitor review_batch::tests
cargo test --offline --profile dev --lib monitor::alert_log::tests
cargo test --offline --profile dev --lib monitor::attribution_deep::tests
cargo test --offline --profile dev --lib risk::env_guard::tests
cargo test --offline --profile dev --lib monitor::alert::tests
```

首次基线构建exit0，6m15s；旧R08 28/28、alert_log 5/5、G5b13/13通过。本批筛选测试使用dev profile复用依赖，不改默认并行。已有lib84项、lib test43项dead_code warnings；全仓cargo fmt检查存在无关既有差异，未整体格式化。完整默认并行全项目测试尚未执行，不以筛选测试冒充发布门禁。

最终对源码2f07ac2由controller重新执行以上命令，采用`set -e`及`set -o pipefail`保留管道真实退出状态，整组exit0：

| 命令/过滤器 | 结果 |
| --- | --- |
| monitor离线构建 | exit0，2m06s，仍有84项既有warnings |
| bin monitor：r08 | 29通过、0失败 |
| bin monitor：review_batch::tests | 54通过、0失败，1个子进程辅助入口忽略，父测试实际启动验证 |
| lib：monitor::alert_log::tests | 9通过、0失败 |
| lib：monitor::attribution_deep::tests | 17通过、0失败 |
| lib：risk::env_guard::tests | 7通过、0失败 |
| lib：monitor::alert::tests | 7通过、0失败 |

过滤集合存在重叠，不将上述计数相加当唯一用例总数。最后两组是既有环境隔离和告警格式的纯函数/单元邻接回归，未扩大为全仓测试。

四个改动Rust文件的`rustfmt --edition 2021 --check`、`git diff --check a673043..HEAD`及工作区diff检查exit0。本批两份文档的2条相对链接、10条源码行号存在/边界检查通过；这只是定位检查，语义证据由源码与独立review核实。原目录冻结分析校验器`ruby docs/push-system/verify-reanalysis.rb`通过166项；它仍对应原混合源快照，不与隔离分支构建证据混算。

最终整组命令执行前后，当前非原始开发参考件JSONL SHA保持`7b065405571d49bb898ec997cf7fe92d43e68c5dbbfe6b3d4f9de0b32aee5470`、MD SHA保持`69265ebc6172a80cbf47cb2beade4420f10186e72915da32d170386dc2427519`。此结果仅证明这轮修复后测试没有继续改写，不能补回事故前的原始保真。

## 4. 未提交改动如何处理

原目录的alert_log、attribution_deep、review_batch文本与起点提交相同；push_templates本地173行差异属于R-07收盘价/名称逻辑，与本批无依赖，暂未移入。PaperBuy、Watchdog及其他未提交业务修改须在所属单元审计后选择性移入，不整体复制混合工作树或选择冲突一侧。

原目录src/tests/Cargo相对HEAD差异SHA-256为 `3b0f746129bcdd108680af75da354fe087c00f300416483fdcab01705b48064e`，阶段复核未变；unmerged仍160。历史alerts/20260831.jsonl SHA为 `6639c0eb6bc236970952881f8753537840c65d6841b5556157c0d584c618f5c9`，g5b/2026-08-31.jsonl SHA为 `cd5f702d1947fa1fc86af39aae22694e1867b3121068e0b8cb649fad4e6c0914`，阶段复核未变。

## 5. 后续仍需要完成

| 后续工作 | 本批不能代替的门禁 |
| --- | --- |
| 完整Foundation | 单应用合同、业务intent/outbox、VerifiedTerminalRef finalizer、CAS/lease、activation manifest及恢复演练 |
| Paper成交通知恢复 | 失败/崩溃后不重新成交，不丢通知；历史回填另行决策 |
| Attribution/G5b completion | 已分析/已归档不等于已接受；复用结果，只恢复通知 |
| Watchdog | 独立检查、不被慢review阻塞、注册期望、区分进度/尝试/Accepted，失败不永久fired |
| NewsAI | 跨批新闻身份与有效修订版本化、保持五年证据，不按卡数简单合并 |
| 文档与发布治理 | 精确Unit目录、正式RFC/WBS、全项目并行门禁、兼容回退制品及受控真实Accepted证据 |

全量工程/交易周期仍不能基于本批两项修复重新承诺。Codex承担开发、测试与取证；用户/指定操作员仍负责生产owner晋级及人工Uncertain处置。本批未发生这些生产操作。

## 本批实施裁决

默认生产I/O保护必须同时覆盖cfg(test)、既有测试进程识别和TradingEnv::Test，不能只保护单元测试；否则正常编译的library仍可能从集成测试接触生产默认归档。如果进程识别发生误判，会阻止该进程默认归档读写，需检查启动方式，而不是放开生产目录权限。显式临时测试归档不受影响。

## 开发区样本操作事故（不隐去）

实施者重复运行旧alert_log基线测试，将本开发区的JSONL从326追加到652字节、MD从278追加到556字节；随后误用恢复操作移除了追加的开发测试行。JSONL回到原hash，但MD最终为277字节、SHA-256 `69265ebc6172a80cbf47cb2beade4420f10186e72915da32d170386dc2427519`，不是最初的278字节快照。这里的对象是本次创建的独立开发区文件，不是原目录08-31生产历史；后者经独立哈希复核未变。

裁决：不再恢复、删除这两份开发文件，保留为**非原始参考件**；后续只对当前文件验证“修复后测试运行前后不再改写”。理由是避免继续破坏操作证据；代价是初始开发区快照已不完整保真，不能把整个开发过程标成样本保真通过。这项执行偏差不能用代码测试通过来抵销。

另外，一次格式化命令连带产生了7个任务外Rust文件差异，已经用精确补丁还原，未进入源码提交。原目录用户改动未被处理。
