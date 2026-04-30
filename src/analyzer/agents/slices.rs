//! 领域数据切片：把行情/技术/资金/基本面/消息/宏观切成独立的 markdown 片段。
//!
//! 每个 Agent 只看自己领域的切片，避免共享同一份长上下文导致的"假分工"。

use crate::data_provider::KlineData;

/// 4 大领域 + 消息 + 宏观的数据切片
pub(crate) struct DomainSlices {
    pub basics: String,
    pub technical: String,
    pub capital: String,
    pub fundamental: String,
    pub news: Option<String>,
    pub macro_ctx: Option<String>,
    /// 行业/板块联动切片：从代码判断中类业务 + 从新闻/宏观中提取板块信号
    pub sector: String,
}

/// 从 K 线 / 额外上下文 / 新闻 / 宏观 构建领域切片。
pub(crate) fn build_slices(
    code: &str,
    name: Option<&str>,
    kline_data: &[KlineData],
    extra_context: Option<&str>,
    news_context: Option<&str>,
    macro_context: Option<&str>,
) -> DomainSlices {
    let latest = &kline_data[0];
    let closes: Vec<f64> = kline_data.iter().map(|k| k.close).collect();
    let n = closes.len();

    // ===== basics =====
    let basics = format!(
        "股票代码: {}\n股票名称: {}\n最新交易日: {}\n最新价: {:.2}\n开盘: {:.2} | 最高: {:.2} | 最低: {:.2}\n\
         成交量: {:.0} | 成交额: {:.0}\n涨跌幅: {:.2}%\n数据点数: {}\n",
        code,
        name.unwrap_or("未知"),
        latest.date,
        latest.close,
        latest.open,
        latest.high,
        latest.low,
        latest.volume,
        latest.amount,
        latest.pct_chg,
        n,
    );

    // ===== technical =====
    let technical = build_technical_slice(latest, &closes, kline_data, code);

    // ===== capital =====
    let capital = build_capital_slice(latest, &closes, kline_data, extra_context);

    // ===== fundamental =====
    let fundamental = build_fundamental_slice(latest);

    // ===== sector (从代码中推断板块属性 + 宏观/新闻提示) =====
    let sector = build_sector_slice(code, name, news_context, macro_context);

    DomainSlices {
        basics,
        technical,
        capital,
        fundamental,
        news: news_context.map(|s| s.to_string()),
        macro_ctx: macro_context.map(|s| s.to_string()),
        sector,
    }
}

fn calc_ma(closes: &[f64], period: usize) -> Option<f64> {
    if closes.len() >= period {
        Some(closes[..period].iter().sum::<f64>() / period as f64)
    } else {
        None
    }
}

fn build_technical_slice(
    latest: &KlineData,
    closes: &[f64],
    kline_data: &[KlineData],
    code: &str,
) -> String {
    let n = closes.len();
    let mut s = String::new();

    // 均线
    let ma5 = calc_ma(closes, 5);
    let ma10 = calc_ma(closes, 10);
    let ma20 = calc_ma(closes, 20);
    let ma60 = calc_ma(closes, 60);

    s.push_str("【均线系统】\n");
    if let Some(v) = ma5 {
        s.push_str(&format!("MA5: {:.2}\n", v));
    }
    if let Some(v) = ma10 {
        s.push_str(&format!("MA10: {:.2}\n", v));
    }
    if let Some(v) = ma20 {
        s.push_str(&format!("MA20: {:.2}\n", v));
    }
    if let Some(v) = ma60 {
        s.push_str(&format!("MA60: {:.2}\n", v));
    }
    if let (Some(m5), Some(m10), Some(m20)) = (ma5, ma10, ma20) {
        let alignment = if m5 > m10 && m10 > m20 {
            "多头排列 ✅ (MA5>MA10>MA20)"
        } else if m5 < m10 && m10 < m20 {
            "空头排列 ❌ (MA5<MA10<MA20)"
        } else {
            "均线粘合/交叉，趋势不明"
        };
        s.push_str(&format!("均线排列: {}\n", alignment));
    }

    // 乖离率
    if let Some(m5) = ma5 {
        if m5 > 0.0 {
            let bias = (latest.close - m5) / m5 * 100.0;
            let warn = if bias.abs() > 5.0 {
                "⚠️ 偏离过大"
            } else if bias.abs() > 2.0 {
                "注意回归"
            } else {
                "正常范围"
            };
            s.push_str(&format!("MA5乖离率: {:.2}% ({})\n", bias, warn));
        }
    }
    if let Some(m10) = ma10 {
        if m10 > 0.0 {
            s.push_str(&format!("MA10乖离率: {:.2}%\n", (latest.close - m10) / m10 * 100.0));
        }
    }
    if let Some(m20) = ma20 {
        if m20 > 0.0 {
            s.push_str(&format!("MA20乖离率: {:.2}%\n", (latest.close - m20) / m20 * 100.0));
        }
    }

    // 涨跌停识别
    let is_gem = code.starts_with("300") || code.starts_with("301");
    let is_star = code.starts_with("688");
    let is_bj = code.starts_with("8") || code.starts_with("9") || code.starts_with("4");
    let limit_pct = if is_gem || is_star {
        20.0
    } else if is_bj {
        30.0
    } else {
        10.0
    };
    if latest.pct_chg >= limit_pct - 0.3 {
        let mut consec = 1;
        for k in kline_data[1..].iter().take(10) {
            if k.pct_chg >= limit_pct - 0.3 {
                consec += 1;
            } else {
                break;
            }
        }
        if consec >= 2 {
            s.push_str(&format!(
                "【涨跌停】🚀 连续 {} 板（涨幅 {:.2}%）— 情绪推动风险陡增\n",
                consec, latest.pct_chg
            ));
        } else {
            s.push_str(&format!(
                "【涨跌停】🚀 首板涨停（涨幅 {:.2}%）— 非追高时机\n",
                latest.pct_chg
            ));
        }
    } else if latest.pct_chg <= -(limit_pct - 0.3) {
        s.push_str(&format!(
            "【涨跌停】📉 跌停（{:.2}%）— 承压严重\n",
            latest.pct_chg
        ));
    } else if latest.pct_chg >= 5.0 {
        s.push_str(&format!(
            "【涨跌停】📈 大涨 {:.2}%（未涨停）— 警惕乖离扩大\n",
            latest.pct_chg
        ));
    }

    // MACD
    if n >= 26 {
        let mut chron: Vec<f64> = closes.iter().rev().copied().collect();
        if chron.len() > 120 {
            chron = chron[chron.len() - 120..].to_vec();
        }
        let ema = |period: usize, src: &[f64]| -> Vec<f64> {
            let alpha = 2.0 / (period as f64 + 1.0);
            let mut out = Vec::with_capacity(src.len());
            let mut prev = src[0];
            out.push(prev);
            for &v in &src[1..] {
                prev = alpha * v + (1.0 - alpha) * prev;
                out.push(prev);
            }
            out
        };
        let ema12 = ema(12, &chron);
        let ema26 = ema(26, &chron);
        let diff: Vec<f64> = ema12.iter().zip(ema26.iter()).map(|(a, b)| a - b).collect();
        let dea = ema(9, &diff);
        let m = diff.len();
        let macd = 2.0 * (diff[m - 1] - dea[m - 1]);
        let sig = if diff[m - 1] > dea[m - 1] && m >= 2 && diff[m - 2] <= dea[m - 2] {
            "金叉 ✅"
        } else if diff[m - 1] < dea[m - 1] && m >= 2 && diff[m - 2] >= dea[m - 2] {
            "死叉 ❌"
        } else if diff[m - 1] > dea[m - 1] {
            "多头区间"
        } else {
            "空头区间"
        };
        s.push_str(&format!(
            "【MACD】DIFF: {:.3}, DEA: {:.3}, MACD柱: {:.3} ({})\n",
            diff[m - 1],
            dea[m - 1],
            macd,
            sig
        ));
    }

    // RSI
    if n >= 15 {
        let chron: Vec<f64> = closes.iter().rev().copied().collect();
        let mut gains = 0.0;
        let mut losses = 0.0;
        for i in 1..=14 {
            let d = chron[i] - chron[i - 1];
            if d > 0.0 {
                gains += d;
            } else {
                losses -= d;
            }
        }
        let mut avg_gain = gains / 14.0;
        let mut avg_loss = losses / 14.0;
        for i in 15..chron.len() {
            let d = chron[i] - chron[i - 1];
            let (g, l) = if d > 0.0 { (d, 0.0) } else { (0.0, -d) };
            avg_gain = (avg_gain * 13.0 + g) / 14.0;
            avg_loss = (avg_loss * 13.0 + l) / 14.0;
        }
        let rsi = if avg_loss.abs() < 1e-9 {
            100.0
        } else {
            100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
        };
        let sig = if rsi > 80.0 {
            "严重超买 🔴"
        } else if rsi > 70.0 {
            "超买"
        } else if rsi < 20.0 {
            "严重超卖 🟢"
        } else if rsi < 30.0 {
            "超卖"
        } else {
            "正常"
        };
        s.push_str(&format!("【RSI(14)】{:.2} ({})\n", rsi, sig));
    }

    // KDJ
    if n >= 9 {
        let chron: Vec<&KlineData> = kline_data.iter().rev().collect();
        let mut k = 50.0;
        let mut d = 50.0;
        let m = chron.len();
        let start = m.saturating_sub(30).max(8);
        for i in start..m {
            let ws = i.saturating_sub(8);
            let win = &chron[ws..=i];
            let hh = win.iter().map(|x| x.high).fold(f64::NEG_INFINITY, f64::max);
            let ll = win.iter().map(|x| x.low).fold(f64::INFINITY, f64::min);
            let rsv = if (hh - ll).abs() < 1e-9 {
                50.0
            } else {
                (chron[i].close - ll) / (hh - ll) * 100.0
            };
            k = 2.0 / 3.0 * k + 1.0 / 3.0 * rsv;
            d = 2.0 / 3.0 * d + 1.0 / 3.0 * k;
        }
        let j = 3.0 * k - 2.0 * d;
        let sig = if k > 80.0 && d > 80.0 {
            "超买区"
        } else if k < 20.0 && d < 20.0 {
            "超卖区"
        } else if k > d {
            "多头"
        } else {
            "空头"
        };
        s.push_str(&format!(
            "【KDJ】K: {:.2}, D: {:.2}, J: {:.2} ({})\n",
            k, d, j, sig
        ));
    }

    // 价格区间
    let week52 = n.min(250);
    if week52 >= 5 {
        let w = &kline_data[..week52];
        let h = w.iter().map(|x| x.high).fold(f64::NEG_INFINITY, f64::max);
        let l = w.iter().map(|x| x.low).fold(f64::INFINITY, f64::min);
        let pos = if (h - l).abs() > 0.001 {
            (latest.close - l) / (h - l) * 100.0
        } else {
            50.0
        };
        s.push_str(&format!(
            "【价格区间】52周高: {:.2} | 52周低: {:.2} | 当前位置: {:.1}%\n",
            h, l, pos
        ));
    }
    let q = n.min(60);
    if q >= 5 {
        let w = &kline_data[..q];
        let h = w.iter().map(|x| x.high).fold(f64::NEG_INFINITY, f64::max);
        let l = w.iter().map(|x| x.low).fold(f64::INFINITY, f64::min);
        let pos = if (h - l).abs() > 0.001 {
            (latest.close - l) / (h - l) * 100.0
        } else {
            50.0
        };
        s.push_str(&format!(
            "近季高: {:.2} | 近季低: {:.2} | 季度区间位置: {:.1}%\n",
            h, l, pos
        ));
    }

    // 近期走势
    let r = n.min(10);
    if r >= 2 {
        s.push_str("【近期走势(近10日)】\n");
        for k in &kline_data[..r] {
            s.push_str(&format!("  {} | 收 {:.2} | {:.2}%\n", k.date, k.close, k.pct_chg));
        }
        let chg5: f64 = kline_data[..n.min(5)].iter().map(|k| k.pct_chg).sum();
        s.push_str(&format!("近5日累计: {:.2}%\n", chg5));
        if r >= 10 {
            let chg10: f64 = kline_data[..10].iter().map(|k| k.pct_chg).sum();
            s.push_str(&format!("近10日累计: {:.2}%\n", chg10));
        }
        let rets: Vec<f64> = kline_data[..r].iter().map(|k| k.pct_chg).collect();
        let mean = rets.iter().sum::<f64>() / rets.len() as f64;
        let var = rets.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / rets.len() as f64;
        s.push_str(&format!("近期日波动率: {:.2}%\n", var.sqrt()));
    }

    // 布林带 + MACD 共振信号（4 条核心规则 + 反误区过滤）
    let bm = crate::strategy::detect_boll_macd_signal(kline_data);
    if bm.action != crate::strategy::BollMacdAction::None {
        s.push_str("【布林+MACD 共振信号（强约束）】\n");
        s.push_str(&format!(
            "动作: {} | 收盘 ¥{:.2} | 上轨 ¥{:.2} / 中轨 ¥{:.2} / 下轨 ¥{:.2}\n",
            bm.action.name(), bm.close, bm.upper, bm.middle, bm.lower
        ));
        s.push_str(&format!(
            "带宽 {:.2}% (5日变化 {:+.2}%) | DIF/DEA/HIST = {:.3}/{:.3}/{:.3} | 背离: {:?}\n",
            bm.band_width_pct, bm.band_change_pct,
            bm.macd_dif, bm.macd_dea, bm.macd_hist, bm.macd_div
        ));
        s.push_str(&format!("解读: {}\n", bm.reason));
        s.push_str("规则: TopSell→禁止买入；BottomBuy→可小仓<30%；UptrendStart→可加仓至60%；PreReversal→仅观察。\n");
    }

    s
}

fn build_capital_slice(
    latest: &KlineData,
    closes: &[f64],
    kline_data: &[KlineData],
    extra_context: Option<&str>,
) -> String {
    let mut s = String::new();
    let n = closes.len();

    // 量能
    s.push_str("【量能分析】\n");
    if n >= 5 {
        let avg5 = kline_data[..5].iter().map(|k| k.volume).sum::<f64>() / 5.0;
        if avg5 > 0.0 {
            let r = latest.volume / avg5;
            let st = if r > 2.0 {
                "显著放量"
            } else if r > 1.2 {
                "温和放量"
            } else if r > 0.8 {
                "量能平稳"
            } else {
                "明显缩量"
            };
            s.push_str(&format!("5日量比: {:.2} ({})\n", r, st));
        }
    }
    if n >= 10 {
        let avg10 = kline_data[..10].iter().map(|k| k.volume).sum::<f64>() / 10.0;
        if avg10 > 0.0 {
            s.push_str(&format!("10日量比: {:.2}\n", latest.volume / avg10));
        }
    }

    // 主力代理
    s.push_str("【主力资金（代理推断）】\n");
    if n >= 5 {
        let avg5 = kline_data[..5].iter().map(|k| k.volume).sum::<f64>() / 5.0;
        let r = if avg5 > 0.0 { latest.volume / avg5 } else { 1.0 };
        let mf = if r > 1.5 && latest.pct_chg > 1.0 {
            "🔥 放量上涨 — 主力介入迹象"
        } else if r > 1.5 && latest.pct_chg < -1.0 {
            "⚠️ 放量下跌 — 主力出货迹象"
        } else if r < 0.7 && latest.pct_chg > 0.5 {
            "缩量上涨 — 惜售但动能不足"
        } else if r < 0.7 && latest.pct_chg < -0.5 {
            "缩量下跌 — 抛压减弱"
        } else if r > 1.3 && latest.pct_chg.abs() < 1.0 {
            "高换手+横盘 — 筹码交换，关注突破"
        } else {
            "量价关系平稳，无明显主力动向"
        };
        s.push_str(&format!("代理判断: {}\n", mf));
    }
    if let Some(t) = latest.turnover_rate {
        s.push_str(&format!(
            "换手率: {:.2}%（>7%活跃，>15%火热）\n",
            t
        ));
    }

    // 真实资金/分时/龙虎榜/筹码（外部注入）
    if let Some(extra) = extra_context {
        if !extra.trim().is_empty() {
            s.push_str("\n【真实资金/分时/龙虎榜/筹码（实测口径）】\n");
            s.push_str(extra.trim());
            s.push('\n');
        }
    }

    s
}

fn build_fundamental_slice(latest: &KlineData) -> String {
    let mut s = String::new();

    let has_val = latest.pe_ratio.is_some()
        || latest.pb_ratio.is_some()
        || latest.market_cap.is_some()
        || latest.turnover_rate.is_some();
    if has_val {
        s.push_str("【估值与市值】\n");
        if let Some(pe) = latest.pe_ratio {
            let lvl = if pe < 0.0 {
                "亏损"
            } else if pe < 15.0 {
                "估值合理 ✅"
            } else if pe < 30.0 {
                "估值适中"
            } else {
                "估值偏高 🔴"
            };
            s.push_str(&format!("PE: {:.2} ({})\n", pe, lvl));
        }
        if let Some(pb) = latest.pb_ratio {
            let lvl = if pb < 1.0 {
                "可能被低估 ✅"
            } else if pb < 3.0 {
                "正常"
            } else {
                "偏高 🔴"
            };
            s.push_str(&format!("PB: {:.2} ({})\n", pb, lvl));
        }
        if let Some(mc) = latest.market_cap {
            let cap = if mc < 50.0 {
                "小盘股"
            } else if mc < 300.0 {
                "中盘股"
            } else if mc < 1000.0 {
                "大盘股"
            } else {
                "超大盘股"
            };
            s.push_str(&format!("总市值: {:.2}亿 ({})\n", mc, cap));
        }
        if let Some(cc) = latest.circulating_cap {
            s.push_str(&format!("流通市值: {:.2}亿\n", cc));
        }
    }

    let has_fin = latest.eps.is_some()
        || latest.roe.is_some()
        || latest.gross_margin.is_some()
        || latest.revenue_yoy.is_some();
    if has_fin {
        s.push_str("【财务指标】\n");
        if let Some(eps) = latest.eps {
            s.push_str(&format!("EPS: {:.3}元\n", eps));
        }
        if let Some(roe) = latest.roe {
            let a = if roe < 5.0 {
                "较低"
            } else if roe < 15.0 {
                "正常"
            } else if roe < 25.0 {
                "优秀"
            } else {
                "卓越"
            };
            s.push_str(&format!("ROE: {:.2}% ({})\n", roe, a));
        }
        if let Some(gm) = latest.gross_margin {
            s.push_str(&format!("毛利率: {:.2}%\n", gm));
        }
        if let Some(nm) = latest.net_margin {
            s.push_str(&format!("净利率: {:.2}%\n", nm));
        }
        if let Some(r) = latest.revenue_yoy {
            s.push_str(&format!("营收同比: {:.2}%\n", r));
        }
        if let Some(p) = latest.net_profit_yoy {
            s.push_str(&format!("净利润同比: {:.2}%\n", p));
        }
    }

    if let Some(sr) = latest.sharpe_ratio {
        s.push_str(&format!("夏普比率: {:.2}\n", sr));
    }

    if s.is_empty() {
        s.push_str("（无估值与财务数据）\n");
    }
    s
}

/// 板块/行业切片：根据代码推断市场分层 + 从新闻/宏观提取板块联动信号。
fn build_sector_slice(
    code: &str,
    name: Option<&str>,
    news_context: Option<&str>,
    macro_context: Option<&str>,
) -> String {
    let mut s = String::new();

    // 市场分层
    let market = if code.starts_with("688") {
        "科创板（高成长高波动，20% 涨跌停）"
    } else if code.starts_with("300") || code.starts_with("301") {
        "创业板（中小成长股，20% 涨跌停）"
    } else if code.starts_with("60") {
        "沪市主板（蓝筹/大盘居多，10% 涨跌停）"
    } else if code.starts_with("00") {
        "深市主板（中小市值居多，10% 涨跌停）"
    } else if code.starts_with("8") || code.starts_with("4") || code.starts_with("9") {
        "北交所（流动性较低，30% 涨跌停）"
    } else {
        "未知板块"
    };
    s.push_str(&format!("【市场分层】{}\n", market));
    if let Some(n) = name {
        s.push_str(&format!("【公司名称】{}\n", n));
        // 名称启发式行业线索
        let hints = [
            ("银行", "银行金融"),
            ("证券", "证券金融"),
            ("保险", "保险金融"),
            ("券商", "证券金融"),
            ("白酒", "食品饮料/白酒"),
            ("酒业", "食品饮料/白酒"),
            ("医药", "医药生物"),
            ("生物", "医药生物"),
            ("制药", "医药生物"),
            ("中药", "中药/医药"),
            ("汽车", "汽车整车/零部件"),
            ("电池", "新能源/电池"),
            ("锂", "新能源/锂电"),
            ("光伏", "新能源/光伏"),
            ("半导体", "半导体"),
            ("芯片", "半导体"),
            ("集成电路", "半导体"),
            ("软件", "软件/IT"),
            ("信息", "软件/IT"),
            ("传媒", "传媒/AIGC"),
            ("AI", "人工智能"),
            ("机器人", "机器人/智能制造"),
            ("地产", "房地产"),
            ("钢铁", "钢铁/周期"),
            ("有色", "有色金属/周期"),
            ("煤炭", "煤炭/能源"),
            ("电力", "公用事业/电力"),
            ("石油", "石油石化"),
            ("化工", "基础化工"),
            ("食品", "食品饮料"),
            ("家电", "家用电器"),
            ("军工", "国防军工"),
            ("航空", "航空/国防"),
            ("航天", "航天/国防"),
            ("通信", "通信"),
            ("5G", "通信/5G"),
            ("消费", "消费"),
            ("零售", "商贸零售"),
            ("农业", "农林牧渔"),
            ("种业", "农林牧渔"),
            ("猪", "农林牧渔/养殖"),
            ("水泥", "建材/周期"),
            ("建材", "建材"),
            ("旅游", "社会服务/旅游"),
            ("酒店", "社会服务"),
            ("教育", "教育"),
        ];
        let mut matched: Vec<&str> = Vec::new();
        for (kw, tag) in hints.iter() {
            if n.contains(kw) {
                matched.push(tag);
            }
        }
        if !matched.is_empty() {
            s.push_str(&format!(
                "【行业启发性标签】{}（仅基于名称推断，需结合宏观/新闻校验）\n",
                matched.join(" / ")
            ));
        }
    }

    // 从新闻/宏观中尽量挑出板块相关字段
    let mut linkage = String::new();
    if let Some(news) = news_context {
        if !news.trim().is_empty() {
            linkage.push_str("\n【新闻舆情原文】（请甄别其中是否提及本股所属板块）\n");
            linkage.push_str(news);
            linkage.push('\n');
        }
    }
    if let Some(mc) = macro_context {
        if !mc.trim().is_empty() {
            linkage.push_str("\n【宏观/板块信息原文】（请甄别其中是否点名本股所属板块）\n");
            linkage.push_str(mc);
            linkage.push('\n');
        }
    }
    if linkage.is_empty() {
        s.push_str("（无新闻/宏观可供板块联动判断）\n");
    } else {
        s.push_str(&linkage);
    }

    s
}
