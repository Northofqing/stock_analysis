# ADR-0003：Gate P 远端不可改写证据 authority

**状态：** Proposed（存储提供方、地域、密钥责任人与预算尚未确定；不得据此宣布 Gate P 通过）
**日期：** 2026-09-29
**范围：** M5 对数据、投资决策、模拟订单/账本和绩效归因四类事实的远端保留与恢复

## 背景

本地 SQLite、追加式 JSONL、哈希链和备份能发现部分误写或篡改，但同一主机的管理员、磁盘故障和密钥丢失仍可能同时破坏事实与本地校验材料。[平台路线图](../superpowers/plans/2026-09-28-platform-complete-roadmap.md)要求 Gate P 有独立的远端 WORM authority、至少五年保留、每日签名根、恢复读取与逐日对账。纸面模拟盘和推送的现有运行结果不能替代这些远端回执。

存储服务的“不可改写”保护通常作用于**确切对象版本**。例如 S3 Object Lock 的合规模式在保留期内禁止包括账户 root 在内的用户删除受保护版本，但新版本或删除标记仍可覆盖普通按 key 读取的视图；启用 Object Lock 的 bucket 必须有版本控制。[AWS Object Lock](https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock.html)、[AWS 管理说明](https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock-managing.html)。因此只保存 bucket/key 或本地上传成功日志不足以作为 Gate P 回执。

## 拟议决定

1. **远端 authority 与本地 owner 分离。** 四类事实仍由各自现有模块拥有；远端存储只接收其规范化、已封存的证据包和每日根，不生成投资决策、不计算成交，也不成为新的投递或账本 owner。任何 Gate P 晋级读取一份独立的、可重查的远端保留回执；没有回执时状态为 `Unverified`，不得把本地文件或上传请求当作成功。
2. **按内容和版本精确定位。** 每个证据包记录 schema、业务日、UTC 时间窗、来源链头/行数、策略与制品/activation 身份、完整字节 SHA-256；对象键包含域、日期和内容摘要且只写一次。上传后记录提供方返回的 object version ID，并针对该版本读取保留模式、retain-until 和字节，重新计算 SHA-256。重复上传相同内容可重查同一版本；同一逻辑槽出现不同内容须显式冲突并保留两份事实，不覆盖旧版本。读取和恢复始终指定 version ID，不能受最新版本或 delete marker 影响。[AWS 对象版本与 delete marker 说明](https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock-managing.html)。
3. **保留策略在远端强制，逐版本验收。** 最低 retain-until 为远端确认写入时刻之后 **1830 天**，以覆盖至少五个日历年并留有闰日余量；采用不可由上传角色缩短的合规模式。bucket 默认值只是防漏配置，不能代替每个确切版本的回读校验。上传角色不得具有绕过保留、删除版本、改变 bucket 保留配置或销毁加密密钥的权限。合规保留不能因为本地配置回滚而缩短；纠错写新版本和修订关联。S3 的 Governance 模式存在授权绕过能力，因此不满足本门的最终证据口径；其 Compliance 模式和逐版本读取接口可作为实现选项。[AWS 保留模式](https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock.html)、[AWS 保留信息读取](https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock-managing.html)。
4. **每日签名根与独立恢复。** 四类事实的已封存包 ID、版本 ID、内容 SHA-256、来源链头及前一日根组成版本化 canonical 日根；使用与上传凭据分离、具有轮换和五年以上验签材料保留方案的非对称签名密钥签名。签名算法、密钥提供方与轮换序列必须在实际部署附录中固定并有 golden；在确定前不得发出可用于 Gate P 的根回执。每日根自身也写入远端 WORM。恢复探针从远端指定版本读取一个新包及一个历史包，验证保留元数据、完整字节、签名、链连续性，并重建可对账的只读投影。加密密钥可用性是恢复条件：对象仍受锁定却因 KMS 密钥被删除而不可读，不算通过。[AWS 关于 Object Lock 与加密密钥的说明](https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock-managing.html)。
5. **故障与范围边界。** 远端请求超时、版本 ID 缺失、保留元数据不合格、内容不符、签名不可验证或恢复失败，都保持 Gate P `Unverified` 并告警；本地未确认包可留在有界 outbox 等待人工/自动重查，不能把未知上传结果当失败后盲目重写或当已保留。现有纸面研究可继续以低于 Gate P 的明确状态运行，任何对外的 Gate P/前瞻晋级必须等逐日远端证据和其他数据、成本、样本外门禁一起满足。

## 选项与取舍

| 选项 | 事实与约束 | 本 ADR 处理 |
| --- | --- | --- |
| S3 Object Lock Compliance | 逐版本 retention 和 legal hold；需要 versioning；删除标记不锁定，必须保存 version ID；若使用 KMS，还要保护解密密钥。 | 可作为首个 adapter 候选，取决于账户、地域、预算和数据驻留裁定。 |
| 阿里云 OSS BucketWorm / ObjectWorm | BucketWorm 锁定后不能缩短保护期；ObjectWorm 提供逐对象版本保留，但官方文档当前标为邀测，两种模式同 bucket 互斥。 | 中国地域候选需先核账号可用能力；不得假定 ObjectWorm 已开通。[阿里云 BucketWorm](https://help.aliyun.com/zh/oss/developer-reference/initiatebucketworm)、[ObjectWorm](https://help.aliyun.com/zh/oss/user-guide/object-level-retention-policy-object-worm)。 |
| Azure Blob 不可变存储 | 支持锁定的时间保留和版本级策略；实际账号、地域及策略状态仍须核验。 | 可比选，但不在本 ADR 中指定。[Microsoft 官方说明](https://learn.microsoft.com/en-us/azure/storage/blobs/immutable-storage-overview)。 |
| 仅本地哈希链/备份 | 没有独立远端权限和恢复 authority。 | 不满足 Gate P。 |

选择提供方前，需要取得数据地域及访问边界、账户控制权、只写/只读/安全管理员分权、签名与加密密钥责任人、容量与请求量样本、五年以上费用估算及故障处置责任。这里不创建 bucket、凭据或不可逆保留策略。

## 交付与验收

1. 记录最终提供方/地域、权限矩阵、保留配置、密钥与费用方案；把本 ADR 更新为 Accepted，固定 canonical 包和日根版本。建隔离资格环境，证明上传角色无法删除/覆盖受保护版本或缩短保留期；对精确版本回读 `retain-until >= 1830 天`，并验证账号/策略漂移会使资格失败。
2. 用一个现有模块的真实但脱敏证据包完成 `seal → upload → exact-version HEAD/GET → hash/retention receipt → signed daily root → independent restore`。注入超时、成功但响应丢失、同槽异内容、错误版本、delete marker、密钥不可读及签名错误；验证重查、告警和 fail closed。再逐模块扩到四类事实，不改其有效 owner。
3. 在生产自然窗口对每个业务日完成本地事实/远端确切版本/日根/模拟账本投影对账，保留恢复演练的可重放回执。Gate P 还须通过 point-in-time、walk-forward、成本后样本外及路线图其他门禁；本 ADR 或单次上传成功均不能替代。
