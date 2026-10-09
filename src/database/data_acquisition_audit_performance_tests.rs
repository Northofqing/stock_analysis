// Private deterministic fixtures. Never opens a production manager or calls a provider.
use super::*;

pub(super) fn seed(path: &std::path::Path, ids: &[i64]) -> String {
    assert!(!path.exists(), "fixture must be new");
    let mut schema = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    create_schema(&mut schema).unwrap();
    drop(schema);
    let mut db = rusqlite::Connection::open(path).unwrap();
    let tx = db.transaction().unwrap();
    let mut previous = AUDIT_CHAIN_GENESIS.to_owned();
    for &id in ids {
        let audit = PersistedAcquisitionAudit {
            id,
            schema_version: 1,
            capability: "TEST_CODE_A01".into(),
            provider: "TEST_CODE_provider".into(),
            source: "TEST_CODE_fixture".into(),
            request_hash: "a".repeat(64),
            source_at: (id % 2 == 0).then(|| "2026-10-08".into()),
            observed_at: "2026-10-08T15:00:00+08:00".into(),
            batch_id: Some(format!("TEST_CODE_batch_{id}")),
            outcome: "available".into(),
            request_count: 1,
            accepted_count: 1,
            rejected_count: 0,
            reason_code: "TEST_CODE_ok".into(),
            retryable: 0,
            created_at: "2026-10-08T07:00:00.000Z".into(),
        };
        let hash = calculate_record_hash(&previous, &audit).unwrap();
        tx.execute("INSERT INTO data_acquisition_audit VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            rusqlite::params![audit.id,audit.schema_version,audit.capability,audit.provider,audit.source,audit.request_hash,audit.source_at,audit.observed_at,audit.batch_id,audit.outcome,audit.request_count,audit.accepted_count,audit.rejected_count,audit.reason_code,audit.retryable,audit.created_at]).unwrap();
        tx.execute(
            "INSERT INTO data_acquisition_audit_chain VALUES (?1,?2,?3,'2026-10-08T07:00:00.000Z')",
            rusqlite::params![id, previous, hash],
        )
        .unwrap();
        previous = hash;
    }
    tx.commit().unwrap();
    previous
}

/// Compile once with `cargo test --lib ... --no-run`; invoke the preserved executable.
/// Generate and measure in separate processes. Read-only/query-only measurement.
#[test]
#[ignore = "opt-in TEST_CODE fixture generation/measurement; see docs/performance.md"]
fn performance_fixture() {
    let path = std::path::PathBuf::from(std::env::var("AUDIT_FIXTURE").expect("AUDIT_FIXTURE"));
    assert!(path.is_absolute() && path.starts_with(std::env::temp_dir()));
    assert!(path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("TEST_CODE_"));
    let mode = std::env::var("AUDIT_MODE").expect("AUDIT_MODE generate|measure|payload");
    if mode == "generate" {
        let rows: i64 = std::env::var("AUDIT_ROWS").unwrap().parse().unwrap();
        assert!((1..=1_000_000).contains(&rows));
        let tail = seed(&path, &(1..=rows).collect::<Vec<_>>());
        println!(
            "{}",
            serde_json::json!({"rows": rows, "tail": tail, "generator": 1})
        );
        return;
    }
    let uri = format!("file:{}?mode=ro", path.to_str().unwrap());
    let mut conn = SqliteConnection::establish(&uri).unwrap();
    diesel::sql_query("PRAGMA query_only=ON")
        .execute(&mut conn)
        .unwrap();
    let rows = diesel::sql_query("SELECT COUNT(*) AS count FROM data_acquisition_audit")
        .get_result::<CountRow>(&mut conn)
        .unwrap()
        .count;
    if mode == "payload" {
        // Exact live string lengths + row structs, excluding capacity/allocator/SQLite/serialization.
        let audits = load_audit_rows(&mut conn).unwrap();
        let chain = load_chain_rows(&mut conn).unwrap();
        let sizes: Vec<usize> = audits
            .iter()
            .zip(&chain)
            .map(|(a, c)| {
                std::mem::size_of_val(a)
                    + std::mem::size_of_val(c)
                    + [
                        &a.capability,
                        &a.provider,
                        &a.source,
                        &a.request_hash,
                        &a.observed_at,
                        &a.outcome,
                        &a.reason_code,
                        &a.created_at,
                        &c.previous_hash,
                        &c.record_hash,
                    ]
                    .iter()
                    .map(|s| s.len())
                    .sum::<usize>()
                    + a.source_at.as_ref().map_or(0, String::len)
                    + a.batch_id.as_ref().map_or(0, String::len)
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({"rows":rows,"owned_payload_bytes": sizes.iter().sum::<usize>(),"max_1024_row_payload_bytes":sizes.chunks(1024).map(|p|p.iter().sum::<usize>()).max()})
        );
        return;
    }
    assert_eq!(mode, "measure");
    for run in 0..5 {
        let started = std::time::Instant::now();
        let tail = validate_data_acquisition_audit_chain(&mut conn).unwrap();
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
        println!(
            "{}",
            serde_json::json!({"run":run,"rows":rows,"elapsed_ms":elapsed_ms,"tail":std::hint::black_box(tail)})
        );
    }
}
