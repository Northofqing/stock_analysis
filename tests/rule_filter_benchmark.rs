//! 修复 v9.1 spec §8: 规则预筛精确率 ≥ 85%
//!
//! 100 条手工标注中文新闻标题 + 期望 event_type, 验证 rule_filter 的精确率.
//! spec §8 验收: "规则预筛精确率: 100 条标注数据, 判对 event_type ≥ 85%"
//!
//! 标注来源: 模拟真实 A 股财经新闻标题 (财联社/东方财富/华尔街见闻/巨潮等).
//! 不依赖任何外部数据源, 纯文本 + 期望分类.
//!
//! 用法: cargo test --test rule_filter_benchmark -- --nocapture
//! 报告会打印到 stdout, 失败会显示哪一类规则需要优化.

use stock_analysis::opportunity::event_extractor::adapter::{RawNewsItem, SourceType};
use stock_analysis::opportunity::event_extractor::rule_filter::{RuleFilter, RuleMatch};
use stock_analysis::signal::market_event::EventType;

/// 标注数据: (title, expected_event_type)
/// expected_event_type = None 表示期望被丢弃 (噪声)
#[allow(dead_code)] // rationale 仅供人工调试, 不参与断言
struct LabeledItem {
    title: &'static str,
    expected: Option<EventType>,
    /// 分类理由 (人类可读, 便于调试)
    rationale: &'static str,
}

fn labeled_dataset() -> Vec<LabeledItem> {
    vec![
        // ════════════════════════════════════════════════════════
        // 类别 1: 收评/复盘/盘面/午评 (期望丢弃)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "A股收评：三大指数震荡上行，成交额破万亿", expected: None, rationale: "收评" },
        LabeledItem { title: "沪指收盘涨0.5% 创业板指跌0.3% 今日盘面梳理", expected: None, rationale: "收盘+盘面" },
        LabeledItem { title: "午评：沪指半日跌0.2% 成交量萎缩", expected: None, rationale: "午评" },
        LabeledItem { title: "早评：科技股领涨 市场情绪回暖", expected: None, rationale: "早评" },
        LabeledItem { title: "今日复盘：周期股集体调整 消费板块逆势走强", expected: None, rationale: "复盘" },
        LabeledItem { title: "本周盘面总结：上证指数累计上涨1.2%", expected: None, rationale: "盘面" },
        LabeledItem { title: "市场收评：北向资金尾盘加速流入", expected: None, rationale: "收评" },
        LabeledItem { title: "午盘解析：新能源板块集体回调", expected: None, rationale: "午盘 (午评变体)" },
        LabeledItem { title: "周复盘：科创50涨幅居前 创业板表现疲软", expected: None, rationale: "复盘" },
        LabeledItem { title: "今日早盘观察：三大指数小幅低开", expected: None, rationale: "早盘 (早评变体)" },
        LabeledItem { title: "A股午后复盘：金融板块发力 沪指翻红", expected: None, rationale: "复盘" },
        LabeledItem { title: "5月收评：上证指数月线三连阳", expected: None, rationale: "收评" },

        // ════════════════════════════════════════════════════════
        // 类别 2: 涨停揭秘/涨停复盘/打板 (期望丢弃)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "涨停揭秘：这只股票为何连续3板？", expected: None, rationale: "涨停揭秘" },
        LabeledItem { title: "今日涨停复盘：20只个股封板", expected: None, rationale: "涨停复盘" },
        LabeledItem { title: "涨停板复盘：主力资金净流入TOP10", expected: None, rationale: "涨停板复盘" },
        LabeledItem { title: "打板策略：如何捕捉强势龙头股", expected: None, rationale: "打板" },
        LabeledItem { title: "打板心得：次日溢价概率分析", expected: None, rationale: "打板" },
        LabeledItem { title: "今日涨停揭秘：储能板块掀涨停潮", expected: None, rationale: "涨停揭秘" },
        LabeledItem { title: "涨停复盘：高股息板块成避风港", expected: None, rationale: "涨停复盘" },
        LabeledItem { title: "打板族必看：龙头战法精解", expected: None, rationale: "打板" },

        // ════════════════════════════════════════════════════════
        // 类别 3: 龙虎榜/大宗交易 (期望丢弃, 单独处理)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "龙虎榜：机构席位净买入2.3亿元", expected: None, rationale: "龙虎榜" },
        LabeledItem { title: "今日龙虎榜：游资接力这只妖股", expected: None, rationale: "龙虎榜" },
        LabeledItem { title: "大宗交易：折价9.5%成交500万股", expected: None, rationale: "大宗交易" },
        LabeledItem { title: "机构龙虎榜：北向资金扫货名单", expected: None, rationale: "龙虎榜" },
        LabeledItem { title: "近一周大宗交易活跃度上升", expected: None, rationale: "大宗交易" },

        // ════════════════════════════════════════════════════════
        // 类别 4: 荐股/操作建议 (期望丢弃)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "明日操作策略：重点关注这三只个股", expected: None, rationale: "明日操作" },
        LabeledItem { title: "操盘必读：主力资金动向揭秘", expected: None, rationale: "操盘" },
        LabeledItem { title: "掘金龙头：这只股有望翻倍", expected: None, rationale: "掘金" },
        LabeledItem { title: "金股推荐：下周最具潜力5只股", expected: None, rationale: "金股" },
        LabeledItem { title: "操盘技巧：分时图买卖点解析", expected: None, rationale: "操盘" },
        LabeledItem { title: "明日布局：科技股龙头有望反弹", expected: None, rationale: "明日布局 (明日操作变体)" },
        LabeledItem { title: "金股池更新：精选3只低位标的", expected: None, rationale: "金股" },

        // ════════════════════════════════════════════════════════
        // 类别 5: 基金/ETF/保险/理财 (期望丢弃)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "基金净值播报：今日多只基金涨幅超2%", expected: None, rationale: "基金净值" },
        LabeledItem { title: "ETF资金净流入：宽基指数持续吸金", expected: None, rationale: "ETF" },
        LabeledItem { title: "保险板块异动：中国平安涨超3%", expected: None, rationale: "保险 (板块名, 易混)" },
        LabeledItem { title: "银行理财产品收益率持续下行", expected: None, rationale: "理财" },
        LabeledItem { title: "今日ETF行情：沪深300ETF成交活跃", expected: None, rationale: "ETF" },
        LabeledItem { title: "基金发行回暖：权益类基金占比上升", expected: None, rationale: "基金" },

        // ════════════════════════════════════════════════════════
        // 类别 6: 期货/外汇/黄金/原油 (期望丢弃)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "原油价格突破80美元/桶 创年内新高", expected: None, rationale: "原油" },
        LabeledItem { title: "黄金期货收涨0.5% 美元走弱支撑金价", expected: None, rationale: "黄金" },
        LabeledItem { title: "外汇市场：人民币兑美元中间价上调", expected: None, rationale: "外汇" },
        LabeledItem { title: "商品期货夜盘收盘：黑色系领跌", expected: None, rationale: "期货" },
        LabeledItem { title: "国际油价大涨 布伦特原油涨超2%", expected: None, rationale: "原油" },

        // ════════════════════════════════════════════════════════
        // 类别 7: TechBreak (期望识别为技术突破)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "重大突破！国产5nm芯片光刻技术取得关键进展", expected: Some(EventType::TechBreak), rationale: "突破" },
        LabeledItem { title: "比亚迪宣布固态电池量产 续航突破1000公里", expected: Some(EventType::TechBreak), rationale: "突破+量产" },
        LabeledItem { title: "全球首发：宁德时代发布钠离子电池新品", expected: Some(EventType::TechBreak), rationale: "首发+发布" },
        LabeledItem { title: "攻克卡脖子技术 国产高端光刻机进入验证阶段", expected: Some(EventType::TechBreak), rationale: "攻克" },
        LabeledItem { title: "华为推出全新昇腾AI芯片 性能翻倍", expected: Some(EventType::TechBreak), rationale: "推出" },
        LabeledItem { title: "中国天眼FAST首次观测到快速射电暴", expected: Some(EventType::TechBreak), rationale: "首次≈首发" },
        LabeledItem { title: "问世！国产大型水陆两栖飞机AG600完成首飞", expected: Some(EventType::TechBreak), rationale: "问世" },
        LabeledItem { title: "突破封锁！国产EDA软件实现7nm工艺支持", expected: Some(EventType::TechBreak), rationale: "突破" },

        // ════════════════════════════════════════════════════════
        // 类别 8: OrderWin (期望识别为订单中标)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "中国中车中标500亿元动车组采购合同", expected: Some(EventType::OrderWin), rationale: "中标+合同" },
        LabeledItem { title: "比亚迪获得特斯拉储能电池大单", expected: Some(EventType::OrderWin), rationale: "获得" },
        LabeledItem { title: "东方电缆签约海上风电项目 总金额超30亿", expected: Some(EventType::OrderWin), rationale: "签约" },
        LabeledItem { title: "中际旭创签下英伟达800G光模块订单", expected: Some(EventType::OrderWin), rationale: "订单" },
        LabeledItem { title: "中国建筑中标多个海外基建项目", expected: Some(EventType::OrderWin), rationale: "中标" },
        LabeledItem { title: "宁德时代获得欧洲车企百亿订单", expected: Some(EventType::OrderWin), rationale: "订单+获得" },

        // ════════════════════════════════════════════════════════
        // 类别 9: PriceUp (期望识别为涨价)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "碳酸锂价格上调5% 锂电板块应声上涨", expected: Some(EventType::PriceUp), rationale: "上调" },
        LabeledItem { title: "硅料价格持续上涨 多晶硅企业利润大增", expected: Some(EventType::PriceUp), rationale: "上涨+涨价" },
        LabeledItem { title: "内存芯片紧缺 DRAM价格大幅上调", expected: Some(EventType::PriceUp), rationale: "紧缺+上调" },
        LabeledItem { title: "焦煤价格上调 煤企盈利预期改善", expected: Some(EventType::PriceUp), rationale: "上调" },
        LabeledItem { title: "钢铁行业涨价潮：螺纹钢提价200元/吨", expected: Some(EventType::PriceUp), rationale: "涨价+提价" },
        LabeledItem { title: "MDI供不应求 万华化学再度提价", expected: Some(EventType::PriceUp), rationale: "供不应求+提价" },

        // ════════════════════════════════════════════════════════
        // 类别 10: PriceDown (期望识别为跌价)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "面板价格下调 京东方股价承压", expected: Some(EventType::PriceDown), rationale: "下调" },
        LabeledItem { title: "动力电池降价 新能源车成本下行", expected: Some(EventType::PriceDown), rationale: "降价" },
        LabeledItem { title: "猪肉价格持续下调 养殖板块承压", expected: Some(EventType::PriceDown), rationale: "下调" },

        // ════════════════════════════════════════════════════════
        // 类别 11: Capacity (期望识别为产能变化)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "宁德时代扩产 拟新建200GWh电池产能", expected: Some(EventType::Capacity), rationale: "扩产+新建" },
        LabeledItem { title: "中芯国际产能持续满载 国产替代加速", expected: Some(EventType::Capacity), rationale: "产能" },
        LabeledItem { title: "光伏巨头隆基绿能投产10GW新组件项目", expected: Some(EventType::Capacity), rationale: "投产" },
        LabeledItem { title: "通威股份新建20万吨硅料产能", expected: Some(EventType::Capacity), rationale: "新建" },
        LabeledItem { title: "中创新航产能扩张 武汉工厂全面投产", expected: Some(EventType::Capacity), rationale: "产能+投产+扩张" },

        // ════════════════════════════════════════════════════════
        // 类别 12: Mna (期望识别为并购)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "中国中化收购先正达 农业巨头重组落地", expected: Some(EventType::Mna), rationale: "收购+重组" },
        LabeledItem { title: "紫光国微入股瑞能半导体 深化产业链布局", expected: Some(EventType::Mna), rationale: "入股" },
        LabeledItem { title: "中国移动合并子公司 整合云计算业务", expected: Some(EventType::Mna), rationale: "合并" },
        LabeledItem { title: "中航电测并购成飞集团 军工资产注入", expected: Some(EventType::Mna), rationale: "并购" },
        LabeledItem { title: "伊利股份重组子公司 聚焦主业", expected: Some(EventType::Mna), rationale: "重组" },

        // ════════════════════════════════════════════════════════
        // 类别 13: Policy (期望识别为政策)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "国务院发布新能源汽车产业发展规划2025", expected: Some(EventType::Policy), rationale: "国务院" },
        LabeledItem { title: "工信部：加快推动集成电路产业高质量发展", expected: Some(EventType::Policy), rationale: "工信部" },
        LabeledItem { title: "央行下调存款准备金率0.25个百分点", expected: Some(EventType::Policy), rationale: "央行" },
        LabeledItem { title: "财政部发布支持上市公司并购重组税费优惠", expected: Some(EventType::Policy), rationale: "财政部" },
        LabeledItem { title: "政策利好：新能源补贴延长至2025年", expected: Some(EventType::Policy), rationale: "政策" },
        LabeledItem { title: "证监会：全面注册制改革正式启动", expected: Some(EventType::Policy), rationale: "证监会≈政策 (隐式)" },

        // ════════════════════════════════════════════════════════
        // 类别 14: Accident (期望识别为事故)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "某化工厂发生爆炸事故 周边居民紧急疏散", expected: Some(EventType::Accident), rationale: "爆炸+事故" },
        LabeledItem { title: "宁德时代子公司工厂火灾 已停产整顿", expected: Some(EventType::Accident), rationale: "火灾+停产" },
        LabeledItem { title: "台积电工厂化学品泄漏 股价盘中下挫", expected: Some(EventType::Accident), rationale: "泄漏" },
        LabeledItem { title: "某煤矿事故停产整顿 多家煤企受波及", expected: Some(EventType::Accident), rationale: "事故+停产" },

        // ════════════════════════════════════════════════════════
        // 类别 15: Overseas (期望识别为海外事件)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "美联储加息25个基点 美元指数走强", expected: Some(EventType::Overseas), rationale: "美联储+加息" },
        LabeledItem { title: "美国加征关税 中国光伏组件出口承压", expected: Some(EventType::Overseas), rationale: "关税" },
        LabeledItem { title: "欧盟对中国电动车实施制裁 商务部回应", expected: Some(EventType::Overseas), rationale: "制裁" },
        LabeledItem { title: "美国对华芯片禁令升级 多家A股公司回应", expected: Some(EventType::Overseas), rationale: "禁令" },
        LabeledItem { title: "英伟达对华出口受限 AI芯片板块波动", expected: Some(EventType::Overseas), rationale: "禁令 (隐式)" },

        // ════════════════════════════════════════════════════════
        // 类别 16: 边界/歧义 (测试鲁棒性, 期望识别为某个类别)
        // ════════════════════════════════════════════════════════
        LabeledItem { title: "宁德时代宣布研发投入翻倍 突破技术瓶颈", expected: Some(EventType::TechBreak), rationale: "突破 (边界: 同时含主体公司)" },
        LabeledItem { title: "美联储政策转向 全球资本市场震荡", expected: Some(EventType::Overseas), rationale: "美联储+政策 (优先 Overseas)" },
        LabeledItem { title: "工信部牵头组建半导体联盟 产业链协同攻关", expected: Some(EventType::Policy), rationale: "工信部 (优先 Policy)" },
        LabeledItem { title: "国资委：加大新一代信息技术产业投资力度", expected: Some(EventType::Policy), rationale: "国资委 (新增关键词)" },
        LabeledItem { title: "证监会发布退市新规 强化风险警示", expected: Some(EventType::Policy), rationale: "证监会 (新增关键词)" },
        LabeledItem { title: "国家发改委出台新型电力系统行动方案", expected: Some(EventType::Policy), rationale: "发改委 (新增关键词)" },
        LabeledItem { title: "特斯拉发布Optimus人形机器人最新进展", expected: Some(EventType::TechBreak), rationale: "发布 (TechBreak 兜底)" },
        LabeledItem { title: "紫光国微收购半导体IP公司", expected: Some(EventType::Mna), rationale: "收购 (Mna)" },
        LabeledItem { title: "今日财经热点一览 涵盖多个板块", expected: Some(EventType::Other), rationale: "无明确事件类型, AI 兜底" },
    ]
}

fn raw_item(title: &str) -> RawNewsItem {
    RawNewsItem {
        title: title.into(),
        body: "".into(),
        source: "benchmark".into(),
        source_priority: 1,
        source_type: SourceType::Search,
        published_at: chrono::Local::now(),
        url: None,
    }
}

/// 计算每个 (类别, 正确/错误) 的统计
struct CategoryStats {
    total: usize,
    correct: usize,
    wrong_titles: Vec<(String, Option<EventType>, Option<EventType>)>,
}

impl CategoryStats {
    fn new() -> Self {
        Self { total: 0, correct: 0, wrong_titles: vec![] }
    }
    fn precision(&self) -> f64 {
        if self.total == 0 { 1.0 } else { self.correct as f64 / self.total as f64 }
    }
}

fn classify_category(expected: Option<EventType>) -> &'static str {
    match expected {
        None => "丢弃 (噪声)",
        Some(EventType::TechBreak) => "TechBreak",
        Some(EventType::OrderWin) => "OrderWin",
        Some(EventType::PriceUp) => "PriceUp",
        Some(EventType::PriceDown) => "PriceDown",
        Some(EventType::Capacity) => "Capacity",
        Some(EventType::Mna) => "Mna",
        Some(EventType::Policy) => "Policy",
        Some(EventType::Accident) => "Accident",
        Some(EventType::Overseas) => "Overseas",
        Some(EventType::Other) => "Other",
    }
}

#[test]
fn test_rule_filter_benchmark_85pct() {
    let dataset = labeled_dataset();
    assert_eq!(dataset.len(), 100, "数据集必为 100 条");

    let mut total_correct = 0usize;
    let mut total = 0usize;
    let mut cat_stats: std::collections::HashMap<&'static str, CategoryStats> = std::collections::HashMap::new();

    for item in &dataset {
        total += 1;
        let raw = raw_item(item.title);
        let rm: RuleMatch = RuleFilter::filter(&raw);
        let cat = classify_category(item.expected);
        let stats = cat_stats.entry(cat).or_insert_with(CategoryStats::new);
        stats.total += 1;

        // 严格匹配: 期望 None → 必须 matched=false; 期望 Some(T) → 必须 matched=true 且 event_type=Some(T)
        let strict_correct = match item.expected {
            None => !rm.matched,
            Some(et) => rm.matched && rm.event_type == Some(et),
        };
        if strict_correct {
            total_correct += 1;
            stats.correct += 1;
        } else {
            stats.wrong_titles.push((
                item.title.to_string(),
                item.expected,
                rm.event_type,
            ));
        }
    }

    let accuracy = total_correct as f64 / total as f64;
    println!("\n═══ rule_filter 基准测试 (spec §8 验收: ≥ 85%) ═══");
    println!("总样本: {} | 严格正确: {} | 严格精确率: {:.1}%", total, total_correct, accuracy * 100.0);

    // 类别细分
    println!("\n{:<20} {:>6} {:>6} {:>10}", "类别", "总数", "正确", "精确率");
    println!("{}", "─".repeat(46));
    let mut sorted_cats: Vec<_> = cat_stats.iter().collect();
    sorted_cats.sort_by(|a, b| a.0.cmp(b.0));
    for (cat, stats) in &sorted_cats {
        println!("{:<20} {:>6} {:>6} {:>9.1}%", cat, stats.total, stats.correct, stats.precision() * 100.0);
    }

    // 错误样本
    let wrong_total: usize = cat_stats.values().map(|s| s.wrong_titles.len()).sum();
    if wrong_total > 0 {
        println!("\n═══ 错误样本 ({} 条) ═══", wrong_total);
        for (cat, stats) in &sorted_cats {
            if stats.wrong_titles.is_empty() { continue; }
            println!("\n[{}] 类别错误 ({} 条):", cat, stats.wrong_titles.len());
            for (title, expected, got) in &stats.wrong_titles {
                let exp_str = match expected {
                    Some(et) => format!("{:?}", et),
                    None => "丢弃".to_string(),
                };
                let got_str = match got {
                    Some(et) => format!("{:?}", et),
                    None => "丢弃".to_string(),
                };
                println!("  ✗ \"{}\"", title);
                println!("    期望: {} → 实际: {}", exp_str, got_str);
            }
        }
    }

    // spec §8 验收门槛
    assert!(
        accuracy >= 0.85,
        "rule_filter 严格精确率 {:.1}% < 85%, 需优化规则 (见上方错误样本)",
        accuracy * 100.0
    );
}

#[test]
fn test_rule_filter_per_category_meets_threshold() {
    // 强化: 每个关键类别单独 ≥ 80%, 防止"以 noise 高精确率掩盖 event_type 漏检"
    let dataset = labeled_dataset();
    let mut cat_stats: std::collections::HashMap<&'static str, CategoryStats> = std::collections::HashMap::new();

    for item in &dataset {
        let raw = raw_item(item.title);
        let rm = RuleFilter::filter(&raw);
        let cat = classify_category(item.expected);
        let stats = cat_stats.entry(cat).or_insert_with(CategoryStats::new);
        stats.total += 1;
        let strict_correct = match item.expected {
            None => !rm.matched,
            Some(et) => rm.matched && rm.event_type == Some(et),
        };
        if strict_correct { stats.correct += 1; }
        else { stats.wrong_titles.push((item.title.to_string(), item.expected, rm.event_type)); }
    }

    // 关键 keep 类别门槛 80% (允许少量边界 case 失败)
    let critical_cats = ["TechBreak", "OrderWin", "PriceUp", "Policy", "Accident", "Overseas", "Capacity", "Mna"];
    for cat_name in critical_cats {
        if let Some(stats) = cat_stats.get(cat_name) {
            assert!(
                stats.precision() >= 0.80,
                "类别 [{}] 精确率 {:.1}% < 80%, 需修复. 错误样本:\n{}",
                cat_name,
                stats.precision() * 100.0,
                stats.wrong_titles.iter().map(|(t, e, g)| format!("  - \"{}\" 期望={:?} 实际={:?}", t, e, g)).collect::<Vec<_>>().join("\n")
            );
        }
    }

    // 丢弃类别门槛 90% (避免噪声漏过滤)
    if let Some(stats) = cat_stats.get("丢弃 (噪声)") {
        assert!(
            stats.precision() >= 0.90,
            "丢弃类别精确率 {:.1}% < 90%, 噪声漏过滤严重. 错误样本:\n{}",
            stats.precision() * 100.0,
            stats.wrong_titles.iter().map(|(t, e, g)| format!("  - \"{}\" 期望={:?} 实际={:?}", t, e, g)).collect::<Vec<_>>().join("\n")
        );
    }
}