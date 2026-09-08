# P-02 冻结横幅与完整业务提案

日期：2026-09-08。状态：实施中，最终合批验证与独立审查待完成。原始BASE `0d84d850ab2c8daff441da6f9bf8e6cf4d091280`。

## 目标与依据

推进完整W17真实业务适配的必要前置：当前生产P-02准备消息时只固定选股列表，却在渲染内部读取账户库/全局说明，入池提案与通知code集合在发送后重新派生。把同次选中快照和冻结横幅变成一个完整、可比较的纯业务结果，让实际dispatcher消费它；不只添加测试专用renderer，也不把同一函数跑两次称为新旧逻辑比较。

规范：RFC通用晋级门禁要求共享context/facts、精确业务语义、零禁止效果；WBS `MU-auction-volume-owner`要求消息/入池/通知集同批、有限正价格/量比、失败不推进。原同批修复0dda4cc已经交付，本任务不重做它。

源码依据：`push_templates.rs:199`的BannerCtx::render在`:222`用then_some立即执行account_status_note，后者可读`user_account_summary::latest()`；`:230`又读closing_valuation_note。`dispatch_auction_volume_snapshot_with:5977`混合render/sink/日志/recorder/notified；`:6020`的PushRecordMeta包含文案未显示的price，`:6044`通知code集合也不是文本相等就能证明。`main.rs:9703`已一次采集并保留完整原始列表供持仓检测。

当前另有真实输入缺口：`MarketAnalyzer::get_limit_up_stocks`直接调用limit_up出口；`limit_up.rs:186`将所有volume_ratio置None，`:303`又丢弃已有来源批次证据。此次不扩展上游合同、不补造量比/来源/VerifiedEmpty。该缺口、source保留出口、catalog/context/projector工厂与全局拒绝端口仍是后续完整W17/Unit接线工作，不能因本任务完成而删除。

## Global Constraints

- 仅隔离工作树`.worktrees/push-reliability-20260905`/分支codex/push-reliability-20260905。根工作树、生产monitor、真实数据库/provider/LLM/sink/order/PAM/.env/owner/批准/部署不操作。
- 不改冻结SQL/八份RFC输入/catalog/WBS/蓝图、既有canonical字节/哈希域或磁盘协议，不新增依赖、自动发送/恢复/重试、身份认证默认值或晋级许可。
- 仅一个实施代理；父线独占Cargo/Git/中文docs。代理不跑Cargo/Git/网络/生产、不起子代理。仅apply_patch编辑、定向rustfmt；禁止全仓cargo fmt。
- 原P-02时间窗、一次采集、筛选/稳定排序、完整原始列表、模板展示元组和governor/sink边界保留。消息文案在输入不变时逐字节兼容；缺量比仍不可发送。

## Task 1: 实际P-02 dispatcher消费冻结业务结果

### 文件和interface

- 主改`src/bin/monitor/push_templates.rs`：BannerCtx动态说明捕获、T-11共享纯渲染、P-02完整业务结果及实际dispatcher消费。同文件相关测试可以扩展，不删除/放宽旧断言。
- 可新增同目录`auction_volume_preparation.rs`作为聚焦module，若确需新增，先告知父线路径/注册方式，不为了包装增加多层interface。不得搬迁整个push_templates或全量模板。
- 不改market_analyzer/Gateway合同、本批不伪造PreparedFacts/强终态或开放任意RunContext构造器。main既有选中快照/共享原始列表不改；若真实调用适配确需main签名调整，先报告父线裁决。

### 行为合同

1. Banner动态说明只在显式捕获步骤读取。每次捕获closing valuation最多一次；账户指标完整时不查询用户账户摘要，或已有closing说明足够生成账户缺失说明时也不查摘要。现有缺失/降级文案优先级和格式保留，不为省读取把缺失信息显示为零。
2. 捕获结果私有字段、只读，保存本次真实读取的最终横幅内容或等价完整输入；重复渲染不读DB/全局时钟/全局说明。捕获之后外部说明改变不能改变已经捕获的输出。普通BannerCtx::render与P-02必须共用同一实现，不复制两套账户文案算法。
3. T-11原body格式继续由同一真实实现产生。保留已有公开render_auction_volume兼容调用者，可以内部委托冻结输入的pure helper；生产P-02必须实际调用该共享实现。注册展示的`T-11-auction-volume / AuctionVolume / auction_volume_dispatcher / render_auction_volume`保持不变。
4. 完整P-02业务结果一次从选中snapshot与冻结banner产生，私有字段、只读；同时包含精确消息、全部逐票PushRecordMeta提案和通知code集合。必须覆盖code/name/price/metric_json/push_kind/source，保留顺序、每项price及metric字节。不要只比较rendered text或CursorDirective动作名，也不引入无消费者的新持久化/哈希协议。
5. 结果只代表待执行提案，不是Accepted/Completed/ProductionVerified。构造/比较结果不得发请求、日志落盘、写DB/入池、推进notified或调用sink；不接收调用方预制的成功结果替代实际P-02计算。可提供只读结构化比较或相等性，使price-only/metric-only/code-set变化即使正文相同也可区分。
6. 实际`dispatch_auction_volume_snapshot_with`先准备一个完整结果，再把其中消息交原sink；仅原sink成功后逐项执行保存的record提案，全部成功才应用保存的notified集合。不得发送后重算选股或提案。缺banner/发送失败/任一recorder失败仍返回false且不推进通知集合。原日志落盘仅留在effectful dispatcher，不能进入pure preparation。
7. 维持旧发送与部分入池失败语义：不新增自动重发，不声称跨多次record原子事务或exactly-once；已写记录不自动撤销。当前此语义的完整可靠恢复留全Unit迁移，不由本任务悄悄改变。
8. 捕获/提案的Debug及错误不额外披露账户摘要、持仓、正文或原始外部错误；纯提案可由实际发送消费者读正文，不等于允许诊断输出正文。沿用本repo的脱敏模式。

### 测试先行与验证

第一条回归针对已证实的`then_some(account_status_note())`立即求值：在真实Banner捕获interface的外部说明读取seam记录访问次数，完整账户应0次账户读取。允许先做不改变行为的最小依赖提取，把实际DB/全局读取函数作为真实adapter；测试不能只验证另一份新写的伪renderer。先冻结原行为+新反例，父线实际运行RED，取得行为失败后才修改逻辑；编译失败不算RED。

之后按真实行为逐步补齐：完整/不完整账户、closing说明有/无、Degraded/Unsafe/Full文案优先级；捕获后更换外部说明而重复渲染字节不变；同P-02快照精确消息/record/code集合，price-only变化文本可同而业务结果不同；metric/code变化和稳定同量比次序；生产dispatcher实际使用结果及sink/record失败不推进。所有外部读取/发送/record替身只放在现有effect seam，不能用手造最终成功report代替实际业务计算。

保留原7项P02、1项T11、5项blocking_market_data验证；Banner共享路径改变需纳入同模块纯Banner相关测试和有明确黄金输出的相邻模板用例，具体函数由实施报告列出，父线统一组成一次最终过滤合批，非每次修改重跑全bin。定向源码核对发现整个push_templates::tests还含固定test_data/test.db初始化与显式可选真实sink E2E，因此本批不用整个组或宽泛banner子串；使用下列完整模块前缀，新增测试统一banner_/auction_volume_p02_前缀。需补充其他受影响测试时先核对其副作用；所有计数以真实终态为准。

```sh
env DISPATCHER_LOG_DIR=/private/tmp/p02-frozen-dispatcher.BLBDjv CARGO_PROFILE_TEST_INCREMENTAL=true cargo test --bin monitor -- --test-threads=1 push_templates::tests::auction_volume_p02_ push_templates::tests::banner_ push_templates::tests::incomplete_banner_ push_templates::tests::confirmed_snapshot_banner_ push_templates::tests::br134_ push_templates::tests::t0 push_templates::tests::t10_ push_templates::tests::t11_ push_templates::tests::t12_ blocking_market_data::tests:: push_templates::tests::p02_preparation_rejects_non_finite_and_non_positive_market_values
cargo clippy --bin monitor --no-deps --message-format=json
git diff --check
ruby scripts/architecture-docs/check-rfc-inputs.rb --root .
```

测试、Clippy与定向fmt分开给证据；lib163旧warning和bin原2项未修改位置warning单列，本批不新增诊断。若main未改不因其基线格式差异修改无关行。固定原始BASE..实际源码提交独立Spec/Quality审查；修复回原实施代理，复审仅FIX_BASE..fix。

### 报告与后续完整范围

代理在本plan同名SDD目录写task-1-report.md，记录真实数据流/实际调用者/所有变更文件/函数与测试/未运行项/RED及后续父线证据；中文docs父线统一更新。报告不得称已完成完整W17或P-02 Foundation迁移。

后续仍须：真实量比能力及源证据保留合同、producer/catalog/context/projector受约束工厂、完整业务新旧投影、精确业务提案进入shadow比较、八类及额外日志副作用实际纳管、authoritative channel与业务最终化/持久游标接线、六门禁和授权晋级。W15/W16/W18/W19剩余/W20/W21/其他51Unit与生产验收也全部保留。
