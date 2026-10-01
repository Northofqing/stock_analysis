use super::*;
use std::sync::{Arc, Barrier};

fn isolated() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(directory.path()).unwrap();
    (directory, path)
}

// Opaque format material, explicitly TEST_CODE and never a Gateway capture.
fn sample() -> ([Vec<u8>; 16], Vec<u8>, ObservedArtifactRef) {
    let mut parts = std::array::from_fn(|index| format!("TEST_CODE_PART_{index}").into_bytes());
    parts[0] = CAPTURE_MATERIAL.to_vec();
    parts[13] = vec![0, 0xff, 0x80, 1, b'\n'];
    parts[14] = br#"{"outcome":"QueryRejected","kind":"Unknown"}"#.to_vec();
    let capture = hash_capture_parts_v1(&parts);
    let bytes = encode_parts(&parts, &capture).unwrap();
    let artifact = reference(&capture, &bytes);
    (parts, bytes, artifact)
}

fn publish(store: &HistoricalObservedStore) -> (ObservedArtifactRef, Publication) {
    let (_, bytes, artifact) = sample();
    store.publish_bytes(&bytes, &artifact).unwrap()
}

#[test]
fn wg06_observed_store_roundtrip_is_byte_preserving_recorded_only_and_exact_idempotent() {
    let (_directory, root) = isolated();
    let store = HistoricalObservedStore::open_existing(&root, &[]).unwrap();
    let (parts, bytes, expected) = sample();
    let (artifact, publication) = publish(&store);
    assert_eq!(publication, Publication::Published);
    assert_eq!(artifact, expected);
    let recorded: StoredObservedEvidence = store.read_checked(&artifact).unwrap();
    assert_eq!(recorded.artifact(), &artifact);
    assert_eq!(recorded.parts, parts);
    assert_eq!(
        recorded.raw_part("raw_status_and_trailer").unwrap(),
        [0, 255, 128, 1, b'\n']
    );
    assert_eq!(recorded.raw_part("typed_query_outcome").unwrap(), parts[14]);
    assert!(recorded.raw_part("LiveCapture").is_none());
    assert_eq!(
        std::fs::read(root.join(artifact.filename())).unwrap(),
        bytes
    );
    assert_eq!(
        publish(&store),
        (artifact.clone(), Publication::ExistingExact)
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(
        std::fs::metadata(root.join(artifact.filename()))
            .unwrap()
            .nlink(),
        1
    );
}

#[test]
fn wg06_observed_store_strict_reader_rejects_promotion_unknown_format_duplicate_and_corrupt_parts()
{
    let (_, bytes, artifact) = sample();
    for mutation in [
        "version",
        "scope",
        "part_name",
        "part_hash",
        "part_length",
        "part_hex",
        "part_count",
        "domain",
    ] {
        let mut file: EvidenceFile = serde_json::from_slice(&bytes).unwrap();
        match mutation {
            "version" => file.version = 2,
            "scope" => file.scope = "LiveCapture".to_owned(),
            "part_name" => file.parts.swap(1, 2),
            "part_hash" => file.parts[13].sha256 = "0".repeat(64),
            "part_length" => file.parts[13].byte_length += 1,
            "part_hex" => file.parts[13].bytes_hex = file.parts[13].bytes_hex.to_uppercase(),
            "part_count" => {
                file.parts.pop();
            }
            "domain" => {
                file.parts[0].bytes_hex = hex::encode(b"TEST_CODE_LiveCapture");
                file.parts[0].byte_length = b"TEST_CODE_LiveCapture".len() as u64;
                file.parts[0].sha256 = digest(b"TEST_CODE_LiveCapture");
            }
            _ => unreachable!(),
        }
        let mut corrupt = serde_json::to_vec(&file).unwrap();
        corrupt.push(b'\n');
        // Even an attacker updating the outer file pin cannot promote scope or
        // substitute inner parts while retaining the original capture identity.
        assert!(
            decode_checked(&corrupt, &reference(&artifact.capture_sha256, &corrupt)).is_err(),
            "{mutation}"
        );
    }
    let text = std::str::from_utf8(&bytes).unwrap();
    for corrupt in [
        text.replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
        text.replacen("\"version\":1", "\"version\":1,\"live_authority\":true", 1),
        format!("{text} "),
        text[..text.len() - 2].to_owned(),
    ] {
        assert!(decode_checked(
            corrupt.as_bytes(),
            &reference(&artifact.capture_sha256, corrupt.as_bytes())
        )
        .is_err());
    }
    let mut invalid_ref = artifact;
    invalid_ref.capture_sha256 = "../TEST_CODE".to_owned();
    assert!(decode_checked(&bytes, &invalid_ref).is_err());
}

#[test]
fn wg06_observed_store_tampering_and_size_limit_never_overwrite_or_adopt_existing_file() {
    let (_directory, root) = isolated();
    let store = HistoricalObservedStore::open_existing(&root, &[]).unwrap();
    let (artifact, _) = publish(&store);
    let path = root.join(artifact.filename());
    std::fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
    let corrupt = b"TEST_CODE_TAMPER";
    std::fs::write(&path, corrupt).unwrap();
    assert!(store.read_checked(&artifact).is_err());
    let (_, expected, _) = sample();
    assert!(store.publish_bytes(&expected, &artifact).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), corrupt);
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(MAX_FILE_BYTES as u64 + 1)
        .unwrap();
    let mut huge = artifact.clone();
    huge.byte_length = MAX_FILE_BYTES as u64 + 1;
    assert!(store.read_checked(&huge).is_err());
    assert!(store.read_checked(&artifact).is_err());
    let mut parts = sample().0;
    parts[1] = vec![0; MAX_PART_BYTES + 1];
    assert!(encode_parts(&parts, &hash_capture_parts_v1(&parts)).is_err());
}

#[test]
fn wg06_observed_store_concurrent_publish_has_one_publication_and_identical_durable_bytes() {
    let (_directory, root) = isolated();
    let barrier = Arc::new(Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let store = HistoricalObservedStore::open_existing(&root, &[]).unwrap();
                barrier.wait();
                let (artifact, publication) = publish(&store);
                assert_eq!(store.read_checked(&artifact).unwrap().parts, sample().0);
                publication
            })
        })
        .collect();
    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| **value == Publication::Published)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| **value == Publication::ExistingExact)
            .count(),
        3
    );
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 1);
}

#[test]
fn wg06_observed_store_rejects_symlink_hardlink_nonregular_and_untrusted_directory() {
    let (_directory, root) = isolated();
    let store = HistoricalObservedStore::open_existing(&root, &[]).unwrap();
    let (_, bytes, artifact) = sample();
    let target = root.join(artifact.filename());
    let other = root.join("TEST_CODE_OTHER");
    std::fs::write(&other, &bytes).unwrap();
    std::fs::set_permissions(&other, Permissions::from_mode(0o400)).unwrap();
    std::os::unix::fs::symlink(&other, &target).unwrap();
    assert!(store.publish_bytes(&bytes, &artifact).is_err());
    assert!(store.read_checked(&artifact).is_err());
    std::fs::remove_file(&target).unwrap();
    std::fs::hard_link(&other, &target).unwrap();
    assert!(store.publish_bytes(&bytes, &artifact).is_err());
    std::fs::remove_file(&target).unwrap();
    std::fs::create_dir(&target).unwrap();
    assert!(store.publish_bytes(&bytes, &artifact).is_err());
    std::fs::remove_dir(&target).unwrap();
    let alias = root.join("TEST_CODE_ALIAS");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    assert!(HistoricalObservedStore::open_existing(&alias, &[]).is_err());
    std::fs::set_permissions(&root, Permissions::from_mode(0o777)).unwrap();
    assert!(HistoricalObservedStore::open_existing(&root, &[]).is_err());
    assert!(store.publish_bytes(&bytes, &artifact).is_err());
}

#[test]
fn wg06_observed_store_namespace_replacement_fails_before_publish_and_after_publish_without_false_success(
) {
    for at in [
        Phase::BeforePublish,
        Phase::AfterPublish,
        Phase::BeforeReturn,
    ] {
        let (_directory, root) = isolated();
        let output = root.join("evidence");
        std::fs::create_dir(&output).unwrap();
        std::fs::set_permissions(&output, Permissions::from_mode(0o700)).unwrap();
        let store = HistoricalObservedStore::open_existing(&output, &[]).unwrap();
        let displaced = root.join("displaced");
        let moved_output = output.clone();
        let moved_displaced = displaced.clone();
        HOOK.with(|hook| {
            *hook.borrow_mut() = Some((
                at,
                Box::new(move || {
                    std::fs::rename(&moved_output, &moved_displaced).unwrap();
                    std::fs::create_dir(&moved_output).unwrap();
                    std::fs::set_permissions(&moved_output, Permissions::from_mode(0o700)).unwrap();
                }),
            ))
        });
        let (_, bytes, artifact) = sample();
        assert!(store.publish_bytes(&bytes, &artifact).is_err());
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
        let published = displaced.join(artifact.filename());
        if at == Phase::BeforePublish {
            assert!(!published.exists());
        } else {
            assert_eq!(std::fs::read(&published).unwrap(), bytes);
            assert_eq!(std::fs::metadata(published).unwrap().nlink(), 1);
        }
        assert!(store.read_checked(&artifact).is_err());
        assert!(std::fs::read_dir(&displaced).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }
}

#[test]
fn wg06_observed_store_identical_leaf_swap_during_read_and_idempotent_return_is_rejected() {
    for at in [Phase::AfterRead, Phase::BeforeReturn] {
        let (_directory, root) = isolated();
        let store = HistoricalObservedStore::open_existing(&root, &[]).unwrap();
        let (artifact, _) = publish(&store);
        let (_, bytes, _) = sample();
        let target = root.join(artifact.filename());
        let replacement = root.join("TEST_CODE_REPLACEMENT");
        std::fs::write(&replacement, &bytes).unwrap();
        std::fs::set_permissions(&replacement, Permissions::from_mode(0o400)).unwrap();
        HOOK.with(|hook| {
            *hook.borrow_mut() = Some((
                at,
                Box::new(move || {
                    std::fs::rename(replacement, target).unwrap();
                }),
            ))
        });
        if at == Phase::AfterRead {
            assert!(store.read_checked(&artifact).is_err());
        } else {
            assert!(store.publish_bytes(&bytes, &artifact).is_err());
        }
    }
}

#[test]
fn wg06_observed_store_rejects_forbidden_namespace_ancestors_aliases_and_missing_roots() {
    let (_directory, root) = isolated();
    let production = root.join("TEST_CODE_PRODUCTION");
    std::fs::create_dir(&production).unwrap();
    std::fs::set_permissions(&production, Permissions::from_mode(0o700)).unwrap();
    let child = production.join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::set_permissions(&child, Permissions::from_mode(0o700)).unwrap();
    for output in [&root, &production, &child] {
        assert!(
            HistoricalObservedStore::open_existing(output, std::slice::from_ref(&production))
                .is_err()
        );
    }
    let alias = root.join("TEST_CODE_PRODUCTION_ALIAS");
    std::os::unix::fs::symlink(&production, &alias).unwrap();
    assert!(
        HistoricalObservedStore::open_existing(&alias, std::slice::from_ref(&production)).is_err()
    );
    assert!(HistoricalObservedStore::open_existing(&root.join("missing"), &[]).is_err());
    assert!(
        HistoricalObservedStore::open_existing(crate::production_root::production_root(), &[])
            .is_err()
    );
    for invalid in [
        "relative",
        "/",
        "/isolated/../evidence",
        "/isolated//evidence",
        "/isolated/evidence/",
    ] {
        assert!(normal_absolute_components(Path::new(invalid)).is_err());
    }
}

#[test]
fn wg06_observed_store_foreign_temporary_replacement_is_never_published_or_cleaned() {
    let (_directory, root) = isolated();
    let store = HistoricalObservedStore::open_existing(&root, &[]).unwrap();
    let moved_root = root.clone();
    let foreign = b"TEST_CODE_FOREIGN_INODE";
    HOOK.with(|hook| {
        *hook.borrow_mut() = Some((
            Phase::BeforePublish,
            Box::new(move || {
                let temporary = std::fs::read_dir(&moved_root)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| path.extension().is_some_and(|extension| extension == "tmp"))
                    .unwrap();
                std::fs::rename(&temporary, moved_root.join("TEST_CODE_OWNED_DISPLACED")).unwrap();
                std::fs::write(&temporary, foreign).unwrap();
                std::fs::set_permissions(&temporary, Permissions::from_mode(0o600)).unwrap();
            }),
        ))
    });
    let (_, bytes, artifact) = sample();
    assert!(store.publish_bytes(&bytes, &artifact).is_err());
    assert!(!root.join(artifact.filename()).exists());
    let temporary = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "tmp"))
        .unwrap();
    assert_eq!(std::fs::read(temporary).unwrap(), foreign);
    assert_eq!(
        std::fs::read(root.join("TEST_CODE_OWNED_DISPLACED")).unwrap(),
        bytes
    );
}

#[test]
fn wg06_observed_store_retained_ancestor_replacement_is_rejected_without_writing_new_namespace() {
    let (_directory, root) = isolated();
    let ancestor = root.join("ancestor");
    let output = ancestor.join("evidence");
    std::fs::create_dir(&ancestor).unwrap();
    std::fs::create_dir(&output).unwrap();
    std::fs::set_permissions(&output, Permissions::from_mode(0o700)).unwrap();
    let store = HistoricalObservedStore::open_existing(&output, &[]).unwrap();
    let moved_ancestor = ancestor.clone();
    let moved_output = output.clone();
    let displaced = root.join("displaced");
    HOOK.with(|hook| {
        *hook.borrow_mut() = Some((
            Phase::BeforePublish,
            Box::new(move || {
                std::fs::rename(&moved_ancestor, &displaced).unwrap();
                std::fs::create_dir(&moved_ancestor).unwrap();
                std::fs::create_dir(&moved_output).unwrap();
                std::fs::set_permissions(&moved_output, Permissions::from_mode(0o700)).unwrap();
            }),
        ))
    });
    let (_, bytes, artifact) = sample();
    assert!(store.publish_bytes(&bytes, &artifact).is_err());
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
    assert!(!root
        .join("displaced/evidence")
        .join(artifact.filename())
        .exists());
}
