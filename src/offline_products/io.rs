//! Local artifacts only. All destinations are exclusive; never initialize a source.
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let file = std::fs::File::open(path)?;
    anyhow::ensure!(file.metadata()?.is_file(), "input must be a regular file");
    let mut bytes = Vec::new();
    file.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 32 * 1024 * 1024, "input exceeds 32 MiB");
    Ok(serde_json::from_slice(&bytes)?)
}
/// Preflight is usability only; create_new remains the race-safe overwrite guard.
pub fn preflight_outputs(paths: &[&Path]) -> anyhow::Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for path in paths {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("output must name a file"))?;
        let destination = parent.canonicalize()?.join(name);
        anyhow::ensure!(seen.insert(destination), "output destinations must differ");
        match std::fs::symlink_metadata(path) {
            Ok(_) => anyhow::bail!("output already exists: {}", path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub fn write_new_private(path: &Path, content: &[u8]) -> anyhow::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(content)?;
    Ok(())
}
pub fn file_hash(path: &Path) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(hex::encode(digest.finalize()))
}
pub fn stable_snapshot(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        std::fs::metadata(path)?.is_file(),
        "existing stable SQLite snapshot required"
    );
    for suffix in ["-wal", "-journal"] {
        let other = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        anyhow::ensure!(
            !other.exists() || other.metadata()?.len() == 0,
            "nonempty WAL/journal: supply a detached normalized snapshot"
        );
    }
    Ok(())
}
pub(crate) fn csv_cell(value: &str) -> String {
    // Formula injection applies even in quoted cells when opened by Excel.
    let numeric = value.parse::<f64>().is_ok_and(|v| v.is_finite());
    let safe = if !numeric
        && value
            .trim_start()
            .starts_with(['=', '+', '-', '@', '\t', '\r'])
    {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    format!("\"{}\"", safe.replace('"', "\"\""))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusive_private_output_and_csv_injection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out");
        write_new_private(&path, b"first").unwrap();
        assert!(write_new_private(&path, b"second").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(csv_cell("=SUM(1)"), "\"'=SUM(1)\"");
        assert_eq!(csv_cell("-5.25"), "\"-5.25\"");
    }
}
