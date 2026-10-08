# 保留范围晨间接续（2026-10-08）

依据 [用户范围裁定](../handoffs/2026-10-07-active-scope.md)。本轮完成 Mac 原始只读业务观测、Windows 勘误回传，以及首轮已验证源码的主仓库合入准备；没有安装生产候选或回填结果。

## Mac 的实际连接与业务观察

2026-10-08 08:38:14–08:38:15 CST，复用原私有客户端身份，通过已有 `127.0.0.1:50052` 隧道执行 mTLS/Bearer grpcurl。GetHealth、GetCapabilities、GlobalNews/Cailianpress（limit=2）和 RealtimeQuotes/Sina（Shenzhen 000001）四项 exit 0。Health 元组为 version `0.2.0`、source `4e4995f8d3f2c7cd504d1dec0f238e6d4b4fc02c`、contract `0c4485545dbfd0979a7d5ea206c840f39fd504ed62fb7eef92f1940bdc9c2f41`、binary `517e0b4c31bb42330f4bc2a0395e3af385a164212414a65775bed40c9eb87ae3`，与共享旧 2026-09-28.2 deployment metadata 相符。Capabilities 111 条，仅为宣告。

新闻返回 complete=true、ADMITTED、2 条 news_item v2，逐条发布时间 08:35:32 和 08:31:11 CST；报价返回 complete=true、ADMITTED、1 条 quote v1，原 source_at 仍为 `2026-09-30T16:30:00+08:00`。新闻不是历史主题命中或生产实际送达，旧报价不能成为今日可执行价格、独立价格带或逐日交易资格。没有执行当前 Rust SDK 的同版验收，也没有用旧服务结果授予 SDK 67/92/841 上线资格。

本机私有最终 receipt SHA-256 `8c0599898b911bff5966274f4e8d6892bd7748e0a8b4ba42de9c9f6060ef7504`；原请求、返回、错误和时间保存在 `.planning/2026-10-08-active-scope/mac-rpc/`。共享非敏感原件 `client-bundle/MAC_ACTIVE_SCOPE_RPC_OBSERVATION_20261008.json` SHA-256 `2f9b86f98f1d9c466810b16ee1f42bf447f99ace00b2a4cea6a83fb0c62dac0f`。原件包含请求及公共新闻/报价，没有凭据；不推送 Git。

## 本次发现并保留的失败

- 共享 `manifest.sha256` 完整核查 8 件匹配、README.md 1 件不匹配：期望 `5b76826fbcde369a123e93e7ce2c9bb9c15b6623a5bc4cdf051e76a93fb4dff3`，实际 `181c733d45524db6972162cd579c5743355bd0b78225dc4cf409b1bed1c4e97f`。首 preflight 在加载凭据/拨号前停止，RPC=0；后继仅针对匹配的 proto、API、deployment metadata 做上述有界原始观测。整包仍没有通过，不改写供应方 manifest。
- Windows 新交接 GlobalNews 示例 `content_type=application/json` 被真实服务拒绝；初次新闻和报价各 exit 67 / InvalidArgument，错误为 `payload content type must be canonical JSON`。按现有 Rust builder 改为 `application/json; charset=utf-8` 后另发精确新请求，得到上述成功结果。首失败没有覆盖或算作来源 outage。
- `18082` LocalBridge 当时仍拒绝连接，其旧 reader 的 BR-172 V3 兼容错误与既有远端 `50052` 是不同路径。当前源码已删除 provider host，不能据此恢复本地 TDX host/fallback。08:42 正式日志已有 ExternalV1 SecurityIdentity 真实接纳，不能称全部 gRPC 链路中断。

已在原 Windows chat **R08 FuturesDelivery 上游合同与部署** 回传勘误与 H08 具体窗口，消息工具成功返回原 thread ID。共享 `MAC_NEWS_FINANCE_REPLY.md` SHA-256 `ed4db650efe79825f57eecf9716eef206d241b7c4e2d88e544297d9f15c4ef75`。发送与共享文件存在不等于 Windows 已读取或新数据已交付。

## 源码合入与验证复用

首轮源码提交 `3f9577ce3358582e9adc2894c709dbecccda07a0` 实现休市扫描、原候选历史窗口入口和周报，完整验证见 [首轮开发交接](../handoffs/2026-10-08-active-scope-development.md)。本次实际重查 7 件相关源码输入和 3 份原验证日志 SHA-256 全部匹配，复用原 41 个不同方法通过的证据；`git diff --check` 通过，不重复 Cargo。共享夹具首轮 8 项失败仍保留，没有宣称整库全绿。

合入按用户已有主仓库提交授权执行；实际 Git OID 和远端结果另存本机发布 receipt。master 合入不能当作正式安装。outcome 前序三项已在 master，不重复 cherry-pick 主仓库。

## 下一项最小发布切片

1. 先核对既有 `news-dedup-f517-20261007` 的冻结输入，而不是直接安装完整 bf0 候选。旧窄计划曾因用户当时的全量目标停止；当前范围允许重新准备必要窄版本，原精确批准仍不可复用。
2. 该旧候选的 748-input manifest 实际 `sdk_retained.source_commit=098021444d7b3c0dea4732c1b5a03e8773047cfb`、binary `22a9726aab44473694141ef78b884c56a99549b23d3c3f9381fead66f6510727`；不匹配上述当前 4e/517e 服务。不能按旧计划文字“保留 4e”直接构建/批准。先重新封存与现网匹配的实际公开合同、源码 delta、配置及兼容回退，再做必要 release 和精确 activation 材料；不得通过环境或 Health 自学习修改信任元组。
3. 窄版本优先包含现有日界去重及休市扫描，沿现网源基线评估 outcome 窗口和 D01 修复的依赖。不得把 master 全部平台/schema14/Financial 代码捆绑上线；确实依赖冻结能力的部分明确延后。
4. H08 继续交付 Shenzhen 000001、2026-08-21..=2026-09-30 显式历史原件与逐日缺口，先 ObservedOnly。独立逐日状态、生命周期及价格资格仍需具体来源和使用合同；已有 1076 证券输入复用，critical 95% 与同版真实 RPC 仍适用。
5. 数据及运行资格到位后有界回填原候选/预测，取得真实接纳与自然成熟观察；此前保持 unavailable/deferred 和完整缺口，不填 0 收益或扩分母。

本轮没有生产二进制/activation/配置写入、数据库替换、schema14 迁移、Uncertain 裁定或自动重发。正式 monitor 文件仍为 `a7376a14cde1f6be3025d6dbbdedf61401c34154f3da928a8e3db30c4950d20b`，08:42 日志仍 Unsafe，缺 Quote/Kline/MoneyFlow/OrderBook；News 在相邻轮次可用/不可用波动。原完整候选的 59 类 admitted dry-run 与要求 61 类的外层 driver 失败是两件证据：源码按当时 selection disabled 的实际 59/13/3 合同验证成功；原 driver expectation 失败保留，不能改写为全 61 类已可投递。
