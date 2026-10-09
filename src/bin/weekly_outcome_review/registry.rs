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
    Observe,
    Maintain,
    Pause,
    Review,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    EvidencePending,
    Observational,
    Qualified,
    Paused,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Signal {
    pub id: String,
    pub name: Family,
    pub signal_version: String,
    pub exit_version: String,
    pub cost_version: String,
    pub action: Action,
    pub status: Status,
    pub entry_assumptions: String,
    pub windows: Vec<usize>,
    pub eligibility: String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema_version: u32,
    pub registry_version: String,
    pub signals: Vec<Signal>,
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
        let registry: Self = toml::from_str(raw)?;
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
        for signal in &registry.signals {
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
                ("exit_version", &signal.exit_version),
                ("cost_version", &signal.cost_version),
                ("entry_assumptions", &signal.entry_assumptions),
                ("eligibility", &signal.eligibility),
            ] {
                anyhow::ensure!(!value.trim().is_empty(), "{} missing {}", signal.id, field);
            }
            let windows: BTreeSet<_> = signal.windows.iter().copied().collect();
            anyhow::ensure!(
                !windows.is_empty()
                    && windows.len() == signal.windows.len()
                    && windows.iter().all(|n| [1, 3, 5].contains(n)),
                "{} invalid windows: only unique supported T+1/3/5 windows",
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
