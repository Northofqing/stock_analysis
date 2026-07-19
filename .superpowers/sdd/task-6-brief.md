wrote /Users/zhangzhen/Desktop/Quant/stock_analysis/.superpowers/sdd/task-6-brief.md: 144 lines
er/baostock_provider.rs`
- Modify: `tests/baostock_provider_test.rs`

- [ ] **Step 1: Add test (parse K线 body)**

```rust
// tests/baostock_provider_test.rs (追加)

#[test]
fn parse_kline_body_format() {
    // Baostock 响应格式 (实测): 
    // code,date,open,high,low,close,volume,amount
    // sh.600000,2024-01-15,13.50,13.60,13.45,13.55,12345,16789.50
    let body = "code,date,open,high,low,close,volume,amount\nsh.600000,2024-01-15,13.50,13.60,13.45,13.55,12345,16789.50\nsh.600000,2024-01-16,13.55,13.70,13.50,13.65,15000,20000.00\n";
    let klines = stock_analysis::data_provider::baostock_provider::parse_kline_body(body, "600000").unwrap();
    assert_eq!(klines.len(), 2);
    assert_eq!(klines[0].open, 13.50);
    assert_eq!(klines[0].close, 13.55);
    assert_eq!(klines[0].volume, 12345.0);
    assert_eq!(klines[0].amount, 16789.50);
    assert_eq!(klines[1].date, chrono::NaiveDate::from_ymd_opt(2024, 1, 16).unwrap());
}
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cargo test --test baostock_provider_test parse_kline
```

Expected: FAIL — `parse_kline_body` not found.

- [ ] **Step 3: Implement get_daily_data + parse_kline_body**

```rust
// src/data_provider/baostock_provider.rs (追加)
use chrono::NaiveDate;
use super::staleness_helper_trait;  // 假设有个 trait, 见 Step 4

/// 解析 Baostock K线 CSV body → Vec<KlineData>.
pub fn parse_kline_body(body: &str, our_code: &str) -> Result<Vec<KlineData>> {
    let mut lines = body.lines();
    // 第 1 行是表头: code,date,open,high,low,close,volume,amount
    let header_line = lines.next().ok_or_else(|| anyhow!("Baostock K线: 空 body"))?;
    let headers: Vec<&str> = header_line.split(',').collect();
    
    let idx = |name: &str| -> usize {
        headers.iter().position(|h| h.trim() == name)
            .ok_or_else(|| anyhow!("Baostock K线: 缺 {} 列", name))
    };
    let i_date = idx("date")?;
    let i_open = idx("open")?;
    let i_high = idx("high")?;
    let i_low = idx("low")?;
    let i_close = idx("close")?;
    let i_volume = idx("volume")?;
    let i_amount = idx("amount")?;
    
    let mut result = Vec::new();
    for line in lines {
        if line.trim().is_empty() { continue; }
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 7 { continue; }
        
        let date = NaiveDate::parse_from_str(&fields[i_date], "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Local::now().date_naive());
        let open = fields[i_open].parse().unwrap_or(0.0);
        let high = fields[i_high].parse().unwrap_or(0.0);
        let low = fields[i_low].parse().unwrap_or(0.0);
        let close = fields[i_close].parse().unwrap_or(0.0);
        let volume = fields[i_volume].parse().unwrap_or(0.0);
        let amount = fields[i_amount].parse::<f64>().unwrap_or(0.0);
        let pct_chg = if open > 0.0 { (close - open) / open * 100.0 } else { 0.0 };
        
        result.push(KlineData {
            date, open, high, low, close, volume, amount, pct_chg,
            intraday_price: None, settled: true,
            pe_ratio: None, pb_ratio: None,
            turnover_rate: None, market_cap: None, circulating_cap: None,
        });
    }
    let _ = our_code;  // 当前解析已用 baostock code
    Ok(result)
}

impl BaostockProvider {
    async fn fetch_kline_async(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        let sid = self.ensure_session().await?;
        let bs_code = to_baostock(code);
        let end_date = chrono::Local::now().date_naive();
        let start_date = end_date - chrono::Duration::days(days as i64 * 2);  // ×2 留 buffer for 停牌
        
        let body = build_kline_query_body(
            &bs_code,
            "date,open,high,low,close,volume,amount",
            &start_date.format("%Y%m%d").to_string(),
            &end_date.format("%Y%m%d").to_string(),
            &sid,
        );
        let resp = self.client.post(&format!("{}/QueryHistoryKLinePlus", self.base_url))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send().await?
            .text().await?;
        let code = parse_baostock_response(&resp, "ErrorCode")?
            .ok_or_else(|| anyhow!("Baostock K线: 无 ErrorCode"))?;
        if code != "0" {
            return Err(anyhow!("Baostock K线失败: code={code}"));
        }
        parse_kline_body(&resp, code)
    }
}

impl DataProvider for BaostockProvider {
    fn name(&self) -> &'static str { "baostock" }
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        tokio::runtime::Handle::current()
            .block_on(self.fetch_kline_async(code, days))
    }
    fn get_stock_name(&self, _code: &str) -> Option<String> { None }
    fn get_realtime_quote(&self, _code: &str) -> Result<Option<RealtimeQuote>> { Ok(None) }
}
```

- [ ] **Step 4: Run test, verify PASS**

```bash
cargo test --test baostock_provider_test
```

Expected: 4 tests passed.

- [ ] **Step 5: Commit**

```bash
git add src/data_provider/baostock_provider.rs tests/baostock_provider_test.rs
git commit -m "feat(baostock): implement get_daily_data + parse_kline_body CSV mapping"
```

---

