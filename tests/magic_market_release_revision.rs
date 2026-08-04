use std::fs;
use std::path::Path;

const RELEASE_REVISION: &str = "5f1ce93656a55854c844065390520cd4aecd9a14";
const OLD_RELEASE_REVISION: &str = "660902ff93a07f18367dc16879cf67732accd25a";
const MAGIC_REPOSITORY: &str = "https://github.com/Northofqing/magic-market-data-rs.git";
const DIRECT_MAGIC_CRATES: [&str; 14] = [
    "magic-baidu-rs",
    "magic-cls-rs",
    "magic-cninfo-rs",
    "magic-eastmoney-rs",
    "magic-exchange-rs",
    "magic-jin10-rs",
    "magic-market-composition",
    "magic-market-core",
    "magic-market-router",
    "magic-sina-rs",
    "magic-tdx-rs",
    "magic-tencent-rs",
    "magic-thepaper-rs",
    "magic-ths-rs",
];
const LOCKED_MAGIC_CRATES: [&str; 15] = [
    "magic-baidu-rs",
    "magic-cls-rs",
    "magic-cninfo-rs",
    "magic-eastmoney-rs",
    "magic-exchange-rs",
    "magic-jin10-rs",
    "magic-market-composition",
    "magic-market-core",
    "magic-market-router",
    "magic-market-transport",
    "magic-sina-rs",
    "magic-tdx-rs",
    "magic-tencent-rs",
    "magic-thepaper-rs",
    "magic-ths-rs",
];

fn read(root: &Path, relative: &str) -> String {
    let path = root.join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn production_source(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

fn lock_field<'a>(block: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{field} = \"");
    block.lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix('"'))
    })
}

#[test]
fn br192_magic_market_release_revision_is_one_atomic_identity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = read(root, "Cargo.toml");
    let lock = read(root, "Cargo.lock");
    let board = read(root, "src/data_gateway/board.rs");
    let outcome_bars = read(root, "src/data_gateway/outcome_daily_bars.rs");
    let global_news = read(root, "src/news/aggregator/raw_v2.rs");
    let schema = read(root, "src/selection/schema_v2.rs");
    let registry = read(root, "config/selection/provider_board_bindings.v1.json");

    let expected_manifest_fragment =
        format!("git = \"{MAGIC_REPOSITORY}\", rev = \"{RELEASE_REVISION}\"");
    let mut dependency_rows = manifest
        .lines()
        .filter(|line| line.trim_start().starts_with("magic-"))
        .map(str::trim)
        .collect::<Vec<_>>();
    dependency_rows.sort_unstable();
    let mut dependency_names = dependency_rows
        .iter()
        .map(|line| {
            line.split_once('=')
                .map(|(name, _)| name.trim())
                .expect("magic dependency row must contain '='")
        })
        .collect::<Vec<_>>();
    dependency_names.sort_unstable();
    assert_eq!(
        dependency_names, DIRECT_MAGIC_CRATES,
        "BR-192 requires the exact fourteen-crate Magic dependency set"
    );
    assert!(
        dependency_rows
            .iter()
            .all(|line| line.contains(&expected_manifest_fragment)
                && line.contains("version = \"=0.2.0\"")),
        "every Magic dependency must pin repository, release commit and exact crate version"
    );

    let expected_lock_source =
        format!("git+{MAGIC_REPOSITORY}?rev={RELEASE_REVISION}#{RELEASE_REVISION}");
    let mut locked_magic = lock
        .split("[[package]]")
        .filter_map(|block| {
            let name = lock_field(block, "name")?;
            let source = lock_field(block, "source");
            if LOCKED_MAGIC_CRATES.contains(&name)
                || source.is_some_and(|value| value.contains("magic-market-data-rs"))
            {
                Some((name, lock_field(block, "version"), source))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    locked_magic.sort_unstable_by_key(|(name, _, _)| *name);
    assert_eq!(
        locked_magic.len(),
        LOCKED_MAGIC_CRATES.len(),
        "Cargo.lock must contain no missing or extra Magic package identity"
    );
    assert_eq!(
        locked_magic
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>(),
        LOCKED_MAGIC_CRATES,
        "Cargo.lock Magic package names must equal the released crate set"
    );
    assert!(
        locked_magic.iter().all(|(_, version, source)| {
            *version == Some("0.2.0") && *source == Some(expected_lock_source.as_str())
        }),
        "each Magic package block must bind version 0.2.0 and the one released source"
    );
    assert!(
        !manifest.contains(OLD_RELEASE_REVISION) && !lock.contains(OLD_RELEASE_REVISION),
        "superseded release revision must not remain in dependency resolution"
    );

    let schema_literal = format!("pub const UPSTREAM_REVISION: &str = \"{RELEASE_REVISION}\";");
    assert!(
        schema.contains(&schema_literal),
        "selection evidence schema must bind the released upstream identity"
    );
    let board_literal =
        format!("pub const PINNED_MAGIC_MARKET_REVISION: &str = \"{RELEASE_REVISION}\";");
    assert!(
        board.contains(&board_literal),
        "raw provider-board admission must bind the released upstream identity"
    );
    let volume_contract = format!("magic-market-data-rs@{RELEASE_REVISION}:BR-022+BR-036");
    assert!(
        production_source(&outcome_bars).contains(&volume_contract),
        "outcome daily-bar volume evidence must bind the released upstream identity"
    );
    let global_news_literal =
        format!("pub const MAGIC_MARKET_DATA_REVISION: &str = \"{RELEASE_REVISION}\";");
    assert!(
        production_source(&global_news).contains(&global_news_literal),
        "global-news evidence must bind the released upstream identity"
    );

    let expected_registry_field = format!("\"upstream_revision\":\"{RELEASE_REVISION}\"");
    assert!(
        registry.contains(&expected_registry_field),
        "checked-in board registry must bind the released upstream identity"
    );
}
