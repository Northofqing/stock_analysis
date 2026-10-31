use magic_market_core::{
    BoardCategory, BoardConstituentProvider, BoardConstituentRequest, BoardDirectoryProvider,
    BoardDirectoryRequest, BoardMembershipProvider, NonEmptyText, PositiveU32,
};
use magic_tdx_rs::TdxBoardProvider;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let server = required_env("MAGIC_TDX_BOARD_SERVER")?;
    let board_name = required_env("MAGIC_TDX_BOARD_NAME")?;
    let category = parse_category(&required_env("MAGIC_TDX_BOARD_CATEGORY")?)?;
    let provider = TdxBoardProvider::with_default(&server);

    println!("provider=tdx-block-files");
    println!("server={server}");
    println!(
        "market_discovery_capabilities={:#?}",
        TdxBoardProvider::market_discovery_capabilities()
    );

    let directory_request = BoardDirectoryRequest::new(category, PositiveU32::new(1_000)?)?;
    let directory = provider.boards(&directory_request)?;
    println!("directory_records={}", directory.records().len());
    println!("directory_provenance={:#?}", directory.provenance());
    if directory.provenance().source_at().is_some() {
        return Err("TDX board directory must not fabricate source_at".into());
    }
    let expected_code = format!("tdx:{}:{board_name}", category_label(category));
    if !directory
        .records()
        .iter()
        .any(|board| board.board_code().as_str() == expected_code)
    {
        let available = directory
            .records()
            .iter()
            .take(20)
            .map(|board| board.board_name().as_str())
            .collect::<Vec<_>>();
        return Err(format!(
            "exact board {board_name:?} was not found; first bounded names: {available:?}"
        )
        .into());
    }

    let constituent_request = BoardConstituentRequest::new(
        NonEmptyText::new(expected_code.clone())?,
        PositiveU32::new(10_000)?,
    )?;
    let constituents = provider.board_constituents(&constituent_request)?;
    println!("constituent_records={}", constituents.records().len());
    println!("constituent_provenance={:#?}", constituents.provenance());
    let sample = constituents
        .records()
        .first()
        .ok_or("TDX exact board returned no constituents")?
        .instrument
        .clone();

    let reverse = provider.board_memberships(std::slice::from_ref(&sample))?;
    println!(
        "reverse_instrument={:?}.{}",
        sample.exchange(),
        sample.code()
    );
    println!("reverse_records={}", reverse.records().len());
    println!("reverse_provenance={:#?}", reverse.provenance());
    if !reverse
        .records()
        .iter()
        .any(|membership| membership.board_code.as_str() == expected_code)
    {
        return Err("TDX reverse membership did not contain the exact source board".into());
    }

    println!("board_live_probe_status=passed");
    Ok(())
}

fn required_env(name: &str) -> Result<String, Box<dyn Error>> {
    std::env::var(name)
        .map_err(|_| format!("{name} is required for the TDX board live probe").into())
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
