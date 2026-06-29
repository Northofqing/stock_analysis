//! HTTP数据提供者
//! 
//! 通过HTTP API直接获取股票数据
//! 参考market_analyzer的实现，使用东方财富API

use super::limit_status::apply_limit_flags_inplace;
use super::{DataProvider, KlineData};
use crate::errors::ProviderError;
use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use serde::Deserialize;

/// HTTP数据提供者
pub struct HttpProvider {
    client: reqwest::Client,
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
    /// 创建新的提供者
    pub fn new() -> Result<Self> {
        // 修复 Top10#7 (2026-06-29 audit): 用 crate::http_client::SHARED_HTTP_CLIENT 共享 client
        // 替代每次 new() 新建. 4 个 HttpProvider 实例共享同一个 client, 节省 4 倍握手.
        Ok(Self { client: crate::http_client::SHARED_HTTP_CLIENT.clone() })
    }
    
    /// 从东方财富API获取K线数据（异步版本）
    ///
    /// 网络层错误（DNS/连接/超时/对端 RST）会做最多 2 次重试，500ms / 1000ms 退避；
    /// HTTP 4xx 等业务错误不重试，直接返回。
    pub async fn fetch_kline_data_internal(client: &reqwest::Client, code: &str, days: usize) -> Result<Vec<KlineData>> {
        const KLINE_HOSTS: [&str; 3] = [
            "push2his.eastmoney.com",
            "push2his-bak.eastmoney.com",
            "82.push2his.eastmoney.com",
        ];

        // 转换股票代码格式 (600519 -> 1.600519 for Shanghai, 000001 -> 0.000001 for Shenzhen)
        let market_code = if code.starts_with('6') {
            format!("1.{}", code) // 上海
        } else if code.starts_with("00") || code.starts_with("30") || code.starts_with("15") || code.starts_with("16") {
            format!("0.{}", code) // 深圳及深交所基金
        } else if code.starts_with("68") || code.starts_with("51") || code.starts_with("58") {
            format!("1.{}", code) // 科创板及上交所基金
        } else {
            format!("1.{}", code) // 默认上海
        };

        // 每个 host 最多尝试 2 次，总尝试数 = host 数 * 2。
        const MAX_ATTEMPTS_PER_HOST: u32 = 2;
        let max_attempts: u32 = (KLINE_HOSTS.len() as u32) * MAX_ATTEMPTS_PER_HOST;
        let mut last_err: Option<ProviderError> = None;

        // 截断超长错误信息（reqwest 错误会内嵌完整 URL），避免日志刷屏。
        fn brief(s: String) -> String {
            const MAX: usize = 120;
            if s.chars().count() <= MAX {
                s
            } else {
                let head: String = s.chars().take(MAX).collect();
                format!("{head}…(截断)")
            }
        }

        for attempt in 1..=max_attempts {
            let host = KLINE_HOSTS[((attempt - 1) as usize) % KLINE_HOSTS.len()];
            let url = format!(
                "https://{}/api/qt/stock/kline/get?secid={}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58&klt=101&fqt=1&end=20500101&lmt={}",
                host, market_code, days
            );

            log::debug!("[HTTP] 请求URL(host={}): {}", host, url);

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
                        attempt, max_attempts, host, code, brief(e.to_string())
                    );
                    last_err = Some(ProviderError::Other { provider: "eastmoney".into(), detail: format!("HTTP请求失败: {}", brief(e.to_string())) });
                    if attempt < max_attempts {
                        tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                    }
                    continue;
                }
            };

            let status = response.status();
            if !status.is_success() {
                // 4xx：客户端错误（URL/参数问题），重试也不会变好，直接失败
                if status.is_client_error() {
                    log::error!("[HTTP] 客户端错误 {} (host={} code={})", status, host, code);
                    return Err(anyhow!("HTTP请求返回错误状态: {}", status));
                }
                // 5xx：可能瞬时，重试
                log::warn!(
                    "[HTTP] 响应状态 {} (attempt {}/{} host={} code={})",
                    status, attempt, max_attempts, host, code
                );
                last_err = Some(ProviderError::Other { provider: "eastmoney".into(), detail: format!("HTTP请求返回错误状态: {}", status) });
                if attempt < max_attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                }
                continue;
            }

            let text = match response.text().await {
                Ok(t) => t,
                Err(e) => {
                    log::warn!(
                        "[HTTP] 读取响应失败 (attempt {}/{} host={} code={}): {}",
                        attempt, max_attempts, host, code, brief(e.to_string())
                    );
                    last_err = Some(ProviderError::ParseError { detail: format!("读取响应失败: {}", brief(e.to_string())) });
                    if attempt < max_attempts {
                        tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                    }
                    continue;
                }
            };

            if text.is_empty() {
                log::warn!(
                    "[HTTP] 响应为空 (attempt {}/{} host={} code={})",
                    attempt, max_attempts, host, code
                );
                last_err = Some(ProviderError::NotFound { code: code.to_string() });
                if attempt < max_attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                }
                continue;
            }

            log::debug!("[HTTP] 响应前200字符: {}", &text[..text.len().min(200)]);

            // 解析JSON (简单的字符串解析，因为API返回的是字符串数组)。
            // 200 但非 JSON（反爬/限流 HTML 页）视为可重试错误：换 host 再试，
            // 而非立刻放弃整个东方财富源。
            match Self::parse_kline_response_internal(&text) {
                Ok(data) => return Ok(data),
                Err(e) => {
                    log::warn!(
                        "[HTTP] 解析失败 (attempt {}/{} host={} code={}): {} - 响应前120字符: {}",
                        attempt,
                        max_attempts,
                        host,
                        code,
                        e,
                        brief(text.clone())
                    );
                    last_err = Some(ProviderError::ParseError { detail: format!("解析失败: {} - {}", e, brief(text.clone())) });
                    if attempt < max_attempts {
                        tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                    }
                    continue;
                }
            }
        }

        // 所有重试都失败：打印一次终态错误，避免上游再次重复输出
        let err = last_err.map(|e| anyhow::Error::from(e)).unwrap_or_else(|| anyhow!("HTTP请求失败（未知错误）"));
        log::error!("[HTTP] 重试 {} 次后仍失败: {}", max_attempts, err);
        Err(err)
    }
    
    /// 解析K线响应（静态方法）
    fn parse_kline_response_internal(text: &str) -> Result<Vec<KlineData>> {
        use serde_json::Value;
        
        let json: Value = serde_json::from_str(text)
            .context("解析JSON失败")?;
        
        let klines = json["data"]["klines"]
            .as_array()
            .ok_or_else(|| anyhow!("未找到klines数据"))?;
        
        let mut result = Vec::new();
        
        for kline_str in klines {
            let kline_str = kline_str.as_str()
                .ok_or_else(|| anyhow!("kline不是字符串"))?;
            
            // 格式: "2026-01-22,10.0,10.5,9.8,10.3,1000000,10000000,2.5,0,0,0"
            let parts: Vec<&str> = kline_str.split(',').collect();
            
            if parts.len() < 8 {
                log::warn!("[HTTP] K线数据格式错误: {}", kline_str);
                continue;
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
                intraday_price: None, settled: true,
                pe_ratio: None,        // K线数据中不包含，需要从实时行情获取
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
            };
            
            result.push(kline);
        }
        
        // 按日期降序排序（最新的在前）
        result.sort_by(|a, b| b.date.cmp(&a.date));
        
        Ok(result)
    }
    
    /// 获取股票名称（静态异步方法）
    async fn fetch_stock_name_internal(client: &reqwest::Client, code: &str) -> Option<String> {
        const QUOTE_HOSTS: [&str; 3] = [
            "push2.eastmoney.com",
            "push2delay.eastmoney.com",
            "82.push2.eastmoney.com",
        ];

        // 转换股票代码格式
        let market_code = if code.starts_with('6') {
            format!("1.{}", code) // 上海
        } else {
            format!("0.{}", code) // 深圳/创业板/科创板
        };

        for host in QUOTE_HOSTS {
            let url = format!(
                "https://{}/api/qt/stock/get?secid={}&fields=f58",
                host, market_code
            );

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
                        // 解析JSON获取名称 {"data":{"f58":"贵州茅台"}}
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(name) = json["data"]["f58"].as_str() {
                                if !name.is_empty() {
                                    log::debug!("[HTTP] 获取股票名称(host={}): {} -> {}", host, code, name);
                                    return Some(name.to_string());
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    log::debug!("[HTTP] 获取股票名称失败(host={}): {}", host, e);
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

        // 修复 Top10#5 (2026-06-29 audit): 用统一 block_on_async 替代 block_in_place + Handle::current().block_on
        let code_for_apply = code.clone();
        let mut data = crate::block_on_async(async move {
            Self::fetch_kline_data_internal(&client, &code, days).await
        })?;

        // 填涨跌停 / 停牌标记
        // 名称暂用 None（保守按非 ST 处理）；后续实时行情步骤会再覆盖
        apply_limit_flags_inplace(&code_for_apply, None, &mut data);

        log::info!("[HTTP] 成功获取 {} 条数据", data.len());

        Ok(data)
    }
    fn get_stock_name(&self, code: &str) -> Option<String> {
        let client = self.client.clone();
        let code_str = code.to_string();

        // 修复 Top10#5: 用统一 block_on_async 替代
        let result = crate::block_on_async(async move {
            Self::fetch_stock_name_internal(&client, &code_str).await
        });

        result.or_else(|| Some(format!("股票{}", code)))
    }
    fn name(&self) -> &'static str {
        "HTTP(东方财富)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
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
    async fn test_get_daily_data() {
        with_provider(|p| {
            match p.get_daily_data("600519", 30) {
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
            }
        })
        .await;
    }

    #[tokio::test]
    async fn test_different_markets() {
        with_provider(|p| {
            for (code, market) in [("600519", "上海"), ("000001", "深圳"), ("300750", "创业板")] {
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
