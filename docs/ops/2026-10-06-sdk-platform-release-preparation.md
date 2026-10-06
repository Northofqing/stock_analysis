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
2. `/Users/zhangzhen/.local/share/stock-analysis-candidates/sdk-eea-news-f517-20261006` 已从已上线新闻版本的封存构建根逐字节复制748个受控输入，私有环境仅在本机保存。Windows 精确的新 binary/公开 bundle 已读回。Mac 以新窄兼容源码构建 monitor、probe 与激活工具；本机桥接是独立既有制品，不在该 Cargo 包中，LocalBridgeV1 合同未改，保留其验证版本与原数据根。随后做同制品隔离演练和业务RPC验收。该切片不迁移 Schema14、不启用资金或额外 Unit。

Windows 现网到 eea 候选包含103文件及63→65 RPC的合同差异，不能只按日期兼容修复的六文件描述发布影响。Windows 新 release 与必要发布检查尚在执行，失败和待验项由其保留；没有提前切换正式服务。

## 剩余切换顺序

1. 核验 Windows 新包的 source/binary/server descriptor/公开 proto/manifest 与旧正式服务的回退绑定。
2. 在 Desktop 外完成匹配的 Mac release、同制品61模板隔离演练；保持认证、根、旧数据与单实例。
3. 生成九位纳秒 UTC Z 的未来 activation，提供精确候选供人工 review；此前 Wave0/Wave1批准不覆盖新制品。
4. 双端按批准的精确元组切换，核验实际 PID、二进制、数据库身份、同版 Health 与真实业务 RPC。失败按同数据根及匹配双端版本回退，不重放 Uncertain。
5. 后续平台发布须单独闭合 Schema14、完整历史兼容、相应 Unit shadow/owner、激活与生产观察；新闻或 SDK 上线不代表 M0–M7完成。

本轮没有修改生产数据、认证、源资格、资金批准或保留策略，也没有取得新SDK或平台的生产完成证据。

## 窄兼容实现与实际构建（13:29 CST）

源码已在独立 `codex/sdk-eea-rollout-20261006` 分支提交 `10b43b3bbfa98f3fa75533603133b586a7b62420`，基线 f517。12件变化包括 65 RPC 当前合同、messages-only 旧63解码、冻结4e历史policy和两测试服务的未实现方法兼容；没有Schema14或额外业务能力。7项相关lib测试实际通过，独立任务审查0问题，整分支集成审查在进行。68项既有warning如实保留，未称全量测试通过。

当前公开proto原件12707字节/SHA `801c2033e72c520b34ca110eda18029699c619b740b71e50f99308a3088689c1` 保留CRLF；实际客户端descriptor31804字节/SHA `14fe7134ba6b9018d773c88d71744cdd52381081e70dd04a558c3147b4a9ea06`。原41db是假设的LF版本，两者仅source_info注释两处换行不同。Mac候选metadata独立副本 SHA `0d8d8c92c97e4c189ced5d94ea0d07ec3efa8e9dc5d1da24cb42f9f25ced4221`，只从封存声明构建元组填预期身份并标ExpectedCandidateOnlyNotObserved；SDK包null原件不改，不表示实际Health。

Windows新包46成员+manifest共47件已安全逐项读回，外manifest SHA `e62a746028189cd05b24940558d0d0192ca0ac8a7e796e643acb5c68e7a6e68e`，预期sourceeea/exe29a/serverabf。其1092文件归档原先被误称Git原始字节，Windows后来核实来自CRLF工作区；正在另给canonical Git blob包及逐路径映射。Mac原归档21项脚本控制检查实际11PASS/10FAIL/exit1，原因是脚本CRLF在pipefail行解析失败，原失败和1092文件前后不变证据保留；不能把之后规范来源检查覆盖成原件PASS。性能并未实测。

Mac新 Desktop 外 candidate 封存751受控输入（旧748基础+精确Git窄修改），正常release三目标已实际启动，原数据、服务、认证与78 Uncertain隔离未变。桥接保留SHA `2546b74d3af6929b5a08de4303f506232c9988b8ff030ec994f45b642cd4b1e4`。发布PASS、同制品dry-run、同版真实RPC、未来activation人审与生产切换仍待完成。

## R1 完成与 R2 激活预览补齐（14:14 CST）

前节为构建开始时的历史状态。本轮 R1 已实际完成，源码 `10b43b3b` 已推送且远端完整 OID 一致；任务及整分支两次独立审查均0问题。751件输入清单 SHA `a6355cea77f3479371a23e87f04bc4202e6778af9fb7215e8d1bf18374bba308`，正常release三目标退出0、耗时797.98秒，前后输入字节不变。monitor SHA `d4f38fc16feb06593a5db9073432cba57879bc71b801169ac72603a760a77610`，probe SHA `8036d04875448895aa477f7178e62b0ffe0a35f5117a29f392c63da27cd3c1fe`，激活工具 SHA `17853b6b63160ead7416cab09014709a6b35c6537fcd524f7c89d490933652b0`。同制品 dry-run 实际61/61、退出0，外部调用和receipt写入均0；完整日志 SHA `51d47854cf76c38e402ce7c24f6356a90c69dffef7d956e00e7750045bcd5c9f`。这组制品没有安装。

R1 的只读诊断调用原 release prepare 加载候选根，正确拒绝 `diagnostic_repository_root_release_forbidden`。因此R1不作为最终激活工具候选。保留原失败和生产根限制，新增只读 `preview-code-only`：验证实际生产根的原材料，要求完整配置路径/长度/内容SHA完全相同，只替换候选源码 executable_revision；输出明确未批准的预测哈希，不初始化DB、安装文件或取得release能力。原 prepare、board限制与现行命令保留。

此两文件实现提交 `3f0b29cdbbc2bb3884eadf99756f6dc97914201f`，实际7项定向lib测试通过，任务独立审查0问题；最终整分支复核、R2正常release、原非测试guard拒绝证明及CLI实际无DB/未来格式验证待完成。此前7项SDK兼容测试仍有效，68项既有warning如实保留。R2按751件原冻结输入加这两件精确Git差异重新封存，不能把R1旧工具混入新候选。

Windows离线追加包23件已由Mac安全逐件验证，manifest SHA `79382e826f1466fcddb0cd77884a6a81b113aa65664d86d197263e71c04b5b6c`。Mac独立解析原日志得到2023通过/0失败/3 ignored及doctest12通过/3 ignored，Clippy/rustdoc/合规声明退出0。规范Git1092件由Mac重建Merkle tree，与SDK提交tree完全一致；其21项POSIX工具控制实际通过，Windows已独立核验收件。原CRLF归档21项中的10失败保留，测量coverage及真实性能仍无回执。

客户端合同包9件 manifest SHA `bd2d2b100cc3d5e5183c4f5b2c8e135a03af7ffd7efb9b378a6902e7a156ebab` 已获Windows独立回读：client14fe与LF参考41db仅一个注释字段的两处CRLF差异，serverabf与参考只差顶层file.name；其余全量字段一致，MarketDataService65、所有服务72方法。合同语义与离线通过不能代替候选真实RPC。

已向原Windows任务发送后续要求：闭合规定coverage证据，细化隔离slot、数据owner、原认证与受控进程办法，待最终Mac制品交付再执行同实例有界metadata/news/quote/bars验收。正式4e、Mac旧新闻实例、原主库/schema9与78隔离保持；双端切换仍需精确activation人审和动态门禁。本轮仍未上线新SDK或完整平台。

## 实际 SDK 发布阻断与继续授权（14:23 CST）

Windows 实际执行原 coverage 结构校验退出1，发现122个critical文件中的4处内联测试体：eastmoney `mx.rs`、composition `derived_products.rs`/`grpc_production.rs`、nbs `api.rs`。封存eea不能通过原结构门；本地没有已配置llvm-cov/llvm-tools，GitHub也没有eea提交或同head的Actions结果。此处是尚无测量且结构已拒绝，不能写成低覆盖率百分比，更不能以离线测试2023通过替代。

控制审计也证明既有keepalive使用全局同名唯一进程，原start/stop绑定正式根并涉及agent，不能安全并行候选。未重命名binary、复制控制器旁路、改端口或强停；正式4e继续运行。后续真实业务验收须采用审阅后的维护窗口式双端切换或明确隔离控制设计，不能拿原控制器当单owned候选入口。

沿用户此前“继续做完并提交到远端”及最新“上线”的直接授权，已发送原VM任务继续最小测试搬移修复及现有CI验证。提供主任务ID供其直接读取人类原文，不以另一agent的技术意见补人审或运行控制授权。新的源码若有变化须另commit/newcandidate并重新绑定Mac预期元组，保留原eea包及失败。原checker、80/95阈值、critical集合与生产业务逻辑不降低。R2仍完成可复用工具/客户端验收，不承诺eea能切换。

本轮源 `3f0b29cd` 已推送，真实ls-remote OID一致；R2完整manifest751件与Git HEAD逐字节一致，清单 SHA `ff28a4a724990076d83bc09d1e0aec8087c7eff54488caa2d781b831cf5c2eaf`。两任务及最终整分支复核均0问题。其activation扫描741件；旧news manifest748/扫描740是不同口径。14:15只读核对Mac仍monitor14998、bridge56417、原哈希及两DB dev/ino不变；新R2构建在独立根进行，未安装。

## R2 发布制品与真实只读预览验收（14:30 CST）

三目标正常release退出0、716.006秒，751输入前后完全一致。monitor 41,655,976B/SHA `a9ddb1a0d4df3f5aa6d210611d79c091744b9a3b31e015ae721bc52caeb3313f`；probe 7,473,808B/SHA `dd52f36e9168d30d1b89f90613147becb50bf460f83287809083ebdd13e5b01a`；prepare 7,636,532B/SHA `414be27fb047ed49d60eeee22296dba41b255597bd9721063c112e39bb729b56`。release完整日志 SHA `593bdde130c5a0d627005d8e53070ff20af31c94012677655fd8ff4c6f830894`。

同最终monitor的61类dry-run退出0、失败0、外部调用0、receipt写入0，日志 SHA `ac9ad143842764797bbbfa75471e1a772a3ac4b0c0c4e7c9c010289da3ff22ba`。最终release rlib链接的独立只读诊断实际证明：原public prepare对候选根仍拒绝；实际生产根prepare与preview哈希相等且保持原 `00f3c35c…`；相同配置候选API与新CLI预测哈希一致 `5abab9050832655b6c1562ea078ddec2a37718f52d480e05545b9c9c476418b0`。没有解除原board根限制。

CLI实际输出紧凑五字段、九位UTC纳秒Z与单LF。指定不存在的数据库路径仍不存在，过期时间和缺参数均退出2/无stdout。未批准文件265B/SHA `4e38f08cc7dd1b7fbad8180a09c5d0c62af546458b495d862c114a91442d7eca`，拟署名zhangzhen，生效2026-10-06 17:00 CST；它不是人审或安装记录。鉴于已确认SDK eea门阻断，不发起无可执行前提的该版上线批准；后续新SDK元组会重新绑定和审阅。

完整R2公共交付包 `client-bundle/mac-sdk-eea-release-r2-20261006.1`，40成员+manifest共41件，83,525,028成员字节，manifest SHA `2b7c1131890b74f24e9b7a84cb45859453c36f12bc7ecfefaad8007d810e1807`。含全部751 Git匹配源字节、三binary、原测试/失败与独立review、完整构建/dry-run/preview证据；私有env/Token/证书/密钥/DB不在包内。已成功发送原VM任务要求独立回读；未把工具发送成功写成对方已验收。

VM最新实际已从Mac原生主任务读取直接人类开发/远端提交/上线原文，确认授权并开始四处测试搬移和独立审查/CI，不再重复要求同一开发授权。它保持原4e与控制文件。源码与工具本地验收完成，SDK新同版coverage/运行证据、精确人审、双端切换及生产观察仍未完成；M0–M7和M8最终裁定不因此关闭。

## SDK 修复提交、独立读回与实际 CI

新SDK机械修复为 `cfdb27683cd06ed51fbfb1c8f4c129067b21c935`，父eea、tree `29391f4e011c3b16310d161f57f672dede5c1259`；非force推送至 `codex/coverage-critical-tests-20261006` 并由Root GitHub API独立核对远端精确OID。仅四处cfg(test)/path外置、四个新测试文件及设计，共9路径；生产逻辑、原61测试/辅助/断言、合同、依赖、准入、critical集合/checker和80/95保持。

新源原日志由Root独立解析：受影响三库303通过/0失败/0ignored；workspace2023通过/0失败/3ignored；doctest12通过/0失败/3ignored。fmt、all-targets/all-features check、Clippy -D warnings、rustdoc、暂存后原合规及diff检查退出0。两个独立Standards/Spec报告均0源码发现；Spec没有独立检查此前结构RED/GREEN工具输出，不把报告补造为实测coverage。Windows原docs checker执行0xC0000005崩溃、初次4未跟踪文件合规拒绝仍封存；同checker/命令/源码的333存在性检查复现退出0，不声称原生崩溃已修复。

公开23文件增量/日志包 `windows-critical-test-source-review-20261006.1` manifest SHA `6c3e62f4a3667c877ce23eb26c658565f0f5f4560b442f334f76b175574749a5`。Root逐项长度/SHA/安全路径核对，9个canonical Git blob加规范eea基线独立重建1097blob tree；原生286B commit正文重算SHA-1精确cfdb，GitHub返回同tree/parent。61原body经独立formatter与新body匹配，生产LF前缀相同。6个checked raw相同，3个原CRLF前缀+新LF尾部重建真实source snapshot SHA；完整Git规范归档不冒称Windows全工作区构建开始快照。Root的三次核验器假设失败另存后按实际字节/trace修正，包未改、Rust suite和21控制未重跑。包外Mac ACK已由Windows实际读取核验；R2的41文件和751受控来源也已取得Windows独立ACK。

[现有security.yml运行37430519677](https://github.com/Northofqing/magic-market-data-rs/actions/runs/37430519677)创建2026-10-06T07:34:35Z，workflow_dispatch、head_sha精确cfdb；Root API独立看到audit成功、coverage job112160058874正在生成证据。这个记录尚无测量JSON、原80/95结果或新的SDK制品元组。旧R2eea二进制不能配cfdb。Gate A设计规定实际发布门合格后生成新runtime候选，CI期间不提前创建/运行候选；Root提出的并行离线构建优化以该具体规则为限，未执行新的SDK build或绕门。

VM源码交接本轮已完成/idle，Root继续持有实际GitHub run/job观察句柄，结果出来后按已有授权续办原VM任务。下一最小切片取决于真实CI：失败保留JSON/日志并针对实际缺口修复；通过才冻结完整新SDK构建开始raw/Git/proto/工具链输入、正常release与实际OUT_DIR descriptor，另封新tuple。随后Mac新绑定/受影响测试/独立复核/新release和实际preview、精确双端维护窗口/activation人审、同实例业务RPC/bridge/生产观察仍必要。没有本轮新生产切换或完整M0–M7结论，原正式服务/认证/数据与78隔离保持。
