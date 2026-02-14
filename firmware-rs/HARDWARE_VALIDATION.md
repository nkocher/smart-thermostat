# Hardware Validation Workflow

This checklist validates the Rust firmware rewrite on real hardware with emphasis on reliable IR command emission.

## Preconditions

- Controller and sensor firmware flashed with the Rust binaries.
- MQTT broker reachable by both devices.
- Controller web API reachable on your LAN.
- `curl` available.
- Optional: `jq` for parsed output.

## 1) Controller API + IR Baseline

Run:

```bash
cd firmware-rs
THERMOSTAT_BASE_URL=http://<controller-ip> ./tools/hardware_validation.sh
```

What it verifies:

- `GET /api/status` is healthy.
- `GET /api/ir/diagnostics` is reachable.
- IR command sequence can be posted.
- `sentFrames` increases after command sequence.

Pass criteria:

- `IR sent frame delta` is positive.
- `IR failed action delta` is `0`.
- Fireplace behavior matches the posted sequence.

## 2) Sensor Provisioning Parity

Validate sensor behavior in both conditions:

1. With valid credentials: sensor joins station WiFi and publishes telemetry.
2. Without valid credentials: sensor starts WPA2 AP `ThermostatSensor-AP` (password `ThermostatSetup`).

In AP mode, validate:

- `GET /api/network`
- `PUT /api/network`
- `POST /api/restart`

After saving valid credentials and restarting, sensor should join station mode and publish to:

- `thermostat/sensor/temperature`
- `thermostat/sensor/humidity`
- `thermostat/sensor/status`

## 3) OTA Status Endpoint

Controller:

```bash
curl -s http://<controller-ip>/api/ota/status
```

Sensor:

```bash
curl -s http://<sensor-ip>/api/ota/status
```

Confirm fields are present:

- `supported`
- `inProgress`
- `bytesWritten`
- `lastError`

## 4) OTA Apply Dry Run (Safe Failure)

Use a known bad URL to validate error propagation without flashing:

```bash
curl -s -X POST http://<controller-ip>/api/ota/apply \
  -H 'content-type: application/json' \
  -d '{"url":"http://invalid.local/firmware.bin","reboot":false}'
```

Expected:

- Start request may succeed (job accepted).
- `/api/ota/status` should eventually show a `lastError` describing download failure.

## 5) OTA Apply Real Image

Use a reachable firmware image URL and optional SHA256:

```bash
curl -s -X POST http://<controller-ip>/api/ota/apply \
  -H 'content-type: application/json' \
  -d '{"url":"https://<host>/controller.bin","sha256":"<64-hex>","reboot":true,"password":"<ota-password-if-set>"}'
```

Monitor:

```bash
watch -n 1 "curl -s http://<controller-ip>/api/ota/status"
```

Pass criteria:

- `inProgress` becomes `false`.
- `lastError` is empty.
- Device reboots (if `reboot=true`) and comes back online.

## Notes

- OTA apply is only meaningful in ESP32 builds.
- In provisioning AP mode, OTA apply is intentionally blocked.
- IR diagnostics counters prove command transmission attempts, not physical reception by the fireplace.
