//! BR-251 默认只读的中文买卖策略历史归因 CLI。

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{json, Value};
use stock_analysis::calendar::{
    resolve_verified_replay_range, resolve_verified_scheduled_replay, VerifiedCalendarError,
    VerifiedCalendarErrorKind, VerifiedReplayCalendar,
};
use stock_analysis::data_gateway::{
    probe_benchmark_request, BenchmarkCapture, BenchmarkError, BenchmarkProbeReport,
    BenchmarkRange, BenchmarkRequest, HS300_CANONICAL,
};
use stock_analysis::database::attribution_epochs::{
    AttributionEpochReceipt, AttributionEpochStore, AttributionEpochStoreError,
    EpochActivationPreview, EpochActivationRequest,
};
use stock_analysis::database::attribution_reports::{
    AttributionDatabaseAccess, AttributionDatabaseSession, AttributionReportReceipt,
    AttributionReportStoreError,
};
use stock_analysis::performance::attribution_epoch::{
    canonical_legacy_carry_manifest_hash, AttributionEpochSelector, EpochActivationSource,
};
use stock_analysis::performance::attribution_replay::{
    AttributionConclusion, AttributionReplayLoader, AttributionReplayRunner, BenchmarkDayManifest,
    CommittedAttributionReport, MetricAggregate, PreparedAttributionReport, ReplayError,
    ReplayErrorClass, ReplayMode, ReplayRequest, ReplayStage,
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
    /// 只读调用真实指数 raw adapter；不进入正式准入，不写审计或数据库。
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
    /// 预览归因样本 epoch；仅显式 commit 才冻结并追加激活凭据。
    ResetSample {
        #[arg(long)]
        db: PathBuf,
        /// 显式 +08:00 激活时刻；省略时使用当前上海时刻。
        #[arg(long)]
        at: Option<DateTime<FixedOffset>>,
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
        #[arg(long, default_value = "active")]
        epoch: EpochSelectorArg,
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
        #[arg(long, default_value = "active")]
        epoch: EpochSelectorArg,
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
        #[arg(long, default_value = "active")]
        epoch: EpochSelectorArg,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct EpochSelectorArg(AttributionEpochSelector);

impl FromStr for EpochSelectorArg {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "active" => Ok(Self(AttributionEpochSelector::Active)),
            "legacy" => Ok(Self(AttributionEpochSelector::Legacy)),
            hash if hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
            {
                Ok(Self(AttributionEpochSelector::Exact(hash.to_owned())))
            }
            _ => Err("epoch 必须为 active、legacy 或64位小写sha256".to_owned()),
        }
    }
}

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

    fn output_integrity(code: impl Into<String>) -> Self {
        Self {
            class: AppErrorClass::FailedIntegrity,
            stage: "output",
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
            "模式": "只读原始协议诊断探针",
            "请求哈希": report.request_hash,
            "身份锚点": {
                "规范基准": report.identity_anchor.canonical_instrument,
                "provider_market": report.identity_anchor.provider_market,
                "provider_code": report.identity_anchor.provider_code,
                "provider_category": report.identity_anchor.provider_category,
                "adjustment_mode": report.identity_anchor.adjustment_mode,
                "provider身份回显": report.identity_anchor.provider_identity_echo,
                "身份核验状态": report.identity_anchor.identity_verification
            },
            "粒度": format!("{:?}", report.granularity),
            "来源提供方": report.provider,
            "来源": report.source,
            "来源时刻": report.source_at,
            "观察时刻": report.observed_at,
            "provider报告总数": report.provider_reported_total,
            "分页轨迹": report.pages,
            "原始记录总数": report.raw_total_count,
            "请求范围内原始记录数": report.raw_in_range_count,
            "首原始标签": report.first_raw_label,
            "末原始标签": report.last_raw_label,
            "原始OHLC摘要": report.raw_ohlc_digest,
            "分钟标签语义": report.minute_label_semantics,
            "分钟原始时间样本": report.minute_raw_time_samples,
            "协议版本": report.protocol_revision,
            "正式准入批次已构造": false,
            "Reader能力已签发": false,
            "BR-159审计已写入": false,
            "attestation已更新": false,
            "数据库已写入": false
        })
        .to_string(),
        OutputFormat::Markdown => {
            let pages = report
                .pages
                .iter()
                .map(|page| {
                    format!(
                        "  - offset={} requested={} received={} raw_labels={} / {}",
                        page.offset,
                        page.requested,
                        page.received,
                        page.first_raw_label,
                        page.last_raw_label
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let minute_samples = if report.minute_raw_time_samples.is_empty() {
                "不适用".to_owned()
            } else {
                report
                    .minute_raw_time_samples
                    .iter()
                    .map(|sample| {
                        format!(
                            "{} [{:04}-{:02}-{:02} {:02}:{:02}]",
                            sample.raw_label,
                            sample.year,
                            sample.month,
                            sample.day,
                            sample.hour,
                            sample.minute
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("、")
            };
            format!(
            "# 基准原始协议只读诊断\n\n- 状态：成功\n- 规范基准：{}\n- Provider 参数：market={} code={} category={} adjustment={}\n- Provider 身份回显：{}\n- 身份核验状态：{}\n- 粒度：{:?}\n- 请求哈希：{}\n- 来源：{} / {}\n- 来源时刻：{}\n- 观察时刻：{}\n- Provider 报告总数：{}\n- 原始记录总数：{}（请求范围内 {}）\n- 首末原始标签：{} / {}\n- 原始 OHLC 摘要：{}\n- 分钟标签语义：{}\n- 分钟原始时间样本：{}\n- 协议版本：{}\n\n## 分页轨迹\n\n{}\n\n- 正式准入批次已构造：否\n- Reader 能力已签发：否\n- BR-159 审计已写入：否\n- attestation 已更新：否\n- 数据库已写入：否",
            report.identity_anchor.canonical_instrument,
            report.identity_anchor.provider_market,
            report.identity_anchor.provider_code,
            report.identity_anchor.provider_category,
            report.identity_anchor.adjustment_mode,
            report
                .identity_anchor
                .provider_identity_echo
                .as_deref()
                .unwrap_or("缺失"),
            report.identity_anchor.identity_verification.as_str(),
            report.granularity,
            report.request_hash,
            report.provider,
            report.source,
            report.source_at.as_deref().unwrap_or("缺失"),
            report.observed_at,
            report
                .provider_reported_total
                .map_or_else(|| "缺失".to_owned(), |total| total.to_string()),
            report.raw_total_count,
            report.raw_in_range_count,
            report.first_raw_label,
            report.last_raw_label,
            report.raw_ohlc_digest,
            report.minute_label_semantics.as_str(),
            minute_samples,
            report.protocol_revision,
            pages
        )
        }
    }
}

fn render_capture_preview(
    request: &BenchmarkRequest,
    format: OutputFormat,
) -> Result<String, AppError> {
    let quarters = request
        .natural_quarter_labels()
        .map_err(map_benchmark_error)?;
    Ok(match format {
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
    })
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

fn render_reset_sample(
    preview: &EpochActivationPreview,
    calendar_authority_hash: &str,
    receipt_hash: Option<&str>,
    database_written: bool,
    format: OutputFormat,
) -> Result<String, AppError> {
    let carry_manifest_hash = canonical_legacy_carry_manifest_hash(&preview.carry);
    Ok(match format {
        OutputFormat::Json => json!({
            "状态": "ResearchOnly",
            "模式": if database_written { "归因样本重置提交" } else { "归因样本重置预览" },
            "数据库已写入": database_written,
            "已存在激活": preview.activated,
            "epoch": {
                "身份": preview.epoch_id,
                "completed交易日": preview.completed_session_date,
                "effective交易日": preview.effective_date
            },
            "冻结源": {
                "paper_trade高水位": preview.paper_trade_high_water,
                "order_audit高水位": preview.order_audit_high_water,
                "legacy成交manifest哈希": preview.legacy_filled_manifest_hash,
                "terminal绑定manifest哈希": preview.terminal_binding_manifest_hash,
                "order_audit_tip哈希": preview.order_audit_tip_hash,
                "position投影哈希": preview.position_projection_hash
            },
            "carry": {
                "manifest哈希": carry_manifest_hash,
                "代码与股数": output_value(&preview.carry, "epoch_carry_serialization_failed")?
            },
            "日历权威哈希": calendar_authority_hash,
            "receipt身份": receipt_hash,
            "研究边界": "ResearchOnly：仅用于归因样本隔离，不构成策略成功、交易或下单结论"
        })
        .to_string(),
        OutputFormat::Markdown => {
            let carry = if preview.carry.is_empty() {
                "无".to_owned()
            } else {
                preview
                    .carry
                    .iter()
                    .map(|position| format!("{}={}", position.code, position.quantity))
                    .collect::<Vec<_>>()
                    .join("、")
            };
            format!(
                "# 归因样本重置{}\n\n- 状态：ResearchOnly\n- 数据库已写入：{}\n- 已存在激活：{}\n- Epoch 身份：{}\n- Completed 交易日：{}\n- Effective 交易日：{}\n- Paper trade 高水位：{}\n- Order audit 高水位：{}\n- Legacy 成交 manifest 哈希：{}\n- Terminal 绑定 manifest 哈希：{}\n- Order audit tip 哈希：{}\n- Position 投影哈希：{}\n- Carry manifest 哈希：{}\n- Carry 代码/股数：{}\n- 日历权威哈希：{}\n- Receipt 身份：{}\n- 研究边界：ResearchOnly，仅用于归因样本隔离，不构成策略成功、交易或下单结论",
                if database_written { "提交" } else { "预览" },
                if database_written { "是" } else { "否" },
                if preview.activated { "是" } else { "否" },
                preview.epoch_id,
                preview.completed_session_date,
                preview.effective_date,
                preview.paper_trade_high_water,
                preview.order_audit_high_water,
                preview.legacy_filled_manifest_hash,
                preview.terminal_binding_manifest_hash,
                preview.order_audit_tip_hash,
                preview.position_projection_hash,
                carry_manifest_hash,
                carry,
                calendar_authority_hash,
                receipt_hash.unwrap_or("未提交")
            )
        }
    })
}

fn output_value<T: Serialize + ?Sized>(value: &T, code: &'static str) -> Result<Value, AppError> {
    serde_json::to_value(value).map_err(|_| AppError::output_integrity(code))
}

fn output_object<const N: usize>(fields: [(&'static str, Value); N]) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn metric_value(metric: &MetricAggregate) -> Result<Value, AppError> {
    Ok(output_object([
        (
            "总周期",
            output_value(&metric.total_cycles, "report_metric_serialization_failed")?,
        ),
        (
            "可用周期",
            output_value(
                &metric.available_cycles,
                "report_metric_serialization_failed",
            )?,
        ),
        (
            "不可用周期",
            output_value(
                &metric.unavailable_cycles,
                "report_metric_serialization_failed",
            )?,
        ),
        (
            "覆盖率",
            output_value(&metric.coverage_ratio, "report_metric_serialization_failed")?,
        ),
        (
            "不可用原因",
            output_value(
                &metric.unavailable_reasons,
                "report_metric_serialization_failed",
            )?,
        ),
        (
            "收益和",
            output_value(&metric.sum_return, "report_metric_serialization_failed")?,
        ),
        (
            "平均收益",
            output_value(&metric.mean_return, "report_metric_serialization_failed")?,
        ),
        (
            "中位收益",
            output_value(&metric.median_return, "report_metric_serialization_failed")?,
        ),
    ]))
}

fn conclusion_value(conclusion: &AttributionConclusion) -> Result<Value, AppError> {
    match conclusion {
        AttributionConclusion::InsufficientSample {
            reasons,
            research_limitations,
        } => Ok(output_object([
            ("样本门状态", Value::String("InsufficientSample".to_owned())),
            (
                "原因",
                output_value(reasons, "report_conclusion_serialization_failed")?,
            ),
            (
                "研究限制",
                output_value(
                    research_limitations,
                    "report_conclusion_serialization_failed",
                )?,
            ),
        ])),
        AttributionConclusion::ResearchOnly {
            research_limitations,
        } => Ok(output_object([
            ("样本门状态", Value::String("ResearchOnly".to_owned())),
            ("原因", Value::Array(Vec::new())),
            (
                "研究限制",
                output_value(
                    research_limitations,
                    "report_conclusion_serialization_failed",
                )?,
            ),
        ])),
    }
}

fn prepared_report_value(prepared: &PreparedAttributionReport) -> Result<Value, AppError> {
    let invocation = prepared.invocation();
    let report = prepared.report();
    let run = output_object([
        ("模式", Value::String(format!("{:?}", invocation.mode))),
        (
            "运行时刻",
            Value::String(invocation.invoked_at.to_rfc3339()),
        ),
        (
            "目标起始",
            Value::String(invocation.target_from.to_string()),
        ),
        ("目标结束", Value::String(invocation.target_to.to_string())),
        ("规则版本", Value::String(invocation.rule_version.clone())),
    ]);
    let manifests = output_object([
        (
            "成交manifest哈希",
            Value::String(prepared.trade_manifest_hash().to_owned()),
        ),
        (
            "个股收盘manifest哈希",
            Value::String(prepared.stock_close_manifest_hash().to_owned()),
        ),
        (
            "基准组合manifest哈希",
            Value::String(prepared.benchmark_manifest_hash().to_owned()),
        ),
        (
            "基准逐日manifest",
            output_value(
                prepared.benchmark_day_manifests(),
                "report_manifest_serialization_failed",
            )?,
        ),
        (
            "日历权威哈希",
            Value::String(prepared.calendar_authority_hash().to_owned()),
        ),
    ]);
    let samples = output_object([
        (
            "来源成交数",
            output_value(
                &report.source_fill_ids().len(),
                "report_sample_serialization_failed",
            )?,
        ),
        (
            "总周期",
            output_value(
                &(report.total_closed_cycles() + report.total_open_cycles()),
                "report_sample_serialization_failed",
            )?,
        ),
        (
            "闭合周期",
            output_value(
                &report.total_closed_cycles(),
                "report_sample_serialization_failed",
            )?,
        ),
        (
            "开放周期_右删失",
            output_value(
                &report.total_open_cycles(),
                "report_sample_serialization_failed",
            )?,
        ),
        (
            "覆盖自然日",
            output_value(
                &report.coverage_days(),
                "report_sample_serialization_failed",
            )?,
        ),
    ]);
    let metrics = output_object([
        ("毛收益", metric_value(report.gross())?),
        ("基准收益", metric_value(report.benchmark())?),
        ("毛超额收益", metric_value(report.gross_excess())?),
        ("净收益", metric_value(report.net())?),
        ("净超额收益", metric_value(report.net_excess())?),
        (
            "毛胜率",
            output_value(
                &report.gross_win_rate(),
                "report_outcome_serialization_failed",
            )?,
        ),
        (
            "净胜率可用性",
            output_value(report.net_win_rate(), "report_outcome_serialization_failed")?,
        ),
        (
            "毛胜负分母",
            output_value(
                report.gross_outcome(),
                "report_outcome_serialization_failed",
            )?,
        ),
        (
            "净胜负分母",
            output_value(report.net_outcome(), "report_outcome_serialization_failed")?,
        ),
        (
            "费用证据可用性",
            output_value(report.fee_basis(), "report_outcome_serialization_failed")?,
        ),
    ]);
    Ok(output_object([
        ("状态", Value::String("ResearchOnly".to_owned())),
        (
            "警告",
            Value::String("本报告只用于研究，不构成策略成功、交易或下单结论".to_owned()),
        ),
        ("运行", run),
        ("证据清单", manifests),
        ("样本", samples),
        ("指标", metrics),
        (
            "按入场族归因",
            output_value(
                report.family_attribution(),
                "report_family_serialization_failed",
            )?,
        ),
        (
            "数据质量",
            output_object([
                ("不可用原因已计入总分母", Value::Bool(true)),
                ("开放周期按右删失展示", Value::Bool(true)),
                ("缺失字段未补零", Value::Bool(true)),
            ]),
        ),
        ("样本门与结论", conclusion_value(report.conclusion())?),
    ]))
}

fn render_prepared_report(
    prepared: &PreparedAttributionReport,
    format: OutputFormat,
) -> Result<String, AppError> {
    let value = prepared_report_value(prepared)?;
    if matches!(format, OutputFormat::Json) {
        return Ok(value.to_string());
    }
    let report = prepared.report();
    let invocation = prepared.invocation();
    let net_win_rate = output_value(
        report.net_win_rate(),
        "report_net_win_rate_serialization_failed",
    )?
    .to_string();
    let family_attribution = output_value(
        report.family_attribution(),
        "report_family_serialization_failed",
    )?
    .to_string();
    Ok(format!(
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
        metric_value(report.gross())?,
        metric_value(report.benchmark())?,
        metric_value(report.gross_excess())?,
        metric_value(report.net())?,
        metric_value(report.net_excess())?,
        report
            .gross_win_rate()
            .map_or_else(|| "不可用".to_owned(), |rate| rate.to_string()),
        net_win_rate,
        family_attribution,
        conclusion_value(report.conclusion())?
    ))
}

fn report_receipt_value(receipt: &AttributionReportReceipt) -> Result<Value, AppError> {
    Ok(output_object([
        ("状态", Value::String("成功".to_owned())),
        ("模式", Value::String("归因提交".to_owned())),
        (
            "运行审计ID",
            output_value(
                &receipt.run.run_audit_id,
                "report_receipt_serialization_failed",
            )?,
        ),
        (
            "报告修订ID",
            output_value(
                &receipt.report_revision_id,
                "report_receipt_serialization_failed",
            )?,
        ),
        ("报告身份", Value::String(receipt.report_identity.clone())),
        ("证据身份", Value::String(receipt.evidence_identity.clone())),
        ("序列身份", Value::String(receipt.series_identity.clone())),
        (
            "结果哈希",
            Value::String(receipt.result_payload_hash.clone()),
        ),
        (
            "报告修订",
            output_value(
                &receipt.report_revision,
                "report_receipt_serialization_failed",
            )?,
        ),
        (
            "前版报告ID",
            output_value(
                &receipt.predecessor_report_id,
                "report_receipt_serialization_failed",
            )?,
        ),
        (
            "报告记录哈希",
            Value::String(receipt.report_record_hash.clone()),
        ),
        ("结论边界", Value::String("ResearchOnly".to_owned())),
    ]))
}

fn render_report_receipt(
    receipt: &AttributionReportReceipt,
    format: OutputFormat,
) -> Result<String, AppError> {
    Ok(match format {
        OutputFormat::Json => report_receipt_value(receipt)?.to_string(),
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
    })
}

fn render_committed_report(
    committed: &CommittedAttributionReport,
    format: OutputFormat,
) -> Result<String, AppError> {
    match format {
        OutputFormat::Json => Ok(output_object([
            ("报告", prepared_report_value(committed.prepared())?),
            ("提交回执", report_receipt_value(committed.receipt())?),
        ])
        .to_string()),
        OutputFormat::Markdown => Ok(format!(
            "{}\n\n{}",
            render_prepared_report(committed.prepared(), format)?,
            render_report_receipt(committed.receipt(), format)?
        )),
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

fn map_epoch_store_error(error: AttributionEpochStoreError) -> AppError {
    match error {
        AttributionEpochStoreError::Unavailable {
            reason_code,
            retryable,
            ..
        } => AppError {
            class: AppErrorClass::Unavailable,
            stage: "epoch",
            code: reason_code.to_owned(),
            retryable,
            failure_audit_id: None,
        },
        AttributionEpochStoreError::FailedIntegrity { reason_code, .. } => AppError {
            class: AppErrorClass::FailedIntegrity,
            stage: "epoch",
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
        ReplayStage::Epoch => "epoch",
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

fn load_reset_sample_preview(
    database_path: &Path,
    request: &EpochActivationRequest,
) -> Result<EpochActivationPreview, AppError> {
    AttributionEpochStore::preview_activation_at_path(database_path, request)
        .map_err(map_epoch_store_error)
}

fn execute_reset_sample_preview(
    database_path: &Path,
    request: &EpochActivationRequest,
    format: OutputFormat,
) -> Result<String, AppError> {
    let preview = load_reset_sample_preview(database_path, request)?;
    let calendar =
        resolve_verified_replay_range(preview.completed_session_date, preview.effective_date)
            .map_err(map_calendar_error)?;
    render_reset_sample(&preview, calendar.authority_hash(), None, false, format)
}

fn preview_matches_receipt(
    preview: &EpochActivationPreview,
    receipt: &AttributionEpochReceipt,
) -> Result<(), AppError> {
    let carry_item_count = u64::try_from(preview.carry.len())
        .map_err(|_| AppError::output_integrity("epoch_carry_count_overflow"))?;
    let carry_total_quantity = preview.carry.iter().try_fold(0_u64, |total, position| {
        total.checked_add(position.quantity)
    });
    let carry_total_quantity = carry_total_quantity
        .ok_or_else(|| AppError::output_integrity("epoch_carry_quantity_overflow"))?;
    if !preview.activated
        || preview.epoch_id != receipt.epoch_id
        || preview.completed_session_date != receipt.cutover_completed_trading_date
        || preview.effective_date != receipt.effective_trading_date
        || preview.paper_trade_high_water != receipt.paper_trade_high_water
        || preview.order_audit_high_water != receipt.order_audit_high_water
        || preview.legacy_filled_manifest_hash != receipt.legacy_filled_manifest_hash
        || preview.terminal_binding_manifest_hash != receipt.terminal_binding_manifest_hash
        || preview.order_audit_tip_hash != receipt.order_audit_tip_hash
        || preview.position_projection_hash != receipt.position_projection_hash
        || canonical_legacy_carry_manifest_hash(&preview.carry)
            != receipt.legacy_carry_manifest_hash
        || carry_item_count != receipt.carry_item_count
        || carry_total_quantity != receipt.carry_total_quantity
    {
        return Err(AppError::output_integrity("epoch_commit_readback_mismatch"));
    }
    Ok(())
}

fn execute_reset_sample_commit(
    database_path: &Path,
    request: &EpochActivationRequest,
    format: OutputFormat,
) -> Result<String, AppError> {
    let session =
        AttributionDatabaseSession::open(database_path, AttributionDatabaseAccess::AppendOnly)
            .map_err(map_database_error)?;
    let receipt = AttributionEpochStore::new(session.database())
        .activate_once(request.clone())
        .map_err(map_epoch_store_error)?;
    drop(session);

    let preview = load_reset_sample_preview(database_path, request)?;
    preview_matches_receipt(&preview, &receipt)?;
    render_reset_sample(
        &preview,
        &receipt.calendar_authority_hash,
        Some(&receipt.receipt_hash),
        true,
        format,
    )
}

fn command_format(command: &Command) -> OutputFormat {
    match command {
        Command::Resolve { format, .. }
        | Command::Probe { format, .. }
        | Command::Capture { format, .. }
        | Command::ResetSample { format, .. }
        | Command::Scheduled { format, .. }
        | Command::Replay { format, .. }
        | Command::Quarter { format, .. } => *format,
    }
}

fn execute_replay(
    database_path: &Path,
    mode: ReplayMode,
    manifests: &[ManifestArg],
    epoch: AttributionEpochSelector,
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
        epoch,
        benchmark_day_manifests: manifest_bindings(manifests),
    };
    if commit {
        let committed = runner
            .commit_with_report(request)
            .map_err(map_replay_error)?;
        render_committed_report(&committed, format)
    } else {
        let prepared = runner.preview(request).map_err(map_replay_error)?;
        render_prepared_report(&prepared, format)
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
                return render_capture_preview(&request, *format);
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
        Command::ResetSample {
            db,
            at,
            commit,
            format,
        } => {
            let invoked_at = invocation_time(*at)?;
            let request = EpochActivationRequest {
                source: EpochActivationSource::Cli,
                invoked_at,
            };
            if *commit {
                return execute_reset_sample_commit(db, &request, *format);
            }
            execute_reset_sample_preview(db, &request, *format)
        }
        Command::Scheduled {
            db,
            at,
            manifests,
            epoch,
            commit,
            format,
        } => {
            let invoked_at = invocation_time(*at)?;
            execute_replay(
                db,
                ReplayMode::Scheduled { invoked_at },
                manifests,
                epoch.0.clone(),
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
            epoch,
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
                epoch.0.clone(),
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
            epoch,
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
                epoch.0.clone(),
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
                    fill_price REAL, not_fill_reason TEXT, virtual_reason TEXT NOT NULL,
                    account_mode TEXT NOT NULL, data_mode TEXT NOT NULL,
                    ts TEXT NOT NULL, updated_at TEXT NOT NULL
                 );
                 CREATE TABLE order_audit (
                    id INTEGER PRIMARY KEY, business_order_id TEXT NOT NULL,
                    source TEXT NOT NULL, decision_basis TEXT NOT NULL, side TEXT NOT NULL,
                    code TEXT NOT NULL, requested_price REAL NOT NULL, execution_price REAL,
                    quantity INTEGER NOT NULL, quote_observed_at TEXT, outcome TEXT NOT NULL,
                    failure_reason TEXT,
                    created_at TEXT NOT NULL DEFAULT '2026-08-27 02:00:01'
                 );
                 CREATE TABLE order_audit_chain (
                    order_audit_id INTEGER PRIMARY KEY, previous_hash TEXT NOT NULL,
                    record_hash TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT '2026-08-27 02:00:01'
                 );
                 CREATE TABLE stock_daily (
                    id INTEGER PRIMARY KEY, code TEXT NOT NULL, date TEXT NOT NULL,
                    close REAL, data_source TEXT, created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );",
            )
            .expect("TEST_CODE source schema");
    }

    fn append_activation_carry_source(path: &Path) {
        use stock_analysis::database::order_audit::OrderAuditRecord;

        let connection = Connection::open(path).expect("TEST_CODE activation source database");
        connection
            .execute(
                "INSERT INTO paper_trades
                 (id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,
                  virtual_reason,account_mode,data_mode,ts,updated_at)
                 VALUES (1,'TEST_CODE_PLAN_BUY','TEST_CODE_600001','TEST_CODE company','buy',
                         10.0,100,'Filled',10.0,NULL,'TEST_CODE activation','Normal','Full',
                         '2026-08-27 02:00:00','2026-08-27 02:00:00')",
                [],
            )
            .expect("TEST_CODE legacy paper fill");
        drop(connection);

        let session = AttributionDatabaseSession::open(path, AttributionDatabaseAccess::AppendOnly)
            .expect("TEST_CODE activation source session");
        session
            .database()
            .record_order_audit(&OrderAuditRecord {
                business_order_id: "TEST_CODE_PLAN_BUY",
                source: "PaperTrade",
                decision_basis: "TEST_CODE activation",
                side: "buy",
                code: "TEST_CODE_600001",
                requested_price: 10.0,
                execution_price: Some(10.0),
                quantity: 100,
                quote_observed_at: Some("2026-08-27T10:00:00+08:00"),
                outcome: "Filled",
                failure_reason: None,
            })
            .expect("TEST_CODE canonical terminal audit chain");
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

    struct FailingSerialize;

    impl Serialize for FailingSerialize {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom(
                "TEST_CODE intentional serialization failure",
            ))
        }
    }

    #[test]
    fn report_serialization_failure_is_a_stable_output_error_instead_of_a_panic() {
        let error = output_value(&FailingSerialize, "report_test_serialization_failed")
            .expect_err("TEST_CODE serialization failure must remain explicit");

        assert_eq!(error.class, AppErrorClass::FailedIntegrity);
        assert_eq!(error.stage, "output");
        assert_eq!(error.code, "report_test_serialization_failed");
        assert!(!error.retryable);
    }

    #[test]
    fn raw_probe_render_exposes_protocol_evidence_without_claiming_admission() {
        use stock_analysis::data_gateway::{
            BenchmarkGranularity, BenchmarkRawIdentityAnchor, BenchmarkRawIdentityVerification,
            BenchmarkRawMinuteLabelSemantics, BenchmarkRawPageTrace, BenchmarkRawTimeSample,
        };

        let report = BenchmarkProbeReport {
            request_hash: "1".repeat(64),
            identity_anchor: BenchmarkRawIdentityAnchor {
                canonical_instrument: "TEST_CODE_000300".to_owned(),
                provider_market: 1,
                provider_code: "000300",
                provider_category: 8,
                adjustment_mode: 0,
                provider_identity_echo: None,
                identity_verification: BenchmarkRawIdentityVerification::Unverified,
            },
            granularity: BenchmarkGranularity::Minute1,
            provider: "Tdx".to_owned(),
            source: "TEST_CODE_magic-tdx-index-bars".to_owned(),
            source_at: None,
            observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
            provider_reported_total: None,
            pages: vec![BenchmarkRawPageTrace {
                offset: 0,
                requested: 800,
                received: 1,
                first_raw_label: "2026-08-21 09:31".to_owned(),
                last_raw_label: "2026-08-21 09:31".to_owned(),
            }],
            raw_total_count: 1,
            raw_in_range_count: 1,
            first_raw_label: "2026-08-21 09:31".to_owned(),
            last_raw_label: "2026-08-21 09:31".to_owned(),
            raw_ohlc_digest: "2".repeat(64),
            minute_label_semantics: BenchmarkRawMinuteLabelSemantics::Unverified,
            minute_raw_time_samples: vec![BenchmarkRawTimeSample {
                raw_label: "2026-08-21 09:31".to_owned(),
                year: 2026,
                month: 8,
                day: 21,
                hour: 9,
                minute: 31,
            }],
            protocol_revision: "TEST_CODE_revision",
        };

        let value: Value = serde_json::from_str(&render_probe(&report, OutputFormat::Json))
            .expect("TEST_CODE raw probe JSON");
        assert_eq!(value["身份锚点"]["规范基准"], "TEST_CODE_000300");
        assert_eq!(value["身份锚点"]["身份核验状态"], "unverified");
        assert_eq!(value["分页轨迹"][0]["offset"], 0);
        assert_eq!(value["分页轨迹"][0]["requested"], 800);
        assert_eq!(value["分页轨迹"][0]["received"], 1);
        assert_eq!(value["原始记录总数"], 1);
        assert_eq!(value["请求范围内原始记录数"], 1);
        assert_eq!(value["原始OHLC摘要"], "2".repeat(64));
        assert_eq!(value["分钟标签语义"], "unverified");
        assert_eq!(
            value["分钟原始时间样本"][0]["raw_label"],
            "2026-08-21 09:31"
        );
        assert_eq!(value["正式准入批次已构造"], false);
        assert_eq!(value["BR-159审计已写入"], false);
        assert_eq!(value["attestation已更新"], false);
        assert_eq!(value["数据库已写入"], false);
    }

    #[test]
    fn parser_exposes_seven_commands_and_requires_explicit_database_and_manifests() {
        for command in [
            "resolve",
            "probe",
            "capture",
            "reset-sample",
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
    fn replay_commands_default_to_active_and_accept_only_stable_epoch_selectors() {
        let scheduled = Cli::try_parse_from([
            "strategy_attribution",
            "scheduled",
            "--db",
            "TEST_CODE.sqlite",
            "--manifest",
            &manifest(),
        ])
        .expect("TEST_CODE scheduled active selector default");
        assert!(matches!(
            scheduled.command,
            Command::Scheduled {
                epoch: EpochSelectorArg(AttributionEpochSelector::Active),
                ..
            }
        ));

        for (raw, expected) in [
            ("legacy".to_owned(), AttributionEpochSelector::Legacy),
            (
                "a".repeat(64),
                AttributionEpochSelector::Exact("a".repeat(64)),
            ),
        ] {
            let parsed = Cli::try_parse_from([
                "strategy_attribution",
                "replay",
                "--db",
                "TEST_CODE.sqlite",
                "--from",
                "2026-08-21",
                "--to",
                "2026-08-21",
                "--manifest",
                &manifest(),
                "--epoch",
                raw.as_str(),
            ])
            .expect("TEST_CODE stable explicit selector");
            assert!(matches!(
                parsed.command,
                Command::Replay {
                    epoch: EpochSelectorArg(ref actual),
                    ..
                } if actual == &expected
            ));
        }

        for invalid in ["arbitrary".to_owned(), "a".repeat(63), "A".repeat(64)] {
            assert!(Cli::try_parse_from([
                "strategy_attribution",
                "quarter",
                "--db",
                "TEST_CODE.sqlite",
                "--year",
                "2026",
                "--quarter",
                "3",
                "--manifest",
                &manifest(),
                "--epoch",
                invalid.as_str(),
            ])
            .is_err());
        }

        assert!(Cli::try_parse_from([
            "strategy_attribution",
            "capture",
            "--granularity",
            "daily",
            "--from",
            "2026-08-21",
            "--to",
            "2026-08-21",
            "--epoch",
            "legacy",
        ])
        .is_err());
        assert!(
            Cli::try_parse_from(["strategy_attribution", "resolve", "--epoch", "legacy",]).is_err()
        );
        assert!(Cli::try_parse_from([
            "strategy_attribution",
            "probe",
            "--granularity",
            "daily",
            "--from",
            "2026-08-21",
            "--to",
            "2026-08-21",
            "--epoch",
            "legacy",
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
    async fn reset_sample_preview_is_research_only_and_byte_immutable() {
        let path = test_database_path("reset_preview");
        create_source_schema(&path);
        let before = database_state(&path);
        let path_text = path.to_string_lossy().into_owned();
        let cli = Cli::try_parse_from([
            "strategy_attribution",
            "reset-sample",
            "--db",
            path_text.as_str(),
            "--at",
            "2026-08-28T15:40:00+08:00",
            "--format",
            "json",
        ])
        .expect("TEST_CODE reset preview command");

        let output = execute(&cli)
            .await
            .expect("TEST_CODE reset preview succeeds");
        let value: Value = serde_json::from_str(&output).expect("TEST_CODE reset preview JSON");
        assert_eq!(value["状态"], "ResearchOnly");
        assert_eq!(value["数据库已写入"], false);
        assert_eq!(value["epoch"]["completed交易日"], "2026-08-28");
        assert_eq!(value["epoch"]["effective交易日"], "2026-08-31");
        assert_eq!(value["冻结源"]["paper_trade高水位"], 0);
        assert_eq!(value["冻结源"]["order_audit高水位"], 0);
        assert_eq!(value["carry"]["代码与股数"], json!([]));
        assert_eq!(value["epoch"]["身份"].as_str().map(str::len), Some(64));
        assert_eq!(
            value["carry"]["manifest哈希"].as_str().map(str::len),
            Some(64)
        );
        assert_eq!(value["日历权威哈希"].as_str().map(str::len), Some(64));
        assert_eq!(value["receipt身份"], Value::Null);

        let markdown = Cli::try_parse_from([
            "strategy_attribution",
            "reset-sample",
            "--db",
            path_text.as_str(),
            "--at",
            "2026-08-28T15:40:00+08:00",
        ])
        .expect("TEST_CODE reset Markdown preview command");
        let markdown = execute(&markdown)
            .await
            .expect("TEST_CODE reset Markdown preview succeeds");
        for field in [
            "数据库已写入：否",
            "Epoch 身份：",
            "Paper trade 高水位：",
            "Order audit 高水位：",
            "Carry manifest 哈希：",
            "Carry 代码/股数：",
            "日历权威哈希：",
            "Receipt 身份：未提交",
            "研究边界：ResearchOnly",
        ] {
            assert!(markdown.contains(field), "TEST_CODE Markdown field {field}");
        }
        assert_eq!(database_state(&path), before);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn reset_sample_preview_preserves_clean_wal_header_without_sidecars() {
        let path = test_database_path("reset_preview_clean_wal");
        create_source_schema(&path);
        let connection = Connection::open(&path).expect("TEST_CODE clean WAL source");
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .expect("TEST_CODE enable WAL header");
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .expect("TEST_CODE clean WAL checkpoint");
        drop(connection);
        for sidecar in [sidecar(&path, "-wal"), sidecar(&path, "-shm")] {
            if sidecar.exists() {
                fs::remove_file(sidecar).expect("TEST_CODE remove clean WAL sidecar");
            }
        }
        let before = database_state(&path);
        assert!(before[1].is_none());
        assert!(before[2].is_none());
        let path_text = path.to_string_lossy().into_owned();
        let cli = Cli::try_parse_from([
            "strategy_attribution",
            "reset-sample",
            "--db",
            path_text.as_str(),
            "--at",
            "2026-08-28T15:40:00+08:00",
            "--format",
            "json",
        ])
        .expect("TEST_CODE clean WAL reset preview command");

        execute(&cli)
            .await
            .expect("TEST_CODE clean WAL reset preview succeeds");
        assert_eq!(database_state(&path), before);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn reset_sample_rejects_wrong_timezone_and_service_marks_ineligible_times_unavailable() {
        let path = test_database_path("reset_time_boundary");
        create_source_schema(&path);
        let before = database_state(&path);
        let path_text = path.to_string_lossy().into_owned();

        assert!(Cli::try_parse_from([
            "strategy_attribution",
            "reset-sample",
            "--db",
            path_text.as_str(),
            "--at",
            "2026-08-28 15:40:00",
        ])
        .is_err());

        let wrong_timezone = Cli::try_parse_from([
            "strategy_attribution",
            "reset-sample",
            "--db",
            path_text.as_str(),
            "--at",
            "2026-08-28T15:40:00+09:00",
        ])
        .expect("TEST_CODE RFC3339 grammar accepts offset before CLI validation");
        let timezone_error = execute(&wrong_timezone)
            .await
            .expect_err("TEST_CODE reset requires exact +08:00");
        assert_eq!(timezone_error.class, AppErrorClass::Usage);
        assert_eq!(
            timezone_error.code,
            "invocation_timezone_must_be_plus_08_00"
        );

        for (at, expected_code, retryable) in [
            (
                "2026-08-28T09:00:00+08:00",
                "attribution_epoch_window_not_open",
                true,
            ),
            (
                "2026-08-28T14:30:00+08:00",
                "attribution_epoch_window_not_open",
                true,
            ),
            (
                "2026-08-28T15:51:00+08:00",
                "attribution_epoch_window_closed",
                false,
            ),
            (
                "2026-08-29T15:40:00+08:00",
                "attribution_epoch_non_trading_day",
                false,
            ),
            (
                "2026-10-01T15:40:00+08:00",
                "attribution_epoch_non_trading_day",
                false,
            ),
        ] {
            let cli = Cli::try_parse_from([
                "strategy_attribution",
                "reset-sample",
                "--db",
                path_text.as_str(),
                "--at",
                at,
            ])
            .expect("TEST_CODE ineligible reset grammar");
            let error = execute(&cli)
                .await
                .expect_err("TEST_CODE activation service rejects ineligible time");
            assert_eq!(error.class, AppErrorClass::Unavailable, "TEST_CODE {at}");
            assert_eq!(error.code, expected_code, "TEST_CODE {at}");
            assert_eq!(error.retryable, retryable, "TEST_CODE {at}");
        }
        assert_eq!(database_state(&path), before);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn reset_sample_commit_returns_complete_receipt_and_retry_reuses_single_epoch() {
        let path = test_database_path("reset_commit_retry");
        create_source_schema(&path);
        append_activation_carry_source(&path);
        let path_text = path.to_string_lossy().into_owned();
        let cli = Cli::try_parse_from([
            "strategy_attribution",
            "reset-sample",
            "--db",
            path_text.as_str(),
            "--at",
            "2026-08-28T15:40:00+08:00",
            "--commit",
            "--format",
            "json",
        ])
        .expect("TEST_CODE reset commit command");

        let first = execute(&cli).await.expect("TEST_CODE first reset commit");
        let retried = execute(&cli)
            .await
            .expect("TEST_CODE idempotent reset retry");
        let first: Value = serde_json::from_str(&first).expect("TEST_CODE first receipt JSON");
        let retried: Value =
            serde_json::from_str(&retried).expect("TEST_CODE retried receipt JSON");
        assert_eq!(first["状态"], "ResearchOnly");
        assert_eq!(first["数据库已写入"], true);
        assert_eq!(first["epoch"]["completed交易日"], "2026-08-28");
        assert_eq!(first["epoch"]["effective交易日"], "2026-08-31");
        assert_eq!(first["冻结源"]["paper_trade高水位"], 1);
        assert_eq!(first["冻结源"]["order_audit高水位"], 1);
        assert_eq!(
            first["carry"]["代码与股数"],
            json!([{"code": "TEST_CODE_600001", "quantity": 100}])
        );
        assert_eq!(first["epoch"]["身份"], retried["epoch"]["身份"]);
        assert_eq!(first["receipt身份"], retried["receipt身份"]);
        assert_eq!(first["receipt身份"].as_str().map(str::len), Some(64));
        assert_eq!(first["日历权威哈希"].as_str().map(str::len), Some(64));
        assert_eq!(
            first["carry"]["manifest哈希"].as_str().map(str::len),
            Some(64)
        );

        let connection = Connection::open(&path).expect("TEST_CODE inspect epoch commit");
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM attribution_sample_epoch_receipt"
            ),
            1
        );
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM attribution_sample_epoch_receipt_chain"
            ),
            1
        );
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(DISTINCT success_receipt_hash) FROM attribution_epoch_attempt_audit WHERE outcome='success'"
            ),
            1
        );
        drop(connection);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn scheduled_active_selector_fails_closed_without_legacy_fallback() {
        let path = test_database_path("active_no_legacy_fallback");
        create_source_schema(&path);
        drop(
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::AppendOnly)
                .expect("TEST_CODE install epoch schema without activation"),
        );
        let before = database_state(&path);
        let path_text = path.to_string_lossy().into_owned();
        let manifest = manifest();
        let cli = Cli::try_parse_from([
            "strategy_attribution",
            "scheduled",
            "--db",
            path_text.as_str(),
            "--at",
            "2026-08-21T15:30:00+08:00",
            "--manifest",
            manifest.as_str(),
        ])
        .expect("TEST_CODE active scheduled command");

        let error = execute(&cli)
            .await
            .expect_err("TEST_CODE missing active epoch must fail closed");
        assert_eq!(error.class, AppErrorClass::Unavailable);
        assert_eq!(error.stage, "epoch");
        assert_eq!(error.code, "attribution_epoch_unavailable");
        assert_eq!(database_state(&path), before);
        cleanup_database(&path);
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
