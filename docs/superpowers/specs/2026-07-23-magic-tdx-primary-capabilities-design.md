# Magic TDX 主行情能力接入设计

日期：2026-07-23  
状态：Gate A 待用户复核  
规则：BR-092、BR-108、BR-128、BR-148；数据红线 2.1、2.2、2.3、2.4、2.7、2.8、2.10

## 1. 目标与范围

把已经通过真实服务器探测的 Magic TDX 能力接入常驻监控：

- Magic TDX 成为现价、行情源时间、K 线和五档盘口的主源。
- 腾讯只保留为涨跌停价边界证据源；不再决定组合报价的现价。
- MoneyFlow 与 News 不在本次范围，继续使用现有真实来源。
- 不推算涨跌停价，不用本机时间替代 provider time，不把未验证盘口标记健康。

本次不改变订单金额、数量、价格边界、去重、二次确认等规则。

## 2. 当前代码与真实数据证据

### 2.1 可复现命令

```bash
CARGO_TARGET_DIR=/private/tmp/magic_tdx_capability_target \
  cargo test --manifest-path ../magic-market-data-rs/Cargo.toml \
  -p magic-tdx-rs --offline --test capabilities -- --nocapture
```

关键输出：

```text
running 3 tests
test unsupported_p0_traits_are_still_callable ... ok
test blocking_and_smart_clients_expose_order_book_contract ... ok
test tdx_advertises_all_core_data_families ... ok
test result: ok. 3 passed
```

能力声明断言：`quotes=true`、`bars=true`、`order_book=true`、`money_flow=false`。

真实只读探测：

```bash
CARGO_TARGET_DIR=/private/tmp/magic_tdx_capability_target \
  cargo run --manifest-path ../magic-market-data-rs/Cargo.toml \
  -p magic-tdx-rs --example live_probe --offline
```

2026-07-23 关键输出：

```text
connected=true
quotes=1 first_price=15.3
bars=5 first_datetime=2026-07-17
order_books=1
order_book code=600396 status=Unavailable total_bid=Some(309.0) total_ask=Some(1133.0) source_at=None
blocks_industry=22235
live_probe_status=passed
```

结论：Quote/Kline/盘口载荷真实可达；盘口尚缺可验证 `source_at`，所以现状不能登记为健康。MoneyFlow 明确不支持。

### 2.2 当前接线事实

复现命令：

```bash
rg -n "MagicTdxProvider|PublicQuoteProvider|register_unsupported|mark_capability_success" \
  src/broker.rs src/data_provider/fallback.rs src/bin/monitor/data_mode_probe.rs src/monitor/data_mode.rs
```

当前关系：

- `broker.rs` 只允许 `public/tencent`，执行报价来自腾讯。
- `fallback.rs` 已把 Magic TDX 作为 Kline P1，只有业务请求成功后才登记 Kline。
- `data_mode_probe.rs` 固定把 OrderBook 注册为 Unsupported。
- `data_mode.rs` 只接受真实成功边界登记，不允许固定伪造新鲜。

## 3. 方案比较

### 方案 A：组合证据（采用）

Magic TDX 提供现价与 provider time，腾讯只提供涨跌停价。组合结果必须同时验证证券代码、价格、时间和上下限。优点是满足“Magic TDX 主源”并保持订单安全；缺点是执行报价仍依赖两个真实来源。

### 方案 B：Magic TDX 独占、缺限价即失败

数据语义最纯，但当前 Magic TDX 元数据明确没有来源支持的涨跌停规则，纸盘与决策执行报价会长期不可用，暂不采用。

### 方案 C：从前收价与板块规则推算限价

拒绝。ST、上市初期、临时无涨跌幅限制、价格最小单位和规则版本都会造成错误，违反数据红线 2.3 与订单安全 2.6。

## 4. 组件与数据流

### 4.1 Quote

新增组合 `QuoteProvider`：

```text
Magic TDX SecurityQuote
  ├─ code
  ├─ price
  └─ servertime ── provider-time parser ── freshness <= 5s

Tencent RealtimeQuote
  ├─ code
  ├─ limit_down_price
  └─ limit_up_price

两批证据均成功且一致
  └─ ExecutionQuote { price=TDX, limits=Tencent, observed_at=TDX }
       └─ mark Quote capability success
```

Magic TDX 失败、腾讯失败、代码不一致、源时间缺失/歧义、现价非法、上下限非法或现价超出合理边界，均显式失败。不得退回腾讯现价。

### 4.2 Kline

复用 `MagicTdxProvider::get_daily_data` 与 BR-092 严格校验。新增独立 Kline 探针，以一个登记的真实市场基准证券请求最小日线批次；只有协议、OHLC、日期连续性、涨跌幅与交易日新鲜度全部通过后登记 Kline capability。

探针用于健康诊断，不写入策略输入、不替换策略自己的完整批次。闭市仍可用已结算当日日线/上一交易日日线执行 1 个交易日门禁。

### 4.3 OrderBook

复用 Magic TDX 五档报价。先修正适配器保留 provider `servertime`，并要求同一批次各证券时间可解析且不晚于本机观察时间。每档价格与数量必须有限且非负；正常交易状态下完整五档才可登记健康。

若源只给本地时钟而无日期，日期只能由当前交易日会话上下文组合，并拒绝跨日/休市歧义。无法证明日期时保持 `Unavailable`，DataMode 不登记成功。

OrderBook 继续作为辅助能力，不改变现有 Full/Degraded/Unsafe 分类权重。

### 4.4 调度

Quote、Kline、OrderBook 探针由独立 60 秒调度器驱动，不读取账户/持仓新鲜度。闭市时按 BR-148 标记 `expected_now=false`；不运行需要实时更新的 Quote/OrderBook 探针，也不制造 Failed。Kline 可按交易日门禁独立执行。

## 5. 失败与审计

- 每次探针记录 capability、provider、开始/结束、本地获取时间、provider time、结果、结构化原因和 retryable。
- Magic TDX 智能换源成功仍保留最终 provider；全部服务器失败为显式失败。
- 源时间缺失不能使用 `Utc::now()`、数据库更新时间或获取时间替代。
- 部分盘口不得标记完整；缺失档位保持不可用并记录问题。
- MoneyFlow/News 不注册为 Magic TDX 能力，也不改变现有来源。

## 6. 旧模块处置

| 模块 | 处置 | 原因 |
| --- | --- | --- |
| `broker::PublicQuoteProvider` | adopt/收窄 | 保留腾讯涨跌停价证据；不再提供主现价 |
| `MagicTdxProvider` | adopt/扩展 | 复用已验证连接、代码校验、Kline 与 provider time 解析 |
| `data_provider::fallback` | adopt | 保留 Magic TDX Kline P1 与多源质量竞速 |
| `data_mode_probe` | replace fixed OrderBook unsupported | 改成真实独立探针状态，失败仍显式 |
| `monitor::data_mode` | adopt | 继续仅接受真实成功时间；不改变治理枚举 |
| 东财 MoneyFlow、独立 News providers | adopt unchanged | Magic TDX 不支持对应数据族 |

## 7. 测试与验收

### Gate B

- 组合 Quote：TDX 现价与时间、腾讯上下限均完整时成功。
- 任一来源失败、TDX stale、代码不一致、限价缺失/倒置时失败。
- Kline 探针只在严格校验及交易日新鲜度通过后登记成功。
- OrderBook 源时间缺失、未来、stale、档位非法/不完整时不登记健康。
- 闭市不会把 Quote/OrderBook 未更新误记为 Failed。

### Gate C

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

### Gate D

```bash
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

真实交易时段验收日志必须同时证明：Magic TDX Quote 成功且 ≤5 秒、Kline 严格批次成功、OrderBook 具有 provider time 并通过质量门、DataMode 对应能力从 Warming/Missing 变为 Healthy。没有真实交易时段证据时只能报告 Gate C 完成，不能报告发布完成。

## 8. 回滚

使用 `git revert <implementation-commit>` 恢复腾讯单源执行报价、现有 Kline 调用时登记和 OrderBook Unsupported。回滚不删除任何审计、行情、账户、持仓或交易证据。

