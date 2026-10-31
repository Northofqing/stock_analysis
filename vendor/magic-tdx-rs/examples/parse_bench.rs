use flate2::write::ZlibEncoder;
use flate2::Compression;
use magic_market_core::{AssetClass, Exchange, InstrumentId, Price, Quantity};
use magic_tdx_rs::net::utils::decompress_zlib;
use magic_tdx_rs::protocol::parsers::parse_security_bars;
use serde_json::{json, Value};
use std::error::Error;
use std::hint::black_box;
use std::io::Write;
use std::time::{Duration, Instant};

const BAR_ITERATIONS: u64 = 20_000;
const JSON_ITERATIONS: u64 = 10_000;
const ZLIB_ITERATIONS: u64 = 5_000;
const ZLIB_ROUNDTRIP_ITERATIONS: u64 = 2_000;

fn main() -> Result<(), Box<dyn Error>> {
    let bar_packet = bar_fixture(64);
    let json_document = json_fixture()?;
    let (payload, compressed) = zlib_fixture()?;

    let workloads = vec![
        benchmark_bar_parse(&bar_packet, BAR_ITERATIONS)?,
        benchmark_json_normalize(&json_document, JSON_ITERATIONS)?,
        benchmark_zlib_decompress(&compressed, ZLIB_ITERATIONS)?,
        benchmark_zlib_roundtrip(&payload, ZLIB_ROUNDTRIP_ITERATIONS)?,
    ];
    serde_json::to_writer(
        std::io::stdout().lock(),
        &json!({"schema": 1, "workloads": workloads}),
    )?;
    println!();
    Ok(())
}

fn benchmark_bar_parse(packet: &[u8], iterations: u64) -> Result<Value, Box<dyn Error>> {
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..iterations {
        let bars = parse_security_bars(black_box(packet), 4)?;
        let last = bars
            .last()
            .ok_or("fixed bar fixture unexpectedly decoded as empty")?;
        checksum = checksum
            .wrapping_add(bars.len() as u64)
            .wrapping_add(last.close.to_bits())
            .wrapping_add(last.vol.to_bits())
            .wrapping_add(u64::from(last.day));
        black_box(&bars);
    }
    Ok(record(
        "tdx_bar_parse",
        iterations,
        started.elapsed(),
        checksum,
    ))
}

fn benchmark_json_normalize(document: &[u8], iterations: u64) -> Result<Value, Box<dyn Error>> {
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..iterations {
        let decoded: Value = serde_json::from_slice(black_box(document))?;
        let rows = decoded
            .get("rows")
            .and_then(Value::as_array)
            .ok_or("fixed JSON fixture is missing rows")?;
        for row in rows {
            let code = row
                .get("code")
                .and_then(Value::as_str)
                .ok_or("fixed JSON row is missing code")?;
            let price = row
                .get("price")
                .and_then(Value::as_f64)
                .ok_or("fixed JSON row is missing price")?;
            let quantity = row
                .get("quantity")
                .and_then(Value::as_f64)
                .ok_or("fixed JSON row is missing quantity")?;
            let instrument = InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity)?;
            let price = Price::new(price)?;
            let quantity = Quantity::new(quantity)?;
            checksum = checksum
                .wrapping_add(instrument.code().len() as u64)
                .wrapping_add(price.get().to_bits())
                .wrapping_add(quantity.get().to_bits());
            black_box((instrument, price, quantity));
        }
    }
    Ok(record(
        "json_normalize",
        iterations,
        started.elapsed(),
        checksum,
    ))
}

fn benchmark_zlib_decompress(compressed: &[u8], iterations: u64) -> Result<Value, Box<dyn Error>> {
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..iterations {
        let decoded = decompress_zlib(black_box(compressed))?;
        let first = decoded
            .first()
            .copied()
            .ok_or("fixed zlib fixture unexpectedly decoded as empty")?;
        let last = decoded
            .last()
            .copied()
            .ok_or("fixed zlib fixture unexpectedly decoded as empty")?;
        checksum = checksum
            .wrapping_add(decoded.len() as u64)
            .wrapping_add(u64::from(first))
            .wrapping_add(u64::from(last));
        black_box(decoded);
    }
    Ok(record(
        "zlib_decompress",
        iterations,
        started.elapsed(),
        checksum,
    ))
}

fn benchmark_zlib_roundtrip(payload: &[u8], iterations: u64) -> Result<Value, Box<dyn Error>> {
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..iterations {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(black_box(payload))?;
        let compressed = encoder.finish()?;
        let decoded = decompress_zlib(black_box(&compressed))?;
        if decoded != payload {
            return Err("fixed zlib roundtrip changed payload bytes".into());
        }
        let first = compressed
            .first()
            .copied()
            .ok_or("fixed zlib roundtrip produced an empty stream")?;
        let last = compressed
            .last()
            .copied()
            .ok_or("fixed zlib roundtrip produced an empty stream")?;
        checksum = checksum
            .wrapping_add(compressed.len() as u64)
            .wrapping_add(decoded.len() as u64)
            .wrapping_add(u64::from(first))
            .wrapping_add(u64::from(last));
        black_box((compressed, decoded));
    }
    Ok(record(
        "zlib_roundtrip",
        iterations,
        started.elapsed(),
        checksum,
    ))
}

fn record(workload: &str, iterations: u64, elapsed: Duration, checksum: u64) -> Value {
    let elapsed_ns = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
    let throughput_per_second = iterations as f64 / elapsed.as_secs_f64();
    json!({
        "workload": workload,
        "iterations": iterations,
        "elapsed_ns": elapsed_ns,
        "throughput_per_second": throughput_per_second,
        "checksum": checksum,
    })
}

fn bar_fixture(records: u16) -> Vec<u8> {
    let mut packet = Vec::with_capacity(2 + usize::from(records) * 20);
    packet.extend_from_slice(&records.to_le_bytes());
    for index in 0..records {
        packet.extend_from_slice(&20_260_729_u32.to_le_bytes());
        packet.extend_from_slice(&[10, 1, 2, 0]);
        packet.extend_from_slice(&(10_000_u32 + u32::from(index)).to_le_bytes());
        packet.extend_from_slice(&(1_000_000_u32 + u32::from(index)).to_le_bytes());
    }
    packet
}

fn json_fixture() -> Result<Vec<u8>, serde_json::Error> {
    let rows: Vec<_> = (0_u32..64)
        .map(|index| {
            json!({
                "code": format!("{:06}", 600_000 + index),
                "price": 10.0 + f64::from(index) / 100.0,
                "quantity": 1_000.0 + f64::from(index),
            })
        })
        .collect();
    serde_json::to_vec(&json!({"rows": rows}))
}

fn zlib_fixture() -> Result<(Vec<u8>, Vec<u8>), std::io::Error> {
    let mut payload = Vec::with_capacity(64 * 1_024);
    for index in 0_u32..2_048 {
        payload.extend_from_slice(
            format!(
                "{index:08},600396,华电辽能,{:.2},{:.2}\n",
                10.0 + f64::from(index % 100) / 100.0,
                1_000.0 + f64::from(index)
            )
            .as_bytes(),
        );
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&payload)?;
    let compressed = encoder.finish()?;
    Ok((payload, compressed))
}
