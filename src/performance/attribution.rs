//! 2026-08-20 Attribution Research Loop — 交付物 A 核心模块.
//! Registered business rules: BR-247.
//!
//! 设计: docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md §4.
//! 数据来源: paper_trades (plan_id + virtual_reason), 证据 E3-E7.
//! 归因口径: 已实现 (FIFO 带 lot 归属) + 未实现浮盈 (未平仓 lot × 收盘价).

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::database::attribution_epochs::{
    AttributionEpochDailyBatchAppend, AttributionEpochDailyFamilyAppend,
    AttributionEpochDailySourceBinding, AttributionEpochStore, AttributionEpochStoreError,
};
use crate::database::{DatabaseConnectionAuthority, DatabaseManager};
use crate::performance::attribution_epoch::{
    canonical_exclusion_manifest_hash, canonical_legacy_carry_manifest_hash,
    canonical_scoped_fill_manifest_hash, scope_epoch_fills, AttributionEpochSelector,
    EpochExclusion, LegacyCarryPosition,
};

/// 入场信号族 (归因维度). spec §4.1.
/// Ord 派生供 Task 3 的 BTreeMap 聚合排序使用.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SignalFamily {
    NewsCatalyst,
    VolumeSurge,
    MainNetInflow,
    Breakout,
    SectorLeader,
    AuctionAnomaly,
    LLMSelect,
    Momentum,
    PostCloseFundInflow,
    /// 仅为历史持久化/序列化兼容保留；BR-247 禁止把退出原因映射成入场族。
    ExitByRule,
    Unknown,
}

impl SignalFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            SignalFamily::NewsCatalyst => "NewsCatalyst",
            SignalFamily::VolumeSurge => "VolumeSurge",
            SignalFamily::MainNetInflow => "MainNetInflow",
            SignalFamily::Breakout => "Breakout",
            SignalFamily::SectorLeader => "SectorLeader",
            SignalFamily::AuctionAnomaly => "AuctionAnomaly",
            SignalFamily::LLMSelect => "LLMSelect",
            SignalFamily::Momentum => "Momentum",
            SignalFamily::PostCloseFundInflow => "PostCloseFundInflow",
            SignalFamily::ExitByRule => "ExitByRule",
            SignalFamily::Unknown => "Unknown",
        }
    }
}

/// virtual_reason → 信号族. 规则表见 spec §4.1; 未命中 → Unknown (报告明示, 不静默).
pub fn signal_family_of(reason: &str) -> SignalFamily {
    let r = reason.trim();
    if r.starts_with("NewsCatalyst") {
        return SignalFamily::NewsCatalyst;
    }
    if r.starts_with("VolumeSurge") {
        return SignalFamily::VolumeSurge;
    }
    if r.starts_with("MainNetInflow") {
        return SignalFamily::MainNetInflow;
    }
    if r.starts_with("Breakout") {
        return SignalFamily::Breakout;
    }
    if r.starts_with("SectorLeader") {
        return SignalFamily::SectorLeader;
    }
    if r.starts_with("AuctionAnomaly") {
        return SignalFamily::AuctionAnomaly;
    }
    if r.starts_with("LLMSelect") {
        return SignalFamily::LLMSelect;
    }
    if r.starts_with("Momentum") {
        return SignalFamily::Momentum;
    }
    if r.starts_with("盘后资金净流入") || r.contains("收盘价买入") {
        return SignalFamily::PostCloseFundInflow;
    }
    SignalFamily::Unknown
}

/// 提取 `涨幅+X.X%` 数值; 无 → None.
pub fn parse_change_pct(reason: &str) -> Option<f64> {
    let (_, rest) = reason.split_once("涨幅")?;
    let value = rest.split('%').next()?.trim();
    value.parse::<f64>().ok()
}

/// 提取 `量比X.X` 数值; 无 → None.
pub fn parse_volume_ratio(reason: &str) -> Option<f64> {
    let (_, rest) = reason.split_once("量比")?;
    let value: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    value.parse::<f64>().ok()
}

/// 可疑数据: |涨幅| > 25 或 量比 ≤ 0 (spec §4.1; 证据 E6: 涨幅+858.9% ×27、量比0.0).
/// 可疑 lot 仍计入所属族 PnL, 由报告「数据质量」节单独标注 — 不删除, 不静默.
pub fn is_suspicious_reason(reason: &str) -> bool {
    if let Some(pct) = parse_change_pct(reason) {
        if pct.abs() > 25.0 {
            return true;
        }
    }
    if let Some(ratio) = parse_volume_ratio(reason) {
        if ratio <= 0.0 {
            return true;
        }
    }
    false
}

#[derive(diesel::QueryableByName, Debug)]
pub struct AttributionFillRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub fill_price: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub local_ts: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub plan_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub virtual_reason: String,
}

/// 已实现交易归因 — 每笔卖出按匹配到的入场 lot 拆分归属.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TradeAttribution {
    pub sell_id: i64,
    pub code: String,
    pub pnl: f64,
    pub entry_plan_id: String,
    pub entry_family: SignalFamily,
    pub exit_reason: String,
    pub suspicious: bool,
    /// 卖出发生日期 (窗口归因以 emit_from 谓词按此判断是否入窗).
    pub sell_date: NaiveDate,
}

/// 未平仓 lot (FIFO 匹配剩余).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenLot {
    pub code: String,
    pub plan_id: String,
    pub family: SignalFamily,
    pub suspicious: bool,
    pub remaining_qty: i64,
    pub cost_price: f64,
}

/// FIFO 匹配 (当日语义): 语义与 performance/snapshot.rs::realized_pnls_for_date 逐条对齐
/// (id>0, code 非空, price>0 finite, qty>0 且 %100==0, 时间序校验, oversell 拒绝,
/// 非 finite PnL 拒绝), 区别: 匹配时携带入场 lot 的 plan_id/family/suspicious 归属.
/// 跨 lot 匹配时 PnL 按数量比例拆分 (每段生成一条 TradeAttribution).
/// 发射谓词 = 仅当日卖出 (fifo_match_from 的 emit_from=None 特例, epoch daily 语义).
/// 返回 (当日已实现归因列表, 未平仓 lot 列表).
pub fn fifo_match(
    rows: &[AttributionFillRow],
    target_date: NaiveDate,
) -> Result<(Vec<TradeAttribution>, Vec<OpenLot>), String> {
    fifo_match_from(rows, target_date, None)
}

/// FIFO 匹配核心 (发射谓词参数化, CRIT-1 修复):
/// - `emit_from = None`    → 仅发射 `timestamp.date() == target_date` 的卖出
///   (与旧 fifo_match 行为逐字节一致; fifo_match 2-arg wrapper 保持公开 API 稳定,
///   epoch daily 与既有日级测试不受影响).
/// - `emit_from = Some(d)` → 发射 `timestamp.date() >= d` 的全部卖出 (epoch window
///   30 天窗口语义; FIFO 匹配仍对全部 rows 执行 — 窗口前买入照常被窗口卖出消耗).
///
/// 校验 (身份/时间戳/越界/无序/oversell 等) 与 emit_from 无关, 全部 rows 一视同仁.
pub fn fifo_match_from(
    rows: &[AttributionFillRow],
    target_date: NaiveDate,
    emit_from: Option<NaiveDate>,
) -> Result<(Vec<TradeAttribution>, Vec<OpenLot>), String> {
    use std::collections::{HashMap, VecDeque};

    #[derive(Clone)]
    struct Lot {
        remaining: u32,
        price: f64,
        plan_id: String,
        family: SignalFamily,
        suspicious: bool,
    }

    let mut lots: HashMap<String, VecDeque<Lot>> = HashMap::new();
    let mut realized = Vec::new();
    let mut previous_order: Option<(chrono::NaiveDateTime, i64)> = None;

    for row in rows {
        if row.id <= 0 || row.code.trim().is_empty() {
            return Err(format!(
                "attribution fill identity invalid: id={} code={:?}",
                row.id, row.code
            ));
        }
        let timestamp = chrono::NaiveDateTime::parse_from_str(&row.local_ts, "%Y-%m-%d %H:%M:%S")
            .map_err(|error| {
            format!("attribution fill id={} timestamp invalid: {error}", row.id)
        })?;
        if timestamp.date() > target_date {
            return Err(format!(
                "attribution fill id={} is later than settlement date {}",
                row.id, target_date
            ));
        }
        if previous_order.is_some_and(|previous| previous > (timestamp, row.id)) {
            return Err(format!(
                "attribution fills are not ordered at id={}",
                row.id
            ));
        }
        previous_order = Some((timestamp, row.id));
        let price = row
            .fill_price
            .filter(|price| price.is_finite() && *price > 0.0)
            .ok_or_else(|| format!("attribution fill id={} fill_price missing/invalid", row.id))?;
        let quantity = u32::try_from(row.quantity)
            .ok()
            .filter(|quantity| *quantity > 0 && quantity.is_multiple_of(100))
            .ok_or_else(|| {
                format!(
                    "attribution fill id={} quantity invalid: {}",
                    row.id, row.quantity
                )
            })?;
        let family = signal_family_of(&row.virtual_reason);
        let suspicious = is_suspicious_reason(&row.virtual_reason);

        match row.direction.as_str() {
            "buy" => lots.entry(row.code.clone()).or_default().push_back(Lot {
                remaining: quantity,
                price,
                plan_id: row.plan_id.clone(),
                family,
                suspicious,
            }),
            "sell" => {
                let queue = lots.get_mut(&row.code).ok_or_else(|| {
                    format!("attribution sell id={} has no matched buy lots", row.id)
                })?;
                let mut remaining = quantity;
                while remaining > 0 {
                    let lot = queue.front_mut().ok_or_else(|| {
                        format!(
                            "attribution sell id={} quantity {} exceeds matched buys",
                            row.id, quantity
                        )
                    })?;
                    let matched = remaining.min(lot.remaining);
                    let portion_pnl = (price - lot.price) * f64::from(matched);
                    let date = timestamp.date();
                    let emit = match emit_from {
                        None => date == target_date,
                        Some(from) => date >= from,
                    };
                    if emit {
                        realized.push(TradeAttribution {
                            sell_id: row.id,
                            code: row.code.clone(),
                            pnl: portion_pnl,
                            entry_plan_id: lot.plan_id.clone(),
                            entry_family: lot.family,
                            exit_reason: row.virtual_reason.clone(),
                            suspicious: lot.suspicious,
                            sell_date: date,
                        });
                    }
                    remaining -= matched;
                    lot.remaining -= matched;
                    if lot.remaining == 0 {
                        queue.pop_front(); // 与 snapshot.rs 同构: 已完成 lot 出队
                    }
                }
            }
            other => {
                return Err(format!(
                    "attribution fill id={} direction invalid: {other}",
                    row.id
                ));
            }
        }
    }
    // 非 finite 校验: 全部已实现 PnL 必须 finite (与 snapshot.rs 一致)
    for attribution in &realized {
        if !attribution.pnl.is_finite() {
            return Err(format!(
                "attribution sell id={} PnL is non-finite",
                attribution.sell_id
            ));
        }
    }
    let open = lots
        .into_iter()
        .flat_map(|(code, queue)| {
            queue.into_iter().map(move |lot| OpenLot {
                code: code.clone(),
                plan_id: lot.plan_id,
                family: lot.family,
                suspicious: lot.suspicious,
                remaining_qty: i64::from(lot.remaining),
                cost_price: lot.price,
            })
        })
        .collect();
    Ok((realized, open))
}

/// 单族聚合 (spec §4.2).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FamilyAggregate {
    pub family: SignalFamily,
    pub realized_trades: i64,
    pub realized_pnl: f64,
    pub open_lots: i64,
    pub unrealized_pnl: f64,
    pub total_pnl: f64,
    pub wins: i64,
    pub losses: i64,
    pub win_rate: Option<f64>,
    pub unvalued_lots: i64,
    pub suspicious_lots: i64,
    /// 可疑 lot 已实现影响金额 (spec §4.4.2; realized-only — 未平仓可疑 lot 只计入
    /// suspicious_lots, 金额待其卖出实现后归入).
    pub suspicious_pnl: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DailyAttribution {
    pub date: NaiveDate,
    pub families: Vec<FamilyAggregate>,
    /// Top 盈亏交易明细 (当日, spec §4.4 item 5): 盈利 (pnl>0) ≤5 在前, 亏损 (pnl<0) ≤5 在后.
    pub top_trades: Vec<TradeAttribution>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowAttribution {
    pub days: u32,
    pub end: NaiveDate,
    pub families: Vec<FamilyAggregate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttributionEpochDailyEvidence {
    pub selector: AttributionEpochSelector,
    pub epoch_id: String,
    pub receipt_hash: String,
    pub effective_date: NaiveDate,
    pub cutoff_date: NaiveDate,
    pub frozen_paper_trade_high_water: i64,
    pub frozen_order_audit_high_water: i64,
    pub source_paper_trade_high_water: i64,
    pub source_order_audit_high_water: i64,
    pub all_status_paper_manifest_hash: String,
    pub legacy_carry_manifest_hash: String,
    pub exclusion_manifest_hash: String,
    pub scoped_fill_manifest_hash: String,
    pub verified_filled_manifest_hash: String,
    pub verified_terminal_binding_manifest_hash: String,
    pub verified_order_audit_tip_hash: String,
    pub exclusions: Vec<EpochExclusion>,
    pub remaining_quarantine: Vec<LegacyCarryPosition>,
    pub released_codes: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EpochDailyAttribution {
    daily: DailyAttribution,
    epoch: AttributionEpochDailyEvidence,
    database_authority: DatabaseConnectionAuthority,
}

impl EpochDailyAttribution {
    pub fn daily(&self) -> &DailyAttribution {
        &self.daily
    }

    pub fn epoch(&self) -> &AttributionEpochDailyEvidence {
        &self.epoch
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EpochWindowAttribution {
    window: WindowAttribution,
    epoch: AttributionEpochDailyEvidence,
    database_authority: DatabaseConnectionAuthority,
}

impl EpochWindowAttribution {
    pub fn window(&self) -> &WindowAttribution {
        &self.window
    }

    pub fn epoch(&self) -> &AttributionEpochDailyEvidence {
        &self.epoch
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionEpochDailyFamilyReceipt {
    pub epoch_daily_id: i64,
    pub signal_family: String,
    pub revision: u64,
    pub payload_hash: String,
    pub record_hash: String,
    pub created_at: String,
    pub retention_deadline: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionEpochDailyReceipt {
    pub epoch_id: String,
    pub date: NaiveDate,
    pub receipts: Vec<AttributionEpochDailyFamilyReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributionEpochRuntimeError {
    Unavailable {
        reason_code: &'static str,
        retryable: bool,
        detail: String,
    },
    FailedIntegrity {
        reason_code: &'static str,
        detail: String,
    },
}

impl AttributionEpochRuntimeError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Unavailable { reason_code, .. } | Self::FailedIntegrity { reason_code, .. } => {
                reason_code
            }
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Unavailable {
                retryable: true,
                ..
            }
        )
    }
}

impl std::fmt::Display for AttributionEpochRuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable {
                reason_code,
                retryable,
                detail,
            } => write!(
                formatter,
                "{reason_code} (unavailable, retryable={retryable}): {detail}"
            ),
            Self::FailedIntegrity {
                reason_code,
                detail,
            } => write!(formatter, "{reason_code} (failed_integrity): {detail}"),
        }
    }
}

impl std::error::Error for AttributionEpochRuntimeError {}

impl From<AttributionEpochStoreError> for AttributionEpochRuntimeError {
    fn from(error: AttributionEpochStoreError) -> Self {
        match error {
            AttributionEpochStoreError::Unavailable {
                reason_code,
                retryable,
                detail,
            } => Self::Unavailable {
                reason_code,
                retryable,
                detail,
            },
            AttributionEpochStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => Self::FailedIntegrity {
                reason_code,
                detail,
            },
        }
    }
}

pub fn compute_epoch_daily(
    database: &DatabaseManager,
    date: NaiveDate,
    prices: &HashMap<String, f64>,
) -> Result<EpochDailyAttribution, AttributionEpochRuntimeError> {
    let (rows, epoch, database_authority) = load_scoped_epoch_rows(database, date, date)?;
    let (attributions, open) = fifo_match(&rows, date).map_err(|detail| {
        runtime_integrity(
            "attribution_epoch_aggregation_failed",
            format!("BR-255 daily attribution aggregation: {detail}"),
        )
    })?;
    let top_trades = top_trades(&attributions);
    let families = aggregate_families(&attributions, &open, prices);
    Ok(EpochDailyAttribution {
        daily: DailyAttribution {
            date,
            families,
            top_trades,
        },
        epoch,
        database_authority,
    })
}

pub fn compute_epoch_window(
    database: &DatabaseManager,
    end: NaiveDate,
    days: u32,
    prices: &HashMap<String, f64>,
) -> Result<EpochWindowAttribution, AttributionEpochRuntimeError> {
    if days == 0 {
        return Err(runtime_integrity(
            "attribution_epoch_window_invalid",
            "BR-255 epoch attribution window must contain at least one day",
        ));
    }
    let start = end
        .checked_sub_signed(chrono::Duration::days(i64::from(days) - 1))
        .ok_or_else(|| {
            runtime_integrity(
                "attribution_epoch_window_invalid",
                "BR-255 epoch attribution window underflowed the supported date range",
            )
        })?;
    let (rows, epoch, database_authority) = load_scoped_epoch_rows(database, start, end)?;
    let window = aggregate_window(end, days, &rows, prices).map_err(|detail| {
        runtime_integrity(
            "attribution_epoch_aggregation_failed",
            format!("BR-255 window attribution aggregation: {detail}"),
        )
    })?;
    Ok(EpochWindowAttribution {
        window,
        epoch,
        database_authority,
    })
}

pub fn persist_epoch_daily(
    database: &DatabaseManager,
    daily: &EpochDailyAttribution,
) -> Result<AttributionEpochDailyReceipt, AttributionEpochRuntimeError> {
    if daily.epoch.selector != AttributionEpochSelector::Active
        || daily.daily.date != daily.epoch.cutoff_date
        || daily.daily.date < daily.epoch.effective_date
    {
        return Err(runtime_integrity(
            "attribution_epoch_daily_binding_invalid",
            "BR-255 daily attribution is not bound to its active epoch cutoff",
        ));
    }

    #[derive(Serialize)]
    struct DailyFamilyPayload<'a> {
        schema_version: &'static str,
        date: NaiveDate,
        epoch: &'a AttributionEpochDailyEvidence,
        family: Option<&'a FamilyAggregate>,
        top_trades: Vec<&'a TradeAttribution>,
    }

    let mut families = Vec::with_capacity(daily.daily.families.len().max(1));
    if daily.daily.families.is_empty() {
        let payload = serde_json::to_value(DailyFamilyPayload {
            schema_version: "BR-255_ATTRIBUTION_EPOCH_DAILY_V1",
            date: daily.daily.date,
            epoch: &daily.epoch,
            family: None,
            top_trades: daily.daily.top_trades.iter().collect(),
        })
        .map_err(|error| {
            runtime_integrity(
                "attribution_epoch_daily_serialization_failed",
                format!("BR-255 serialize empty daily attribution: {error}"),
            )
        })?;
        families.push(AttributionEpochDailyFamilyAppend {
            signal_family: "all".to_owned(),
            payload,
        });
    } else {
        for family in &daily.daily.families {
            let payload = serde_json::to_value(DailyFamilyPayload {
                schema_version: "BR-255_ATTRIBUTION_EPOCH_DAILY_V1",
                date: daily.daily.date,
                epoch: &daily.epoch,
                family: Some(family),
                top_trades: daily
                    .daily
                    .top_trades
                    .iter()
                    .filter(|trade| trade.entry_family == family.family)
                    .collect(),
            })
            .map_err(|error| {
                runtime_integrity(
                    "attribution_epoch_daily_serialization_failed",
                    format!(
                        "BR-255 serialize {} daily attribution: {error}",
                        family.family.as_str()
                    ),
                )
            })?;
            families.push(AttributionEpochDailyFamilyAppend {
                signal_family: family.family.as_str().to_owned(),
                payload,
            });
        }
    }

    let stored = AttributionEpochStore::new(database)
        .append_verified_daily_batch(
            AttributionEpochDailyBatchAppend {
                epoch_id: daily.epoch.epoch_id.clone(),
                date: daily.daily.date,
                families,
            },
            AttributionEpochDailySourceBinding {
                database_authority: daily.database_authority.clone(),
                epoch_id: daily.epoch.epoch_id.clone(),
                receipt_hash: daily.epoch.receipt_hash.clone(),
                effective_date: daily.epoch.effective_date,
                cutoff_date: daily.epoch.cutoff_date,
                frozen_paper_trade_high_water: daily.epoch.frozen_paper_trade_high_water,
                frozen_order_audit_high_water: daily.epoch.frozen_order_audit_high_water,
                source_paper_trade_high_water: daily.epoch.source_paper_trade_high_water,
                source_order_audit_high_water: daily.epoch.source_order_audit_high_water,
                all_status_paper_manifest_hash: daily.epoch.all_status_paper_manifest_hash.clone(),
                legacy_carry_manifest_hash: daily.epoch.legacy_carry_manifest_hash.clone(),
                verified_filled_manifest_hash: daily.epoch.verified_filled_manifest_hash.clone(),
                verified_terminal_binding_manifest_hash: daily
                    .epoch
                    .verified_terminal_binding_manifest_hash
                    .clone(),
                verified_order_audit_tip_hash: daily.epoch.verified_order_audit_tip_hash.clone(),
                exclusion_manifest_hash: daily.epoch.exclusion_manifest_hash.clone(),
                scoped_fill_manifest_hash: daily.epoch.scoped_fill_manifest_hash.clone(),
                remaining_quarantine_manifest_hash: canonical_legacy_carry_manifest_hash(
                    &daily.epoch.remaining_quarantine,
                ),
                released_codes: daily.epoch.released_codes,
            },
        )
        .map_err(AttributionEpochRuntimeError::from)?;
    Ok(AttributionEpochDailyReceipt {
        epoch_id: stored.epoch_id,
        date: stored.date,
        receipts: stored
            .receipts
            .into_iter()
            .map(|receipt| AttributionEpochDailyFamilyReceipt {
                epoch_daily_id: receipt.epoch_daily_id,
                signal_family: receipt.signal_family,
                revision: receipt.revision,
                payload_hash: receipt.payload_hash,
                record_hash: receipt.record_hash,
                created_at: receipt.created_at,
                retention_deadline: receipt.retention_deadline,
            })
            .collect(),
    })
}

fn load_scoped_epoch_rows(
    database: &DatabaseManager,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<
    (
        Vec<AttributionFillRow>,
        AttributionEpochDailyEvidence,
        DatabaseConnectionAuthority,
    ),
    AttributionEpochRuntimeError,
> {
    let (receipt, verified, database_authority) = AttributionEpochStore::new(database)
        .load_active_verified_fills_until(from, to)
        .map_err(AttributionEpochRuntimeError::from)?;
    let source_rows = verified
        .fills()
        .iter()
        .map(|fill| fill.fill().clone())
        .collect::<Vec<_>>();
    let scoped = scope_epoch_fills(
        &source_rows,
        receipt.effective_trading_date,
        verified.carry(),
    )
    .map_err(|detail| {
        runtime_integrity(
            "attribution_epoch_scope_failed",
            format!("BR-255 daily attribution fill scoping: {detail}"),
        )
    })?;
    let exclusion_manifest_hash =
        canonical_exclusion_manifest_hash(&scoped.exclusions, &source_rows).map_err(|detail| {
            runtime_integrity(
                "attribution_epoch_scope_failed",
                format!("BR-255 daily attribution exclusion manifest: {detail}"),
            )
        })?;
    let scoped_fill_manifest_hash = canonical_scoped_fill_manifest_hash(&scoped.attributable)
        .map_err(|detail| {
            runtime_integrity(
                "attribution_epoch_scope_failed",
                format!("BR-255 daily attribution scoped manifest: {detail}"),
            )
        })?;
    let rows = scoped
        .attributable
        .iter()
        .map(|fill| AttributionFillRow {
            id: fill.id,
            code: fill.code.clone(),
            direction: fill.direction.clone(),
            fill_price: fill.fill_price,
            quantity: fill.quantity,
            local_ts: fill.occurred_at.clone(),
            plan_id: fill.plan_id.clone(),
            virtual_reason: fill.virtual_reason.clone(),
        })
        .collect();
    Ok((
        rows,
        AttributionEpochDailyEvidence {
            selector: AttributionEpochSelector::Active,
            epoch_id: receipt.epoch_id,
            receipt_hash: receipt.receipt_hash,
            effective_date: receipt.effective_trading_date,
            cutoff_date: to,
            frozen_paper_trade_high_water: receipt.paper_trade_high_water,
            frozen_order_audit_high_water: receipt.order_audit_high_water,
            source_paper_trade_high_water: verified.current_paper_trade_high_water(),
            source_order_audit_high_water: verified.current_order_audit_high_water(),
            all_status_paper_manifest_hash: verified.all_status_paper_manifest_hash().to_owned(),
            legacy_carry_manifest_hash: receipt.legacy_carry_manifest_hash,
            exclusion_manifest_hash,
            scoped_fill_manifest_hash,
            verified_filled_manifest_hash: verified.filled_manifest_hash().to_owned(),
            verified_terminal_binding_manifest_hash: verified
                .terminal_binding_manifest_hash()
                .to_owned(),
            verified_order_audit_tip_hash: verified.order_audit_tip_hash().to_owned(),
            exclusions: scoped.exclusions,
            remaining_quarantine: scoped.remaining_quarantine,
            released_codes: scoped.released_codes,
        },
        database_authority,
    ))
}

fn runtime_integrity(
    reason_code: &'static str,
    detail: impl Into<String>,
) -> AttributionEpochRuntimeError {
    AttributionEpochRuntimeError::FailedIntegrity {
        reason_code,
        detail: detail.into(),
    }
}

/// 聚合: 已实现 (卖出归因) + 未实现浮盈 (未平仓 lot × close).
/// 缺失 close → unvalued_lots 计数, 浮盈记 0 (不静默: 计数与报告明示).
/// suspicious_pnl 仅计已实现 (可疑卖出归因的 pnl 合计; 未平仓可疑 lot 只计
/// suspicious_lots 计数, 金额待卖出后归入).
pub fn aggregate_families(
    attributions: &[TradeAttribution],
    open: &[OpenLot],
    prices: &HashMap<String, f64>,
) -> Vec<FamilyAggregate> {
    use std::collections::BTreeMap;
    // 注意: rustc 1.95 拒绝「闭包返回指向捕获变量的引用」(captured variable cannot
    // escape FnMut closure body), 故用嵌套 fn 而非闭包实现 entry 复用.
    fn ensure(
        map: &mut BTreeMap<SignalFamily, FamilyAggregate>,
        family: SignalFamily,
    ) -> &mut FamilyAggregate {
        map.entry(family).or_insert_with(|| FamilyAggregate {
            family,
            realized_trades: 0,
            realized_pnl: 0.0,
            open_lots: 0,
            unrealized_pnl: 0.0,
            total_pnl: 0.0,
            wins: 0,
            losses: 0,
            win_rate: None,
            unvalued_lots: 0,
            suspicious_lots: 0,
            suspicious_pnl: 0.0,
        })
    }
    let mut map: BTreeMap<SignalFamily, FamilyAggregate> = BTreeMap::new();
    for a in attributions {
        let row = ensure(&mut map, a.entry_family);
        row.realized_trades += 1;
        row.realized_pnl += a.pnl;
        if a.pnl > 0.0 {
            row.wins += 1;
        } else {
            row.losses += 1;
        }
        if a.suspicious {
            row.suspicious_lots += 1;
            row.suspicious_pnl += a.pnl; // realized-only 影响金额 (spec §4.4.2)
        }
    }
    for lot in open {
        let row = ensure(&mut map, lot.family);
        row.open_lots += 1;
        if lot.suspicious {
            row.suspicious_lots += 1;
        }
        match prices
            .get(&lot.code)
            .copied()
            .filter(|p| p.is_finite() && *p > 0.0)
        {
            Some(close) => {
                row.unrealized_pnl += (close - lot.cost_price) * lot.remaining_qty as f64
            }
            None => row.unvalued_lots += 1,
        }
    }
    let mut families: Vec<FamilyAggregate> = map.into_values().collect();
    for row in &mut families {
        row.total_pnl = row.realized_pnl + row.unrealized_pnl;
        row.win_rate =
            (row.realized_trades > 0).then_some(row.wins as f64 / row.realized_trades as f64);
    }
    families.sort_by_key(|f| f.family);
    families
}

/// Top 盈亏交易明细 (spec §4.4 item 5, 当日): 盈利 (pnl>0) 按 pnl 降序 ≤5 在前,
/// 亏损 (pnl<0) 按 pnl 升序 (最负在前) ≤5 在后; pnl==0 不入列.
fn top_trades(attributions: &[TradeAttribution]) -> Vec<TradeAttribution> {
    let mut winners: Vec<&TradeAttribution> = attributions.iter().filter(|a| a.pnl > 0.0).collect();
    winners.sort_by(|a, b| {
        b.pnl
            .partial_cmp(&a.pnl)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut losers: Vec<&TradeAttribution> = attributions.iter().filter(|a| a.pnl < 0.0).collect();
    losers.sort_by(|a, b| {
        a.pnl
            .partial_cmp(&b.pnl)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    winners.truncate(5);
    losers.truncate(5);
    winners.into_iter().chain(losers).cloned().collect()
}

/// 窗口聚合纯函数 (不触 DB, 供单测直测): start = end − (days−1) 天 (含首尾共 days 个
/// 自然日); FIFO 匹配跑全部 rows (窗口前买入照常被窗口卖出消耗), 发射谓词 =
/// `timestamp.date() >= start` (CRIT-1: 已实现必须为窗口累计, 非单日).
pub fn aggregate_window(
    end: NaiveDate,
    days: u32,
    rows: &[AttributionFillRow],
    prices: &HashMap<String, f64>,
) -> Result<WindowAttribution, String> {
    let start = end
        .checked_sub_signed(chrono::Duration::days(i64::from(days) - 1))
        .unwrap_or(NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch"));
    let (attributions, open) = fifo_match_from(rows, end, Some(start))?;
    let families = aggregate_families(&attributions, &open, prices);
    Ok(WindowAttribution {
        days,
        end,
        families,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_from_reason_prefixes() {
        assert_eq!(signal_family_of("NewsCatalyst"), SignalFamily::NewsCatalyst);
        assert_eq!(signal_family_of("VolumeSurge"), SignalFamily::VolumeSurge);
        assert_eq!(
            signal_family_of("MainNetInflow"),
            SignalFamily::MainNetInflow
        );
        assert_eq!(signal_family_of("Breakout"), SignalFamily::Breakout);
        assert_eq!(signal_family_of("SectorLeader"), SignalFamily::SectorLeader);
        assert_eq!(
            signal_family_of("AuctionAnomaly"),
            SignalFamily::AuctionAnomaly
        );
        assert_eq!(signal_family_of("LLMSelect"), SignalFamily::LLMSelect);
        assert_eq!(signal_family_of("Momentum"), SignalFamily::Momentum);
        assert_eq!(
            signal_family_of("BR-234四大铁律卖出:结构止损（破中期趋势）"),
            SignalFamily::Unknown,
            "exit reason is not an entry strategy family"
        );
        assert_eq!(
            signal_family_of("盘后资金净流入Top10 收盘价买入: 主力+9.96亿 量比1.5 涨幅-2.9%"),
            SignalFamily::PostCloseFundInflow
        );
        assert_eq!(
            signal_family_of("均线策略 收盘价买入 量比1.2 涨幅+3%"),
            SignalFamily::PostCloseFundInflow
        );
        assert_eq!(signal_family_of("未知原因"), SignalFamily::Unknown);
    }

    #[test]
    fn suspicious_rules_capture_garbage_but_keep_sane() {
        assert!(is_suspicious_reason(
            "盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%"
        ));
        assert!(is_suspicious_reason("... 涨幅+999.0%"));
        assert!(!is_suspicious_reason("... 涨幅+10.0% 量比1.5"));
        assert!(!is_suspicious_reason("NewsCatalyst"));
    }

    #[test]
    fn parse_helpers_extract_structured_fields() {
        let reason = "盘后资金净流入Top10 收盘价买入: 主力+9.96亿 量比1.5 涨幅-2.9%";
        assert_eq!(parse_change_pct(reason), Some(-2.9));
        assert_eq!(parse_volume_ratio(reason), Some(1.5));
        assert_eq!(parse_change_pct("NewsCatalyst"), None);
        assert_eq!(parse_volume_ratio("NewsCatalyst"), None);
    }

    #[test]
    fn family_names_are_stable_snake_case() {
        assert_eq!(
            SignalFamily::PostCloseFundInflow.as_str(),
            "PostCloseFundInflow"
        );
        assert_eq!(SignalFamily::ExitByRule.as_str(), "ExitByRule");
    }

    // TEST_CODE fixture mirrors one persisted fill row; named columns are clearer here.
    #[allow(clippy::too_many_arguments)]
    fn fill(
        id: i64,
        code: &str,
        direction: &str,
        price: f64,
        quantity: i64,
        local_ts: &str,
        plan_id: &str,
        virtual_reason: &str,
    ) -> AttributionFillRow {
        AttributionFillRow {
            id,
            code: code.to_string(),
            direction: direction.to_string(),
            fill_price: Some(price),
            quantity,
            local_ts: local_ts.to_string(),
            plan_id: plan_id.to_string(),
            virtual_reason: virtual_reason.to_string(),
        }
    }

    #[test]
    fn fifo_carries_lot_attribution() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                100,
                "2026-07-17 10:00:00",
                "news-1",
                "NewsCatalyst",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "buy",
                12.0,
                200,
                "2026-07-18 09:31:00",
                "fund-2",
                "MainNetInflow",
            ),
            fill(
                3,
                "TEST_CODE_600000",
                "sell",
                15.0,
                200,
                "2026-07-18 14:00:00",
                "sell-3",
                "BR-234四大铁律卖出:结构止损",
            ),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");

        // 200 股卖出: 100 股归 NewsCatalyst lot (10.0→15.0 = +500), 100 股归 MainNetInflow lot (12.0→15.0 = +300)
        assert_eq!(attributions.len(), 2);
        let news: Vec<_> = attributions
            .iter()
            .filter(|a| a.entry_family == SignalFamily::NewsCatalyst)
            .collect();
        let fund: Vec<_> = attributions
            .iter()
            .filter(|a| a.entry_family == SignalFamily::MainNetInflow)
            .collect();
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].pnl, 500.0);
        assert_eq!(news[0].entry_plan_id, "news-1");
        assert_eq!(fund.len(), 1);
        assert_eq!(fund[0].pnl, 300.0);
        assert_eq!(fund[0].entry_plan_id, "fund-2");
        assert_eq!(attributions.iter().map(|a| a.pnl).sum::<f64>(), 800.0); // 与 snapshot.rs 已知结果一致
        assert_eq!(open.len(), 1); // MainNetInflow lot 剩 100 股
        assert_eq!(open[0].remaining_qty, 100);
        assert_eq!(open[0].cost_price, 12.0);
    }

    #[test]
    fn fifo_rejects_oversell_and_invalid_rows() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let oversell = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                100,
                "2026-07-18 10:00:00",
                "p1",
                "NewsCatalyst",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "sell",
                11.0,
                200,
                "2026-07-18 14:00:00",
                "s1",
                "BR-234四大铁律卖出",
            ),
        ];
        let err = fifo_match(&oversell, target).expect_err("oversell must fail");
        assert!(err.contains("exceeds matched buys"));

        let mut missing_price = fill(
            1,
            "TEST_CODE_600000",
            "buy",
            10.0,
            100,
            "2026-07-18 10:00:00",
            "p1",
            "NewsCatalyst",
        );
        missing_price.fill_price = None;
        let err = fifo_match(&[missing_price], target).expect_err("missing price must fail");
        assert!(err.contains("fill_price missing/invalid"));
    }

    #[test]
    fn fifo_only_emits_target_date_sells() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                200,
                "2026-07-16 10:00:00",
                "p1",
                "NewsCatalyst",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "sell",
                11.0,
                100,
                "2026-07-17 14:00:00",
                "s1",
                "BR-234四大铁律卖出",
            ),
            fill(
                3,
                "TEST_CODE_600000",
                "sell",
                12.0,
                100,
                "2026-07-18 14:00:00",
                "s2",
                "BR-234四大铁律卖出",
            ),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        assert_eq!(attributions.len(), 1); // 只归当日卖出
        assert_eq!(attributions[0].pnl, 200.0);
        assert_eq!(open.len(), 0);
    }

    #[test]
    fn window_realized_is_cumulative_across_days() {
        // CRIT-1 回归锚点: 窗口已实现 = 窗口内每日卖出累计, 非仅末日单日.
        let end = NaiveDate::from_ymd_opt(2026, 7, 20).expect("valid date");
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                200,
                "2026-07-16 10:00:00",
                "p1",
                "NewsCatalyst",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "sell",
                11.0,
                100,
                "2026-07-17 14:00:00",
                "s1",
                "BR-234四大铁律卖出",
            ),
            fill(
                3,
                "TEST_CODE_600000",
                "sell",
                12.0,
                100,
                "2026-07-20 14:00:00",
                "s2",
                "BR-234四大铁律卖出",
            ),
        ];
        let window = aggregate_window(end, 30, &rows, &HashMap::new()).expect("valid window");
        let window_realized: f64 = window.families.iter().map(|f| f.realized_pnl).sum();
        // 7/17 卖出 (11.0-10.0)*100 = +100; 7/20 卖出 (12.0-10.0)*100 = +200 → 累计 +300
        assert_eq!(window_realized, 300.0);
        assert_eq!(
            window
                .families
                .iter()
                .map(|f| f.realized_trades)
                .sum::<i64>(),
            2
        );
        // 对照: 当日口径只含 7/20 卖出
        let (daily_attributions, _) = fifo_match(&rows, end).expect("valid FIFO fills");
        assert_eq!(daily_attributions.len(), 1);
        assert_eq!(daily_attributions[0].pnl, 200.0);
    }

    #[test]
    fn window_includes_exactly_days() {
        // 30 自然日含首尾: start = end − 29; end−29 卖出入窗, end−30 卖出出窗 (off-by-one 锚点).
        let end = NaiveDate::from_ymd_opt(2026, 7, 20).expect("valid date");
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                300,
                "2026-06-01 10:00:00",
                "p1",
                "NewsCatalyst",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "sell",
                11.0,
                100,
                "2026-06-20 14:00:00",
                "s1",
                "BR-234四大铁律卖出",
            ), // end−30 → 出窗
            fill(
                3,
                "TEST_CODE_600000",
                "sell",
                12.0,
                100,
                "2026-06-21 14:00:00",
                "s2",
                "BR-234四大铁律卖出",
            ), // end−29 → 入窗
        ];
        let window = aggregate_window(end, 30, &rows, &HashMap::new()).expect("valid window");
        let window_realized: f64 = window.families.iter().map(|f| f.realized_pnl).sum();
        // 只有 6/21 卖出 (12.0-10.0)*100 = +200; 若 6/20 误入窗则 +300 (与旧 31 天 off-by-one 同形)
        assert_eq!(window_realized, 200.0);
        assert_eq!(
            window
                .families
                .iter()
                .map(|f| f.realized_trades)
                .sum::<i64>(),
            1
        );
    }

    #[test]
    fn daily_emission_unchanged_with_emit_from_none() {
        // CRIT-1 守卫: fifo_match 2-arg wrapper 与显式 None 均只发射当日卖出 (日级契约不变).
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                200,
                "2026-07-16 10:00:00",
                "p1",
                "NewsCatalyst",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "sell",
                11.0,
                100,
                "2026-07-17 14:00:00",
                "s1",
                "BR-234四大铁律卖出",
            ),
            fill(
                3,
                "TEST_CODE_600000",
                "sell",
                12.0,
                100,
                "2026-07-18 14:00:00",
                "s2",
                "BR-234四大铁律卖出",
            ),
        ];
        let (attributions, _) = fifo_match(&rows, target).expect("valid FIFO fills");
        assert_eq!(attributions.len(), 1);
        assert_eq!(attributions[0].pnl, 200.0);
        assert_eq!(attributions[0].sell_date, target);
        let (from_none, _) = fifo_match_from(&rows, target, None).expect("valid FIFO fills");
        assert_eq!(from_none, attributions); // 显式 None 与 wrapper 逐字节一致
    }

    #[test]
    fn fifo_rejects_invalid_identity_timestamp_and_late_fills() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let err = fifo_match(
            &[fill(
                0,
                "TEST_CODE_600000",
                "buy",
                10.0,
                100,
                "2026-07-18 10:00:00",
                "p1",
                "NewsCatalyst",
            )],
            target,
        )
        .expect_err("id<=0 must fail");
        assert!(err.contains("identity invalid"));
        let empty_code = fill(
            1,
            "",
            "buy",
            10.0,
            100,
            "2026-07-18 10:00:00",
            "p1",
            "NewsCatalyst",
        );
        let err = fifo_match(&[empty_code], target).expect_err("empty code must fail");
        assert!(err.contains("identity invalid"));
        let bad_ts = fill(
            1,
            "TEST_CODE_600000",
            "buy",
            10.0,
            100,
            "not-a-timestamp",
            "p1",
            "NewsCatalyst",
        );
        let err = fifo_match(&[bad_ts], target).expect_err("bad timestamp must fail");
        assert!(err.contains("timestamp invalid"));
        let late = fill(
            1,
            "TEST_CODE_600000",
            "buy",
            10.0,
            100,
            "2026-07-19 10:00:00",
            "p1",
            "NewsCatalyst",
        );
        let err = fifo_match(&[late], target).expect_err("later than settlement must fail");
        assert!(err.contains("later than settlement date"));
    }

    #[test]
    fn fifo_rejects_unordered_fills_invalid_direction_and_quantity() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let unordered = vec![
            fill(
                2,
                "TEST_CODE_600000",
                "buy",
                10.0,
                100,
                "2026-07-18 10:00:00",
                "p1",
                "NewsCatalyst",
            ),
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                100,
                "2026-07-18 09:00:00",
                "p2",
                "NewsCatalyst",
            ),
        ];
        let err = fifo_match(&unordered, target).expect_err("unordered fills must fail");
        assert!(err.contains("not ordered"));
        let bad_dir = fill(
            1,
            "TEST_CODE_600000",
            "hold",
            10.0,
            100,
            "2026-07-18 10:00:00",
            "p1",
            "NewsCatalyst",
        );
        let err = fifo_match(&[bad_dir], target).expect_err("invalid direction must fail");
        assert!(err.contains("direction invalid"));
        let bad_qty = fill(
            1,
            "TEST_CODE_600000",
            "buy",
            10.0,
            150,
            "2026-07-18 10:00:00",
            "p1",
            "NewsCatalyst",
        );
        let err = fifo_match(&[bad_qty], target).expect_err("invalid quantity must fail");
        assert!(err.contains("quantity invalid"));
    }

    #[test]
    fn fifo_rejects_sell_without_matched_buys() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let sell_only = vec![fill(
            1,
            "TEST_CODE_600000",
            "sell",
            11.0,
            100,
            "2026-07-18 14:00:00",
            "s1",
            "BR-234四大铁律卖出",
        )];
        let err = fifo_match(&sell_only, target).expect_err("sell without buys must fail");
        assert!(err.contains("no matched buy lots"));
        // 注: non-finite PnL 分支 (fifo_match 末尾) 在 price/quantity 前置校验下不可达,
        // 不做直测 — 与 snapshot.rs 移植副本同理由.
    }

    #[test]
    fn top_trades_keeps_five_per_side_ordered() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        // 6 盈利 (+10..+60) + 6 亏损 (-10..-60), 各截断到 ≤5
        let mut attributions = Vec::new();
        for i in 1..=6 {
            attributions.push(TradeAttribution {
                sell_id: i,
                code: format!("TEST_CODE_60000{i}"),
                pnl: (i * 10) as f64,
                entry_plan_id: format!("w{i}"),
                entry_family: SignalFamily::NewsCatalyst,
                exit_reason: "BR-234四大铁律卖出".to_string(),
                suspicious: false,
                sell_date: target,
            });
            attributions.push(TradeAttribution {
                sell_id: 10 + i,
                code: format!("TEST_CODE_6000{i}0"),
                pnl: -(i * 10) as f64,
                entry_plan_id: format!("l{i}"),
                entry_family: SignalFamily::ExitByRule,
                exit_reason: "BR-234四大铁律卖出".to_string(),
                suspicious: false,
                sell_date: target,
            });
        }
        let top = top_trades(&attributions);
        assert_eq!(top.len(), 10); // 盈利 5 + 亏损 5
        assert_eq!(top[0].pnl, 60.0); // 盈利降序在前
        assert_eq!(top[4].pnl, 20.0);
        assert_eq!(top[5].pnl, -60.0); // 亏损升序 (最负在前) 在后
        assert_eq!(top[9].pnl, -20.0);
    }

    #[test]
    fn aggregate_families_sums_realized_and_unrealized() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                100,
                "2026-07-17 10:00:00",
                "news-1",
                "NewsCatalyst",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "buy",
                12.0,
                200,
                "2026-07-18 09:31:00",
                "fund-2",
                "MainNetInflow",
            ),
            fill(
                3,
                "TEST_CODE_600000",
                "sell",
                15.0,
                200,
                "2026-07-18 14:00:00",
                "sell-3",
                "BR-234四大铁律卖出:结构止损",
            ),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        // T2 review Minor-2 (carried): 锁 open lot 契约 — plan_id/family 贯通 fifo_match → 聚合
        assert_eq!(open[0].plan_id, "fund-2");
        assert_eq!(open[0].family, SignalFamily::MainNetInflow);
        let mut prices = HashMap::new();
        prices.insert("TEST_CODE_600000".to_string(), 16.0);
        let families = aggregate_families(&attributions, &open, &prices);

        let news = families
            .iter()
            .find(|f| f.family == SignalFamily::NewsCatalyst)
            .expect("news family");
        assert_eq!(news.realized_pnl, 500.0);
        assert_eq!(news.realized_trades, 1);
        assert_eq!(news.wins, 1);
        assert_eq!(news.losses, 0);
        assert_eq!(news.win_rate, Some(1.0));
        assert_eq!(news.unrealized_pnl, 0.0);
        assert_eq!(news.open_lots, 0);

        let fund = families
            .iter()
            .find(|f| f.family == SignalFamily::MainNetInflow)
            .expect("fund family");
        assert_eq!(fund.realized_pnl, 300.0);
        // 剩余 100 股 × (16.0 - 12.0) = +400 浮盈
        assert_eq!(fund.unrealized_pnl, 400.0);
        assert_eq!(fund.open_lots, 1);
        assert_eq!(fund.total_pnl, 700.0);
    }

    #[test]
    fn missing_close_price_counts_unvalued_not_silent() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                100,
                "2026-07-17 10:00:00",
                "news-1",
                "NewsCatalyst",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "buy",
                12.0,
                100,
                "2026-07-18 09:31:00",
                "news-2",
                "NewsCatalyst",
            ),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        let prices = HashMap::new(); // 无任何收盘价
        let families = aggregate_families(&attributions, &open, &prices);
        let news = families
            .iter()
            .find(|f| f.family == SignalFamily::NewsCatalyst)
            .expect("news family");
        assert_eq!(news.open_lots, 2);
        assert_eq!(news.unvalued_lots, 2);
        assert_eq!(news.unrealized_pnl, 0.0); // 未估值不填零假装, 但计数出声
        assert_eq!(news.suspicious_lots, 0);
    }

    #[test]
    fn suspicious_lots_are_counted_per_family() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![fill(
            1,
            "TEST_CODE_600000",
            "buy",
            10.0,
            100,
            "2026-07-17 10:00:00",
            "p1",
            "盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%",
        )];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        let families = aggregate_families(&attributions, &open, &HashMap::new());
        let fund = families
            .iter()
            .find(|f| f.family == SignalFamily::PostCloseFundInflow)
            .expect("fund family");
        assert_eq!(fund.suspicious_lots, 1);
    }
}
