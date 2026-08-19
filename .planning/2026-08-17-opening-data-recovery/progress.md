# Progress Log

## Session: 2026-08-17

### Current Status
- **Phase:** 4 - Compliance and release verification
- **Started:** 2026-08-17

### Actions Taken
- Ran bare-monitor diagnosis against the already-running production instance instead of starting a duplicate.
- Captured startup, 30-second valuation, 60-second selection, and 120-second news cycles.
- Located the private `client-bundle` and inventoried its contract without printing credentials.
- Compared the upstream client contract with the local extended proto and no-feature monitor topology.
- Initialized this isolated autonomous plan after finding the `.codex` helper link incomplete.
- Installed operator-only `grpcurl`; verified remote mTLS, health, capabilities, and live reflection without exposing credentials.
- Completed mandatory instruction pre-flight and the systematic-debugging root-cause phase.
- Selected the authenticated-upstream/local-normalization design under the user's standing authorization; no further business confirmation is required.
- Verified and removed the stale `.superpowers` planning conflict by reversible archive plus links to the active opening-recovery plan.
- Wrote and self-reviewed `docs/superpowers/specs/2026-08-17-client-bundle-opening-readiness-design.md` and registered BR-238.
- Ran the business-rule checker: BR-238 is registered; the duplicate BR-224/BR-225 meanings were reassigned to BR-232/BR-233 and the checker passes.
- Wrote `docs/superpowers/plans/2026-08-17-client-bundle-opening-readiness.md` for inline test-first execution.
- Started three isolated parallel workstreams after the user's explicit parallel-execution request: local evidence propagation, secure bundle loading, and read-only external contract freeze.
- Froze the ExternalV1 evidence boundary: the bundle proto stops at operation 55, a referenced derived-products document is missing, and only SecurityMetadata/InstrumentNews have sufficiently proven request contracts.
- Added a RED test requiring production QueryRequest construction to set `allow_unadmitted=false`, then changed the builder accordingly.
- Added a RED test for request-scoped Bearer injection without environment mutation, and implemented `attach_bearer_value`; its GREEN run is queued behind the shared Cargo artifact lock.
- The request-scoped Bearer test is GREEN (1/1). The production `allow_unadmitted=false` focused rerun is queued behind another shared Cargo build.
- Changed the design from whole-connection replacement to a dual-client, per-operation closed dispatch after proving the ExternalV1 bundle incomplete.
- Added RED ExternalV1 contract tests: SecurityMetadata/InstrumentNews must use delivered schemas; RealtimeQuotes/BoardConstituents/UpperLimitPoolReview must be rejected before I/O until their contracts are delivered.
- Re-numbered duplicate business rules mechanically with no logic change; `check_business_rules.sh` now passes (229 rules, 153 non-blocking existing path warnings).
- Continued live-log collection found a separate v26 report blocker: historical dispatcher JSONL contains an invalid escape produced by manual writer escaping. Registered the writer-side BR-132 requirement and added a RED arbitrary-error JSON round-trip test; no audit/log evidence was modified.
- Completed the closed ExternalV1 request adapter: SecurityMetadata and InstrumentNews tests are GREEN; undelivered quote/board/review contracts reject before I/O; duplicate/ambiguous instrument inputs now reject explicitly.
- Completed the transport/auth subtask: 8/8 no-feature tests pass for local/external profiles, mTLS bundle construction, instance Bearer auth, precise contract-error mapping, and acquisition-source preservation.
- Added response-model coverage for `diagnostic_blocker` and queued converter enforcement for admission/completeness/diagnostic rejection plus removal of `tdx-dev` compatibility evidence.
- Completed the secret-safe bundle loader (9/9), transport/auth profile tests (8/8), strict ExternalV1 request adapter, and local server evidence-preservation tests.
- Fixed the no-feature blocking board-membership bridge and wired `MarketCapabilitiesGateway::security_identities` to a separate authenticated ExternalV1 connection.
- Added an opening-readiness probe and ran the real mTLS canary: health live/ready, all 11 opening capabilities ADMITTED+runtime, SecurityMetadata partial identity schema verified, InstrumentNews complete schema verified, and the real identity converter accepted the evidence.
- Added a production startup gate: before monitor producer loops, a real read-only SecurityMetadata canary must pass transport, health, capability, contract and evidence admission.
- Fixed BR-132 dispatcher writes with Serde JSONL serialization without altering historical evidence; the one 2026-07-17 malformed row remains explicit and does not block `--review` or push loops.
- Fixed BR-139 review phase ordering and passed the focused BR-139/194/199 regression group (37 tests).
- Fixed ExternalV1 error mapping to preserve remote provider/reason/retryable evidence and reject undelivered contracts before I/O.
- Bound SecurityIdentity admission to the exact request set and fixed evidence clocks: observations stay within 30 seconds, source evidence within one trading day, and an authenticated remote may lead the local wall clock by at most two seconds without rewriting either timestamp.
- Reproduced the clock boundary RED at +1 second, then passed +1-second admission, +3-second rejection, and the full converter group (30/30).
- Expanded production opening readiness from one operation to all 11 opening-critical ADMITTED/runtime capabilities before the real identity canary.
- Re-ran the real bundle probe after the clock fix: health live/ready, all 11 capabilities ready, SecurityMetadata projection valid, InstrumentNews complete, `opening_bundle_ready=true`.
- `cargo check --lib`, `cargo check --bin monitor`, and the business-rule compliance gate pass on the integrated tree.
- Began exact Gate-C remediation: isolated historical rustfmt/clippy drift from task-introduced findings and split mechanical lint fixes into non-overlapping parallel groups.

### Test Results
| Test | Expected | Actual | Status |
|------|----------|--------|--------|
| Production process check | Exactly one monitor | PID 89949 only | PASS |
| Local gRPC connectivity | Established client/server socket | monitor ↔ PID 73473 on 127.0.0.1:18082 | PASS |
| Position-chain startup refresh | 7 records assigned or verified-empty | 0 assigned, 0 verified-empty, 7 failed | FAIL |
| Account freshness | <=30 seconds | ~1,072,338 seconds | FAIL |
| News audience/index cycle | Complete gRPC-backed identity/board evidence | SecurityIdentity unavailable; blocking memberships bypass bridge | FAIL |
| selection-v2 recovery tick | Authority-bound read | `production_database_connection_unverified` every 60s | FAIL |
| Remote mTLS and health | Verified, live, ready | TLS verified; live=true; ready=true | PASS |
| Remote opening capabilities | 11 opening-critical operations admitted/runtime available | all 11 ready; unrelated diagnostic-only capabilities remain excluded | PASS |
| Bundle/server proto parity | Same operation set | bundle ends at 55; server exposes 60 | FAIL |
| Production admission request | `allow_unadmitted=false` | focused BR-238 regression group passed | PASS |
| Instance-owned Bearer injection | request header set; process env unchanged | 1/1 focused test passed | PASS |
| Business-rule unique IDs | no duplicate numeric BR ID | checker passed; 229 rules | PASS |
| ExternalV1 request contracts | only delivered schemas admitted | positive/negative/duplicate tests GREEN | PASS |
| Bundle transport/auth | mTLS + instance Bearer; no secret environment mutation | 8/8 no-feature client tests passed | PASS |
| Bundle loader | Canonical contained nonempty files; secrets zeroized | 9/9 focused tests passed | PASS |
| External live readiness | Health + critical capabilities + frozen schemas | `opening_bundle_ready=true` | PASS |
| External identity projection | Partial metadata only, immutable evidence | real canary projected 1 record | PASS |
| Blocking board bridge | no-feature sync consumer uses gRPC | 1/1 focused test + default/no-default check | PASS |
| Dispatcher JSON writer | arbitrary error text remains valid JSONL | 2/2 focused tests | PASS |
| BR-139 review ordering | R04/R08/R09 closed initial phase | 37 focused/adjacent tests passed | PASS |
| SecurityIdentity clock boundary | accept bounded skew, reject future evidence | +1s accepted; +3s rejected; 30/30 converter tests | PASS |
| Integrated compilation | library + monitor | both cargo checks passed without warnings | PASS |

### Errors
| Error | Resolution |
|-------|------------|
| Catchup helper missing at `.codex` path | Re-ran from `.agents` complete package. |
| Process inspection denied by sandbox | Used approved escalation for read-only PID metadata. |
| Two jq projections failed on mixed known/unknown enum encoding | Converted operation to text for capability filtering; final query passed. |
