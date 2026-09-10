# P-02 选集拒绝诊断实施记录

日期：2026-09-10。BASE：`19310148fe84ab00652ec314e6c9f0c5461dcaef`；SOURCE：`298ab0d7b089a8c9173a90740b40f6d2148147bf`。状态：实现、定向验证和独立Spec/Quality任务审查完成，本Task已关闭。仅隔离工作区的单文件修改，不代表已部署、推送恢复或完整W17/Unit迁移完成。

合同见[选集诊断计划](../superpowers/plans/2026-09-10-auction-selection-diagnostics.md)。产品仅修改`src/bin/monitor/push_templates.rs`，478行新增、17行删除；没有改main、真实来源、发送/写库路径或生产状态。

## 实际解决的问题

原选择器对非空列表中的缺量比、非法数值和全部已通知统一返回同一个字符串，后续无法区分原因。本次在**实际选择器和tick返回值**保留结构化事实，不是新增无人调用的诊断函数。

| 接点 | 改动与证据 |
| --- | --- |
| 只读统计 | [AuctionVolumeRejectionCounts](../../src/bin/monitor/push_templates.rs#L5961)：七个私有计数、只读getter及严格的all_source_rows_valid_and_notified事实 |
| 失败分类 | [AuctionVolumeSelectionError](../../src/bin/monitor/push_templates.rs#L6008)：SourceRowsEmpty；或携带统计的NoEligibleUnnotifiedRows |
| 实际筛选 | [prepare_auction_volume_snapshot](../../src/bin/monitor/push_templates.rs#L6041)：同一遍输入同时统计缺陷、计算有效行和排除已通知；不二次采集或复制过滤规则 |
| 真实tick | [AuctionVolumeTickData](../../src/bin/monitor/push_templates.rs#L6130)：snapshot改为typed错误；纯loader和[真实loader](../../src/bin/monitor/push_templates.rs#L6165)仍共用该选择器，外层provider错误仍是String |
| 既有日志消费 | [main.rs:9736](../../src/bin/monitor/main.rs#L9736)：原Display日志直接输出这些字段；本次未改main，也未调用真实monitor |
| 成功提案 | [完整旧提案](../../src/bin/monitor/push_templates.rs#L6187)、[判等](../../src/bin/monitor/push_templates.rs#L6218)、[实际dispatcher](../../src/bin/monitor/push_templates.rs#L6295)未改行为；由既有纯测试核对消息/记录/集合 |

七个计数按原始**行**计算，不按股票代码去重：

- source_rows：原始行数。
- valid_rows：存在有限正量比、有限正价格及有限涨跌幅；尚未排除已通知。
- notified_valid_rows：上述有效行中，代码在本tick起始通知集合内的行数。
- missing_volume_ratio_rows：量比为None。
- invalid_volume_ratio_rows：Some量比非有限或不大于0。
- invalid_price_rows：价格非有限或不大于0。
- invalid_change_pct_rows：涨跌幅非有限；有限负涨跌幅仍是合法值。

缺陷计数可重叠：一行可以同时缺量比、价格非法及涨跌幅非法，不能把缺陷数相加当总行数。已通知也不隐藏该行缺陷。`all_source_rows_valid_and_notified()`只有原始行非空、每行有效且每行都已通知时为true；这不是Suppressed、NoData或完成许可。

成功结果保持稳定量比降序、先排除已通知再Top10、重复代码行保留、全部五元组字段、sentiment和watch_status。原始股票和通知集合均借用读取，失败不推进通知集合。

## 反例、修复与验证边界

1. 首个typed接口测试在缺少新类型时实际exit 101/E0433；这是编译RED，不是运行期行为证据。
2. 随后通过原String入口断言期望的计数字段，实际运行1项、0 passed / 1 failed / 731 filtered、exit 101。失败左值只有旧前缀，右值包含`source_rows=2 valid_rows=0 notified_valid_rows=0 missing_volume_ratio_rows=2 invalid_volume_ratio_rows=0 invalid_price_rows=0 invalid_change_pct_rows=0`，证明旧日志确实丢失原因。
3. 实现后首轮7项通过。随后仅对新增测试表达式做定向格式修正，再在最终固定源码完成下面全部必要验证；没有把重复运行算新增覆盖。
4. 两次RED在落盘捕获器创建前直接运行，保留了工具中的真实退出码与相关失败片段，但没有完整独立stdout/stderr文件。此局限不隐去，也未为补报告重跑旧反例。初次GREEN是完整合并日志，最终必要验证均为完整分流日志。

中间曾发生实现agent未及时收回已结束测试会话的停滞：主线定点检查本次Cargo命令后恢复同一agent，取回4分24秒后已结束的运行期RED。那段额外等待不是编译耗时。另一次消息中的完整SHA转录错误已用实际命令纠正，没有以记录修正为由重复运行验证。

最终源码SHA-256（验证前后及主线独立检查一致）：

`5096e5697f67338b45346d11efa8f9ec988774c17a8f9623b280adeaeb2044f4`

### 最终命令与结果

- `cargo test --bin monitor p02_selection_ -- --test-threads=1`：7 passed / 0 failed，exit 0；stdout SHA-256 `579f4325c7267a522e2b7e8c650242f6da9c3ef89ca8e9f709bcdbaa5f965f1d`，stderr SHA-256 `22f69577da12130abceda15c9d0e5c04dd88fcdf37400c64d562f8a1f16e8544`。
- `cargo test --bin monitor p02_preparation_rejects_non_finite_and_non_positive_market_values -- --test-threads=1`：1 passed / 0 failed，exit 0；stdout SHA-256 `758fdd948b0957a473809a36df5d1b81d39adaaae0ab2dcdd79ef41d31ef355e`，stderr SHA-256 `6d008f21a0d676974cf167f75917cf9912e22fc83acc0bfd4f103a4063f284ad`。
- `cargo test --bin monitor auction_volume_p02_preparation_contains_exact_message_records_and_codes -- --test-threads=1`：1 passed / 0 failed，exit 0；stdout SHA-256 `b52cf2b0cc6a7793404f7c78f5b0b66bb240ae8806aefe5dcc4a2ee1b99865f7`，stderr SHA-256 `ef8255f7ec7f8bdf64190b5672a26246c9753610a1912be0ffe5c70487c4cdaf`。
- `cargo test --bin monitor auction_volume_p02_preparation_comparison_detects_price_metric_and_code_changes -- --test-threads=1`：1 passed / 0 failed，exit 0；stdout SHA-256 `c3f6a2bce76a9e5d2b167cdb7be559b09c32c0965aa6b36cc10f320bf47d9139`，stderr SHA-256 `f98d447be149fd5be23493161b7c083c126c2f3bbaae4baada15367e98b8ad28`。

新增测试位于[source:18107](../../src/bin/monitor/push_templates.rs#L18107)以后，覆盖7个用例；原数值规则、[完整消息与记录预期](../../src/bin/monitor/push_templates.rs#L18491)、[隐藏差异反例](../../src/bin/monitor/push_templates.rs#L18548)均未删除或弱化。合计10个不同测试用例通过，不是整个monitor或全项目测试通过。

`cargo clippy --bin monitor --message-format=json`实际exit 0；完整812条JSON，190条warning、0 error；任意诊断span均未命中本批产品文件。主线还确认实际monitor bin编译制品及build-finished成功。没有BASE Clippy对照，因此不能把其余190条警告一概称为既有，也不能报告全项目零警告。stdout SHA-256为`e6780cd17bfd1d06bffc0a3ea59d9b616def9d624ba185c08da44522a5ae7c2e`，stderr SHA-256为`e961c6a010e31e89e7cc6f129b592e3b18202dab0fcc9f53b320736a301a4d8d`。

`rustfmt --edition 2021 --check src/bin/monitor/push_templates.rs`和`git diff --check`均exit 0、stdout/stderr为空。格式检查只读；实际格式修正仅涉及本批新增测试，没有重排无关代码。

完整最终输出及元信息位于本地私有工作目录`.superpowers/sdd/2026-09-10-auction-selection-diagnostics/`的`final-*.stdout/.stderr/.meta`，实现报告为`task-1-report.md`。主线`verify-evidence.rb`只读检查了精确cwd/命令/退出码、测试数、全部Clippy JSON及内容摘要，没有重跑Rust。这些原始日志未纳入Git，本记录保留命令、摘要和结果，不冒称远端CI验收。

## 独立审查与尚未完成项

独立审查范围固定为BASE..SOURCE，结论为Spec compliant / Task quality Approved，Critical和Important均为0。实现者已释放Rust/Cargo，没有代码修复轮。

审查提出的三项证据限制均已明确处理：早期RED缺完整分流原始日志，保留上文真实工具退出/失败片段而不补造历史；真实来源和生产验收属于明确未做的后续集成依赖；没有BASE Clippy对照，因此保留警告来源未知的边界。它们不被记成已验证事实。另有一项Minor：每条定向test命令均输出113条库警告，纳入全分支最终审查的清理清单，不在此单文件诊断任务扩张修复。最终验证及当前文件无诊断证据不变。

本次错误信息脱敏限于新增选择错误/统计，不宣称所有旧日志或整个tick的Debug都已完成隐私改造。空Vec也没有获得真实来源观察或VerifiedEmpty权限。

真实涨停池投影仍为[volume_ratio: None](../../src/market_analyzer/limit_up.rs#L295)，本次没有补量比、跨源join或放宽发送条件。因此“错误原因可诊断”不等于“已有可发送数据”或“已恢复线上推送”。

完整下一层仍需[真实P-02影子接线](p02-shadow-integration-handoff-2026-09-10.md)、同次完整业务提案比较、真实注册与来源认证、八类实际效果纳管及W16 owner/fence。已准备的[业务提案比较计划](../superpowers/plans/2026-09-10-shadow-business-proposals.md)尚未派发；它也不代表上述全部工作完成。
