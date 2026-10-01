# Windows gRPC 交接后的开发任务（2026-10-01）

证据截至 2026-10-01 23:59 CST。目标仍是[研究与模拟盘平台完整上线](2026-09-28-platform-complete-roadmap.md)。本文件把新交接变成可执行任务，不将接口登记、健康回执或源码通过等同于业务准入和生产完成。

## 1. 本次实际收到并验证的内容

- [Windows 原交接说明](/Users/zhangzhen/Desktop/Quant/stock_analysis/client-bundle/WINDOWS_GRPC_HANDOFF_20261001.md)及同目录 windows-grpc-20261001.3 公开包已到 Mac。
- 公开 manifest 的 9 个文件全部匹配 SHA-256。包含 65 个 MarketDataService RPC；相对旧包新增 OfficialPublications / OfficialPublication。
- Mac 于 23:55:58 CST 运行交接 verify-macos.sh，退出 0：双向 TLS、认证、请求 ID、GetHealth live/ready、四项部署身份全部通过；GetCapabilities 返回 129 条 provider/operation 能力登记。
- [Mac Health 原始公开回执](/Users/zhangzhen/Desktop/Quant/stock_analysis/client-bundle/windows-grpc-20261001.3/verification/mac-grpc-handoff-20261001T155558Z-38979-health.json)和[Capabilities 原始公开回执](/Users/zhangzhen/Desktop/Quant/stock_analysis/client-bundle/windows-grpc-20261001.3/verification/mac-grpc-handoff-20261001T155558Z-38979-capabilities.json)已保存。凭据未写入报告或 Git；脚本仅限制目录/私钥/Token 权限，未改应用配置或重启服务。
- 当前服务为 10.211.55.3:50051，TLS 名称 magic-market.local。传输 protocolVersion=1；业务 schemaVersion 按各接口选取。
- 尚未取得本轮 MoneyFlows、BoardFlows、全量公告、完整历史窗口、D14/D17/D20 或生产 monitor 消费的业务验收回执。

当前 Windows 部署身份：

| 字段 | 精确值 |
| --- | --- |
| service_version | 0.2.0 |
| source_revision | 67c832e43f36f188e4d769f409691c0b1d9a2ea2 |
| contract_sha256（编译 descriptor） | abf28a3e0028488a7579da4d961e1a7c1408482bdc0500122c1956d225e480cf |
| binary_sha256 | 9302a3036c7ed272fdf24dd84b4a66881ab682649e9af76b68ef62b2ad083854 |
| 公开 market.proto SHA-256 | 39694bb9650ee54b18cb44e4dbd27b42dbca4f3f797572ee86f5d15601d2ab6d |
| 公开 metadata SHA-256 | 010cfd26409b486ffa655111f27e7847a4b7ae4d844231ca79997d6308d2d36b |

原始 proto 文件哈希与 descriptor 哈希有不同范围，不能互换比较。

## 2. 必须先处理的版本边界

Mac 根目录、生产根和获批 Wave 1 仍为 2026-09-28.2 / 63 RPC，编译信任策略固定 source 4e4995f8d3f2c7cd504d1dec0f238e6d4b4fc02c、contract 0c4485545dbfd0979a7d5ea206c840f39fd504ed62fb7eef92f1940bdc9c2f41、binary 517e0b4c31bb42330f4bc2a0395e3af385a164212414a65775bed40c9eb87ae3。新服务直连身份与之不匹配。

src/grpc_client/build_identity.rs 编译嵌入 metadata；运行时换目录不能改变现有二进制策略。新子目录 connection.json 指向父目录凭据，现 Rust bundle loader 会拒绝 PathEscape。因此 grpcurl 已验通，现有 Rust probe/monitor 尚不能据此宣称接入完成。

Wave 1 精确批准仍覆盖原封存 source/public-input/binary/activation 元组，生效门为 2026-10-02 09:00 CST。切换前重查来源准入；若仍返回新身份，按动态门禁记录不满足并准备独立后继候选。新公开输入不得混入旧已批准制品。735 项配置清单未涵盖 client-bundle，单看 config hash 不足以证明信任策略未变。

## 3. 可执行任务与验收

状态：已验证＝本节明确范围已取得证据；待开发＝需源码/工具修改；待验收＝能力已存在但缺当前版本真实回执；外部合同待交付＝需 Windows 提供合同和源事实。

| ID / 优先级 | 当前状态 | 工作与责任 | 依赖 | 完成证据 |
| --- | --- | --- | --- | --- |
| WG01 / P0 | 已验证 | Mac 接收公开包并验证连接、身份、能力登记。 | 已有凭据 | 本次 exit 0、manifest 9/9、Health 身份及 129 条登记；只关闭基础接入任务。 |
| WG02 / P0 | 待开发 | Mac 建权限受控的自包含独立探针 bundle，凭据路径保持在 bundle 根内；封存公开文件及连接身份。 | WG01 | Rust loader 成功；缺文件、越界、错误证书/Token仍拒绝；凭据不入 Git/回执。保持现有路径边界。 |
| WG03 / P0 | 待开发 | Mac 在独立开发切片同步 proto/metadata、生成 descriptor、更新当前来源信任；为旧已存回执保留明确的版本化历史验证策略。 | WG01 | 新当前身份通过；错误身份拒绝；旧回执仍按其原身份可核验，不能被当成新版本回执。运行受影响 lib/transport/replay 定向检查。 |
| WG04 / P0 | 待验收 | Mac + Windows 核 MoneyFlows / BoardFlows 当前同版真实数据。Mac 已有隔离 typed transport/admission 切片，生产路由接线仍需单独完成。 | WG02–03 | 同源码/二进制/进程的 Health、Capabilities、request ID、原始 trailer、日志、完整 records、source date/interval、金额单位及正式 admission；成功与 typed unavailable 都正确。runtimeAvailable=true 不替代成功回包。 |
| WG05 / P0 | 外部合同待交付 | Windows 修 MarketAnnouncements 全量分页/截断语义；Mac 消费全量覆盖证据及保留饱和拒绝。 | WG03；Windows 合同 | 取得全部页或明确 incomplete；验证总数/去重/漏页/终态/同批次身份和权威空结果。2026-10-01 的 300/722 不能通过 full-market coverage。当前 v1 start/end/limit 上限 300 未补此合同。 |
| WG06 / P1 | 部分合同已有；Mac 待开发 | Mac 把 HistoricalBars 精确 start/end 映射到适合的 Provider，保留精确日期向量、请求身份、实际窗口和逐代码覆盖；Windows 补未具备的覆盖/PIT证明。 | WG03；有效交易日历/来源 | HithinkFinance 已公开 inclusive start/end，但 caller limit 可截断；latest-N 不能代替全窗。绑定请求 ID/hash、OHLCV/amount/adjust、逐日缺口、公司行动/as_of；完整和缺口反例均通过，再取得同版真实回执。EmQuant 当前桥不可发现，不作为必需可用前提。 |
| WG07 / P1 | 外部合同待交付 | Windows 交付 D14 来源内证券身份、精确区间、逐代码终态；Mac 完成 QualifiedDailyChangeDiscovery 消费。 | WG06；Windows 身份合同 | provider 返回身份能与请求绑定；TDX 无回显代码不能借外层请求补齐。身份、日期、终态冲突和空结果均有测试/真实证据。 |
| WG08 / P1 | 外部合同待交付 | Windows 交付 D17/D20 所需生命周期、上市/退市、板块/ST/tick/价格上下限、停复牌及权威逐交易日/空结果覆盖；Mac 接 QualifiedTradingFactsGateway。 | Windows 来源合同；WG03 | instrument/effective_on、规则版本、覆盖和来源事件ID/hash完整，缺字段维持typed unavailable；BR-092价格门及BR-171断档解释有证据。验688277 07-16至07-29断档/07-30复牌及688561对照；已登记CorporateActions或一个正事件不能关闭整组。 |
| WG09 / P1 | Mac已有Planned读取；确认源未交付 | Mac 复用 PlannedFact/confirmed Fact 分离实现，验 FuturesDelivery v2 的 planned-calendar 展示；Windows 为 R08 confirmed-event 另交月度交易所确认源。 | WG03；确认源合同 | 2026 IF/IH/IC/IM 四行只标 Planned；v1/2027/非法月份拒绝；不得虚构 sourceAt。R08 Confirmed 与结算事实必须另有实际确认和日期证据。 |
| WG10 / P1 | 主要adapter已有；同版/生产验收待补 | Mac 核 GlobalNews/InstrumentNews v2、来源登记和消费者映射；需要新Provider时再扩枚举；Windows 补源归属不安全/缺时源。 | WG03 | 按provider独立验真实批次、captured_through/cutoff、record evidence、时间精度、来源、去重及错误类型；有界最新新闻不变成全日完整。当前Mac显式支持Eastmoney/Cailianpress/Jin10/ThePaper，默认WallstreetCn不能重标为现有来源；SecuritiesTimes当前UNADMITTED。 |
| WG11 / P2 | 新公开合同已到；Mac 待开发 | Mac 明确 OfficialPublications/OfficialPublication 的产品用途后，做64/65 operation/method、类型化native evidence和消费者绑定；需常驻时Windows另交collector部署/恢复证据。 | WG03；明确消费者 | 列表/原文同host、provider、链接、native日期标签与精度保留；日期标签不生成publication instant/sourceAt；无历史/PDF/整站覆盖。Gacc未准入，Nea仅指定窗口；守每源至少1秒请求门并留原始响应hash。先做有需求的最小切片。 |
| WG12 / P0–P1 | 待接线/真实观察 | Mac 对 Quote/Kline/MoneyFlow/News/OrderBook 建当前 source→gateway→health→消费者逐项矩阵；优先恢复必需能力。 | WG03–10中相应数据源 | 五项各有准入与故障回执、新鲜度及真实消费者记录，健康有原因；任一缺失时正确显示Unsafe。Health ready或 capability登记不提升生产DataMode。 |
| WG13 / 上线门 | 待构建候选 | Mac 从冻结生产基线制作最小后继来源候选，配套 public输入、monitor、probe、activation与回退材料。 | WG03；相应业务验收/动态门禁 | 相关目标测试、release产物、隔离dry-run、完整输入封存与精确activation审阅；获批后单实例切换并核PID/lease/DB/来源/实际消费。保留78条Uncertain，不自动重发/裁定。 |
| WG14 / P1 | 待开发 | Mac 将编译嵌入的公开 proto/metadata加入构建输入封存或持续强制独立核验；统一候选审批与探针记录。 | WG03 | 只改公开输入、config hash未变时也能检出制品/信任变化；原始proto/descriptor哈希范围明确；不把旧审批套到新二进制。 |

每张执行卡实施前再锁定输入范围和最小检查。代码完成后按来源、真实回执和生产观察分别晋级，不能一次关闭整组。

## 4. 开发顺序

1. **先解除版本阻断：WG02 → WG03，同时做 WG14 的制品封存。** 保留 Wave 1 原封存制品及批准记录，在独立切片准备新身份。WG01 已完成，复用本次基础检查证据。
2. **并行完成业务源：** Mac 实施 WG04、WG06、WG10 和 WG12 的必要消费者；Windows 完成 WG05、WG07、WG08 及 WG09 的确认源，双方按同版证据汇合。新业务RPC测试只在需要的target及隔离数据根内执行。
3. **做一次最小后继上线：WG13。** 选择已验收路径，完整准备后提交精确审阅；尚缺数据的路径保持typed失败及可见原因。不能以成功启动宣称整平台完成。
4. **恢复原路线图 M1–M7。** 产业链同事实shadow、全渠道精确字节与真实回执 → 52 Unit逐个physical owner/intent/finalizer迁移 → 统一健康/结果评价 → GlobalSchema、精确历史/PIT与PaperLedger单owner、费用parity、seed/cutover/日对账 → WORM/Gate P → 必要研究能力及前瞻paper观察。新交接未关闭这些后续阶段。

自然观察窗、人工Uncertain裁定和远端WORM设施仍是独立前置；不按“接口已到”承诺全部上线日期。M8按实测需求作实施/不实施裁定。

## 5. 本轮验证范围和后续协调

本轮只做公开文件及源码对照、Mac最小连接检查、计划内容和diff检查；未改Rust源码、生产bundle、二进制、activation、数据库或monitor进程。未运行业务RPC或策略/投递验收。

Windows 续办以其对话“R08 FuturesDelivery 上游合同与部署”为owner。回传本次Mac exit 0和公开身份，明确上述仍需Windows交付的任务及接收位置。Windows提供新的source/descriptor/binary或合同后，重新评估相关任务，不重复已无变化的验证。

2026-10-02 已使用官方send_message_to_thread将本次公开回执位置、身份、计划路径及六组Windows续办验收要求送达该任务；工具返回目标threadId且isError=false。发送成功不代表任务已完成，等待交付反馈。

本文与[当前平台欠项](../../audits/2026-10-01-platform-open-items.md)、主任务续接记录 .planning/2026-09-29-platform-production/task_plan.md 一同使用。后者的旧历史段落保留时间点，最新段落优先。
