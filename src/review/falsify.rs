//! 证伪清单 — 每日一条"我会在哪里亏钱"提醒。

const FALSIFY_ITEMS: &[&str] = &[
    "我把「方向对」当成了「会赚钱」，买在了对的赛道、错的时机",
    "我把「缩量阴跌出货」误判成了「健康洗盘」，安心拿着等腰斩",
    "我「越跌越买」却没设止损线，在趋势结束时越套越深",
    "我因为「它是龙头」就死扛，无视了龙头逻辑已被证伪",
    "我追了「高位放量」，买在了主力派发区",
    "我被某份「夸我」的分析喂养了信心，放松了纪律",
    "我让现金睡着了，回撤来时不敢加，或乱加补了主题仓",
    "我轮动太频繁被反复打脸，或太迟钝抱着旧主线不放",
];

/// 每日证伪提醒（按日期轮换，每天一条）
pub fn daily_falsify() -> String {
    let today = chrono::Local::now().format("%j").to_string();
    let day_of_year: usize = today.parse().unwrap_or(1);
    let idx = day_of_year % FALSIFY_ITEMS.len();
    format!("⚠️ 今日证伪:\n  「{}」", FALSIFY_ITEMS[idx])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_falsify_returns_something() {
        let text = daily_falsify();
        assert!(text.contains("证伪"));
        assert!(!text.is_empty());
    }

    #[test]
    fn test_all_items_accessible() {
        for i in 0..FALSIFY_ITEMS.len() {
            assert!(!FALSIFY_ITEMS[i].is_empty());
        }
    }
}
