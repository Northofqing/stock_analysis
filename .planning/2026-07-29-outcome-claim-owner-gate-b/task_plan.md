# Outcome Claim Owner Gate B

## Goal

Implement the frozen BR-178 production outcome-claim lifecycle without provider I/O,
production database writes, or migration apply.

## Steps

1. Register the ExpectedWait no-attempt and typed claim lifecycle wording in BR-178.
2. Add strict typed outcome-claim schema, parser, subject kind, manifest and receipt bindings.
3. Add claim-specific Prepared/Committed audit phases.
4. Add repository request/recovery/stage/receipt support for `outcome_claim`.
5. Require outcome stages to bind the exact receipted claim.
6. Permit ExpectedWait outcome receipts with zero provider-attempt rows and no outcome row.
7. Run scoped format, tests and clippy during the coordinated Cargo window.
8. Perform an independent static review and report evidence.
9. Bind one strict `DateTime<FixedOffset>` Shanghai tick through due read,
   locked revalidation, claim identity, session gate and Gateway admission;
   suppress a latest receipted ExpectedWait until
   `15:00:00.000000001 +08:00`, then include its receipt in the next attempt
   lineage.

## Guardrails

- Do not edit `global_schema_v1.rs` or `global_schema_catalog_v1.rs`.
- Do not access providers or write a production database.
- Do not apply migrations.
- Preserve exact replay/recovery identities.
