//! Adapter for the validated `magic-tdx-rs` client.
//!
//! This provider is deliberately strict: transport/protocol failures are
//! returned to the fallback manager and no synthetic quote or close is made.
//!
//! Business rules: BR-092 (strict K-line validation), BR-147 (settled close
//! evidence).

use super::{validate_kline_series_strict, AdjustType, DataProvider, KlineData, RealtimeQuote};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone, Utc};
use magic_tdx_rs::TdxHqClient;

pub struct MagicTdxProvider;

impl MagicTdxProvider {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn code(code: &str) -> Result<String> {
        let raw = code.trim().trim_end_matches(".SH").trim_end_matches(".SZ");
        if raw.len() != 6 || !raw.chars().all(|c| c.is_ascii_digit()) {
            return Err(anyhow!("TDX code must be six digits: {code}"));
        }
        Ok(raw.to_string())
    }

    fn market(code: &str) -> u8 {
        if code.starts_with('6') {
            1
        } else {
            0
        }
    }

    fn connected() -> Result<TdxHqClient> {
        let client = TdxHqClient::new();
        client
            .connect_to_any(Some(5.0))
            .map_err(|e| anyhow!("magic-tdx connect failed: {e}"))?;
        Ok(client)
    }

    fn source_time(raw: &str) -> Result<DateTime<Utc>> {
        // TDX provides a clock time only. Associate it with today's local
        // trading date; do not replace an unavailable source time with the
        // process clock, which would falsely pass the freshness gate.
        let time = NaiveTime::parse_from_str(raw.trim(), "%H:%M")
            .or_else(|_| NaiveTime::parse_from_str(raw.trim(), "%H:%M:%S"))
            .map_err(|e| anyhow!("magic-tdx invalid source time {raw:?}: {e}"))?;
        let local = Local::now().date_naive().and_time(time);
        Local
            .from_local_datetime(&local)
            .single()
            .map(|value| value.with_timezone(&Utc))
            .ok_or_else(|| anyhow!("magic-tdx ambiguous source time {raw:?}"))
    }
}

impl DataProvider for MagicTdxProvider {
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        let code = Self::code(code)?;
        let client = Self::connected()?;
        let bars = client
            .get_security_bars(
                9,
                Self::market(&code),
                &code,
                0,
                days.min(u16::MAX as usize) as u16,
                0,
            )
            .map_err(|e| anyhow!("magic-tdx daily bars {code}: {e}"))?;
        let mut out = bars
            .into_iter()
            .map(|bar| {
                let date = NaiveDate::from_ymd_opt(bar.year as i32, bar.month, bar.day)
                    .ok_or_else(|| anyhow!("magic-tdx invalid date for {code}"))?;
                Ok(KlineData {
                    date,
                    open: bar.open,
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                    volume: bar.vol,
                    amount: bar.amount,
                    pct_chg: 0.0,
                    intraday_price: None,
                    settled: true,
                    pe_ratio: None,
                    pb_ratio: None,
                    turnover_rate: None,
                    market_cap: None,
                    circulating_cap: None,
                    eps: None,
                    roe: None,
                    revenue_yoy: None,
                    net_profit_yoy: None,
                    gross_margin: None,
                    net_margin: None,
                    sharpe_ratio: None,
                    financials_history: None,
                    valuation_history: None,
                    consensus: None,
                    industry: None,
                    is_limit_up: false,
                    is_limit_down: false,
                    is_suspended: false,
                    adjust: AdjustType::None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        out.sort_by_key(|bar| std::cmp::Reverse(bar.date));
        for index in 0..out.len().saturating_sub(1) {
            let previous_close = out[index + 1].close;
            if previous_close > 0.0 {
                out[index].pct_chg = (out[index].close / previous_close - 1.0) * 100.0;
            }
        }
        validate_kline_series_strict(&mut out, &code)?;
        Ok(out)
    }

    fn get_stock_name(&self, code: &str) -> Option<String> {
        let code = Self::code(code).ok()?;
        let client = Self::connected().ok()?;
        let rows = client.get_security_list(Self::market(&code), 0).ok()?;
        rows.into_iter()
            .find(|row| row.code == code)
            .map(|row| row.name)
    }

    fn get_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        let code = Self::code(code)?;
        let client = Self::connected()?;
        let mut quotes = client
            .get_security_quotes(&[(Self::market(&code), code.as_str())])
            .map_err(|e| anyhow!("magic-tdx quote {code}: {e}"))?;
        let Some(q) = quotes.pop() else {
            return Ok(None);
        };
        if !q.price.is_finite() || q.price <= 0.0 {
            return Err(anyhow!("magic-tdx invalid price for {code}"));
        }
        let pct = if q.last_close > 0.0 {
            (q.price - q.last_close) / q.last_close * 100.0
        } else {
            0.0
        };
        Ok(Some(RealtimeQuote {
            code,
            name: String::new(),
            price: q.price,
            pct_chg: pct,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            volume: Some(q.vol),
            amount: Some(q.amount),
            limit_up_price: None,
            limit_down_price: None,
            source_time: Self::source_time(&q.servertime)?,
        }))
    }

    fn name(&self) -> &'static str {
        "magic-tdx"
    }
}
