# Futures Delivery-Day Reminder Audit

## Goal

Verify whether the production monitor obtains source-backed delivery dates for
CFFEX, SHFE, DCE, CZCE, INE, and GFEX contracts and sends an advance reminder.
If the unified upstream already exposes a suitable typed contract, cut the
downstream production path over through a small Gateway. Otherwise document the
exact contract gap and add no formula-derived or fabricated dates.

## Scope Guard

- Do not edit `src/data_gateway/mod.rs` or Cargo files.
- Avoid files currently owned by other agents; message `/root` before any
  necessary shared-boundary edit.
- Do not modify the unified upstream repository in this slice.

## Phases

1. **Instruction and context preflight** — complete
2. **Build red-capable production-chain audit** — complete
3. **Trace stock_analysis production reminder path** — complete
4. **Audit unified upstream typed exchange evidence** — complete
5. **Choose outcome: implement cutover or document blocker** — complete
6. **Focused verification and evidence report** — complete

## Final status

Audit complete. Production implementation remains **Blocked** because the
unified upstream lacks the required source-backed six-exchange contract.
Repository Gate C also remains red on unrelated shared BR-161/BR-158 work.

## Required Evidence

- Exact production caller(s), scheduler condition, push kind, and audit event.
- Exact upstream type/provider methods for all six exchanges, or exact missing
  exchange/field/provider list.
- Proof that no generic delivery-date formula is used as source evidence.
- Focused command outputs and changed-file list.

## Errors

| Error | Attempt | Resolution |
| --- | --- | --- |
| `bash tools/docs/check_links.sh`: file not found | 1 | Repository has no docs link checker at that path; use available compliance checks plus `git diff --check` and scoped content checks. |
| Full compliance exits 1 | 1 | Existing shared Gate work is incomplete: BR-161 requires missing `src/data_gateway/event_calendar.rs`, and `src/data_gateway/review.rs` lacks BR-158 citation. This audit does not own those files and adds no active path. |
| New docs file ignored by `/docs` rule; staged diff check found three trailing-space lines | 1 | Force-added only the scoped document, removed trailing spaces, and reran staged checks. |
| One composite staged-check command hit `.git/index.lock` sandbox denial and later commands masked the segment exit | 1 | Reran `git add -f` as a single approved command, then ran each validation as a separate command; all scoped staged checks passed. |
