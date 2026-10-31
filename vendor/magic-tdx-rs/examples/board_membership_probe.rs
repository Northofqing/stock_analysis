use magic_market_core::{AssetClass, BoardMembershipProvider, Exchange, InstrumentId};
use magic_tdx_rs::{protocol::constants::PRIMARY_SERVERS, BlockService};
use std::collections::HashSet;

fn instrument(exchange: Exchange, code: &str) -> InstrumentId {
    InstrumentId::new(exchange, code, AssetClass::Equity)
        .expect("fixed probe instrument must be valid")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (_, default_ip, default_port) = PRIMARY_SERVERS
        .first()
        .copied()
        .ok_or("TDX primary server list is empty")?;
    let mut args = std::env::args().skip(1);
    let ip = args.next().unwrap_or_else(|| default_ip.to_owned());
    let port = args
        .next()
        .map(|value| value.parse::<u16>())
        .transpose()?
        .unwrap_or(default_port);
    if args.next().is_some() {
        return Err("usage: board_membership_probe [ip] [port]".into());
    }

    let requested = [
        instrument(Exchange::Shanghai, "600396"),
        instrument(Exchange::Shenzhen, "000001"),
    ];
    let service = BlockService::new(&ip, port, 5.0);
    let batch = service.board_memberships(&requested)?;
    println!(
        "state={} provider=tdx source={} source_at={} observed_at={} batch_id={} records={}",
        if batch.records().is_empty() {
            "verified_empty"
        } else {
            "available"
        },
        batch.provenance().source(),
        batch.provenance().source_at().unwrap_or("absent"),
        batch.provenance().fetched_at(),
        batch.provenance().batch_id().unwrap_or("absent"),
        batch.records().len(),
    );
    if batch.records().is_empty() {
        return Ok(());
    }

    let mut represented = HashSet::new();
    for record in batch.records() {
        represented.insert(record.instrument.code());
        println!(
            "instrument={:?}:{} board_code={} board_name={} category={:?} provider={:?} source_at={} observed_at={} batch_id={}",
            record.instrument.exchange(),
            record.instrument.code(),
            record.board_code.as_str(),
            record.board_name.as_str(),
            record.category,
            record.evidence.provider(),
            record.evidence.source_at().unwrap_or("absent"),
            record.evidence.observed_at(),
            record.evidence.batch_id(),
        );
    }
    for expected in ["600396", "000001"] {
        if !represented.contains(expected) {
            return Err(format!(
                "TDX complete board batch did not represent requested instrument {expected}"
            )
            .into());
        }
    }
    Ok(())
}
