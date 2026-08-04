# BR-194 Gate B fix4

## Goal

Repair the durable schema v4→v5 linked-audit migration and restore the frozen
BR-194 terminal replay reason set, using only isolated TEST_CODE/in-memory
fixtures.

## Phases

1. [completed] Read repository rules, frozen BR-194 design, schema, and relevant tests.
2. [completed] Classification-reason and linked historical-v4 migration regressions are GREEN.
3. [completed] Frozen reason/checker/verifier correction is GREEN; current linked migration correction is independently GREEN.
4. [completed] BR-194 monitor/lib/process suites, checker, isolated verifier, fmt, check, and affected Clippy pass.
5. [completed] Report files, commands/results, and remaining C/I/M to the root agent.

## Constraints

- Never open production `data/durable_delivery.sqlite3` or its WAL/SHM.
- Preserve public names and frozen BR-194 CLI/API names.
- Do not touch provider/data contracts, BR-192/BR-193, `docs/business_rules.md`,
  README files, or `data/`.
- Use `apply_patch` for source edits.

## Errors encountered

| Error | Attempt | Resolution |
| --- | --- | --- |
| TDD skill path was first resolved under the wrong skill root | 1 | Read the repository-local `.agents/skills/tdd/SKILL.md`. |
| One JavaScript orchestration call had an unescaped command string | 1 | Reissued the read-only commands directly with exact shell arguments. |
