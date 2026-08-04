# Independent Static Self-review

Date: 2026-07-29

Scope: typed outcome claim, outcome settlement owner, verified due read model,
claim recovery capability and ExpectedWait zero-attempt semantics.

## Static Result

The local owner choreography is ordered correctly:

1. fixed per-logical-subject nonblocking OS lock;
2. exact due snapshot revalidation;
3. durable outcome-claim receipt;
4. market-session gate;
5. Magic TDX provider acquisition only for a complete session;
6. durable outcome receipt.

The subject-lock leaf now opens with `O_NOFOLLOW`, mode `0600`, and a
post-open path/descriptor dev+inode equality check. The descriptor remains
owned until the outcome receipt completes; the retained lock file is never
deleted, leased, renewed or stolen. The private owner constructs exactly one
guard per invocation; no thread-local nesting state is used, so the async
future may safely migrate across Tokio worker threads while holding the OS
descriptor lock.

ExpectedWait carries no provider request pair, provider evidence, provider
error, provider-attempt row or outcome row. Its durable run still has a
manifest and receipt.

The scheduler now captures one `DateTime<FixedOffset>` and the outcome owner
rejects any offset other than `+08:00`. Exact clones of that instant—not new
wall-clock reads—drive recovery, due filtering, locked revalidation, claim
identity/timestamp, the market-session decision and Gateway freshness
projection. A latest receipted ExpectedWait is suppressed strictly before
`stored_due_date 15:00:00.000000001 +08:00`; at that instant it becomes
eligible. The next due binding includes the prior wait receipt in sorted
same-subject attempt lineage, so restart and serial-process behavior derive
from durable receipts rather than memory.

No `rustdx` or `mootdx` identifier exists in the reviewed outcome/read-model
files. Scoped `rustfmt` and `git diff --check` pass. Cargo has deliberately not
been run while another lane owns the serialized build window.

## Exact Added/Changed Tests

Schema and recovery:

- `expected_wait_has_no_provider_attempt_or_outcome_row`
- `outcome_claim_is_the_fifth_closed_subject_kind`
- `outcome_claim_binding_validation_is_scoped_to_outcome_stage`
- `expected_wait_rejects_a_half_present_transport_attempt_pair`
- `recovery_stage_payload_is_strict_and_canonically_reserialized`

Audit and repository persistence:

- `outcome_claim_audit_phases_are_permanently_parseable`
- `outcome_claim_owner_commits_exact_lineage_replays_and_rejects_tamper`
- `outcome_settled_binds_attempt_to_outcome_and_expected_wait_stores_zero_outcomes`
- `outcome_production_entrypoint_only_consumes_opaque_prepared_stage`

Settlement owner:

- `expected_wait_has_no_request_provider_or_error_and_no_provider_call_shape`
- `public_surface_has_no_caller_forgeable_outcome_stage_inputs`
- `owner_generated_run_identity_is_uuid_v7_and_unique`
- `owner_rejects_a_receipted_claim_from_another_due_or_request`
- `outcome_subject_lock_is_nonblocking_retained_and_reacquirable`
- `outcome_subject_lock_rejects_a_symlink_leaf`
- `claim_owner_surface_has_closed_skip_algebra_and_no_lease_or_steal`
- `production_owner_orders_lock_claim_provider_and_outcome_receipt`
- `expected_wait_deadline_is_strictly_one_nanosecond_after_close`
- `non_shanghai_tick_instant_is_rejected`
- `latest_expected_wait_is_suppressed_until_one_nanosecond_after_close`
- `receipted_expected_wait_suppression_is_restart_stable`
- `serial_owners_share_one_preclose_expected_wait_budget`
- `non_shanghai_due_tick_fails_closed`
- `eligible_after_deadline_carries_prior_wait_receipt_lineage`
- `settlement_owner_threads_one_fixed_tick_instant_without_recapturing_wall_clock`

## Remaining Blocking Items

1. **No Cargo evidence for the latest owner/read-model edits.** Required after
   the shared Cargo window is released: `cargo check --lib`, focused tests,
   full tests, clippy and compliance.
2. **Global descriptor-pinned read snapshot is absent.** The current read model
   uses `canonicalize` and metadata, so it cannot be Gate-D authority. The
   required seam is specified in
   `pinned_read_only_db_binding_contract.md`; this lane must not duplicate or
   modify the global schema owner.
3. **Recovery static implementation is present but has no Cargo evidence.**
   One shared classifier now drives the due anti-join and claim guard.
   Recovery distinguishes partial claim, receipted active claim, persisted
   outcome and exact closure. `ClaimActive` reuses the original claim receipt
   and preallocated run; `OutcomeRecovery` consumes the persisted outcome
   stage without a Gateway call. A recovery-purpose verified snapshot admits
   Committed-audit-before-receipt crash state without weakening ordinary
   authoritative reads. The production coordinator drains this work under the
   subject lock before requesting any new due work.

   Added recovery behavior tests:

   - `receipted_claim_without_outcome_receipt_is_recovery_not_due`
   - `recovery_reuses_exact_claim_and_planned_outcome_run_ids`
   - `second_claim_is_rejected_while_exact_claim_is_unclosed`
   - `active_claim_recovery_closes_original_claim_without_new_claim`
   - `outcome_envelope_without_receipt_recovers_without_provider_refetch`
   - `exact_outcome_receipt_removes_claim_from_recovery`
   - `cross_claim_or_wrong_planned_run_is_integrity_error_not_closed`
   - `crash_after_claim_receipt_before_outcome_envelope_recovers_one_logical_claim`
4. **The production scheduler call site is static-only evidence.** The
   post-session scheduler invokes `OutcomeSettlementOwner::settle_tick`, which
   drains recovery before due work and logs the complete result counts.
   Release build and isolated live-binary evidence remain required before this
   can be called active in production.
5. **The final DDL ExpectedWait trigger amendment is owned by the concurrent
   legacy-schema lane.** This lane must not edit `selection_v2.rs`; merge is
   blocked until its zero-attempt semantics match the typed schema.
6. **Owner order is currently tested statically, not behaviorally.** A
   physically isolated integration harness is still required to prove
   lock-busy skip, supersession before claim, ExpectedWait without provider
   I/O, claim-before-provider, and exact recovery after simulated interruption.
7. **Subject-lock ancestor traversal still uses path validation plus
   `create_dir_all`.** The leaf is no-follow and identity checked, but complete
   hostile-filesystem resistance requires descriptor-relative pinned directory
   traversal (or a narrow global lock-namespace owner seam).

## DoD Status

Status: **In Progress / Blocked**, not Done.

No provider I/O, production database write, migration apply or production
audit append was performed during this slice.
