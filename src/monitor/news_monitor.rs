//! 消息监控中枢（Phase 1.5 独立子系统）。
//!
//! 与价格扫描器平级，共用 SignalStateMachine + AlertRouter。
//! 运行窗口独立：消息通知时段可由 `config/monitor.toml` 配置，默认 08:00-22:00。
//!
//! 核心流程：采集 → 去重 → 实体关联 → 分类分级 → 衰减策略 → 告警

use crate::data_provider::announcement::{self, AnnLevel};
use crate::monitor::detector::{AlertCategory, AlertDetail, AlertEvent, AlertLevel};
use crate::monitor::entity_linker::EntityLinker;
use chrono::{Local, Timelike};
use diesel::prelude::*;
use log::info;
use std::collections::HashSet;

// ── 消息类型 ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsType { Flash, Policy, Announcement, Research }

impl NewsType {
    /// 衰减策略：快讯正常衰减，政策/公告开盘前不衰减
    pub fn decay_lambda(&self) -> f64 {
        match self { NewsType::Flash => 0.05, NewsType::Policy => 0.0, NewsType::Announcement => 0.0, NewsType::Research => 0.005 }
    }

    /// 开盘后衰减系数（政策/公告在 09:30 后切换到正常衰减）
    pub fn post_open_lambda(&self) -> f64 {
        match self { NewsType::Flash => 0.05, NewsType::Policy => 0.01, NewsType::Announcement => 0.01, NewsType::Research => 0.005 }
    }
}

// ── 消息事件 ──

#[derive(Debug, Clone)]
pub struct NewsEvent {
    pub title: String,
    pub source: String,
    pub news_type: NewsType,
    pub direction: i8,  // +1 利好, -1 利空, 0 中性
    pub importance: u8, // 1-5
    pub hits: Vec<crate::monitor::entity_linker::EntityHit>,
    pub received_at: chrono::DateTime<Local>,
}

// ── 消息监控器 ──

pub struct NewsMonitor {
    linker: EntityLinker,
    /// 已处理的事件标题（去重用）
    seen_titles: HashSet<String>,
    /// 被动源统计（金十/见闻/公告，用于舆情放量）
    passive_count_today: u64,
    /// 主动搜索统计（SerpAPI等，不计入舆情放量）
    active_count_today: u64,
    /// 告警计数器
    emergency_count: u32,
    important_count: u32,
    info_count: u32,
}

impl NewsMonitor {
    pub fn new() -> Self {
        let mut linker = EntityLinker::new();
        // 加载持仓
        if let Ok(db) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::database::DatabaseManager::get()
        })) {
            if let Ok(positions) = db.get_all_open_positions() {
                for p in &positions {
                    linker.register_position(&p.code, &p.name);
                }
                info!("[NewsMonitor] 注册 {} 只持仓股", positions.len());
            }
        }
        // 加载自选股（即使无持仓也监控）
        let mut watchlist_count = 0;
        if let Ok(codes) = crate::portfolio::get_all_codes() {
            for code in codes {
                linker.register_position(&code, &format!("股票{}", code));
                watchlist_count += 1;
            }
        }
        if watchlist_count > 0 {
            info!("[NewsMonitor] 注册 {} 只自选股", watchlist_count);
        }
        Self {
            linker, seen_titles: HashSet::new(), passive_count_today: 0,
            active_count_today: 0, emergency_count: 0, important_count: 0, info_count: 0,
        }
    }

    /// 新闻监控运行窗口由 `config/monitor.toml` 控制，默认 08:00 — 22:00。
    /// 覆盖盘前隔夜消息 + 盘中快讯 + 盘后公告高峰(21:00)。
    pub fn should_run() -> bool {
        Self::should_run_at(Local::now().hour())
    }

    /// 可测试的窗口判断：按小时判断当前是否处于消息通知窗口。
    pub fn should_run_at(hour: u32) -> bool {
        let cfg = crate::config::get_monitor_config();
        let start = u32::from(cfg.news_window_start_hour);
        let end = u32::from(cfg.news_window_end_hour);

        if start >= end {
            return (8..22).contains(&hour);
        }

        hour >= start && hour < end
    }

    /// 处理已拉取的公告列表（去重+关联+分级，纯CPU无阻塞）
    /// `resolved_codes`: 异步预解析的 name→code 映射（补API缺失的代码）
    pub fn process_announcements(
        &mut self,
        anns: &[announcement::Announcement],
        resolved_codes: &std::collections::HashMap<String, String>,
    ) -> Vec<AlertEvent> {
        let mut events = Vec::new();
        self.passive_count_today += anns.len() as u64;

        for ann in anns {
            // 去重
            let key = format!("ann:{}", &ann.title.chars().take(40).collect::<String>());
            if !self.seen_titles.insert(key) { continue; }

            // 实体关联（纯 CPU 计算，但输入短不需要 spawn_blocking）
            let hits = self.linker.link(&ann.title);

            // 分级（L1/L2 匹配后再决定级别，Skip 不再提前过滤）
            let (level, cat) = match ann.level {
                AnnLevel::Emergency => (AlertLevel::Emergency, AlertCategory::ChainRisk),
                AnnLevel::Important => (AlertLevel::Important, AlertCategory::ChainRisk),
                AnnLevel::Info => (AlertLevel::Info, AlertCategory::FlashNews),
                AnnLevel::Skip => (AlertLevel::Info, AlertCategory::FlashNews), // L2匹配的公告用Info级
            };

            match level {
                AlertLevel::Emergency => self.emergency_count += 1,
                AlertLevel::Important => self.important_count += 1,
                AlertLevel::Info => self.info_count += 1,
                _ => {}
            }

            // 名称/代码兜底：API 有时返回空，从标题解析
            let name = if ann.name.is_empty() {
                parse_company_from_title(&ann.title)
            } else {
                ann.name.clone()
            };
            let code = if ann.code.is_empty() {
                // 层级1: 反向索引查全名（已缓存过的 name→code）
                self.linker.lookup_code_by_name(&name).map(|s| s.to_string())
                    // 层级2: entity_linker 模糊匹配（简称/片段）
                    .or_else(|| self.linker.link(&name).first().map(|h| h.code.clone()))
                    // 层级3: 外部预解析的 name→code（异步反查结果）
                    .or_else(|| resolved_codes.get(&name).cloned())
                    .unwrap_or_default()
            } else {
                ann.code.clone()
            };

            // 自学习：完整 name+code 缓存到反向索引
            if !name.is_empty() && !code.is_empty() {
                self.linker.register_name_code(&name, &code);
            }

            // L1 实体过滤：代码/名称精确匹配持仓自选
            let l1_match = self.linker.is_registered(&code, &name)
                || hits.iter().any(|h| h.confidence > 0.8);

            // L2 概念匹配：公告标题命中板块名 → 成份股里有我们的标的
            let mut l2_concept = String::new();
            let mut l2_codes: Vec<String> = Vec::new();
            if !l1_match {
                for (concept, codes) in self.linker.concept_index() {
                    if ann.title.contains(concept.as_str()) {
                        l2_concept = concept.clone();
                        l2_codes = codes.clone();
                        break;
                    }
                }
            }

            if !l1_match && l2_concept.is_empty() {
                continue; // L1和L2都不匹配 → 丢弃
            }

            let hit_names: Vec<String> = {
                let mut names: Vec<String> = hits.iter()
                    .map(|h| format!("{}({})", h.name, h.code))
                    .collect();
                if !l2_concept.is_empty() {
                    names.push(format!("板块'{}'关联: {}", l2_concept, l2_codes.join(",")));
                }
                names
            };

            // 标题中剥离公司名前缀（避免和header的公司名重复）
            let short_title = strip_company_prefix(&ann.title, &name);

            events.push(AlertEvent {
                level,
                category: cat,
                code,
                name,
                message: format!("[公告] {} | {}", short_title, ann.reason),
                detail: AlertDetail {
                    price: None, change_pct: None, volume_ratio: None,
                    main_flow_yi: None, threshold: None,
                    news_title: Some(ann.title.clone()),
                    news_summary: {
                        // 回退链：正文 → API摘要 → 标题内容
                        if !ann.content.is_empty() {
                            Some(truncate_str(&ann.content, 150))
                        } else if !ann.summary.is_empty() {
                            Some(truncate_str(&ann.summary, 150))
                        } else {
                            // 最后回退：显示公告标题（剥离公司名前缀）
                            Some(truncate_str(&short_title, 100))
                        }
                    },
                    ai_decision: None,
                    t1_locked: false,
                    extra: if hit_names.is_empty() { None } else {
                        Some(format!("命中: {}", hit_names.join(", ")))
                    },
                },
                triggered_at: Local::now(),
            });
        }

        events
    }

    /// 对快讯文本做关联+分级（供外部轮询调用）。
    /// 支持重要性跳跃击穿：官方公告/标星快讯可突破30分钟去重缓存。
    pub fn process_flash(&mut self, title: &str, source: &str, importance: u8) -> Option<AlertEvent> {
        let key = format!("flash:{}", &title.chars().take(40).collect::<String>());
        // 击穿检测：来源升级或重要级跳跃
        let should_bypass = source.contains("公告") || importance >= 4;
        if !should_bypass && !self.seen_titles.insert(key.clone()) { return None; }
        if should_bypass { self.seen_titles.insert(key); }
        self.passive_count_today += 1;

        let hits = self.linker.link(title);

        // 分级：命中持仓 → 升级
        let has_position = hits.iter().any(|h| h.confidence > 0.8);
        let level = match (importance, has_position) {
            (4.., true) => AlertLevel::Emergency,
            (3.., true) => AlertLevel::Important,
            (4.., false) => AlertLevel::Important,
            (2.., false) => AlertLevel::Info,
            _ => return None,
        };

        let hit_names: Vec<String> = hits.iter().map(|h| format!("{}({})", h.name, h.code)).collect();

        Some(AlertEvent {
            level,
            category: AlertCategory::FlashNews,
            code: hits.first().map(|h| h.code.clone()).unwrap_or_default(),
            name: hits.first().map(|h| h.name.clone()).unwrap_or_default(),
            message: format!("[{}] {}", source, title),
            detail: AlertDetail {
                price: None, change_pct: None, volume_ratio: None,
                main_flow_yi: None, threshold: None,
                news_title: Some(title.to_string()),
                news_summary: None,
                ai_decision: None,
                t1_locked: false,
                extra: if hit_names.is_empty() { None } else { Some(format!("命中: {}", hit_names.join(", "))) },
            },
            triggered_at: Local::now(),
        })
    }

    /// 统计摘要
    pub fn stats(&self) -> String {
        format!(
            "被动{}条/主动{}条 | 🔴{} 🟠{} 🟡{}",
            self.passive_count_today, self.active_count_today,
            self.emergency_count, self.important_count, self.info_count
        )
    }

    /// 标记主动搜索（不计入舆情放量统计）
    pub fn mark_active_search(&mut self, count: u64) {
        self.active_count_today += count;
    }

    /// 暴露 linker 引用（供外部预查 name→code）
    pub fn linker_ref(&self) -> &EntityLinker {
        &self.linker
    }

    /// 暴露 linker 可变引用（供外部注入概念索引）
    pub fn linker_mut(&mut self) -> &mut EntityLinker {
        &mut self.linker
    }

    /// 将 seen_titles 批量写入 news_dedup 表，每 5 分钟调用一次
    pub fn flush_dedup(&self) {
        let db = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::database::DatabaseManager::get()
        })) {
            Ok(db) => db,
            Err(_) => return,
        };
        let mut conn = match db.get_conn() {
            Ok(c) => c,
            Err(_) => return,
        };
        for key in &self.seen_titles {
            let sql = format!("INSERT OR IGNORE INTO news_dedup(key) VALUES ('{}')", key.replace('\'', "''"));
            let _ = diesel::sql_query(&sql).execute(&mut *conn);
        }
    }

    /// 启动时从 news_dedup 恢复今天的 seen_titles，清理过期 key
    pub fn restore_dedup(&mut self) {
        let db = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::database::DatabaseManager::get()
        })) {
            Ok(db) => db,
            Err(_) => return,
        };
        let mut conn = match db.get_conn() {
            Ok(c) => c,
            Err(_) => return,
        };
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        // 恢复今天的 key
        #[derive(QueryableByName, Debug)]
        struct DedupKey { #[diesel(sql_type = diesel::sql_types::Text)] key: String }
        let sql = format!("SELECT key FROM news_dedup WHERE created_at >= '{}'", today);
        if let Ok(rows) = diesel::sql_query(&sql).load::<DedupKey>(&mut *conn) {
            for r in rows { self.seen_titles.insert(r.key); }
        }
        // 清理非今天的过期 key
        let _ = diesel::sql_query(&format!("DELETE FROM news_dedup WHERE created_at < '{}'", today))
            .execute(&mut *conn);
    }
}

/// L2 概念索引刷新（独立函数，在 spawn_blocking 中执行，避免 reqwest::blocking runtime 冲突）
/// 返回新的 concept_index 供主线程注入
pub fn refresh_concept_index_blocking(
    our_codes: &std::collections::HashSet<String>,
) -> Option<std::collections::HashMap<String, Vec<String>>> {
    use crate::market_analyzer::sector_monitor;
    let mut new_index: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();

    let boards = match sector_monitor::fetch_board_ranking("f3", 15) {
        Ok(b) => b,
        Err(e) => { log::warn!("[NewsMonitor] 概念板块拉取失败: {}", e); return None; }
    };
    if boards.is_empty() { return None; }
    log::info!("[NewsMonitor] L2 拉取 {} 个概念板块，构建反向索引...", boards.len());

    for board in &boards {
        let stocks = match sector_monitor::fetch_board_components(&board.code, 30) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let matched: Vec<String> = stocks.iter()
            .filter_map(|s| if our_codes.contains(&s.code) { Some(s.code.clone()) } else { None })
            .collect();
        if !matched.is_empty() {
            log::info!("[NewsMonitor] L2 板块'{}'命中 {} 只标的", board.name, matched.len());
            new_index.insert(board.name.clone(), matched);
        }
    }
    Some(new_index)
}

impl Default for NewsMonitor {
    fn default() -> Self { Self::new() }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max).collect::<String>()) }
}

/// 从公告标题中提取公司名（格式："公司名:公告内容" 或 "公司名关于..."）
fn parse_company_from_title(title: &str) -> String {
    // 取冒号前部分
    if let Some(pos) = title.find(['：', ':']) {
        let prefix = title[..pos].trim();
        if prefix.chars().count() >= 2 && prefix.chars().count() <= 8 {
            return prefix.to_string();
        }
    }
    // 取"关于"前部分（如"达实智能关于..."）
    if let Some(pos) = title.find("关于") {
        let prefix = title[..pos].trim();
        if prefix.chars().count() >= 2 && prefix.chars().count() <= 8 {
            return prefix.to_string();
        }
    }
    String::new()
}

/// 从标题中剥离公司名前缀（"达实智能:关于XXX" → "关于XXX"）
fn strip_company_prefix(title: &str, name: &str) -> String {
    if name.is_empty() { return title.to_string(); }
    // "公司名:内容" 或 "公司名：内容"
    for sep in [":", "："] {
        if let Some(rest) = title.strip_prefix(&format!("{}{}", name, sep)) {
            return rest.trim().to_string();
        }
    }
    // "公司名关于内容"
    if let Some(rest) = title.strip_prefix(name) {
        return rest.trim().to_string();
    }
    title.to_string()
}

/// 通过东方财富搜索 API 反查股票代码（公司名 → 代码），异步版本
pub async fn resolve_code_by_name(name: &str, client: &reqwest::Client) -> Option<String> {
    if name.is_empty() || name.chars().count() < 2 { return None; }
    let url = format!(
        "https://searchapi.eastmoney.com/api/suggest/get?input={}&type=14&token=D43BF722C8E33BDC906FB84D85E326E8&count=3",
        urlencoding::encode(name)
    );
    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(std::time::Duration::from_secs(3))
        .send().await.ok()?;
    let body: serde_json::Value = resp.json().await.ok()?;
    body["Result"]["QuotationCodeTable"]["Data"]
        .as_array()?
        .iter()
        .filter_map(|item| {
            let code = item["Code"].as_str()?;
            let market = item["MarketId"].as_str()?;
            if code.len() == 6 && code.chars().all(|c| c.is_ascii_digit())
                && !code.starts_with('8') && !code.starts_with('4') && !code.starts_with('9')
            {
                let _ = market; // 仅保留A股
                Some(code.to_string())
            } else { None }
        })
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_news_type_decay() {
        assert!((NewsType::Flash.decay_lambda() - 0.05).abs() < 0.01);
        assert!((NewsType::Policy.decay_lambda() - 0.0).abs() < 0.01);  // 开盘前不衰减
        assert!((NewsType::Announcement.decay_lambda() - 0.0).abs() < 0.01);
        assert!((NewsType::Policy.post_open_lambda() - 0.01).abs() < 0.01); // 开盘后恢复
    }

    #[test]
    fn test_should_run_detects_session() {
        let _ = NewsMonitor::should_run(); // 不应 panic
    }

    #[test]
    fn test_should_run_at_uses_configured_window() {
        assert!(!NewsMonitor::should_run_at(7));
        assert!(NewsMonitor::should_run_at(8));
        assert!(NewsMonitor::should_run_at(21));
        assert!(!NewsMonitor::should_run_at(22));
    }

    #[test]
    fn test_process_flash_with_hit() {
        let mut nm = NewsMonitor::new();
        nm.linker.register_position("000547", "航天发展");
        let event = nm.process_flash("航天发展获大额订单", "金十", 4);
        assert!(event.is_some());
        assert_eq!(event.unwrap().level, AlertLevel::Emergency); // 重要+命中持仓=紧急
    }

    #[test]
    fn test_process_flash_no_hit_low_importance() {
        let mut nm = NewsMonitor::new();
        let event = nm.process_flash("普通行业新闻", "金十", 1);
        assert!(event.is_none()); // 不重要且未命中 → 静默
    }

    #[test]
    fn test_dedup_blocks_duplicate() {
        let mut nm = NewsMonitor::new();
        nm.linker.register_position("000547", "航天发展");
        let first = nm.process_flash("航天发展中标大单", "金十", 4);
        let second = nm.process_flash("航天发展中标大单", "见闻", 3);
        assert!(first.is_some());
        assert!(second.is_none()); // 去重
    }

    #[test]
    fn test_active_search_not_counted() {
        let mut nm = NewsMonitor::new();
        nm.mark_active_search(5);
        assert_eq!(nm.active_count_today, 5);
        assert_eq!(nm.passive_count_today, 0);
    }

    #[test]
    fn test_parse_company_from_colon_title() {
        assert_eq!(parse_company_from_title("达实智能:关于控股股东及实际控制人股份解除质押的公告"), "达实智能");
    }

    #[test]
    fn test_parse_company_from_guanyu_title() {
        assert_eq!(parse_company_from_title("航天发展关于收到中标通知书的公告"), "航天发展");
    }

    #[test]
    fn test_parse_company_empty() {
        assert_eq!(parse_company_from_title("关于召开股东大会的通知"), "");
    }

    #[test]
    fn test_strip_company_colon_prefix() {
        assert_eq!(strip_company_prefix("达实智能:关于股东股份解除质押的公告", "达实智能"), "关于股东股份解除质押的公告");
    }

    #[test]
    fn test_strip_company_guanyu_prefix() {
        assert_eq!(strip_company_prefix("航天发展关于收到中标通知书的公告", "航天发展"), "关于收到中标通知书的公告");
    }

    #[test]
    fn test_strip_company_no_match() {
        assert_eq!(strip_company_prefix("关于召开股东大会的通知", "某公司"), "关于召开股东大会的通知");
    }

    #[test]
    fn test_process_ann_with_empty_code_name_parses_title() {
        let _ = crate::database::DatabaseManager::init(Some(std::path::PathBuf::from("./test_data/test_ai.db")));
        let mut nm = NewsMonitor::new();
        nm.linker.register_position("002421", "达实智能"); // L1过滤需要
        // 模拟API返回空code/name，但标题含公司名
        let ann = announcement::Announcement {
            code: String::new(),
            name: String::new(),
            title: "达实智能:关于股东股份解除质押的公告".into(),
            date: "2026-06-14".into(),
            summary: "编辑摘要内容".into(),
            content: String::new(),  // 正文为空，回退到summary
            level: announcement::AnnLevel::Important,
            reason: "标题含'质押'".into(),
        };
        let events = nm.process_announcements(&[ann], &std::collections::HashMap::new());
        assert_eq!(events.len(), 1);
        let e = &events[0];
        // name从标题解析
        assert_eq!(e.name, "达实智能");
        // news_summary回退到API摘要
        assert!(e.detail.news_summary.as_ref().unwrap().contains("编辑摘要"));
    }

    #[test]
    fn test_unrelated_stock_filtered_out() {
        let _ = crate::database::DatabaseManager::init(Some(std::path::PathBuf::from("./test_data/test_ai.db")));
        let mut nm = NewsMonitor::new();
        // 科森科技不在持仓/自选 → L1过滤
        let ann = announcement::Announcement {
            code: "603626".into(),
            name: "科森科技".into(),
            title: "科森科技:关于控股股东部分股份质押的公告".into(),
            date: "2026-06-15".into(),
            summary: String::new(),
            content: String::new(),
            level: announcement::AnnLevel::Important,
            reason: "标题含'质押'".into(),
        };
        let events = nm.process_announcements(&[ann], &std::collections::HashMap::new());
        assert!(events.is_empty()); // L1过滤器拦截
    }
}
