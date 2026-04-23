//! 邮件渠道实现（纯文本 + 带图片）

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Local;
use log::{info, warn};
use lettre::{Message, SmtpTransport, Transport};
use lettre::message::{header, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;

use super::service::NotificationService;

impl NotificationService {
    /// 发送邮件
    pub fn send_to_email(&self, content: &str) -> Result<bool> {
        let sender = self.config.email_sender.as_ref()
            .context("邮件发送者未配置 (EMAIL_SENDER)")?;
        let password = self.config.email_password.as_ref()
            .context("邮件密码未配置 (EMAIL_PASSWORD)")?;
        let smtp_server = self.config.smtp_server.as_ref()
            .context("SMTP服务器未配置 (SMTP_SERVER)")?;
        let smtp_port = self.config.smtp_port
            .context("SMTP端口未配置 (SMTP_PORT)")?;
        
        if self.config.email_receivers.is_empty() {
            return Err(anyhow::anyhow!("邮件接收者列表为空 (EMAIL_RECEIVERS)"));
        }
        
        let primary = &self.config.email_receivers[0];
        let cc_list: Vec<&String> = self.config.email_receivers.iter().skip(1).collect();
        
        info!("准备发送邮件到主收件人: {}，抄送 {} 位，SMTP: {}:{}", 
            primary, cc_list.len(), smtp_server, smtp_port);
        
        // 转换 Markdown 为 HTML
        let html_content = self.markdown_to_html(content);
        
        // 构建邮件主题
        let subject = format!("A股分析日报 - {}", Local::now().format("%Y-%m-%d"));
        
        self.send_single_email(
            sender, 
            primary,
            &cc_list,
            &subject, 
            content, 
            &html_content,
            smtp_server,
            smtp_port,
            password
        )?;
        
        info!("邮件发送成功: 主收件人 {}，抄送 {} 位", primary, cc_list.len());
        Ok(true)
    }
    
    /// 发送单封邮件（第一个收件人为主地址，其余为抄送）
    pub(super) fn send_single_email(
        &self,
        from: &str,
        to: &str,
        cc_list: &[&String],
        subject: &str,
        text_content: &str,
        html_content: &str,
        smtp_server: &str,
        smtp_port: u16,
        password: &str,
    ) -> Result<()> {
        // 构建邮件
        let mut builder = Message::builder()
            .from(from.parse()?)
            .to(to.parse()?);
        
        for cc in cc_list {
            builder = builder.cc(cc.parse()?);
        }
        
        let email = builder
            .subject(subject)
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(header::ContentType::TEXT_PLAIN)
                            .body(text_content.to_string())
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(header::ContentType::TEXT_HTML)
                            .body(html_content.to_string())
                    )
            )?;
        
        // 配置 SMTP
        let creds = Credentials::new(from.to_string(), password.to_string());
        
        let mailer = if smtp_port == 465 {
            SmtpTransport::relay(smtp_server)?
                .credentials(creds)
                .timeout(Some(Duration::from_secs(120)))
                .build()
        } else {
            SmtpTransport::starttls_relay(smtp_server)?
                .port(smtp_port)
                .credentials(creds)
                .timeout(Some(Duration::from_secs(120)))
                .build()
        };
        
        // 发送（带重试）
        let max_retries = 3;
        let mut last_err = None;
        for attempt in 1..=max_retries {
            match mailer.send(&email) {
                Ok(_) => return Ok(()),
                Err(e) => {
                    warn!("邮件发送第 {} 次尝试失败: {}", attempt, e);
                    last_err = Some(e);
                    if attempt < max_retries {
                        std::thread::sleep(Duration::from_secs(attempt as u64 * 3));
                    }
                }
            }
        }
        
        Err(last_err.unwrap().into())
    }

    /// 发送带图片的邮件
    pub(super) fn send_email_with_image(&self, content: &str, image_path: &Path) -> Result<bool> {
        let sender = self.config.email_sender.as_ref()
            .context("邮件发送者未配置 (EMAIL_SENDER)")?;
        let password = self.config.email_password.as_ref()
            .context("邮件密码未配置 (EMAIL_PASSWORD)")?;
        let smtp_server = self.config.smtp_server.as_ref()
            .context("SMTP服务器未配置 (SMTP_SERVER)")?;
        let smtp_port = self.config.smtp_port
            .context("SMTP端口未配置 (SMTP_PORT)")?;
        
        if self.config.email_receivers.is_empty() {
            return Err(anyhow::anyhow!("邮件接收者列表为空 (EMAIL_RECEIVERS)"));
        }
        
        let primary = &self.config.email_receivers[0];
        let cc_list: Vec<&String> = self.config.email_receivers.iter().skip(1).collect();
        
        info!("准备发送带图片的邮件到主收件人: {}，抄送 {} 位", primary, cc_list.len());
        
        // 转换 Markdown 为 HTML
        let html_content = self.markdown_to_html(content);
        
        // 读取图片
        let image_data = std::fs::read(image_path)
            .context("读取图片文件失败")?;
        let image_filename = image_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("chart.png");
        
        // 构建邮件主题
        let subject = format!("A股分析日报（含图表） - {}", Local::now().format("%Y-%m-%d"));
        
        self.send_single_email_with_image(
            sender,
            primary,
            &cc_list,
            &subject,
            content,
            &html_content,
            &image_data,
            image_filename,
            smtp_server,
            smtp_port,
            password
        )?;
        
        info!("邮件（含图表）发送成功: 主收件人 {}，抄送 {} 位", primary, cc_list.len());
        Ok(true)
    }

    /// 发送单封带图片的邮件（第一个收件人为主地址，其余为抄送）
    pub(super) fn send_single_email_with_image(
        &self,
        from: &str,
        to: &str,
        cc_list: &[&String],
        subject: &str,
        text_content: &str,
        html_content: &str,
        image_data: &[u8],
        image_filename: &str,
        smtp_server: &str,
        smtp_port: u16,
        password: &str,
    ) -> Result<()> {
        use lettre::message::Attachment;
        
        // 在 HTML 中嵌入图片引用
        let html_with_image = format!(
            "{}<br/><br/><img src=\"cid:{}\" alt=\"分析图表\" style=\"max-width:100%; height:auto;\"/>",
            html_content,
            image_filename
        );
        
        // 构建邮件
        let mut builder = Message::builder()
            .from(from.parse()?)
            .to(to.parse()?);
        
        for cc in cc_list {
            builder = builder.cc(cc.parse()?);
        }
        
        let email = builder
            .subject(subject)
            .multipart(
                MultiPart::mixed()
                    .multipart(
                        MultiPart::alternative()
                            .singlepart(
                                SinglePart::builder()
                                    .header(header::ContentType::TEXT_PLAIN)
                                    .body(text_content.to_string())
                            )
                            .singlepart(
                                SinglePart::builder()
                                    .header(header::ContentType::TEXT_HTML)
                                    .body(html_with_image)
                            )
                    )
                    .singlepart(
                        Attachment::new_inline(image_filename.to_string())
                            .body(image_data.to_vec(), "image/png".parse()?)
                    )
            )?;
        
        // 配置 SMTP
        let creds = Credentials::new(from.to_string(), password.to_string());
        
        let mailer = if smtp_port == 465 {
            SmtpTransport::relay(smtp_server)?
                .credentials(creds)
                .timeout(Some(Duration::from_secs(120)))
                .build()
        } else {
            SmtpTransport::starttls_relay(smtp_server)?
                .port(smtp_port)
                .credentials(creds)
                .timeout(Some(Duration::from_secs(120)))
                .build()
        };
        
        // 发送邮件（带重试）
        let max_retries = 3;
        let mut last_err = None;
        for attempt in 1..=max_retries {
            match mailer.send(&email) {
                Ok(_) => return Ok(()),
                Err(e) => {
                    warn!("邮件发送第 {} 次尝试失败: {}", attempt, e);
                    last_err = Some(e);
                    if attempt < max_retries {
                        std::thread::sleep(Duration::from_secs(attempt as u64 * 3));
                    }
                }
            }
        }
        
        Err(last_err.unwrap().into())
    }
}
