#!/usr/bin/env bash
# Memory watch script for Hacienda Studio dev session
# Monitors Chromium renderer RSS and aborts if it exceeds threshold
set -euo pipefail

THRESHOLD_KB=$((2800 * 1024))  # 2.8 GB
INTERVAL=2
LOG_FILE="${1:-/tmp/memory-watch.log}"

echo "=== Memory Watch ===" | tee "$LOG_FILE"
echo "Threshold: $((THRESHOLD_KB / 1024 / 1024)) GB" | tee -a "$LOG_FILE"
echo "Interval: ${INTERVAL}s" | tee -a "$LOG_FILE"
echo "Log: $LOG_FILE" | tee -a "$LOG_FILE"
echo "" | tee -a "$LOG_FILE"

while true; do
  # Find Chromium renderer processes (not the GPU or utility processes)
  PIDS=$(pgrep -f "chromium.*--type=renderer" 2>/dev/null || true)
  if [ -z "$PIDS" ]; then
    # Try Chrome instead
    PIDS=$(pgrep -f "chrome.*--type=renderer" 2>/dev/null || true)
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

  # Also show total system memory
  free -h | head -2 | tail -1 | awk '{print "[$(date +%H:%M:%S)] System: "$2" total, "$3" used, "$4" free"}' | tee -a "$LOG_FILE"

  sleep "$INTERVAL"
done
