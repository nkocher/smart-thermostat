#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${THERMOSTAT_BASE_URL:-http://127.0.0.1:8080}"
SLEEP_SECONDS="${THERMOSTAT_IR_STEP_DELAY:-1}"

log() {
  printf '[validate] %s\n' "$*"
}

fetch_json() {
  local path="$1"
  curl -fsS "${BASE_URL}${path}"
}

post_no_body() {
  local path="$1"
  curl -fsS -X POST "${BASE_URL}${path}" >/dev/null
}

json_field() {
  local json="$1"
  local field="$2"

  if command -v jq >/dev/null 2>&1; then
    jq -r "${field}" <<<"${json}"
  else
    echo ""
  fi
}

log "Controller base URL: ${BASE_URL}"

log "Checking required controller endpoints"
status_json="$(fetch_json /api/status)"
ir_before="$(fetch_json /api/ir/diagnostics)"
ota_status="$(fetch_json /api/ota/status)"

if command -v jq >/dev/null 2>&1; then
  log "Current thermostat state: $(jq -r '.state' <<<"${status_json}")"
  log "Current temperature (F): $(jq -r '.currentTemp' <<<"${status_json}")"
  log "Sensor valid: $(jq -r '.sensorValid' <<<"${status_json}")"
  log "IR sent frames before test: $(jq -r '.sentFrames // 0' <<<"${ir_before}")"
  log "IR failed actions before test: $(jq -r '.failedActions // 0' <<<"${ir_before}")"
  log "OTA supported: $(jq -r '.supported // false' <<<"${ota_status}")"
else
  log "Install jq for parsed output. Raw /api/ir/diagnostics follows:"
  printf '%s\n' "${ir_before}"
fi

log "Running IR smoke sequence (on/off/heat/light/timer)"
sequence=(
  "/api/ir/on"
  "/api/ir/off"
  "/api/ir/heat/on"
  "/api/ir/heat/off"
  "/api/ir/light/toggle"
  "/api/ir/timer/toggle"
)

for endpoint in "${sequence[@]}"; do
  log "POST ${endpoint}"
  post_no_body "${endpoint}"
  sleep "${SLEEP_SECONDS}"
done

ir_after="$(fetch_json /api/ir/diagnostics)"

if command -v jq >/dev/null 2>&1; then
  before_sent="$(jq -r '.sentFrames // 0' <<<"${ir_before}")"
  after_sent="$(jq -r '.sentFrames // 0' <<<"${ir_after}")"
  before_failed="$(jq -r '.failedActions // 0' <<<"${ir_before}")"
  after_failed="$(jq -r '.failedActions // 0' <<<"${ir_after}")"

  sent_delta=$((after_sent - before_sent))
  failed_delta=$((after_failed - before_failed))

  log "IR sent frame delta: ${sent_delta}"
  log "IR failed action delta: ${failed_delta}"

  if (( sent_delta <= 0 )); then
    log "ERROR: IR sent frame counter did not increase."
    exit 1
  fi

  if (( failed_delta > 0 )); then
    log "WARNING: IR failed actions increased by ${failed_delta}."
  fi
else
  log "Raw /api/ir/diagnostics after test:"
  printf '%s\n' "${ir_after}"
fi

log "Validation run complete."
log "Manual check: confirm each command visibly changed fireplace state."
