# 归因样本纪元重置设计

> 状态：Gate A 书面设计待复核；未开发、未发布、未写入生产纪元。
>
> 日期：2026-08-28
>
> 业务规则：BR-255（本设计提交同步登记，任何实现代码必须晚于规则登记）。

## 1. 目标

在不删除、改写或伪造任何历史 `paper_trades`、`order_audit`、归因报告和模拟持仓的前提下，建立一个新的归因样本纪元。新纪元从下一完整交易日开始，只统计在纪元内完整形成的买卖闭环；旧账本继续作为只读审计事实存在，但其中已确认的 T+1 违规不再阻塞新纪元经济归因。

该切换由 `monitor` 在部署后首次满足安全收盘窗口时自动、幂等地执行一次。模拟交易引擎继续使用完整账本和原虚拟持仓，不执行清仓、重置或订单动作。

## 2. 问题证据

最新 `master` 的真实只读验证得到：

- `strategy_attribution scheduled` 在 2026-08-28 盘中按预期返回 `current_session_incomplete`；
- 对 2026-08-27 的显式 replay 通过已准入沪深 300 manifest 后，在 `trade_evidence` 返回 `paper_trade_source_failed`；
- `economic_position_probe --as-of 2026-08-27` 精确定位为 paper sell `id=520` 消耗了同日 buy `id=490` 的 100 股，违反 A 股 T+1；
- 898 条 Filled 纸面成交中共有 9 个卖单消耗同日买入 lot；
- 对应订单审计逐笔存在且来源可信，问题是旧模拟执行真实记录了非法成交，不是时区解析或来源伪造；
- `order_audit` 受禁止 UPDATE/DELETE 触发器保护。删除纸面行既违反至少五年留存，也会破坏成交与 terminal audit 的双向精确绑定；
- 旧账本仍有非零虚拟持仓，因此不能把切换点伪装成真实空仓。

结论：根因是旧模拟执行与归因合同不一致。修复不能删除事实，也不能让新归因从虚构零持仓开始；必须同时隔离旧经济样本和真实存在的旧持仓数量。

## 3. 已确认决策

- 只重置归因样本，不重置模拟交易引擎或虚拟持仓。
- 旧成交、审计和旧报告永久保留；CLI 默认展示活动纪元，legacy 访问必须显式请求。
- 新纪元在功能部署后的下一个完整、经验证交易日生效。
- 使用不可变高水位切换凭证，不给历史成交回填 `epoch_id`，不修改所有成交 writer。
- `monitor` 在首次安全收盘窗口自动提交一次，以后只验证，不自动产生第二次重置。
- 切换时建立“旧持仓隔离池”：只记录每个代码的剩余数量，不采用旧成本、不计算旧收益；有 carry 的代码从边界起整段隔离，直到其实际数量首次回到零。
- 新纪元样本不足现有 200 个闭环或 84 天门槛时保持 `InsufficientSample` / `ResearchOnly`，禁止用 legacy 样本补足。
- 归因错误不影响行情、模拟交易或订单运行，但归因本身必须保持 `Unavailable` 并留下失败审计。

## 4. 范围与非目标

### 4.1 本次范围

- 追加式归因纪元成功凭证、旧持仓隔离项和切换尝试审计。
- 受验证的下一交易日解析。
- 旧账本高水位、manifest、订单审计链 tip 和隔离池 manifest 的固化与重验。
- 统一的活动纪元读取 seam，供 BR-251 replay 与 monitor 日归因使用。
- 新报告与纪元的显式、不可变绑定。
- `monitor` 15:35–15:50 一次性自动激活与后续校验。
- `strategy_attribution reset-sample` 的只读预览和受同一规则约束的人工恢复。
- TEST_CODE 隔离测试、失败路径、合规与覆盖率证据。

### 4.2 非目标

- 不删除、改时、缩量、改方向或重写旧成交和订单审计。
- 不声称旧账本合法，也不使 legacy replay 变成成功。
- 不从旧账本推导旧成本、旧收益或策略有效性。
- 不清空或重建模拟交易引擎持仓。
- 不解除 `paper_sell_paused`，不修改订单、现金、仓位或价格安全。
- 不启用 Minute1、TechnicalBars、默认 benchmark 或任何 mock/fallback。
- 不增加新的推送类型；现有 monitor 日归因只能在取得活动纪元证据后沿原治理路径运行。
- 不修改 `config/*.toml` 或现有 200 闭环、84 天等阈值。
- 不授权第二次自动重置；未来若需要新纪元，必须重新进入 Gate A。

## 5. 旧模块关系

| 模块 | adopt/reject | 处理 |
| --- | --- | --- |
| `performance::attribution_replay` | adopt | 保留完整成交/terminal audit/benchmark/费用验证；经济重建输入改为活动纪元作用域。 |
| `performance::economic_position` | adopt | 保留新纪元 FIFO、T+1、超卖、费用和样本门；不用于证明 legacy T+1 合法。 |
| `performance::attribution` | adopt and narrow | monitor 日归因必须通过同一纪元 seam 取行，不得继续直读全部 `paper_trades`。 |
| `database::attribution_reports` | adopt | 保留现有 append-only run/report/failure 链，并给新报告增加不可变纪元绑定。 |
| `calendar::VerifiedReplayCalendar` | adopt | 复用 checked-in authority hash；新增 fail-closed 的 verified next-session 能力。 |
| `bin/strategy_attribution` | adopt | 新增预览/恢复命令和显式 epoch 选择，不复制 repository SQL。 |
| `bin/monitor/main.rs` | narrow exception | 只增加薄的 15:35–15:50 调度调用和活动纪元校验；复杂逻辑留在 library 模块。 |
| 旧 `paper_attribution_daily` | preserve | 不覆盖旧行；新写入必须绑定活动纪元，旧行显式视为 legacy。 |
| `paper_trade` / `paper_sell` / 订单路径 | reject | 不修改行为，不利用归因重置改变成交或持仓。 |
| 日期 SQL 过滤或用户提供空仓基线 | reject | 日期过滤不能处理迟到行；用户不能提供的权威空仓事实不得伪造。 |

## 6. 领域模型与不可变存储

### 6.1 `AttributionSampleEpochReceipt`

成功凭证至少包含：

- `epoch_id`：由规范前像计算的域分离 SHA-256；
- `previous_epoch_receipt_hash`：首个 BR-255 凭证可为空，后续能力当前不开放；
- `cutover_completed_trading_date`；
- `effective_trading_date`；
- `paper_trade_high_water`：切换事务内全部 `paper_trades` 的 `MAX(id)`，不是只看 Filled；
- `legacy_filled_manifest_hash`：高水位内全部归因相关 Filled 行的规范内容 hash；
- `order_audit_high_water` 和该位置的 canonical chain tip hash；
- `terminal_binding_manifest_hash`：高水位内 Filled paper 与 terminal order audit 的精确绑定清单；
- `legacy_carry_manifest_hash` 和 carry item 数量/总股数；
- `calendar_authority_hash`；
- `decision_basis="BR-255"`、canonical UTC `created_at`、精确 retention deadline；
- `receipt_hash`。

成功表使用 `INTEGER PRIMARY KEY AUTOINCREMENT`，并安装、逐字验证 canonical no-update/no-delete triggers。活动纪元由完整校验后的最新成功链确定，不使用可变 `active` 标记；读取必须验证全部 success rows，任何尾部缺失、漂移或断链都整次失败，禁止跳过坏尾后回退到更早纪元。BR-255 v1 的 monitor 只允许创建一个固定 activation domain；同内容重试返回原凭证，不同内容冲突失败。

### 6.2 `AttributionLegacyCarryItem`

每个切换时仍有数量的代码保存一条不可变 carry item：

- `epoch_id`、canonical `code`、`quantity > 0`；
- 对应高水位和来源 manifest hash；
- canonical item hash、创建时间和 retention deadline。

隔离池只证明旧账本在边界处记录的剩余数量。它不包含 cost price、buy date、PnL、signal family 或虚构 fill。构建器仍严格验证身份、方向、正价格、100 股整数数量、`(ts,id)` 顺序和累计不得超卖；唯一不在 legacy 阶段执行的是 T+1 经济合法性，因为该失败正是被隔离的历史问题。某代码只要 carry 非零，边界后的买入和卖出都进入该代码的 quarantine overlay；overlay 只维护严格数量连续性，直到总数量首次回到零，不拆分成交或费用。

### 6.3 `AttributionEpochAttemptAudit`

每次 monitor/CLI 尝试都追加：调用来源、invoked-at、完成交易日、生效交易日、outcome、结构化 reason code、retryable、成功 receipt hash（若有）、前序 attempt hash 和本记录 hash。成功 receipt 与 success attempt 在同一事务提交。

失败前像在原事务回滚后通过独立审计事务追加；若数据库身份、审计链或写入本身已不可验证，则不得声称已审计，返回 `epoch_attempt_audit_unavailable` 并保留结构化进程日志。日志不能代替本可正常持久化的审计。

### 6.4 报告绑定

旧 attribution run/report 不改写。新报告提交事务必须追加 `report_id -> epoch_id -> epoch_manifest_hash` 绑定；缺绑定的新纪元报告为完整性失败。没有绑定的既有报告只能显示为 `legacy`，不能自动重绑到活动纪元。

## 7. 切换数据流

1. `monitor` 捕获一次明确的上海时区 `now`。
2. checked-in 日历验证当日为真实交易日、当前时间位于 15:35–15:50，并解析下一真实交易日；盘中、周末、覆盖外或日历不可用均不切换。
3. 共享 epoch service 以 `BEGIN IMMEDIATE` 打开一个写事务，避免高水位采样与 writer 并发变化。
4. 验证纪元、attempt、carry、order-audit 及报告绑定表的精确 schema、触发器、sequence 和 hash chain。
5. 若已存在内容一致的 BR-255 成功凭证，返回幂等成功；若 activation domain 已存在但内容冲突，返回完整性失败。
6. 加载完整 paper/audit 来源，严格验证结构、身份、顺序、价格、数量、方向、terminal binding 和完整 order-audit chain；不对 legacy 运行 T+1 经济重建。
7. 计算全部行高水位、legacy Filled manifest、binding manifest、audit tip 和旧持仓隔离池。
8. 在同一事务内计算切换前模拟持仓数量投影 hash，插入 epoch/carry/success-attempt，再重新计算投影；两次必须逐代码、数量和 hash 完全一致。
9. 提交并用新的只读连接重验成功凭证、carry manifest、trigger、chain 和高水位前缀。
10. 返回 typed receipt；不创建订单、不修改 paper row、不推送策略结论。

## 8. 活动纪元读取与旧持仓隔离

### 8.1 成员边界

活动纪元候选成交必须同时满足：

- `paper_trade.id > paper_trade_high_water`；
- `status='Filled'`；
- 规范成交日期 `>= effective_trading_date`；
- terminal audit `id > order_audit_high_water` 且精确绑定；
- 完整来源验证通过。

任一高水位后成交落在生效日前、旧前缀 hash 漂移、audit chain 不能从旧 tip 连续延伸、缺少/重复 terminal binding，整次归因为 `FailedIntegrity`。请求起点早于生效日也明确失败，不静默裁剪。

### 8.2 carry 消耗

纪元预处理器为每个代码维护 `LegacyCarry`、边界后 quarantine 数量和“已回到零”状态；它先产生可归因完整行与明确排除项，再把可归因行交给既有 BR-248 FIFO/T+1/费用引擎。固定顺序为：

1. 某代码初始 carry 为零时，从生效日起直接把其完整成交行交给 BR-248 引擎。
2. 某代码初始 carry 非零时，边界后的每条 buy/sell 都以完整 fill 为 `LegacyCarryOverlap` 排除，只更新严格数量 overlay；sell 在数量意义上先消耗原 carry，再消耗边界后买入，但不拆分该 fill 或其费用。
3. overlay 累计不得为负；首次精确回到零的那条 sell 仍属于排除段。只有其后的下一条完整成交才可开始新归因周期。
4. 跨 carry 与边界后买入的 sell 额外记为 `MixedLegacyCarryExit`；整条 sell 和隔离段内全部买入保持排除，禁止按比例猜测费用、缩量或构造 synthetic fill。
5. 隔离解除后的完整成交继续执行既有 T+1、超卖、费用和闭环验证；任何失败使整次新纪元 replay 失败。

报告必须展示仍在 quarantine 的代码/股数、`LegacyCarryOverlap` 买卖数量、mixed exit 数量、已解除隔离代码数、新闭环数量和所有排除 reason。排除项不进入胜率、收益、费用或样本门槛分母。

### 8.3 legacy 访问

CLI 默认选择活动纪元。显式 `--epoch legacy` 可读取旧报告或重放旧诊断，但旧 T+1 错误仍会明确失败；不得因为新纪元存在而改写 legacy 结论。显式 epoch ID 必须精确存在且 receipt/chain 完整，不提供 latest-by-date 猜测或 fallback。

## 9. Monitor 集成

- 新 epoch service 位于 library 深模块；`monitor/main.rs` 只负责捕获 `now`、判断固定窗口、调用 typed API 和记录 typed outcome。
- 安全窗口固定为交易日 15:35–15:50，避开已有 15:05–15:30 盘后成交和 15:30 卖出扫描。
- 窗口内每个 tick 可重试；数据库 activation domain、unique key 和内容 hash 是真正幂等权威，进程内日期 latch 只减少调用。
- 首次成功后，每次启动和每日窗口只验证现有凭证，不创建下一纪元。
- 错过窗口时等待下一个完整交易日收盘，不在盘中、周末或次日开盘前追补。
- 切换失败不终止独立行情/模拟交易任务，但 monitor 必须记录 structured error，归因保持 `Unavailable`，窗口内保留重试资格。
- monitor 当前 `compute_daily/compute_window` 路径必须通过活动纪元 seam；没有有效活动纪元时不得继续读取完整旧账本或推送成功归因。
- 本规则只允许上述窄接线。BR-251 对 TechnicalBars、benchmark、paper sell、订单和额外推送副作用的禁令继续有效。

## 10. CLI 合同

新增 `strategy_attribution reset-sample`：

- 默认 preview，只读解析日历、显示拟切换完成日/生效日及数据库中可验证的拟高水位，不初始化 writer；
- `--commit` 必须同时有显式 `--db`，调用与 monitor 相同的 service；
- preview 数据库必须保持逐字节不变；
- 盘中返回 `current_session_incomplete`；不在安全窗口或日期不可用返回 typed Unavailable；
- 同内容 commit 返回原 receipt；不同内容或已有冲突 activation 返回 integrity failure。

`scheduled`、`replay`、`quarter` 增加可选 `--epoch`；省略时选择活动纪元，输出固定包含 epoch ID、生效日、carry/exclusion 统计和纪元内 manifest。`capture` 的 benchmark 语义不变。

## 11. 失败语义

| 场景 | 分类 | 行为 |
| --- | --- | --- |
| 当前交易日未完整、错过窗口、日历暂不可用 | Unavailable / retryable 按来源 | 不写成功凭证，不改归因边界。 |
| 日期超出 checked-in 日历覆盖 | Unavailable / non-retryable | 等待权威日历更新，不用工作日推算。 |
| SQLite busy/locked | Unavailable / retryable | 失败审计可写时追加，窗口内重试。 |
| 旧结构、trigger、sequence、manifest、binding 或 audit chain 冲突 | FailedIntegrity | 不创建纪元；不绕过坏结构。 |
| legacy T+1 违规 | Quarantined legacy economic defect | 允许构建数量隔离池，但 legacy replay 仍失败。 |
| legacy 累计超卖、坏身份、坏价格、坏数量或乱序 | FailedIntegrity | 不能建立可信数量隔离池，切换失败。 |
| 高水位后出现生效日前成交 | FailedIntegrity | 不自动归到 legacy，不改高水位。 |
| carry-only/mixed exit | Explicit exclusion | 维持数量连续性，不产生归因指标。 |
| 新纪元 T+1、超卖或费用/benchmark/close 缺失 | 既有 BR-251 typed failure/unavailable | 不用 legacy 或默认值补齐。 |
| 新样本不足 | InsufficientSample / ResearchOnly | 输出事实统计，不输出策略成功结论。 |
| 成功凭证提交后代码故障 | Attribution Unavailable | 保留凭证，停止新归因，修复前滚。 |

## 12. 数据红线与业务规则

- **2.1**：只读真实 `paper_trades`/`order_audit`；生产不使用 mock 或空 fallback。
- **2.2**：旧成本、费用、provider time 等缺失保持缺失；carry 不填成本。
- **2.3**：新纪元继续验证正价格、数量、顺序、T+1 和超卖；legacy 只隔离已知 T+1 经济缺陷，不放宽结构验证。
- **2.4**：生效日只来自 checked-in verified trading calendar；不使用环境覆盖或普通工作日推算。
- **2.5**：测试只使用 TEST_CODE、隔离数据库和固定时钟；生产拒绝 TEST_CODE 污染。
- **2.7**：epoch、carry、attempt、report binding 均 append-only、hash-chained、至少保留五年。
- **2.8**：`save/verify` 必须真实写入/完整重验目标表，日志不能冒充审计。
- **2.10 / BR-255**：登记高水位过滤、`(ts,id)` 排序、carry-first 消耗、monitor 窗口互斥及内容哈希幂等。
- **2.6 / 2.9**：不改订单安全和配置阈值，均为 N/A。

BR-255 只 supersede BR-251 的两处窄限制：允许 monitor 自动创建/验证归因纪元，以及要求 monitor 日归因读取活动纪元。BR-251 的其他数据、benchmark、结论门、ResearchOnly 和无订单副作用约束保持不变。

## 13. 测试矩阵

### 13.1 单元与数据库合同

- canonical receipt/carry/attempt/report-binding hash 的 golden 与逐字段 mutation；
- exact schema、AUTOINCREMENT sequence、canonical trigger registry 和 retention deadline；
- verified next-session 跨周末/节假日/覆盖边界；禁止 legacy `next_trading_day` fallback；
- legacy FIFO 数量投影接受已知 T+1 缺陷但拒绝超卖、坏价、坏量、坏方向、乱序和重复；
- activity selection、相同内容幂等、冲突内容失败、完整 hash-chain continuation。

### 13.2 归因行为

- fixture 含旧 sell id 520 同构 T+1 缺陷及合法新纪元闭环：legacy 明确失败，新纪元成功构造研究报告；
- carry-only sell、隔离期新 buy、mixed carry/new sell、回到零的 terminal sell、归零后的全新闭环；隔离段不拆 fill、不按比例猜费用；
- 高水位后迟到成交、旧前缀改写、audit 缺失/重复/断链全部失败；
- range 早于 effective date 不裁剪；样本不足不借 legacy；
- report commit 必须有 epoch binding，旧报告保持 legacy。

### 13.3 Monitor 与 CLI

- `15:34:59` 不切换，`15:35:00` 可尝试，`15:50:00` 边界明确覆盖，窗口外不写；
- 连续 tick、并发调用、进程重启只产生一个 success receipt；失败保留窗口内重试；
- 激活前后模拟持仓投影逐代码/数量完全相同；paper/order 表行与内容不变；
- preview 数据库逐字节不变；manual commit 与 monitor 得到同一幂等结果；
- monitor 日归因没有 epoch 时保持 Unavailable，存在 epoch 时只读 scoped rows；
- TEST_CODE 数据库、时间和代码与 production 物理隔离。

### 13.4 Gate C

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh --policy pr
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
cargo llvm-cov report --lcov --output-path target/coverage/lcov.info
python3 tools/coverage/check_thresholds.py --policy pr \
  --report target/coverage/coverage.json \
  --lcov target/coverage/lcov.info \
  --base-ref <merge-base>
cargo build --release --bin monitor
```

核心 patch 覆盖至少 90%，其他 production patch 至少 85%，global/core 不低于 BR-252 已审计 baseline。Gate C 通过只代表可合并；生产 freshness、首次真实切换 receipt、live attribution 和审计签字仍属于 Gate D。

## 14. 发布、观测与回滚

1. 先合并只包含设计与 BR-255 的 Gate A 提交。
2. 按书面实施计划完成 Gate B/C；PR 不得在任一门禁失败时合并。
3. 部署后 monitor 在首次 15:35–15:50 安全窗口提交切换；提交前归因仍按旧路径失败关闭，不生成成功结论。
4. 成功日志只显示脱敏 epoch/receipt hash、完成日、生效日、carry 代码/股数和高水位，不输出真实持仓列表。
5. 首个生效交易日后验证：旧前缀不变、carry 消耗可解释、新成交/terminal audit 连续、报告明确 ResearchOnly。

回滚按根因：

- 凭证提交前：`git revert <merge-commit>`，重新构建 monitor；
- 凭证提交后：绝不删除/改写 epoch、carry、attempt、trade、audit 或 report；停止新归因并保持 `Unavailable`，保留行情和模拟交易，修复后前滚；
- 架构/数据流问题回 Gate A，实现问题回 Gate B，红线问题在 Gate B 修复并重做 Gate A failure-mode review。

## 15. 成功标准

- legacy 事实完整保留，legacy replay 仍真实报告其错误；
- 模拟交易引擎的持仓和成交行为在切换前后不变；
- monitor 自动且仅一次建立可重验的纪元凭证；
- 旧持仓及其与边界后买入重叠的整段退出不会导致新归因 oversell，也不会进入新收益样本；该代码归零后的新闭环可正常进入；
- 纯新纪元闭环按既有 BR-251 FIFO/T+1/费用/benchmark 合同正常归因；
- 样本不足、证据缺失、迟到数据和篡改均显式失败或降为 ResearchOnly，不产生虚假成功；
- Gate C 全部通过且 PR 证据完整后才允许合并；Gate D 未闭合前不发布策略有效性结论。
