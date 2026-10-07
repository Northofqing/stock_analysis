#!/usr/bin/env bash
# BR-009: bound an owned command group and preserve its actual exit status.
# with_timeout <seconds> <command> [args...]; deadline => 2, cancellation => 128+signal.
with_timeout() {
    local timeout_secs="$1"
    shift
    local default_secs="${DEFAULT_TIMEOUT_SECS:-300}"
    if ! [[ "$timeout_secs" =~ ^[1-9][0-9]*$ ]]; then
        echo "BR-009: invalid timeout; using configured default" >&2
        timeout_secs="$default_secs"
    fi
    if ! [[ "$timeout_secs" =~ ^[1-9][0-9]*$ ]] || [ "$#" -eq 0 ]; then
        echo "BR-009: a positive timeout and command are required" >&2
        return 2
    fi
    python3 -c '
import math, os, signal, subprocess, sys, time
seconds = float(sys.argv[1])
if not math.isfinite(seconds) or seconds <= 0:
    print("BR-009: timeout must be finite and positive", file=sys.stderr)
    sys.exit(2)
interrupted = None
def request_stop(number, frame):
    global interrupted
    interrupted = number
for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
    signal.signal(number, request_stop)
try:
    child = subprocess.Popen(sys.argv[2:], start_new_session=True)
except OSError as error:
    print("BR-009: command could not start: " + error.strerror, file=sys.stderr)
    sys.exit(127)
deadline = time.monotonic() + seconds
while True:
    remaining = deadline - time.monotonic()
    if interrupted is not None or remaining <= 0:
        # The leader is still retained by Popen until wait reaps it. Kill only
        # its newly created session group, including children ignoring TERM.
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()
        if interrupted is not None:
            sys.exit(128 + interrupted)
        print("BR-009: command deadline exceeded; owned group stopped", file=sys.stderr)
        sys.exit(2)
    try:
        result = child.wait(timeout=min(remaining, 0.1))
        sys.exit(result if result >= 0 else 128 - result)
    except subprocess.TimeoutExpired:
        continue
' "$timeout_secs" "$@"
}
