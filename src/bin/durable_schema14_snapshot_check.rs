//! Read only, sealed-copy preflight; never point this at a live WAL database.
use clap::Parser;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use stock_analysis::durable_delivery::{inspect_schema14_extensions, Schema14ExtensionObservation};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(about = "Check schema14 extensions in an isolated SQLite copy; no migration or approval")]
struct Args {
    /// Completed, stable copy with no WAL, SHM or rollback journal; never a live DB.
    #[arg(long)]
    isolated_snapshot: PathBuf,
}

#[derive(Serialize)]
struct Report {
    snapshot_bytes: u64,
    snapshot_sha256: String,
    observation: Schema14ExtensionObservation,
}

fn identity(meta: &Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
    )
}

fn reject_sidecars(path: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        match std::fs::symlink_metadata(Path::new(&sidecar)) {
            Ok(_) => return Err("snapshot has a sidecar; supply a completed isolated copy".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn file_sha256(file: &mut File) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn inspect(path: &Path) -> Result<Report> {
    if !std::fs::symlink_metadata(path)?.is_file() {
        return Err("snapshot must be a regular file, not a symlink".into());
    }
    let path = path.canonicalize()?;
    reject_sidecars(&path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    let before = identity(&file.metadata()?);
    if identity(&std::fs::symlink_metadata(&path)?) != before {
        return Err("snapshot file identity changed before inspection".into());
    }
    let sha = file_sha256(&mut file)?;
    // immutable avoids creating SQLite WAL/SHM files. Sidecars are rejected:
    // ignoring a live WAL would omit committed facts from the observation.
    let mut uri = url::Url::from_file_path(&path).map_err(|_| "invalid snapshot path")?;
    uri.query_pairs_mut()
        .append_pair("mode", "ro")
        .append_pair("immutable", "1");
    let mut connection = Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let observation = inspect_schema14_extensions(&mut connection)?;
    connection.close().map_err(|(_, error)| error)?;
    reject_sidecars(&path)?;
    if identity(&file.metadata()?) != before
        || identity(&std::fs::symlink_metadata(&path)?) != before
        || file_sha256(&mut file)? != sha
    {
        return Err("snapshot bytes or file identity changed during inspection".into());
    }
    Ok(Report {
        snapshot_bytes: before.2,
        snapshot_sha256: sha,
        observation,
    })
}

fn main() -> Result<()> {
    let args = Args::parse();
    println!(
        "{}",
        serde_json::to_string_pretty(&inspect(&args.isolated_snapshot)?)?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecars_including_dangling_links_are_rejected_without_sqlite_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST_CODE_snapshot.sqlite3");
        std::fs::write(&path, b"TEST_CODE invalid SQLite bytes").unwrap();
        for suffix in ["-wal", "-shm", "-journal"] {
            let sidecar = dir
                .path()
                .join(format!("TEST_CODE_snapshot.sqlite3{suffix}"));
            std::os::unix::fs::symlink("TEST_CODE_missing", &sidecar).unwrap();
            let error = inspect(&path).err().unwrap().to_string();
            assert!(error.contains("sidecar"), "{error}");
            std::fs::remove_file(sidecar).unwrap();
        }
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"TEST_CODE invalid SQLite bytes"
        );
    }

    #[test]
    fn snapshot_leaf_link_is_rejected_without_following() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST_CODE_link");
        let target = dir.path().join("TEST_CODE_original");
        std::fs::write(&target, b"TEST_CODE original").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(inspect(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("symlink"));
        assert_eq!(std::fs::read(target).unwrap(), b"TEST_CODE original");
    }
}
