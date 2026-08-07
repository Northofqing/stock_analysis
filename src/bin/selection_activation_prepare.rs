//! BR-183/193 selection-v2 激活材料准备工具 (只读 + 封存输出)。
//!
//! 命令:
//!   selection_activation_prepare seal-board <reviewed_by> [valid_from]
//!     从真实 Magic TDX 目录取 Concept/Industry 各 1 板块, 封存
//!     provider_board_binding_proposal.v1.json + provider_board_bindings.v1.json
//!     到 config/selection/ (canonical JSON)。TDX 不可用 → 报错退出 (fail-closed)。
//!
//!   selection_activation_prepare print-activation <reviewed_by> <effective_from>
//!     校验现有材料, 输出 expected_config_hash + 可直接落盘的
//!     selection_activation.v1.json (到 stdout)。
//!
//! 仪式: executable_revision 覆盖全部 src/+config/ 文件 — 任何代码/配置改动
//! 都会使 expected_config_hash 失效, 需重新 prepare + 人工 review。

use chrono::{DateTime, Utc};
use std::path::Path;

fn main() {
    // BR-159: TDX gateway 审计需要 core 数据库 (gateway_result 落库)。
    let database_path = std::env::var("DATABASE_PATH")
        .unwrap_or_else(|_| "./data/stock_analysis.db".to_string());
    std::env::set_var("MAGICLAW_DB_PATH", &database_path);
    if let Err(error) =
        stock_analysis::database::DatabaseManager::init(Some(std::path::PathBuf::from(&database_path)))
    {
        eprintln!("core 数据库初始化失败 ({database_path}): {error}");
        std::process::exit(1);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("seal-board") => cmd_seal_board(&args[1..]),
        Some("print-activation") => cmd_print_activation(&args[1..]),
        Some(other) => {
            eprintln!("未知子命令: {other}");
            2
        }
        None => {
            eprintln!("用法: selection_activation_prepare <seal-board|print-activation> ...");
            2
        }
    };
    std::process::exit(code);
}

fn cmd_seal_board(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("用法: seal-board <reviewed_by> [valid_from RFC3339]");
        return 2;
    }
    let reviewed_by = &args[0];
    let now = Utc::now();
    let valid_from = match args.get(1) {
        Some(value) => match DateTime::parse_from_rfc3339(value) {
            Ok(parsed) => parsed.with_timezone(&Utc),
            Err(error) => {
                eprintln!("valid_from 解析失败: {error}");
                return 2;
            }
        },
        None => now + chrono::Duration::hours(24),
    };
    // reviewed_at(now) <= valid_from <= reviewed_at + 24h
    if reviewed_at_after_valid_from(now, valid_from) {
        eprintln!(
            "时序非法: now 必须 <= valid_from <= now+24h, 当前 now={now} valid_from={valid_from}"
        );
        return 2;
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let concept = match runtime.block_on(fetch_first(kind_concept())) {
        Ok(fact) => fact,
        Err(error) => {
            eprintln!("[封存] Magic TDX Concept 目录不可用, fail-closed: {error}");
            return 1;
        }
    };
    let industry = match runtime.block_on(fetch_first(kind_industry())) {
        Ok(fact) => fact,
        Err(error) => {
            eprintln!("[封存] Magic TDX Industry 目录不可用, fail-closed: {error}");
            return 1;
        }
    };
    eprintln!(
        "[封存] TDX 目录: Concept={}({}成员, observed_at={}) Industry={}({}成员, observed_at={})",
        concept.name,
        concept.member_count,
        concept.evidence.observed_at,
        industry.name,
        industry.member_count,
        industry.evidence.observed_at
    );

    match stock_analysis::data_gateway::board::seal_board_binding_release(
        reviewed_by,
        now,
        valid_from,
        &concept,
        &industry,
    ) {
        Ok(files) => {
            let proposal_path = root.join(
                stock_analysis::data_gateway::board::BOARD_BINDING_PROPOSAL_PATH,
            );
            let artifact_path = root.join(stock_analysis::data_gateway::board::BOARD_BINDINGS_PATH);
            if let Err(error) = std::fs::write(&proposal_path, files.proposal_json) {
                eprintln!("[封存] 写入 {proposal_path:?} 失败: {error}");
                return 1;
            }
            if let Err(error) = std::fs::write(&artifact_path, files.artifact_json) {
                eprintln!("[封存] 写入 {artifact_path:?} 失败: {error}");
                return 1;
            }
            eprintln!("[封存] 已写盘:");
            eprintln!("  {}", proposal_path.display());
            eprintln!("  {}", artifact_path.display());
            eprintln!(
                "[封存] valid_from={} expires_at={}",
                rfc3339(valid_from),
                rfc3339(valid_from + chrono::Duration::days(30))
            );
            0
        }
        Err(error) => {
            eprintln!("[封存] 失败: {error}");
            1
        }
    }
}

fn cmd_print_activation(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("用法: print-activation <reviewed_by> <effective_from RFC3339>");
        return 2;
    }
    let reviewed_by = &args[0];
    let effective_from = match DateTime::parse_from_rfc3339(&args[1]) {
        Ok(parsed) => parsed.with_timezone(&Utc),
        Err(error) => {
            eprintln!("effective_from 解析失败: {error}");
            return 2;
        }
    };
    let now = Utc::now();
    if effective_from < now {
        eprintln!("effective_from 必须在未来 (门未生效前不能提前激活): {effective_from}");
        return 2;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // 生成激活文件用 stages 1-3 (config hash) — 文件不能要求自身已存在。
    match stock_analysis::selection::config_activation_v2::prepare_activation_config_hash(
        root, now,
    ) {
        Ok(materials) => {
            let reviewed_at = rfc3339(now);
            let json = format!(
                "{{\"schema_version\":\"selection-config-activation-v1\",\
                 \"expected_config_hash\":\"{}\",\
                 \"effective_from\":\"{}\",\
                 \"reviewed_by\":\"{}\",\
                 \"reviewed_at\":\"{}\"}}",
                materials.config_hash,
                rfc3339(effective_from),
                json_escape(reviewed_by),
                reviewed_at
            );
            println!("{json}");
            eprintln!("[激活] config_hash={}", materials.config_hash);
            eprintln!("[激活] 请人工 review 后写入 config/selection/selection_activation.v1.json (紧凑 JSON + 单 LF)");
            0
        }
        Err(error) => {
            eprintln!("[激活] 材料校验失败: code={} detail={}", error.code, error.detail);
            1
        }
    }
}

fn reviewed_at_after_valid_from(now: DateTime<Utc>, valid_from: DateTime<Utc>) -> bool {
    now > valid_from || (valid_from - now) > chrono::Duration::hours(24)
}

fn kind_concept() -> stock_analysis::data_gateway::BoardKind {
    stock_analysis::data_gateway::BoardKind::Concept
}

fn kind_industry() -> stock_analysis::data_gateway::BoardKind {
    stock_analysis::data_gateway::BoardKind::Industry
}

async fn fetch_first(
    kind: stock_analysis::data_gateway::BoardKind,
) -> Result<stock_analysis::data_gateway::BoardDirectoryFact, String> {
    use stock_analysis::data_gateway::{BoardDataGateway, GatewayBatch};
    let batch = BoardDataGateway::production_tdx()
        .directory(kind, 10_000)
        .await
        .map_err(|error| format!("{kind:?} 目录不可用: {error}"))?;
    match batch {
        GatewayBatch::Available { records, .. } => records
            .into_iter()
            .next()
            .ok_or_else(|| format!("{kind:?} 目录为空 (无板块记录)")),
        GatewayBatch::VerifiedEmpty(_) => Err(format!("{kind:?} 目录已验证为空")),
    }
}

fn rfc3339(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
