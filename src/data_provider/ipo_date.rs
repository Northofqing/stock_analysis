//! v11-P0-3 commit 1: IPO 日期数据源
//!
//! 东方财富 HTTP API 拉取股票 IPO 日期 (f26 字段).
//! 复用 `SHARED_HTTP_CLIENT` (lib.rs), 零新依赖.
//!
//! 数据流: fetch_ipo_date(code) → HTTP → 解析 f26 (YYYYMMDD) → NaiveDate
//! 调用方: src/data_provider/ipo_date_filler.rs (P0-3 commit 1 末) 调 fetch_ipo_date
//!          → mark_ipo (data_quality.rs) → 填入 IPO_DATES 缓存.
//!
//! 失败策略: 网络错 / 字段空 / 解析失败 → 返回 Err, 上游决定是否重试/降级.

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::http_client::SHARED_HTTP_CLIENT;

/// 拉取股票 IPO 日期 (东方财富 f26 字段).
///
/// # Arguments
/// - `code`: 6 位股票代码 (如 "600519", "000001")
///
/// # Returns
/// - `Ok(NaiveDate)`: 成功解析
/// - `Err(_)`: 网络错 / 字段缺失 / 解析失败
///
/// # Example
/// ```ignore
/// let date = fetch_ipo_date("600519").await?;
/// assert!(date.year() >= 2000);
/// ```
pub async fn fetch_ipo_date(code: &str) -> Result<NaiveDate> {
    // 1. 6 位代码 → 1.600519 (沪) 或 0.000001 (深)
    let secid = match code.chars().next() {
        Some('6') => format!("1.{}", code),
        Some('0') | Some('2') | Some('3') => format!("0.{}", code),
        Some('8') | Some('9') => format!("0.{}", code), // 北交所: 8/92 都走 sz
        _ => anyhow::bail!("未知市场前缀: {}", code),
    };

    // 2. 东方财富 f26 = 上市日期 (YYYYMMDD, 8 位字符串)
    let url = format!(
        "https://push2.eastmoney.com/api/qt/stock/get?secid={}&fields=f26",
        secid
    );

    let body = SHARED_HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .context("东方财富 f26 请求失败")?
        .text()
        .await
        .context("东方财富 f26 读取 body 失败")?;

    // 3. 解析 JSON: {"data": {"f26": "20010827"}}
    let json: serde_json::Value =
        serde_json::from_str(&body).context("东方财富 f26 JSON 解析失败")?;

    let f26 = json["data"]["f26"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("东方财富 f26 字段缺失 (code={})", code))?;

    NaiveDate::parse_from_str(f26, "%Y%m%d")
        .with_context(|| format!("东方财富 f26 解析失败: '{}'", f26))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v11-P0-3 commit 1: secid 路由正确
    #[test]
    fn test_secid_routing() {
        assert_eq!(
            secid_for_code("600519"),
            "1.600519",
            "沪市 (6开头) → secid=1"
        );
        assert_eq!(
            secid_for_code("000001"),
            "0.000001",
            "深市主板 (0开头) → secid=0"
        );
        assert_eq!(
            secid_for_code("300750"),
            "0.300750",
            "创业板 (3开头) → secid=0"
        );
        assert_eq!(
            secid_for_code("688981"),
            "1.688981",
            "科创板 (6开头但 secid=1) → secid=1"
        );
        assert_eq!(
            secid_for_code("830799"),
            "0.830799",
            "北交所 (8开头) → secid=0"
        );
    }

    /// secid 路由 (单元测试可见)
    fn secid_for_code(code: &str) -> String {
        match code.chars().next() {
            Some('6') => format!("1.{}", code),
            Some('0') | Some('2') | Some('3') | Some('8') | Some('9') => {
                format!("0.{}", code)
            }
            _ => "unknown".to_string(),
        }
    }

    /// ⚠️ 网络依赖, `#[ignore]` 跳过 CI, 手动跑:
    ///   cargo test --lib fetch_ipo_date -- --ignored
    #[tokio::test]
    #[ignore]
    async fn fetch_ipo_date_real_network() {
        // 贵州茅台 600519, 上市日 2001-08-27
        let date = fetch_ipo_date("600519")
            .await
            .expect("600519 应该有 IPO 日期");
        assert_eq!(date, NaiveDate::from_ymd_opt(2001, 8, 27).unwrap());

        // 平安银行 000001, 上市日 1991-04-03
        let date = fetch_ipo_date("000001")
            .await
            .expect("000001 应该有 IPO 日期");
        assert_eq!(date, NaiveDate::from_ymd_opt(1991, 4, 3).unwrap());
    }
}
