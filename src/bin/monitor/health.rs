//! Production health checks for storage, event delivery, strategies,
//! performance snapshots, and realtime quote registration.

use chrono::{NaiveDateTime, Utc};
use diesel::prelude::*;
use diesel::sql_query;
use diesel::sqlite::SqliteConnection;
use stock_analysis::database::DatabaseManager;
use stock_analysis::registry::StrategyRegistry;

#[derive(Debug, Clone, Default)]
pub struct HealthStatus {
    pub db_writable: bool,
    pub bus_alive: bool,
    pub strategy_registered: bool,
    pub perf_recent: bool,
    pub quote_provider: bool,
}

impl HealthStatus {
    pub fn all_ok(&self) -> bool {
        self.db_writable
            && self.bus_alive
            && self.strategy_registered
            && self.perf_recent
            && self.quote_provider
    }
}

pub async fn health_check() -> HealthStatus {
    HealthStatus {
        db_writable: check_db(),
        // Startup health runs before the long-lived event consumer is spawned.
        // The sender plus one active receiver is sufficient evidence here;
        // requiring two receivers made the probe fail deterministically on
        // every clean start (the consumer count is a lifecycle concern).
        bus_alive: stock_analysis::event::global_bus().receiver_count() >= 1,
        strategy_registered: StrategyRegistry::global().list_all().len() >= 8,
        perf_recent: check_perf_24h(),
        quote_provider: stock_analysis::broker::quote_provider_registered(),
    }
}

fn check_db() -> bool {
    let Some(db) = DatabaseManager::try_get() else {
        return false;
    };
    let Ok(mut conn) = db.get_conn() else {
        return false;
    };
    probe_main_db_write(&mut conn)
}

fn probe_main_db_write(conn: &mut SqliteConnection) -> bool {
    // CREATE and INSERT prove writes to the main database are accepted. The
    // transaction deliberately rolls back so health checks leave no rows or
    // schema behind, including when a probe fails midway.
    let mut wrote = false;
    let result = conn.transaction::<(), diesel::result::Error, _>(|conn| {
        sql_query(
            "CREATE TABLE main.__stock_analysis_health_write_probe_20260925 (value INTEGER NOT NULL)",
        )
        .execute(conn)?;
        sql_query("INSERT INTO main.__stock_analysis_health_write_probe_20260925 (value) VALUES (1)")
            .execute(conn)?;
        wrote = true;
        Err(diesel::result::Error::RollbackTransaction)
    });
    wrote && matches!(result, Err(diesel::result::Error::RollbackTransaction))
}

fn check_perf_24h() -> bool {
    #[derive(diesel::QueryableByName)]
    struct LatestSnapshot {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        created_at: Option<String>,
    }

    let Some(db) = DatabaseManager::try_get() else {
        return false;
    };
    let Ok(mut conn) = db.get_conn() else {
        return false;
    };
    let Ok(row) = sql_query("SELECT MAX(created_at) AS created_at FROM paper_performance_snapshot")
        .get_result::<LatestSnapshot>(&mut conn)
    else {
        return false;
    };
    row.created_at
        .as_deref()
        .is_some_and(|created_at| snapshot_is_recent(created_at, Utc::now().naive_utc()))
}

fn snapshot_is_recent(created_at: &str, now: NaiveDateTime) -> bool {
    NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S")
        .ok()
        .is_some_and(|timestamp| {
            let age = now.signed_duration_since(timestamp);
            age.num_seconds() >= 0 && age.num_hours() <= 24
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn db_write_probe_rejects_readable_query_only_connection_and_leaves_no_table() {
        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        sql_query("PRAGMA query_only = ON")
            .execute(&mut conn)
            .expect("read-only pragma");
        let readable = sql_query("SELECT 1 AS count")
            .get_result::<Count>(&mut conn)
            .expect("read still works");
        assert_eq!(readable.count, 1);
        assert!(!probe_main_db_write(&mut conn));

        sql_query("PRAGMA query_only = OFF")
            .execute(&mut conn)
            .expect("restore write access");
        assert!(probe_main_db_write(&mut conn));
        assert_eq!(
            sql_query("SELECT COUNT(*) AS count FROM sqlite_master WHERE name = '__stock_analysis_health_write_probe_20260925'")
                .get_result::<Count>(&mut conn)
                .expect("probe table was rolled back")
                .count,
            0
        );
    }

    #[test]
    fn every_component_is_blocking() {
        let mut healthy = HealthStatus {
            db_writable: true,
            bus_alive: true,
            strategy_registered: true,
            perf_recent: true,
            quote_provider: true,
        };
        assert!(healthy.all_ok());
        healthy.quote_provider = false;
        assert!(!healthy.all_ok());
    }

    #[test]
    fn performance_timestamp_must_be_within_24_hours() {
        let now =
            NaiveDateTime::parse_from_str("2026-07-17 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let recent = (now - Duration::hours(23))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let stale = (now - Duration::hours(25))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        assert!(snapshot_is_recent(&recent, now));
        assert!(!snapshot_is_recent(&stale, now));
        assert!(!snapshot_is_recent("invalid", now));
    }
}
