//! 宏观新闻上下文获取：优先复用传入的预取文本，否则在线搜索。

use std::sync::Arc;

use log::{info, warn};

use crate::search_service::get_search_service;

/// 返回一个共享的宏观新闻上下文 (`Arc<String>`)。
///
/// - `prefetched`: 调用方已经获取到的宏观新闻（如有）。非空则直接复用。
/// - `use_news_search`: 搜索服务可用时才会在线搜索。
pub(super) async fn resolve_macro_context(
    prefetched: Option<String>,
    use_news_search: bool,
) -> Arc<String> {
    if let Some(mc) = prefetched {
        if !mc.is_empty() {
            info!("✓ 复用已获取的宏观新闻（{} 字符），跳过重复搜索", mc.len());
            return Arc::new(mc);
        }
        return Arc::new(String::new());
    }

    if !use_news_search {
        return Arc::new(String::new());
    }

    info!("📡 搜索今日宏观/市场最新新闻（所有股票共享）...");
    let search_service = get_search_service();
    let mc = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        search_service.search_macro_news(3),
    )
    .await
    {
        Ok(text) if !text.is_empty() => {
            info!("✓ 宏观新闻获取成功，共 {} 字符", text.len());
            text
        }
        Ok(_) => {
            warn!("宏观新闻搜索返回为空");
            String::new()
        }
        Err(_) => {
            warn!("宏观新闻搜索超时(15s)");
            String::new()
        }
    };
    Arc::new(mc)
}
