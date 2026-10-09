//! Bounded offline observations only. No replay publisher, database, clock or provider.
use chrono::{DateTime, FixedOffset, NaiveDate};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Component, Path},
};
use stock_analysis::{event::envelope::EventEnvelope, risk::stop_loss::check_stops};
const MAX_LINE: u64 = 1_048_576;
const MAX_TOTAL: u64 = 67_108_864;
const MAX_ROWS: u64 = 100_000;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    manifest: std::path::PathBuf,
    #[arg(long)]
    manifest_sha256: String,
    #[arg(long)]
    business_date: NaiveDate,
    #[arg(long)]
    as_of: DateTime<FixedOffset>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    timezone: String,
    business_date: NaiveDate,
    as_of: DateTime<FixedOffset>,
    provenance: String,
    source_contract: String,
    complete: bool,
    file: String,
    sha256: String,
    rows: u64,
    bytes: u64,
    sources: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopInput {
    version: u32,
    code: String,
    name: String,
    current_price: f64,
    cost_price: f64,
    hard_stop: Option<f64>,
    ma20: Option<f64>,
    ma60: Option<f64>,
}
#[derive(Debug, Default, Serialize)]
struct Observations {
    observations: u64,
    unique_identities: u64,
    identical_repeated_identities: u64,
    conflicting_identities: u64,
    stop_rule_evaluations: u64,
    stop_observations: Vec<serde_json::Value>,
}
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn regular(path: &Path) -> Result<File, String> {
    require(
        std::fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .is_file(),
        "input must be a regular non-symlink file",
    )?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    require(
        file.metadata().map_err(|e| e.to_string())?.is_file(),
        "opened input must be regular",
    )?;
    Ok(file)
}
fn inspect(args: &Args) -> Result<serde_json::Value, String> {
    let mut bytes = Vec::new();
    regular(&args.manifest)?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    require(bytes.len() <= 65_536, "manifest exceeds limit")?;
    require(
        digest(&bytes) == args.manifest_sha256,
        "manifest hash mismatch",
    )?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    require(
        manifest.version == 1 && manifest.timezone == "Asia/Shanghai",
        "manifest version/timezone mismatch",
    )?;
    let zone = FixedOffset::east_opt(8 * 3600).unwrap();
    require(
        manifest.business_date == args.business_date
            && manifest.as_of == args.as_of
            && args.as_of.with_timezone(&zone).date_naive() == args.business_date,
        "date/as-of mismatch",
    )?;
    require(
        !manifest.provenance.trim().is_empty() && !manifest.source_contract.trim().is_empty(),
        "missing caller provenance/source contract",
    )?;
    require(
        !manifest.sources.is_empty() && manifest.sources.iter().all(|s| !s.trim().is_empty()),
        "missing sources",
    )?;
    require(
        (1..=MAX_ROWS).contains(&manifest.rows) && (1..=MAX_TOTAL).contains(&manifest.bytes),
        "empty or excessive declared capture",
    )?;
    let parts: Vec<_> = Path::new(&manifest.file).components().collect();
    require(
        parts.len() == 1 && matches!(parts[0], Component::Normal(_)),
        "capture must be an adjacent filename",
    )?;
    let file = regular(
        &args
            .manifest
            .parent()
            .unwrap_or(Path::new("."))
            .join(&manifest.file),
    )?;
    let mut reader = BufReader::new(file);
    let result = scan(&mut reader, &manifest)?;
    Ok(serde_json::json!({
        "status": if manifest.complete {"observed"} else {"unavailable_incomplete_capture"},
        "business_date":args.business_date,"as_of":args.as_of,
        "manifest_sha256":args.manifest_sha256,"capture_sha256":manifest.sha256,
        "caller_provenance":manifest.provenance,"source_contract":manifest.source_contract,
        "source_authority":"not established by hashes or caller declarations",
        "historical_counted_replay_acceptance":"unmeasured_pending_genuine_capture_and_counted_receipt_join",
        "observations":result
    }))
}
fn scan(reader: &mut impl BufRead, manifest: &Manifest) -> Result<Observations, String> {
    let mut output = Observations::default();
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    // Full original envelope bytes represented as JSON, bounded by MAX_TOTAL.
    let mut seen = BTreeMap::<String, serde_json::Value>::new();
    let zone = FixedOffset::east_opt(8 * 3600).unwrap();
    loop {
        let mut line = Vec::new();
        let size = (&mut *reader)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if size == 0 {
            break;
        }
        total += size as u64;
        require(
            size as u64 <= MAX_LINE && total <= MAX_TOTAL && total <= manifest.bytes,
            "capture byte limit exceeded",
        )?;
        output.observations += 1;
        require(
            output.observations <= MAX_ROWS && output.observations <= manifest.rows,
            "capture row limit exceeded",
        )?;
        hasher.update(&line);
        let raw: serde_json::Value = serde_json::from_slice(&line).map_err(|e| e.to_string())?;
        let env: EventEnvelope = serde_json::from_slice(&line).map_err(|e| e.to_string())?;
        require(
            env.version == 1 && env.replay_of.is_none(),
            "unsupported or rewritten envelope",
        )?;
        require(
            [&env.id, &env.trace_id, &env.source, &env.event_type]
                .iter()
                .all(|s| !s.trim().is_empty()),
            "blank identity",
        )?;
        require(manifest.sources.contains(&env.source), "undeclared source")?;
        require(
            env.ts <= manifest.as_of
                && env.ts.with_timezone(&zone).date_naive() == manifest.business_date,
            "future/wrong-date observation",
        )?;
        if let Some(prior) = seen.get(&env.id) {
            if prior != &raw {
                output.conflicting_identities += 1;
                return Err(format!("conflicting identity; observations={}, unique={}, identical_repeats={}, conflicting={}",output.observations,seen.len(),output.identical_repeated_identities,output.conflicting_identities));
            }
            output.identical_repeated_identities += 1;
            continue;
        }
        if env.event_type == "risk.stop_input.observed.v1" {
            #[derive(Deserialize)]
            struct StopEnvelope {
                payload: StopInput,
            }
            // Deserialize from original bytes so duplicate payload fields cannot disappear in Value.
            let input = serde_json::from_slice::<StopEnvelope>(&line)
                .map_err(|e| e.to_string())?
                .payload;
            require(
                input.version == 1
                    && !input.code.trim().is_empty()
                    && !input.name.trim().is_empty()
                    && env.entity_key.as_deref() == Some(&input.code),
                "stop identity/version mismatch",
            )?;
            require(
                [
                    Some(input.current_price),
                    Some(input.cost_price),
                    input.hard_stop,
                    input.ma20,
                    input.ma60,
                ]
                .into_iter()
                .flatten()
                .all(|p| p.is_finite() && p > 0.),
                "invalid captured stop value",
            )?;
            let signals = check_stops(
                &input.code,
                &input.name,
                input.current_price,
                input.cost_price,
                input.hard_stop,
                input.ma20,
                input.ma60,
            );
            output.stop_rule_evaluations += 1;
            // Retain exact captured timestamp/source/identity and input, even with zero signals.
            output.stop_observations.push(serde_json::json!({"id":raw["id"],"ts":raw["ts"],"source":raw["source"],"input":raw["payload"],"signals":signals.iter().map(|s|serde_json::json!({"level":format!("{:?}",s.level),"trigger_price":s.trigger_price})).collect::<Vec<_>>()}));
        }
        seen.insert(env.id, raw);
    }
    output.unique_identities = seen.len() as u64;
    require(
        output.observations == manifest.rows && total == manifest.bytes,
        "declared size/rows mismatch",
    )?;
    require(
        hex::encode(hasher.finalize()) == manifest.sha256,
        "capture hash mismatch",
    )?;
    Ok(output)
}
fn main() {
    match inspect(&Args::parse()) {
        Ok(report) => println!("{}", report),
        Err(reason) => {
            eprintln!(
                "{}",
                serde_json::json!({"status":"rejected","reason":reason,"historical_counted_replay_acceptance":"unmeasured"})
            );
            std::process::exit(2);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn event() -> serde_json::Value {
        serde_json::json!({"id":"TEST_CODE_e1","trace_id":"TEST_CODE_trace","source":"TEST_CODE_source","event_type":"risk.stop_input.observed.v1","version":1,"replay_of":null,"entity_key":"TEST_CODE_stock","ts":"2026-10-08T10:00:00+08:00","payload":{"version":1,"code":"TEST_CODE_stock","name":"fixture","current_price":8.,"cost_price":10.,"hard_stop":9.,"ma20":8.5,"ma60":9.5}})
    }
    fn run(events: &[serde_json::Value]) -> Result<Observations, String> {
        let bytes = events
            .iter()
            .map(|e| format!("{e}\n"))
            .collect::<String>()
            .into_bytes();
        let m = Manifest {
            version: 1,
            timezone: "Asia/Shanghai".into(),
            business_date: NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
            as_of: DateTime::parse_from_rfc3339("2026-10-08T15:00:00+08:00").unwrap(),
            provenance: "synthetic".into(),
            source_contract: "fixture/v1".into(),
            complete: false,
            file: "fixture.jsonl".into(),
            sha256: digest(&bytes),
            rows: events.len() as u64,
            bytes: bytes.len() as u64,
            sources: vec!["TEST_CODE_source".into()],
        };
        scan(&mut std::io::Cursor::new(bytes), &m)
    }
    #[test]
    fn duplicates_are_observations_and_stops_use_production_rule() {
        let result = run(&[event(), event()]).unwrap();
        assert_eq!(
            (
                result.observations,
                result.unique_identities,
                result.identical_repeated_identities,
                result.stop_rule_evaluations
            ),
            (2, 1, 1, 1)
        );
        assert_eq!(
            result.stop_observations[0]["signals"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
    #[test]
    fn rejects_conflict_future_wrong_date_identity_version_and_bounds() {
        for (key, value) in [
            ("ts", serde_json::json!("2026-10-08T16:00:00+08:00")),
            ("ts", serde_json::json!("2026-10-07T10:00:00+08:00")),
            ("id", serde_json::json!(" ")),
            ("version", serde_json::json!(2)),
            ("source", serde_json::json!("other")),
        ] {
            let mut e = event();
            e[key] = value;
            assert!(run(&[e]).is_err());
        }
        let mut conflict = event();
        conflict["payload"]["current_price"] = serde_json::json!(7.);
        assert!(run(&[event(), conflict])
            .unwrap_err()
            .contains("conflicting"));
        let mut huge = event();
        huge["payload"]["name"] = serde_json::json!("x".repeat(MAX_LINE as usize));
        assert!(run(&[huge]).is_err());
    }
    #[test]
    fn manifest_hash_date_and_capture_identity_are_checked_without_writes() {
        let root = tempfile::tempdir().unwrap();
        let data = format!("{}\n", event());
        let capture = root.path().join("day.jsonl");
        std::fs::write(&capture, &data).unwrap();
        let manifest = serde_json::json!({"version":1,"timezone":"Asia/Shanghai","business_date":"2026-10-08","as_of":"2026-10-08T15:00:00+08:00","provenance":"TEST_CODE_synthetic","source_contract":"TEST_CODE/v1","complete":false,"file":"day.jsonl","sha256":digest(data.as_bytes()),"rows":1,"bytes":data.len(),"sources":["TEST_CODE_source"]});
        let path = root.path().join("manifest.json");
        let bytes = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let mut args = Args {
            manifest: path.clone(),
            manifest_sha256: digest(&bytes),
            business_date: NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
            as_of: DateTime::parse_from_rfc3339("2026-10-08T15:00:00+08:00").unwrap(),
        };
        assert_eq!(
            inspect(&args).unwrap()["status"],
            "unavailable_incomplete_capture"
        );
        args.business_date = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
        assert!(inspect(&args).unwrap_err().contains("date"));
        args.business_date = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        for (key, value) in [
            ("rows", serde_json::json!(2)),
            ("bytes", serde_json::json!(data.len() + 1)),
            ("sha256", serde_json::json!("bad")),
            ("file", serde_json::json!("../day.jsonl")),
        ] {
            let mut changed = manifest.clone();
            changed[key] = value;
            let changed = serde_json::to_vec(&changed).unwrap();
            std::fs::write(&path, &changed).unwrap();
            args.manifest_sha256 = digest(&changed);
            assert!(inspect(&args).is_err());
        }
        args.manifest_sha256 = "bad".into();
        assert!(inspect(&args).unwrap_err().contains("manifest hash"));
        assert_eq!(std::fs::read_to_string(capture).unwrap(), data);
    }
}
