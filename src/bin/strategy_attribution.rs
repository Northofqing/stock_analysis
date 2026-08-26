//! BR-251 默认只读的中文买卖策略历史归因 CLI。

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};
use stock_analysis::calendar::{
    resolve_verified_scheduled_replay, VerifiedCalendarError, VerifiedCalendarErrorKind,
    VerifiedReplayCalendar,
};
use stock_analysis::data_gateway::{
    probe_benchmark_request, BenchmarkCapture, BenchmarkError, BenchmarkProbeReport,
    BenchmarkRange, BenchmarkRequest, HS300_CANONICAL,
};
use stock_analysis::database::attribution_reports::{
    AttributionDatabaseAccess, AttributionDatabaseSession, AttributionReportReceipt,
    AttributionReportStoreError,
};
use stock_analysis::performance::attribution_replay::{
    AttributionConclusion, AttributionReplayLoader, AttributionReplayRunner, BenchmarkDayManifest,
    MetricAggregate, PreparedAttributionReport, ReplayError, ReplayErrorClass, ReplayMode,
    ReplayRequest, ReplayStage,
};

const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;
const USAGE_EXIT: u8 = 2;
const UNAVAILABLE_EXIT: u8 = 3;
const INTEGRITY_EXIT: u8 = 4;
const STORAGE_EXIT: u8 = 5;

#[derive(Debug, Parser)]
#[command(
    name = "strategy_attribution",
    about = "BR-251 默认只读的中文买卖策略历史归因工具",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 只读解析业务日期，不连接数据库。
    Resolve {
        /// 显式 +08:00 运行时刻；省略时使用当前上海时刻。
        #[arg(long)]
        at: Option<DateTime<FixedOffset>>,
        #[arg(long, value_enum, default_value_t)]
        format: OutputFormat,
    },
    /// 只读调用真实指数 adapter；不签发 Reader 能力，不写审计或数据库。
    Probe {
        #[command(flatten)]
        request: BenchmarkRequestArgs,
        #[arg(long, value_enum, default_value_t)]
        format: OutputFormat,
    },
    /// 预览规范请求；仅显式 commit 才采集并追加新基准表。
    Capture {
        #[command(flatten)]
        request: BenchmarkRequestArgs,
        #[arg(long)]
        db: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        commit: bool,
        #[arg(long, value_enum, default_value_t)]
        format: OutputFormat,
    },
    /// 解析最近完成业务日并运行归因；默认只读预览。
    Scheduled {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        at: Option<DateTime<FixedOffset>>,
        #[arg(long = "manifest", required = true)]
        manifests: Vec<ManifestArg>,
        #[arg(long, default_value_t = false)]
        commit: bool,
        #[arg(long, value_enum, default_value_t)]
        format: OutputFormat,
    },
    /// 按显式闭区间运行历史归因；默认只读预览。
    Replay {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        from: NaiveDate,
        #[arg(long)]
        to: NaiveDate,
        #[arg(long)]
        at: Option<DateTime<FixedOffset>>,
        #[arg(long = "manifest", required = true)]
        manifests: Vec<ManifestArg>,
        #[arg(long, default_value_t = false)]
        commit: bool,
        #[arg(long, value_enum, default_value_t)]
        format: OutputFormat,
    },
    /// 按三个月自然季度运行历史归因；默认只读预览。
    Quarter {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        year: i32,
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=4))]
        quarter: u8,
        #[arg(long)]
        at: Option<DateTime<FixedOffset>>,
        #[arg(long = "manifest", required = true)]
        manifests: Vec<ManifestArg>,
        #[arg(long, default_value_t = false)]
        commit: bool,
        #[arg(long, value_enum, default_value_t)]
        format: OutputFormat,
    },
}

#[derive(Debug, Clone, Args)]
struct BenchmarkRequestArgs {
    #[arg(long, default_value = HS300_CANONICAL)]
    instrument: String,
    #[arg(long, value_enum)]
    granularity: BenchmarkGranularityArg,
    #[arg(long)]
    from: String,
    #[arg(long)]
    to: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BenchmarkGranularityArg {
    Daily,
    Minute1,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum OutputFormat {
    #[default]
    Markdown,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestArg(BenchmarkDayManifest);

impl FromStr for ManifestArg {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let (date, hash) = raw
            .split_once('=')
            .ok_or_else(|| "manifest 必须为 YYYY-MM-DD=<64位小写sha256>".to_owned())?;
        let trading_date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| "manifest 日期必须为 YYYY-MM-DD".to_owned())?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("manifest hash 必须为64位小写sha256".to_owned());
        }
        Ok(Self(BenchmarkDayManifest {
            trading_date,
            manifest_hash: hash.to_owned(),
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppErrorClass {
    Usage,
    Unavailable,
    FailedIntegrity,
    Storage,
}

#[derive(Debug)]
struct AppError {
    class: AppErrorClass,
    stage: &'static str,
    code: String,
    retryable: bool,
    failure_audit_id: Option<i64>,
}

impl AppError {
    fn usage(code: impl Into<String>) -> Self {
        Self {
            class: AppErrorClass::Usage,
            stage: "request",
            code: code.into(),
            retryable: false,
            failure_audit_id: None,
        }
    }

    fn exit_code(&self) -> ExitCode {
        ExitCode::from(match self.class {
            AppErrorClass::Usage => USAGE_EXIT,
            AppErrorClass::Unavailable => UNAVAILABLE_EXIT,
            AppErrorClass::FailedIntegrity => INTEGRITY_EXIT,
            AppErrorClass::Storage => STORAGE_EXIT,
        })
    }

    fn render(&self, format: OutputFormat) -> String {
        let class = match self.class {
            AppErrorClass::Usage => "参数错误",
            AppErrorClass::Unavailable => "数据不可用",
            AppErrorClass::FailedIntegrity => "完整性失败",
            AppErrorClass::Storage => "存储失败",
        };
        match format {
            OutputFormat::Json => json!({
                "状态": "失败",
                "分类": class,
                "阶段": self.stage,
                "原因代码": self.code,
                "可重试": self.retryable,
                "失败审计ID": self.failure_audit_id,
                "说明": "错误已脱敏；未产生策略成功结论"
            })
            .to_string(),
            OutputFormat::Markdown => format!(
                "# 策略归因失败\n\n- 分类：{class}\n- 阶段：{}\n- 原因代码：{}\n- 可重试：{}\n- 失败审计 ID：{}\n- 说明：错误已脱敏；未产生策略成功结论",
                self.stage,
                self.code,
                self.retryable,
                self.failure_audit_id
                    .map_or_else(|| "无".to_owned(), |id| id.to_string())
            ),
        }
    }
}

fn validate_command(cli: &Cli) -> Result<(), AppError> {
    if let Command::Capture { db, commit, .. } = &cli.command {
        match (*commit, db.is_some()) {
            (true, false) => return Err(AppError::usage("capture_commit_requires_database")),
            (false, true) => return Err(AppError::usage("capture_preview_rejects_database")),
            _ => {}
        }
    }
    Ok(())
}

fn shanghai_now() -> Result<DateTime<FixedOffset>, AppError> {
    let offset = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS)
        .ok_or_else(|| AppError::usage("shanghai_offset_unavailable"))?;
    Ok(Utc::now().with_timezone(&offset))
}

fn invocation_time(
    supplied: Option<DateTime<FixedOffset>>,
) -> Result<DateTime<FixedOffset>, AppError> {
    let value = match supplied {
        Some(value) => value,
        None => shanghai_now()?,
    };
    if value.offset().local_minus_utc() != SHANGHAI_OFFSET_SECONDS {
        return Err(AppError::usage("invocation_timezone_must_be_plus_08_00"));
    }
    Ok(value)
}

fn parse_benchmark_request(args: &BenchmarkRequestArgs) -> Result<BenchmarkRequest, AppError> {
    if args.instrument != HS300_CANONICAL {
        return Err(AppError::usage("unsupported_benchmark_instrument"));
    }
    let range = match args.granularity {
        BenchmarkGranularityArg::Daily => {
            let from = NaiveDate::parse_from_str(&args.from, "%Y-%m-%d")
                .map_err(|_| AppError::usage("invalid_daily_from"))?;
            let to = NaiveDate::parse_from_str(&args.to, "%Y-%m-%d")
                .map_err(|_| AppError::usage("invalid_daily_to"))?;
            if from > to {
                return Err(AppError::usage("invalid_benchmark_range"));
            }
            BenchmarkRange::Daily { from, to }
        }
        BenchmarkGranularityArg::Minute1 => {
            let from = DateTime::parse_from_rfc3339(&args.from)
                .map_err(|_| AppError::usage("invalid_minute_from"))?;
            let to = DateTime::parse_from_rfc3339(&args.to)
                .map_err(|_| AppError::usage("invalid_minute_to"))?;
            if from.offset().local_minus_utc() != SHANGHAI_OFFSET_SECONDS
                || to.offset().local_minus_utc() != SHANGHAI_OFFSET_SECONDS
                || from.date_naive() != to.date_naive()
                || from > to
            {
                return Err(AppError::usage("invalid_minute_range"));
            }
            BenchmarkRange::Minute1 { from, to }
        }
    };
    Ok(BenchmarkRequest {
        instrument: args.instrument.clone(),
        range,
    })
}

fn natural_quarters(request: &BenchmarkRequest) -> Vec<String> {
    let (from, to) = match &request.range {
        BenchmarkRange::Daily { from, to } => (*from, *to),
        BenchmarkRange::Minute1 { from, to } => (from.date_naive(), to.date_naive()),
    };
    let mut year = from.year();
    let mut quarter = ((from.month() - 1) / 3) + 1;
    let end_year = to.year();
    let end_quarter = ((to.month() - 1) / 3) + 1;
    let mut labels = Vec::new();
    loop {
        labels.push(format!("{year}-Q{quarter}"));
        if year == end_year && quarter == end_quarter {
            break;
        }
        if quarter == 4 {
            year += 1;
            quarter = 1;
        } else {
            quarter += 1;
        }
    }
    labels
}

fn benchmark_request_value(request: &BenchmarkRequest) -> Value {
    match &request.range {
        BenchmarkRange::Daily { from, to } => json!({
            "基准": request.instrument,
            "粒度": "日线",
            "起始": from,
            "结束": to,
            "时区": "Asia/Shanghai"
        }),
        BenchmarkRange::Minute1 { from, to } => json!({
            "基准": request.instrument,
            "粒度": "1分钟",
            "起始": from.to_rfc3339(),
            "结束": to.to_rfc3339(),
            "时区": "Asia/Shanghai"
        }),
    }
}

fn render_calendar(
    calendar: &VerifiedReplayCalendar,
    invoked_at: DateTime<FixedOffset>,
    format: OutputFormat,
) -> String {
    match format {
        OutputFormat::Json => json!({
            "状态": "成功",
            "模式": "只读日期解析",
            "运行时刻": invoked_at.to_rfc3339(),
            "目标起始": calendar.target_from(),
            "目标结束": calendar.target_to(),
            "交易日": calendar.required_trading_dates(),
            "日历权威哈希": calendar.authority_hash(),
            "数据库已连接": false
        })
        .to_string(),
        OutputFormat::Markdown => format!(
            "# 业务日期解析\n\n- 状态：成功\n- 模式：只读日期解析\n- 运行时刻：{}\n- 目标范围：{} 至 {}\n- 交易日数：{}\n- 日历权威哈希：{}\n- 数据库已连接：否",
            invoked_at.to_rfc3339(),
            calendar.target_from(),
            calendar.target_to(),
            calendar.required_trading_dates().len(),
            calendar.authority_hash()
        ),
    }
}

fn render_probe(report: &BenchmarkProbeReport, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => json!({
            "状态": "成功",
            "模式": "只读真实适配器探针",
            "请求哈希": report.request_hash,
            "基准": report.instrument,
            "粒度": format!("{:?}", report.granularity),
            "来源提供方": report.provider,
            "来源": report.source,
            "来源时刻": report.source_at,
            "观察时刻": report.observed_at,
            "批次ID": report.batch_id,
            "接纳记录数": report.accepted_records,
            "提供方页大小": report.provider_page_size,
            "首标签": report.first_label,
            "末标签": report.last_label,
            "分钟标签语义": report.minute_label_semantics,
            "协议版本": report.protocol_revision,
            "Reader能力已签发": false,
            "数据库已写入": false
        })
        .to_string(),
        OutputFormat::Markdown => format!(
            "# 基准只读探针\n\n- 状态：成功\n- 基准：{}\n- 粒度：{:?}\n- 请求哈希：{}\n- 来源：{} / {}\n- 来源时刻：{}\n- 观察时刻：{}\n- 批次 ID：{}\n- 接纳记录数：{}\n- 首末标签：{} / {}\n- 分钟标签语义：{}\n- 协议版本：{}\n- Reader 能力已签发：否\n- 数据库已写入：否",
            report.instrument,
            report.granularity,
            report.request_hash,
            report.provider,
            report.source,
            report.source_at.as_deref().unwrap_or("缺失"),
            report.observed_at,
            report.batch_id,
            report.accepted_records,
            report.first_label,
            report.last_label,
            report.minute_label_semantics,
            report.protocol_revision
        ),
    }
}

fn render_capture_preview(request: &BenchmarkRequest, format: OutputFormat) -> String {
    let quarters = natural_quarters(request);
    match format {
        OutputFormat::Json => json!({
            "状态": "成功",
            "模式": "基准采集预览",
            "规范请求": benchmark_request_value(request),
            "预期自然季度": quarters,
            "真实适配器已调用": false,
            "writer已初始化": false,
            "数据库已连接": false,
            "提示": "添加 --commit 与 --db 后才会真实采集并追加新表"
        })
        .to_string(),
        OutputFormat::Markdown => format!(
            "# 基准采集预览\n\n- 状态：成功\n- 规范请求：{}\n- 预期自然季度：{}\n- 真实适配器已调用：否\n- writer 已初始化：否\n- 数据库已连接：否\n- 提示：添加 `--commit` 与 `--db` 后才会真实采集并追加新表",
            benchmark_request_value(request),
            quarters.join("、")
        ),
    }
}

fn render_capture_receipt(
    receipt: &stock_analysis::database::benchmark_segments::BenchmarkManifestRef,
    format: OutputFormat,
) -> String {
    match format {
        OutputFormat::Json => json!({
            "状态": "成功",
            "模式": "基准采集提交",
            "manifest哈希": receipt.manifest_hash,
            "基准": receipt.instrument,
            "粒度": format!("{:?}", receipt.granularity),
            "覆盖起始": receipt.from_key,
            "覆盖结束": receipt.to_key,
            "segment哈希": receipt.segment_hashes
        })
        .to_string(),
        OutputFormat::Markdown => format!(
            "# 基准采集提交\n\n- 状态：成功\n- Manifest 哈希：{}\n- 基准：{}\n- 粒度：{:?}\n- 覆盖：{} 至 {}\n- Segment 数：{}\n- Segment 哈希：{}",
            receipt.manifest_hash,
            receipt.instrument,
            receipt.granularity,
            receipt.from_key,
            receipt.to_key,
            receipt.segment_hashes.len(),
            receipt.segment_hashes.join("、")
        ),
    }
}

fn metric_value(metric: &MetricAggregate) -> Value {
    json!({
        "总周期": metric.total_cycles,
        "可用周期": metric.available_cycles,
        "不可用周期": metric.unavailable_cycles,
        "覆盖率": metric.coverage_ratio,
        "不可用原因": metric.unavailable_reasons,
        "收益和": metric.sum_return,
        "平均收益": metric.mean_return,
        "中位收益": metric.median_return
    })
}

fn conclusion_value(conclusion: &AttributionConclusion) -> Value {
    match conclusion {
        AttributionConclusion::InsufficientSample {
            reasons,
            research_limitations,
        } => json!({
            "样本门状态": "InsufficientSample",
            "原因": reasons,
            "研究限制": research_limitations
        }),
        AttributionConclusion::ResearchOnly {
            research_limitations,
        } => json!({
            "样本门状态": "ResearchOnly",
            "原因": [],
            "研究限制": research_limitations
        }),
    }
}

fn prepared_report_value(prepared: &PreparedAttributionReport) -> Value {
    let invocation = prepared.invocation();
    let report = prepared.report();
    json!({
        "状态": "ResearchOnly",
        "警告": "本报告只用于研究，不构成策略成功、交易或下单结论",
        "运行": {
            "模式": format!("{:?}", invocation.mode),
            "运行时刻": invocation.invoked_at.to_rfc3339(),
            "目标起始": invocation.target_from,
            "目标结束": invocation.target_to,
            "规则版本": invocation.rule_version
        },
        "证据清单": {
            "成交manifest哈希": prepared.trade_manifest_hash(),
            "个股收盘manifest哈希": prepared.stock_close_manifest_hash(),
            "基准组合manifest哈希": prepared.benchmark_manifest_hash(),
            "基准逐日manifest": prepared.benchmark_day_manifests(),
            "日历权威哈希": prepared.calendar_authority_hash()
        },
        "样本": {
            "来源成交数": report.source_fill_ids().len(),
            "总周期": report.total_closed_cycles() + report.total_open_cycles(),
            "闭合周期": report.total_closed_cycles(),
            "开放周期_右删失": report.total_open_cycles(),
            "覆盖自然日": report.coverage_days()
        },
        "指标": {
            "毛收益": metric_value(report.gross()),
            "基准收益": metric_value(report.benchmark()),
            "毛超额收益": metric_value(report.gross_excess()),
            "净收益": metric_value(report.net()),
            "净超额收益": metric_value(report.net_excess()),
            "毛胜率": report.gross_win_rate(),
            "净胜率可用性": report.net_win_rate(),
            "毛胜负分母": report.gross_outcome(),
            "净胜负分母": report.net_outcome(),
            "费用证据可用性": report.fee_basis()
        },
        "按入场族归因": report.family_attribution(),
        "数据质量": {
            "不可用原因已计入总分母": true,
            "开放周期按右删失展示": true,
            "缺失字段未补零": true
        },
        "样本门与结论": conclusion_value(report.conclusion())
    })
}

fn render_prepared_report(prepared: &PreparedAttributionReport, format: OutputFormat) -> String {
    let value = prepared_report_value(prepared);
    if matches!(format, OutputFormat::Json) {
        return value.to_string();
    }
    let report = prepared.report();
    let invocation = prepared.invocation();
    format!(
        "# 买卖策略历史归因（ResearchOnly）\n\n- 运行模式：{:?}\n- 运行时刻：{}\n- 目标范围：{} 至 {}\n- 规则版本：{}\n- 总周期：{}（闭合 {}，开放/右删失 {}）\n- 覆盖自然日：{}\n- 成交 Manifest：{}\n- 个股收盘 Manifest：{}\n- 基准组合 Manifest：{}\n- 日历权威 Manifest：{}\n\n## 指标与完整分母\n\n- 毛收益：{}\n- 基准收益：{}\n- 毛超额收益：{}\n- 净收益：{}\n- 净超额收益：{}\n- 毛胜率：{}\n- 净胜率可用性：{}\n- 按入场族归因：{}\n\n## 样本门与结论\n\n{}\n\n> 本报告只用于研究，不构成策略成功、交易或下单结论。不可用原因保留在总分母中，缺失字段未补零。",
        invocation.mode,
        invocation.invoked_at.to_rfc3339(),
        invocation.target_from,
        invocation.target_to,
        invocation.rule_version,
        report.total_closed_cycles() + report.total_open_cycles(),
        report.total_closed_cycles(),
        report.total_open_cycles(),
        report
            .coverage_days()
            .map_or_else(|| "不可用".to_owned(), |days| days.to_string()),
        prepared.trade_manifest_hash(),
        prepared.stock_close_manifest_hash(),
        prepared.benchmark_manifest_hash(),
        prepared.calendar_authority_hash(),
        metric_value(report.gross()),
        metric_value(report.benchmark()),
        metric_value(report.gross_excess()),
        metric_value(report.net()),
        metric_value(report.net_excess()),
        report
            .gross_win_rate()
            .map_or_else(|| "不可用".to_owned(), |rate| rate.to_string()),
        serde_json::to_string(report.net_win_rate()).unwrap_or_else(|_| "不可用".to_owned()),
        serde_json::to_string(report.family_attribution())
            .unwrap_or_else(|_| "不可用".to_owned()),
        conclusion_value(report.conclusion())
    )
}

fn render_report_receipt(receipt: &AttributionReportReceipt, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => json!({
            "状态": "成功",
            "模式": "归因提交",
            "运行审计ID": receipt.run.run_audit_id,
            "报告修订ID": receipt.report_revision_id,
            "报告身份": receipt.report_identity,
            "证据身份": receipt.evidence_identity,
            "序列身份": receipt.series_identity,
            "结果哈希": receipt.result_payload_hash,
            "报告修订": receipt.report_revision,
            "前版报告ID": receipt.predecessor_report_id,
            "报告记录哈希": receipt.report_record_hash,
            "结论边界": "ResearchOnly"
        })
        .to_string(),
        OutputFormat::Markdown => format!(
            "# 归因提交回执\n\n- 状态：成功\n- 运行审计 ID：{}\n- 报告修订 ID：{}\n- 报告身份：{}\n- 证据身份：{}\n- 序列身份：{}\n- 结果哈希：{}\n- 报告修订：{}\n- 前版报告 ID：{}\n- 报告记录哈希：{}\n- 结论边界：ResearchOnly",
            receipt.run.run_audit_id,
            receipt.report_revision_id,
            receipt.report_identity,
            receipt.evidence_identity,
            receipt.series_identity,
            receipt.result_payload_hash,
            receipt.report_revision,
            receipt
                .predecessor_report_id
                .map_or_else(|| "无".to_owned(), |id| id.to_string()),
            receipt.report_record_hash
        ),
    }
}

fn map_calendar_error(error: VerifiedCalendarError) -> AppError {
    let class = match error.kind() {
        VerifiedCalendarErrorKind::InvalidRequest => AppErrorClass::Usage,
        VerifiedCalendarErrorKind::CurrentSessionIncomplete
        | VerifiedCalendarErrorKind::TradingCalendarUnavailable => AppErrorClass::Unavailable,
    };
    AppError {
        class,
        stage: "calendar",
        code: error.code().to_owned(),
        retryable: error.retryable(),
        failure_audit_id: None,
    }
}

fn map_benchmark_error(error: BenchmarkError) -> AppError {
    match error {
        BenchmarkError::Unsupported(_) => AppError {
            class: AppErrorClass::Usage,
            stage: "benchmark",
            code: "unsupported_benchmark_request".to_owned(),
            retryable: false,
            failure_audit_id: None,
        },
        BenchmarkError::Unavailable { code, retryable } => AppError {
            class: AppErrorClass::Unavailable,
            stage: "benchmark",
            code: code.to_owned(),
            retryable,
            failure_audit_id: None,
        },
        BenchmarkError::FailedIntegrity { code } => AppError {
            class: AppErrorClass::FailedIntegrity,
            stage: "benchmark",
            code: code.to_owned(),
            retryable: false,
            failure_audit_id: None,
        },
    }
}

fn map_database_error(error: AttributionReportStoreError) -> AppError {
    match error {
        AttributionReportStoreError::Unavailable {
            reason_code,
            retryable,
            ..
        } => AppError {
            class: AppErrorClass::Unavailable,
            stage: "database",
            code: reason_code.to_owned(),
            retryable,
            failure_audit_id: None,
        },
        AttributionReportStoreError::FailedIntegrity { reason_code, .. } => AppError {
            class: AppErrorClass::FailedIntegrity,
            stage: "database",
            code: reason_code.to_owned(),
            retryable: false,
            failure_audit_id: None,
        },
    }
}

fn replay_stage(stage: ReplayStage) -> &'static str {
    match stage {
        ReplayStage::Request => "request",
        ReplayStage::Calendar => "calendar",
        ReplayStage::TradeEvidence => "trade_evidence",
        ReplayStage::Benchmark => "benchmark",
        ReplayStage::Compute => "compute",
        ReplayStage::Store => "store",
    }
}

fn map_replay_error(error: ReplayError) -> AppError {
    let class = match error.class() {
        ReplayErrorClass::Unavailable => AppErrorClass::Unavailable,
        ReplayErrorClass::FailedIntegrity => AppErrorClass::FailedIntegrity,
        ReplayErrorClass::Storage => AppErrorClass::Storage,
    };
    AppError {
        class,
        stage: replay_stage(error.stage()),
        code: error.code().to_owned(),
        retryable: error.retryable(),
        failure_audit_id: error
            .failure_receipt()
            .map(|receipt| receipt.failure_audit_id),
    }
}

fn manifest_bindings(manifests: &[ManifestArg]) -> Vec<BenchmarkDayManifest> {
    manifests.iter().map(|item| item.0.clone()).collect()
}

fn command_format(command: &Command) -> OutputFormat {
    match command {
        Command::Resolve { format, .. }
        | Command::Probe { format, .. }
        | Command::Capture { format, .. }
        | Command::Scheduled { format, .. }
        | Command::Replay { format, .. }
        | Command::Quarter { format, .. } => *format,
    }
}

fn execute_replay(
    database_path: &Path,
    mode: ReplayMode,
    manifests: &[ManifestArg],
    commit: bool,
    format: OutputFormat,
) -> Result<String, AppError> {
    let access = if commit {
        AttributionDatabaseAccess::AppendOnly
    } else {
        AttributionDatabaseAccess::ReadOnly
    };
    let session =
        AttributionDatabaseSession::open(database_path, access).map_err(map_database_error)?;
    let loader = AttributionReplayLoader::new(session.database_path());
    let runner = AttributionReplayRunner::new(session.database(), loader);
    let request = ReplayRequest {
        mode,
        benchmark_day_manifests: manifest_bindings(manifests),
    };
    if commit {
        runner
            .commit(request)
            .map(|receipt| render_report_receipt(&receipt, format))
            .map_err(map_replay_error)
    } else {
        runner
            .preview(request)
            .map(|prepared| render_prepared_report(&prepared, format))
            .map_err(map_replay_error)
    }
}

async fn execute(cli: &Cli) -> Result<String, AppError> {
    validate_command(cli)?;
    match &cli.command {
        Command::Resolve { at, format } => {
            let invoked_at = invocation_time(*at)?;
            let calendar =
                resolve_verified_scheduled_replay(invoked_at).map_err(map_calendar_error)?;
            Ok(render_calendar(&calendar, invoked_at, *format))
        }
        Command::Probe { request, format } => {
            let request = parse_benchmark_request(request)?;
            let report = probe_benchmark_request(request)
                .await
                .map_err(map_benchmark_error)?;
            Ok(render_probe(&report, *format))
        }
        Command::Capture {
            request,
            db,
            commit,
            format,
        } => {
            let request = parse_benchmark_request(request)?;
            if !commit {
                return Ok(render_capture_preview(&request, *format));
            }
            let database_path = db
                .as_deref()
                .ok_or_else(|| AppError::usage("capture_commit_requires_database"))?;
            let session = AttributionDatabaseSession::open(
                database_path,
                AttributionDatabaseAccess::AppendOnly,
            )
            .map_err(map_database_error)?;
            let capture = BenchmarkCapture::new(session.database());
            let preview = capture
                .preview(request)
                .await
                .map_err(map_benchmark_error)?;
            let receipt = capture.commit(preview).map_err(map_benchmark_error)?;
            Ok(render_capture_receipt(&receipt, *format))
        }
        Command::Scheduled {
            db,
            at,
            manifests,
            commit,
            format,
        } => {
            let invoked_at = invocation_time(*at)?;
            execute_replay(
                db,
                ReplayMode::Scheduled { invoked_at },
                manifests,
                *commit,
                *format,
            )
        }
        Command::Replay {
            db,
            from,
            to,
            at,
            manifests,
            commit,
            format,
        } => {
            if from > to {
                return Err(AppError::usage("invalid_replay_range"));
            }
            let invoked_at = invocation_time(*at)?;
            execute_replay(
                db,
                ReplayMode::Range {
                    from: *from,
                    to: *to,
                    invoked_at,
                },
                manifests,
                *commit,
                *format,
            )
        }
        Command::Quarter {
            db,
            year,
            quarter,
            at,
            manifests,
            commit,
            format,
        } => {
            let invoked_at = invocation_time(*at)?;
            execute_replay(
                db,
                ReplayMode::Quarter {
                    year: *year,
                    quarter: *quarter,
                    invoked_at,
                },
                manifests,
                *commit,
                *format,
            )
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = command_format(&cli.command);
    match execute(&cli).await {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}", error.render(format));
            error.exit_code()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use clap::Parser;
    use rusqlite::Connection;

    use super::*;

    fn manifest() -> String {
        format!("2026-08-21={}", "a".repeat(64))
    }

    fn test_database_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "TEST_CODE_strategy_attribution_{label}_{}_{}.sqlite3",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("TEST_CODE clock")
                .as_nanos()
        ))
    }

    fn create_source_schema(path: &Path) {
        let connection = Connection::open(path).expect("TEST_CODE create source database");
        connection
            .execute_batch(
                "CREATE TABLE legacy_guard (value TEXT NOT NULL);
                 INSERT INTO legacy_guard VALUES ('TEST_CODE_preserve');
                 CREATE TABLE paper_trades (
                    id INTEGER PRIMARY KEY, plan_id TEXT NOT NULL UNIQUE,
                    code TEXT NOT NULL, name TEXT NOT NULL, direction TEXT NOT NULL,
                    price REAL NOT NULL, quantity INTEGER NOT NULL, status TEXT NOT NULL,
                    fill_price REAL, virtual_reason TEXT NOT NULL, ts TEXT NOT NULL
                 );
                 CREATE TABLE order_audit (
                    id INTEGER PRIMARY KEY, business_order_id TEXT NOT NULL,
                    source TEXT NOT NULL, decision_basis TEXT NOT NULL, side TEXT NOT NULL,
                    code TEXT NOT NULL, requested_price REAL NOT NULL, execution_price REAL,
                    quantity INTEGER NOT NULL, quote_observed_at TEXT, outcome TEXT NOT NULL,
                    failure_reason TEXT, created_at TEXT NOT NULL
                 );
                 CREATE TABLE order_audit_chain (
                    order_audit_id INTEGER PRIMARY KEY, previous_hash TEXT NOT NULL,
                    record_hash TEXT NOT NULL, created_at TEXT NOT NULL
                 );
                 CREATE TABLE stock_daily (
                    id INTEGER PRIMARY KEY, code TEXT NOT NULL, date TEXT NOT NULL,
                    close REAL, data_source TEXT, created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );",
            )
            .expect("TEST_CODE source schema");
    }

    fn sidecar(path: &Path, suffix: &str) -> PathBuf {
        PathBuf::from(format!("{}{suffix}", path.display()))
    }

    fn file_state(path: &Path) -> Option<(Vec<u8>, SystemTime)> {
        let metadata = fs::metadata(path).ok()?;
        Some((
            fs::read(path).expect("TEST_CODE read database state"),
            metadata.modified().expect("TEST_CODE modified time"),
        ))
    }

    fn database_state(path: &Path) -> [Option<(Vec<u8>, SystemTime)>; 3] {
        [
            file_state(path),
            file_state(&sidecar(path, "-wal")),
            file_state(&sidecar(path, "-shm")),
        ]
    }

    fn cleanup_database(path: &Path) {
        for target in [
            path.to_path_buf(),
            sidecar(path, "-wal"),
            sidecar(path, "-shm"),
        ] {
            if target.exists() {
                fs::remove_file(target).expect("TEST_CODE remove exact temporary database file");
            }
        }
    }

    fn scalar(connection: &Connection, sql: &str) -> i64 {
        connection
            .query_row(sql, [], |row| row.get(0))
            .expect("TEST_CODE scalar")
    }

    #[test]
    fn parser_exposes_six_commands_and_requires_explicit_database_and_manifests() {
        for command in [
            "resolve",
            "probe",
            "capture",
            "scheduled",
            "replay",
            "quarter",
        ] {
            let result = Cli::try_parse_from(["strategy_attribution", command, "--help"]);
            assert!(
                result.is_err(),
                "TEST_CODE {command} help exits through Clap"
            );
        }

        assert!(Cli::try_parse_from([
            "strategy_attribution",
            "scheduled",
            "--manifest",
            &manifest(),
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "strategy_attribution",
            "scheduled",
            "--db",
            "TEST_CODE.sqlite",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "strategy_attribution",
            "quarter",
            "--db",
            "TEST_CODE.sqlite",
            "--year",
            "2026",
            "--quarter",
            "5",
            "--manifest",
            &manifest(),
        ])
        .is_err());
    }

    #[test]
    fn capture_commit_requires_database_and_preview_rejects_unused_database() {
        let preview = Cli::try_parse_from([
            "strategy_attribution",
            "capture",
            "--instrument",
            "sh000300",
            "--granularity",
            "daily",
            "--from",
            "2026-08-21",
            "--to",
            "2026-08-21",
        ])
        .expect("TEST_CODE capture preview grammar");
        validate_command(&preview).expect("TEST_CODE capture defaults to no-writer preview");

        let missing_database = Cli::try_parse_from([
            "strategy_attribution",
            "capture",
            "--instrument",
            "sh000300",
            "--granularity",
            "daily",
            "--from",
            "2026-08-21",
            "--to",
            "2026-08-21",
            "--commit",
        ])
        .expect("TEST_CODE commit grammar parses before semantic validation");
        assert!(validate_command(&missing_database).is_err());

        let unused_database = Cli::try_parse_from([
            "strategy_attribution",
            "capture",
            "--instrument",
            "sh000300",
            "--granularity",
            "daily",
            "--from",
            "2026-08-21",
            "--to",
            "2026-08-21",
            "--db",
            "TEST_CODE.sqlite",
        ])
        .expect("TEST_CODE unused DB grammar");
        assert!(validate_command(&unused_database).is_err());

        assert!(Cli::try_parse_from([
            "strategy_attribution",
            "probe",
            "--instrument",
            "sh000300",
            "--granularity",
            "daily",
            "--from",
            "2026-08-21",
            "--to",
            "2026-08-21",
            "--commit",
        ])
        .is_err());
    }

    #[tokio::test]
    async fn capture_preview_is_canonical_and_does_not_open_a_database_or_provider() {
        let cli = Cli::try_parse_from([
            "strategy_attribution",
            "capture",
            "--granularity",
            "daily",
            "--from",
            "2026-03-31",
            "--to",
            "2026-04-01",
            "--format",
            "json",
        ])
        .expect("TEST_CODE canonical preview command");
        let output = execute(&cli).await.expect("TEST_CODE preview succeeds");
        let value: Value = serde_json::from_str(&output).expect("TEST_CODE Chinese JSON");
        assert_eq!(value["数据库已连接"], false);
        assert_eq!(value["真实适配器已调用"], false);
        assert_eq!(value["writer已初始化"], false);
        assert_eq!(value["预期自然季度"], json!(["2026-Q1", "2026-Q2"]));
    }

    #[tokio::test]
    async fn preview_is_byte_immutable_and_commit_only_appends_allowed_audit_rows() {
        let path = test_database_path("readonly_commit_boundary");
        create_source_schema(&path);
        drop(
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::AppendOnly)
                .expect("TEST_CODE install only narrow schemas"),
        );
        let before = database_state(&path);
        let manifest = manifest();
        let path_text = path.to_string_lossy().into_owned();

        let preview = Cli::try_parse_from([
            "strategy_attribution",
            "scheduled",
            "--db",
            path_text.as_str(),
            "--at",
            "2026-08-21T15:30:00+08:00",
            "--manifest",
            manifest.as_str(),
            "--format",
            "json",
        ])
        .expect("TEST_CODE scheduled preview command");
        let preview_error = execute(&preview)
            .await
            .expect_err("TEST_CODE absent benchmark must fail explicitly");
        assert!(matches!(
            preview_error.class,
            AppErrorClass::Unavailable | AppErrorClass::FailedIntegrity
        ));
        assert_eq!(database_state(&path), before);

        let commit = Cli::try_parse_from([
            "strategy_attribution",
            "scheduled",
            "--db",
            path_text.as_str(),
            "--at",
            "2026-08-21T15:30:00+08:00",
            "--manifest",
            manifest.as_str(),
            "--commit",
            "--format",
            "json",
        ])
        .expect("TEST_CODE scheduled commit command");
        let commit_error = execute(&commit)
            .await
            .expect_err("TEST_CODE failed run appends a failure audit, not a report");
        assert!(commit_error.failure_audit_id.is_some());

        let connection = Connection::open(&path).expect("TEST_CODE inspect commit boundary");
        assert_eq!(
            scalar(&connection, "SELECT COUNT(*) FROM attribution_run_audit"),
            1
        );
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM attribution_failure_audit"
            ),
            1
        );
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM attribution_report_revision"
            ),
            0
        );
        for table in [
            "paper_trades",
            "order_audit",
            "order_audit_chain",
            "stock_daily",
        ] {
            assert_eq!(
                scalar(&connection, &format!("SELECT COUNT(*) FROM {table}")),
                0,
                "TEST_CODE source table remains unchanged: {table}"
            );
        }
        let sentinel: String = connection
            .query_row("SELECT value FROM legacy_guard", [], |row| row.get(0))
            .expect("TEST_CODE legacy sentinel");
        assert_eq!(sentinel, "TEST_CODE_preserve");
        drop(connection);
        cleanup_database(&path);
    }
}
