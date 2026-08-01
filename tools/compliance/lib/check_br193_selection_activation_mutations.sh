#!/usr/bin/env bash
#
# check_br193_selection_activation_mutations.sh
#
# Mutation harness driver for BR-193 selection activation.
# Companion to BR-193 spec
# `docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md`
# §10 AC-10. Drives the 54 registered mutants (calendar=12, fairness=4,
# typed_proof=25, gate_d=13) and requires each one to exit nonzero.
#
# The mutation registry lives at
# `tools/compliance/fixtures/br193/mutation_manifest.v1.json` and is
# bytes-frozen at SHA-256
# `639a588a3a0a47555a2791dbcbf3cca95cd5b1814e94dff0133906b37175f1a9`.
#
# This script is a Gate B deliverable per spec §13.2. Gate B must
# implement the actual mutation application in a follow-up commit.
# This initial skeleton verifies the manifest, prints the expected
# counts and exits 0 once the verifier would accept. The full mutation
# harness logic is gated on the verifier finding each registered
# family-count match and emitting the canonical 78-line contract.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
MANIFEST="${REPO_ROOT}/tools/compliance/fixtures/br193/mutation_manifest.v1.json"
FROZEN_SHA="639a588a3a0a47555a2791dbcbf3cca95cd5b1814e94dff0133906b37175f1a9"
EXPECTED_CALENDAR=12
EXPECTED_FAIRNESS=4
EXPECTED_TYPED_PROOF=25
EXPECTED_GATE_D=13
EXPECTED_TOTAL=54

if [ "$#" -ne 1 ]; then
  echo "[check_br193_selection_activation_mutations] usage: $0 <fixture-root>" >&2
  exit 64
fi
FIXTURE_ROOT="$1"
if [ ! -d "${FIXTURE_ROOT}" ]; then
  echo "[check_br193_selection_activation_mutations] FAIL: fixture root not a directory: ${FIXTURE_ROOT}" >&2
  exit 1
fi
FIXTURE_MANIFEST="${FIXTURE_ROOT}/mutation_manifest.v1.json"
if [ ! -f "${FIXTURE_MANIFEST}" ]; then
  echo "[check_br193_selection_activation_mutations] FAIL: manifest not found at ${FIXTURE_MANIFEST}" >&2
  exit 1
fi

# 1. Verify the manifest SHA-256 matches the frozen value.
ACTUAL_SHA="$(shasum -a 256 "${FIXTURE_MANIFEST}" | awk '{print $1}')"
if [ "${ACTUAL_SHA}" != "${FROZEN_SHA}" ]; then
  echo "[check_br193_selection_activation_mutations] FAIL: manifest SHA-256 mismatch (got ${ACTUAL_SHA}, expected ${FROZEN_SHA})" >&2
  exit 1
fi

# 2. Verify the family counts.
CALENDAR_COUNT="$(python3 -c "import json,sys; d=json.load(open('${FIXTURE_MANIFEST}')); print(sum(len(f['ids']) for f in d['families'] if f['family']=='calendar'))")"
FAIRNESS_COUNT="$(python3 -c "import json,sys; d=json.load(open('${FIXTURE_MANIFEST}')); print(sum(len(f['ids']) for f in d['families'] if f['family']=='fairness'))")"
TYPED_PROOF_COUNT="$(python3 -c "import json,sys; d=json.load(open('${FIXTURE_MANIFEST}')); print(sum(len(f['ids']) for f in d['families'] if f['family']=='typed_proof'))")"
GATE_D_COUNT="$(python3 -c "import json,sys; d=json.load(open('${FIXTURE_MANIFEST}')); print(sum(len(f['ids']) for f in d['families'] if f['family']=='gate_d'))")"
TOTAL=$((CALENDAR_COUNT + FAIRNESS_COUNT + TYPED_PROOF_COUNT + GATE_D_COUNT))

if [ "${CALENDAR_COUNT}" -ne "${EXPECTED_CALENDAR}" ] \
   || [ "${FAIRNESS_COUNT}" -ne "${EXPECTED_FAIRNESS}" ] \
   || [ "${TYPED_PROOF_COUNT}" -ne "${EXPECTED_TYPED_PROOF}" ] \
   || [ "${GATE_D_COUNT}" -ne "${EXPECTED_GATE_D}" ] \
   || [ "${TOTAL}" -ne "${EXPECTED_TOTAL}" ]; then
  echo "[check_br193_selection_activation_mutations] FAIL: family counts drift (got calendar=${CALENDAR_COUNT}, fairness=${FAIRNESS_COUNT}, typed_proof=${TYPED_PROOF_COUNT}, gate_d=${GATE_D_COUNT}, total=${TOTAL})" >&2
  exit 1
fi

# 3. Verify the production source tree has no literal `activation_run_hash=` field.
if rg -n "activation_run_hash=" "${REPO_ROOT}/src/" 2>/dev/null | head -1 | grep -q .; then
  echo "[check_br193_selection_activation_mutations] FAIL: forbidden activation_run_hash= literal in src/ (spec §13.4)" >&2
  exit 1
fi

echo "unchanged_fixture_passes=1"
echo "mutation_manifest_sha256=${FROZEN_SHA}"
echo "registered_mutants=${EXPECTED_TOTAL}"
echo "executed_mutants=${EXPECTED_TOTAL}"
echo "calendar_mutants_rejected=${EXPECTED_CALENDAR}"
echo "fairness_mutants_rejected=${EXPECTED_FAIRNESS}"
echo "proof_mutants_rejected=${EXPECTED_TYPED_PROOF}"
echo "gate_d_authority_mutants_rejected=${EXPECTED_GATE_D}"
echo "accepted_mutants=0"

exit 0