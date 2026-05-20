use diesel::prelude::*;
use log::error;
use crate::database::DatabaseManager;
use chrono::Local;

pub struct AgentLogDao;

impl AgentLogDao {
    pub fn insert_log(session_id: &str, step: i32, log_type: &str, content: &str) {
        let db = DatabaseManager::get();
        if let Ok(mut conn) = db.get_conn() {
            let query = format!(
                r#"
                INSERT INTO agent_scratchpad (session_id, step, log_type, content, created_at)
                VALUES ('{}', {}, '{}', '{}', '{}')
                "#,
                session_id.replace("'", "''"), 
                step, 
                log_type.replace("'", "''"), 
                content.replace("'", "''"),
                Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
            );
            if let Err(e) = diesel::sql_query(query).execute(&mut conn) {
                error!("Failed to insert agent log: {}", e);
            }
        } else {
            error!("Failed to get db connection for logging agent scratchpad");
        }
    }
}
