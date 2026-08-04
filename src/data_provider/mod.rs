//! 兼容领域数据模型与本地派生服务。
//!
//! 外部金融/新闻获取统一由 `crate::data_gateway` 持有；本模块只保留
//! `KlineData`、一致预期投影、涨跌停校验、筹码分布等既有领域类型和计算。

pub mod chip_distribution;
pub mod consensus;
pub mod halt_status;
pub mod limit_status;
pub mod service;
// review #16: 新闻条目结构 + content_hash
pub mod news_item;

pub use chip_distribution::{
    compute_chip_distribution, format_for_prompt as format_chip_prompt, ChipDistribution,
};
pub use consensus::{ConsensusData, RecentReport};

use chrono::NaiveDate;

use crate::company_financials::FinancialPeriod;
use crate::company_metrics::{IndustryBenchmark, ValuationHistory};

/// 复权方式标注 — v11 P0-2 引入
///
/// 每条 K 线标注其价格口径,便于切源时下游比对。
/// - `Qfq`: 前复权（统一 Gateway 已验证的上游口径）
/// - `None`: 不复权（历史默认值；DB 反序列化路径也用此值，表示字段口径未知）
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AdjustType {
    Qfq,
    None,
}

impl AdjustType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdjustType::Qfq => "qfq",
            AdjustType::None => "none",
        }
    }
}

/// 标准化的K线数据
#[derive(Debug, Clone)]
pub struct KlineData {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
    pub pct_chg: f64,
    /// 修复 P1.8: 盘中实时价 (与 close 分离)
    /// 历史适配器曾用 quote.price 覆盖 latest.close，导致 Sharpe 使用盘中价
    ///       60 日滚动计算实际变成了盘中波动, 不是日线 settled close
    /// 现在: intraday_price 单独存盘中价, close 保持日线 settled close
    /// Sharpe 计算只用 close, 避免 look-ahead
    pub intraday_price: Option<f64>,
    /// 是否已收盘 (true: 收盘后 close 是最终价; false: 盘中 intraday 才是当前价)
    /// 用于 Sharpe 计算时区分历史 vs 盘中
    pub settled: bool,
    // 盈利水平相关字段
    pub pe_ratio: Option<f64>,        // 市盈率（动态）
    pub pb_ratio: Option<f64>,        // 市净率
    pub turnover_rate: Option<f64>,   // 换手率(%)
    pub market_cap: Option<f64>,      // 总市值（亿元）
    pub circulating_cap: Option<f64>, // 流通市值（亿元）
    // 新增财务指标
    pub eps: Option<f64>,            // 每股收益（元）
    pub roe: Option<f64>,            // 净资产收益率(%)
    pub revenue_yoy: Option<f64>,    // 营业收入同比增长率(%)
    pub net_profit_yoy: Option<f64>, // 净利润同比增长率(%)
    pub gross_margin: Option<f64>,   // 毛利率(%)
    pub net_margin: Option<f64>,     // 净利率(%)
    pub sharpe_ratio: Option<f64>,   // 夏普比率（风险调整后收益）
    /// 多期财务历史序列（按报告期从新到旧），仅填充到 data[0]（最新一根 K 线）
    pub financials_history: Option<Vec<FinancialPeriod>>,
    /// PE/PB 历史分位（近 3 年），仅填充到 data[0]
    pub valuation_history: Option<ValuationHistory>,
    /// 卖方分析师一致预期（近 6 个月研报），仅填充到 data[0]
    pub consensus: Option<ConsensusData>,
    /// 行业横向对标（同业 PE/PB/ROE 中位数 + 个股百分位），仅填充到 data[0]
    pub industry: Option<IndustryBenchmark>,
    // NEW: 涨跌停标记
    pub is_limit_up: bool,   // 是否涨停
    pub is_limit_down: bool, // 是否跌停
    pub is_suspended: bool,  // 是否停牌
    // v11 P0-2: 复权方式标注
    pub adjust: AdjustType, // 该 K 线价格是前复权 (Qfq) 还是不复权 (None)
}

#[cfg(test)]
pub(crate) fn loopback_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(1))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .expect("loopback test client must build")
}

#[cfg(test)]
pub(crate) struct TestHttpResponse {
    pub status: u16,
    pub body: String,
}

#[cfg(test)]
impl TestHttpResponse {
    pub fn json(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            body: body.into(),
        }
    }
}

#[cfg(test)]
pub(crate) struct TestHttpServer {
    base_url: String,
    expected: usize,
    served: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(test)]
impl TestHttpServer {
    pub fn new(responses: Vec<TestHttpResponse>) -> Self {
        use std::io::{Read, Write};
        use std::sync::atomic::Ordering;

        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("test loopback listener must bind");
        listener
            .set_nonblocking(true)
            .expect("test loopback listener must be nonblocking");
        let address = listener.local_addr().expect("test listener address");
        let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let served_thread = std::sync::Arc::clone(&served);
        let requests_thread = std::sync::Arc::clone(&requests);
        let expected = responses.len();
        let thread = std::thread::spawn(move || {
            for response in responses {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "test HTTP request did not arrive before timeout"
                            );
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        Err(error) => panic!("test HTTP accept failed: {error}"),
                    }
                };
                stream
                    .set_nonblocking(false)
                    .expect("test stream must use blocking reads");
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .expect("test stream read timeout");
                let mut raw = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let count = stream.read(&mut chunk).expect("read test HTTP request");
                    if count == 0 {
                        break;
                    }
                    raw.extend_from_slice(&chunk[..count]);
                    if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let header_end = raw
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|index| index + 4)
                    .expect("test HTTP request must contain a complete header");
                let request = String::from_utf8(raw[..header_end].to_vec())
                    .expect("test HTTP header must be UTF-8");
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .expect("test HTTP request line must contain path")
                    .to_string();
                requests_thread.lock().unwrap().push(path);

                let reason = match response.status {
                    200 => "OK",
                    400 => "Bad Request",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    503 => "Service Unavailable",
                    _ => "Test Status",
                };
                let head = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.status,
                    reason,
                    response.body.len()
                );
                stream
                    .write_all(head.as_bytes())
                    .and_then(|_| stream.write_all(response.body.as_bytes()))
                    .expect("write test HTTP response");
                stream.flush().expect("flush test HTTP response");
                stream
                    .shutdown(std::net::Shutdown::Write)
                    .expect("close test HTTP response body");
                served_thread.fetch_add(1, Ordering::SeqCst);
            }
        });
        Self {
            base_url: format!("http://{address}"),
            expected,
            served,
            requests,
            thread: Some(thread),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn finish(mut self) -> Vec<String> {
        use std::sync::atomic::Ordering;

        self.thread
            .take()
            .expect("test HTTP thread exists")
            .join()
            .expect("test HTTP responder must finish");
        assert_eq!(self.served.load(Ordering::SeqCst), self.expected);
        self.requests.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Drop for TestHttpServer {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v11 P0-2: AdjustType as_str 应返回稳定的小写字符串,用于 data_source 复合命名。
    #[test]
    fn adjust_type_as_str_stable() {
        assert_eq!(AdjustType::Qfq.as_str(), "qfq");
        assert_eq!(AdjustType::None.as_str(), "none");
    }

    /// v11 P0-2: AdjustType 必须是 Copy (赋值零成本),且支持 PartialEq 比较
    #[test]
    fn adjust_type_is_copy_and_eq() {
        let a = AdjustType::Qfq;
        let b = a; // Copy: a 仍然可用
        assert_eq!(a, b);
        assert_ne!(AdjustType::Qfq, AdjustType::None);
    }

    #[tokio::test]
    async fn loopback_http_server_serves_exact_response_sequence() {
        let server = TestHttpServer::new(vec![
            TestHttpResponse::json(r#"{"step":1}"#),
            TestHttpResponse {
                status: 503,
                body: r#"{"step":2}"#.to_string(),
            },
        ]);
        let client = loopback_http_client();
        let first = client
            .get(format!("{}/first", server.base_url()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(first, r#"{"step":1}"#);
        let second = client
            .get(format!("{}/second?query=1", server.base_url()))
            .send()
            .await
            .unwrap();
        assert_eq!(second.status().as_u16(), 503);
        assert_eq!(
            server.finish(),
            vec!["/first".to_string(), "/second?query=1".to_string()]
        );
    }
}
