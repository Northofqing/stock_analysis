#![cfg(unix)]

use super::*;
use crate::monitor::alert_log::{AlertLog, G5bDateFence};
use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

const RAW: &str = "{\"origin\":\"production\",\"triggered_at\":\"2026-10-02T15:00:00+08:00\",\"code\":\"600001\",\"name\":\"fixture\",\"level\":\"重要\",\"category\":\"fixture\",\"message\":\"same\",\"t1_locked\":false}\n";

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()
}

#[derive(Serialize, Deserialize)]
struct FixtureHead {
    version: u8,
    business_date: NaiveDate,
    generation: u64,
    committed_offset: u64,
    prefix_sha256: String,
    source_identity: Option<SourceIdentity>,
}

fn canonical_head(head: &FixtureHead) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(head).unwrap();
    bytes.push(b'\n');
    bytes
}

struct Fixture {
    _root: tempfile::TempDir,
    log: AlertLog,
    source: PathBuf,
    head: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let log = AlertLog::for_test(root.path()).unwrap();
        log.initialize_date_input_head(date()).unwrap();
        Self {
            source: root.path().join("20261002.jsonl"),
            head: root.path().join("20261002.input-head.v1.json"),
            log,
            _root: root,
        }
    }

    // Original Production-tag fixture bytes in an isolated Test namespace.
    // The actual writer derives and publishes the head/dev/inode/hash.
    fn append_lines(&self, lines: &[Vec<u8>]) {
        for line in lines {
            self.log
                .append_test_date_raw_production_fixture(date(), line)
                .unwrap();
        }
    }

    fn write_head(&self, head: &FixtureHead) {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&self.head)
            .unwrap();
        file.write_all(&canonical_head(head)).unwrap();
        file.sync_all().unwrap();
    }

    fn fence(&self) -> G5bDateFence {
        self.log.acquire_date_writer_fence(date()).unwrap()
    }

    fn candidate(&self) -> G5bSelectionV2Candidate {
        let fence = self.fence();
        let prefix = self
            .log
            .inspect_date_input_prefix_locked(date(), &fence)
            .unwrap();
        G5bSelectionV2Candidate::from_locked_prefix(&prefix).unwrap()
    }
}

fn raw_record(code: &str, level: &str, triggered_at: &str) -> Vec<u8> {
    let mut record: AlertRecord = serde_json::from_str(RAW).unwrap();
    record.code = code.to_owned();
    record.level = level.to_owned();
    record.triggered_at = triggered_at.to_owned();
    let mut bytes = serde_json::to_vec(&record).unwrap();
    bytes.push(b'\n');
    bytes
}

#[test]
fn g5b_selection_v2_raw_lf_offsets_and_identical_lines_remain_distinct() {
    let fixture = Fixture::new();
    fixture.append_lines(&[RAW.as_bytes().to_vec(), RAW.as_bytes().to_vec()]);
    let fence = fixture.fence();
    let prefix = fixture
        .log
        .inspect_date_input_prefix_locked(date(), &fence)
        .unwrap();
    let candidate = G5bSelectionV2Candidate::from_locked_prefix(&prefix).unwrap();
    assert_eq!(candidate.selected().len(), 2);
    let first = &candidate.selected()[0];
    let second = &candidate.selected()[1];
    assert_eq!(
        (first.ordinal(), first.start_offset(), first.end_offset()),
        (1, 0, 173)
    );
    assert_eq!(
        (second.ordinal(), second.start_offset(), second.end_offset()),
        (2, 173, 346)
    );
    assert_eq!(first.raw_bytes(), RAW.as_bytes());
    assert_eq!(first.raw_bytes(), second.raw_bytes());
    // Independent Python/hashlib vector for the original LF-inclusive bytes.
    assert_eq!(
        first.raw_sha256(),
        "b7f57d725d68d1f5e6ca2b6ac64cf60f59c191bddaad68f41aead2781620a5cb"
    );
    assert_eq!(first.raw_sha256(), second.raw_sha256());
    assert_ne!(first.identity(), second.identity());
    assert_ne!(
        first.raw_bytes(),
        serde_json::to_vec(first.record()).unwrap()
    );
    let cutoff = prefix.current_cutoff().unwrap();
    assert_eq!(cutoff.head().generation(), 2);
    assert_eq!(cutoff.head().committed_offset(), 346);
    let physical = fs::metadata(&fixture.source).unwrap();
    assert_eq!(
        cutoff.source_identity(),
        Some((physical.dev(), physical.ino()))
    );
    assert_eq!(cutoff.head_canonical(), fs::read(&fixture.head).unwrap());
    candidate
        .verify_encoding_against_locked_prefix(&prefix, candidate.canonical_bytes())
        .unwrap();
}

#[test]
fn g5b_selection_v2_priority_matches_existing_top3_and_append_order() {
    let fixture = Fixture::new();
    let lines = [
        raw_record("600001", "重要", "2026-10-02T15:19:00+08:00"),
        raw_record("600002", "重要", "2026-10-02T10:00:00+08:00"),
        raw_record("600003", "紧急", "2026-10-02T14:00:00+08:00"),
        raw_record("600004", "信息", "2026-10-02T09:00:00+08:00"),
    ];
    fixture.append_lines(&lines);
    let candidate = fixture.candidate();
    assert_eq!(
        candidate
            .selected()
            .iter()
            .map(|row| row.ordinal())
            .collect::<Vec<_>>(),
        vec![3, 1, 2]
    );
    let records = lines
        .iter()
        .map(|bytes| serde_json::from_slice(bytes).unwrap())
        .collect();
    let old =
        crate::monitor::attribution_deep::top_events_for_deep(records, DEEP_ATTRIBUTION_MAX_EVENTS);
    assert_eq!(
        candidate
            .selected()
            .iter()
            .map(|row| row.record().code.clone())
            .collect::<Vec<_>>(),
        old.iter().map(|row| row.code.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn g5b_selection_v2_valid_suffix_does_not_replace_existing_cutoff_or_top3() {
    let fixture = Fixture::new();
    let mut lines = vec![RAW.as_bytes().to_vec(), RAW.as_bytes().to_vec()];
    fixture.append_lines(&lines);
    let original = fixture.candidate();
    let before_bytes = original.canonical_bytes().to_vec();
    let before_cohort = original.cohort_identity().to_owned();
    lines.push(raw_record("600099", "紧急", "2026-10-02T15:20:00+08:00"));
    fixture.append_lines(&[lines.last().unwrap().clone()]);
    let fence = fixture.fence();
    let prefix = fixture
        .log
        .inspect_date_input_prefix_locked(date(), &fence)
        .unwrap();
    assert_eq!(prefix.current_cutoff().unwrap().head().generation(), 3);
    original
        .verify_encoding_against_locked_prefix(&prefix, &before_bytes)
        .unwrap();
    assert_eq!(original.cohort_identity(), before_cohort);
    assert_eq!(
        original
            .selected()
            .iter()
            .map(|row| row.ordinal())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let current_candidate = G5bSelectionV2Candidate::from_locked_prefix(&prefix).unwrap();
    assert_ne!(current_candidate.cohort_identity(), before_cohort);
    assert_eq!(current_candidate.selected()[0].ordinal(), 3);
}

#[test]
fn g5b_selection_v2_encoded_anchor_and_closed_policy_mutations_fail() {
    let fixture = Fixture::new();
    fixture.append_lines(&[RAW.as_bytes().to_vec()]);
    let candidate = fixture.candidate();
    let fence = fixture.fence();
    let prefix = fixture
        .log
        .inspect_date_input_prefix_locked(date(), &fence)
        .unwrap();
    for mutation in 0..14 {
        let mut encoded = candidate.encoded.clone();
        match mutation {
            0 => encoded.cutoff.generation += 1,
            1 => encoded.cutoff.committed_offset += 1,
            2 => encoded.cutoff.prefix_sha256 = "0".repeat(64),
            3 => encoded.cutoff.source_identity.as_mut().unwrap().inode += 1,
            4 => encoded.cutoff.input_head_canonical[0] ^= 1,
            5 => encoded.cutoff.input_head_sha256 = "0".repeat(64),
            6 => encoded.selected[0].line_ordinal = 0,
            7 => encoded.selected[0].end_offset += 1,
            8 => encoded.selected[0].raw_line_bytes[0] ^= 1,
            9 => encoded.selected[0].raw_line_sha256 = "0".repeat(64),
            10 => encoded.selected[0].record_canonical[0] ^= 1,
            11 => encoded.selected[0].record_sha256 = "0".repeat(64),
            12 => encoded.selection_policy = "g5b-sort-by-event-time".to_owned(),
            13 => encoded.cutoff_policy = "g5b-latest-prefix".to_owned(),
            _ => unreachable!(),
        }
        let mut bytes = serde_json::to_vec(&encoded).unwrap();
        bytes.push(b'\n');
        assert!(
            candidate
                .verify_encoding_against_locked_prefix(&prefix, &bytes)
                .is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn g5b_selection_v2_codec_rejects_unknown_duplicate_and_noncanonical_json() {
    let fixture = Fixture::new();
    fixture.append_lines(&[RAW.as_bytes().to_vec()]);
    let candidate = fixture.candidate();
    let fence = fixture.fence();
    let prefix = fixture
        .log
        .inspect_date_input_prefix_locked(date(), &fence)
        .unwrap();
    let original = String::from_utf8(candidate.canonical_bytes().to_vec()).unwrap();
    let variants = [
        original.replacen('{', "{\"unknown\":0,", 1),
        original.replacen(
            "\"schema\":",
            "\"schema\":\"g5b-selection-v2\",\"schema\":",
            1,
        ),
        original.replacen("\"cutoff\":{", "\"cutoff\":{\"unknown\":0,", 1),
        original.replacen("\"generation\":1", "\"generation\":1,\"generation\":1", 1),
        original.replacen(
            "\"source_identity\":{",
            "\"source_identity\":{\"unknown\":0,",
            1,
        ),
        original.replacen("\"selected\":[{", "\"selected\":[{\"unknown\":0,", 1),
        original.replacen(
            "\"line_ordinal\":1",
            "\"line_ordinal\":1,\"line_ordinal\":1",
            1,
        ),
        format!("{original}\n"),
        original.trim_end().to_owned(),
        original.replacen('{', "{ ", 1),
    ];
    for (index, bytes) in variants.iter().enumerate() {
        assert!(
            candidate
                .verify_encoding_against_locked_prefix(&prefix, bytes.as_bytes())
                .is_err(),
            "variant {index}"
        );
    }
}

#[test]
fn g5b_selection_v2_actual_head_corruption_remains_unknown() {
    for mutation in 0..4 {
        let fixture = Fixture::new();
        fixture.append_lines(&[RAW.as_bytes().to_vec()]);
        let original = fixture.candidate();
        let mut head: FixtureHead =
            serde_json::from_slice(&fs::read(&fixture.head).unwrap()).unwrap();
        match mutation {
            0 => head.generation += 1,
            1 => head.committed_offset += 1,
            2 => head.source_identity.as_mut().unwrap().inode += 1,
            3 => head.prefix_sha256 = "0".repeat(64),
            _ => unreachable!(),
        }
        fixture.write_head(&head);
        let fence = fixture.fence();
        assert!(
            fixture
                .log
                .inspect_date_input_prefix_locked(date(), &fence)
                .is_err(),
            "mutation {mutation}"
        );
        assert_eq!(original.selected().len(), 1); // never reclassify as Empty
    }
}

#[test]
fn g5b_selection_v2_source_replacement_and_changes_under_held_guard_fail() {
    let fixture = Fixture::new();
    fixture.append_lines(&[RAW.as_bytes().to_vec()]);
    let original = fixture.candidate();
    fs::rename(&fixture.source, fixture.source.with_extension("retained")).unwrap();
    fs::write(&fixture.source, RAW).unwrap(); // same bytes, new actual inode
    let fence = fixture.fence();
    assert!(matches!(
        fixture.log.inspect_date_input_prefix_locked(date(), &fence),
        Err(AlertInputHeadUnknown::SourceIdentityMismatch)
    ));
    drop(fence);
    let replacement = Fixture::new();
    replacement.append_lines(&[RAW.as_bytes().to_vec()]);
    let fence = replacement.fence();
    let prefix = replacement
        .log
        .inspect_date_input_prefix_locked(date(), &fence)
        .unwrap();
    assert!(original
        .verify_encoding_against_locked_prefix(&prefix, original.canonical_bytes())
        .is_err());
    let current = G5bSelectionV2Candidate::from_locked_prefix(&prefix).unwrap();
    assert_ne!(current.cohort_identity(), original.cohort_identity());
    let changed = raw_record("600009", "紧急", "2026-10-02T15:01:00+08:00");
    fs::write(&replacement.source, changed).unwrap(); // hostile change while held
    assert!(G5bSelectionV2Candidate::from_locked_prefix(&prefix).is_err());
    assert!(current
        .verify_encoding_against_locked_prefix(&prefix, current.canonical_bytes())
        .is_err());
}

#[test]
fn g5b_selection_v2_only_matching_fence_and_namespace_can_capture() {
    let first = Fixture::new();
    let second = Fixture::new();
    first.append_lines(&[RAW.as_bytes().to_vec()]);
    second.append_lines(&[RAW.as_bytes().to_vec()]);
    let fence = first.fence();
    assert!(second
        .log
        .inspect_date_input_prefix_locked(date(), &fence)
        .is_err());
    let next_date = date().succ_opt().unwrap();
    assert!(first
        .log
        .inspect_date_input_prefix_locked(next_date, &fence)
        .is_err());
    // Permission rejection precedes production filesystem access.
    assert!(matches!(
        AlertLog::production().inspect_date_input_prefix_locked(date(), &fence),
        Err(AlertInputHeadUnknown::AccessDenied)
    ));
    assert_eq!(
        AlertLog::production()
            .append_test_date_raw_production_fixture(date(), RAW.as_bytes())
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let prefix = first
        .log
        .inspect_date_input_prefix_locked(date(), &fence)
        .unwrap();
    assert!(G5bSelectionV2Candidate::from_locked_prefix(&prefix).is_ok());
}

#[test]
fn g5b_selection_v2_legacy_record_extension_retains_raw_without_schema_promotion() {
    let fixture = Fixture::new();
    let extended = RAW.replacen('{', "{\"future_alert_fact\":\"preserved\",", 1);
    fixture.append_lines(&[extended.as_bytes().to_vec()]);
    let candidate = fixture.candidate();
    let selected = &candidate.selected()[0];
    assert_eq!(selected.raw_bytes(), extended.as_bytes());
    assert_eq!(selected.raw_sha256(), hash(extended.as_bytes()));
    assert_eq!(selected.record().code, "600001");
    // AlertRecord's existing compatibility ignores this extension in the
    // decoded canonical facts; v2 keeps the actual source evidence separately.
    let decoded =
        String::from_utf8(candidate.encoded.selected[0].record_canonical.clone()).unwrap();
    assert!(!decoded.contains("future_alert_fact"));
    let fence = fixture.fence();
    let prefix = fixture
        .log
        .inspect_date_input_prefix_locked(date(), &fence)
        .unwrap();
    candidate
        .verify_encoding_against_locked_prefix(&prefix, candidate.canonical_bytes())
        .unwrap();
}

#[test]
fn g5b_selection_v2_zero_missing_prehead_and_bad_lf_never_make_completion() {
    let fixture = Fixture::new();
    let fence = fixture.fence();
    let prefix = fixture
        .log
        .inspect_date_input_prefix_locked(date(), &fence)
        .unwrap();
    assert!(matches!(
        G5bSelectionV2Candidate::from_locked_prefix(&prefix),
        Err(G5bSelectionV2Error::NoEligibleInput)
    ));
    drop(prefix);
    drop(fence);
    fs::remove_file(&fixture.head).unwrap();
    fs::write(&fixture.source, RAW).unwrap();
    let fence = fixture.fence();
    assert!(matches!(
        fixture.log.inspect_date_input_prefix_locked(date(), &fence),
        Err(AlertInputHeadUnknown::MissingHead)
    ));
    drop(fence);
    assert!(fixture.log.initialize_date_input_head(date()).is_err());
    // Deliberately corrupt the fixture head too: a matching hash cannot
    // promote an unterminated line. This is never positive evidence.
    let malformed = RAW.trim_end().as_bytes().to_vec();
    fs::write(&fixture.source, &malformed).unwrap();
    let physical = fs::metadata(&fixture.source).unwrap();
    fixture.write_head(&FixtureHead {
        version: 1,
        business_date: date(),
        generation: 1,
        committed_offset: malformed.len() as u64,
        prefix_sha256: hash(&malformed),
        source_identity: Some(SourceIdentity {
            device: physical.dev(),
            inode: physical.ino(),
        }),
    });
    let fence = fixture.fence();
    assert!(matches!(
        fixture.log.inspect_date_input_prefix_locked(date(), &fence),
        Err(AlertInputHeadUnknown::TruncatedFinalLine)
    ));
}

#[test]
fn g5b_selection_v2_codec_hash_golden_is_untrusted_data_not_a_capability() {
    // A pure codec golden with fixed synthetic identity. It never constructs
    // LockedAlertInputPrefix, VerifiedAlertInputCutoff, or a trusted Candidate.
    let raw = RAW.as_bytes();
    let record: AlertRecord = serde_json::from_slice(raw).unwrap();
    let canonical_record = serde_json::to_vec(&record).unwrap();
    let head = FixtureHead {
        version: 1,
        business_date: date(),
        generation: 2,
        committed_offset: 346,
        prefix_sha256: "a5cd44eb766d7b2e4f89e5988307813e353d6c33755f2bd0bc250605184b3a4e"
            .to_owned(),
        source_identity: Some(SourceIdentity {
            device: 1,
            inode: 2,
        }),
    };
    let head_bytes = canonical_head(&head);
    assert_eq!(
        hash(&head_bytes),
        "0a28ba1e3f70dc0165cfac4db8803b7a8bf2859e4e70e240bf2ff9c843139924"
    );
    assert_eq!(
        hash(&canonical_record),
        "c873a0fcfb83f262793b20366bc1180954f157603b03d1da5fa1c2c043c265ff"
    );
    let selected = (0..2)
        .map(|index| SelectedLine {
            line_ordinal: index + 1,
            start_offset: index * 173,
            end_offset: (index + 1) * 173,
            raw_line_bytes: raw.to_vec(),
            raw_line_sha256: hash(raw),
            record_canonical: canonical_record.clone(),
            record_sha256: hash(&canonical_record),
        })
        .collect();
    let encoded = EncodedSelection {
        schema: SCHEMA.to_owned(),
        business_date: date(),
        selection_policy: SELECTION_POLICY.to_owned(),
        selection_policy_sha256: hash(SELECTION_POLICY.as_bytes()),
        cutoff_policy: CUTOFF_POLICY.to_owned(),
        cutoff_policy_sha256: hash(CUTOFF_POLICY.as_bytes()),
        cutoff: Cutoff {
            input_head_sha256: hash(&head_bytes),
            input_head_canonical: head_bytes,
            generation: 2,
            committed_offset: 346,
            prefix_sha256: head.prefix_sha256,
            source_identity: head.source_identity,
        },
        selected,
    };
    let mut bytes = serde_json::to_vec(&encoded).unwrap();
    bytes.push(b'\n');
    let cohort = domain_hash(COHORT_DOMAIN, &bytes);
    assert_eq!(
        cohort,
        "f15649927103a21844a22fa08a94a27f4de4d9d3bdbce49ad36f296a349df536"
    );
    let parsed: EncodedSelection = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed, encoded);
    for (index, expected) in [
        "c1a9339198ef5ed21c6ecb6d6d1c5ae054f0dc9699a9fc66bd00e16b3914ef5c",
        "ade7062f992b7eebd6e42637e2f9043934c8e9d5c2bf176f3e96763adfb2c0cb",
    ]
    .into_iter()
    .enumerate()
    {
        let line = &encoded.selected[index];
        let preimage = OccurrencePreimage {
            business_date: date(),
            cohort_identity: &cohort,
            source_identity: &encoded.cutoff.source_identity,
            line_ordinal: line.line_ordinal,
            start_offset: line.start_offset,
            end_offset: line.end_offset,
            raw_line_sha256: &line.raw_line_sha256,
        };
        assert_eq!(
            domain_hash(OCCURRENCE_DOMAIN, &serde_json::to_vec(&preimage).unwrap()),
            expected
        );
    }
}
