//! BR-003: 宏观新闻入 macro 通道, 不入 chain_mapper 流
//!
//! 修复 R-6: fetch_flash_titles() 必须在去重后、return 前过滤宏观关键词。
//! 修复 M3: 测试用 filter_macro_titles 纯函数 + 注入已知输入, 不依赖真实网络
//! (原版依赖 SearchService::fetch_flash_titles 是退化测试 — CI 无网时 trivially pass).

use stock_analysis::search_service::service::filter_macro_titles;

#[test]
fn test_filter_macro_titles_removes_macro_news() {
    let input = vec![
        // 4 条宏观新闻 (应被过滤)
        "美联储宣布降息 25 个基点".to_string(),
        "美股三大指数集体收涨".to_string(),
        "美元指数创近期新高".to_string(),
        "国际原油价格大幅波动".to_string(),
        // 3 条非宏观 (应保留)
        "国内半导体板块大涨".to_string(),
        "新一批 AI 算力订单落地".to_string(),
        "机器人概念股表现活跃".to_string(),
    ];

    let (filtered, count) = filter_macro_titles(input);

    // 4 条宏观新闻被过滤
    assert_eq!(count, 4, "应过滤 4 条宏观新闻");
    assert_eq!(filtered.len(), 3, "应保留 3 条非宏观新闻");

    // 断言: 过滤后列表不含任何宏观关键词
    for title in &filtered {
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
        assert!(
            !title.contains("原油"),
            "BR-003: 宏观新闻应被过滤, 实际: {}",
            title
        );
    }
}

#[test]
fn test_filter_macro_titles_handles_empty_input() {
    let (filtered, count) = filter_macro_titles(vec![]);
    assert_eq!(filtered.len(), 0);
    assert_eq!(count, 0);
}

#[test]
fn test_filter_macro_titles_handles_all_macro() {
    let input = vec![
        "美联储降息".to_string(),
        "美股纳指新高".to_string(),
        "黄金价格上涨".to_string(),
    ];
    let (filtered, count) = filter_macro_titles(input);
    assert_eq!(filtered.len(), 0, "全部宏观应被过滤");
    assert_eq!(count, 3);
}