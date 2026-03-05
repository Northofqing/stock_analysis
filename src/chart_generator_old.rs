// -*- coding: utf-8 -*-
//! 图表生成模块
//!
//! 职责：
//! 1. 将分析结果转换为可视化图表
//! 2. 生成股票评分排行榜
//! 3. 生成操作建议分布图

use anyhow::Result;
use plotters::prelude::*;
use std::path::PathBuf;

use crate::pipeline::AnalysisResult;

/// 图表生成器
pub struct ChartGenerator;

impl ChartGenerator {
    /// 生成分析结果汇总图表
    pub fn generate_summary_chart(
        results: &[AnalysisResult],
        output_path: &str,
    ) -> Result<PathBuf> {
        let path = PathBuf::from(output_path);
        
        // 按评分排序
        let mut sorted_results = results.to_vec();
        sorted_results.sort_by(|a, b| b.sentiment_score.cmp(&a.sentiment_score));
        
        // 只取前15只股票（图表太长会不清晰）
        let display_results = if sorted_results.len() > 15 {
            &sorted_results[..15]
        } else {
            &sorted_results[..]
        };
        
        {
            // 创建绘图区域 (1200x800)
            let root = BitMapBackend::new(&path, (1200, 800)).into_drawing_area();
            root.fill(&WHITE)?;
            
            // 分成上下两部分：上面是评分柱状图，下面是操作建议分布图
            let areas = root.split_evenly((2, 1));
            
            // 上半部分：评分柱状图
            Self::draw_score_bar_chart(&areas[0], display_results)?;
            
            // 下半部分：操作建议分布图
            Self::draw_operation_pie_chart(&areas[1], results)?;
            
            root.present()?;
        } // root 在这里被 drop
        
        Ok(path)
    }
    
    /// 绘制评分柱状图
    fn draw_score_bar_chart(
        area: &DrawingArea<BitMapBackend, plotters::coord::Shift>,
        results: &[AnalysisResult],
    ) -> Result<()> {
        let max_score = results.iter().map(|r| r.sentiment_score).max().unwrap_or(100);
        
        let mut chart = ChartBuilder::on(area)
            .caption("📊 股票评分排行榜 Top 15", ("sans-serif", 30).into_font().color(&BLACK))
            .margin(15)
            .x_label_area_size(80)
            .y_label_area_size(60)
            .build_cartesian_2d(
                0..results.len(),
                0..(max_score + 10),
            )?;
        
        chart
            .configure_mesh()
            .y_desc("评分")
            .x_labels(results.len())
            .y_labels(10)
            .x_label_formatter(&|x| {
                if *x < results.len() {
                    format!("{}\n{}", results[*x].name, results[*x].code)
                } else {
                    String::new()
                }
            })
            .x_label_style(("sans-serif", 12).into_font())
            .y_label_style(("sans-serif", 14).into_font())
            .draw()?;
        
        // 绘制柱状图
        chart.draw_series(
            results.iter().enumerate().map(|(i, result)| {
                let color = match result.sentiment_score {
                    80.. => RGBColor(76, 175, 80),   // 绿色 - 优秀
                    70..=79 => RGBColor(139, 195, 74), // 浅绿 - 良好
                    50..=69 => RGBColor(255, 193, 7),  // 黄色 - 中性
                    40..=49 => RGBColor(255, 152, 0),  // 橙色 - 偏弱
                    _ => RGBColor(244, 67, 54),        // 红色 - 弱势
                };
                
                Rectangle::new(
                    [(i, 0), (i + 1, result.sentiment_score)],
                    color.filled(),
                )
            }),
        )?;
        
        // 在柱状图上方显示评分数值
        for (i, result) in results.iter().enumerate() {
            let text_color = if result.sentiment_score >= 70 {
                &GREEN
            } else if result.sentiment_score >= 50 {
                &YELLOW
            } else {
                &RED
            };
            
            chart.draw_series(std::iter::once(Text::new(
                format!("{}", result.sentiment_score),
                (i, result.sentiment_score + 2),
                ("sans-serif", 14).into_font().color(text_color),
            )))?;
        }
        
        Ok(())
    }
    
    /// 绘制操作建议分布饼图
    fn draw_operation_pie_chart(
        area: &DrawingArea<BitMapBackend, plotters::coord::Shift>,
        results: &[AnalysisResult],
    ) -> Result<()> {
        // 统计操作建议分布
        let mut buy_count = 0;
        let mut sell_count = 0;
        let mut hold_count = 0;
        
        for result in results {
            match result.operation_advice.as_str() {
                "买入" | "加仓" | "强烈买入" | "强烈建议买入" | "建议买入" => buy_count += 1,
                "卖出" | "减仓" | "强烈卖出" | "建议减仓" => sell_count += 1,
                _ => hold_count += 1,
            }
        }
        
        let total = results.len() as f64;
        
        // 创建标题
        area.titled(
            "📈 操作建议分布",
            ("sans-serif", 28).into_font().color(&BLACK),
        )?;
        
        // 绘制简化的统计信息（因为plotters的饼图支持有限，我们用矩形块表示）
        let block_height = 80;
        let start_y = 100;
        let colors = vec![
            (&GREEN, buy_count, "买入/加仓"),
            (&YELLOW, hold_count, "持有/观望"),
            (&RED, sell_count, "卖出/减仓"),
        ];
        
        for (idx, (color, count, label)) in colors.iter().enumerate() {
            let y_pos = start_y + idx * (block_height + 20);
            let percentage = (*count as f64 / total * 100.0) as i32;
            let bar_width = (800.0 * (*count as f64 / total)) as i32;
            
            // 绘制彩色条
            let rect = Rectangle::new(
                [(50, y_pos), (50 + bar_width, y_pos + block_height)],
                color.filled(),
            );
            area.draw(&rect)?;
            
            // 绘制边框
            let border = Rectangle::new(
                [(50, y_pos), (50 + bar_width, y_pos + block_height)],
                BLACK.stroke_width(2),
            );
            area.draw(&border)?;
            
            // 绘制标签
            let label_text = Text::new(
                format!("{}: {} 只 ({}%)", label, count, percentage),
                (900, y_pos + 30),
                ("sans-serif", 24).into_font().color(&BLACK),
            );
            area.draw(&label_text)?;
            
            // 在条内显示数量
            if bar_width > 50 {
                let count_text = Text::new(
                    format!("{}", count),
                    (50 + bar_width / 2 - 10, y_pos + 30),
                    ("sans-serif", 28).into_font().color(&WHITE),
                );
                area.draw(&count_text)?;
            }
        }
        
        // 添加总计信息
        let total_text = Text::new(
            format!("总计: {} 只股票", results.len()),
            (50, start_y + colors.len() * (block_height + 20) + 40),
            ("sans-serif", 22).into_font().color(&BLACK),
        );
        area.draw(&total_text)?;
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_generate_chart() {
        let results = vec![
            AnalysisResult {
                code: "600519".to_string(),
                name: "贵州茅台".to_string(),
                sentiment_score: 85,
                operation_advice: "买入".to_string(),
                trend_prediction: "看多".to_string(),
                analysis_content: "测试".to_string(),
                pe_ratio: Some(30.0),
                pb_ratio: Some(10.0),
                turnover_rate: Some(2.0),
                market_cap: Some(20000.0),
                circulating_cap: Some(20000.0),
            },
            AnalysisResult {
                code: "000858".to_string(),
                name: "五粮液".to_string(),
                sentiment_score: 75,
                operation_advice: "持有".to_string(),
                trend_prediction: "震荡".to_string(),
                analysis_content: "测试".to_string(),
                pe_ratio: Some(25.0),
                pb_ratio: Some(8.0),
                turnover_rate: Some(1.5),
                market_cap: Some(15000.0),
                circulating_cap: Some(15000.0),
            },
        ];
        
        let result = ChartGenerator::generate_summary_chart(&results, "test_chart.png");
        assert!(result.is_ok());
    }
}
