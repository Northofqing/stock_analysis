# 2026-10-02 平台开发与验证续接

目标为研究与 paper 平台 M0–M7 必要能力全部上线，阶段定义沿用[完整路线图](../superpowers/plans/2026-09-28-platform-complete-roadmap.md)。本记录区分代码、实连和生产证据。

## 合同、历史回放与激活输入

当前开发分支为 `codex/platform-roadmap-implementation-20261002`，工作树位于 `/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis`。`7d7184be` 整合已有开发，最终跟踪树与 `3bf93171` 相同，保留后续显式费用策略与兼容 wrapper。

| 提交 | 实施内容 | 实际验证 |
| --- | --- | --- |
| `fb1e658b` | 原始字节封存 2026-10-01.3 current 与 Sep28.2 historical proto/metadata。 | 原始 SHA 一致；CRLF 不改写。 |
| `623da65f`、`81152e12` | 新 current trust、Sep28 独立 messages-only decoder、旧 v4 回执与公开编译输入 receipt。 | current/archived 实际 descriptor 均吻合；旧 v4 保留字节并拒 crossed trust/downgrade。 |
| `1dfeef19` | 激活 executable manifest v2 加入 7 项公开 gRPC 编译输入，检查父目录及文件；脚本识别缺失目录的 v2 入口。 | Rust 激活 10 项、Python 9 项通过；旧 Wave1 manifest 735/735 仍通过。 |

定向库验证覆盖 59 个不同用例：build identity 4、decoder 1、connection qualification 3、External transport 9、flow reader 8、macro codec 24、activation 10。单独运行的两项 Sep28 v4 回归已包含在 macro codec 24 项中，不重复计数。最初两个文件名过滤词未匹配用例，随后改用实际模块路径并取得非零测试结果。

当前客户端 descriptor 为 `41db4b931010d7dfed7240dd1713ad85dac91ddee6338265737fcc6970ace90b`（31802 字节）；Sep28 为 `59158661146ff429f092c49584601147080b9e631e941e4acb2d96fe7a22bf7b`（31349 字节）。Windows server descriptor 为另一范围，不能互换。当前 policy 为 `a7bf22f2693e768349f8e53ec5180bf88128f88992966264377c69ec288c2fa8`。

独立审查发现 Python verifier 仅用目录 exists 会在整目录缺失时误降 v1；已结合 lexists、manifest 声明路径及 sealed build 入口修复，反例通过。运行凭据未加入 Git、激活清单或公开 receipt。

## 独立新版 Rust 实连

冻结探针源码为 `1dfeef19c12ff443fda044eb76b4521496712191`，位于 `/Users/zhangzhen/.local/share/stock-analysis-candidates/probe-source-1dfeef19`。probe bin 12 项通过；debug probe SHA-256 为 `5115857bf19da3c29799e92b81a00f259490f457ab41fce9ebb048bf43b3f730`。源码快照补齐同提交的编译引用 docs 后完成构建。

自包含 bundle 位于 `/Users/zhangzhen/.local/share/stock-analysis-candidates/windows-grpc-20261001.3-arry4487`，目录 0700、文件 0600，凭据引用保持在根内；public manifest 9/9。探针实际退出 0，compiled receipt 指向新公开输入，Rust loader、双向 TLS、认证及 Health 四项身份匹配。

- 2026 年 10 月 CFFEX 返回四条 `Planned` 日历记录，原生 schema/evidence 检查通过，`source_at` 缺失保持缺失。尚无确认交割事实。
- 开盘静态探针退出 0：SecurityIdentity、InstrumentNews v2 及 Eastmoney/Cailianpress/Jin10/ThePaper 四个 GlobalNews v2 各一条通过投影；九条诊断路线全部 ready。
- SecurityMetadata `complete=false` 只验证身份投影；新闻为有界最新批次，LocalBridge Announcements 零条只代表该次诊断。以上证据不证明全日全市场覆盖、D17/D20 或生产 monitor 已消费。
- Windows 新同版回执保留 MoneyFlows/BoardFlows 的 typed Unavailable，严格验收 exit 2；上游 TLS 失败仍未修复，不以能力登记代替成功。

原始探针日志及 descriptor/test receipt 在工作树 `.planning/2026-10-02-platform-continued-implementation/validation/`，仅记录公开身份和业务诊断，不包含 Token、私钥或证书内容。

## 后续源码切片

| 切片 | 当前状态 | 下一道验证 |
| --- | --- | --- |
| G5b 共同 writer，`b446aefd`、`2230d417` | JSONL writer 与四类 journal writer/recovery 共用日期锁；新日期可信，历史 pre-head 保持 Unknown，嵌套 locked helper 不重入。独立审查通过。 | 2230 同源定向 AlertLog 26、AttributionDeep 47 项通过。仍未完成跨文件/DB day seal。 |
| NewsFlash 每 tick 恢复，`e0dbb9bf`、`dc554745` | 两次恢复移到 blocking worker，typed error/panic 关闭发送；恢复完成后重采 reservation 时间。 | 最小候选同输入 monitor 24 项通过：helper 3、BR244 19、startup 1、公告绑定 1。 |
| WG06-A，`afdcd9d1`、`fa9fe103`、`491c6682` | 精确日期/日历 limit、External-only method、同连接控制回执和原始 capture；请求 bytes/correlation 绑定、调用时完成窗、encoded raw trailer hash。修复 closed ProviderId 分类缺 HithinkFinance 的真实遗漏。 | 最终 13 项通过。仍是 observed 入口，无覆盖/PIT准入或持久 store。 |
| 固定生产路径，`105de492` | AlertLog/journal 日期锁与 Coordinator 使用同一 immutable build root；Test 旧 relative/canonical/symlink 护栏保留。 | d920 同源 harness 的 AlertLog 27、AttributionDeep 48 项通过，包含两项真实 child 入口。 |
| 历史记录观察投影，`25ae008f` | 精确 identity/Day/calendar/原始 bytes、OHLC、lots100/CNY、Unadjusted、flattened Tonghuashun 与 outer Hithink；缺日 Unknown。 | 独立审查、fixture 字节检查通过；d920 同源新增 9 项通过。 |
| WG14 脚本，`6c1ebe7a` | 严格检查额外/缺失/非 regular 根 Cargo/build 输入；旧 Wave1 v1 兼容。 | Python 13 项通过，现最小候选 strict744 通过。 |
| namespace 漂移，`d0d343a7` | 对同 inode、正 nlink 的变化最多完整重查 4 次；身份、no-follow、权限和零链接异常立即拒绝。 | b21 同源默认并行 durable 224/224 通过，包含三项真实 FS 回归；独立静态审查通过。 |
| G5b 主循环，`3bde11e8` | 移除无持久证据的进程 LAST_RUN；模型失败、冻结/投递未确认不再遮住后续 tick。原 attempt/Frozen 分支防重算、防重发。 | 两次独立审查；8cd 同源 monitor `g5b_` 7/7 通过。持久完成门仍待后续 seal 接线。 |
| WG06 原始观察存储，`3a92648a`、`8cd978d3` | 保持 16 项 capture hash bytes；不可覆盖发布、外部文件 hash pin、私有 recorded-only reader、窄 facade 与独立 probe。负观察也保留，磁盘不恢复 live 权威。 | 首次 7/16 通过，9 项 fixture 因默认 0755 目录被拒；只修测试目录为 0700 后，8cd 同源 16/16 lib、1/1 bin 通过。封存诊断产物及两次真实 bundle 请求已完成，见下方实际观察。 |
| G5b raw 选集候选，`b21eef89` | 借实际同 namespace/date guard，保存 LF/ordinal/offset/hash/inode/head；原 stable top3、同内容两行不同 occurrence、合法 suffix 保持 cutoff。 | b21 同源 harness 11/11 通过，正例由真实 writer 发布 head；独立审查通过。尚未接入 provider/window、main/counting、cohort DB 或 seal。 |

## 生产与协调

10/2 00:58 CST 只读检查：launchd monitor PID 4371，心跳和快照新鲜；账户 Frozen、数据 Unsafe，Quote/Kline/MoneyFlow/News/OrderBook 五项缺失。未修改生产根、二进制、activation 或数据库。

原 Wave1 精确批准元组保持原封存文件；2026-10-02 09:00 CST 前不能切换。新版 Windows source 与旧候选 pin 不同，后继候选必须独立封存和精确激活审阅，不能混用新公开输入与旧批准。

Windows 对话“R08 FuturesDelivery 上游合同与部署”已收到本轮 Mac 实施状态及 HistoricalBars 原始 record/身份/日期/单位/复权/覆盖要求，公告截断修复已通过其 15 项 Provider 测试，业务回执采集中。继续按[WG01–WG14 任务](../superpowers/plans/2026-10-01-windows-grpc-development-plan.md)验收上游交付。

整体尚需 M1 全部 owner/intent/finalizer 迁移、持久封日及 P05 真正来源关联、M3 统一健康和真实观察、M4 精确历史/PIT与 paper 单 owner/cutover/对账、M5 WORM/Gate P 和 M6–M7 必要研究与前瞻裁定。自然观察与外部设施证据按阶段积累，不能由本轮测试替代。


## Windows 新共享交接与最小候选

已读取 `client-bundle/WINDOWS_CODEX_HANDOFF_20261002.md` 和两个 public evidence 目录：35/66 文件的 manifest 原始摘要及全部逐文件 SHA 均通过，验证 receipt 在 planning validation。生产仍 source `67c832e`；source-only 公告修复 `9963887` 未部署，metadata 的 deployment identity=null。原生 722 total / 300 records 的 complete=false 修复不代替同版 v1/v2 gRPC 验收。历史 688277 一行、688561 十一行、limit1 仍 complete=true，正式字段已提供；缺日理由、修订/PIT 和权威空结果仍不具备。新闻九 selectors 是八来源，Cls/Cailianpress 为同源别名。资金流 typed TLS 失败、D14/D17/D20 与 R08 confirmed 缺口已继续交给获授权 VM 任务。

最小 Wave2 worktree `/Users/zhangzhen/.codex/worktrees/wave2-current-contract-20261002/stock_analysis`，HEAD `c0f3ab7d`，base 为获批原 Wave1 `3a3a48f8`，不带入主开发树 G5b/历史/Paper/Research 新行为。744 项 manifest v2 SHA `9ba989d406f98a192751828cd727b0ebd0351d6f6f6b42d2480dfa5b34d0d9d4`；60 个不同 lib、24 个 monitor、13 Python 通过。production-root release 三 bin 已成功；monitor SHA `75cc914053893840d87a841426bcdb0c8f429a06aa37eb93b80600d5bdc251f2`。封存 dry-run exit0，release同版 FuturesDelivery/opening 和现生产认证 bundle 的只读 canary 均 exit0；继续区分 Planned/有界静态资格与生产健康。

same-input preview config `df933c736423d0e687c143e939b5166810e04f84b563a8c55683e3f8e2ed73f8`，activation SHA `ba5c08a3e3c61846f48ae3060138ea3f45bdaa5b8b1c4ba6efb8b5a1099d886c`，10/2 09:00 CST生效；精确人审已发，批准待取得。CLAUDE/helper 的人审要求只涉及这一新增具体元组。获批并到生效时再执行动态同源/身份/lease/DB门与实际 root helper 复算。本轮不改生产 root/进程/DB。

02:28 CST durable 只读：schema9，5053 decisions；981 Delivered、3988 RejectedDurable、6 ManualResolvedRejected、78 UncertainManualReview；1065 attempts/results、981 raw Accepted。PID4371 打开 production lease 路径（唯一 owner/generation 仍须动态门），所有 DB inode 已记录。旧735/source/public/binary/activation回退元组只读复制并通过strict735，DB/WAL不复制回退、不自动重发或裁定。

G5b SQLite 共同日期锁已提交 `98062f74`，独立行为测试 `d9208037`。保持 schema11；路由来自 stored immutable envelope，先释放 DB locks 再取得日期锁；两个 after-SQL hooks 后、COMMIT 前重查 fence/schema/refs，真实 after-COMMIT 故障明确保留已提交事实；外部 sink/immutable append 不持日期锁。实际九类 mutator、最终投影、stored-owner 冲突、同 Arc 释放 DB mutex、目录/锁替换及晚到原始回执均有覆盖。

d920 同源库编译成功。首次默认并行 durable 207 通过、14 失败，全为既有 namespace 链校验遇到共享测试目录 nlink 漂移，三个失败发生于 fixture open、尚未进入 G5b 逻辑。相同不可变 harness（SHA `d05b9482bf2c4f2b788d3528c383e68fe6e2b8deba920893ba5cb9aa74fe99d8`）串行 durable 221、历史 projection 9、AlertLog 27、AttributionDeep 48 全通过。原始失败与串行结果均保留在 planning validation；`d0d343a7` 修复这项既有并发假阳性，只允许正链接且同 inode 的漂移做有界完整链重查，真实身份/链接/权限错误仍立即拒绝，后续 b21 默认并行 durable 224/224 通过。

后续 selection v2/cutoff/cohort/owner 与 schema/CAS day seal 仍按单独版本闭合；这些写入锁检查和 terminal outcome 观察不能提升 main 的日终完成门。

## 第三批 Windows 交接与本轮验证快照

Windows 第三批 36 项 public evidence 全部逐文件通过，manifest SHA `027395f16f0d60cb69714f43c2517c8e8b561fb66454d86658ef000760c46df2`。东财新官网路径只有一次个股 JSONP 正样本，板块仍失败；9 月独立 CFFEX 实际交割原文存在，但无精确发布时间/修订与正式 Confirmed 准入。50056 的组合隔离动作在进程创建前被执行政策拒绝，未得到具体命中规则，不改启动器绕过。

Mac 反馈已保存到共享 `client-bundle/mac-evidence-20261002.1/`：5 项 public 文件，manifest SHA `7a4cbffd67ca2b9bc2fa2b04f207e171dcc3109c6f77bb2b829e2be5fb5cd381`。03:10 CST 系统 DNS 返回三个 `198.18.*` 地址；系统 HTTP/HTTPS 代理开启 port 6152，Surge 进程存在，具体匹配规则尚未观察，未修改网络或导出账号/订阅/凭据。已有 release canary 的 stdout 是脱敏资格摘要，不能当完整 raw 控制/请求/响应。反馈及后续源码任务的 native send 成功返回；03:36 的只读状态调用返回 host unavailable，当前远端运行状态未知，不将它说成发送失败。

03:47 开始 `b21eef8976f58d2de03fa1ab93587c5de0535289` 同源定向检查：durable 默认并行、WG06 observed/exact/projection、G5b raw 选集、AlertLog/Attribution、monitor `g5b_` 和 probe bin。所有 source/HEAD 冻结；每项真实结果写入 `validation/dev-b21-tests.json`。未经结果不称通过。

观察 store 是本地 byte-integrity 边界，不能替代远端 WORM、签名、覆盖或 PIT。link 发布至 unlink 临时名之间崩溃可能留下 nlink=2 文件，API 拒绝读取/收养并保留现场；不宣称自动崩溃恢复。next schema12 预审坚持原表/BLOB、真实 decision FK 和历史 v1 原字节保留，Prepared/Committed 与 Clean/revision 先闭合，随后才接 exact owner/handoff 和 CAS seal。

b21 默认并行 durable 224/224 已通过。复用该提交实际 Cargo 生成并封存 SHA 的 lib harness，WG06 exact 13、历史 projection 9、G5b selection v2 11、AlertLog 27、AttributionDeep 48 全通过；receipt 为 `dev-b21-harness-tests.json`。没有重复计入 d920 串行结果。首次 WG06 store 的九个失败根因为 tempfile 默认目录 0755，修正 `8cd978d3` 仅涉及三个测试文件的 0700 初始化，生产校验不变。8cd 新鲜定向检查已取得 WG06 observed 16/16、monitor g5b 7/7、probe 1/1，并成功构建真实诊断 bin；receipt 为 `dev-8cd-tests.json`。

## WG06 实际原始观察与下一阶段

10/2 04:12 CST 封存 `8cd978d3` 的 debug `historical_observed_probe`，SHA `d36145870228baabe4221e5620db39e5a53459e4415734254c17b15503d85ce7`，18935080 字节。使用现生产认证 bundle 只读连接；输出位于 Desktop 外独立 0700 目录，不启动 monitor 或改投递库。

| 实际请求 | 真实结果 | 保持的资格 |
| --- | --- | --- |
| Shanghai 688561，7/16–7/30，可信日历 limit11 | exit0，11 个请求交易日全部观察到，server complete=true。artifact capture `7ded7755bd97f9728e4d9694ee1c34ccfa1cefcd382b4dd22274aa82860c14e0`，file SHA `93caf2f3826fbd95176eafaa083292d4c94d538742ce2e0086290ad987b583e1`。 | ObservedOnly / NotAdmitted / coverage Unknown / PIT NotCertified。 |
| Shanghai 688277，同窗口和 limit | exit0，仅 7/30，一共缺 10 个交易日，server complete=true。artifact capture `5c71710123e36c2241131ae70ee10db96db8da2fd37309290b035cda40a1ffde`，file SHA `46199724fac9bd3d31e8e599c8bfcce94ea5ee728e28cbbf1ae5abea40df8e17`。 | 如实保存缺日，不能从 complete 或数量授予覆盖。 |

两份原始文件均独立复算 16 part 的长度/plain SHA、原 capture v1 哈希、外部文件 SHA，确认 0400 单链接、issued/observed request bytes 相同。Health BuildIdentity 均为当前 source67c832e/service0.2.0/server descriptor abf28a3e/binary9302a303。实连 receipt 和复算记录为 `wg06-live-8cd-receipt.json`、`wg06-live-8cd-artifact-verification.json`。

共享反馈 `client-bundle/mac-evidence-20261002.2/` 含五项公开文件，manifest SHA `bfd4a7a2514adb29e0b949719efa76a9bc636f816cdc50f25b11de5b2fcdb631`，附实际原始 Health/BuildIdentity/Capabilities/capability 和 issued/observed request；未共享凭据材料或完整业务捕获。native send 成功，随后状态 revision19 确认 Windows 对话 active/inProgress。继续要求精确请求 echo、coverage/missing reason/sourceRevision/PIT 和此前资金流/公告/身份来源任务。

G5b B 已在独立 managed worktree `/Users/zhangzhen/.codex/worktrees/g5b-cohort-schema12-20261002/stock_analysis` 从 8cd 开始，分支 `codex/g5b-cohort-schema12-20261002`；实施 additive schema12、私有 date session、cohort/artifact intent/revision 与同内容恢复，并进行交叉审查。尚无 B 编译/行为结果，不称 B 完成；后续 C/D 的唯一 owner/handoff、真实终态与 CAS seal 仍未接线。
