//! v11 P0-2: 共享 fallback 函数
//!
//! 两个 K 线入口 (`DataFetcherManager::get_daily_data` sync + `service::get_kline` async)
//! 共用同一套 fallback 顺序和 1-失败-触发-fallback 逻辑, 避免两套实现漂移。
//!
//! 顺序: **腾讯 → 东财 → RustDX**
//! - 腾讯/东财: 前复权 (`adjust = Qfq`), HTTP 稳定, 刚修过盘中价
//! - RustDX: 不复权 (`adjust = None`), TCP 仅做兜底 (B 方案回退, 见 v11-p0-1-p0-2-设计定稿v2-2026-07-02 §5.3)
//!
//! v11 P0-1 commit 3: 每个 provider 返回 Ok 后调 `validate_daily_kline_quality`,
//! 校验失败 → 自动 fallback 下一个源 (单条 skip 在质检内部处理, 整批 reject 触发 fallback).

use anyhow::{anyhow, Result};

use crate::data_provider::{DataProvider, GtimgProvider, HttpProvider, KlineData, RustdxProvider, is_ban_error};
use crate::monitor::data_quality::{max_gap_for, validate_daily_kline_quality};

/// 截断超长错误信息, 避免日志刷屏 (reqwest 错误会内嵌完整 URL)。
fn brief(s: &str) -> String {
    const MAX: usize = 120;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let head: String = s.chars().take(MAX).collect();
        format!("{head}…(截断)")
    }
}

/// 多源 fallback 拉取 K 线 + 质检门禁。
///
/// 返回 `(data, source_name)`:
/// - `data`: K 线列表 (可能标 `Qfq` 或 `None`, 见 `KlineData.adjust`)
/// - `source_name`: `tencent_qfq` / `eastmoney_qfq` / `rustdx_none`
///
/// 失败策略:
/// - 1 个源失败/返回空 → 立即 fallback 下一个
/// - 1 个源返回 Ok 但质检 reject (跳空超阈值) → 立即 fallback 下一个
/// - 全失败 → `Err` (含失败原因)
pub async fn fetch_kline_with_fallback(
    code: &str,
    days: usize,
) -> Result<(Vec<KlineData>, &'static str)> {
    let client = crate::http_client::SHARED_HTTP_CLIENT.clone();
    let qc_threshold = max_gap_for(code);

    // 主源: 腾讯
    match GtimgProvider::fetch_kline_data_internal(&client, code, days).await {
        Ok(mut data) if !data.is_empty() => match validate_daily_kline_quality(&mut data, code, qc_threshold) {
            Ok(()) => {
                log::info!("[fallback] {} 腾讯 OK + 质检通过, {} 条", code, data.len());
                return Ok((data, "tencent_qfq"));
            }
            Err(e) => log::warn!(
                "[fallback] {} 腾讯质检 reject, 触发 fallback: {}",
                code,
                brief(&e)
            ),
        },
        Ok(_) => log::warn!("[fallback] {} 腾讯返回空数据", code),
        Err(e) => {
            // Codex review P1 #4 修复: 区分 ban/non-ban, 运维一眼看出"切代理 vs 临时错误"
            let msg = brief(&format!("{:#}", e));
            if is_ban_error(&msg) {
                log::warn!("[fallback] {} 腾讯失败 (ban suspected) → 回落到东财: {}", code, msg);
            } else {
                log::warn!("[fallback] {} 腾讯失败 (non-ban error) → 回落到东财: {}", code, msg);
            }
        }
    }

    // Fallback 1: 东财
    match HttpProvider::fetch_kline_data_internal(&client, code, days).await {
        Ok(mut data) if !data.is_empty() => match validate_daily_kline_quality(&mut data, code, qc_threshold) {
            Ok(()) => {
                log::info!("[fallback] {} 东财 OK + 质检通过, {} 条", code, data.len());
                return Ok((data, "eastmoney_qfq"));
            }
            Err(e) => log::warn!(
                "[fallback] {} 东财质检 reject, 触发 fallback: {}",
                code,
                brief(&e)
            ),
        },
        Ok(_) => log::warn!("[fallback] {} 东财返回空数据", code),
        Err(e) => {
            let msg = brief(&format!("{:#}", e));
            if is_ban_error(&msg) {
                log::warn!("[fallback] {} 东财失败 (ban suspected) → 回落到RustDX: {}", code, msg);
            } else {
                log::warn!("[fallback] {} 东财失败 (non-ban error) → 回落到RustDX: {}", code, msg);
            }
        }
    }

    // Fallback 2: RustDX (TCP, 仅兜底, 不复权)
    let rustdx_code = code.to_string();
    let rustdx_result = tokio::task::spawn_blocking(move || -> Result<Vec<KlineData>> {
        let provider = RustdxProvider::new()?;
        provider.get_daily_data(&rustdx_code, days)
    })
    .await
    .map_err(|e| anyhow!("RustDX 任务执行失败: {}", e))?;

    match rustdx_result {
        Ok(mut data) if !data.is_empty() => {
            match validate_daily_kline_quality(&mut data, code, qc_threshold) {
                Ok(()) => {
                    log::info!("[fallback] {} RustDX OK + 质检通过, {} 条", code, data.len());
                    Ok((data, "rustdx_none"))
                }
                Err(e) => Err(anyhow!(
                    "RustDX 质检 reject ({code}): {e}",
                    code = code
                )),
            }
        }
        Ok(_) => Err(anyhow!(
            "所有数据源均返回空: 腾讯=空, 东财=空, RustDX=空 ({})",
            code
        )),
        Err(e) => Err(anyhow!(
            "所有数据源均获取失败 ({code}): 腾讯/东财 失败, RustDX={e}",
            code = code
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// brief 截断函数: 短字符串原样返回
    #[test]
    fn brief_short_unchanged() {
        let s = "hello world".to_string();
        assert_eq!(brief(&s), "hello world");
    }

    /// brief 截断函数: 长字符串截断到 120 字符 + 截断标记
    #[test]
    fn brief_long_truncated() {
        let s = "x".repeat(200);
        let out = brief(&s);
        assert!(out.ends_with("…(截断)"), "long string should be truncated");
        assert_eq!(out.chars().count(), 120 + "…(截断)".chars().count());
    }
}
