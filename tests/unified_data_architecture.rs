//! BR-164 production acquisition boundary.
//!
//! Financial/news protocols and Magic provider construction belong to
//! `src/data_gateway/**`. Business code may consume gateway/domain types but
//! must not own source hosts, wire clients, or provider selection.

use std::fs;
use std::path::{Path, PathBuf};

const FINANCIAL_HOSTS: &[&str] = &[
    "qt.gtimg.cn",
    "eastmoney.com",
    "eastmoney.cn",
    "sinajs.cn",
    "sina.com.cn",
    "sina.cn",
    "cninfo.com.cn",
    "10jqka.com.cn",
    "iwencai.com",
    "cls.cn",
    "jin10.com",
    "thepaper.cn",
    "wallstreetcn.com",
    "gelonghui.com",
    "xueqiu.com",
    "baidu.com/api/finance",
    "cffex.com.cn",
    "sse.com.cn",
    "szse.cn",
    "hkex.com.hk",
    "api.bocha.cn",
    "api.tavily.com",
    "serpapi.com",
];

const LEGACY_ACQUISITION_SYMBOLS: &[&str] = &[
    "fetch_announcements(",
    "fetch_with_fallback_async(",
    "fetch_with_fallback_blocking(",
    "fetch_flow_history_async(",
    "fetch_intraday_shape_async(",
    "fetch_money_flow_blocking(",
    "fetch_intraday_shape_blocking(",
    "intraday_kline::fetch_async(",
    "intraday_kline::fetch_blocking(",
    "NorthFlowClient",
];

const GENERAL_WEB_CREDENTIAL_ENV_VARS: &[&str] =
    &["BOCHA_API_KEYS", "TAVILY_API_KEYS", "SERPAPI_KEYS"];

// Keep this path allowlist exact. A new transport owner must be reviewed here
// instead of inheriting permission from an entire business or Gateway directory.
const REQWEST_TRANSPORT_OWNER_PATHS: &[&str] = &[
    "analyzer/mod.rs",
    "bin/monitor/notify.rs",
    "bin/monitor/webhook_alert.rs",
    "broker.rs",
    "data_gateway/board_ranking.rs",
    "data_gateway/general_web_research.rs",
    "http_client.rs",
    "notification/service.rs",
    "push_l6/external_sinks.rs",
];

// These exact binaries are operator diagnostics/replay tools, not production
// business consumers. `magic_compat` is the reviewed upstream type boundary.
// A new direct Magic provider owner must be reviewed and named individually.
const MAGIC_PROVIDER_OWNER_PATHS: &[&str] = &[
    "bin/t0_minute_probe.rs",
    "bin/t0_replay.rs",
    "bin/tdx_5min_probe.rs",
    "bin/tdx_raw_probe.rs",
    "bin/tdx_server_probe.rs",
    "bin/tencent_quote_probe.rs",
    "bin/virtual_pnl.rs",
    "magic_compat/mod.rs",
];

fn is_allowed_magic_provider_owner(relative_path: &Path) -> bool {
    MAGIC_PROVIDER_OWNER_PATHS
        .iter()
        .any(|allowed| relative_path == Path::new(allowed))
}

fn line_owns_reqwest_transport(line: &str) -> bool {
    let code = line.trim_start();
    !code.starts_with("//")
        && (code.contains("reqwest::")
            || code.contains("reqwest_011::")
            || code.contains("SHARED_HTTP_CLIENT")
            || code.contains("SHARED_FAST_HTTP_CLIENT"))
}

fn is_allowed_reqwest_transport_owner(relative_path: &Path) -> bool {
    REQWEST_TRANSPORT_OWNER_PATHS
        .iter()
        .any(|allowed| relative_path == Path::new(allowed))
}

fn reqwest_transport_violations_for_source(relative_path: &Path, body: &str) -> Vec<String> {
    if is_allowed_reqwest_transport_owner(relative_path) {
        return Vec::new();
    }
    let production = body.split("#[cfg(test)]").next().unwrap_or(body);
    production
        .lines()
        .enumerate()
        .filter(|(_, line)| line_owns_reqwest_transport(line))
        .map(|(line_index, line)| {
            format!(
                "{}:{} owns reqwest transport: {}",
                relative_path.display(),
                line_index + 1,
                line.trim()
            )
        })
        .collect()
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in
            fs::read_dir(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        {
            let entry = entry.expect("read source entry");
            let entry_path = entry.path();
            if entry_path.is_dir() {
                pending.push(entry_path);
            } else if entry_path.extension().and_then(|value| value.to_str()) == Some("rs") {
                files.push(entry_path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn br164_financial_and_news_acquisition_is_gateway_owned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let gateway = root.join("data_gateway");
    let mut violations = Vec::new();

    for path in rust_files(&root) {
        if path.starts_with(&gateway) {
            continue;
        }
        let relative_path = path.strip_prefix(&root).unwrap_or_else(|error| {
            panic!("strip {} from {}: {error}", root.display(), path.display())
        });
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for host in FINANCIAL_HOSTS {
            if body.contains(host) {
                violations.push(format!("{} owns financial host {host}", path.display()));
            }
        }
        for line in body.lines() {
            let trimmed = line.trim_start();
            let imports_magic_provider = (trimmed.starts_with("use magic_")
                || trimmed.starts_with("pub use magic_"))
                && !trimmed.starts_with("use magic_market_core")
                && !trimmed.starts_with("pub use magic_market_core");
            if imports_magic_provider && !is_allowed_magic_provider_owner(relative_path) {
                violations.push(format!(
                    "{} imports a Magic provider outside data_gateway: {}",
                    path.display(),
                    trimmed
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "BR-164 violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn br164_reqwest_transport_is_owned_by_explicit_boundaries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();

    for path in rust_files(&root) {
        let relative_path = path.strip_prefix(&root).unwrap_or_else(|error| {
            panic!("strip {} from {}: {error}", root.display(), path.display())
        });
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        violations.extend(reqwest_transport_violations_for_source(
            relative_path,
            &body,
        ));
    }

    assert!(
        violations.is_empty(),
        "BR-164 consumer-owned reqwest transport violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn br164_legacy_data_provider_acquisition_entry_points_are_deleted() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let gateway = root.join("data_gateway");
    let mut violations = Vec::new();

    for path in rust_files(&root) {
        if path.starts_with(&gateway) {
            continue;
        }
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for (line_index, line) in body.lines().enumerate() {
            for symbol in LEGACY_ACQUISITION_SYMBOLS {
                if line.contains(symbol) {
                    violations.push(format!(
                        "{}:{} retains legacy acquisition symbol {symbol}",
                        path.display(),
                        line_index + 1
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "BR-164 legacy acquisition violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn br167_legacy_jin10_calendar_protocol_is_deleted() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/search_service/providers/jin10.rs");
    assert!(
        !path.exists(),
        "BR-167 legacy Jin10 provider must be deleted: {}",
        path.display()
    );
}

#[test]
fn br175_legacy_general_web_protocol_owners_are_deleted() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/search_service/providers");
    for file_name in ["bocha.rs", "tavily.rs", "serpapi.rs"] {
        let path = root.join(file_name);
        assert!(
            !path.exists(),
            "BR-175 legacy general-web protocol owner must be deleted: {}",
            path.display()
        );
    }
}

#[test]
fn br175_general_web_credentials_are_gateway_owned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let gateway = root.join("data_gateway");
    let mut violations = Vec::new();
    for path in rust_files(&root) {
        if path.starts_with(&gateway) {
            continue;
        }
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for env_name in GENERAL_WEB_CREDENTIAL_ENV_VARS {
            if body.contains(env_name) {
                violations.push(format!(
                    "{} owns general-web credential name {env_name}",
                    path.display()
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "BR-175 credential ownership violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn br164_unused_qmt_parser_dependency_is_deleted() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let body = fs::read_to_string(&manifest)
        .unwrap_or_else(|error| panic!("read {}: {error}", manifest.display()));
    assert!(
        !body.contains("qmt-parser"),
        "BR-164 forbids retaining the unused qmt-parser dependency"
    );
}

#[test]
fn br164_dead_legacy_lhb_and_news_facades_are_deleted() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let legacy_lhb = root.join("market_analyzer/lhb_review.rs");
    assert!(
        !legacy_lhb.exists(),
        "BR-164 legacy market-analyzer LHB loader must be deleted: {}",
        legacy_lhb.display()
    );

    let checks = [
        (root.join("lhb_analyzer.rs"), "LhbDataFetcher"),
        (root.join("http_client.rs"), "SHARED_TENCENT_HTTP_CLIENT"),
        (root.join("http_client.rs"), "SHARED_FALLBACK_HTTP_CLIENT"),
        (root.join("news/aggregator/mod.rs"), "pub async fn tick("),
    ];
    for (path, forbidden) in checks {
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            !body.contains(forbidden),
            "BR-164 forbids {forbidden} in {}",
            path.display()
        );
    }
}

#[test]
fn br170_static_position_chain_sources_and_fallbacks_are_deleted() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let registry = root.join("data_provider/chain_registry.rs");
    assert!(
        !registry.exists(),
        "BR-170 static chain registry must be deleted: {}",
        registry.display()
    );

    let checks = [
        (root.join("data_provider/mod.rs"), "pub mod chain_registry"),
        (
            root.join("database/positions.rs"),
            "pub fn backfill_chain_name(",
        ),
        (
            root.join("portfolio/store.rs"),
            "data_provider::chain_registry",
        ),
        (
            root.join("pipeline/position_tracker.rs"),
            "data_provider::chain_registry",
        ),
    ];
    for (path, forbidden) in checks {
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let production = body.split("#[cfg(test)]").next().unwrap_or(&body);
        assert!(
            !production.contains(forbidden),
            "BR-170 forbids {forbidden} in {}",
            path.display()
        );
    }

    let tracker = root.join("pipeline/position_tracker.rs");
    let body = fs::read_to_string(&tracker)
        .unwrap_or_else(|error| panic!("read {}: {error}", tracker.display()));
    let production = body.split("#[cfg(test)]").next().unwrap_or(&body);
    assert!(
        !production.contains("stock_concepts"),
        "BR-170 concentration checks must not consume stock_concepts"
    );
}

#[test]
fn br171_static_lifecycle_confirmation_caches_are_deleted() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let checks = [
        (root.join("monitor/data_quality.rs"), "IPO_DATES"),
        (root.join("monitor/data_quality.rs"), "EX_RIGHTS_DATES"),
        (root.join("monitor/data_quality.rs"), "mark_ipo("),
        (root.join("monitor/data_quality.rs"), "mark_ex_rights("),
        (
            root.join("monitor/data_quality.rs"),
            "is_within_5_days_of_ipo(",
        ),
        (
            root.join("data_provider/limit_status.rs"),
            "is_ipo_first_5_days(",
        ),
    ];

    for (path, forbidden) in checks {
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            !body.contains(forbidden),
            "BR-171 forbids static lifecycle evidence {forbidden} in {}",
            path.display()
        );
    }
}

#[test]
fn br158_a01_history_reuses_the_canonical_historical_bars_gateway() {
    let review = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/data_gateway/review.rs");
    let body = fs::read_to_string(&review)
        .unwrap_or_else(|error| panic!("read {}: {error}", review.display()));

    assert!(
        body.contains("HistoricalBarsGateway"),
        "BR-158 A-01 history must consume HistoricalBarsGateway"
    );
    assert!(
        !body.contains("TdxSmartClient"),
        "BR-158 ReviewDataGateway must not construct a TDX provider"
    );
    assert!(
        !body.contains("BarsRouter"),
        "BR-158 ReviewDataGateway must not own a second daily-bar router"
    );
}

#[test]
fn br164_t0_evidence_is_provider_neutral() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/data_gateway/t0_evidence.rs");
    let body = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

    assert!(
        body.contains("pub struct T0Batch") && body.contains("validate_quote_freshness"),
        "BR-153 must retain provider-neutral T0 evidence and freshness contracts"
    );
    for forbidden in [
        "magic_tdx_rs",
        "TdxHqClient",
        "get_security_quotes",
        "get_security_bars",
        "fetch_magic_tdx",
        "TdxSmartClient",
    ] {
        assert!(
            !body.contains(forbidden),
            "BR-164 forbids duplicate realtime owner {forbidden} in {}",
            path.display()
        );
    }
}

#[test]
fn br164_reqwest_transport_markers_detect_code_not_comments() {
    for source_line in [
        "use reqwest::Client;",
        "let client = reqwest::Client::new();",
        "let client = reqwest_011::Client::new();",
        "let response = crate::http_client::SHARED_HTTP_CLIENT.get(url);",
        "let response = stock_analysis::http_client::SHARED_FAST_HTTP_CLIENT.get(url);",
    ] {
        assert!(
            line_owns_reqwest_transport(source_line),
            "BR-164 must detect reqwest transport ownership: {source_line}"
        );
    }

    for source_line in [
        "// reqwest::Client belongs to a reviewed transport owner",
        "//! SHARED_HTTP_CLIENT is documented here",
        "let label = \"reqwest\";",
    ] {
        assert!(
            !line_owns_reqwest_transport(source_line),
            "BR-164 must not reject prose without transport ownership: {source_line}"
        );
    }
}

#[test]
fn br164_reqwest_transport_owner_allowlist_is_exact() {
    for relative_path in [
        "analyzer/mod.rs",
        "bin/monitor/notify.rs",
        "bin/monitor/webhook_alert.rs",
        "broker.rs",
        "data_gateway/general_web_research.rs",
        "http_client.rs",
        "notification/service.rs",
        "push_l6/external_sinks.rs",
    ] {
        assert!(
            is_allowed_reqwest_transport_owner(Path::new(relative_path)),
            "reviewed reqwest transport owner must remain allowed: {relative_path}"
        );
    }

    for relative_path in [
        "data_gateway/market_data.rs",
        "data_provider/service.rs",
        "market_analyzer/sector_monitor.rs",
        "pipeline/data.rs",
        "search_service/service.rs",
    ] {
        assert!(
            !is_allowed_reqwest_transport_owner(Path::new(relative_path)),
            "financial/news consumer must not own reqwest transport: {relative_path}"
        );
    }
}

#[test]
fn br164_reqwest_policy_rejects_consumers_but_ignores_reviewed_and_test_only_owners() {
    let consumer_violations = reqwest_transport_violations_for_source(
        Path::new("pipeline/data.rs"),
        "use reqwest::Client;\nfn fetch() { let _client = Client::new(); }\n",
    );
    assert_eq!(
        consumer_violations,
        vec!["pipeline/data.rs:1 owns reqwest transport: use reqwest::Client;"]
    );

    let reviewed_owner_violations = reqwest_transport_violations_for_source(
        Path::new("data_gateway/general_web_research.rs"),
        "use reqwest::Client;\n",
    );
    assert!(reviewed_owner_violations.is_empty());

    let test_only_violations = reqwest_transport_violations_for_source(
        Path::new("data_provider/mod.rs"),
        "pub struct DomainValue;\n#[cfg(test)]\nfn loopback() { let _client = reqwest::Client::new(); }\n",
    );
    assert!(test_only_violations.is_empty());
}
