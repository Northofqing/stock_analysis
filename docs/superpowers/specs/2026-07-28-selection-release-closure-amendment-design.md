# Selection Release Closure Amendment

**Status:** Gate A amendment to
`2026-07-28-selection-evidence-closure-design.md`

**Scope:** BR-174, BR-178 and BR-180 only. This amendment closes three
release-blocking ambiguities without changing candidate scoring, trading
thresholds, provider choice or the fixed production database path.

**Frozen identity rule:** `AMENDMENT_DESIGN_SHA256` is SHA-256 over this
file's exact UTF-8 bytes after removing the complete line that starts with
`AMENDMENT_DESIGN_SHA256 =`. The parent `DESIGN_SHA` is not recomputed.

```text
AMENDMENT_DESIGN_SHA256 = "5c36c4a9d8b871de524186e9939717b8d888c15b06ac9543b9bec215796bc906" <!-- BR-182: §§5-7 outcome-claim closure -->
```

## 1. Why this amendment exists

Independent review found three contracts that the parent design did not make
executable:

1. the verified outcome-due database binding named SQLite
   `application_id`/`user_version` fields without fixing their values or legal
   transition;
2. the parent described a schema hash over “twelve v2 objects”, although the
   managed schema also contains five explicit indexes and static/dynamic
   triggers;
3. adaptive Magic TDX acquisition retained every transport attempt in memory,
   but the outcome-attempt row had no typed field that could preserve those
   attempts across persistence and recovery.

Production remains fail-closed until the amended schema, parser, owner,
migration and release evidence all pass Gate D.

## 2. Considered approaches

### 2.1 SQLite identity

- **Derive a version from outcome payload `v3`: rejected.** Payload schema
  version and whole-database migration generation are different domains.
- **Leave `application_id`/`user_version` at zero: rejected.** Zero cannot
  distinguish the fixed application database from an unrelated SQLite file
  and cannot prove a migration transition.
- **Fixed application identity plus first managed database generation:
  adopted.**

### 2.2 Schema hash

- **Hash only the twelve tables: rejected.** An index or trigger weakening
  would retain the same hash.
- **Whitespace-normalized SQL: rejected.** Removing or folding whitespace
  inside quoted SQL literals changes CHECK/trigger semantics and can create
  collisions.
- **Hash the complete named managed catalog using exact SQLite-emitted bytes:
  adopted.**

### 2.3 Transport-attempt evidence

- **Put transport attempts inside partial provider evidence: rejected.**
  Transport history and returned market records are distinct facts.
- **Keep attempts only in the recovery envelope: rejected.** Reporting and
  attempt audit would lose them after the envelope is no longer the primary
  read surface.
- **Add a typed JSON/hash pair to each outcome attempt: adopted.**

## 3. Whole-database identity and sole owner

The fixed application ID is the big-endian ASCII token `STSA`:

```text
STOCK_ANALYSIS_SQLITE_APPLICATION_ID_HEX = 0x53545341
STOCK_ANALYSIS_SQLITE_APPLICATION_ID = 1398035265
STOCK_ANALYSIS_DB_SCHEMA_GENERATION = 1
```

These are global `stock_analysis.db` values, not selection payload versions.
`DatabaseManager::GlobalSchemaVersionOwner` is the sole writer of
`PRAGMA application_id` and `PRAGMA user_version`. Selection, account,
position, order, news and reporting modules cannot write either PRAGMA.
Every future schema change anywhere in the shared database allocates the next
global generation in one checked-in `GlobalSchemaMigrationRegistry`; no
subsystem-local version may be stored in `user_version`. Selection migration
may only consume a non-forgeable `GlobalSchemaMaintenanceLease` issued by that
owner.

Generation allocation for this release is:

| Global generation | Application ID | Meaning |
| --- | ---: | --- |
| unmanaged source | `0` | exact historical database before global ownership |
| `1` | `1398035265` | first globally managed schema containing the final five selection payload schemas |

The only accepted identity transition for the exact pre-amendment database is:

```text
(application_id=0, user_version=0)
    -> (application_id=1398035265, user_version=1)
```

Fresh initialization writes `1398035265/1` only after one global exclusive
transaction has created the complete whole-application generation-1 schema,
not merely the twelve selection tables. The transaction commits and reads the
identity back before the connection pool is constructed. A nonempty `0/0`
database is never silently claimed during ordinary startup; only the offline
operator migration may perform that transition after proving preservation of
every application object and row. An already-amended database must already
contain exactly `1398035265/1`. Every other matrix, including
`0/1`, `1398035265/0`, a different nonzero application ID, a future user
version, or a negative PRAGMA representation, is unsupported identity and
fails closed. A future `STSA/N` is reported as `UnsupportedFutureGeneration`,
not reclassified as corruption and not opened by this release.

The migration manifest records both source and candidate values. The
candidate is not eligible for exchange until all schema/data/audit checks pass
and the candidate reads back `1398035265/1`.

This paragraph explicitly supersedes the parent design statement at
`§7.14.1` that only the allow-listed v2 objects and `user_version` may differ.
For the one transition above, the allow-list also includes the exact
`application_id: 0 -> 1398035265` change. No other parent text or frozen
parent identity changes. This amendment's own hash above is included in the
migration manifest, the candidate verification receipt and Gate D evidence.

## 4. Two complete, frozen managed schema catalogs

There are exactly two mode-keyed catalogs, never one union:

- `SelectionProductionCatalogV1`: 12 tables + 5 explicit indexes + 53
  triggers = 70 named objects;
- `SelectionTestCatalogV1`: the same 12 tables, 5 indexes and 50 common
  triggers, with the three test symbol triggers replacing the three
  production symbol triggers = 70 named objects.

The twelve table names are frozen in this order:

```text
selection_source_batch_attempts
selection_source_facts_v2
selection_source_fact_attempts
selection_relation_attempts
selection_evaluation_attempts
selection_samples
selection_rejections
selection_sample_outcomes
selection_outcome_attempts
selection_v2_recovery_envelopes
selection_v2_run_stages
selection_v2_commit_receipts
```

The five explicit index names are:

```text
selection_v2_one_activation_per_config
selection_v2_source_facts_pending
selection_v2_samples_generation
selection_v2_outcome_attempt_run
selection_v2_receipt_subject
```

The seventeen common static trigger names are:

```text
selection_v2_batch_lineage
selection_v2_fact_lineage
selection_v2_fact_attempt_lineage
selection_v2_relation_requires_admitted_source
selection_v2_evaluation_requires_admitted_source
selection_v2_sample_requires_admitted_source
selection_v2_rejection_requires_admitted_source
selection_v2_manifest_envelope_binding
selection_v2_config_manifest_closure
selection_v2_ingress_manifest_closure
selection_v2_generation_manifest_closure
selection_v2_outcome_manifest_closure
selection_v2_receipt_manifest_binding
selection_v2_config_receipt_closure
selection_v2_ingress_receipt_closure
selection_v2_generation_receipt_closure
selection_v2_outcome_receipt_closure
```

The nine common stage-membership triggers are exactly
`selection_v2_<table>_stage_membership` for, in order:

```text
selection_source_batch_attempts
selection_source_facts_v2
selection_source_fact_attempts
selection_relation_attempts
selection_evaluation_attempts
selection_samples
selection_rejections
selection_sample_outcomes
selection_outcome_attempts
```

The twenty-four common append-only triggers are exactly `<table>_deny_update`
and `<table>_deny_delete` for each of the twelve table names above, in table
order with update before delete. The three production-only names are:

```text
selection_v2_relation_symbol_isolation_production
selection_v2_evaluation_symbol_isolation_production
selection_v2_sample_symbol_isolation_production
```

The test catalog replaces those with:

```text
selection_v2_relation_symbol_isolation_test
selection_v2_evaluation_symbol_isolation_test
selection_v2_sample_symbol_isolation_test
```

Each catalog is an immutable registry of exact `(type,name,tbl_name,ddl_id)`
entries. Gate B builds a separate in-memory SQLite reference from the
checked-in DDL selected by `ddl_id`, reads the exact SQLite-emitted `sql` bytes
for every name, and compares the actual database against the same-mode
reference. Production rejects every test symbol trigger; test rejects every
production symbol trigger. It must not normalize whitespace, quotes, case,
comments, numeric spellings, string literals or identifier literals.

The schema hash preimage is:

```text
domain = "stock_analysis.br180.selection_managed_schema_catalog.v2"
mode = "production" | "test"
sqlite_runtime_identity
rows = exact registered sqlite_schema(type,name,tbl_name,sql) rows
```

Rows are sorted by:

```text
type ordinal: table=0, index=1, trigger=2
then name UTF-8 bytes
then tbl_name UTF-8 bytes
then sql UTF-8 bytes
```

`sqlite_schema.sql IS NULL` is forbidden for a registered named object.
SQLite-owned autoindexes are not in this named hash because they have no SQL.
For every table, the autoindex preimage contains
`(table,index_name,unique,origin,partial)` from `index_list` and ordered
`(seqno,cid,name,desc,coll,key)` from `index_xinfo`; it is sorted by table
UTF-8 bytes, index-name UTF-8 bytes and `seqno`. Unstable `index_list.seq` is
read but neither hashed nor used for ordering. `cid=-1/-2`, expression
columns, collation, descending order, key flags, generated names and
`WITHOUT ROWID` behavior must equal the same-runtime reference database.
The table's exact SQL independently binds every PK/UNIQUE declaration.

Any extra/missing/conflicting `selection_`-managed object fails. Every
non-catalog table is inspected through `PRAGMA foreign_key_list`; a reference
to a managed table fails. Every non-catalog index/trigger whose `tbl_name`
equals a managed table fails. For all other non-catalog `sqlite_schema.sql`,
the scanner removes `--` line comments and rejects nested/unterminated
`/* ... */` block comments, then tokenizes bare, single-quoted,
`"double-quoted"`, `` `backtick` `` and `[bracket]` tokens with escaped
closing delimiters. SQLite accepts a single-quoted token as an identifier in
some table-reference positions, so a decoded single-quoted token equal to any
managed table name fails even when the scanner cannot prove its syntactic
role; only `X'...'`/`x'...'` hex blobs are unconditionally treated as
literals. ASCII keywords are case-folded; identifier contents are compared
after SQLite delimiter unescaping. If any identifier token, including the
right side of `main.<identifier>`, equals a managed table name, inspection
fails. `OLD`/`NEW` are pseudo-table tokens only inside a trigger and never
authorize a managed reference. Unterminated comments/quotes, another schema
qualifier, or any unparsed token class fails closed. Objects owned by SQLite
are limited to the exact same-runtime reference autoindexes and documented
`sqlite_*` system objects; application-created `sqlite_` names are forbidden.

Exact SQLite-emitted bytes are compared only against a reference database
created by the same linked SQLite runtime. The runtime identity is
`sqlite3_libversion_number + sqlite_source_id + sorted compile_options hash`
and is bound into the catalog preimage and migration manifest. Supported
policy is SQLite `>=3.35.0,<4.0.0`; Gate D runs the exact catalog suite on the
release macOS runtime and CI Linux runtime. A runtime outside the range, or a
same-runtime reference that emits a different registry, fails closed.

### 4.1 Authority-safe DB/audit snapshot

The only authoritative classifier is
`GlobalSchemaVersionOwner::inspect_selection_with_audit`. Its lock order is
fixed and identical to the migration command:

1. acquire the process and OS **global database-maintenance lock** exclusively;
2. while it is held, acquire `LockedSelectionAuditSession` exclusively;
3. pin the manifest root, database and audit objects no-follow and record
   device/inode/size identity;
4. open the pinned database through the private inspection owner and acquire
   SQLite `BEGIN IMMEDIATE`; the owner exposes no DML method while the
   inspection capability exists;
5. record the validated audit prefix hash/high-water, PRAGMAs, catalog,
   dependencies, integrity checks and all twelve row counts;
6. revalidate the audit chain and require the same prefix/high-water, re-read
   PRAGMAs/catalog counts, and revalidate all object identities before commit;
7. finish the SQLite transaction, then the audit session, then release the
   maintenance lock.

Normal startup acquires one shared process/OS maintenance lease before pool
construction and stores it for the full `DatabaseManager` lifetime; individual
`get_conn()` calls do not re-enter or upgrade the OS lock. Every
selection-audit writer requires that shared lease. The offline operator is a
separate process and must acquire the exclusive lease before opening audit or
SQLite. An uncoordinated owner is a release blocker. The private,
non-`Clone` `VerifiedSelectionSchemaSnapshot<'locks>` retains the maintenance
guard, locked audit session, pinned database connection/transaction and
recorded high-water until it is consumed by eligibility or migration
validation. A detached DTO is diagnostic-only and can never enable selection
or exchange a candidate.

The authoritative state matrix is:

| Database half | Audit half | Result |
| --- | --- | --- |
| managed objects absent, `0/0` | valid chain with no v2 phase | `Absent` |
| exact historical catalog | valid matching historical prefix | `PreAmendment` |
| exact current four-payload catalog | valid matching v2 prefix | `TransitionalIncomplete` |
| exact final five-payload mode catalog, `STSA/1` | valid matching final receipts | `Amended` capability |
| any half missing, changed, future, mixed or contradictory | any | fail-closed drift |

A database-only inspection may return `DatabaseHalfOnly`, never authoritative
`Absent`, `PreAmendment`, `TransitionalIncomplete` or `Amended`.

Until all five payload schemas
`config-activation-stage-v1 | source-ingress-stage-v2 |
generation-stage-v3 | outcome-claim-stage-v2 | outcome-stage-v3` and their
complete closure triggers exist, the current schema is
`TransitionalIncomplete`, not `Amended`, and cannot enable new selection work.

## 5. Typed outcome transport-attempt evidence

`SelectionOutcomeAttemptRowContentPreimageV3` and
`selection_outcome_attempts` add this adjacent optional pair immediately
after the semantic request evidence fields:

```text
transport_attempts_json  TEXT NULL
transport_attempts_hash  TEXT NULL
```

The canonical preimage is:

```text
OutcomeTransportAttemptsPreimage {
    domain: "stock_analysis.br174.outcome_transport_attempts.v1",
    amendment_design_sha256,
    row_request_hash,
    request_evidence_hash,
    provider_capability_hash,
    provider_revision,
    request_parameters_hash,
    provider_request_hash,
    verified_due_binding_hash,
    adaptive_policy_version,
    expected_bar_count,
    maximum_latest_n,
    selected_transport_result_hash,
    attempts_in_request_order,
}
```

All nested request/result/provider-error/provider-batch types are strict
`Serialize + Deserialize` types with `deny_unknown_fields`. They move to the
shared schema module so the Gateway, stage validator, repository read-back and
recovery parser validate the same type. `serde_json::Value`, untagged
catch-alls, original-byte hashing and trailing input are forbidden.

The immutable schema rotation is exact:

```text
outcome attempt content: stock_analysis.br174.outcome_attempt.v3
outcome attempt row:     stock_analysis.br174.selection_outcome_attempts_row.v3
outcome payload:         stock_analysis.br174.outcome_payload.v2
outcome payload schema:  outcome-stage-v3
outcome stage:           stock_analysis.br174.outcome_stage.v3
provider request:         stock_analysis.br174.outcome_provider_request.v2
```

The recovery envelope row domain remains v1 because its structure is
unchanged, but its `payload_schema`, payload bytes and hashes bind
`outcome-stage-v3`. Every staged-db, manifest, prepared/committed audit and
receipt hash is recomputed from the v3 row bytes. Existing
`outcome_attempt.v2`, `selection_outcome_attempts_row.v2`,
`outcome_payload.v1` and `outcome-stage-v2` are accepted only by the
version-pinned sealed rollback parser; they are never reinterpreted or written
to the live database. Nonempty v2 outcome tables block automatic migration.

The v2 provider request and outer pair both bind the frozen parent
`DESIGN_SHA256` and this amendment's `AMENDMENT_DESIGN_SHA256`; a parent-only
provider request is permanently rejected by the live v3 parser. The outer pair
must equal the row's `request_hash`,
`request_evidence_hash`, decoded provider-capability hash, exact released
provider revision `660902ff93a07f18367dc16879cf67732accd25a`, canonical
request-parameters hash, canonical provider-request hash, immutable
`verified_due_binding_hash` and
`adaptive_policy_version="magic-tdx-latest-n-v1"`. Every nested attempt must
satisfy:

- ordinal equals its zero-based position;
- provider/source/instrument/market/interval/adjustment equal the semantic
  request capability and parameters;
- `latest_n` equals the one deterministic adaptive request at that ordinal;
- request/result/provider-evidence/provider-error hashes recompute exactly;
- provider records remain in provider order and bind their batch evidence;
- a typed historical-cardinality error obeys the upstream fixed 800-row page
  geometry and exact `requested_total`;
- successful earlier pages remain present when a later page fails.

The adaptive sequence is canonical. Let `E=expected_bar_count` and
`M=maximum_latest_n`, with `0<E<=M`. The first request is `E`. After an exact
success that does not cover the immutable window start, the next request is
`min(2*current,M)`. A non-cardinality provider error terminates. For a typed
cardinality failure at rejected request `R` with available count `A`, set
`low=last_success_latest_n or 0` and
`high=min(A,R-1,M)`. While `low<high`, request
`candidate=low+ceil((high-low)/2)`; exact success sets `low=candidate`, and a
typed cardinality failure sets `high=min(new_A,candidate-1)`. Any regression,
duplicate/omitted request, request after terminal error or sequence outside
these rules is invalid. `E`, `M`, window start and provider-request hash must
come from the same canonical row request evidence, not caller parameters.

For a settled result, the non-NULL `selected_transport_result_hash` must
identify exactly one successful attempt; its complete batch
content/evidence hashes must equal the admitted provider response and the
settled outcome's source evidence.
For a downstream validation/freshness failure after transport success, the
last attempt may be successful, but the outcome error stage must be typed and
available evidence must be an exact ordered subset derived from that named
successful attempt; this is `post_transport_validation_error`, never a provider
error, and its selected hash is non-NULL. For a provider failure, the last
attempt must be the typed terminal provider error and the selected hash is
NULL unless a retained highest-success result is explicitly used as available
evidence. In the cardinality upper-bound path, any selected result must equal
the retained highest-success result and no attempt may follow the terminal
decision. No partial evidence may exist without a successful attempt that
exactly derives it.

Field matrix:

- `expected_wait`: pair is NULL because no provider call occurred;
- `settled`: pair is non-NULL and contains every successful/adaptive request;
- provider `error`: pair is non-NULL and contains every request through the
  terminal failure;
- a failure before provider access is a claim/scheduling failure, writes no
  `selection_outcome_attempts` row, and is recorded only by the typed claim
  audit/recovery owner; it cannot fabricate an empty provider pair.

The outcome error fingerprint adds `transport_attempts_hash`; settled outcome
content, manifest and receipt hashes bind the exact same pair and selected
result hash. The recovery envelope carries the same typed v3 row bytes.
Receipt closure re-parses, canonicalizes and rehashes the pair and all
cross-links before visibility.

## 6. Data flow and failure behavior

```text
verified due / durable claim
  -> exact typed Magic TDX request
  -> ordered transport attempts
  -> typed JSON/hash pair
  -> outcome attempt row + outcome-stage-v3 envelope
  -> manifest/audit/receipt closure
  -> receipt-only report/backtest visibility
```

Malformed page geometry, cross-symbol attempts, missing attempts after a
provider call, hash mismatch, schema drift, audit mismatch or object-identity
change is an explicit integrity failure. None becomes an empty batch,
ExpectedWait, fabricated partial evidence or another provider fallback.

## 7. Validation

Gate B must include tests for:

- all legal and illegal application/user-version matrices;
- exact catalog mutation of each table/index/trigger family;
- quoted-literal whitespace/case mutations;
- extra dependent view, external trigger and external foreign key;
- one-snapshot row counts and concurrent mutation rejection;
- database-only/audit-missing and audit-v2/DB-absent matrices;
- legal 800-row page geometry plus malformed offset/page combinations;
- 801 through 899 and 1535 available-row recovery;
- successful request followed by typed short-page failure;
- cross-code/market/provider attempt injection;
- settled/error persistence and typed recovery of the complete attempt pair;
- ExpectedWait NULL matrix and pre-provider failure rejection.

Gate C and D remain the repository-wide commands in
`docs/ENGINEERING_RULES_V2.md`. Gate D additionally requires:

- fixed-path dry-runs against a complete real production DB+audit copy and a
  physically isolated `TEST_CODE` DB+audit copy;
- exact catalog mutation coverage in both production and test modes on the
  release macOS SQLite runtime and CI Linux SQLite runtime;
- live `monitor --review` and `monitor --test` evidence that exposes the same
  row request, verified-due, provider request, amendment-design, ordered
  transport-attempt and selected-result hashes;
- a migration manifest and candidate receipt containing the same
  `AMENDMENT_DESIGN_SHA256`.

## 8. Old modules and rollback

| Module | Disposition | Reason |
| --- | --- | --- |
| cardinality error-text parser | reject/delete | typed upstream error is authoritative |
| whitespace-compacted DDL comparison | reject/delete | can change quoted SQL semantics |
| database-only authoritative migration preflight | reject/delete | audit half is mandatory |
| partial provider-evidence field | adopt unchanged | returned records are not transport history |
| recovery envelope choreography | adopt/upgrade | binds the new typed row and claim lineage |

Before any production write, rollback is a scoped Git revert. After the first
receipted schema-v3/claim artifact, parser compatibility is permanent and
rollback is roll-forward only; audit, market, account, position, order and
selection evidence is never deleted.
