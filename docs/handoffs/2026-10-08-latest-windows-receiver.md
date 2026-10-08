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

最终行为源码 `62477bdc1` 已合入并推送 master。259 个不同定向方法通过；保留首个失败、两次因审阅补项主动中止的 release 输入和日志。普通 release 配置未改，从 Desktop 外 1710 个精确 Git 输入构建，前后字节保持；monitor SHA `afa1bf8b4f64f8c67374a9006d90d176f6a18c1f2a64b3a2244970f9454b36c2`。同制品 59 家族 dry-run 和独立 shadow launchd 均退出0，物理进程和回执写入均0。第一次并行 shadow 撞到同测试根实例锁，原失败保留，随后串行通过。

实际 Windows 新制品探针 Health live/ready，编译身份匹配841；证券信息、个股新闻及财联社/金十/澎湃实际记录接纳。静态9路尝试8路ready；东方财富继续显式 source_precondition_failed。探针的 family opening ready/退出0不能据此声称四家全部接纳或CI门禁通过。

正常 release 的非 test 库重开原两库一致副本：主库审计初始化通过；Schema9 全19表数据、93对象、49策略及5073条投递状态逐字节逻辑保持（1001 Delivered、3988 RejectedDurable、6 ManualResolvedRejected、78 Uncertain）。主库普通初始化只新增当前P01必要的generation表、索引和三个失效trigger，旧P01行无generation回填；未安装P05 prospective storage。生产 P05/G5bv2启动与tick及P05 startup owner检查已明确冻结，隔离平台回归仍保留。

13:08新版源、配置、合同和二进制已装入原运行根，两库dev/inode保持；activation hash `955ef7c37426adf87d4eb4add7444ef3ecb11a40be960a0b91454e866fb36af5`，13:17:42 CST生效。桥接保留原修复制品 `a13ed075…`，13:15:52完整审计就绪，PID16315。正式monitor由launchd于13:20:29启动，PID17439，13:21:45原库初始化完成，13:21:46投递恢复fixed point：resumed_sink_calls=0、manual_review_boundaries=78。生产日志确认P05 Unit及G5bv2均user_scope_frozen，provider/sql/file operations=0。

13:22:18正式monitor四家GlobalNews及证券身份请求出现真实accepted，CLS/金十具有新鲜内容。该批EM成功并不撤销12:59探针的原生域名失败：含insurance.eastmoney.com的特定批次资格仍未修复，也不能把成功批次推广成所有文章接纳。13:24只读Health：进程/heartbeat/snapshot新鲜，账户Frozen且metrics不完整；data由启动Unsafe改善为Degraded，缺Kline/MoneyFlow；四源raw recovery均closed/成功。Health整体仍exit1，不宣称交易或全部数据健康。

13:21:53自然SnapshotStale告警物理接纳，飞书message `om_x100b6346c9ed28b8c45f1f0bddfbff5`；13:23平台GET逐字节正文匹配（223字节，SHA `7e75da79ac5068a4077e94f4187aa7add31dd1e564583ff82b3f89e36284a9bf`），目标为原配置Stock机器人p2p，read_users=0。这是新版正常路径平台回读，不是手动测试；无法据此声称用户客户端收到。尚未观察新版新闻聚合的物理发送，下一原定窗口15:00；不回放已过时段、Delivered或Uncertain。

13:24生产原Schema9目录93对象和49策略完全不变；原5073条decision逐行全字段保持。新增自然告警使Delivered增至1002，原78 Uncertain未重发。主库六组原NewsAI审计/卡片前缀逐字节保持。outcome实际运行报告pending61/deferred61，216个窗口缺资格，4个原候选落在闭市日期而明确拒绝；未制造T+1/3/5结果。

Windows新H08观察包`windows-h08-841e4ae-observation-20261008.1`另行核验12项，manifest `f8dc4e50f13e962d7e39ba193bf22969d90ba3c4fb263ac46c452b9f1ed74fe2`，包外写入Mac ACK并续催原Windows任务。000001指定区间28条为只读样本，不建立来源穷尽、PIT、逐日交易状态或同日价格资格；原1076证券缺项继续保留。

证据目录：`~/.local/share/stock-analysis-candidates/latest-windows-receiver-20261008`，重点为`production-start-readback.json`、`feishu-after-start-readback.json`、`production-health.stdout.json`、release及canary receipts。一次诊断read_users缺参数HTTP400保留独立失败回执，补齐参数后GET成功；没有因此重发消息。

Windows 原 CI 关键覆盖率 89.40%/95% 未通过；窄消费者兼容验收不能改写原 release gate。账户原日期、价格/日线/停牌资格及 Uncertain 保持原事实。用户是否查看Stock私聊或群聊仍待明确；没有擅自改收件人。
