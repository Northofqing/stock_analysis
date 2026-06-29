//! BR-003: 宏观新闻入 macro 通道, 不入 chain_mapper 流
//!
//! 修复 R-6: fetch_flash_titles() 必须在去重后、return 前过滤宏观关键词。
//! 测试策略: 真实 fetch 不会 panic; 任何返回的标题都不应含 MACRO_KEYWORDS 关键词。
//!   端到端 mock provider 复杂, 这里只断言 BR-003 的不变式: 返列表不出现宏观关键词。

use stock_analysis::search_service::SearchService;

#[tokio::test]
async fn test_fetch_flash_filters_macro_news() {
    let svc = SearchService::new(None, None, None, true);
    let titles = svc.fetch_flash_titles(20).await;

    // 断言: 任何返回的标题都不含 BR-003 列出的宏观关键词
    for title in &titles {
        assert!(
            !title.contains("美联储"),
            "BR-003: 宏观新闻应被过滤, 实际: {}",
            title
        );
        assert!(
            !title.contains("美股"),
            "BR-003: 宏观新闻应被过滤, 实际: {}",
            title
        );
        assert!(
            !title.contains("美元指数"),
            "BR-003: 宏观新闻应被过滤, 实际: {}",
            title
        );
    }
}
