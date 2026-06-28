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
        // 修复 v9.1 基准测试: 补充同义变体, 减少噪声漏过滤
        let discard_rules: &[(&[&str], &str)] = &[
            (&["收评", "复盘", "盘面", "午评", "早评", "午盘", "早盘", "午盘解析", "早盘观察"], "收评/复盘"),
            (&["涨停揭秘", "涨停复盘", "涨停板复盘", "打板"], "涨停揭秘"),
            (&["龙虎榜", "大宗交易"], "龙虎榜/大宗交易"),
            (&["明日操作", "明日布局", "操盘", "掘金", "金股"], "荐股号"),
            (&["基金净值", "基金发行", "ETF", "保险", "理财"], "非股票域"),
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
        // 修复 v9.1 基准测试:
        //   - 顺序按"具体 > 通用"排, 避免低层关键词抢高层 (例 "国务院发布..." 优先归 Policy)
        //   - 补充同义变体: "受限"/"首次"/"价格上涨" 等
        let keep_rules: &[(&[&str], EventType)] = &[
            // 海外事件优先: 央行/美联储类宏观动作
            (&["美联储", "加息", "制裁", "禁令", "关税", "受限", "出口管制"], EventType::Overseas),
            // 事故优先: 停产/火灾等危害性事件
            (&["停产", "事故", "火灾", "爆炸", "泄漏"], EventType::Accident),
            // 政策优先: 官方主体动作 (含证监会/发改委/国资委)
            (&["政策", "国务院", "工信部", "央行", "财政部", "证监会", "发改委", "国资委", "回购股份"], EventType::Policy),
            // 价格变动 (含"价格上涨"/"价格上调"等变体)
            (&["涨价", "提价", "上调", "紧缺", "供不应求", "价格上涨", "价格持续上涨"], EventType::PriceUp),
            (&["降价", "下调", "价格下调"], EventType::PriceDown),
            // 并购重组
            (&["收购", "重组", "合并", "并购", "入股"], EventType::Mna),
            // 产能变化
            (&["扩产", "投产", "产能", "新建"], EventType::Capacity),
            // 订单中标
            (&["订单", "中标", "签约", "合同", "获得"], EventType::OrderWin),
            // 技术突破 (最后, 因为 "发布"/"推出" 容易被其他规则先匹配)
            (&["突破", "量产", "首发", "全球首", "攻克", "发布", "推出", "问世", "首飞", "首次", "新进展"], EventType::TechBreak),
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
