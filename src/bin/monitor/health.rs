//! Production health checks for storage, event delivery, strategies,
//! performance snapshots, and realtime quote registration.

use chrono::{NaiveDateTime, Utc};
use diesel::prelude::*;
use diesel::sql_query;
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
        bus_alive: stock_analysis::event::global_bus().receiver_count() >= 2,
        strategy_registered: StrategyRegistry::global().list_all().len() >= 8,
        perf_recent: check_perf_24h(),
        quote_provider: stock_analysis::broker::quote_provider_registered(),
    }
}

fn check_db() -> bool {
    #[derive(diesel::QueryableByName)]
    struct QueryRow {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        ok: i32,
    }

    let Some(db) = DatabaseManager::try_get() else {
        return false;
    };
    let Ok(mut conn) = db.get_conn() else {
        return false;
    };
    sql_query("SELECT 1 AS ok")
        .get_result::<QueryRow>(&mut conn)
        .is_ok_and(|row| row.ok == 1)
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
