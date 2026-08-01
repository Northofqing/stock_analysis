#!/usr/bin/env bash
#
# check_br193_scheduler_signature.sh
#
# Static signature check for the locked BR-193 production scheduler
# function. Per BR-193 spec §13.5, the production scheduler function
# is locked to:
#
#   pub async fn selection_v2_generation_scheduler_loop(
#       capability: &SelectionRuntimeCapability,
#       cadence: &CadenceOwner,
#       namespace: &SelectionNamespaceOwner,
#   ) -> Result<(), SelectionRuntimeError>
#
# This script:
# - finds the function declaration in src/selection/ via rg (multiline-aware);
# - asserts the parameter list matches the closed signature verbatim;
# - asserts the return type matches;
# - emits a one-line PASS/FAIL summary.
#
# §10 AC-7 multiline static evidence also requires exactly one
# production callsite for ReceiptedTerminalDecisionProof construction.
# That secondary check is encoded as `selected_projection_named_production_consumer=1`
# in §10 AC-10, and is asserted by verify_br193_selection_activation.py
# (separate file); this script focuses on the signature itself.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${REPO_ROOT}"

EXPECTED_FN="selection_v2_generation_scheduler_loop"
EXPECTED_PARAMS="capability: &SelectionRuntimeCapability, cadence: &CadenceOwner, namespace: &SelectionNamespaceOwner"
EXPECTED_RETURN="Result<(), SelectionRuntimeError>"

# 1. Find the function declaration (multiline-aware).
DECL_HIT="$(rg -U -n "fn ${EXPECTED_FN}\(" src/selection/ 2>/dev/null || true)"
if [ -z "${DECL_HIT}" ]; then
  echo "[check_br193_scheduler_signature] FAIL: function ${EXPECTED_FN} not found in src/selection/" >&2
  exit 1
fi

# 2. Extract the parameter block from the declaration. We expect:
#   pub async fn selection_v2_generation_scheduler_loop(
#       capability: &SelectionRuntimeCapability,
#       cadence: &CadenceOwner,
#       namespace: &SelectionNamespaceOwner,
#   ) -> Result<(), SelectionRuntimeError>
PARAM_BLOCK="$(rg -U -o "fn ${EXPECTED_FN}\(\s*\n\s*${EXPECTED_PARAMS//,/, }" src/selection/ 2>/dev/null || true)"
if [ -z "${PARAM_BLOCK}" ]; then
  echo "[check_br193_scheduler_signature] FAIL: parameter block does not match the locked signature" >&2
  echo "[check_br193_scheduler_signature] expected parameters: ${EXPECTED_PARAMS}" >&2
  echo "[check_br193_scheduler_signature] actual declarations:" >&2
  echo "${DECL_HIT}" >&2
  exit 1
fi

# 3. Confirm the return type.
RETURN_HIT="$(rg -U -B1 "fn ${EXPECTED_FN}\(" src/selection/ 2>/dev/null | rg "\) -> ${EXPECTED_RETURN// / }" || true)"
RETURN_LIT_HIT="$(rg -U -B1 "fn ${EXPECTED_FN}\(" src/selection/ 2>/dev/null | rg "\) -> Result<\(\), SelectionRuntimeError>" || true)"
if [ -z "${RETURN_HIT}" ] && [ -z "${RETURN_LIT_HIT}" ]; then
  echo "[check_br193_scheduler_signature] FAIL: return type does not match the locked signature" >&2
  echo "[check_br193_scheduler_signature] expected return: ${EXPECTED_RETURN}" >&2
  exit 1
fi

# 4. Spec §10 AC-7 secondary check: exactly one production callsite for
#    ReceiptedTerminalDecisionProof construction.
PROOF_CALLSITES="$(rg -U -n "ReceiptedTerminalDecisionProof::" src/ 2>/dev/null || true)"
PROOF_PROD_CALLSITES="$(printf "%s\n" "${PROOF_CALLSITES}" | rg -v "_tests/" | rg -v "selection_v2_verify_join" | rg -v "/tests/" || true)"
PROOF_CALLSITE_COUNT="$(printf "%s\n" "${PROOF_PROD_CALLSITES}" | rg -c "." || true)"
if [ "${PROOF_CALLSITE_COUNT}" -ne 1 ]; then
  echo "[check_br193_scheduler_signature] FAIL: ReceiptedTerminalDecisionProof production callsite count != 1 (got ${PROOF_CALLSITE_COUNT})" >&2
  echo "[check_br193_scheduler_signature] expected exactly 1 production callsite in selection_v2_generation_scheduler_loop" >&2
  if [ -n "${PROOF_PROD_CALLSITES}" ]; then
    echo "${PROOF_PROD_CALLSITES}" >&2
  fi
  exit 1
fi

echo "[check_br193_scheduler_signature] PASS"
echo "function=${EXPECTED_FN}"
echo "parameter_block=${EXPECTED_PARAMS}"
echo "return_type=${EXPECTED_RETURN}"
echo "receipted_terminal_decision_proof_production_callsites=${PROOF_CALLSITE_COUNT}"

exit 0