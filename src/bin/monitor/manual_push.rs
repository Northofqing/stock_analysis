use stock_analysis::opportunity::scheduler::PushWindow;

pub(super) struct ManualPushContext<'a> {
    pub(super) window: PushWindow,
    pub(super) date: &'a str,
    pub(super) hhmm: &'a str,
}

#[async_trait::async_trait]
pub(super) trait ManualPushEffects {
    type Banner: Send + Sync;

    async fn refresh_banner(&mut self) -> Result<(), String>;
    fn read_banner(&mut self) -> Result<Self::Banner, String>;

    async fn dispatch_intraday_market(&mut self, hhmm: &str, banner: &Self::Banner) -> bool;
    async fn dispatch_news_catalyst(&mut self, hhmm: &str, banner: &Self::Banner) -> bool;
    async fn dispatch_industry_chain(&mut self, hhmm: &str, banner: &Self::Banner) -> bool;
    async fn dispatch_news_to_idea(&mut self, hhmm: &str, banner: &Self::Banner) -> bool;
    async fn dispatch_holding_plan(&mut self, banner: &Self::Banner) -> Vec<String>;
    async fn dispatch_paper_review(&mut self, date: &str) -> bool;
    async fn dispatch_catalyst_review(&mut self, date: &str) -> bool;
}

fn report_dispatch_outcome(name: &str, delivered: bool, failures: &mut Vec<String>) {
    if delivered {
        log::info!("[v22] {} dispatcher completed", name);
    } else {
        log::warn!(
            "[v22] {} dispatcher did not confirm delivery, continue to next dispatcher",
            name
        );
        failures.push(format!("{name} did not confirm delivery"));
    }
}

pub(super) async fn run_manual_push<E: ManualPushEffects>(
    context: ManualPushContext<'_>,
    effects: &mut E,
) -> Result<(), String> {
    let mut failures = Vec::new();

    match context.window {
        PushWindow::Preopen => {
            failures.push(
                "P-01 is owned by the BR-241 resident scheduler or the exclusive --compensate=P-01 command"
                    .to_owned(),
            );
        }
        PushWindow::Intraday => {
            effects
                .refresh_banner()
                .await
                .map_err(|error| format!("BR-108 --push health refresh failed: {error}"))?;
            let banner = effects.read_banner().map_err(|error| {
                format!("BR-108 --push banner unavailable after health refresh: {error}")
            })?;

            report_dispatch_outcome(
                "I-01",
                effects
                    .dispatch_intraday_market(context.hhmm, &banner)
                    .await,
                &mut failures,
            );
            report_dispatch_outcome(
                "I-02",
                effects.dispatch_news_catalyst(context.hhmm, &banner).await,
                &mut failures,
            );
            report_dispatch_outcome(
                "I-03",
                effects.dispatch_industry_chain(context.hhmm, &banner).await,
                &mut failures,
            );
            report_dispatch_outcome(
                "D-01",
                effects.dispatch_news_to_idea(context.hhmm, &banner).await,
                &mut failures,
            );
            let holding_failures = effects.dispatch_holding_plan(&banner).await;
            if holding_failures.is_empty() {
                report_dispatch_outcome("I-04", true, &mut failures);
            } else {
                failures.extend(holding_failures);
            }
        }
        PushWindow::Evening | PushWindow::Outside => {
            match effects.refresh_banner().await {
                Ok(()) => match effects.read_banner() {
                    Ok(_) => report_dispatch_outcome(
                        "A-01",
                        effects.dispatch_paper_review(context.date).await,
                        &mut failures,
                    ),
                    Err(error) => failures.push(format!(
                        "A-01 banner unavailable after health refresh: {error}"
                    )),
                },
                Err(error) => failures.push(format!("A-01 health refresh failed: {error}")),
            }
            report_dispatch_outcome(
                "A-10",
                effects.dispatch_catalyst_review(context.date).await,
                &mut failures,
            );
        }
    }

    if failures.is_empty() {
        log::info!("[v22] --push 完成 (HHMM: {})", context.hhmm);
        Ok(())
    } else {
        log::error!(
            "[v22] --push 已完成但部分调度未确认: {} / {}, fails={:?}",
            failures.len(),
            context.hhmm,
            failures
        );
        Err(format!(
            "{} manual push failure(s): {}",
            failures.len(),
            failures.join("; ")
        ))
    }
}

pub(super) struct RealManualPushEffects;

#[async_trait::async_trait]
impl ManualPushEffects for RealManualPushEffects {
    type Banner = crate::push_templates::BannerCtx;

    async fn refresh_banner(&mut self) -> Result<(), String> {
        crate::refresh_banner_state().await
    }

    fn read_banner(&mut self) -> Result<Self::Banner, String> {
        crate::current_banner()
    }

    async fn dispatch_intraday_market(&mut self, hhmm: &str, banner: &Self::Banner) -> bool {
        crate::push_templates::dispatch_intraday_market_daily(hhmm, banner).await
    }

    async fn dispatch_news_catalyst(&mut self, hhmm: &str, banner: &Self::Banner) -> bool {
        crate::push_templates::dispatch_news_catalyst_daily(hhmm, banner).await
    }

    async fn dispatch_industry_chain(&mut self, hhmm: &str, banner: &Self::Banner) -> bool {
        crate::push_templates::dispatch_industry_chain_intraday_daily(hhmm, banner).await
    }

    async fn dispatch_news_to_idea(&mut self, hhmm: &str, banner: &Self::Banner) -> bool {
        crate::push_templates::dispatch_news_to_idea_daily(hhmm, banner).await
    }

    async fn dispatch_holding_plan(&mut self, banner: &Self::Banner) -> Vec<String> {
        let messages = match crate::prepare_holding_plan_messages(banner).await {
            Ok(messages) => messages,
            Err(error) => return vec![format!("I-04 T-03 batch rejected: {error}")],
        };

        let mut failures = Vec::new();
        for prepared in messages {
            let token = match crate::presentation_registry::acquire_token(
                "T-03-holding-plan",
                crate::PushKind::HoldingPlan,
                "holding_plan_dispatcher",
                "render_holding_plan",
            ) {
                Ok(token) => token,
                Err(reason) => {
                    failures.push(format!(
                        "I-04 T-03 token rejected code={}: {reason}",
                        prepared.code
                    ));
                    continue;
                }
            };
            let outcome = crate::notify::push_counted_with_binding(
                token,
                &prepared.text,
                None,
                prepared.binding,
            )
            .await;
            if !matches!(
                outcome,
                crate::notify::PushOutcome::Pushed | crate::notify::PushOutcome::Deduped
            ) {
                failures.push(format!(
                    "I-04 T-03 delivery unconfirmed code={}: {:?}",
                    prepared.code, outcome
                ));
            }
        }
        failures
    }

    async fn dispatch_paper_review(&mut self, date: &str) -> bool {
        crate::push_templates::dispatch_paper_review_daily(date).await
    }

    async fn dispatch_catalyst_review(&mut self, date: &str) -> bool {
        crate::push_templates::dispatch_catalyst_review_daily(date).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum Event {
        RefreshBanner,
        ReadBanner,
        Intraday(&'static str, String, String),
        HoldingPlan(String),
        PaperReview(String),
        CatalystReview(String),
    }

    struct MemoryEffects {
        banner: Option<&'static str>,
        refreshed_banner: Option<&'static str>,
        refresh_error: Option<&'static str>,
        unconfirmed: Vec<&'static str>,
        holding_failures: Vec<String>,
        events: Vec<Event>,
    }

    #[async_trait::async_trait]
    impl ManualPushEffects for MemoryEffects {
        type Banner = &'static str;

        async fn refresh_banner(&mut self) -> Result<(), String> {
            self.events.push(Event::RefreshBanner);
            if let Some(error) = self.refresh_error {
                return Err(error.to_owned());
            }
            self.banner = self.refreshed_banner;
            Ok(())
        }

        fn read_banner(&mut self) -> Result<Self::Banner, String> {
            self.events.push(Event::ReadBanner);
            self.banner
                .ok_or_else(|| "evaluated banner unavailable".to_owned())
        }

        async fn dispatch_intraday_market(&mut self, hhmm: &str, banner: &Self::Banner) -> bool {
            self.events.push(Event::Intraday(
                "I-01",
                hhmm.to_owned(),
                (*banner).to_owned(),
            ));
            !self.unconfirmed.contains(&"I-01")
        }

        async fn dispatch_news_catalyst(&mut self, hhmm: &str, banner: &Self::Banner) -> bool {
            self.events.push(Event::Intraday(
                "I-02",
                hhmm.to_owned(),
                (*banner).to_owned(),
            ));
            !self.unconfirmed.contains(&"I-02")
        }

        async fn dispatch_industry_chain(&mut self, hhmm: &str, banner: &Self::Banner) -> bool {
            self.events.push(Event::Intraday(
                "I-03",
                hhmm.to_owned(),
                (*banner).to_owned(),
            ));
            !self.unconfirmed.contains(&"I-03")
        }

        async fn dispatch_news_to_idea(&mut self, hhmm: &str, banner: &Self::Banner) -> bool {
            self.events.push(Event::Intraday(
                "D-01",
                hhmm.to_owned(),
                (*banner).to_owned(),
            ));
            !self.unconfirmed.contains(&"D-01")
        }

        async fn dispatch_holding_plan(&mut self, banner: &Self::Banner) -> Vec<String> {
            self.events.push(Event::HoldingPlan((*banner).to_owned()));
            std::mem::take(&mut self.holding_failures)
        }

        async fn dispatch_paper_review(&mut self, date: &str) -> bool {
            self.events.push(Event::PaperReview(date.to_owned()));
            !self.unconfirmed.contains(&"A-01")
        }

        async fn dispatch_catalyst_review(&mut self, date: &str) -> bool {
            self.events.push(Event::CatalystReview(date.to_owned()));
            !self.unconfirmed.contains(&"A-10")
        }
    }

    #[tokio::test]
    async fn existing_banner_intraday_batch_refreshes_before_preserving_order_and_context() {
        let mut effects = MemoryEffects {
            banner: Some("existing-conservative-banner"),
            refreshed_banner: Some("refreshed-conservative-banner"),
            refresh_error: None,
            unconfirmed: Vec::new(),
            holding_failures: Vec::new(),
            events: Vec::new(),
        };

        let result = run_manual_push(
            ManualPushContext {
                window: PushWindow::Intraday,
                date: "2026-09-10",
                hhmm: "14:30",
            },
            &mut effects,
        )
        .await;

        assert_eq!(result, Ok(()));
        assert_eq!(
            effects.events,
            vec![
                Event::RefreshBanner,
                Event::ReadBanner,
                Event::Intraday(
                    "I-01",
                    "14:30".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::Intraday(
                    "I-02",
                    "14:30".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::Intraday(
                    "I-03",
                    "14:30".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::Intraday(
                    "D-01",
                    "14:30".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::HoldingPlan("refreshed-conservative-banner".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn intraday_refreshes_health_before_dispatching_with_the_refreshed_banner() {
        let mut effects = MemoryEffects {
            banner: None,
            refreshed_banner: Some("refreshed-conservative-banner"),
            refresh_error: None,
            unconfirmed: Vec::new(),
            holding_failures: Vec::new(),
            events: Vec::new(),
        };

        let result = run_manual_push(
            ManualPushContext {
                window: PushWindow::Intraday,
                date: "2026-09-10",
                hhmm: "10:30",
            },
            &mut effects,
        )
        .await;

        assert_eq!(result, Ok(()));
        assert_eq!(
            effects.events,
            vec![
                Event::RefreshBanner,
                Event::ReadBanner,
                Event::Intraday(
                    "I-01",
                    "10:30".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::Intraday(
                    "I-02",
                    "10:30".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::Intraday(
                    "I-03",
                    "10:30".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::Intraday(
                    "D-01",
                    "10:30".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::HoldingPlan("refreshed-conservative-banner".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn evening_health_failure_skips_a01_and_still_dispatches_a10() {
        let mut effects = MemoryEffects {
            banner: Some("stale-banner-must-not-be-used"),
            refreshed_banner: None,
            refresh_error: Some("health source unavailable"),
            unconfirmed: Vec::new(),
            holding_failures: Vec::new(),
            events: Vec::new(),
        };

        let error = run_manual_push(
            ManualPushContext {
                window: PushWindow::Evening,
                date: "2026-09-10",
                hhmm: "19:00",
            },
            &mut effects,
        )
        .await
        .expect_err("A-01 health failure must reject the batch after A-10 continues");

        assert_eq!(
            error,
            "1 manual push failure(s): A-01 health refresh failed: health source unavailable"
        );

        assert_eq!(
            effects.events,
            vec![
                Event::RefreshBanner,
                Event::CatalystReview("2026-09-10".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn partial_intraday_failure_continues_remaining_dispatchers_and_returns_details() {
        let mut effects = MemoryEffects {
            banner: None,
            refreshed_banner: Some("refreshed-conservative-banner"),
            refresh_error: None,
            unconfirmed: vec!["I-02"],
            holding_failures: Vec::new(),
            events: Vec::new(),
        };

        let error = run_manual_push(
            ManualPushContext {
                window: PushWindow::Intraday,
                date: "2026-09-10",
                hhmm: "11:00",
            },
            &mut effects,
        )
        .await
        .expect_err("one unconfirmed dispatcher must reject the manual batch");

        assert_eq!(
            error,
            "1 manual push failure(s): I-02 did not confirm delivery"
        );
        assert_eq!(
            effects.events,
            vec![
                Event::RefreshBanner,
                Event::ReadBanner,
                Event::Intraday(
                    "I-01",
                    "11:00".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::Intraday(
                    "I-02",
                    "11:00".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::Intraday(
                    "I-03",
                    "11:00".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::Intraday(
                    "D-01",
                    "11:00".to_owned(),
                    "refreshed-conservative-banner".to_owned(),
                ),
                Event::HoldingPlan("refreshed-conservative-banner".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn multiple_intraday_failures_are_returned_in_dispatch_order() {
        let mut effects = MemoryEffects {
            banner: None,
            refreshed_banner: Some("refreshed-conservative-banner"),
            refresh_error: None,
            unconfirmed: vec!["I-01", "I-03", "D-01"],
            holding_failures: Vec::new(),
            events: Vec::new(),
        };

        let error = run_manual_push(
            ManualPushContext {
                window: PushWindow::Intraday,
                date: "2026-09-10",
                hhmm: "14:30",
            },
            &mut effects,
        )
        .await
        .expect_err("all unconfirmed dispatcher details must reject the batch");

        assert_eq!(
            error,
            "3 manual push failure(s): I-01 did not confirm delivery; I-03 did not confirm delivery; D-01 did not confirm delivery"
        );
    }

    #[tokio::test]
    async fn holding_plan_failure_details_are_preserved() {
        let mut effects = MemoryEffects {
            banner: None,
            refreshed_banner: Some("refreshed-conservative-banner"),
            refresh_error: None,
            unconfirmed: Vec::new(),
            holding_failures: vec![
                "I-04 T-03 delivery unconfirmed code=600000: Rejected".to_owned(),
                "I-04 T-03 token rejected code=000001: missing token".to_owned(),
            ],
            events: Vec::new(),
        };

        let error = run_manual_push(
            ManualPushContext {
                window: PushWindow::Intraday,
                date: "2026-09-10",
                hhmm: "10:30",
            },
            &mut effects,
        )
        .await
        .expect_err("holding-plan item failures must reject the batch");

        assert_eq!(
            error,
            "2 manual push failure(s): I-04 T-03 delivery unconfirmed code=600000: Rejected; I-04 T-03 token rejected code=000001: missing token"
        );
    }

    #[tokio::test]
    async fn evening_and_outside_require_health_before_routing_a01_and_a10_with_one_date() {
        for window in [PushWindow::Evening, PushWindow::Outside] {
            let mut effects = MemoryEffects {
                banner: Some("stale-banner-must-be-replaced"),
                refreshed_banner: Some("refreshed-conservative-banner"),
                refresh_error: None,
                unconfirmed: Vec::new(),
                holding_failures: Vec::new(),
                events: Vec::new(),
            };

            let result = run_manual_push(
                ManualPushContext {
                    window,
                    date: "2026-09-10",
                    hhmm: "19:00",
                },
                &mut effects,
            )
            .await;

            assert_eq!(result, Ok(()), "window={window:?}");
            assert_eq!(
                effects.events,
                vec![
                    Event::RefreshBanner,
                    Event::ReadBanner,
                    Event::PaperReview("2026-09-10".to_owned()),
                    Event::CatalystReview("2026-09-10".to_owned()),
                ],
                "window={window:?}"
            );
        }
    }

    #[tokio::test]
    async fn post_market_health_failures_reject_only_a01_and_preserve_a10() {
        for window in [PushWindow::Evening, PushWindow::Outside] {
            for (refresh_error, refreshed_banner, expected_detail, expects_read) in [
                (
                    Some("health source unavailable"),
                    Some("unused"),
                    "A-01 health refresh failed: health source unavailable",
                    false,
                ),
                (
                    None,
                    None,
                    "A-01 banner unavailable after health refresh: evaluated banner unavailable",
                    true,
                ),
            ] {
                let mut effects = MemoryEffects {
                    banner: Some("stale-banner-must-not-be-used"),
                    refreshed_banner,
                    refresh_error,
                    unconfirmed: Vec::new(),
                    holding_failures: Vec::new(),
                    events: Vec::new(),
                };

                let error = run_manual_push(
                    ManualPushContext {
                        window,
                        date: "2026-09-10",
                        hhmm: "20:15",
                    },
                    &mut effects,
                )
                .await
                .expect_err("A-01 health preparation failure must reject the batch");

                assert_eq!(
                    error,
                    format!("1 manual push failure(s): {expected_detail}")
                );
                let expected_events = if expects_read {
                    vec![
                        Event::RefreshBanner,
                        Event::ReadBanner,
                        Event::CatalystReview("2026-09-10".to_owned()),
                    ]
                } else {
                    vec![
                        Event::RefreshBanner,
                        Event::CatalystReview("2026-09-10".to_owned()),
                    ]
                };
                assert_eq!(effects.events, expected_events, "window={window:?}");
            }
        }
    }

    #[tokio::test]
    async fn preopen_is_rejected_without_health_or_dispatch_effects() {
        let mut effects = MemoryEffects {
            banner: None,
            refreshed_banner: Some("must-not-be-read"),
            refresh_error: None,
            unconfirmed: Vec::new(),
            holding_failures: Vec::new(),
            events: Vec::new(),
        };

        let error = run_manual_push(
            ManualPushContext {
                window: PushWindow::Preopen,
                date: "2026-09-10",
                hhmm: "09:00",
            },
            &mut effects,
        )
        .await
        .expect_err("manual P-01 ownership must be rejected");

        assert!(error.contains("P-01 is owned by the BR-241 resident scheduler"));
        assert!(effects.events.is_empty());
    }

    #[tokio::test]
    async fn intraday_health_failures_do_not_reuse_a_stale_banner() {
        for (refresh_error, refreshed_banner, expected_error, expected_events) in [
            (
                Some("health source unavailable"),
                Some("unused"),
                "BR-108 --push health refresh failed: health source unavailable",
                vec![Event::RefreshBanner],
            ),
            (
                None,
                None,
                "BR-108 --push banner unavailable after health refresh: evaluated banner unavailable",
                vec![Event::RefreshBanner, Event::ReadBanner],
            ),
        ] {
            let mut effects = MemoryEffects {
                banner: Some("stale-banner-must-not-be-used"),
                refreshed_banner,
                refresh_error,
                unconfirmed: Vec::new(),
                holding_failures: Vec::new(),
                events: Vec::new(),
            };

            let error = run_manual_push(
                ManualPushContext {
                    window: PushWindow::Intraday,
                    date: "2026-09-10",
                    hhmm: "11:00",
                },
                &mut effects,
            )
            .await
            .expect_err("intraday requires a fresh evaluated banner");

            assert_eq!(error, expected_error);
            assert_eq!(effects.events, expected_events);
        }
    }
}
