//! Local artifacts only. All destinations are exclusive; never initialize a source.
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    #[cfg(unix)]
    {
        let before = std::fs::symlink_metadata(path)?;
        let mut file = open_regular_json(path, &before)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 32 * 1024 * 1024, "input exceeds 32 MiB");
        let after = file.metadata()?;
        anyhow::ensure!(
            same_file_version(&before, &after)
                && same_file_version(&after, &std::fs::symlink_metadata(path)?),
            "input identity/content metadata changed during read"
        );
        Ok(serde_json::from_slice(&bytes)?)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        anyhow::bail!("safe regular JSON descriptor reader unavailable on this platform")
    }
}
#[cfg(unix)]
fn same_file_version(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.is_file()
        && b.is_file()
        && a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.uid() == b.uid()
        && a.mode() == b.mode()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
#[cfg(unix)]
fn open_regular_json(path: &Path, before: &std::fs::Metadata) -> anyhow::Result<std::fs::File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    anyhow::ensure!(
        before.is_file(),
        "input must be a regular file, not a link/device/pipe"
    );
    // SAFETY: geteuid reads the process identity and takes no pointer arguments.
    anyhow::ensure!(
        before.uid() == unsafe { libc::geteuid() } && before.mode() & 0o022 == 0,
        "input must be owned by this user and not writable by group/others"
    );
    anyhow::ensure!(before.len() <= 32 * 1024 * 1024, "input exceeds 32 MiB");
    // O_NONBLOCK prevents a FIFO swapped in after the path check from blocking
    // open. O_NOFOLLOW rejects a substituted symlink; fstat binds the actual fd.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    anyhow::ensure!(
        same_file_version(before, &file.metadata()?),
        "input was replaced or is not a regular file"
    );
    Ok(file)
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
    #[test]
    #[cfg(unix)]
    fn regular_json_rejects_links_permissions_and_size_before_read() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        std::fs::write(&path, b"{\"n\":1}").unwrap();
        assert_eq!(read_json::<serde_json::Value>(&path).unwrap()["n"], 1);
        assert!(read_json::<serde_json::Value>(dir.path()).is_err());
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(read_json::<serde_json::Value>(&link).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(read_json::<serde_json::Value>(&path)
            .unwrap_err()
            .to_string()
            .contains("owned"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(32 * 1024 * 1024 + 1)
            .unwrap();
        assert!(read_json::<serde_json::Value>(&path)
            .unwrap_err()
            .to_string()
            .contains("32 MiB"));
    }
    #[test]
    #[cfg(unix)]
    fn fifo_input_and_regular_to_fifo_replacement_finish_within_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "offline_products::io::tests::fifo_reader_child",
                "--nocapture",
            ])
            .env("TASK4_FIFO_CHILD_DIR", dir.path())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("JSON FIFO reader blocked beyond deadline");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[test]
    #[cfg(unix)]
    fn fifo_reader_child() {
        use std::os::unix::ffi::OsStrExt;
        let Some(dir) = std::env::var_os("TASK4_FIFO_CHILD_DIR") else {
            return;
        };
        let path = std::path::PathBuf::from(dir).join("fifo");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: a valid terminated path to a private temporary fixture.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read_json::<serde_json::Value>(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"{}").unwrap();
        let before = std::fs::symlink_metadata(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(open_regular_json(&path, &before).is_err());
    }
}
