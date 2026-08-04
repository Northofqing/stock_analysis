//! BR-155 deterministic event-to-chain and exact security relationships.

use crate::config::ChainRuleConfig;
use crate::selection::model::{
    DirectMentionEvidence, DirectMentionKind, SecurityIdentity, SecurityMasterSnapshot,
};
use crate::signal::market_event::MarketEvent;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainConfigError {
    reason_code: &'static str,
    message: String,
}

impl ChainConfigError {
    pub fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

impl std::fmt::Display for ChainConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ChainConfigError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidatedChainRule {
    pub chain_id: String,
    pub logic: String,
    pub board_keyword: Option<String>,
    pub keywords: Vec<String>,
    pub priority: u32,
    pub category: Option<String>,
    pub generic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainConfigSnapshot {
    rules: Vec<ValidatedChainRule>,
    content_hash: String,
}

impl ChainConfigSnapshot {
    pub fn from_rules(rules: &[ChainRuleConfig]) -> Result<Self, ChainConfigError> {
        let mut seen = HashSet::new();
        let mut validated = Vec::new();

        for rule in rules {
            let chain_id = rule.chain.trim();
            if chain_id.is_empty() {
                return Err(config_error("chain_id_empty", "chain ID is empty"));
            }
            if !seen.insert(chain_id.to_string()) {
                return Err(config_error(
                    "duplicate_chain_id",
                    format!("duplicate chain ID: {chain_id}"),
                ));
            }
            if rule.logic.trim().is_empty() {
                return Err(config_error(
                    "chain_logic_empty",
                    format!("chain logic is empty: {chain_id}"),
                ));
            }
            if rule.priority > 100 {
                return Err(config_error(
                    "chain_priority_out_of_range",
                    format!("chain priority exceeds 100: {chain_id}"),
                ));
            }

            let mut keywords = rule
                .keywords
                .iter()
                .map(|keyword| keyword.trim().to_string())
                .collect::<Vec<_>>();
            if keywords.is_empty() || keywords.iter().any(String::is_empty) {
                return Err(config_error(
                    "chain_keywords_invalid",
                    format!("chain keywords are empty: {chain_id}"),
                ));
            }
            keywords.sort();
            keywords.dedup();

            if rule.enabled {
                validated.push(ValidatedChainRule {
                    chain_id: chain_id.to_string(),
                    logic: rule.logic.trim().to_string(),
                    board_keyword: non_empty(&rule.board_keyword),
                    keywords,
                    priority: rule.priority,
                    category: non_empty(&rule.category),
                    generic: rule.generic,
                });
            }
        }

        if validated.is_empty() {
            return Err(config_error(
                "no_enabled_chain_rules",
                "no enabled chain rules",
            ));
        }
        validated.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.chain_id.cmp(&right.chain_id))
        });
        let canonical = serde_json::to_vec(&validated).map_err(|error| {
            config_error(
                "chain_snapshot_serialize_failed",
                format!("chain snapshot serialization failed: {error}"),
            )
        })?;
        let mut hasher = Sha256::new();
        hasher.update(b"stock_analysis.selection_chain_config.v1\0");
        hasher.update(canonical);

        Ok(Self {
            rules: validated,
            content_hash: format!(
                "selection_chain_config_v1_{}",
                hex::encode(hasher.finalize())
            ),
        })
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn rules(&self) -> &[ValidatedChainRule] {
        &self.rules
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainMatch {
    pub chain_id: String,
    pub priority: u32,
    pub matched_keywords: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventChainMapping {
    pub event_id: String,
    pub chains: Vec<ChainMatch>,
}

impl EventChainMapping {
    pub fn chain_ids(&self) -> Vec<&str> {
        self.chains
            .iter()
            .map(|chain| chain.chain_id.as_str())
            .collect()
    }
}

pub fn map_events(
    events: &[MarketEvent],
    snapshot: &ChainConfigSnapshot,
) -> Vec<EventChainMapping> {
    events
        .iter()
        .map(|event| {
            let text = event_text(event);
            let chains = snapshot
                .rules()
                .iter()
                .filter_map(|rule| {
                    let matched_keywords = rule
                        .keywords
                        .iter()
                        .filter(|keyword| text.contains(keyword.as_str()))
                        .cloned()
                        .collect::<Vec<_>>();
                    (!matched_keywords.is_empty()).then(|| ChainMatch {
                        chain_id: rule.chain_id.clone(),
                        priority: rule.priority,
                        matched_keywords,
                    })
                })
                .collect();
            EventChainMapping {
                event_id: event.event_id.clone(),
                chains,
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectMentionError {
    reason_code: &'static str,
    message: String,
}

impl DirectMentionError {
    pub fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

impl std::fmt::Display for DirectMentionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DirectMentionError {}

pub fn direct_mentions(
    event_text: &str,
    master: &SecurityMasterSnapshot,
) -> Result<Vec<DirectMentionEvidence>, DirectMentionError> {
    let mut selected: BTreeMap<String, DirectMentionEvidence> = BTreeMap::new();
    for identity in master.identities() {
        if contains_identity_token(event_text, &identity.code) {
            selected.insert(
                identity.code.clone(),
                mention(identity, DirectMentionKind::ExactSecurityCode, master),
            );
        }
    }

    let mut identities_by_name: BTreeMap<&str, Vec<&SecurityIdentity>> = BTreeMap::new();
    for identity in master.identities() {
        identities_by_name
            .entry(identity.name.as_str())
            .or_default()
            .push(identity);
    }
    for (name, identities) in identities_by_name {
        if !event_text.contains(name) {
            continue;
        }
        if identities.len() > 1
            && identities
                .iter()
                .all(|identity| !selected.contains_key(&identity.code))
        {
            return Err(DirectMentionError {
                reason_code: "ambiguous_security_name",
                message: format!("security name maps to multiple codes: {name}"),
            });
        }
        for identity in identities {
            selected
                .entry(identity.code.clone())
                .or_insert_with(|| mention(identity, DirectMentionKind::ExactSecurityName, master));
        }
    }

    Ok(selected.into_values().collect())
}

fn mention(
    identity: &SecurityIdentity,
    matched_by: DirectMentionKind,
    master: &SecurityMasterSnapshot,
) -> DirectMentionEvidence {
    DirectMentionEvidence {
        security: identity.clone(),
        matched_by,
        master_batch_id: master.batch_id.clone(),
    }
}

fn contains_identity_token(text: &str, code: &str) -> bool {
    text.match_indices(code).any(|(start, matched)| {
        let end = start + matched.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        !before.is_some_and(identity_token_char) && !after.is_some_and(identity_token_char)
    })
}

fn identity_token_char(value: char) -> bool {
    value.is_ascii_alphanumeric() || value == '_'
}

fn event_text(event: &MarketEvent) -> String {
    let mut text = String::with_capacity(
        event.full_title.len()
            + event.subject.len()
            + event.object.as_deref().map_or(0, str::len)
            + 2,
    );
    text.push_str(&event.full_title);
    text.push('\n');
    text.push_str(&event.subject);
    if let Some(object) = &event.object {
        text.push('\n');
        text.push_str(object);
    }
    text
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn config_error(reason_code: &'static str, message: impl Into<String>) -> ChainConfigError {
    ChainConfigError {
        reason_code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ChainRuleConfig;
    use crate::selection::model::{
        DirectMentionKind, SecurityIdentity, SecurityMarket, SecurityMasterSnapshot,
    };
    use crate::signal::market_event::{
        Direction, EventType, MarketEvent, ProviderPublication, SourceRef,
    };
    use chrono::{Local, TimeZone};

    fn test_rule(chain: &str, priority: u32, keywords: &[&str]) -> ChainRuleConfig {
        ChainRuleConfig {
            chain: chain.to_string(),
            logic: format!("{chain} rule"),
            board_keyword: String::new(),
            keywords: keywords.iter().map(|value| (*value).to_string()).collect(),
            priority,
            category: "TEST_CODE_category".to_string(),
            generic: false,
            enabled: true,
        }
    }

    fn test_event(event_id: &str, title: &str) -> MarketEvent {
        let occurred_at = Local
            .with_ymd_and_hms(2026, 7, 23, 8, 30, 0)
            .single()
            .expect("fixed local time");
        MarketEvent {
            event_id: event_id.to_string(),
            simhash: 1,
            full_title: title.to_string(),
            event_type: EventType::Other,
            subject: "TEST_CODE_provider".to_string(),
            object: None,
            direction: Direction::Neutral,
            strength: 50,
            certainty: 50,
            chains: Vec::new(),
            occurred_at,
            provider_publication: Some(ProviderPublication {
                published_on: occurred_at.date_naive(),
                published_at: Some(occurred_at),
            }),
            provenance: vec![SourceRef {
                provider: "TEST_CODE_provider".to_string(),
                url: None,
                fetched_at: occurred_at,
            }],
            ai_degraded: false,
            stale: false,
        }
    }

    #[test]
    fn two_events_are_mapped_independently() {
        let snapshot = ChainConfigSnapshot::from_rules(&[
            test_rule("chip", 100, &["芯片"]),
            test_rule("gold", 90, &["黄金"]),
        ])
        .expect("valid chain snapshot");

        let mapped = map_events(
            &[
                test_event("event-chip", "芯片扩产"),
                test_event("event-gold", "黄金涨价"),
            ],
            &snapshot,
        );

        assert_eq!(mapped[0].event_id, "event-chip");
        assert_eq!(mapped[0].chain_ids(), vec!["chip"]);
        assert_eq!(mapped[1].event_id, "event-gold");
        assert_eq!(mapped[1].chain_ids(), vec!["gold"]);
    }

    #[test]
    fn direct_company_mention_requires_an_exact_unique_master_identity() {
        let observed_at = Local
            .with_ymd_and_hms(2026, 7, 23, 8, 30, 0)
            .single()
            .expect("fixed local time");
        let master = SecurityMasterSnapshot::new(
            vec![
                SecurityIdentity {
                    code: "TEST_CODE_000001".to_string(),
                    name: "测试甲".to_string(),
                    market: SecurityMarket::Shanghai,
                },
                SecurityIdentity {
                    code: "TEST_CODE_000002".to_string(),
                    name: "测试乙".to_string(),
                    market: SecurityMarket::Shenzhen,
                },
            ],
            "TEST_CODE_master_batch".to_string(),
            observed_at,
        )
        .expect("valid master");

        let evidence = direct_mentions("测试甲获得芯片订单", &master).expect("direct mention");

        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].security.code, "TEST_CODE_000001");
        assert_eq!(evidence[0].security.name, "测试甲");
        assert_eq!(evidence[0].matched_by, DirectMentionKind::ExactSecurityName);
        assert!(direct_mentions("测试获得芯片订单", &master)
            .expect("partial name is not an error")
            .is_empty());
    }

    #[test]
    fn chain_snapshot_rejects_duplicate_ids_and_keeps_all_stably_sorted_matches() {
        let duplicate = ChainConfigSnapshot::from_rules(&[
            test_rule("chip", 100, &["芯片"]),
            test_rule("chip", 90, &["半导体"]),
        ])
        .expect_err("duplicate chain IDs must fail");
        assert_eq!(duplicate.reason_code(), "duplicate_chain_id");

        let snapshot = ChainConfigSnapshot::from_rules(&[
            test_rule("low", 10, &["共同词"]),
            test_rule("high-b", 90, &["共同词"]),
            test_rule("high-a", 90, &["共同词"]),
        ])
        .expect("valid chain snapshot");
        let mapped = map_events(&[test_event("event-all", "共同词")], &snapshot);

        assert_eq!(
            mapped[0].chain_ids(),
            vec!["high-a", "high-b", "low"],
            "phase 1 keeps every matched chain and applies no Top-N"
        );
        assert!(snapshot
            .content_hash()
            .starts_with("selection_chain_config_v1_"));
    }

    #[test]
    fn ambiguous_security_name_is_rejected_and_code_token_must_be_complete() {
        let observed_at = Local
            .with_ymd_and_hms(2026, 7, 23, 8, 30, 0)
            .single()
            .expect("fixed local time");
        let master = SecurityMasterSnapshot::new(
            vec![
                SecurityIdentity {
                    code: "TEST_CODE_000001".to_string(),
                    name: "同名公司".to_string(),
                    market: SecurityMarket::Shanghai,
                },
                SecurityIdentity {
                    code: "TEST_CODE_000002".to_string(),
                    name: "同名公司".to_string(),
                    market: SecurityMarket::Shenzhen,
                },
            ],
            "TEST_CODE_master_batch".to_string(),
            observed_at,
        )
        .expect("valid master");

        assert_eq!(
            direct_mentions("同名公司发布公告", &master)
                .expect_err("ambiguous name must fail")
                .reason_code(),
            "ambiguous_security_name"
        );
        assert!(direct_mentions("XTEST_CODE_000001Y", &master)
            .expect("embedded token is not an error")
            .is_empty());
        assert_eq!(
            direct_mentions("(TEST_CODE_000001)发布公告", &master)
                .expect("bounded exact code")
                .len(),
            1
        );
    }

    #[test]
    fn chain_snapshot_rejects_each_invalid_rule_contract() {
        let mut empty_id = test_rule("chip", 100, &["芯片"]);
        empty_id.chain = " ".to_owned();
        let mut empty_logic = test_rule("chip", 100, &["芯片"]);
        empty_logic.logic = " ".to_owned();
        let too_high = test_rule("chip", 101, &["芯片"]);
        let empty_keywords = test_rule("chip", 100, &[]);
        let blank_keyword = test_rule("chip", 100, &[" "]);
        let mut disabled = test_rule("chip", 100, &["芯片"]);
        disabled.enabled = false;

        for (rule, expected) in [
            (empty_id, "chain_id_empty"),
            (empty_logic, "chain_logic_empty"),
            (too_high, "chain_priority_out_of_range"),
            (empty_keywords, "chain_keywords_invalid"),
            (blank_keyword, "chain_keywords_invalid"),
            (disabled, "no_enabled_chain_rules"),
        ] {
            let error = ChainConfigSnapshot::from_rules(&[rule]).expect_err("invalid chain rule");
            assert_eq!(error.reason_code(), expected);
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn event_object_participates_in_mapping_and_direct_error_is_displayable() {
        let snapshot = ChainConfigSnapshot::from_rules(&[test_rule("chip", 100, &["芯片"])])
            .expect("snapshot");
        let mut event = test_event("TEST_CODE_event", "普通公告");
        event.object = Some("芯片".to_owned());
        assert_eq!(map_events(&[event], &snapshot)[0].chain_ids(), ["chip"]);

        let observed_at = Local
            .with_ymd_and_hms(2026, 7, 23, 8, 30, 0)
            .single()
            .expect("time");
        let master = SecurityMasterSnapshot::new(
            vec![
                SecurityIdentity {
                    code: "TEST_CODE_000001".to_owned(),
                    name: "同名".to_owned(),
                    market: SecurityMarket::Shanghai,
                },
                SecurityIdentity {
                    code: "TEST_CODE_000002".to_owned(),
                    name: "同名".to_owned(),
                    market: SecurityMarket::Shenzhen,
                },
            ],
            "TEST_CODE_master".to_owned(),
            observed_at,
        )
        .expect("master");
        let error = direct_mentions("同名", &master).expect_err("ambiguous");
        assert_eq!(error.reason_code(), "ambiguous_security_name");
        assert!(!error.to_string().is_empty());
    }
}
