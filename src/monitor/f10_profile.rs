//! TDX F10 公司资料落库（2026-10-18）。
//!
//! ## 为什么
//!
//! 探针验证过：Mac 能直连 `117.34.114.14:7709`，四类数据全通。
//! 现在把它们落库，供 AI 分析产业链使用。
//!
//! ## 四类
//!
//! - **经营分析** 主营业务 + 主营构成（收入占比、毛利率）← AI 的核心输入
//! - **行业分析** 行业地位、竞争格局
//! - **最新提示** 停牌、风险警示
//! - **股本结构** 流通盘
//!
//! ## 分类名带填充的坑（实测）
//!
//! 部分分类名后面跟着 `\0` 和 `\x01\x02\x04\x0c` 等控制字节（600396 有 7 个）。
//! 所以按**前缀匹配**找 `F10Category`，再传给 `get_content`——它内部优先用
//! `category.filename_raw`（原始 GBK 字节），完全绕开名称匹配。
//!
//! ## 边界
//!
//! - 只落原文，不做结构化解析（解析交给下游 / AI）
//! - F10 是"最新快照"，每次抓覆盖同一 `(code, category)`
//! - 单票失败不阻塞其他票

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};

use magic_tdx_rs::net::f10_client::TdxF10Client;

use crate::database::DatabaseManager;

/// 需要的四个分类
pub const CATEGORY_BUSINESS: &str = "经营分析";
pub const CATEGORY_INDUSTRY: &str = "行业分析";
pub const CATEGORY_NOTICE: &str = "最新提示";
pub const CATEGORY_EQUITY: &str = "股本结构";

const WANTED: [&str; 4] = [
    CATEGORY_BUSINESS,
    CATEGORY_INDUSTRY,
    CATEGORY_NOTICE,
    CATEGORY_EQUITY,
];

const SERVER_IP: &str = "117.34.114.14";
const SERVER_PORT: u16 = 7709;
const TIMEOUT_SECS: f64 = 15.0;

#[derive(Debug, Default)]
pub struct F10Report {
    pub codes: usize,
    pub fetched: usize,
    pub failed: Vec<String>,
    pub total_bytes: usize,
}

impl F10Report {
    pub fn log(&self) {
        log::info!(
            "[F10] codes={} fetched={} failed={} total_bytes={}",
            self.codes,
            self.fetched,
            self.failed.len(),
            self.total_bytes
        );
        for item in &self.failed {
            log::warn!("[F10] {item}");
        }
    }
}

fn create_schema(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS stock_f10_profile (\
            code TEXT NOT NULL,\
            category TEXT NOT NULL,\
            content TEXT NOT NULL,\
            byte_length INTEGER NOT NULL,\
            fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,\
            PRIMARY KEY (code, category)\
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS ix_stock_f10_profile_code \
         ON stock_f10_profile (code)",
    )
    .execute(conn)?;
    Ok(())
}

fn market_for(code: &str) -> u8 {
    match code.chars().next() {
        Some('6') => 1, // 上海
        _ => 0,         // 深圳
    }
}

/// 拉一批票的 F10 资料并落库。
pub fn refresh_f10(db: &DatabaseManager, codes: &[String]) -> Result<F10Report, String> {
    if codes.is_empty() {
        return Ok(F10Report::default());
    }

    let mut report = F10Report {
        codes: codes.len(),
        ..Default::default()
    };
    let client = TdxF10Client::new(SERVER_IP, SERVER_PORT, Some(TIMEOUT_SECS));

    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    create_schema(&mut conn).map_err(|error| error.to_string())?;

    for code in codes {
        let market = market_for(code);
        let categories = match client.get_category(market, code) {
            Ok(value) => value,
            Err(error) => {
                report.failed.push(format!("{code} 列分类失败: {error}"));
                continue;
            }
        };

        for name in WANTED {
            let Some(category) = categories
                .iter()
                .find(|candidate| candidate.name.starts_with(name))
            else {
                report.failed.push(format!("{code} 缺分类 {name}"));
                continue;
            };

            match client.get_content(market, code, category) {
                Ok(content) => {
                    let written = diesel::sql_query(
                        "INSERT INTO stock_f10_profile (code, category, content, byte_length) \
                         VALUES (?, ?, ?, ?) \
                         ON CONFLICT(code, category) DO UPDATE SET \
                            content=excluded.content, \
                            byte_length=excluded.byte_length, \
                            fetched_at=CURRENT_TIMESTAMP",
                    )
                    .bind::<Text, _>(code)
                    .bind::<Text, _>(name)
                    .bind::<Text, _>(&content.content)
                    .bind::<BigInt, _>(content.byte_length as i64)
                    .execute(&mut conn);

                    match written {
                        Ok(_) => {
                            report.fetched += 1;
                            report.total_bytes += content.byte_length;
                        }
                        Err(error) => report
                            .failed
                            .push(format!("{code} {name} 落库失败: {error}")),
                    }
                }
                Err(error) => report
                    .failed
                    .push(format!("{code} {name} 取数失败: {error}")),
            }
        }
    }

    Ok(report)
}

/// 读某只票的某个分类原文。没有记录返回 `Ok(None)`（不是错误）。
pub fn load_category(
    conn: &mut SqliteConnection,
    code: &str,
    category: &str,
) -> Result<Option<String>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<Text>)]
        content: Option<String>,
    }
    let row: Option<Row> =
        diesel::sql_query("SELECT content FROM stock_f10_profile WHERE code = ? AND category = ?")
            .bind::<Text, _>(code)
            .bind::<Text, _>(category)
            .get_result(conn)
            .ok();
    Ok(row.and_then(|value| value.content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wanted_categories_are_four() {
        assert_eq!(WANTED.len(), 4);
        assert!(WANTED.contains(&"经营分析"));
        assert!(WANTED.contains(&"行业分析"));
    }

    #[test]
    fn market_mapping() {
        assert_eq!(market_for("600396"), 1);
        assert_eq!(market_for("000338"), 0);
        assert_eq!(market_for("300750"), 0);
    }
}
