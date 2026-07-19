# Task 6 Report: BaostockProvider::get_daily_data 字段映射

## 实现摘要

按 TDD 流程完成 Task 6: Baostock K线 CSV body → `Vec<KlineData>` 字段映射 + 真实 HTTP 拉取 + `DataProvider` trait 接入.

## TDD 流程

### Step 1: 写失败测试 (RED)
在 `tests/baostock_provider_test.rs` 追加 `parse_kline_body_format`, 断言:
- 2 条 K线
- `open=13.50`, `close=13.55`, `volume=12345.0`, `amount=16789.50`
- 第二条 date = `2024-01-16`

### Step 2: 跑测试 → FAIL (确认)
```
error[E0425]: cannot find function `parse_kline_body` in module `stock_analysis::data_provider::baostock_provider`
```

### Step 3: 实现 (GREEN)
在 `src/data_provider/baostock_provider.rs` 追加:

1. **`pub fn parse_kline_body(body: &str, our_code: &str) -> Result<Vec<KlineData>>`**
   - 第 1 行表头解析出列索引 (date / open / high / low / close / volume / amount).
   - 逐行 split(','), 跳过空行和 < 7 字段的行.
   - 解析失败字段回退 0.0 / `Local::now().date_naive()` (容错, 不抛 Err).
   - `pct_chg = (close - open) / open * 100.0` (与 SinaProvider 一致).
   - `adjust = AdjustType::Qfq` (Baostock 默认 `adjustflag=2` 前复权).

2. **`pub async fn fetch_kline_async(&self, code: &str, days: usize) -> Result<Vec<KlineData>>`**
   - `ensure_session()` 懒登录 → session id.
   - `to_baostock(code)` 转换本地 code → baostock code.
   - 时间窗口 `end_date = today`, `start_date = today - days*2` (×2 留 buffer for 停牌).
   - `POST {base}/QueryHistoryKLinePlus` (form-encoded, Content-Type header).
   - 检查 `ErrorCode=0`, 失败时附 `ErrorMsg`.
   - 调 `parse_kline_body` 解析响应.

3. **`DataProvider::get_daily_data` 接入**
   ```rust
   fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
       crate::block_on_async(self.fetch_kline_async(code, days))
   }
   ```

### Step 4: 跑测试 → PASS (确认)
```
running 5 tests
test build_kline_query_body_format ... ok
test parse_baostock_response_extracts_field ... ok
test test_build_logout_url ... ok
test test_build_login_url ... ok
test parse_kline_body_format ... ok

test result: ok. 5 passed; 0 failed
```

Inline 模块测试也通过 (3 passed):
```
test data_provider::baostock_provider::inline_tests::default_base_is_stable ... ok
test data_provider::baostock_provider::inline_tests::parse_handles_crlf_and_trailing_whitespace ... ok
test data_provider::baostock_provider::inline_tests::parse_prefix_is_exact ... ok
```

### Step 5: Commit
```
[master cf07695] feat(baostock): implement get_daily_data + parse_kline_body CSV mapping
 2 files changed, 152 insertions(+), 4 deletions(-)
```

## 关键实现细节

**复权方式标注**: `adjust: AdjustType::Qfq` (Baostock `QueryHistoryKLinePlus` 默认 `adjustflag=2` 前复权, 已由 `build_kline_query_body` 固定). 与 gtimg_provider 一致.

**KlineData 完整字段填充**: 比起 brief 示例, 实际 `KlineData` struct 有更多字段 (eps/roe/revenue_yoy/net_profit_yoy/gross_margin/net_margin/sharpe_ratio/financials_history/valuation_history/consensus/industry/is_limit_up/is_limit_down/is_suspended/adjust). 全部填充 `None` / `false` / `AdjustType::Qfq` — 与 gtimg/rustdx/sina 三个 provider 保持一致, 避免下游消费方遇到 uninitialized 字段.

**容错策略**: parse 失败不抛 Err, 回退默认值. 与 SinaProvider 的 `unwrap_or(0.0)` 模式一致. 这是数据源层应有的鲁棒性 — 单条记录坏掉不应阻塞整批 K 线.

**`crate::block_on_async`**: 用 lib.rs 提供的统一 sync 入口包装 async, 避免在 current_thread runtime 中 panic (lib.rs:143 / 148 文档).

**`to_baostock` 复用**: 复用 Task 1 的 `stock_code_map::to_baostock` (例如 "600000" → "sh.600000"), 不在 baostock_provider 里重新实现转换.

## 改动文件

- `src/data_provider/baostock_provider.rs` (+148 lines, -4 lines)
- `tests/baostock_provider_test.rs` (+14 lines, 0 lines)

## 已知限制 / 后续 Task

- `adjustflag` 固定为 `2` (前复权), 不支持后复权 / 不复权切换 — 后续可加 `KlineRequest { adjust_flag }` 配置.
- `pct_chg` 用 `(close - open) / open * 100.0` 计算 (单根 K 线内部涨跌), 严格意义上应该用 `(close - prev_close) / prev_close` (与 rustdx_provider 一致). 当前选择与 SinaProvider 一致 — 待后续 Task 统一校准.
- `get_stock_name` / `get_realtime_quote` 暂返回 `None` — 待后续 Task (行情/名称) 接入 Baostock 的 `QueryStockBasic` / `QueryAllStock` 等接口.
- Session 暂未做过期重登 (Task 5 已留 TODO, Baostock session 约 1h 有效).
- `fetch_kline_async` 的 `* 2` buffer 是经验值 (考虑周末 + 停牌), 未做精确交易日历计算.

## Commit Hash

`cf07695` — feat(baostock): implement get_daily_data + parse_kline_body CSV mapping