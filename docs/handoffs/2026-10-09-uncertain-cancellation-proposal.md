# 旧不确定投递的人工取消提案

状态：仅提案，尚未获人工裁定；生产 79 条原件未改。

79 条原始 Uncertain 均没有可验证平台回执。当前只有下列 4 个 Rolling 冷却头会持续阻断对应新决策；另 75 条不阻断今天的这些滚动路径。

|日期|类型|证券|原决策 ID|
|---|---|---|---|
|2026-08-25|CloseCall|600396|`bf6abbdae4211cbf29366186be6d59991b245073837aa3521db8714d2c714d82`|
|2026-08-25|CloseCall|600703|`037395453f91e3646c193774fe0215c1f35ba9769ee4b8e2ddd24b55cffa4635`|
|2026-08-25|CloseCall|603948|`28cb52c430f8b4b3a1132185eb5736fa54dbe5ffaaabc1e9a030759283cacb36`|
|2026-09-10|T0Advice|600396|`0843cd398f10a97632b653eefa82ac7c3c35780329a1b3d33a754ea146c868d2`|

建议由用户裁定取消这 4 个过期请求。执行使用既有 ManualDisposition::Rejected，并明确原因是人工取消过期请求，原物理尝试是否成功仍未知；原 Uncertain sink/result/disposition 字节保留。只追加人工授权、处置及审计，释放对应冷却/预算占用。不会重发旧卡片，也不认定历史送达或未送达。其余 75 条保留原状。

执行前逐条重核原决策、attempt、envelope 与当前 head；暂停正式 monitor；限定既有 Schema9 和这 4 条；不调用物理 sink、不初始化平台 schema、不宽泛处理其它 pending 决策。故障重试必须使用相同授权、证据和冻结裁定时间，不生成另一份裁定。

依据 CLAUDE.md："durable 层 Uncertain 决策需人工裁定（resolve_stale_uncertain 工具）；冷却头 Uncertain 会永续阻塞新决策"。已有同名脚本只针对 HoldingPlan 且没有 preview 参数，本次不能直接运行它。

全部原件和 4 条完整关联预览保存在本机 0700 私有调查目录；预览 SHA256：`2dbaa97e1a46d3223fd3c1a2cc137124f42463e7e6dd6d6ec2f8dafe15cd59dc`。
