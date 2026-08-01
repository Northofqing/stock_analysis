# BR-193 Spec Gate A Re-review Brief

**Spec under review**: `docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md`
**Frozen SHA-256**: `e203a98a012bb86efb51bba184300426de4128a7fdfdfe04412ed07fae4c22b4`
**Previous SHA-256**: `9f604c33e3687eed9ed06c5b4790e71bfe65de227c106669dc9a21dc9b297dae` (prior corrective draft with `C0/I6/M0`)
**Status**: Corrective Gate A draft after C0/I6/M0; §13 prerequisites and out-of-scope blockers added; fresh re-review required
**Date prepared**: 2026-08-01
**Brief prepared by**: claude session (this corrective draft adds §13 explicitly to resolve previously-implicit scope gaps)

## 1. Scope of the corrective draft

This corrective draft replaces — does not layer on — every earlier unaccepted
BR-193 Gate A/B draft. Its entire textual basis is the bytes at SHA
`e203a98a...`. The previous draft at SHA `9f604c33...` is now historical
context only; re-reviewers must evaluate these exact bytes, not diff against
the prior SHA.

The corrective draft claims to have addressed the six Important items
returned by the prior independent Gate A review (`C0/I6/M0`). Reviewers must
verify whether the §13 additions actually remove the prior implicit scope gaps
or merely shift them. Each §13 subsection carries one or more AC numbers
whose implementation it makes concrete.

## 2. What changed since the previous SHA (`9f604c33...`)

Diff summary: +211 lines, -1 line.

| New / changed section | Lines (approx) | Purpose |
|---|---|---|
| Header status line updated | 3 | Adds `§13 prerequisites and out-of-scope blockers added` |
| New §0.2 "Pre-existing corrections in this draft" | 35-46 | Declares §13 normative and AC-numbered |
| New §13.1 Checked-in calendar authority prerequisites | end | Lists 5 missing calendar artifacts as Gate B deliverable |
| New §13.2 Verifier and mutation harness prerequisites | end | Lists 5 missing verifier/mutation files as Gate B deliverable |
| New §13.3 Known pre-existing test failures that affect Gate C | end | Documents 47 pre-existing failures as Gate C blocker; forbids BR-193 silent fix/weaken/`#[ignore]` |
| New §13.4 Log-line invariant enforcement (AC-9) | end | Splits AC-9 `activation_run_hash=` ban into static (Gate B) + runtime (Gate D) |
| New §13.5 Production scheduler function name | end | Locks `selection_v2_generation_scheduler_loop(capability, cadence, namespace)` signature |
| New §13.6 Migration binary CLI surface | end | Requires Gate B to commit CLI contract `2026-07-30-migrate-selection-v2-cli.md` |
| New §13.7 Gate D environmental prerequisites | end | Acknowledges Gate D needs production broker + push + real trading day |
| New §13.8 Closure rule for this §13 | end | §13 normative; same bar as §1-§12 |

## 3. Items to verify (reviewer's positive checklist)

| AC | Location | What to verify |
|---|---|---|
| **AC-1** | §10 | `TerminalDecisionKind` retained `Admitted`/`HardRejected`; no rename; static verifier |
| **AC-2** | §10 | Activation fail closed; spy test proves zero selection side effects when Disabled |
| **AC-3** | §10 | Offline production migration; atomic exchange; crash closure; golden vectors |
| **AC-4** | §10 | Pending query + concurrency; fairness round; 451-row paging; sealed-prefix recovery |
| **AC-5** | §10 | Canonical industry-chain candidates; direct mention + board evidence merge |
| **AC-6** | §10 | Magic TDX evidence; component persistence; lifecycle + corporate-action + BR-171 |
| **AC-7** | §10 | Terminal rejection is real; golden vectors for proof preimages; 25 mutation harness |
| **AC-8** | §10 | Forbidden dependencies (zero notification/sink/order/paper imports) |
| **AC-9** | §10 | Operational evidence; live producer-to-receipt summary; `activation_run_hash=` forbidden (now split per §13.4) |
| **AC-10** | §10 | Repository gates; mutation manifest SHA frozen; 78-line verifier output |

## 4. Items to NOT verify (explicitly out of re-review scope)

The re-review must NOT fail on the following because they are documented
out-of-scope blockers per §13:

1. **Absence of `config/selection/a_share_trading_calendar*.json`** (§13.1).
   Gate B must create these in the same PR; until then, activation returns
   typed Disabled reasons. The re-reviewer evaluates §13.1's enumeration, not
   the artifacts' content.
2. **Absence of `tools/compliance/fixtures/br193/mutation_manifest.v1.json`** (§13.2).
   Gate B must commit the file with bytes SHA-256
   `639a588a3a0a47555a2791dbcbf3cca95cd5b1814e94dff0133906b37175f1a9`.
3. **Absence of `selection_v2_verify_join` binary and `verify_br193_*.py`** scripts (§13.2).
4. **47 pre-existing lib-test failures** (§13.3). Verified at the time this
   brief was written that all 47 fail with `SelectionAuthorityContradiction`
   assertion mismatch on un-modified mainline HEAD; root cause documented in
   `progress.md` 2026-08-01 entry. Re-review must NOT request BR-193 to silently
   fix, weaken assertions or `#[ignore]` these. Fix path is a separate PR owned
   by the database context owner.
5. **Gate D live producer-to-receipt evidence** (§13.7). Not reproducible
   in sandbox. Gate D PR status must be `Gate C / Gate D pending live run`.

## 5. Reviewer commands

The reviewer must evaluate the frozen bytes literally. Use:

```bash
# 1. Confirm frozen SHA
sha256sum docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md
# expected: e203a98a012bb86efb51bba184300426de4128a7fdfdfe04412ed07fae4c22b4

# 2. Confirm mutation manifest SHA is locked at the expected value
sha256sum tools/compliance/fixtures/br193/mutation_manifest.v1.json
# expected: 639a588a3a0a47555a2791dbcbf3cca95cd5b1814e94dff0133906b37175f1a9
# (will fail until Gate B creates the file; that is acceptable for re-review)

# 3. Verify §13 subsections do not introduce new I-class issues
rg -n "^### 13\." docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md
# expected: 8 subsections, ordered 13.1..13.8

# 4. Verify each §13 subsection carries at least one AC number reference
rg -n "AC-[0-9]" docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md | tail -50
# expected: matches from §13 sections for AC-4 (pending query), AC-7 (proof preimages),
# AC-9 (log-line invariant), AC-10 (mutation manifest SHA)

# 5. Verify no `activation_run_hash=` literal anywhere
rg -n "activation_run_hash=" docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md
# expected: zero hits (spec is the reference, must not contain the forbidden log field)

# 6. Verify §0.2 and §13.8 closure rule are consistent
rg -A2 "^### 0\.2" docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md
rg -A2 "^### 13\.8" docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md
```

## 6. Acceptance criteria checklist for the reviewer

A reviewer may declare Gate A Green only when ALL of the following are true:

- [ ] Frozen SHA `e203a98a...` matches the bytes the reviewer evaluated.
- [ ] No §1-§12 text was silently weakened by §13 additions (diff against SHA `9f604c33...` in §1-§12 must be zero or comment-only).
- [ ] §13.1 enumerates 5 missing calendar artifacts; the names match §3.5 + §5.1 + §6.1 + §11.
- [ ] §13.2 enumerates 5 missing verifier/mutation files; the mutation manifest SHA `639a588a...` is locked at the expected value.
- [ ] §13.3 forbids silent fix / weaken / `#[ignore]` for the 47 pre-existing failures; references `progress.md` for evidence.
- [ ] §13.4 splits AC-9 log-line enforcement into static Gate B + runtime Gate D.
- [ ] §13.5 locks `selection_v2_generation_scheduler_loop` signature; static `rg` verification described.
- [ ] §13.6 requires a CLI contract doc for `migrate_selection_v2`; flag/exit-code/stdout carrier specified.
- [ ] §13.7 marks Gate D as `pending live run`; lists the 5 environmental prerequisites.
- [ ] §13.8 declares §13 normative with the same bar as §1-§12; if any Critical or Important objection is raised on §13, the corrective draft must be re-edited before Gate B starts.
- [ ] The full reviewer's check matrix (each AC-1..AC-10) has a verdict of either PASS or `OUT OF SCOPE per §13.X`.

## 7. Re-review verdict template

The reviewer should output a single verdict block at the end of their
re-review:

```text
Verdict on SHA e203a98a012bb86efb51bba184300426de4128a7fdfdfe04412ed07fae4c22b4:

AC-1: <PASS | OUT OF SCOPE | FAIL>
AC-2: ...
AC-3: ...
AC-4: ...
AC-5: ...
AC-6: ...
AC-7: ...
AC-8: ...
AC-9: ...
AC-10: ...

§0.1 / §0.2 normative authority: <PASS | FAIL>
§1 Decision: <PASS | FAIL>
§2 Rules and invariants: <PASS | FAIL>
§3 Current evidence and exact gaps: <PASS | FAIL>
§4 Adopted and rejected modules: <PASS | FAIL>
§5 Target interfaces: <PASS | FAIL>
§6 Offline migration and activation: <PASS | FAIL>
§7 Runtime data flow: <PASS | FAIL>
§8 Failure matrix: <PASS | FAIL>
§9 Observability and production evidence: <PASS | FAIL>
§10 Machine-checkable acceptance criteria: <PASS | FAIL>
§11 Implementation order and rollback: <PASS | FAIL>
§12 PR evidence fields: <PASS | FAIL>
§13 Prerequisites and out-of-scope blockers: <PASS | FAIL>

Overall: <GATE A GREEN | GATE A RED>

Blocking objections (if any):
- ...

Required revisions before Gate B (if any):
- ...
```

If Overall is `GATE A RED`, the reviewer's report MUST enumerate each
blocking objection as a line item and map it to the §13 subsection it
invalidates, so the corrective author can address each one specifically.

## 8. Hand-off

On `GATE A GREEN`: the next Gate B author must commit the §13.1 + §13.2 + §13.6
artifacts in a single PR, run the §10 AC-10 `run_exact_named_test` script for
all 20 named tests, and produce the 78-line `verify_br193_selection_activation.py`
output verbatim. None of those Gate B artifacts are subject to Gate A review.

On `GATE A RED`: the corrective author re-edits the spec to address each
blocking objection, computes a new SHA-256, and re-runs this review. Per
§0.1, no layer-on or stacking of corrective drafts is permitted; each
revision replaces the prior draft.