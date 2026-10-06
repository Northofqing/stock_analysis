# SDK 与平台发布准备（2026-10-06）

用户授权“上线”。本次尚未切换生产；按依赖先准备同版 SDK 与 Mac 新闻 monitor/桥接制品，Schema14 与逐项结果反馈另批发布。本文记录准备状态，不能作为上线完成或激活批准。

## 现场身份

Mac 正式 monitor PID14998，SHA-256 `a7376a14cde1f6be3025d6dbbdedf61401c34154f3da928a8e3db30c4950d20b`，新闻修复 `f517f45c91ec9489f63d0156a1b8f9cf45400c3b` 已在原生产根运行。桥接 PID56417，SHA-256 `2546b74d3af6929b5a08de4303f506232c9988b8ff030ec994f45b642cd4b1e4`。详见[新闻上线记录](2026-10-06-news-critical-score-rollout.md)。

Windows 通过真实 mTLS Health 确认正式服务仍为 `4e4995f8d3f2c7cd504d1dec0f238e6d4b4fc02c`，binary SHA-256 `517e0b4c31bb42330f4bc2a0395e3af385a164212414a65775bed40c9eb87ae3`，server descriptor `0c4485545dbfd0979a7d5ea206c840f39fd504ed62fb7eef92f1940bdc9c2f41`。Mac 新闻制品从旧 `client-bundle/` 编译对应身份；未部署的平台分支 `a6b87992` 从 `contracts/external_v1_current/` 编译的是 `67c832…`。两个 Mac 来源不能混称同版。

旧 Mac probe SHA-256 `6fe9aee7b5932e603cc512112dbe15abc2cfb3ac310d5239a467f1dd755d81f9` 的本次真实 `--opening` 退出0：9/9静态路由、四家新闻及 InstrumentNews 通过。SecurityMetadata 为 `ADMITTED/complete=false`。这仅核验旧正式版本；不证明新 SDK、完整证券资格、WG07 或 Financial。

## 数据与准备结果

- 两个原数据库 dev/ino 保持：主库 `16777220/154379673`，durable `16777220/154266271`；durable user_version=9。
- 只读统计：Delivered991、RejectedDurable3988、ManualResolvedRejected6、UncertainManualReview78。78条按来源为 DataMode73、CloseCall3、T0Advice1、WatchlistTracking1，未裁定、重试或覆盖。
- fresh health：Frozen/Unsafe、账户指标不完整，缺 Quote/Kline/MoneyFlow/OrderBook，News不再缺失。既有不确定投递保持隔离；各 Unit 晋级仍需自己的来源、owner 与观察证据。
- SDK 源码包 `windows-wg07-sdk-verification-20261006.1` 的37成员与manifest共38件，安全路径、原字节长度与SHA逐项通过；source `eea9cc6eea57725da1bc602dd66de68448c8d8ab`。公开 ACK SHA-256 `ca5ce4e1ca2967b9f52fc2ad6e72e85d947012a59777eda2ed04cd8cf2da9d43` 已经发送，Windows确认收到。
- 原 Schema14/v1-v2 fallback 的1582输入和三件封存 binary 字节验证通过，但其公开 pin67 与当前正式 VM4e 不匹配。它没有新 activation 或双端回退验收，不能把字节通过写成可直接上线回退。

## 两件候选及边界

1. `/Users/zhangzhen/.local/share/stock-analysis-candidates/platform-a6b87992-20261006` 封存1663个 Git 输入。预备 release 在新实测 VM 身份出现后由 Root主动停止，原日志和退出-15保留；536秒的运行没有制品或PASS结论。没有安装、不重启生产。这版 pin67 不能用于本轮双端切换。
2. `/Users/zhangzhen/.local/share/stock-analysis-candidates/sdk-eea-news-f517-20261006` 已从已上线新闻版本的封存构建根逐字节复制748个受控输入，私有环境仅在本机保存。等待 Windows 精确的新 binary/公开 bundle，然后构建匹配的 monitor、bridge、probe 与激活工具，做同制品隔离演练和业务RPC验收。该切片不迁移 Schema14、不启用资金或额外 Unit。

Windows 现网到 eea 候选包含103文件及63→65 RPC的合同差异，不能只按日期兼容修复的六文件描述发布影响。Windows 新 release 与必要发布检查尚在执行，失败和待验项由其保留；没有提前切换正式服务。

## 剩余切换顺序

1. 核验 Windows 新包的 source/binary/server descriptor/公开 proto/manifest 与旧正式服务的回退绑定。
2. 在 Desktop 外完成匹配的 Mac release、同制品61模板隔离演练；保持认证、根、旧数据与单实例。
3. 生成九位纳秒 UTC Z 的未来 activation，提供精确候选供人工 review；此前 Wave0/Wave1批准不覆盖新制品。
4. 双端按批准的精确元组切换，核验实际 PID、二进制、数据库身份、同版 Health 与真实业务 RPC。失败按同数据根及匹配双端版本回退，不重放 Uncertain。
5. 后续平台发布须单独闭合 Schema14、完整历史兼容、相应 Unit shadow/owner、激活与生产观察；新闻或 SDK 上线不代表 M0–M7完成。

本轮没有修改生产数据、认证、源资格、资金批准或保留策略，也没有取得新SDK或平台的生产完成证据。
