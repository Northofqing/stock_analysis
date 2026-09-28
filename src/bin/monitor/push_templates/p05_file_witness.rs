//! Same-read identity for the four legacy P5 JSONL candidate inputs.
//! Declared metadata is retained as observed; file mtime is never a source clock.

use sha2::{Digest, Sha256};
use stock_analysis::opportunity::candidate_panel::{CandidateEntry, CandidateSource};

type SourceItem = (CandidateSource, String, String);

const SOURCE_NAMES: [&str; 4] = [
    "stock_pick",
    "optimal_close",
    "volume_watchlist",
    "volume_real_trade",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum P5FileMetadataState {
    /// The file declares one consistent time and version; producer authority
    /// and the other P-05 origins still need separate verification.
    DeclaredMetadataPresent,
    UnqualifiedEmptyFile,
    UnqualifiedMissingGeneratedAt,
    UnqualifiedInvalidGeneratedAt,
    UnqualifiedMissingSelectionVersion,
    UnqualifiedInconsistentMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P5SourceRowWitness {
    pub(super) physical_line: usize,
    pub(super) raw_line_sha256: String,
    pub(super) code: String,
    pub(super) name: String,
    pub(super) generated_at: Option<String>,
    pub(super) selection_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P5SourceFileWitness {
    pub(super) source: CandidateSource,
    /// Stable logical identity, independent of the runtime base directory.
    pub(super) file_identity: String,
    pub(super) raw_file_sha256: String,
    pub(super) generated_at: Option<String>,
    pub(super) selection_version: Option<String>,
    pub(super) metadata_state: P5FileMetadataState,
    pub(super) rows: Vec<P5SourceRowWitness>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P5CandidateFileRef {
    pub(super) code: String,
    pub(super) source: CandidateSource,
    pub(super) file_identity: String,
    pub(super) raw_file_sha256: String,
    pub(super) physical_line: usize,
    pub(super) raw_line_sha256: String,
}

#[derive(Debug)]
pub(super) struct P5SourceFiles {
    pub(super) items: Vec<SourceItem>,
    pub(super) witnesses: Vec<P5SourceFileWitness>,
}

fn source_for_name(name: &str) -> Result<CandidateSource, String> {
    match name {
        "stock_pick" => Ok(CandidateSource::StockPick),
        "optimal_close" => Ok(CandidateSource::OptimalClose),
        "volume_watchlist" => Ok(CandidateSource::VolumeWatchlist),
        "volume_real_trade" => Ok(CandidateSource::VolumeRealTrade),
        _ => Err(format!("未知 P5 候选来源: {name}")),
    }
}

fn declared_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn file_metadata(
    rows: &[P5SourceRowWitness],
) -> (Option<String>, Option<String>, P5FileMetadataState) {
    let Some(first) = rows.first() else {
        return (None, None, P5FileMetadataState::UnqualifiedEmptyFile);
    };
    let generated_at = rows
        .iter()
        .all(|row| row.generated_at == first.generated_at)
        .then(|| first.generated_at.clone())
        .flatten();
    let selection_version = rows
        .iter()
        .all(|row| row.selection_version == first.selection_version)
        .then(|| first.selection_version.clone())
        .flatten();
    let state = if rows.iter().any(|row| row.generated_at.is_none()) {
        P5FileMetadataState::UnqualifiedMissingGeneratedAt
    } else if rows.iter().any(|row| {
        row.generated_at.as_deref().is_some_and(|value| {
            value.trim() != value || chrono::DateTime::parse_from_rfc3339(value).is_err()
        })
    }) {
        P5FileMetadataState::UnqualifiedInvalidGeneratedAt
    } else if rows.iter().any(|row| {
        row.selection_version
            .as_deref()
            .map_or(true, |value| value.is_empty() || value.trim() != value)
    }) {
        P5FileMetadataState::UnqualifiedMissingSelectionVersion
    } else if rows.iter().any(|row| {
        row.generated_at != first.generated_at || row.selection_version != first.selection_version
    }) {
        P5FileMetadataState::UnqualifiedInconsistentMetadata
    } else {
        P5FileMetadataState::DeclaredMetadataPresent
    };
    (generated_at, selection_version, state)
}

/// Read each file once, hash its original bytes, and derive items and row
/// identities from that same buffer. Missing legacy files remain empty inputs.
pub(super) fn load_one_from_dir(
    source_name: &str,
    base_dir: &std::path::Path,
) -> Result<P5SourceFiles, String> {
    use std::io::ErrorKind;

    let source = source_for_name(source_name)?;
    let path = base_dir.join(format!("{source_name}.jsonl"));
    let raw = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(P5SourceFiles {
                items: Vec::new(),
                witnesses: Vec::new(),
            });
        }
        Err(error) => return Err(format!("读取 P5 候选源 {} 失败: {error}", path.display())),
    };
    let raw_file_sha256 = hex::encode(Sha256::digest(&raw));
    // The old reader required valid UTF-8 for the entire file, including lines
    // it would skip. Preserve that admission boundary before splitting bytes.
    std::str::from_utf8(&raw)
        .map_err(|error| format!("读取 P5 候选源 {} 失败: {error}", path.display()))?;

    #[derive(serde::Deserialize)]
    struct P5Item {
        code: String,
        name: String,
    }

    let mut items = Vec::new();
    let mut rows = Vec::new();
    for (line_index, raw_line) in raw.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let line_bytes = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
        let line_bytes = line_bytes.strip_suffix(b"\r").unwrap_or(line_bytes);
        let line = std::str::from_utf8(line_bytes).expect("whole file UTF-8 checked above");
        if line.trim().is_empty() {
            continue;
        }
        let item = serde_json::from_str::<P5Item>(line).map_err(|error| {
            format!(
                "P5 候选源 {path} 第 {} 行 JSON 非法: {error}",
                line_index + 1,
                path = path.display()
            )
        })?;
        let code = item.code.trim();
        let name = item.name.trim();
        if !super::valid_source_stock_code(code) {
            return Err(format!(
                "P5 候选源 {} 第 {} 行 code 非法: {}",
                path.display(),
                line_index + 1,
                item.code
            ));
        }
        if name.is_empty() {
            return Err(format!(
                "P5 候选源 {} 第 {} 行 name 为空",
                path.display(),
                line_index + 1
            ));
        }
        let metadata: serde_json::Value =
            serde_json::from_str(line).expect("P5Item JSON parsed above");
        rows.push(P5SourceRowWitness {
            physical_line: line_index + 1,
            raw_line_sha256: hex::encode(Sha256::digest(raw_line)),
            code: code.to_string(),
            name: name.to_string(),
            generated_at: declared_string(&metadata, "generated_at"),
            selection_version: declared_string(&metadata, "selection_version"),
        });
        items.push((source, code.to_string(), name.to_string()));
    }
    let (generated_at, selection_version, metadata_state) = file_metadata(&rows);
    Ok(P5SourceFiles {
        items,
        witnesses: vec![P5SourceFileWitness {
            source,
            file_identity: format!("data/p5_sources/{source_name}.jsonl"),
            raw_file_sha256,
            generated_at,
            selection_version,
            metadata_state,
            rows,
        }],
    })
}

pub(super) fn load_all_from_dir(base_dir: &std::path::Path) -> Result<P5SourceFiles, String> {
    let mut all = P5SourceFiles {
        items: Vec::new(),
        witnesses: Vec::new(),
    };
    for name in SOURCE_NAMES {
        let one = load_one_from_dir(name, base_dir)?;
        all.items.extend(one.items);
        all.witnesses.extend(one.witnesses);
    }
    Ok(all)
}

/// Match the rows read above to the merged candidates from that same read.
/// Each ref retains its file hash and physical line, including duplicate rows.
pub(super) fn link_candidates(
    entries: &[CandidateEntry],
    witnesses: &[P5SourceFileWitness],
) -> Result<Vec<P5CandidateFileRef>, String> {
    let mut refs = Vec::new();
    for witness in witnesses {
        for row in &witness.rows {
            if !entries
                .iter()
                .any(|entry| entry.code == row.code && entry.sources.contains(&witness.source))
            {
                return Err(format!(
                    "P-05 file row {}:{} has no same-read merged candidate {}",
                    witness.file_identity, row.physical_line, row.code
                ));
            }
            refs.push(P5CandidateFileRef {
                code: row.code.clone(),
                source: witness.source,
                file_identity: witness.file_identity.clone(),
                raw_file_sha256: witness.raw_file_sha256.clone(),
                physical_line: row.physical_line,
                raw_line_sha256: row.raw_line_sha256.clone(),
            });
        }
    }
    Ok(refs)
}

/// Carry all consumed file witnesses into the final batch, while linking only
/// candidates that survived the existing hard gates and ordering step.
pub(super) fn selected_refs(
    selected_entries: &[CandidateEntry],
    refs: Vec<P5CandidateFileRef>,
) -> Vec<P5CandidateFileRef> {
    let selected_codes: std::collections::HashSet<&str> = selected_entries
        .iter()
        .map(|entry| entry.code.as_str())
        .collect();
    refs.into_iter()
        .filter(|reference| selected_codes.contains(reference.code.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stock_analysis::opportunity::candidate_panel::merge_candidates;

    const TIME: &str = "2026-09-24T09:30:00+08:00";
    const VERSION: &str = "TEST_CODE_p5-selection-v1";

    fn line(code: &str, name: &str) -> String {
        format!(
            "{{\"code\":\"{code}\",\"name\":\"{name}\",\"generated_at\":\"{TIME}\",\"selection_version\":\"{VERSION}\"}}\n"
        )
    }

    #[test]
    fn raw_file_and_line_ids_follow_actual_bytes_and_same_read_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let first = line("TEST_CODE_600001", "甲");
        let second = line("TEST_CODE_600002", "乙");
        let optimal = line("TEST_CODE_600001", "甲");
        let stock_pick_path = dir.path().join("stock_pick.jsonl");
        std::fs::write(&stock_pick_path, format!("{first}{second}")).unwrap();
        std::fs::write(dir.path().join("optimal_close.jsonl"), optimal).unwrap();

        let loaded = load_all_from_dir(dir.path()).unwrap();
        assert_eq!(loaded.items.len(), 3);
        assert_eq!(loaded.witnesses.len(), 2);
        let stock_pick = &loaded.witnesses[0];
        assert_eq!(stock_pick.file_identity, "data/p5_sources/stock_pick.jsonl");
        assert_eq!(
            stock_pick.metadata_state,
            P5FileMetadataState::DeclaredMetadataPresent
        );
        assert_eq!(stock_pick.generated_at.as_deref(), Some(TIME));
        assert_eq!(stock_pick.selection_version.as_deref(), Some(VERSION));
        assert_eq!(
            stock_pick.raw_file_sha256,
            hex::encode(Sha256::digest(format!("{first}{second}").as_bytes()))
        );
        assert_eq!(stock_pick.rows[0].physical_line, 1);
        assert_eq!(
            stock_pick.rows[0].raw_line_sha256,
            hex::encode(Sha256::digest(first.as_bytes()))
        );
        let merged = merge_candidates(loaded.items.clone());
        let refs = link_candidates(&merged, &loaded.witnesses).unwrap();
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].code, "TEST_CODE_600001");
        assert_eq!(refs[0].source, CandidateSource::StockPick);
        assert_eq!(refs[0].raw_file_sha256, stock_pick.raw_file_sha256);
        assert_eq!(refs[2].code, "TEST_CODE_600001");
        assert_eq!(refs[2].source, CandidateSource::OptimalClose);
        let selected_entries = merged
            .iter()
            .filter(|entry| entry.code == "TEST_CODE_600001")
            .cloned()
            .collect::<Vec<_>>();
        let selected = selected_refs(&selected_entries, refs.clone());
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].raw_file_sha256, stock_pick.raw_file_sha256);
        assert_eq!(
            selected[1].raw_file_sha256,
            loaded.witnesses[1].raw_file_sha256
        );
        assert_eq!(selected_refs(&[], refs.clone()).len(), 0);

        // The witness and candidate refs are from the first read. Replacing the
        // file afterward cannot silently rebind those refs to different bytes.
        std::fs::write(&stock_pick_path, format!("{second}{first}")).unwrap();
        let reordered = load_all_from_dir(dir.path()).unwrap();
        assert_ne!(
            stock_pick.raw_file_sha256,
            reordered.witnesses[0].raw_file_sha256
        );
        assert_eq!(reordered.witnesses[0].rows[0].code, "TEST_CODE_600002");
        assert_eq!(reordered.witnesses[0].rows[1].code, refs[0].code);
        assert_eq!(refs[0].physical_line, 1);
        assert_eq!(refs[0].raw_file_sha256, stock_pick.raw_file_sha256);

        std::fs::write(
            &stock_pick_path,
            format!("{}{first}", line("TEST_CODE_600002", "乙改")),
        )
        .unwrap();
        let tampered = load_all_from_dir(dir.path()).unwrap();
        assert_ne!(
            reordered.witnesses[0].raw_file_sha256,
            tampered.witnesses[0].raw_file_sha256
        );
        assert_ne!(
            reordered.witnesses[0].rows[0].raw_line_sha256,
            tampered.witnesses[0].rows[0].raw_line_sha256
        );
        assert_eq!(
            tampered.witnesses[0].rows[1].raw_line_sha256,
            stock_pick.rows[0].raw_line_sha256
        );
    }

    #[test]
    fn legacy_missing_or_invalid_metadata_stays_unqualified_without_changing_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stock_pick.jsonl");
        std::fs::write(&path, "{\"code\":\"TEST_CODE_600001\",\"name\":\"甲\"}\n").unwrap();
        let legacy = load_one_from_dir("stock_pick", dir.path()).unwrap();
        assert_eq!(legacy.items.len(), 1);
        assert_eq!(
            legacy.witnesses[0].metadata_state,
            P5FileMetadataState::UnqualifiedMissingGeneratedAt
        );
        assert_eq!(legacy.witnesses[0].generated_at, None);
        assert_eq!(legacy.witnesses[0].selection_version, None);

        std::fs::write(
            &path,
            format!(
                "{{\"code\":\"TEST_CODE_600001\",\"name\":\"甲\",\"generated_at\":\"{TIME}\"}}\n"
            ),
        )
        .unwrap();
        let missing_version = load_one_from_dir("stock_pick", dir.path()).unwrap();
        assert_eq!(missing_version.items, legacy.items);
        assert_eq!(
            missing_version.witnesses[0].generated_at.as_deref(),
            Some(TIME)
        );
        assert_eq!(
            missing_version.witnesses[0].metadata_state,
            P5FileMetadataState::UnqualifiedMissingSelectionVersion
        );

        std::fs::write(&path, format!("{{\"code\":\"TEST_CODE_600001\",\"name\":\"甲\",\"generated_at\":\"2026-09-24 09:30:00\",\"selection_version\":\"{VERSION}\"}}\n")).unwrap();
        let invalid_time = load_one_from_dir("stock_pick", dir.path()).unwrap();
        assert_eq!(invalid_time.items, legacy.items);
        assert_eq!(
            invalid_time.witnesses[0].metadata_state,
            P5FileMetadataState::UnqualifiedInvalidGeneratedAt
        );

        let mut merged = merge_candidates(legacy.items);
        merged[0].sources.clear();
        assert!(link_candidates(&merged, &invalid_time.witnesses).is_err());
    }
}
