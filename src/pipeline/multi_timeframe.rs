//! 多周期下钻：当日线产生买入信号时，去 60min/15min K 线寻找精准入场点。
//!
//! 触发条件由调用方判定（评分≥60 / BB+MACD BottomBuy/UptrendStart / RSI Buy 任一）。
//! 本模块只负责在 blocking 线程池抓取 60min+15min K 线并跑入场点评估，返回可注入
//! AI prompt 的 Markdown 片段。数据不足返回 `Ok(None)`，来源失败显式返回 `Err`。

pub(super) async fn fetch_multi_timeframe_section(code: &str) -> Result<Option<String>, String> {
    use once_cell::sync::Lazy;
    // 复用单个 HTTP client，避免每次调用都重建连接池
    static MTF_CLIENT: Lazy<Result<reqwest::Client, String>> = Lazy::new(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .build()
            .map_err(|error| format!("创建多周期 HTTP client 失败: {error}"))
    });

    let client = MTF_CLIENT.as_ref().map_err(Clone::clone)?;
    let (h1_result, m15_result) = tokio::join!(
        crate::data_provider::intraday_kline::fetch_async(client, code, 60, 120),
        crate::data_provider::intraday_kline::fetch_async(client, code, 15, 80)
    );
    let h1 = h1_result.map_err(|error| format!("[{code}] 60min K线不可用: {error}"))?;
    let m15 = m15_result.map_err(|error| format!("[{code}] 15min K线不可用: {error}"))?;
    let assess = crate::strategy::assess_multi_timeframe_entry(&h1, &m15);
    let section = assess.to_prompt_section();
    if section.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(section))
    }
}
