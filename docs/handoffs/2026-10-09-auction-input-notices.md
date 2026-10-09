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
本改动范围。源码交付后仍需独立审查；部署和自然窗口观察由主任务处理。
