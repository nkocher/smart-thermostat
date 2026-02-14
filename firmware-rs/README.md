# Rust Rewrite Workspace

This directory contains the Rust rewrite for the smart thermostat project.

## Workspace crates

- `common`: Shared thermostat state machine, schedule engine, MQTT topics, and API models.
- `controller`: Controller service (REST + MQTT + schedule + control loop).
- `sensor`: Sensor publisher service (MQTT temperature/humidity publisher).

## Current status

This is the first migration slice with contract parity and new scheduling APIs implemented in Rust.

- Existing controller REST routes are preserved (`/api/status`, `/api/target`, `/api/mode`, `/api/hysteresis`, `/api/offset`, manual IR/hold/safety routes).
- Existing MQTT topics are preserved.
- Additive scheduling APIs are present (`/api/schedule`, `/api/time`, `/api/timezone`).
- Provisioning API is available for persisted network config (`/api/network` GET/PUT) in both host and ESP controller modes.
- New web UI is in `controller/web` and served by the controller binary.
- `controller` and `sensor` now support dual runtime paths:
  - default host mode (`tokio` + `axum` + `rumqttc`)
  - `esp32` feature mode (`esp-idf-svc` WiFi/MQTT/HTTP/NVS scaffolding)
- Host controller mode now persists runtime settings and schedules to JSON files in `./.thermostat` by default.
- `controller` ESP mode now includes an RMT-backed IR sender (36 kHz carrier) that maps `EngineAction` to captured raw code tables.
  - Current defaults use `RMT channel0` and `GPIO4` for IR transmit, matching the existing hardware wiring.
  - IR hardware config is now persisted and configurable via `/api/ir/config` (`txPin`, `rmtChannel`, `carrierKHz`), with existing defaults preserved.
  - IR diagnostics are exposed via `/api/ir/diagnostics` (transmit counters, last error, runtime IR state).
- Host controller mode now mirrors `/api/ir/config`, `/api/ir/diagnostics`, and `/api/ota/*` routes so the web UI stays target-agnostic.
- `controller` ESP mode now falls back to a WPA2 provisioning AP (`ThermostatController-AP`) when station WiFi is missing/invalid/unreachable.
  - Default provisioning password is `ThermostatSetup`.
  - Provisioning mode serves a minimal setup portal and supports `/api/network` + `/api/restart`.
- `sensor` ESP mode now falls back to a WPA2 provisioning AP (`ThermostatSensor-AP`) when station WiFi is missing/invalid/unreachable.
  - Default provisioning password is `ThermostatSetup`.
  - Sensor provisioning mode serves `/api/network` + `/api/restart` parity endpoints.
- ESP static IP settings are now applied at WiFi netif bring-up for both controller and sensor when `useStaticIp=true`.
- `sensor` ESP mode now reads hardware sensors directly:
  - DS18B20 temperature on `GPIO4` via one-wire
  - DHT11 humidity on `GPIO16`
- OTA status/apply APIs are now wired for ESP controller + sensor:
  - `GET /api/ota/status`
  - `POST /api/ota/apply` (`url`, optional `sha256`, optional `password`, optional `reboot`)
  - OTA apply is blocked while device is in provisioning AP mode.
- Hardware validation workflow is documented in `HARDWARE_VALIDATION.md` with an executable smoke script at `tools/hardware_validation.sh`.

## Run locally

```bash
cd firmware-rs
cargo check
cargo test -p thermostat-common

# terminal 1
MQTT_HOST=127.0.0.1 cargo run -p thermostat-controller

# terminal 2
MQTT_HOST=127.0.0.1 cargo run -p thermostat-sensor
```

By default, controller HTTP listens on `0.0.0.0:8080`.

## ESP32 build mode

Enable ESP mode with:

```bash
cargo check -p thermostat-controller --features esp32
cargo check -p thermostat-sensor --features esp32
```

This requires an ESP-IDF target/toolchain environment and target triple (for example `xtensa-esp32-espidf` or `xtensa-esp32s3-espidf`). Running `--features esp32` on a host target like `x86_64-apple-darwin` fails as expected.

## Environment variables

- `MQTT_HOST` (default `127.0.0.1`)
- `MQTT_PORT` (default `1883`)
- `MQTT_USER` (optional)
- `MQTT_PASS` (optional)
- `CONTROLLER_HTTP_PORT` (controller only, default `8080`)
- `THERMOSTAT_DATA_DIR` (controller host mode only, default `./.thermostat`)

## Next ESP32 integration steps

- Add signed firmware validation and release-channel controls for OTA artifacts.
- Add persisted telemetry for command delivery quality (including missed-command heuristics).
- Expand captive-portal polish (DNS hijack + vendor probe nuances) and AP UX hardening.
