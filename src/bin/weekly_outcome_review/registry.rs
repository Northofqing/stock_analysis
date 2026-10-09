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
            Some(path) => read_registry_file(path)?,
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
// Ordinary readable TOML is accepted; private-artifact permission policy belongs
// to the consumer. Bind the explicit producer input to one finite descriptor.
fn read_registry_file(path: &Path) -> anyhow::Result<(String, Vec<u8>)> {
    use std::fs::OpenOptions;
    let filename = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("registry filename"))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let pinned = parent.canonicalize()?.join(filename);
    let before = std::fs::symlink_metadata(&pinned)?;
    anyhow::ensure!(
        before.is_file(),
        "registry must be a regular file, not a symlink"
    );
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(&pinned)?;
    let bytes = read_registry_descriptor(path, &mut file, &before)?;
    // Original parent/name must still resolve to the same bytes as the descriptor.
    anyhow::ensure!(
        same_registry_file(&before, &std::fs::symlink_metadata(&pinned)?),
        "registry path changed"
    );
    Ok((pinned.display().to_string(), bytes))
}

fn same_registry_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.is_file()
            && b.is_file()
            && (
                a.dev(),
                a.ino(),
                a.len(),
                a.mtime(),
                a.mtime_nsec(),
                a.ctime(),
                a.ctime_nsec(),
            ) == (
                b.dev(),
                b.ino(),
                b.len(),
                b.mtime(),
                b.mtime_nsec(),
                b.ctime(),
                b.ctime_nsec(),
            )
    }
    #[cfg(not(unix))]
    {
        a.is_file() && b.is_file() && a.len() == b.len() && a.modified().ok() == b.modified().ok()
    }
}

fn read_registry_descriptor(
    path: &Path,
    file: &mut std::fs::File,
    before: &std::fs::Metadata,
) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    anyhow::ensure!(
        same_registry_file(before, &file.metadata()?),
        "registry descriptor changed or is not regular"
    );
    let mut bytes = Vec::new();
    file.take(128 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 128 * 1024, "registry exceeds 128 KiB bound");
    anyhow::ensure!(
        bytes.len() as u64 == before.len()
            && same_registry_file(before, &file.metadata()?)
            && same_registry_file(before, &std::fs::symlink_metadata(path)?),
        "registry changed during read"
    );
    Ok(bytes)
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
#[cfg(all(test, unix))]
mod bounded_input_tests {
    use super::*;
    use std::{
        fs::OpenOptions,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };
    #[test]
    fn ordinary_0644_original_bytes_and_embedded_hash_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.toml");
        let raw = format!("# original byte comment\n{DEFAULT_REGISTRY}");
        std::fs::write(&path, raw.as_bytes()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let loaded = RegistryInput::load(Some(&path)).unwrap();
        assert_eq!(loaded.sha256, super::super::bytes_sha256(raw.as_bytes()));
        assert_eq!(
            loaded.source,
            path.canonicalize().unwrap().display().to_string()
        );
        assert_eq!(
            RegistryInput::load(None).unwrap().sha256,
            super::super::bytes_sha256(DEFAULT_REGISTRY.as_bytes())
        );
    }
    #[test]
    fn actual_reader_accepts_exact_cap_and_rejects_cap_plus_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.toml");
        let mut raw = DEFAULT_REGISTRY.as_bytes().to_vec();
        raw.extend_from_slice(b"\n#");
        raw.resize(128 * 1024, b' ');
        std::fs::write(&path, &raw).unwrap();
        assert!(RegistryInput::load(Some(&path)).is_ok());
        raw.push(b' ');
        std::fs::write(&path, &raw).unwrap();
        assert!(RegistryInput::load(Some(&path))
            .unwrap_err()
            .to_string()
            .contains("128 KiB"));
    }
    #[test]
    fn descriptor_growth_and_replacement_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.toml");
        std::fs::write(&path, DEFAULT_REGISTRY).unwrap();
        let before = path.metadata().unwrap();
        let mut file = std::fs::File::open(&path).unwrap();
        std::fs::write(&path, vec![b' '; 128 * 1024 + 1]).unwrap();
        assert!(read_registry_descriptor(&path, &mut file, &before).is_err());
        std::fs::write(&path, DEFAULT_REGISTRY).unwrap();
        let before = path.metadata().unwrap();
        let mut file = std::fs::File::open(&path).unwrap();
        std::fs::rename(&path, dir.path().join("old.toml")).unwrap();
        std::fs::write(&path, DEFAULT_REGISTRY).unwrap();
        assert!(read_registry_descriptor(&path, &mut file, &before).is_err());
    }
    #[test]
    fn regular_to_fifo_and_leaf_symlink_do_not_follow_or_block() {
        use std::ffi::CString;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.toml");
        std::fs::write(&path, DEFAULT_REGISTRY).unwrap();
        let before = path.metadata().unwrap();
        std::fs::remove_file(&path).unwrap();
        let cpath = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
            .unwrap();
        assert!(read_registry_descriptor(&path, &mut file, &before).is_err());
        assert!(RegistryInput::load(Some(&path)).is_err());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(dir.path().join("missing"), &path).unwrap();
        assert!(RegistryInput::load(Some(&path)).is_err());
        assert!(RegistryInput::load(Some(dir.path())).is_err());
    }
}
