//! v15.3 Phase D6: NewsDispatcher — impact → v14 push

use crate::news::impact::NewsImpact;
use crate::signal::market_event::Direction;

#[derive(Debug, Clone)]
pub struct NewsPush {
    pub text: String,
    pub headline: String,
    pub code: Option<String>,
    pub score: f64,
    pub direction: Direction,
}

/// A 股常用名 lookup (60 只 + 扩展空间). 真生产应该走 entity_linker, 但那需要 db cache.
pub fn lookup_name(code: &str) -> &'static str {
    match code {
        // 沪深 300 头部
        "600519" => "贵州茅台",
        "601318" => "中国平安",
        "000001" => "平安银行",
        "600036" => "招商银行",
        "000858" => "五粮液",
        "601398" => "工商银行",
        "601939" => "建设银行",
        "601988" => "中国银行",
        "600276" => "恒瑞医药",
        "000333" => "美的集团",
        // 科技 / 半导体
        "688981" => "中芯国际",
        "603986" => "兆易创新",
        "002371" => "北方华创",
        "688012" => "中微公司",
        "002409" => "雅克科技",
        "002049" => "紫光国微",
        // 新能源车
        "300750" => "宁德时代",
        "002594" => "比亚迪",
        "300014" => "亿纬锂能",
        "002460" => "赣锋锂业",
        "300274" => "阳光电源",
        "002050" => "三花智控",
        // 机器人 / 商业航天 / 储能
        "002472" => "双环传动",
        "601608" => "中信重工",
        "002379" => "宏桥控股",
        "002156" => "通富微电",
        "002436" => "兴森科技",
        "002185" => "华天科技",
        "002421" => "达实智能",
        "002413" => "雷科防务",
        "600703" => "三安光电",
        "002008" => "大族激光",
        "600641" => "先导基电",
        "603082" => "北自科技",
        "000657" => "中钨高新",
        "000636" => "风华高科",
        _ => "未知",
    }
}

pub fn decide(impact: &NewsImpact) -> Option<NewsPush> {
    if impact.score < 40.0 {
        return None;
    }
    let score_clipped = impact.score.min(100.0);
    let name = lookup_name(&impact.code);
    // P1-2: 类别事件(政府监管/宏观)或未收录代码 → lookup_name 返回 "未知"
    //   不再显示 "未知(政府监管)"; 直接用 code (类别名或裸代码) 作展示, 避免 d7c2360 格式回归
    let name_display = if name == "未知" {
        impact.code.clone()
    } else {
        format!("{}({})", name, impact.code)
    };
    let age_str = if impact.age_hours < 0.5 {
        "新".to_string()
    } else {
        format!("{:.1}h", impact.age_hours)
    };
    Some(NewsPush {
        text: format!(
            "📊 {} | 分数 {:.0} | {} {} | 多源×{} | {}",
            name_display,
            score_clipped,
            match impact.direction {
                Direction::Bull => "🟢",
                Direction::Bear => "🔴",
                _ => "⚪",
            },
            impact.reason,
            impact.source_count,
            age_str,
        ),
        headline: impact.reason.clone(),
        code: Some(impact.code.clone()),
        score: score_clipped,
        direction: impact.direction,
    })
}

pub fn is_important(p: &NewsPush) -> bool {
    p.score >= 70.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news::impact::{RelationType, score_event};
    use crate::signal::market_event::{Direction, EventType, MarketEvent, SourceRef};
    use chrono::{Local, Utc};

    fn mk_event(dir: Direction, strength: u8, code: &str, title: &str) -> MarketEvent {
        let now = Utc::now().with_timezone(&Local);
        MarketEvent {
            event_id: format!("test-{code}"),
            simhash: 42,
            full_title: title.into(),
            event_type: EventType::Other,
            subject: code.into(),
            object: Some(code.into()),
            direction: dir,
            strength,
            certainty: 80,
            chains: vec![],
            occurred_at: now,
            provenance: vec![SourceRef { provider: "test".into(), url: None, fetched_at: now }],
            ai_degraded: false,
            stale: false,
        }
    }

    #[test]
    fn test_decide_high_score_pushed() {
        let e = mk_event(Direction::Bull, 100, "000001", "长鑫 IPO");
        let imp = score_event(&e, RelationType::SelfCode, 2);
        assert!(imp.score > 70.0);
        assert!(decide(&imp).is_some());
        assert!(is_important(&decide(&imp).unwrap()));
    }

    #[test]
    fn test_decide_low_score_dropped() {
        let e = mk_event(Direction::Neutral, 10, "600519", "弱产业传闻");
        let imp = score_event(&e, RelationType::Industry, 1);
        assert!(imp.score < 40.0, "low score {}, should be < 40", imp.score);
        assert!(decide(&imp).is_none());
    }

    #[test]
    fn test_decide_mid_score_pushed_not_important() {
        let e = mk_event(Direction::Bear, 30, "300750", "新能源车政策微调");
        let imp = score_event(&e, RelationType::PolicyImpact, 1);
        let p = decide(&imp).expect("should be decided above 40 threshold");
        assert!(p.score >= 40.0, "score {} should be ≥ 40", p.score);
        assert!(p.score < 70.0, "score {} should be < 70 (info tier)", p.score);
        assert!(!is_important(&p));
    }

    #[test]
    fn test_decide_category_event_no_unknown_label() {
        // P1-2: 类别事件(政府监管, 非股票代码) 不应显示 "未知(政府监管)", 应直接 "政府监管"
        let e = mk_event(Direction::Neutral, 30, "政府监管", "国家发展改革委公告");
        let imp = score_event(&e, RelationType::PolicyImpact, 1);
        let p = decide(&imp).expect("政策类应 ≥40 推送");
        assert!(!p.text.contains("未知("), "类别事件不应出现 '未知(': {}", p.text);
        assert!(p.text.contains("📊 政府监管 |"), "应直接显示类别名: {}", p.text);
    }
}
