//! 行情数据抓取 — 从 main.rs 提取（BR-210 evidence timestamp contract）。
//! BR-236 禁止过期或未来行情刷新实时 Quote/DataMode 能力。

use crate::freshness::validate_daily_snapshot_freshness;

pub(super) fn validate_quote_batch_codes(
    requested: &[String],
    quotes: &[stock_analysis::market_data::TopStock],
    source: &str,
) -> Result<(), String> {
    use std::collections::HashSet;

    let requested_set: HashSet<&str> = requested.iter().map(String::as_str).collect();
    if requested_set.len() != requested.len() {
        return Err(format!("{source} 请求代码包含重复项"));
    }
    let mut returned_set = HashSet::new();
    for quote in quotes {
        if !returned_set.insert(quote.code.as_str()) {
            return Err(format!("{source} 行情重复代码: {}", quote.code));
        }
    }
    if returned_set != requested_set {
        let mut missing: Vec<&str> = requested_set.difference(&returned_set).copied().collect();
        let mut extra: Vec<&str> = returned_set.difference(&requested_set).copied().collect();
        missing.sort_unstable();
        extra.sort_unstable();
        return Err(format!(
            "{source} 行情批次代码不完整: missing={missing:?} extra={extra:?}"
        ));
    }
    Ok(())
}

/// BR-218: a freshness-partitioned quote batch may legitimately be a strict
/// subset of the request. Consumers that only monitor a list of instruments
/// accept the subset; excluded codes stay absent (AGENTS §2.2) and are
/// re-acquired next round. Duplicates and unrequested codes remain failures.
pub(super) fn validate_quote_batch_subset(
    requested: &[String],
    quotes: &[stock_analysis::market_data::TopStock],
    source: &str,
) -> Result<(), String> {
    use std::collections::HashSet;

    let requested_set: HashSet<&str> = requested.iter().map(String::as_str).collect();
    if requested_set.len() != requested.len() {
        return Err(format!("{source} 请求代码包含重复项"));
    }
    if quotes.is_empty() {
        return Err(format!("{source} 行情批次为空"));
    }
    let mut returned_set = HashSet::new();
    for quote in quotes {
        if !returned_set.insert(quote.code.as_str()) {
            return Err(format!("{source} 行情重复代码: {}", quote.code));
        }
    }
    let mut extra: Vec<&str> = returned_set.difference(&requested_set).copied().collect();
    if !extra.is_empty() {
        extra.sort_unstable();
        return Err(format!("{source} 行情批次含未请求代码: extra={extra:?}"));
    }
    if returned_set.len() != requested_set.len() {
        let mut missing: Vec<&str> = requested_set.difference(&returned_set).copied().collect();
        missing.sort_unstable();
        log::warn!(
            "[BR-218][{source}] 行情批次为请求子集 requested={} admitted={} missing={missing:?}",
            requested_set.len(),
            returned_set.len()
        );
    }
    Ok(())
}

fn mark_capability_success(
    capability: stock_analysis::monitor::data_mode::Capability,
) -> Result<(), String> {
    stock_analysis::monitor::data_mode::mark_capability_success(capability)
}

/// BR-255 归因收盘价 (2026-09-02): 15:05 收盘后 RealtimeQuotes 因 BR-217/218
/// 五秒新鲜度门 100% fail-closed (数据源冻结后报价年龄必超 5s, 9/1+9/2 归因
/// 两次实锤 Sina cooldown), 归因改用 HistoricalDailyBars (tdx-smart) — 与
/// attribution_backfill 工具同源: 无新鲜度门, 收盘价落库后 24/7 可用 (R-13
/// 复盘同源)。目标日 bar 缺失按 Err 返回 (调用方按 retryable 处理, 15:05-15:20
/// 窗口内每 tick 重试), 绝不 panic。
pub fn fetch_attribution_close_prices(
    today: chrono::NaiveDate,
) -> Result<std::collections::HashMap<String, f64>, String> {
    // 持仓代码: BR-226 用户快照 (24h 新鲜) 优先, 否则本地持仓 (与归因同规则)。
    let mut codes: Vec<String> =
        match stock_analysis::database::user_position_snapshot::latest_user_position_snapshot() {
            Ok(Some(snapshot)) if !snapshot.confirm_empty => {
                let fresh = chrono::Local::now()
                    .signed_duration_since(snapshot.effective_at.with_timezone(&chrono::Local))
                    .num_hours()
                    <= 24;
                if fresh {
                    snapshot
                        .items
                        .iter()
                        .map(|item| item.code.clone())
                        .collect()
                } else {
                    stock_analysis::portfolio::get_positions()
                        .map_err(|error| format!("持仓批次查询失败: {error}"))?
                        .into_iter()
                        .map(|position| position.code)
                        .collect()
                }
            }
            _ => stock_analysis::portfolio::get_positions()
                .map_err(|error| format!("持仓批次查询失败: {error}"))?
                .into_iter()
                .map(|position| position.code)
                .collect(),
        };
    if codes.is_empty() {
        return Err("持仓列表为空, 无法拉取归因收盘价".to_string());
    }
    // 当日成交代码并入覆盖: 未估值 lot 全部来自当日新开仓 (快照只覆盖持仓代码),
    // 不并入则当日新开仓浮盈无法估值 (2026-09-01 实测教训)。只读查询。
    {
        let effective =
            stock_analysis::performance::economic_position::query_effective_fills_through(today)?;
        // Include the whole scoped inventory history, not only today's raw fills:
        // a historical correction may change an open lot's economic contribution.
        let trade_codes: Vec<String> = effective
            .rows()
            .map_err(|error| error.to_string())?
            .iter()
            .map(|row| row.code.clone())
            .collect();
        codes.extend(trade_codes);
        codes.sort();
        codes.dedup();
    }

    let gateway = stock_analysis::data_gateway::HistoricalBarsGateway::new();
    let mut prices: std::collections::HashMap<String, f64> =
        std::collections::HashMap::with_capacity(codes.len());
    for code in &codes {
        // 回拉窗口 5 天 (目标日=今天, 保证今日 bar 在返回区间内)。
        // 主路径: gRPC HistoricalBars (fail-closed)。2026-09-22 实测该通道
        // 全天 no_verified_batch → 归因日推整批丢失; 追加服务端 adaptive 链
        // (OutcomeDailyBars op: TDX→腾讯→Sina→Baidu 回退, selection-v2 同源)
        // 兜底, 与 9/3 R-07 tdx 日线回退同精神。
        match gateway.required_daily_bars(code, 5) {
            Ok(admitted) => {
                let bar = admitted
                    .records()
                    .iter()
                    .find(|k| k.date == today)
                    .ok_or_else(|| {
                        let dates: Vec<String> = admitted
                            .records()
                            .iter()
                            .map(|k| k.date.to_string())
                            .collect();
                        format!(
                            "{code}: 目标日 {today} 无日线记录 (可用: {})",
                            dates.join(",")
                        )
                    })?;
                // A second ordinary bar write would erase status just bound
                // by the outcome collector. Reacquire independent dated facts
                // and use the same atomic writer when authority is available.
                let facts = admitted
                    .records()
                    .iter()
                    .map(|bar| qualified_trading_facts(code, bar.date))
                    .collect::<Result<Vec<_>, _>>()?;
                let all_available = facts.iter().all(|fact| {
                    fact.lifecycle().require()
                        == Ok(&stock_analysis::data_gateway::QualifiedListingStatus::Listed)
                        && fact.suspension().require().is_ok()
                });
                let db = stock_analysis::database::DatabaseManager::get();
                if all_available {
                    db.save_admitted_kline_with_trading_facts(&admitted, &facts)
                } else {
                    log::warn!("[attribution] {code} admitted bars have no complete independent daily authority; outcome windows remain unqualified");
                    db.save_admitted_kline_data(&admitted)
                }
                .map_err(|error| format!("{code}: 已准入日线/资格落库失败: {error}"))?;
                prices.insert(code.clone(), bar.close);
            }
            Err(primary_error) => match fetch_close_via_outcome_adaptive(code, today) {
                Some(close) => {
                    log::warn!(
                        "[attribution] {code} HistoricalBars 主路径失败, adaptive 回退成功 close={close}: {primary_error}"
                    );
                    prices.insert(code.clone(), close);
                }
                None => {
                    return Err(format!(
                        "统一行情网关日线不可用 (上游 gRPC 失败): {primary_error}; adaptive 回退同样失败: {code}"
                    ));
                }
            },
        }
    }
    Ok(prices)
}

/// 2026-09-22 回退: 服务端 adaptive 日线链 (OutcomeDailyBars op)。
///
/// gRPC HistoricalBars 通道 fail-closed 无本地回退, 上游全窗口失败时
/// 归因日推整批丢失 (9/22 实测)。OutcomeDailyBars op 由服务端执行
/// TDX→腾讯→Sina→Baidu 的 adaptive transport, 同样走 gRPC 但服务端
/// 侧有提供方回退 (今晨 Baidu accepted=5 实证可用)。
fn fetch_close_via_outcome_adaptive(code: &str, today: chrono::NaiveDate) -> Option<f64> {
    use stock_analysis::data_gateway::grpc_source::bridge_for;
    use stock_analysis::market_domain::instrument::{AssetClass, Exchange, InstrumentId};
    let bridge = bridge_for("OutcomeDailyBars").ok()?;
    let (exchange, market) = match code.chars().next() {
        Some('6') => (Exchange::Shanghai, "SH"),
        Some('0') | Some('3') => (Exchange::Shenzhen, "SZ"),
        Some('4') | Some('8') => (Exchange::Beijing, "BJ"),
        _ => return None,
    };
    let instrument = InstrumentId::new(exchange, code.to_string(), AssetClass::Equity).ok()?;
    let window_start = today - chrono::Duration::days(5);
    let fetched = bridge
        .outcome_daily_bars_adaptive(
            instrument,
            market.to_string(),
            code.to_string(),
            5,
            5,
            window_start,
        )
        .ok()?;
    fetched
        .batch
        .records()
        .iter()
        .find(|bar| {
            chrono::NaiveDate::parse_from_str(bar.bar_start(), "%Y-%m-%d").ok() == Some(today)
        })
        .map(|bar| bar.close().get())
}

/// BR-164 持仓实时行情：只消费统一 Magic provider Gateway。
pub fn fetch_position_quotes() -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    let codes = current_position_quote_codes()?;
    if codes.is_empty() {
        return Ok(vec![]);
    }

    let quotes = fetch_realtime_quotes(&codes)?;
    if quotes.is_empty() {
        return Err("持仓行情源成功响应但无有效行".to_string());
    }
    // This exact current-position acquisition owns Quote health. Arbitrary
    // scanner/probe consumption checks must never refresh it.
    mark_capability_success(stock_analysis::monitor::data_mode::Capability::Quote)?;
    Ok(quotes)
}

fn current_position_quote_codes() -> Result<Vec<String>, String> {
    // BR-227: 无券商时持仓代码来自 BR-226 用户确认快照 (24h 新鲜度),
    // 行情经统一网关获取 (自带 source_at 证据); 持仓批次来源时间门
    // 不再连坐行情获取 (BR-217 的券商批次要求由用户快照替代)。
    match stock_analysis::database::user_position_snapshot::latest_user_position_snapshot() {
        Ok(Some(snapshot))
            if !snapshot.confirm_empty
                && chrono::Local::now()
                    .signed_duration_since(snapshot.effective_at.with_timezone(&chrono::Local))
                    .num_hours()
                    <= 24 =>
        {
            Ok(snapshot
                .items
                .iter()
                .map(|item| item.code.clone())
                .collect())
        }
        Ok(Some(_)) | Ok(None) => {
            // 快照缺失/过期: 回退本地持仓代码 (仅行情展示用途, 行情自带来源时间)
            Ok(stock_analysis::portfolio::get_positions()
                .map_err(|error| format!("持仓批次查询失败: {error}"))?
                .into_iter()
                .map(|position| position.code)
                .collect())
        }
        Err(error) => Err(format!("用户持仓快照读取失败: {error}")),
    }
}

/// A scanner route keeps the Gateway capability and original request together.
/// NoPositions is absence of a request, not empty-source or Quote readiness.
#[derive(Debug)]
pub(super) enum ScannerPositionQuotes {
    NoPositions,
    Available(ScannerPositionQuoteBatch),
}

#[derive(Debug)]
pub(super) struct ScannerPositionQuoteBatch {
    requested: Vec<String>,
    admitted: stock_analysis::data_gateway::market_data::AdmittedRealtimeQuotes,
}

impl ScannerPositionQuotes {
    pub(super) fn quotes(
        &self,
    ) -> &[stock_analysis::data_gateway::market_data::AdmittedRealtimeQuote] {
        match self {
            Self::NoPositions => &[],
            Self::Available(batch) => batch.admitted.quotes(),
        }
    }

    /// Legacy display-only projection. Detector must consume quotes() through
    /// TieredScanner's point-of-use check, rather than this projection.
    pub(super) fn top_stocks(&self) -> Vec<stock_analysis::market_data::TopStock> {
        self.quotes()
            .iter()
            .map(|quote| stock_analysis::market_data::TopStock {
                code: quote.code().to_owned(),
                name: quote.name().to_owned(),
                price: quote.price(),
                change_pct: quote.change_percent(),
                volume_ratio: None,
                main_net_yi: None,
            })
            .collect()
    }

    pub(super) fn requested(&self) -> &[String] {
        match self {
            Self::NoPositions => &[],
            Self::Available(batch) => &batch.requested,
        }
    }
}

/// The single current-position acquisition used by an intraday scanner tick.
/// Reuses the original position resolver; no per-row fetch or code guessing.
pub(super) fn fetch_scanner_position_quotes() -> Result<ScannerPositionQuotes, String> {
    let codes = current_position_quote_codes()?;
    if codes.is_empty() {
        return Ok(ScannerPositionQuotes::NoPositions);
    }
    let admitted = stock_analysis::data_gateway::MarketDataGateway::new()
        .required_realtime_quotes(&codes)
        .map_err(|error| format!("持仓行情严格原始批次不可用: {error}"))?;
    validate_scanner_quote_membership(&codes, admitted.quotes())?;
    // Same exact current-position readiness owner as the legacy Vec route.
    // Point-of-use validation below never touches this capability.
    mark_capability_success(stock_analysis::monitor::data_mode::Capability::Quote)?;
    Ok(ScannerPositionQuotes::Available(
        ScannerPositionQuoteBatch {
            requested: codes,
            admitted,
        },
    ))
}

fn validate_scanner_quote_membership(
    requested: &[String],
    quotes: &[stock_analysis::data_gateway::market_data::AdmittedRealtimeQuote],
) -> Result<(), String> {
    validate_scanner_requested_set(requested, quotes.iter().map(|quote| quote.code()))?;
    let first = quotes
        .first()
        .ok_or_else(|| "持仓行情原始批次为空".to_string())?;
    for quote in quotes {
        if quote.evidence() != first.evidence()
            || quote.source_at() != first.source_at()
            || quote.observed_at() != first.observed_at()
        {
            return Err("持仓行情原始批次身份或时间不一致".to_string());
        }
    }
    Ok(())
}

fn validate_scanner_requested_set<'code>(
    requested: &[String],
    returned: impl IntoIterator<Item = &'code str>,
) -> Result<(), String> {
    use std::collections::HashSet;
    let expected: HashSet<&str> = requested.iter().map(String::as_str).collect();
    if requested.is_empty() || expected.len() != requested.len() || expected.contains("") {
        return Err("持仓行情请求代码必须为非空唯一集合".to_string());
    }
    let mut actual = HashSet::new();
    for code in returned {
        if !actual.insert(code) {
            return Err("持仓行情原始批次含重复代码".to_string());
        }
    }
    if actual != expected {
        return Err("持仓行情原始批次与完整请求集合不一致".to_string());
    }
    Ok(())
}

/// BR-164 public quote projection over the evidence-preserving Gateway batch.
pub fn fetch_realtime_quotes(
    codes: &[String],
) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    let coverage =
        stock_analysis::data_gateway::MarketDataGateway::new().realtime_quote_coverage(codes);
    let projected = project_top_stock_coverage(codes, coverage, true)?;
    Ok(projected.stocks)
}

/// Explicit scanner-only partial projection. The result retains disposition,
/// missing identities, rejected records and raw batch evidence; it never
/// refreshes account-wide Quote health.
#[allow(dead_code)]
pub(super) fn fetch_realtime_quote_partial(codes: &[String]) -> Result<TopStockBatch, String> {
    let coverage =
        stock_analysis::data_gateway::MarketDataGateway::new().realtime_quote_coverage(codes);
    project_top_stock_coverage(codes, coverage, false)
}

/// BR-159 evidence-preserving quote batch for downstream atomic joins.
pub(super) fn fetch_realtime_quote_batch(codes: &[String]) -> Result<TopStockBatch, String> {
    let coverage =
        stock_analysis::data_gateway::MarketDataGateway::new().realtime_quote_coverage(codes);
    let projected = project_top_stock_coverage(codes, coverage, true)?;
    audit_top_stock_projection(&projected);
    Ok(projected)
}

#[derive(Debug)]
pub(super) struct TopStockBatch {
    pub(super) stocks: Vec<stock_analysis::market_data::TopStock>,
    pub(super) evidence: stock_analysis::data_gateway::BatchEvidence,
    pub(super) coverage: stock_analysis::data_gateway::QuoteCoverageDisposition,
    pub(super) requested: Vec<String>,
    pub(super) rejected: Vec<stock_analysis::data_gateway::QuoteRecordRejection>,
    pub(super) missing: Vec<String>,
}

fn project_top_stock_batch(
    codes: &[String],
    batch: stock_analysis::data_gateway::GatewayBatch<
        stock_analysis::data_gateway::RealtimeMarketQuote,
    >,
) -> Result<TopStockBatch, String> {
    let coverage = stock_analysis::data_gateway::RealtimeQuoteCoverage::classify(codes, Ok(batch));
    project_top_stock_coverage(codes, coverage, true)
}

fn project_top_stock_coverage(
    codes: &[String],
    coverage: stock_analysis::data_gateway::RealtimeQuoteCoverage,
    require_complete: bool,
) -> Result<TopStockBatch, String> {
    use stock_analysis::data_gateway::{GatewayBatch, QuoteCoverageDisposition};
    use stock_analysis::market_data::TopStock;

    let disposition = coverage.disposition();
    if disposition == QuoteCoverageDisposition::Unavailable {
        let detail = coverage
            .unavailable_error()
            .map(ToString::to_string)
            .unwrap_or_else(|| "no classified upstream error".to_owned());
        return Err(format!("统一实时行情覆盖不可用: {detail}"));
    }
    let requested = coverage.requested().to_vec();
    let rejected = coverage.rejected().to_vec();
    let missing = coverage.missing().to_vec();
    let evidence = coverage
        .evidence()
        .cloned()
        .ok_or_else(|| "统一实时行情覆盖结果缺少原始批次证据".to_string())?;
    if disposition == QuoteCoverageDisposition::Partial && !require_complete {
        log::warn!(
            "[Task9][QuoteCoverage] partial requested={} accepted={} rejected={:?} missing={:?} batch_id={}",
            coverage.requested().len(),
            coverage.accepted().len(),
            coverage
                .rejected()
                .iter()
                .map(|rejection| (rejection.code.as_deref(), rejection.reason_code))
                .collect::<Vec<_>>(),
            coverage.missing(),
            evidence.batch_id
        );
    }
    let quotes = if require_complete {
        match coverage
            .require_complete()
            .map_err(|error| format!("统一实时行情严格覆盖不可用: {error}"))?
        {
            GatewayBatch::Available { records, .. } => records,
            GatewayBatch::VerifiedEmpty(_) => unreachable!("complete quote coverage is non-empty"),
        }
    } else {
        coverage.into_accepted()
    };
    let evidence_observed_at = stock_analysis::data_gateway::parse_evidence_instant(
        "RealtimeMarketQuotes",
        evidence.provider,
        "observed_at",
        &evidence.observed_at,
    )
    .map_err(|error| {
        format!(
            "统一实时行情批次 observed_at 非法: {:?}: {error}",
            evidence.observed_at
        )
    })?;
    let stocks = quotes
        .into_iter()
        .map(|quote| {
            if quote.provider != evidence.provider
                || quote.batch_id != evidence.batch_id
                || quote.observed_at != evidence_observed_at
            {
                return Err(format!(
                    "统一实时行情 {} 批次身份与证据不一致 provider={:?} batch_id={}",
                    quote.code, quote.provider, quote.batch_id
                ));
            }
            if !quote.price.is_finite() || quote.price <= 0.0 {
                return Err(format!(
                    "统一实时行情 {}({}) price 缺失/非法: {:?}",
                    quote.code, quote.name, quote.price
                ));
            }
            Ok(TopStock {
                code: quote.code,
                name: quote.name,
                price: quote.price,
                change_pct: quote.change_percent,
                volume_ratio: None,
                main_net_yi: None,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    validate_quote_batch_subset(codes, &stocks, "unified_market_gateway")?;
    Ok(TopStockBatch {
        stocks,
        evidence,
        coverage: disposition,
        requested,
        rejected,
        missing,
    })
}

fn audit_top_stock_projection(batch: &TopStockBatch) {
    log::info!(
        "[BR-159][BR-164][TopStockProjection] records={} provider={:?} source={} source_at={} observed_at={} batch_id={}",
        batch.stocks.len(),
        batch.evidence.provider,
        batch.evidence.source,
        batch.evidence.source_at.as_deref().unwrap_or("absent"),
        batch.evidence.observed_at,
        batch.evidence.batch_id
    );
}

fn qualified_trading_facts(
    code: &str,
    effective_on: chrono::NaiveDate,
) -> Result<stock_analysis::data_gateway::QualifiedTradingFacts, String> {
    let identity =
        stock_analysis::data_gateway::instrument_identity::resolve_production_equity(code, None);
    let identity = identity.map_err(|error| format!("{code} 证券身份无效: {error}"))?;
    let request = stock_analysis::data_gateway::QualifiedTradingFactsRequest::new(
        identity.instrument().clone(),
        effective_on,
    );
    Ok(stock_analysis::data_gateway::QualifiedTradingFactsGateway::new().acquire(request))
}

fn qualified_price_band(
    code: &str,
    effective_on: chrono::NaiveDate,
) -> Result<stock_analysis::data_gateway::QualifiedPriceBand, String> {
    let facts = qualified_trading_facts(code, effective_on)?;
    match facts.lifecycle().require() {
        Ok(stock_analysis::data_gateway::QualifiedListingStatus::Listed) => {}
        Ok(status) => {
            return Err(format!(
                "{code} {effective_on} 上市状态不可交易: {status:?}"
            ))
        }
        Err(error) => {
            return Err(format!(
                "{code} {effective_on} lifecycle unavailable reason_code={}: {}",
                error.reason_code(),
                error.message()
            ))
        }
    }
    match facts.suspension().require() {
        Ok(stock_analysis::data_gateway::QualifiedSuspensionStatus::Trading) => {}
        Ok(status) => {
            return Err(format!(
                "{code} {effective_on} 停复牌状态不可交易: {status:?}"
            ))
        }
        Err(error) => {
            return Err(format!(
                "{code} {effective_on} suspension unavailable reason_code={}: {}",
                error.reason_code(),
                error.message()
            ))
        }
    }
    facts.price_regime().require().cloned().map_err(|error| {
        format!(
            "{code} {effective_on} price_regime unavailable reason_code={}: {}",
            error.reason_code(),
            error.message()
        )
    })
}

fn price_to_micros(code: &str, price: f64) -> Result<i64, String> {
    let scaled = price * 1_000_000.0;
    if !scaled.is_finite() || scaled <= 0.0 || scaled >= i64::MAX as f64 {
        return Err(format!("{code} 价格不可转换为 micro-CNY: {price:?}"));
    }
    let rounded = scaled.round();
    if (scaled - rounded).abs() > 1e-6 {
        return Err(format!("{code} 价格精度超过 micro-CNY: {price:?}"));
    }
    Ok(rounded as i64)
}

pub(super) fn is_qualified_limit_up_quote(
    quote: &stock_analysis::market_data::TopStock,
    effective_on: chrono::NaiveDate,
) -> Result<bool, String> {
    let band = qualified_price_band(&quote.code, effective_on)?;
    let price_micros = price_to_micros(&quote.code, quote.price)?;
    band.validate_price_micros(price_micros).map_err(|error| {
        format!(
            "{} {} 价格不满足合格制度: {error}",
            quote.code, effective_on
        )
    })?;
    Ok(price_micros == band.upper_price_micros())
}

/// 批量查询连板数，返回 1=首板 / 2=二板 / 3=三板+
/// 仅向前看 4 个交易日的 K 线，够判断三板就够了。
#[derive(Debug)]
struct BoardLevelFact {
    code: String,
    level: u8,
    evidence: stock_analysis::data_gateway::BatchEvidence,
}

fn classify_board_level(
    code: &str,
    name: &str,
    batch: &stock_analysis::data_gateway::AdmittedDailyBars,
    today: chrono::NaiveDate,
) -> Result<BoardLevelFact, String> {
    classify_board_level_from_parts(code, name, batch.records(), batch.evidence(), today)
}

fn classify_board_level_from_parts(
    code: &str,
    name: &str,
    kline: &[stock_analysis::data_provider::KlineData],
    evidence: &stock_analysis::data_gateway::BatchEvidence,
    today: chrono::NaiveDate,
) -> Result<BoardLevelFact, String> {
    if kline.len() < 3 {
        return Err(format!(
            "[连板识别] {name}({code}) 日线样本不足: required>=3 actual={} source={} batch_id={}",
            kline.len(),
            evidence.source,
            evidence.batch_id
        ));
    }
    let latest = kline
        .first()
        .ok_or_else(|| format!("[连板识别] {name}({code}) K 线为空"))?;
    let history_start = usize::from(latest.date == today);
    let prior_limit_days = kline
        .iter()
        .skip(history_start)
        .take(2)
        .take_while(|bar| bar.is_limit_up)
        .count();
    let level = u8::try_from(1 + prior_limit_days)
        .map_err(|_| format!("[连板识别] {name}({code}) 连板数溢出"))?;
    Ok(BoardLevelFact {
        code: code.to_string(),
        level,
        evidence: evidence.clone(),
    })
}

fn lookup_board_level_facts(codes: &[(String, String)]) -> Result<Vec<BoardLevelFact>, String> {
    let mut seen = std::collections::HashSet::with_capacity(codes.len());
    for (code, _) in codes {
        if !seen.insert(code.as_str()) {
            return Err(format!("[连板识别] 请求代码包含重复项: {code}"));
        }
    }
    let gateway = stock_analysis::data_gateway::HistoricalBarsGateway::new();
    let today = chrono::Local::now().date_naive();
    let mut facts = Vec::with_capacity(codes.len());

    for (code, name) in codes {
        // Board-level decisions are price-limit decisions. Until the exact
        // per-security/day regime is qualified, do not let historical pct/name
        // heuristics manufacture a limit-up streak.
        let _current_regime = qualified_price_band(code, today)?;
        let batch = gateway
            .required_daily_bars(code, 5)
            .map_err(|error| format!("[连板识别] {name}({code}) 统一日线不可用: {error}"))?;
        let latest = batch
            .records()
            .first()
            .ok_or_else(|| format!("[连板识别] {name}({code}) K 线为空"))?;
        if !validate_daily_snapshot_freshness(latest.date, &batch.evidence().source, code) {
            return Err(format!(
                "[连板识别] {name}({code}) 最新日 K {} 不满足时效门 source={} batch_id={}",
                latest.date,
                batch.evidence().source,
                batch.evidence().batch_id
            ));
        }
        facts.push(classify_board_level(code, name, &batch, today)?);
    }
    Ok(facts)
}

pub fn lookup_board_level_batch(
    codes: &[(String, String)],
) -> Result<std::collections::HashMap<String, u8>, String> {
    let facts = lookup_board_level_facts(codes)?;
    let mut out = std::collections::HashMap::with_capacity(facts.len());
    for fact in facts {
        log::info!(
            "[BR-159][BR-164][连板识别] code={} level={} provider={:?} source={} source_at={} observed_at={} batch_id={}",
            fact.code,
            fact.level,
            fact.evidence.provider,
            fact.evidence.source,
            fact.evidence.source_at.as_deref().unwrap_or("absent"),
            fact.evidence.observed_at,
            fact.evidence.batch_id
        );
        if out.insert(fact.code.clone(), fact.level).is_some() {
            return Err(format!(
                "[连板识别] 已分类批次出现重复代码，拒绝覆盖: {}",
                fact.code
            ));
        }
    }
    Ok(out)
}

pub(super) const FULL_MARKET_RANKINGS_UNAVAILABLE_REASON: &str =
    "provider_capability_not_live_admitted";
pub(super) const FULL_MARKET_RANKINGS_UNAVAILABLE_AUDIT: &str =
    "capability_unavailable:provider_capability_not_live_admitted";

/// BR-190: one explicit state marker for the retired full-market ranking paths.
///
/// This is deliberately not a fetch facade: provider admission is false, so a
/// request would be a dead call and an empty result would misstate unavailable
/// evidence as a verified empty ranking.
pub(super) fn log_full_market_rankings_unavailable(owner: &str) {
    log::warn!(
        "[BR-190][FullMarketRankings] owner={} status=unavailable reason_code={} metrics=volume_ratio,main_net_inflow retryable=false",
        owner,
        FULL_MARKET_RANKINGS_UNAVAILABLE_REASON
    );
}

#[cfg(test)]
mod quote_batch_tests {
    use super::*;
    use stock_analysis::data_gateway::{BatchEvidence, GatewayBatch, RealtimeMarketQuote};
    use stock_analysis::data_provider::{AdjustType, KlineData};
    use stock_analysis::market_data::TopStock;
    use stock_analysis::market_domain::ProviderId;

    fn quote(code: &str) -> TopStock {
        TopStock {
            code: code.to_string(),
            name: code.to_string(),
            change_pct: 1.0,
            price: 10.0,
            volume_ratio: None,
            main_net_yi: None,
        }
    }

    #[test]
    fn scanner_quote_complete_request_membership_rejects_subset_duplicates_and_extras() {
        let request = vec!["TEST_CODE_000001".into(), "TEST_CODE_600000".into()];
        assert!(
            validate_scanner_requested_set(&request, ["TEST_CODE_600000", "TEST_CODE_000001"])
                .is_ok()
        );
        for returned in [
            vec!["TEST_CODE_000001"],
            vec!["TEST_CODE_000001", "TEST_CODE_000001"],
            vec!["TEST_CODE_000001", "TEST_CODE_300001"],
            vec!["TEST_CODE_000001", "TEST_CODE_600000", "TEST_CODE_300001"],
            vec![],
        ] {
            assert!(validate_scanner_requested_set(&request, returned).is_err());
        }
        assert!(validate_scanner_requested_set(&[], ["TEST_CODE_000001"]).is_err());
        assert!(validate_scanner_requested_set(
            &["TEST_CODE_000001".into(), "TEST_CODE_000001".into()],
            ["TEST_CODE_000001"]
        )
        .is_err());
        assert!(validate_scanner_requested_set(&["".into()], [""]).is_err());
    }

    #[test]
    fn scanner_quote_no_positions_is_not_an_available_empty_gateway_batch() {
        let observation = ScannerPositionQuotes::NoPositions;
        assert!(matches!(&observation, ScannerPositionQuotes::NoPositions));
        assert!(observation.requested().is_empty());
        assert!(observation.quotes().is_empty());
        assert!(observation.top_stocks().is_empty());
    }

    #[test]
    fn br190_unavailable_disposition_is_not_empty_or_retryable() {
        assert_eq!(
            FULL_MARKET_RANKINGS_UNAVAILABLE_REASON,
            "provider_capability_not_live_admitted"
        );
        assert_eq!(
            FULL_MARKET_RANKINGS_UNAVAILABLE_AUDIT,
            "capability_unavailable:provider_capability_not_live_admitted"
        );
        assert!(!FULL_MARKET_RANKINGS_UNAVAILABLE_AUDIT.contains("empty"));
        assert!(!FULL_MARKET_RANKINGS_UNAVAILABLE_AUDIT.contains("retry"));
    }

    fn daily_evidence() -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_magic_tdx_daily".to_string(),
            source_at: Some("2026-07-26".to_string()),
            observed_at: "2026-07-26T08:00:00Z".to_string(),
            batch_id: "TEST_CODE_daily_batch".to_string(),
        }
    }

    fn daily_bar(date: chrono::NaiveDate, pct_chg: f64, is_limit_up: bool) -> KlineData {
        KlineData {
            date,
            open: 10.0,
            high: 10.5,
            low: 9.8,
            close: 10.4,
            volume: 1_000.0,
            amount: 10_000.0,
            pct_chg,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up,
            is_limit_down: false,
            is_suspended: false,
            adjust: AdjustType::None,
        }
    }

    #[test]
    fn br097_quote_batch_requires_exact_code_set() {
        let requested = vec![
            "TEST_CODE_000001".to_string(),
            "TEST_CODE_600000".to_string(),
        ];
        assert!(validate_quote_batch_codes(
            &requested,
            &[quote("TEST_CODE_000001"), quote("TEST_CODE_600000")],
            "test"
        )
        .is_ok());
        assert!(
            validate_quote_batch_codes(&requested, &[quote("TEST_CODE_000001")], "test").is_err()
        );
        assert!(validate_quote_batch_codes(
            &requested,
            &[quote("TEST_CODE_000001"), quote("TEST_CODE_000001")],
            "test"
        )
        .is_err());
        assert!(validate_quote_batch_codes(
            &requested,
            &[quote("TEST_CODE_000001"), quote("TEST_CODE_300001")],
            "test"
        )
        .is_err());
    }

    #[test]
    fn br159_top_stock_projection_retains_evidence_and_rejects_bad_market_data_without_network() {
        let observed_at = chrono::DateTime::parse_from_rfc3339("2026-07-26T08:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let evidence = BatchEvidence {
            provider: ProviderId::Tencent,
            source: "TEST_CODE_magic_tencent_quote".to_string(),
            source_at: Some("2026-07-26T08:00:00Z".to_string()),
            observed_at: "2026-07-26T08:00:00Z".to_string(),
            batch_id: "TEST_CODE_quote_batch".to_string(),
        };
        let quote = RealtimeMarketQuote {
            code: "TEST_CODE_600001".to_string(),
            name: "普通测试股".to_string(),
            price: 10.0,
            previous_close: 9.5,
            change_percent: 5.0,
            source_at: observed_at,
            observed_at,
            provider: ProviderId::Tencent,
            batch_id: evidence.batch_id.clone(),
        };
        let projected = project_top_stock_batch(
            &["TEST_CODE_600001".to_string()],
            GatewayBatch::Available {
                records: vec![quote.clone()],
                evidence: evidence.clone(),
            },
        )
        .unwrap();
        assert_eq!(projected.stocks.len(), 1);
        assert_eq!(projected.evidence, evidence);

        let bad_quote = RealtimeMarketQuote {
            price: f64::NAN,
            ..quote
        };
        let error = project_top_stock_batch(
            &["TEST_CODE_600001".to_string()],
            GatewayBatch::Available {
                records: vec![bad_quote],
                evidence: projected.evidence,
            },
        )
        .unwrap_err();
        assert!(error.contains("quote_coverage_unavailable"), "{error}");
    }

    fn projection_batch_with_observed_at(
        provider: ProviderId,
        encoded_observed_at: &str,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> GatewayBatch<RealtimeMarketQuote> {
        let batch_id = format!("TEST_CODE_BR210_{provider:?}_BATCH");
        GatewayBatch::Available {
            records: vec![RealtimeMarketQuote {
                code: "TEST_CODE_600001".to_string(),
                name: "普通测试股".to_string(),
                price: 10.0,
                previous_close: 9.5,
                change_percent: 5.0,
                source_at: observed_at,
                observed_at,
                provider,
                batch_id: batch_id.clone(),
            }],
            evidence: BatchEvidence {
                provider,
                source: format!("TEST_CODE_magic_{provider:?}_quote"),
                source_at: Some(encoded_observed_at.to_string()),
                observed_at: encoded_observed_at.to_string(),
                batch_id,
            },
        }
    }

    #[test]
    fn br210_projection_accepts_magic_tdx_integer_epoch_seconds() {
        let observed_at = chrono::DateTime::<chrono::Utc>::from_timestamp(1_785_799_979, 0)
            .expect("valid TEST_CODE epoch seconds");
        let projected = project_top_stock_batch(
            &["TEST_CODE_600001".to_string()],
            projection_batch_with_observed_at(ProviderId::Tdx, "1785799979", observed_at),
        )
        .expect("Magic TDX integer epoch evidence must project");

        assert_eq!(projected.stocks.len(), 1);
        assert_eq!(projected.evidence.observed_at, "1785799979");
    }

    #[test]
    fn br210_projection_accepts_tencent_and_sina_fractional_epoch_seconds() {
        for (provider, encoded, nanos) in [
            (ProviderId::Tencent, "1785799979.851045000", 851_045_000),
            (ProviderId::Sina, "1785799979.3", 300_000_000),
        ] {
            let observed_at = chrono::DateTime::<chrono::Utc>::from_timestamp(1_785_799_979, nanos)
                .expect("valid TEST_CODE fractional epoch seconds");
            let projected = project_top_stock_batch(
                &["TEST_CODE_600001".to_string()],
                projection_batch_with_observed_at(provider, encoded, observed_at),
            )
            .unwrap_or_else(|error| panic!("provider={provider:?} encoding={encoded}: {error}"));

            assert_eq!(projected.stocks.len(), 1);
            assert_eq!(projected.evidence.observed_at, encoded);
        }
    }

    #[test]
    fn br210_projection_rejects_malformed_magic_observation_evidence() {
        let observed_at = chrono::DateTime::<chrono::Utc>::from_timestamp(1_785_799_979, 0)
            .expect("valid TEST_CODE epoch seconds");
        let error = project_top_stock_batch(
            &["TEST_CODE_600001".to_string()],
            projection_batch_with_observed_at(
                ProviderId::Tencent,
                "1785799979.8510450000",
                observed_at,
            ),
        )
        .expect_err("over-precision Magic observation evidence must fail closed");

        assert!(error.contains("invalid observed_at timestamp"), "{error}");
        assert!(error.contains("reason_code=invalid_evidence"), "{error}");
    }

    #[test]
    fn br164_board_level_uses_admitted_daily_bars_and_retains_evidence() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 26).unwrap();
        let records = vec![
            daily_bar(today, 10.0, true),
            daily_bar(today - chrono::Duration::days(1), 10.0, true),
            daily_bar(today - chrono::Duration::days(2), 10.0, true),
        ];
        let evidence = daily_evidence();

        let fact = classify_board_level_from_parts(
            "TEST_CODE_600001",
            "普通测试股",
            &records,
            &evidence,
            today,
        )
        .unwrap();

        assert_eq!(fact.code, "TEST_CODE_600001");
        assert_eq!(fact.level, 3);
        assert_eq!(fact.evidence.batch_id, "TEST_CODE_daily_batch");
        assert_eq!(fact.evidence.source, "TEST_CODE_magic_tdx_daily");
    }

    #[test]
    fn br106_board_level_rejects_insufficient_admitted_sample() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 26).unwrap();
        let records = vec![
            daily_bar(today, 10.0, true),
            daily_bar(today - chrono::Duration::days(1), 10.0, true),
        ];
        let evidence = daily_evidence();

        let error = classify_board_level_from_parts(
            "TEST_CODE_600001",
            "普通测试股",
            &records,
            &evidence,
            today,
        )
        .expect_err("short sample must be rejected");

        assert!(error.contains("日线样本不足"));
        assert!(error.contains("TEST_CODE_daily_batch"));
    }

    #[test]
    fn br164_duplicate_board_request_is_rejected_before_any_network_call() {
        let duplicate = vec![
            ("TEST_CODE_600001".to_string(), "协议测试股".to_string()),
            ("TEST_CODE_600001".to_string(), "协议测试股".to_string()),
        ];
        let error = lookup_board_level_facts(&duplicate).unwrap_err();
        assert!(error.contains("请求代码包含重复项"));
    }
}
