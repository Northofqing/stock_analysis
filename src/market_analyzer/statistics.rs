//! statistics（从 market_analyzer.rs 拆分）

use anyhow::{Context, Result};
use log::info;
use serde_json::Value;
use std::time::Duration;

use crate::market_data::{MarketOverview, SectorInfo};

use super::MarketAnalyzer;

impl MarketAnalyzer {
    /// 获取市场涨跌统计
    pub(super) fn get_market_statistics(&self, overview: &mut MarketOverview) -> Result<()> {
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
        let mut seen_codes = std::collections::HashSet::new();
        let mut saw_terminal_page = false;

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

            let data =
                self.call_api_with_retry(&format!("A股实时行情-第{}页", page), 1, || {
                    let response = self
                        .client
                        .get(url)
                        .query(&params)
                        .header(
                            "User-Agent",
                            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
                        )
                        .timeout(Duration::from_secs(10))
                        .send()
                        .context("请求失败")?
                        .error_for_status()
                        .context("HTTP 状态失败")?;

                    let text = response.text().context("读取响应失败")?;
                    let json: Value = serde_json::from_str(&text).context("解析JSON失败")?;
                    Ok(json)
                });

            let json_data = data
                .ok_or_else(|| anyhow::anyhow!("A股实时行情第 {page} 页请求/解析失败，整批拒绝"))?;
            let stocks = json_data
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("A股实时行情第 {page} 页不是数组"))?;
            if stocks.is_empty() {
                saw_terminal_page = true;
                break;
            }

            for (row_index, item) in stocks.iter().enumerate() {
                let raw_code = item.get("symbol").and_then(Value::as_str).ok_or_else(|| {
                    anyhow::anyhow!("第 {page} 页第 {} 行缺 symbol", row_index + 1)
                })?;
                let code = raw_code
                    .strip_prefix("sh")
                    .or_else(|| raw_code.strip_prefix("sz"))
                    .or_else(|| raw_code.strip_prefix("bj"))
                    .unwrap_or(raw_code);
                let stock_name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| anyhow::anyhow!("行情 {code} 缺 name"))?;
                if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
                    anyhow::bail!("行情 code 非法: {code:?}");
                }
                if !seen_codes.insert(code.to_string()) {
                    anyhow::bail!("行情 code 重复: {code}");
                }
                let change_pct = item
                    .get("changepercent")
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && value.abs() <= 30.0)
                    .ok_or_else(|| anyhow::anyhow!("行情 {code} changepercent 缺失/非法"))?;
                let price = item
                    .get("trade")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or_else(|| anyhow::anyhow!("行情 {code} trade 缺失/非法"))?;
                let amount = item
                    .get("amount")
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .ok_or_else(|| anyhow::anyhow!("行情 {code} amount 缺失/非法"))?;

                total_stocks += 1;
                total_amount += amount;
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
                if change_pct >= limit_pct - 0.1 {
                    limit_up_stocks.push((
                        code.to_string(),
                        stock_name.to_string(),
                        change_pct,
                        price,
                    ));
                }
                all_stocks.push((code.to_string(), stock_name.to_string(), change_pct, price));
            }
        }

        if !saw_terminal_page
            || total_stocks == 0
            || !total_amount.is_finite()
            || total_amount <= 0.0
        {
            anyhow::bail!(
                "A股行情批次不完整: terminal_page={saw_terminal_page} rows={total_stocks} amount={total_amount}"
            );
        }

        overview.up_count = up;
        overview.down_count = down;
        overview.flat_count = flat;
        overview.limit_up_count = limit_up;
        overview.limit_down_count = limit_down;
        overview.total_amount = total_amount / 1e8; // 转为亿元

        // 按涨跌幅排序并取前10
        all_stocks.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        overview.top_stocks = all_stocks
            .iter()
            .take(10)
            .map(|(code, name, change_pct, price)| {
                use crate::market_data::TopStock;
                TopStock {
                    code: code.clone(),
                    name: name.clone(),
                    change_pct: *change_pct,
                    price: *price,
                    volume_ratio: None,
                    main_net_yi: None,
                }
            })
            .collect();

        overview.limit_up_stocks = limit_up_stocks
            .iter()
            .map(|(code, name, change_pct, price)| {
                use crate::market_data::TopStock;
                TopStock {
                    code: code.clone(),
                    name: name.clone(),
                    change_pct: *change_pct,
                    price: *price,
                    volume_ratio: None,
                    main_net_yi: None,
                }
            })
            .collect();

        info!(
            "[大盘] 统计完成: 共{}只股票 涨:{} 跌:{} 平:{} 涨停:{} 跌停:{} 成交额:{:.0}亿",
            total_stocks, up, down, flat, limit_up, limit_down, overview.total_amount
        );

        Ok(())
    }

    /// 获取板块涨跌榜
    ///
    /// 修复：QUANT_ANALYST_REVIEW §1.3
    /// 原 bug：用 `name.len() MOD 3` 伪随机生成板块涨跌数据，喂给 AI 提示词。
    /// 违反 AGENTS.md "All data must be real. No mock data in production paths"。
    ///
    /// 修复方案：调用东财 `clist/get` 接口（m:90+t:2 行业板块）拉真实板块涨跌榜。
    /// 失败时返回 Err，让上层走"数据不可用"分支；不静默填充任何伪数据。
    pub(super) fn get_sector_rankings(&self, overview: &mut MarketOverview) -> Result<()> {
        info!("[大盘] 获取板块涨跌榜...");

        // 拉取真实板块数据
        let sectors_data = self.fetch_real_sector_rankings(20)?;

        if sectors_data.is_empty() {
            anyhow::bail!("板块涨跌榜为空，不使用任何替代数据");
        }

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
            "[大盘] 真实板块数据 {} 条, 领涨:{} 领跌:{}",
            sectors_data.len(),
            overview.top_sectors.len(),
            overview.bottom_sectors.len()
        );

        Ok(())
    }

    /// 调用东财 clist/get 拉行业板块涨跌幅榜
    ///
    /// 字段：f3=涨跌幅(%), f12=板块代码, f14=板块名称
    /// m:90+t:2 是"行业板块"
    /// 返回按涨跌幅降序排列的 (name, change_pct) 列表
    fn fetch_real_sector_rankings(&self, top_n: usize) -> Result<Vec<(String, f64)>> {
        fetch_sector_rankings_impl(top_n)
    }
}

fn fetch_sector_rankings_impl(top_n: usize) -> Result<Vec<(String, f64)>> {
    let url = "https://push2.eastmoney.com/api/qt/clist/get";
    let params: &[(&str, &str)] = &[
        ("pn", "1"),
        ("pz", &top_n.to_string()),
        ("po", "1"), // 降序
        ("np", "1"),
        ("fltt", "2"),
        ("invt", "2"),
        ("fid", "f3"),
        ("fs", "m:90+t:2"), // 行业板块
        ("fields", "f1,f2,f3,f4,f12,f14"),
        ("_", "0"),
    ];
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .context("创建 HTTP 客户端失败 (sector)")?;
    // 修复 Top10#5 (2026-06-29 audit): 用统一 block_on_async 替代 block_in_place + Handle::current().block_on
    let resp_text = crate::block_on_async(async {
        client
            .get(url)
            .query(params)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
            )
            .header("Referer", "https://quote.eastmoney.com/")
            .send()
            .await?
            .text()
            .await
    })
    .map_err(|e: reqwest::Error| anyhow::anyhow!("板块接口 HTTP 失败: {e}"))?;

    let json: Value =
        serde_json::from_str(&resp_text).map_err(|e| anyhow::anyhow!("板块响应非 JSON: {e}"))?;
    let diff = json
        .get("data")
        .and_then(|d| d.get("diff"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("板块响应无 data.diff 数组"))?;
    if diff.is_empty() {
        anyhow::bail!("板块响应 diff 数组为空");
    }
    let mut out: Vec<(String, f64)> = Vec::with_capacity(diff.len());
    for item in diff {
        let name = item
            .get("f14")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("板块项缺少 f14 (name)"))?
            .to_string();
        let change_pct = item
            .get("f3")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow::anyhow!("板块项缺少 f3 (change_pct)"))?;
        out.push((name, change_pct));
    }
    // 按 f3 降序（接口已 po=1, 这里再保险排一次）
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

#[cfg(test)]
mod tests {
    /// 静态检查：非测试代码中不能出现伪随机作为板块数据源。
    /// 修复：QUANT_ANALYST_REVIEW §1.3
    ///
    /// 实现思路：把 `mod tests {` 之前的所有源码单独拿出来检查，
    /// 避免本测试模块自身的字符串污染检查。
    #[test]
    fn no_mock_random_in_sector_data() {
        let src = include_str!("statistics.rs");
        let test_mod_start = src.find("#[cfg(test)]\nmod tests {").unwrap_or(src.len());
        let production_src = &src[..test_mod_start];
        // 真正禁止的伪随机模式
        assert!(
            !production_src.contains("name.len() % 3"),
            "禁止使用 name.len() 模运算等伪随机作为板块数据源（AGENTS.md 红线）"
        );
        assert!(
            !production_src.contains("sectors_template"),
            "禁止在生产路径使用硬编码 sectors_template（AGENTS.md 红线）"
        );
    }
}
