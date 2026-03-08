//! 腾讯财经数据提供者
//! 
//! 通过腾讯财经API获取股票数据
//! API文档: http://qt.gtimg.cn

use super::{DataProvider, KlineData, RealtimeQuote};
use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;

/// 腾讯财经数据提供者
pub struct GtimgProvider {
    client: reqwest::Client,
}

impl GtimgProvider {
    /// 创建新的提供者
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()?;
        
        Ok(Self { client })
    }
    
    /// 公开方法：获取实时行情（用于其他数据提供者调用）
    pub fn fetch_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        self.get_realtime_quote(code)
    }
    
    /// 从腾讯财经API获取K线数据（异步版本）
    async fn fetch_kline_data_internal(client: &reqwest::Client, code: &str, days: usize) -> Result<Vec<KlineData>> {
        // 转换股票代码格式 (600519 -> sh600519, 000001 -> sz000001)
        let market_code = if code.starts_with('6') {
            format!("sh{}", code) // 上海
        } else {
            format!("sz{}", code) // 深圳/创业板/科创板
        };
        
        // 腾讯财经K线API
        // ktype: day(日线), week(周线), month(月线)
        let url = format!(
            "http://web.ifzq.gtimg.cn/appstock/app/fqkline/get?param={},day,,,{},qfq",
            market_code, days
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
                log::error!("[腾讯] 请求失败: {} - URL: {}", e, url);
                return Err(anyhow!("HTTP请求失败: {}", e));
            }
        };
        
        if !response.status().is_success() {
            log::error!("[腾讯] 响应状态码: {} - URL: {}", response.status(), url);
            return Err(anyhow!("HTTP请求返回错误状态: {}", response.status()));
        }
        
        let text = response.text().await.context("读取响应失败")?;
        
        if text.is_empty() {
            log::error!("[腾讯] 响应为空 - URL: {}", url);
            return Err(anyhow!("API返回空响应"));
        }
        
        log::debug!("[腾讯] 响应前200字符: {}", &text[..text.len().min(200)]);
        
        // 解析JSON
        let mut klines = Self::parse_kline_response_internal(&text, code)?;
        
        // 获取实时行情数据，补充盈利指标到最新K线
        if !klines.is_empty() {
            log::info!("[腾讯] {} K线数据条数: {}, 最新K线日期: {}, 收盘价: {:.2}元", 
                code, klines.len(), klines[0].date, klines[0].close);
            
            if let Ok(Some(quote)) = Self::fetch_realtime_quote_internal(client, code).await {
                // 将实时数据填充到最新的K线数据中
                if let Some(latest) = klines.first_mut() {
                    let old_close = latest.close;
                    // 使用实时价格替换K线收盘价（K线收盘价是历史数据，实时价格才是当前价格）
                    latest.close = quote.price;
                    latest.pe_ratio = Some(quote.pe_ratio);
                    latest.pb_ratio = Some(quote.pb_ratio);
                    latest.turnover_rate = Some(quote.turnover_rate);
                    latest.market_cap = Some(quote.market_cap);
                    latest.circulating_cap = Some(quote.circulating_cap);
                    
                    log::info!("[腾讯] {} 更新价格: {:.2}元 -> {:.2}元, PE={:.2}, PB={:.2}, 换手率={:.2}%, 总市值={:.2}亿, 流通市值={:.2}亿", 
                        code, old_close, quote.price, quote.pe_ratio, quote.pb_ratio, quote.turnover_rate, 
                        quote.market_cap, quote.circulating_cap);
                }
            } else {
                log::warn!("[腾讯] {} 无法获取实时行情数据", code);
            }
        }
        
        Ok(klines)
    }
    
    /// 解析K线响应
    fn parse_kline_response_internal(text: &str, code: &str) -> Result<Vec<KlineData>> {
        use serde_json::Value;
        
        let json: Value = serde_json::from_str(text)
            .context("解析JSON失败")?;
        
        // 获取K线数据数组
        // 数据路径: data.{code}.day 或 data.{code}.qfqday
        let market_code = if code.starts_with('6') {
            format!("sh{}", code)
        } else {
            format!("sz{}", code)
        };
        
        let klines = json["data"][&market_code]["qfqday"]
            .as_array()
            .or_else(|| json["data"][&market_code]["day"].as_array())
            .ok_or_else(|| anyhow!("未找到K线数据"))?;
        
        let mut result: Vec<KlineData> = Vec::new();
        
        for kline in klines {
            let kline_array = kline.as_array()
                .ok_or_else(|| anyhow!("K线数据格式错误"))?;
            
            if kline_array.len() < 6 {
                log::warn!("[腾讯] K线数据字段不足: {:?}", kline_array);
                continue;
            }
            
            // 腾讯K线格式: [日期, 开, 收, 高, 低, 成交量]
            // 例: ["2026-01-23", "14.22", "15.00", "15.49", "14.22", "335918000"]
            let date_str = kline_array[0].as_str()
                .ok_or_else(|| anyhow!("日期格式错误"))?;
            let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .context(format!("解析日期失败: {}", date_str))?;
            
            let open: f64 = kline_array[1].as_str()
                .ok_or_else(|| anyhow!("开盘价格式错误"))?
                .parse()?;
            let close: f64 = kline_array[2].as_str()
                .ok_or_else(|| anyhow!("收盘价格式错误"))?
                .parse()?;
            let high: f64 = kline_array[3].as_str()
                .ok_or_else(|| anyhow!("最高价格式错误"))?
                .parse()?;
            let low: f64 = kline_array[4].as_str()
                .ok_or_else(|| anyhow!("最低价格式错误"))?
                .parse()?;
            let volume: f64 = kline_array[5].as_str()
                .ok_or_else(|| anyhow!("成交量格式错误"))?
                .parse()?;
            
            // 计算涨跌幅和成交额
            let pct_chg = if result.is_empty() {
                0.0
            } else {
                let prev_close = result.last().unwrap().close;
                ((close - prev_close) / prev_close) * 100.0
            };
            
            let amount = volume * close; // 简单估算成交额
            
            let kline_data = KlineData {
                date,
                open,
                close,
                high,
                low,
                volume,
                amount,
                pct_chg,
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
            
            result.push(kline_data);
        }
        
        // 按日期降序排序（最新的在前）
        result.sort_by(|a, b| b.date.cmp(&a.date));
        
        Ok(result)
    }
    
    /// 获取股票名称（静态异步方法）
    async fn fetch_stock_name_internal(client: &reqwest::Client, code: &str) -> Option<String> {
        // 转换股票代码格式
        let market_code = if code.starts_with('6') {
            format!("sh{}", code)
        } else {
            format!("sz{}", code)
        };
        
        // 使用腾讯实时行情接口获取股票名称
        let url = format!("http://qt.gtimg.cn/q={}", market_code);
        
        match client.get(&url)
            .header("Referer", "http://gu.qq.com/")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(response) => {
                if let Ok(text) = response.text().await {
                    // 解析格式: v_sz002413="51~雷科防务~002413~15.00~..."
                    if let Some(start) = text.find('"') {
                        if let Some(end) = text.rfind('"') {
                            if start < end {
                                let data = &text[start + 1..end];
                                // 分割字段，第2个字段（索引1）是股票名称
                                let parts: Vec<&str> = data.split('~').collect();
                                if parts.len() > 1 {
                                    let name = parts[1].to_string();
                                    if !name.is_empty() {
                                        log::debug!("[腾讯] 获取股票名称: {} -> {}", code, name);
                                        return Some(name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                log::debug!("[腾讯] 获取股票名称失败: {}", e);
            }
        }
        None
    }
    
    /// 获取实时行情（包含盈利指标）
    async fn fetch_realtime_quote_internal(client: &reqwest::Client, code: &str) -> Result<Option<RealtimeQuote>> {
        let market_code = if code.starts_with('6') {
            format!("sh{}", code)
        } else {
            format!("sz{}", code)
        };
        
        let url = format!("http://qt.gtimg.cn/q={}", market_code);
        
        match client.get(&url)
            .header("Referer", "http://gu.qq.com/")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(response) => {
                if let Ok(text) = response.text().await {
                    // 解析格式: v_sz002413="51~雷科防务~002413~15.00~14.80~14.85~335918~167675~..."
                    if let Some(start) = text.find('"') {
                        if let Some(end) = text.rfind('"') {
                            if start < end {
                                let data = &text[start + 1..end];
                                let parts: Vec<&str> = data.split('~').collect();
                                
                                // 腾讯API字段说明（索引，实际验证）：
                                // 0: 未知 1: 名称 2: 代码 3: 当前价 4: 昨收 5: 今开 
                                // 6: 成交量(手) 7: 外盘 8: 内盘 9: 买一 10: 买一量
                                // ...
                                // 33: 涨跌幅% 
                                // 38: 换手率%
                                // 39: 市盈率(PE)
                                // 40: (空)
                                // 41: 最高
                                // 42: 最低
                                // 43: 成交量/成交额/换手率(组合字段)
                                // 44: 流通市值(亿)
                                // 45: 总市值(亿)
                                // 46: 市净率(PB)
                                // 47: 涨停价
                                // 48: 跌停价
                                // 49: 量比
                                // 50-52: 未知
                                // 53: 市盈率(TTM)
                                // ...更多字段待研究
                                
                                if parts.len() >= 47 {
                                    let price = parts[3].parse::<f64>().unwrap_or(0.0);
                                    let prev_close = parts[4].parse::<f64>().unwrap_or(0.0);
                                    let pct_chg = if prev_close > 0.0 {
                                        ((price - prev_close) / prev_close) * 100.0
                                    } else {
                                        0.0
                                    };
                                    
                                    let quote = RealtimeQuote {
                                        code: code.to_string(),
                                        name: parts[1].to_string(),
                                        price,
                                        pct_chg,
                                        pe_ratio: parts[39].parse::<f64>().unwrap_or(0.0),
                                        pb_ratio: parts[46].parse::<f64>().unwrap_or(0.0),
                                        turnover_rate: parts[38].parse::<f64>().unwrap_or(0.0),
                                        market_cap: parts[45].parse::<f64>().unwrap_or(0.0), // 已经是亿为单位
                                        circulating_cap: parts[44].parse::<f64>().unwrap_or(0.0), // 已经是亿为单位
                                        volume: parts[6].parse::<f64>().unwrap_or(0.0) * 100.0, // 手 -> 股
                                        amount: parts[37].parse::<f64>().unwrap_or(0.0) * 10000.0, // 万 -> 元
                                    };
                                    
                                    return Ok(Some(quote));
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("[腾讯] 获取实时行情失败: {}", e);
            }
        }
        Ok(None)
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
        let code = code.to_string();
        
        // 使用 tokio::task::block_in_place 来运行异步代码
        let data = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                Self::fetch_kline_data_internal(&client, &code, days).await
            })
        })?;
        
        log::info!("[腾讯] 成功获取 {} 条数据", data.len());
        
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
    
    fn get_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        let client = self.client.clone();
        let code_str = code.to_string();
        
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                Self::fetch_realtime_quote_internal(&client, &code_str).await
            })
        })
    }
    
    fn name(&self) -> &'static str {
        "腾讯财经"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
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
    
    #[test]
    fn test_different_markets() {
        let provider = GtimgProvider::new().unwrap();
        
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
