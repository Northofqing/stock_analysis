//! HTTP数据提供者
//! 
//! 通过HTTP API直接获取股票数据
//! 参考market_analyzer的实现，使用东方财富API

use super::{DataProvider, KlineData};
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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()?;
        
        Ok(Self { client })
    }
    
    /// 从东方财富API获取K线数据（异步版本）
    async fn fetch_kline_data_internal(client: &reqwest::Client, code: &str, days: usize) -> Result<Vec<KlineData>> {
        // 转换股票代码格式 (600519 -> 1.600519 for Shanghai, 000001 -> 0.000001 for Shenzhen)
        let market_code = if code.starts_with('6') || code.starts_with("00") && code.len() == 6 {
            if code.starts_with('6') {
                format!("1.{}", code) // 上海
            } else {
                format!("0.{}", code) // 深圳
            }
        } else if code.starts_with("30") || code.starts_with("68") {
            format!("0.{}", code) // 创业板/科创板
        } else {
            format!("1.{}", code) // 默认上海
        };
        
        // 构建URL
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58&klt=101&fqt=1&end=20500101&lmt={}",
            market_code, days
        );
        
        log::debug!("[HTTP] 请求URL: {}", url);
        
        // 发送请求（添加更多请求头模拟浏览器）
        let response = client
            .get(&url)
            .header("Accept", "application/json, text/plain, */*")
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .header("Accept-Encoding", "gzip, deflate, br")
            .header("Referer", "https://quote.eastmoney.com/")
            .header("Connection", "keep-alive")
            .send()
            .await;
        
        let response = match response {
            Ok(resp) => resp,
            Err(e) => {
                log::error!("[HTTP] 请求失败: {} - URL: {}", e, url);
                return Err(anyhow!("HTTP请求失败: {}", e));
            }
        };
        
        if !response.status().is_success() {
            log::error!("[HTTP] 响应状态码: {} - URL: {}", response.status(), url);
            return Err(anyhow!("HTTP请求返回错误状态: {}", response.status()));
        }
        
        let text = response.text().await.context("读取响应失败")?;
        
        if text.is_empty() {
            log::error!("[HTTP] 响应为空 - URL: {}", url);
            return Err(anyhow!("API返回空响应"));
        }
        
        log::debug!("[HTTP] 响应前200字符: {}", &text[..text.len().min(200)]);
        
        // 解析JSON (简单的字符串解析，因为API返回的是字符串数组)
        let klines = Self::parse_kline_response_internal(&text)?;
        
        Ok(klines)
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
            };
            
            result.push(kline);
        }
        
        // 按日期降序排序（最新的在前）
        result.sort_by(|a, b| b.date.cmp(&a.date));
        
        Ok(result)
    }
    
    /// 获取股票名称（静态异步方法）
    async fn fetch_stock_name_internal(client: &reqwest::Client, code: &str) -> Option<String> {
        // 转换股票代码格式
        let market_code = if code.starts_with('6') {
            format!("1.{}", code) // 上海
        } else {
            format!("0.{}", code) // 深圳/创业板/科创板
        };
        
        let url = format!(
            "https://push2.eastmoney.com/api/qt/stock/get?secid={}&fields=f58",
            market_code
        );
        
        match client.get(&url)
            .header("Accept", "application/json, text/plain, */*")
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .header("Referer", "https://quote.eastmoney.com/")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(response) => {
                if let Ok(text) = response.text().await {
                    // 解析JSON获取名称 {"data":{"f58":"贵州茅台"}}
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(name) = json["data"]["f58"].as_str() {
                            if !name.is_empty() {
                                log::debug!("[HTTP] 获取股票名称: {} -> {}", code, name);
                                return Some(name.to_string());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                log::debug!("[HTTP] 获取股票名称失败: {}", e);
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
        
        // 使用 tokio::task::block_in_place 来运行异步代码
        let data = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                Self::fetch_kline_data_internal(&client, &code, days).await
            })
        })?;
        
        log::info!("[HTTP] 成功获取 {} 条数据", data.len());
        
        Ok(data)
    }
    fn get_stock_name(&self, code: &str) -> Option<String> {
        let client = self.client.clone();
        let code_str = code.to_string();
        
        // 使用 tokio::task::block_in_place 来运行异步代码
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                Self::fetch_stock_name_internal(&client, &code_str).await
            })
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
    
    #[test]
    fn test_get_stock_name() {
        let provider = HttpProvider::new().unwrap();
        
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
    
    #[test]
    fn test_get_daily_data() {
        let provider = HttpProvider::new().unwrap();
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
    
    #[test]
    fn test_different_markets() {
        let provider = HttpProvider::new().unwrap();
        
        // 测试不同市场的股票
        let test_codes = vec![
            ("600519", "上海"),
            ("000001", "深圳"),
            ("300750", "创业板"),
        ];
        
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
