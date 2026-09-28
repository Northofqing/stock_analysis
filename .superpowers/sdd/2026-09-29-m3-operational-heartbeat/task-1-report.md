# M3 operational heartbeat — Task 1 report

## Implemented

- Added a private version 1 process heartbeat record with only `version`, lease `boot_id`, and UTC `observed_at`. The production and test paths are `data/health/heartbeat.json` and `data/test/health/heartbeat.json` under an explicit root.
- Added an explicit-path writer with injected observation time, a mode-root wrapper for Task 2, and a bounded reader with injected current time for Task 3. Readers reject unknown fields, malformed or oversized JSON, invalid version or three-part boot ID, pre-epoch time, and observations more than five seconds ahead. Old observations remain readable so Task 3 can report staleness separately.
- Reused one private atomic replacement helper for banner and heartbeat files. It writes a private 0600 create-new temp file, syncs it, renames it, and syncs the parent directory on Unix. Errors before rename remove the temp file and leave the previous destination intact. A directory-sync error after rename is returned as a durability error; it makes no claim that the old destination survived.
- Added named 60-second interval and 10-minute maximum-age constants as process-liveness reporting policy. Task 1 does not start a scheduler or change health status.

## Verification and self-review

- `cargo test --locked --offline --bin monitor health_cmd::tests` — passed, 12 tests; 855 filtered out. Repeated after the final validation split as `cargo test --locked --offline --bin monitor health_cmd::tests 2>&1 | tail -n 20` — passed, 12 tests. Existing compiler warnings remain unrelated to this task.
- Review follow-up: added a genuinely truncated JSON payload to the heartbeat rejection test. Reran `cargo test --locked --offline --bin monitor health_cmd::tests` — passed, 12 tests; 855 filtered out. The same unrelated compiler warnings remain.
- `git diff --check` — passed after the final source edit.
- `rustfmt --edition 2021 src/bin/monitor/health_cmd.rs` — applied only to the changed file. Initial repository-wide `cargo fmt --check` showed pre-existing formatting differences in other files, so no other file was reformatted.
- Self-review: the existing banner schema, destination path, read-side reason semantics, and private file mode remain intact. The only banner write behavior added is payload-bound enforcement and parent-directory sync. Tests cover roundtrip, schema/identity/time/size rejection, root isolation, private mode, and a deterministic failure after temp-file sync but before rename, with prior bytes and temp cleanup checked. No production root, provider, webhook, deployment, or monitor loop was touched.

## Remaining gates

- Task 2: start the immediate and periodic best-effort writer only for a leased resident invocation, before awaited startup notification; supervise/abort its task and test that pending or failed evaluations and writer errors do not stop it.
- Task 3: join fresh heartbeat evidence to the live lease in read-only `--health`, retain existing banner reason precedence, add process-liveness fields and text/JSON parity, and test missing, malformed, stale, future, and wrong-boot cases.
- Operational evidence after a separately authorized rollout remains required before any Production Verified claim.
