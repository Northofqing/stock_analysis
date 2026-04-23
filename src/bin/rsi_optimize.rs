//! RSI 策略优化迭代工具
//!
//! 用法：
//!   cargo run --bin rsi_optimize -- list                    # 列出所有 preset
//!   cargo run --bin rsi_optimize -- run <preset>            # 跑单个 preset
//!   cargo run --bin rsi_optimize -- compare                 # 跑全部 preset 对比
//!
//! 结果会追加到 reports/rsi_optimization_log.md。

use anyhow::Result;
use chrono::Local;
use stock_analysis::data_provider::{DataFetcherManager, KlineData};
use stock_analysis::strategy::rsi::{RsiBacktest, RsiConfig, SingleRsiResult};
use stock_analysis::strategy::TradeAction;
use std::fs::OpenOptions;
use std::io::Write;

/// 股票池：覆盖不同板块与行情特征，共 30 只
const STOCK_POOL: &[(&str, &str)] = &[
    // 大盘蓝筹 / 权重
    ("600519", "贵州茅台"),
    ("601318", "中国平安"),
    ("601857", "中国石油"),
    ("600036", "招商银行"),
    ("601988", "中国银行"),
    // 券商
    ("600030", "中信证券"),
    ("601688", "华泰证券"),
    // 白酒 / 消费
    ("000858", "五粮液"),
    ("600276", "恒瑞医药"),
    ("000568", "泸州老窖"),
    // 新能源 / 光伏
    ("300750", "宁德时代"),
    ("002594", "比亚迪"),
    ("601012", "隆基绿能"),
    // 周期 / 资源
    ("600362", "江西铜业"),
    ("601899", "紫金矿业"),
    ("601398", "工商银行"),
    // 科技 / 半导体
    ("002415", "海康威视"),
    ("000725", "京东方A"),
    ("002475", "立讯精密"),
    // 家电 / 机械
    ("000333", "美的集团"),
    ("000651", "格力电器"),
    // 地产 / 建筑
    ("600048", "保利发展"),
    ("601668", "中国建筑"),
    // 中小盘题材
    ("002131", "利欧股份"),
    ("300059", "东方财富"),
    ("002352", "顺丰控股"),
    ("600887", "伊利股份"),
    ("600050", "中国联通"),
    ("601166", "兴业银行"),
    ("000001", "平安银行"),
];

struct PresetStats {
    name: String,
    config: RsiConfig,
    total_trades: usize,
    total_round_trips: usize,
    total_wins: usize,
    stocks_with_trades: usize,
    stocks_tested: usize,
    per_stock: Vec<SingleStockStat>,
    avg_return_pct: f64,
}

struct SingleStockStat {
    code: String,
    name: String,
    trades: usize,
    round_trips: usize,
    wins: usize,
    win_rate: f64,
    return_pct: f64,
}

fn compute_stock_stat(r: &SingleRsiResult) -> SingleStockStat {
    // 计算平仓交易胜率：同一代码 Buy → Sell 配对
    let mut wins = 0usize;
    let mut round_trips = 0usize;
    let mut avg_cost = 0.0f64;
    let mut total_shares = 0.0f64;
    for t in &r.trades {
        match t.action {
            TradeAction::Buy => {
                let old_val = avg_cost * total_shares;
                total_shares += t.shares;
                if total_shares > 0.0 {
                    avg_cost = (old_val + t.amount) / total_shares;
                }
            }
            TradeAction::Sell => {
                if total_shares > 0.0 {
                    round_trips += 1;
                    if t.price > avg_cost {
                        wins += 1;
                    }
                    total_shares -= t.shares;
                    if total_shares < 0.01 {
                        total_shares = 0.0;
                        avg_cost = 0.0;
                    }
                }
            }
        }
    }
    let win_rate = if round_trips > 0 {
        wins as f64 / round_trips as f64
    } else {
        0.0
    };
    let return_pct = (r.final_value - r.initial_capital) / r.initial_capital * 100.0;
    SingleStockStat {
        code: r.code.clone(),
        name: r.name.clone(),
        trades: r.trades.len(),
        round_trips,
        wins,
        win_rate,
        return_pct,
    }
}

fn fetch_pool(
    dm: &DataFetcherManager,
) -> Vec<(String, String, Vec<KlineData>)> {
    let mut out = Vec::new();
    for (code, name) in STOCK_POOL.iter() {
        match dm.get_daily_data(code, 7000) {
            Ok((data, _)) if data.len() >= 250 => {
                out.push((code.to_string(), name.to_string(), data));
            }
            Ok(d) => {
                eprintln!("  [skip] {} {} 数据不足 ({} 条)", code, name, d.0.len());
            }
            Err(e) => {
                eprintln!("  [skip] {} {} 拉取失败: {}", code, name, e);
            }
        }
    }
    out
}

fn run_preset(
    name: &str,
    config: RsiConfig,
    pool: &[(String, String, Vec<KlineData>)],
) -> Result<PresetStats> {
    let engine = RsiBacktest::new(config.clone());
    let mut per_stock = Vec::new();
    let mut total_trades = 0;
    let mut total_round_trips = 0;
    let mut total_wins = 0;
    let mut stocks_with_trades = 0;
    let mut total_return_sum = 0.0f64;

    for (code, sname, data) in pool {
        match engine.run_single(code, sname, data) {
            Ok(res) => {
                let st = compute_stock_stat(&res);
                if st.round_trips > 0 {
                    stocks_with_trades += 1;
                }
                total_trades += st.trades;
                total_round_trips += st.round_trips;
                total_wins += st.wins;
                total_return_sum += st.return_pct;
                per_stock.push(st);
            }
            Err(e) => {
                eprintln!("  [{}] 回测失败: {}", code, e);
            }
        }
    }

    let stocks_tested = pool.len();
    let avg_return = if stocks_tested > 0 {
        total_return_sum / stocks_tested as f64
    } else {
        0.0
    };

    Ok(PresetStats {
        name: name.to_string(),
        config,
        total_trades,
        total_round_trips,
        total_wins,
        stocks_with_trades,
        stocks_tested,
        per_stock,
        avg_return_pct: avg_return,
    })
}

fn format_preset_report(s: &PresetStats) -> String {
    let wr = if s.total_round_trips > 0 {
        s.total_wins as f64 / s.total_round_trips as f64 * 100.0
    } else {
        0.0
    };
    let mut out = String::new();
    out.push_str(&format!(
        "\n### Preset: `{}`  (生成时间 {})\n\n",
        s.name,
        Local::now().format("%Y-%m-%d %H:%M:%S")
    ));
    out.push_str(&format!(
        "- 股票池：{} 只（有交易 {} 只）\n",
        s.stocks_tested, s.stocks_with_trades
    ));
    out.push_str(&format!(
        "- 总交易：{} 次，平仓圆次：{}，胜：{}\n",
        s.total_trades, s.total_round_trips, s.total_wins
    ));
    out.push_str(&format!(
        "- **整体胜率**：{:.2}%  {}\n",
        wr,
        if wr >= 80.0 { "✅ 达标" } else if wr >= 70.0 { "🟡 接近" } else { "🔴 未达" }
    ));
    out.push_str(&format!("- 平均收益：{:.2}%\n", s.avg_return_pct));
    out.push_str(&format!(
        "- 关键参数：RSI({}) oversold={} overbought={} exit_level={} cooldown={} trend_filter={}(MA{}) stop={:.1}% tp={:.1}% min_hold={} require_all={} add_on_delta={}\n\n",
        s.config.rsi_period,
        s.config.oversold,
        s.config.overbought,
        s.config.exit_level,
        s.config.cooldown_bars,
        s.config.use_trend_filter,
        s.config.trend_ma_period,
        s.config.stop_loss_pct * 100.0,
        s.config.take_profit_pct * 100.0,
        s.config.min_hold_bars,
        s.config.require_all_filters,
        s.config.add_on_rsi_delta,
    ));
    out.push_str("| 代码 | 名称 | 交易次数 | 平仓次数 | 胜 | 胜率 | 收益率 |\n");
    out.push_str("|------|------|---------|---------|----|------|--------|\n");
    let mut sorted = s.per_stock.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| b.win_rate.partial_cmp(&a.win_rate).unwrap_or(std::cmp::Ordering::Equal));
    for st in sorted.iter().filter(|x| x.round_trips > 0) {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.1}% | {:+.2}% |\n",
            st.code, st.name, st.trades, st.round_trips, st.wins, st.win_rate * 100.0, st.return_pct
        ));
    }
    out.push('\n');
    out
}

fn append_to_log(text: &str) -> Result<()> {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open("reports/rsi_optimization_log.md")?;
    f.write_all(text.as_bytes())?;
    Ok(())
}

fn all_presets() -> Vec<(&'static str, RsiConfig)> {
    vec![
        ("baseline", RsiConfig::preset_baseline()),
        ("daily_v1", RsiConfig::preset_daily_v1()),
        ("daily_v2", RsiConfig::preset_daily_v2()),
        ("daily_v3_strict", RsiConfig::preset_daily_v3_strict()),
        ("daily_v4_stop_take", RsiConfig::preset_daily_v4_stop_take()),
        ("daily_v5_high_winrate", RsiConfig::preset_daily_v5_high_winrate()),
        ("daily_v6_clean", RsiConfig::preset_daily_v6_clean()),
        ("daily_v7_deeper", RsiConfig::preset_daily_v7_deeper()),
        ("daily_v8_score2", RsiConfig::preset_daily_v8_score2()),
        ("daily_v9_reversal", RsiConfig::preset_daily_v9_reversal()),
        ("daily_v10_no_stop", RsiConfig::preset_daily_v10_no_stop()),
        ("daily_v11_no_stop_rising", RsiConfig::preset_daily_v11_no_stop_rising()),
        ("daily_v12_deep_no_stop", RsiConfig::preset_daily_v12_deep_no_stop()),
        ("daily_v13_strict_rising", RsiConfig::preset_daily_v13_strict_rising()),
    ]
}

fn main() -> Result<()> {
    dotenv::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // gtimg 等 provider 使用 tokio runtime，需要在 tokio context 下运行
    let rt = tokio::runtime::Runtime::new()?;
    let _guard = rt.enter();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: cargo run --bin rsi_optimize -- <list|run <preset>|compare>");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "list" => {
            println!("可用 preset:");
            for (name, _) in all_presets() {
                println!("  - {}", name);
            }
        }
        "run" => {
            let name = args.get(2).map(|s| s.as_str()).unwrap_or("daily_v1");
            let cfg = all_presets()
                .into_iter()
                .find(|(n, _)| *n == name)
                .map(|(_, c)| c)
                .ok_or_else(|| anyhow::anyhow!("未找到 preset: {}", name))?;
            println!("📊 拉取股票池数据 …");
            let dm = DataFetcherManager::new()?;
            let pool = fetch_pool(&dm);
            println!("✓ 有效股票 {} 只\n", pool.len());
            println!("📈 跑 preset: {}", name);
            let stats = run_preset(name, cfg, &pool)?;
            let report = format_preset_report(&stats);
            println!("{}", report);
            append_to_log(&report)?;
            println!("已追加到 reports/rsi_optimization_log.md");
        }
        "compare" => {
            println!("📊 拉取股票池数据 …");
            let dm = DataFetcherManager::new()?;
            let pool = fetch_pool(&dm);
            println!("✓ 有效股票 {} 只\n", pool.len());
            let mut combined = String::new();
            combined.push_str(&format!(
                "\n## 🔄 Compare 批次（{}）\n\n",
                Local::now().format("%Y-%m-%d %H:%M:%S")
            ));
            combined.push_str("| Preset | 股票(有交易) | 总交易 | 平仓 | 胜 | 胜率 | 平均收益 |\n");
            combined.push_str("|--------|------------|--------|------|----|------|---------|\n");
            let mut details = String::new();
            for (name, cfg) in all_presets() {
                println!("📈 跑 {}", name);
                let s = run_preset(name, cfg, &pool)?;
                let wr = if s.total_round_trips > 0 {
                    s.total_wins as f64 / s.total_round_trips as f64 * 100.0
                } else {
                    0.0
                };
                combined.push_str(&format!(
                    "| {} | {}/{} | {} | {} | {} | **{:.2}%** | {:+.2}% |\n",
                    s.name,
                    s.stocks_with_trades,
                    s.stocks_tested,
                    s.total_trades,
                    s.total_round_trips,
                    s.total_wins,
                    wr,
                    s.avg_return_pct
                ));
                details.push_str(&format_preset_report(&s));
            }
            combined.push('\n');
            combined.push_str(&details);
            println!("{}", combined);
            append_to_log(&combined)?;
            println!("已追加到 reports/rsi_optimization_log.md");
        }
        other => {
            eprintln!("未知命令: {}", other);
            std::process::exit(1);
        }
    }
    Ok(())
}
