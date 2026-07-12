//! BR-001: 同一只票近 3 个日历日最多推送 1 次 (实际按日历日计算, 非交易日)
//!
//! 修复 R-4: discover() 必须过滤近 3 日内已被 push 过的标的
//! 测试策略: 插一条 TEST_CODE_002 的 prediction (今天) → 调 discover() 命中同代码 → 断言被过滤
//! 测试代码使用 TEST_CODE_ 前缀以通过 env_guard (AGENTS.md §2.5 隔离)

use chrono::Local;
use diesel::RunQueryDsl;
use std::collections::HashSet;
use std::path::PathBuf;
use stock_analysis::database::DatabaseManager;

fn init_test_db() {
    std::fs::create_dir_all("./test_data").ok();
    let path = PathBuf::from("./test_data/test.db");
    let _ = DatabaseManager::init(Some(path));
}

#[test]
fn test_discover_dedups_recently_pushed_stocks() {
    init_test_db();
    let db = DatabaseManager::get();
    let today = Local::now().format("%Y-%m-%d").to_string();

    // 准备: 把 TEST_CODE_002 标记为"今天已推"
    let _ = db.save_prediction_legacy(
        &today,
        &today,
        Some("T"),
        Some("TEST_CODE_002"),
        "看多",
        60.0,
        Some("setup"),
    );

    // 准备: hits 包含 TEST_CODE_002
    use stock_analysis::opportunity::chain_mapper::{ChainHit, ChainSource, StockInfo};
    let hits = vec![ChainHit {
        chain: "测试链".into(),
        keywords: vec!["T".into()],
        logic: "test".into(),
        stocks: vec![StockInfo {
            code: "TEST_CODE_002".into(),
            name: "测试2".into(),
            change_pct: 0.0,
            vol_ratio: 1.0,
        }],
        source: ChainSource::Rule,
        board_keyword: "".into(),
        fund_flow_pct: None,
        board_code: None,
        board_change_pct: None,
    }];

    let candidates = stock_analysis::opportunity::discover::discover(&hits, &[], 5);
    let codes: HashSet<String> = candidates.iter().map(|c| c.code.clone()).collect();
    assert!(
        !codes.contains("TEST_CODE_002"),
        "TEST_CODE_002 今日已推过, 应被 BR-001 去重"
    );

    // 清理: 删 TEST_CODE_002 残留 (prediction_tracker 是 OnceCell 全局,
    // 后续测试运行会被污染 — 见 Codex 第二轮 review F4)
    let _ = diesel::sql_query("DELETE FROM prediction_tracker WHERE stock_code = 'TEST_CODE_002'")
        .execute(&mut *db.get_conn().unwrap());
}
