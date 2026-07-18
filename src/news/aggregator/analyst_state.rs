//! v17.7 Task 4: Stateful analyst rating upgrade/downgrade detection
//!
//! Tracks per (code, broker) the last observed rating and detects:
//! - Upgrade: rank increased (e.g. 中性 → 增持 → 买入)
//! - Downgrade: rank decreased (e.g. 增持 → 中性)
//! - Duplicate: same report_id AND same publish_date as last observation
//! - UnknownRating: label not in the recognized set
//!
//! Bounded to `capacity` entries (default 10_000). On overflow the oldest
//! entry is evicted and a `warn` is logged.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct AnalystKey {
    pub code: String,
    pub broker: String,
}

#[derive(Debug, Clone)]
pub struct AnalystObservation {
    pub rating: String,
    pub publish_date: chrono::NaiveDate,
    pub report_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationDecision {
    Observed,
    Upgrade { from: String, to: String },
    Downgrade { from: String, to: String },
    Duplicate,
    UnknownRating,
}

#[derive(Debug, Clone)]
struct StoredObservation {
    rating_rank: i8,
    rating_label: String,
    publish_date: chrono::NaiveDate,
    report_id: String,
}

struct Inner {
    map: HashMap<AnalystKey, StoredObservation>,
    order: VecDeque<AnalystKey>,
}

pub struct AnalystStateStore {
    capacity: usize,
    inner: Mutex<Inner>,
}

impl AnalystStateStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                order: VecDeque::new(),
            }),
        }
    }

    /// Observe a new rating for (code, broker).
    ///
    /// Returns an `ObservationDecision` that tells the caller whether this
    /// represents an upgrade, downgrade, duplicate, or first observation.
    pub fn observe(&self, key: AnalystKey, value: AnalystObservation) -> ObservationDecision {
        // 1. rank the incoming rating; unknown → UnknownRating, no store change
        let Some(incoming_rank) = Self::rating_rank(&value.rating) else {
            return ObservationDecision::UnknownRating;
        };

        // 2. lock inner
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };

        // 3. lookup existing entry
        match inner.map.get(&key) {
            None => {
                // First time we see this (code, broker) → store and return Observed
                let stored = StoredObservation {
                    rating_rank: incoming_rank,
                    rating_label: value.rating.clone(),
                    publish_date: value.publish_date,
                    report_id: value.report_id,
                };
                inner.map.insert(key.clone(), stored);
                inner.order.push_back(key);
                Self::evict_if_needed(&mut inner, self.capacity);
                ObservationDecision::Observed
            }
            Some(existing) => {
                // 4. Duplicate: same report_id AND same publish_date
                if existing.report_id == value.report_id
                    && existing.publish_date == value.publish_date
                {
                    return ObservationDecision::Duplicate;
                }

                // 5. Same rank → store newer date + Observed (no upgrade/downgrade)
                if existing.rating_rank == incoming_rank {
                    let stored = StoredObservation {
                        rating_rank: incoming_rank,
                        rating_label: value.rating.clone(),
                        publish_date: value.publish_date,
                        report_id: value.report_id,
                    };
                    inner.map.insert(key.clone(), stored);
                    // Move to back of order (newer observations are at back for LRU)
                    // First remove the key from its current position in order, then push back
                    if let Some(pos) = inner.order.iter().position(|k| k == &key) {
                        inner.order.remove(pos);
                    }
                    inner.order.push_back(key);
                    Self::evict_if_needed(&mut inner, self.capacity);
                    return ObservationDecision::Observed;
                }

                // 6. Upgrade: existing rank < incoming rank AND date not earlier
                if existing.rating_rank < incoming_rank
                    && value.publish_date >= existing.publish_date
                {
                    let from_label = existing.rating_label.clone();
                    let to_label = value.rating.clone();
                    let stored = StoredObservation {
                        rating_rank: incoming_rank,
                        rating_label: value.rating.clone(),
                        publish_date: value.publish_date,
                        report_id: value.report_id,
                    };
                    inner.map.insert(key.clone(), stored);
                    if let Some(pos) = inner.order.iter().position(|k| k == &key) {
                        inner.order.remove(pos);
                    }
                    inner.order.push_back(key);
                    Self::evict_if_needed(&mut inner, self.capacity);
                    return ObservationDecision::Upgrade {
                        from: from_label,
                        to: to_label,
                    };
                }

                // 7. Downgrade: existing rank > incoming rank
                // (store newer date regardless of date relation)
                let from_label = existing.rating_label.clone();
                let to_label = value.rating.clone();
                let stored = StoredObservation {
                    rating_rank: incoming_rank,
                    rating_label: value.rating.clone(),
                    publish_date: value.publish_date,
                    report_id: value.report_id,
                };
                inner.map.insert(key.clone(), stored);
                if let Some(pos) = inner.order.iter().position(|k| k == &key) {
                    inner.order.remove(pos);
                }
                inner.order.push_back(key);
                Self::evict_if_needed(&mut inner, self.capacity);
                ObservationDecision::Downgrade {
                    from: from_label,
                    to: to_label,
                }
            }
        }
    }

    /// Evict oldest entry if map exceeds capacity.
    fn evict_if_needed(inner: &mut Inner, capacity: usize) {
        while inner.map.len() > capacity {
            if let Some(oldest) = inner.order.pop_front() {
                let removed = inner.map.remove(&oldest);
                if removed.is_some() {
                    log::warn!(
                        "[AnalystStateStore] evicted oldest entry, remaining keys: {}",
                        inner.map.len()
                    );
                }
            } else {
                break;
            }
        }
    }

    /// Rating rank for recognized labels.
    /// Rank order: 卖出(1) < 减持(2) < 中性(3) < 增持(4) < 买入(5).
    /// Unknown labels return None.
    fn rating_rank(label: &str) -> Option<i8> {
        match label {
            "卖出" => Some(1),
            "减持" => Some(2),
            "中性" => Some(3),
            "增持" => Some(4),
            "买入" => Some(5),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn key(code: &str, broker: &str) -> AnalystKey {
        AnalystKey {
            code: code.to_string(),
            broker: broker.to_string(),
        }
    }

    fn observation(rating: &str, date: &str) -> AnalystObservation {
        AnalystObservation {
            rating: rating.to_string(),
            publish_date: chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").expect("valid date"),
            report_id: format!("report-{}-{}", date, rating),
        }
    }

    // -------------------------------------------------------------------------
    // Tests from brief
    // -------------------------------------------------------------------------

    #[test]
    fn first_rating_is_observation_not_upgrade() {
        let state = AnalystStateStore::new(10_000);
        let decision = state.observe(
            key("TEST_CODE_600519", "券商A"),
            observation("增持", "2026-07-16"),
        );
        assert_eq!(decision, ObservationDecision::Observed);
    }

    #[test]
    fn buy_after_hold_is_upgrade_once() {
        let state = AnalystStateStore::new(10_000);
        state.observe(
            key("TEST_CODE_600519", "券商A"),
            observation("中性", "2026-07-15"),
        );
        assert!(matches!(
            state.observe(
                key("TEST_CODE_600519", "券商A"),
                observation("买入", "2026-07-16")
            ),
            ObservationDecision::Upgrade { .. }
        ));
        assert_eq!(
            state.observe(
                key("TEST_CODE_600519", "券商A"),
                observation("买入", "2026-07-16")
            ),
            ObservationDecision::Duplicate
        );
    }

    #[test]
    fn unknown_rating_and_downgrade_do_not_trigger() {
        let state = AnalystStateStore::new(10_000);
        state.observe(
            key("TEST_CODE_600519", "券商A"),
            observation("买入", "2026-07-15"),
        );
        assert_eq!(
            state.observe(
                key("TEST_CODE_600519", "券商A"),
                observation("推荐", "2026-07-16")
            ),
            ObservationDecision::UnknownRating
        );
        assert_eq!(
            state.observe(
                key("TEST_CODE_600519", "券商A"),
                observation("增持", "2026-07-17")
            ),
            ObservationDecision::Downgrade {
                from: "买入".to_string(),
                to: "增持".to_string()
            }
        );
    }

    // -------------------------------------------------------------------------
    // Additional sanity tests
    // -------------------------------------------------------------------------

    #[test]
    fn downgrade_does_not_push() {
        // Start with 买入, observe 减持 → Downgrade, no Upgrade path
        let state = AnalystStateStore::new(10_000);
        state.observe(
            key("TEST_CODE_600519", "券商A"),
            observation("买入", "2026-07-15"),
        );
        let decision = state.observe(
            key("TEST_CODE_600519", "券商A"),
            observation("减持", "2026-07-16"),
        );
        assert!(matches!(decision, ObservationDecision::Downgrade { .. }));
        // Verify it was NOT an Upgrade
        assert!(!matches!(decision, ObservationDecision::Upgrade { .. }));
    }

    #[test]
    fn eviction_at_capacity() {
        // Fill to 10_000 keys then insert 10_001st → warn emitted, oldest removed
        let capacity = 10_000;
        let state = AnalystStateStore::new(capacity);

        // Insert 10_000 distinct keys
        for i in 0..capacity {
            let k = AnalystKey {
                code: format!("{:05}", i),
                broker: format!("broker{}", i),
            };
            state.observe(k, observation("增持", "2026-07-16"));
        }

        // Map should be exactly at capacity
        let inner = state.inner.lock().unwrap();
        assert_eq!(inner.map.len(), capacity);
        drop(inner);

        // Insert one more — eviction should happen
        let overflow_key = AnalystKey {
            code: "99999".to_string(),
            broker: "overflow_broker".to_string(),
        };
        state.observe(overflow_key.clone(), observation("增持", "2026-07-17"));

        let inner = state.inner.lock().unwrap();
        // Oldest key ("00000", "broker0") should have been evicted
        assert!(!inner.map.contains_key(&AnalystKey {
            code: "00000".to_string(),
            broker: "broker0".to_string(),
        }));
        // Overflow key should be present
        assert!(inner.map.contains_key(&overflow_key));
        // Size should still be at capacity
        assert_eq!(inner.map.len(), capacity);
    }

    #[test]
    fn duplicate_requires_both_report_id_and_date() {
        // Same rating, same date but different report_id → NOT a duplicate
        let state = AnalystStateStore::new(10_000);
        let key = key("TEST_CODE_600519", "券商A");

        state.observe(key.clone(), observation("增持", "2026-07-16"));

        // Different report_id (observation() generates one from date+rating)
        // → should be Observed, not Duplicate
        let obs = AnalystObservation {
            rating: "增持".to_string(),
            publish_date: chrono::NaiveDate::parse_from_str("2026-07-16", "%Y-%m-%d").unwrap(),
            report_id: "different-report-id".to_string(),
        };
        let decision = state.observe(key, obs);
        assert!(matches!(decision, ObservationDecision::Observed));
    }

    #[test]
    fn upgrade_requires_not_earlier_date() {
        // 中性 (rank 3) on 7/15, then 买入 (rank 5) on 7/10 (earlier) → NOT an upgrade
        let state = AnalystStateStore::new(10_000);
        let key = key("TEST_CODE_600519", "券商A");

        state.observe(key.clone(), observation("中性", "2026-07-15"));

        // Earlier date → downgrade even though rank is higher
        let decision = state.observe(key.clone(), observation("买入", "2026-07-10"));
        assert!(matches!(decision, ObservationDecision::Downgrade { .. }));
    }

    #[test]
    fn same_rank_stores_newer_date() {
        let state = AnalystStateStore::new(10_000);
        let key = key("TEST_CODE_600519", "券商A");

        state.observe(key.clone(), observation("增持", "2026-07-15"));
        let decision = state.observe(key.clone(), observation("增持", "2026-07-16"));
        assert_eq!(decision, ObservationDecision::Observed);

        // Verify the stored date is the newer one
        let inner = state.inner.lock().unwrap();
        let stored = inner.map.get(&key).unwrap();
        assert_eq!(stored.publish_date.day(), 16);
    }
}
