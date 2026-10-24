#!/bin/bash
# 通达信盘后日线：VM 打包 → 传到 Mac → 解压 → 增量导入 stock_daily。
#
# 为什么有这条：2026-10-19 起 provider 四路（Tdx/Tencent/Sina/Baidu）都拿不到日线，
# 唯一稳定的是 Windows VM 里通达信的盘后文件（C:\new_tdx64\vipdoc\*\lday\*.day，
# 32 字节定长、自带成交额）。这里把它做成可调度的一步，由 monitor 的 21:30 任务先跑。
#
# 约定（monitor 依赖这些退出码）：
#   0   成功：目标日期已入库且覆盖 >= COVERAGE_MIN 只
#   20  文件过期：TDX 客户端没下载当日盘后数据（max_date < 目标日）
#   21  传输失败：VM 不在线 / prlctl 失败 / 上传没到
#   22  导入失败：tdx_day_import 非 0 退出
#   23  覆盖不足：导入成功但当日 < COVERAGE_MIN
#
# 环境变量：SYNC_TARGET_DATE（默认今天）、STOCK_DB（默认 data/stock_analysis.db）、
#           TDX_VM（默认 "Windows 11"）、COVERAGE_MIN（默认 800）、ALLOW_STALE=1 允许非当日。
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || exit 21

DB="${STOCK_DB:-data/stock_analysis.db}"
VM="${TDX_VM:-Windows 11}"
TARGET="${SYNC_TARGET_DATE:-$(date +%F)}"
COVERAGE_MIN="${COVERAGE_MIN:-800}"
PORT="${TDX_PUT_PORT:-8911}"
BASE="$HOME/tdx_lday"
ALLOW_STALE="${ALLOW_STALE:-0}"
IMPORTER="$ROOT/target/release/tdx_day_import"

log() { echo "[tdx-sync] $*"; }

if [ ! -x "$IMPORTER" ]; then
  log "导入器不存在: ${IMPORTER}（先 cargo build --release --bin tdx_day_import）"
  exit 22
fi
if [ ! -f "$DB" ]; then
  log "STOCK_DB 不存在: $DB"
  exit 22
fi

MAC_IP="$(ifconfig 2>/dev/null | awk '/inet 10\.211\.55\./{print $2; exit}')"
if [ -z "$MAC_IP" ]; then
  log "找不到 10.211.55.x 地址（Parallels 共享网络未就绪）"
  exit 21
fi

mkdir -p "$BASE/incoming" "$BASE/sh_unzip" "$BASE/sz_unzip"
# 上一次异常退出可能留下占用端口的接收端
lsof -ti tcp:"$PORT" 2>/dev/null | xargs -r kill 2>/dev/null || true
rm -f "$BASE/incoming/tdx_sh.zip" "$BASE/incoming/tdx_sz.zip"

python3 "$ROOT/scripts/tdx_put_server.py" "$PORT" "$BASE/incoming" >"$BASE/put_server.log" 2>&1 &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null' EXIT
sleep 1

log "传输开始 vm=$VM mac=$MAC_IP port=$PORT target=$TARGET"
if ! prlctl exec "$VM" --current-user cmd /c \
  "tar -a -c -f %USERPROFILE%\\tdx_sh.zip -C C:\\new_tdx64\\vipdoc\\sh lday & \
   tar -a -c -f %USERPROFILE%\\tdx_sz.zip -C C:\\new_tdx64\\vipdoc\\sz lday & \
   curl -s -T %USERPROFILE%\\tdx_sh.zip http://$MAC_IP:$PORT/tdx_sh.zip & \
   curl -s -T %USERPROFILE%\\tdx_sz.zip http://$MAC_IP:$PORT/tdx_sz.zip" >/dev/null 2>&1; then
  log "prlctl 执行失败（VM 未运行或无法登录）"
  exit 21
fi
sleep 2
if [ ! -s "$BASE/incoming/tdx_sh.zip" ] || [ ! -s "$BASE/incoming/tdx_sz.zip" ]; then
  log "上传未完成：$(ls -l "$BASE/incoming" 2>/dev/null | tail -n +2 | tr '\n' ' ')"
  exit 21
fi

rm -rf "$BASE/sh_unzip/lday" "$BASE/sz_unzip/lday"
unzip -q -o "$BASE/incoming/tdx_sh.zip" -d "$BASE/sh_unzip" || { log "sh 解压失败"; exit 21; }
unzip -q -o "$BASE/incoming/tdx_sz.zip" -d "$BASE/sz_unzip" || { log "sz 解压失败"; exit 21; }

MAX_DATE="$(python3 - "$BASE/sh_unzip/lday" <<'PY'
import glob, os, struct, sys
best = 0
for path in glob.glob(os.path.join(sys.argv[1], "*.day")):
    size = os.path.getsize(path)
    if size < 32:
        continue
    with open(path, "rb") as handle:
        handle.seek(size - 32)
        raw = handle.read(32)
    date = struct.unpack("<I", raw[:4])[0]
    if date > best:
        best = date
print(f"{best // 10000:04d}-{best // 100 % 100:02d}-{best % 100:02d}" if best else "")
PY
)"
log "盘后文件最新日期: ${MAX_DATE:-未知}"
if [ -z "$MAX_DATE" ] || { [ "$MAX_DATE" \< "$TARGET" ] && [ "$ALLOW_STALE" != "1" ]; }; then
  log "文件未更新到 ${TARGET}（TDX 客户端需要跑一次「盘后数据下载」），跳过导入"
  exit 20
fi

SINCE="$(python3 -c 'import datetime,sys; print((datetime.date.fromisoformat(sys.argv[1]) - datetime.timedelta(days=10)).isoformat())' "$TARGET")"
log "增量导入 since=$SINCE db=$DB"
if ! STOCK_DB="$DB" "$IMPORTER" --dir "$BASE" --since "$SINCE" 2>&1 | sed 's/^/[tdx-sync] /'; then
  log "导入器失败"
  exit 22
fi

COVERAGE="$(sqlite3 "file:$DB?mode=ro" "SELECT COUNT(*) FROM stock_daily WHERE date='$TARGET';" 2>/dev/null || echo 0)"
log "当日覆盖 $TARGET = ${COVERAGE} 只（门槛 ${COVERAGE_MIN}）"
if [ "${COVERAGE:-0}" -lt "$COVERAGE_MIN" ]; then
  exit 23
fi
log "完成"
exit 0
