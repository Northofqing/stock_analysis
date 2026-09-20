# 盘前产业链与 CLI 产业链：实际调用链和迁移约束

日期：2026-09-13。范围：隔离开发树 codex/push-reliability-20260905，HEAD b3742abab641565bab9b6893b4fdb75bb81261de 加当前未提交改动。本页只记录源码事实与条件反例，未运行 CLI/monitor、查询生产数据、调用真实来源或发送消息，不是生产故障复现或迁移验收。

对应当前目录的两个独立单元：[MU-cli-chain 与 MU-chain-preopen](push-current-capability-catalog.v1.json)。目录定位分别为11198/11215行；盘后单元见[此前盘后调用链](chain-post-close-call-chain-2026-09-10.md)。三者共享分析函数，但执行时机和完成状态分别归属 CLI 调用、盘前日期门、盘后日期门，不能合并成一个已迁移单元。

## 1. 盘前入口：09:05 ≤ 本轮时钟 < 09:15

实际入口在 [monitor/main.rs:9183](../../src/bin/monitor/main.rs:9183)。

1. [8663行](../../src/bin/monitor/main.rs:8663)取一次 Local::now；本轮多个任务之后，9190行使用同一个 now 判断 hour=9 且 minute∈5..15。旧注释的9:05–9:09已被后面的补偿注释和实际条件扩为09:05–09:14，不能只引用旧注释。
2. 9191行的 CHAIN_PREOPEN_LAST 是进程内 Mutex<Option<NaiveDate>>。9193行取自然日；9194–9198行比较当天是否已运行，之后释放锁。
3. 未运行时，9200行调用 run_chain_analysis_mode(true)。只有该调用返回 Err 才走9210行的保留重试分支；返回 Ok 后，9206–9207行写自然日。
4. 9215行等待30秒进入下一轮。该日期位不会在进程重启后恢复，也不是网络发送前的持久占位。
5. 本分支自身没有交易日校验。9221行的 today_is_trading_day 位于另一个 market_loop；[11060行](../../src/bin/monitor/main.rs:11060)将两个循环并列运行，不能把另一个循环的条件当作盘前分支的保护。此为源码边界，不声称非交易日实际已推送。

## 2. CLI 入口：命令分派与参数边界

入口在 [main.rs:25](../../src/main.rs:25)，参数在 [cli.rs:14](../../src/cli.rs:14)。

1. 先加载环境；30行调用既有 operator 检查；35行解析参数；61行校验启动配置；69行尝试初始化数据库，失败只记日志。这里只说明现有顺序，不执行真实认证/读取环境，不把复杂身份平台重新加入本次范围。
2. 79行先判断 --schedule 或 SCHEDULE_ENABLED=true，该分支返回后不会走后面的产业链分支。因此二者同时存在时，不能声称必执行一次产业链模式。
3. 85–88行才判断 --chain-analysis，唯一传入参数是 !args.no_notify。该分支位于默认股票列表及其他分析模式之前。
4. --dry-run 在 cli.rs:20 的帮助语义是“仅获取数据，不进行分析”；产业链分支没有检查或传递该参数。因此不能用 --chain-analysis --dry-run 作为保证不分析/不产生副作用的诊断命令。是否实际执行仍取决于前面分派、配置、认证和来源是否通过。
5. --no-notify 只关闭最后的发送条件，不跳过采集、分析、业务库写入或报告文件保存。它不等于只读运行。

## 3. 共享模式的真实顺序

共享实现：[app/modes.rs:106](../../src/app/modes.rs:106)。

1. 112–113行固定本次观察时刻和最近已完成交易日。实际[calendar.rs:600](../../src/calendar.rs:600)在交易日收盘后使用当天，否则回到前一交易日；盘前不是简单的自然日减一天。
2. 117–121行通过 spawn_blocking 创建 MarketAnalyzer 并按该 business_date 取涨停池；任务及业务错误均由 await?? 向外返回。
3. 129行请求财联社快讯20条，137–140行只取前15个标题并用分号拼接。Available空、VerifiedEmpty、Err分别记降级信息，最终都可用 None 继续；不得把它们在持久事实中混为同一种已验证空。
4. 157–162行调用同一个 run_chain_analysis。当前[mod.rs:464](../../src/pipeline/chain_analysis/mod.rs:464)委托准备流程；[preparation.rs:1029](../../src/pipeline/chain_analysis/preparation.rs:1029)仍使用 ProductionIo，不会自动选择新 LocalChainPostClose/v9恢复适配器。新存储测试通过不意味着旧入口已接管。
5. 164行构造通知服务。167–172行命名 chain_analysis_业务日_HHMM.md 并先保存文件；保存失败返回 Err，尚未开始通知。
6. 175–180行按 send_notify 决定是否发送。Ok(true)记成功；Ok(false)及Err只记警告。182行最终仍返回 Ok(())。

## 4. 发送与文件效果：已存在什么、缺什么

[NotificationService::send:151](../../src/notification/service.rs:151)调用 send_report 后取 has_success，并包成 Ok。当前已经保留逐目标的本次调用内弱观察，不能继续描述成“完全没有逐目标结果”；但 mode 只消费 bool，未保存详细结果。

[send_report:156](../../src/notification/service.rs:156)在无可用渠道时返回空报告；有渠道时按 available_channels 顺序处理，Custom的每个配置URL独立执行。[has_success:62](../../src/notification/send_report.rs:62)只要任一目标为弱 Accepted 就返回true，不证明所有必达目标、完整正文、用户已读或跨重启防重。

[save_report_to_file:345](../../src/notification/service.rs:345)使用 reports/，最终363–366行创建目录、拼接文件名、fs::write。当前相同业务日和同一HHMM仍可能生成同名路径，fs::write会覆盖；时分命名不能证明不覆盖。本页没有观察到真实碰撞，也不执行覆盖实验。

## 5. 已确认的条件反例及后续验收要求

- **盘前发送失败误封日**：所有渠道均未成功→send得到false→mode只warn后Ok→盘前日期位置当天。无需假定程序抛异常；当前显式分支就足以形成该反例。应让真实定时器区分准备/保存/弱发送/完成事实，不能只把最后Ok改Err后盲目重跑整个含副作用pipeline。
- **CLI失败退出成功**：相同false→Ok链让main打印执行完成并正常返回；文件存在不能当投递成功。后续命令结果应准确表达失败/未决/部分成功，不把弱Accepted升级成强回执。
- **过窗仍执行的条件**：now在8663行取值，在9190行才检查；如果中间工作跨过09:15，实际调用仍可能使用旧时刻放行。须在副作用开始前重读时钟，并区分窗口内新建与窗口外恢复；这里只证明代码允许该情形，未实测延迟。
- **重启与重复风险**：盘前自然日位不持久，且检查与标记之间包含整个await。当前单一intraday_loop是顺序执行，不能因此直接宣称单进程必然并发重复；但进程重开、多实例和CLI独立调用并无共同持久效果身份。以后应保留三个独立Unit身份及其原业务日规则。
- **报告原字节与来源恢复**：首次输入/源结果/模型/报告及文件效果需分别固定，不能仅保存最终字符串。异内容同名文件须显式冲突，不覆盖；发送未知结果须保留，不自动重发。
- **CLI诊断边界**：补齐 dry-run 的产业链分支语义需有独立测试和明确参数合同；不能把现有 --no-notify 包装成安全只读命令。当前页面不改CLI行为。
- **日期门边界**：自然日资格与最近已完成交易日不同；同一业务日可能被不同自然日/不同入口使用。必须明确通知 occurrence，不仅按报告业务日全局去重，也不能仅按进程日期位宣称完成。

单用户本地裁决仍以[范围文档](single-user-local-scope-2026-09-11.md)为准：外部可信身份/多角色审批不在本次范围，同库事务、任务锁、效果身份、来源校验和实际切换授权保持。盘后Task2的共享恢复基础是后续两个单元的依赖，不是它们已经完成迁移的证明。

## 6. 源码身份

下列九个文件在本页核对时与 local-position-concept-v9-success-first-implementation 捕获的 source_after 完全一致；该捕获只验证指定v9测试，不验证本页timer/CLI运行。行号均对应下表字节，不沿用冻结蓝图的旧行号。

| 文件（src/下） | SHA-256 |
| --- | --- |
| main.rs | 0d405c8e876ec09af5c3251b206c0b8519b59b463c40ff804bee0a677d0c6d6b |
| cli.rs | b86f6ea8abd70b666850c8576966c32c8f0653f4d34775d4fab75bb4d4072947 |
| app/modes.rs | 9ff92644f781ac2ccbfc5d673fe33ea1e64f25cf8d600792af9bbc3bd1888da5 |
| bin/monitor/main.rs | c87fa4bb56df1c786e11516f754316e0a3fb9d9f5f75aad8aa2a7cb02016488c |
| calendar.rs | 2a03f9e0114a13e820133faadcf2eb631b3ce4ff2424aea4830319ada0a5c3f4 |
| notification/service.rs | da4873e087ab6f239bbdaea98b7ff4f41db2f2c4902b1f92387aa1e49ef3f0d4 |
| notification/send_report.rs | ddf7bae2f39857b65cb7908cd56abf811708bdd1735a648f3a249bc83885d2da |
| pipeline/chain_analysis/mod.rs | 5f8e78a648e9e90c6f7fc68c78c2f6565fc9b56c12d4ba0f7e86cfa2c68b1a2e |
| pipeline/chain_analysis/preparation.rs | ba88960d1ae94c21f8da690c8ed93dc94119e33ed6fe8bd32b84d24c296a137f |

与[当前架构蓝图](../architecture/current/Project_Architecture_Blueprint.md)共同阅读：蓝图说明组件关系与其冻结审计时点，本页补当前实际入口；能力目录、候选存储、单例测试、生产接管是不同层次的证据。本页新增两条调用链核对，累计12条已追链/40条尚未逐链，不增加任何已迁移认证数量。
