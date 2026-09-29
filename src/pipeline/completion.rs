//! Analysis persistence and BestEffort notification observations are separate facts.
use super::cli_identity::{CliBusinessIdentity, CliInvocationIdentity, CliReportSnapshot};
use super::AnalysisResult;
use crate::notification::{NotificationCompletion, NotificationSendReport};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnalysisSaveStatus {
    NotAttempted,
    Saved,
    Failed(String),
}

#[derive(Clone, Debug)]
pub enum AnalysisNotification {
    NotRequested,
    NotAttempted,
    Attempted(NotificationSendReport),
    /// Cancellation/timeout after invocation cannot prove that nothing was sent.
    Unknown(String),
}

impl AnalysisNotification {
    pub fn completion(&self) -> Option<NotificationCompletion> {
        match self {
            Self::Attempted(report) => Some(report.completion()),
            _ => None,
        }
    }
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::NotRequested)
            || self.completion() == Some(NotificationCompletion::AllAccepted)
    }
    pub fn ensure_cli_success(&self) -> anyhow::Result<()> {
        match self {
            Self::NotRequested => Ok(()),
            Self::Attempted(report) => {
                match report.completion() {
                    NotificationCompletion::AllAccepted => Ok(()),
                    NotificationCompletion::Partial => {
                        anyhow::bail!("通知本轮不完整：部分渠道弱接受，其他渠道结果未知；不自动重发已接受渠道")
                    }
                    state => anyhow::bail!("通知未完成: {state:?}（不代表权威送达）"),
                }
            }
            state => anyhow::bail!("通知未完成: {state:?}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StockAnalysisOutcome {
    pub code: String,
    pub business_identity: CliBusinessIdentity,
    pub analysis: Option<AnalysisResult>,
    pub saved: AnalysisSaveStatus,
    /// Present only when the report was handed to the notification path.
    pub report_snapshot: Option<CliReportSnapshot>,
    pub notification: AnalysisNotification,
    pub failure: Option<String>,
}
impl StockAnalysisOutcome {
    pub(super) fn new(
        code: String,
        input_ordinal: usize,
        notify: bool,
        invocation: &CliInvocationIdentity,
    ) -> Self {
        Self {
            business_identity: invocation.stock_business(&code, input_ordinal),
            code,
            analysis: None,
            saved: AnalysisSaveStatus::NotAttempted,
            report_snapshot: None,
            notification: if notify {
                AnalysisNotification::NotAttempted
            } else {
                AnalysisNotification::NotRequested
            },
            failure: None,
        }
    }
    pub fn ensure_cli_success(&self) -> anyhow::Result<()> {
        if let Some(error) = &self.failure {
            anyhow::bail!("{}: {}", self.code, error);
        }
        if let AnalysisSaveStatus::Failed(error) = &self.saved {
            anyhow::bail!("{} 分析保存失败: {}", self.code, error);
        }
        self.notification.ensure_cli_success()
    }
}

#[derive(Clone, Debug)]
pub struct SummaryCompletion {
    pub business_identity: Option<CliBusinessIdentity>,
    pub saved_paths: Vec<std::path::PathBuf>,
    pub report_snapshot: Option<CliReportSnapshot>,
    pub notification: AnalysisNotification,
    pub failure: Option<String>,
}
impl Default for SummaryCompletion {
    fn default() -> Self {
        Self {
            business_identity: None,
            saved_paths: Vec::new(),
            report_snapshot: None,
            notification: AnalysisNotification::NotRequested,
            failure: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AnalysisRunReport {
    pub invocation: Option<CliInvocationIdentity>,
    /// Saved analyses remain available even when their notification was not accepted.
    pub results: Vec<AnalysisResult>,
    pub stocks: Vec<StockAnalysisOutcome>,
    pub summary: SummaryCompletion,
}
impl AnalysisRunReport {
    pub fn ensure_cli_success(&self) -> anyhow::Result<()> {
        for stock in &self.stocks {
            stock.ensure_cli_success()?;
        }
        if let Some(error) = &self.summary.failure {
            anyhow::bail!("汇总失败: {}", error);
        }
        self.summary.notification.ensure_cli_success()
    }
    pub fn is_complete(&self) -> bool {
        self.stocks.iter().all(|s| {
            s.failure.is_none()
                && !matches!(s.saved, AnalysisSaveStatus::Failed(_))
                && s.notification.is_complete()
        }) && self.summary.failure.is_none()
            && self.summary.notification.is_complete()
    }
    pub fn log_completion(&self) {
        for stock in &self.stocks {
            log::info!(
                "分析完成状态 {}: save={:?}, notification={:?}, failure={:?}",
                stock.code,
                stock.saved,
                stock.notification,
                stock.failure
            );
        }
        log::info!("汇总完成状态: {:?}", self.summary);
        if !self.is_complete() {
            log::warn!("分析本轮不完整；弱接受不代表权威送达，保留已保存结果");
        }
    }
}
