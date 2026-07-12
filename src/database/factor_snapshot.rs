//! 多因子日频快照 DAO
//!
//! 修复：QUANT_ANALYST_REVIEW §1.5
//! 原 bug：多因子回测承认"用切片末日截面因子"，等于用未来信息回测。
//!
//! 设计：
//! - `factor_snapshot` 表存每个 (code, snapshot_date) 的 PE/PB/ROE/市值/换手率
//! - 多因子回测每天调 `get_as_of(code, today)` 只取 ≤ today 的快照
//! - 快照缺失时返回 None，不静默回退到末日
//!
//! 关键不变量：`get_as_of` 严格 `<= as_of`，绝不返回未来快照。

use diesel::prelude::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Factors {
    pub pe_ttm: Option<f64>,
    pub pb: Option<f64>,
    pub roe: Option<f64>,
    pub market_cap: Option<f64>,
    pub turnover_rate: Option<f64>,
}

/// Diesel 行结构
#[derive(QueryableByName, Debug, Clone)]
pub struct FactorSnapshotRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub snapshot_date: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub pe_ttm: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub pb: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub roe: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub market_cap: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub turnover_rate: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub source: Option<String>,
}

/// 因子快照 DAO（包装 Diesel 连接）
pub struct FactorSnapshotDao<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> FactorSnapshotDao<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    /// 写入或替换某 (code, snapshot_date) 的因子
    pub fn upsert(
        &mut self,
        code: &str,
        snapshot_date: &str,
        pe_ttm: Option<f64>,
        pb: Option<f64>,
        roe: Option<f64>,
        market_cap: Option<f64>,
        turnover_rate: Option<f64>,
        source: &str,
    ) -> Result<(), diesel::result::Error> {
        use diesel::RunQueryDsl;
        diesel::sql_query(
            "INSERT OR REPLACE INTO factor_snapshot \
             (code, snapshot_date, pe_ttm, pb, roe, market_cap, turnover_rate, source, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now', 'localtime'))",
        )
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(snapshot_date)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(pe_ttm)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(pb)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(roe)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(market_cap)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(turnover_rate)
        .bind::<diesel::sql_types::Text, _>(source)
        .execute(self.conn)?;
        Ok(())
    }

    /// Point-in-time 查询：取 ≤ as_of 的最近一次快照
    /// **关键不变量**：严格 `<= as_of`，禁止 look-ahead
    pub fn get_as_of(
        &mut self,
        code: &str,
        as_of: &str,
    ) -> Result<Option<FactorSnapshotRow>, diesel::result::Error> {
        use diesel::RunQueryDsl;
        let row: Option<FactorSnapshotRow> = diesel::sql_query(
            "SELECT code, snapshot_date, pe_ttm, pb, roe, market_cap, turnover_rate, source \
             FROM factor_snapshot \
             WHERE code = ? AND snapshot_date <= ? \
             ORDER BY snapshot_date DESC LIMIT 1",
        )
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(as_of)
        .get_result(self.conn)
        .optional()?;
        Ok(row)
    }

    /// 创建 factor_snapshot 表（迁移入口）
    pub fn create_table(&mut self) -> Result<(), diesel::result::Error> {
        use diesel::RunQueryDsl;
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS factor_snapshot (
                code TEXT NOT NULL,
                snapshot_date TEXT NOT NULL,
                pe_ttm REAL,
                pb REAL,
                roe REAL,
                market_cap REAL,
                turnover_rate REAL,
                source TEXT,
                created_at TEXT NOT NULL,
                PRIMARY KEY (code, snapshot_date)
            )
            "#,
        )
        .execute(self.conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_factor_snapshot_date ON factor_snapshot(snapshot_date)",
        )
        .execute(self.conn)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_conn() -> SqliteConnection {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp = std::env::temp_dir().join(format!(
            "factor_snapshot_test_{}_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n,
        ));
        let path = temp.to_str().unwrap();
        let mut conn = SqliteConnection::establish(path).expect("failed to open temp sqlite");
        let mut dao = FactorSnapshotDao::new(&mut conn);
        dao.create_table().expect("create_table failed");
        std::mem::forget(temp);
        conn
    }

    #[test]
    fn save_and_query_factors_by_date() {
        let mut conn = in_memory_conn();
        {
            let mut dao = FactorSnapshotDao::new(&mut conn);
            dao.upsert(
                "600000",
                "2026-06-01",
                Some(12.5),
                Some(1.2),
                Some(0.18),
                Some(1e10),
                Some(0.02),
                "test",
            )
            .unwrap();
            dao.upsert(
                "600000",
                "2026-06-02",
                Some(13.0),
                Some(1.3),
                Some(0.18),
                Some(1.01e10),
                Some(0.025),
                "test",
            )
            .unwrap();
        }
        {
            let mut dao = FactorSnapshotDao::new(&mut conn);
            let f = dao.get_as_of("600000", "2026-06-01").unwrap().unwrap();
            assert!((f.pe_ttm.unwrap() - 12.5).abs() < 1e-6);
        }
        {
            let mut dao = FactorSnapshotDao::new(&mut conn);
            let f = dao.get_as_of("600000", "2026-06-02").unwrap().unwrap();
            assert!((f.pe_ttm.unwrap() - 13.0).abs() < 1e-6);
        }
    }

    #[test]
    fn returns_latest_known_when_no_exact_match() {
        let mut conn = in_memory_conn();
        {
            let mut dao = FactorSnapshotDao::new(&mut conn);
            dao.upsert(
                "600000",
                "2026-06-01",
                Some(12.5),
                None,
                None,
                None,
                None,
                "test",
            )
            .unwrap();
        }
        {
            let mut dao = FactorSnapshotDao::new(&mut conn);
            // 2026-06-03 还没有快照, 应回退到 2026-06-01
            let f = dao.get_as_of("600000", "2026-06-03").unwrap().unwrap();
            assert!((f.pe_ttm.unwrap() - 12.5).abs() < 1e-6);
        }
    }

    #[test]
    fn no_lookahead_filter() {
        let mut conn = in_memory_conn();
        {
            let mut dao = FactorSnapshotDao::new(&mut conn);
            dao.upsert(
                "600000",
                "2026-06-01",
                Some(12.5),
                None,
                None,
                None,
                None,
                "test",
            )
            .unwrap();
            // 事后填入的"未来"快照
            dao.upsert(
                "600000",
                "2026-06-10",
                Some(99.9),
                None,
                None,
                None,
                None,
                "test",
            )
            .unwrap();
        }
        {
            let mut dao = FactorSnapshotDao::new(&mut conn);
            // 在 2026-06-05 查询, 应得到 06-01 的值, 绝不能是 06-10
            let f = dao.get_as_of("600000", "2026-06-05").unwrap().unwrap();
            assert!((f.pe_ttm.unwrap() - 12.5).abs() < 1e-6);
        }
    }

    #[test]
    fn missing_stock_returns_none() {
        let mut conn = in_memory_conn();
        let mut dao = FactorSnapshotDao::new(&mut conn);
        let f = dao.get_as_of("999999", "2026-06-01").unwrap();
        assert!(f.is_none());
    }

    #[test]
    fn no_snapshots_at_all() {
        let mut conn = in_memory_conn();
        let mut dao = FactorSnapshotDao::new(&mut conn);
        let f = dao.get_as_of("600000", "2026-06-01").unwrap();
        assert!(f.is_none());
    }
}
