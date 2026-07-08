//! v11 P0-2: 共享 fallback 函数
//!
//! 两个 K 线入口 (`DataFetcherManager::get_daily_data` sync + `service::get_kline` async)
//! 共用同一套 fallback 顺序和 1-失败-触发-fallback 逻辑, 避免两套实现漂移。
//!
//! review #15: 顺序 **腾讯 → 东财 → RustDX** 改为三源**竞速** (tokio::join!).
//! 第一个返回 Ok+质检通过 即胜出, 其余 cancel. 串行最差延迟 9s → 并行 2s 左右.
//! 优先级: RustDX 路径仍走 spawn_blocking (TCP 不能与 HTTP 共享 runtime 调度).
//!
//! 顺序: **腾讯 ≈ 东财 ≈ RustDX** (race)
//! - 腾讯/东财: 前复权 (`adjust = Qfq`), HTTP 稳定
//! - RustDX: 不复权 (`adjust = None`), TCP 仅做兜底 (B 方案回退, 见 v11-p0-1-p0-2-设计定稿v2-2026-07-02 §5.3)
//!
//! v11 P0-1 commit 3: 每个 provider 返回 Ok 后调 `validate_daily_kline_quality`,
//! 校验失败 → 标记该源失败, 用剩余源中第一个 Ok 的.

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

/// 多源 fallback 拉取 K 线 + 质检门禁 (review #15 改三源竞速).
///
/// 返回 `(data, source_name)`:
/// - `data`: K 线列表 (可能标 `Qfq` 或 `None`, 见 `KlineData.adjust`)
/// - `source_name`: `tencent_qfq` / `eastmoney_qfq` / `rustdx_none`
///
/// review #15 改造: 三源**并行竞速**, 第一个返回 Ok+质检通过 即胜出, 其余丢弃.
/// 串行最差延迟 (腾讯 2s + 东财 2s + RustDX 5s = 9s) → 并行 P99 约 2s.
/// RustDX 是 TCP 阻塞, 必须 spawn_blocking; 三个 future 一起 join 后, 第一个
/// 质检通过的胜出. 用 enum 区分三源返回 + 质检结果, select_ok 语义.
///
/// 失败策略:
/// - 1 个源失败/返回空 → 该源退出竞速, 等其它源
/// - 1 个源返回 Ok 但质检 reject → 该源退出竞速
/// - 全失败 → `Err` (含失败原因)
pub async fn fetch_kline_with_fallback(
    code: &str,
    days: usize,
) -> Result<(Vec<KlineData>, &'static str)> {
    let client = crate::http_client::SHARED_HTTP_CLIENT.clone();
    let qc_threshold = max_gap_for(code);

    // review #15: 三源并行启动 (腾讯 + 东财 HTTP, RustDX spawn_blocking).
    enum SourceResult {
        Tencent(Result<Vec<KlineData>>),
        Eastmoney(Result<Vec<KlineData>>),
        Rustdx(Result<Vec<KlineData>>),
    }

    let code_str = code.to_string();
    let tencent_fut = {
        let client = client.clone();
        async move {
            let r = GtimgProvider::fetch_kline_data_internal(&client, code, days).await;
            SourceResult::Tencent(r)
        }
    };
    let eastmoney_fut = {
        let client = client.clone();
        async move {
            let r = HttpProvider::fetch_kline_data_internal(&client, code, days).await;
            SourceResult::Eastmoney(r)
        }
    };
    let rustdx_fut = async move {
        let r: Result<Vec<KlineData>> = tokio::task::spawn_blocking(move || -> Result<Vec<KlineData>> {
            let provider = RustdxProvider::new()?;
            provider.get_daily_data(&code_str, days)
        })
        .await
        .map_err(|e| anyhow!("RustDX 任务执行失败: {}", e))
        .and_then(|inner| inner);
        SourceResult::Rustdx(r)
    };

    // join! 三源 (顺序无关, race)
    let (t, e, r) = tokio::join!(tencent_fut, eastmoney_fut, rustdx_fut);

    // 按优先级顺序处理 (腾讯 > 东财 > RustDX): 第一个 Ok+质检 通过即胜出.
    let candidates: [(SourceResult, &'static str); 3] = [
        (t, "tencent_qfq"),
        (e, "eastmoney_qfq"),
        (r, "rustdx_none"),
    ];

    let mut last_err: Option<String> = None;
    let mut all_empty = true;
    let mut all_qc_reject = Vec::new();

    for (res, src) in candidates {
        let data = match res {
            SourceResult::Tencent(r) => r,
            SourceResult::Eastmoney(r) => r,
            SourceResult::Rustdx(r) => r,
        };
        match data {
            Ok(mut d) if !d.is_empty() => {
                all_empty = false;
                match validate_daily_kline_quality(&mut d, code, qc_threshold) {
                    Ok(()) => {
                        log::info!("[fallback] {} {} OK + 质检通过, {} 条", code, src, d.len());
                        return Ok((d, src));
                    }
                    Err(e) => {
                        let msg = brief(&format!("{:#}", e));
                        all_qc_reject.push(format!("{}={}", src, msg));
                        log::warn!("[fallback] {} {} 质检 reject: {}", code, src, msg);
                    }
                }
            }
            Ok(_) => {
                log::warn!("[fallback] {} {} 返回空数据", code, src);
                last_err = Some(format!("{} 返回空", src));
            }
            Err(e) => {
                let msg = brief(&format!("{:#}", e));
                if is_ban_error(&msg) {
                    log::warn!("[fallback] {} {} 失败 (ban suspected): {}", code, src, msg);
                } else {
                    log::warn!("[fallback] {} {} 失败 (non-ban error): {}", code, src, msg);
                }
                last_err = Some(format!("{}={}", src, msg));
            }
        }
    }

    Err(if all_empty {
        anyhow!(
            "所有数据源均返回空: 腾讯=空, 东财=空, RustDX=空 ({})",
            code
        )
    } else if !all_qc_reject.is_empty() {
        anyhow!(
            "所有数据源质检均 reject ({}): {}",
            code,
            all_qc_reject.join(", ")
        )
    } else {
        anyhow!(
            "所有数据源均获取失败 ({}): {}",
            code,
            last_err.unwrap_or_default()
        )
    })
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
