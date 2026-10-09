# Offline performance and observation checks

These commands never initialize the production database, contact a provider, send notifications, or grant source/admission/delivery authority. Fixtures and observations are not genuine historical counted-replay acceptance.

## Audit validation measurement

Build the private ignored harness in the test profile (incremental remains false):

```sh
cargo test --lib database::data_acquisition_audit::tests::performance_tests::performance_fixture --no-run
```

Copy the emitted `target/debug/deps/stock_analysis-…` executable to a task-owned directory **before changing the validator**. Record `rustc -Vv`, `cargo -V`, hardware/OS, source revision/diff, executable SHA-256, target, profile and features. Set `AUDIT_FIXTURE` to a new absolute filename starting `TEST_CODE_` inside the OS temp directory (`python3 -c 'import tempfile; print(tempfile.gettempdir())'`). Generate 100 and 100,000 rows in separate files/processes:

```sh
AUDIT_MODE=generate AUDIT_ROWS=100 AUDIT_FIXTURE="$fixture" "$baseline_executable" database::data_acquisition_audit::tests::performance_tests::performance_fixture --ignored --exact --nocapture --test-threads=1
```

Generation uses the production schema and hash serializer, fixed stored dates, null patterns and explicit IDs. Record each closed SQLite file's SHA-256, byte size, row count and returned tail. Keep the files and both executables. Reuse **the same bytes** for every measurement:

```sh
AUDIT_MODE=measure AUDIT_FIXTURE="$fixture" /usr/bin/time -l "$baseline_executable" database::data_acquisition_audit::tests::performance_tests::performance_fixture --ignored --exact --nocapture --test-threads=1
AUDIT_MODE=measure AUDIT_FIXTURE="$fixture" /usr/bin/time -l "$paged_executable" database::data_acquisition_audit::tests::performance_tests::performance_fixture --ignored --exact --nocapture --test-threads=1
```

Use `/usr/bin/time -v` on Linux. Each process prints five validation-only durations and verified tails; run 0 is cold for the connection (OS cache is not cleared), later runs are warm. Alternate old/new processes where possible. Peak RSS includes SQLite/runtime/allocator; it is not peak Rust allocation. Separate `AUDIT_MODE=payload` reports exact retained string lengths plus row struct sizes, excluding capacity, allocator, SQLite and transient serialization. It loads vectors and must **not** share the RSS measurement process. The reported maximum chunk of 1,024 rows estimates the paged retained payload.

The paging validator must verify every historical row in one read transaction. Hash domain, Serde field order and representations, genesis, historical admissibility, receipt reads and tail append behavior remain unchanged. Receipt and caller-owned rusqlite full-chain reads still allocate O(history). Hashing within the transaction extends the reader lifetime and can increase WAL retention/checkpoint pressure (or writer blocking outside WAL). No checkpoint is trusted and no prefix is skipped.

## Selected production benchmarks

Cargo declares `intraday_tick` with `harness = false`. Be aware that even `cargo bench --bench intraday_tick --no-run --profile test` can auto-build utility binaries for `CARGO_BIN_EXE`; stop if target selection expands. The Task 5 check instead compiled only `benches/intraday_tick.rs` with `rustc`, using the already-built test-profile library and Criterion rlibs, then ran `--test`. This is a smoke check, not optimized timing.

A finite direct command uses explicit artifact paths (never guess the profile of an arbitrary cached rlib):

```sh
rustc --edition=2021 --crate-name intraday_tick --crate-type bin -C opt-level=0 -C debuginfo=0 benches/intraday_tick.rs --extern "stock_analysis=$LIB_TEST_PROFILE_RLIB" --extern "criterion=$CRITERION_RLIB" -L "dependency=$DEPS" -L "native=$NATIVE_OUT_DIR" -o "$SMOKE_EXECUTABLE"
"$SMOKE_EXECUTABLE" --test
```

Repeat `-L native=…` for the native output directories required by those existing dependencies. The evidence bundle's `criterion-direct-command.json` records the exact successful local command, and `criterion-smoke.log` records all four cases. An optimized Criterion run remains explicit opt-in and unmeasured here; inspect target selection before using Cargo to build it.

The Criterion target evaluates actual pure veto observations for clear, veto, missing input and config-off cases. Assertions run before timing; fixtures and chains are constructed outside the timed body except the intentionally named config-off factory case. No end-to-end tick latency is claimed.

Bounded prediction SQL and market writes currently require private `#[cfg(test)]` manager/admission fixtures. The manager constructor initializes the complete isolated schema; there is no supported external Criterion fixture contract. Their timing remains unavailable here; existing targeted library regressions verify behavior. No admission constructor is made public solely for benchmarks, and mock timing is not substituted for real SQL/write timing.

## Opt-in isolated three-way build

From the repository, choose a committed revision and a new directory with an existing parent:

```sh
python3 scripts/performance_build_baseline.py --execute --commit HEAD --root /tmp/TEST_CODE_build_no_incremental --incremental 0 --profile dev
python3 scripts/performance_build_baseline.py --execute --commit HEAD --root /tmp/TEST_CODE_build_incremental --incremental 1 --profile dev
```

Each command exports committed source and owns a fresh target. It explicitly binds `STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT` to that disposable source directory and records it; an inherited production root is overridden. It measures cold target, unchanged repeat, then a controlled comment append to `src/bin/monitor/main.rs` in the disposable copy. All three use the same command/target/profile within that experiment. `--offline --locked` requires cached dependencies; failure is recorded, not treated as a timing success. Report records compiler/Cargo/OS identity, incremental override, timings, statuses, logs and logical target bytes. Cold target does not imply cold OS/dependency caches. A comment edit tests invalidation, not representative logic changes. Existing source/targets are never cleaned or edited. Cleanup is a separate explicit caller action limited to the chosen experiment root. Unless actually run, all three timings and incremental benefit remain **unmeasured**.

## SLI compatibility and soft comparison

```sh
python3 scripts/performance_sli.py compare baseline.json current.json
python3 scripts/performance_sli.py collect captured.log context.json
python3 -m unittest discover -s scripts/tests -p test_performance_sli.py
```

A record requires `version: 1`, `metric`, `stage`, `unit`, `statistic`, `scenario`, `snapshot`, `rows`, `target`, `profile`, `features`, `toolchain`, `hardware` and finite positive `value`. All identity fields must match; source commit/page size may be additional descriptive fields because the algorithm can change. Use immutable fixture/snapshot identity and explicit hardware identity. Missing, invalid or incompatible records return `unavailable`; >20% returns `warning`; both intentionally exit 0. CI checks the Python comparator contract only; no historical comparable CI baseline is assumed and no Rust target is added to CI for performance.

The log collector retains named startup stages and database initialization phases. Acquisition initialization includes schema work and must not be labeled validation-only. Validation-only timing comes from the audit harness. Existing review-duration logs are unavailable and are reported as such; do not replace them with provider latency or startup time.

## Bounded caller-owned day capture

```sh
cargo run --bin day_capture_check -- --manifest /path/capture/manifest.json --manifest-sha256 SHA256 --business-date 2026-10-08 --as-of 2026-10-08T15:00:00+08:00
```

The operator supplies the expected manifest digest, business date and as-of from their independently verified capture provenance. The tool validates their consistency, not that provenance's authority. A version-1 manifest is limited to 64 KiB:

```json
{
  "version": 1, "timezone": "Asia/Shanghai", "business_date": "2026-10-08",
  "as_of": "2026-10-08T15:00:00+08:00", "provenance": "caller-owned capture location/identity",
  "source_contract": "declared capture contract/version", "complete": false,
  "file": "2026-10-08.jsonl", "sha256": "SHA256_OF_EXACT_FILE_BYTES",
  "rows": 100, "bytes": 12345, "sources": ["captured_source"]
}
```

The adjacent regular JSONL file uses the existing EventEnvelope shape. Limits: 1 MiB per line including newline, 64 MiB total, 100,000 rows. Hashing and decoding consume the same descriptor; declared bytes/rows and final hash must match. Wrong date (Shanghai), future timestamp relative to explicit as-of, version, blank/undeclared identity, replay rewrite, or contradictory repeated IDs are rejected. Identical repeated IDs are reported separately from unique observations. Unknown event types are observations only. No timestamp/ID is generated, no capture is rewritten, no publisher or database is called.

Explicit `risk.stop_input.observed.v1` envelopes bind `entity_key` to `payload.code` and carry this payload:

```json
{"version":1,"code":"captured_code","name":"captured_name","current_price":8,"cost_price":10,"hard_stop":9,"ma20":8.5,"ma60":9.5}
```

Optional thresholds may be null; supplied prices must be finite and positive. Original envelope timestamp/source/identity and captured inputs are retained in stop observations. The existing `risk::stop_loss::check_stops` computes deterministic signals once per unique observation. This new capture contract requires source-bound values; hashes and this tool do not establish market admission or paper facts.

Incomplete capture reports unavailable. Even a structurally valid declared-complete capture always leaves **historical counted-replay acceptance unmeasured/pending** until genuine completeness and the actual counted decision/attempt/artifact/receipt join are independently established. Duplicate event IDs are neither counted opportunities nor physical receipt proof.
