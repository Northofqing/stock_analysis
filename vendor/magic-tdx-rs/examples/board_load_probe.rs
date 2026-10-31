use magic_market_core::{
    BoardCategory, BoardConstituentProvider, BoardConstituentRequest, BoardDirectoryProvider,
    BoardDirectoryRequest, NonEmptyText, PositiveU32,
};
use magic_tdx_rs::TdxBoardProvider;
use std::error::Error;
use std::time::{Duration, Instant};

const MAX_ATTEMPTS: u32 = 3;
const MIN_PACING: Duration = Duration::from_secs(1);

fn main() -> Result<(), Box<dyn Error>> {
    let server = required_env("MAGIC_TDX_BOARD_SERVER")?;
    let board_name = required_env("MAGIC_TDX_BOARD_NAME")?;
    let category = parse_category(&required_env("MAGIC_TDX_BOARD_CATEGORY")?)?;
    let concurrency = env_u32("MAGIC_TDX_BOARD_LOAD_CONCURRENCY", 1)?;
    if concurrency != 1 {
        return Err("TDX board load probe requires concurrency=1".into());
    }
    let attempts = env_u32("MAGIC_TDX_BOARD_LOAD_REQUESTS", 3)?;
    if attempts == 0 || attempts > MAX_ATTEMPTS {
        return Err(format!("MAGIC_TDX_BOARD_LOAD_REQUESTS must be in 1..={MAX_ATTEMPTS}").into());
    }
    let operation =
        std::env::var("MAGIC_TDX_BOARD_LOAD_OPERATION").unwrap_or_else(|_| "directory".into());
    if !matches!(operation.as_str(), "directory" | "constituents") {
        return Err("MAGIC_TDX_BOARD_LOAD_OPERATION must be directory or constituents".into());
    }
    let provider = TdxBoardProvider::with_default(&server);
    let board_code = format!("tdx:{}:{board_name}", category_label(category));
    let started = Instant::now();
    let mut previous_start = None;
    let mut successes = 0_u32;

    for attempt in 0..attempts {
        if let Some(previous) = previous_start {
            let elapsed = Instant::now().duration_since(previous);
            if elapsed < MIN_PACING {
                std::thread::sleep(MIN_PACING - elapsed);
            }
        }
        let attempt_started = Instant::now();
        previous_start = Some(attempt_started);
        let records = match operation.as_str() {
            "directory" => provider
                .boards(&BoardDirectoryRequest::new(
                    category,
                    PositiveU32::new(1_000)?,
                )?)?
                .records()
                .len(),
            "constituents" => provider
                .board_constituents(&BoardConstituentRequest::new(
                    NonEmptyText::new(board_code.clone())?,
                    PositiveU32::new(10_000)?,
                )?)?
                .records()
                .len(),
            _ => unreachable!("validated operation"),
        };
        if records == 0 {
            return Err(format!("attempt {} returned no board records", attempt + 1).into());
        }
        successes += 1;
        println!(
            "attempt={} operation={} records={} latency_ms={}",
            attempt + 1,
            operation,
            records,
            attempt_started.elapsed().as_millis()
        );
    }

    println!("provider=tdx-block-files");
    println!("concurrency={concurrency}");
    println!("attempts={attempts}");
    println!("successes={successes}");
    println!("elapsed_ms={}", started.elapsed().as_millis());
    println!("board_load_probe_status=passed");
    Ok(())
}

fn required_env(name: &str) -> Result<String, Box<dyn Error>> {
    std::env::var(name)
        .map_err(|_| format!("{name} is required for the TDX board load probe").into())
}

fn env_u32(name: &str, default: u32) -> Result<u32, Box<dyn Error>> {
    Ok(std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .parse()?)
}

fn parse_category(value: &str) -> Result<BoardCategory, Box<dyn Error>> {
    match value {
        "industry" => Ok(BoardCategory::Industry),
        "concept" => Ok(BoardCategory::Concept),
        _ => Err("MAGIC_TDX_BOARD_CATEGORY must be industry or concept".into()),
    }
}

fn category_label(category: BoardCategory) -> &'static str {
    match category {
        BoardCategory::Industry => "industry",
        BoardCategory::Concept => "concept",
        BoardCategory::Region | BoardCategory::Unknown => unreachable!("validated category"),
    }
}
