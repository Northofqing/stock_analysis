# Monitor 盘中数据源隔离热修设计

状态：已批准。账户所有者已明确授权运行阻碍出现时直接完成最小修复、通过 PR 合并并立即重启监控。

## 1. 问题与边界

2026-07-20 09:30 开盘后，release `monitor` 的盘中循环把全市场涨停池和真实持仓报价装配在同一个短路 `Result` 中。涨停池主、备源均因 `change_pct` 未通过既有 ±20% 质量门而显式失败；由于涨停池先执行，持仓报价函数没有被调用，整个盘中处理块被跳过。

本热修只解除两个真实数据源之间不必要的短路依赖。它不放宽涨跌幅校验，不把失败源替换为空集合，不生成行情、持仓、净值或账户占位数据，也不改变任何阈值、排序、过滤、通知或下单规则。

### 1.1 可复现证据

在 commit `f03351be0820b535127d9979ae71d35292df5338` 的 release binary 上执行：

```bash
perl -ne 'if (/^\[([0-9]{2}:[0-9]{2}):[0-9]{2}/ && $1 ge "09:30") {$n{"intraday_batch_rejected"}++ if /\[盘中监控\] 行情批次拒绝:/; $n{"limit_quality_rejected"}++ if /涨停榜.*change(?:_pct|percent) 缺失\/超过±20%/; $n{"data_mode_unsafe"}++ if /\[DataMode-hook\] 模式 .* → Unsafe,/;} END{for $k(sort keys %n){print "$k=$n{$k}\n"}}' /private/tmp/stock_analysis_monitor.log
```

10:05 的脱敏输出：

```text
data_mode_unsafe=69
intraday_batch_rejected=69
limit_quality_rejected=138
```

源码事实：`src/bin/monitor/main.rs` 当前在 `get_limit_up_stocks()?` 成功后才调用 `fetch_position_quotes()?`。因此涨停池失败必然阻止持仓报价，随后持仓健康、信号检测和独立做 T 扫描所在的外层处理块也不会执行。

## 2. 方案比较

### 方案 A：保留独立 `Result`（采用）

同一 blocking task 依次调用两个真实源，但分别保存 `Result`，不使用 `?` 在第一个错误处返回。回到异步循环后，每个错误分别记录；成功结果只进入自己的消费路径。

- 优点：改动窄；两个失败边界均可测试；不改变质量规则；一个源失败时另一个源仍可服务。
- 风险：下游原来接收两个 `Vec`，需要显式区分 unavailable 与真实空批次。

### 方案 B：先取持仓报价，再保持整体短路（拒绝）

这会让 Quote capability 被登记，但涨停池失败后仍跳过持仓处理，健康状态与实际功能不一致。

### 方案 C：忽略坏行或放宽 ±20%（拒绝）

这会绕过 AGENTS 2.3，且可能把未经人工确认的坏数据带入涨停计算。

## 3. 数据流

```text
30 秒盘中 tick
  ├─ 涨停池真实源 + 质量门 ──> Result<LimitStocks, Error>
  └─ 真实持仓快照 30 秒门 + 实时报价 5 秒门 ──> Result<PositionQuotes, Error>

结果汇合（不互相短路）
  ├─ LimitStocks=Ok   ──> 连板、涨停榜和排名路径
  ├─ LimitStocks=Err  ──> 明确错误；上述路径 unavailable
  ├─ PositionQuotes=Ok  ──> 持仓信号、健康度和报价 capability
  └─ PositionQuotes=Err ──> 明确错误；上述路径 unavailable
```

只有 `PositionQuotes=Ok` 时才处理持仓。涨停池不可用时，主力排名保持缺失，不用 `0` 或空榜代表真实结果。只有 `LimitStocks=Ok` 时才执行涨停池相关计算。

## 4. 接口与实现

新增一个窄接缝，接受两个外部边界函数并返回具名的两个独立 `Result`。生产调用仍使用现有 `MarketAnalyzer::get_limit_up_stocks` 和 `market_data::fetch_position_quotes`；测试注入 `TEST_CODE` 数据与显式错误，验证第一个源失败时第二个源仍执行并保留成功值。

盘中循环将：

1. 分别记录涨停池和持仓报价失败。
2. 用 `Option` 表示 unavailable，不把错误转成 `Vec::new()`。
3. 仅在对应 `Option::Some` 时运行该源的消费逻辑。
4. 当两个源都失败时跳过本轮数据计算，但进程继续退避重试。
5. 保持已有 30 秒节奏、去重、排序和推送规则不变。

## 5. 失败模式

| 失败 | 行为 | 禁止行为 |
| --- | --- | --- |
| 市场分析器初始化失败 | 涨停路径显式 unavailable；仍尝试持仓报价 | 伪装为当日无涨停 |
| 涨停池传输/解析/质量失败 | 保留原错误；仍尝试持仓报价 | 跳坏行、放宽 ±20% |
| 持仓快照缺时间或超过 30 秒 | 持仓路径显式 unavailable；涨停路径可独立运行 | 使用启动时持仓或成本价 |
| 持仓报价传输/质量失败 | 持仓路径显式 unavailable | 使用成本价或上一轮报价 |
| blocking task join 失败 | 两个结果均 unavailable，本轮明确失败 | 静默继续计算 |
| 一个源成功、一个源失败 | 只运行成功源对应的计算 | 以空集合代替失败源 |

## 6. 旧模块关系

| 模块 | 决策 | 原因 |
| --- | --- | --- |
| `MarketAnalyzer::get_limit_up_stocks` | adopt | 保留主备源与 BR-106 质量语义 |
| `market_data::fetch_position_quotes` | adopt | 保留真实持仓 30 秒门和 Quote capability 登记 |
| `monitor::data_mode` | adopt | 继续只由真实成功边界登记 capability |
| 盘中外层 `Option<(Vec, Vec)>` | replace | 单一失败会错误屏蔽另一独立数据源 |

## 7. 测试与验收

TDD tracer bullet：涨停源返回错误，持仓源返回一条 `TEST_CODE` 合法报价；断言持仓闭包确实执行、涨停错误保留、持仓成功值可取。第二个用例反向验证持仓失败不抹掉涨停成功。

验收命令：

```bash
cargo test --bin monitor intraday_market_inputs
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash tools/compliance/check.sh
cargo llvm-cov --all-features --all-targets --summary-only
cargo build --release --bin monitor
```

生产验收：重启后在真实交易时段，若涨停池仍被质量门拒绝，日志必须同时出现持仓报价边界已执行的脱敏证据；DataMode 的 Quote capability 只在真实报价成功后改变。若真实持仓源本身不满足 30 秒门，则保持 Unsafe 并记录该独立原因。

## 8. 回滚

```bash
git revert <merge-commit>
cargo build --release --bin monitor
```

回滚后恢复旧的整体短路行为；不得通过关闭质量校验或填充假数据回滚。运行日志继续只保存在本机权限 `0600` 的文件中。
