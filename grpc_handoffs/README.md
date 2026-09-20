# gRPC 下游数据问题交接

本目录汇总当前项目中已经有运行证据或直接源码证据的 gRPC 下游数据问题，入口是 [2026-09-16 下游数据缺口](2026-09-16-downstream-data-gaps.md)。文档用于分派协议、服务、转换器和消费业务的后续核验，不是新协议批准、生产修复、迁移完成或上线验收。

新增：[公告/大宗交易绑定补充 GD-014～015](2026-09-16-announcements-block-trades-addendum.md)。原独立交接 13 项保持，主控结合最后 14 个 Unit 的静态追链补入两项，当前共 15 项；不会把业务消费者缺接线归咎于行情 gRPC。

新版接口：[client-bundle 2026-09-15.1 兼容审计](2026-09-16-client-bundle-interface-update.md)已完成，[双合同实施计划](2026-09-16-client-bundle-implementation-plan.md)正在执行。S0 Local冻结/External独立生成已验收，旧Local descriptor及生成代码逐字保持；S1a 的profile错误身份、双载体完整性、External attempts与在线/恢复解析已通过102项回归、全部目标编译检查及独立审查。实际External native数据/控制/事件、三个新RPC、健康身份及版本化恢复仍待完成，未部署，也未取得真实服务验收。

## 范围

- 运行事实只覆盖 2026-09-16 00:47–00:49（Asia/Shanghai）的本机 `127.0.0.1:18082`：两次运行 GetHealth 检查命令都在 dial 阶段超时，没有 HealthResponse、Capabilities，也没有已发送业务 RPC 的证据。
- 静态事实绑定详细文档末尾列出的原项目 `R` 与隔离开发树 `W=.worktrees/push-reliability-20260905` 文件 SHA；不把当前源码等同于当时运行制品。
- 只整理数据可用性、wire/投影/消费差异、来源身份、时间与单位、账户依赖和可执行验收。推送冷却、通用通知迁移和生产操作不在本交接中改动。

## 分类

| 分类 | 含义 |
| --- | --- |
| 运行证据缺失 | 端口或接口存在不等于 RPC、能力或真实业务批次可用 |
| 上游/协议合同待确认 | 客户端需要的字段或请求意图没有被实际 wire 明确表达，不能据此断言服务端不存在数据 |
| 客户端投影丢失 | wire/批次进入本项目后，转换器或兼容模型主动丢字段、批次身份或失败分类 |
| 消费口径不一致 | 数字存在，但期间、单位、issuer、集合或时点不足以支持业务比较 |
| 普通未配置/账户依赖 | 行情 gRPC 不能替代账户来源、搜索提供方配置或用户确认快照 |
| 已有保护、待运行验收 | 静态代码已有精确门或显式降级，尚未取得真实批次证明；不得当作待删除限制 |

## 问题索引

| ID | 优先级 | 时段 | 摘要 |
| --- | --- | --- | --- |
| GD-001 | P0 | 全时段 | 本机监听但 GetHealth 两次 dial deadline，运行能力未认证 |
| GD-002 | P0 | 集合竞价 | 涨停池 `TopStock.volume_ratio=None`，P-02 要求有限正量比后无合格行 |
| GD-003 | P0 | 盘中 | 涨停池 `main_net_yi=None`，连板和持仓旧分支直接排除；另一资金流 overlay 不能冒充所有路径已补齐 |
| GD-004 | P0 | 盘后 | Consensus 转换固定丢最近报告、日期和目标价，评级循环静态为空 |
| GD-005 | P0 | 盘后 | EPS 仅按同年比较，没有同报告期、预测年度、累计/单季口径和 issuer 绑定 |
| GD-006 | P0 | 盘后 | R03 新消息入口被账户依赖门无条件置为 `AccountMetricsIncomplete` |
| GD-007 | P1 | 盘后 | ProviderTopN 实际 wire 只有日期，本地 limit/filter 不是远端已接收字段；全市场排行仍是独立不可用合同 |
| GD-008 | P1 | 盘中 | T0 已有精确请求集合门和显式超龄标注；本轮未调用 T0 业务批次，不得误写成严格五秒硬拒或擅自收紧 |
| GD-009 | P1 | 盘后 | Economic 计划记录 limit/country，实际请求为 `{}`，意图与 wire 证据没有同层表达 |
| GD-010 | P1 | 盘后 | 搜索聚合吞 provider 超时/失败/空结果，外层只能得到空 Vec/Unknown，丢原始失败和调用身份 |
| GD-011 | P1 | 尾盘 | CloseCall 重读最新持仓快照，24h 判断截断且允许未来快照，最终 canonical 丢快照与行情批次 lineage |
| GD-012 | P1 | 盘中 | SectorTop 的板块 Gateway 返回 records-only，通知 canonical 只留展示数字和本机当前时间 |
| GD-013 | P1 | 盘中 | SectorAnomaly 合并两份板块榜与新闻文本，但最终 canonical 未保存两榜批次、原因字段或新闻归因输入 |
| GD-014 | P1 | 盘后 | Announcements 本地意图含 date/limit，实际 wire 为 `{}`，返回集合未绑定请求日/条数；详见补充 |
| GD-015 | P1 | 盘后 | BlockTrades wire 已带 codes/date，本地审计 hash 缺 date，成交行/单位/时点与固定投影未闭合；详见补充 |

## 使用边界

修复顺序应先恢复可验证的运行入口，再确认真实合同可提供什么，随后修客户端投影和业务口径，最后做逐业务真实批次验收。缺失值不能用 `0`、旧缓存或本机当前时间补造；不能覆盖 `source_at`、取消身份/新鲜度校验或把 `VerifiedEmpty`、Unavailable、普通未配置混成同一状态。

旧 8 月运营记录只作历史导航；未经当前运行证据核验，不写成今天仍故障。EMFILE 运行现象与已诊断的 SQLite 合法 FD 复用误拒绝是两个问题，根因尚未合并。

## 独立证据复核注记

2026-09-16 的独立复核逐项抽核了 GD-001～015 的关键 claim。复核期间并行开发已经使 `chain-macro-full-implementation-2026-09-15.md`、`news-virtual-side-routes-call-chain-2026-09-16.md` 以及 W 中 `grpc_source.rs`、`chain_post_close_macro_codec.rs`、`search_service/service.rs`、`pipeline/chain_analysis/preparation.rs` 的当前整文件摘要不同于交接记录的冻结 SHA；其他主交接所列快照仍匹配。原冻结 SHA 和运行时间窗保持不改，当前同名符号只用于交叉佐证，不能反向冒充原读取字节或当时运行制品。本次复核没有启动服务、调用 RPC、网络、数据库、Cargo 或监控。
