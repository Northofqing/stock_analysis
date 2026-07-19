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
    fetch_ipo_date_with_client(&SHARED_HTTP_CLIENT, code).await
}

fn secid_for_code(code: &str) -> Result<String> {
    #[cfg(not(test))]
    let normalized = code;
    #[cfg(test)]
    let normalized = code.strip_prefix("TEST_CODE_").unwrap_or(code);
    let secid = match normalized.chars().next() {
        Some('6') => format!("1.{}", normalized),
        Some('0') | Some('2') | Some('3') => format!("0.{}", normalized),
        Some('8') | Some('9') => format!("0.{}", normalized), // 北交所: 8/92 都走 sz
        _ => anyhow::bail!("未知市场前缀: {}", code),
    };
    Ok(secid)
}

fn parse_ipo_date_body(body: &str, code: &str) -> Result<NaiveDate> {
    let json: serde_json::Value =
        serde_json::from_str(body).context("东方财富 f26 JSON 解析失败")?;
    let f26 = json["data"]["f26"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("东方财富 f26 字段缺失 (code={})", code))?;
    NaiveDate::parse_from_str(f26, "%Y%m%d")
        .with_context(|| format!("东方财富 f26 解析失败: '{}'", f26))
}

async fn fetch_ipo_date_with_client(client: &reqwest::Client, code: &str) -> Result<NaiveDate> {
    fetch_ipo_date_from_base(client, code, "https://push2.eastmoney.com").await
}

async fn fetch_ipo_date_from_base(
    client: &reqwest::Client,
    code: &str,
    base: &str,
) -> Result<NaiveDate> {
    // 1. 6 位代码 → 1.600519 (沪) 或 0.000001 (深)
    let secid = secid_for_code(code)?;

    // 2. 东方财富 f26 = 上市日期 (YYYYMMDD, 8 位字符串)
    let url = format!(
        "{}/api/qt/stock/get?secid={}&fields=f26",
        base.trim_end_matches('/'),
        secid,
    );

    let body = client
        .get(&url)
        .send()
        .await
        .context("东方财富 f26 请求失败")?
        .text()
        .await
        .context("东方财富 f26 读取 body 失败")?;

    // 3. 解析 JSON: {"data": {"f26": "20010827"}}
    parse_ipo_date_body(&body, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v11-P0-3 commit 1: secid 路由正确
    #[test]
    fn test_secid_routing() {
        assert_eq!(secid_for_code("TEST_CODE_600519").unwrap(), "1.600519");
        assert_eq!(secid_for_code("TEST_CODE_000001").unwrap(), "0.000001");
        assert_eq!(secid_for_code("TEST_CODE_300750").unwrap(), "0.300750");
        assert_eq!(secid_for_code("TEST_CODE_688981").unwrap(), "1.688981");
        assert_eq!(secid_for_code("TEST_CODE_830799").unwrap(), "0.830799");
        assert_eq!(secid_for_code("TEST_CODE_920001").unwrap(), "0.920001");
        assert!(secid_for_code("TEST_CODE_BAD").is_err());
    }

    #[test]
    fn ipo_body_parser_requires_a_real_eight_digit_date() {
        assert_eq!(
            parse_ipo_date_body(r#"{"data":{"f26":"20010827"}}"#, "TEST_CODE_000001").unwrap(),
            NaiveDate::from_ymd_opt(2001, 8, 27).unwrap()
        );
        assert!(parse_ipo_date_body("not-json", "TEST_CODE_000001").is_err());
        assert!(parse_ipo_date_body(r#"{"data":{}}"#, "TEST_CODE_000001").is_err());
        assert!(parse_ipo_date_body(r#"{"data":{"f26":"20260230"}}"#, "TEST_CODE_000001").is_err());
    }

    #[tokio::test]
    async fn ipo_transport_failure_is_explicit() {
        let client = super::super::unreachable_http_client();
        assert!(fetch_ipo_date_with_client(&client, "TEST_CODE_920001")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn loopback_ipo_transport_preserves_route_and_real_date() {
        use super::super::{loopback_http_client, TestHttpResponse, TestHttpServer};

        let server = TestHttpServer::new(vec![TestHttpResponse::json(
            r#"{"data":{"f26":"20010827"}}"#,
        )]);
        let date = fetch_ipo_date_from_base(
            &loopback_http_client(),
            "TEST_CODE_600519",
            server.base_url(),
        )
        .await
        .unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2001, 8, 27).unwrap());
        assert!(server.finish()[0].contains("secid=1.600519"));
    }

    #[tokio::test]
    async fn two_market_routes_preserve_complete_ipo_dates_without_live_network() {
        use super::super::{loopback_http_client, TestHttpResponse, TestHttpServer};
        let server = TestHttpServer::new(vec![
            TestHttpResponse::json(r#"{"data":{"f26":"20010827"}}"#),
            TestHttpResponse::json(r#"{"data":{"f26":"19910403"}}"#),
        ]);
        let client = loopback_http_client();
        let sh = fetch_ipo_date_from_base(&client, "TEST_CODE_600519", server.base_url()).await;
        let sz = fetch_ipo_date_from_base(&client, "TEST_CODE_000001", server.base_url()).await;
        assert_eq!(sh.unwrap(), NaiveDate::from_ymd_opt(2001, 8, 27).unwrap());
        assert_eq!(sz.unwrap(), NaiveDate::from_ymd_opt(1991, 4, 3).unwrap());
        assert_eq!(server.finish().len(), 2);
    }
}
