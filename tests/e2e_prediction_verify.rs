//! E2E 测试: prediction 闭环真实 verify
//! 修复 R-1: 插一条 TEST_CODE_xxx 前缀的 prediction，插入 stock_daily，
//! 调 verify，断言 actual_change / hit 被正确更新。
//!
//! 之前的实现是假实现 (硬编码 0.0, false)，现在必须真实计算变化率。
//! 测试代码使用 TEST_CODE_ 前缀以通过 env_guard (AGENTS.md §2.5 隔离)。

use chrono::{Duration, Local, NaiveDate};
use diesel::RunQueryDsl;
use stock_analysis::database::DatabaseManager;

/// 初始化测试数据库（与 `database/mod.rs` 的单元测试共享 `./test_data/test.db`，
/// 因为 `DatabaseManager` 是全局 OnceCell 单例 — 并行 test binary 之间无法重置）。
/// WAL 模式 + busy_timeout=5000ms 让"database is locked"瞬态争用可重试。
fn init_test_db() {
    use std::path::PathBuf;
    std::fs::create_dir_all("./test_data").ok();
    let path = PathBuf::from("./test_data/test.db");
    let _ = DatabaseManager::init(Some(path));
}

/// 简单的"重试到成功"包装，应对 SQLite 在并行测试二进制之间的瞬态锁。
/// busy_timeout=5000ms 已经覆盖大部分场景, 这里再补 50ms x 60 次 = 3s 重试，
/// 捕获 lock-panic 后短暂 sleep 再试。
fn retry_db<F>(mut f: F)
where
    F: FnMut(),
{
    use std::thread::sleep;
    use std::time::Duration as StdDuration;
    for attempt in 0..60 {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut f));
        match result {
            Ok(()) => return,
            Err(_) if attempt < 59 => {
                sleep(StdDuration::from_millis(50));
            }
            Err(e) => std::panic::resume_unwind(e),
        }
    }
}

#[tokio::test]
async fn test_verify_predictions_writes_real_actual_change() {
    init_test_db();
    let db = DatabaseManager::get();
    let today = Local::now().format("%Y-%m-%d").to_string();
    let yesterday = (Local::now() - Duration::days(1)).format("%Y-%m-%d").to_string();

    // 清理: 删 TEST_CODE_001 残留 (单独测试, 不与其他测试共享 code)
    retry_db(|| {
        let _ = diesel::sql_query("DELETE FROM prediction_tracker WHERE stock_code = 'TEST_CODE_001'")
            .execute(&mut *db.get_conn().unwrap());
        let _ = diesel::sql_query("DELETE FROM stock_daily WHERE code = 'TEST_CODE_001'")
            .execute(&mut *db.get_conn().unwrap());
    });

    // 1. 准备: 插入 prediction (pred_date = yesterday 让 verify 找到)
    retry_db(|| {
        db.save_prediction(&yesterday, &today, Some("测试主题"), Some("TEST_CODE_001"), "看多", 75.0, Some("unit test"))
            .expect("save_prediction 应成功");
    });

    // 2. 准备: stock_daily 昨日 close=10, 今日 close=11
    let yesterday_date = NaiveDate::parse_from_str(&yesterday, "%Y-%m-%d").unwrap();
    let today_date = NaiveDate::parse_from_str(&today, "%Y-%m-%d").unwrap();
    retry_db(|| {
        db.save_daily_record(
            "TEST_CODE_001", yesterday_date,
            Some(9.5), Some(10.2), Some(9.3), Some(10.0), Some(1_000_000.0), Some(10_000_000.0),
            Some(0.0), Some(10.0), Some(10.0), Some(10.0), Some(1.0), Some("TestSource"),
        ).expect("save yesterday daily 失败");
        db.save_daily_record(
            "TEST_CODE_001", today_date,
            Some(10.5), Some(11.2), Some(10.3), Some(11.0), Some(1_500_000.0), Some(16_500_000.0),
            Some(10.0), Some(10.5), Some(10.5), Some(10.5), Some(1.5), Some("TestSource"),
        ).expect("save today daily 失败");
    });

    // 3. 执行: 真实 verify
    stock_analysis::monitor::prediction::verify_predictions().await;

    // 4. 断言: hit=1, actual_change≈10%
    let row = db.get_prediction_by_code_date("TEST_CODE_001", &yesterday)
        .expect("应能查回 prediction");
    assert_eq!(row.hit, Some(1), "看多+次日涨 10% → hit=1, row={:?}", row);
    let actual = row.actual_change.expect("actual_change 必须有值");
    assert!((actual - 10.0).abs() < 0.5,
        "actual_change 应≈10.0, 实际={}", actual);

    // 清理
    retry_db(|| {
        let _ = diesel::sql_query("DELETE FROM prediction_tracker WHERE stock_code = 'TEST_CODE_001'")
            .execute(&mut *db.get_conn().unwrap());
        let _ = diesel::sql_query("DELETE FROM stock_daily WHERE code = 'TEST_CODE_001'")
            .execute(&mut *db.get_conn().unwrap());
    });
}

#[tokio::test]
async fn test_verify_predictions_miss_for_bearish_prediction_on_up_day() {
    init_test_db();
    let db = DatabaseManager::get();
    let today = Local::now().format("%Y-%m-%d").to_string();
    let yesterday = (Local::now() - Duration::days(1)).format("%Y-%m-%d").to_string();

    retry_db(|| {
        let _ = diesel::sql_query("DELETE FROM prediction_tracker WHERE stock_code = 'TEST_CODE_002'")
            .execute(&mut *db.get_conn().unwrap());
        let _ = diesel::sql_query("DELETE FROM stock_daily WHERE code = 'TEST_CODE_002'")
            .execute(&mut *db.get_conn().unwrap());
    });

    // 1. 看空预测
    retry_db(|| {
        db.save_prediction(&yesterday, &today, Some("测试主题"), Some("TEST_CODE_002"), "看空", 75.0, Some("unit test"))
            .expect("save_prediction 应成功");
    });

    // 2. 昨日 10 → 今日 11 (实际涨)
    let yesterday_date = NaiveDate::parse_from_str(&yesterday, "%Y-%m-%d").unwrap();
    let today_date = NaiveDate::parse_from_str(&today, "%Y-%m-%d").unwrap();
    retry_db(|| {
        db.save_daily_record("TEST_CODE_002", yesterday_date,
            Some(9.5), Some(10.2), Some(9.3), Some(10.0), Some(1_000_000.0), Some(10_000_000.0),
            Some(0.0), None, None, None, None, Some("TestSource"),
        ).expect("save 失败");
        db.save_daily_record("TEST_CODE_002", today_date,
            Some(10.5), Some(11.2), Some(10.3), Some(11.0), Some(1_500_000.0), Some(16_500_000.0),
            Some(10.0), None, None, None, None, Some("TestSource"),
        ).expect("save 失败");
    });

    // 3. 执行
    stock_analysis::monitor::prediction::verify_predictions().await;

    // 4. 断言: 看空+次日涨 → hit=0
    let row = db.get_prediction_by_code_date("TEST_CODE_002", &yesterday)
        .expect("应能查回");
    assert_eq!(row.hit, Some(0), "看空+次日涨 → hit=0 (未命中), row={:?}", row);

    // 清理
    retry_db(|| {
        let _ = diesel::sql_query("DELETE FROM prediction_tracker WHERE stock_code = 'TEST_CODE_002'")
            .execute(&mut *db.get_conn().unwrap());
        let _ = diesel::sql_query("DELETE FROM stock_daily WHERE code = 'TEST_CODE_002'")
            .execute(&mut *db.get_conn().unwrap());
    });
}