# 竞价输入状态提醒

09:20–09:25 的持仓报价采集和 P-02 输入失败原先只记日志。普通 DataMode
发生身份可复用当天早先的冻结消息，因此全局 Unsafe 未变时不能表示新的
竞价输入事件。新增提醒使用独立来源事实，仍走 T-02/DataMode 的 ordinary
Schema9 counted 投递，不改数据库目录、平台 schema、行情准入或交易门。

## 来源和文案

- 持仓报价：消费既有 `fetch_scanner_position_quotes` 的实际结果。采集错误
  只显示“本轮未取得可用批次”，不外发原错误、请求细节或凭据。空持仓不
  是 RPC 故障，也不授权恢复。
- P-02：消费本 tick 原 `AuctionVolumeTickData` 和真实
  `LimitUpObservation`，保留空、缺量比、数值不合格与采集不可用的区别。
  verified empty 显示“量能候选为空”；全部有效且已经通知是正常无新候选。
  当前涨停池合同没有量比字段，取得新涨停池本身不证明量能恢复。
- 提醒只显示输入事件和北京时间，不带账户资金 banner，不宣称全局健康
  或交易恢复。binding 的 old/new 均为采集时真实 aggregate DataMode。

## 持久身份和再次变化

来源指纹采用 `auction-input-alert-v1` 独立域，固定上海业务日、09:20–09:25
窗口、封闭输入类别、episode 和 unavailable/recovered 阶段。错误全文、
轮询时刻、当前 mode 和批次 ID 不进入发生指纹。binding 继续使用既有
`data-mode-v2:{day}:{actual_mode}:{fingerprint}`；新建前查询 Full、Degraded、
Unsafe 三种原 owner，任意状态已有 owner 都不替换消息或重发该阶段。

episode 从原持久事实推导，内存不是状态权威。相同不可用原因的重复轮询
不新建；真实恢复后再次失败开下一 episode。P-02 从空候选变成缺量比等
不同输入状态也开下一 episode，并绑定被替代故障的来源 SHA，避免继续
展示较早空候选为本轮状态。恢复仅关闭最新 episode，不把旧 Unknown 改
成 Delivered，也不证明客户端看到原消息。

恢复要求同类新批次观察在原故障之后、当前同日同窗口、非未来且不超过
30 秒。报价另外沿用 scanner 原 5 秒 source freshness/准入检查。P-02
必须取得可用候选或全部有效且已通知，只有涨停池成功、旧缓存或普通
TopStock 不能授权恢复。来源观察时间沿用公共 evidence instant 合同，含
`unix-ms:` 编码。P-02 的 day-only source_at 不解释为实时行情时间。

## 封闭治理入口

只有私有 `PreparedAuctionInputAlert` 能请求 source-only T-02 路由；不新增
通用 raw text/binding/profile 选择器。沿用 DataMode 原 operational profile，
quiet hours、launch gate、计数投递、原不可变审计、sink 结果结算保持。
AuctionVolume 等交易相关默认门不降低。

新提醒 `retry_authorized=false`，不补发过去的窗口。最终 Magiclaw 前的
denial-only guard 重读原冻结 envelope/source，校验身份、内容/来源 SHA、
日期窗口、阶段时间和原故障关系。旧日/旧窗口消息拒绝物理发送。普通
DataMode 来源没有此元数据，保持其既有行为。

竞价报价输入观察在普通报价卡已拥有发生身份后仍持续每 30 秒采集，才能
发现恢复后的再次失败；普通报价卡继续只占一个原身份。盘内观察原采集
节奏不变。未增加 provider 探针、行情资格、交易建议或接收身份配置。

## 验证边界

相关回归统一为 `cargo test --bin monitor auction_input_`：封闭分类、重复
轮询、状态改变、旧/新批次恢复、再次失败、实际 mode binding、冻结内容
与窗口 guard，以及原 Schema9 目录的隔离 Unknown/reopen 跨 mode 去重。
Schema9 测试使用新 `data/test/TEST_CODE_*` namespace、原 schema 合同、
真实 coordinator 和内存 append/sink；没有真实 RPC、Feishu、环境文件或
生产数据读取。该检查不代表全仓测试，也不代表生产消息已经发送。

历史 79 Unknown、四条人工取消提案、Windows 841 和交易/资金功能均不在
本改动范围。

## 实际上线与观测（2026-10-09）

行为源码 `64514f55d741b2c44d5baeff60b045d7544dda75` 已合入并推送 master。
最终相关回归实际通过 15/15，独立源码审查无未解 P1/P2；同次发布包含
此前已通过 27 项库回归的安全 gRPC 失败关联日志。没有运行全仓测试。

普通 release 六个目标构建成功；CLI 和唯一 launchd shadow 各渲染 61 个
模板、0 失败、0 外部发送，shadow 已卸载。生产仅安装八处源码、六个
release 目标、三个既有脚本别名及新 activation。activation 生效时间为
10:31:54（北京时间）；新 monitor PID 6251 已完成数据库初始化、本地桥
重连，持有原 lease，版本、配置和新进程心跳、快照检查通过。原本地桥
PID 36126、producer、凭据、四个 plist、日历和两个数据库 inode 保留。
生产 monitor SHA-256 为
`3fc3d571b8c90410042c281c821153cdcd3d36020f64d510c567b4838174efc7`。

原 observer 的“整个投递库逐表完全相同”检查实际退出 1：运行中的服务
已经产生新的自然消息。原失败及原脚本保持，没有把它改称通过。分离只读
验收已重新核对上述运行证据，13 个保护业务表和全部 2,589 条 paper 原行
与安装前完全一致；旧投递记录的保全另外按同一冻结逻辑快照审查，不能
把整个持续写入的投递库说成永久不变。

另一次只读事务在 10:52:36 冻结全部原类型逻辑行。该快照的 10 个新增
决策已逐图核对：7 个 Delivered、2 个 RejectedDurable、1 个 NewsAI
AttemptInFlight；在离线比较中按实际事件反推受影响的 head 和 sequence
后，原 19 张投递表的完整旧行数量与 SHA-256 全部匹配安装前，原目录、
79 条不确定记录和四条人工提案的完整绑定保持。没有写回、删除或重发
生产记录。原 observer 失败仍保留；该证明限于明确冻结时刻。

10:40 的 DataMode 和 10:43 的 NewsAiAnalysis 自然消息已有持久 Delivered
结算；飞书只读 GET 进一步确认原 Stock 私聊中存在相同正文的新消息，
未编辑、未删除。没有手发验证消息。这只证明两条消息到达飞书服务器，
不证明手机显示通知，也不代表竞价或盘内观察已经投递。

10:33 和 10:45 的盘内报价观察实际被原五秒 source freshness 守卫拒绝，
没有进入 Magiclaw。10:33 原报价在 prepare 入口仅剩 0.657 秒资格，
prepare 至 resume 入口的组合区间为 4.208 秒；当前持久时间不能把该耗时
归到某条 SQL，也不是物理发送或事务提交完成的精确计时。保留五秒门、
原审计和不重放边界；未擅自删减全局 reconciliation。真实样本已告知原
Windows gRPC Codex，收到 7,148 字节与 SHA-256 相符的只读收件回执，
Windows `841e4ae7a9df62be0c4536fa9009c66089d282bf` / 0.2.0 未改变。

10:53 的健康读回显示进程、心跳和快照鲜活，数据模式 Full、新闻来源 ok；
账户仍 Frozen，缺 MoneyFlow，健康命令退出 1。竞价窗口已结束，新故障
提醒的自然窗口发送与用户客户端实际收件尚未验证，不能声称所有推送或
数据资格已经恢复。
