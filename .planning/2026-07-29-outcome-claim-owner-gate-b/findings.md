# Findings

- `SubjectKind` currently exposes four variants although the amended database schema
  already permits the fifth token `outcome_claim`.
- `selection_v2.rs` final tables already contain outcome-claim lineage columns and
  closure triggers, but the outcome trigger still requires exactly one attempt for
  ExpectedWait.
- `selection_v2_repository.rs` loads receipted claim binding only under `cfg(test)`;
  production currently sets the binding to `None`.
- `outcome_v2.rs` constructs an ExpectedWait attempt before any provider call.
- `audit.rs` has outcome-run phases but no outcome-claim-specific phases.
- The frozen evidence-closure design gives the exact typed claim structs and domains
  in §7.13.2; the release amendment requires a NULL provider attempt pair for
  ExpectedWait and no attempt row for pre-provider failure.

