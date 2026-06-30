//! 真正的 sync 版 get_market_overview (P1.1 真正修复 v4)
//!
//! 与 `MarketAnalyzer::get_market_overview` 的区别:
//!   - 所有 HTTP 用 `reqwest::blocking`, 完全不用 tokio machinery
//!   - 不需要 async runtime 上下文, 不需要 block_in_place
//!   - 在 --review 这种"调一次就退出"的场景下 100% 安全
//!
//! 历史:
//!   - v1: 用 reqwest::Client (async) + .await, 触发 tokio runtime drop panic
//!   - v2: 改成 reqwest::blocking, 仍 panic (MarketAnalyzer::new 用 reqwest::blocking::Client
//!         builder().build() 内部创建 tokio runtime, 在 async context 里 drop 触发 panic)
//!   - v3 (本版): 彻底不要 MarketAnalyzer 引用, 直接 free function, 只用 reqwest::blocking
//!         不创建任何 tokio runtime, 安全

use crate::data_provider::north_flow::NorthFlowClient;
use crate::market_data::{MarketIndex, MarketOverview, SectorInfo};
use anyhow::{Context, Result};
use chrono::Local;
use log::{info, warn};
use std::time::Duration;

/// 同步版 get_market_overview, 纯 free function
/// 不接受 MarketAnalyzer 引用, 避免 Client::builder().build() 触发 panic
pub fn get_market_overview_blocking() -> Result<MarketOverview> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let mut overview = MarketOverview::new(today);

    // 1. 指数 (同步)
    match fetch_indices_blocking() {
        Ok(indices) => overview.indices = indices,
        Err(e) => warn!("[大盘 blocking] 指数拉取失败: {}", e),
    }

    // 2. 北向资金 (同步)
    match fetch_north_flow_blocking() {
        Ok(n) => {
            info!("[大盘 blocking] 北向资金: {:+.2}亿", n);
            overview.north_flow = Some(n);
        }
        Err(e) => warn!("[大盘 blocking] 北向资金拉取失败: {}", e),
    }
    // 修复 P1-3: 0.0 视为数据缺失 (unwrap_or 兜底假成功), 不写入 overview
    if let Some(0.0) = overview.north_flow {
        warn!("[大盘 blocking] 北向资金返回 0.0 (疑似 unwrap_or 假成功), 标记为缺失");
        overview.north_flow = None;
    }

    // 3. 板块涨跌榜 (同步)
    match fetch_sectors_blocking() {
        Ok((top, bottom)) => {
            overview.top_sectors = top;
            overview.bottom_sectors = bottom;
        }
        Err(e) => warn!("[大盘 blocking] 板块涨跌榜失败: {}", e),
    }

    // 4. 涨跌统计 (同步, 取首页 500 只)
    match fetch_market_stats_blocking() {
        Ok((up, down, flat, lim_up, lim_down, amount)) => {
            overview.up_count = up;
            overview.down_count = down;
            overview.flat_count = flat;
            overview.limit_up_count = lim_up;
            overview.limit_down_count = lim_down;
            overview.total_amount = amount;
        }
        Err(e) => warn!("[大盘 blocking] 涨跌统计失败: {}", e),
    }

    Ok(overview)
}

/// 生成市场概览报告文本 (在 --review 模式直接调用, 不需要 MarketAnalyzer)
pub fn generate_market_overview_text_blocking() -> String {
    match get_market_overview_blocking() {
        Ok(overview) => {
            // 用 review.rs 的 generate_market_review 但需要 analyzer
            // 这里直接复用 format_market_report 的逻辑
            format_market_report(&overview)
        }
        Err(e) => {
            warn!("[大盘 blocking] 获取失败: {}", e);
            String::new()
        }
    }
}

/// 同步拉主要指数 (腾讯财经)
fn fetch_indices_blocking() -> Result<Vec<MarketIndex>> {
    let url = "http://qt.gtimg.cn/q=sh000001,sz399001,sz399006,sh000300,sh000688";
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .context("指数 HTTP 客户端构建失败")?;
    let text = client
        .get(url)
        .send()
        .context("指数 HTTP 请求失败")?
        .text()
        .context("指数响应读取失败")?;
    parse_tencent_indices(&text)
}

/// 解析腾讯财经指数响应 (v_sh000001="1~上证指数~...";)
fn parse_tencent_indices(text: &str) -> Result<Vec<MarketIndex>> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(start) = line.find('"') {
            if let Some(end) = line.rfind('"') {
                if start < end {
                    let data = &line[start + 1..end];
                    let parts: Vec<&str> = data.split('~').collect();
                    if parts.len() >= 6 {
                        let name = parts.get(1).unwrap_or(&"").to_string();
                        let code = parts.get(2).unwrap_or(&"").to_string();
                        let current: f64 = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                        let prev: f64 = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                        let pct = if prev > 0.0 {
                            (current - prev) / prev * 100.0
                        } else {
                            0.0
                        };
                        out.push(MarketIndex {
                            code,
                            name,
                            current,
                            change: current - prev,
                            change_pct: pct,
                            open: parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(0.0),
                            high: 0.0,
                            low: 0.0,
                            prev_close: prev,
                            volume: 0.0,
                            amount: 0.0,
                            amplitude: 0.0,
                        });
                    }
                }
            }
        }
    }
    if out.is_empty() {
        anyhow::bail!("指数解析无结果");
    }
    Ok(out)
}

/// 同步拉北向资金
fn fetch_north_flow_blocking() -> Result<f64> {
    let client = NorthFlowClient::new();
    let series = client
        .fetch_blocking()
        .map_err(|e| anyhow::anyhow!("北向资金拉取失败: {e}"))?;
    let v = series.latest_total().unwrap_or(0.0);
    Ok(v)
}

/// 同步拉板块涨跌榜 (东财 clist/get, m:90+t:2)
fn fetch_sectors_blocking() -> Result<(Vec<SectorInfo>, Vec<SectorInfo>)> {
    let url = "https://push2.eastmoney.com/api/qt/clist/get";
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
        .build()
        .context("板块 HTTP 客户端构建失败")?;
    let resp = client
        .get(url)
        .query(&[
            ("pn", "1"),
            ("pz", "20"),
            ("po", "1"),
            ("np", "1"),
            ("fltt", "2"),
            ("invt", "2"),
            ("fid", "f3"),
            ("fs", "m:90+t:2"),
            ("fields", "f1,f2,f3,f4,f12,f14"),
        ])
        .header("Referer", "https://quote.eastmoney.com/")
        .send()
        .context("板块 HTTP 请求失败")?;
    let json: serde_json::Value = resp
        .json()
        .context("板块响应非 JSON")?;
    let diff = json
        .get("data")
        .and_then(|d| d.get("diff"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("板块响应无 data.diff"))?;
    let mut entries: Vec<(String, f64)> = Vec::new();
    for item in diff {
        let name = item
            .get("f14")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("板块项缺 f14"))?
            .to_string();
        let change_pct = item
            .get("f3")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow::anyhow!("板块项缺 f3"))?;
        entries.push((name, change_pct));
    }
    entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top: Vec<SectorInfo> = entries
        .iter()
        .take(5)
        .map(|(name, change_pct)| SectorInfo {
            name: name.clone(),
            change_pct: *change_pct,
        })
        .collect();
    let bottom: Vec<SectorInfo> = entries
        .iter()
        .rev()
        .take(5)
        .map(|(name, change_pct)| SectorInfo {
            name: name.clone(),
            change_pct: *change_pct,
        })
        .collect();
    Ok((top, bottom))
}

/// 同步拉涨跌统计 (新浪 API, 取首页 500 只)
fn fetch_market_stats_blocking() -> Result<(i32, i32, i32, i32, i32, f64)> {
    let url = "http://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/Market_Center.getHQNodeData";
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("涨跌统计 HTTP 客户端构建失败")?;
    let resp = client
        .get(url)
        .query(&[
            ("page", "1"),
            ("num", "500"),
            ("sort", "symbol"),
            ("asc", "1"),
            ("node", "hs_a"),
            ("symbol", ""),
            ("_s_r_a", "page"),
        ])
        .send()
        .context("涨跌统计 HTTP 请求失败")?;
    let json: serde_json::Value = resp
        .json()
        .context("涨跌统计响应非 JSON")?;
    let arr = json
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("涨跌统计响应不是数组"))?;
    let mut up = 0;
    let mut down = 0;
    let mut flat = 0;
    let mut lim_up = 0;
    let mut lim_down = 0;
    let mut amount = 0.0;
    for stock in arr {
        let change_pct: f64 = stock
            .get("changepercent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if change_pct > 0.0 {
            up += 1;
        } else if change_pct < 0.0 {
            down += 1;
        } else {
            flat += 1;
        }
        if change_pct >= 9.9 {
            lim_up += 1;
        } else if change_pct <= -9.9 {
            lim_down += 1;
        }
        let a: f64 = stock
            .get("amount")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        amount += a;
    }
    Ok((up, down, flat, lim_up, lim_down, amount / 1e8))
}

/// 生成市场概览报告 (替代 MarketAnalyzer::generate_market_review)
/// 这里我们内联 review.rs 的 format_market_report 逻辑, 避免依赖 analyzer
fn format_market_report(overview: &MarketOverview) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "# 📊 A股市场概览 ({})", overview.date);
    let _ = writeln!(s);
    let _ = writeln!(s, "## 一、主要指数");
    for idx in overview.indices.iter().take(5) {
        let _ = writeln!(
            s,
            "- {}: {:.2} ({:+.2}%)",
            idx.name, idx.current, idx.change_pct
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "## 二、涨跌统计");
    let _ = writeln!(s, "| 指标 | 数值 |");
    let _ = writeln!(s, "|------|------|");
    let _ = writeln!(s, "| 上涨家数 | {} |", overview.up_count);
    let _ = writeln!(s, "| 下跌家数 | {} |", overview.down_count);
    let _ = writeln!(s, "| 平盘家数 | {} |", overview.flat_count);
    let _ = writeln!(s, "| 涨停 | {} |", overview.limit_up_count);
    let _ = writeln!(s, "| 跌停 | {} |", overview.limit_down_count);
    let _ = writeln!(s, "| 两市成交额 (500只样本) | {:.0}亿 |", overview.total_amount);
    // 修复 P1-3 (2026-06-30 codex review, BR-012): None 时显式打 [数据缺失], 禁止显示 0.00.
    match overview.north_flow {
        Some(v) => { let _ = writeln!(s, "| 北向资金 | {:+.2}亿 |", v); }
        None => { let _ = writeln!(s, "| 北向资金 | [数据缺失] |"); }
    }
    let _ = writeln!(s);
    if !overview.top_sectors.is_empty() {
        let _ = writeln!(s, "## 三、领涨板块");
        for s2 in overview.top_sectors.iter() {
            let _ = writeln!(s, "- **{}**: {:+.2}%", s2.name, s2.change_pct);
        }
    }
    // 修复: 只有当存在真正下跌的板块时才显示"领跌"段
    // 之前可能显示 +0.5% 这种"最弱涨幅"被错标为"领跌", 误导
    if !overview.bottom_sectors.is_empty()
        && overview.bottom_sectors.iter().any(|s| s.change_pct < 0.0)
    {
        let _ = writeln!(s);
        let _ = writeln!(s, "## 四、领跌板块");
        for s2 in overview.bottom_sectors.iter() {
            let _ = writeln!(s, "- **{}**: {:+.2}%", s2.name, s2.change_pct);
        }
    } else if !overview.bottom_sectors.is_empty() {
        // 全市场普涨, 把最弱的几个标为"涨幅靠后"而不是"领跌"
        let _ = writeln!(s);
        let _ = writeln!(s, "## 四、涨幅靠后板块 (全市场普涨, 无下跌板块)");
        for s2 in overview.bottom_sectors.iter() {
            let _ = writeln!(s, "- **{}**: {:+.2}%", s2.name, s2.change_pct);
        }
    }
    s
}

// =============================================================================
// 测试
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tencent_indices_basic() {
        let text = r#"v_sh000001="1~上证指数~000001~4139.90~4132.61~4125.22~50000000~5000000000";
v_sz399001="1~深证成指~399001~12500.00~12450.00~12400.00~80000000~9000000000";"#;
        let indices = parse_tencent_indices(text).unwrap();
        assert_eq!(indices.len(), 2);
        assert_eq!(indices[0].code, "000001");
        assert!((indices[0].current - 4139.90).abs() < 1e-6);
        assert!(indices[0].change_pct > 0.0);
        assert!((indices[0].prev_close - 4132.61).abs() < 1e-6);
    }

    #[test]
    fn parse_tencent_indices_empty() {
        let text = "";
        let result = parse_tencent_indices(text);
        assert!(result.is_err());
    }

    #[test]
    fn parse_tencent_indices_malformed() {
        let text = "garbage data without quotes";
        let result = parse_tencent_indices(text);
        assert!(result.is_err());
    }

    #[test]
    fn format_market_report_basic() {
        let mut overview = MarketOverview::new("2026-06-27".to_string());
        overview.indices = vec![MarketIndex {
            code: "000001".into(),
            name: "上证指数".into(),
            current: 4139.90,
            change: 7.29,
            change_pct: 0.18,
            open: 4132.61,
            high: 4140.0,
            low: 4125.0,
            prev_close: 4132.61,
            volume: 0.0,
            amount: 0.0,
            amplitude: 0.0,
        }];
        overview.up_count = 2500;
        overview.down_count = 2000;
        overview.north_flow = Some(12.34);
        let s = format_market_report(&overview);
        assert!(s.contains("上证指数"));
        assert!(s.contains("+0.18%"));
        assert!(s.contains("+12.34亿"));
        assert!(s.contains("2500"));
        assert!(s.contains("2000"));
    }
}
