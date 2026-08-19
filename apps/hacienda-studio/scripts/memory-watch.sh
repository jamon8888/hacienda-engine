#!/usr/bin/env bash
# Memory watch script for Hacienda Studio dev session
# Monitors Chromium renderer RSS and aborts if it exceeds threshold
set -euo pipefail

THRESHOLD_KB=$((2800 * 1024))  # 2.8 GB
INTERVAL=2
LOG_FILE="${1:-/tmp/memory-watch.log}"

# Session id of the shell that launched this script — used below to scope renderer
# discovery to browsers started in this session, not every renderer on the machine.
SESSION_ID=$(ps -o sid= -p $$ | tr -d ' ')

echo "=== Memory Watch ===" | tee "$LOG_FILE"
echo "Threshold: $((THRESHOLD_KB / 1024))MB" | tee -a "$LOG_FILE"
echo "Interval: ${INTERVAL}s" | tee -a "$LOG_FILE"
echo "Log: $LOG_FILE" | tee -a "$LOG_FILE"
echo "" | tee -a "$LOG_FILE"

while true; do
  # Find Chromium renderer processes belonging to this session only — `pgrep -f` alone
  # matches every renderer on the machine, including ones from unrelated browser
  # windows, and would SIGTERM them if they happened to exceed the threshold.
  PIDS=$(pgrep -f "chromium.*--type=renderer" 2>/dev/null | while read -r pid; do
    [ "$(ps -o sid= -p "$pid" 2>/dev/null | tr -d ' ')" = "$SESSION_ID" ] && echo "$pid"
  done || true)
  if [ -z "$PIDS" ]; then
    # Try Chrome instead
    PIDS=$(pgrep -f "chrome.*--type=renderer" 2>/dev/null | while read -r pid; do
      [ "$(ps -o sid= -p "$pid" 2>/dev/null | tr -d ' ')" = "$SESSION_ID" ] && echo "$pid"
    done || true)
  fi

  if [ -z "$PIDS" ]; then
    echo "[$(date +%H:%M:%S)] No renderer process found" | tee -a "$LOG_FILE"
  else
    for PID in $PIDS; do
      RSS_KB=$(ps -p "$PID" -o rss= 2>/dev/null | tr -d ' ' || echo "0")
      RSS_MB=$((RSS_KB / 1024))
      TIMESTAMP=$(date +%H:%M:%S)

      echo "[$TIMESTAMP] PID=$PID RSS=${RSS_MB}MB" | tee -a "$LOG_FILE"

      if [ "$RSS_KB" -gt "$THRESHOLD_KB" ]; then
        echo "" | tee -a "$LOG_FILE"
        echo "[$TIMESTAMP] ABORT: RSS ${RSS_MB}MB exceeds threshold $((THRESHOLD_KB / 1024))MB" | tee -a "$LOG_FILE"
        echo "Killing renderer PID $PID..." | tee -a "$LOG_FILE"
        kill -TERM "$PID" 2>/dev/null || true
        exit 1
      fi
    done
  fi

  # Also show total system memory. The timestamp is computed in the shell and passed in
  # via -v — a single-quoted awk script never expands `$(date ...)`, so a literal,
  # unexpanded "$(date +%H:%M:%S)" was being written to every line here before this fix.
  NOW=$(date +%H:%M:%S)
  free -h | head -2 | tail -1 | awk -v ts="$NOW" '{print "["ts"] System: "$2" total, "$3" used, "$4" free"}' | tee -a "$LOG_FILE"

  sleep "$INTERVAL"
done
