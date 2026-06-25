//! 主力资金流向 + 日内分时形态
//!
//! 数据源：东方财富 push2his
//!   - 历史资金流：`/api/qt/stock/fflow/daykline/get`
//!   - 当日分时：  `/api/qt/stock/trends2/get`
//!
//! 返回结构面向 AI Prompt 组装：直接读字段构建【主力资金】【日内走势】两段。

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct SinaMoneyflowRow {
    opendate: String,
    r0_net: String,
    changeratio: String,
}

/// 单日资金流数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoneyFlowDay {
    pub date: String,
    pub main_net: f64,       // 主力净流入（元）
    pub xl_net: f64,         // 超大单净流入
    pub big_net: f64,        // 大单净流入
    pub main_pct: f64,       // 主力净占比 (%)
    pub pct_chg: f64,        // 当日涨跌幅
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
    pub date: String,          // 交易日
    pub pre_close: f64,        // 昨收
    pub open_pct: f64,         // 开盘涨幅 (%)
    pub high_pct: f64,         // 日内最高涨幅 (%)
    pub low_pct: f64,          // 日内最低涨幅 (%)
    pub close_pct: f64,        // 收盘涨幅 (%)
    pub amplitude: f64,        // 日内振幅 = (high-low)/pre_close (%)
    pub tail_30m_pct: f64,     // 尾盘 30 分钟涨幅（14:30→15:00）
    pub shape_label: &'static str, // 形态标签（中文描述）
    pub present: bool,         // 是否成功获取数据
}

/// 转 A 股代码为 EM 数字市场前缀
fn to_em_numeric_secid(code: &str) -> String {
    let market = if code.starts_with('6')
        || code.starts_with("900")
        || code.starts_with("688")
    {
        "1" // 沪市
    } else if code.starts_with('8') || code.starts_with('4') {
        "0" // 北交所实际是 0（EM 约定），但 push2his 对北交所资金流无数据，保持一致
    } else {
        "0" // 深市 / 创业板
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

fn to_sina_symbol(code: &str) -> String {
    if code.starts_with('6') {
        format!("sh{}", code)
    } else {
        format!("sz{}", code)
    }
}

fn parse_ratio_to_percent(raw: &str) -> Option<f64> {
    let v = raw.trim().parse::<f64>().ok()?;
    if v.abs() <= 1.0 {
        Some(v * 100.0)
    } else {
        Some(v)
    }
}

async fn fetch_flow_history_sina_async(
    client: &reqwest::Client,
    code: &str,
    lmt: usize,
) -> Result<MoneyFlowSummary> {
    let symbol = to_sina_symbol(code);
    let url = format!(
        "https://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/MoneyFlow.ssl_qsfx_zjlrqs?page=1&num={}&sort=opendate&asc=0&daima={}",
        lmt.max(5), symbol
    );
    let response = client
        .get(&url)
        .send()
        .await
        .context("Sina HTTP请求失败")?;
    if !response.status().is_success() {
        return Err(anyhow!("Sina 状态码 {}", response.status()));
    }

    let text = response.text().await.context("Sina 读取响应失败")?;
    if text.trim().is_empty() {
        return Err(anyhow!("Sina 返回空响应"));
    }
    let rows: Vec<SinaMoneyflowRow> = serde_json::from_str(&text)
        .context("Sina JSON解析失败")?;
    if rows.is_empty() {
        return Ok(MoneyFlowSummary::default());
    }
    let mut days = Vec::new();
    for row in rows {
        let main_net = row
            .r0_net
            .trim()
            .parse::<f64>()
            .map_err(|_| anyhow!("Sina r0_net 解析失败"))?;
        let pct_chg = parse_ratio_to_percent(&row.changeratio)
            .ok_or_else(|| anyhow!("Sina changeratio 解析失败"))?;

        days.push(MoneyFlowDay {
            date: row.opendate,
            main_net,
            xl_net: 0.0,
            big_net: 0.0,
            main_pct: 0.0,
            pct_chg,
        });
    }
    days.sort_by(|a, b| a.date.cmp(&b.date));
    if days.len() > lmt {
        let skip = days.len() - lmt;
        days = days.into_iter().skip(skip).collect();
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
        let mut days = Vec::new();
        for kline in klines {
            let s = match kline.as_str() {
                Some(s) => s,
                None => continue,
            };
            let parts: Vec<&str> = s.split(',').collect();
            if parts.len() < 13 {
                continue;
            }
            let parse_f = |i: usize| parts.get(i).and_then(|p| p.parse::<f64>().ok());
            let (Some(main_net), Some(big_net), Some(xl_net), Some(main_pct), Some(pct_chg)) = (
                parse_f(1),
                parse_f(4),
                parse_f(5),
                parse_f(6),
                parse_f(12),
            ) else {
                continue;
            };
            days.push(MoneyFlowDay {
                date: parts[0].to_string(),
                main_net,
                big_net,
                xl_net,
                main_pct,
                pct_chg,
            });
        }

        return Ok(MoneyFlowSummary { days });
    }

    let east_err = last_err.clone();
    let sina_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    match fetch_flow_history_sina_async(&sina_client, code, lmt).await {
        Ok(summary) if !summary.is_empty() => {
            log::info!("[资金流][Sina] {} 取得 {} 天数据", code, summary.days.len());
            Ok(summary)
        }
        Ok(_) => Err(anyhow!("资金流历史全部主机失败: {}；Sina 返回空数据", east_err)),
        Err(sina_err) => Err(anyhow!(
            "资金流历史全部主机失败: {}；Sina fallback失败: {}",
            east_err,
            sina_err
        )),
    }
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

        let resp = match client.get(&url).header("Referer", "https://quote.eastmoney.com/").send().await {
            Ok(r) => r,
            Err(e) => { last_err = format!("{}: {}", host, e); continue; }
        };
        if !resp.status().is_success() {
            last_err = format!("{}: 状态码 {}", host, resp.status());
            continue;
        }
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => { last_err = format!("{}: 读取失败 {}", host, e); continue; }
        };
        let body = text.trim_start();
        if body.starts_with('<') {
            last_err = format!("{}: 非JSON回包（网关拦截）", host);
            continue;
        }
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => { json_opt = Some(v); break; }
            Err(e) => { last_err = format!("{}: JSON解析失败 {}", host, e); continue; }
        }
    }

    let json = json_opt.ok_or_else(|| anyhow!("分时全部主机失败: {}", last_err))?;

    let data = json
        .get("data")
        .ok_or_else(|| anyhow!("分时 无 data"))?;
    let pre_close = data
        .get("preClose")
        .and_then(as_f64)
        .ok_or_else(|| anyhow!("分时 无 preClose"))?;
    if pre_close <= 0.0 {
        return Err(anyhow!("分时 preClose 非法: {}", pre_close));
    }
    let trends = data
        .get("trends")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("分时 无 trends 数组"))?;
    if trends.is_empty() {
        return Err(anyhow!("分时 trends 为空"));
    }

    // 格式: "date time, open, close, high, low, volume, amount, avg"
    let parse_row = |s: &str| -> Option<(String, f64, f64, f64, f64)> {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() < 6 {
            return None;
        }
        let ts = parts[0].to_string();
        let open = parts[1].parse::<f64>().ok()?;
        let close = parts[2].parse::<f64>().ok()?;
        let high = parts[3].parse::<f64>().ok()?;
        let low = parts[4].parse::<f64>().ok()?;
        Some((ts, open, close, high, low))
    };

    // 9:30 第一根
    let first = trends
        .first()
        .and_then(|v| v.as_str())
        .and_then(parse_row)
        .ok_or_else(|| anyhow!("分时 首根解析失败"))?;
    let last = trends
        .last()
        .and_then(|v| v.as_str())
        .and_then(parse_row)
        .ok_or_else(|| anyhow!("分时 尾根解析失败"))?;

    let date = last.0.split_whitespace().next().unwrap_or("").to_string();

    // 扫描所有分钟：最高/最低价（取日内最高 high 与最低 low）
    let mut day_high = f64::NEG_INFINITY;
    let mut day_low = f64::INFINITY;
    // 尾盘定位：第一根 time >= 14:30 的 close 作为尾盘起始价
    let mut tail_start_close: Option<f64> = None;
    for v in trends {
        let s = match v.as_str() {
            Some(s) => s,
            None => continue,
        };
        let Some((ts, _o, close, high, low)) = parse_row(s) else {
            continue;
        };
        if high > day_high {
            day_high = high;
        }
        if low < day_low {
            day_low = low;
        }
        if tail_start_close.is_none() {
            // ts 形如 "2026-04-22 14:30"
            if let Some(hm) = ts.split_whitespace().nth(1) {
                if hm >= "14:30" {
                    tail_start_close = Some(close);
                }
            }
        }
    }
    if !day_high.is_finite() || !day_low.is_finite() {
        return Err(anyhow!("分时 高低价未找到"));
    }

    let open_pct = (first.1 / pre_close - 1.0) * 100.0;     // 首根 open
    let high_pct = (day_high / pre_close - 1.0) * 100.0;
    let low_pct = (day_low / pre_close - 1.0) * 100.0;
    let close_pct = (last.2 / pre_close - 1.0) * 100.0;
    let amplitude = (day_high - day_low) / pre_close * 100.0;
    let tail_30m_pct = match tail_start_close {
        Some(start) if start > 0.0 => (last.2 / start - 1.0) * 100.0,
        _ => 0.0,
    };

    // 形态识别
    let gap_from_high = high_pct - close_pct; // 收盘距日内最高回落幅度
    let gap_from_low = close_pct - low_pct;   // 收盘距日内最低拉升幅度
    let shape_label = if high_pct >= 2.0 && gap_from_high >= 2.0 && close_pct < high_pct * 0.5 {
        "⚠️ 冲高回落（尾盘跳水风险大）"
    } else if tail_30m_pct <= -1.5 {
        "⚠️ 尾盘跳水"
    } else if tail_30m_pct >= 1.5 && close_pct > open_pct {
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
    };

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

/// 同步包装（在已有 tokio runtime 上下文调用）
pub fn fetch_money_flow_blocking(
    client: &reqwest::Client,
    code: &str,
    lmt: usize,
) -> MoneyFlowSummary {
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return MoneyFlowSummary::default(),
    };
    let client = client.clone();
    let code = code.to_string();
    tokio::task::block_in_place(|| {
        handle.block_on(async move {
            match fetch_flow_history_async(&client, &code, lmt).await {
                Ok(s) => {
                    log::info!(
                        "[资金流] {} 取得 {} 天数据（最新 {:?}）",
                        code,
                        s.days.len(),
                        s.latest().map(|d| d.date.as_str())
                    );
                    s
                }
                Err(e) => {
                    log::warn!("[资金流] {} 抓取失败: {}", code, e);
                    MoneyFlowSummary::default()
                }
            }
        })
    })
}

pub fn fetch_intraday_shape_blocking(client: &reqwest::Client, code: &str) -> IntradayShape {
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return IntradayShape::default(),
    };
    let client = client.clone();
    let code = code.to_string();
    tokio::task::block_in_place(|| {
        handle.block_on(async move {
            match fetch_intraday_shape_async(&client, &code).await {
                Ok(s) => {
                    log::info!(
                        "[分时] {} {} open={:+.2}% high={:+.2}% close={:+.2}% tail30={:+.2}% 形态={}",
                        code, s.date, s.open_pct, s.high_pct, s.close_pct, s.tail_30m_pct, s.shape_label
                    );
                    s
                }
                Err(e) => {
                    log::warn!("[分时] {} 抓取失败: {}", code, e);
                    IntradayShape::default()
                }
            }
        })
    })
}

/// 将资金流 + 分时形态格式化为 prompt 片段
pub fn format_for_prompt(flow: &MoneyFlowSummary, shape: &IntradayShape) -> String {
    let mut out = String::new();

    // ---- 主力资金 ----
    if !flow.is_empty() {
        out.push_str("\n【主力资金流向（真实口径，单位：亿元）】\n");
        out.push_str("日期 | 涨跌幅% | 主力净流入 | 主力占比% | 超大单 | 大单\n");
        for d in flow.days.iter().rev().take(5).collect::<Vec<_>>().iter().rev() {
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
        out.push_str(&format!(
            "日内振幅: {:.2}%  尾盘30分钟涨幅: {:+.2}%\n",
            shape.amplitude, shape.tail_30m_pct
        ));
        out.push_str(&format!("日内形态: {}\n", shape.shape_label));
    }

    out
}
