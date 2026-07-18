//! 主力资金流向 + 日内分时形态
//!
//! 数据源：东方财富 push2his
//!   - 历史资金流：`/api/qt/stock/fflow/daykline/get`
//!   - 当日分时：  `/api/qt/stock/trends2/get`
//!
//! 返回结构面向 AI Prompt 组装：直接读字段构建【主力资金】【日内走势】两段。

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 单日资金流数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoneyFlowDay {
    pub date: String,
    pub main_net: f64, // 主力净流入（元）
    pub xl_net: f64,   // 超大单净流入
    pub big_net: f64,  // 大单净流入
    pub main_pct: f64, // 主力净占比 (%)
    pub pct_chg: f64,  // 当日涨跌幅
}

/// 近期资金流汇总
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MoneyFlowSummary {
    pub days: Vec<MoneyFlowDay>, // 最近 N 天，时间升序
}

impl MoneyFlowSummary {
    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    /// 返回最新一天（时间最大）
    pub fn latest(&self) -> Option<&MoneyFlowDay> {
        self.days.last()
    }

    /// 近 n 天主力净流入累计（元）
    pub fn recent_main_sum(&self, n: usize) -> f64 {
        let len = self.days.len();
        let start = len.saturating_sub(n);
        self.days[start..].iter().map(|d| d.main_net).sum()
    }

    /// 指数加权主力净流入（以亿为单位）
    /// 权重：最新日 0.4 / -1 0.25 / -2 0.15 / -3 0.1 / -4 0.1
    pub fn ewma_main_net_yi(&self) -> Option<f64> {
        if self.days.is_empty() {
            return None;
        }
        const W: [f64; 5] = [0.1, 0.1, 0.15, 0.25, 0.4]; // 从老到新
        let take = self.days.len().min(5);
        let slice = &self.days[self.days.len() - take..];
        // 取对应尾部权重
        let w_slice = &W[5 - take..];
        let total_w: f64 = w_slice.iter().sum();
        let weighted: f64 = slice
            .iter()
            .zip(w_slice.iter())
            .map(|(d, w)| d.main_net * w)
            .sum();
        Some(weighted / total_w / 1e8)
    }

    /// 检测“单日反弹但趋势未逆转”：
    /// 近 5 日累计流出 > 30 亿 且 最新日流入 < 5 日累计流出绝对值的 20%
    pub fn is_one_day_bounce(&self) -> bool {
        let sum5_yi = self.recent_main_sum(5) / 1e8;
        let Some(latest) = self.latest() else {
            return false;
        };
        let latest_yi = latest.main_net / 1e8;
        sum5_yi < -30.0 && latest_yi > 0.0 && latest_yi < (-sum5_yi) * 0.2
    }
}

/// 日内分时形态
#[derive(Debug, Clone, Default)]
pub struct IntradayShape {
    pub date: String,              // 交易日
    pub pre_close: f64,            // 昨收
    pub open_pct: f64,             // 开盘涨幅 (%)
    pub high_pct: f64,             // 日内最高涨幅 (%)
    pub low_pct: f64,              // 日内最低涨幅 (%)
    pub close_pct: f64,            // 收盘涨幅 (%)
    pub amplitude: f64,            // 日内振幅 = (high-low)/pre_close (%)
    pub tail_30m_pct: Option<f64>, // 尾盘 30 分钟涨幅（14:30→15:00）；未到 14:30 为 None
    pub shape_label: &'static str, // 形态标签（中文描述）
    pub present: bool,             // 是否成功获取数据
}

/// 转 A 股代码为 EM 数字市场前缀
fn to_em_numeric_secid(code: &str) -> String {
    let market = if code.starts_with('6') || code.starts_with("900") {
        "1" // 沪市
    } else {
        "0" // 深市 / 创业板 / 北交所（EM 约定）
    };
    format!("{}.{}", market, code)
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn parse_em_money_flow_rows(rows: &[Value]) -> Result<MoneyFlowSummary> {
    if rows.is_empty() {
        return Err(anyhow!("资金流 klines 为空"));
    }
    let mut days = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let raw = row
            .as_str()
            .ok_or_else(|| anyhow!("资金流第 {} 行不是字符串", index + 1))?;
        let parts: Vec<&str> = raw.split(',').collect();
        if parts.len() < 13 {
            return Err(anyhow!(
                "资金流第 {} 行字段不足: expected>=13 actual={}",
                index + 1,
                parts.len()
            ));
        }
        chrono::NaiveDate::parse_from_str(parts[0], "%Y-%m-%d")
            .map_err(|error| anyhow!("资金流第 {} 行日期非法: {error}", index + 1))?;
        let parse = |field: usize, name: &str| -> Result<f64> {
            let value = parts[field]
                .parse::<f64>()
                .map_err(|error| anyhow!("资金流第 {} 行 {name} 解析失败: {error}", index + 1))?;
            if !value.is_finite() {
                return Err(anyhow!("资金流第 {} 行 {name} 非有限", index + 1));
            }
            Ok(value)
        };
        let main_net = parse(1, "main_net")?;
        let big_net = parse(4, "big_net")?;
        let xl_net = parse(5, "xl_net")?;
        let main_pct = parse(6, "main_pct")?;
        let pct_chg = parse(12, "pct_chg")?;
        if main_pct.abs() > 100.0 || pct_chg.abs() > 20.0 {
            return Err(anyhow!(
                "资金流第 {} 行比例越界: main_pct={} pct_chg={}",
                index + 1,
                main_pct,
                pct_chg
            ));
        }
        days.push(MoneyFlowDay {
            date: parts[0].to_string(),
            main_net,
            big_net,
            xl_net,
            main_pct,
            pct_chg,
        });
    }
    days.sort_by(|left, right| left.date.cmp(&right.date));
    for pair in days.windows(2) {
        let left = chrono::NaiveDate::parse_from_str(&pair[0].date, "%Y-%m-%d")?;
        let right = chrono::NaiveDate::parse_from_str(&pair[1].date, "%Y-%m-%d")?;
        if left == right {
            return Err(anyhow!("资金流日期重复: {left}"));
        }
        let expected = crate::calendar::next_trading_day(left);
        if right != expected {
            return Err(anyhow!(
                "资金流交易日断档: {left} 后应为 {expected}, 实际为 {right}"
            ));
        }
    }
    Ok(MoneyFlowSummary { days })
}

/// push2his 多主机列表，主→备顺序。
const PUSH2HIS_HOSTS: [&str; 3] = [
    "push2his.eastmoney.com",
    "push2his-bak.eastmoney.com",
    "82.push2his.eastmoney.com",
];

/// 抓取近 `lmt` 天资金流（daykline）
pub async fn fetch_flow_history_async(
    client: &reqwest::Client,
    code: &str,
    lmt: usize,
) -> Result<MoneyFlowSummary> {
    let secid = to_em_numeric_secid(code);
    let mut last_err = String::new();

    for host in PUSH2HIS_HOSTS {
        let url = format!(
            "https://{}/api/qt/stock/fflow/daykline/get?\
             secid={}&lmt={}&klt=101&\
             fields1=f1,f2,f3,f7&\
             fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61,f62,f63,f64,f65",
            host, secid, lmt
        );
        log::debug!("[资金流][日线] host={} {}", host, url);

        let resp = match client
            .get(&url)
            .header("Referer", "https://data.eastmoney.com/")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("{}: {}", host, e);
                continue;
            }
        };
        if !resp.status().is_success() {
            last_err = format!("{}: 状态码 {}", host, resp.status());
            continue;
        }
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                last_err = format!("{}: 读取响应失败 {}", host, e);
                continue;
            }
        };
        let body = text.trim_start();
        if body.starts_with('<') {
            last_err = format!("{}: 非JSON回包（网关拦截）", host);
            continue;
        }
        let json: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                last_err = format!("{}: JSON解析失败 {}", host, e);
                continue;
            }
        };

        let Some(klines) = json
            .get("data")
            .and_then(|d| d.get("klines"))
            .and_then(|v| v.as_array())
        else {
            last_err = format!("{}: 资金流无 klines 数组", host);
            continue;
        };

        // 字段顺序（EM 实测）：date, f52(主力), f53(小单), f54(中单), f55(大单),
        //                      f56(超大单), f57(主力%), f58(小单%), f59(中单%),
        //                      f60(大单%), f61(超大单%), f62(收盘价), f63(涨跌幅%), _, _
        match parse_em_money_flow_rows(klines) {
            Ok(summary) => return Ok(summary),
            Err(error) => {
                last_err = format!("{host}: 资金流批次校验失败: {error}");
            }
        }
    }

    Err(anyhow!("资金流历史全部完整来源失败: {last_err}"))
}

type IntradayRow = (String, f64, f64, f64, f64);

fn parse_intraday_rows(rows: &[Value]) -> Result<Vec<IntradayRow>> {
    let mut parsed: Vec<IntradayRow> = Vec::with_capacity(rows.len());
    let mut previous_ts: Option<chrono::NaiveDateTime> = None;
    for (index, row) in rows.iter().enumerate() {
        let raw = row
            .as_str()
            .ok_or_else(|| anyhow!("分时第 {} 行不是字符串", index + 1))?;
        let parts: Vec<&str> = raw.split(',').collect();
        if parts.len() < 6 {
            return Err(anyhow!(
                "分时第 {} 行字段不足: expected>=6 actual={}",
                index + 1,
                parts.len()
            ));
        }
        let ts = chrono::NaiveDateTime::parse_from_str(parts[0], "%Y-%m-%d %H:%M")
            .map_err(|error| anyhow!("分时第 {} 行时间非法: {error}", index + 1))?;
        if let Some(previous) = previous_ts {
            let delta = ts - previous;
            let valid_lunch = previous.time()
                == chrono::NaiveTime::from_hms_opt(11, 30, 0).unwrap()
                && ts.time() == chrono::NaiveTime::from_hms_opt(13, 0, 0).unwrap()
                && ts.date() == previous.date();
            if ts <= previous || (delta != chrono::Duration::minutes(1) && !valid_lunch) {
                return Err(anyhow!(
                    "分时时间重复、倒序或缺口: {} -> {}",
                    previous,
                    parts[0]
                ));
            }
        }
        previous_ts = Some(ts);
        let parse = |field: usize, name: &str| -> Result<f64> {
            let value = parts[field]
                .parse::<f64>()
                .map_err(|error| anyhow!("分时第 {} 行 {name} 解析失败: {error}", index + 1))?;
            if !value.is_finite() {
                return Err(anyhow!("分时第 {} 行 {name} 非有限", index + 1));
            }
            Ok(value)
        };
        let open = parse(1, "open")?;
        let close = parse(2, "close")?;
        let high = parse(3, "high")?;
        let low = parse(4, "low")?;
        let volume = parse(5, "volume")?;
        if open <= 0.0
            || close <= 0.0
            || high <= 0.0
            || low <= 0.0
            || volume < 0.0
            || high < open.max(close)
            || low > open.min(close)
            || high < low
        {
            return Err(anyhow!("分时第 {} 行 OHLCV 非法", index + 1));
        }
        if let Some(previous) = parsed.last() {
            let change = (close / previous.2 - 1.0).abs();
            if change > 0.20 {
                return Err(anyhow!(
                    "分时相邻收盘变化超过 ±20%: {} -> {}",
                    previous.2,
                    close
                ));
            }
        }
        parsed.push((parts[0].to_string(), open, close, high, low));
    }
    if parsed.is_empty() {
        return Err(anyhow!("分时批次为空"));
    }
    Ok(parsed)
}

fn classify_intraday_shape(
    open_pct: f64,
    high_pct: f64,
    low_pct: f64,
    close_pct: f64,
    amplitude: f64,
    tail_30m_pct: Option<f64>,
) -> &'static str {
    let gap_from_high = high_pct - close_pct;
    let gap_from_low = close_pct - low_pct;
    if high_pct >= 2.0 && gap_from_high >= 2.0 && close_pct < high_pct * 0.5 {
        "⚠️ 冲高回落（尾盘跳水风险大）"
    } else if tail_30m_pct.is_some_and(|value| value <= -1.5) {
        "⚠️ 尾盘跳水"
    } else if tail_30m_pct.is_some_and(|value| value >= 1.5) && close_pct > open_pct {
        "🔥 尾盘拉升（资金抢筹）"
    } else if open_pct >= 2.0 && close_pct <= open_pct - 1.5 {
        "⚠️ 高开低走"
    } else if open_pct <= -1.5 && close_pct >= open_pct + 2.0 {
        "🔥 低开高走（空头回补）"
    } else if close_pct >= high_pct - 0.5 && high_pct > 1.5 {
        "✅ 稳步推高，收在日内高点"
    } else if close_pct <= low_pct + 0.5 && low_pct < -1.5 {
        "🔴 持续下行，收在日内低点"
    } else if amplitude >= 4.0 && gap_from_low >= 2.0 && gap_from_high >= 2.0 {
        "中阳/中阴，日内震荡剧烈"
    } else {
        "窄幅整理"
    }
}

/// BR-115: build a shape only from a complete, locally validated intraday batch.
fn build_intraday_shape(pre_close: f64, trends: &[Value]) -> Result<IntradayShape> {
    if pre_close <= 0.0 || !pre_close.is_finite() {
        return Err(anyhow!("分时 preClose 非法: {}", pre_close));
    }
    if trends.is_empty() {
        return Err(anyhow!("分时 trends 为空"));
    }

    let parsed_rows = parse_intraday_rows(trends)?;
    let first = parsed_rows
        .first()
        .ok_or_else(|| anyhow!("分时 首根缺失"))?;
    let last = parsed_rows.last().ok_or_else(|| anyhow!("分时 尾根缺失"))?;

    let date = last
        .0
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("分时 尾根缺日期"))?
        .to_string();

    let mut day_high = f64::NEG_INFINITY;
    let mut day_low = f64::INFINITY;
    let mut tail_start_close: Option<f64> = None;
    for (ts, _open, close, high, low) in &parsed_rows {
        if *high > day_high {
            day_high = *high;
        }
        if *low < day_low {
            day_low = *low;
        }
        if tail_start_close.is_none() {
            if let Some(hm) = ts.split_whitespace().nth(1) {
                if hm >= "14:30" {
                    tail_start_close = Some(*close);
                }
            }
        }
    }
    if !day_high.is_finite() || !day_low.is_finite() {
        return Err(anyhow!("分时 高低价未找到"));
    }

    let open_pct = (first.1 / pre_close - 1.0) * 100.0;
    let high_pct = (day_high / pre_close - 1.0) * 100.0;
    let low_pct = (day_low / pre_close - 1.0) * 100.0;
    let close_pct = (last.2 / pre_close - 1.0) * 100.0;
    let amplitude = (day_high - day_low) / pre_close * 100.0;
    let tail_30m_pct = tail_start_close.map(|start| (last.2 / start - 1.0) * 100.0);
    if [open_pct, high_pct, low_pct, close_pct]
        .iter()
        .any(|value| !value.is_finite() || value.abs() > 20.0)
        || tail_30m_pct.is_some_and(|value| !value.is_finite() || value.abs() > 20.0)
    {
        return Err(anyhow!("分时涨跌幅非法或超过 ±20%，需要人工确认"));
    }

    let shape_label = classify_intraday_shape(
        open_pct,
        high_pct,
        low_pct,
        close_pct,
        amplitude,
        tail_30m_pct,
    );

    Ok(IntradayShape {
        date,
        pre_close,
        open_pct,
        high_pct,
        low_pct,
        close_pct,
        amplitude,
        tail_30m_pct,
        shape_label,
        present: true,
    })
}

/// 抓取当日分时（trends2）并计算形态
pub async fn fetch_intraday_shape_async(
    client: &reqwest::Client,
    code: &str,
) -> Result<IntradayShape> {
    let secid = to_em_numeric_secid(code);
    let mut last_err = String::new();
    let mut json_opt: Option<Value> = None;

    for host in PUSH2HIS_HOSTS {
        let url = format!(
            "https://{}/api/qt/stock/trends2/get?\
             secid={}&ndays=1&iscr=0&iscca=0&\
             fields1=f1,f2,f3,f4,f5,f6,f7,f8,f9,f10,f11,f12,f13&\
             fields2=f51,f52,f53,f54,f55,f56,f57,f58",
            host, secid
        );
        log::debug!("[分时] host={} {}", host, url);

        let resp = match client
            .get(&url)
            .header("Referer", "https://quote.eastmoney.com/")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("{}: {}", host, e);
                continue;
            }
        };
        if !resp.status().is_success() {
            last_err = format!("{}: 状态码 {}", host, resp.status());
            continue;
        }
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                last_err = format!("{}: 读取失败 {}", host, e);
                continue;
            }
        };
        let body = text.trim_start();
        if body.starts_with('<') {
            last_err = format!("{}: 非JSON回包（网关拦截）", host);
            continue;
        }
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => {
                json_opt = Some(v);
                break;
            }
            Err(e) => {
                last_err = format!("{}: JSON解析失败 {}", host, e);
                continue;
            }
        }
    }

    let json = json_opt.ok_or_else(|| anyhow!("分时全部主机失败: {}", last_err))?;

    let data = json.get("data").ok_or_else(|| anyhow!("分时 无 data"))?;
    let pre_close = data
        .get("preClose")
        .and_then(as_f64)
        .ok_or_else(|| anyhow!("分时 无 preClose"))?;
    let trends = data
        .get("trends")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("分时 无 trends 数组"))?;
    build_intraday_shape(pre_close, trends)
}

/// 同步包装（在已有 tokio runtime 上下文调用）
pub fn fetch_money_flow_blocking(
    client: &reqwest::Client,
    code: &str,
    lmt: usize,
) -> Result<MoneyFlowSummary> {
    // 修复 Top10#5 (2026-06-29 audit): 用 crate::block_on_async 统一替代
    // Handle::try_current + block_in_place + block_on pattern. 不在 runtime 时 fallback 到 default
    // (保留旧 behavior, 旧 pattern 在不在 runtime 时 return default).
    let client = client.clone();
    let code_s = code.to_string();
    if tokio::runtime::Handle::try_current().is_err() {
        return Err(anyhow!("[资金流] 无 tokio runtime，无法抓取 {code}"));
    }
    crate::block_on_async(async move { fetch_flow_history_async(&client, &code_s, lmt).await })
}

pub fn fetch_intraday_shape_blocking(
    client: &reqwest::Client,
    code: &str,
) -> Result<IntradayShape> {
    // 修复 Top10#5: 同上, 统一 block_on_async
    let client = client.clone();
    let code_s = code.to_string();
    if tokio::runtime::Handle::try_current().is_err() {
        return Err(anyhow!("[分时] 无 tokio runtime，无法抓取 {code}"));
    }
    crate::block_on_async(async move { fetch_intraday_shape_async(&client, &code_s).await })
}

/// 将资金流 + 分时形态格式化为 prompt 片段
/// BR-118: `近5日:` 标签是旧评分兼容路径的稳定字段边界。
pub fn format_for_prompt(flow: &MoneyFlowSummary, shape: &IntradayShape) -> String {
    let mut out = String::new();

    // ---- 主力资金 ----
    if !flow.is_empty() {
        out.push_str("\n【主力资金流向（真实口径，单位：亿元）】\n");
        out.push_str("日期 | 涨跌幅% | 主力净流入 | 主力占比% | 超大单 | 大单\n");
        for d in flow
            .days
            .iter()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .iter()
            .rev()
        {
            out.push_str(&format!(
                "{} | {:+.2}% | {:+.2} | {:+.2}% | {:+.2} | {:+.2}\n",
                d.date,
                d.pct_chg,
                d.main_net / 1e8,
                d.main_pct,
                d.xl_net / 1e8,
                d.big_net / 1e8,
            ));
        }
        if let Some(latest) = flow.latest() {
            let label = if latest.main_net > 0.0 && latest.pct_chg > 1.0 {
                "🔥 今日主力真金白银买入，与股价同向上涨"
            } else if latest.main_net < 0.0 && latest.pct_chg > 1.0 {
                "⚠️ 今日股价上涨但主力净流出 — 典型诱多/拉高出货"
            } else if latest.main_net > 0.0 && latest.pct_chg < -1.0 {
                "📈 今日股价下跌但主力净流入 — 可能是主力低吸"
            } else if latest.main_net < 0.0 && latest.pct_chg < -1.0 {
                "🔴 今日股价下跌且主力净流出 — 杀跌趋势"
            } else {
                "资金流与股价方向基本一致"
            };
            out.push_str(&format!("最新信号: {}\n", label));
        }
        let sum_3 = flow.recent_main_sum(3);
        let sum_5 = flow.recent_main_sum(5);
        out.push_str(&format!(
            "近3日主力累计净流入: {:+.2}亿 | 近5日: {:+.2}亿\n",
            sum_3 / 1e8,
            sum_5 / 1e8
        ));
    }

    // ---- 日内分时 ----
    if shape.present {
        out.push_str("\n【日内分时形态】\n");
        out.push_str(&format!(
            "开盘{:+.2}% | 最高{:+.2}% | 最低{:+.2}% | 收盘{:+.2}%\n",
            shape.open_pct, shape.high_pct, shape.low_pct, shape.close_pct,
        ));
        let tail = shape
            .tail_30m_pct
            .map(|value| format!("{value:+.2}%"))
            .unwrap_or_else(|| "暂无（未到14:30）".to_string());
        out.push_str(&format!(
            "日内振幅: {:.2}%  尾盘30分钟涨幅: {}\n",
            shape.amplitude, tail
        ));
        out.push_str(&format!("日内形态: {}\n", shape.shape_label));
    }

    out
}

#[cfg(test)]
mod br115_tests {
    use super::*;

    #[test]
    fn malformed_money_flow_row_rejects_entire_batch() {
        let rows = vec![
            Value::String("2026-07-16,100,0,0,40,60,3.2,0,0,0,0,10,1.5".to_string()),
            Value::String("2026-07-17,bad,0,0,40,60,3.2,0,0,0,0,10,1.5".to_string()),
        ];
        assert!(parse_em_money_flow_rows(&rows).is_err());
    }

    #[test]
    fn money_flow_rejects_duplicate_and_missing_trading_days() {
        let row = |date: &str| Value::String(format!("{date},100,0,0,40,60,3.2,0,0,0,0,10,1.5"));
        assert!(parse_em_money_flow_rows(&[row("2026-07-16"), row("2026-07-16")]).is_err());
        assert!(parse_em_money_flow_rows(&[row("2026-07-15"), row("2026-07-17")]).is_err());
    }

    #[test]
    fn malformed_intraday_row_rejects_entire_batch() {
        let rows = vec![
            Value::String("2026-07-18 09:30,10,10.1,10.2,9.9,100".to_string()),
            Value::String("broken".to_string()),
        ];
        assert!(parse_intraday_rows(&rows).is_err());
    }

    #[test]
    fn intraday_batch_rejects_time_gap_and_adjacent_price_jump() {
        let gap = vec![
            Value::String("2026-07-17 09:30,10,10,10.1,9.9,100".to_string()),
            Value::String("2026-07-17 09:32,10,10.1,10.2,9.9,100".to_string()),
        ];
        assert!(parse_intraday_rows(&gap).is_err());

        let jump = vec![
            Value::String("2026-07-17 09:30,10,10,10.1,9.9,100".to_string()),
            Value::String("2026-07-17 09:31,13,13,13.1,12.9,100".to_string()),
        ];
        assert!(parse_intraday_rows(&jump).is_err());
    }

    fn flow_day(date: &str, main_net: f64, pct_chg: f64) -> MoneyFlowDay {
        MoneyFlowDay {
            date: date.into(),
            main_net,
            xl_net: main_net * 0.6,
            big_net: main_net * 0.4,
            main_pct: 3.0,
            pct_chg,
        }
    }

    #[test]
    fn summary_math_and_bounce_detection_use_latest_five_real_days() {
        let empty = MoneyFlowSummary::default();
        assert!(empty.is_empty());
        assert!(empty.latest().is_none());
        assert_eq!(empty.recent_main_sum(5), 0.0);
        assert_eq!(empty.ewma_main_net_yi(), None);
        assert!(!empty.is_one_day_bounce());

        let flow = MoneyFlowSummary {
            days: vec![
                flow_day("2026-07-13", -1_000_000_000.0, -1.0),
                flow_day("2026-07-14", -1_000_000_000.0, -1.0),
                flow_day("2026-07-15", -1_000_000_000.0, -1.0),
                flow_day("2026-07-16", -1_000_000_000.0, -1.0),
                flow_day("2026-07-17", 500_000_000.0, 1.0),
            ],
        };
        assert_eq!(flow.latest().expect("latest").date, "2026-07-17");
        assert_eq!(flow.recent_main_sum(3), -1_500_000_000.0);
        assert!((flow.ewma_main_net_yi().expect("ewma") + 4.0).abs() < 1e-9);
        assert!(flow.is_one_day_bounce());

        let large_bounce = MoneyFlowSummary {
            days: vec![flow_day("2026-07-17", 5_000_000_000.0, 1.0)],
        };
        assert!(!large_bounce.is_one_day_bounce());
    }

    #[test]
    fn money_flow_parser_accepts_sorted_trading_days_and_rejects_each_bad_field_class() {
        let row = |date: &str, main: &str, main_pct: &str, pct: &str| {
            Value::String(format!(
                "{date},{main},0,0,40,60,{main_pct},0,0,0,0,10,{pct}"
            ))
        };
        let parsed = parse_em_money_flow_rows(&[
            row("2026-07-17", "200", "3.2", "1.5"),
            row("2026-07-16", "100", "2.2", "-1.0"),
        ])
        .expect("valid consecutive trading days");
        assert_eq!(parsed.days[0].date, "2026-07-16");
        assert_eq!(parsed.days[1].main_net, 200.0);

        assert!(parse_em_money_flow_rows(&[]).is_err());
        assert!(parse_em_money_flow_rows(&[Value::Bool(true)]).is_err());
        assert!(parse_em_money_flow_rows(&[Value::String("short".into())]).is_err());
        assert!(parse_em_money_flow_rows(&[row("bad-date", "1", "1", "1")]).is_err());
        assert!(parse_em_money_flow_rows(&[row("2026-07-17", "NaN", "1", "1")]).is_err());
        assert!(parse_em_money_flow_rows(&[row("2026-07-17", "1", "101", "1")]).is_err());
        assert!(parse_em_money_flow_rows(&[row("2026-07-17", "1", "1", "21")]).is_err());
    }

    #[test]
    fn intraday_parser_accepts_lunch_break_and_rejects_bad_protocol_rows() {
        let lunch = vec![
            Value::String("2026-07-17 11:30,10,10,10.1,9.9,100".into()),
            Value::String("2026-07-17 13:00,10,10.1,10.2,9.9,0".into()),
        ];
        assert_eq!(parse_intraday_rows(&lunch).expect("lunch break").len(), 2);

        let bad_rows = [
            vec![],
            vec![Value::Bool(true)],
            vec![Value::String("2026-07-17 09:30,10".into())],
            vec![Value::String("bad,10,10,10,10,1".into())],
            vec![Value::String("2026-07-17 09:30,bad,10,10,10,1".into())],
            vec![Value::String("2026-07-17 09:30,10,10,10,10,-1".into())],
            vec![Value::String("2026-07-17 09:30,10,10,9,10,1".into())],
            vec![Value::String("2026-07-17 09:30,10,10,10,11,1".into())],
        ];
        for rows in bad_rows {
            assert!(parse_intraday_rows(&rows).is_err(), "rows={rows:?}");
        }

        let reversed = vec![
            Value::String("2026-07-17 09:31,10,10,10,10,1".into()),
            Value::String("2026-07-17 09:30,10,10,10,10,1".into()),
        ];
        assert!(parse_intraday_rows(&reversed).is_err());
    }

    #[test]
    fn scalar_helpers_and_blocking_wrappers_fail_closed() {
        assert_eq!(to_em_numeric_secid("600519"), "1.600519");
        assert_eq!(to_em_numeric_secid("900901"), "1.900901");
        assert_eq!(to_em_numeric_secid("000001"), "0.000001");
        assert_eq!(as_f64(&serde_json::json!(1.25)), Some(1.25));
        assert_eq!(as_f64(&serde_json::json!(" 2.5 ")), Some(2.5));
        assert_eq!(as_f64(&Value::Bool(true)), None);

        let client = reqwest::Client::new();
        assert!(fetch_money_flow_blocking(&client, "TEST_CODE_000001", 5).is_err());
        assert!(fetch_intraday_shape_blocking(&client, "TEST_CODE_000001").is_err());
    }

    #[test]
    fn prompt_renders_each_money_direction_and_optional_intraday_tail() {
        let shape = IntradayShape {
            date: "2026-07-17".into(),
            pre_close: 10.0,
            open_pct: 0.5,
            high_pct: 2.0,
            low_pct: -1.0,
            close_pct: 1.5,
            amplitude: 3.0,
            tail_30m_pct: None,
            shape_label: "窄幅整理",
            present: true,
        };
        let cases = [
            (1.0, 2.0, "真金白银买入"),
            (-1.0, 2.0, "诱多/拉高出货"),
            (1.0, -2.0, "主力低吸"),
            (-1.0, -2.0, "杀跌趋势"),
            (1.0, 0.0, "方向基本一致"),
        ];
        for (main, pct, expected) in cases {
            let flow = MoneyFlowSummary {
                days: vec![flow_day("2026-07-17", main * 1e8, pct)],
            };
            let rendered = format_for_prompt(&flow, &shape);
            assert!(rendered.contains(expected));
            assert!(rendered.contains("暂无（未到14:30）"));
        }

        let tail = IntradayShape {
            tail_30m_pct: Some(1.25),
            ..shape
        };
        let rendered = format_for_prompt(&MoneyFlowSummary::default(), &tail);
        assert!(!rendered.contains("主力资金流向"));
        assert!(rendered.contains("+1.25%"));
        assert!(
            format_for_prompt(&MoneyFlowSummary::default(), &IntradayShape::default()).is_empty()
        );
    }

    #[test]
    fn intraday_shape_builder_classifies_validated_rows() {
        let trends = vec![
            Value::String("2026-07-17 14:30,10,10,10.2,9.8,100".into()),
            Value::String("2026-07-17 14:31,10,10.2,10.3,9.9,100".into()),
        ];
        let shape = build_intraday_shape(10.0, &trends).expect("valid local protocol batch");
        assert_eq!(shape.date, "2026-07-17");
        assert!(shape.present);
        assert_eq!(shape.shape_label, "🔥 尾盘拉升（资金抢筹）");
        assert!(shape.tail_30m_pct.expect("tail") > 0.0);
    }

    #[test]
    fn intraday_shape_classifier_covers_every_documented_band() {
        let cases = [
            ((0.0, 4.0, -1.0, 1.0, 5.0, None), "冲高回落"),
            ((0.0, 1.0, -1.0, 0.0, 2.0, Some(-1.5)), "尾盘跳水"),
            ((0.0, 1.0, -1.0, 0.5, 2.0, Some(1.5)), "尾盘拉升"),
            ((2.0, 2.0, 0.0, 0.5, 2.0, None), "高开低走"),
            ((-2.0, 0.5, -2.5, 0.0, 3.0, None), "低开高走"),
            ((0.0, 2.0, -0.5, 1.8, 2.5, None), "稳步推高"),
            ((0.0, 0.5, -2.0, -1.8, 2.5, None), "持续下行"),
            ((0.0, 4.0, 0.0, 2.0, 4.0, None), "震荡剧烈"),
            ((0.0, 1.0, -1.0, 0.0, 2.0, None), "窄幅整理"),
        ];
        for ((open, high, low, close, amplitude, tail), expected) in cases {
            let label = classify_intraday_shape(open, high, low, close, amplitude, tail);
            assert!(
                label.contains(expected),
                "expected={expected} label={label}"
            );
        }
    }

    #[test]
    fn intraday_shape_builder_rejects_missing_or_out_of_range_evidence() {
        assert!(build_intraday_shape(0.0, &[]).is_err());
        assert!(build_intraday_shape(10.0, &[]).is_err());
        let too_far = vec![Value::String("2026-07-17 09:30,13,13,13,13,100".into())];
        assert!(build_intraday_shape(10.0, &too_far).is_err());
    }
}
