//! 搜索服务聚合器（原 search_service.rs 尾部）

use std::collections::HashMap;
use std::time::Duration;

use log::{info, warn};

use super::providers::{
    BochaSearchProvider, EastmoneyNewsProvider, Jin10CalendarEvent, Jin10Provider,
    SerpAPISearchProvider, TavilySearchProvider, WallStreetCnProvider,
};
use super::types::{SearchProvider, SearchResponse, SearchResult};

// ============================================================================
// SearchService 主服务
// ============================================================================

/// 搜索服务
///
/// 功能：
/// 1. 管理多个搜索引擎
/// 2. 自动故障转移
/// 3. 结果聚合和格式化
pub struct SearchService {
    providers: Vec<Box<dyn SearchProvider>>,
    /// 华尔街见闻直连（免费，用于宏观新闻）
    wscn: WallStreetCnProvider,
    /// 金十数据直连（免费，快讯 + 财经日历）
    jin10: Jin10Provider,
}

impl SearchService {
    /// 创建新的搜索服务
    pub fn new(
        bocha_keys: Option<Vec<String>>,
        tavily_keys: Option<Vec<String>>,
        serpapi_keys: Option<Vec<String>>,
        enable_eastmoney: bool,
    ) -> Self {
        let mut providers: Vec<Box<dyn SearchProvider>> = Vec::new();

        // 按优先级添加搜索引擎
        // 1. SerpAPI 最优先（Google搜索结果，质量高）
        if let Some(keys) = serpapi_keys {
            if !keys.is_empty() {
                info!("已配置 SerpAPI 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(SerpAPISearchProvider::new(keys)));
            }
        }

        // 2. 东方财富（免费，A股专业，无需API Key）
        if enable_eastmoney {
            info!("已启用 东方财富 新闻搜索（免费，无限制）");
            providers.push(Box::new(EastmoneyNewsProvider::new()));
        }

        // 3. Bocha（中文搜索优化，AI摘要）
        if let Some(keys) = bocha_keys {
            if !keys.is_empty() {
                info!("已配置 Bocha 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(BochaSearchProvider::new(keys)));
            }
        }

        // 4. Tavily（免费额度更多，每月 1000 次）
        if let Some(keys) = tavily_keys {
            if !keys.is_empty() {
                info!("已配置 Tavily 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(TavilySearchProvider::new(keys)));
            }
        }

        if providers.is_empty() {
            warn!("未配置任何搜索引擎，新闻搜索功能将不可用");
        }

        info!("已启用 华尔街见闻 直连（免费，全球财经快讯）");
        info!("已启用 金十数据 直连（免费，快讯 + 财经日历）");
        Self {
            providers,
            wscn: WallStreetCnProvider::new(),
            jin10: Jin10Provider::new(),
        }
    }

    /// 获取金十财经日历（未来 `days_ahead` 天，重要性 >= `min_star`）
    pub async fn fetch_financial_calendar(&self, days_ahead: u32, min_star: u8) -> Vec<Jin10CalendarEvent> {
        match self.jin10.fetch_calendar(days_ahead, min_star).await {
            Ok(v) => v,
            Err(e) => {
                warn!("[金十日历] 抓取失败: {}", e);
                Vec::new()
            }
        }
    }

    /// 检查是否有可用的搜索引擎
    pub fn is_available(&self) -> bool {
        self.providers.iter().any(|p| p.is_available())
    }

    /// 搜索股票相关新闻（多维度扩展关键词）
    pub async fn search_stock_news(
        &self,
        stock_code: &str,
        stock_name: &str,
        max_results: usize,
    ) -> SearchResponse {
        info!("搜索股票新闻: {}({})", stock_name, stock_code);

        // 提取股票简称（去掉常见后缀，如"科技"、"集团"、"股份"等保留核心词）
        let short_name = Self::extract_short_name(stock_name);

        // 构建多维度搜索查询
        let mut queries = vec![
            // 维度1: 基本新闻（全称 + 代码）
            format!("{} {} 股票 最新消息", stock_name, stock_code),
        ];

        // 维度2: 持股/投资/并购相关（用简称扩大搜索范围）
        let invest_name = if short_name != stock_name { &short_name } else { stock_name };
        queries.push(format!("{} 持股 投资 收购 参股", invest_name));

        // 维度3: 行业/合作/订单（简称搜索）
        queries.push(format!("{} 合作 中标 订单 签约", invest_name));

        let mut all_results: Vec<SearchResult> = Vec::new();
        let mut success_provider = String::new();
        let mut total_search_time = 0.0;

        for (dim_idx, query) in queries.iter().enumerate() {
            // 每个维度取少量结果，合并后再截断
            let per_query_max = if dim_idx == 0 { max_results } else { 3_usize.min(max_results) };

            for provider in &self.providers {
                if !provider.is_available() {
                    continue;
                }

                let response = provider.search(query, per_query_max).await;
                total_search_time += response.search_time;

                if response.success && !response.results.is_empty() {
                    if success_provider.is_empty() {
                        success_provider = response.provider.clone();
                    } else if !success_provider.contains(&response.provider) {
                        success_provider = format!("{}+{}", success_provider, response.provider);
                    }
                    info!("[维度{}] 使用 {} 搜索 '{}' 获得 {} 条结果",
                        dim_idx + 1, response.provider, query, response.results.len());
                    all_results.extend(response.results);
                    break; // 该维度搜索成功，不再尝试其他引擎
                } else {
                    warn!(
                        "[维度{}] {} 搜索失败: {}，尝试下一个引擎",
                        dim_idx + 1,
                        provider.name(),
                        response.error_message.as_deref().unwrap_or("未知错误")
                    );
                }
            }

            // 维度之间短暂延迟，避免请求过快
            if dim_idx < queries.len() - 1 {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }

        if all_results.is_empty() {
            return SearchResponse::error(
                queries[0].clone(),
                "None".to_string(),
                "所有搜索引擎都不可用或搜索失败".to_string(),
            );
        }

        // 去重（按URL去重）
        let mut seen_urls = std::collections::HashSet::new();
        all_results.retain(|r| seen_urls.insert(r.url.clone()));

        // 为每个结果提取关键词并计算相关性
        for result in &mut all_results {
            result.extract_keywords(stock_name, stock_code);

            let title_lower = result.title.to_lowercase();
            let stock_name_lower = stock_name.to_lowercase();
            let short_name_lower = short_name.to_lowercase();
            // 全称匹配加分最多
            if title_lower.contains(&stock_name_lower) || title_lower.contains(stock_code) {
                result.relevance = (result.relevance + 0.3).min(1.0);
            }
            // 简称匹配也加分
            if short_name_lower != stock_name_lower && title_lower.contains(&short_name_lower) {
                result.relevance = (result.relevance + 0.2).min(1.0);
            }
            // 包含持股/投资/并购等高价值关键词加重要性
            let high_value_keywords = ["持股", "投资", "收购", "参股", "并购", "入股", "中标", "签约", "订单"];
            for kw in &high_value_keywords {
                if title_lower.contains(kw) || result.snippet.contains(kw) {
                    result.importance = result.importance.saturating_add(1).min(10);
                    break;
                }
            }
        }

        // 按重要性和相关性排序
        all_results.sort_by(|a, b| {
            let score_a = (a.importance as f32) * a.relevance;
            let score_b = (b.importance as f32) * b.relevance;
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        // 截断到max_results + 2（多保留一点给AI更多上下文）
        all_results.truncate(max_results + 2);

        info!("多维度搜索完成: {} 条结果（去重后），来源: {}", all_results.len(), success_provider);

        SearchResponse {
            query: format!("{} {} 多维度搜索", stock_name, stock_code),
            results: all_results,
            success: true,
            provider: success_provider,
            error_message: None,
            search_time: total_search_time,
        }
    }

    /// 从股票全称中提取简称/核心词
    /// 例如: "金风科技" -> "金风", "越秀资本" -> "越秀", "贵州茅台" -> "茅台"
    fn extract_short_name(stock_name: &str) -> String {
        // 常见后缀词（按长度从长到短排列，优先匹配长的）
        let suffixes = [
            "电子科技", "高新技术", "信息技术",
            "新材料", "新能源", "生物科技",
            "科技", "集团", "股份", "控股", "实业", "产业",
            "资本", "投资", "金融", "银行", "证券", "保险",
            "医药", "制药", "生物",
            "电气", "电子", "电力", "能源", "环保",
            "汽车", "机械", "材料", "化工", "建设",
            "通信", "传媒", "文化", "教育", "旅游",
            "食品", "乳业", "酿酒", "地产", "置业",
            "物流", "航空", "航天",
        ];

        // 常见地名前缀
        let prefixes = [
            "贵州", "云南", "四川", "山东", "江苏", "浙江", "广东", "福建",
            "河南", "河北", "湖南", "湖北", "安徽", "江西", "陕西", "山西",
            "辽宁", "吉林", "黑龙", "甘肃", "青海", "海南", "广西", "内蒙",
            "新疆", "西藏", "宁夏", "上海", "北京", "天津", "重庆", "深圳",
        ];

        let mut name = stock_name.to_string();

        // 先去后缀
        for suffix in &suffixes {
            if name.ends_with(suffix) && name.len() > suffix.len() {
                name = name[..name.len() - suffix.len()].to_string();
                break;
            }
        }

        // 再去地名前缀
        for prefix in &prefixes {
            if name.starts_with(prefix) && name.chars().count() > prefix.chars().count() {
                let prefix_len = prefix.len();
                name = name[prefix_len..].to_string();
                break;
            }
        }

        // 如果处理后太短（<2个字），返回原名
        if name.chars().count() < 2 {
            return stock_name.to_string();
        }

        name
    }

    /// 搜索股票特定事件
    pub async fn search_stock_events(
        &self,
        stock_code: &str,
        stock_name: &str,
        event_types: Option<Vec<&str>>,
    ) -> SearchResponse {
        let events = event_types.unwrap_or_else(|| vec!["年报预告", "减持公告", "业绩快报"]);
        let event_query = events.join(" OR ");
        let query = format!("{} ({})", stock_name, event_query);

        info!("搜索股票事件: {}({}) - {:?}", stock_name, stock_code, events);

        for provider in &self.providers {
            if !provider.is_available() {
                continue;
            }

            let response = provider.search(&query, 5).await;

            if response.success {
                return response;
            }
        }

        SearchResponse::error(query, "None".to_string(), "事件搜索失败".to_string())
    }

    /// 多维度情报搜索
    pub async fn search_comprehensive_intel(
        &self,
        stock_code: &str,
        stock_name: &str,
        max_searches: usize,
    ) -> HashMap<String, SearchResponse> {
        let mut results = HashMap::new();
        let mut search_count = 0;

        // 定义搜索维度
        let search_dimensions = vec![
            (
                "latest_news",
                format!("{} {} 最新 新闻 2026年1月", stock_name, stock_code),
                "最新消息",
            ),
            (
                "risk_check",
                format!("{} 减持 处罚 利空 风险", stock_name),
                "风险排查",
            ),
            (
                "earnings",
                format!("{} 年报预告 业绩预告 业绩快报 2025年报", stock_name),
                "业绩预期",
            ),
        ];

        info!("开始多维度情报搜索: {}({})", stock_name, stock_code);

        let mut provider_index = 0;
        let available_providers: Vec<_> = self
            .providers
            .iter()
            .filter(|p| p.is_available())
            .collect();

        if available_providers.is_empty() {
            return results;
        }

        for (dim_name, query, desc) in search_dimensions {
            if search_count >= max_searches {
                break;
            }

            let provider = available_providers[provider_index % available_providers.len()];
            provider_index += 1;

            info!("[情报搜索] {}: 使用 {}", desc, provider.name());

            let response = provider.search(&query, 3).await;

            if response.success {
                info!("[情报搜索] {}: 获取 {} 条结果", desc, response.results.len());
            } else {
                warn!(
                    "[情报搜索] {}: 搜索失败 - {}",
                    desc,
                    response.error_message.as_deref().unwrap_or("未知错误")
                );
            }

            results.insert(dim_name.to_string(), response);
            search_count += 1;

            // 短暂延迟避免请求过快
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        results
    }

    /// 格式化情报搜索结果为报告
    pub fn format_intel_report(
        intel_results: &HashMap<String, SearchResponse>,
        stock_name: &str,
    ) -> String {
        let mut lines = vec![format!("【{} 情报搜索结果】", stock_name)];

        // 最新消息
        if let Some(resp) = intel_results.get("latest_news") {
            lines.push(format!("\n📰 最新消息 (来源: {}):", resp.provider));
            if resp.success && !resp.results.is_empty() {
                for (i, r) in resp.results.iter().take(3).enumerate() {
                    let date_str = r
                        .published_date
                        .as_ref()
                        .map(|d| format!(" [{}]", d))
                        .unwrap_or_default();
                    lines.push(format!("  {}. {}{}", i + 1, r.title, date_str));
                    lines.push(format!(
                        "     {}...",
                        r.snippet.chars().take(100).collect::<String>()
                    ));
                }
            } else {
                lines.push("  未找到相关消息".to_string());
            }
        }

        // 风险排查
        if let Some(resp) = intel_results.get("risk_check") {
            lines.push(format!("\n⚠️ 风险排查 (来源: {}):", resp.provider));
            if resp.success && !resp.results.is_empty() {
                for (i, r) in resp.results.iter().take(3).enumerate() {
                    lines.push(format!("  {}. {}", i + 1, r.title));
                    lines.push(format!(
                        "     {}...",
                        r.snippet.chars().take(100).collect::<String>()
                    ));
                }
            } else {
                lines.push("  未发现明显风险信号".to_string());
            }
        }

        // 业绩预期
        if let Some(resp) = intel_results.get("earnings") {
            lines.push(format!("\n📊 业绩预期 (来源: {}):", resp.provider));
            if resp.success && !resp.results.is_empty() {
                for (i, r) in resp.results.iter().take(3).enumerate() {
                    lines.push(format!("  {}. {}", i + 1, r.title));
                    lines.push(format!(
                        "     {}...",
                        r.snippet.chars().take(100).collect::<String>()
                    ));
                }
            } else {
                lines.push("  未找到业绩相关信息".to_string());
            }
        }

        lines.join("\n")
    }

    /// 搜索当日宏观/国际/市场最新新闻（所有股票共享，只搜索一次）
    ///
    /// 搜索维度：
    /// 1. 今日 A 股市场 + 大盘动态
    /// 2. 国际财经 + 地缘政治最新要闻
    /// 3. 美股 / 欧股 / 大宗商品今日行情
    /// 4. 国内宏观政策（央行、财政、产业）
    pub async fn search_macro_news(&self, max_results: usize) -> String {
        let today = chrono::Local::now().format("%Y年%m月%d日").to_string();

        let mut sections: Vec<String> = Vec::new();

        // ── 第一步：免费直连源（华尔街见闻 + 金十数据）并发拉取 ──
        let (live_res, article_res, jin10_flash_res, jin10_imp_res, calendar_res) = tokio::join!(
            self.wscn.fetch_live_news(30),
            self.wscn.fetch_articles(10),
            self.jin10.fetch_flash_news(20, false),
            self.jin10.fetch_flash_news(10, true),
            self.jin10.fetch_calendar(1, 2), // 今天+明天，重要性≥2
        );
        let mut wscn_items: Vec<String> = Vec::new();
        if let Ok(lives) = live_res {
            for r in lives.iter().take(5) {
                let t = r.published_date.as_deref().unwrap_or("");
                let snippet: String = r.snippet.chars().take(120).collect();
                wscn_items.push(format!("- **{}** {}  \n  {}", r.title, t, snippet));
            }
        }
        if let Ok(articles) = article_res {
            for r in articles.iter().take(3) {
                let t = r.published_date.as_deref().unwrap_or("");
                let snippet: String = r.snippet.chars().take(120).collect();
                wscn_items.push(format!("- **{}** {}  \n  {}", r.title, t, snippet));
            }
        }
        if !wscn_items.is_empty() {
            sections.push(format!("### 🌐 华尔街见闻快讯（今日实时）\n{}", wscn_items.join("\n")));
            info!("[宏观新闻][wscn] 华尔街见闻获取 {} 条", wscn_items.len());
        } else {
            warn!("[宏观新闻][wscn] 华尔街见闻返回为空或超时");
        }

        // ── 金十快讯（标星重要优先，普通快讯补充）──
        let mut jin10_imp_items: Vec<String> = Vec::new();
        if let Ok(lst) = jin10_imp_res {
            for r in lst.iter().take(5) {
                let t = r.published_date.as_deref().unwrap_or("");
                let snippet: String = r.snippet.chars().take(160).collect();
                jin10_imp_items.push(format!("- **{}** {}  \n  {}", r.title, t, snippet));
            }
        }
        if !jin10_imp_items.is_empty() {
            sections.push(format!("### ⭐ 金十重磅快讯（近6小时标星）\n{}", jin10_imp_items.join("\n")));
            info!("[宏观新闻][jin10] 金十标星 {} 条", jin10_imp_items.len());
        }

        let mut jin10_flash_items: Vec<String> = Vec::new();
        if let Ok(lst) = jin10_flash_res {
            // 去重：不再包含已在 important 列表里的
            let imp_titles: std::collections::HashSet<String> = jin10_imp_items.iter()
                .map(|s| s.chars().take(40).collect())
                .collect();
            for r in lst.iter() {
                let key: String = format!("- **{}**", r.title).chars().take(40).collect();
                if imp_titles.contains(&key) { continue; }
                let t = r.published_date.as_deref().unwrap_or("");
                let snippet: String = r.snippet.chars().take(140).collect();
                jin10_flash_items.push(format!("- **{}** {}  \n  {}", r.title, t, snippet));
                if jin10_flash_items.len() >= 6 { break; }
            }
        }
        if !jin10_flash_items.is_empty() {
            sections.push(format!("### 📣 金十快讯（今日实时）\n{}", jin10_flash_items.join("\n")));
            info!("[宏观新闻][jin10] 金十快讯补充 {} 条", jin10_flash_items.len());
        } else if jin10_imp_items.is_empty() {
            warn!("[宏观新闻][jin10] 金十快讯为空或超时");
        }

        // ── 金十财经日历（今天 + 明天的重要经济数据 / 事件）──
        match calendar_res {
            Ok(events) if !events.is_empty() => {
                let mut cal_lines: Vec<String> = Vec::new();
                for ev in events.iter().take(15) {
                    let stars = "★".repeat(ev.star.min(3) as usize);
                    let mut extra: Vec<String> = Vec::new();
                    if let Some(p) = &ev.previous { extra.push(format!("前值 {}", p)); }
                    if let Some(f) = &ev.forecast { extra.push(format!("预期 {}", f)); }
                    if let Some(a) = &ev.actual { extra.push(format!("公布 {}", a)); }
                    let tail = if extra.is_empty() { String::new() } else { format!("  \n  {}", extra.join(" | ")) };
                    cal_lines.push(format!(
                        "- `{} {}` {} **[{}]** {}{}",
                        ev.date, ev.time, stars, ev.country, ev.name, tail
                    ));
                }
                sections.push(format!("### 📅 财经日历（金十，未来48h重要事件）\n{}", cal_lines.join("\n")));
                info!("[宏观新闻][jin10] 财经日历 {} 条", events.len());
            }
            Ok(_) => {
                info!("[宏观新闻][jin10] 财经日历窗口内无重要事件");
            }
            Err(e) => {
                warn!("[宏观新闻][jin10] 财经日历抓取失败: {}", e);
            }
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        // ── 第二步：搜索引擎多维度查询 ──
        // (维度key, 查询关键词, 展示标题)
        let search_dims: Vec<(&str, String, &str)> = vec![
            ("a_market",    format!("{}A股 大盘 股市 最新动态", today),              "### 🇨🇳 A股市场动态"),
            ("global",      format!("{}国际财经 地缘政治 最新消息", today),           "### 🌍 国际财经 / 地缘政治"),
            ("us_market",   format!("{}美股 美联储 大宗商品 今日", today),            "### 🇺🇸 美股 / 大宗商品"),
            ("cn_policy",   format!("{}中国 央行 财政 产业政策 重要新闻", today),     "### 📋 宏观政策"),
            ("institution", format!("{}高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报", today),
                                                                                    "### 🏦 投行观点（高盛/摩根/美银）"),
            ("fin_media",   format!("{}证券时报 第一财经 21世纪经济报道 重要财经", today),
                                                                                    "### 📰 财经媒体要闻"),
        ];

        for (dim, query, header) in &search_dims {
            let mut found = false;
            for provider in &self.providers {
                if !provider.is_available() { continue; }
                let resp = provider.search(query, max_results.min(3)).await;
                if resp.success && !resp.results.is_empty() {
                    let lines: Vec<String> = resp.results.iter().take(3).map(|r| {
                        let date_tag = r.published_date.as_deref().unwrap_or("");
                        let snippet_short: String = r.snippet.chars().take(150).collect();
                        format!("- **{}** {}  \n  {}", r.title, date_tag, snippet_short)
                    }).collect();
                    sections.push(format!("{}\n{}", header, lines.join("\n")));
                    info!("[宏观新闻][{}] {} 获取 {} 条", dim, resp.provider, resp.results.len());
                    found = true;
                    break;
                }
            }
            if !found {
                warn!("[宏观新闻][{}] 所有引擎均失败，跳过该维度", dim);
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        if sections.is_empty() {
            return String::new();
        }

        format!("## 📡 今日宏观 / 市场背景（{}）\n\n{}", today, sections.join("\n\n"))
    }

    /// 批量搜索多只股票新闻
    pub async fn batch_search(
        &self,
        stocks: Vec<(&str, &str)>, // (code, name)
        max_results_per_stock: usize,
        delay_between: Duration,
    ) -> HashMap<String, SearchResponse> {
        let mut results = HashMap::new();

        for (i, (code, name)) in stocks.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(delay_between).await;
            }

            let response = self.search_stock_news(code, name, max_results_per_stock).await;
            results.insert(code.to_string(), response);
        }

        results
    }
}

// ============================================================================
// 工具函数
// ============================================================================

// extract_domain 已迁移至 super::types::extract_domain

// ============================================================================
// 单例服务
// ============================================================================

use once_cell::sync::OnceCell;

static SEARCH_SERVICE: OnceCell<SearchService> = OnceCell::new();

/// 获取搜索服务单例
pub fn get_search_service() -> &'static SearchService {
    SEARCH_SERVICE.get_or_init(|| {
        // 从环境变量读取配置
        let bocha_keys = std::env::var("BOCHA_API_KEYS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.split(',').map(|k| k.trim().to_string()).collect());

        let tavily_keys = std::env::var("TAVILY_API_KEYS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.split(',').map(|k| k.trim().to_string()).collect());

        let serpapi_keys = std::env::var("SERPAPI_KEYS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.split(',').map(|k| k.trim().to_string()).collect());

        // 东方财富默认启用（免费无限制）
        let enable_eastmoney = std::env::var("ENABLE_EASTMONEY_NEWS")
            .unwrap_or_else(|_| "true".to_string())
            .to_lowercase() != "false";

        SearchService::new(bocha_keys, tavily_keys, serpapi_keys, enable_eastmoney)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_search_service() {
        env_logger::init();

        let service = get_search_service();

        if service.is_available() {
            println!("=== 测试股票新闻搜索 ===");
            let response = service.search_stock_news("300389", "艾比森", 5).await;
            println!("搜索状态: {}", if response.success { "成功" } else { "失败" });
            println!("搜索引擎: {}", response.provider);
            println!("结果数量: {}", response.results.len());
            println!("耗时: {:.2}s", response.search_time);
            println!("\n{}", response.to_context(5));
        } else {
            println!("未配置搜索引擎 API Key，跳过测试");
        }
    }
}
