//! 修复 Top10#7 (2026-06-29 audit): 共享 HTTP client, 避免每次调用新建.
//!
//! 背景: 全代码库有 28 处 `reqwest::Client::builder()` 调用 (audit §2.1), 每个 HTTP
//! 调用新建 client = TCP 1 RTT + TLS 2 RTT + DNS + Arc 分配. 200 只股票批量查询
//! = 额外 ~1000 次握手.
//!
//! 修法: 在 src/lib.rs 通过 `Lazy<Client>` 共享两个仍有调用方的预配置 client:
//!   - `SHARED_HTTP_CLIENT`: 默认 30s timeout（非行情 HTTP 业务）
//!   - `SHARED_FAST_HTTP_CLIENT`: 5s timeout (e.g. flash news)
//!
//! 调用方: `use crate::http_client::SHARED_HTTP_CLIENT;` 然后 `SHARED_HTTP_CLIENT.get(...)`
//! 不需要 `client.clone()` — reqwest::Client 内部 Arc, 共享安全.

use std::time::Duration;

use once_cell::sync::Lazy;

/// 默认 30s timeout，供尚未迁移的非行情 HTTP 业务使用
pub static SHARED_HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .expect("SHARED_HTTP_CLIENT: 创建 reqwest Client 失败")
});

/// 5s timeout, 快讯/news 短调用
pub static SHARED_FAST_HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(3))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .expect("SHARED_FAST_HTTP_CLIENT: 创建 reqwest Client 失败")
});
