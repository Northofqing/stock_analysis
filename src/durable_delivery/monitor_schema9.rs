//! The retained monitor reopens its existing Schema9 store. Frozen platform
//! code keeps the default Schema14 boundary; this path performs no migration.
use super::model::{DurableDeliveryError, Result};
use rusqlite::Connection;
use std::sync::OnceLock;

pub(super) const DDL: &str = include_str!("../../contracts/durable_monitor_v9/schema.sql");
type Catalog = Vec<(String, String, String, Option<String>)>;
static REFERENCE: OnceLock<std::result::Result<Catalog, String>> = OnceLock::new();

pub(super) fn verify(connection: &Connection) -> Result<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version != 9 {
        return Err(DurableDeliveryError::InvalidConfiguration(format!(
            "retained monitor requires an existing Schema9 store, got {version}"
        )));
    }
    let expected = REFERENCE.get_or_init(|| {
        let reference = Connection::open_in_memory().map_err(|e| e.to_string())?;
        reference.execute_batch(DDL).map_err(|e| e.to_string())?;
        catalog(&reference).map_err(|e| e.to_string())
    });
    let expected = expected.as_ref().map_err(|e| {
        DurableDeliveryError::InvalidConfiguration(format!("frozen Schema9 catalog: {e}"))
    })?;
    if &catalog(connection)? != expected {
        return Err(DurableDeliveryError::InvalidConfiguration(
            "retained Schema9 catalog differs; migration and repair are disabled".into(),
        ));
    }
    super::schema::verify_existing_policy_catalog(connection)
}

fn catalog(connection: &Connection) -> Result<Catalog> {
    let mut statement = connection.prepare(
        "SELECT type,name,tbl_name,sql FROM main.sqlite_master ORDER BY type,name,tbl_name",
    )?;
    Ok(statement
        .query_map([], |r| {
            let sql: Option<String> = r.get(3)?;
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                sql.map(|s| normalize_sql(&s)),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?)
}

// SQLite retains formatting from different creation/migration paths. Ignore
// whitespace only outside quoted tokens; preserve literals and doubled quotes.
fn normalize_sql(sql: &str) -> String {
    let mut result = String::new();
    let mut quote = None;
    let mut space = false;
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(end) = quote {
            result.push(ch);
            if ch == end {
                if chars.peek() == Some(&end) && end != ']' {
                    result.push(chars.next().expect("peeked quote"));
                } else {
                    quote = None;
                }
            }
        } else if ch.is_ascii_whitespace() {
            space = !result.is_empty();
        } else {
            if space {
                result.push(' ');
                space = false;
            }
            result.push(ch);
            quote = match ch {
                '\'' | '"' | '`' => Some(ch),
                '[' => Some(']'),
                _ => None,
            };
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema9_catalog_normalization_preserves_quoted_bytes() {
        assert_eq!(
            normalize_sql("CREATE  TABLE\nx('a  b','it''s  x')"),
            "CREATE TABLE x('a  b','it''s  x')"
        );
        assert_ne!(
            normalize_sql("SELECT 'a b'"),
            normalize_sql("SELECT 'a  b'")
        );
    }
}
