//! 腾讯财经数据提供者
//!
//! 通过腾讯财经API获取股票数据
//! API文档: http://qt.gtimg.cn

use super::{DataProvider, KlineData, RealtimeQuote};
use anyhow::{anyhow, Context, Result};
use chrono::{FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::future::Future;

fn parse_tencent_source_time(raw: &str, code: &str) -> Result<chrono::DateTime<Utc>> {
    let local = NaiveDateTime::parse_from_str(raw, "%Y%m%d%H%M%S")
        .map_err(|error| anyhow!("腾讯实时行情 {code}: source_time 非法 {raw:?}: {error}"))?;
    let shanghai = FixedOffset::east_opt(8 * 60 * 60)
        .ok_or_else(|| anyhow!("腾讯实时行情 {code}: 无法构造 UTC+8 时区"))?;
    shanghai
        .from_local_datetime(&local)
        .single()
        .map(|value| value.with_timezone(&Utc))
        .ok_or_else(|| anyhow!("腾讯实时行情 {code}: source_time 不唯一 {raw:?}"))
}

/// 腾讯财经数据提供者
pub struct GtimgProvider {
    client: reqwest::Client,
    kline_base: String,
    quote_base: String,
}

impl GtimgProvider {
    fn parse_kline_http_response(
        status: u16,
        body: std::result::Result<String, String>,
        code: &str,
    ) -> Result<Vec<KlineData>> {
        if !(200..300).contains(&status) {
            return Err(anyhow!("HTTP请求返回错误状态: {status}"));
        }
        let text = body.map_err(|error| anyhow!("读取响应失败: {error}"))?;
        if text.is_empty() {
            return Err(anyhow!("API返回空响应"));
        }
        let trimmed = text.trim_start();
        if trimmed.starts_with('<') {
            let preview: String = trimmed.chars().take(120).collect();
            return Err(anyhow!(
                "腾讯K线接口返回非JSON内容（可能被网关重定向/拦截）: {preview}"
            ));
        }
        Self::parse_kline_response_internal(&text, code)
    }

    fn enrich_latest_kline(klines: &mut [KlineData], quote: Option<&RealtimeQuote>) {
        let (Some(latest), Some(quote)) = (klines.first_mut(), quote) else {
            return;
        };
        latest.intraday_price = Some(quote.price);
        latest.settled = false;
        latest.pe_ratio = latest.pe_ratio.or(quote.pe_ratio);
        latest.pb_ratio = latest.pb_ratio.or(quote.pb_ratio);
        latest.turnover_rate = latest.turnover_rate.or(quote.turnover_rate);
        latest.market_cap = latest.market_cap.or(quote.market_cap);
        latest.circulating_cap = latest.circulating_cap.or(quote.circulating_cap);
    }

    fn parse_realtime_http_response(
        status: u16,
        body: std::result::Result<String, String>,
        code: &str,
    ) -> Result<Option<RealtimeQuote>> {
        if !(200..300).contains(&status) {
            return Err(anyhow!("腾讯实时行情 {code} HTTP 失败: status={status}"));
        }
        let text = body.map_err(|error| anyhow!("腾讯实时行情 {code} 响应读取失败: {error}"))?;
        Self::parse_realtime_quote_response(&text, code)
    }

    /// 统一规范化股票代码，并推导腾讯接口所需交易所前缀。
    ///
    /// 支持输入：
    /// - 纯数字：300114
    /// - 带前缀：sz300114 / sh600519 / bj430047
    /// - 带后缀：300114.SZ / 600519.SH / 430047.BJ
    fn normalize_for_tencent(code: &str) -> Option<(String, String)> {
        let raw = code.trim();
        if raw.is_empty() {
            return None;
        }

        let lower = raw.to_ascii_lowercase();
        #[cfg(test)]
        let lower = lower
            .strip_prefix("test_code_")
            .unwrap_or(&lower)
            .to_string();

        // 1) 先处理显式前缀
        if let Some(rest) = lower.strip_prefix("sh") {
            if rest.len() == 6 && rest.chars().all(|c| c.is_ascii_digit()) {
                return Some((rest.to_string(), format!("sh{}", rest)));
            }
        }
        if let Some(rest) = lower.strip_prefix("sz") {
            if rest.len() == 6 && rest.chars().all(|c| c.is_ascii_digit()) {
                return Some((rest.to_string(), format!("sz{}", rest)));
            }
        }
        if let Some(rest) = lower.strip_prefix("bj") {
            if rest.len() == 6 && rest.chars().all(|c| c.is_ascii_digit()) {
                return Some((rest.to_string(), format!("bj{}", rest)));
            }
        }

        // 2) 再处理后缀格式（如 300114.SZ）
        if let Some((num, suffix)) = lower.split_once('.') {
            if num.len() == 6 && num.chars().all(|c| c.is_ascii_digit()) {
                match suffix {
                    "sh" => return Some((num.to_string(), format!("sh{}", num))),
                    "sz" => return Some((num.to_string(), format!("sz{}", num))),
                    "bj" => return Some((num.to_string(), format!("bj{}", num))),
                    _ => {}
                }
            }
        }

        // 3) 纯数字按首位推导市场
        if lower.len() == 6 && lower.chars().all(|c| c.is_ascii_digit()) {
            let prefix = match lower.chars().next().unwrap_or('0') {
                '5' | '6' | '9' => "sh",
                '0' | '1' | '2' | '3' => "sz",
                '4' | '8' => "bj",
                _ => "sz",
            };
            return Some((lower.clone(), format!("{}{}", prefix, lower)));
        }

        None
    }

    /// 创建新的提供者
    pub fn new() -> Result<Self> {
        // 修复 Top10#7 (2026-06-29 audit): 用 SHARED_TENCENT_HTTP_CLIENT 共享 client
        Ok(Self {
            client: crate::http_client::SHARED_TENCENT_HTTP_CLIENT.clone(),
            kline_base: "https://web.ifzq.gtimg.cn".to_string(),
            quote_base: "http://qt.gtimg.cn".to_string(),
        })
    }

    #[cfg(test)]
    pub(super) fn with_bases(
        client: reqwest::Client,
        kline_base: String,
        quote_base: String,
    ) -> Self {
        Self {
            client,
            kline_base,
            quote_base,
        }
    }

    /// 公开方法：获取实时行情（用于其他数据提供者调用）
    pub fn fetch_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        self.get_realtime_quote(code)
    }

    /// 在同步上下文中安全执行异步任务 (委托给 crate::block_on_async).
    /// 修复 Top10#5 (2026-06-29 audit): 统一全代码库 block_on pattern, 避免重复实现.
    fn run_async_blocking<T, F>(fut: F) -> Result<T>
    where
        T: Send + 'static,
        F: Future<Output = Result<T>> + Send + 'static,
    {
        // F::Output = Result<T>, 所以 block_on_async 直接返回 Result<T>
        crate::block_on_async(fut)
    }

    /// 与 `run_async_blocking` 类似，但返回任意值（非 Result）。
    fn run_async_blocking_value<T, F>(fut: F) -> Result<T>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        // F::Output = T, block_on_async 返回 T, 用 Ok() 包装
        Ok(crate::block_on_async::<F, T>(fut))
    }

    /// 从腾讯财经API获取K线数据（异步版本）
    pub(crate) async fn fetch_kline_data_internal(
        client: &reqwest::Client,
        code: &str,
        days: usize,
    ) -> Result<Vec<KlineData>> {
        Self::fetch_kline_data_from_bases(
            client,
            code,
            days,
            "https://web.ifzq.gtimg.cn",
            "http://qt.gtimg.cn",
        )
        .await
    }

    async fn fetch_kline_data_from_bases(
        client: &reqwest::Client,
        code: &str,
        days: usize,
        kline_base: &str,
        quote_base: &str,
    ) -> Result<Vec<KlineData>> {
        let (normalized_code, market_code) = Self::normalize_for_tencent(code)
            .ok_or_else(|| anyhow!("无效股票代码格式: {}", code))?;

        // 腾讯财经K线API
        // ktype: day(日线), week(周线), month(月线)
        // 优先 HTTPS，避免部分网络环境下 HTTP 被网关重定向/拦截返回 HTML。
        let url = format!(
            "{}/appstock/app/fqkline/get?param={},day,,,{},qfq",
            kline_base.trim_end_matches('/'),
            market_code,
            days
        );

        log::debug!("[腾讯] 请求URL: {}", url);

        // 发送请求
        let response = client
            .get(&url)
            .header("Referer", "http://gu.qq.com/")
            .send()
            .await;

        let response = match response {
            Ok(resp) => resp,
            Err(e) => {
                log::error!("[腾讯] 请求失败 (code={}): {}", code, e);
                return Err(anyhow!("HTTP请求失败: {}", e));
            }
        };

        let status = response.status().as_u16();
        let body = response.text().await.map_err(|error| error.to_string());
        let mut klines = Self::parse_kline_http_response(status, body, &normalized_code)?;

        // 获取实时行情数据，补充盈利指标到最新K线
        if !klines.is_empty() {
            log::info!(
                "[腾讯] {} K线数据条数: {}, 最新K线日期: {}, 收盘价: {:.2}元",
                code,
                klines.len(),
                klines[0].date,
                klines[0].close
            );

            let quote = Self::fetch_realtime_quote_from_base(client, &normalized_code, quote_base)
                .await
                .ok()
                .flatten();
            if quote.is_none() {
                log::warn!("[腾讯] {} 无法获取实时行情数据", code);
            }
            Self::enrich_latest_kline(&mut klines, quote.as_ref());
        }

        Ok(klines)
    }

    /// 解析K线响应
    fn parse_kline_response_internal(text: &str, code: &str) -> Result<Vec<KlineData>> {
        use serde_json::Value;

        let json: Value = serde_json::from_str(text).context("解析JSON失败")?;

        // 获取K线数据数组
        // 数据路径: data.{market_code}.day 或 data.{market_code}.qfqday
        let (_, market_code) = Self::normalize_for_tencent(code)
            .ok_or_else(|| anyhow!("无效股票代码格式: {}", code))?;

        let klines = json["data"][&market_code]["qfqday"]
            .as_array()
            .or_else(|| json["data"][&market_code]["day"].as_array())
            .ok_or_else(|| anyhow!("未找到K线数据"))?;

        let mut result: Vec<KlineData> = Vec::with_capacity(klines.len());

        for (index, kline) in klines.iter().enumerate() {
            let kline_array = kline.as_array().ok_or_else(|| anyhow!("K线数据格式错误"))?;

            if kline_array.len() < 7 {
                return Err(anyhow!(
                    "腾讯K线 {code} 第 {} 行缺真实 amount 字段: expected>=7 actual={}",
                    index + 1,
                    kline_array.len()
                ));
            }

            // 腾讯K线格式: [日期, 开, 收, 高, 低, 成交量]
            // 例: ["2026-01-23", "14.22", "15.00", "15.49", "14.22", "335918000"]
            let date_str = kline_array[0]
                .as_str()
                .ok_or_else(|| anyhow!("日期格式错误"))?;
            let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .context(format!("解析日期失败: {}", date_str))?;

            let open: f64 = kline_array[1]
                .as_str()
                .ok_or_else(|| anyhow!("开盘价格式错误"))?
                .parse()?;
            let close: f64 = kline_array[2]
                .as_str()
                .ok_or_else(|| anyhow!("收盘价格式错误"))?
                .parse()?;
            let high: f64 = kline_array[3]
                .as_str()
                .ok_or_else(|| anyhow!("最高价格式错误"))?
                .parse()?;
            let low: f64 = kline_array[4]
                .as_str()
                .ok_or_else(|| anyhow!("最低价格式错误"))?
                .parse()?;
            let volume: f64 = kline_array[5]
                .as_str()
                .ok_or_else(|| anyhow!("成交量格式错误"))?
                .parse()?;
            let amount: f64 = kline_array[6]
                .as_str()
                .ok_or_else(|| anyhow!("成交额格式错误"))?
                .parse()?;

            let kline_data = KlineData {
                date,
                open,
                close,
                high,
                low,
                volume,
                amount,
                pct_chg: 0.0,
                intraday_price: None,
                settled: true,
                pe_ratio: None, // K线数据中不包含，需要从实时行情获取
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
                adjust: crate::data_provider::AdjustType::Qfq, // 腾讯 URL ,qfq 前复权
            };

            result.push(kline_data);
        }

        result.sort_by_key(|item| item.date);
        for index in 1..result.len() {
            let prev_close = result[index - 1].close;
            if prev_close.is_finite() && prev_close > 0.0 {
                result[index].pct_chg = ((result[index].close - prev_close) / prev_close) * 100.0;
            }
        }

        // BR-125: complete-batch validation also restores newest-first order.
        super::validate_kline_series_strict(&mut result, code)?;

        Ok(result)
    }

    /// 获取股票名称（静态异步方法）
    #[cfg(test)]
    async fn fetch_stock_name_internal(client: &reqwest::Client, code: &str) -> Option<String> {
        Self::fetch_stock_name_from_base(client, code, "http://qt.gtimg.cn").await
    }

    async fn fetch_stock_name_from_base(
        client: &reqwest::Client,
        code: &str,
        quote_base: &str,
    ) -> Option<String> {
        let (_, market_code) = Self::normalize_for_tencent(code)?;

        // 使用腾讯实时行情接口获取股票名称
        let url = format!("{}/q={}", quote_base.trim_end_matches('/'), market_code);

        match client
            .get(&url)
            .header("Referer", "http://gu.qq.com/")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(response) => {
                if let Ok(text) = response.text().await {
                    if let Some(name) = Self::parse_stock_name_response(&text) {
                        log::debug!("[腾讯] 获取股票名称: {} -> {}", code, name);
                        return Some(name);
                    }
                    log::debug!("[腾讯] 股票名称解析失败: code={}, text={}", code, text);
                }
            }
            Err(e) => {
                log::debug!("[腾讯] 获取股票名称失败: {}", e);
            }
        }
        None
    }

    fn parse_stock_name_response(text: &str) -> Option<String> {
        // 解析格式: v_sz002413="51~雷科防务~002413~15.00~..."
        let start = text.find('"')?;
        let end = text.rfind('"').filter(|end| *end > start)?;
        // v13.10.6: 只取第二个 `~` 字段，不把后续行情注入名称。
        text[start + 1..end]
            .split('~')
            .nth(1)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    }

    /// 获取实时行情（包含盈利指标）
    #[cfg(test)]
    async fn fetch_realtime_quote_internal(
        client: &reqwest::Client,
        code: &str,
    ) -> Result<Option<RealtimeQuote>> {
        Self::fetch_realtime_quote_from_base(client, code, "http://qt.gtimg.cn").await
    }

    async fn fetch_realtime_quote_from_base(
        client: &reqwest::Client,
        code: &str,
        quote_base: &str,
    ) -> Result<Option<RealtimeQuote>> {
        let (_, market_code) = Self::normalize_for_tencent(code)
            .ok_or_else(|| anyhow!("无效股票代码格式: {}", code))?;

        let url = format!("{}/q={}", quote_base.trim_end_matches('/'), market_code);

        let response = client
            .get(&url)
            .header("Referer", "http://gu.qq.com/")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|error| anyhow!("腾讯实时行情 {code} 请求失败: {error}"))?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|error| error.to_string());
        Self::parse_realtime_http_response(status, body, code)
    }

    fn parse_realtime_quote_response(text: &str, code: &str) -> Result<Option<RealtimeQuote>> {
        let (normalized_code, _) = Self::normalize_for_tencent(code)
            .ok_or_else(|| anyhow!("无效股票代码格式: {}", code))?;
        let start = text
            .find('"')
            .ok_or_else(|| anyhow!("腾讯实时行情 {code}: 响应缺少起始引号"))?;
        let end = text
            .rfind('"')
            .filter(|end| *end > start)
            .ok_or_else(|| anyhow!("腾讯实时行情 {code}: 响应缺少结束引号"))?;
        let parts: Vec<&str> = text[start + 1..end].splitn(60, '~').collect();
        if parts.len() < 54 {
            return Err(anyhow!("腾讯实时行情 {code}: 字段数 {} < 54", parts.len()));
        }

        let required_positive = |index: usize, field: &str| -> Result<f64> {
            let value = parts[index]
                .parse::<f64>()
                .map_err(|error| anyhow!("腾讯实时行情 {code}: {field} 非法: {error}"))?;
            if value.is_finite() && value > 0.0 {
                Ok(value)
            } else {
                Err(anyhow!(
                    "腾讯实时行情 {code}: {field} 必须 > 0, got {value}"
                ))
            }
        };
        let optional_non_negative = |index: usize| -> Option<f64> {
            parts[index]
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && *value >= 0.0)
        };

        let price = required_positive(3, "price")?;
        let prev_close = required_positive(4, "prev_close")?;
        let limit_up_price = required_positive(47, "limit_up_price")?;
        let limit_down_price = required_positive(48, "limit_down_price")?;
        let source_time = parse_tencent_source_time(parts[30], code)?;
        if limit_down_price > limit_up_price {
            return Err(anyhow!(
                "腾讯实时行情 {code}: 跌停价 {limit_down_price} > 涨停价 {limit_up_price}"
            ));
        }

        Ok(Some(RealtimeQuote {
            code: normalized_code,
            name: parts[1].to_string(),
            price,
            pct_chg: ((price - prev_close) / prev_close) * 100.0,
            pe_ratio: optional_non_negative(52).or_else(|| optional_non_negative(39)),
            pb_ratio: optional_non_negative(46),
            turnover_rate: optional_non_negative(38),
            market_cap: optional_non_negative(45),
            circulating_cap: optional_non_negative(44),
            volume: optional_non_negative(6).map(|value| value * 100.0),
            amount: optional_non_negative(37).map(|value| value * 10_000.0),
            limit_up_price: Some(limit_up_price),
            limit_down_price: Some(limit_down_price),
            source_time,
        }))
    }
}

impl Default for GtimgProvider {
    fn default() -> Self {
        Self::new().expect("创建GtimgProvider失败")
    }
}

impl DataProvider for GtimgProvider {
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        log::info!("[腾讯] 获取股票 {} 最近 {} 天数据", code, days);

        // 克隆必要的数据用于 async block
        let client = self.client.clone();
        let code_owned = code.to_string();
        let kline_base = self.kline_base.clone();
        let quote_base = self.quote_base.clone();

        let mut data = Self::run_async_blocking(async move {
            Self::fetch_kline_data_from_bases(&client, &code_owned, days, &kline_base, &quote_base)
                .await
        })?;

        // v11-P0-3 commit 2: K 线缺口推断 → 喂入 HALTED_PERIODS
        super::halt_status::infer_halt_from_kline_gaps(code, &data);

        // 填涨跌停 / 停牌标记（名称后续用 get_stock_name 单独覆盖）
        super::limit_status::apply_limit_flags_inplace(code, None, &mut data);

        log::info!("[腾讯] 成功获取 {} 条数据", data.len());

        Ok(data)
    }

    fn get_stock_name(&self, code: &str) -> Option<String> {
        let client = self.client.clone();
        let code_str = code.to_string();
        let quote_base = self.quote_base.clone();

        let result = Self::run_async_blocking_value(async move {
            Self::fetch_stock_name_from_base(&client, &code_str, &quote_base).await
        })
        .ok()
        .flatten();

        result
    }

    fn get_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        let client = self.client.clone();
        let code_str = code.to_string();
        let quote_base = self.quote_base.clone();

        Self::run_async_blocking(async move {
            Self::fetch_realtime_quote_from_base(&client, &code_str, &quote_base).await
        })
    }

    fn name(&self) -> &'static str {
        "腾讯财经"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kline_body(field: &str, rows: serde_json::Value) -> String {
        serde_json::json!({"data": {"sh600519": {field: rows}}}).to_string()
    }

    fn complete_kline_body() -> String {
        kline_body(
            "qfqday",
            serde_json::json!([
                [
                    "2026-07-15",
                    "10.00",
                    "10.00",
                    "10.20",
                    "9.90",
                    "1000",
                    "100000"
                ],
                [
                    "2026-07-16",
                    "10.00",
                    "10.10",
                    "10.20",
                    "9.95",
                    "1100",
                    "101000"
                ]
            ]),
        )
    }

    #[test]
    fn kline_http_decision_rejects_transport_protocol_and_bad_batches() {
        for (status, body, expected) in [
            (503, Ok(String::new()), "503"),
            (200, Err("read failed".to_string()), "读取响应失败"),
            (200, Ok(String::new()), "空响应"),
            (200, Ok("  <html>blocked</html>".to_string()), "非JSON"),
            (200, Ok("not-json".to_string()), "解析JSON失败"),
        ] {
            let error = GtimgProvider::parse_kline_http_response(status, body, "600519")
                .expect_err("incomplete HTTP fact must fail");
            assert!(error.to_string().contains(expected), "{error}");
        }
        let data =
            GtimgProvider::parse_kline_http_response(200, Ok(complete_kline_body()), "600519")
                .expect("complete strict HTTP body");
        assert_eq!(data.len(), 2);
    }

    #[test]
    fn realtime_http_decision_and_kline_enrichment_preserve_nullable_fields() {
        assert!(GtimgProvider::parse_realtime_http_response(
            429,
            Ok(String::new()),
            "TEST_CODE_000001"
        )
        .unwrap_err()
        .to_string()
        .contains("429"));
        assert!(GtimgProvider::parse_realtime_http_response(
            200,
            Err("read failed".to_string()),
            "TEST_CODE_000001"
        )
        .unwrap_err()
        .to_string()
        .contains("响应读取失败"));

        let quote = GtimgProvider::parse_realtime_http_response(
            200,
            Ok(realtime_body(&[])),
            "TEST_CODE_000001",
        )
        .expect("complete real-time protocol")
        .expect("present quote");
        let mut data =
            GtimgProvider::parse_kline_http_response(200, Ok(complete_kline_body()), "600519")
                .expect("complete daily batch");
        data[0].pe_ratio = Some(99.0);
        let settled_close = data[0].close;
        GtimgProvider::enrich_latest_kline(&mut data, Some(&quote));
        assert_eq!(data[0].close, settled_close);
        assert_eq!(data[0].intraday_price, Some(10.10));
        assert!(!data[0].settled);
        assert_eq!(data[0].pe_ratio, Some(99.0));
        assert_eq!(data[0].pb_ratio, Some(2.0));

        let unchanged = data.clone();
        GtimgProvider::enrich_latest_kline(&mut data, None);
        assert_eq!(data[0].intraday_price, unchanged[0].intraday_price);
        GtimgProvider::enrich_latest_kline(&mut [], Some(&quote));
    }

    #[test]
    fn br125_complete_tencent_batch_is_newest_first_with_real_pct() {
        let rows = serde_json::json!([
            [
                "2026-07-15",
                "10.00",
                "10.00",
                "10.20",
                "9.90",
                "1000",
                "100000"
            ],
            [
                "2026-07-16",
                "10.00",
                "10.10",
                "10.20",
                "9.95",
                "1100",
                "101000"
            ]
        ]);
        for field in ["qfqday", "day"] {
            let parsed = GtimgProvider::parse_kline_response_internal(
                &kline_body(field, rows.clone()),
                "600519",
            )
            .unwrap();
            assert_eq!(parsed.len(), 2);
            assert_eq!(
                parsed[0].date,
                NaiveDate::from_ymd_opt(2026, 7, 16).unwrap()
            );
            assert_eq!(parsed[0].amount, 101_000.0);
            assert!((parsed[0].pct_chg - 1.0).abs() < 1e-12);
            assert_eq!(parsed[0].adjust, crate::data_provider::AdjustType::Qfq);
        }
    }

    #[test]
    fn br125_tencent_parser_rejects_incomplete_or_bad_batches() {
        let cases = [
            kline_body("qfqday", serde_json::json!([])),
            "{}".to_string(),
            kline_body("qfqday", serde_json::json!(["not-array"])),
            kline_body("qfqday", serde_json::json!([["2026-07-16", "10"]])),
            kline_body(
                "qfqday",
                serde_json::json!([[1, "10", "10", "10", "10", "1", "1"]]),
            ),
            kline_body(
                "qfqday",
                serde_json::json!([["bad", "10", "10", "10", "10", "1", "1"]]),
            ),
            kline_body(
                "qfqday",
                serde_json::json!([["2026-07-16", 10, "10", "10", "10", "1", "1"]]),
            ),
            kline_body(
                "qfqday",
                serde_json::json!([["2026-07-16", "bad", "10", "10", "10", "1", "1"]]),
            ),
            kline_body(
                "qfqday",
                serde_json::json!([["2026-07-16", "10", "10", "9", "10", "1", "1"]]),
            ),
            kline_body(
                "qfqday",
                serde_json::json!([["2026-07-16", "10", "10", "10", "10", "-1", "1"]]),
            ),
            kline_body(
                "qfqday",
                serde_json::json!([["2026-07-16", "10", "10", "10", "10", "1", "NaN"]]),
            ),
            kline_body(
                "qfqday",
                serde_json::json!([
                    ["2026-07-16", "10", "10", "10", "10", "1", "1"],
                    ["2026-07-16", "10", "10", "10", "10", "1", "1"]
                ]),
            ),
            kline_body(
                "qfqday",
                serde_json::json!([
                    ["2026-07-14", "10", "10", "10", "10", "1", "1"],
                    ["2026-07-16", "10", "10", "10", "10", "1", "1"]
                ]),
            ),
            kline_body(
                "qfqday",
                serde_json::json!([
                    ["2026-07-15", "10", "10", "10", "10", "1", "1"],
                    ["2026-07-16", "13", "13", "13", "13", "1", "1"]
                ]),
            ),
        ];
        for body in cases {
            assert!(
                GtimgProvider::parse_kline_response_internal(&body, "600519").is_err(),
                "body={body}"
            );
        }
        assert!(GtimgProvider::parse_kline_response_internal("not-json", "600519").is_err());
        assert!(GtimgProvider::parse_kline_response_internal(
            &kline_body(
                "qfqday",
                serde_json::json!([["2026-07-16", "10", "10", "10", "10", "1", "1"]])
            ),
            "bad-code",
        )
        .is_err());
    }

    #[test]
    fn br097_tencent_source_timestamp_is_parsed_as_shanghai_time() {
        let observed_at = parse_tencent_source_time("20260718101530", "TEST_CODE_000001")
            .expect("source timestamp must parse");
        assert_eq!(observed_at.to_rfc3339(), "2026-07-18T02:15:30+00:00");
    }

    #[test]
    fn br097_tencent_source_timestamp_rejects_missing_or_invalid_values() {
        assert!(parse_tencent_source_time("", "TEST_CODE_000001").is_err());
        assert!(parse_tencent_source_time("20260718999999", "TEST_CODE_000001").is_err());
    }

    #[test]
    fn test_normalize_for_tencent_formats() {
        let cases = vec![
            ("300114", "300114", "sz300114"),
            ("sz300114", "300114", "sz300114"),
            ("300114.SZ", "300114", "sz300114"),
            ("600519", "600519", "sh600519"),
            ("sh600519", "600519", "sh600519"),
            ("600519.SH", "600519", "sh600519"),
            ("bj430047", "430047", "bj430047"),
            ("430047.BJ", "430047", "bj430047"),
            ("800001", "800001", "bj800001"),
            ("500001", "500001", "sh500001"),
            ("700001", "700001", "sz700001"),
            (" TEST_CODE_000001 ", "000001", "sz000001"),
        ];

        for (input, expected_code, expected_market_code) in cases {
            let (code, market_code) =
                GtimgProvider::normalize_for_tencent(input).expect("规范化不应失败");
            assert_eq!(code, expected_code);
            assert_eq!(market_code, expected_market_code);
        }

        for invalid in [
            "",
            " ",
            "sh12345",
            "sz12345x",
            "bj1234567",
            "12345",
            "123456.HK",
            "12345.SH",
            "abcdef",
        ] {
            assert_eq!(GtimgProvider::normalize_for_tencent(invalid), None);
        }
    }

    #[test]
    fn loopback_provider_interface_uses_strict_real_protocol_adapters() {
        use super::super::{loopback_http_client, TestHttpResponse, TestHttpServer};

        let server = TestHttpServer::new(vec![
            TestHttpResponse::json(complete_kline_body()),
            TestHttpResponse::json(realtime_body(&[])),
            TestHttpResponse::json("v_sh600519=\"51~接口股票~600519~10.10\";"),
            TestHttpResponse::json(realtime_body(&[])),
        ]);
        let base = server.base_url().to_string();
        let provider = GtimgProvider::with_bases(loopback_http_client(), base.clone(), base);

        let data = provider
            .get_daily_data("TEST_CODE_600519", 2)
            .expect("provider must return the strictly parsed loopback batch");
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].intraday_price, Some(10.10));
        assert_eq!(
            provider.get_stock_name("TEST_CODE_600519").as_deref(),
            Some("接口股票")
        );
        let quote = provider
            .fetch_realtime_quote("TEST_CODE_000001")
            .expect("provider quote transport")
            .expect("complete quote");
        assert_eq!(quote.code, "000001");
        assert_eq!(provider.name(), "腾讯财经");

        assert_eq!(
            server.finish(),
            vec![
                "/appstock/app/fqkline/get?param=sh600519,day,,,2,qfq",
                "/q=sh600519",
                "/q=sh600519",
                "/q=sz000001",
            ]
        );

        let default = GtimgProvider::default();
        assert_eq!(default.kline_base, "https://web.ifzq.gtimg.cn");
        assert_eq!(default.quote_base, "http://qt.gtimg.cn");
    }

    #[test]
    fn test_get_stock_name() {
        let provider = GtimgProvider::new().unwrap();

        let test_codes = vec![
            "600519", // 贵州茅台
            "000001", // 平安银行
            "002413", // 雷科防务
        ];

        for code in test_codes {
            if let Some(name) = provider.get_stock_name(code) {
                println!("{} -> {}", code, name);
                assert!(!name.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn real_tencent_transports_fail_without_quote_or_kline_fallback() {
        let client = super::super::unreachable_http_client();
        assert!(
            GtimgProvider::fetch_kline_data_internal(&client, "TEST_CODE_000001", 5)
                .await
                .is_err()
        );
        assert_eq!(
            GtimgProvider::fetch_stock_name_internal(&client, "TEST_CODE_000001").await,
            None
        );
        assert!(
            GtimgProvider::fetch_realtime_quote_internal(&client, "TEST_CODE_000001")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn loopback_tencent_transports_parse_complete_kline_quote_and_name() {
        use super::super::{loopback_http_client, TestHttpResponse, TestHttpServer};

        let server = TestHttpServer::new(vec![
            TestHttpResponse::json(complete_kline_body()),
            TestHttpResponse::json(realtime_body(&[])),
        ]);
        let base = server.base_url().to_string();
        let data = GtimgProvider::fetch_kline_data_from_bases(
            &loopback_http_client(),
            "TEST_CODE_600519",
            2,
            &base,
            &base,
        )
        .await
        .expect("complete K-line and quote responses must parse");
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].intraday_price, Some(10.10));
        assert_eq!(data[0].pb_ratio, Some(2.0));
        let requests = server.finish();
        assert!(requests[0].contains("param=sh600519,day,,,2,qfq"));
        assert_eq!(requests[1], "/q=sh600519");

        let server = TestHttpServer::new(vec![TestHttpResponse::json(
            "v_sh600519=\"51~协议测试股票~600519~10.10\";",
        )]);
        let base = server.base_url().to_string();
        let name = GtimgProvider::fetch_stock_name_from_base(
            &loopback_http_client(),
            "TEST_CODE_600519",
            &base,
        )
        .await;
        assert_eq!(name.as_deref(), Some("协议测试股票"));
        assert_eq!(server.finish(), vec!["/q=sh600519"]);

        let server = TestHttpServer::new(vec![TestHttpResponse {
            status: 503,
            body: "unavailable".to_string(),
        }]);
        let base = server.base_url().to_string();
        let error = GtimgProvider::fetch_realtime_quote_from_base(
            &loopback_http_client(),
            "TEST_CODE_600519",
            &base,
        )
        .await
        .expect_err("non-2xx quote transport must fail");
        assert!(error.to_string().contains("503"));
        assert_eq!(server.finish(), vec!["/q=sh600519"]);
    }

    #[test]
    fn test_get_daily_data() {
        let provider = GtimgProvider::new().unwrap();
        let result = provider.get_daily_data("600519", 30);

        match result {
            Ok(data) => {
                assert!(!data.is_empty(), "数据不应为空");
                println!("获取到 {} 条数据", data.len());
                if let Some(first) = data.first() {
                    println!("最新数据: {:?}", first);
                    assert!(first.close > 0.0, "收盘价应大于0");
                }
            }
            Err(e) => {
                println!("获取数据失败（可能是网络问题）: {}", e);
            }
        }
    }

    /// v13.10.6: 验证 review #14 修复后, parse 不再吞掉股票名
    /// 腾讯返回: `v_sz002413="51~雷科防务~002413~15.00~..."`
    /// 旧 splitn(2, '~').collect()[1] = "雷科防务~002413~15.00~..." (整个尾部)
    /// 新 splitn(3, '~').nth(1) = "雷科防务" (第二个字段)
    #[test]
    fn test_parse_name_does_not_inject_quote_data() {
        let data = "v_sz002413=\"51~雷科防务~002413~15.00~15.50~52080~24286~27817\";";
        let name = GtimgProvider::parse_stock_name_response(data);
        assert_eq!(name.as_deref(), Some("雷科防务"), "应只取第二个字段");
    }

    /// v13.10.6: 单元素 (异常格式) 应返回 None
    #[test]
    fn test_parse_name_handles_short_response() {
        for data in ["51", "v=\"51\";", "v=\"51~~002413\";", "v=\"51~name"] {
            assert!(
                GtimgProvider::parse_stock_name_response(data).is_none(),
                "异常格式应返回 None: {data}"
            );
        }
    }

    fn realtime_body(overrides: &[(usize, &str)]) -> String {
        let mut parts = vec!["0".to_string(); 54];
        parts[1] = "测试股票".to_string();
        parts[3] = "10.10".to_string();
        parts[4] = "10.00".to_string();
        parts[6] = "1234".to_string();
        parts[30] = "20260718101530".to_string();
        parts[37] = "12.5".to_string();
        parts[38] = "2.5".to_string();
        parts[39] = "15.0".to_string();
        parts[44] = "100.0".to_string();
        parts[45] = "120.0".to_string();
        parts[46] = "2.0".to_string();
        parts[47] = "11.00".to_string();
        parts[48] = "9.00".to_string();
        parts[52] = "16.0".to_string();
        for (index, value) in overrides {
            parts[*index] = (*value).to_string();
        }
        format!("v_sz000001=\"{}\";", parts.join("~"))
    }

    #[test]
    fn realtime_response_preserves_complete_quote_evidence() {
        let quote =
            GtimgProvider::parse_realtime_quote_response(&realtime_body(&[]), "TEST_CODE_000001")
                .unwrap()
                .unwrap();
        assert_eq!(quote.code, "000001");
        assert_eq!(quote.name, "测试股票");
        assert_eq!(quote.price, 10.10);
        assert!((quote.pct_chg - 1.0).abs() < 1e-12);
        assert_eq!(quote.volume, Some(123_400.0));
        assert_eq!(quote.amount, Some(125_000.0));
        assert_eq!(quote.pe_ratio, Some(16.0));
        assert_eq!(quote.pb_ratio, Some(2.0));
        assert_eq!(quote.limit_up_price, Some(11.0));
        assert_eq!(quote.limit_down_price, Some(9.0));
        assert_eq!(quote.source_time.to_rfc3339(), "2026-07-18T02:15:30+00:00");
    }

    #[test]
    fn realtime_response_rejects_protocol_and_price_failures() {
        for body in [
            "no quotes".to_string(),
            "v=\"short\";".to_string(),
            realtime_body(&[(3, "0")]),
            realtime_body(&[(4, "NaN")]),
            realtime_body(&[(30, "bad")]),
            realtime_body(&[(47, "8.00"), (48, "9.00")]),
        ] {
            assert!(
                GtimgProvider::parse_realtime_quote_response(&body, "TEST_CODE_000001").is_err(),
                "body={body}"
            );
        }
        assert!(
            GtimgProvider::parse_realtime_quote_response(&realtime_body(&[]), "bad-code").is_err()
        );
    }

    #[test]
    fn realtime_optional_metrics_remain_absent_when_invalid() {
        let quote = GtimgProvider::parse_realtime_quote_response(
            &realtime_body(&[
                (6, "bad"),
                (37, "-1"),
                (38, "NaN"),
                (39, "-1"),
                (44, "bad"),
                (45, "-1"),
                (46, "bad"),
                (52, "-1"),
            ]),
            "TEST_CODE_000001",
        )
        .unwrap()
        .unwrap();
        assert_eq!(quote.volume, None);
        assert_eq!(quote.amount, None);
        assert_eq!(quote.turnover_rate, None);
        assert_eq!(quote.market_cap, None);
        assert_eq!(quote.circulating_cap, None);
        assert_eq!(quote.pb_ratio, None);
        assert_eq!(quote.pe_ratio, None);
    }

    #[test]
    fn test_different_markets() {
        let provider = GtimgProvider::new().unwrap();

        // 测试不同市场的股票
        let test_codes = vec![("600519", "上海"), ("000001", "深圳"), ("300750", "创业板")];

        for (code, market) in test_codes {
            println!("\n测试 {} 市场股票: {}", market, code);
            match provider.get_daily_data(code, 5) {
                Ok(data) => {
                    println!("  成功获取 {} 条数据", data.len());
                    if let Some(first) = data.first() {
                        println!("  最新: {} 收盘={}", first.date, first.close);
                    }
                }
                Err(e) => {
                    println!("  失败: {}", e);
                }
            }
        }
    }
}
