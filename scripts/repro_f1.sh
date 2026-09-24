#!/bin/bash
# F-1 reproduction: kill x in-flight resume.
#
# Requires the local harness from docs/e2b-f1-repro.md:
#   - Postgres + Redis containers
#   - E2B api on :3000 (ENVIRONMENT=local, local orchestrator discovery)
#   - dummy-orchestrator on :5008 with DUMMY_CREATE_DELAY_MS>0
#   - database seeded (team, API key, template)
#
# Usage: scripts/repro_f1.sh [iterations]
set -u

API=${API:-http://localhost:3000}
API_KEY=${API_KEY:?Set API_KEY to the local E2B test key}
TPL=${TPL:-11111111-1111-1111-1111-111111111111}
H="X-API-Key: $API_KEY"
ITER=${1:-3}

sid_of() { python3 -c "import sys,json;print(json.load(sys.stdin)['sandboxID'])"; }
state_of() { python3 -c "import sys,json;print(json.load(sys.stdin).get('state','<none>'))" 2>/dev/null || true; }

echo "# F-1 reproduction, $(date -u +%FT%TZ)"
echo "# api=$API template=$TPL delay_ms=${DUMMY_CREATE_DELAY_MS:-?}"
echo

for i in $(seq 1 "$ITER"); do
  echo "## race iteration $i"
  RESP=$(curl -s -X POST "$API/sandboxes" -H "$H" -H 'Content-Type: application/json' \
    -d "{\"templateID\":\"$TPL\",\"timeout\":600}")
  SID=$(echo "$RESP" | sid_of)
  echo "sandbox=$SID"
  curl -s -X POST "$API/sandboxes/$SID/pause" -H "$H" -o /dev/null \
    -w "pause_http=%{http_code}\n"

  # resume in flight; node create is held by DUMMY_CREATE_DELAY_MS.
  curl -s -X POST "$API/sandboxes/$SID/resume" -H "$H" -H 'Content-Type: application/json' \
    -d '{}' -o /dev/null -w "resume_http=%{http_code}\n" > /tmp/f1_resume_code &
  RPID=$!
  sleep 2
  curl -s -X DELETE "$API/sandboxes/$SID" -H "$H" \
    -o /dev/null -w "kill_http=%{http_code}\n"
  wait "$RPID"; cat /tmp/f1_resume_code
  sleep 1
  FINAL=$(curl -s "$API/sandboxes/$SID" -H "$H")
  echo "final_state=$(echo "$FINAL" | state_of)"
  echo
done

echo "## control: kill a paused sandbox with no resume in flight"
RESP=$(curl -s -X POST "$API/sandboxes" -H "$H" -H 'Content-Type: application/json' \
  -d "{\"templateID\":\"$TPL\",\"timeout\":600}")
SID=$(echo "$RESP" | sid_of)
echo "sandbox=$SID"
curl -s -X POST "$API/sandboxes/$SID/pause" -H "$H" -o /dev/null -w "pause_http=%{http_code}\n"
curl -s -X DELETE "$API/sandboxes/$SID" -H "$H" -o /dev/null -w "kill_http=%{http_code}\n"
sleep 2
curl -s "$API/sandboxes/$SID" -H "$H" -w " [http %{http_code}]\n"
