use crate::database::DatabaseManager;
use diesel::prelude::*;
use diesel::sql_types::{Integer, Text};

pub struct AgentLogDao;

impl AgentLogDao {
    pub fn insert_log(
        session_id: &str,
        step: i32,
        log_type: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        let db = DatabaseManager::try_get()
            .ok_or_else(|| anyhow::anyhow!("agent audit database is not initialized"))?;
        let mut conn = db
            .get_conn()
            .map_err(|error| anyhow::anyhow!("agent audit database connection: {error}"))?;
        diesel::sql_query(
            "INSERT INTO agent_scratchpad (session_id, step, log_type, content) \
             VALUES (?, ?, ?, ?)",
        )
        .bind::<Text, _>(session_id)
        .bind::<Integer, _>(step)
        .bind::<Text, _>(log_type)
        .bind::<Text, _>(content)
        .execute(&mut conn)
        .map_err(|error| anyhow::anyhow!("persist agent audit log: {error}"))?;
        Ok(())
    }
}
