# migrate_selection_v2 CLI Contract (BR-193 §13.6)

**Status**: Gate B deliverable; frozen alongside BR-193 spec SHA `e203a98a...`
**Date**: 2026-08-01
**Owner**: BR-193 Gate B implementer
**Companion spec**: `docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md` §6.2 + §11

## 1. Surface

The `migrate_selection_v2` binary is a fixed-root, zero-argument CLI
release helper. It accepts exactly these flags in any combination, in
any order, with `--key=value` or `--key value` syntax:

| Flag | Argument | Required? | Subcommand |
|---|---|---|---|
| `--apply` | none | exclusive | production apply path |
| `--recover-pre-exchange` | none | exclusive | recovery before atomic exchange |
| `--recover-forward` | none | exclusive | recovery after forward closure |
| `--deactivate-generation` | none | exclusive | forward deactivation |
| `--restore-approved` | none | exclusive | Controlled Exception Path restore |
| `--help` | none | none | render help and exit 0 |
| `--log-format=<json\|text>` | enum | optional | stdout format; default `json` on success subcommands, `text` on error |

Exactly one of the five subcommand flags must be present, or `--help`,
or `--log-format` alone. Any other combination exits 2 with reason
`subcommand_conflict`. `--apply` and any recovery flag together exit 2
with reason `subcommand_conflict`. Any positional argument exits 2 with
reason `positional_argument_forbidden`.

A recovery subcommand flag is also forbidden if the production database
is missing the v2 catalog (`ApplicationId != 0x436f7273` or catalog
missing). In that case the binary exits 2 with reason
`production_database_not_v2`, regardless of the recovery flag.

The binary rejects all of the following CLI surface:

- any positional argument after the flags
- any `--db=<path>` / `--audit=<path>` / `--calendar=<path>` /
  `--url=<url>` / `--run=<id>` / `--hash=<hex>` / `--clock=<rfc3339>`
  override
- any environment variable prefixed `MIGRATE_SELECTION_V2_*` other than
  the `RUST_LOG` log filter (which `tracing_subscriber` reads)

## 2. Exit codes

| Code | Meaning | stdout carrier |
|---|---|---|
| 0 | success | one closed JSON object (see §3) |
| 1 | refused (approval missing/expired, lease cannot be acquired, or `migrate_selection_v2` CLI contract precondition violated) | one closed JSON object `{"reason":"<typed_reason_code>"}` |
| 2 | exchange refused (catalog hash drift, migration refused by safety gate, or frozen audit line refuses this migration) | one closed JSON object `{"reason":"<typed_reason_code>"}` |
| 3 | migration aborted pre-exchange (file/directory fsync failure, catalog hash invalid after revalidation, or any non-fatal exchange-step failure recoverable via `--recover-pre-exchange`) | one closed JSON object `{"reason":"<typed_reason_code>"}` |

Exit codes are exhaustive: any other value indicates a bug and is
itself a Gate C blocker (`migrate_selection_v2_exit_code_drift`).

## 3. Stdout carrier on success

Exactly one JSON object, written in `serde_json` compact format (no
insignificant whitespace, no trailing newline). The closed payload is:

```json
{
  "domain": "stock_analysis.selection_v2_migration_result.v1",
  "schema_version": 1,
  "subcommand": "<apply|recover_pre_exchange|recover_forward|deactivate_generation|restore_approved>",
  "exchanged": <bool>,
  "exchange_performed_at": <canonical nanos UTC, RFC 3339 with `Z` and 9 fractional digits>,
  "migration_run_identity": <canonical uuidv7>,
  "backup_descriptor": {
    "device": <u64>,
    "inode": <u64>,
    "size": <u64>,
    "content_sha256": <64 lowercase hex>
  },
  "post_exchange_descriptor": {
    "device": <u64>,
    "inode": <u64>,
    "size": <u64>,
    "content_sha256": <64 lowercase hex>
  },
  "source_catalog_hash": <64 lowercase hex>,
  "target_catalog_hash": <64 lowercase hex>,
  "selection_audit_prefix_record_count": <safe integer>,
  "selection_audit_prefix_tail_hash": <64 lowercase hex>,
  "selection_audit_record_count": <safe integer>,
  "gate_d_audit_record_hash": <64 lowercase hex>,
  "deactivation_or_restore_identity": <canonical uuidv7, present iff subcommand in {deactivate_generation, restore_approved}>,
  "deactivation_or_restore_expiry": <canonical nanos UTC, present iff subcommand in {deactivate_generation, restore_approved}>
}
```

The carrier is closed: any additional field, any reordered key, any
non-canonical encoding, any unsafe integer value, any non-lower-case
hex hash, any non-canonical uuidv7 form, any missing required field
fails `verify_br193_production_join.py` and is a Gate C blocker
(`migrate_selection_v2_stdout_drift`).

`safe integer` is the I-JSON subset: any integer in
`[-(2^53)+1, (2^53)-1]`. `exchanged` is `false` for `--recover-pre-exchange`
regardless of whether the recovery succeeded (the success carrier's
purpose is to log recovery, not exchange).

## 4. Stdout carrier on failure (exit 1 / 2 / 3)

Exactly one JSON object with closed payload:

```json
{"reason": "<typed_reason_code>", "subcommand": "<matching subcommand>", "exchanged": <bool, always false on exit 1/2/3>}
```

The closed `<typed_reason_code>` vocabulary:

- `subcommand_conflict`
- `positional_argument_forbidden`
- `production_database_not_v2`
- `approval_missing` (deactivate_generation, restore_approved only)
- `approval_expired` (deactivate_generation, restore_approved only)
- `approval_hash_mismatch` (deactivate_generation, restore_approved only)
- `exclusive_maintenance_lease_unavailable`
- `catalog_hash_drift`
- `catalog_sidecar_drift`
- `migration_safety_gate_refused`
- `migration_audit_line_refused`
- `descriptor_pinned_root_mismatch`
- `atomic_exchange_unsupported_filesystem`
- `backup_descriptor_invalid`
- `post_exchange_descriptor_invalid`
- `gate_d_audit_append_or_ack_failed`
- `recover_pre_exchange_no_pending_candidate`
- `recover_pre_exchange_identity_mismatch`
- `recover_forward_already_committed`
- `deactivate_already_deactivated`
- `restore_already_restored`
- `gate_d_official_revalidation_failed`
- `gate_d_official_revalidation_offline_only`

Unknown `<typed_reason_code>` strings fail Gate C
(`migrate_selection_v2_reason_code_unknown`).

## 5. Help / version

`--help` renders a single line to stdout:

```
migrate_selection_v2 [--apply | --recover-pre-exchange | --recover-forward | --deactivate-generation | --restore-approved] [--log-format=<json|text>]
```

No version flag, no interactive prompt, no stdin reads. Help exits 0
without taking any lease, lock, audit, or descriptor.

## 6. Log structure

If `--log-format=json`, every line on stdout after the single result
object is a closed JSON object with `{"level","ts","module","msg"}`
where `ts` is canonical nanos UTC, `level` is one of
`error/warn/info/debug/trace`, `module` is the Rust module path
shortened to top 3 segments, and `msg` is a single human sentence. If
`--log-format=text`, the format is `YYYY-MM-DDTHH:MM:SS.NNNNNNNNNZ
LEVEL MODULE: msg`.

Logs do not contain credentials, raw envelope bytes, raw payload bytes,
account identifiers, account balance, holding lists, provider token
bodies, or sink message bodies. A log line containing any of those
fails Gate C (`migrate_selection_v2_log_redaction_violation`).

## 7. Precondition matrix

The binary refuses to start (exit 2) when ANY of the following holds:

1. `os.platform()` is not in `{linux, macos}`;
2. The current working directory is not the compile-time manifest
   root;
3. The runtime caller is not root (uid 0) OR does not own the
   production database file (uid of the file) — production-only
   precondition;
4. `--apply` is invoked while the production maintenance lease is
   held by another process — Gate C blocker for concurrent apply.

Test runs (`--test` runtime, `TEST_CODE_*` namespace) bypass
precondition 3 and replace precondition 1 with "any unix-like".

## 8. Recovery subcommand identity binding

The `--recover-pre-exchange` flag is allowed ONLY when a previous
`--apply` left a candidate with `exchanged=false` and
`migration_state='Planned'` in `v2_migration_runs`. The recovery
subcommand binds to that exact candidate via `migration_run_identity`
(closed UUIDv7); attempting to recover a different
`migration_run_identity` exits 1 with reason
`recover_pre_exchange_identity_mismatch`.

The `--recover-forward` flag is allowed ONLY when the prior
`--apply` left `migration_state='Planned'` AND a forward closure
audit line is present at the captured prefix. It produces the
`Committed + authoritative activation receipt + ActivationReceipted`
audit chain and exits 0. Attempting `--recover-forward` when
`migration_state='Committed'` exits 1 with reason
`recover_forward_already_committed`.

## 9. Compatibility with `verify_br193_production_join.py`

The success carrier in §3 is the same closed JSON object that
`verify_br193_production_join.py` accepts (per BR-193 spec §10
AC-10). The verifier runs the binary once with each subcommand and
checks:

- exit code is one of the four in §2,
- stdout parses as exactly one closed JSON object,
- all required fields are present, no extra fields,
- `safe integer` fields parse as I-JSON integers,
- hash fields match `[0-9a-f]{64}`,
- `gate_d_audit_record_hash == selection_audit_prefix_tail_hash` per
  the prefix-tail equation from BR-193 §10 AC-10,
- failure carriers contain one of the §4 typed reason codes.

## 10. Change contract

Adding a new subcommand, a new reason code, or a new stdout field
requires a new spec revision (this document or BR-193) and a new
frozen SHA. The verifier rejects any carrier that does not match the
currently frozen schema.