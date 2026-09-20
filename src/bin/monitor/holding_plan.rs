use sha2::{Digest, Sha256};
use stock_analysis::database::user_position_snapshot::UserPositionSnapshot;
use stock_analysis::market_domain::{AssetClass, Exchange, InstrumentId};

use super::{durable_delivery_runtime, market_data, push_templates, PreparedHoldingPlan};

pub(super) fn prepare_holding_plan_messages_with(
    banner: &push_templates::BannerCtx,
    load_snapshot: impl FnOnce() -> Result<Option<UserPositionSnapshot>, String>,
    fetch_quote_batch: impl FnOnce(&[String]) -> Result<market_data::TopStockBatch, String>,
    capture_now: impl FnOnce() -> chrono::DateTime<chrono::FixedOffset>,
) -> Result<Vec<PreparedHoldingPlan>, String> {
    let observed_at = capture_now();
    let snapshot = load_snapshot()
        .map_err(|error| format!("持仓快照读取失败: {error}"))?
        .ok_or_else(|| "无用户确认持仓快照 (BR-226)".to_string())?;
    if snapshot.confirm_empty || snapshot.items.is_empty() {
        return Ok(Vec::new());
    }

    let requested_codes = snapshot
        .items
        .iter()
        .map(|item| item.code.clone())
        .collect::<Vec<_>>();
    let quote_batch = fetch_quote_batch(&requested_codes)
        .map_err(|error| format!("持仓行情批次拒绝: {error}"))?;
    let quote_map: std::collections::HashMap<String, &stock_analysis::market_data::TopStock> =
        quote_batch
            .stocks
            .iter()
            .map(|quote| (quote.code.clone(), quote))
            .collect();

    let business_date = observed_at.date_naive();
    let hhmm = observed_at.format("%H:%M").to_string();
    let mut out = Vec::new();
    for item in &snapshot.items {
        let Some(quote) = quote_map.get(&item.code) else {
            log::warn!("[T-03] code={} 行情缺失, 跳过该票 (其余照常)", item.code);
            continue;
        };
        if item.cost_price <= 0.0 {
            log::warn!("[T-03] code={} 成本价非法, 跳过", item.code);
            continue;
        }
        let pnl_pct = (quote.price / item.cost_price - 1.0) * 100.0;
        let intent = if pnl_pct > 5.0 {
            push_templates::Intent::Reduce
        } else if pnl_pct < -3.0 {
            push_templates::Intent::Add
        } else {
            push_templates::Intent::Hold
        };
        let reason = match intent {
            push_templates::Intent::Reduce => {
                format!("浮盈 {pnl_pct:.1}% 触发减仓观察 (>+5%)")
            }
            push_templates::Intent::Add => format!("浮亏 {pnl_pct:.1}% 触发加仓观察 (<-3%)"),
            push_templates::Intent::Hold => format!("浮盈 {pnl_pct:.1}%, 持有观望区间"),
            _ => unreachable!("T-03 只产出 Reduce/Add/Hold"),
        };
        let reasons = vec![reason];
        let text = push_templates::render_holding_plan(
            banner,
            push_templates::HoldingPlanParams {
                name: &item.name,
                code: &item.code,
                hhmm: &hhmm,
                intent,
                price: quote.price,
                cost: item.cost_price,
                avail: u32::try_from(item.quantity).unwrap_or(u32::MAX),
                reduce_zone: Some((item.cost_price * 1.02, item.cost_price * 1.05)),
                support: item.cost_price * 0.95,
                pressure: item.cost_price * 1.10,
                stop: item.cost_price * 0.92,
                invalidations: &[],
                reasons: &reasons,
            },
        );
        let canonical = serde_json::json!({
            "schema_version": "HOLDING_PLAN_SOURCE_BINDING_V1",
            "code": item.code,
            "name": item.name,
            "intent": intent.label(),
            "price": quote.price,
            "cost": item.cost_price,
            "quantity": item.quantity,
            "pnl_pct": pnl_pct,
            "observed_at": observed_at.to_rfc3339(),
            "snapshot": {
                "snapshot_row_id": snapshot.snapshot_row_id,
                "snapshot_id": snapshot.snapshot_id,
                "effective_at": snapshot.effective_at.to_rfc3339(),
                "confirmed_at": snapshot.confirmed_at.to_rfc3339(),
                "source": snapshot.source,
                "confirm_empty": snapshot.confirm_empty,
                "evidence_sha256": snapshot.evidence_sha256,
            },
            "quote_batch": {
                "provider": quote_batch.evidence.provider,
                "source": quote_batch.evidence.source,
                "source_at": quote_batch.evidence.source_at,
                "observed_at": quote_batch.evidence.observed_at,
                "batch_id": quote_batch.evidence.batch_id,
            },
            "requested_codes": requested_codes,
        });
        let canonical_bytes = canonical.to_string().into_bytes();
        let subject_hash = hex::encode(Sha256::digest(&canonical_bytes));
        let exchange = if item.code.starts_with('6') {
            Exchange::Shanghai
        } else {
            Exchange::Shenzhen
        };
        let instrument = InstrumentId::new(exchange, item.code.clone(), AssetClass::Equity)
            .map_err(|error| format!("instrument 构造失败 code={}: {error}", item.code))?;
        let binding = durable_delivery_runtime::CountedDeliveryBinding::new(
            business_date,
            format!("holding-plan:{business_date}:{}", item.code),
            canonical_bytes,
            durable_delivery_runtime::CountedDeliveryScope::Ticket { instrument },
            subject_hash,
            durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
            None,
            true,
        )
        .map_err(|error| format!("counted binding 构造失败 code={}: {error}", item.code))?;
        out.push(PreparedHoldingPlan {
            code: item.code.clone(),
            text,
            binding,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use stock_analysis::data_gateway::BatchEvidence;
    use stock_analysis::market_data::TopStock;
    use stock_analysis::market_domain::ProviderId;
    use stock_analysis::portfolio::user_position_snapshot::UserPositionItemInput;

    use super::*;

    fn position(code: &str, name: &str, quantity: u64, cost_price: f64) -> UserPositionItemInput {
        UserPositionItemInput {
            code: code.to_owned(),
            name: name.to_owned(),
            quantity,
            cost_price,
        }
    }

    fn snapshot_with(snapshot_id: &str, items: Vec<UserPositionItemInput>) -> UserPositionSnapshot {
        UserPositionSnapshot {
            snapshot_row_id: 41,
            snapshot_id: snapshot_id.to_owned(),
            effective_at: chrono::DateTime::parse_from_rfc3339("2026-09-09T15:00:00+08:00")
                .expect("fixture effective_at"),
            confirmed_at: chrono::DateTime::parse_from_rfc3339("2026-09-09T15:05:00+08:00")
                .expect("fixture confirmed_at"),
            source: "TEST_CODE_USER_CONFIRMED".to_owned(),
            confirm_empty: false,
            evidence_sha256: "a".repeat(64),
            items,
        }
    }

    fn snapshot() -> UserPositionSnapshot {
        snapshot_with(
            "TEST_CODE_SNAPSHOT_A",
            vec![position("600000", "浦发银行", 300, 8.0)],
        )
    }

    fn quote(code: &str, name: &str, price: f64) -> TopStock {
        TopStock {
            code: code.to_owned(),
            name: name.to_owned(),
            price,
            change_pct: 1.0,
            volume_ratio: None,
            main_net_yi: None,
        }
    }

    fn quote_batch_with(
        stocks: Vec<TopStock>,
        batch_id: &str,
        source_at: Option<&str>,
    ) -> market_data::TopStockBatch {
        market_data::TopStockBatch {
            stocks,
            evidence: BatchEvidence {
                provider: ProviderId::Tencent,
                source: "TEST_CODE_QUOTE_SOURCE".to_owned(),
                source_at: source_at.map(str::to_owned),
                observed_at: "2026-09-10T09:30:01+08:00".to_owned(),
                batch_id: batch_id.to_owned(),
            },
        }
    }

    fn quote_batch() -> market_data::TopStockBatch {
        quote_batch_with(
            vec![quote("600000", "浦发银行", 8.5)],
            "TEST_CODE_QUOTE_BATCH_A",
            Some("2026-09-10T09:29:58+08:00"),
        )
    }

    fn now() -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339("2026-09-10T09:30:02+08:00")
            .expect("fixture observed_at")
    }

    fn canonical(prepared: &PreparedHoldingPlan) -> serde_json::Value {
        serde_json::from_slice(prepared.binding.source_binding_canonical())
            .expect("canonical source binding")
    }

    #[test]
    fn holding_plan_source_binding_retains_snapshot_and_quote_batch_provenance() {
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot())),
            |_| Ok(quote_batch()),
            now,
        )
        .expect("prepare holding plan");

        let canonical = canonical(&prepared[0]);
        assert_eq!(
            canonical["schema_version"],
            "HOLDING_PLAN_SOURCE_BINDING_V1"
        );
        assert_eq!(canonical["code"], "600000");
        assert_eq!(canonical["name"], "浦发银行");
        assert_eq!(canonical["intent"], "逢高减仓");
        assert_eq!(canonical["price"], 8.5);
        assert_eq!(canonical["cost"], 8.0);
        assert_eq!(canonical["quantity"], 300);
        assert_eq!(canonical["pnl_pct"], 6.25);
        assert_eq!(canonical["observed_at"], "2026-09-10T09:30:02+08:00");
        assert_eq!(canonical["snapshot"]["snapshot_row_id"], 41);
        assert_eq!(canonical["snapshot"]["snapshot_id"], "TEST_CODE_SNAPSHOT_A");
        assert_eq!(
            canonical["snapshot"]["effective_at"],
            "2026-09-09T15:00:00+08:00"
        );
        assert_eq!(
            canonical["snapshot"]["confirmed_at"],
            "2026-09-09T15:05:00+08:00"
        );
        assert_eq!(canonical["snapshot"]["source"], "TEST_CODE_USER_CONFIRMED");
        assert_eq!(canonical["snapshot"]["confirm_empty"], false);
        assert_eq!(canonical["snapshot"]["evidence_sha256"], "a".repeat(64));
        assert_eq!(canonical["quote_batch"]["provider"], "Tencent");
        assert_eq!(canonical["quote_batch"]["source"], "TEST_CODE_QUOTE_SOURCE");
        assert_eq!(
            canonical["quote_batch"]["source_at"],
            "2026-09-10T09:29:58+08:00"
        );
        assert_eq!(
            canonical["quote_batch"]["observed_at"],
            "2026-09-10T09:30:01+08:00"
        );
        assert_eq!(
            canonical["quote_batch"]["batch_id"],
            "TEST_CODE_QUOTE_BATCH_A"
        );
        assert_eq!(canonical["requested_codes"], serde_json::json!(["600000"]));
    }

    #[test]
    fn missing_or_failed_snapshot_never_requests_quotes() {
        for (snapshot_result, expected_error) in [
            (Ok(None), "无用户确认持仓快照 (BR-226)"),
            (
                Err("TEST_CODE_DB_DOWN".to_owned()),
                "持仓快照读取失败: TEST_CODE_DB_DOWN",
            ),
        ] {
            let quote_called = Cell::new(false);
            let error = prepare_holding_plan_messages_with(
                &push_templates::BannerCtx::test_default(),
                || snapshot_result,
                |_| {
                    quote_called.set(true);
                    Ok(quote_batch())
                },
                now,
            )
            .err()
            .expect("snapshot failure must stop preparation");
            assert_eq!(error, expected_error);
            assert!(!quote_called.get());
        }
    }

    #[test]
    fn confirmed_empty_or_itemless_snapshot_is_silent_without_quotes() {
        let mut confirmed_empty = snapshot();
        confirmed_empty.confirm_empty = true;
        let itemless = snapshot_with("TEST_CODE_ITEMLESS", Vec::new());

        for snapshot in [confirmed_empty, itemless] {
            let quote_called = Cell::new(false);
            let prepared = prepare_holding_plan_messages_with(
                &push_templates::BannerCtx::test_default(),
                || Ok(Some(snapshot)),
                |_| {
                    quote_called.set(true);
                    Ok(quote_batch())
                },
                now,
            )
            .expect("empty snapshot is a successful no-op");
            assert!(prepared.is_empty());
            assert!(!quote_called.get());
        }
    }

    #[test]
    fn quote_batch_failure_is_propagated() {
        let error = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot())),
            |_| Err("TEST_CODE_QUOTES_DOWN".to_owned()),
            now,
        )
        .err()
        .expect("quote failure must stop preparation");

        assert_eq!(error, "持仓行情批次拒绝: TEST_CODE_QUOTES_DOWN");
    }

    #[test]
    fn partial_quote_coverage_prepares_only_covered_positions() {
        let snapshot = snapshot_with(
            "TEST_CODE_PARTIAL",
            vec![
                position("600000", "浦发银行", 300, 8.0),
                position("000001", "平安银行", 200, 10.0),
            ],
        );
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot)),
            |_| Ok(quote_batch()),
            now,
        )
        .expect("partial batch remains usable");

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].code, "600000");
        assert!(prepared[0].text.contains("浦发银行(600000)"));
    }

    #[test]
    fn nonpositive_cost_is_skipped_without_hiding_other_positions() {
        let snapshot = snapshot_with(
            "TEST_CODE_INVALID_COST",
            vec![
                position("600000", "非法成本", 300, 0.0),
                position("000001", "有效持仓", 200, 10.0),
            ],
        );
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot)),
            |_| {
                Ok(quote_batch_with(
                    vec![
                        quote("600000", "非法成本", 8.5),
                        quote("000001", "有效持仓", 10.0),
                    ],
                    "TEST_CODE_INVALID_COST_BATCH",
                    Some("2026-09-10T09:29:58+08:00"),
                ))
            },
            now,
        )
        .expect("invalid item is isolated");

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].code, "000001");
    }

    #[test]
    fn reduce_and_add_positions_use_snapshot_cost_quantity_and_batch_price() {
        let snapshot = snapshot_with(
            "TEST_CODE_REDUCE_ADD",
            vec![
                position("600000", "减仓票", 300, 8.0),
                position("000001", "加仓票", 200, 10.0),
            ],
        );
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot)),
            |_| {
                Ok(quote_batch_with(
                    vec![
                        quote("600000", "行情名称不绑定", 8.5),
                        quote("000001", "行情名称不绑定", 9.5),
                    ],
                    "TEST_CODE_REDUCE_ADD_BATCH",
                    Some("2026-09-10T09:29:58+08:00"),
                ))
            },
            now,
        )
        .expect("prepare reduce and add proposals");

        assert_eq!(prepared.len(), 2);
        assert!(prepared[0]
            .text
            .contains("持仓建议 减仓票(600000)（09:30）"));
        assert!(prepared[0]
            .text
            .contains("动作倾向: 逢高减仓 | 现价8.50 成本8.00 可用300股"));
        assert!(prepared[1]
            .text
            .contains("持仓建议 加仓票(000001)（09:30）"));
        assert!(prepared[1]
            .text
            .contains("动作倾向: 加仓 | 现价9.50 成本10.00 可用200股"));
        assert_eq!(canonical(&prepared[0])["intent"], "逢高减仓");
        assert_eq!(canonical(&prepared[1])["intent"], "加仓");
    }

    #[test]
    fn hold_position_keeps_the_existing_fixed_example() {
        let snapshot = snapshot_with(
            "TEST_CODE_HOLD",
            vec![position("600000", "持有票", u64::from(u32::MAX) + 1, 10.0)],
        );
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot)),
            |_| {
                Ok(quote_batch_with(
                    vec![quote("600000", "行情名称不绑定", 10.0)],
                    "TEST_CODE_HOLD_BATCH",
                    Some("2026-09-10T09:29:58+08:00"),
                ))
            },
            now,
        )
        .expect("prepare hold proposal");

        assert_eq!(canonical(&prepared[0])["intent"], "持有观望");
        assert!(prepared[0].text.contains("动作倾向: 持有观望"));
        assert!(prepared[0]
            .text
            .contains("现价10.00 成本10.00 可用4294967295股"));
        assert!(prepared[0]
            .text
            .contains("支撑9.50 | 压力11.00 | 硬止损9.20"));
    }

    #[test]
    fn one_captured_local_time_drives_every_proposal_date_text_and_binding() {
        let clock_calls = Cell::new(0);
        let snapshot = snapshot_with(
            "TEST_CODE_ONE_CLOCK",
            vec![
                position("600000", "甲", 300, 8.0),
                position("000001", "乙", 200, 10.0),
            ],
        );
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot)),
            |_| {
                Ok(quote_batch_with(
                    vec![quote("600000", "甲", 8.5), quote("000001", "乙", 10.0)],
                    "TEST_CODE_ONE_CLOCK_BATCH",
                    Some("2026-09-10T09:29:58+08:00"),
                ))
            },
            || {
                clock_calls.set(clock_calls.get() + 1);
                now()
            },
        )
        .expect("prepare one clock round");

        assert_eq!(clock_calls.get(), 1);
        assert_eq!(prepared.len(), 2);
        for message in prepared {
            assert_eq!(message.binding.business_date().to_string(), "2026-09-10");
            assert!(message.text.contains("（09:30）"));
            assert_eq!(
                canonical(&message)["observed_at"],
                "2026-09-10T09:30:02+08:00"
            );
        }
    }

    #[test]
    fn absent_quote_source_at_remains_null() {
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot())),
            |_| {
                Ok(quote_batch_with(
                    vec![quote("600000", "浦发银行", 8.5)],
                    "TEST_CODE_NO_SOURCE_AT",
                    None,
                ))
            },
            now,
        )
        .expect("prepare quote without provider source time");

        let canonical = canonical(&prepared[0]);
        let quote_batch = canonical["quote_batch"]
            .as_object()
            .expect("quote_batch object");
        assert!(quote_batch.contains_key("source_at"));
        assert!(quote_batch["source_at"].is_null());
    }

    #[test]
    fn snapshot_and_quote_batch_identity_changes_are_visible_without_changing_text() {
        let prepare = |snapshot_id: &str, batch_id: &str| {
            prepare_holding_plan_messages_with(
                &push_templates::BannerCtx::test_default(),
                || {
                    Ok(Some(snapshot_with(
                        snapshot_id,
                        vec![position("600000", "浦发银行", 300, 8.0)],
                    )))
                },
                |_| {
                    Ok(quote_batch_with(
                        vec![quote("600000", "浦发银行", 8.5)],
                        batch_id,
                        Some("2026-09-10T09:29:58+08:00"),
                    ))
                },
                now,
            )
            .expect("prepare identity variant")
            .remove(0)
        };
        let base = prepare("TEST_CODE_SNAPSHOT_A", "TEST_CODE_QUOTE_BATCH_A");
        let changed_snapshot = prepare("TEST_CODE_SNAPSHOT_B", "TEST_CODE_QUOTE_BATCH_A");
        let changed_quote = prepare("TEST_CODE_SNAPSHOT_A", "TEST_CODE_QUOTE_BATCH_B");

        assert_eq!(base.text, changed_snapshot.text);
        assert_eq!(base.text, changed_quote.text);
        assert_ne!(
            base.binding.source_binding_canonical(),
            changed_snapshot.binding.source_binding_canonical()
        );
        assert_ne!(
            base.binding.source_evidence_fingerprint(),
            changed_snapshot.binding.source_evidence_fingerprint()
        );
        assert_ne!(
            base.binding.source_binding_canonical(),
            changed_quote.binding.source_binding_canonical()
        );
        assert_ne!(
            base.binding.source_evidence_fingerprint(),
            changed_quote.binding.source_evidence_fingerprint()
        );
    }

    #[test]
    fn snapshot_selected_before_external_latest_changes_remains_the_join_source() {
        let latest = RefCell::new(snapshot_with(
            "TEST_CODE_SNAPSHOT_A",
            vec![position("600000", "快照A名称", 300, 8.0)],
        ));
        let requested = RefCell::new(Vec::<String>::new());
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || {
                let selected = latest.borrow().clone();
                *latest.borrow_mut() = snapshot_with(
                    "TEST_CODE_SNAPSHOT_B",
                    vec![position("000001", "快照B名称", 200, 10.0)],
                );
                Ok(Some(selected))
            },
            |codes| {
                *requested.borrow_mut() = codes.to_vec();
                Ok(quote_batch_with(
                    vec![quote("600000", "行情名称不绑定", 8.5)],
                    "TEST_CODE_SNAPSHOT_RACE_BATCH",
                    Some("2026-09-10T09:29:58+08:00"),
                ))
            },
            now,
        )
        .expect("prepare selected snapshot");

        assert_eq!(requested.borrow().clone(), vec!["600000".to_owned()]);
        assert_eq!(prepared.len(), 1);
        assert!(prepared[0].text.contains("快照A名称(600000)"));
        let canonical = canonical(&prepared[0]);
        assert_eq!(canonical["snapshot"]["snapshot_id"], "TEST_CODE_SNAPSHOT_A");
        assert_eq!(canonical["requested_codes"], serde_json::json!(["600000"]));
        assert_eq!(canonical["code"], "600000");
    }
}
