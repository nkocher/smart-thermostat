# Smart Thermostat

A DIY smart thermostat system for IR-controlled fireplaces, built with Rust on ESP32 microcontrollers. Features automatic temperature control, a responsive web interface, MQTT integration, and over-the-air firmware updates.

## Features

- **Automatic Temperature Control** — set a target temperature and the thermostat manages the fireplace via IR
- **Web Interface** — Nest-style thermostat dial, mobile-first, served directly from the ESP32
- **MQTT Integration** — works with Home Assistant, Node-RED, or any MQTT broker
- **Safety System** — runtime limits (4h), sensor stale detection (5m), absolute max temperature ceiling (95F), immediate shutoff on mode-off or sensor loss
- **OTA Updates** — pull-model firmware updates over WiFi (A/B partition scheme with rollback)
- **Captive Portal** — WiFi provisioning on first boot via access point
- **Hold Mode** — temporary temperature override with configurable duration
- **Scheduling** — time-based temperature profiles with timezone support

## Hardware

### Controller Node (ESP32-S3)
- ESP32-S3-DevKitC-1
- IR LED (940nm) with 2N2222 driver circuit on GPIO4
- Optional: IR receiver on GPIO14

### Sensor Node (ESP32)
- ESP32 DevKit
- DHT11 on GPIO4 (temperature + humidity)

### Wiring

**Controller IR Circuit:**
```
GPIO4 -> 1kOhm -> 2N2222 Base
5V -> 56-68 Ohm -> IR LED Anode
IR LED Cathode -> 2N2222 Collector
2N2222 Emitter -> GND
```

**Sensor Node:**
```
DHT11 VCC  -> 3.3V
DHT11 DATA -> GPIO4 (+ 10K pull-up to 3.3V)
DHT11 GND  -> GND
```

## Project Structure

```
firmware-rs/             # Active Rust/ESP-IDF firmware
├── common/              # Shared crate (thermostat logic, config, scheduling)
├── controller/          # Controller firmware (WiFi, HTTP, IR, MQTT, OTA)
│   ├── src/esp.rs       # Main application
│   ├── src/ir.rs        # IR transmitter (RMT driver)
│   ├── src/ir_codes.rs  # Raw IR timing arrays
│   └── web/             # Embedded webapp (HTML/CSS/JS)
└── sensor/              # Sensor firmware (temp/humidity via MQTT)

controller-node/         # Legacy C++ firmware (reference only)
sensor-node/             # Legacy C++ firmware (reference only)
tools/ir_learner/        # IR code capture utility
```

## Build & Flash

Requires the [Rust ESP32 toolchain](https://github.com/esp-rs/rust-build) and `espflash`.

**Controller:**
```bash
cd firmware-rs/controller
cargo build --release --features esp32
espflash flash -p /dev/cu.usbmodem* target/xtensa-esp32s3-espidf/release/thermostat-controller
```

**Sensor:**
```bash
cd firmware-rs/sensor
cargo build --release --features esp32
espflash flash -p /dev/cu.usbserial* target/xtensa-esp32-espidf/release/thermostat-sensor
```

## OTA Updates

The controller and sensor support pull-model OTA: the device downloads a binary from a URL you provide.

```bash
# Create OTA binary
espflash save-image --chip esp32s3 target/xtensa-esp32s3-espidf/release/thermostat-controller /tmp/ota.bin

# Serve it
cd /tmp && python3 -m http.server 8080 --bind 0.0.0.0

# Trigger update
curl -X POST http://<controller-ip>/api/ota/apply \
  -H 'Content-Type: application/json' \
  -d '{"url":"http://<your-ip>:8080/ota.bin","reboot":true}'
```

Uses A/B partitions (ota_0/ota_1) with automatic rollback safety.

## MQTT Topics

**Sensor readings:**
- `thermostat/sensor/temperature` — current temperature (F)
- `thermostat/sensor/humidity` — current humidity (%)

**Controller state:**
- `thermostat/controller/state` — JSON with full system state

**Commands:**
- `thermostat/cmnd/fireplace/power` — `on` or `off`
- `thermostat/cmnd/thermostat/target` — target temp (e.g., `72`)
- `thermostat/cmnd/thermostat/mode` — `HEAT` or `OFF`
- `thermostat/cmnd/thermostat/hold` — `on`, `off`, or minutes (e.g., `30`)

## API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/status` | Full thermostat state |
| POST | `/api/target?value=XX` | Set target temperature |
| POST | `/api/mode?value=OFF\|HEAT` | Set operating mode |
| POST | `/api/ir/{on,off,heat/on,heat/off,...}` | Manual IR commands |
| POST | `/api/hold/enter?minutes=N` | Enter hold mode |
| POST | `/api/hold/exit` | Exit hold mode |
| GET/PUT | `/api/network` | WiFi, MQTT, static IP config |
| GET/PUT | `/api/schedule` | Schedule entries |
| POST | `/api/safety/reset` | Reset safety lockout |

## Safety

The thermostat prioritizes safety with multiple independent shutoff mechanisms:

- **Runtime limit** — automatic PowerOff after 4 hours of continuous heating
- **Sensor stale** — immediate PowerOff if no sensor data for 5 minutes (bypasses cycle throttle)
- **Temperature ceiling** — emergency shutoff at 95F, overrides all other logic
- **Mode-off** — immediate PowerOff when user sets mode to OFF (bypasses cycle throttle)

## License

MIT License — see [LICENSE](LICENSE).
