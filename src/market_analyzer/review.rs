//! review（从 market_analyzer.rs 拆分）

use log::{error, info, warn};
use chrono::Local;

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
| 北向资金 | {:+.2}亿 |

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
            overview.north_flow,
            top_text,
            bottom_text,
            self.format_top_stocks(&overview.top_stocks),
            now
        )
    }

    /// 构建复盘报告 Prompt
    pub(super) fn build_review_prompt(&self, overview: &MarketOverview, news: &[SearchResponse]) -> String {
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
- 北向资金: {:+.2} 亿元

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
            overview.north_flow,
            top_sectors_text,
            bottom_sectors_text,
            news_text,
            overview.date
        )
    }

    /// 使用AI生成大盘复盘报告
    pub fn generate_market_review(&self, overview: &MarketOverview, news: &[SearchResponse]) -> String {
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
