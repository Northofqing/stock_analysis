//! 行情数据抓取 — 从 main.rs 提取（BR-210 evidence timestamp contract）

use crate::freshness::validate_daily_snapshot_freshness;
use crate::validate_position_freshness;

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

/// BR-164 持仓实时行情：只消费统一 Magic provider Gateway。
pub fn fetch_position_quotes() -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    // BR-227: 无券商时持仓代码来自 BR-226 用户确认快照 (24h 新鲜度),
    // 行情经统一网关获取 (自带 source_at 证据); 持仓批次来源时间门
    // 不再连坐行情获取 (BR-217 的券商批次要求由用户快照替代)。
    let codes: Vec<String> =
        match stock_analysis::database::user_position_snapshot::latest_user_position_snapshot() {
            Ok(Some(snapshot))
                if !snapshot.confirm_empty
                    && chrono::Local::now()
                        .signed_duration_since(snapshot.effective_at.with_timezone(&chrono::Local))
                        .num_hours()
                        <= 24 =>
            {
                snapshot.items.iter().map(|item| item.code.clone()).collect()
            }
            Ok(Some(_)) | Ok(None) => {
                // 快照缺失/过期: 回退本地持仓代码 (仅行情展示用途, 行情自带来源时间)
                stock_analysis::portfolio::get_positions()
                    .map_err(|error| format!("持仓批次查询失败: {error}"))?
                    .into_iter()
                    .map(|position| position.code)
                    .collect()
            }
            Err(error) => {
                return Err(format!("用户持仓快照读取失败: {error}"));
            }
        };
    if codes.is_empty() {
        return Ok(vec![]);
    }

    let quotes = fetch_realtime_quotes(&codes)?;
    if quotes.is_empty() {
        return Err("持仓行情源成功响应但无有效行".to_string());
    }
    mark_capability_success(stock_analysis::monitor::data_mode::Capability::Quote)?;
    Ok(quotes)
}

/// BR-164 public quote projection over the evidence-preserving Gateway batch.
pub fn fetch_realtime_quotes(
    codes: &[String],
) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    Ok(fetch_realtime_quote_batch(codes)?.stocks)
}

/// BR-159 evidence-preserving quote batch for downstream atomic joins.
pub(super) fn fetch_realtime_quote_batch(codes: &[String]) -> Result<TopStockBatch, String> {
    let batch = stock_analysis::data_gateway::MarketDataGateway::new()
        .realtime_quotes(codes)
        .map_err(|error| format!("统一实时行情 Gateway 不可用: {error}"))?;
    let projected = project_top_stock_batch(codes, batch)?;
    audit_top_stock_projection(&projected);
    mark_capability_success(stock_analysis::monitor::data_mode::Capability::Quote)?;
    Ok(projected)
}

#[derive(Debug)]
pub(super) struct TopStockBatch {
    pub(super) stocks: Vec<stock_analysis::market_data::TopStock>,
    pub(super) evidence: stock_analysis::data_gateway::BatchEvidence,
}

fn project_top_stock_batch(
    codes: &[String],
    batch: stock_analysis::data_gateway::GatewayBatch<
        stock_analysis::data_gateway::RealtimeMarketQuote,
    >,
) -> Result<TopStockBatch, String> {
    use stock_analysis::data_gateway::GatewayBatch;
    use stock_analysis::market_data::TopStock;

    let (quotes, evidence) = match batch {
        GatewayBatch::Available { records, evidence } if !records.is_empty() => (records, evidence),
        GatewayBatch::Available { .. } | GatewayBatch::VerifiedEmpty(_) => {
            return Err("统一实时行情 Gateway 返回不允许的空批次".to_string());
        }
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
            let change_pct = validate_change_pct(
                &quote.code,
                &quote.name,
                quote.change_percent,
                "统一实时行情",
            )?;
            Ok(TopStock {
                code: quote.code,
                name: quote.name,
                price: quote.price,
                change_pct,
                volume_ratio: None,
                main_net_yi: None,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    validate_quote_batch_subset(codes, &stocks, "unified_market_gateway")?;
    Ok(TopStockBatch { stocks, evidence })
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

pub fn infer_limit_pct(code: &str, name: &str) -> f64 {
    #[cfg(test)]
    let code = code.strip_prefix("TEST_CODE_").unwrap_or(code);
    if name.contains("ST") || name.contains("st") {
        5.0
    } else if code.starts_with("30") || code.starts_with("688") {
        20.0
    } else if code.starts_with('8') || code.starts_with('4') || code.starts_with("92") {
        30.0
    } else {
        10.0
    }
}

fn validate_change_pct(
    code: &str,
    name: &str,
    change_pct: f64,
    source: &str,
) -> Result<f64, String> {
    let limit = infer_limit_pct(code, name);
    if !change_pct.is_finite() {
        return Err(format!(
            "{source} {code}({name}) change_pct 缺失/非法: {change_pct:?}"
        ));
    }
    if change_pct.abs() > limit {
        return Err(format!(
            "[DQ-2.3] {source} {code}({name}) change_pct={change_pct:.2}% 超过证券板块±{limit:.0}%规则阈值"
        ));
    }
    Ok(change_pct)
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
    let threshold = infer_limit_pct(code, name) - 0.2;
    let history_start = usize::from(latest.date == today);
    let prior_limit_days = kline
        .iter()
        .skip(history_start)
        .take(2)
        .take_while(|bar| bar.is_limit_up || bar.pct_chg >= threshold)
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
    use stock_analysis::magic_compat::ProviderId;
    use stock_analysis::data_gateway::{BatchEvidence, GatewayBatch, RealtimeMarketQuote};
    use stock_analysis::data_provider::{AdjustType, KlineData};
    use stock_analysis::market_data::TopStock;

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
    fn limit_percent_inference_covers_st_and_all_registered_boards() {
        assert_eq!(infer_limit_pct("TEST_CODE_600000", "普通测试股"), 10.0);
        assert_eq!(infer_limit_pct("TEST_CODE_300001", "创业板测试股"), 20.0);
        assert_eq!(infer_limit_pct("TEST_CODE_688001", "科创板测试股"), 20.0);
        assert_eq!(infer_limit_pct("TEST_CODE_830001", "北交所测试股"), 30.0);
        assert_eq!(infer_limit_pct("TEST_CODE_920001", "北交所测试股"), 30.0);
        assert_eq!(infer_limit_pct("TEST_CODE_600001", "*ST测试"), 5.0);
    }

    #[test]
    fn board_aware_change_pct_validation_accepts_real_values_and_flags_overrun() {
        assert!(validate_change_pct("TEST_CODE_300001", "创业板测试股", 20.0, "test").is_ok());
        assert!(validate_change_pct("TEST_CODE_920305", "北交所测试股", 30.0, "test").is_ok());
        assert!(validate_change_pct("TEST_CODE_920305", "北交所测试股", 30.01, "test").is_err());
        assert!(validate_change_pct("TEST_CODE_600001", "普通测试股", f64::NAN, "test").is_err());
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
            change_percent: 10.01,
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
        assert!(error.contains("超过证券板块"));
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

        assert!(error.contains("observed_at 非法"), "{error}");
        assert!(error.contains("invalid_evidence"), "{error}");
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
