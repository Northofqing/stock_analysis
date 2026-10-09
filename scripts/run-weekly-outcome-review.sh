#!/bin/bash
set -euo pipefail

weekly_runtime_root="${STOCK_ANALYSIS_RUNTIME_ROOT:-${HOME}/.local/share/stock-analysis-runtime}"
exec /usr/bin/python3 "${weekly_runtime_root}/bin/weekly-outcome-review.py" \
  --binding-env-file "${weekly_runtime_root}/.env" \
  --database "${weekly_runtime_root}/data/stock_analysis.db" \
  --binary "${weekly_runtime_root}/bin/weekly_outcome_review" \
  --assistant-binary "${weekly_runtime_root}/bin/assistant_review" \
  --assistant-script "${weekly_runtime_root}/bin/run-weekly-assistant-review.py" \
  --weekly-output-root "${weekly_runtime_root}/reports/weekly-outcome-review"
