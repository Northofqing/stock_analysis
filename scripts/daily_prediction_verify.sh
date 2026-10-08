#!/bin/bash
# launchd com.stockanalysis.prediction-verify 的盘后入口。
# 安装到 runtime/bin/；默认数据库、工具及日志始终属于同一个运行根。
# STOCK_ANALYSIS_RUNTIME_ROOT 仅用于显式选择另一完整运行根（如隔离测试）。
set -u

ROOT="${STOCK_ANALYSIS_RUNTIME_ROOT:-/Users/zhangzhen/.local/share/stock-analysis-runtime}"
export STOCK_DB="$ROOT/data/stock_analysis.db"
export GRPC_MARKET_ADDR="http://127.0.0.1:18082"
export BACKFILL_DAYS=10
BIN="$ROOT/bin"

selection_exit_code=0
daily_exit_code=null
prediction_exit_code=null
job_exit_code=0

finish() {
  echo "=== done $(date '+%F %T') exit_code=$job_exit_code ==="
  # 这里只汇总工具退出码；原 verifier 的 deferred/错误诊断保留原样。
  # 命令 exit0 不证明全部窗口成熟、历史资格到位或合格收益回填完成。
  printf '[daily] report {"schema_version":1,"selection_exit_code":%s,"daily_exit_code":%s,"prediction_exit_code":%s,"exit_code":%s,"outcome_result_status":"not_evaluated_by_wrapper"}\n' \
    "$selection_exit_code" "$daily_exit_code" "$prediction_exit_code" "$job_exit_code"
  exit "$job_exit_code"
}

echo "=== daily prediction verify $(date '+%F %T') ==="
case "$ROOT" in
  /*) ;;
  *)
    echo "[daily] 运行根必须为绝对路径，停止回填" >&2
    selection_exit_code=2
    job_exit_code=2
    finish
    ;;
esac
if ! cd "$ROOT" || [ ! -f "$STOCK_DB" ]; then
  echo "[daily] 运行根或原数据库不存在，停止回填，不创建数据库" >&2
  selection_exit_code=2
  job_exit_code=2
  finish
fi

# SQLite 只读选取所有尚有 NULL 窗口的证券；查询失败不能当成无待验证证券。
if CODES=$(sqlite3 -readonly "$STOCK_DB" "
SELECT group_concat(stock_code, ',') FROM (
  SELECT DISTINCT stock_code FROM prediction_tracker
  WHERE stock_code IS NOT NULL AND stock_code != ''
    AND (actual_change_t1 IS NULL OR actual_change_t3 IS NULL OR actual_change_t5 IS NULL)
  ORDER BY stock_code
);
"); then
  :
else
  selection_exit_code=$?
  job_exit_code=$selection_exit_code
  echo "[daily] 待验证证券查询失败 exit_code=${selection_exit_code}，停止回填" >&2
  finish
fi

if [ -n "$CODES" ]; then
  echo "[daily] 补日线: $CODES"
  # 整批只初始化一次。部分写入成功仍保留；后续预测阶段仍检查可用数据。
  if "$BIN/backfill_daily" "$CODES" >>"$ROOT/logs/backfill-daily.log" 2>&1; then
    daily_exit_code=0
  else
    daily_exit_code=$?
    job_exit_code=$daily_exit_code
    echo "[daily] backfill_daily exit_code=${daily_exit_code}，已成功行保留；详见 backfill-daily.log" >&2
  fi
else
  echo "[daily] 无待验证股票，跳过补日线"
fi

echo "[daily] 运行预测验证回填"
if "$BIN/backfill_predictions"; then
  prediction_exit_code=0
else
  prediction_exit_code=$?
  if [ "$job_exit_code" -eq 0 ]; then
    job_exit_code=$prediction_exit_code
  fi
  echo "[daily] backfill_predictions exit_code=${prediction_exit_code}，原错误/deferred 行保持 pending" >&2
fi

finish
