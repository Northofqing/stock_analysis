#!/bin/bash
set -euo pipefail

weekly_runtime_root="${STOCK_ANALYSIS_RUNTIME_ROOT:-${HOME}/.local/share/stock-analysis-runtime}"
exec /usr/bin/python3 "${weekly_runtime_root}/bin/weekly-outcome-review.py" \
  --database "${weekly_runtime_root}/data/stock_analysis.db" \
  --binary "${weekly_runtime_root}/bin/weekly_outcome_review" \
  --weekly-output-root "${weekly_runtime_root}/reports/weekly-outcome-review"
