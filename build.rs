//! 编译 magic.market.v1 proto (合同唯一源, 不得修改):
//! - 旧本机服务合同: provider_host_contract/market.proto (固定兼容快照);
//! - 本地扩展 / 旧 bundle 兼容声明:
//!   * 当前上游已发布 Operation/RPC 56-60；旧 bundle 缺失时仍按精确声明补入;
//!   * Operation 61 CHAIN_BATCH 与 62 BENCHMARK_BARS 仍仅供本地 grpc_market_server 使用;
//!   * QueryResponse.source = 11 (证据链 source 透传, 上游用字段 10 做 diagnostic_blocker);
//!   * 当前上游已发布前 5 个派生 RPC；ChainBatch RPC 仍为本地扩展。
//!
//! 合并 proto 生成到 OUT_DIR (按每条精确声明幂等补齐),
//! 当前上游合同文件零修改；旧服务使用独立的固定快照。
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[path = "build_support/magic_tdx_lock.rs"]
mod magic_tdx_lock;

fn main() {
    // tonic 0.14 重构: configure()/compile() 从 tonic_build 移到 tonic-prost-build
    // (tonic_build 0.14 只保留 Service codegen, "Prost functionality has been moved
    //  to tonic-prost-build" — 见 tonic-build-0.14.6 lib.rs 顶部注释)。API 等价。
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let lock_path = manifest_dir.join("Cargo.lock");
    let magic_tdx_revision = locked_magic_tdx_revision(&manifest_dir, &lock_path);
    println!("cargo:rustc-env=MAGIC_TDX_DEPENDENCY_REVISION={magic_tdx_revision}");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    println!("cargo:rerun-if-changed=build_support/magic_tdx_lock.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    // The legacy provider host reserves operation IDs 61/62 for local RPCs.
    // Pin its compatible contract snapshot so a newer ignored client-bundle
    // cannot silently collide with those IDs during a recovery build.
    let upstream = "provider_host_contract/market.proto";
    let content = std::fs::read_to_string(upstream)
        .unwrap_or_else(|e| panic!("read {upstream}: {e} (上游合同必须存在)"));
    let merged_content = merge_local_extensions(&content);
    let merged_path = out_dir.join("market_local.proto");
    std::fs::write(&merged_path, merged_content).expect("write merged proto");
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[merged_path.to_str().expect("path")],
            &[out_dir.to_str().expect("path")],
        )
        .expect("compile market_local.proto (上游合同 + 本地扩展)");
    println!("cargo:rerun-if-changed={upstream}");
    println!("cargo:rerun-if-changed=build.rs");
}

fn locked_magic_tdx_revision(
    manifest_dir: &std::path::Path,
    lock_path: &std::path::Path,
) -> String {
    const PACKAGE: &str = "magic-tdx-rs";
    let lock_bytes = std::fs::read_to_string(lock_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", lock_path.display()));
    let lock: toml::Value = lock_bytes
        .parse()
        .unwrap_or_else(|error| panic!("parse {}: {error}", lock_path.display()));
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .unwrap_or_else(|| panic!("{} has no package array", lock_path.display()));
    let matches: Vec<_> = packages
        .iter()
        .filter(|package| package.get("name").and_then(toml::Value::as_str) == Some(PACKAGE))
        .collect();
    let [package] = matches.as_slice() else {
        panic!(
            "{} must contain exactly one {PACKAGE} package, found {}",
            lock_path.display(),
            matches.len()
        );
    };
    if let Some(source) = package.get("source").and_then(toml::Value::as_str) {
        return magic_tdx_lock::exact_locked_magic_tdx_revision(source)
            .unwrap_or_else(|error| {
                panic!("{PACKAGE} source is not an exact locked revision: {error}")
            })
            .to_owned();
    }

    // The legacy local provider host carries a narrow backport of upstream
    // 98207a4. Its provenance must identify the actual vendored bytes, rather
    // than falsely claiming to be the unmodified 75ee2a2 Git dependency.
    let manifest: toml::Value = std::fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .expect("read Cargo.toml")
        .parse()
        .expect("parse Cargo.toml");
    let patch_path = manifest
        .get("patch")
        .and_then(|patch| patch.get("https://github.com/Northofqing/magic-market-data-rs.git"))
        .and_then(|patch| patch.get(PACKAGE))
        .and_then(|patch| patch.get("path"))
        .and_then(toml::Value::as_str);
    assert_eq!(
        patch_path,
        Some("vendor/magic-tdx-rs"),
        "{PACKAGE} has no admitted local backport"
    );
    let vendor = manifest_dir.join("vendor/magic-tdx-rs");
    let mut files = vec![vendor.join("Cargo.toml")];
    collect_source_files(&vendor.join("src"), &mut files);
    files.sort();
    let mut hash = Sha256::new();
    for file in files {
        let relative = file.strip_prefix(&vendor).expect("vendor source path");
        let bytes =
            std::fs::read(&file).unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
        hash.update(relative.to_string_lossy().as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    println!("cargo:rerun-if-changed={}", vendor.display());
    format!(
        "75ee2a2+backport-98207a4+sha256:{}",
        hex::encode(hash.finalize())
    )
}

fn collect_source_files(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
    for entry in
        std::fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
    {
        let path = entry.expect("vendor source entry").path();
        if path.is_dir() {
            collect_source_files(&path, files);
        } else {
            files.push(path);
        }
    }
}

/// 本地扩展块 (注释解释来源与用户决策)。
const EXT_OPERATIONS: &[&str] = &[
    "  // 本地扩展 / 旧 bundle 兼容声明: 用户决策 2026-08-16 「保留本地 server 扩展」。",
    "  OPERATION_INDEX_QUOTES = 56;",
    "  OPERATION_INTRADAY_SHAPE = 57;",
    "  OPERATION_T0_EVIDENCE = 58;",
    "  OPERATION_OUTCOME_DAILY_BARS = 59;",
    "  OPERATION_UPPER_LIMIT_POOL_REVIEW = 60;",
    // M4c: A-10 题材链完整 batch (monitor 复盘消费, 44/45 视图不可重建 VisibleChainBatch)。
    "  OPERATION_CHAIN_BATCH = 61;",
    // BR-251: 指数专用历史基准批次；不得复用 equity HistoricalBars/TechnicalBars。
    "  OPERATION_BENCHMARK_BARS = 62;",
];

const EXT_QUERY_RESPONSE_FIELD: &[&str] = &[
    "  // 本地扩展: 证据链 source 透传 (客户端桥构造 BatchEvidence.source; 上游合同无此字段)。",
    "  string source = 11;",
];

const EXT_RPCS: &[&str] = &[
    "  // 本地扩展 / 旧 bundle 兼容 RPC (客户端按 implemented 集合区分)。",
    "  rpc IndexQuotes(QueryRequest) returns (QueryResponse);",
    "  rpc IntradayShape(QueryRequest) returns (QueryResponse);",
    "  rpc T0Evidence(QueryRequest) returns (QueryResponse);",
    "  rpc OutcomeDailyBars(QueryRequest) returns (QueryResponse);",
    "  rpc UpperLimitPoolReview(QueryRequest) returns (QueryResponse);",
    "  rpc ChainBatch(QueryRequest) returns (QueryResponse);",
    "  rpc BenchmarkBars(QueryRequest) returns (QueryResponse);",
];

const EXT_BENCHMARK_ERROR_DETAIL: &[&str] = &[
    "",
    "// 本地扩展: BR-251 服务端审计结果分类；客户端不得从 reason_code 反推。",
    "enum BenchmarkAuditState {",
    "  BENCHMARK_AUDIT_STATE_UNSPECIFIED = 0;",
    "  BENCHMARK_AUDIT_STATE_PERSISTED = 1;",
    "  BENCHMARK_AUDIT_STATE_APPEND_FAILED = 2;",
    "}",
    "message BenchmarkErrorDetail {",
    "  ErrorDetail error = 1;",
    "  string audit_outcome = 2;",
    "  BenchmarkAuditState audit_state = 3;",
    "}",
];

fn merge_local_extensions(content: &str) -> String {
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    // Upstream may publish former local extensions independently. Merge each
    // declaration by exact line instead of treating one sentinel as authority
    // for the entire block.
    let missing_operations = missing_extension_lines(&lines, EXT_OPERATIONS);
    let missing_response_fields = missing_extension_lines(&lines, EXT_QUERY_RESPONSE_FIELD);
    let missing_rpcs = missing_extension_lines(&lines, EXT_RPCS);

    // 1. Operation enum 块末尾追加仍未由上游发布的扩展值。
    if let Some((_, end)) = find_block(&lines, "enum Operation {") {
        lines.splice(end..end, missing_operations);
    } else {
        panic!("market.proto 缺少 enum Operation (合同结构变化, 需人工同步 build.rs)");
    }
    // 2. QueryResponse 块末尾追加 source = 11。
    if let Some((_, end)) = find_block(&lines, "message QueryResponse {") {
        lines.splice(end..end, missing_response_fields);
    } else {
        panic!("market.proto 缺少 message QueryResponse");
    }
    // 3. MarketDataService 块末尾追加仍未由上游发布的扩展 RPC。
    if let Some((_, end)) = find_block(&lines, "service MarketDataService {") {
        lines.splice(end..end, missing_rpcs);
    } else {
        panic!("market.proto 缺少 service MarketDataService");
    }
    if !lines
        .iter()
        .any(|line| line.trim() == "message BenchmarkErrorDetail {")
    {
        lines.extend(EXT_BENCHMARK_ERROR_DETAIL.iter().map(ToString::to_string));
    }
    lines.join("\n") + "\n"
}

fn missing_extension_lines(lines: &[String], extension: &[&str]) -> Vec<String> {
    let missing = extension
        .iter()
        .copied()
        .filter(|line| !line.trim_start().starts_with("//"))
        .filter(|line| {
            let expected = line.trim();
            !lines.iter().any(|existing| existing.trim() == expected)
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Vec::new();
    }
    extension
        .iter()
        .copied()
        .filter(|line| line.trim_start().starts_with("//") || missing.contains(line))
        .map(str::to_owned)
        .collect()
}

/// 定位 `marker` 所在行到其块结束行 (大括号深度归零, 含嵌套; 行内无字符串字面量 — proto 注释用 //)。
fn find_block(lines: &[String], marker: &str) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| l.contains(marker))?;
    let mut depth = 0usize;
    for (i, l) in lines.iter().enumerate().skip(start) {
        depth = depth + l.matches('{').count() - l.matches('}').count();
        if depth == 0 && i > start {
            return Some((start, i));
        }
    }
    None
}
