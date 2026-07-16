# v18.x — Quant Platform Closure

> **Status**: design approved; implementation not started
> **Theme**: make the existing A-share monitor a reproducible research-to-paper-trading platform, then earn the right to enable controlled live execution.

## Position in the evolution

v17.x continues the push/event migration. v18.x does not replace that work. It establishes the safety and product contracts that every signal, push, virtual trade, and future broker order must share.

## Documents

| Document | Role | Status |
| --- | --- | --- |
| `v18.0-2026-07-16-review-quant-platform-assessment.md` | Evidence-based engineering, data, product, and institutional-practice assessment | complete |
| `v18.0-2026-07-16-brainstorming-quant-platform-closure-design-active.md` | Active architecture and acceptance design | active |
| `v18.0-2026-07-16-writing-plans-implementation-roadmap.md` | Sequenced implementation roadmap and merge gates | ready for approval by workstream |

## Non-negotiable boundary

Until the Live Execution Gate in the active design is satisfied, this system is a **research, monitoring, notification, and paper-trading platform**. It must not represent simulated orders, manually entered position adjustments, or notification delivery as broker-confirmed live execution.

## Relationship to prior work

- Adopt v17.x L1/L4/L5/L6/L7 push migration as the notification transport baseline.
- Reuse existing daily-bar quality validation, paper-trade isolation, risk checks, portfolio persistence, and post-close review where their contracts meet v18 requirements.
- Replace synthetic data-health construction and error-to-default decision inputs before they can influence an actionable recommendation or paper order.
