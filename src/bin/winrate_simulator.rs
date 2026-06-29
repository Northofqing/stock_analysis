//! Winrate Simulator — 基于 prediction_tracker 已 verify 数据, 评估
//! "如果关停 X 主题, 胜率会变成多少?"
//!
//! 用法:
//!   STOCK_DB=data/stock_analysis.db cargo run --bin winrate_simulator
//!   STOCK_DB=data/stock_analysis.db cargo run --bin winrate_simulator -- --blacklist 半导体-制造代工,AI算力,AI硬件-MLCC
//!   STOCK_DB=data/stock_analysis.db cargo run --bin winrate_simulator -- --days 30
//!
//! 决策:
//!   默认黑名单 = 7 个 0% 主题 (BR-006 关停的清单).
//!   输入: 黑名单 + 数据范围 (默认全库)
//!   输出: 调整前后 (主题级 + 全局) 胜率 + 推送数 + 命中数
//!
//! 设计: 只读不写. 与 backfill_predictions / backfill_daily 保持一致风格 —
//!       直接调 lib 公共 API, 不触发 monitor pipeline.

use std::env;
use std::path::PathBuf;
use chrono::{Local, Duration};
use diesel::sql_types::{Text, Integer};
use diesel::{QueryableByName, RunQueryDsl};
use stock_analysis::database::DatabaseManager;

/// BR-006 默认黑名单 + 主题 priority + generic 标记: 从 config/chain_rules.toml 动态读取.
fn load_br006_default_blacklist() -> (Vec<String>, std::collections::HashMap<String, u32>, std::collections::HashSet<String>) {
    let toml_text = include_str!("../../config/chain_rules.toml");
    #[derive(serde::Deserialize)]
    struct Rule {
        chain: String,
        priority: u32,
        #[serde(default = "default_true")]
        enabled: bool,
        #[serde(default)]
        generic: bool,
    }
    #[derive(serde::Deserialize)]
    struct RulesFile {
        rules: Vec<Rule>,
    }
    fn default_true() -> bool { true }
    match toml::from_str::<RulesFile>(toml_text) {
        Ok(file) => {
            let blacklist: Vec<String> = file.rules.iter()
                .filter(|r| !r.enabled).map(|r| r.chain.clone()).collect();
            let priorities: std::collections::HashMap<String, u32> = file.rules.iter()
                .map(|r| (r.chain.clone(), r.priority)).collect();
            let generics: std::collections::HashSet<String> = file.rules.iter()
                .filter(|r| r.generic).map(|r| r.chain.clone()).collect();
            (blacklist, priorities, generics)
        }
        Err(e) => {
            // 修复 I-10 (2026-06-29 codex review): 原 silently fallback 到空 list,
            // operator 看到的"全库 0% / 全库 100% 主题"会误以为没数据, 违反 §2.2.
            // 改为 fail-fast exit 2 让 CI / cron 立刻知道配置出问题.
            eprintln!("[winrate_simulator] 解析 chain_rules.toml 失败: {}", e);
            eprintln!("[winrate_simulator] 拒绝输出错误结果, exit 2 (修复 I-10)");
            std::process::exit(2);
        }
    }
}

#[derive(Debug, Clone, QueryableByName)]
struct ThemeRow {
    #[diesel(sql_type = Text)]
    theme_name: String,
    #[diesel(sql_type = Integer)]
    total: i32,
    #[diesel(sql_type = Integer)]
    hits: i32,
}

#[derive(Debug, Clone, QueryableByName)]
struct GlobalRow {
    #[diesel(sql_type = Integer)]
    total: i32,
    #[diesel(sql_type = Integer)]
    hits: i32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 解析参数
    let args: Vec<String> = env::args().collect();
    let (default_blacklist, theme_priorities, theme_generics) = load_br006_default_blacklist();
    let mut blacklist: Vec<String> = default_blacklist;
    let mut days: Option<i64> = None;
    let mut explicit_min_samples: usize = 5; // 主题级最小样本, <此值不展示

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--blacklist" | "-b" => {
                i += 1;
                if let Some(list) = args.get(i) {
                    blacklist = list.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                }
            }
            "--days" | "-d" => {
                i += 1;
                if let Some(d) = args.get(i).and_then(|s| s.parse::<i64>().ok()) {
                    days = Some(d);
                }
            }
            "--min-samples" => {
                i += 1;
                if let Some(n) = args.get(i).and_then(|s| s.parse::<usize>().ok()) {
                    explicit_min_samples = n;
                }
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            _ => {
                eprintln!("[winrate_simulator] 未知参数: {}", args[i]);
                print_help();
                std::process::exit(2);
            }
        }
        i += 1;
    }

    // 2. 初始化 DB
    let db_path = env::var("STOCK_DB").ok().map(PathBuf::from);
    let _ = DatabaseManager::init(db_path.clone());
    let db = DatabaseManager::get();
    let mut conn = db.get_conn()?;

    // 3. 构造 SQL 时间过滤
    let since_clause = match days {
        Some(d) => {
            let since = (Local::now() - Duration::days(d)).format("%Y-%m-%d").to_string();
            format!(" AND pred_date >= '{}' ", since)
        }
        None => String::new(),
    };
    let label_days = match days {
        Some(d) => format!("最近 {} 天", d),
        None => "全库".to_string(),
    };

    // 4. 拉主题级数据
    let theme_sql = format!(
        "SELECT theme_name, COUNT(*) as total, SUM(CASE WHEN hit = 1 THEN 1 ELSE 0 END) as hits
         FROM prediction_tracker
         WHERE hit IS NOT NULL AND theme_name != '' {sc}
         GROUP BY theme_name
         ORDER BY total DESC",
        sc = since_clause,
    );
    let theme_rows: Vec<ThemeRow> = diesel::sql_query(theme_sql).load(&mut conn)?;

    // 5. 全局: 调整前
    let global_sql = format!(
        "SELECT COUNT(*) as total, SUM(CASE WHEN hit = 1 THEN 1 ELSE 0 END) as hits
         FROM prediction_tracker
         WHERE hit IS NOT NULL AND theme_name != '' {sc}",
        sc = since_clause,
    );
    let pre: GlobalRow = diesel::sql_query(global_sql).get_result(&mut conn)?;

    // 6. 全局: 调整后 (按黑名单过滤)
    let blacklist_escaped: Vec<String> = blacklist.iter().map(|b| format!("'{}'", b.replace('\'', "''"))).collect();
    let blacklist_in = blacklist_escaped.join(",");
    let post_sql = format!(
        "SELECT COUNT(*) as total, SUM(CASE WHEN hit = 1 THEN 1 ELSE 0 END) as hits
         FROM prediction_tracker
         WHERE hit IS NOT NULL AND theme_name != '' AND theme_name NOT IN ({bl}) {sc}",
        bl = blacklist_in, sc = since_clause,
    );
    let post: GlobalRow = diesel::sql_query(post_sql).get_result(&mut conn)?;

    // 7. 输出
    println!("═══════════════════════════════════════════════════════════════");
    println!(" Winrate Simulator — 评估关停黑名单主题对全局胜率的影响");
    println!(" 数据范围: {}", label_days);
    println!(" 黑名单 ({} 个): {}", blacklist.len(), blacklist.join(", "));
    println!("═══════════════════════════════════════════════════════════════");
    println!();
    println!("【全局胜率】");
    print_global("调整前 (全量)", &pre);
    print_global("调整后 (剔除黑名单)", &post);
    print_delta(&pre, &post);
    println!();

    println!("【主题级明细】 (min-samples = {})", explicit_min_samples);
    println!("{:<24} {:>8} {:>8} {:>8} {:>10}  {}", "主题", "推送", "命中", "未中", "胜率", "建议");
    println!("{}", "─".repeat(80));
    let blacklist_set: std::collections::HashSet<&String> = blacklist.iter().collect();
    let mut theme_summaries: Vec<(String, i32, i32, f64)> = Vec::new();
    for r in &theme_rows {
        let total = r.total;
        let hits = r.hits;
        let losses = total - hits;
        let rate = if total > 0 { hits as f64 / total as f64 * 100.0 } else { 0.0 };
        theme_summaries.push((r.theme_name.clone(), total, hits, rate));
        if total < explicit_min_samples as i32 { continue; }
        let recommendation = if blacklist_set.contains(&r.theme_name) {
            "关停中 (BR-006)"
        } else if rate < 5.0 {
            "考虑关停"
        } else if rate < 20.0 {
            "收紧"
        } else if rate >= 30.0 {
            "加权"
        } else {
            "维持"
        };
        println!("{:<24} {:>8} {:>8} {:>8} {:>9.1}%  {}", r.theme_name, total, hits, losses, rate, recommendation);
    }
    println!();

    // 8. 决策建议
    println!("【决策建议】");
    let pre_rate = rate(&pre);
    let post_rate = rate(&post);
    let delta = post_rate - pre_rate;
    if delta.abs() < 1.0 {
        println!("  当前黑名单对全局胜率影响 < 1pp, 继续观察.");
    } else if delta > 0.0 {
        println!("  关停黑名单后全局胜率 +{:.1}pp ({:.1}% → {:.1}%), 推荐保留关停.", delta, pre_rate, post_rate);
    } else {
        println!("  关停黑名单后全局胜率 {:.1}pp 下降 — 黑名单可能错伤, 复核清单.", delta);
    }
    let low_perf: Vec<&(String, i32, i32, f64)> = theme_summaries.iter()
        .filter(|(name, total, _, rate)| !blacklist_set.contains(name) && *total >= explicit_min_samples as i32 && *rate < 5.0)
        .collect();
    if !low_perf.is_empty() {
        println!("  以下未关停主题胜率 < 5%, 建议下次评估纳入黑名单:");
        for (name, total, _, rate) in &low_perf {
            println!("    - {} ({} 推送, {:.1}%)", name, total, rate);
        }
    }
    let high_perf: Vec<&(String, i32, i32, f64)> = theme_summaries.iter()
        .filter(|(name, total, _, rate)| !blacklist_set.contains(name) && *total >= explicit_min_samples as i32 && *rate >= 30.0)
        .collect();
    // 修复 simulator false positive: 区分"已加权" (priority >= 80) vs "真正未加权"
    const WEIGHTED_PRIORITY_THRESHOLD: u32 = 80;
    let weighted: Vec<&&(String, i32, i32, f64)> = high_perf.iter()
        .filter(|(name, _, _, _)| theme_priorities.get(name.as_str()).copied().unwrap_or(0) >= WEIGHTED_PRIORITY_THRESHOLD)
        .collect();
    let unweighted: Vec<&&(String, i32, i32, f64)> = high_perf.iter()
        .filter(|(name, _, _, _)| theme_priorities.get(name.as_str()).copied().unwrap_or(0) < WEIGHTED_PRIORITY_THRESHOLD
            && !theme_generics.contains(name.as_str()))
        .collect();
    if !weighted.is_empty() {
        println!("  以下 ≥ 30% 主题已加权 (priority ≥ {}), 无需再调:", WEIGHTED_PRIORITY_THRESHOLD);
        for &&(name, total, _, rate) in &weighted {
            let p = theme_priorities.get(name.as_str()).copied().unwrap_or(0);
            println!("    - {} [priority {}] ({} 推送, {:.1}%)", name, p, total, rate);
        }
    }
    if !unweighted.is_empty() {
        println!("  以下 ≥ 30% 主题未充分加权 (priority < {}), 建议下次评估提权:", WEIGHTED_PRIORITY_THRESHOLD);
        for &&(name, total, _, rate) in &unweighted {
            let p = theme_priorities.get(name.as_str()).copied().unwrap_or(0);
            println!("    - {} [priority {}] ({} 推送, {:.1}%)", name, p, total, rate);
        }
    }

    // 修复 F17 (2026-06-29 codex review): dynamic priority 公式化建议.
    // 解决 v9.3 commit 480b74f 批量 priority=100 注入破坏 simulator "已加权/未加权" 提示能力问题.
    // 公式: dynamic_priority = winrate × log(samples + 1) × scale
    //   - winrate ∈ [0, 100] 真实胜率
    //   - log(samples + 1) ∈ [0.7, ~3.7] 样本量加权 (1 推送 → 0.7, 100 推送 → 2.3, 1000 → 3.0)
    //   - scale = 25 让 winrate=30% samples=20 → 75, winrate=50% samples=100 → 115 (clamp 100)
    // 输出推荐 priority, operator 手动复制到 chain_rules.toml.
    const PRIORITY_SCALE: f64 = 25.0;
    println!("【F17 dynamic_priority 推荐】 (修复 v9.3 批量 priority=100 注入破坏提示)");
    println!("  公式: dynamic_priority = winrate × log(samples + 1) × {}, clamp [0, 100]", PRIORITY_SCALE);
    println!("  {:<24} {:>8} {:>8} {:>10} {:>10} {:>10}", "主题", "胜率", "样本", "log加权", "dyn_prior", "static");
    println!("  {}", "─".repeat(80));
    let mut dyn_recommendations: Vec<(String, f64, f64, u32)> = Vec::new();
    for (name, total, _, rate) in &theme_summaries {
        if *total < explicit_min_samples as i32 { continue; }
        let log_weight = ((*total as f64) + 1.0).ln();
        let dyn_p = ((*rate / 100.0) * log_weight * PRIORITY_SCALE).clamp(0.0, 100.0);
        let static_p = theme_priorities.get(name.as_str()).copied().unwrap_or(0);
        // 只输出有意义的: dyn vs static 差 > 15 或 dyn > 50 但 static < 80
        if (dyn_p - static_p as f64).abs() > 15.0 || (dyn_p > 50.0 && static_p < 80) {
            println!("  {:<24} {:>7.1}% {:>8} {:>9.2} {:>9.1} {:>10}",
                name, rate, total, log_weight, dyn_p, static_p);
            dyn_recommendations.push((name.clone(), dyn_p, *rate, static_p));
        }
    }
    if !dyn_recommendations.is_empty() {
        println!("\n  建议: 把上述 dyn_prior 复制到 config/chain_rules.toml 的 priority 字段.");
        println!("  注意: dyn_prior 只反映历史胜率, 不包含 forward-looking 因子 (市场风格切换 / 政策催化).");
        println!("  使用 AGENTS §2.9 边界证明模板: dyn_prior={} ± {} (95% CI), 与样本量={}",
            "?", "?", "?");
    }
    println!();
    println!("═══════════════════════════════════════════════════════════════");
    Ok(())
}

fn print_global(label: &str, row: &GlobalRow) {
    let rate = rate(row);
    let losses = row.total - row.hits;
    println!(
        "  {:<24} 推送 {:>4}  命中 {:>4}  未中 {:>4}  胜率 {:>5.1}%",
        label, row.total, row.hits, losses, rate
    );
}

fn print_delta(pre: &GlobalRow, post: &GlobalRow) {
    let pre_rate = rate(pre);
    let post_rate = rate(post);
    let delta = post_rate - pre_rate;
    let removed = pre.total - post.total;
    let removed_hits = pre.hits - post.hits;
    println!(
        "  {:<24} 推送 Δ {:>+4}  命中 Δ {:>+4}  胜率 Δ {:>+5.1}pp",
        "差值 (后-前)", -removed, -removed_hits, delta
    );
}

fn rate(row: &GlobalRow) -> f64 {
    if row.total > 0 { row.hits as f64 / row.total as f64 * 100.0 } else { 0.0 }
}

fn print_help() {
    println!("用法: STOCK_DB=... cargo run --bin winrate_simulator -- [选项]");
    println!();
    println!("选项:");
    println!("  -b, --blacklist <themes>   逗号分隔黑名单主题 (默认: BR-006 关停的 7 个)");
    println!("  -d, --days <N>             仅看最近 N 天 (默认: 全库)");
    println!("      --min-samples <N>      主题级最小样本数 (默认: 5, <此值不展示)");
    println!("  -h, --help                 显示本帮助");
    println!();
    println!("示例:");
    println!("  # 看默认 BR-006 黑名单的影响 (全库)");
    println!("  STOCK_DB=data/stock_analysis.db cargo run --bin winrate_simulator");
    println!();
    println!("  # 评估再加 1 个主题的影响");
    println!("  STOCK_DB=data/stock_analysis.db cargo run --bin winrate_simulator -- \\");
    println!("    --blacklist 'AI硬件-液冷,AI硬件-CPO,AI硬件-MLCC,AI算力,Rubin,半导体-制造代工,创新药-CXO,稀有金属'");
}
