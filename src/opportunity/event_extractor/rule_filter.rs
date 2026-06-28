use crate::signal::market_event::EventType;
use super::adapter::RawNewsItem;

pub struct RuleMatch {
    pub item: RawNewsItem,
    pub matched: bool,
    pub event_type: Option<EventType>,
    pub discard_reason: Option<String>,
}

pub struct RuleFilter;

impl RuleFilter {
    pub fn filter(item: &RawNewsItem) -> RuleMatch {
        let t = &item.title;

        // 1. Discard rules (priority over keep)
        let discard_rules: &[(&[&str], &str)] = &[
            (&["收评", "复盘", "盘面", "午评", "早评"], "收评/复盘"),
            (&["涨停揭秘", "涨停复盘", "打板"], "涨停揭秘"),
            (&["龙虎榜", "大宗交易"], "龙虎榜/大宗交易"),
            (&["明日操作", "操盘", "掘金", "金股"], "荐股号"),
            (&["基金净值", "ETF", "保险", "理财"], "非股票域"),
            (&["期货", "外汇", "黄金", "原油"], "非A股域"),
        ];
        for (keywords, reason) in discard_rules {
            if keywords.iter().any(|k| t.contains(k)) {
                return RuleMatch {
                    item: item.clone(), matched: false,
                    event_type: None,
                    discard_reason: Some(reason.to_string()),
                };
            }
        }

        // 2. Keep rules (anchor event_type)
        let keep_rules: &[(&[&str], EventType)] = &[
            (&["突破", "量产", "首发", "全球首", "攻克", "发布", "推出", "问世"], EventType::TechBreak),
            (&["订单", "中标", "签约", "合同", "获得"], EventType::OrderWin),
            (&["涨价", "提价", "上调", "紧缺", "供不应求"], EventType::PriceUp),
            (&["降价", "下调"], EventType::PriceDown),
            (&["扩产", "投产", "产能", "新建"], EventType::Capacity),
            (&["收购", "重组", "合并", "并购", "入股"], EventType::Mna),
            (&["政策", "国务院", "工信部", "央行", "财政部"], EventType::Policy),
            (&["停产", "事故", "火灾", "爆炸", "泄漏"], EventType::Accident),
            (&["制裁", "禁令", "关税", "美联储", "加息"], EventType::Overseas),
        ];
        for (keywords, event_type) in keep_rules {
            if keywords.iter().any(|k| t.contains(k)) {
                return RuleMatch {
                    item: item.clone(), matched: true,
                    event_type: Some(*event_type),
                    discard_reason: None,
                };
            }
        }

        // 3. Unknown keyword → keep (AI fallback), mark as Other
        RuleMatch {
            item: item.clone(), matched: true,
            event_type: Some(EventType::Other),
            discard_reason: None,
        }
    }
}
