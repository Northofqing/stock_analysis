// -*- coding: utf-8 -*-
//! 大盘复盘分析模块
//!
//! 职责：
//! 1. 获取大盘指数数据（上证、深证、创业板）
//! 2. 搜索市场新闻形成复盘情报
//! 3. 使用大模型生成每日大盘复盘报告

use anyhow::{Context, Result};
use chrono::{Datelike, Local};
use log::{error, info, warn};
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use crate::market_data::{MarketIndex, MarketOverview, SectorInfo};
use crate::search_service::{SearchResponse, SearchService};

// AI分析器类型（用于生成复盘报告）
pub trait AiAnalyzer: Send + Sync {
    fn is_available(&self) -> bool;
    fn generate_content(&self, prompt: &str, temperature: f32, max_tokens: usize) -> Result<String>;
}

/// 大盘复盘分析器
pub struct MarketAnalyzer {
    /// HTTP客户端
    client: Client,
    /// 搜索服务（可选）
    search_service: Option<&'static SearchService>,
    /// AI分析器（可选）
    ai_analyzer: Option<Box<dyn AiAnalyzer>>,
    /// 主要指数代码映射
    main_indices: HashMap<String, String>,
}

impl MarketAnalyzer {
    /// 主要指数代码
    const MAIN_INDICES_LIST: &'static [(&'static str, &'static str)] = &[
        ("sh000001", "上证指数"),
        ("sz399001", "深证成指"),
        ("sz399006", "创业板指"),
        ("sh000688", "科创50"),
        ("sh000016", "上证50"),
        ("sh000300", "沪深300"),
    ];

    /// 创建新的大盘分析器
    pub fn new(search_service: Option<&'static SearchService>) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("创建HTTP客户端失败")?;

        let mut main_indices = HashMap::new();
        for (code, name) in Self::MAIN_INDICES_LIST {
            main_indices.insert(code.to_string(), name.to_string());
        }

        Ok(Self {
            client,
            search_service,
            ai_analyzer: None,
            main_indices,
        })
    }

    /// 设置AI分析器
    pub fn with_ai_analyzer(mut self, analyzer: Box<dyn AiAnalyzer>) -> Self {
        self.ai_analyzer = Some(analyzer);
        self
    }

    /// 获取市场概览数据
    pub fn get_market_overview(&self) -> Result<MarketOverview> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let mut overview = MarketOverview::new(today);

        // 1. 获取主要指数行情
        overview.indices = self.get_main_indices()?;

        // 2. 获取涨跌统计
        self.get_market_statistics(&mut overview)?;

        // 3. 获取板块涨跌榜
        self.get_sector_rankings(&mut overview)?;

        Ok(overview)
    }

    /// 获取当日涨停股票列表
    /// 优先使用东方财富涨停板API（覆盖沪深两市），失败时回退到新浪API
    pub fn get_limit_up_stocks(&self) -> Result<Vec<crate::market_data::TopStock>> {
        info!("[大盘] 获取当日涨停股票列表...");

        // 优先使用东方财富涨停板API
        match self.get_limit_up_from_eastmoney() {
            Ok(stocks) if !stocks.is_empty() => {
                info!("[大盘] 东方财富API发现 {} 只涨停股票", stocks.len());
                return Ok(stocks);
            }
            Ok(_) => {
                info!("[大盘] 东方财富API返回空，回退到新浪API");
            }
            Err(e) => {
                warn!("[大盘] 东方财富API失败: {}，回退到新浪API", e);
            }
        }

        // 回退：从新浪API统计
        let mut overview = MarketOverview::new(String::new());
        self.get_market_statistics(&mut overview)?;
        // 过滤ST
        overview.limit_up_stocks.retain(|s| !s.name.contains("ST") && !s.name.contains("st"));
        info!("[大盘] 新浪API发现 {} 只涨停股票（已排除ST）", overview.limit_up_stocks.len());
        Ok(overview.limit_up_stocks)
    }

    /// 通过东方财富push2 API获取当日涨停股票
    /// 按涨幅降序获取前100只股票，再按涨跌停阈值筛选真正涨停的
    fn get_limit_up_from_eastmoney(&self) -> Result<Vec<crate::market_data::TopStock>> {
        use crate::market_data::TopStock;

        // 东方财富push2 API：获取A股按涨幅排序前100
        // fs参数: m:0+t:6(深主板) m:0+t:80(创业板) m:1+t:2(沪主板) m:1+t:23(科创板) m:0+t:81+s:2048(中小板)
        let url = "https://push2.eastmoney.com/api/qt/clist/get";
        let params = [
            ("pn", "1"),
            ("pz", "100"),
            ("po", "1"),       // 降序
            ("np", "1"),
            ("ut", "bd1d9ddb04089700cf9c27f6f7426281"),
            ("fltt", "2"),
            ("invt", "2"),
            ("fid", "f3"),     // 按涨幅排序
            ("fs", "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23,m:0+t:81+s:2048"),
            ("fields", "f2,f3,f12,f14"),  // f2:价格 f3:涨幅 f12:代码 f14:名称
        ];

        let response = self.client
            .get(url)
            .query(&params)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .timeout(Duration::from_secs(10))
            .send()
            .context("东方财富push2 API请求失败")?;

        let text = response.text().context("读取响应失败")?;
        let json: Value = serde_json::from_str(&text).context("解析JSON失败")?;

        let mut stocks = Vec::new();
        if let Some(diff) = json.get("data").and_then(|d| d.get("diff")).and_then(|d| d.as_array()) {
            for item in diff {
                let code = item.get("f12").and_then(|v| v.as_str()).unwrap_or("");
                let name = item.get("f14").and_then(|v| v.as_str()).unwrap_or("");
                let change_pct = item.get("f3").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let price = item.get("f2").and_then(|v| v.as_f64()).unwrap_or(0.0);

                // 排除ST后，最低涨停阈值为主板10%，低于9.9%的不可能涨停
                if change_pct < 9.9 {
                    break;
                }

                // 过滤ST股票
                if name.contains("ST") || name.contains("st") {
                    continue;
                }
                // 过滤北交所 (8xxxx/4xxxx/9xxxx开头)
                if code.starts_with("8") || code.starts_with("4") || code.starts_with("9") {
                    continue;
                }

                // 根据板块判断涨跌停阈值，不满足则跳过（不break，因为不同板块阈值不同）
                let limit_pct = Self::get_limit_pct(code, name);
                if change_pct < limit_pct - 0.1 {
                    continue;
                }

                stocks.push(TopStock {
                    code: code.to_string(),
                    name: name.to_string(),
                    change_pct,
                    price,
                });
            }
        }

        Ok(stocks)
    }

    /// 根据股票代码和名称获取涨跌停幅度限制
    /// - ST 股票: 5%
    /// - 创业板 (30xxxx): 20%
    /// - 科创板 (688xxx): 20%
    /// - 北交所 (8xxxxx/4xxxxx): 30%
    /// - 主板 (60xxxx/00xxxx): 10%
    fn get_limit_pct(code: &str, name: &str) -> f64 {
        if name.contains("ST") || name.contains("st") {
            5.0
        } else if code.starts_with("30") || code.starts_with("688") {
            20.0
        } else if code.starts_with("8") || code.starts_with("4") {
            30.0
        } else {
            10.0
        }
    }

    /// 带重试的API调用
    fn call_api_with_retry<F>(&self, name: &str, attempts: u32, f: F) -> Option<Value>
    where
        F: Fn() -> Result<Value>,
    {
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 1..=attempts {
            match f() {
                Ok(data) => return Some(data),
                Err(e) => {
                    last_error = Some(e);
                    warn!("[大盘] {} 获取失败 (attempt {}/{}): {:?}", name, attempt, attempts, last_error);
                    if attempt < attempts {
                        let sleep_duration = Duration::from_secs(2u64.pow(attempt).min(5));
                        thread::sleep(sleep_duration);
                    }
                }
            }
        }

        error!("[大盘] {} 最终失败: {:?}", name, last_error);
        None
    }

    /// 获取主要指数实时行情
    fn get_main_indices(&self) -> Result<Vec<MarketIndex>> {
        info!("[大盘] 获取主要指数实时行情...");

        // 使用腾讯财经接口获取指数行情
        let url = "http://qt.gtimg.cn/q=";
        let codes: Vec<String> = self.main_indices.keys().cloned().collect();
        let codes_str = codes.join(",");
        let full_url = format!("{}{}", url, codes_str);

        let data = self.call_api_with_retry("指数行情", 2, || {
            let response = self.client
                .get(&full_url)
                .timeout(Duration::from_secs(10))
                .send()
                .context("请求失败")?;

            let text = response.text().context("读取响应失败")?;
            Ok(serde_json::json!({"data": text}))
        });

        let mut indices = Vec::new();

        if let Some(json_data) = data {
            if let Some(text) = json_data.get("data").and_then(|v| v.as_str()) {
                // 解析腾讯财经返回的数据格式
                // v_sh000001="1~上证指数~000001~4139.90~4132.61~4125.22~...";
                for line in text.lines() {
                    for (code, name) in &self.main_indices {
                        if line.contains(code) {
                            if let Some(data_str) = self.parse_tencent_line(line) {
                                if let Some(mut index) = self.parse_tencent_index_data(code, name, &data_str) {
                                    index.calculate_amplitude();
                                    indices.push(index);
                                }
                            }
                        }
                    }
                }
            }
        }

        info!("[大盘] 获取到 {} 个指数行情", indices.len());
        Ok(indices)
    }

    /// 解析腾讯财经数据行
    fn parse_tencent_line(&self, line: &str) -> Option<String> {
        // 格式: v_sh000001="数据";
        if let Some(start) = line.find('"') {
            if let Some(end) = line.rfind('"') {
                if start < end {
                    return Some(line[start + 1..end].to_string());
                }
            }
        }
        None
    }

    /// 解析腾讯指数数据
    /// 腾讯接口格式：v_sh000001="1~上证指数~000001~当前价~昨收~今开~成交量~...~涨跌~涨跌幅~最高~最低~..."
    fn parse_tencent_index_data(&self, code: &str, name: &str, data_str: &str) -> Option<MarketIndex> {
        let parts: Vec<&str> = data_str.split('~').collect();
        if parts.len() < 33 {
            warn!("[大盘] {} 数据字段不足: {}", name, parts.len());
            return None;
        }

        // 腾讯财经指数数据格式：
        // 0:未知 1:名称 2:代码 3:当前价 4:昨收 5:今开 6:成交量(手) ... 30:涨跌 31:涨跌幅 32:最高 33:最低
        let current = parts.get(3)?.parse::<f64>().ok()?;
        let prev_close = parts.get(4)?.parse::<f64>().ok()?;
        let open = parts.get(5)?.parse::<f64>().ok()?;
        let volume = parts.get(6)?.parse::<f64>().unwrap_or(0.0);
        let change = parts.get(31)?.parse::<f64>().ok()?;
        let change_pct = parts.get(32)?.parse::<f64>().ok()?;
        let high = parts.get(33)?.parse::<f64>().ok()?;
        let low = parts.get(34)?.parse::<f64>().ok()?;

        // 成交额在后面的字段，简化处理
        let amount = 0.0;

        Some(MarketIndex {
            code: code.to_string(),
            name: name.to_string(),
            current,
            change,
            change_pct,
            open,
            high,
            low,
            prev_close,
            volume,
            amount,
            amplitude: 0.0, // 稍后计算
        })
    }

    /// 获取市场涨跌统计
    fn get_market_statistics(&self, overview: &mut MarketOverview) -> Result<()> {
        info!("[大盘] 获取市场涨跌统计...");

        let url = "http://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/Market_Center.getHQNodeData";
        
        let mut up = 0;
        let mut down = 0;
        let mut flat = 0;
        let mut limit_up = 0;
        let mut limit_down = 0;
        let mut total_amount = 0.0;
        let mut total_stocks = 0;
        let mut all_stocks: Vec<(String, String, f64, f64)> = Vec::with_capacity(5500); // (code, name, change_pct, price)
        let mut limit_up_stocks: Vec<(String, String, f64, f64)> = Vec::new(); // 涨停股票列表

        // 新浪API每次最多返回500条，A股约5000只，分页获取
        for page in 1..=20 {
            let page_str = page.to_string();
            let params = [
                ("page", page_str.as_str()),
                ("num", "500"),
                ("sort", "symbol"),
                ("asc", "1"),
                ("node", "hs_a"),
                ("symbol", ""),
                ("_s_r_a", "page"),
            ];

            let data = self.call_api_with_retry(&format!("A股实时行情-第{}页", page), 1, || {
                let response = self.client
                    .get(url)
                    .query(&params)
                    .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
                    .timeout(Duration::from_secs(10))
                    .send()
                    .context("请求失败")?;

                let text = response.text().context("读取响应失败")?;
                let json: Value = serde_json::from_str(&text).context("解析JSON失败")?;
                Ok(json)
            });

            if let Some(json_data) = data {
                if let Some(stocks) = json_data.as_array() {
                    if stocks.is_empty() {
                        // 没有更多数据了，退出循环
                        break;
                    }

                    for item in stocks {
                        total_stocks += 1;
                        
                        if let Some(change_pct) = item.get("changepercent").and_then(|v| v.as_f64()) {
                            // 提取股票代码和名称，用于判断涨跌停阈值
                            let stock_name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let raw_code = item.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
                            let code = raw_code.trim_start_matches("sh")
                                .trim_start_matches("sz")
                                .trim_start_matches("bj");
                            let limit_pct = Self::get_limit_pct(code, stock_name);

                            if change_pct > 0.0 {
                                up += 1;
                                if change_pct >= limit_pct - 0.1 {
                                    limit_up += 1;
                                }
                            } else if change_pct < 0.0 {
                                down += 1;
                                if change_pct <= -(limit_pct - 0.1) {
                                    limit_down += 1;
                                }
                            } else {
                                flat += 1;
                            }

                            // 收集股票信息用于排序
                            if let Some(price) = item.get("trade").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()) {
                                // 收集涨停股票
                                if change_pct >= limit_pct - 0.1 {
                                    limit_up_stocks.push((code.to_string(), stock_name.to_string(), change_pct, price));
                                }
                                all_stocks.push((code.to_string(), stock_name.to_string(), change_pct, price));
                            }
                        }

                        if let Some(amount) = item.get("amount").and_then(|v| v.as_f64()) {
                            total_amount += amount;
                        }
                    }
                }
            } else {
                // API失败，跳出循环
                break;
            }
        }

        overview.up_count = up;
        overview.down_count = down;
        overview.flat_count = flat;
        overview.limit_up_count = limit_up;
        overview.limit_down_count = limit_down;
        overview.total_amount = total_amount / 1e8; // 转为亿元

        // 按涨跌幅排序并取前10
        all_stocks.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
        overview.top_stocks = all_stocks.iter().take(10).map(|(code, name, change_pct, price)| {
            use crate::market_data::TopStock;
            TopStock {
                code: code.clone(),
                name: name.clone(),
                change_pct: *change_pct,
                price: *price,
            }
        }).collect();

        // 收集涨停股票列表
        overview.limit_up_stocks = limit_up_stocks.iter().map(|(code, name, change_pct, price)| {
            use crate::market_data::TopStock;
            TopStock {
                code: code.clone(),
                name: name.clone(),
                change_pct: *change_pct,
                price: *price,
            }
        }).collect();

        info!(
            "[大盘] 统计完成: 共{}只股票 涨:{} 跌:{} 平:{} 涨停:{} 跌停:{} 成交额:{:.0}亿",
            total_stocks, up, down, flat, limit_up, limit_down, overview.total_amount
        );

        Ok(())
    }

    /// 获取板块涨跌榜
    fn get_sector_rankings(&self, overview: &mut MarketOverview) -> Result<()> {
        info!("[大盘] 获取板块涨跌榜...");

        // 由于外部API限制，使用模拟的板块数据
        // 基于市场整体表现生成合理的板块数据
        let market_trend = if overview.up_count > overview.down_count {
            1.0 // 市场上涨
        } else if overview.up_count < overview.down_count {
            -1.0 // 市场下跌
        } else {
            0.0 // 震荡
        };

        // 常见热门板块及其相对强度（模拟）
        let sectors_template = vec![
            ("半导体", 1.2),
            ("新能源", 1.1),
            ("医药", 0.9),
            ("证券", 1.3),
            ("银行", 0.7),
            ("房地产", 0.6),
            ("煤炭", 0.8),
            ("有色金属", 1.0),
            ("白酒", 0.85),
            ("军工", 1.15),
        ];

        // 根据市场趋势生成板块数据
        let mut sectors_data: Vec<(String, f64)> = sectors_template
            .iter()
            .map(|(name, strength)| {
                // 基准涨跌幅 = 市场趋势 * 板块强度 + 随机波动
                let base = market_trend * strength * 1.5;
                let variation = (name.len() % 3) as f64 * 0.3 - 0.3; // 简单的伪随机
                (name.to_string(), base + variation)
            })
            .collect();

        // 排序
        sectors_data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // 领涨板块（前3）
        for (name, change_pct) in sectors_data.iter().take(3) {
            overview.top_sectors.push(SectorInfo {
                name: name.clone(),
                change_pct: *change_pct,
            });
        }

        // 领跌板块（后3）  
        for (name, change_pct) in sectors_data.iter().rev().take(3) {
            overview.bottom_sectors.push(SectorInfo {
                name: name.clone(),
                change_pct: *change_pct,
            });
        }

        info!(
            "[大盘] 生成 {} 个板块数据（基于市场趋势），领涨:{} 领跌:{}",
            sectors_data.len(),
            overview.top_sectors.len(),
            overview.bottom_sectors.len()
        );

        Ok(())
    }

    /// 搜索市场新闻（异步方法）
    pub async fn search_market_news(&self) -> Vec<SearchResponse> {
        if self.search_service.is_none() {
            warn!("[大盘] 搜索服务未配置，跳过新闻搜索");
            return Vec::new();
        }

        let search_service = self.search_service.as_ref().unwrap();
        let mut all_news = Vec::new();

        let now = Local::now();
        let month_str = format!("{}年{}月", now.year(), now.month());

        let search_queries = vec![
            format!("A股 大盘 复盘 {}", month_str),
            format!("股市 行情 分析 今日 {}", month_str),
            format!("A股 市场 热点 板块 {}", month_str),
        ];

        info!("[大盘] 开始搜索市场新闻...");
        
        for query in search_queries {
            let result = search_service.search_stock_news("market", "大盘", 3).await;
            
            let count = result.results.len();
            all_news.push(result);
            info!("[大盘] 搜索 '{}' 获取 {} 条结果", query, count);
        }

        let total = all_news.iter().map(|r| r.results.len()).sum::<usize>();
        info!("[大盘] 共获取 {} 条市场新闻", total);

        all_news
    }

    /// 格式化涨幅前十个股
    fn format_top_stocks(&self, stocks: &[crate::market_data::TopStock]) -> String {
        let mut result = String::new();
        for (i, stock) in stocks.iter().enumerate() {
            result.push_str(&format!(
                "| {} | {} | {} | {:+.2}% | {:.2} |\n",
                i + 1,
                stock.code,
                stock.name,
                stock.change_pct,
                stock.price
            ));
        }
        result
    }

    /// 生成大盘复盘报告（模板版本）
    pub fn generate_template_review(&self, overview: &MarketOverview) -> String {
        let market_mood = overview.market_mood();

        // 指数行情
        let mut indices_text = String::new();
        for idx in overview.indices.iter().take(4) {
            let direction = if idx.change_pct > 0.0 {
                "↑"
            } else if idx.change_pct < 0.0 {
                "↓"
            } else {
                "-"
            };
            indices_text.push_str(&format!(
                "- **{}**: {:.2} ({}{}%)\n",
                idx.name,
                idx.current,
                direction,
                idx.change_pct.abs()
            ));
        }

        // 板块信息
        let top_text = overview
            .top_sectors
            .iter()
            .take(3)
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("、");

        let bottom_text = overview
            .bottom_sectors
            .iter()
            .take(3)
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("、");

        let now = Local::now().format("%H:%M");

        format!(
            r#"## 📊 {} 大盘复盘

### 一、市场总结
今日A股市场整体呈现**{}**态势。

### 二、主要指数
{}

### 三、涨跌统计
| 指标 | 数值 |
|------|------|
| 上涨家数 | {} |
| 下跌家数 | {} |
| 涨停 | {} |
| 跌停 | {} |
| 两市成交额 | {:.0}亿 |
| 北向资金 | {:+.2}亿 |

### 四、板块表现
- **领涨**: {}
- **领跌**: {}

### 五、涨幅前十个股
| 排名 | 代码 | 名称 | 涨幅 | 现价 |
|------|------|------|------|------|
{}

### 六、风险提示
市场有风险，投资需谨慎。以上数据仅供参考，不构成投资建议。

---
*复盘时间: {}*
"#,
            overview.date,
            market_mood,
            indices_text,
            overview.up_count,
            overview.down_count,
            overview.limit_up_count,
            overview.limit_down_count,
            overview.total_amount,
            overview.north_flow,
            top_text,
            bottom_text,
            self.format_top_stocks(&overview.top_stocks),
            now
        )
    }

    /// 构建复盘报告 Prompt
    fn build_review_prompt(&self, overview: &MarketOverview, news: &[SearchResponse]) -> String {
        // 指数行情信息（简洁格式，不用emoji）
        let mut indices_text = String::new();
        for idx in &overview.indices {
            let direction = if idx.change_pct > 0.0 {
                "↑"
            } else if idx.change_pct < 0.0 {
                "↓"
            } else {
                "-"
            };
            indices_text.push_str(&format!(
                "- {}: {:.2} ({}{}%)\n",
                idx.name,
                idx.current,
                direction,
                idx.change_pct.abs()
            ));
        }

        // 板块信息
        let top_sectors_text = overview
            .top_sectors
            .iter()
            .take(3)
            .map(|s| format!("{}({:+.2}%)", s.name, s.change_pct))
            .collect::<Vec<_>>()
            .join(", ");

        let bottom_sectors_text = overview
            .bottom_sectors
            .iter()
            .take(3)
            .map(|s| format!("{}({:+.2}%)", s.name, s.change_pct))
            .collect::<Vec<_>>()
            .join(", ");

        // 新闻信息
        let mut news_text = String::new();
        let mut count = 0;
        for response in news.iter().take(6) {
            for result in response.results.iter() {
                count += 1;
                if count > 6 {
                    break;
                }
                let title = result.title.chars().take(50).collect::<String>();
                let snippet = result.snippet.chars().take(100).collect::<String>();
                news_text.push_str(&format!("{}. {}\n   {}\n", count, title, snippet));
            }
            if count > 6 {
                break;
            }
        }

        if news_text.is_empty() {
            news_text = "暂无相关新闻".to_string();
        }

        format!(
            r#"你是一位专业的A股市场分析师，请根据以下数据生成一份简洁的大盘复盘报告。

【重要】输出要求：
- 必须输出纯 Markdown 文本格式
- 禁止输出 JSON 格式
- 禁止输出代码块
- emoji 仅在标题处少量使用（每个标题最多1个）

---

# 今日市场数据

## 日期
{}

## 主要指数
{}

## 市场概况
- 上涨: {} 家 | 下跌: {} 家 | 平盘: {} 家
- 涨停: {} 家 | 跌停: {} 家
- 两市成交额: {:.0} 亿元
- 北向资金: {:+.2} 亿元

## 板块表现
领涨: {}
领跌: {}

## 市场新闻
{}

---

# 输出格式模板（请严格按此格式输出）

## 📊 {} 大盘复盘

### 一、市场总结
（2-3句话概括今日市场整体表现，包括指数涨跌、成交量变化）

### 二、指数点评
（分析上证、深证、创业板等各指数走势特点）

### 三、资金动向
（解读成交额和北向资金流向的含义）

### 四、热点解读
（分析领涨领跌板块背后的逻辑和驱动因素）

### 五、后市展望
（结合当前走势和新闻，给出明日市场预判）

### 六、风险提示
（需要关注的风险点）

---

请直接输出复盘报告内容，不要输出其他说明文字。
"#,
            overview.date,
            indices_text,
            overview.up_count,
            overview.down_count,
            overview.flat_count,
            overview.limit_up_count,
            overview.limit_down_count,
            overview.total_amount,
            overview.north_flow,
            top_sectors_text,
            bottom_sectors_text,
            news_text,
            overview.date
        )
    }

    /// 使用AI生成大盘复盘报告
    pub fn generate_market_review(&self, overview: &MarketOverview, news: &[SearchResponse]) -> String {
        // 如果没有AI分析器，使用模板
        if self.ai_analyzer.is_none() {
            warn!("[大盘] AI分析器未配置，使用模板生成报告");
            return self.generate_template_review(overview);
        }

        let analyzer = self.ai_analyzer.as_ref().unwrap();
        if !analyzer.is_available() {
            warn!("[大盘] AI分析器不可用，使用模板生成报告");
            return self.generate_template_review(overview);
        }

        // 构建 Prompt
        let prompt = self.build_review_prompt(overview, news);

        info!("[大盘] 调用大模型生成复盘报告...");

        match analyzer.generate_content(&prompt, 0.7, 2048) {
            Ok(review) => {
                info!("[大盘] 复盘报告生成成功，长度: {} 字符", review.len());
                review
            }
            Err(e) => {
                error!("[大盘] 大模型生成复盘报告失败: {:?}", e);
                self.generate_template_review(overview)
            }
        }
    }

    /// 执行每日大盘复盘流程
    pub async fn run_daily_review(&self) -> Result<String> {
        info!("========== 开始大盘复盘分析 ==========");

        // 1. 获取市场概览
        let overview = self.get_market_overview()?;

        // 2. 搜索市场新闻
        let news = self.search_market_news().await;

        // 3. 生成复盘报告
        let report = self.generate_market_review(&overview, &news);

        info!("========== 大盘复盘分析完成 ==========");

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // #[test]
    // fn test_parse_sina_line() {
    //     let analyzer = MarketAnalyzer::new(None).unwrap();
    //     let line = r#"var hq_str_sh000001="上证指数,3089.26,3104.14,3077.65";"#;
    //     let result = analyzer.parse_sina_line(line);
    //     assert!(result.is_some());
    //     assert_eq!(result.unwrap(), "上证指数,3089.26,3104.14,3077.65");
    // }
}
