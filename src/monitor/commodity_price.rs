//! 5.1（2026-10-24）：产业链商品价格日更。
//!
//! 源：新浪 `hq.sinajs.cn`（GBK，必须带 `Referer: https://finance.sina.com.cn`），
//! 一次请求取全部品种。字段位置：内盘 `nf_*` 现价在第 8 个字段，外盘 `hf_*` 在第 1 个。
//! 探测结论（2026-10-24）：`nf_ZC0` 动力煤长期空报价 → 不写行、日志记「无源」，**不补 0**。
//!
//! 表：`commodity_price_daily(code, trade_date)` 主键，逐日 upsert；
//! `commodity_symbol_map` 存 code → 显示名 → 新浪代码，后续接 `chain_relation_candidate`
//! 的商品名时在这里加映射，不在代码里散落字符串。

use crate::database::DatabaseManager;
use diesel::sql_types::Text;
use diesel::RunQueryDsl as _;
use std::collections::HashMap;

/// (code, 显示名, 新浪代码)。顺序即日志顺序。
pub const COMMODITIES: &[(&str, &str, &str)] = &[
    ("RB", "螺纹钢", "nf_RB0"),
    ("I", "铁矿石", "nf_I0"),
    ("CU", "铜", "nf_CU0"),
    ("AL", "铝", "nf_AL0"),
    ("J", "焦炭", "nf_J0"),
    ("AU", "黄金", "nf_AU0"),
    ("GC", "纽约黄金", "hf_GC"),
    ("CL", "纽约原油", "hf_CL"),
    ("ZC", "动力煤", "nf_ZC0"),
];

const ENDPOINT: &str = "https://hq.sinajs.cn/list=";

#[derive(Debug, Default, Clone)]
pub struct CommodityReport {
    pub requested: usize,
    pub written: usize,
    pub missing: Vec<String>,
    pub errors: Vec<String>,
}

impl CommodityReport {
    pub fn log(&self) {
        log::info!(
            "[商品价格] requested={} written={} missing={} errors={}",
            self.requested,
            self.written,
            self.missing.len(),
            self.errors.len()
        );
        if !self.missing.is_empty() {
            log::warn!("[商品价格] 无报价（不补 0）: {}", self.missing.join(","));
        }
    }
}

pub fn ensure_schema(conn: &mut diesel::SqliteConnection) -> Result<(), String> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS commodity_price_daily (
            code TEXT NOT NULL,
            name TEXT NOT NULL,
            price REAL NOT NULL,
            source TEXT NOT NULL DEFAULT 'sina',
            trade_date TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (code, trade_date)
        )",
    )
    .execute(conn)
    .map_err(|error| format!("commodity_price_daily 建表失败: {error}"))?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS commodity_symbol_map (
            code TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            sina_symbol TEXT NOT NULL,
            note TEXT
        )",
    )
    .execute(conn)
    .map_err(|error| format!("commodity_symbol_map 建表失败: {error}"))?;
    // 映射表随代码走：每次 ensure 都把常量写进去（INSERT OR REPLACE，幂等）。
    for (code, name, symbol) in COMMODITIES {
        let _ = diesel::sql_query(
            "INSERT INTO commodity_symbol_map (code, name, sina_symbol) VALUES (?, ?, ?) \
             ON CONFLICT(code) DO UPDATE SET name=excluded.name, sina_symbol=excluded.sina_symbol",
        )
        .bind::<Text, _>(*code)
        .bind::<Text, _>(*name)
        .bind::<Text, _>(*symbol)
        .execute(conn);
    }
    Ok(())
}

/// 只做网络请求 + 解析（不碰库）；探测/单测与日更共用。
pub async fn fetch_quotes() -> Result<HashMap<String, Option<f64>>, String> {
    let symbols: Vec<&str> = COMMODITIES.iter().map(|(_, _, symbol)| *symbol).collect();
    let url = format!("{ENDPOINT}{}", symbols.join(","));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|error| format!("http client: {error}"))?;
    let response = client
        .get(&url)
        .header("Referer", "https://finance.sina.com.cn")
        .send()
        .await
        .map_err(|error| format!("新浪请求失败: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("新浪返回 HTTP {status}"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取响应失败: {error}"))?;
    let (text, _, _) = encoding_rs::GBK.decode(&bytes);
    Ok(parse_payload(&text))
}

/// 落库：逐品种 upsert；空报价只记 missing，不写 0（BR-222）。
pub fn persist(
    conn: &mut diesel::SqliteConnection,
    fallback_date: &str,
    parsed: &HashMap<String, Option<f64>>,
) -> Result<CommodityReport, String> {
    ensure_schema(conn)?;
    let mut report = CommodityReport {
        requested: COMMODITIES.len(),
        ..Default::default()
    };
    let mut written = 0usize;
    for (code, name, symbol) in COMMODITIES {
        match parsed.get(*symbol).copied().flatten() {
            Some(price) if price > 0.0 => {
                diesel::sql_query(
                    "INSERT INTO commodity_price_daily (code, name, price, source, trade_date) \
                     VALUES (?1, ?2, ?3, 'sina', ?4) \
                     ON CONFLICT(code, trade_date) DO UPDATE SET \
                       name=excluded.name, price=excluded.price, source=excluded.source, \
                       updated_at=CURRENT_TIMESTAMP",
                )
                .bind::<Text, _>(*code)
                .bind::<Text, _>(*name)
                .bind::<diesel::sql_types::Double, _>(price)
                .bind::<Text, _>(fallback_date.to_owned())
                .execute(&mut *conn)
                .map_err(|error| format!("{code} 落库失败: {error}"))?;
                written += 1;
            }
            _ => report.missing.push((*code).to_string()),
        }
    }
    report.written = written;
    Ok(report)
}

/// 拉一次并落库（monitor 16:10 调度用）。
pub async fn refresh(db: &DatabaseManager, fallback_date: &str) -> Result<CommodityReport, String> {
    let parsed = fetch_quotes().await?;
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    persist(&mut conn, fallback_date, &parsed)
}

/// 解析新浪返回文本。返回 symbol → 现价（None = 空报价）。
pub(crate) fn parse_payload(text: &str) -> HashMap<String, Option<f64>> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("var hq_str_") else {
            continue;
        };
        let Some((symbol, payload)) = rest.split_once('=') else {
            continue;
        };
        let payload = payload
            .trim()
            .trim_end_matches(';')
            .trim()
            .trim_matches('"');
        if payload.is_empty() {
            out.insert(symbol.to_string(), None);
            continue;
        }
        let fields: Vec<&str> = payload.split(',').collect();
        // 内盘 nf_*：现价在索引 8；外盘 hf_*：索引 0（索引 1 是空字段，2026-10-24 实测）。
        let index = if symbol.starts_with("hf_") { 0 } else { 8 };
        let price = fields
            .get(index)
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| *value > 0.0);
        out.insert(symbol.to_string(), price);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_domestic_foreign_and_empty_quotes() {
        let payload = concat!(
            "var hq_str_nf_RB0=\"螺纹钢连续,230000,3053.000,3053.000,3037.000,0.000,3041.000,3042.000,3042.000,0.000\";\n",
            "var hq_str_nf_ZC0=\"\";\n",
            "var hq_str_hf_GC=\"4007.910,,4009.500,4010.000,4050.200,3989.100,04:59:58,4060.400,4042.200,0,2,1,2026-10-24,纽约黄金,0\";\n"
        );
        let parsed = parse_payload(payload);
        assert_eq!(parsed.get("nf_RB0").copied().flatten(), Some(3042.0));
        assert_eq!(parsed.get("nf_ZC0").copied().flatten(), None);
        assert_eq!(parsed.get("hf_GC").copied().flatten(), Some(4007.91));
    }

    #[test]
    fn symbol_map_covers_every_commodity() {
        assert!(COMMODITIES.len() >= 8);
        let mut codes: Vec<&str> = COMMODITIES.iter().map(|(code, _, _)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), COMMODITIES.len(), "code 不能重复");
    }
}
