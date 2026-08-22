//! T0 server-side library transport probe: exactly what the server's
//! fetch_t0_evidence produces (fetch_magic_tdx_t0_batch with wall clock).
//! Prints records/rejections raw — reveals why records can be empty.
//! Usage: cargo run --bin t0_lib_probe
#[cfg(feature = "magic-gateway")]
fn main() {
    use std::time::Instant;
    let codes: Vec<String> = vec![
        "000813".to_string(),
        "002131".to_string(),
        "002208".to_string(),
        "002421".to_string(),
        "600396".to_string(),
        "600703".to_string(),
        "603948".to_string(),
    ];
    let start = Instant::now();
    match stock_analysis::data_gateway::magic_tdx_t0::fetch_magic_tdx_t0_batch(
        &codes,
        chrono::Utc::now(),
    ) {
        Ok(batch) => {
            println!(
                "OK source_at={} observed_at={} records={} rejections={} elapsed={:?}",
                batch.source_at,
                batch.observed_at,
                batch.records.len(),
                batch.rejections.len(),
                start.elapsed()
            );
            for rejection in &batch.rejections {
                println!(
                    "  reject code={} reason_code={} retryable={} detail={}",
                    rejection.code, rejection.reason_code, rejection.retryable, rejection.detail
                );
            }
            for record in &batch.records {
                println!("  record code={}", record.code);
            }
        }
        Err(e) => println!("FAIL elapsed={:?} err={e}", start.elapsed()),
    }
}

#[cfg(not(feature = "magic-gateway"))]
fn main() {
    eprintln!("requires --features magic-gateway");
    std::process::exit(2);
}
