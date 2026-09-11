# 盘后产业链固定准备：实施进度与验证边界

日期：2026-09-11。状态：**Task 1 执行中，尚未完成独立审查或生产切换**。实施基线42ce098；本记录对应隔离工作树中的在开发代码，不冒充最终提交或部署版本。[完整实施计划](../superpowers/plans/2026-09-11-chain-post-close-recovery.md)的Task 2–4及全项目W15–W21、52个迁移单元范围不变。

## 已写入实际业务路径的改动

| 行为 | 当前代码接点 | 已有证据与限制 |
| --- | --- | --- |
| 旧入口只执行一次准备，再返回原报告 | [mod.rs](../../src/pipeline/chain_analysis/mod.rs)的run_chain_analysis；[preparation.rs](../../src/pipeline/chain_analysis/preparation.rs)的prepare_chain_analysis | 新旧空池及显式外部adapter非空流程通过；不是生产部署证明 |
| 保留实际输入和采用的事实 | preparation.rs的PreparedChainAnalysis、prepare_chain_analysis_with_io | 固定业务日、涨停输入、概念聚类、生命周期、持仓匹配、龙虎榜和宏观输入已进入只读结果；缓存和chain_daily仍会由生产adapter写入，不是纯函数或持久检查点 |
| 留存本地模型调用和搜索材料 | preparation.rs的IoModel、observed_search、render_with_io | 本机真实HTTP协议覆盖检索词生成、深度、简化、总览；保留prompt/system/mode和分析器原返回文本，逐次保留搜索结果。没有证明远端请求正文、实际物理模型版本或真实提供方身份 |
| 模型原文与报告格式分开 | preparation.rs的ModelCall；mod.rs的build_report | 模型返回的空白、Unicode原样保留；报告继续原有去结论/评分行及清洗规则。旧成功/失败HTTP协议和纯报告回归通过 |
| 保留龙虎榜原日期政策 | [fetchers.rs](../../src/pipeline/chain_analysis/fetchers.rs)的fetch_lhb_observed | 源码保持按实际请求时Local自然日取数，单独保留请求日期/本地时间及Gateway证据；不改成pipeline业务日。此项未调用真实龙虎榜来源验收 |
| 中途失败保留已取得的事实 | preparation.rs的PreparationFailure、observe_stage | 已写入固定输入、先前事实和失败阶段的只读观察；验证状态见下表。返回错误不证明先前写入回滚，也不是可自动重试许可 |

## 实际验证

所有命令均由主控单队列运行；每次Rust源码冻结，记录执行前后摘要及原始日志。测试不运行monitor、不读取真实.env或业务库，不调用真实模型、行情或通知渠道。

| 验证 | 实际结果 | 能证明的范围 |
| --- | --- | --- |
| 新旧空池及非空无模型准备 | session93513：3通过、0失败、3395过滤，源码未变 | 固定日期/原报告与显式外部效果序列；是早于后续观察扩展的切片证据 |
| 模型/搜索留存及旧协议、报告回归 | session92921：9通过、0失败、3390过滤；编译2分20秒、测试0.07秒，源码未变 | 本机合成模型协议、5次合成搜索、原响应、跨自然日催化观察与原报告政策 |
| 旧真实SQLite写入/生命周期回归 | session95144：1通过、0失败、3398过滤；编译1.61秒、测试0.17秒，源码未变 | 新测试进程、完整名称精确单例、独占0700临时目录，保留真实DAO/streak覆盖；不是新准备同事务恢复证明 |
| 核心失败事实保留及相关回归 | session26279取得缺接口编译RED；实现后session1035：7通过、0失败、3393过滤；编译2分48秒、测试0.01秒，源码未变 | 概念失败、写入后生命周期失败、持仓失败均保留此前实际事实且不继续模型；只证明受控失败行为，不是生产事故复现或持久恢复 |

上述成功批次均仍有43条既有测试编译告警；没有声称全仓零告警或全仓测试通过。9项与SQLite单例来自两个命令，不写成一次10项合批。具体用例位于[preparation_tests.rs](../../src/pipeline/chain_analysis/preparation_tests.rs)、mod.rs的tests与[旧失败协议回归](../../src/gate_d_chain_analysis_regression.rs)。

## 仍须完成

- Task 1：板块目录与候选批次的完整证据；带版本的确定性artifact及无外部效果解码；其余缺失/限额/失败分支；旧协议测试迁入新入口并移除过渡测试循环；最终消费者编译、静态检查和独立规格/质量审查。核心失败7项批次没有重跑模型HTTP，最终相关协议仍需验证。
- Task 2：同业务库中的实际来源/模型阶段检查点、原报告字节、chain_daily原子进度、报告保存、冻结目标及逐目标发送日志，包含重开和崩溃验证；当前只有只读存储设计，不计为实现。
- Task 3–4：真实应用/定时器/启动入口、窗口与业务日资格、可信身份、必达策略、强完成游标和切换/回滚门禁。发送失败误封日尚未修复，不用局部准备成功来关闭这个问题。

安全范围须分清：新准备观察类型的Debug/错误展示及本模块改动日志只输出安全元数据；[analyzer/client.rs](../../src/analyzer/client.rs)的原有非2xx响应正文日志未在本Task整改，不能称整条上游日志已全面脱敏。此风险保留到后续安全工作，不借扩展本次源码范围掩盖。
