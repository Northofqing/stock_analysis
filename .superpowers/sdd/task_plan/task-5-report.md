# Task 5 实施报告（源码完成，带验证限制；未部署）

基线：`eea99a71b8e10604e9c80ed6196f024c7d9ef585`。唯一实现者；未操作生产库、部署、发送或历史裁定。

状态：DONE_WITH_CONCERNS，提交供独立 review。Task5 源码范围已实现并完成限定验证；一项既有 activation 锁竞争测试在混合运行失败、隔离重跑通过，完整记录保留于最终表。没有将 Task10 生产迁移/授权裁定/上线验收算作已完成。以下 RED/GREEN 过程保留当时状态，以最终表和运维边界为准。

## 已确认设计边界

- 裁定使用原 `paper_ledger_event` 的版本化 payload 与原 head/CAS；不新增现金账本或 DDL，不改变 CatalogV1/V2。
- 原始链验证与经济可用性分离：依赖不满足时记录 unavailable，不伪造零仓位；仍可恢复原 terminal，追加 correction 恢复经济可用性。
- 根 agent 确认：无 account 的合法旧库只读 LegacyRaw/as-known；绑定 account 后显式 Epoch/LegacyBeforeCutover。历史域只到 seed 冻结 high-water，不能改变 seed/current cash。无终端的旧行显式 LegacyNoTerminal，不伪造 audit hash。
- 历史选择用显式 `AsKnown{head/cutoff}` 与 `RestatedLatest`，不能以布尔值或默认当前偷偷重述历史。旧库 `LegacyRaw` 不称为 restated。

## 垂直切片 1

RED：`cargo test --lib effective_fill_quarantine_reverses -- --nocapture`

- session 2548；编译 3m44s；0 passed/1 failed。
- 真实行为失败：`EvidenceUnavailable("adjudication not implemented")`，在 adjudicate 调用处失败，并非编译错误。
- 中间一次只读 ps 提权等待被中断；Cargo 已自行结束，后续回收原 session，没有重启同一 RED。

GREEN：同一命令，session 11962；编译 3m46s；1 passed/0 failed。

- 覆盖隔离后现金/费用/库存恢复、raw Filled 原样保留、重复裁定幂等、旧成交 command 仍恢复原终端。
- 这里只证明首切片；其余 brief 验收、消费者迁移与最终验证尚未完成。

## 垂直切片 2（测试运行中）

新增真实行为测试：在首次捕获 source fingerprint 前篡改 raw Filled，必须因与 immutable event/audit 矛盾而拒绝；200 股买入、100 股卖出后 quarantine 必须 unavailable，仍可恢复原 terminal，再 correction 恢复，且旧卖单 CAS 不得成交。手算更正价格 9 元后的现金 99,388.80、总费 11.20、已实现盈亏 291.30、剩余买费 2.50。

RED 命令：`cargo test --lib effective_fill_ -- --nocapture`，session 45538；3m45s，4 tests，2 passed/2 failed。

- 原行被改后竟能重新捕获指纹；更正重建把名称写成代码。二者均为真实行为失败。
- 修复增加 raw ↔ immutable event ↔ terminal 精确校验、保留原 buy lot 名称，并消除裁定重放的递归调用。
- GREEN：同命令，session 34127；3m43s，4 passed/0 failed。
- 部分卖出手算值、unavailable 后 correction 恢复、raw terminal 恢复、陈旧卖单 CAS、旧 version 恒等均通过。

## 垂直切片 3（测试运行中）

RED：`cargo test --lib effective_fill_ -- --nocapture`，session 24430。

新增单读事务 receipt/as-known/reopen 行为，以及裁定 commit-unknown 同 request 恢复。统一 set Interface 已有明确 scope/history 类型；尚未实现的 reader 以 typed unavailable 作为 RED seam。

- RED 实际输出：3m46s，6 tests，4 passed/2 failed；未实现 reader 与未路由公共 commit-unknown seam 分别失败。
- GREEN：同命令 session 37795，3m48s，6 passed/0 failed。
- 裁定进入公共 `PaperCommand` 事务；epoch 有效集冻结来源/账本 head/裁定 lineage，费用证据绑定同 projection hash，旧 AsKnown 与重开恒等。

## 垂直切片 4（实现后验证中）

- RED：同命令 session 89933，3m49s，9 tests，6 passed/3 failed：LegacyRaw 未实现、LegacyBeforeCutover 未实现、user_version=999 错误接受。
- GREEN 命令同上，session 71627：3m51s，9 passed/0 failed；测试 1.09s。根 agent 复用产物再次运行 effective_fill：9/9 GREEN（1.11s）。
- 最小实现：复用现有 attribution 原始来源/terminal 验证，新增 frozen legacy prefix 入口；historical 裁定记录进入同事件链，绝不调整 current seed/cash。
- 同事务实际读取 PaperLedger 命名空间对象/DDL/index/trigger 与 generation，精确比较冻结 V1 扩展，receipt 记录实际对象 hash。whole-app catalog/authority 仍归原全局 owner/Task10，不由第二连接拼接资格。
- Task5 夹具显式声明 application_id=1398035265/user_version=2；未更改全局 helper、冻结 catalog 或生产启动迁移。
- 设计停留过久曾延迟推进；不是环境阻塞，随后按已裁定最小方案落码。不能将本记录视作消费者已迁移。

## 垂直切片 5：PaperSell 与经济消费者（进行中）

- RED：`cargo test --lib effective_fill_sell_candidate_cas -- --nocapture`，session 5930，3m51s，0 passed/1 failed。
- 真实失败：读取候选后发生同数量改价 correction，runtime 重新读取最新 head，抛弃原候选依据，竟产生 Filled @12；不是编译失败。
- 接入 `InventoryCheckpoint`：候选带绑定/version/event hash/inventory fingerprint，经 PaperSellReadIo 传入 runtime。恢复原 terminal 仍先于 checkpoint；新提交要求候选与当前 head 完全一致，再执行原写事务 CAS。
- 当前 `cargo test --lib effective_fill_ -- --nocapture` session 80215 同时验证上述 GREEN 和新经济消费者的首个 RED；未回收最终输出。
- session 80215：3m46s，10 passed/1 failed；PaperSell CAS GREEN，唯一 RED 为新 effective economic seam 未实现。
- 实现冻结 set 同时提供 rows/costs/report，monitor 与 account_metrics_probe 移除两次查询混版本路径，报告保留完整 projection receipt。
- GREEN：同命令 session 31670，3m44s，11 passed/0 failed（测试 1.14s）。

## 垂直切片 6：snapshot revision 与 R12（进行中）

- 根 agent 同意同 event chain 的 DerivedSnapshotV1：财务 no-op；stable economic source head 排除 derived 事件；完整读取 head 只作 provenance；date+projection+algorithm 幂等；旧表只读。
- 新 RED 测试覆盖重复计算/重开不增 revision、旧快照 bytes 不变，以及 correction 后 R12 使用原 fill ID 与新版经济价格。
- RED：`cargo test --lib effective_fill_ -- --nocapture` session 29325，3m41s，11 passed/2 failed；新 snapshot/R12 seam 明确未实现。
- 实现追加 DerivedSnapshotV1、stable economic head/hash、旧表不写、精确 micro-CNY FIFO 分摊买费与每卖单净损益；R12 从同 set 读取并保留 projection receipt；health 检查业务日+当前有效 revision。
- 首次 GREEN 尝试 session 10743 未运行测试：移除懒建表函数后 performance/mod.rs 留下旧 re-export，1 个编译错误，已修正后重跑限定目标。
- GREEN：session 49299，3m52s，13 passed/0 failed（测试 1.29s）；根 agent 复用产物 13/13 GREEN（1.38s）。

## 垂直切片 7：归因 restated（进行中）

- 根 agent 确认归因 epoch selector 与 PaperLedger scope 正交；旧 terminal/Observed fee 不可被 corrected values 替换，新报告使用独立版本化 result identity 与显式 economic anchors、Scenario 费用、原始 terminal lineage。
- RED：`cargo test --lib effective_fill_attribution -- --nocapture` session 30149，3m57s，0 passed/1 failed：effective attribution seam 未实现。
- 最小实现从同 VerifiedEffectiveFillSet 计算位置/费用/周期，使用现有 benchmark 对齐器；新 PaperEffectiveAttributionV1 结果保留完整 projection/lineage，旧 BR-251 codec/hash 未改。
- 下一轮同时验证净值 188.8→288.8、新 result hash 与旧冻结 bytes 不变，以及旧 raw loader 未显式 paper scope 的新 RED。
- session 99793：3m49s，1 passed/1 failed；经济 restatement GREEN，旧 raw loader 在绑定 PaperLedger 后返回 Ok(empty) 是真实 RED。已加同读事务 paper_scope_required 守卫，并将同一约束接到 active/exact 未显式范围路径；纯原始审计/prefix 验证保留不变。
- session 10380：`cargo test --lib effective_fill_ -- --nocapture`，同时验证上述 GREEN 与 P04 首个 RED（原始终端卡片遇 correction/quarantine 不应重新作为普通 Filled 发出）。
- session 10380：3m51s，14 passed/2 failed。P04 真实 RED：correction 后仍返回允许原 Filled 卡；归因已返回正确 typed PaperScopeRequired，但测试错误检查 Display 的 snake_case，改为匹配公开枚举。
- GREEN session 50786：同 effective_fill_ 命令，3m47s，16 passed/0 failed（1.39s）。P04 以原 audit hash 与 effective lineage 精确绑定，有 ruling 不重发原普通 Filled 卡；不可用与错绑定 fail-closed。runtime 接线另待 bin 验证。
- 新增真实 append-only attribution report owner 的幂等/旧 revision/derived snapshot 尾事件稳定性测试，先以未实现的 preview_effective seam 观察 RED。
- 根 agent 补充裁定：OpeningInventorySample 必须独立命名，账户财务照 seed lot 基准正常计算；策略样本不能伪造 seed buy/family/terminal；一笔 sell 混合 seed/new buy 时按 FIFO 数量及费用拆分可归因/不可归因份额，不能整笔丢弃。
- 尚未声称完成：归因持久接线、opening-inventory 策略样本、P04 bin 行为测试、运维 preview CLI、完整消费者 closure 和最终限定回归仍在推进。
- session 8366：`cargo test --lib effective_fill_attribution_append_owner -- --nocapture`，3m51s，0 passed/1 failed，preview_effective 未实现的真实 RED。
- GREEN session 78895：同命令，3m46s，1 passed（0.35s）。现有 append-only attribution report owner 实际写入/重用/追加 revision；snapshot derived 尾事件不影响 financial-frontier 结果身份，旧报告存储 bytes 不变，AsKnown 可恢复旧经济值。显式经济 scope 与归因 selector 正交；active/exact 需 cutover 日/high-water/seed carry 与归因边界精确对齐，否则 unavailable。无 benchmark manifest 时新版 realized-only 报告明确保留 benchmark unavailable，不伪造 Observed fees；提供 manifest 仍走原严格 reader。
- 正在运行 effective_fill_ 新 RED：opening inventory mixed sell 必须按 FIFO 分量及费用拆分；R12 将统一经济本地时间准确转为 UTC bar anchor。
- RED session 65924：同命令，3m44s，16 passed/2 failed；mixed sell 被误判超卖 100 股，R12 02:00 UTC 实际错误变为 10:00。
- GREEN session 58438：3m45s，18 passed（1.45s）。OpeningInventorySample 拆分原 sell 份额、一次费用、opening 396.30 / strategy 191.30 / account 587.60 手算证据；统一 effective 本地经济事实时间，R12 明确转 UTC。
- 下一轮检验下游对 opening 份额的完整展示：PaperSell 库存、snapshot 策略指标、attribution payload lineage；并验证 legacy 只改价不改事实时间。
- RED session 76341：`cargo test --lib effective_fill_ -- --nocapture`，3m42s，16 passed/2 failed；legacy 只改价格却把事实时间从 18:00 移至 10:00；opening 下游实际 (100股,10均价,587.60策略快照,无opening lineage)，预期 (200股,9均价,191.30策略快照,100股opening exclusion)。
- GREEN session 38166：同命令，3m41s，18 passed/0 failed（1.47s）。PaperSell、snapshot 与 attribution 共享 OpeningInventorySample；账户实现盈亏 587.60 独立保留，策略实现盈亏 191.30，opening 分量 396.30 不伪造成 BR-103 策略周期。legacy 指纹事实时钟明确按原存储 UTC 读取，价格更正不移动日期。

## 垂直切片 8：P04 与实际归因消费者接线（进行中）

- P04 session 2613 `cargo test --bin monitor effective_fill_p04 -- --nocapture` 未到行为测试：旧 R12 渲染夹具缺少新增 `effective_projection` 字段。补 `None` 后 session 99342 重跑同一目标；不将编译错误记作行为 RED。
- 接线前调用图：monitor 15:05 → `compute_epoch_daily/window` → 旧 `load_active_verified_fills_until`；attribution_backfill 同路径；strategy_attribution → `AttributionReplayRunner::preview/commit_with_report`。这些旧经济入口在已绑定 PaperLedger 时正确 fail-closed，但这不等于迁移完成。
- 最小目标接口：提取现有 `preview_effective/commit_effective` 实现为只依赖 DatabaseManager 的 `EffectiveAttributionRunner`，仍复用同一个归因 append-only store；旧 runner 的显式方法转委托，旧 codec 无变化。生产 monitor/backfill 仅在显式 PAPER_LEDGER_ACCOUNT_BINDING 下调用新版，缺省旧库保留旧入口守卫。strategy_attribution 提供显式 effective request manifest，要求 scope/history/as-of，禁止猜 account/cutover。
- 在线日/窗采用同一 effective report（窗口范围 + 当日子集），因此费用/策略周期/opening exclusions 共用 receipt。active/exact 继续严格校验归因边界对齐；未对齐 typed unavailable，Task10 建立资格。本任务不猜生产边界。
- 文件报告以版本化内容 hash 新路径追加，保留旧 `{date}.md` bytes；DB 报告仍由原 append-owner 冻结。P04 只允许未裁定原始 Filled 卡，有裁定显式抑制，不自动补发历史消息。
- P04 真正 RED session 99342：33.70s，1 failed；带 `.125` 的 terminal_at 被整秒 parser 拒绝（trailing input）。增加实际临时 SQLite join 后 session 76707：34.75s，2 failed，原始 native `09:31:00.125` 同样无法解析。
- P04 GREEN session 52898：36.36s，2 passed（0.02s）。带明确 PaperLedger audit 后缀的上海存储时间转 UTC，legacy 原时间字符串不变；业务日按显式 UTC+8 划分，不能随主机 localtime 偏移。
- 在线窗口首轮 session 8701：`cargo test --lib effective_fill_online_window -- --nocapture`，3m57s，夹具使用未持有 descriptor-attribution pool 的旧 runner 测试 manager，激活前置被正确拒绝，未到目标行为 RED。改用已有 `open_isolated_for_test` 实际 descriptor-attested manager 后同目标重跑（29690），不弱化资格。
- session 29690：4m06s、1 failed（0.38s），激活前置已通过，真正失败于尚未实现的 commit_effective_window。已实现 descriptor 单读事务的 EffectiveAttributionRunner、日窗同 receipt 与现有 report store，并接 monitor/backfill。
- session 4652：`cargo test --lib effective_fill_ -- --nocapture`，3m58s，18 passed/2 failed（2.47s）。文件版本 owner stub 产生预期 RED；在线测试进入 compute 但夹具 buy 的 `TEST_CODE_evidence` 不属于现有合法 entry family，经济引擎按 `economic_position.rs` 的 Unknown 拒绝合同失败。按系统化排错对比已通过样本，改为 `Momentum: TEST_CODE_evidence`，未放宽生产 family 校验；下一轮 90473 验证。
- 文件 artifact 实现 create_new + 内容 hash，重算追加新路径，同 bytes 重用；已有路径字节不同则 fail-closed，绝不覆盖旧日文件。若写入中断遗留不完整文件，后续明确报错待运维，不把部分内容当成功。
- GREEN session 90473：`cargo test --lib effective_fill_ -- --nocapture`，3m53s，20 passed（2.34s）。在线 active 对齐窗口实际 append 后 correction 从 188.80 到 288.80 元，重复重算重用 revision；旧日报文件字节保持不变。

## 垂直切片 9：实际 owner 闭包、原子失败与命令行

- RED session 5489：`cargo test --lib effective_fill_ -- --nocapture`，4m09s，19 passed/5 failed（2.82s）。实际 PaperSell owner 没有绑定统一 receipt；旧 reconstruction 绕过 scope；TEMP paper namespace 没有拒绝；原始 reason 前缀缩短仍能指纹通过；写入之后提交之前的注入故障未接 seam。双真实 SQLite 连接并发同 predecessor 已在本轮通过，仅一个胜者。
- 实现统一 sell owner 从同一 effective receipt 取得库存和 checkpoint；旧经济 reconstruction 增加范围门；TEMP 命名空间 fail-closed；原 reason 与 PaperLedger 后缀精确匹配；同事务写入后故障 seam 验证回滚。
- session 20349：同命令，3m47s，23 passed/1 failed（2.21s）。以上五项 GREEN；新测试的后半段首次到达并产生真实 RED：seed 尚未发生的 as-of 日期竟返回 Epoch 投影。新增 cutover 业务日边界，等待后续 scoped GREEN。
- 显式历史 CLI RED 命令：`cargo test --bin strategy_attribution --bin economic_position_probe effective_fill_ -- --nocapture`，session 38527；执行中。仅测试临时参数，不接生产数据库。
- session 38527：2m10s，economic_position_probe 0/1，真实 RED 为拒绝 `--paper-request`；Cargo 遇首个 bin 失败而停止。随后 `cargo test --bin strategy_attribution effective_fill_ -- --nocapture` session 36790：0.81s、0/1，真实 RED 为 `effective-replay` 不存在。
- 第一轮实现 session 70030 未执行测试：两个旧类型没有 Serialize。仅修正 CLI 输出边界，复用原 report receipt JSON 映射、明确输出经济报告与 opening/receipt，不改变旧类型 codec。
- GREEN session 4041：原双 bin 命令，5.93s；两个 bin 各 1/1。strategy_attribution manifest 必填、默认 preview；economic probe 显式 manifest 可达。随后补实际临时 SQLite 端到端只读 preview（最终验证执行），不仅依赖参数解析。
- `effective_fill_` session 45174：3m47s、26 passed/1 failed（2.10s）。cutover 前日期、原行缺失、恢复完整 schema 后链截断/未知 payload 通过拒绝验证；唯一真实 RED 是 generation=2/空 account 仍回退隐式 raw。旧 raw 经济入口新增 generation 门，显式 reader 仍负责完整 namespace 资格；纯 raw audit/prefix 不加经济门。
- 周末更正测试在 45174 意外提前命中“事实不得晚于裁定时间”，不算周末行为证据；已将 decision_at 调整到同一个周日，session 95125 验证。补 snapshot 不同业务日不能借刚写入旧日结果变健康的回归。
- session 95125：3m49s，26 passed/1 failed（2.07s）；generation 门与 snapshot 业务日 GREEN，真正 RED 为周末 fact 被允许成交。新增同一 `validate_fact_time`：preview/apply/replay 共用经过验证的 A 股交易日判断，不改变原始时间字段。
- 进入最终范围验证：`cargo test --lib paper_ledger -- --nocapture` session 6797，同时覆盖 Task3 既有现金/库存/CAS/幂等与 Task5 裁定。源码格式化仅限本任务新增模块及直接涉及的小模块，未运行全仓库格式化或无关全量测试。

## 运维接口与未部署边界

- `paper_adjudication_preview --db <显式路径> --manifest <完整 Adjudication JSON>` 仅只读 preview；不接受 `--apply`，不猜价格/股数，不硬编码历史 ID。金额 JSON 为 `micro-cny-half-up-v1` 的整数 micro-CNY，输出明确 money_model；后续运维不得把元的小数误填为该整数。
- `strategy_attribution effective-replay --db ... --from ... --to ... --paper-request <EffectiveFillRequest JSON> --epoch ...` 默认 preview；显式 `--commit` 只追加新报告，绝不应用裁定、修改成交或重发旧卡片。Paper scope 与 attribution epoch 分开声明，无法对齐即拒绝。
- `economic_position_probe --db ... --as-of ... --paper-request ...` 只读；回执包含 scope/history/as-of/lineage/opening，费用明确 Scenario。省略 manifest 仅保留合法旧 raw 诊断，不授权新账本。
- 本任务未新增 DDL/表或改变 CatalogV1/V2 冻结字节。裁定与派生快照复用同一 PaperLedgerV1 append-only event；旧表与卡片没有 UPDATE/DELETE。

## 自审与关注项

- 全量链回放/裁定验证仍为正确性优先实现，有多次遍历；未进行长历史/高并发规模压测。没有在无证据时声称满足生产吞吐。
- result file owner 使用 create_new + 内容校验；进程在写文件中途退出可能留下部分文件，后续明确报错，不覆盖或误报成功；数据库 immutable report revision 仍是权威 owner。
- 原始历史 pre-audit 行明确 LegacyNoTerminal；截图 seed opening 不伪造策略买入/terminal。需要完整策略起点的 BR-103/R12/归因样本排除 opening 份额，账户现金/权益/realized 独立保留。
- 未访问生产数据库、未 apply 历史裁定、未发布/启动/重启 monitor、未发送任何真实消息；实际生产状态不在本任务证据范围。
- 兼容边界：Task3 旧二进制不认识新增 `AdjudicatedV1/DerivedSnapshotV1` payload，应 fail-closed，不能在新事件落库后直接回滚二进制并继续写。Task10 必须完成旧 worker drain、备份/回滚和 owner 激活；CatalogV2 DDL 不变不代表旧代码理解新事件。

## 最终限定回归

| 命令 | session | 实际结果 |
| --- | --- | --- |
| `cargo test --lib paper_ledger -- --nocapture` | 6797 | 43/43；编译 3m47s，测试 8.76s；包含 Task3 历史回归、Task5 effective/correction、双连接并发、global catalog 冻结边界 |
| `cargo test --lib trading::paper_sell -- --nocapture` | 94997 | 5/16，测试 0.39s；3 个 runtime 测试缺 CatalogV2 声明，8 个旧聚合夹具任意长 TEST_CODE 不符合已存在的 BR-255 规范代码；未到目标卖出行为。只修隔离夹具；缺价改为断言严格 terminal facts 错误；T+1 采用 pre-audit 时期合法日期 |
| `cargo test --lib performance:: -- --nocapture` | 17491 | 111/111；复用编译 0.80s，测试 30.96s；economic/fees/snapshot/report/attribution/replay 及旧历史兼容 |
| `cargo test --lib trading::paper_sell -- --nocapture` | 8967 | 15/16；3m47s/0.34s；夹具修正后暴露真正 runtime 回归：A 成交推进 head，C 仍用整批初始 checkpoint 被 CAS 拒绝，只卖出首只 |
| `cargo test --lib trading::paper_sell -- --nocapture` | 21742 | 16/16；3m43s/0.32s。实际扫描后续逐只重取完整 effective 候选并重新评估，不把新 head 拼到旧决策；消失库存跳过，保留有界初始 code list、取消与提交 CAS |
| `cargo test --lib -- trading::paper_lot_ledger database::attribution_epochs review::backtest --nocapture` | 45108 | 105/106；0.87s。唯一失败 `activation_busy_is_retryable_and_never_claims_success_when_audit_is_locked` 报 database is locked；其余历史归因、FIFO、R12 通过 |
| `cargo test --lib activation_busy_is_retryable_and_never_claims_success_when_audit_is_locked -- --nocapture` | 14390 | 隔离重跑 1/1，0.77s/10.34s。未改该测试或其 activation 生产路径；保留首次混合运行锁竞争失败记录，不声称证明无 flake，也不把首轮整组写成全绿 |
| `cargo test --bin monitor -- effective_fill_ paper_trade health:: --nocapture` | 74139 | 10/10；2m34s/0.01s。P04 milli/native/legacy、terminal/card 回归与当前上海业务日 health |
| `cargo test --bin strategy_attribution --bin economic_position_probe --bin paper_adjudication_preview -- --nocapture` | 61313 | 18/18 + 1/1 + 1/1；6.63s，strategy 测试 0.16s。含真实 SQLite effective preview 字节不变、旧 CLI 兼容、裁定工具拒绝 apply |
| `cargo check --bin attribution_backfill --bin account_metrics_probe` | 12715 | 通过，1m22s；两个未被 bin 行为测试覆盖的实际工具只检查类型，不运行工具本身 |

关键源码位置（格式化后）：`paper_ledger_adjudication.rs:142` 源指纹、`:268` 同规则 prepare、`:318` replay 验证、`:408` 同账本经济重算；`paper_effective_fills.rs:175` 单读事务集合、`:367` exact catalog；`paper_opening_inventory.rs:91` opening/strategy FIFO 拆分；`paper_ledger_snapshot.rs:18` 追加 revision、`:75` 当日当前投影匹配；`paper_ledger_runtime.rs:225` 真实卖出候选；`attribution_effective.rs:100` 在线日窗、`:191` 显式历史 preview、`:335` 原 store 追加 owner。

最终闭包补检：`rg -n '\.effective_fills\(' src --glob '*.rs'` 只有 `paper_ledger_tests.rs` 的三个 Task3/兼容断言；生产经济调用统一使用 VerifiedEffectiveFillSet。旧 helper 的 raw terminal 引用不是新版 restated report 的 audit authority，不由线上消费者使用。`git diff --check` 当前通过，最后提交前再检查已暂存范围。

## 改动文件

- 新增深模块：`src/trading/paper_effective_fills.rs`、`paper_ledger_adjudication.rs`、`paper_ledger_snapshot.rs`、`paper_opening_inventory.rs`；`src/performance/attribution_effective.rs`；只读运维 `src/bin/paper_adjudication_preview.rs`。
- 同链接入与回归：`src/trading/paper_ledger.rs`、`paper_ledger_execution.rs`、`paper_ledger_runtime.rs`、`paper_ledger_tests.rs`、`paper_sell.rs`、`paper_sell_runtime_tests.rs`、`paper_trade.rs`（只新增原 terminal hash 读取接口）。
- 有效读取/历史兼容：`src/database/attribution_epochs.rs`；`src/performance/attribution_replay.rs`、`economic_position.rs`、`mod.rs`、`report.rs`、`snapshot.rs`；`src/review/backtest.rs`。
- 实际消费者：`src/bin/monitor/{main,health,market_data,push_templates}.rs`；`src/bin/{account_metrics_probe,attribution_backfill,economic_position_probe,strategy_attribution}.rs`。
- 本报告；没有更改 frozen schema/catalog、真实账户 stock_position、部署配置或 progress。

## Raw Filled 消费闭包（接线中，最终核验前不视为完成）

检索：`rg -n 'paper_trades|load_active_verified_fills_until|load_verified_legacy_prefix|load_verified_fills_until|load_scoped_epoch_rows' src --glob '*.rs' --glob '!**/*tests*'`，继续区分文件内 cfg(test) 与真实运行路径。

| 分类 | 保留 raw 的 owner / 入口 | 允许原因或迁移状态 |
| --- | --- | --- |
| 原始事实写入与取证 | paper_ledger_execution、paper_ledger_adjudication、paper_effective_fills、order_audit | 成交、源指纹、raw high-water/hash、完整审计链是不可改原事实；经济结果由统一 effective set 派生 |
| 幂等控制 | paper_ledger_runtime 原始一票一卖查询；decision/intraday_monitor plan-consumed 恢复 | quarantine/correction 不能撤销尝试已经发生的事实，更不能触发第二成交 |
| 失败审计 | paper_sell 库存失败时保留原来源行 | 仅用于 BR-249 诊断，不提供兜底库存、不授权交易 |
| 历史兼容与冻结审计 | database/attribution_epochs 的原 prefix/terminal/carry 验证；attribution_replay 旧证据 loader | 旧 as-known 验证必须保留原字段；经济入口遇已绑定 PaperLedger 且无明确 scope 时拒绝，不修改旧 epoch/remaining_quarantine |
| 经济消费者已迁移 | PaperSell、economic_position/BR-103/account probe、snapshot/health、R12、行情 coverage | 一次业务计算共用 effective receipt；seed opening 以独立样本呈现，不造策略 buy |
| P04 | 原 terminal query + effective disposition | 原卡片绑定 raw terminal；有裁定不作为普通 Filled 再发送，不让修正价冒充旧审计 |
| 经济归因已接线 | monitor/backfill 日窗归因；strategy_attribution 显式历史重算；economic_position_probe | monitor/backfill 显式 active binding → `commit_effective_window`；CLI 显式 scope/history manifest → EffectiveAttributionRunner / VerifiedEffectiveFillSet；未激活边界拒绝，不以 raw 兜底；最终验证结果见后文 |
| 不属于在线消费者 | paper_engine cfg(test) 加载器、paper_trade 测试写入、各种测试 fixture | production run_once 保持 fail-closed；本任务不改变真实 stock_position 或另一条真实交易 OrderStatus::Filled 路径 |

Task10 明确保留：生产扩展显式迁移/全局 catalog 重资格、一次性 seed/cutover 与旧 worker drain、归因 epoch 与 paper 边界对齐资格、历史 manifest preview 后的用户授权 apply/重算、真实部署与运行验收。本任务没有读取生产记录、裁定历史 ID、部署或发送消息。

## Fix round 1 — I01 历史裁定预览 / M01 目标日快照身份

固定基线 `afb43a40157ba7611846719fea35452d5f73f7f2`，依据 `task-5-review.md` 的 I01 与 M01；只修改这两个可复现行为，不涉及生产操作。

- I01：使用真实隔离库的 cutover 前 buy→sell，分别预览 quarantine 和改价；要求显式区分 current account 与 LegacyBeforeCutover 历史范围，历史依赖不可用与 apply/reopen 一致，preview 重复/重开零写，seed/cash 不变。
- M01：snapshot 的目标日内容身份与完整 source/CAS receipt 分层；完整源与链仍先验证，只让目标日以后无关成交退出派生缓存键，晚到且作用于目标日的裁定仍使旧结果失效。不得更改原 snapshot bytes 或削弱 execution head CAS。
- TDD 首条命令 `cargo test --lib effective_fill_legacy_preview -- --nocapture`，session 87493：测试编译发现 `LedgerError` 没有 PartialEq。仅将断言改为匹配 typed error/reason；此条不是业务 RED，后续需真实执行用例。
- 同命令 session 11919 仍未到行为：匹配 guard 不能移动 String；改成 `as_str/as_deref` 借用比较。保留两次测试作者错误，不把编译失败充当业务回归证据。session 89091 再执行同一用例。
- I01 真正 RED：session 89091，3m47s 编译、0.24s 测试，0 passed / 1 failed。历史 quarantine preview 实际仅返回 `cash=100000000000, fees=0, unavailable=null` 和当前账户 hash，没有历史影响。最小实现保留原 `AdjudicatedFact` bytes/schema，preview 明确返回 `current_account` 与 `historical_scope`（scope、LegacyEconomicV1 before/after hash、同 apply 的依赖失败原因）；CLI 输出显式复用这份结果。
- I01 GREEN：`cargo test --lib -- effective_fill_legacy_preview effective_fill_ruling_failure_after_writes --nocapture`，session 9534，2/2；编译 3m47s、测试 0.77s。历史 quarantine/改价、零写/重复/重开/apply 一致，以及原 epoch preview/事务失败均通过。
- M01 真正 RED：`cargo test --lib effective_fill_snapshot_period -- --nocapture`，session 39493，0 passed / 1 failed，测试 0.26s；次日普通买入使昨日 `current_snapshot` 无法返回原 revision。新用例同时要求无关未来改价不影响昨日、相关晚到改价失效并追加、旧 AsKnown bytes 可恢复、重开幂等，以及未来原始行被篡改时昨日查询仍失败封闭。
- M01 最小实现：完整 `projection_hash`/raw high-water/head/inventory/lineage/CAS 合同保持不变；仅 snapshot command 使用版本化 `PaperSnapshotPeriodInputV1`，绑定显式 scope/as-of/cutover/rules/catalog、目标日前经济行、原始或更正后事实日期在目标日前的 lineage（含隔离）与 seed lots。先完成全链取证，后按日计算结果身份；晚到的相关裁定仍进入该身份。
- 兼容性：原 `DerivedSnapshotV1` codec、result hash 验证和既有 bytes 不变；command 前缀版本为 `derived-snapshot:period-v1`。pre-fix 旧键结果保留可审计，新 owner 首次 settle 追加新缓存合同的 revision，不自动覆盖或部署补算。
- 最终 lib GREEN：`cargo test --lib paper_ledger -- --nocapture`，session 36228，45/45；编译 3m44s，测试 8.67s。覆盖 I01/M01 新行为、旧 DerivedSnapshot 尾事件稳定、源篡改/未知 schema/链截断失败封闭、并发/原子性/陈旧 CAS、opening 与已迁移经济消费者。没有重复无关全量测试。
- 最终 bin GREEN：`cargo test --bin monitor --bin paper_adjudication_preview -- health:: tests::output_preserves tests::explicit_paths --nocapture`，session 21837，monitor health 3/3 + preview CLI 2/2；编译 2m38s，测试均 <0.01s。CLI 不仅检查参数，也断言真实输出映射保留历史 scope/hash/依赖诊断、当前现金可用与 preview_only/applied=false。
- 最终 `git diff --check` 通过。lib GREEN 后仅对新增测试做空白/换行整理，没有再次修改行为，也不因此重复编译。

### Fix round 1 自审与交付边界

- I01 当前账户输出位于 `src/trading/paper_ledger_adjudication.rs:105`；历史 hash 直接使用与 apply 相同的 `historical_projection`，不从当前现金伪造。新公开类型仅用于 preview，持久化 `AdjudicatedFact` codec 未变；CLI 映射在 `src/bin/paper_adjudication_preview.rs:55`。
- M01 目标日身份位于 `src/trading/paper_effective_fills.rs:76`，原始/更正后日期选择在 `:293` 与 `:709`，消费在 `src/trading/paper_ledger_snapshot.rs:103`。所有 `verified_on/load` 完整验证仍在查缓存之前；没有修改 execute/sell CAS。
- 回归位于 `src/trading/paper_ledger_tests.rs:804`（跨日、相关/无关晚到改价、旧 bytes、重开、未来原始行篡改）及 `:1402`（历史 quarantine/改价 preview/apply/reopen、seed/cash 不变）。旧 epoch preview 同时继续通过。
- 本轮只改上述六个 Rust 文件（含公开 re-export 的 `paper_ledger.rs`）及本报告；progress 仅追加状态，不纳入实现提交。未修改 schema/catalog/部署配置或生产数据。
- 未执行 production legacy adjudication、旧键 snapshot 补算/新 owner 激活、长期规模测试、真实 monitor 运行验收；这些仍留 Task10。原报告记录的混合 activation busy 测试 flake 本轮没有扩大重测，也不声称已消除。
