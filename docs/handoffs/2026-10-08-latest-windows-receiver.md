# 最新 Windows 接收端兼容切片

用户明确保留最新 Windows gRPC 服务，并在 Mac 最新代码修改接收端。本切片以 master `0e739dcce` 为开发基线，包含 `3f9577ce3` 的 outcome 历史范围及闭市保护修复；不是旧 f517 monitor 开发分支。

## 当前公开输入

Windows 交接目录：`client-bundle/windows-grpc-production-841e4ae-20261008.1`（仅公开原件、诊断和部署回执，无凭据）。顶层 manifest `859ee3108af179ca79cd99e5e8a16a64c8cae4748f95059a5d310a36c7b56347` 的 31 项及 public manifest 的 13 项在 Mac 逐字节核验。

- Bundle `2026-10-08.1`，source `841e4ae7a9df62be0c4536fa9009c66089d282bf`。
- 原 proto `39694bb9650ee54b18cb44e4dbd27b42dbca4f3f797572ee86f5d15601d2ab6d`，与 Mac 当前主线相同，65 RPC；metadata `ec307e5f115f9c7772bd8d66e899751fe8480ef6ffd191c152ba3293786f3d93`。
- Windows 原编译 descriptor `abf28a3e0028488a7579da4d961e1a7c1408482bdc0500122c1956d225e480cf`；正式 EXE `7407a03b744dc4a0040257c5c8de4cef49ec283d5ddc3529fe4506ef04832a4f`。两者均由公开部署回执及在线 Health 绑定，区分 Mac descriptor `41db4b93…`。

当前 live 编译信任更新为上述公开元组；冻结 Oct1 67c832 release 的原 proto、metadata、decoder 和 policy 只用于历史记录，不能授权旧版 live 连接。旧 Sep17/Sep28 reader 继续保留。不通过运行时响应、环境变量或 mutable 私有 bundle 学习信任。

## 保留范围与迁移边界

沿 [当前范围](2026-10-07-active-scope.md)：H08、outcome、monitor 稳定性；H01/H02/H03/H09/H10/H14/H17 冻结；H04–H06 现有 paper；H16 周报。正式 monitor 新增显式 existing Schema9 reopen 边界，以旧已发布 DDL 在内存生成固定目录进行核验。它不对正式库执行建表/迁移、不 seed 策略、不改 user_version、不修补缺对象，也不启用新 Unit/正式资金链。默认平台 coordinator 仍需 Schema14，平台扩展和 origin 不能通过这个窄边界取得资格。

普通 counted 流程沿用当前主线的底层 attestation、事务、不可变引用、预算、冷却和幂等。P01 的现有生产 envelope 与 counted 权威继续使用，新增 correlation 表由冻结平台负责。新闻专用 BR244 的 L4 结算以真实源 reservation 绑定；通用 counted API 仍拒绝 legacy commit/rollback。该修复处理真实发送后误报 legacy settlement 错误，不重放已经发送的消息。

## 验证状态

公开文件核验已通过；源码实现已准备。相关回归、精确 release、原库副本演练、生产激活与实际数据/飞书回读另补真实结果，当前文档不宣称正式安装完成。

Windows 原 CI 关键覆盖率 89.40%/95% 未通过；窄消费者兼容验收不能改写原 release gate。其交接中的东方财富原生查询因未获准域名失败；其他三源查询成功仍需 Mac 当前制品独立验证。账户原日期、价格/日线/停牌资格及 Uncertain 保持原事实。
