//! Tushare Pro 数据提供者
//! 
//! 通过 Tushare Pro API 获取股票数据
//! API文档: https://tushare.pro/document/2

use super::{DataProvider, KlineData, RealtimeQuote};
use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Tushare Pro API 请求参数
#[derive(Debug, Serialize)]
struct TushareRequest {
    api_name: String,
    token: String,
    params: Value,
    fields: String,
}

/// Tushare Pro API 响应
#[derive(Debug, Deserialize)]
struct TushareResponse {
    code: i32,
    msg: Option<String>,
    data: Option<TushareData>,
}

/// Tushare Pro 数据部分
#[derive(Debug, Deserialize)]
struct TushareData {
    fields: Option<Vec<String>>,
    items: Option<Vec<Vec<Value>>>,
}

/// Tushare Pro 数据提供者
pub struct TushareProvider {
    client: reqwest::Client,
    token: String,
}

impl TushareProvider {
    /// 创建新的提供者
    pub fn new() -> Result<Self> {
        // 从环境变量获取 Token
        let token = std::env::var("TUSHARE_TOKEN")
            .context("请设置环境变量 TUSHARE_TOKEN")?;
        
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")
            .build()?;
        
        Ok(Self { client, token })
    }
    
    /// 从 Tushare Pro API 获取日线数据（异步版本）
    async fn fetch_daily_data_internal(
        client: &reqwest::Client,
        token: &str,
        code: &str,
        days: usize,
    ) -> Result<Vec<KlineData>> {
        // 转换股票代码格式 (600519 -> 600519.SH, 000001 -> 000001.SZ)
        let ts_code = Self::convert_to_ts_code(code);
        
        // 计算起始日期（从今天往前推 days 天）
        let end_date = chrono::Local::now().format("%Y%m%d").to_string();
        let start_date = (chrono::Local::now() - chrono::Duration::days(days as i64 * 2))
            .format("%Y%m%d")
            .to_string();
        
        // 构建请求参数
        let params = serde_json::json!({
            "ts_code": ts_code,
            "start_date": start_date,
            "end_date": end_date,
        });
        
        let request = TushareRequest {
            api_name: "daily".to_string(),
            token: token.to_string(),
            params,
            fields: "".to_string(), // 空字符串表示返回所有字段
        };
        
        log::debug!("[Tushare] 请求参数: {:?}", request);
        
        // 发送 POST 请求到 Tushare Pro API
        let response = client
            .post("http://api.tushare.pro")
            .json(&request)
            .send()
            .await;
        
        let response = match response {
            Ok(resp) => resp,
            Err(e) => {
                log::error!("[Tushare] 请求失败: {}", e);
                return Err(anyhow!("HTTP请求失败: {}", e));
            }
        };
        
        if !response.status().is_success() {
            log::error!("[Tushare] 响应状态码: {}", response.status());
            return Err(anyhow!("HTTP请求返回错误状态: {}", response.status()));
        }
        
        let text = response.text().await.context("读取响应失败")?;
        
        if text.is_empty() {
            log::error!("[Tushare] 响应为空");
            return Err(anyhow!("API返回空响应"));
        }
        
        log::debug!("[Tushare] 完整响应: {}", &text);
        
        // 解析响应
        let api_response: TushareResponse = serde_json::from_str(&text)
            .context(format!("解析JSON失败，响应内容: {}", &text[..text.len().min(200)]))?;
        
        log::debug!("[Tushare] API响应码: {}, 消息: {:?}", api_response.code, api_response.msg);
        
        if api_response.code != 0 {
            let msg = api_response.msg.unwrap_or_else(|| "未知错误".to_string());
            log::error!("[Tushare] API错误 (code={}): {}", api_response.code, msg);
            return Err(anyhow!("Tushare API错误: {}", msg));
        }
        
        let data = api_response.data.ok_or_else(|| anyhow!("API返回的data字段为空"))?;
        
        // 解析K线数据
        let klines = Self::parse_daily_data(&data)?;
        
        // Tushare返回的数据是降序的，需要反转
        let mut klines = klines;
        klines.reverse();
        
        // 限制返回天数
        if klines.len() > days {
            let skip_count = klines.len() - days;
            klines = klines.into_iter().skip(skip_count).collect();
        }
        
        Ok(klines)
    }
    
    /// 解析日线数据
    fn parse_daily_data(data: &TushareData) -> Result<Vec<KlineData>> {
        let mut result = Vec::new();
        
        let fields = data.fields.as_ref().ok_or_else(|| anyhow!("API返回的fields字段为空"))?;
        let items = data.items.as_ref().ok_or_else(|| anyhow!("API返回的items字段为空"))?;
        
        log::debug!("[Tushare] 字段列表: {:?}", fields);
        log::debug!("[Tushare] 数据行数: {}", items.len());
        
        // 构建字段索引映射
        let mut field_indices = std::collections::HashMap::new();
        for (i, field) in fields.iter().enumerate() {
            field_indices.insert(field.as_str(), i);
        }
        
        // 检查必需字段
        let required_fields = ["trade_date", "open", "high", "low", "close", "vol", "amount", "pct_chg"];
        for field in &required_fields {
            if !field_indices.contains_key(field) {
                log::error!("[Tushare] 缺少必需字段: {}, 可用字段: {:?}", field, fields);
                return Err(anyhow!("缺少必需字段: {}", field));
            }
        }
        
        // 解析每一行数据
        for (_idx, item) in items.iter().enumerate() {
            let date_str = item[field_indices["trade_date"]]
                .as_str()
                .ok_or_else(|| anyhow!("日期格式错误"))?;
            
            // Tushare日期格式: 20260123
            let date = NaiveDate::parse_from_str(date_str, "%Y%m%d")
                .context(format!("解析日期失败: {}", date_str))?;
            
            let open = Self::parse_f64(&item[field_indices["open"]])?;
            let high = Self::parse_f64(&item[field_indices["high"]])?;
            let low = Self::parse_f64(&item[field_indices["low"]])?;
            let close = Self::parse_f64(&item[field_indices["close"]])?;
            let volume = Self::parse_f64(&item[field_indices["vol"]])? * 100.0; // Tushare单位是手（100股）
            let amount = Self::parse_f64(&item[field_indices["amount"]])? * 1000.0; // Tushare单位是千元
            let pct_chg = Self::parse_f64(&item[field_indices["pct_chg"]])?;
            
            result.push(KlineData {
                date,
                open,
                high,
                low,
                close,
                volume,
                amount,
                pct_chg,
                // 盈利指标暂时设为 None，可以通过其他API获取
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
            });
        }
        
        Ok(result)
    }
    
    /// 获取实时行情（包含盈利指标）
    async fn fetch_realtime_quote_internal(
        client: &reqwest::Client,
        token: &str,
        code: &str,
    ) -> Result<Option<RealtimeQuote>> {
        let ts_code = Self::convert_to_ts_code(code);
        
        // 构建请求参数
        let params = serde_json::json!({
            "ts_code": ts_code,
        });
        
        let request = TushareRequest {
            api_name: "daily_basic".to_string(),
            token: token.to_string(),
            params,
            fields: "ts_code,trade_date,close,turnover_rate,volume_ratio,pe,pb,total_mv,circ_mv".to_string(),
        };
        
        log::debug!("[Tushare] 实时行情请求: {:?}", request);
        
        let response = client
            .post("http://api.tushare.pro")
            .json(&request)
            .send()
            .await;
        
        let response = match response {
            Ok(resp) => resp,
            Err(e) => {
                log::error!("[Tushare] 实时行情请求失败: {}", e);
                return Ok(None);
            }
        };
        
        if !response.status().is_success() {
            log::error!("[Tushare] 实时行情响应状态码: {}", response.status());
            return Ok(None);
        }
        
        let text = response.text().await.context("读取响应失败")?;
        
        log::debug!("[Tushare] 实时行情响应: {}", &text[..text.len().min(500)]);
        
        let api_response: TushareResponse = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                log::error!("[Tushare] 实时行情JSON解析失败: {}", e);
                return Ok(None);
            }
        };
        
        if api_response.code != 0 {
            log::error!("[Tushare] 实时行情API错误: {:?}", api_response.msg);
            return Ok(None);
        }
        
        let data = match api_response.data {
            Some(d) => d,
            None => return Ok(None),
        };
        
        let items = match data.items {
            Some(ref items) if !items.is_empty() => items,
            _ => return Ok(None),
        };
        
        // 解析实时行情
        let fields = data.fields.as_ref().ok_or_else(|| anyhow!("fields字段为空"))?;
        let mut field_indices = std::collections::HashMap::new();
        for (i, field) in fields.iter().enumerate() {
            field_indices.insert(field.as_str(), i);
        }
        
        let item = &items[0];
        
        let quote = RealtimeQuote {
            code: code.to_string(),
            name: String::new(), // Tushare需要额外调用API获取股票名称
            price: Self::parse_f64(&item[field_indices["close"]]).unwrap_or(0.0),
            pct_chg: 0.0, // 需要从其他接口获取
            pe_ratio: Self::parse_f64(&item[field_indices["pe"]]).unwrap_or(0.0),
            pb_ratio: Self::parse_f64(&item[field_indices["pb"]]).unwrap_or(0.0),
            turnover_rate: Self::parse_f64(&item[field_indices["turnover_rate"]]).unwrap_or(0.0),
            market_cap: Self::parse_f64(&item[field_indices["total_mv"]]).unwrap_or(0.0) / 10000.0, // 万元转亿元
            circulating_cap: Self::parse_f64(&item[field_indices["circ_mv"]]).unwrap_or(0.0) / 10000.0,
            volume: 0.0,
            amount: 0.0,
        };
        
        Ok(Some(quote))
    }
    
    /// 获取股票名称
    async fn fetch_stock_name_internal(
        client: &reqwest::Client,
        token: &str,
        code: &str,
    ) -> Result<Option<String>> {
        let ts_code = Self::convert_to_ts_code(code);
        
        let params = serde_json::json!({
            "ts_code": ts_code,
        });
        
        let request = TushareRequest {
            api_name: "stock_basic".to_string(),
            token: token.to_string(),
            params,
            fields: "ts_code,name".to_string(),
        };
        
        let response = client
            .post("http://api.tushare.pro")
            .json(&request)
            .send()
            .await;
        
        let response = match response {
            Ok(resp) => resp,
            Err(e) => {
                log::warn!("[Tushare] 获取股票名称失败: {}", e);
                return Ok(None);
            }
        };
        
        if !response.status().is_success() {
            return Ok(None);
        }
        
        let text = match response.text().await {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        
        let api_response: TushareResponse = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        
        if api_response.code != 0 {
            return Ok(None);
        }
        
        let data = match api_response.data {
            Some(d) => d,
            None => return Ok(None),
        };
        
        let fields = match data.fields {
            Some(ref f) => f,
            None => return Ok(None),
        };
        
        let items = match data.items {
            Some(ref i) if !i.is_empty() => i,
            _ => return Ok(None),
        };
        
        let name_index = match fields.iter().position(|f| f == "name") {
            Some(idx) => idx,
            None => return Ok(None),
        };
        
        let name = match items[0][name_index].as_str() {
            Some(n) => n.to_string(),
            None => return Ok(None),
        };
        
        Ok(Some(name))
    }
    
    /// 转换股票代码为 Tushare 格式
    /// 600519 -> 600519.SH
    /// 000001 -> 000001.SZ
    /// 300001 -> 300001.SZ
    fn convert_to_ts_code(code: &str) -> String {
        if code.starts_with('6') {
            format!("{}.SH", code) // 上海
        } else if code.starts_with("688") {
            format!("{}.SH", code) // 科创板
        } else {
            format!("{}.SZ", code) // 深圳/创业板
        }
    }
    
    /// 解析 f64 值
    fn parse_f64(value: &Value) -> Result<f64> {
        match value {
            Value::Number(n) => n.as_f64().ok_or_else(|| anyhow!("无法转换为f64")),
            Value::String(s) => s.parse::<f64>().context("解析数字字符串失败"),
            Value::Null => Ok(0.0),
            _ => Err(anyhow!("不支持的数字类型")),
        }
    }
}

impl DataProvider for TushareProvider {
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        log::info!("[Tushare] 获取 {} 最近 {} 天的K线数据", code, days);
        
        // 使用 tokio::runtime::Handle 在同步上下文中运行异步代码
        let handle = tokio::runtime::Handle::current();
        let client = self.client.clone();
        let token = self.token.clone();
        let code = code.to_string();
        
        // 使用 block_in_place 避免阻塞异步运行时
        tokio::task::block_in_place(move || {
            handle.block_on(async move {
                Self::fetch_daily_data_internal(&client, &token, &code, days).await
            })
        })
    }
    
    fn get_stock_name(&self, code: &str) -> Option<String> {
        let handle = tokio::runtime::Handle::current();
        let client = self.client.clone();
        let token = self.token.clone();
        let code = code.to_string();
        
        tokio::task::block_in_place(move || {
            handle.block_on(async move {
                Self::fetch_stock_name_internal(&client, &token, &code)
                    .await
                    .ok()
                    .flatten()
            })
        })
    }
    
    fn get_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        log::info!("[Tushare] 获取 {} 的实时行情", code);
        
        let handle = tokio::runtime::Handle::current();
        let client = self.client.clone();
        let token = self.token.clone();
        let code = code.to_string();
        
        tokio::task::block_in_place(move || {
            handle.block_on(async move {
                Self::fetch_realtime_quote_internal(&client, &token, &code).await
            })
        })
    }
    
    fn name(&self) -> &'static str {
        "Tushare Pro"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_to_ts_code() {
        assert_eq!(TushareProvider::convert_to_ts_code("600519"), "600519.SH");
        assert_eq!(TushareProvider::convert_to_ts_code("000001"), "000001.SZ");
        assert_eq!(TushareProvider::convert_to_ts_code("300001"), "300001.SZ");
        assert_eq!(TushareProvider::convert_to_ts_code("688001"), "688001.SH");
    }
}
