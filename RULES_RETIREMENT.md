# Retire Repository Agent Rules

## Intent

Retire the repository-level agent instruction chain so future development is not governed by
the rules formerly loaded from `AGENTS.md` and its mandatory companion files.

## Scope

- Delete the root agent rules and the files whose purpose is to load, restate, or collect PR
  evidence for them.
- Remove active README and Claude guidance that enforced the retired development process.
- Preserve runtime code, tests, business-rule records, compliance tooling, and historical design,
  audit, handoff, and incident documents. Those files describe implemented behavior or history;
  this change does not reverse their underlying product behavior.

## Data Flow and Failure Modes

There is no runtime data-flow change. The relevant documentation failure mode is a dangling active
pointer that still tells an agent or developer to read a deleted file. Active entry points are
checked after the deletion. Historical references are retained as records of the rules in force
when those documents were written.

## Existing Modules

| Module | Decision | Reason |
| --- | --- | --- |
| `tools/compliance/` | retain | Executable project checks are outside the agent-instruction deletion. |
| `docs/business_rules.md` | retain | Records product behavior rather than loading repository agent instructions. |
| historical `docs/**` | retain | Preserve auditability of earlier decisions and incidents. |

## Rollback

Before commit, reverse the patch. After commit, use `git revert <commit>` to restore the retired
instruction files and navigation links.
