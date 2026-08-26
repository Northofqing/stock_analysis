# Repository Instructions

Before changing this repository, read these files in order:

1. `AGENTS.md` — highest-priority repository rules.
2. `docs/ENGINEERING_RULES_V2.md` — engineering and release gates.
3. This file.
4. `CLAUDE.md` — project-specific implementation guidance.

Mandatory constraints:

- Output the AGENTS §1.3 pre-flight plan before editing any file.
- Treat AGENTS §§2.1–2.10 as blocking Definition-of-Done checks.
- Never use mock, zero, empty, or stale fallbacks as production evidence.
- Missing fields remain absent and failures remain explicit.
- Test and live symbols, accounts, databases, logs, and notifications stay isolated.
- Register deduplication, mutex, filter, sort, and limit logic in `docs/business_rules.md` before implementation.
- Run format, strict Clippy, all tests, offline compliance, patch coverage, and the coverage ratchet before declaring Gate C merge readiness.
- Keep the PR Draft and do not merge while any Gate C requirement is missing. Missing Gate D production freshness/live evidence blocks release and deployment, not a Gate C-complete merge.
