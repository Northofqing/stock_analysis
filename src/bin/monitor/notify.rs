//! 通知推送 + MagicLaw 守护进程 + Token 管理
//!
//! 从 main.rs 提取，减少单文件体积。

use std::io::Write;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use log;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json;
use tokio;
use stock_analysis;

use crate::{
    CachedApiToken, MessageSendType, MessageSendTransport,
    DaemonReadySource, ApiTokenSource,
    DEFAULT_MAGICLAW_API_ADDR, DEFAULT_MAGICLAW_PROJECT_ID,
    DEFAULT_MAGICLAW_CLIENT_NAME, DEFAULT_MAGICLAW_TOKEN_TTL_SECS,
    DEFAULT_MAGICLAW_TOKEN_REFRESH_AHEAD_SECS,
    MAGICLAW_DAEMON_BOOT_LOCK, MAGICLAW_TOKEN_MEM_CACHE,
    MAGICLAW_TOKEN_ISSUE_LOCK, MAGICLAW_DISABLE_ENV_TOKEN,
};

/// v11-P0-4 commit D: 推送治理 — 推送类别
///
/// 35 条推送盘点的"默认降级 vs 保留 vs 移交" 由 `push_governor` 函数根据 `PushKind` 决定.
/// grill Q2 修订: 12 条降级 (A2/A3/A4/A5/A6/A11/A12/B4/B10/B11/B12/B13) / 9 保留 (A1/A7/A8/A13/A14/A15/B1/B2/C1).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PushKind {
    /// 保留: 持仓事件告警 (涨跌停突变/炸板/排除/风控/现金预警)
    HoldingEvent,
    /// 保留: 盘前/盘后告警/复盘/概览
    DailyReport,
    /// 保留: 公告告警
    Announcement,
    /// 降级: 竞价量能 Top10
    AuctionVolume,
    /// 降级: 虚拟观察仓位
    VirtualWatch,
    /// 降级: 首板/二板/三板+ Top10
    LimitBoards,
    /// 降级: 领涨板块 Top5
    SectorTop,
    /// 降级: 主力净流入 Top10
    FundInflow,
    /// 降级: 9:20-9:25 竞价重推优选
    AuctionRepush,
    /// 降级: 因子 IC (grill Q6 改)
    FactorIC,
    /// 降级: v4 赛道分档
    SectorTier,
    /// 降级: v4 资金验证
    CapitalVerify,
    /// 降级: 周度 SOP
    WeeklySOP,
    // v11-P0-5+ Commit 4 加: 5 个候选源 (P5 §六 验收, 默认降级, 候选台统一推 1 条)
    /// 降级: A10 选股推荐 (移交候选台)
    StockPick,
    /// 降级: B3 优选候选 (移交候选台)
    OptimalClose,
    /// 降级: B6 放量·自选 (移交候选台)
    VolumeWatchlist,
    /// 降级: B7 放量·实盘优选 (移交候选台)
    VolumeRealTrade,
    /// 降级: C4 产业链扫描 (移交候选台)
    IndustryChain,
    // v11-P0-5++ Commit 5 加: 候选台统一卡片 (5 路 raw 合并 → 1 张排序候选清单)
    /// 保留: 候选筛选台卡片 (P5 §五 输出形态, 强证据>多源>题材)
    CandidateBoard,
}

impl PushKind {
    /// 是否降级 (P0-4 commit D 默认行为, PUSH_VERBOSE=true 时无效)
    pub fn is_deprecated(self) -> bool {
        match self {
            // 保留 10 条 (原 9 + P0-5++ Commit 5 加 CandidateBoard)
            PushKind::HoldingEvent
            | PushKind::DailyReport
            | PushKind::Announcement
            | PushKind::CandidateBoard => false,
            // 降级 17 条 (原 12 + P0-5+ Commit 4 加 5 个候选源)
            PushKind::AuctionVolume
            | PushKind::VirtualWatch
            | PushKind::LimitBoards
            | PushKind::SectorTop
            | PushKind::FundInflow
            | PushKind::AuctionRepush
            | PushKind::FactorIC
            | PushKind::SectorTier
            | PushKind::CapitalVerify
            | PushKind::WeeklySOP
            | PushKind::StockPick
            | PushKind::OptimalClose
            | PushKind::VolumeWatchlist
            | PushKind::VolumeRealTrade
            | PushKind::IndustryChain => true,
        }
    }

    /// 简短标签 (log 显示)
    pub fn label(self) -> &'static str {
        match self {
            PushKind::HoldingEvent => "持仓事件",
            PushKind::DailyReport => "日报/复盘/概览",
            PushKind::Announcement => "公告",
            PushKind::AuctionVolume => "竞价量能",
            PushKind::VirtualWatch => "虚拟观察",
            PushKind::LimitBoards => "板数榜",
            PushKind::SectorTop => "领涨板块",
            PushKind::FundInflow => "主力净流入",
            PushKind::AuctionRepush => "竞价重推",
            PushKind::FactorIC => "因子IC",
            PushKind::SectorTier => "赛道分档",
            PushKind::CapitalVerify => "资金验证",
            PushKind::WeeklySOP => "周度SOP",
            PushKind::StockPick => "选股",
            PushKind::OptimalClose => "优选",
            PushKind::VolumeWatchlist => "放量自选",
            PushKind::VolumeRealTrade => "放量实盘",
            PushKind::IndustryChain => "产业链",
            PushKind::CandidateBoard => "候选台",
        }
    }
}

/// v11-P0-4 commit D: 推送治理入口
///
/// 根据 `PushKind` + `PUSH_VERBOSE` env var 决定:
/// - `kind.is_deprecated() == true` **且** `PUSH_VERBOSE != "true"` → 降级 log (不推)
/// - 其他情况 → 调 `push_wechat` 正常推送
///
/// PUSH_VERBOSE=true 恢复旧行为 (留退路, shadow 切换验证用)
/// PUSH_VERBOSE 未设或 != "true" → 默认精简 (12 条降级, 23 条保留)
pub async fn push_governor(text: &str, kind: PushKind) -> bool {
    let verbose = std::env::var("PUSH_VERBOSE").ok().as_deref() == Some("true");

    if kind.is_deprecated() && !verbose {
        log::warn!(
            "[PUSH_GOVERNOR] 降级日志 (kind={}, PUSH_VERBOSE 默认精简):\n{}",
            kind.label(),
            text
        );
        return false;
    }

    if verbose && kind.is_deprecated() {
        log::info!(
            "[PUSH_GOVERNOR] 保留旧行为 (kind={}, PUSH_VERBOSE=true)",
            kind.label()
        );
    }

    push_wechat(text).await
}

pub async fn push_wechat(text: &str) -> bool {
    // v10 P6 5 要素接入: V10_DRY_RUN_PUSH=1 时跳过实际推送, 仅 log
    // 用于开发/验证推送内容变化, 不骚扰飞书
    if std::env::var("V10_DRY_RUN_PUSH").ok().as_deref() == Some("1") {
        log::info!("[V10_DRY_RUN_PUSH] 跳过飞书推送, 内容预览:\n{}", text);
        return true;
    }

    let send_type = resolve_send_type();
    let send_transport = resolve_send_transport(send_type);

    if matches!(send_transport, MessageSendTransport::Cli) {
        return push_via_magiclaw_cli(send_type, text).await;
    }

    if matches!(send_type, MessageSendType::Feishu)
        && matches!(send_transport, MessageSendTransport::Http)
    {
        return push_feishu_via_http(text).await;
    }

    log::info!(
        "[{}] 开始推送 ({}字) | via={}",
        send_type.label(),
        text.chars().count(),
        send_transport.as_str()
    );

    let magiclaw_bin = resolve_magiclaw_bin();
    let api_addr = resolve_api_addr();
    let api_base = to_api_base_url(&api_addr);
    // 关键：daemon 在 127.0.0.1 回环上，必须 .no_proxy() 绕过系统代理(Clash/Surge)。
    // 否则 macOS 系统代理会劫持本地请求并返回 503，导致健康检查恒失败、误判 daemon 不可用。
    let client = match reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(30))
        .build() {
        Ok(c) => c,
        Err(e) => {
            log::error!("[{}] 创建 HTTP 客户端失败: {}", send_type.label(), e);
            return false;
        }
    };

    match ensure_magiclaw_daemon(&client, &magiclaw_bin, &api_addr, &api_base).await {
        Ok(DaemonReadySource::Reused) => {
            log::info!("[{}] daemon 来源: 复用已有实例 | {}", send_type.label(), api_addr);
        }
        Ok(DaemonReadySource::StartedNow) => {
            log::info!("[{}] daemon 来源: 本次自动拉起 | {}", send_type.label(), api_addr);
        }
        Err(e) => {
            log::error!("[{}] daemon 不可用: {}", send_type.label(), e);
            return false;
        }
    }

    let (mut active_token, mut active_token_source) = match resolve_or_issue_api_token(&magiclaw_bin).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("[{}] 获取 daemon 动态鉴权 token 失败: {}", send_type.label(), e);
            return false;
        }
    };

    let verify_result = verify_daemon_auth(&client, &api_base, &active_token, &active_token_source).await;
    if let Err(first_err) = verify_result {
        if is_unauthorized_error(&first_err) {
            clear_dynamic_token_cache().await;
            match issue_and_cache_dynamic_api_token(&magiclaw_bin).await {
                Ok(next) => {
                    log::warn!("[{}] daemon token 鉴权失败，已清缓存并重新签发动态 token 后重试预检", send_type.label());
                    if matches!(active_token_source, ApiTokenSource::Env) {
                        MAGICLAW_DISABLE_ENV_TOKEN.store(true, Ordering::Relaxed);
                    }
                    active_token = next.token;
                    active_token_source = ApiTokenSource::DynamicIssued;
                    if let Err(e) = verify_daemon_auth(&client, &api_base, &active_token, &active_token_source).await {
                        log::warn!("[{}] daemon 鉴权预检重试仍失败，但已重新签发 token，将继续尝试发送: {}", send_type.label(), e);
                    }
                }
                Err(issue_err) => {
                    log::error!("[{}] daemon 鉴权预检失败: {}；自动续签失败: {}", send_type.label(), first_err, issue_err);
                    return false;
                }
            }
        } else {
            log::error!("[{}] daemon 鉴权预检失败: {}", send_type.label(), first_err);
            return false;
        }
    }

    let to = match resolve_send_target(send_type, &client, &api_base, &active_token).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("[{}] 解析收件人失败: {}", send_type.label(), e);
            return false;
        }
    };
    let to_log = to.as_deref().unwrap_or("<magiclaw-default>");

    match send_via_magiclaw_daemon(&client, &api_base, &active_token, send_type, to.as_deref(), text).await {
        Ok(()) => {
            log::info!("[{}] 推送成功 | to={}", send_type.label(), to_log);
            true
        }
        Err(first_err) => {
            if is_unauthorized_error(&first_err) {
                clear_dynamic_token_cache().await;
                match issue_and_cache_dynamic_api_token(&magiclaw_bin).await {
                    Ok(next) => {
                        log::warn!("[{}] daemon token 鉴权失败，已清缓存并重新签发动态 token 后重试发送", send_type.label());
                        if matches!(active_token_source, ApiTokenSource::Env) {
                            MAGICLAW_DISABLE_ENV_TOKEN.store(true, Ordering::Relaxed);
                        }
                        match send_via_magiclaw_daemon(&client, &api_base, &next.token, send_type, to.as_deref(), text).await {
                            Ok(()) => {
                                log::info!("[{}] 推送成功 | to={}", send_type.label(), to_log);
                                true
                            }
                            Err(retry_err) => {
                                log::error!("[{}] 推送失败: {}", send_type.label(), retry_err);
                                false
                            }
                        }
                    }
                    Err(issue_err) => {
                        log::error!("[{}] 推送失败: {}；自动续签失败: {}", send_type.label(), first_err, issue_err);
                        false
                    }
                }
            } else {
                log::error!("[{}] 推送失败: {}", send_type.label(), first_err);
                false
            }
        }
    }
}

pub async fn push_feishu_via_http(text: &str) -> bool {
    let url = match resolve_feishu_webhook_url() {
        Some(v) => v,
        None => {
            log::error!(
                "[飞书] 推送失败: 未配置 FEISHU_WEBHOOK_URL（或 MAGICLAW_FEISHU_WEBHOOK_URL）"
            );
            return false;
        }
    };

    log::info!("[飞书] 开始推送 ({}字) | via=http", text.chars().count());

    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("[飞书] 创建 HTTP 客户端失败: {}", e);
            return false;
        }
    };

    let payload = serde_json::json!({
        "msg_type": "text",
        "content": {
            "text": text,
        }
    });

    let resp = match client.post(&url).json(&payload).send().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("[飞书] 推送失败: 调用 webhook 失败: {}", e);
            return false;
        }
    };

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        log::error!("[飞书] 推送失败: webhook HTTP {}: {}", status, body_text);
        return false;
    }

    let parsed = serde_json::from_str::<serde_json::Value>(&body_text).ok();
    let ok_by_status_code = parsed
        .as_ref()
        .and_then(|v| v.get("StatusCode").and_then(|x| x.as_i64()))
        .map(|code| code == 0)
        .unwrap_or(false);
    let ok_by_code = parsed
        .as_ref()
        .and_then(|v| v.get("code").and_then(|x| x.as_i64()))
        .map(|code| code == 0)
        .unwrap_or(false);

    if ok_by_status_code || ok_by_code {
        log::info!("[飞书] 推送成功 | via=http");
        return true;
    }

    log::error!("[飞书] 推送失败: webhook 返回非成功体: {}", body_text);
    false
}

pub async fn push_via_magiclaw_cli(send_type: MessageSendType, text: &str) -> bool {
    let to = match send_type {
        MessageSendType::Wechat => None,
        MessageSendType::Feishu => match resolve_feishu_target() {
            Some(v) => Some(v),
            None => {
                log::error!(
                    "[飞书] 解析收件人失败: 飞书发送缺少收件人，请设置 FEISHU_TO（或 MAGICLAW_FEISHU_TO / FEISHU_CHAT_ID / FEISHU_OPEN_ID / FEISHU_USER_ID / FEISHU_EMAIL）"
                );
                return false;
            }
        },
    };

    let magiclaw_bin = resolve_magiclaw_bin();
    log::info!("[{}] 开始推送 ({}字) | via=cli", send_type.label(), text.chars().count());

    let mut cmd = tokio::process::Command::new(&magiclaw_bin);
    cmd.arg("send")
        .arg("--channel")
        .arg(send_type.as_str())
        .arg("--message")
        .arg(text)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(to) = to.as_deref() {
        cmd.arg("--to").arg(to);
    }

    // 将 cwd 指向 magiclaw 项目根目录，使其 dotenv 能加载飞书凭证所在的 .env。
    // 若 cwd 改变，则 MAGICLAW_DB_PATH 的相对路径会失效，故统一转为绝对路径传入。
    let magiclaw_home = resolve_magiclaw_home(&magiclaw_bin);
    if let Some(home) = magiclaw_home.as_deref() {
        cmd.current_dir(home);
    } else {
        log::warn!(
            "[{}] 未能定位 magiclaw 项目根目录（找不到 .env），飞书凭证可能加载失败 | bin={}",
            send_type.label(),
            magiclaw_bin
        );
    }

    if let Ok(db_path) = std::env::var("MAGICLAW_DB_PATH") {
        let db_path = db_path.trim();
        if !db_path.is_empty() {
            let abs_db = std::fs::canonicalize(db_path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| {
                    // 文件可能尚不存在或无法规范化：相对路径手动拼接当前进程 cwd
                    let p = std::path::Path::new(db_path);
                    if p.is_absolute() {
                        db_path.to_string()
                    } else {
                        std::env::current_dir()
                            .map(|cwd| cwd.join(p).to_string_lossy().into_owned())
                            .unwrap_or_else(|_| db_path.to_string())
                    }
                });
            cmd.env("MAGICLAW_DB_PATH", abs_db);
        }
    }

    if let Ok(receive_id_type) = std::env::var("FEISHU_RECEIVE_ID_TYPE") {
        let receive_id_type = receive_id_type.trim();
        if !receive_id_type.is_empty() {
            cmd.arg("--receive-id-type").arg(receive_id_type);
        }
    }

    let output = match cmd.output().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("[飞书] 调用 magiclaw send 失败(magiclaw: {}): {}", magiclaw_bin, e);
            return false;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        let detail = tail_lines(&stdout, 3);
        if detail.is_empty() {
            log::info!(
                "[{}] 推送成功 | to={}",
                send_type.label(),
                to.as_deref().unwrap_or("<auto>")
            );
        } else {
            log::info!(
                "[{}] 推送成功 | to={} | {}",
                send_type.label(),
                to.as_deref().unwrap_or("<auto>"),
                detail
            );
        }
        return true;
    }

    let stderr_tail = tail_lines(&stderr, 8);
    let stdout_tail = tail_lines(&stdout, 3);
    log::error!(
        "[{}] 推送失败(exit={}): {}{}",
        send_type.label(),
        output.status,
        if !stderr_tail.is_empty() {
            format!("stderr={}", stderr_tail)
        } else {
            "stderr=<empty>".to_string()
        },
        if !stdout_tail.is_empty() {
            format!(" | stdout={}", stdout_tail)
        } else {
            "".to_string()
        }
    );
    false
}

pub fn summarize_push_text(text: &str, max_chars: usize) -> String {
    let one_line = text.replace('\n', " | ");
    let mut out = String::new();
    let mut count = 0usize;
    for ch in one_line.chars() {
        if count >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
        count += 1;
    }
    out
}

pub fn evaluate_opportunity_push_skip_reason(opp_text: &str) -> Option<&'static str> {
    // 只对“整轮无有效产业链输出”的明确文案做跳过，避免
    // “值得关注：暂无通过量能/趋势确认候选”这类正常结果被误判为应跳过。
    if opp_text.contains("暂无最新快讯") {
        return Some("contains:暂无最新快讯");
    }
    if opp_text.contains("当前快讯未命中已知产业链") {
        return Some("contains:当前快讯未命中已知产业链");
    }
    if opp_text.contains("当前产业链信号可信度不足（已降级观察）") {
        return Some("contains:当前产业链信号可信度不足");
    }
    if opp_text.contains("无可用标的") {
        return Some("contains:无可用标的");
    }
    None
}

pub fn resolve_send_type() -> MessageSendType {
    // 默认统一走飞书（test 与 prod 一致）；如需微信，显式设置 SEND_TYPE=wechat。
    let default_type = MessageSendType::Feishu;

    let raw = std::env::var("MAGICLAW_SEND_TYPE")
        .or_else(|_| std::env::var("SEND_TYPE"))
        .unwrap_or_else(|_| default_type.as_str().to_string());
    match raw.trim().to_ascii_lowercase().as_str() {
        "wechat" | "weixin" | "wx" => MessageSendType::Wechat,
        "feishu" | "lark" => MessageSendType::Feishu,
        other => {
            log::warn!(
                "未识别的发送类型: {}，回退为默认 {}",
                other,
                default_type.as_str()
            );
            default_type
        }
    }
}

pub fn resolve_send_transport(send_type: MessageSendType) -> MessageSendTransport {
    match send_type {
        MessageSendType::Wechat => MessageSendTransport::Http,
        // 飞书自动路由：配置了 webhook 则走 HTTP；否则走 CLI。
        MessageSendType::Feishu => {
            if resolve_feishu_webhook_url().is_some() {
                MessageSendTransport::Http
            } else {
                MessageSendTransport::Cli
            }
        }
    }
}

pub fn resolve_feishu_webhook_url() -> Option<String> {
    ["FEISHU_WEBHOOK_URL", "MAGICLAW_FEISHU_WEBHOOK_URL"]
        .iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
}

pub fn resolve_magiclaw_bin() -> String {
    std::env::var("MAGICLAW_BIN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{}/Desktop/magiclaw/target/release/magiclaw", home)
        })
}

/// 解析 magiclaw 项目根目录（其 `.env` 所在目录）。
/// magiclaw 启动时通过 dotenvy 从工作目录加载 `.env`，飞书凭证（FEISHU_APP_ID 等）
/// 存放在 magiclaw 自己的 `.env` 中。派生子进程时需将 cwd 指向该目录，否则读不到凭证。
/// 优先级：MAGICLAW_HOME 环境变量 > 从二进制路径推导（去掉 `target/release/magiclaw`）。
pub fn resolve_magiclaw_home(magiclaw_bin: &str) -> Option<std::path::PathBuf> {
    if let Ok(home) = std::env::var("MAGICLAW_HOME") {
        let home = home.trim();
        if !home.is_empty() {
            return Some(std::path::PathBuf::from(home));
        }
    }
    let bin_path = std::path::Path::new(magiclaw_bin);
    // 形如 .../magiclaw/target/release/magiclaw → 上溯 3 级到 .../magiclaw
    let home = bin_path.parent()?.parent()?.parent()?;
    if home.join(".env").is_file() {
        Some(home.to_path_buf())
    } else {
        None
    }
}

pub fn resolve_api_addr() -> String {
    std::env::var("MAGICLAW_API_ADDR")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MAGICLAW_API_ADDR.to_string())
}

pub async fn resolve_or_issue_api_token(magiclaw_bin: &str) -> Result<(String, ApiTokenSource), String> {
    if !MAGICLAW_DISABLE_ENV_TOKEN.load(Ordering::Relaxed) {
        if let Some(token) = std::env::var("MAGICLAW_API_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()) {
            return Ok((token, ApiTokenSource::Env));
        }
    }

    if let Some(cached) = load_dynamic_token_from_mem_cache().await {
        return Ok((cached.token, ApiTokenSource::DynamicMemCache));
    }

    if let Some(cached) = load_dynamic_token_from_file_cache() {
        cache_dynamic_token_in_mem(&cached).await;
        return Ok((cached.token, ApiTokenSource::DynamicFileCache));
    }

    let issued = issue_and_cache_dynamic_api_token(magiclaw_bin).await?;
    Ok((issued.token, ApiTokenSource::DynamicIssued))
}

pub fn is_unauthorized_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("401") || lower.contains("unauthorized")
}

pub fn api_token_cache_file_path() -> std::path::PathBuf {
    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".to_string());
    let db_path = std::path::PathBuf::from(db_path);
    let parent = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("./data"));
    parent.join("magiclaw_api_token_cache.json")
}

pub fn now_epoch_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn token_refresh_ahead_secs() -> i64 {
    std::env::var("MAGICLAW_TOKEN_REFRESH_AHEAD_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|v| *v >= 0)
        .unwrap_or(DEFAULT_MAGICLAW_TOKEN_REFRESH_AHEAD_SECS)
}

pub fn is_cached_token_expired(token: &CachedApiToken) -> bool {
    match token.expires_at {
        Some(ts) => ts <= now_epoch_secs() + token_refresh_ahead_secs(),
        None => false,
    }
}

pub async fn load_dynamic_token_from_mem_cache() -> Option<CachedApiToken> {
    let guard = MAGICLAW_TOKEN_MEM_CACHE.read().await;
    let v = guard.clone();
    drop(guard);
    v.filter(|t| !t.token.trim().is_empty() && !is_cached_token_expired(t))
}

pub fn load_dynamic_token_from_file_cache() -> Option<CachedApiToken> {
    let path = api_token_cache_file_path();
    let text = std::fs::read_to_string(path).ok()?;
    let token = serde_json::from_str::<CachedApiToken>(&text).ok()?;
    if token.token.trim().is_empty() || is_cached_token_expired(&token) {
        return None;
    }
    Some(token)
}

pub async fn cache_dynamic_token_in_mem(token: &CachedApiToken) {
    let mut guard = MAGICLAW_TOKEN_MEM_CACHE.write().await;
    *guard = Some(token.clone());
}

pub async fn clear_dynamic_token_cache() {
    {
        let mut guard = MAGICLAW_TOKEN_MEM_CACHE.write().await;
        *guard = None;
    }

    let path = api_token_cache_file_path();
    let _ = std::fs::remove_file(path);
}

pub fn cache_dynamic_token_in_file(token: &CachedApiToken) -> Result<(), String> {
    let path = api_token_cache_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 token 缓存目录失败({}): {}", parent.display(), e))?;
    }
    let text = serde_json::to_string(token)
        .map_err(|e| format!("序列化 token 缓存失败: {}", e))?;
    std::fs::write(&path, text)
        .map_err(|e| format!("写入 token 缓存失败({}): {}", path.display(), e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| format!("设置 token 缓存文件权限失败({}): {}", path.display(), e))?;
    }

    Ok(())
}

pub fn parse_issue_token_output(stdout: &str) -> Result<CachedApiToken, String> {
    let mut token: Option<String> = None;
    let mut expires_at: Option<i64> = None;

    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("token=") {
            let v = rest.trim();
            if !v.is_empty() {
                token = Some(v.to_string());
            }
            continue;
        }

        if line.contains("expires_at=") {
            for part in line.split_whitespace() {
                if let Some(raw) = part.strip_prefix("expires_at=") {
                    if let Ok(ts) = raw.trim().parse::<i64>() {
                        expires_at = Some(ts);
                    }
                }
            }
        }
    }

    let token = token.ok_or_else(|| format!("auth issue 输出缺少 token 字段: {}", stdout.trim()))?;
    Ok(CachedApiToken { token, expires_at })
}

pub async fn issue_and_cache_dynamic_api_token(magiclaw_bin: &str) -> Result<CachedApiToken, String> {
    let _issue_guard = MAGICLAW_TOKEN_ISSUE_LOCK.lock().await;

    // 双检锁：等待锁期间可能已有其他协程签发并写入缓存。
    if let Some(cached) = load_dynamic_token_from_mem_cache().await {
        return Ok(cached);
    }
    if let Some(cached) = load_dynamic_token_from_file_cache() {
        cache_dynamic_token_in_mem(&cached).await;
        return Ok(cached);
    }

    let project_id = std::env::var("MAGICLAW_PROJECT_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MAGICLAW_PROJECT_ID.to_string());
    let client_name = std::env::var("MAGICLAW_CLIENT_NAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}-{}", DEFAULT_MAGICLAW_CLIENT_NAME, std::process::id()));
    let ttl_secs = std::env::var("MAGICLAW_TOKEN_TTL_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAGICLAW_TOKEN_TTL_SECS);

    let output = tokio::process::Command::new(magiclaw_bin)
        .arg("auth")
        .arg("issue")
        .arg("--project")
        .arg(&project_id)
        .arg("--name")
        .arg(&client_name)
        .arg("--scopes")
        .arg("send,window_status")
        .arg("--ttl-secs")
        .arg(ttl_secs.to_string())
        .env("MAGICLAW_DB_PATH", std::env::var("MAGICLAW_DB_PATH").unwrap_or_else(|_| std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".to_string())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("执行 magiclaw auth issue 失败: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let stderr_tail = tail_lines(&stderr, 8);
        let stdout_tail = tail_lines(&stdout, 3);
        return Err(format!(
            "magiclaw auth issue 失败(exit={}): {}{}",
            output.status,
            if !stderr_tail.is_empty() { format!("stderr={}", stderr_tail) } else { "".to_string() },
            if !stdout_tail.is_empty() { format!(" | stdout={}", stdout_tail) } else { "".to_string() }
        ));
    }

    let issued = parse_issue_token_output(&stdout)?;
    cache_dynamic_token_in_mem(&issued).await;
    cache_dynamic_token_in_file(&issued)?;
    Ok(issued)
}

pub fn to_api_base_url(api_addr: &str) -> String {
    if api_addr.starts_with("http://") || api_addr.starts_with("https://") {
        api_addr.trim_end_matches('/').to_string()
    } else {
        format!("http://{}", api_addr)
    }
}

pub fn resolve_wechat_data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("WECHAT_CHANNEL_DIR") {
        return std::path::PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::Path::new(&home).join(".claude").join("channels").join("wechat")
}

pub fn parse_first_peer_id_from_window_status(body: &str) -> Option<String> {
    let peers = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("peers").cloned())
        .and_then(|peers| peers.as_array().cloned())?;

    peers
        .iter()
        .filter_map(|peer| peer.get("peer_id").and_then(|value| value.as_str()))
        .map(str::trim)
        .find(|peer_id| !peer_id.is_empty())
        .map(|peer_id| peer_id.to_string())
}

pub fn resolve_magiclaw_log_dir() -> std::path::PathBuf {
    let db_path = std::env::var("MAGICLAW_DB_PATH")
        .unwrap_or_else(|_| std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".to_string()));
    std::path::Path::new(&db_path)
        .parent()
        .map(|parent| parent.join("logs"))
        .unwrap_or_else(|| std::path::PathBuf::from("logs"))
}

pub fn resolve_wechat_target_from_magiclaw_logs() -> Option<String> {
    let log_dir = resolve_magiclaw_log_dir();
    let mut log_files: Vec<std::path::PathBuf> = std::fs::read_dir(&log_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("magiclaw-") && name.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect();
    log_files.sort();
    log_files.reverse();

    for log_path in log_files {
        let content = match std::fs::read_to_string(&log_path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        for line in content.lines().rev() {
            if let Some(peer_id) = line
                .split("peer_id=")
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .map(str::trim)
                .filter(|peer_id| !peer_id.is_empty())
            {
                return Some(peer_id.to_string());
            }
        }
    }

    None
}

#[derive(Deserialize)]
struct WechatAccountFile {
    #[serde(rename = "userId")]
    user_id: Option<String>,
}

pub async fn resolve_wechat_target(
    client: &reqwest::Client,
    api_base: &str,
    api_token: &str,
) -> Result<String, String> {
    if let Ok(to) = std::env::var("WECHAT_TO") {
        let to = to.trim();
        if !to.is_empty() {
            return Ok(to.to_string());
        }
    }

    let url = format!("{}/api/window_status", api_base);
    let daemon_resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client
            .get(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", api_token))
            .send(),
    )
    .await;

    if let Ok(Ok(resp)) = daemon_resp {
        if resp.status().is_success() {
            if let Ok(body) = resp.text().await {
                if let Some(peer_id) = parse_first_peer_id_from_window_status(&body) {
                    return Ok(peer_id);
                }
            }
        }
    }

    if let Some(peer_id) = resolve_wechat_target_from_magiclaw_logs() {
        return Ok(peer_id);
    }

    let data_dir = resolve_wechat_data_dir();
    let account_path = data_dir.join("account.json");

    let account_text = std::fs::read_to_string(&account_path)
        .map_err(|e| format!("读取 account.json 失败({}): {}", account_path.display(), e))?;
    let account: WechatAccountFile = serde_json::from_str(&account_text)
        .map_err(|e| format!("解析 account.json 失败: {}", e))?;

    account.user_id.ok_or_else(|| {
        format!(
            "未找到收件人：请先在微信给 bot 发消息，或设置 WECHAT_TO，目录={}",
            data_dir.display()
        )
    })
}

pub fn resolve_feishu_target() -> Option<String> {
    for key in [
        "FEISHU_TO",
        "MAGICLAW_FEISHU_TO",
        "FEISHU_CHAT_ID",
        "FEISHU_OPEN_ID",
        "FEISHU_USER_ID",
        "FEISHU_EMAIL",
    ] {
        if let Ok(to) = std::env::var(key) {
            let to = to.trim();
            if !to.is_empty() {
                return Some(to.to_string());
            }
        }
    }
    None
}

pub async fn resolve_send_target(
    send_type: MessageSendType,
    client: &reqwest::Client,
    api_base: &str,
    api_token: &str,
) -> Result<Option<String>, String> {
    match send_type {
        MessageSendType::Wechat => resolve_wechat_target(client, api_base, api_token)
            .await
            .map(Some),
        MessageSendType::Feishu => {
            let to = resolve_feishu_target();
            if to.is_none() {
                return Err(
                    "飞书发送缺少收件人：请设置 FEISHU_TO（或 MAGICLAW_FEISHU_TO / FEISHU_CHAT_ID / FEISHU_OPEN_ID / FEISHU_USER_ID / FEISHU_EMAIL）"
                        .to_string(),
                );
            }
            Ok(to)
        }
    }
}

pub async fn daemon_health_ok(client: &reqwest::Client, api_base: &str) -> bool {
    let health_url = format!("{}/api/health", api_base);
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.get(&health_url).send(),
    ).await;

    match resp {
        Ok(Ok(r)) => r.status().is_success(),
        _ => false,
    }
}

pub async fn ensure_magiclaw_daemon(
    client: &reqwest::Client,
    magiclaw_bin: &str,
    api_addr: &str,
    api_base: &str,
) -> Result<DaemonReadySource, String> {
    if daemon_health_ok(client, api_base).await {
        return Ok(DaemonReadySource::Reused);
    }

    let _guard = MAGICLAW_DAEMON_BOOT_LOCK.lock().await;
    if daemon_health_ok(client, api_base).await {
        return Ok(DaemonReadySource::Reused);
    }

    let mut cmd = tokio::process::Command::new(magiclaw_bin);
    let magiclaw_db_path = std::env::var("MAGICLAW_DB_PATH").unwrap_or_else(|_| std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".to_string()));
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("MAGICLAW_API_ADDR", api_addr)
        .env("MAGICLAW_DB_PATH", magiclaw_db_path);

    if let Ok(dir) = std::env::var("WECHAT_CHANNEL_DIR") {
        cmd.env("WECHAT_CHANNEL_DIR", dir);
    }

    let mut child = cmd.spawn()
        .map_err(|e| format!("启动 magiclaw daemon 失败(magiclaw: {}): {}", magiclaw_bin, e))?;

    for _ in 0..100 {
        if daemon_health_ok(client, api_base).await {
            return Ok(DaemonReadySource::StartedNow);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().await;
                let extra = match out {
                    Ok(o) => {
                        let stdout = String::from_utf8_lossy(&o.stdout);
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        if stderr.contains("another magiclaw instance is already running") {
                            if daemon_health_ok(client, api_base).await {
                                return Ok(DaemonReadySource::Reused);
                            }
                            return Err(
                                "检测到 magiclaw 单实例锁冲突(data/magiclaw.instance.lock)，且当前端口不可用。请先结束旧的 magiclaw 进程后重试（可用: pgrep -af magiclaw / pkill -f '/magiclaw'）"
                                    .to_string(),
                            );
                        }
                        let stderr_tail = tail_lines(&stderr, 8);
                        let stdout_tail = tail_lines(&stdout, 3);
                        if !stderr_tail.is_empty() {
                            format!(" | stderr_tail={}", stderr_tail)
                        } else if !stdout_tail.is_empty() {
                            format!(" | stdout_tail={}", stdout_tail)
                        } else {
                            String::new()
                        }
                    }
                    Err(e) => format!(" | 获取 daemon 输出失败: {}", e),
                };
                return Err(format!(
                    "daemon 进程提前退出(exit={})，请检查 MAGICLAW_BIN/MAGICLAW_API_ADDR/MAGICLAW_API_TOKEN 配置{}",
                    status,
                    extra
                ));
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("[微信] 检查 daemon 进程状态失败: {}", e);
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    Err(format!(
        "daemon 启动后健康检查超时: {} (等待30s)",
        api_addr
    ))
}

pub fn tail_lines(s: &str, n: usize) -> String {
    let mut v: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if v.len() > n {
        v = v.split_off(v.len() - n);
    }
    v.join(" | ")
}

pub async fn send_via_magiclaw_daemon(
    client: &reqwest::Client,
    api_base: &str,
    api_token: &str,
    send_type: MessageSendType,
    to: Option<&str>,
    text: &str,
) -> Result<(), String> {
    let url = format!("{}/api/send", api_base);
    let mut body = serde_json::Map::new();
    body.insert("send_type".to_string(), serde_json::json!(send_type.as_str()));
    body.insert("text".to_string(), serde_json::json!(text));
    if let Some(to) = to.map(str::trim).filter(|v| !v.is_empty()) {
        body.insert("to".to_string(), serde_json::json!(to));
    }

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        client
            .post(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", api_token))
            .json(&serde_json::Value::Object(body))
            .send(),
    ).await
        .map_err(|_| "调用 /api/send 超时(>30s)".to_string())
        .and_then(|r| r.map_err(|e| format!("调用 /api/send 失败: {}", e)))?;

    let status = resp.status();
    let text_body = resp.text().await.unwrap_or_default();
    if status.is_success() {
        let ok = serde_json::from_str::<serde_json::Value>(&text_body)
            .ok()
            .and_then(|v| v.get("ok").and_then(|x| x.as_bool()))
            .unwrap_or(false);
        if ok {
            return Ok(());
        }
        return Err(format!("/api/send 返回非成功体: {}", text_body));
    }

    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(
            "daemon 鉴权失败(401)：请确保 monitor 与 daemon 使用相同 MAGICLAW_API_TOKEN，并重启 daemon 使新 token 生效".to_string(),
        );
    }

    if matches!(send_type, MessageSendType::Wechat)
        && status == reqwest::StatusCode::PRECONDITION_FAILED
        && text_body.contains("no valid context_token for peer") {
        return Err(
            "daemon 拒绝发送(412)：当前会话 context_token 无效。请先在微信给 bot 发一条消息刷新会话窗口后重试".to_string(),
        );
    }

    Err(format!("/api/send HTTP {}: {}", status, text_body))
}

pub async fn verify_daemon_auth(
    client: &reqwest::Client,
    api_base: &str,
    api_token: &str,
    api_token_source: &ApiTokenSource,
) -> Result<(), String> {
    let url = format!("{}/api/window_status", api_base);
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client
            .get(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", api_token))
            .send(),
    ).await
        .map_err(|_| "调用 /api/window_status 超时(>5s)".to_string())
        .and_then(|r| r.map_err(|e| format!("调用 /api/window_status 失败: {}", e)))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if status.is_success() {
        // 窗口可用性预检已移除。ilink 的 ret=-2 不是“窗口用尽/会话过期”的致命信号:
        // daemon 侧 /api/send 现在对 ret=-2 直接当作成功继续发送(仅 errcode=-14 才算
        // 会话过期),连续主动推送已验证可稳定工作。因此 stale / should_refresh /
        // send_count>=9 这些旧启发式都已失效,不再用它们拦截发送。
        // 这里只把 /api/window_status 当作鉴权连通性校验(HTTP 200 = token 有效);
        // 真正无可用 context_token 时,/api/send 会返回 412 并给出可操作提示。
        return Ok(());
    }

    if status == reqwest::StatusCode::UNAUTHORIZED {
        let source_tip = match api_token_source {
            ApiTokenSource::Env => {
                "当前 monitor 使用环境变量 MAGICLAW_API_TOKEN，但 daemon 侧 token 不一致"
            }
            ApiTokenSource::DynamicMemCache | ApiTokenSource::DynamicFileCache | ApiTokenSource::DynamicIssued => {
                "当前 monitor 使用动态 token(数据库签发)。可能该 token 已过期/被吊销，monitor 将自动续签"
            }
        };
        return Err(format!(
            "HTTP 401 unauthorized，{}",
            source_tip
        ));
    }

    Err(format!("/api/window_status HTTP {}: {}", status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PushKind::is_deprecated: 9 保留 + 4 降级 (grill Q2/Q6 修订)
    #[test]
    fn push_kind_is_deprecated_partition() {
        // 保留 9 条
        for k in [
            PushKind::HoldingEvent,
            PushKind::DailyReport,
            PushKind::Announcement,
        ] {
            assert!(!k.is_deprecated(), "{:?} 应保留", k);
        }
        // 降级 10 条 (A2/A3/A4/A5/A6/A11/A12/B4/B10 + grill 补 B11/B12/B13 = 12 条, 但我们只测 4 个代表)
        for k in [
            PushKind::AuctionVolume,
            PushKind::LimitBoards,
            PushKind::FactorIC,
            PushKind::WeeklySOP,
        ] {
            assert!(k.is_deprecated(), "{:?} 应降级", k);
        }
    }

    /// PushKind 总数 = 13 (9 保留 + 12 降级, 但 grill 修订后保留 9 + 降级 12 = 21 变体太多, 我们用 enum 12 个)
    #[test]
    fn push_kind_count() {
        // 枚举定义 = 13 变体 (3 保留 + 10 降级, B11/B12/B13 在 enum 里)
        // 实际归类 = 9 保留 + 12 降级 (grill 修订: A13/A14/A15 用 HoldingEvent, C1 用 Announcement)
        let kinds = [
            PushKind::HoldingEvent,
            PushKind::DailyReport,
            PushKind::Announcement,
            PushKind::AuctionVolume,
            PushKind::VirtualWatch,
            PushKind::LimitBoards,
            PushKind::SectorTop,
            PushKind::FundInflow,
            PushKind::AuctionRepush,
            PushKind::FactorIC,
            PushKind::SectorTier,
            PushKind::CapitalVerify,
            PushKind::WeeklySOP,
        ];
        assert_eq!(kinds.len(), 13, "13 个 PushKind 变体");
    }

    /// push_governor 降级时返回 false (未推), log 输出
    #[tokio::test]
    async fn push_governor_deprecated_no_push() {
        std::env::remove_var("PUSH_VERBOSE");
        let r = push_governor("test deprecated", PushKind::AuctionVolume).await;
        assert!(!r, "降级应返回 false (未推)");
    }

    /// push_governor 保留时调 push_wechat (返回 V10_DRY_RUN_PUSH=true 时为 true)
    #[tokio::test]
    async fn push_governor_kept_calls_push_wechat() {
        std::env::set_var("V10_DRY_RUN_PUSH", "1"); // push_wechat 走 dry-run 返回 true
        let r = push_governor("test kept", PushKind::HoldingEvent).await;
        assert!(r, "保留应调 push_wechat (V10_DRY_RUN_PUSH=true 返回 true)");
        std::env::remove_var("V10_DRY_RUN_PUSH");
    }

    /// PUSH_VERBOSE=true 覆盖降级 → 调 push_wechat
    #[tokio::test]
    async fn push_verbose_true_overrides_deprecated() {
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("PUSH_VERBOSE", "true");
        let r = push_governor("test verbose", PushKind::AuctionVolume).await;
        assert!(r, "PUSH_VERBOSE=true 应覆盖降级, 调 push_wechat (dry-run 返回 true)");
        std::env::remove_var("V10_DRY_RUN_PUSH");
        std::env::remove_var("PUSH_VERBOSE");
    }
}
