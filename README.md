# Smart Thermostat

A DIY smart thermostat system for IR-controlled fireplaces, built with ESP32 microcontrollers. Features automatic temperature control, a responsive web interface, and MQTT integration for home automation.

## Features

- **Automatic Temperature Control**: Set a target temperature and let the thermostat manage your fireplace
- **Web Interface**: Control everything from your phone or computer via a responsive web UI
- **MQTT Integration**: Works with Home Assistant, Node-RED, or any MQTT-based home automation
- **Safety Features**:
  - Maximum runtime limits (4 hours) with automatic cooldown
  - Sensor timeout detection
  - Manual override with hold functionality
- **External Remote Detection**: Detects when someone uses the physical remote and adjusts accordingly
- **OTA Updates**: Flash firmware over WiFi without needing physical access
- **WiFiManager**: Easy WiFi setup via captive portal on first boot

## Hardware Requirements

### Controller Node (ESP32-S3)
- ESP32-S3-DevKitC-1 (or similar ESP32-S3 board)
- IR LED (940nm recommended)
- 2N2222 transistor (for IR LED driver circuit)
- 1k ohm resistor (base resistor)
- 56-68 ohm resistor (LED current limiting)
- Optional: IR receiver module (for learning new remotes)

### Sensor Node (ESP32)
- ESP32 DevKit or similar
- DHT11 temperature/humidity sensor
- 10k ohm pull-up resistor

### Wiring

**Controller Node IR Circuit:**
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
thermostat/
├── controller-node/     # ESP32-S3: IR control, thermostat logic, web UI
│   ├── src/
│   │   ├── main.cpp         # Main application
│   │   ├── thermostat.cpp   # Temperature control logic
│   │   ├── ir_controller.cpp # IR transmission
│   │   ├── web_server.cpp   # REST API endpoints
│   │   ├── config.h         # IR codes and settings
│   │   └── secrets.h        # Your credentials (not committed)
│   └── data/                # Web UI files (LittleFS)
├── sensor-node/         # ESP32: Temperature sensor
│   └── src/
│       ├── main.cpp
│       └── secrets.h        # Your credentials (not committed)
└── tools/
    └── ir_learner/      # Utility for capturing IR codes
```

## Setup

### 1. Install PlatformIO

Install [PlatformIO IDE](https://platformio.org/install/ide?install=vscode) for VS Code or the CLI.

### 2. Configure Credentials

Copy the example secrets file and fill in your values:

```bash
# Controller node
cp controller-node/src/secrets.h.example controller-node/src/secrets.h

# Sensor node
cp sensor-node/src/secrets.h.example sensor-node/src/secrets.h
```

Edit each `secrets.h` with your:
- MQTT broker IP address
- MQTT username and password
- OTA update password (controller only)
- Static IP configuration (controller only, optional)

### 3. Build and Flash

**Controller Node:**
```bash
cd controller-node

# Build and flash firmware (first time via USB)
pio run -t upload

# Upload web UI files to LittleFS
pio run -t uploadfs
```

**Sensor Node:**
```bash
cd sensor-node
pio run -t upload
```

### 4. WiFi Setup

On first boot, each device creates a WiFi access point:
- Controller: `ThermostatController-AP`
- Sensor: `ThermostatSensor-AP`

Connect to the AP and configure your WiFi credentials via the captive portal.

## Usage

### Web Interface

Access the web UI at your controller's IP address (default: `http://192.168.0.118` if using static IP).

Features:
- View current temperature and humidity
- Set target temperature
- Toggle thermostat mode (OFF/HEAT)
- Manual fireplace control (power, heat level, lights, timer)
- Configure hysteresis setting

### MQTT Topics

**Sensor Data (published by sensor node):**
- `thermostat/sensor/temperature` - Current temperature (°F)
- `thermostat/sensor/humidity` - Current humidity (%)

**Controller State (published by controller):**
- `thermostat/controller/state` - JSON with full system state

**Commands (subscribe to control):**
- `thermostat/cmnd/fireplace/power` - `on` or `off`
- `thermostat/cmnd/thermostat/target` - Target temp (e.g., `72`)
- `thermostat/cmnd/thermostat/mode` - `HEAT` or `OFF`
- `thermostat/cmnd/thermostat/hold` - `on`, `off`, or minutes (e.g., `30`)

### OTA Updates

After initial setup, flash updates over WiFi:

```bash
# Set your OTA password
export PLATFORMIO_UPLOAD_FLAGS="--auth=YOUR_OTA_PASSWORD"

# Flash (uses espota protocol)
cd controller-node
pio run -t upload
```

## Configuration

### Static IP (Optional)

The controller uses static IP by default. To use DHCP instead, comment out `#define USE_STATIC_IP` in `main.cpp`.

### IR Codes

IR codes for SimpliFire fireplaces are included in `config.h`. To capture codes for other remotes:

1. Build and flash `tools/ir_learner`
2. Point your remote at the IR receiver
3. Press buttons and copy the captured raw timing arrays
4. Add them to `config.h`

### Safety Limits

Configurable in `config.h`:
- `MAX_RUNTIME_MS` - Maximum continuous heating (default: 4 hours)
- `COOLDOWN_DURATION_MS` - Cooldown after max runtime (default: 30 min)
- `SENSOR_STALE_TIMEOUT` - Turn off if no sensor data (default: 5 min)

## Troubleshooting

**Can't connect to WiFi:**
- ESP32 only supports 2.4GHz networks, not 5GHz
- Reset WiFi settings by uncommenting `wifiManager.resetSettings()` in setup

**IR commands not working:**
- Verify GPIO4 is connected correctly
- Check IR LED circuit with phone camera (IR is visible on most cameras)
- Ensure you're using the correct IR codes for your fireplace

**Web UI not loading:**
- Ensure LittleFS was uploaded: `pio run -t uploadfs`
- Check serial monitor for errors

**OTA upload fails:**
- Verify OTA password matches
- Ensure device is on same network
- Close serial monitor before flashing

## License

MIT License - see [LICENSE](LICENSE) file.
