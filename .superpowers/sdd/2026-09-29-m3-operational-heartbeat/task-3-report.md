# Task 3 report — read-only heartbeat evidence

## Change

- `monitor --health` now reads the bounded typed heartbeat file alongside the banner and live locked lease. The shared report exposes `heartbeat_fresh`, `heartbeat_status`, `heartbeat_reason_code`, `heartbeat_observed_at`, and `heartbeat_age_seconds` in text and JSON.
- A current heartbeat with the live lease boot ID is required for overall `status=ok`. Missing, malformed, stale, too-far-future, and wrong-boot records remain unhealthy with fixed heartbeat reasons. Freshness describes the timestamp only; `heartbeat_status` also checks the live lease identity.
- Banner, lease, account, data, and banner-mode primary reason precedence stays intact. Heartbeat fields are calculated even when a banner error wins. Coverage is explicitly limited to banner account/data evidence and process liveness.
- Tests cover the outcomes above, text/JSON parity, and real lease turnover. A new owner only becomes healthy once both banner and heartbeat identify its boot ID.

## Verification

- `cargo test --locked --offline --bin monitor health_cmd::tests` — 13 passed, 0 failed, 857 filtered out.
- `git diff --check` — passed.

Only the monitor bin's matching unit tests ran. No full suite, release build, production `--health` invocation, activation, provider, notification, or production root was used.

## Self-review and deployment gate

The CLI path remains filesystem-only and read-only. The existing bounded heartbeat reader validates before reporting, and the report only promotes heartbeat failure to the primary reason after all previous banner and lease checks. Heartbeat data does not change banner evaluation times or claim source readiness. The current production binary predates the heartbeat writer, so its missing heartbeat file will correctly make this new reader unhealthy until the same-version resident writer is deployed and observed.

This source slice is not Production Verified. After a separately authorized rollout, retain fresh read-only health samples across an agreed observation window and a monitor restart that changes the lease boot ID. Observe a natural or staging-injected evaluator/notification failure while heartbeat advances, exercise a controlled non-production write failure, and confirm test-root writes do not appear under production data. No faults should be injected into live production.
