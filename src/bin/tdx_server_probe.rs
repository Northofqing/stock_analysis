//! TDX 主站延迟探针 — 方案 A 可行性验证（找 servertime age < 5s 的主站）。
//!
//! 背景: TDX 免费公共主站 servertime 结构性滞后 6-63s（median ~18s, 2026-08-10
//! 全天 0 次通过 5s 门, 见 magic_tdx_t0.rs 模块头注释）。本探针逐台连接
//! ALL_KNOWN_SERVERS（或 --primary 只测 PRIMARY_SERVERS 10 台）, 每台取
//! 600396(贵州茅台) 实时 quote, 解析 servertime 计算 age, 按 age 升序输出。
//!
//! 判定: age_secs < 5 的主站可过 T0_QUOTE_MAX_AGE_SECS / BR-217 新鲜度门。
//! 盘中跑才有意义（盘后 servertime 停在 15:00, age 必然数小时——此时跑用于
//! 验证探针本身工作正常 + 服务器可达性）。
//!
//! 用法:
//!   cargo run --bin tdx_server_probe            # 全量 101 台 (~5 分钟)
//!   cargo run --bin tdx_server_probe -- --primary  # 只测 10 台优先主站 (~30s)
//!
//! 只读探测, 无写入、无推送、不修改上游服务器列表。找到快主站后另行在
//! magic-market-data-rs 上游调 PRIMARY_SERVERS 顺序 / reorder_servers。

use std::time::Instant;

#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::protocol::constants::{ALL_KNOWN_SERVERS, PRIMARY_SERVERS};
#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::TdxHqClient;

/// 探测目标: 贵州茅台（沪市 market=1, 全天活跃）
const PROBE_CODE: u8 = 1;
const PROBE_INSTRUMENT: &str = "600396";
/// 与生产一致的 5s 新鲜度门
const FRESHNESS_GATE_SECS: i64 = 5;
/// 单台连接超时
const CONNECT_TIMEOUT_SECS: f64 = 3.0;

#[derive(Debug)]
struct ProbeResult {
    name: &'static str,
    ip: &'static str,
    port: u16,
    tcp_ms: u128,
    servertime: Option<String>,
    age_secs: Option<i64>,
    note: &'static str,
}

fn main() {
    let only_primary = std::env::args().any(|arg| arg == "--primary");
    let servers: &[(&str, &str, u16)] = if only_primary {
        PRIMARY_SERVERS
    } else {
        ALL_KNOWN_SERVERS
    };
    let now = chrono::Local::now();
    println!(
        "[probe] TDX 主站延迟探针 start={} servers={} timeout={}s code={}{}",
        now.format("%Y-%m-%d %H:%M:%S %z"),
        servers.len(),
        CONNECT_TIMEOUT_SECS,
        PROBE_INSTRUMENT,
        if only_primary { " (--primary)" } else { "" }
    );

    let mut results: Vec<ProbeResult> = Vec::with_capacity(servers.len());
    let mut unreachable = 0_usize;
    for &(name, ip, port) in servers {
        let t0 = Instant::now();
        let client = TdxHqClient::new();
        match client.connect(ip, port, Some(CONNECT_TIMEOUT_SECS)) {
            Ok(true) => {
                let tcp_ms = t0.elapsed().as_millis();
                match client.get_security_quotes(&[(PROBE_CODE, PROBE_INSTRUMENT)]) {
                    Ok(quotes) => {
                        if let Some(quote) = quotes.first() {
                            let (servertime, age_secs, note) =
                                evaluate_servertime(&quote.servertime, now.time());
                            results.push(ProbeResult {
                                name,
                                ip,
                                port,
                                tcp_ms,
                                servertime: Some(servertime),
                                age_secs,
                                note,
                            });
                        } else {
                            results.push(ProbeResult {
                                name,
                                ip,
                                port,
                                tcp_ms,
                                servertime: None,
                                age_secs: None,
                                note: "empty-quote",
                            });
                        }
                    }
                    Err(error) => {
                        eprintln!("[probe] quote-error {name} {ip}:{port}: {error}");
                        results.push(ProbeResult {
                            name,
                            ip,
                            port,
                            tcp_ms,
                            servertime: None,
                            age_secs: None,
                            note: "quote-error",
                        });
                    }
                }
            }
            Ok(false) => {
                unreachable += 1;
                // 连接被拒/无响应不细报, 汇总即可
            }
            Err(_) => {
                unreachable += 1;
            }
        }
    }

    // 按 age 升序（无 age 的排最后, 按 tcp_ms）
    results.sort_by(|left, right| match (left.age_secs, right.age_secs) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.tcp_ms.cmp(&right.tcp_ms),
    });

    println!(
        "[probe] unreachable={} reachable={}",
        unreachable,
        results.len()
    );
    println!("[probe] ── 按 servertime age 升序 (age<5s 可通过新鲜度门) ──");
    let mut pass_count = 0_usize;
    for r in &results {
        match (r.age_secs, &r.servertime) {
            (Some(age), Some(st)) => {
                let gate = if age < FRESHNESS_GATE_SECS {
                    " <== PASS"
                } else {
                    ""
                };
                if age < FRESHNESS_GATE_SECS {
                    pass_count += 1;
                }
                println!(
                    "[probe] {:<12} {:<15}:{} tcp_ms={:>4} servertime={} age={:>4}s{}",
                    r.name, r.ip, r.port, r.tcp_ms, st, age, gate
                );
            }
            _ => println!(
                "[probe] {:<12} {:<15}:{} tcp_ms={:>4} note={}",
                r.name, r.ip, r.port, r.tcp_ms, r.note
            ),
        }
    }
    println!(
        "[probe] 汇总: age<5s 可过门的主站 = {} 台 / 可达 {} 台",
        pass_count,
        results.len()
    );
}

/// 解析 TDX servertime (HH:MM[:SS], 无日期 — 用观测时刻的日期补全, 与
/// magic_tdx_t0::source_time 同语义) 并计算 age。
fn evaluate_servertime(raw: &str, now: chrono::NaiveTime) -> (String, Option<i64>, &'static str) {
    let st = raw.trim();
    if st.is_empty() {
        return (String::new(), None, "no-servertime");
    }
    let parsed = chrono::NaiveTime::parse_from_str(st, "%H:%M:%S")
        .or_else(|_| chrono::NaiveTime::parse_from_str(st, "%H:%M"));
    match parsed {
        Ok(t) => {
            let age = now.signed_duration_since(t).num_seconds();
            (st.to_string(), Some(age), "ok")
        }
        Err(_) => (st.to_string(), None, "servertime-unparseable"),
    }
}
