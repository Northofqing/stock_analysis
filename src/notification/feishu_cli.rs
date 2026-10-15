// -*- coding: utf-8 -*-
//! 飞书 CLI (magiclaw) 报告发送适配器。
//!
//! 2026-10-15 用户决策: 产业链报告「整理简化 → 只推飞书 (magiclaw CLI)」,
//! 不再走邮件/多渠道路径。
//!
//! 与 monitor bin 的推送管道 (`bin/monitor/notify.rs`, 带 push_log/durable
//! 审计) 独立: 本模块只做最小的 CLI 调用 (`magiclaw send --channel feishu`),
//! 报告的送达证据由调用方 (chain_schedule 的 send_attempt 表 + 调度日志) 承担。
//!
//! 配置来源与 bin 保持一致的环境变量:
//! - `MAGICLAW_BIN`   可执行文件路径 (默认 `magiclaw`)
//! - `MAGICLAW_HOME`  运行目录 (存在时作为 cwd)
//! - `MAGICLAW_DB_PATH` 数据库路径 (存在时透传)
//! - 收件人: `FEISHU_TO` / `MAGICLAW_FEISHU_TO` / `FEISHU_CHAT_ID` /
//!   `FEISHU_OPEN_ID` / `FEISHU_USER_ID` / `FEISHU_EMAIL`

use std::process::Stdio;

/// 解析 magiclaw 可执行文件路径。
pub fn resolve_magiclaw_bin() -> String {
    std::env::var("MAGICLAW_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "magiclaw".to_string())
}

/// 解析飞书收件人 (与 bin/monitor/notify.rs 的优先级一致)。
pub fn resolve_feishu_target() -> Option<String> {
    for key in [
        "FEISHU_TO",
        "MAGICLAW_FEISHU_TO",
        "FEISHU_CHAT_ID",
        "FEISHU_OPEN_ID",
        "FEISHU_USER_ID",
        "FEISHU_EMAIL",
    ] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 构建 magiclaw CLI 发送命令 (不启动进程, 便于单测校验参数序列)。
fn build_send_command(bin: &str, to: &str, text: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg("send")
        .arg("--channel")
        .arg("feishu")
        .arg("--message")
        .arg(text)
        .arg("--to")
        .arg(to)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// 通过 magiclaw CLI 发送文本到飞书。
///
/// 成功 = CLI 退出码 0 (弱接受: CLI 已接收并可投递); 失败携带截断的 stderr。
pub async fn send_text_to_feishu(text: &str) -> Result<(), String> {
    let to = resolve_feishu_target().ok_or_else(|| {
        "飞书发送缺少收件人 (FEISHU_TO / MAGICLAW_FEISHU_TO / FEISHU_CHAT_ID 等未配置)".to_string()
    })?;
    let bin = resolve_magiclaw_bin();
    let mut cmd = build_send_command(&bin, &to, text);
    if let Ok(home) = std::env::var("MAGICLAW_HOME") {
        let home = home.trim();
        if !home.is_empty() {
            cmd.current_dir(home);
        }
    }
    let output = cmd
        .output()
        .await
        .map_err(|error| format!("magiclaw CLI 启动失败 ({bin}): {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr_tail: String = stderr.chars().take(300).collect();
    Err(format!(
        "magiclaw CLI 退出码 {:?}: {}",
        output.status.code(),
        stderr_tail.trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_command_pins_feishu_channel_message_and_target() {
        let cmd = build_send_command("/opt/test/magiclaw", "oc_test_target", "简化版正文");
        let args: Vec<&std::ffi::OsStr> = cmd.as_std().get_args().collect();
        let args: Vec<&str> = args.iter().filter_map(|arg| arg.to_str()).collect();
        assert_eq!(
            args,
            vec![
                "send",
                "--channel",
                "feishu",
                "--message",
                "简化版正文",
                "--to",
                "oc_test_target"
            ]
        );
    }

    #[test]
    fn bin_resolution_prefers_explicit_env_and_keeps_default() {
        let resolved = resolve_magiclaw_bin();
        match std::env::var("MAGICLAW_BIN") {
            Ok(value) if !value.trim().is_empty() => assert_eq!(resolved, value.trim()),
            _ => assert_eq!(resolved, "magiclaw"),
        }
    }
}
