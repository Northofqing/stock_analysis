use plotters::prelude::*;
use anyhow::Result;
use std::path::PathBuf;
use crate::pipeline::AnalysisResult;

pub struct ChartGenerator;

impl ChartGenerator {
    /// 生成分析结果汇总图表（简化版本）
    pub fn generate_summary_chart(
        results: &[AnalysisResult],
        output_path: &str,
    ) -> Result<PathBuf> {
        let path_buf = PathBuf::from(output_path);
        let path_str = path_buf.to_str().ok_or_else(|| anyhow::anyhow!("Invalid path"))?;
        
        // 按评分排序（索引排序），取前15只
        let mut indices: Vec<usize> = (0..results.len()).collect();
        indices.sort_by(|&a, &b| results[b].sentiment_score.cmp(&results[a].sentiment_score));
        indices.truncate(15);
        let display_results: Vec<&AnalysisResult> = indices.iter().map(|&i| &results[i]).collect();
        
        {
            let root = BitMapBackend::new(path_str, (1400, 900)).into_drawing_area();
            root.fill(&WHITE)?;
            
            let areas = root.split_evenly((2, 1));
            
            // 上半部分：评分柱状图
            Self::draw_score_bar_chart(&areas[0], &display_results)?;
            
            // 下半部分：操作建议统计
            Self::draw_operation_stats(&areas[1], results)?;
            
            root.present()?;
        }
        
        Ok(path_buf)
    }
    
    /// 绘制评分柱状图（简化版）
    fn draw_score_bar_chart(
        area: &DrawingArea<BitMapBackend, plotters::coord::Shift>,
        results: &[&AnalysisResult],
    ) -> Result<()> {
        if results.is_empty() {
            return Ok(());
        }
        
        let max_score = results.iter().map(|r| r.sentiment_score).max().unwrap_or(100);
        
        let mut chart = ChartBuilder::on(area)
            .caption("股票评分 TOP15", ("sans-serif", 30))
            .margin(15)
            .x_label_area_size(100)
            .y_label_area_size(50)
            .build_cartesian_2d(
                0..(results.len()),
                0..(max_score + 10),
            )?;
        
        chart
            .configure_mesh()
            .y_desc("评分")
            .y_labels(10)
            .x_labels(results.len())
            .x_label_formatter(&|x| {
                if *x < results.len() {
                    format!("{}", results[*x].code)
                } else {
                    String::new()
                }
            })
            .draw()?;
        
        // 绘制柱状图
        for (i, result) in results.iter().enumerate() {
            let color = if result.sentiment_score >= 80 {
                &GREEN
            } else if result.sentiment_score >= 60 {
                &BLUE
            } else if result.sentiment_score >= 40 {
                &YELLOW
            } else {
                &RED
            };
            
            chart.draw_series(std::iter::once(
                Rectangle::new(
                    [(i, 0), (i, result.sentiment_score)],
                    color.filled(),
                )
            ))?;
        }
        
        Ok(())
    }
    
    /// 绘制操作建议统计（使用文本和矩形块）
    fn draw_operation_stats(
        area: &DrawingArea<BitMapBackend, plotters::coord::Shift>,
        results: &[AnalysisResult],
    ) -> Result<()> {
        // 统计操作建议分布
        let mut buy_count = 0;
        let mut sell_count = 0;
        let mut hold_count = 0;
        
        for result in results {
            match result.operation_advice.as_str() {
                s if s.contains("买入") || s.contains("加仓") => buy_count += 1,
                s if s.contains("卖出") || s.contains("减仓") => sell_count += 1,
                _ => hold_count += 1,
            }
        }
        
        let total = results.len() as f64;
        if total == 0.0 {
            return Ok(());
        }
        
        // 绘制标题
        area.draw(&Text::new(
            "操作建议分布",
            (600, 50),
            ("sans-serif", 30).into_font(),
        ))?;
        
        // 绘制统计条
        let stats = vec![
            ("买入/加仓", buy_count, &GREEN),
            ("持有/观望", hold_count, &BLUE),
            ("卖出/减仓", sell_count, &RED),
        ];
        
        let bar_height = 60i32;
        let start_y = 150i32;
        let max_width = 1000;
        
        for (idx, (label, count, color)) in stats.iter().enumerate() {
            let y = start_y + (idx as i32 * (bar_height + 40));
            let percentage = *count as f64 / total * 100.0;
            let bar_width = (max_width as f64 * (*count as f64 / total)) as i32;
            
            // 绘制彩色条
            area.draw(&Rectangle::new(
                [(200, y), (200 + bar_width, y + bar_height)],
                color.filled(),
            ))?;
            
            // 绘制标签
            area.draw(&Text::new(
                format!("{}: {} ({:.1}%)", label, count, percentage),
                (220, y + 20),
                ("sans-serif", 20).into_font(),
            ))?;
        }
        
        Ok(())
    }
}
