//! HTTP数据提供者
//!
//! 通过HTTP API直接获取股票数据
//! 参考market_analyzer的实现，使用东方财富API

use super::limit_status::apply_limit_flags_inplace;
use super::{DataProvider, KlineData};
use crate::errors::ProviderError;
use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use serde::Deserialize;

fn kline_retry_delay(attempt: u32) -> std::time::Duration {
    if cfg!(test) {
        std::time::Duration::from_millis(1)
    } else {
        std::time::Duration::from_millis(500 * u64::from(attempt))
    }
}

fn eastmoney_market_code(code: &str) -> String {
    if code.starts_with("00")
        || code.starts_with("30")
        || code.starts_with("15")
        || code.starts_with("16")
    {
        format!("0.{code}")
    } else {
        format!("1.{code}")
    }
}

fn brief_provider_error(text: String) -> String {
    const MAX: usize = 120;
    if text.chars().count() <= MAX {
        text
    } else {
        format!("{}…(截断)", text.chars().take(MAX).collect::<String>())
    }
}

enum KlineAttemptOutcome {
    Complete(Vec<KlineData>),
    Retry(ProviderError),
    Fatal(anyhow::Error),
}

/// HTTP数据提供者
pub struct HttpProvider {
    client: reqwest::Client,
    kline_bases: Vec<String>,
    quote_bases: Vec<String>,
    max_attempts_per_host: u32,
}

/// API返回的K线数据
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ApiKlineData {
    #[serde(rename = "日期")]
    date: String,
    #[serde(rename = "开盘")]
    open: f64,
    #[serde(rename = "收盘")]
    close: f64,
    #[serde(rename = "最高")]
    high: f64,
    #[serde(rename = "最低")]
    low: f64,
    #[serde(rename = "成交量")]
    volume: f64,
    #[serde(rename = "成交额")]
    amount: f64,
    #[serde(rename = "涨跌幅")]
    pct_chg: f64,
}

/// API响应
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ApiResponse {
    data: Option<ApiDataWrapper>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ApiDataWrapper {
    klines: Option<Vec<String>>,
}

impl HttpProvider {
    fn parse_stock_name_response(text: &str) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(text)
            .ok()?
            .get("data")?
            .get("f58")?
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .map(str::to_string)
    }

    fn decide_kline_attempt(
        status: u16,
        body: std::result::Result<String, String>,
        code: &str,
    ) -> KlineAttemptOutcome {
        if (400..500).contains(&status) {
            return KlineAttemptOutcome::Fatal(anyhow!("HTTP请求返回错误状态: {status}"));
        }
        if !(200..300).contains(&status) {
            return KlineAttemptOutcome::Retry(ProviderError::Other {
                provider: "eastmoney".into(),
                detail: format!("HTTP请求返回错误状态: {status}"),
            });
        }
        let text = match body {
            Ok(text) => text,
            Err(error) => {
                return KlineAttemptOutcome::Retry(ProviderError::ParseError {
                    detail: format!("读取响应失败: {}", brief_provider_error(error)),
                });
            }
        };
        if text.is_empty() {
            return KlineAttemptOutcome::Retry(ProviderError::ParseError {
                detail: format!("所有 host 返回空响应, code={code}"),
            });
        }
        match Self::parse_kline_response_internal(&text, code) {
            Ok(data) => KlineAttemptOutcome::Complete(data),
            Err(error) => KlineAttemptOutcome::Retry(ProviderError::ParseError {
                detail: format!("解析失败: {error} - {}", brief_provider_error(text)),
            }),
        }
    }

    /// 创建新的提供者
    pub fn new() -> Result<Self> {
        // 修复 Top10#7 (2026-06-29 audit): 用 crate::http_client::SHARED_HTTP_CLIENT 共享 client
        // 替代每次 new() 新建. 4 个 HttpProvider 实例共享同一个 client, 节省 4 倍握手.
        Ok(Self {
            client: crate::http_client::SHARED_HTTP_CLIENT.clone(),
            kline_bases: vec![
                "https://push2his.eastmoney.com".into(),
                "https://push2his-bak.eastmoney.com".into(),
                "https://82.push2his.eastmoney.com".into(),
            ],
            quote_bases: vec![
                "https://push2.eastmoney.com".into(),
                "https://push2delay.eastmoney.com".into(),
                "https://82.push2.eastmoney.com".into(),
            ],
            max_attempts_per_host: 2,
        })
    }

    #[cfg(test)]
    fn with_bases(
        client: reqwest::Client,
        kline_bases: Vec<String>,
        quote_bases: Vec<String>,
        max_attempts_per_host: u32,
    ) -> Self {
        Self {
            client,
            kline_bases,
            quote_bases,
            max_attempts_per_host,
        }
    }

    /// 从东方财富API获取K线数据（异步版本）
    ///
    /// 网络层错误（DNS/连接/超时/对端 RST）会做最多 2 次重试，500ms / 1000ms 退避；
    /// HTTP 4xx 等业务错误不重试，直接返回。
    pub async fn fetch_kline_data_internal(
        client: &reqwest::Client,
        code: &str,
        days: usize,
    ) -> Result<Vec<KlineData>> {
        const KLINE_BASES: [&str; 3] = [
            "https://push2his.eastmoney.com",
            "https://push2his-bak.eastmoney.com",
            "https://82.push2his.eastmoney.com",
        ];
        Self::fetch_kline_data_from_bases(client, code, days, &KLINE_BASES, 2).await
    }

    async fn fetch_kline_data_from_bases(
        client: &reqwest::Client,
        code: &str,
        days: usize,
        bases: &[&str],
        max_attempts_per_host: u32,
    ) -> Result<Vec<KlineData>> {
        if bases.is_empty() || max_attempts_per_host == 0 {
            return Err(anyhow!("东方财富 K 线主机配置为空"));
        }

        // 转换股票代码格式 (600519 -> 1.600519 for Shanghai, 000001 -> 0.000001 for Shenzhen)
        let market_code = eastmoney_market_code(code);

        // 每个 host 最多尝试 2 次，总尝试数 = host 数 * 2。
        let max_attempts: u32 = (bases.len() as u32) * max_attempts_per_host;
        let mut last_err: Option<ProviderError> = None;

        for attempt in 1..=max_attempts {
            let base = bases[((attempt - 1) as usize) % bases.len()].trim_end_matches('/');
            let url = format!(
                "{}/api/qt/stock/kline/get?secid={}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58&klt=101&fqt=1&end=20500101&lmt={}",
                base, market_code, days
            );

            log::debug!("[HTTP] 请求URL(host={}): {}", base, url);

            // 发送请求（添加更多请求头模拟浏览器）
            let send_result = client
                .get(&url)
                .header("Accept", "application/json, text/plain, */*")
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .header("Referer", "https://quote.eastmoney.com/")
                .header("Connection", "keep-alive")
                .send()
                .await;

            let response = match send_result {
                Ok(resp) => resp,
                Err(e) => {
                    // 网络层错误 → 重试
                    log::warn!(
                        "[HTTP] 请求失败 (attempt {}/{} host={} code={}): {}",
                        attempt,
                        max_attempts,
                        base,
                        code,
                        brief_provider_error(e.to_string())
                    );
                    last_err = Some(ProviderError::Other {
                        provider: "eastmoney".into(),
                        detail: format!("HTTP请求失败: {}", brief_provider_error(e.to_string())),
                    });
                    if attempt < max_attempts {
                        tokio::time::sleep(kline_retry_delay(attempt)).await;
                    }
                    continue;
                }
            };

            let status = response.status().as_u16();
            let body = response.text().await.map_err(|error| error.to_string());
            match Self::decide_kline_attempt(status, body, code) {
                KlineAttemptOutcome::Complete(data) => return Ok(data),
                KlineAttemptOutcome::Fatal(error) => {
                    log::error!(
                        "[HTTP] 不可重试响应 (attempt {attempt}/{max_attempts} host={host} code={code}): {error}"
                        , host = base
                    );
                    return Err(error);
                }
                KlineAttemptOutcome::Retry(error) => {
                    log::warn!(
                        "[HTTP] 可重试响应失败 (attempt {attempt}/{max_attempts} host={host} code={code}): {error}"
                        , host = base
                    );
                    last_err = Some(error);
                    if attempt < max_attempts {
                        tokio::time::sleep(kline_retry_delay(attempt)).await;
                    }
                }
            }
        }

        // 所有重试都失败：打印一次终态错误，避免上游再次重复输出
        let err = last_err
            .map(anyhow::Error::from)
            .unwrap_or_else(|| anyhow!("HTTP请求失败（未知错误）"));
        log::error!("[HTTP] 重试 {} 次后仍失败: {}", max_attempts, err);
        Err(err)
    }

    /// 解析K线响应（静态方法）
    fn parse_kline_response_internal(text: &str, code: &str) -> Result<Vec<KlineData>> {
        use serde_json::Value;

        let json: Value = serde_json::from_str(text).context("解析JSON失败")?;

        let klines = json["data"]["klines"]
            .as_array()
            .ok_or_else(|| anyhow!("未找到klines数据"))?;

        let mut result = Vec::with_capacity(klines.len());

        for (index, kline_str) in klines.iter().enumerate() {
            let kline_str = kline_str
                .as_str()
                .ok_or_else(|| anyhow!("K线第 {} 行不是字符串", index + 1))?;

            // review #14: splitn(8, ',') 限制切分数量, 11 字段也只切 8 个 Vec 元素.
            // 格式: "2026-01-22,10.0,10.5,9.8,10.3,1000000,10000000,2.5,0,0,0"
            let parts: Vec<&str> = kline_str.splitn(8, ',').collect();

            if parts.len() < 8 {
                return Err(anyhow!(
                    "K线第 {} 行字段不足: expected>=8 actual={} raw={}",
                    index + 1,
                    parts.len(),
                    kline_str
                ));
            }

            let date = NaiveDate::parse_from_str(parts[0], "%Y-%m-%d")
                .context(format!("解析日期失败: {}", parts[0]))?;

            let kline = KlineData {
                date,
                open: parts[1].parse()?,
                close: parts[2].parse()?,
                high: parts[3].parse()?,
                low: parts[4].parse()?,
                volume: parts[5].parse()?,
                amount: parts[6].parse()?,
                pct_chg: parts[7].parse()?,
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
                adjust: crate::data_provider::AdjustType::Qfq, // 东财 URL fqt=1 前复权
            };

            result.push(kline);
        }

        // BR-125: reject the complete batch on any price/date/continuity failure;
        // the shared validator restores newest-first order.
        super::validate_kline_series_strict(&mut result, code)?;

        Ok(result)
    }

    /// 获取股票名称（静态异步方法）
    #[cfg(test)]
    async fn fetch_stock_name_internal(client: &reqwest::Client, code: &str) -> Option<String> {
        const QUOTE_BASES: [&str; 3] = [
            "https://push2.eastmoney.com",
            "https://push2delay.eastmoney.com",
            "https://82.push2.eastmoney.com",
        ];
        Self::fetch_stock_name_from_bases(client, code, &QUOTE_BASES).await
    }

    async fn fetch_stock_name_from_bases<S: AsRef<str>>(
        client: &reqwest::Client,
        code: &str,
        quote_bases: &[S],
    ) -> Option<String> {
        // 转换股票代码格式
        let market_code = if code.starts_with('6') {
            format!("1.{}", code) // 上海
        } else {
            format!("0.{}", code) // 深圳/创业板/科创板
        };

        for base in quote_bases {
            let base = base.as_ref().trim_end_matches('/');
            let url = format!("{base}/api/qt/stock/get?secid={market_code}&fields=f58");

            match client
                .get(&url)
                .header("Accept", "application/json, text/plain, */*")
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .header("Referer", "https://quote.eastmoney.com/")
                .send()
                .await
            {
                Ok(response) => {
                    if let Ok(text) = response.text().await {
                        if let Some(name) = Self::parse_stock_name_response(&text) {
                            log::debug!("[HTTP] 获取股票名称(host={}): {} -> {}", base, code, name);
                            return Some(name);
                        }
                    }
                }
                Err(e) => {
                    log::debug!("[HTTP] 获取股票名称失败(host={}): {}", base, e);
                }
            }
        }

        None
    }
}

impl Default for HttpProvider {
    fn default() -> Self {
        Self::new().expect("创建HttpProvider失败")
    }
}

impl DataProvider for HttpProvider {
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        log::info!("[HTTP] 获取股票 {} 最近 {} 天数据", code, days);

        // 克隆必要的数据用于 async block
        let client = self.client.clone();
        let code = code.to_string();
        let kline_bases = self.kline_bases.clone();
        let max_attempts_per_host = self.max_attempts_per_host;

        // 修复 Top10#5 (2026-06-29 audit): 用统一 block_on_async 替代 block_in_place + Handle::current().block_on
        let code_for_apply = code.clone();
        let mut data = crate::block_on_async(async move {
            let bases = kline_bases.iter().map(String::as_str).collect::<Vec<_>>();
            Self::fetch_kline_data_from_bases(&client, &code, days, &bases, max_attempts_per_host)
                .await
        })?;

        // v11-P0-3 commit 2: K 线缺口推断 → 喂入 HALTED_PERIODS
        super::halt_status::infer_halt_from_kline_gaps(&code_for_apply, &data);

        // 填涨跌停 / 停牌标记
        // 名称暂用 None（保守按非 ST 处理）；后续实时行情步骤会再覆盖
        apply_limit_flags_inplace(&code_for_apply, None, &mut data);

        log::info!("[HTTP] 成功获取 {} 条数据", data.len());

        Ok(data)
    }
    fn get_stock_name(&self, code: &str) -> Option<String> {
        let client = self.client.clone();
        let code_str = code.to_string();
        let quote_bases = self.quote_bases.clone();

        // 修复 Top10#5: 用统一 block_on_async 替代
        let result = crate::block_on_async(async move {
            Self::fetch_stock_name_from_bases(&client, &code_str, &quote_bases).await
        });

        result
    }
    fn name(&self) -> &'static str {
        "HTTP(东方财富)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kline_body(rows: serde_json::Value) -> String {
        serde_json::json!({"data": {"klines": rows}}).to_string()
    }

    #[test]
    fn real_http_attempt_decision_preserves_terminal_retry_and_complete_states() {
        assert_eq!(eastmoney_market_code("000001"), "0.000001");
        assert_eq!(eastmoney_market_code("300001"), "0.300001");
        assert_eq!(eastmoney_market_code("150001"), "0.150001");
        assert_eq!(eastmoney_market_code("160001"), "0.160001");
        assert_eq!(eastmoney_market_code("600519"), "1.600519");
        assert!(brief_provider_error("x".repeat(121)).contains("截断"));
        assert_eq!(brief_provider_error("short".into()), "short");

        match HttpProvider::decide_kline_attempt(404, Ok(String::new()), "600519") {
            KlineAttemptOutcome::Fatal(error) => assert!(error.to_string().contains("404")),
            _ => panic!("4xx must be terminal"),
        }
        for (status, body, expected) in [
            (503, Ok("ignored".to_string()), "503"),
            (200, Err("body failed".to_string()), "读取响应失败"),
            (200, Ok(String::new()), "空响应"),
            (200, Ok("not-json".to_string()), "解析失败"),
        ] {
            match HttpProvider::decide_kline_attempt(status, body, "600519") {
                KlineAttemptOutcome::Retry(error) => {
                    assert!(error.to_string().contains(expected), "{error}")
                }
                _ => panic!("status/body should remain retryable"),
            }
        }

        let complete = kline_body(serde_json::json!([
            "2026-07-15,10.00,10.00,10.20,9.90,1000,100000,0.0",
            "2026-07-16,10.00,10.10,10.20,9.95,1100,101000,1.0"
        ]));
        match HttpProvider::decide_kline_attempt(200, Ok(complete), "600519") {
            KlineAttemptOutcome::Complete(data) => assert_eq!(data.len(), 2),
            _ => panic!("complete strict body must succeed"),
        }
    }

    #[test]
    fn stock_name_parser_requires_complete_nonempty_protocol_identity() {
        assert_eq!(
            HttpProvider::parse_stock_name_response(r#"{"data":{"f58":"贵州茅台"}}"#),
            Some("贵州茅台".to_string())
        );
        for body in [
            "not-json",
            r#"{}"#,
            r#"{"data":{}}"#,
            r#"{"data":{"f58":null}}"#,
            r#"{"data":{"f58":"  "}}"#,
        ] {
            assert_eq!(
                HttpProvider::parse_stock_name_response(body),
                None,
                "{body}"
            );
        }
    }

    #[test]
    fn br125_complete_eastmoney_batch_is_strict_and_newest_first() {
        let body = kline_body(serde_json::json!([
            "2026-07-15,10.00,10.00,10.20,9.90,1000,100000,0.0",
            "2026-07-16,10.00,10.10,10.20,9.95,1100,101000,1.0"
        ]));
        let parsed = HttpProvider::parse_kline_response_internal(&body, "600519").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0].date,
            NaiveDate::from_ymd_opt(2026, 7, 16).unwrap()
        );
        assert_eq!(parsed[0].close, 10.1);
        assert_eq!(parsed[0].amount, 101_000.0);
        assert_eq!(parsed[0].pct_chg, 1.0);
        assert_eq!(parsed[0].adjust, crate::data_provider::AdjustType::Qfq);
    }

    #[test]
    fn br125_eastmoney_parser_rejects_incomplete_or_bad_batches() {
        let cases = [
            kline_body(serde_json::json!([])),
            "{}".to_string(),
            kline_body(serde_json::json!([1])),
            kline_body(serde_json::json!(["bad-date,10,10,10,10,1,1,0"])),
            kline_body(serde_json::json!(["2026-07-16,bad,10,10,10,1,1,0"])),
            kline_body(serde_json::json!(["2026-07-16,10,10,9,10,1,1,0"])),
            kline_body(serde_json::json!(["2026-07-16,10,10,10,10,-1,1,0"])),
            kline_body(serde_json::json!(["2026-07-16,10,10,10,10,1,-1,0"])),
            kline_body(serde_json::json!(["2026-07-16,10,10,10,10,1,1,20.1"])),
            kline_body(serde_json::json!([
                "2026-07-16,10,10,10,10,1,1,0",
                "2026-07-16,10,10,10,10,1,1,0"
            ])),
            kline_body(serde_json::json!([
                "2026-07-14,10,10,10,10,1,1,0",
                "2026-07-16,10,10,10,10,1,1,0"
            ])),
            kline_body(serde_json::json!([
                "2026-07-15,10,10,10,10,1,1,0",
                "2026-07-16,13,13,13,13,1,1,0"
            ])),
        ];
        for body in cases {
            assert!(
                HttpProvider::parse_kline_response_internal(&body, "600519").is_err(),
                "body={body}"
            );
        }
        assert!(HttpProvider::parse_kline_response_internal("not-json", "600519").is_err());
    }

    #[test]
    fn br092_kline_parser_rejects_any_short_row() {
        let body = r#"{"data":{"klines":["2026-07-16,10,10.2,10.3,9.9,1000,10000,2.0","2026-07-17,10,10.2"]}}"#;
        let error = HttpProvider::parse_kline_response_internal(body, "600519")
            .expect_err("a malformed row must reject the complete provider batch");
        assert!(error.to_string().contains("第 2 行"));
    }

    /// 在 tokio runtime 的 spawn_blocking 中执行阻塞调用
    async fn with_provider<F, T>(f: F) -> T
    where
        F: FnOnce(&HttpProvider) -> T + Send + 'static,
        T: Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            let provider = HttpProvider::new().unwrap();
            f(&provider)
        })
        .await
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_stock_name() {
        with_provider(|p| {
            for code in ["600519", "000001", "002413"] {
                if let Some(name) = p.get_stock_name(code) {
                    println!("{} -> {}", code, name);
                    assert!(!name.is_empty());
                }
            }
        })
        .await;
    }

    #[tokio::test]
    async fn real_eastmoney_transports_exhaust_hosts_without_default_data() {
        let client = super::super::unreachable_http_client();
        let error = HttpProvider::fetch_kline_data_internal(&client, "TEST_CODE_000001", 5)
            .await
            .expect_err("all real K-line hosts are unreachable");
        assert!(error.to_string().contains("HTTP请求失败"));
        assert_eq!(
            HttpProvider::fetch_stock_name_internal(&client, "TEST_CODE_000001").await,
            None
        );
        assert_eq!(kline_retry_delay(3), std::time::Duration::from_millis(1));
    }

    #[tokio::test]
    async fn loopback_kline_transport_preserves_retry_terminal_and_complete_states() {
        use super::super::{loopback_http_client, TestHttpResponse, TestHttpServer};

        let complete = kline_body(serde_json::json!([
            "2026-07-15,10.00,10.00,10.20,9.90,1000,100000,0.0",
            "2026-07-16,10.00,10.10,10.20,9.95,1100,101000,1.0"
        ]));
        let server = TestHttpServer::new(vec![
            TestHttpResponse {
                status: 503,
                body: "temporarily unavailable".to_string(),
            },
            TestHttpResponse::json(complete),
        ]);
        let base = server.base_url().to_string();
        let data = HttpProvider::fetch_kline_data_from_bases(
            &loopback_http_client(),
            "TEST_CODE_000001",
            2,
            &[&base],
            2,
        )
        .await
        .expect("retry must reach the complete second response");
        assert_eq!(data.len(), 2);
        let requests = server.finish();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("secid=1.TEST_CODE_000001"));
        assert!(requests[0].contains("lmt=2"));

        let server = TestHttpServer::new(vec![TestHttpResponse {
            status: 404,
            body: "not found".to_string(),
        }]);
        let base = server.base_url().to_string();
        let error = HttpProvider::fetch_kline_data_from_bases(
            &loopback_http_client(),
            "TEST_CODE_000001",
            2,
            &[&base],
            2,
        )
        .await
        .expect_err("4xx must terminate without a fabricated batch");
        assert!(error.to_string().contains("404"));
        assert_eq!(server.finish().len(), 1);

        assert!(HttpProvider::fetch_kline_data_from_bases(
            &loopback_http_client(),
            "TEST_CODE_000001",
            2,
            &[],
            2,
        )
        .await
        .is_err());
    }

    #[test]
    fn loopback_provider_interface_uses_configured_strict_protocol_endpoints() {
        use super::super::{loopback_http_client, TestHttpResponse, TestHttpServer};

        let complete = kline_body(serde_json::json!([
            "2026-07-15,10.00,10.00,10.20,9.90,1000,100000,0.0",
            "2026-07-16,10.00,10.10,10.20,9.95,1100,101000,1.0"
        ]));
        let server = TestHttpServer::new(vec![
            TestHttpResponse::json(complete),
            TestHttpResponse::json(r#"{"data":{"f58":"接口股票"}}"#),
        ]);
        let base = server.base_url().to_string();
        let provider =
            HttpProvider::with_bases(loopback_http_client(), vec![base.clone()], vec![base], 1);

        let data = provider
            .get_daily_data("TEST_CODE_000001", 2)
            .expect("provider interface must preserve the strict parsed batch");
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].close, 10.10);
        assert_eq!(
            provider.get_stock_name("TEST_CODE_000001").as_deref(),
            Some("接口股票")
        );
        assert_eq!(provider.name(), "HTTP(东方财富)");
        let requests = server.finish();
        assert!(requests[0].contains("secid=1.TEST_CODE_000001"));
        assert!(requests[0].contains("lmt=2"));
        assert!(requests[1].contains("fields=f58"));

        let default = HttpProvider::default();
        assert_eq!(default.kline_bases.len(), 3);
        assert_eq!(default.quote_bases.len(), 3);
        assert_eq!(default.max_attempts_per_host, 2);
    }

    #[ignore = "b013 异常处理: 实机调 eastmoney HTTP, 沙箱环境必失败 (非 deterministic)"]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_daily_data() {
        with_provider(|p| match p.get_daily_data("600519", 30) {
            Ok(data) => {
                assert!(!data.is_empty(), "数据不应为空");
                println!("获取到 {} 条数据", data.len());
                if let Some(first) = data.first() {
                    assert!(first.close > 0.0, "收盘价应大于0");
                }
            }
            Err(e) => {
                println!("获取数据失败（可能是网络问题）: {}", e);
            }
        })
        .await;
    }

    #[ignore = "b013 异常处理: 实机调 eastmoney HTTP, 沙箱环境必失败"]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_different_markets() {
        with_provider(|p| {
            for (code, market) in [("600519", "上海"), ("000001", "深圳"), ("300750", "创业板")]
            {
                println!("\n测试 {} 市场股票: {}", market, code);
                match p.get_daily_data(code, 5) {
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
        })
        .await;
    }
}
