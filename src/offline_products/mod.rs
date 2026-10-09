//! Explicit offline products. No acquisition, execution, delivery or global DB initialization.
pub mod io;
pub mod sell_reminder;
pub mod streak_leader_research;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type Clock = DateTime<FixedOffset>;
pub fn shanghai_clock(value: &str) -> Result<Clock, String> {
    let time = DateTime::parse_from_rfc3339(value).map_err(|e| e.to_string())?;
    if time.offset().local_minus_utc() != 28800 {
        return Err("clock must use +08:00".into());
    }
    Ok(time)
}
pub(crate) fn at(day: NaiveDate, hour: u32, minute: u32) -> Clock {
    FixedOffset::east_opt(28800)
        .unwrap()
        .from_local_datetime(&day.and_hms_opt(hour, minute, 0).unwrap())
        .single()
        .unwrap()
}
pub(crate) fn hash<T: Serialize>(value: &T) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_vec(value).expect("canonical finite DTO"),
    ))
}
pub(crate) fn escaped(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| match c {
            '&' => "&amp;".chars().collect::<Vec<_>>(),
            '<' => "&lt;".chars().collect(),
            '>' => "&gt;".chars().collect(),
            '|' => "&#124;".chars().collect(),
            '[' => "&#91;".chars().collect(),
            ']' => "&#93;".chars().collect(),
            '`' => "&#96;".chars().collect(),
            '*' => "&#42;".chars().collect(),
            '_' => "&#95;".chars().collect(),
            '\\' => "&#92;".chars().collect(),
            c if c.is_control() => vec![' '],
            c => vec![c],
        })
        .collect()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub source: String,
    pub revision: String,
    pub sha256: String,
    pub known_at: Clock,
    pub conflicted: bool,
    pub invalidated: bool,
}
impl Source {
    pub(crate) fn validate(&self, as_of: Clock) -> Result<(), String> {
        if self.source.trim().is_empty()
            || self.revision.trim().is_empty()
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|v| v.is_ascii_hexdigit())
            || self.known_at > as_of
            || self.conflicted
            || self.invalidated
        {
            Err("source/revision/hash/known_at/conflict/invalidated evidence invalid".into())
        } else {
            Ok(())
        }
    }
}
