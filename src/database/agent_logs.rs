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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(QueryableByName)]
    struct StoredLog {
        #[diesel(sql_type = Text)]
        session_id: String,
        #[diesel(sql_type = Integer)]
        step: i32,
        #[diesel(sql_type = Text)]
        log_type: String,
        #[diesel(sql_type = Text)]
        content: String,
    }

    #[test]
    fn insert_log_persists_the_complete_audit_record() {
        DatabaseManager::init(None).expect("test database initialization must succeed");
        let session_id = "TEST_CODE_AGENT_LOG_SESSION";
        AgentLogDao::insert_log(session_id, 7, "tool", "validated content")
            .expect("agent audit log must persist");

        let db = DatabaseManager::try_get().expect("test database must remain initialized");
        let mut conn = db.get_conn().expect("test database connection");
        let stored = diesel::sql_query(
            "SELECT session_id, step, log_type, content FROM agent_scratchpad \
             WHERE session_id = ? ORDER BY id DESC LIMIT 1",
        )
        .bind::<Text, _>(session_id)
        .get_result::<StoredLog>(&mut conn)
        .expect("persisted agent audit row");
        assert_eq!(stored.session_id, session_id);
        assert_eq!(stored.step, 7);
        assert_eq!(stored.log_type, "tool");
        assert_eq!(stored.content, "validated content");
    }
}
