# 新闻连接恢复热修（2026-10-05）

状态：源码迁移、独立静态复审、63项定向测试、release构建与隔离dry-run完成；精确activation待人工复核，尚未部署。

## 范围与基线

当前生产冻结源码为 `1f0fc6a7f2c90916066a937e128500c532ae1c85`。2026-10-05 12:04 CST 对实际运行根的735个激活输入逐 Git blob 核对全部相同；生产 activation 单独核对。此热修仅迁移 `09a9370a43dc593cf06cfc7b473ef9cac734aa60` 的三个代码文件差量，没有复制其父提交中尚未部署的平台变化。

- `grpc_source.rs`：失败或取消的连接资格复核消耗缓存；下一次独立采集重新连接并通过完整 Health 身份与当前方法 Capabilities 门禁。查询和开盘消费者直接使用本次核验的连接与能力，避免随后重读被并发调用移出的缓存。同一次失败采集不会透明重试。
- `news_ai_shadow.rs`：待人工审核通知使用合法 `Denied/internal_audit` 审计词汇，发布成功后才确认notice；不裁定人工审核、不构成外部投递回执。
- `external_query_wire_fixture.rs`：测试专用的有界优雅停机、重绑定与身份拒绝夹具。

735项候选清单只有以上三个路径与生产不同。外部公共编译输入与生产一致；生产原 bundle 的凭据和端点继续保留。生产公告仍使用原路由，此热修没有顺带迁入后续公告接线。

## 验证与制品

证据目录：`/Users/zhangzhen/.local/share/stock-analysis-news-hotfix-20261005/validation/`。

- `baseline-verification.json`、生产/候选735项清单、公共编译输入清单已保存。
- 候选清单严格检查通过：735项匹配，无额外激活输入。
- 独立静态复审未发现阻塞问题；并发消费者结论来自调用链检查，尚无额外压力测试。
- 定向验证63项全部通过：缓存连接5项、BR-172 monitor11项、BR-238开盘/跨消费者合同42项、外部配置3项、notice持久确认1项、原公告请求合同1项。库检查复用同一harness；BR-238过滤实际包含20项额外跨消费者检查，与预估数量不同，已按实际结果记录且没有重跑。未运行全量测试。
- 新activation预览已由同基线准备工具生成，使用独立数据库，未写入生产配置；expected_config_hash=`ea045f580b00cb9cfd850e7e4af110d598c55280fc40afa3e0f4ef621b9ba14a`，effective_from=`2026-10-05T06:00:00.000000000Z`（14:00 CST）。预览SHA-256=`98f74b5645fbdf2408034cec2ae98eccc1bc427474037c67896b7ba644f97aea`，位于证据目录的上级 `activation-preview.json`。其中reviewer字段只是待复核候选内容，人审尚未完成。
- release `monitor` 构建退出0，编译绑定生产根 `/Users/zhangzhen/.local/share/stock-analysis-runtime`；SHA-256=`6b03444121106af7a94a6c54e6aef1d28ff03a41257def24e6be8ff5993e169e`。冻结制品保存于 `/Users/zhangzhen/.local/share/stock-analysis-news-hotfix-20261005/candidate/monitor`，与候选735项输入和5项公共编译输入一起封存。
- 同一release的 `--test --push-dry-run` 退出0，61个family、failed=0、external_process_attempted=0、receipt_audit_appended=0，实际bound root为本热修checkout下的独立TEST_CODE命名空间。能力缺失的家族明确跳过；这不证明生产消息送达或整个平台验收通过。
- 完整制品、激活预览、源清单和验证日志哈希见 `/Users/zhangzhen/.local/share/stock-analysis-news-hotfix-20261005/candidate-release.json`。新activation须在精确人审后才可安装。

## 生产边界与回退

2026-10-05 11:39 CST已对同版生产monitor做单实例连接恢复；四家新闻采集恢复，不等于新新闻已Delivered。12:28 CST只读持久库确认连接恢复后没有新的news投递决策；其他类型有Delivered。正常聚合窗口仍为9:30、11:30、13:00、15:00，各300秒；Critical因缺权威强度来源保持禁用。

旧生产monitor SHA-256：`851a5fb9f384e5ffbb437ef04979bd21d26fa1991f11a264229d5eb93703d658`。
旧activation SHA-256：`f574faf1868fc6e1a3963b83793e79fbcbb4a875116f864a29e4280e6a206b60`。
回退目录：`/Users/zhangzhen/.local/share/stock-analysis-news-hotfix-20261005/rollback/`，保存以上制品及三个旧源码，均已校验；不包含数据库。

新源码上线须重新计算activation，核对精确候选并完成人工复核后，等待未来生效时刻再按单实例切换。桥接维持既有版本；回退保持同一运行根与数据库，不覆盖数据或重发历史Uncertain。旧Wave0批准不适用于这次新候选。
