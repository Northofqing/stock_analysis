//! review（从 market_analyzer.rs 拆分）

use chrono::Local;
use log::{error, info, warn};

use crate::market_data::MarketOverview;
use crate::search_service::SearchResponse;

use super::MarketAnalyzer;

impl MarketAnalyzer {
    /// 生成大盘复盘报告（模板版本）
    pub fn generate_template_review(&self, overview: &MarketOverview) -> String {
        let market_mood = overview.market_mood();

        // 指数行情
        let mut indices_text = String::new();
        for idx in overview.indices.iter().take(4) {
            let direction = if idx.change_pct > 0.0 {
                "↑"
            } else if idx.change_pct < 0.0 {
                "↓"
            } else {
                "-"
            };
            indices_text.push_str(&format!(
                "- **{}**: {:.2} ({}{}%)\n",
                idx.name,
                idx.current,
                direction,
                idx.change_pct.abs()
            ));
        }

        // 板块信息
        let top_text = overview
            .top_sectors
            .iter()
            .take(3)
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("、");

        let bottom_text = overview
            .bottom_sectors
            .iter()
            .take(3)
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("、");

        let now = Local::now().format("%H:%M");

        format!(
            r#"## 📊 {} 大盘复盘

### 一、市场总结
今日A股市场整体呈现**{}**态势。

### 二、主要指数
{}

### 三、涨跌统计
| 指标 | 数值 |
|------|------|
| 上涨家数 | {} |
| 下跌家数 | {} |
| 涨停 | {} |
| 跌停 | {} |
| 两市成交额 | {:.0}亿 |
| 北向资金 | {} |

### 四、板块表现
- **领涨**: {}
- **领跌**: {}

### 五、涨幅前十个股
| 排名 | 代码 | 名称 | 涨幅 | 现价 |
|------|------|------|------|------|
{}

### 六、风险提示
市场有风险，投资需谨慎。以上数据仅供参考，不构成投资建议。

---
*复盘时间: {}*
"#,
            overview.date,
            market_mood,
            indices_text,
            overview.up_count,
            overview.down_count,
            overview.limit_up_count,
            overview.limit_down_count,
            overview.total_amount,
            // 修复 P1-3: None 时打 [数据缺失], 禁止 0.00 假数据 (BR-012)
            overview
                .north_flow
                .map(|v| format!("{:+.2}亿", v))
                .unwrap_or_else(|| "[数据缺失]".to_string()),
            top_text,
            bottom_text,
            self.format_top_stocks(&overview.top_stocks),
            now
        )
    }

    /// 构建复盘报告 Prompt
    pub(super) fn build_review_prompt(
        &self,
        overview: &MarketOverview,
        news: &[SearchResponse],
    ) -> String {
        // 指数行情信息（简洁格式，不用emoji）
        let mut indices_text = String::new();
        for idx in &overview.indices {
            let direction = if idx.change_pct > 0.0 {
                "↑"
            } else if idx.change_pct < 0.0 {
                "↓"
            } else {
                "-"
            };
            indices_text.push_str(&format!(
                "- {}: {:.2} ({}{}%)\n",
                idx.name,
                idx.current,
                direction,
                idx.change_pct.abs()
            ));
        }

        // 板块信息
        let top_sectors_text = overview
            .top_sectors
            .iter()
            .take(3)
            .map(|s| format!("{}({:+.2}%)", s.name, s.change_pct))
            .collect::<Vec<_>>()
            .join(", ");

        let bottom_sectors_text = overview
            .bottom_sectors
            .iter()
            .take(3)
            .map(|s| format!("{}({:+.2}%)", s.name, s.change_pct))
            .collect::<Vec<_>>()
            .join(", ");

        // 新闻信息
        let mut news_text = String::new();
        let mut count = 0;
        for response in news.iter().take(6) {
            for result in response.results.iter() {
                count += 1;
                if count > 6 {
                    break;
                }
                let title = result.title.chars().take(50).collect::<String>();
                let snippet = result.snippet.chars().take(100).collect::<String>();
                news_text.push_str(&format!("{}. {}\n   {}\n", count, title, snippet));
            }
            if count > 6 {
                break;
            }
        }

        if news_text.is_empty() {
            news_text = "暂无相关新闻".to_string();
        }

        format!(
            r#"你是一位专业的A股市场分析师，请根据以下数据生成一份简洁的大盘复盘报告。

【重要】输出要求：
- 必须输出纯 Markdown 文本格式
- 禁止输出 JSON 格式
- 禁止输出代码块
- emoji 仅在标题处少量使用（每个标题最多1个）

---

# 今日市场数据

## 日期
{}

## 主要指数
{}

## 市场概况
- 上涨: {} 家 | 下跌: {} 家 | 平盘: {} 家
- 涨停: {} 家 | 跌停: {} 家
- 两市成交额: {:.0} 亿元
 - 北向资金: {}

## 板块表现
领涨: {}
领跌: {}

## 市场新闻
{}

---

# 输出格式模板（请严格按此格式输出）

## 📊 {} 大盘复盘

### 一、市场总结
（2-3句话概括今日市场整体表现，包括指数涨跌、成交量变化）

### 二、指数点评
（分析上证、深证、创业板等各指数走势特点）

### 三、资金动向
（解读成交额和北向资金流向的含义）

### 四、热点解读
（分析领涨领跌板块背后的逻辑和驱动因素）

### 五、后市展望
（结合当前走势和新闻，给出明日市场预判）

### 六、风险提示
（需要关注的风险点）

---

请直接输出复盘报告内容，不要输出其他说明文字。
"#,
            overview.date,
            indices_text,
            overview.up_count,
            overview.down_count,
            overview.flat_count,
            overview.limit_up_count,
            overview.limit_down_count,
            overview.total_amount,
            // 修复 P1-3: None → [数据缺失] (BR-012)
            overview
                .north_flow
                .map(|v| format!("{:+.2} 亿元", v))
                .unwrap_or_else(|| "[数据缺失]".to_string()),
            top_sectors_text,
            bottom_sectors_text,
            news_text,
            overview.date
        )
    }

    /// 使用AI生成大盘复盘报告
    pub fn generate_market_review(
        &self,
        overview: &MarketOverview,
        news: &[SearchResponse],
    ) -> String {
        // 如果没有AI分析器，使用模板
        if self.ai_analyzer.is_none() {
            warn!("[大盘] AI分析器未配置，使用模板生成报告");
            return self.generate_template_review(overview);
        }

        let analyzer = self.ai_analyzer.as_ref().unwrap();
        if !analyzer.is_available() {
            warn!("[大盘] AI分析器不可用，使用模板生成报告");
            return self.generate_template_review(overview);
        }

        // 构建 Prompt
        let prompt = self.build_review_prompt(overview, news);

        info!("[大盘] 调用大模型生成复盘报告...");

        match analyzer.generate_content(&prompt, 0.7, 2048) {
            Ok(review) => {
                info!("[大盘] 复盘报告生成成功，长度: {} 字符", review.len());
                review
            }
            Err(e) => {
                error!("[大盘] 大模型生成复盘报告失败: {:?}", e);
                self.generate_template_review(overview)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_data::{MarketIndex, SectorInfo, TopStock};
    use crate::search_service::{SearchResponse, SearchResult};

    fn overview() -> MarketOverview {
        let mut overview = MarketOverview::new("2026-07-18".to_string());
        for (code, name, change) in [
            ("sh000001", "上证指数", 1.2),
            ("sz399001", "深证成指", -0.8),
            ("sz399006", "创业板指", 0.0),
            ("sh000688", "科创50", 0.3),
        ] {
            let mut index = MarketIndex::new(code.to_string(), name.to_string());
            index.current = 3_000.0;
            index.change_pct = change;
            overview.indices.push(index);
        }
        overview.up_count = 3_200;
        overview.down_count = 1_500;
        overview.flat_count = 100;
        overview.limit_up_count = 60;
        overview.limit_down_count = 5;
        overview.total_amount = 12_345.0;
        overview.north_flow = Some(12.5);
        overview.top_sectors = vec![SectorInfo {
            name: "TEST_CODE_机器人".to_string(),
            change_pct: 3.2,
        }];
        overview.bottom_sectors = vec![SectorInfo {
            name: "TEST_CODE_煤炭".to_string(),
            change_pct: -2.1,
        }];
        overview.top_stocks = vec![TopStock {
            code: "TEST_CODE_000001".to_string(),
            name: "TEST_CODE_样本".to_string(),
            change_pct: 9.9,
            price: 12.34,
            ..TopStock::default()
        }];
        overview
    }

    #[test]
    fn template_prompt_and_no_ai_fallback_preserve_complete_market_facts() {
        let analyzer = MarketAnalyzer::new(None).expect("local analyzer");
        let overview = overview();
        let news = vec![SearchResponse::success(
            "TEST_CODE_market".to_string(),
            "TEST_CODE_fixture".to_string(),
            vec![SearchResult::new(
                "TEST_CODE 政策发布".to_string(),
                "TEST_CODE 市场成交活跃".to_string(),
                "https://example.invalid/news".to_string(),
                "TEST_CODE_fixture".to_string(),
            )],
        )];

        let template = analyzer.generate_template_review(&overview);
        assert!(template.contains("强势上涨"));
        assert!(template.contains("北向资金 | +12.50亿"));
        assert!(template.contains("TEST_CODE_样本"));

        let prompt = analyzer.build_review_prompt(&overview, &news);
        assert!(prompt.contains("TEST_CODE 政策发布"));
        assert!(prompt.contains("TEST_CODE_机器人(+3.20%)"));
        assert!(prompt.contains("+12.50 亿元"));

        let generated = analyzer.generate_market_review(&overview, &news);
        assert_eq!(generated, template);
    }

    #[test]
    fn missing_optional_market_facts_are_marked_not_filled() {
        let analyzer = MarketAnalyzer::new(None).expect("local analyzer");
        let mut overview = overview();
        overview.indices.clear();
        overview.north_flow = None;
        let prompt = analyzer.build_review_prompt(&overview, &[]);
        assert!(prompt.contains("[数据缺失]"));
        assert!(prompt.contains("暂无相关新闻"));
        assert!(analyzer
            .generate_template_review(&overview)
            .contains("震荡整理"));
    }
}
