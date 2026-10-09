//! Descriptive registry only: never imported by strategy or selection owners.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const DEFAULT_REGISTRY: &str = include_str!("../../../config/signal_registry.toml");

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Family {
    NewsCatalyst,
    MainNetInflow,
    VolumeSurge,
    PostCloseFundInflow,
    StreakLeader,
    ThemePrediction,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    PaperBuy,
    InfoOnly,
    Disabled,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Watch,
    Demoted,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Signal {
    #[serde(default, deserialize_with = "explicit_id")]
    pub id: String,
    pub name: Family,
    pub signal_version: String,
    pub exit_rule_version: String,
    pub cost_model_version: String,
    pub action: Action,
    pub status: Status,
    pub entry_assumption: String,
    pub observation_window: Vec<String>,
    pub source_module: String,
    pub eligibility_price_observation: String,
    pub eligibility_simulated_fill: String,
    pub eligibility_net_return: String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    #[serde(default = "registry_version")]
    pub registry_version: String,
    pub signal: Vec<Signal>,
}
#[derive(Debug, Serialize)]
pub struct RegistryInput {
    pub source: String,
    pub sha256: String,
    pub content: Registry,
    pub authority: &'static str,
}
impl RegistryInput {
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let (source, bytes) = match path {
            Some(path) => {
                anyhow::ensure!(
                    std::fs::metadata(path)?.len() <= 128 * 1024,
                    "registry exceeds 128 KiB bound"
                );
                (
                    path.canonicalize()?.display().to_string(),
                    std::fs::read(path)?,
                )
            }
            None => (
                "embedded:config/signal_registry.toml".into(),
                DEFAULT_REGISTRY.as_bytes().to_vec(),
            ),
        };
        anyhow::ensure!(bytes.len() <= 128 * 1024, "registry exceeds 128 KiB bound");
        let content = Registry::parse(std::str::from_utf8(&bytes)?)?;
        Ok(Self { source, sha256: super::bytes_sha256(&bytes), content,
            authority: "manual descriptive review contracts only; actions/status never change strategy, activation, delivery or DB schema" })
    }
}
impl Registry {
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        let mut registry: Self = toml::from_str(raw)?;
        anyhow::ensure!(
            registry.schema_version == 1,
            "unknown signal registry schema_version"
        );
        anyhow::ensure!(
            !registry.registry_version.trim().is_empty(),
            "missing registry_version"
        );
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for signal in &mut registry.signal {
            if signal.id.is_empty() {
                signal.id = signal.name.default_id().into();
            }
            anyhow::ensure!(
                !signal.id.is_empty()
                    && signal
                        .id
                        .bytes()
                        .all(|v| v.is_ascii_lowercase() || v.is_ascii_digit() || v == b'_'),
                "invalid signal id"
            );
            anyhow::ensure!(ids.insert(&signal.id), "duplicate signal id: {}", signal.id);
            anyhow::ensure!(
                names.insert(signal.name),
                "duplicate signal name: {:?}",
                signal.name
            );
            for (field, value) in [
                ("signal_version", &signal.signal_version),
                ("exit_rule_version", &signal.exit_rule_version),
                ("cost_model_version", &signal.cost_model_version),
                ("entry_assumption", &signal.entry_assumption),
                ("source_module", &signal.source_module),
                (
                    "eligibility_price_observation",
                    &signal.eligibility_price_observation,
                ),
                (
                    "eligibility_simulated_fill",
                    &signal.eligibility_simulated_fill,
                ),
                ("eligibility_net_return", &signal.eligibility_net_return),
            ] {
                anyhow::ensure!(!value.trim().is_empty(), "{} missing {}", signal.id, field);
            }
            let windows: BTreeSet<_> = signal
                .observation_window
                .iter()
                .map(String::as_str)
                .collect();
            anyhow::ensure!(
                !windows.is_empty()
                    && windows.len() == signal.observation_window.len()
                    && windows.iter().all(|n| ["t1", "t3", "t5"].contains(n)),
                "{} invalid observation_window: only unique supported t1/t3/t5 windows",
                signal.id
            );
        }
        anyhow::ensure!(
            names.len() == 6,
            "registry must cover all six plan families"
        );
        Ok(registry)
    }
}

fn schema_version() -> u32 {
    1
}
fn registry_version() -> String {
    "signal-registry-v0".into()
}
fn explicit_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let id = String::deserialize(deserializer)?;
    if id.is_empty() {
        return Err(serde::de::Error::custom(
            "explicit signal id must be nonempty",
        ));
    }
    Ok(id)
}
impl Family {
    fn default_id(self) -> &'static str {
        match self {
            Self::NewsCatalyst => "news_catalyst",
            Self::MainNetInflow => "main_net_inflow",
            Self::VolumeSurge => "volume_surge",
            Self::PostCloseFundInflow => "post_close_fund_inflow",
            Self::StreakLeader => "streak_leader",
            Self::ThemePrediction => "theme_prediction",
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plan_fields_are_required_independently_and_do_not_alias_legacy_names() {
        for field in [
            "source_module",
            "eligibility_price_observation",
            "eligibility_simulated_fill",
            "eligibility_net_return",
            "entry_assumption",
            "exit_rule_version",
            "cost_model_version",
            "observation_window",
        ] {
            let missing = DEFAULT_REGISTRY
                .lines()
                .filter(|line| !line.starts_with(&format!("{field} =")))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                Registry::parse(&missing).is_err(),
                "accepted missing independent field {field}"
            );
            if field != "observation_window" {
                let empty = DEFAULT_REGISTRY
                    .lines()
                    .map(|line| {
                        if line.starts_with(&format!("{field} =")) {
                            format!("{field} = \"\"")
                        } else {
                            line.into()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(
                    Registry::parse(&empty).is_err(),
                    "accepted empty independent field {field}"
                );
            }
        }
        assert!(Registry::parse(
            &DEFAULT_REGISTRY.replace("eligibility_price_observation", "eligibility")
        )
        .is_err());
        assert!(Registry::parse(&DEFAULT_REGISTRY.replace("[[signal]]", "[[signals]]")).is_err());
    }
}
