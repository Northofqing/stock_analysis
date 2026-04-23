// -*- coding: utf-8 -*-
//! 通知层模块（原 `src/notification.rs` 1731 行，已拆分）
//!
//! 职责：
//! 1. 汇总分析结果生成日报
//! 2. 支持 Markdown 格式输出
//! 3. 多渠道推送：企业微信、飞书、Telegram、邮件、Pushover
//!
//! 子模块：
//! - `config`  — 渠道枚举与 `NotificationConfig`
//! - `service` — `NotificationService` 主入口 + 统一 send + save_report_to_file
//! - `report`  — 日报 Markdown 生成
//! - `wechat`  — 企业微信 Webhook
//! - `feishu`  — 飞书 Webhook + Markdown→HTML
//! - `email`   — SMTP 邮件（含嵌入图片）

pub mod config;
pub mod email;
pub mod feishu;
pub mod report;
pub mod service;
pub mod wechat;

pub use config::{NotificationChannel, NotificationConfig, SmtpConfig};
pub use service::{send_daily_report, AnalysisResult, NotificationService};
