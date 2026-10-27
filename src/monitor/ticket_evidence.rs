//! 单票证据包（WP6，只读）。
//!
//! 这一层只做三件事：
//! 1. 用 SQLite `mode=ro` 读取指定票/日期的原始行；
//! 2. 把结构化结果、命中的 push_log、缺口清单和哈希清单写到独立目录；
//! 3. 对缺失的原始竞价/成交证据如实记录 `EvidenceGap`。
//!
//! 它不迁移 schema、不写生产库、不把派生表当原始证据，也不计算成交概率。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::types::ValueRef;
use rusqlite::{params_from_iter, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceGap {
    pub field: String,
    pub source: String,
    pub blocked_conclusion: String,
}

#[derive(Debug, Clone)]
pub struct EvidenceRequest {
    pub db: PathBuf,
    pub delivery_db: PathBuf,
    pub code: String,
    pub as_of: String,
    pub out: PathBuf,
    pub push_log_root: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct EvidenceSummary {
    pub out: String,
    pub tickets: usize,
    pub events: usize,
    pub decisions: usize,
    pub gaps: usize,
    pub push_log_files: usize,
}

fn open_ro(path: &Path) -> Result<Connection, String> {
    if !path.is_file() {
        return Err(format!("数据库不存在: {}", path.display()));
    }
    let uri = url::Url::from_file_path(path)
        .map_err(|_| format!("非法数据库路径: {}", path.display()))?;
    let mut uri = uri;
    uri.query_pairs_mut().append_pair("mode", "ro");
    Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("只读打开失败 {}: {error}", path.display()))
}

fn value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => json!(value),
        ValueRef::Real(value) => {
            if value.is_finite() {
                json!(value)
            } else {
                Value::Null
            }
        }
        ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => Value::String(hex::encode(value)),
    }
}

fn query_json_params<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<Map<String, Value>>, String> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|error| format!("准备查询失败: {error}"))?;
    let names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let mut rows = stmt
        .query(params)
        .map_err(|error| format!("执行查询失败: {error}"))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("读取查询失败: {error}"))?
    {
        let mut map = Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = row
                .get_ref(index)
                .map(value_to_json)
                .map_err(|error| format!("读取列 {name} 失败: {error}"))?;
            map.insert(name.clone(), value);
        }
        out.push(map);
    }
    Ok(out)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("读取哈希文件失败 {}: {error}", path.display()))?;
    Ok(sha256_bytes(&bytes))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建目录失败 {}: {error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("序列化 JSON 失败 {}: {error}", path.display()))?;
    let mut bytes = bytes;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|error| format!("写 JSON 失败 {}: {error}", path.display()))
}

fn write_jsonl(path: &Path, rows: &[Map<String, Value>]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建目录失败 {}: {error}", parent.display()))?;
    }
    let mut text = String::new();
    for row in rows {
        text.push_str(
            &serde_json::to_string(row)
                .map_err(|error| format!("序列化 JSONL 行失败 {}: {error}", path.display()))?,
        );
        text.push('\n');
    }
    fs::write(path, text).map_err(|error| format!("写 JSONL 失败 {}: {error}", path.display()))
}

fn collect_push_logs(root: &Path, code: &str, limit: usize) -> Vec<PathBuf> {
    fn visit(dir: &Path, code: &str, limit: usize, out: &mut Vec<PathBuf>) {
        if out.len() >= limit {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        paths.sort();
        for path in paths {
            if out.len() >= limit {
                return;
            }
            if path.is_dir() {
                visit(&path, code, limit, out);
                continue;
            }
            let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
                continue;
            };
            if !matches!(extension, "md" | "json" | "txt") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                if text.contains(code) {
                    out.push(path);
                }
            }
        }
    }

    let mut out = Vec::new();
    visit(root, code, limit, &mut out);
    out
}

fn copy_push_logs(root: &Path, code: &str, out: &Path) -> Result<Vec<Value>, String> {
    let files = collect_push_logs(root, code, 200);
    let mut manifest = Vec::new();
    for source in files {
        let relative = source
            .strip_prefix(root)
            .map_err(|_| format!("push_log 路径不在根目录下: {}", source.display()))?;
        let target = out.join("raw").join("push_log").join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建 push_log 目录失败: {error}"))?;
        }
        let bytes = fs::read(&source)
            .map_err(|error| format!("读取 push_log 失败 {}: {error}", source.display()))?;
        fs::write(&target, &bytes)
            .map_err(|error| format!("复制 push_log 失败 {}: {error}", target.display()))?;
        manifest.push(json!({
            "source": source.to_string_lossy(),
            "relative": target.strip_prefix(out).unwrap_or(&target).to_string_lossy(),
            "sha256": sha256_bytes(&bytes),
        }));
    }
    Ok(manifest)
}

fn delivery_rows(
    conn: &Connection,
    business_date: &str,
    code: &str,
) -> Result<Vec<Map<String, Value>>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT decision_identity, business_date, push_kind, sub_kind, state, \
             envelope_canonical, envelope_sha256, created_at, updated_at \
             FROM delivery_decisions WHERE business_date = ? ORDER BY created_at, decision_identity",
        )
        .map_err(|error| format!("准备投递查询失败: {error}"))?;
    let mut rows = stmt
        .query([business_date])
        .map_err(|error| format!("执行投递查询失败: {error}"))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("读取投递查询失败: {error}"))?
    {
        let blob: Vec<u8> = row.get(5).unwrap_or_default();
        let text = String::from_utf8(blob.clone()).unwrap_or_default();
        out.push(json!({
            "decision_identity": row.get::<_, String>(0).unwrap_or_default(),
            "business_date": row.get::<_, String>(1).unwrap_or_default(),
            "push_kind": row.get::<_, String>(2).unwrap_or_default(),
            "sub_kind": row.get::<_, String>(3).unwrap_or_default(),
            "state": row.get::<_, String>(4).unwrap_or_default(),
            "envelope_hex": hex::encode(&blob),
            "envelope_text_sha256": if text.is_empty() { Value::Null } else { json!(sha256_bytes(text.as_bytes())) },
            "envelope_sha256": row.get::<_, Option<String>>(6).unwrap_or_default(),
            "contains_code": text.contains(code),
            "created_at": row.get::<_, String>(7).unwrap_or_default(),
            "updated_at": row.get::<_, String>(8).unwrap_or_default(),
        }).as_object().cloned().unwrap_or_default());
    }
    Ok(out)
}

fn build_gaps(
    tickets: &[Map<String, Value>],
    events: &[Map<String, Value>],
    decisions: &[Map<String, Value>],
    daily: &[Map<String, Value>],
    deliveries: &[Map<String, Value>],
    data_root: &Path,
    as_of: &str,
) -> Vec<EvidenceGap> {
    let mut gaps = Vec::new();
    if decisions.is_empty() {
        gaps.push(EvidenceGap {
            field: "ticket_decision".to_owned(),
            source: "ticket_decision".to_owned(),
            blocked_conclusion: "没有 DecisionContext/EntryPolicy 记录；不能声称已按新内核决策"
                .to_owned(),
        });
    }
    if !tickets
        .iter()
        .any(|row| row.get("stage").and_then(Value::as_str) == Some("entry_proposed"))
    {
        gaps.push(EvidenceGap {
            field: "ticket_stage".to_owned(),
            source: "ticket_lifecycle".to_owned(),
            blocked_conclusion: "该票在 as_of 不是 entry_proposed；不能按入场触发语义评价"
                .to_owned(),
        });
    }
    if !events
        .iter()
        .any(|row| row.get("event_kind").and_then(Value::as_str) == Some("entry_proposed"))
    {
        gaps.push(EvidenceGap {
            field: "entry_proposed_event".to_owned(),
            source: "ticket_event".to_owned(),
            blocked_conclusion: "没有 entry_proposed 事件；缺少提案原文和时间".to_owned(),
        });
    }
    if !daily
        .iter()
        .any(|row| row.get("date").and_then(Value::as_str) == Some(as_of))
    {
        gaps.push(EvidenceGap {
            field: "stock_daily_exact_date".to_owned(),
            source: "stock_daily".to_owned(),
            blocked_conclusion: format!("{as_of} 无该票收盘价；不能用相邻日期替代"),
        });
    }
    if !data_root.join("auction_raw").exists() {
        gaps.push(EvidenceGap {
            field: "auction_raw".to_owned(),
            source: "data/auction_raw".to_owned(),
            blocked_conclusion: "没有原始竞价/逐笔/成交证据；不能证明成交概率或排队".to_owned(),
        });
    }
    if !deliveries.iter().any(|row| {
        row.get("push_kind").and_then(Value::as_str) == Some("HoldingPlan")
            && row.get("state").and_then(Value::as_str) == Some("Delivered")
    }) {
        gaps.push(EvidenceGap {
            field: "HoldingPlan_delivery".to_owned(),
            source: "delivery_decisions".to_owned(),
            blocked_conclusion: "没有发现该日 HoldingPlan 的 Delivered 记录".to_owned(),
        });
    }
    gaps
}

pub fn build_evidence_pack(request: &EvidenceRequest) -> Result<EvidenceSummary, String> {
    if request.out.exists()
        && fs::read_dir(&request.out)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false)
    {
        return Err(format!("输出目录非空，拒绝覆盖: {}", request.out.display()));
    }
    fs::create_dir_all(request.out.join("raw"))
        .map_err(|error| format!("创建证据包目录失败: {error}"))?;

    let main = open_ro(&request.db)?;
    let delivery = open_ro(&request.delivery_db)?;

    let tickets = query_json_params(
        &main,
        "SELECT id, code, open_date, close_date, stage, source, note, created_at, updated_at \
         FROM ticket_lifecycle WHERE code = ? ORDER BY open_date, id",
        [request.code.as_str()],
    )?;
    let ticket_ids: Vec<i64> = tickets
        .iter()
        .filter_map(|row| row.get("id").and_then(Value::as_i64))
        .collect();
    let (events, decisions) = if ticket_ids.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let placeholders = std::iter::repeat("?")
            .take(ticket_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let event_sql = format!(
            "SELECT id, ticket_id, event_at, event_kind, push_kind, occurrence_identity, summary, created_at \
             FROM ticket_event WHERE ticket_id IN ({placeholders}) ORDER BY event_at, id"
        );
        let events = query_json_params(&main, &event_sql, params_from_iter(ticket_ids.iter()))?;
        let decision_sql = format!(
            "SELECT decision_id, ticket_id, code, as_of, source, regime, entry_mode, proposal_status, \
             price_low, price_high, trigger_level, abandon, valid_from, valid_until, evidence_grade, \
             source_health, context_json, rule_version, supersedes, created_at, updated_at \
             FROM ticket_decision WHERE ticket_id IN ({placeholders}) ORDER BY as_of, decision_id"
        );
        let decisions =
            query_json_params(&main, &decision_sql, params_from_iter(ticket_ids.iter()))?;
        (events, decisions)
    };

    let daily = query_json_params(
        &main,
        "SELECT code, date, open, high, low, close, volume, amount, pct_chg, ma5, ma10, ma20, data_source \
         FROM stock_daily WHERE code = ? AND date BETWEEN date(?, '-10 day') AND date(?, '+10 day') ORDER BY date",
        [request.code.as_str(), request.as_of.as_str(), request.as_of.as_str()],
    )?;
    let regime = query_json_params(
        &main,
        "SELECT trade_date, limit_up_count, max_streak, broken_count, limit_down_count, prev_pool_count, \
         prev_premium, phase, captured_at FROM market_regime_daily \
         WHERE trade_date BETWEEN date(?, '-10 day') AND date(?, '+10 day') ORDER BY trade_date",
        [request.as_of.as_str(), request.as_of.as_str()],
    )?;
    let benchmark = query_json_params(
        &main,
        "SELECT trade_date, code, stock_pct, market_pct, board_pct, excess_pct, sample_size, benchmark \
         FROM attribution_benchmark_daily WHERE code = ? AND trade_date BETWEEN date(?, '-10 day') AND date(?, '+10 day') \
         ORDER BY trade_date",
        [request.code.as_str(), request.as_of.as_str(), request.as_of.as_str()],
    )?;
    let advice = query_json_params(
        &main,
        "SELECT occurrence_identity, business_date, push_kind, code, pushed_at, executed, execution_source, \
         outcome_pct, expected_price, change_t1, change_t3, change_t5, mfe, mae, note \
         FROM advice_outcome_tracking WHERE code = ? ORDER BY business_date, id",
        [request.code.as_str()],
    )?;
    let deliveries = delivery_rows(&delivery, &request.as_of, &request.code)?;
    let push_manifest = copy_push_logs(&request.push_log_root, &request.code, &request.out)?;
    let push_log_files = push_manifest.len();
    let gaps = build_gaps(
        &tickets,
        &events,
        &decisions,
        &daily,
        &deliveries,
        request.db.parent().unwrap_or_else(|| Path::new(".")),
        &request.as_of,
    );

    write_json(
        &request.out.join("raw").join("ticket_lifecycle.json"),
        &Value::Array(tickets.iter().cloned().map(Value::Object).collect()),
    )?;
    write_jsonl(&request.out.join("raw").join("ticket_event.jsonl"), &events)?;
    write_jsonl(
        &request.out.join("raw").join("ticket_decision.jsonl"),
        &decisions,
    )?;
    write_jsonl(&request.out.join("raw").join("stock_daily.jsonl"), &daily)?;
    write_json(
        &request.out.join("raw").join("market_regime_daily.json"),
        &Value::Array(regime.iter().cloned().map(Value::Object).collect()),
    )?;
    write_json(
        &request
            .out
            .join("raw")
            .join("attribution_benchmark_daily.json"),
        &Value::Array(benchmark.iter().cloned().map(Value::Object).collect()),
    )?;
    write_json(
        &request.out.join("raw").join("advice_outcome_tracking.json"),
        &Value::Array(advice.iter().cloned().map(Value::Object).collect()),
    )?;
    write_json(
        &request.out.join("raw").join("delivery_decisions.json"),
        &Value::Array(deliveries.iter().cloned().map(Value::Object).collect()),
    )?;
    write_json(
        &request.out.join("raw").join("push_log_index.json"),
        &Value::Array(push_manifest),
    )?;

    let gaps_json =
        serde_json::to_value(&gaps).map_err(|error| format!("序列化 gaps 失败: {error}"))?;
    write_json(&request.out.join("gaps.json"), &gaps_json)?;

    let mut timeline: Vec<Value> = Vec::new();
    for row in &events {
        timeline.push(json!({"at": row.get("event_at"), "kind": "ticket_event", "data": row}));
    }
    for row in &daily {
        timeline.push(json!({"at": row.get("date"), "kind": "stock_daily", "data": row}));
    }
    for row in &regime {
        timeline.push(json!({"at": row.get("trade_date"), "kind": "market_regime", "data": row}));
    }
    for row in &deliveries {
        timeline
            .push(json!({"at": row.get("created_at"), "kind": "delivery_decision", "data": row}));
    }
    timeline.sort_by_key(|item| {
        item.get("at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    });
    let timeline_text = timeline
        .iter()
        .map(|item| serde_json::to_string(item).unwrap_or_else(|_| "{}".to_owned()))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        request.out.join("timeline.jsonl"),
        format!("{timeline_text}\n"),
    )
    .map_err(|error| format!("写 timeline 失败: {error}"))?;

    let memo = format!(
        "# 单票证据包：{} / {}\n\n## 事实\n- ticket 行数：{}\n- ticket_event：{} 条\n- ticket_decision：{} 条\n- stock_daily：{} 条（含 as-of 前后）\n- delivery_decisions：{} 条（该业务日）\n- push_log 命中文件：{} 个\n\n## 证据缺口\n{}\n\n## 边界\n- 本包只读生产库；派生表不能替代原始竞价/成交证据。\n- 缺失字段不补 0、不补价格、不推断成交概率。\n",
        request.code,
        request.as_of,
        tickets.len(),
        events.len(),
        decisions.len(),
        daily.len(),
        deliveries.len(),
        push_log_files,
        if gaps.is_empty() {
            "- 无结构性缺口；仍不能据此证明成交概率".to_owned()
        } else {
            gaps.iter().map(|gap| format!("- {}：{}", gap.field, gap.blocked_conclusion)).collect::<Vec<_>>().join("\n")
        }
    );
    fs::write(request.out.join("memo.md"), memo)
        .map_err(|error| format!("写 memo 失败: {error}"))?;

    write_json(
        &request.out.join("sources.json"),
        &json!({
            "code": request.code,
            "as_of": request.as_of,
            "main_db": request.db.to_string_lossy(),
            "delivery_db": request.delivery_db.to_string_lossy(),
            "push_log_root": request.push_log_root.to_string_lossy(),
        }),
    )?;

    let mut files = BTreeMap::new();
    fn walk(dir: &Path, root: &Path, files: &mut BTreeMap<String, String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, files);
            } else if path.file_name().and_then(|name| name.to_str()) != Some("manifest.sha256") {
                if let Ok(relative) = path.strip_prefix(root) {
                    files.insert(
                        relative.to_string_lossy().to_string(),
                        sha256_file(&path).unwrap_or_default(),
                    );
                }
            }
        }
    }
    walk(&request.out, &request.out, &mut files);
    let manifest = files
        .iter()
        .map(|(path, hash)| format!("{hash}  {path}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(request.out.join("manifest.sha256"), format!("{manifest}\n"))
        .map_err(|error| format!("写 manifest 失败: {error}"))?;

    Ok(EvidenceSummary {
        out: request.out.to_string_lossy().to_string(),
        tickets: tickets.len(),
        events: events.len(),
        decisions: decisions.len(),
        gaps: gaps.len(),
        push_log_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_serializes_as_machine_readable_item() {
        let gap = EvidenceGap {
            field: "auction_raw".to_owned(),
            source: "data/auction_raw".to_owned(),
            blocked_conclusion: "不能证明成交概率".to_owned(),
        };
        let value = serde_json::to_value(gap).unwrap();
        assert_eq!(value["field"], "auction_raw");
        assert_eq!(value["source"], "data/auction_raw");
    }

    #[test]
    fn value_to_json_keeps_blob_as_hex_not_lossy_text() {
        let value = value_to_json(ValueRef::Blob(&[0x00, 0xff, 0x10]));
        assert_eq!(value, json!("00ff10"));
    }
}
