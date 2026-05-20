//! 多周期下钻：当日线产生买入信号时，去 60min/15min K 线寻找精准入场点。
//!
//! 触发条件由调用方判定（评分≥60 / BB+MACD BottomBuy/UptrendStart / RSI Buy 任一）。
//! 本模块只负责在 blocking 线程池抓取 60min+15min K 线并跑入场点评估，返回可注入
//! AI prompt 的 Markdown 片段。失败或数据不足时返回 `None`。

pub(super) async fn fetch_multi_timeframe_section(code: &str) -> Option<String> {
    let code_owned = code.to_string();
    tokio::task::spawn_blocking(move || {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .build()
            .ok()?;
        // 60min 拉 120 根（约 30 个交易日，足够算 MA20/MACD），15min 拉 80 根
        let h1 = crate::data_provider::intraday_kline::fetch_blocking(&client, &code_owned, 60, 120);
        let m15 = crate::data_provider::intraday_kline::fetch_blocking(&client, &code_owned, 15, 80);
        if h1.is_empty() || m15.is_empty() {
            return None;
        }
        let assess = crate::strategy::assess_multi_timeframe_entry(&h1, &m15);
        let section = assess.to_prompt_section();
        if section.trim().is_empty() {
            None
        } else {
            Some(section)
        }
    })
    .await
    .ok()
    .flatten()
}
