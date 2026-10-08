# 推送效果与历史数据完整性修复（2026-10-07）

> 当前执行范围见 [2026-10-07 用户裁定](2026-10-07-active-scope.md)：本修复保留合并、回填与生产观察；平台工程冻结，paper 沿既有引擎，研究改为周报。本文原验证与历史数据结论保留，旧 Financial/资金/预注册前置不再触发整个平台开发。

> 2026-10-08部署续记：本批修复及`3f9577ce3`历史范围/闭市保护已随最新行为源码`62477bdc1`的普通release安装，13:20 CST正式launchd monitor启动；保留Windows841及原Schema9，不捆绑冻结平台迁移。下文“尚未安装”是原验收时点记录，当前安装与真实观察见[最新接收端交接](2026-10-08-latest-windows-receiver.md)。实际预测验证pending61/deferred61，216窗口缺资格，4个原闭市日期明确拒绝；真实交易状态、价格带、issuer尚缺，未把空结果填成收益，也未应用历史坏价裁定。

本次对应用户“把这些问题全部修复”，依据原自用审计及真实运行库复核。后继源码提交 `2cd571578ec1f100b720a179bd9505e041081159`（23个源文件），开发分支为 `codex/platform-roadmap-implementation-20261002`，远端为 `stock_analysis`。本文区分源码修复、实际数据资格、历史裁定和生产部署；当前执行按上述范围裁定，历史完整平台未完成部分保持冻结。

## 1. 已完成的源码

| 原问题 | 本次处理 | 仍需验收的部分 |
| --- | --- | --- |
| 公告去重在上海 00:00–08:00 重启后丢失 | `ef00ac085`：保留既有 UTC `created_at`，上海日界转成 UTC 后同事务恢复/清理；失败回滚、旧表兼容 | 此源码尚未安装到正式 monitor，需精确 activation 与重启后观察 |
| T+1/3/5 列一直为空 | `8d79fe947`：真实交易日窗口、独立逐窗口成熟判断、原字段和方向保留、逐行事务/CAS；调度和人工 backfill 使用同一实现 | 原 76 行不能靠裸日线补成有效结果；缺逐日权威交易状态时保持 NULL/deferred |
| 胜率将不完整样本计入分母 | `8d79fe947`：只有命中值有效且收益完整、有限、可能的样本进入相应窗口；明确不是送达消息胜率 | 成交利润、成本后收益和因果效果仍各需原始证据 |
| 日线只覆盖 57 只且停更 | 新 `outcome_data`：从原推送候选与待验证预测计算完整范围，逐证券 keyset 分页；`backfill_daily --outcomes`；盘后独立有界采集，不依赖持仓非空；归因拉到的 admitted 日线落库 | 生产数据尚未回填；上游最新窗口 API 不等于任意历史 through 查询；缺 bar/资格均显式失败，不写 0 收益 |
| 日线与交易状态未绑定 | 新 typed writer：一根 admitted 日线对应一个独立同证券、同日期的 `QualifiedTradingFacts`；原始来源/批次/时刻持久化，一事务写 bar、状态与旧停牌标志；普通 bar 重写使旧资格失效 | 当前生产 gateway 返回 `trading_fact_contract_not_delivered`。真实来源、修订/冲突规则和 issuer 仍需 Windows 交付，不由 OHLC/量或缺 bar 推断 |
| paper 信号价与报价同源自证 | 新私有字段价格资格：取锁前取得独立同证券、同交易日价格带；锁内对信号价和报价检查带宽/最小价位，坏价不写成交/现金/事件；已提交终态先恢复 | 生产价格资格合同尚未交付；旧 Filled 不受此增量检查自动裁定 |
| D01 新闻结果只有旧演示样例 | 新 owner 只读 reader：原 counted envelope、实体/日期/内容/occurrence/hash、实际接纳回执及 terminal evidence；逐窗口使用精确收盘价和交易资格；盘后报告接线 | V1 原卡片没有发行者证明的新闻发布时间或原推送价格，报告明确为业务日收盘后的价格观察；不能制造“推送买入收益” |
| D01 证券与 counted ticket 可能不一致 | 修复 orchestrator 将实际 snapshot.code 传入；渲染实体绑定 counted scope，旧空 caller 采用渲染实体，冲突实体拒绝 | 生产安装后验证同一实体/原回执，历史冲突不会被改写成正确卡片 |

新闻报告写入运行工作目录的 `data/news_outcome_original_cards/<业务日>.<内容SHA>.md`，同内容复用、旧版本保留。数据库读取不创建新 receipt、迁移、reconcile 或 sink attempt。原日期分页每页最多 32 日，每次最多 128 日，超限返回明确的 continuation 阻断；不宣称已覆盖超限历史。实际日线、物理接纳和完整窗口分母分列；手工 Accepted、Uncertain、Reserved、纯归档和 raw `pushed_stocks` 不进入物理接纳收益分母。

## 2. 真实库结论与历史坏价

只读复核的是 `/Users/zhangzhen/.local/share/stock-analysis-runtime/data/stock_analysis.db`，不是仓库旧副本：

- 日线 5070 行/57 证券，最新 2026-08-21；`qualified_daily_trading_status` 为 0 行。
- `prediction_tracker` 76 行，所有 T+1/3/5 为空。单目标已完成 15 行，4 行命中；这不能证明主题/新闻的可成交利润。
- paper 2310 条 Filled；低于 1 元筛选出精确 19 条候选，其中 002463 为 17 条。低价筛选本身不是独立错误价格证明。
- 最后 Filled 为 2026-09-24。没有后续成交样本支持“9月27日以后都干净”的结论。
- 归档 13284 份、重复正文最高 150 份、有测试标记；归档份数不是物理发出/实际接纳数，不能自动删成真正送达记录。

候选原件：`.planning/2026-10-07-outcome-integrity-repair/historical-bad-price-candidates.json`，SHA-256 `83e7216d6692aa35449718e46c69ad97903e0a342a708ae734e76c1089650662`。记录原 row、只读 SQL、观察时刻与诊断 hash；该 hash 不是 PaperLedger `FillFingerprint`。候选没有可用账户 binding 或裁定指纹，原库未找到 paper seed/head/event，且 19 条未找到对应 `order_audit.business_order_id`。

存量处理顺序：独立同日价格依据 → 实际资金 B/seed、代际与 Financial owner → 原不可变 fingerprint → `preview_adjudication` 给出 FIFO/依赖卖出、费用、历史/当前账户影响 → 精确显式裁定。沿既有 append-only adjudication，保留原 Filled、终态、审计和版本。没有执行批量 UPDATE，也不把 7月14–16日全部 Filled 置 Invalidated。

新增全局 SQL `Filled => fill_price 非空` 约束仍需版本化 Catalog 迁移与原件保全验收。既有表/trigger 是冻结 legacy DDL，不能在启动路径原地改写其身份。当前 controlled terminal/成交 writer 已拒绝缺失或非法 Filled 价格；本片新增的是独立信号/报价资格，没有伪称已追加 SQL CHECK。

## 3. Windows 交付与明确阻断

已有用户授权继续协调同一个 Windows chat：**R08 FuturesDelivery 上游合同与部署**，thread `01a0e0cf-2276-7512-96ee-3a94bdfa8ca5`。

- 已向 Windows 交付 `mac-outcome-data-diagnostic-20261007.1`，manifest `a4617473d47b5e69660207fe44fe0892e07d71b29dbad4a393b4b0afcf30a1a4`：2702 原候选/1076 证券、逐候选窗口/缺口、原 SQL/DDL、原日历、实际 Health 与哈希。最晚候选 T+5 为 2026-10-09。SQL 缺口是实际观察，RPC 标记为未执行，不伪装成调用失败证据。
- Windows 核验诊断和两个 ACK 后确认真实逐日来源仍无，行情陈旧原因尚未定位，未切服务。已续办来源准入、合同设计/实现与同版 RPC，不因“能力扩展”再次等待范围确认。付费/凭据/许可或来源人审必须提出具体材料与输入。
- SDK `7174f09f0b34d082bc584180ba4260d53f379d6c` 已提交并推送；真实 CI run `37513568982`：overall 84.96% 达 80%，critical 88.89% 未达 95%，audit 通过，checker exit 1/workflow failure。不能降门槛或把该候选当运行资格。
- Bash `0xC0000005` 无根因修复；多次偶发完成不等于通过。原 CI/诊断共享包均已逐字节核验并 ACK，无需重复索要 SQL。
- 当前 Historical/TDX 不提供真实历史逐日 Trading/Suspended authority。CurrentAuction 的当前 suspended 标签没有对应历史日期/source_at，不能作为历史资格。

本轮真实 monitor `--health --json` 返回 exit 1，2026-10-07 08:44 CST 快照为 `unhealthy / Frozen / Unsafe`，账户指标不完整。只证明该时刻局部健康状态；未刷新/重裁之前的 78 条 Uncertain，未把旧快照当作当前条数。

## 4. 统计与策略处理

保留逐笔事实，排除测试/未送达/不完整窗口等不适合对应分母的记录。观察涨幅、新闻因果、可成交收益、成本后净收益和退出规则有效性分别需要证据。原粘贴文本的 FIFO 剔除口径没有可复核脚本/原件，本片不复述其“正期望龙头”“首板必亏”或“止盈已验证有效”作为事实。

没有据此将主力/放量权重归零、拉黑主题、增加连板权重或生成自动买入批准。当前以可靠、可追溯事实做周报复盘，保留成本、样本缺口与自然成熟窗口；不自动调整策略。原治理 Draft、预注册与正式晋级工程已冻结。

## 5. 验证与提交

已提交且远端完整 OID 回读一致：

- `ef00ac085`：去重日界 2 项通过。
- `8d79fe947ba9af0c87156b40b73afb080c8a6d81`：最终预测验证 35 项＋胜率完整性 2 项通过。首轮旧读错计数断言失败原件保留，修正断言同时验证主目标和新增窗口各自错误。

后继库验证最终 **89 个不同方法通过，0失败、0 ignored**：paper 66＋日线16＋范围2＋D01 3＋两个实际 Catalog/迁移入口。库 harness `6e39872929df19468be72048dde4987576d18b60594b1ffd953a3436f530a09f`，940件输入前后不变，manifest `e5d6ec6000529954c5dc09b28fb09b50bc96940787edbfdec1768493a20e403c`。paper log `59b466911d8811d1d2f2d4e95f02980c53598094929808eb4483a4b7eb9dbc24`，D01 log `54d4da4eb424c55f46f43cc016e8433eac25410587e1458588b89d085cdb45ce`。其后仅 monitor 四个源文件的接线/格式改动，库输入保持；bin 入口与 dry-run 证据另列，不能混称同一份源清单。monitor：D01 实体回归1项、相关行情5项、调度15项通过；`backfill_daily` 3项、`backfill_predictions` 4项通过。后继批次合计117个不同定向方法通过，分属上述库/入口清单；未运行全量。bin清单940件，manifest `9e58244dc153e4fbe4a452c2cb6c88d5ea71abf49dbf11bf143225f66879246c`，monitor harness `05eef3fe535981aa76210271d76988e999f94293438df2a85d06d3c3cab4005d`；monitor log `5bb19b62ec498e32ad7efd0c190a00b9c10ca4d7d16e8d1c95b418a6e17d3528`。实际 `monitor --test --push-dry-run` EXIT 0，940输入前后相同，debug binary SHA `77f377fe9ff7b373b6d98c4bfa7f88c0112627219f5fa7d202600ca70443853b`，log `411ded8e5b020089e6cc49d1211a6a411f74674e257317c14766f014d87f6a80`。该 debug/隔离 smoke 明确跳过8个缺绑定的 counted家族（包含D01）；启动 Health 报 perf_recent=false，也没有生产SDK/投递验收资格。测试与dry-run编译于提交前的精确工作树，内容清单对应新源码提交，不将debug二进制的编译元数据冒称release身份。

首轮 paper 66 项为 65通过/1失败，独立测试带先拦住原金额溢出夹具，已拓宽明确 TEST_CODE 隔离带，生产资格保持不变。全部首失败日志/receipt 保存在 `.planning/2026-10-07-outcome-integrity-repair/`，不累计重复检查为新增方法数。

按 requesting-code-review 直接复核源差异和真实测试；本轮委派规则不允许新增独立 reviewer，没有独立 Approved。未执行全量测试、release、生产数据回填、历史裁定 apply 或生产切换。

## 6. 接续顺序

1. 本批最小充分验证及源码提交已完成，修复已纳入 master；先核实际 Git 身份，避免重复合入。合入不等于生产安装。
2. Windows 交付真实逐日来源/issuer、历史范围与修订合同，critical CI 达标后取同版本真实 RPC/原件证据，Mac 接纳验证；Health 不能代替业务合同。
3. 准备本修复必要的新精确 release/配置/activation 审阅单和兼容回退；批准后重新检查单实例、durable/Uncertain、同版数据，再切正式运行根。避免捆绑冻结的平台迁移；旧 Wave 批准不能复用于本批。
4. 数据资格及回填运行版本具备后，实际回填 1076 证券的原始日期范围，核对逐日状态、缺口与原推送窗口；再运行预测窗口和 D01 原卡片反馈。未成熟/缺行情/停牌保持显式 deferred。
5. 历史坏价逐笔裁定及 Catalog 约束中依赖 H01–H03/正式资金/Financial 的部分单列延后，保留原件与候选状态；现有 paper 防护和可靠统计可独立推进。
6. 取得重启日界、真实接纳/幂等、独立价格防护及自然 T+5 的生产观察，按周报列出可靠结果、缺口与后续动作。自然观察和缺失源不是离线测试可以替代的完成条件。

当前执行以 [范围裁定](2026-10-07-active-scope.md) 为准；[平台剩余任务](2026-10-06-platform-remaining-work-handoff.md) 和 [H02 V1 接续](2026-10-07-v1-content-development.md)保留为冻结设计及历史证据。
